//! What AI coding tools have spent, as their own records on this PC tell it: Claude Code's transcripts, Codex's
//! session logs, Kimi Code's, Qwen Code's and MiniMax Code's sessions' records and ZCode's database, each read as it grows (only what was added since it was last read), every response counted once
//! however many of the files hold it (a resumed or forked session copies what came before into a file of its own:
//! a third of Claude Code's responses on the owner's PC were in more than one file), in quarter hours of their own
//! time. What was read is kept in Glance's local folder, with how far each file was read, so as not to read a
//! transcript of gigabytes again as Glance starts. Codex's limits (its 5-hour and weekly windows) come with its
//! records: as they were when Codex last ran. Claude's come as Claude Code tells them to its status line, where the
//! user put Glance there (see `statusline`); the AI providers' accounts (their plans' windows, their balances) are
//! read by the user's keys (see `providers`).

mod claude;
mod codex;
mod kimi;
mod minimax;
mod qwen;
#[cfg(windows)]
mod sqlite;
#[cfg(windows)]
mod zcode;
#[cfg(windows)]
pub mod keys;
#[cfg(windows)]
pub mod providers;
#[cfg(windows)]
pub mod statusline;
mod time;
#[cfg(windows)]
mod watch;

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, Read as _, Seek};
use std::path::{Path, PathBuf};

pub use time::today_start;
#[cfg(windows)]
pub use watch::{keys_changed, look_now, stamp, start, usage};

/// How long what was spent is kept: a month, and a day more for a month counted from any day of it.
pub const KEPT_DAYS: u64 = 32;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;
/// What was spent is told by the quarter hour (any time zone's day, even one a quarter or half an hour off, is whole
/// quarters).
pub const QUARTER_MS: u64 = 15 * 60 * 1000;
/// The file Glance keeps what it read in, in its local folder.
const FILE: &str = "ai-usage.bin";
/// How the records are read, told with what was kept: one more each time more is read of a record than before, so what
/// was kept from a reading that read less is let go of and read afresh (each file is read on from where it was left
/// off, which would never see what the older reading did not read). 1: Claude Code's own titles and its users' prompts.
const READ_AS: u32 = 1;

/// Whose records, or whose account's limits (an AI provider's, read by the user's key: see `providers`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub enum Tool {
    /// Claude Code.
    Claude,
    /// Codex (ChatGPT's).
    Codex,
    /// DeepSeek's open platform: what is left of its balance.
    DeepSeek,
    /// Moonshot AI's open platform (Kimi's API): its balance.
    Moonshot,
    /// OpenRouter: its credits, and the key's own limit.
    OpenRouter,
    /// Kimi Code: its plan, and what it spent on this PC.
    Kimi,
    /// Zhipu's GLM Coding Plan (bigmodel.cn, or Z.ai's abroad), and what ZCode spent on this PC.
    Glm,
    /// MiniMax's Token Plan, or its open platform's balance, and what MiniMax Code spent on this PC.
    MiniMax,
    /// Volcengine Ark's Coding Plan.
    Volcengine,
    /// Qwen Code.
    Qwen,
}

/// Tokens spent, by kind. `input` is what was read afresh (neither from nor into a cache), `output` what was written
/// (of which `reasoning`, where a tool tells it apart).
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub reasoning: u64,
}

impl Tokens {
    /// Every token spent (reasoning is among the output).
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }

    pub fn add(&mut self, other: Tokens) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.reasoning += other.reasoning;
    }
}

/// One session (a conversation in a tool): what it is called, where, by which model last, when it last spent, and
/// what it spent in each quarter hour.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Session {
    pub tool: Tool,
    pub id: String,
    /// The name it was given, or else what the tool summed it up as, or else the last thing asked in it.
    pub title: Option<String>,
    /// The folder it was worked in.
    pub folder: Option<String>,
    pub model: Option<String>,
    /// When it last spent (ms since 1970).
    pub last: u64,
    /// Tokens spent, by quarter hour (since 1970).
    pub spent: BTreeMap<u32, u64>,
    /// When a file that told of it was last written to (ms since 1970): what was told of it (its title) is kept as
    /// long as such a file is, whether it spent or not.
    #[serde(default)]
    seen: u64,
}

impl Session {
    /// Tokens spent from `from` up to `to` (ms since 1970).
    pub fn spent_between(&self, from: u64, to: u64) -> u64 {
        self.spent.range(quarter(from)..quarter(to.saturating_add(QUARTER_MS - 1))).map(|(_, tokens)| tokens).sum()
    }
}

/// One of a tool's limits as it last told it: its windows, each used so far (0 to 100) and when it starts again.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Limit {
    pub tool: Tool,
    /// Which limit (Codex: "codex", or another for some models).
    pub id: String,
    pub name: Option<String>,
    /// The plan (Codex: "plus", "pro" …).
    pub plan: Option<String>,
    pub windows: Vec<Window>,
    /// What is left of the money paid in, where its account has a balance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance: Option<Balance>,
    /// When it was told (ms since 1970).
    pub told: u64,
}

/// How reading a provider's account failed: its key refused, asked too often, or not reached (or its answer not one
/// that tells it).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Failed {
    Key,
    Limited,
    Unavailable,
}

/// An account's money: what is left of it, and what was spent today, in its currency ("CNY", "USD").
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Balance {
    pub currency: String,
    pub amount: Option<f64>,
    /// What was spent today, as the provider tells it; else (none told) as `Ledger` counts it from what was left of
    /// the balance, or `spent` mounted, from one reading to the next.
    pub today: Option<f64>,
    /// Since when today's is counted, where that was after today began (the first reading of the day was its start).
    #[serde(default)]
    pub since: Option<u64>,
    /// All the account ever spent, where the provider tells it (today's is counted from it as it mounts).
    #[serde(default)]
    pub spent: Option<f64>,
}

/// What today's spending of a provider's account is counted from: what its balance was last read as (or all it spent),
/// in which currency, which day it is of, what was spent that day so far, since when it was counted, and by which key
/// it was read (a hash of it: another key is another account, whose balance is not counted from this one's).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
struct Paid {
    tool: Tool,
    currency: String,
    /// All it spent (mounting), else what was left (falling).
    mounting: bool,
    last: f64,
    day: u64,
    today: f64,
    since: u64,
    #[serde(default)]
    key: u64,
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Window {
    /// How long it is (300: five hours; 10080: a week).
    pub minutes: u32,
    pub used: f32,
    /// When it starts again (ms since 1970).
    pub resets: Option<u64>,
}

/// How far one file was read: its length and when it was written to as it was read, up to which byte its lines were
/// taken (the end of its last whole line), and what it told that later lines need (its session, and Codex's model and
/// last total).
#[derive(Clone, Default, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
struct Read {
    len: u64,
    modified: u64,
    at: u64,
    session: Option<String>,
    model: Option<String>,
    total: Option<u64>,
    /// A summary read before the file named its session.
    summary: Option<String>,
    /// How the file began as it was read (see `head_of`): one that begins otherwise was written anew.
    head: Option<u64>,
    /// How what was read of it ended (see `tail_of`): one that ends there otherwise was written anew, though it begins
    /// as it did and is longer (a file rewritten whole as its tool compacts it, then grown past where it was read to).
    tail: Option<u64>,
}

/// How many of a file's first bytes, and of the last of it read, tell it apart from one written anew in its place.
const HEAD: u64 = 1024;

/// A hash of the first bytes of `file` (as many as `HEAD`, or `upto` where that is fewer: those of it read, which
/// lines added after them leave as they were).
fn head_of(file: &mut std::fs::File, upto: u64) -> Option<u64> {
    hash_of(file, 0, upto.min(HEAD))
}

/// A hash of the bytes of `file` just before `upto` (as many as `HEAD`): the last of it read.
fn tail_of(file: &mut std::fs::File, upto: u64) -> Option<u64> {
    hash_of(file, upto.saturating_sub(HEAD), upto.min(HEAD))
}

/// A hash of the `len` bytes of `file` from `start`.
fn hash_of(file: &mut std::fs::File, start: u64, len: u64) -> Option<u64> {
    file.seek(std::io::SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.take(len).read_to_end(&mut bytes).ok()?;
    Some(bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| (hash ^ *byte as u64).wrapping_mul(0x0000_0100_0000_01B3)))
}

/// The longest line of a record read (bytes): one longer is let pass unread, never held whole (what a line of the
/// tools' records tells of what was spent is far shorter, a line holding a picture a few megabytes).
const LINE_MOST: usize = 64 << 20;

/// The next whole line of `lines` into `line` (its ending kept), at most `most` of it held (of one longer, a byte more
/// than that, to be let pass), and how many bytes it took: none at the end, or where the line has not ended yet.
fn next_line(lines: &mut impl BufRead, line: &mut Vec<u8>, most: usize) -> Option<usize> {
    line.clear();
    let mut took = 0;
    loop {
        let buffer = lines.fill_buf().ok()?;
        if buffer.is_empty() {
            return None;
        }
        let (part, ended) = match buffer.iter().position(|byte| *byte == b'\n') {
            Some(at) => (&buffer[..=at], true),
            None => (buffer, false),
        };
        let n = part.len();
        if line.len() <= most {
            line.extend_from_slice(&part[..n.min(most + 1 - line.len())]);
        }
        lines.consume(n);
        took += n;
        if ended {
            return Some(took);
        }
    }
}

/// What a line of a record says, as the reader of its tool makes of it.
#[derive(PartialEq, Debug)]
enum Told {
    /// Tokens spent at `at` (ms) by `model` in `session`, counted once by `key` wherever it is read.
    Spent { key: u64, at: u64, session: String, model: String, tokens: Tokens },
    /// What session `session` is called.
    Title { session: String, title: String, kind: TitleKind },
    /// Session `session` is worked in `folder`.
    Folder { session: String, folder: String },
    /// A limit as it stands.
    Limit(Limit),
}

/// Where a session's title came from: one given by hand outranks the one its tool gave it, that a summary, and a summary
/// the last thing asked.
#[derive(Clone, Copy, PartialEq, PartialOrd, Debug)]
enum TitleKind {
    Asked,
    Summed,
    Titled,
    Named,
}

/// Everything read: what each file was read up to, what was spent by tool, model and quarter hour, the sessions, the
/// limits, and which responses were counted (by day, to let go of those of days no longer kept).
#[derive(Default)]
pub struct Ledger {
    files: HashMap<PathBuf, Read>,
    spent: HashMap<(Tool, String, u32), Tokens>,
    sessions: HashMap<(Tool, String), Session>,
    titled: HashMap<(Tool, String), TitleKind>,
    limits: Vec<Limit>,
    counted: BTreeMap<u32, HashSet<u64>>,
    /// What each provider's account was last read as, today's spending counted from it.
    paid: Vec<Paid>,
    /// How the last reading of each provider's account failed, where it did, and those whose keys are not set (not
    /// kept from one run to the next).
    failed: BTreeMap<Tool, Failed>,
    unkeyed: std::collections::BTreeSet<Tool>,
}

/// What was spent as it stood at `now`, as what shows it needs it: today's by tool and model, each of the last seven
/// days' and each quarter hour of today's by tool, the sessions that spent within a day, and the limits.
#[derive(Clone, PartialEq, Debug)]
pub struct Usage {
    pub now: u64,
    /// When today began (ms since 1970).
    pub today: u64,
    pub spent_today: Vec<(Tool, String, Tokens)>,
    /// Each tool's tokens spent on each of the last seven days, today last (as this PC's clock has its days).
    pub days: Vec<(Tool, [u64; 7])>,
    /// Each tool's tokens spent in each quarter hour of today, up to now.
    pub quarters: Vec<(Tool, Vec<u64>)>,
    /// The sessions that spent within the last day, the latest first.
    pub sessions: Vec<Session>,
    pub limits: Vec<Limit>,
    /// The providers whose accounts could not be read when last asked, and why, and those whose keys are not set.
    pub failed: Vec<(Tool, Failed)>,
    pub unkeyed: Vec<Tool>,
}

/// The sessions told of (at most).
const SESSIONS: usize = 20;

/// The quarter hour (since 1970) the moment `ms` falls in.
pub fn quarter(ms: u64) -> u32 {
    (ms / QUARTER_MS) as u32
}

/// Where each tool keeps its records on this PC: Claude Code's transcripts (in each of `CLAUDE_CONFIG_DIR`'s folders
/// where it is set, else in `.claude` and `.config\claude` in the user's folder), and Codex's sessions (in
/// `CODEX_HOME`, else `.codex`), its archived ones too, and the index of their names beside them, and those of Kimi
/// Code, Qwen Code, ZCode and MiniMax Code, each where it keeps them.
pub fn roots() -> Vec<(Tool, PathBuf)> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from);
    let mut roots = Vec::new();
    match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dirs) => roots.extend(std::env::split_paths(&dirs).map(|dir| (Tool::Claude, dir.join("projects")))),
        None => roots.extend(home.iter().flat_map(|home| [home.join(".claude"), home.join(".config").join("claude")]).map(|dir| (Tool::Claude, dir.join("projects")))),
    }
    let codex = std::env::var_os("CODEX_HOME").map(PathBuf::from).or_else(|| home.clone().map(|home| home.join(".codex")));
    roots.extend(codex.iter().flat_map(|codex| [codex.join("sessions"), codex.join("archived_sessions"), codex.join("session_index.jsonl")]).map(|dir| (Tool::Codex, dir)));
    // Kimi Code's, and those of the Kimi app's agent, which runs Kimi Code where the app keeps its files (its own
    // folder, or the one chosen for them).
    let kimi = std::env::var_os("KIMI_CODE_HOME").filter(|dir| !dir.to_string_lossy().trim().is_empty()).map(PathBuf::from).or_else(|| home.clone().map(|home| home.join(".kimi-code")));
    roots.extend(kimi.map(|kimi| (Tool::Kimi, kimi.join("sessions"))));
    // Qwen Code's, and ZCode's database (what its model spent: GLM's).
    roots.extend(home.iter().map(|home| (Tool::Qwen, home.join(".qwen").join("projects"))));
    roots.extend(home.iter().map(|home| (Tool::Glm, home.join(".zcode").join("cli").join("db"))));
    if let Some(app) = std::env::var_os("APPDATA").filter(|dir| !dir.is_empty()).map(PathBuf::from) {
        let runtime = Path::new("daimon").join("runtime").join("kimi-code").join("home").join("sessions");
        let shared = std::fs::read(app.join("kimi-desktop").join("daimon-storage.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|storage| storage.get("shareDir")?.as_str().map(str::trim).filter(|dir| !dir.is_empty()).map(PathBuf::from));
        roots.push((Tool::Kimi, shared.unwrap_or_else(|| app.join("kimi-desktop").join("daimon-share")).join(runtime)));
    }
    // MiniMax Code's sessions, and its runtime's database (what they are called).
    for data in minimax::data_dirs(home.as_deref()) {
        let data = data.join("v2");
        roots.extend([data.join("sessions"), data.join("sqlite")].map(|dir| (Tool::MiniMax, dir)));
    }
    // (A folder linked to another, `.mavis` to `.minimax` as it often is, read once: by the first name it has.)
    let mut seen = HashSet::new();
    roots.retain(|(_, root)| std::fs::canonicalize(root).map_or(true, |real| seen.insert(real)));
    roots
}

impl Ledger {
    /// What was kept in `dir` (none kept, or kept unreadably: nothing, to be read afresh).
    pub fn load(dir: &Path) -> Ledger {
        std::fs::read(dir.join(FILE)).ok().and_then(|bytes| Ledger::from_bytes(&bytes)).unwrap_or_default()
    }

    /// Kept in `dir`, whole or not at all.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        write_whole(dir, FILE, &self.to_bytes())
    }

    /// Every record under `roots` read as far as it has grown since it was last read, at `now` (ms since 1970): files
    /// not written to within the days kept are not read, and what was spent before them is let go of. Whether
    /// anything new was read.
    pub fn refresh(&mut self, roots: &[(Tool, PathBuf)], now: u64) -> bool {
        let from = now.saturating_sub(KEPT_DAYS * DAY_MS);
        let mut changed = self.forget_before(from);
        for (tool, root) in roots {
            let mut files = Vec::new();
            records_in(root, &mut files);
            for (path, len, modified) in files {
                if modified < from && !self.files.contains_key(&path) {
                    continue;
                }
                changed |= self.read_file(*tool, &path, len, modified, from);
            }
        }
        changed
    }

    /// The file at `path` (of `len` bytes, written to at `modified`) read from where it was left, or from its start
    /// where it is shorter than that (written anew): what was counted before is not counted again.
    fn read_file(&mut self, tool: Tool, path: &Path, len: u64, modified: u64, from: u64) -> bool {
        let mut read = self.files.get(path).cloned().unwrap_or_default();
        if read.len == len && read.modified == modified {
            return false;
        }
        // A database (ZCode's, or MiniMax Code's of its sessions' names) asked again whole as it is written to: what
        // was counted is not counted again.
        if is_database(path) {
            // (Not read, the database busy or failing: not taken as read, to be read again.)
            #[cfg(windows)]
            let Some(told) = (match tool {
                Tool::Glm => zcode::told_in(path, from, modified),
                Tool::MiniMax => minimax::told_in(path),
                _ => Some(Vec::new()),
            }) else {
                return false;
            };
            #[cfg(not(windows))]
            let told: Vec<Told> = Vec::new();
            self.files.insert(path.to_path_buf(), Read { len, modified, at: len, ..Read::default() });
            let mut changed = false;
            for told in told {
                changed |= self.take(tool, told, from, modified);
            }
            return changed;
        }
        let Ok(mut file) = std::fs::File::open(path) else { return false };
        // (Written anew: shorter than it was read, or beginning, or ending where it was read to, otherwise than it did.)
        if len < read.at || read.head.is_some_and(|head| head_of(&mut file, read.at) != Some(head)) || read.tail.is_some_and(|tail| tail_of(&mut file, read.at) != Some(tail)) {
            read = Read::default();
        }
        // (Kimi Code names a record's session by its folder alone, MiniMax Code by its folder's manifest.)
        if read.session.is_none() {
            read.session = match tool {
                Tool::Kimi => kimi::session_of(path),
                Tool::MiniMax => minimax::session_of(path),
                _ => None,
            };
        }
        if file.seek(std::io::SeekFrom::Start(read.at)).is_err() {
            return false;
        }
        let mut lines = std::io::BufReader::with_capacity(1 << 20, file.take(len - read.at));
        let mut line = Vec::new();
        let mut told = Vec::new();
        let mut changed = false;
        // Each whole line (one not ended yet left to be read once it is), what it told taken in at once.
        while let Some(n) = next_line(&mut lines, &mut line, LINE_MOST) {
            read.at += n as u64;
            if line.len() > LINE_MOST {
                continue;
            }
            match tool {
                Tool::Claude => claude::line(&line, &mut read, &mut told),
                Tool::Codex => codex::line(&line, &mut read, &mut told),
                Tool::Kimi => kimi::line(&line, &mut read, &mut told),
                Tool::Qwen => qwen::line(&line, &mut read, &mut told),
                Tool::MiniMax => minimax::line(&line, &mut read, &mut told),
                // (The providers keep no records here: see `providers`.)
                _ => {}
            }
            for told in told.drain(..) {
                changed |= self.take(tool, told, from, modified);
            }
        }
        let mut file = lines.into_inner().into_inner();
        read.head = head_of(&mut file, read.at);
        read.tail = tail_of(&mut file, read.at);
        read.len = len;
        read.modified = modified;
        self.files.insert(path.to_path_buf(), read);
        changed
    }

    /// What a line of a file written to at `modified` told taken in: what was spent before `from`, or counted
    /// already, is not, nor a limit told before it.
    fn take(&mut self, tool: Tool, told: Told, from: u64, modified: u64) -> bool {
        match told {
            Told::Spent { key, at, session, model, tokens } => {
                if at < from || self.counted.values().any(|keys| keys.contains(&key)) {
                    return false;
                }
                self.counted.entry(day(at)).or_default().insert(key);
                self.spent.entry((tool, model.clone(), quarter(at))).or_default().add(tokens);
                let session = self.session(tool, &session, modified);
                session.model = Some(model);
                session.last = session.last.max(at);
                *session.spent.entry(quarter(at)).or_default() += tokens.total();
                true
            }
            Told::Title { session, title, kind } => {
                let had = self.titled.get(&(tool, session.clone())).copied();
                if had.is_some_and(|had| had > kind) {
                    return false;
                }
                self.titled.insert((tool, session.clone()), kind);
                let session = self.session(tool, &session, modified);
                let changed = session.title.as_deref() != Some(&title);
                session.title = Some(title);
                changed
            }
            Told::Folder { session, folder } => {
                let session = self.session(tool, &session, modified);
                let changed = session.folder.as_deref() != Some(&folder);
                session.folder = Some(folder);
                changed
            }
            Told::Limit(limit) if limit.told < from => false,
            Told::Limit(limit) => self.told_limit(limit),
        }
    }

    /// How reading provider `tool`'s account by the key hashed as `key` at `now` went: what it told kept (today's
    /// spending counted, where it does not tell it, from what it told before by that key), or how it failed (what it
    /// told before kept, shown as it was). Whether what is shown changed.
    pub(crate) fn read_of(&mut self, tool: Tool, key: u64, read: Result<Limit, Failed>, now: u64) -> bool {
        let keyed = self.unkeyed.remove(&tool);
        keyed | match read {
            Ok(mut limit) => {
                if let Some(balance) = limit.balance.as_mut() {
                    self.count_spent(tool, key, balance, now);
                }
                let failed = self.failed.remove(&tool).is_some();
                self.told_limit(limit) | failed
            }
            Err(failed) => self.failed.insert(tool, failed) != Some(failed),
        }
    }

    /// Provider `tool`'s account no longer read (its key taken away): what it told let go of. Whether there was any.
    pub(crate) fn unkeyed(&mut self, tool: Tool) -> bool {
        let before = (self.limits.len(), self.paid.len());
        let told = self.unkeyed.insert(tool);
        self.limits.retain(|limit| limit.tool != tool);
        self.paid.retain(|paid| paid.tool != tool);
        told | self.failed.remove(&tool).is_some() | (before != (self.limits.len(), self.paid.len()))
    }

    /// Today's spending of `tool`'s account, where it does not tell it, counted into `balance` at `now`: from what it
    /// spent in all (as that mounts), else from what is left (as that falls: a rise is money paid in, from which it is
    /// counted on). The day's first reading counts from itself; one in another currency counts anew.
    fn count_spent(&mut self, tool: Tool, key: u64, balance: &mut Balance, now: u64) {
        if balance.today.is_some() {
            return;
        }
        let (value, mounting) = match (balance.spent, balance.amount) {
            (Some(spent), _) => (spent, true),
            (None, Some(amount)) => (amount, false),
            (None, None) => return,
        };
        let day = time::today_start(now);
        let at = self.paid.iter().position(|paid| paid.tool == tool);
        let paid = match at {
            Some(at) if self.paid[at].currency == balance.currency && self.paid[at].mounting == mounting && self.paid[at].key == key => &mut self.paid[at],
            _ => {
                self.paid.retain(|paid| paid.tool != tool);
                self.paid.push(Paid { tool, currency: balance.currency.clone(), mounting, last: value, day, today: 0.0, since: now, key });
                self.paid.last_mut().unwrap()
            }
        };
        if paid.day != day {
            // (What was spent since the last reading, a day or more before, is not known to be today's, nor how much of
            // it was: today's is counted from this reading, told as counted since it.)
            paid.since = now;
            paid.day = day;
            paid.today = 0.0;
            paid.last = value;
        }
        let spent = if mounting { value - paid.last } else { paid.last - value };
        paid.today += spent.max(0.0);
        paid.last = value;
        balance.today = Some((paid.today * 100.0).round() / 100.0);
        balance.since = (paid.since > day).then_some(paid.since);
    }

    /// `tool`'s limit `id` no longer kept. Whether there was one.
    pub(crate) fn forget_limit(&mut self, tool: Tool, id: &str) -> bool {
        let before = self.limits.len();
        self.limits.retain(|limit| limit.tool != tool || limit.id != id);
        self.limits.len() != before
    }

    /// A limit as it was told, kept where it is newer than what was told of it before. Whether it was.
    pub(crate) fn told_limit(&mut self, limit: Limit) -> bool {
        match self.limits.iter_mut().find(|had| had.tool == limit.tool && had.id == limit.id) {
            Some(had) if had.told >= limit.told => false,
            Some(had) => {
                *had = limit;
                true
            }
            None => {
                self.limits.push(limit);
                true
            }
        }
    }

    /// Session `id` of `tool`, told of in a file written to at `modified`.
    fn session(&mut self, tool: Tool, id: &str, modified: u64) -> &mut Session {
        let session = self.sessions.entry((tool, id.to_string())).or_insert_with(|| Session { tool, id: id.to_string(), title: None, folder: None, model: None, last: 0, spent: BTreeMap::new(), seen: 0 });
        session.seen = session.seen.max(modified);
        session
    }

    /// What was spent before `from` let go of, with the files last written before it, the sessions only they told
    /// of, and the limits told before it. Whether anything was.
    fn forget_before(&mut self, from: u64) -> bool {
        let (first_quarter, first_day) = (quarter(from), day(from));
        let before = (self.spent.len(), self.sessions.len(), self.files.len(), self.counted.len());
        let before_limits = self.limits.len();
        self.spent.retain(|(_, _, at), _| *at >= first_quarter);
        self.sessions.retain(|_, session| session.seen >= from || session.last >= from);
        for session in self.sessions.values_mut() {
            session.spent.retain(|at, _| *at >= first_quarter);
        }
        let sessions = &self.sessions;
        self.titled.retain(|key, _| sessions.contains_key(key));
        self.files.retain(|_, read| read.modified >= from);
        self.counted.retain(|at, _| *at >= first_day);
        self.limits.retain(|limit| limit.told >= from);
        before != (self.spent.len(), self.sessions.len(), self.files.len(), self.counted.len()) || before_limits != self.limits.len()
    }

    /// What was spent from `from` up to `to` (ms since 1970), by tool and model, most first.
    pub fn spent_between(&self, from: u64, to: u64) -> Vec<(Tool, String, Tokens)> {
        let (first, end) = (quarter(from), quarter(to.saturating_add(QUARTER_MS - 1)));
        let mut by: BTreeMap<(Tool, &str), Tokens> = BTreeMap::new();
        for ((tool, model, at), tokens) in &self.spent {
            if (first..end).contains(at) {
                by.entry((*tool, model)).or_default().add(*tokens);
            }
        }
        let mut spent: Vec<_> = by.into_iter().map(|((tool, model), tokens)| (tool, model.to_string(), tokens)).collect();
        spent.sort_by(|a, b| b.2.total().cmp(&a.2.total()).then_with(|| a.1.cmp(&b.1)));
        spent
    }

    /// Tokens spent by `tool` (any, where none) in each quarter hour from `from` up to `to`.
    pub fn by_quarter(&self, tool: Option<Tool>, from: u64, to: u64) -> Vec<u64> {
        let first = quarter(from);
        let mut quarters = vec![0; (quarter(to.saturating_add(QUARTER_MS - 1)) - first) as usize];
        for ((which, _, at), tokens) in &self.spent {
            if tool.is_none_or(|tool| tool == *which) && *at >= first && ((*at - first) as usize) < quarters.len() {
                quarters[(*at - first) as usize] += tokens.total();
            }
        }
        quarters
    }

    /// The sessions that spent since `since` (ms since 1970), the latest first.
    pub fn sessions_since(&self, since: u64) -> Vec<&Session> {
        let mut sessions: Vec<_> = self.sessions.values().filter(|session| !session.spent.is_empty() && session.last >= since).collect();
        sessions.sort_by(|a, b| b.last.cmp(&a.last).then_with(|| a.id.cmp(&b.id)));
        sessions
    }

    /// Each limit as it was last told (what is shown has them with the rest: see `usage`).
    #[cfg(test)]
    pub fn limits(&self) -> &[Limit] {
        &self.limits
    }

    /// What was spent as it stands at `now` (see `Usage`).
    pub fn usage(&self, now: u64) -> Usage {
        let today = today_start(now);
        // (Each day's start as this PC's clock has it: a day of summer time beginning or ending is not 24 hours.)
        let mut starts = [today; 7];
        for at in (0..6).rev() {
            starts[at] = today_start(starts[at + 1] - 1);
        }
        let mut tools: Vec<Tool> = self.spent.keys().map(|(tool, _, _)| *tool).collect();
        tools.sort();
        tools.dedup();
        let days = tools
            .iter()
            .map(|tool| {
                let mut spent = [0; 7];
                for (at, start) in starts.iter().enumerate() {
                    let end = starts.get(at + 1).copied().unwrap_or(now);
                    spent[at] = self.by_quarter(Some(*tool), *start, end).iter().sum();
                }
                (*tool, spent)
            })
            .collect();
        let quarters = tools.iter().map(|tool| (*tool, self.by_quarter(Some(*tool), today, now))).collect();
        let sessions = self.sessions_since(now.saturating_sub(DAY_MS)).into_iter().take(SESSIONS).cloned().collect();
        let failed = self.failed.iter().map(|(tool, failed)| (*tool, *failed)).collect();
        let unkeyed = self.unkeyed.iter().copied().collect();
        Usage { now, today, spent_today: self.spent_between(today, now), days, quarters, sessions, limits: self.limits.clone(), failed, unkeyed }
    }

    /// The records at `paths` of `tool` (each a file, or a folder of them) read as far as they have grown, at `now`
    /// (one gone, or not written to within the days kept, is not). Whether anything new was read.
    pub fn read_paths(&mut self, tool: Tool, paths: &[PathBuf], now: u64) -> bool {
        let from = now.saturating_sub(KEPT_DAYS * DAY_MS);
        let mut changed = false;
        for path in paths {
            let mut found = Vec::new();
            records_in(path, &mut found);
            for (path, len, modified) in found {
                if modified >= from {
                    changed |= self.read_file(tool, &path, len, modified, from);
                }
            }
        }
        changed
    }

    /// The files under `roots` written to since `since` as last read, and the roots that are files (not watched: see
    /// `watch`), read again as far as they have grown, at `now`: Windows tells of a file kept open as it is written to
    /// only now and then, so what is at work is looked at itself.
    pub fn read_recent(&mut self, roots: &[(Tool, PathBuf)], since: u64, now: u64) -> bool {
        let recent: Vec<PathBuf> = self.files.iter().filter(|(_, read)| read.modified >= since).map(|(path, _)| path.clone()).collect();
        let mut changed = false;
        for (tool, root) in roots {
            let mut under: Vec<PathBuf> = recent.iter().filter(|path| path.starts_with(root) && *path != root).cloned().collect();
            if root.is_file() {
                under.push(root.clone());
            }
            changed |= self.read_paths(*tool, &under, now);
        }
        changed
    }

    /// Kept as bytes: what was read, as JSON on a line of its own, then which responses were counted (each day, its
    /// number and its count of keys, then the keys, each little-endian).
    fn to_bytes(&self) -> Vec<u8> {
        let kept = Kept {
            read_as: READ_AS,
            files: self.files.iter().map(|(path, read)| (path.to_string_lossy().into_owned(), read.clone())).collect(),
            spent: self.spent.iter().map(|((tool, model, at), tokens)| (*tool, model.clone(), *at, *tokens)).collect(),
            sessions: self.sessions.values().cloned().collect(),
            titled: self.titled.iter().map(|((tool, id), kind)| (*tool, id.clone(), *kind as u8)).collect(),
            limits: self.limits.clone(),
            paid: self.paid.clone(),
        };
        let mut bytes = serde_json::to_vec(&kept).expect("what was read is written");
        bytes.push(b'\n');
        for (at, keys) in &self.counted {
            bytes.extend(at.to_le_bytes());
            bytes.extend((keys.len() as u32).to_le_bytes());
            for key in keys {
                bytes.extend(key.to_le_bytes());
            }
        }
        bytes
    }

    fn from_bytes(bytes: &[u8]) -> Option<Ledger> {
        let line = memchr::memchr(b'\n', bytes)?;
        let kept: Kept = serde_json::from_slice(&bytes[..line]).ok()?;
        let mut counted = BTreeMap::new();
        let mut rest = &bytes[line + 1..];
        while !rest.is_empty() {
            let at = u32::from_le_bytes(rest.get(..4)?.try_into().ok()?);
            let n = u32::from_le_bytes(rest.get(4..8)?.try_into().ok()?) as usize;
            let keys = rest.get(8..8 + n * 8)?;
            counted.insert(at, keys.chunks_exact(8).map(|key| u64::from_le_bytes(key.try_into().unwrap())).collect::<HashSet<u64>>());
            rest = &rest[8 + n * 8..];
        }
        if kept.read_as != READ_AS {
            return None;
        }
        let kind = |kind: u8| [TitleKind::Asked, TitleKind::Summed, TitleKind::Titled, TitleKind::Named].get(kind as usize).copied();
        Some(Ledger {
            files: kept.files.into_iter().map(|(path, read)| (PathBuf::from(path), read)).collect(),
            spent: kept.spent.into_iter().map(|(tool, model, at, tokens)| ((tool, model, at), tokens)).collect(),
            sessions: kept.sessions.into_iter().map(|session| ((session.tool, session.id.clone()), session)).collect(),
            titled: kept.titled.into_iter().filter_map(|(tool, id, kind_of)| Some(((tool, id), kind(kind_of)?))).collect(),
            limits: kept.limits,
            counted,
            paid: kept.paid,
            failed: BTreeMap::new(),
            unkeyed: Default::default(),
        })
    }
}

/// What a `Ledger` is kept as, but for the responses counted.
#[derive(Serialize, Deserialize)]
struct Kept {
    /// How the records were read (see `READ_AS`): none, before it was told.
    #[serde(default)]
    read_as: u32,
    files: Vec<(String, Read)>,
    spent: Vec<(Tool, String, u32, Tokens)>,
    sessions: Vec<Session>,
    titled: Vec<(Tool, String, u8)>,
    limits: Vec<Limit>,
    #[serde(default)]
    paid: Vec<Paid>,
}

/// The day (since 1970, by UTC) the moment `ms` falls in: responses counted are let go of a day at a time.
fn day(ms: u64) -> u32 {
    (ms / DAY_MS) as u32
}

/// Every record (`.jsonl`) at `path` (one, or those in the folder it is and in the folders in it), with its length and
/// when it was last written to (ms since 1970).
fn records_in(path: &Path, found: &mut Vec<(PathBuf, u64, u64)>) {
    let modified_of = |meta: &std::fs::Metadata| meta.modified().ok().and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |since| since.as_millis() as u64);
    // A database, as it and what is written ahead of it (its `-wal`, gone as it is taken in) stand together: written
    // ahead, it is.
    let database = path.to_str().and_then(|name| name.strip_suffix("-wal")).map(PathBuf::from).filter(|database| is_database(database)).or_else(|| is_database(path).then(|| path.to_path_buf()));
    if let Some(database) = database {
        let parts: Vec<std::fs::Metadata> = [database.clone(), PathBuf::from(format!("{}-wal", database.display()))].iter().filter_map(|part| std::fs::metadata(part).ok()).filter(std::fs::Metadata::is_file).collect();
        if !parts.is_empty() {
            found.push((database, parts.iter().map(|part| part.len()).sum(), parts.iter().map(modified_of).max().unwrap_or(0)));
        }
        return;
    }
    let Ok(meta) = std::fs::metadata(path) else { return };
    if meta.is_file() {
        if path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl")) {
            found.push((path.to_path_buf(), meta.len(), modified_of(&meta)));
        }
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else { return };
    for entry in entries.flatten() {
        // (A link is not followed: a folder linked into itself is not listed for ever.)
        if entry.file_type().is_ok_and(|kind| !kind.is_symlink()) {
            records_in(&entry.path(), found);
        }
    }
}

/// Whether the file at `path` is a database of a tool's records (by its name: `.sqlite`).
fn is_database(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("sqlite"))
}

/// A key that stays the same for the same words in every build of Glance (FNV-1a, of each part and a byte between
/// them): what a response is counted by, kept from one run to the next.
fn key(parts: &[&str]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for part in parts {
        for byte in part.bytes().chain([0xFF]) {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    hash
}

/// The moment an ISO 8601 time in UTC ("2026-11-18T12:59:29.123Z") names, in ms since 1970 (another kind: none).
pub(super) fn moment(text: &str) -> Option<u64> {
    let b = text.as_bytes();
    let digits = |at: usize, n: usize| -> Option<u64> { b.get(at..at + n)?.iter().try_fold(0u64, |v, c| c.is_ascii_digit().then(|| v * 10 + (c - b'0') as u64)) };
    if b.get(4) != Some(&b'-') || b.get(10) != Some(&b'T') {
        return None;
    }
    let date = crate::todo::Day { year: digits(0, 4)? as i32, month: digits(5, 2)? as u32, day: digits(8, 2)? as u32 };
    let (hour, minute, second) = (digits(11, 2)?, digits(14, 2)?, digits(17, 2)?);
    let mut rest = &text[19..];
    let mut ms = 0;
    if let Some(fraction) = rest.strip_prefix('.') {
        let n = fraction.bytes().take_while(u8::is_ascii_digit).count();
        ms = fraction[..n.min(3)].bytes().fold(0u64, |v, c| v * 10 + (c - b'0') as u64) * 10u64.pow(3 - n.min(3) as u32);
        rest = &fraction[n..];
    }
    let offset_minutes: i64 = match rest.as_bytes() {
        [b'Z'] | [] => 0,
        [sign @ (b'+' | b'-'), h1, h2, b':', m1, m2] => {
            let value = |a: &u8, b: &u8| ((a - b'0') * 10 + (b - b'0')) as i64;
            let minutes = value(h1, h2) * 60 + value(m1, m2);
            if *sign == b'+' { minutes } else { -minutes }
        }
        _ => return None,
    };
    let seconds = date.number() * 86_400 + (hour * 3600 + minute * 60 + second) as i64 - offset_minutes * 60;
    u64::try_from(seconds).ok().map(|seconds| seconds * 1000 + ms)
}

/// `bytes` written to `name` in `dir`, whole or not at all.
#[cfg(windows)]
fn write_whole(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    crate::elevation::write_in_place(dir, name, bytes)
}

#[cfg(not(windows))]
fn write_whole(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    let beside = dir.join(format!("{name}.new"));
    std::fs::write(&beside, bytes)?;
    std::fs::rename(beside, dir.join(name))
}

#[cfg(test)]
mod tests;
