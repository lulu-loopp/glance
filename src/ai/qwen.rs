//! Qwen Code's records: a session's lines in `~/.qwen/projects/<project>/chats/<session>.jsonl`, each the model's
//! answer's (`assistant`) with what it spent as Gemini tells it (`usageMetadata`), the folder it works in (`cwd`), and
//! each thing the user asked (`user`). What it spent is read as tokscale (MIT, Copyright (c) 2025 Junho Yeo) reads
//! it: its prompt counts what came from the cache among it, and its thoughts are among its answer's tokens where the
//! total it tells adds up only so.

use super::{key, moment, Read, TitleKind, Told, Tokens};
use serde::Deserialize;

/// Lines that say anything read here hold one of these words: the rest are not parsed.
const WORTH: [&[u8]; 2] = [b"\"usageMetadata\"", b"\"user\""];

#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: Option<String>,
    uuid: Option<String>,
    #[serde(rename = "sessionId")]
    session: Option<String>,
    timestamp: Option<String>,
    cwd: Option<String>,
    model: Option<String>,
    message: Option<Message>,
    #[serde(rename = "usageMetadata")]
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Message {
    parts: Option<Vec<Part>>,
}

#[derive(Deserialize)]
struct Part {
    text: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Usage {
    #[serde(default)]
    pub(super) prompt_token_count: u64,
    #[serde(default)]
    pub(super) candidates_token_count: u64,
    #[serde(default)]
    pub(super) thoughts_token_count: u64,
    #[serde(default)]
    pub(super) cached_content_token_count: u64,
    pub(super) total_token_count: Option<u64>,
}

/// What `usage` spent: its prompt less what came from the cache, read from the cache, and its answer with its
/// thoughts (among the answer's count where the total told is the prompt's and the answer's alone, else beside it).
pub(super) fn tokens_of(usage: &Usage) -> Tokens {
    let (prompt, answer, thoughts, cached) = (usage.prompt_token_count, usage.candidates_token_count, usage.thoughts_token_count, usage.cached_content_token_count);
    let input = if cached <= prompt { prompt - cached } else { prompt };
    let among = thoughts > 0
        && match usage.total_token_count {
            Some(0) => answer > 0,
            Some(total) => total == prompt + answer,
            None => false,
        };
    let (output, reasoning) = if among { (answer, thoughts.min(answer)) } else { (answer + thoughts, thoughts) };
    Tokens { input, output, cache_read: cached, cache_write: 0, reasoning }
}

/// What line `bytes` of a session's record tells, into `told`.
pub(super) fn line(bytes: &[u8], read: &mut Read, told: &mut Vec<Told>) {
    if !WORTH.iter().any(|word| memchr::memmem::find(bytes, word).is_some()) {
        return;
    }
    let Ok(line) = serde_json::from_slice::<Line>(bytes) else { return };
    let Some(session) = line.session.filter(|session| !session.is_empty()).or_else(|| read.session.clone()) else { return };
    read.session = Some(session.clone());
    if let Some(folder) = line.cwd {
        told.push(Told::Folder { session: session.clone(), folder });
    }
    match line.kind.as_deref() {
        Some("assistant") => {
            let (Some(usage), Some(at)) = (line.usage, line.timestamp.as_deref().and_then(moment)) else { return };
            let tokens = tokens_of(&usage);
            if tokens.total() == 0 {
                return;
            }
            let model = line.model.unwrap_or_else(|| "unknown".into());
            // (An answer is known by its own id, else by its session and when it came.)
            let key = match &line.uuid {
                Some(uuid) => key(&["qwen", uuid]),
                None => key(&["qwen", &session, &at.to_string()]),
            };
            told.push(Told::Spent { key, at, session, model, tokens });
        }
        Some("user") => {
            let asked = line.message.and_then(|message| message.parts).unwrap_or_default().into_iter().find_map(|part| part.text);
            super::claude::title(told, session, asked, TitleKind::Asked);
        }
        _ => {}
    }
}
