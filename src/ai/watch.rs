//! What AI tools spent, kept current on a thread of its own while something shows it: read whole as it is first
//! wanted (once the machine has settled after it started, or at once where the user asked for it), then only the
//! files Windows tells were written to (each of the tools' folders watched), the files at work looked at every minute
//! besides (Windows tells of a file kept open as it is written to only now and then), and all of them again every hour
//! (what was missed, and what is no longer kept, let go of). What was read is kept in Glance's local folder now and
//! then. Claude's limits, as Claude Code tells them to its status line where Glance is that (see `statusline`), are read
//! as they are written. The providers' accounts (see `providers`) are read on a thread of their own, each one whose
//! lane is on and whose key is set, every few minutes while what was spent is wanted, and at once as its keys change.

use super::{Ledger, Tool, Usage};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FILE_NOTIFY_INFORMATION,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};

/// How often the files at work are looked at, and what was spent told again (today's, as a day ends).
const TICK: Duration = Duration::from_secs(60);
/// Files written to within this long are at work.
const AT_WORK_MS: u64 = 30 * 60 * 1000;
/// How often all the records are read again.
const WHOLE: Duration = Duration::from_secs(60 * 60);
/// How often a provider's account is read again, while it is wanted.
const ASKED_EVERY: Duration = Duration::from_secs(5 * 60);
/// How often the providers' thread looks whether one is due.
const ASK_TICK: Duration = Duration::from_secs(60);
/// How long after Windows tells a file was written to it is read (what else is written meanwhile read with it).
const GATHER: Duration = Duration::from_secs(2);
/// How often what was read is kept, while more is.
const KEEP: Duration = Duration::from_secs(10 * 60);

/// What the thread hears.
enum Heard {
    /// Asked to look whether it is wanted.
    Look,
    /// Claude Code told its status line Claude's limits.
    Told,
    /// These records of `Tool` were written to.
    Written(Tool, Vec<PathBuf>),
    /// More was written under a folder than Windows kept: all read again.
    Lost,
    /// A folder is watched no more (gone, or it could not be watched): watched again, where it can be, as all are read
    /// again.
    Ended(PathBuf),
    /// How reading a provider's account went.
    Read(Tool, u64, Result<super::Limit, super::Failed>),
    /// A provider's key was taken away.
    Unkeyed(Tool),
}

/// What was spent as last read (none: not yet read, or not wanted), and a number that changes as it does (see `stamp`).
static NOW: Mutex<Option<Arc<Usage>>> = Mutex::new(None);
static STAMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// The thread's ear, once it has started.
static EAR: Mutex<Option<Sender<Heard>>> = Mutex::new(None);
/// The providers' thread's ear: told to look whether one is due, or (true) that keys changed.
static ASK: Mutex<Option<Sender<bool>>> = Mutex::new(None);

/// What was spent as last read. Asked for, it is shown: its first reading begins now, the machine settled or not.
pub fn usage() -> Option<Arc<Usage>> {
    crate::settle::want(crate::settle::Work::AiUsage);
    NOW.lock().unwrap().clone()
}

/// A number that changes whenever what was spent as last read does: what shows it, read as it is drawn (the AI
/// component's dock), is drawn again as it changes, not drawn from what was kept of it.
pub fn stamp() -> u64 {
    STAMP.load(std::sync::atomic::Ordering::Relaxed)
}

/// What was spent as last read, as it now is.
fn publish(usage: Option<Arc<Usage>>, held: &mut Option<Arc<Usage>>) {
    *held = usage;
    STAMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Asked to look at once whether what was spent is wanted (something that shows it came on or went).
pub fn look_now() {
    if let Some(ear) = &*EAR.lock().unwrap() {
        let _ = ear.send(Heard::Look);
    }
    if let Some(ask) = &*ASK.lock().unwrap() {
        let _ = ask.send(false);
    }
}

/// A provider's keys were given or taken away: each provider read again at once (where it is wanted).
pub fn keys_changed() {
    if let Some(ask) = &*ASK.lock().unwrap() {
        let _ = ask.send(true);
    }
}

/// Keeps what was spent current while `wanted` says something shows it (looked at again as `look_now` asks), telling
/// `told` as it changes; the providers' accounts too, each whose lane `on` says is on.
pub fn start(wanted: impl Fn() -> bool + Clone + Send + 'static, on: impl Fn(Tool) -> bool + Send + 'static, told: impl Fn() + Send + 'static) {
    start_in(super::roots, wanted, on, told);
}

/// As `start`, with the records where `roots` says (asked again as all are read again).
fn start_in(roots: impl Fn() -> Vec<(Tool, PathBuf)> + Send + 'static, wanted: impl Fn() -> bool + Clone + Send + 'static, on: impl Fn(Tool) -> bool + Send + 'static, told: impl Fn() + Send + 'static) {
    let (ear, heard) = channel();
    *EAR.lock().unwrap() = Some(ear.clone());
    let (ask, asked) = channel();
    *ASK.lock().unwrap() = Some(ask);
    let (providers_wanted, providers_ear) = (wanted.clone(), ear.clone());
    std::thread::Builder::new().name("glance: AI usage".into()).spawn(move || keep(roots, wanted, told, ear, heard)).expect("a thread started");
    std::thread::Builder::new().name("glance: AI providers".into()).spawn(move || ask_providers(providers_wanted, on, providers_ear, asked)).expect("a thread started");
}

/// Reads each provider's account whose lane `on` says is on and whose keys are set, while `wanted`, as it is due
/// (`ASKED_EVERY` after it was last read, or at once where its keys changed since), telling `ear` how it went; one
/// whose keys are not set, that they are not (once, till they are). Looks again as `asked` tells it to, or every `ASK_TICK`.
fn ask_providers(wanted: impl Fn() -> bool, on: impl Fn(Tool) -> bool, ear: Sender<Heard>, asked: Receiver<bool>) {
    use std::hash::{Hash, Hasher};
    // When each was last read, and a hash of the keys it was read with (only in memory).
    let mut read: HashMap<Tool, (Instant, u64)> = HashMap::new();
    // Those told to have no keys (once each, till they have some again).
    let mut unkeyed: std::collections::HashSet<Tool> = std::collections::HashSet::new();
    loop {
        if wanted() {
            for provider in &super::providers::ALL {
                let tool = provider.tool;
                let Some(keys) = provider.keys() else {
                    read.remove(&tool);
                    if unkeyed.insert(tool) && ear.send(Heard::Unkeyed(tool)).is_err() {
                        return;
                    }
                    continue;
                };
                unkeyed.remove(&tool);
                if !on(tool) {
                    continue;
                }
                let mut hash = std::hash::DefaultHasher::new();
                keys.hash(&mut hash);
                let hash = hash.finish();
                if read.get(&tool).is_some_and(|(at, with)| at.elapsed() < ASKED_EVERY && *with == hash) {
                    continue;
                }
                let told = provider.read(&keys, crate::station::now_ms());
                read.insert(tool, (Instant::now(), hash));
                if ear.send(Heard::Read(tool, hash, told)).is_err() {
                    return;
                }
            }
            // Codex's limits, asked of the ChatGPT account it is signed in with (its logs here tell them only as Codex
            // last worked here: see `codex`). Where it cannot be asked (signed out, its sign-in expired, ChatGPT not
            // reached), what its logs tell stands: nothing is told but what it answers.
            if on(Tool::Codex) {
                if let Some((token, account)) = super::codex::signed_in() {
                    let mut hash = std::hash::DefaultHasher::new();
                    (&token, &account).hash(&mut hash);
                    let hash = hash.finish();
                    if read.get(&Tool::Codex).is_none_or(|(at, with)| at.elapsed() >= ASKED_EVERY || *with != hash) {
                        let told = super::providers::codex(&token, account.as_deref(), crate::station::now_ms());
                        read.insert(Tool::Codex, (Instant::now(), hash));
                        if let Ok(limit) = told {
                            if ear.send(Heard::Read(Tool::Codex, hash, Ok(limit))).is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        }
        match asked.recv_timeout(ASK_TICK) {
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn keep(roots: impl Fn() -> Vec<(Tool, PathBuf)>, wanted: impl Fn() -> bool, told: impl Fn(), ear: Sender<Heard>, heard: Receiver<Heard>) {
    crate::settle::background("AI usage");
    let dir = crate::settings::local_dir();
    let mut ledger: Option<Ledger> = None;
    let mut watched: Vec<PathBuf> = Vec::new();
    loop {
        // (Not wanted: what Windows tells is let go of, all read again as it is.)
        while !wanted() {
            publish(None, &mut NOW.lock().unwrap());
            if heard.recv().is_err() {
                return;
            }
        }
        crate::settle::wait(crate::settle::Work::AiUsage);
        // (Nothing of it shown yet: what the user is waiting for, read as their programs read, and in the background
        // again once it is told. The first time, it is a month of records.)
        let waited = NOW.lock().unwrap().is_none().then(|| crate::settle::Waited::begin("AI usage"));
        let ledger = ledger.get_or_insert_with(|| Ledger::load(&dir));
        let roots = roots();
        for (tool, root) in &roots {
            if !watched.contains(root) && root.is_dir() {
                watched.push(root.clone());
                let tool = *tool;
                watch(root.clone(), ear.clone(), move |paths| Heard::Written(tool, paths));
            }
        }
        // (Its folder made, to be watched before the status line first writes in it.)
        let told_in = super::statusline::folder();
        if !watched.contains(&told_in) && std::fs::create_dir_all(&told_in).is_ok() {
            watched.push(told_in.clone());
            watch(told_in.clone(), ear.clone(), |_| Heard::Told);
        }
        let now = crate::station::now_ms();
        if ledger.refresh(&roots, now) | limits_told(ledger, &told_in) {
            keep_in(ledger, &dir);
        }
        tell(ledger, now, &told);
        drop(waited);
        let mut kept = Instant::now();
        let whole = Instant::now();
        let mut ticked = Instant::now();
        let mut dirty = false;
        // Till it is not wanted, or an hour on.
        while wanted() && whole.elapsed() < WHOLE {
            let mut written: Vec<(Tool, Vec<PathBuf>)> = Vec::new();
            let (mut lost, mut limits) = (false, false);
            let mut accounts: Vec<Heard> = Vec::new();
            match heard.recv_timeout(TICK.saturating_sub(ticked.elapsed())) {
                Ok(Heard::Look) => continue,
                Ok(Heard::Told) => limits = true,
                Ok(account @ (Heard::Read(..) | Heard::Unkeyed(_))) => accounts.push(account),
                Ok(Heard::Written(tool, paths)) => written.push((tool, paths)),
                Ok(Heard::Lost) => lost = true,
                Ok(Heard::Ended(root)) => watched.retain(|watched| *watched != root),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            // (What else is written meanwhile read with it.)
            if !written.is_empty() {
                let until = Instant::now() + GATHER;
                while let Some(left) = until.checked_duration_since(Instant::now()) {
                    match heard.recv_timeout(left) {
                        Ok(Heard::Written(tool, paths)) => written.push((tool, paths)),
                        Ok(Heard::Lost) => lost = true,
                        Ok(Heard::Ended(root)) => watched.retain(|watched| *watched != root),
                        Ok(Heard::Look) => {}
                        Ok(Heard::Told) => limits = true,
                        Ok(account @ (Heard::Read(..) | Heard::Unkeyed(_))) => accounts.push(account),
                        Err(_) => break,
                    }
                }
            }
            if lost {
                break;
            }
            let now = crate::station::now_ms();
            let mut changed = limits && limits_told(ledger, &told_in);
            for account in accounts {
                changed |= match account {
                    Heard::Read(tool, key, told) => ledger.read_of(tool, key, told, now),
                    Heard::Unkeyed(tool) => ledger.unkeyed(tool),
                    _ => false,
                };
            }
            for (tool, mut paths) in written {
                paths.sort();
                paths.dedup();
                changed |= ledger.read_paths(tool, &paths, now);
            }
            let tick = ticked.elapsed() >= TICK;
            if tick {
                ticked = Instant::now();
                changed |= ledger.read_recent(&roots, now.saturating_sub(AT_WORK_MS), now);
            }
            dirty |= changed;
            // (Told every minute besides: today's, as a day ends.)
            if changed || tick {
                tell(ledger, now, &told);
            }
            if dirty && kept.elapsed() >= KEEP {
                keep_in(ledger, &dir);
                kept = Instant::now();
                dirty = false;
            }
        }
        if dirty {
            keep_in(ledger, &dir);
        }
    }
}

/// Claude's limits as the status line last kept them in `folder`, taken where they are newer; none kept there (Glance
/// taken out of Claude Code's status line), those it told before no longer kept. Whether they changed.
fn limits_told(ledger: &mut Ledger, folder: &std::path::Path) -> bool {
    match super::statusline::told_in(folder) {
        Some(limit) => ledger.told_limit(limit),
        None => ledger.forget_limit(Tool::Claude, super::statusline::ID),
    }
}

/// What was spent as it stands at `now` told, where it changed.
fn tell(ledger: &Ledger, now: u64, told: &impl Fn()) {
    let usage = ledger.usage(now);
    let mut held = NOW.lock().unwrap();
    // (Only the time it was told at changed: not told again.)
    if held.as_deref().is_some_and(|held| Usage { now: usage.now, ..held.clone() } == usage) {
        return;
    }
    publish(Some(Arc::new(usage)), &mut held);
    drop(held);
    told();
}

/// What was read kept in `dir` (not kept: noted, read afresh as Glance starts).
fn keep_in(ledger: &Ledger, dir: &std::path::Path) {
    if let Err(error) = ledger.save(dir) {
        crate::journal::note(format!("AI usage: not kept ({error})"));
    }
}

/// Watches the folder `root` and those in it on a thread of its own, telling `ear` what `heard` makes of the files
/// written to there, till it is gone (told as ended).
fn watch(root: PathBuf, ear: Sender<Heard>, heard: impl Fn(Vec<PathBuf>) -> Heard + Send + 'static) {
    /// FILE_LIST_DIRECTORY.
    const LIST: u32 = 0x1;
    let _ = std::thread::Builder::new().name("glance: AI records watched".into()).spawn(move || {
        let held = unsafe { CreateFileW(&windows::core::HSTRING::from(root.as_os_str()), LIST, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, None, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, None) };
        let Ok(folder) = held else {
            let _ = ear.send(Heard::Ended(root));
            return;
        };
        // (u64s: the records in it are on 4-byte boundaries.)
        let mut buffer = vec![0u64; 64 * 1024 / 8];
        loop {
            let mut got = 0u32;
            let read = unsafe { ReadDirectoryChangesW(folder, buffer.as_mut_ptr() as *mut _, (buffer.len() * 8) as u32, true, FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_SIZE, Some(&mut got), None, None) };
            let heard = match read {
                Err(_) => None,
                Ok(()) if got == 0 => Some(Heard::Lost),
                Ok(()) => Some(heard(names(unsafe { std::slice::from_raw_parts(buffer.as_ptr() as *const u8, got as usize) }).into_iter().map(|name| root.join(name)).collect())),
            };
            let Some(heard) = heard else { break };
            if ear.send(heard).is_err() {
                break;
            }
        }
        let _ = ear.send(Heard::Ended(root));
        close(folder);
    });
}

fn close(handle: HANDLE) {
    unsafe {
        let _ = CloseHandle(handle);
    }
}

/// The names (paths from the folder watched) in the records in `bytes`.
fn names(bytes: &[u8]) -> Vec<PathBuf> {
    let mut names = Vec::new();
    let mut at = 0usize;
    while at + size_of::<FILE_NOTIFY_INFORMATION>() <= bytes.len() {
        let record = unsafe { &*(bytes.as_ptr().add(at) as *const FILE_NOTIFY_INFORMATION) };
        let len = record.FileNameLength as usize / 2;
        let start = at + std::mem::offset_of!(FILE_NOTIFY_INFORMATION, FileName);
        if start + len * 2 > bytes.len() {
            break;
        }
        let name = unsafe { std::slice::from_raw_parts(bytes.as_ptr().add(start) as *const u16, len) };
        names.push(PathBuf::from(<std::ffi::OsString as std::os::windows::ffi::OsStringExt>::from_wide(name)));
        if record.NextEntryOffset == 0 {
            break;
        }
        at += record.NextEntryOffset as usize;
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// What was spent asked for (to be shown: the AI component's lanes, its dock, its card) has its first reading begin
    /// at once, the machine settled or not (one kept busy would otherwise show it being read for as long as it is).
    /// (Run alone, this process's machine never settled: a test run is never ten seconds idle.)
    #[test]
    fn what_was_spent_asked_for_is_read_at_once() {
        let _ = usage();
        assert!(crate::settle::ready(crate::settle::Work::AiUsage));
    }

    /// What was spent told anew changes the stamp what shows it is drawn again by (the dock reads it as it is drawn,
    /// and what is drawn of it is kept by that), told again as it was it does not (what is drawn of it is kept).
    #[test]
    fn what_was_spent_told_anew_is_stamped() {
        let mut ledger = Ledger::default();
        ledger.unkeyed(Tool::DeepSeek);
        let before = stamp();
        tell(&ledger, 1, &|| {});
        let told = stamp();
        assert!(told > before, "told anew: stamped");
        tell(&ledger, 2, &|| {});
        assert_eq!(stamp(), told, "only the time it was told at changed: not stamped");
        ledger.unkeyed(Tool::Kimi);
        tell(&ledger, 3, &|| {});
        assert!(stamp() > told, "changed: stamped");
    }

    /// Claude's limits taken from what the status line kept, where newer than those taken before; what it kept gone
    /// (Glance taken out of Claude Code's status line), no longer kept.
    #[test]
    fn takes_claudes_limits_as_the_status_line_keeps_them() {
        let folder = std::env::temp_dir().join(format!("glance-test-{}", std::process::id())).join("limits-told");
        std::fs::create_dir_all(&folder).unwrap();
        let limit = |told| super::super::Limit { tool: Tool::Claude, id: super::super::statusline::ID.into(), name: None, plan: None, windows: vec![super::super::Window { minutes: 300, used: 40.0, resets: None }], balance: None, told };
        let keep = |told| std::fs::write(folder.join(super::super::statusline::LIMITS), serde_json::to_vec(&limit(told)).unwrap()).unwrap();
        let mut ledger = Ledger::default();
        assert!(!limits_told(&mut ledger, &folder));
        keep(5);
        assert!(limits_told(&mut ledger, &folder) && ledger.limits.len() == 1 && ledger.limits[0].told == 5);
        assert!(!limits_told(&mut ledger, &folder));
        std::fs::remove_file(folder.join(super::super::statusline::LIMITS)).unwrap();
        assert!(limits_told(&mut ledger, &folder) && ledger.limits.is_empty());
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Waits up to five seconds for `done`.
    fn soon(done: impl Fn() -> bool) -> bool {
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        done()
    }

    fn today(usage: &Usage) -> u64 {
        usage.spent_today.iter().map(|(_, _, tokens)| tokens.total()).sum()
    }

    /// Read whole as it is wanted, then what is written as Windows tells it, told each time; nothing told while it is
    /// not wanted, read again as it is wanted again.
    #[test]
    fn keeps_what_was_spent_current_while_it_is_wanted() {
        static WANTED: AtomicBool = AtomicBool::new(true);
        static TOLD: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!("glance-ai-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let file = root.join("p").join("s.jsonl");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let line = |id: &str, input: u64| {
            let at = crate::ai::tests::stamp(crate::station::now_ms());
            format!(r#"{{"type":"assistant","sessionId":"s1","requestId":"r{id}","timestamp":"{at}","message":{{"id":"{id}","model":"claude-opus-5-5","usage":{{"input_tokens":{input},"output_tokens":0}}}}}}"#) + "
"
        };
        let append = |text: String| std::fs::OpenOptions::new().create(true).append(true).open(&file).unwrap().write_all(text.as_bytes()).unwrap();
        append(line("m1", 5));
        crate::settle::want(crate::settle::Work::AiUsage);
        let watched = root.clone();
        // (No provider's account read: a test sends no key of the user's anywhere.)
        start_in(move || vec![(Tool::Claude, watched.clone())], || WANTED.load(Ordering::SeqCst), |_| false, || { TOLD.fetch_add(1, Ordering::SeqCst); });
        assert!(soon(|| usage().is_some_and(|usage| today(&usage) == 5)), "read whole as it is wanted");
        let told = TOLD.load(Ordering::SeqCst);
        append(line("m2", 7));
        assert!(soon(|| usage().is_some_and(|usage| today(&usage) == 12)), "read as Windows tells it was written to");
        assert!(TOLD.load(Ordering::SeqCst) > told);
        WANTED.store(false, Ordering::SeqCst);
        look_now();
        assert!(soon(|| usage().is_none()), "let go of while it is not wanted");
        append(line("m3", 1));
        WANTED.store(true, Ordering::SeqCst);
        look_now();
        assert!(soon(|| usage().is_some_and(|usage| today(&usage) == 13)), "read again as it is wanted again");
        let _ = std::fs::remove_dir_all(root);
    }
}
