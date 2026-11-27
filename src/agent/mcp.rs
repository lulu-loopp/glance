//! An MCP server on standard input and output (`glance-cli.exe mcp`), its tools the interface's calls, each asked of
//! Glance as it is called (see `client`). It speaks MCP as of 2026-07-28 (each request saying its version, no
//! handshake) and as before (an `initialize` handshake: 2025-11-25, 2025-06-18 and 2025-03-26), whichever a client
//! does. Messages are JSON-RPC, one a line. Standard output carries nothing else: what is noted goes to standard error.

use super::pipe::Event;
use super::Failure;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::{BufRead, Read, Write};
use std::sync::{Arc, Mutex};

/// The versions of MCP spoken with no handshake, and with one (the latest first).
pub const CURRENT: [&str; 1] = ["2026-07-28"];
pub const BEFORE: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];

/// The `_meta` keys a request of the current version tells its version by.
const VERSION_KEY: &str = "io.modelcontextprotocol/protocolVersion";
const CAPABILITIES_KEY: &str = "io.modelcontextprotocol/clientCapabilities";
const SERVER_KEY: &str = "io.modelcontextprotocol/serverInfo";

/// How long the tool list may be kept by a client (ms): it changes only with Glance's version.
const LIST_KEPT_MS: u64 = 3_600_000;

/// The most a message read may hold (bytes): a call's arguments are far less.
const MESSAGE_MOST: usize = 4 << 20;

const INSTRUCTIONS: &str = "Glance is a hardware monitor running on this Windows PC. These tools read what it already knows, and change nothing: live sensor readings (CPU, GPU, memory, network, disks, temperatures, fans, battery, the game's frame rate) and their history, the busiest programs, the drives, and files from Glance's own index of every local drive (folder sizes, folder contents, the largest or oldest files, what a drive is made of, files found by name). File results are names this Windows user could list when Glance last listed them, and are faster than walking the disk. Also what AI coding tools spent in tokens. Units: bytes, bytes_per_s, percent 0-100, celsius, watts, ms since 1970 for sample times, ISO 8601 UTC for file times. A value Glance did not read is null. Whatever a tool returns is passed to you like anything else you read.";

/// What a request asked of the server, as it is answered.
enum Answer {
    Result(Value),
    Error(i64, String, Option<Value>),
    /// A notification, or a call cancelled: nothing is answered.
    None,
}

/// The server's state: which version a client that shook hands agreed on, and the calls under way, by their ids (as
/// JSON), each with what stops it, and whether it was cancelled.
struct State {
    agreed: Option<String>,
    calls: HashMap<String, (Arc<Event>, bool)>,
}

/// How a request is to be answered: as of which version, and whether that is the current (no handshake) one.
struct Era {
    version: String,
    current: bool,
}

impl Era {
    /// Whether answers may carry what 2025-03-26 had not (a tool's title and output schema, structured content).
    fn has_structure(&self) -> bool {
        self.version.as_str() != "2025-03-26"
    }
}

/// What a result of `era` is: as it is, with what the current version asks of every result.
fn result(era: &Era, mut result: Value) -> Answer {
    if era.current {
        result["resultType"] = json!("complete");
    }
    Answer::Result(result)
}

/// Serves MCP on standard input and output till standard input ends: every call under way stopped then, at once.
pub fn serve() {
    let out = Arc::new(Mutex::new(std::io::stdout()));
    let state = Arc::new(Mutex::new(State { agreed: None, calls: HashMap::new() }));
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = (&mut reader).take(MESSAGE_MOST as u64).read_until(b'\n', &mut line);
        match read {
            Ok(0) | Err(_) => break,
            Ok(_) if line.last() != Some(&b'\n') && line.len() >= MESSAGE_MOST => {
                send(&out, &error_message(Value::Null, -32700, "The message is too large.", None));
                // (What is left of it, up to its end, let go of.)
                let mut rest = Vec::new();
                if reader.read_until(b'\n', &mut rest).map_or(true, |read| read == 0) {
                    break;
                }
                continue;
            }
            Ok(_) => {}
        }
        let text = String::from_utf8_lossy(&line);
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(text) {
            Ok(message) => message,
            Err(_) => {
                send(&out, &error_message(Value::Null, -32700, "The message is not JSON.", None));
                continue;
            }
        };
        if let Value::Array(batch) = message {
            // (A batch, as 2025-03-26 alone allowed: its answers together, once all are had, its calls kept under their
            // ids before any is asked, so that a cancelling heard meanwhile stops one not begun yet too.)
            if state.lock().unwrap().agreed.as_deref() != Some("2025-03-26") {
                send(&out, &error_message(Value::Null, -32600, "Batches are not supported in this protocol version.", None));
                continue;
            }
            if batch.is_empty() {
                send(&out, &error_message(Value::Null, -32600, "An empty batch.", None));
                continue;
            }
            for message in &batch {
                kept_as_call(message, &state);
            }
            let (out, state) = (out.clone(), state.clone());
            std::thread::spawn(move || {
                let answers: Vec<Value> = batch.iter().filter_map(|message| answered(message, &state)).collect();
                if !answers.is_empty() {
                    send(&out, &Value::Array(answers));
                }
            });
            continue;
        }
        // A call asked of Glance answered on a thread of its own: what comes meanwhile (its cancelling) is heard, the
        // call kept under its id first, before anything after it is read.
        if kept_as_call(&message, &state) {
            let (out, state) = (out.clone(), state.clone());
            std::thread::spawn(move || {
                if let Some(answer) = answered(&message, &state) {
                    send(&out, &answer);
                }
            });
            continue;
        }
        if let Some(answer) = answered(&message, &state) {
            send(&out, &answer);
        }
    }
    // Standard input ended: every call under way stopped (the pipe closed stops it in Glance), and gone.
    for (stop, _) in state.lock().unwrap().calls.values() {
        stop.set();
    }
}

fn send(out: &Mutex<std::io::Stdout>, message: &Value) {
    let mut line = serde_json::to_vec(message).unwrap_or_default();
    line.push(b'\n');
    let mut out = out.lock().unwrap();
    let _ = out.write_all(&line);
    let _ = out.flush();
}

fn error_message(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({ "code": code, "message": message });
    if let Some(data) = data {
        error["data"] = data;
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": error })
}

/// Whether `message` is a call of a tool (a request): kept under its id, as one under way, if it is.
fn kept_as_call(message: &Value, state: &Mutex<State>) -> bool {
    let id = message.get("id").filter(|id| id.is_string() || id.is_i64() || id.is_u64());
    match id {
        Some(id) if message.get("method").and_then(Value::as_str) == Some("tools/call") => {
            state.lock().unwrap().calls.insert(id.to_string(), (Arc::new(Event::new()), false));
            true
        }
        _ => false,
    }
}

/// The answer to `message` (none to a notification, or a call cancelled): a call asked of Glance waited for here.
fn answered(message: &Value, state: &Mutex<State>) -> Option<Value> {
    let Some(object) = message.as_object() else {
        return Some(error_message(Value::Null, -32600, "A message must be an object.", None));
    };
    let id = object.get("id").cloned();
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        // (A response from the client: this server asks it nothing.)
        if object.contains_key("result") || object.contains_key("error") {
            return None;
        }
        return Some(error_message(id.unwrap_or(Value::Null), -32600, "A request must name its method.", None));
    };
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return id.map(|id| error_message(id, -32600, "Not a JSON-RPC 2.0 request.", None));
    }
    if matches!(&id, Some(Value::Null) | Some(Value::Bool(_)) | Some(Value::Array(_)) | Some(Value::Object(_))) || id.as_ref().is_some_and(|id| id.is_f64()) {
        return Some(error_message(Value::Null, -32600, "A request's id must be a string or a whole number.", None));
    }
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    let answer = match &id {
        None => {
            notified(method, &params, state);
            Answer::None
        }
        Some(id) => {
            let answer = asked(id, method, &params, state);
            // (A call refused before it ran is no longer kept as one under way.)
            if method == "tools/call" {
                state.lock().unwrap().calls.remove(&id.to_string());
            }
            answer
        }
    };
    let id = id?;
    match answer {
        Answer::Result(result) => Some(json!({ "jsonrpc": "2.0", "id": id, "result": result })),
        Answer::Error(code, message, data) => Some(error_message(id, code, &message, data)),
        Answer::None => None,
    }
}

/// What a notification tells, acted on: a call cancelled is stopped (and not answered).
fn notified(method: &str, params: &Value, state: &Mutex<State>) {
    if method == "notifications/cancelled" {
        if let Some(id) = params.get("requestId") {
            if let Some((stop, cancelled)) = state.lock().unwrap().calls.get_mut(&id.to_string()) {
                *cancelled = true;
                stop.set();
            }
        }
    }
}

/// The answer to request `id`, `method` with `params`.
fn asked(id: &Value, method: &str, params: &Value, state: &Mutex<State>) -> Answer {
    // As of which version: the one it says (the current way, with what else that asks of every request), else the
    // one agreed on shaking hands.
    let meta = params.get("_meta");
    let said = match meta.and_then(|meta| meta.get(VERSION_KEY)) {
        None => None,
        Some(Value::String(version)) => Some(version.as_str()),
        Some(_) => return Answer::Error(-32602, format!("{VERSION_KEY} must be a string."), None),
    };
    let era = match said {
        Some(version) if CURRENT.contains(&version) => {
            if !meta.and_then(|meta| meta.get(CAPABILITIES_KEY)).is_some_and(Value::is_object) {
                return Answer::Error(-32602, format!("{CAPABILITIES_KEY} must be given, as an object."), None);
            }
            Era { version: version.to_string(), current: true }
        }
        Some(version) if BEFORE.contains(&version) => Era { version: version.to_string(), current: false },
        Some(version) => {
            let supported: Vec<&str> = CURRENT.iter().chain(BEFORE.iter()).copied().collect();
            return Answer::Error(-32022, "Unsupported protocol version".into(), Some(json!({ "supported": supported, "requested": version })));
        }
        None => Era { version: state.lock().unwrap().agreed.clone().unwrap_or_else(|| BEFORE[0].to_string()), current: false },
    };
    match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str);
            let agreed = asked.filter(|asked| BEFORE.contains(asked)).unwrap_or(BEFORE[0]).to_string();
            state.lock().unwrap().agreed = Some(agreed.clone());
            let mut info = json!({ "name": "glance", "version": super::VERSION });
            if agreed != "2025-03-26" {
                info["title"] = json!("Glance");
            }
            Answer::Result(json!({ "protocolVersion": agreed, "capabilities": { "tools": {} }, "serverInfo": info, "instructions": INSTRUCTIONS }))
        }
        "server/discover" => {
            let supported: Vec<&str> = CURRENT.iter().chain(BEFORE.iter()).copied().collect();
            result(
                &Era { version: CURRENT[0].into(), current: true },
                json!({
                    "supportedVersions": supported,
                    "capabilities": { "tools": {} },
                    "instructions": INSTRUCTIONS,
                    "ttlMs": LIST_KEPT_MS,
                    "cacheScope": "public",
                    "_meta": { SERVER_KEY: { "name": "glance", "title": "Glance", "version": super::VERSION } },
                }),
            )
        }
        "ping" => result(&era, json!({})),
        "tools/list" => {
            if params.get("cursor").is_some_and(|cursor| !cursor.is_null()) {
                return Answer::Error(-32602, "Invalid cursor".into(), None);
            }
            let tools: Vec<Value> = super::tools::all().iter().map(|tool| listed(tool, &era)).collect();
            let mut list = json!({ "tools": tools });
            if era.current {
                list["ttlMs"] = json!(LIST_KEPT_MS);
                list["cacheScope"] = json!("public");
            }
            result(&era, list)
        }
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Answer::Error(-32602, "tools/call must name the tool.".into(), None);
            };
            if super::tools::named(name).is_none() {
                return Answer::Error(-32602, format!("Unknown tool: {name}"), None);
            }
            let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
            if !(arguments.is_object() || arguments.is_null()) {
                return Answer::Error(-32602, "A tool's arguments must be an object.".into(), None);
            }
            // (Kept under its id while it runs, as it was read: a cancelling heard meanwhile stops it, one heard before
            // it began keeps it from beginning, and a call cancelled is not answered.)
            let key = id.to_string();
            let stop = state.lock().unwrap().calls.entry(key.clone()).or_insert_with(|| (Arc::new(Event::new()), false)).0.clone();
            let answer = (!stop.is_set()).then(|| super::client::call(name, &arguments, &stop));
            let (_, cancelled) = state.lock().unwrap().calls.remove(&key).unwrap_or((stop, false));
            let Some(answer) = answer.filter(|_| !cancelled) else {
                return Answer::None;
            };
            result(&era, called(answer, &era))
        }
        _ => Answer::Error(-32601, format!("Method not found: {method}"), None),
    }
}

/// `tool` as `tools/list` lists it to a client of `era`.
fn listed(tool: &super::tools::Tool, era: &Era) -> Value {
    let mut listed = json!({
        "name": tool.name,
        "description": tool.description,
        "inputSchema": tool.input,
        "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false },
    });
    if era.has_structure() {
        listed["title"] = json!(tool.title);
        listed["annotations"]["title"] = json!(tool.title);
        listed["outputSchema"] = tool.output.clone();
    }
    listed
}

/// A call's answer, or its failure, as `tools/call` tells it to a client of `era`: the result as structured content
/// and, written out, as text (for a client that reads text alone); a failure as a sentence, `isError` set.
fn called(answer: Result<Value, Failure>, era: &Era) -> Value {
    let (structured, text, failed) = match answer {
        Ok(result) => {
            let text = serde_json::to_string(&result).unwrap_or_default();
            (result, text, false)
        }
        Err(failure) => {
            let text = format!("{} ({})", failure.message, failure.code);
            (json!({ "error": failure.told() }), text, true)
        }
    };
    let mut told = Map::new();
    told.insert("content".into(), json!([{ "type": "text", "text": text }]));
    if era.has_structure() {
        told.insert("structuredContent".into(), structured);
    }
    told.insert("isError".into(), json!(failed));
    Value::Object(told)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> Mutex<State> {
        Mutex::new(State { agreed: None, calls: HashMap::new() })
    }

    fn ask(state: &Mutex<State>, message: Value) -> Option<Value> {
        answered(&message, state)
    }

    /// A client that shakes hands is answered the version it asked for where it is one spoken, else the latest of
    /// those, and its later requests as of that version; the handshake's notification is not answered.
    #[test]
    fn a_handshake_agrees_on_a_version() {
        let state = state();
        let answer = ask(&state, json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": { "name": "x", "version": "1" } } })).unwrap();
        assert_eq!(answer["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(answer["result"]["capabilities"], json!({ "tools": {} }));
        assert!(answer["result"]["serverInfo"].get("title").is_none(), "2025-03-26 had no titles");
        assert_eq!(ask(&state, json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })), None);
        let tools = ask(&state, json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })).unwrap();
        let first = &tools["result"]["tools"][0];
        assert!(first.get("outputSchema").is_none() && first.get("title").is_none(), "as 2025-03-26 lists tools");
        assert!(tools["result"].get("resultType").is_none());
        let answer = ask(&state, json!({ "jsonrpc": "2.0", "id": 3, "method": "initialize", "params": { "protocolVersion": "2099-01-01" } })).unwrap();
        assert_eq!(answer["result"]["protocolVersion"], BEFORE[0]);
        let tools = ask(&state, json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/list" })).unwrap();
        assert!(tools["result"]["tools"][0]["outputSchema"].is_object());
        assert_eq!(tools["result"]["tools"][0]["annotations"]["readOnlyHint"], true);
    }

    /// A request of the current version says its version itself: answered with what that version asks of every
    /// result, the tool list with how long it may be kept. One of a version not spoken is refused, saying those that are.
    #[test]
    fn a_request_of_the_current_version_needs_no_handshake() {
        let state = state();
        let meta = json!({ "io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientCapabilities": {} });
        let discovered = ask(&state, json!({ "jsonrpc": "2.0", "id": "a", "method": "server/discover", "params": { "_meta": meta } })).unwrap();
        assert_eq!(discovered["result"]["resultType"], "complete");
        assert_eq!(discovered["result"]["supportedVersions"][0], "2026-07-28");
        assert_eq!(discovered["result"]["_meta"][SERVER_KEY]["name"], "glance");
        let tools = ask(&state, json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": { "_meta": meta } })).unwrap();
        assert_eq!((tools["result"]["resultType"].as_str(), tools["result"]["ttlMs"].as_u64(), tools["result"]["cacheScope"].as_str()), (Some("complete"), Some(LIST_KEPT_MS), Some("public")));
        let refused = ask(&state, json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list", "params": { "_meta": { "io.modelcontextprotocol/protocolVersion": "1900-01-01" } } })).unwrap();
        assert_eq!(refused["error"]["code"], -32022);
        assert_eq!(refused["error"]["data"]["requested"], "1900-01-01");
        assert_eq!(refused["error"]["data"]["supported"][0], "2026-07-28");
    }

    /// A request of the current version says, beside its version as a string, what its client can do: refused
    /// without, as with a version that is not a string.
    #[test]
    fn a_request_of_the_current_version_says_what_it_asks_of_every_one() {
        let state = state();
        let missing = ask(&state, json!({ "jsonrpc": "2.0", "id": 1, "method": "ping", "params": { "_meta": { "io.modelcontextprotocol/protocolVersion": "2026-07-28" } } })).unwrap();
        assert_eq!(missing["error"]["code"], -32602);
        let number = ask(&state, json!({ "jsonrpc": "2.0", "id": 2, "method": "ping", "params": { "_meta": { "io.modelcontextprotocol/protocolVersion": 20260728, "io.modelcontextprotocol/clientCapabilities": {} } } })).unwrap();
        assert_eq!(number["error"]["code"], -32602);
        let fine = ask(&state, json!({ "jsonrpc": "2.0", "id": 3, "method": "ping", "params": { "_meta": { "io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientCapabilities": {} } } })).unwrap();
        assert_eq!(fine["result"]["resultType"], "complete");
    }

    /// A call cancelled before it began is not asked of Glance, nor answered, and is no longer kept as one under way;
    /// one refused before it ran is not kept either.
    #[test]
    fn a_call_cancelled_before_it_begins_is_not_answered() {
        let state = state();
        let call = json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": { "name": "glance_status", "arguments": {} } });
        assert!(kept_as_call(&call, &state));
        assert_eq!(ask(&state, json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": { "requestId": 9 } })), None);
        assert_eq!(ask(&state, call), None);
        assert!(state.lock().unwrap().calls.is_empty());
        let unknown = json!({ "jsonrpc": "2.0", "id": "u", "method": "tools/call", "params": { "name": "delete_files" } });
        assert!(kept_as_call(&unknown, &state));
        assert_eq!(ask(&state, unknown).unwrap()["error"]["code"], -32602);
        assert!(state.lock().unwrap().calls.is_empty());
        assert!(!kept_as_call(&json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }), &state));
    }

    /// What is not a request is refused as JSON-RPC asks, a method not had told apart from a tool not had.
    #[test]
    fn what_is_not_a_request_is_refused() {
        let state = state();
        assert_eq!(ask(&state, json!({ "jsonrpc": "2.0", "id": 1, "method": "resources/list" })).unwrap()["error"]["code"], -32601);
        assert_eq!(ask(&state, json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "delete_files" } })).unwrap()["error"]["code"], -32602);
        assert_eq!(ask(&state, json!({ "jsonrpc": "2.0", "id": 1 })).unwrap()["error"]["code"], -32600);
        assert_eq!(ask(&state, json!({ "jsonrpc": "2.0", "id": null, "method": "ping" })).unwrap()["error"]["code"], -32600);
        assert_eq!(ask(&state, json!(5)).unwrap()["error"]["code"], -32600);
        assert_eq!(ask(&state, json!({ "jsonrpc": "2.0", "method": "notifications/whatever" })), None);
        assert_eq!(ask(&state, json!({ "jsonrpc": "2.0", "id": 7, "method": "ping" })).unwrap()["result"], json!({}));
    }

    /// A tool's failure is its result, told as an error, as a sentence and as structured content alike.
    #[test]
    fn a_failure_is_a_result_told_as_an_error() {
        let era = Era { version: "2025-06-18".into(), current: false };
        let told = called(Err(Failure::new("glance_not_running", "Glance is not running.")), &era);
        assert_eq!(told["isError"], true);
        assert_eq!(told["content"][0]["text"], "Glance is not running. (glance_not_running)");
        assert_eq!(told["structuredContent"]["error"]["code"], "glance_not_running");
        let told = called(Ok(json!({ "a": 1 })), &era);
        assert_eq!((told["isError"].as_bool(), told["content"][0]["text"].as_str()), (Some(false), Some(r#"{"a":1}"#)));
    }
}
