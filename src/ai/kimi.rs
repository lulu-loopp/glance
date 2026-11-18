//! Kimi Code's records: each of a session's agents writes what happens in it to its own `wire.jsonl`
//! (`sessions/<workspace>/session_<id>/agents/<agent>/wire.jsonl`), a line for each event: what each request to the
//! model spent (`usage.record`: its model, and its tokens by kind), the folder it works in (`profile.bind`), and each
//! thing asked (`turn.prompt`, by the user or, a subagent's task, by the agent that started it). A session is known by
//! its folder's name.

use super::{key, Read, TitleKind, Told, Tokens};
use serde::Deserialize;
use std::path::Path;

/// Lines that say anything read here hold one of these words: the rest are not parsed.
const WORTH: [&[u8]; 3] = [b"\"usage.record\"", b"\"profile.bind\"", b"\"turn.prompt\""];

#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "agentId")]
    agent: Option<String>,
    time: Option<u64>,
    model: Option<String>,
    usage: Option<Usage>,
    #[serde(rename = "usageScope")]
    scope: Option<String>,
    #[serde(rename = "environmentDisclosure")]
    environment: Option<Environment>,
    input: Option<Vec<Input>>,
    origin: Option<Origin>,
}

/// Who asked: the user, or (a subagent's task) the agent that started it.
#[derive(Deserialize)]
struct Origin {
    kind: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Usage {
    #[serde(default)]
    input_other: u64,
    #[serde(default)]
    output: u64,
    #[serde(default)]
    input_cache_read: u64,
    #[serde(default)]
    input_cache_creation: u64,
}

#[derive(Deserialize)]
struct Environment {
    cwd: Option<String>,
}

#[derive(Deserialize)]
struct Input {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
}

/// The session the record at `path` is of: the name of its `session_<id>` folder, its id.
pub(super) fn session_of(path: &Path) -> Option<String> {
    path.ancestors().filter_map(|folder| folder.file_name()?.to_str()?.strip_prefix("session_")).next().map(String::from)
}

/// What line `bytes` of a session's record tells, into `told`; `read` holds the session it is of (see `session_of`).
pub(super) fn line(bytes: &[u8], read: &mut Read, told: &mut Vec<Told>) {
    if !WORTH.iter().any(|word| memchr::memmem::find(bytes, word).is_some()) {
        return;
    }
    let Some(session) = read.session.clone() else { return };
    let Ok(line) = serde_json::from_slice::<Line>(bytes) else { return };
    match line.kind.as_deref() {
        Some("usage.record") => {
            let (Some(usage), Some(model), Some(at)) = (line.usage, line.model, line.time) else { return };
            let tokens = Tokens { input: usage.input_other, output: usage.output, cache_read: usage.input_cache_read, cache_write: usage.input_cache_creation, reasoning: 0 };
            // (A request is known by its session, its agent and when it was answered.)
            let key = key(&["kimi", &session, line.agent.as_deref().unwrap_or_default(), &at.to_string(), line.scope.as_deref().unwrap_or_default()]);
            told.push(Told::Spent { key, at, session, model, tokens });
        }
        Some("profile.bind") => {
            if let Some(folder) = line.environment.and_then(|environment| environment.cwd) {
                told.push(Told::Folder { session, folder });
            }
        }
        // (What the user asked, not a subagent's task.)
        Some("turn.prompt") if line.origin.and_then(|origin| origin.kind).as_deref() == Some("user") => {
            let asked = line.input.unwrap_or_default().into_iter().filter(|input| input.kind.as_deref() == Some("text")).find_map(|input| input.text);
            super::claude::title(told, session, asked, TitleKind::Asked);
        }
        _ => {}
    }
}
