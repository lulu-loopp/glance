//! Claude Code's transcripts: a line for each thing said, the model's carrying what its response spent
//! (`message.usage`), repeated whole on each line of a response written in several (counted once, by the response's
//! id and its request's); a session's name given by hand (`custom-title`), the one Claude Code gave it (`ai-title`), its
//! summary (`summary`, which names no session: the file's), and what was last asked in it (`last-prompt`, which often
//! holds no prompt, and the prompts its user typed themselves).

/// A user's line whose message is text, not what tools gave back: what was asked, or what Claude Code put in as the
/// user's (see `asked`).
const PROMPT: &[u8] = b"\"role\":\"user\",\"content\":\"";

use super::{key, moment, Read, TitleKind, Told, Tokens};
use serde::Deserialize;

/// Lines that say anything read here hold one of these words: the rest (most of a transcript's bytes) are not parsed.
const WORTH: [&[u8]; 6] = [b"\"usage\"", b"\"custom-title\"", b"\"ai-title\"", b"\"summary\"", b"\"last-prompt\"", PROMPT];
/// The longest title kept (in characters).
const TITLE: usize = 80;

#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "sessionId")]
    session: Option<String>,
    #[serde(rename = "requestId")]
    request: Option<String>,
    cwd: Option<String>,
    message: Option<Message>,
    #[serde(rename = "customTitle")]
    custom_title: Option<String>,
    #[serde(rename = "aiTitle")]
    ai_title: Option<String>,
    summary: Option<String>,
    #[serde(rename = "lastPrompt")]
    last_prompt: Option<String>,
    /// A user's line Claude Code put in itself (what a command said, a caveat), not asked.
    #[serde(rename = "isMeta", default)]
    meta: bool,
    /// A user's line holding a summary of what came before, the session carried on from it.
    #[serde(rename = "isCompactSummary", default)]
    summed: bool,
    /// A subagent's line (its task, as the user's, in a file of its own under the session's).
    #[serde(rename = "isSidechain", default)]
    sidechain: bool,
    /// Where a user's line came from: typed, queued, a suggestion taken, or the system (none before Claude Code told).
    #[serde(rename = "promptSource")]
    source: Option<String>,
}

/// A user's line whose message is text: the text.
#[derive(Deserialize)]
struct Asked {
    message: Said,
}

#[derive(Deserialize)]
struct Said {
    content: String,
}

#[derive(Deserialize)]
struct Message {
    id: Option<String>,
    model: Option<String>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
}

/// What line `bytes` of a transcript tells, into `told`; `read` keeps the session the file is of (for a summary).
pub(super) fn line(bytes: &[u8], read: &mut Read, told: &mut Vec<Told>) {
    if !WORTH.iter().any(|word| memchr::memmem::find(bytes, word).is_some()) {
        return;
    }
    let Ok(line) = serde_json::from_slice::<Line>(bytes) else { return };
    if let Some(session) = &line.session {
        // (A summary read before the file named its session is its session's.)
        if read.session.is_none() {
            title(told, session.clone(), read.summary.take(), TitleKind::Summed);
        }
        read.session = Some(session.clone());
    }
    let session = line.session.clone().or_else(|| read.session.clone());
    if line.kind.as_deref() == Some("summary") && session.is_none() {
        read.summary = line.summary;
        return;
    }
    match (line.kind.as_deref(), session) {
        (Some("assistant"), Some(session)) => {
            if let Some(folder) = line.cwd {
                told.push(Told::Folder { session: session.clone(), folder });
            }
            let Some(message) = line.message else { return };
            let (Some(id), Some(usage), Some(model), Some(at)) = (message.id, message.usage, message.model, line.timestamp.as_deref().and_then(moment)) else { return };
            // (A response Claude Code made up itself, an error shown as one, spent nothing.)
            if model.starts_with('<') {
                return;
            }
            let tokens = Tokens { input: usage.input_tokens, output: usage.output_tokens, cache_read: usage.cache_read_input_tokens, cache_write: usage.cache_creation_input_tokens, reasoning: 0 };
            told.push(Told::Spent { key: key(&["claude", &id, line.request.as_deref().unwrap_or_default()]), at, session, model, tokens });
        }
        (Some("custom-title"), Some(session)) => title(told, session, line.custom_title, TitleKind::Named),
        (Some("ai-title"), Some(session)) => title(told, session, line.ai_title, TitleKind::Titled),
        (Some("summary"), Some(session)) => title(told, session, line.summary, TitleKind::Summed),
        (Some("last-prompt"), Some(session)) => title(told, session, line.last_prompt, TitleKind::Asked),
        (Some("user"), Some(session)) if asked(&line) => title(told, session, prompt(bytes), TitleKind::Asked),
        _ => {}
    }
}

/// Whether user's line `line` is what its user asked: not one Claude Code put in itself, a summary carried on from,
/// what the system queued, or a subagent's.
fn asked(line: &Line) -> bool {
    !line.meta && !line.summed && !line.sidechain && line.source.as_deref() != Some("system")
}

/// What user's line `bytes` asked, where its message is text and not a command's tags (`<command-name>` and the like,
/// how Claude Code writes a command typed and what it said back).
fn prompt(bytes: &[u8]) -> Option<String> {
    memchr::memmem::find(bytes, PROMPT)?;
    let text = serde_json::from_slice::<Asked>(bytes).ok()?.message.content;
    (!text.trim_start().starts_with('<')).then_some(text)
}

/// Session `session` called `title` (its first line, cut to `TITLE` characters), where it says anything.
pub(super) fn title(told: &mut Vec<Told>, session: String, title: Option<String>, kind: TitleKind) {
    let Some(title) = title else { return };
    let first = title.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default();
    if first.is_empty() {
        return;
    }
    let cut: String = first.chars().take(TITLE).collect();
    let title = if cut.len() < first.len() { format!("{cut}…") } else { cut };
    told.push(Told::Title { session, title, kind });
}
