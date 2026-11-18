//! MiniMax Code's records: its sessions as the local runtime it (and MiniMax's desktop app) runs on keeps them, each in
//! `<data>/v2/sessions/<yyyy>/<mm>/<dd>/<folder>/`: `manifest.json` names the session, `messages.jsonl` holds its
//! messages as they stand (each in an envelope with its own id), the model's answers with what they spent (`usage`)
//! and the model that answered, and `snapshots/*.jsonl` its messages as they stood before each time it was compacted
//! (an answer kept by the compaction keeps its id, and is counted once). Its runtime's database
//! (`<data>/v2/sqlite/runtime-state.sqlite`) names each session and the folder it works in: what was spent is never read
//! from it (it is told there only where writing it did not fail, and with no model). Read as Token Monitor's tokscale
//! (MIT, Copyright (c) 2025 Junho Yeo) reads them: an answer's input counts nothing read from the cache, and its
//! reasoning is among its output unless it cannot be (more than it, or counted beside it by the total told).

use super::{key, Read, Told, Tokens};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Lines that say anything read here hold this word: the rest are not parsed.
const WORTH: &[u8] = b"\"assistant\"";

/// Where MiniMax Code keeps its data: in the folder `MINIMAX_DATA_DIR` names, else `MAVIS_DATA_DIR` (its earlier
/// name), where one is set; else in `.minimax` in the user's folder `home`, `.mavis` (where earlier releases kept it),
/// and those of each profile beside them (`.minimax-<profile>`, `.mavis-<profile>`).
pub(super) fn data_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    if let Some(dir) = ["MINIMAX_DATA_DIR", "MAVIS_DATA_DIR"].iter().find_map(|name| std::env::var_os(name).filter(|dir| !dir.to_string_lossy().trim().is_empty())) {
        return vec![PathBuf::from(dir)];
    }
    let Some(home) = home else { return Vec::new() };
    const NAMES: [&str; 2] = [".minimax", ".mavis"];
    let mut profiles: Vec<PathBuf> = std::fs::read_dir(home)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            NAMES.iter().any(|base| name.strip_prefix(base).and_then(|rest| rest.strip_prefix('-')).is_some_and(|profile| !profile.is_empty()))
        })
        .map(|entry| entry.path())
        .collect();
    profiles.sort();
    NAMES.iter().map(|name| home.join(name)).chain(profiles).collect()
}

#[derive(Deserialize)]
struct Manifest {
    #[serde(rename = "sessionId")]
    session: String,
}

/// The session the record at `path` is of: as the manifest of its session's folder (the folder it is in, or that
/// folder's, for a snapshot) names it.
pub(super) fn session_of(path: &Path) -> Option<String> {
    path.ancestors().skip(1).take(2).find_map(|folder| {
        let manifest: Manifest = serde_json::from_slice(&std::fs::read(folder.join("manifest.json")).ok()?).ok()?;
        Some(manifest.session.trim().to_string()).filter(|session| !session.is_empty())
    })
}

#[derive(Deserialize)]
struct Envelope {
    message_id: String,
    message: Message,
}

#[derive(Deserialize)]
struct Message {
    role: String,
    model: Option<String>,
    usage: Option<serde_json::Value>,
    timestamp: Option<serde_json::Value>,
}

/// A number, or words that are one (the runtime takes both): a count or a time.
fn number(value: &serde_json::Value) -> Option<f64> {
    let number = match value {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }?;
    number.is_finite().then_some(number)
}

/// What `usage` spent, as the runtime itself reads it: its input (nothing read from the cache among it), or, telling
/// neither input nor output, its `total_tokens` as input, what was read from and written to the cache (as each kind
/// of provider tells it), and its reasoning among its output, or beside it where it cannot be among it.
pub(super) fn tokens_of(usage: &serde_json::Value) -> Tokens {
    let field = |name: &str| usage.get(name).and_then(number);
    let count = |value: f64| if value > 0.0 { value as u64 } else { 0 };
    let (input, output) = (field("input").unwrap_or(0.0), field("output").unwrap_or(0.0));
    let (input, output) = if input > 0.0 || output > 0.0 { (input, output) } else { (field("total_tokens").unwrap_or(0.0), 0.0) };
    let cached = |flat: &str, nested: &str, snake: &str| field(flat).or_else(|| usage.get("cache").and_then(|cache| cache.get(nested)).and_then(number)).or_else(|| field(snake)).unwrap_or(0.0);
    let (cache_read, cache_write) = (cached("cacheRead", "read", "cache_read"), cached("cacheWrite", "write", "cache_write"));
    let reasoning = field("reasoning").unwrap_or(0.0);
    let total = field("totalTokens").unwrap_or(0.0);
    let beside = reasoning > output || (total > 0.0 && total == input + output + cache_read + cache_write + reasoning);
    let (output, reasoning) = (count(output), count(reasoning));
    Tokens { input: count(input), output: if beside { output + reasoning } else { output }, cache_read: count(cache_read), cache_write: count(cache_write), reasoning }
}

/// What line `bytes` of a session's record tells, into `told`; `read` holds the session it is of (see `session_of`).
pub(super) fn line(bytes: &[u8], read: &mut Read, told: &mut Vec<Told>) {
    if memchr::memmem::find(bytes, WORTH).is_none() {
        return;
    }
    let Some(session) = read.session.clone() else { return };
    let Ok(envelope) = serde_json::from_slice::<Envelope>(bytes) else { return };
    let message = envelope.message;
    if message.role != "assistant" {
        return;
    }
    let Some(tokens) = message.usage.as_ref().map(tokens_of).filter(|tokens| tokens.total() > 0) else { return };
    // (Its time in ms, or in seconds.)
    let Some(at) = message.timestamp.as_ref().and_then(number).filter(|at| *at > 0.0).map(|at| at as u64) else { return };
    let at = if at < 10_000_000_000 { at * 1000 } else { at };
    let model = message.model.map(|model| model.trim().to_string()).filter(|model| !model.is_empty()).unwrap_or_else(|| "unknown".into());
    told.push(Told::Spent { key: key(&["mcode", &session, &envelope.message_id]), at, session, model, tokens });
}

/// What the runtime's database at `path` tells of its sessions: the name each was given, and the folder it works in
/// (none where it works in the runtime's own).
#[cfg(windows)]
pub(super) fn told_in(path: &Path) -> Vec<Told> {
    let Some(database) = super::sqlite::Database::read(path) else { return Vec::new() };
    let ask = "SELECT session_id, title, CASE WHEN is_default_workspace = 1 THEN NULL ELSE COALESCE(NULLIF(project_workspace_dir, ''), workspace_dir) END FROM local_runtime_sessions";
    let Some(rows) = database.rows(ask) else { return Vec::new() };
    let mut told = Vec::new();
    for row in rows {
        let at = |index: usize| row.get(index).and_then(super::sqlite::Value::text);
        let Some(session) = at(0).map(|session| session.trim().to_string()).filter(|session| !session.is_empty()) else { continue };
        if let Some(folder) = at(2).filter(|folder| !folder.trim().is_empty()) {
            told.push(Told::Folder { session: session.clone(), folder });
        }
        super::claude::title(&mut told, session, at(1), super::TitleKind::Named);
    }
    told
}
