//! Glance's log: what went wrong, and a few things worth knowing later
//! (an update found or refused), a line each with the local time, kept in
//! the settings folder beside settings.json. Only the latest lines are kept.
//! Written as the settings are (see `elevation::write_in_place`): Glance
//! runs elevated and never follows a link someone put in the user's folder.
//! Nothing that names the user goes in: no paths.

use std::collections::VecDeque;
use std::fmt::Display;
use std::sync::Mutex;

use windows::Win32::System::SystemInformation::GetLocalTime;

/// The log's file, in the settings folder.
pub const FILE: &str = "glance.log";
/// How many lines are kept.
const KEPT: usize = 200;

/// The lines so far, read from the file at first use.
static LINES: Mutex<Option<VecDeque<String>>> = Mutex::new(None);

/// Adds a line to the log, and keeps the log.
pub fn note(what: impl Display) {
    note_in(&crate::settings::config_dir(), what);
}

/// `note`, with the log kept in `dir`.
fn note_in(dir: &std::path::Path, what: impl Display) {
    let time = unsafe { GetLocalTime() };
    let line = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}  {what}",
        time.wYear, time.wMonth, time.wDay, time.wHour, time.wMinute, time.wSecond
    );
    // A note while a note is being kept (a crash inside it) is let go.
    let Ok(mut lines) = LINES.try_lock() else { return };
    let lines = lines.get_or_insert_with(|| read(dir));
    lines.push_back(line.replace(['\r', '\n'], " "));
    while lines.len() > KEPT {
        lines.pop_front();
    }
    let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
    if crate::elevation::ensure_folder(dir).is_ok() {
        let _ = crate::elevation::write_in_place(dir, FILE, text.as_bytes());
    }
}

/// The latest `count` lines, oldest first.
pub fn recent(count: usize) -> Vec<String> {
    let mut lines = LINES.lock().unwrap();
    let lines = lines.get_or_insert_with(|| read(&crate::settings::config_dir()));
    lines.iter().skip(lines.len().saturating_sub(count)).cloned().collect()
}

fn read(dir: &std::path::Path) -> VecDeque<String> {
    crate::elevation::read_in_place(dir, FILE).map_or_else(VecDeque::new, |text| text.lines().map(String::from).collect())
}

/// Notes a crash before the process ends (release builds abort on panic).
pub fn note_crashes() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        note(format!("crashed: {info}"));
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_latest_lines_one_a_line() {
        let dir = std::env::temp_dir().join(format!("glance-journal-test-{}", std::process::id()));
        for i in 0..KEPT + 5 {
            note_in(&dir, format!("line {i}\nwith a break"));
        }
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), KEPT);
        assert!(lines[0].ends_with("  line 5 with a break"), "{}", lines[0]);
        assert!(lines[KEPT - 1].ends_with(&format!("  line {} with a break", KEPT + 4)));
        assert_eq!(recent(2).len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
