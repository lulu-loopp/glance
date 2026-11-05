//! Glance's log: what went wrong, and a few things worth knowing later
//! (an update found or refused), a line each with the local time, kept in
//! the settings folder beside settings.json. Only the latest lines are kept,
//! a line repeated within minutes is counted rather than written again, and
//! nothing that names the user goes in: a user's folder in a path is written
//! `<user>`. Written as the settings are (see `elevation::write_in_place`):
//! Glance runs elevated and never follows a link someone put in the user's
//! folder; and as they are, on a thread of its own (see `write_later`).

use std::cell::Cell;
use std::collections::VecDeque;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use windows::Win32::System::SystemInformation::GetLocalTime;

/// The log's file, in the settings folder.
pub const FILE: &str = "glance.log";
/// How many lines are kept, how long each may be, and so how large the
/// file can be (a larger one is not Glance's, and is started afresh).
const KEPT: usize = 200;
const LINE_MOST: usize = 400;
const FILE_MOST: u64 = (KEPT * (LINE_MOST + 32) * 4) as u64;
/// The same line again within this long is counted, not written.
const REPEATS_WITHIN: Duration = Duration::from_secs(10 * 60);

struct Log {
    lines: VecDeque<String>,
    /// The last line written, when, and how often it came again since.
    last: Option<(String, Instant, u32)>,
}

/// The log so far, read from the file at first use.
static LOG: Mutex<Option<Log>> = Mutex::new(None);

thread_local! {
    /// This thread is keeping a note: one more from it (a crash inside the
    /// keeping) would wait on itself, and is let go.
    static NOTING: Cell<bool> = const { Cell::new(false) };
}

/// Adds a line to the log, and keeps the log.
pub fn note(what: impl Display) {
    // The sensors' process writes no file of the user's: its lines go to the rest of Glance, which writes them.
    if KEPT_FOR_THE_APP.load(std::sync::atomic::Ordering::Relaxed) {
        let mut kept = HELD.lock().unwrap_or_else(PoisonError::into_inner);
        if kept.len() >= KEPT {
            kept.pop_front();
        }
        kept.push_back(private(&what.to_string()));
        return;
    }
    note_in(&crate::settings::config_dir(), what);
}

/// Whether this process's lines are held for the rest of Glance (in the sensors' process), and those held.
static KEPT_FOR_THE_APP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static HELD: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

/// From now on this process's lines are held for the rest of Glance (see `take_held`), the file left to it.
pub fn hold_for_the_app() {
    KEPT_FOR_THE_APP.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Says the lines held (the latest `KEPT` of them) by `say`, for the rest of Glance to write; whether all were said. A
/// line `say` could not say stays held, with those after it: the pipe that broke takes no line along with it.
pub fn say_held(say: impl FnMut(String) -> bool) -> bool {
    say_each(&HELD, say)
}

fn say_each(held: &Mutex<VecDeque<String>>, mut say: impl FnMut(String) -> bool) -> bool {
    loop {
        // (The lock not held as it is said: a pipe no one reads holds whoever writes to it, and what notes a line waits
        // for the lock.)
        let Some(line) = held.lock().unwrap_or_else(PoisonError::into_inner).pop_front() else { return true };
        if !say(line.clone()) {
            held.lock().unwrap_or_else(PoisonError::into_inner).push_front(line);
            return false;
        }
    }
}

/// What the sensors' process had still to say as it ended, kept in the settings folder for the rest of Glance to find
/// (there was no link to say it over, or it had gone). A file of its own: two processes writing one file lose lines.
const LEFT: &str = "sensors-left.log";

/// Leaves the lines held for the rest of Glance to find (see `take_left`), as this process ends.
pub fn leave_held() {
    if KEPT_FOR_THE_APP.load(std::sync::atomic::Ordering::Relaxed) {
        let lines: Vec<String> = HELD.lock().unwrap_or_else(PoisonError::into_inner).drain(..).collect();
        leave_in(&crate::settings::config_dir(), &lines);
    }
}

/// The latest `KEPT` of what was left in `dir`, `lines` added, each with the time it is left.
fn leave_in(dir: &Path, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let before = crate::elevation::read_in_place(dir, LEFT, FILE_MOST).unwrap_or_default();
    let mut left: VecDeque<String> = before.lines().map(str::to_string).collect();
    lines.iter().for_each(|line| push(&mut left, line.clone()));
    let text: String = left.iter().map(|line| format!("{line}\n")).collect();
    write_as(dir, LEFT, &text);
}

/// What the sensors' process left (see `leave_held`), oldest first, taken: it is not found twice.
pub fn take_left() -> Vec<String> {
    take_left_in(&crate::settings::config_dir())
}

fn take_left_in(dir: &Path) -> Vec<String> {
    let left = crate::elevation::read_in_place(dir, LEFT, FILE_MOST).unwrap_or_default();
    let _ = std::fs::remove_file(dir.join(LEFT));
    left.lines().map(str::to_string).collect()
}

/// `note`, with the log kept in `dir`.
fn note_in(dir: &std::path::Path, what: impl Display) {
    if NOTING.with(|noting| noting.replace(true)) {
        return;
    }
    let mut log = LOG.lock().unwrap_or_else(PoisonError::into_inner);
    log.get_or_insert_with(|| read(dir)).add(dir, what);
    NOTING.with(|noting| noting.set(false));
}

impl Log {
    /// Adds `what` and keeps the log in `dir` (a line that came just before is counted, not written).
    fn add(&mut self, dir: &std::path::Path, what: impl Display) {
        let what = private(&what.to_string());
        let now = Instant::now();
        let again = self.last.as_mut().filter(|(last, at, _)| *last == what && now.duration_since(*at) < REPEATS_WITHIN);
        if let Some((_, _, more)) = again {
            *more += 1;
            return;
        }
        if let Some((_, _, more)) = self.last.take().filter(|(_, _, more)| *more > 0) {
            push(&mut self.lines, format!("(the line before came {more} more times)"));
        }
        push(&mut self.lines, what.clone());
        self.last = Some((what, now, 0));
        let text: String = self.lines.iter().map(|line| format!("{line}\n")).collect();
        match APART.load(std::sync::atomic::Ordering::Relaxed) {
            true => write_later(dir.to_path_buf(), text),
            false => write(dir, &text),
        }
    }
}

/// Whether the log is written on a thread of its own (see `write_later`): in Glance's own run, which waits for it as
/// it ends; else as it is noted (the elevated helpers', which end as soon as they are done).
static APART: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// From now on the log is written on a thread of its own (see `write_later`); `written` waits for it.
pub fn write_apart() {
    APART.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// The log's text to be written in each folder it is kept in (the latest: one noted before it is not written too), and
/// whether some is being written. Written to the disk as it is (see `elevation::write_in_place`), a line noted on a
/// thread that draws held it up for as long: it is written on a thread of its own, as the settings are (see
/// `save_later`).
static UNWRITTEN: Mutex<(Vec<(PathBuf, String)>, bool)> = Mutex::new((Vec::new(), false));
static WRITTEN: Condvar = Condvar::new();
/// Held while the log's file is written, and while what is written next is taken (see `written_now`).
static WRITING: Mutex<()> = Mutex::new(());

thread_local! {
    /// This is the thread that writes the log.
    static WRITER: Cell<bool> = const { Cell::new(false) };
}

/// Writes `text` in `dir` on the thread that writes the log, after what came before.
fn write_later(dir: PathBuf, text: String) {
    let mut unwritten = UNWRITTEN.lock().unwrap_or_else(PoisonError::into_inner);
    let idle = unwritten.0.is_empty() && !unwritten.1;
    match unwritten.0.iter_mut().find(|(kept_in, _)| *kept_in == dir) {
        Some(waiting) => waiting.1 = text,
        None => unwritten.0.push((dir, text)),
    }
    if !idle {
        return;
    }
    unwritten.1 = true;
    drop(unwritten);
    std::thread::Builder::new()
        .name("glance: journal written".into())
        .spawn(|| {
            WRITER.with(|writer| writer.set(true));
            loop {
                let _writing = WRITING.lock().unwrap_or_else(PoisonError::into_inner);
                let next = {
                    let mut unwritten = UNWRITTEN.lock().unwrap_or_else(PoisonError::into_inner);
                    if unwritten.0.is_empty() {
                        unwritten.1 = false;
                        WRITTEN.notify_all();
                        return;
                    }
                    std::mem::take(&mut unwritten.0)
                };
                next.iter().for_each(|(dir, text)| write(dir, text));
            }
        })
        .expect("a thread started");
}

fn write(dir: &Path, text: &str) {
    write_as(dir, FILE, text);
}

fn write_as(dir: &Path, name: &str, text: &str) {
    if crate::elevation::ensure_folder(dir).is_ok() {
        let _ = crate::elevation::write_in_place(dir, name, text.as_bytes());
    }
}

/// Waits till what was noted is written.
pub fn written() {
    let mut unwritten = UNWRITTEN.lock().unwrap_or_else(PoisonError::into_inner);
    while !unwritten.0.is_empty() || unwritten.1 {
        unwritten = WRITTEN.wait(unwritten).unwrap_or_else(PoisonError::into_inner);
    }
}

/// Writes what was noted here and now, as the process ends at once (a crash): what the thread that writes the log
/// takes after was noted after it. (Not on that thread: it would wait on itself.)
fn written_now() {
    if WRITER.with(Cell::get) {
        return;
    }
    let _writing = WRITING.lock().unwrap_or_else(PoisonError::into_inner);
    let next = std::mem::take(&mut UNWRITTEN.lock().unwrap_or_else(PoisonError::into_inner).0);
    next.iter().for_each(|(dir, text)| write(dir, text));
}

/// Adds `what`, stamped with the local time, keeping only the latest lines.
fn push(lines: &mut VecDeque<String>, what: String) {
    let time = unsafe { GetLocalTime() };
    lines.push_back(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}  {what}",
        time.wYear, time.wMonth, time.wDay, time.wHour, time.wMinute, time.wSecond
    ));
    while lines.len() > KEPT {
        lines.pop_front();
    }
}

/// `text` on one line, of at most `LINE_MOST` characters, with the user's
/// folder in any path (what follows `\Users\`) written `<user>`.
fn private(text: &str) -> String {
    let flat = text.replace(['\r', '\n'], " ");
    let mut out = String::with_capacity(flat.len());
    let mut rest = flat.as_str();
    const USERS: &str = "\\users\\";
    while let Some(at) = rest.to_ascii_lowercase().find(USERS) {
        let after = at + USERS.len();
        out.push_str(&rest[..after]);
        out.push_str("<user>");
        // A user's name may hold spaces and apostrophes: it ends at the next
        // separator or double quote, or with the text.
        let name = rest[after..].find(['\\', '/', '"']).unwrap_or(rest.len() - after);
        rest = &rest[after + name..];
    }
    out.push_str(rest);
    out.chars().take(LINE_MOST).collect()
}

/// The latest `count` lines, oldest first.
pub fn recent(count: usize) -> Vec<String> {
    let mut log = LOG.lock().unwrap_or_else(PoisonError::into_inner);
    let log = log.get_or_insert_with(|| read(&crate::settings::config_dir()));
    log.lines.iter().skip(log.lines.len().saturating_sub(count)).cloned().collect()
}

fn read(dir: &std::path::Path) -> Log {
    let text = crate::elevation::read_in_place(dir, FILE, FILE_MOST).unwrap_or_default();
    let mut lines: VecDeque<String> = text.lines().map(|line| line.chars().take(LINE_MOST + 21).collect()).collect();
    while lines.len() > KEPT {
        lines.pop_front();
    }
    Log { lines, last: None }
}

/// Notes a crash before the process ends (release builds abort on panic):
/// its message, and the source file and line, by the file's name alone.
pub fn note_crashes() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let message = payload.downcast_ref::<&str>().copied().or_else(|| payload.downcast_ref::<String>().map(String::as_str)).unwrap_or("");
        let place = info.location().map(|at| {
            let file = std::path::Path::new(at.file()).file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
            format!(" at {file}:{}", at.line())
        });
        note(format!("crashed: {message}{}", place.unwrap_or_default()));
        written_now();
        leave_held();
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_latest_lines_counts_repeats_and_names_no_one() {
        // (A log of the test's own, in a folder of its own: the process's is written by the other tests meanwhile.)
        let dir = std::env::temp_dir().join(format!("glance-journal-test-{}", std::process::id()));
        let mut log = read(&dir);
        for i in 0..KEPT + 5 {
            log.add(&dir, format!("line {i}\nwith a break"));
        }
        // The same line again: counted, and told once another comes.
        for _ in 0..3 {
            log.add(&dir, "again");
        }
        log.add(&dir, r"could not read C:\Users\Someone O'Else\AppData\x and D:\Users\me/y");
        written();
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), KEPT);
        assert!(lines[KEPT - 4].ends_with(&format!("  line {} with a break", KEPT + 4)), "{}", lines[KEPT - 4]);
        assert!(lines[KEPT - 3].ends_with("  again"));
        assert!(lines[KEPT - 2].ends_with("  (the line before came 2 more times)"));
        assert!(lines[KEPT - 1].ends_with(r"could not read C:\Users\<user>\AppData\x and D:\Users\<user>/y"), "{}", lines[KEPT - 1]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A line the pipe would not take stays held, with those after it, and goes first the next time.
    #[test]
    fn a_line_not_said_stays_held() {
        let held = Mutex::new(VecDeque::from(["one".to_string(), "two".to_string(), "three".to_string()]));
        let mut said = Vec::new();
        assert!(!say_each(&held, |line| {
            let taken = line != "two";
            if taken {
                said.push(line);
            }
            taken
        }));
        assert_eq!(said, ["one"]);
        assert_eq!(*held.lock().unwrap(), ["two", "three"]);
        assert!(say_each(&held, |line| {
            said.push(line);
            true
        }));
        assert_eq!(said, ["one", "two", "three"]);
        assert!(held.lock().unwrap().is_empty());
    }

    /// What the sensors left as they ended is found once, in order, from one end after another; no more of it is kept
    /// than of the log.
    #[test]
    fn what_was_left_is_found_once() {
        let dir = std::env::temp_dir().join(format!("glance-journal-left-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(take_left_in(&dir).is_empty());
        leave_in(&dir, &["first".to_string()]);
        leave_in(&dir, &[]);
        leave_in(&dir, &["second".to_string()]);
        let left = take_left_in(&dir);
        assert_eq!(left.len(), 2);
        assert!(left[0].ends_with("  first") && left[1].ends_with("  second"), "{left:?}");
        assert!(take_left_in(&dir).is_empty());
        let many: Vec<String> = (0..KEPT + 3).map(|i| format!("line {i}")).collect();
        leave_in(&dir, &many);
        let left = take_left_in(&dir);
        assert_eq!(left.len(), KEPT);
        assert!(left[KEPT - 1].ends_with(&format!("  line {}", KEPT + 2)), "{}", left[KEPT - 1]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Written apart (Glance's own run), what was noted is in the file once `written` returns, the latest of it: lines
    /// noted one after another while one is written are written together, after it.
    #[test]
    fn what_was_noted_apart_is_written_once_waited_for() {
        let dir = std::env::temp_dir().join(format!("glance-journal-apart-test-{}", std::process::id()));
        // (The other tests' lines, written apart too from now on, are written where they always are.)
        write_apart();
        let mut log = read(&dir);
        for i in 0..50 {
            log.add(&dir, format!("apart {i}"));
        }
        written();
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 50);
        assert!(lines[49].ends_with("  apart 49"), "{}", lines[49]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
