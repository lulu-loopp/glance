//! Glance's log: what went wrong, and a few things worth knowing later
//! (an update found or refused), a line each with the local time, kept in
//! the settings folder beside settings.json. Only the latest lines are kept,
//! a line repeated within minutes is counted rather than written again, and
//! nothing that names the user goes in: a user's folder in a path is written
//! `<user>`. Written as the settings are (see `elevation::write_in_place`):
//! Glance runs elevated and never follows a link someone put in the user's
//! folder.

use std::cell::Cell;
use std::collections::VecDeque;
use std::fmt::Display;
use std::sync::{Mutex, PoisonError};
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
    note_in(&crate::settings::config_dir(), what);
}

/// `note`, with the log kept in `dir`.
fn note_in(dir: &std::path::Path, what: impl Display) {
    if NOTING.with(|noting| noting.replace(true)) {
        return;
    }
    let what = private(&what.to_string());
    let mut log = LOG.lock().unwrap_or_else(PoisonError::into_inner);
    let log = log.get_or_insert_with(|| read(dir));
    let now = Instant::now();
    let again = log.last.as_mut().filter(|(last, at, _)| *last == what && now.duration_since(*at) < REPEATS_WITHIN);
    if let Some((_, _, more)) = again {
        *more += 1;
    } else {
        if let Some((_, _, more)) = log.last.take().filter(|(_, _, more)| *more > 0) {
            push(&mut log.lines, format!("(the line before came {more} more times)"));
        }
        push(&mut log.lines, what.clone());
        log.last = Some((what, now, 0));
        let text: String = log.lines.iter().map(|line| format!("{line}\n")).collect();
        if crate::elevation::ensure_folder(dir).is_ok() {
            let _ = crate::elevation::write_in_place(dir, FILE, text.as_bytes());
        }
    }
    NOTING.with(|noting| noting.set(false));
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
        // A user's name may hold spaces: it ends at the next separator or
        // quote, or with the text.
        let name = rest[after..].find(['\\', '/', '"', '\'']).unwrap_or(rest.len() - after);
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
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_latest_lines_counts_repeats_and_names_no_one() {
        let dir = std::env::temp_dir().join(format!("glance-journal-test-{}", std::process::id()));
        for i in 0..KEPT + 5 {
            note_in(&dir, format!("line {i}\nwith a break"));
        }
        // The same line again: counted, and told once another comes.
        for _ in 0..3 {
            note_in(&dir, "again");
        }
        note_in(&dir, r"could not read C:\Users\Someone Else\AppData\x and D:\Users\me/y");
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), KEPT);
        assert!(lines[KEPT - 4].ends_with(&format!("  line {} with a break", KEPT + 4)), "{}", lines[KEPT - 4]);
        assert!(lines[KEPT - 3].ends_with("  again"));
        assert!(lines[KEPT - 2].ends_with("  (the line before came 2 more times)"));
        assert!(lines[KEPT - 1].ends_with(r"could not read C:\Users\<user>\AppData\x and D:\Users\<user>/y"), "{}", lines[KEPT - 1]);
        assert_eq!(recent(2).len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
