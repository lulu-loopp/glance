//! Codex's session logs: the session and its folder first (`session_meta`), the model of each turn
//! (`turn_context`), and after each response what it spent and the session's total so far (`token_count`, told again
//! unchanged at times: one whose total is the one before it spent nothing new), with the limits as they stood; and
//! the sessions' names, in the index beside them (`session_index.jsonl`: `thread_name`).
//!
//! Codex's input counts what came from its cache among it, and its output its reasoning: here input is only what was
//! read afresh.
//!
//! The limits are the ChatGPT account's, shared by every place it is used in (another PC, another person signed in
//! with it): what the logs on this PC tell is only what they stood at as Codex last worked here. So they are also
//! asked of the account itself, as Codex asks for them, with the sign-in Codex keeps (see `signed_in`,
//! `providers::codex`): read, never changed, and sent only to ChatGPT.

use super::{key, moment, Limit, Read, TitleKind, Told, Tokens, Tool, Window};
use serde::Deserialize;

/// The id of the account's own limit (others are those of a model limited apart).
pub(super) const ACCOUNT_LIMIT: &str = "codex";

const WORTH: [&[u8]; 4] = [b"\"token_count\"", b"\"session_meta\"", b"\"turn_context\"", b"\"thread_name\""];

#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: Option<String>,
    timestamp: Option<String>,
    payload: Option<Payload>,
    // (The index's.)
    id: Option<String>,
    thread_name: Option<String>,
}

#[derive(Deserialize)]
struct Payload {
    #[serde(rename = "type")]
    kind: Option<String>,
    id: Option<String>,
    cwd: Option<String>,
    model: Option<String>,
    info: Option<Info>,
    rate_limits: Option<Limits>,
}

#[derive(Deserialize)]
struct Info {
    total_token_usage: Option<Usage>,
    last_token_usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    cached_input_tokens: u64,
    #[serde(default)]
    cache_write_input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    reasoning_output_tokens: u64,
    #[serde(default)]
    total_tokens: u64,
}

#[derive(Deserialize)]
struct Limits {
    limit_id: Option<String>,
    limit_name: Option<String>,
    plan_type: Option<String>,
    primary: Option<LimitWindow>,
    secondary: Option<LimitWindow>,
}

#[derive(Deserialize)]
struct LimitWindow {
    used_percent: Option<f32>,
    window_minutes: Option<u32>,
    /// When it starts again: seconds since 1970 (as Codex tells it now), or from the line's time (as it did).
    resets_at: Option<u64>,
    resets_in_seconds: Option<u64>,
}

/// What line `bytes` of a session log (or of the index of their names) tells, into `told`; `read` keeps the session,
/// the model and the last total.
pub(super) fn line(bytes: &[u8], read: &mut Read, told: &mut Vec<Told>) {
    if !WORTH.iter().any(|word| memchr::memmem::find(bytes, word).is_some()) {
        return;
    }
    let Ok(line) = serde_json::from_slice::<Line>(bytes) else { return };
    if let (Some(session), Some(name)) = (line.id, line.thread_name) {
        if !name.trim().is_empty() {
            told.push(Told::Title { session, title: name.trim().to_string(), kind: TitleKind::Named });
        }
        return;
    }
    let Some(payload) = line.payload else { return };
    let at = line.timestamp.as_deref().and_then(moment);
    match (line.kind.as_deref(), payload.kind.as_deref()) {
        (Some("session_meta"), _) => {
            read.session = payload.id;
            read.total = None;
            folder(read, payload.cwd, told);
        }
        (Some("turn_context"), _) => {
            if payload.model.is_some() {
                read.model = payload.model;
            }
            folder(read, payload.cwd, told);
        }
        (Some("event_msg"), Some("token_count")) => {
            let Some(at) = at else { return };
            if let Some(limits) = payload.rate_limits {
                limit(limits, at, told);
            }
            let Some(Info { total_token_usage: Some(total), last_token_usage: Some(last) }) = payload.info else { return };
            let (Some(session), Some(model)) = (read.session.clone(), read.model.clone()) else { return };
            if read.total == Some(total.total_tokens) {
                return;
            }
            read.total = Some(total.total_tokens);
            let tokens = Tokens {
                input: last.input_tokens.saturating_sub(last.cached_input_tokens + last.cache_write_input_tokens),
                output: last.output_tokens,
                cache_read: last.cached_input_tokens,
                cache_write: last.cache_write_input_tokens,
                reasoning: last.reasoning_output_tokens,
            };
            // (Counted by when it was told and what it told, not by its session: a session forked from another starts
            // with a copy of what that one spent, under its own name.)
            let stamp = line.timestamp.unwrap_or_default();
            told.push(Told::Spent { key: key(&["codex", &stamp, &total.total_tokens.to_string(), &last.total_tokens.to_string()]), at, session, model, tokens });
        }
        _ => {}
    }
}

/// The sign-in Codex keeps in its folder (`CODEX_HOME`, else `.codex`), where it signed in with a ChatGPT account and
/// keeps it in a file (as it does unless told to keep it in Windows' own store): its access token, and the account's
/// id where it names one. Read as it stands each time, never written: Codex renews it itself.
pub fn signed_in() -> Option<(String, Option<String>)> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(std::path::PathBuf::from);
    let folder = std::env::var_os("CODEX_HOME").map(std::path::PathBuf::from).or_else(|| home.map(|home| home.join(".codex")))?;
    sign_in_of(&std::fs::read(folder.join("auth.json")).ok()?)
}

/// The access token and the account's id `bytes` (Codex's `auth.json`) keep, where it is a ChatGPT account's.
fn sign_in_of(bytes: &[u8]) -> Option<(String, Option<String>)> {
    #[derive(Deserialize)]
    struct Auth {
        tokens: Option<Tokens>,
    }
    #[derive(Deserialize)]
    struct Tokens {
        access_token: String,
        account_id: Option<String>,
    }
    let tokens = serde_json::from_slice::<Auth>(bytes).ok()?.tokens?;
    let token = tokens.access_token.trim();
    (!token.is_empty()).then(|| (token.to_string(), tokens.account_id.filter(|id| !id.trim().is_empty())))
}

/// The session worked in `folder`.
fn folder(read: &Read, folder: Option<String>, told: &mut Vec<Told>) {
    if let (Some(session), Some(folder)) = (read.session.clone(), folder) {
        told.push(Told::Folder { session, folder });
    }
}

/// A limit as it stood at `at` (one that names neither itself nor a window: none).
fn limit(limits: Limits, at: u64, told: &mut Vec<Told>) {
    let windows: Vec<Window> = [limits.primary, limits.secondary]
        .into_iter()
        .flatten()
        .filter_map(|window| {
            let resets = window.resets_at.map(|at| at * 1000).or(window.resets_in_seconds.map(|after| at + after * 1000));
            Some(Window { minutes: window.window_minutes?, used: window.used_percent?, resets })
        })
        .collect();
    if windows.is_empty() {
        return;
    }
    let id = limits.limit_id.unwrap_or_else(|| ACCOUNT_LIMIT.to_string());
    told.push(Told::Limit(Limit { tool: Tool::Codex, id, name: limits.limit_name, plan: limits.plan_type, windows, balance: None, told: at }));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sign-in Codex keeps, as it writes it: a ChatGPT account's access token and its id; none where it is signed
    /// in with an API key alone, or not at all.
    #[test]
    fn reads_the_chatgpt_sign_in_codex_keeps() {
        let chatgpt = br#"{"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":{"id_token":"x.y.z","access_token":"a.b.c","refresh_token":"r","account_id":"acct-1"},"last_refresh":"2026-11-19T22:03:24Z"}"#;
        assert_eq!(sign_in_of(chatgpt), Some(("a.b.c".to_string(), Some("acct-1".to_string()))));
        let no_account = br#"{"tokens":{"id_token":"x.y.z","access_token":"a.b.c","refresh_token":"r"}}"#;
        assert_eq!(sign_in_of(no_account), Some(("a.b.c".to_string(), None)));
        assert_eq!(sign_in_of(br#"{"OPENAI_API_KEY":"sk-1"}"#), None);
        assert_eq!(sign_in_of(b"not json"), None);
    }
}
