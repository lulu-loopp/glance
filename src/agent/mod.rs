//! What AI tools on this PC may read of what Glance knows, while the user lets them (`Settings::agent_interface`): the
//! readings and their history, the hardware, the drives, folders' sizes and what is in them, the largest and oldest
//! files, what a drive is made of, files found by name, and what AI tools spent. Read only: no call changes anything
//! of the user's.
//!
//! Glance serves it on a pipe of this user's (see `server`), as lines of JSON. `glance-cli.exe` speaks to it: as an MCP
//! server on its standard input and output for AI tools (see `mcp`), and as a command for a terminal (see `cli`). Its
//! calls, their arguments and what each answers are in `tools`, and told in `docs/agent.md`.

pub mod cli;
pub mod client;
mod files;
pub mod history;
pub mod mcp;
pub mod pipe;
mod readings;
mod server;
pub mod tools;

use serde_json::{json, Value};
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::Mutex;

/// The interface's version: what a client and Glance must both speak. Within one, calls and what they answer only
/// grow (new calls, new arguments, new fields).
pub const INTERFACE: u64 = 1;

/// Glance's version, as its package names it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The pipe's name, this user's (a test's its own, so that it never meets the Glance running).
pub fn pipe_name() -> Option<String> {
    let sid = crate::elevation::user_sid()?;
    // (A debug build is a Glance of its own, beside an installed one: its own pipe too.)
    let test = if cfg!(test) {
        format!(".test{}", std::process::id())
    } else if cfg!(debug_assertions) {
        ".debug".to_string()
    } else {
        String::new()
    };
    Some(format!(r"\\.\pipe\dev.weiyi.glance.agent.{sid}{test}"))
}

/// Why a call was not answered as asked: a code an agent may act on, and a sentence saying it, and what else tells
/// it (how far a drive's names are found, say).
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub code: &'static str,
    pub message: String,
    pub more: Option<Value>,
}

impl Failure {
    pub fn new(code: &'static str, message: impl Into<String>) -> Failure {
        Failure { code, message: message.into(), more: None }
    }

    pub fn invalid(message: impl Into<String>) -> Failure {
        Failure::new("invalid_argument", message)
    }

    /// As it is told: `{code, message, ...more}`.
    pub fn told(&self) -> Value {
        let mut told = json!({ "code": self.code, "message": self.message });
        if let (Some(Value::Object(more)), Value::Object(told)) = (&self.more, &mut told) {
            told.extend(more.clone());
        }
        told
    }

    /// As it was told (see `told`); none where it is not one.
    pub fn read(told: &Value) -> Option<Failure> {
        let code = told.get("code")?.as_str()?;
        let message = told.get("message")?.as_str()?.to_string();
        let mut more = told.as_object()?.clone();
        more.remove("code");
        more.remove("message");
        // (A code Glance gives is one of its own: one it does not know is told as it came, under a code of its own.)
        let code = tools::CODES.iter().copied().find(|known| *known == code).unwrap_or("unknown_error");
        Some(Failure { code, message, more: (!more.is_empty()).then_some(Value::Object(more)) })
    }
}

/// Whether the interface is off, on, or could not be turned on (another program had its pipe's name).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Off,
    On,
    Failed,
}

static STATE: AtomicU8 = AtomicU8::new(0);

/// How the interface stands.
pub fn state() -> State {
    match STATE.load(Ordering::Relaxed) {
        1 => State::On,
        2 => State::Failed,
        _ => State::Off,
    }
}

fn set_state(state: State) {
    STATE.store(
        match state {
            State::Off => 0,
            State::On => 1,
            State::Failed => 2,
        },
        Ordering::Relaxed,
    );
}

/// What the interface is to be, as the settings were last told (see `follow`), and the thread that makes it so.
static WANTED: Mutex<Option<std::sync::mpsc::Sender<bool>>> = Mutex::new(None);

/// The interface on or off as `settings` say, from now: turned on or off on a thread of its own (its pipe made, or
/// its clients let go of and their calls stopped), and the longer history kept only while it is on.
pub fn follow(settings: &crate::settings::Settings) {
    let on = settings.agent_interface;
    let mut wanted = WANTED.lock().unwrap();
    let sender = wanted.get_or_insert_with(|| {
        let (sender, asked) = std::sync::mpsc::channel::<bool>();
        let _ = std::thread::Builder::new().name("glance: agent interface".into()).spawn(move || {
            let mut running: Option<server::Server> = None;
            while let Ok(mut on) = asked.recv() {
                // (Only the last asked meanwhile.)
                for later in asked.try_iter() {
                    on = later;
                }
                match (on, running.is_some()) {
                    (true, false) => {
                        history::keep(true);
                        running = server::Server::start();
                        set_state(if running.is_some() { State::On } else { State::Failed });
                    }
                    (false, true) => {
                        if let Some(server) = running.take() {
                            server.stop();
                        }
                        history::keep(false);
                        set_state(State::Off);
                    }
                    (false, false) => {
                        history::keep(false);
                        set_state(State::Off);
                    }
                    (true, true) => {}
                }
            }
        });
        sender
    });
    let _ = sender.send(on);
}

/// The integrity level (a token's label's RID) a process must have at least to be served by the pipe or to serve it:
/// medium, an ordinary program's (a browser's sandbox, at low, is neither).
pub(crate) const MEDIUM: u32 = 0x2000;

/// Whether `token` (opened to be queried) is of this Windows user's (`sid`), at medium integrity or above.
pub(crate) fn token_is_users(token: windows::Win32::Foundation::HANDLE, sid: &str) -> bool {
    crate::link::token_facts(token).is_some_and(|(user, integrity)| user == sid && integrity >= MEDIUM)
}

/// When an agent last asked what AI tools spent (ms since 1970): it is read for a while after, as if it were shown.
static AI_ASKED: AtomicU64 = AtomicU64::new(0);

/// How long after an agent asked what AI tools spent it is kept current.
const AI_WANTED_MS: u64 = 15 * 60 * 1000;

/// Whether an agent asked lately what AI tools spent (see `AI_ASKED`).
pub fn wants_ai() -> bool {
    let asked = AI_ASKED.load(Ordering::Relaxed);
    asked > 0 && now_ms().saturating_sub(asked) < AI_WANTED_MS
}

/// Now, in milliseconds since 1970.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_millis() as u64)
}

/// What an AI tool's MCP settings take to start Glance's server: `glance-cli.exe mcp`, by its full path beside this
/// Glance (no folder of Glance's is put on PATH), as most tools' settings files write it.
pub fn mcp_config() -> String {
    let cli = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|folder| folder.join("glance-cli.exe"))).unwrap_or_else(|| "glance-cli.exe".into());
    let config = json!({ "mcpServers": { "glance": { "command": cli, "args": ["mcp"] } } });
    serde_json::to_string_pretty(&config).unwrap_or_default()
}
