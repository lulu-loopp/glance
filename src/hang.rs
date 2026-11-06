//! Glance's own windows watched for a thread of theirs that stops answering, as the user sees it: the window greyed,
//! "not responding" (Windows' own test, `IsHungAppWindow`: no message taken in five seconds). Five seconds more of it
//! and where each of Glance's threads stands is written down where Glance keeps its settings (`FOLDER`: the threads'
//! stacks, handles and the modules loaded, not the rest of what Glance holds; never sent anywhere), the newest few
//! kept; and the journal says which thread it was and how the PC's memory stood (a PC out of memory stalls every
//! program, Glance with it), and again as it answers once more. Nothing of Windows' is set or changed: it is a thread
//! of Glance's, gone as Glance is, and its folder goes with the settings as Glance is uninstalled.

use std::collections::HashMap;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use windows::core::{BOOL, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL, HWND, LPARAM};
use windows::Win32::System::Diagnostics::Debug::{MiniDumpWithHandleData, MiniDumpWithThreadInfo, MiniDumpWithUnloadedModules, MiniDumpWriteDump, MINIDUMP_TYPE};
use windows::Win32::System::SystemInformation::{GetLocalTime, GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, GetThreadDescription, OpenThread, THREAD_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, FindWindowExW, GetClassNameW, GetWindowThreadProcessId, IsHungAppWindow, HWND_MESSAGE};

/// Where what the threads stood at is written, in the settings' folder.
pub const FOLDER: &str = "hangs";
/// How many are kept: the newest.
const KEPT: usize = 3;
/// How long Windows waits before it calls a window's thread hung (see `IsHungAppWindow`).
const WINDOWS_WAITS: Duration = Duration::from_secs(5);
/// How long after that it is written down: a thread busy a moment past Windows' wait is not one hung.
const PAST_WINDOWS: Duration = Duration::from_secs(5);
/// How often Glance's windows are looked at.
const EVERY: Duration = Duration::from_secs(1);

/// Watches Glance's windows for as long as Glance runs (a thread of its own: see the top).
pub fn watch() {
    let mut watcher = Watcher::new(crate::settings::config_dir().join(FOLDER), PAST_WINDOWS);
    loop {
        std::thread::sleep(EVERY);
        watcher.look(Instant::now(), &mut |line| crate::journal::note(line));
    }
}

/// A thread found hung: since when it was seen so, what it is called, one of its windows' class, and whether it was
/// written down.
struct Hung {
    since: Instant,
    name: String,
    class: String,
    written: bool,
}

struct Watcher {
    folder: PathBuf,
    past: Duration,
    hung: HashMap<u32, Hung>,
}

impl Watcher {
    fn new(folder: PathBuf, past: Duration) -> Self {
        Watcher { folder, past, hung: HashMap::new() }
    }

    /// Looks at each thread of Glance's that has a window, as of `now`: one hung `past` longer than Windows waits is
    /// written down (what every thread stood at, then a line by `say`), once; one that answers again after it was is
    /// told by `say` too.
    fn look(&mut self, now: Instant, say: &mut dyn FnMut(String)) {
        let hung_now: HashMap<u32, HWND> = own_threads().into_iter().filter(|(_, window)| unsafe { IsHungAppWindow(*window) }.as_bool()).collect();
        self.hung.retain(|thread, hung| {
            let still = hung_now.contains_key(thread);
            if !still && hung.written {
                say(format!("hang: thread {} answers again, {} s after it stopped", hung.name, (now.duration_since(hung.since) + WINDOWS_WAITS).as_secs()));
            }
            still
        });
        for (thread, window) in hung_now {
            let hung = self.hung.entry(thread).or_insert_with(|| Hung { since: now, name: thread_name(thread), class: class_of(window), written: false });
            if hung.written || now.duration_since(hung.since) < self.past {
                continue;
            }
            hung.written = true;
            let memory = memory();
            // Written before anything is said: the thread hung may hold what saying a line waits for.
            let written = write_down(&self.folder);
            let over = (now.duration_since(hung.since) + WINDOWS_WAITS).as_secs();
            let what = format!("hang: thread {} (a window of class {}) has answered nothing for {over} s ({memory})", hung.name, hung.class);
            say(match written {
                Ok(name) => format!("{what}: where Glance's threads stood written as {FOLDER}\\{name}"),
                Err(error) => format!("{what}: where Glance's threads stood could not be written ({error})"),
            });
        }
    }
}

/// Each thread of this process's that has a window, with one of its windows: those listed at the top, and those that
/// only take messages (see `HWND_MESSAGE`, not listed with them).
fn own_threads() -> HashMap<u32, HWND> {
    unsafe extern "system" fn each(window: HWND, found: LPARAM) -> BOOL {
        own(window, unsafe { &mut *(found.0 as *mut HashMap<u32, HWND>) });
        true.into()
    }
    let mut found = HashMap::new();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut found as *mut HashMap<u32, HWND> as isize));
        let mut after = None;
        while let Ok(window) = FindWindowExW(Some(HWND_MESSAGE), after, PCWSTR::null(), PCWSTR::null()) {
            own(window, &mut found);
            after = Some(window);
        }
    }
    found
}

/// `window` put in `found` by its thread, if it is this process's (the first of its thread's to come).
fn own(window: HWND, found: &mut HashMap<u32, HWND>) {
    let mut process = 0;
    let thread = unsafe { GetWindowThreadProcessId(window, Some(&mut process)) };
    if thread != 0 && process == unsafe { GetCurrentProcessId() } {
        found.entry(thread).or_insert(window);
    }
}

/// A thread by the name it was given (Glance names its threads), quoted, and its number; its number alone without.
fn thread_name(thread: u32) -> String {
    let named = unsafe {
        OpenThread(THREAD_QUERY_LIMITED_INFORMATION, false, thread).ok().and_then(|handle| {
            let name = GetThreadDescription(handle).ok().map(|text| {
                let name = text.to_string().unwrap_or_default();
                LocalFree(Some(HLOCAL(text.0.cast())));
                name
            });
            let _ = CloseHandle(handle);
            name
        })
    };
    match named.filter(|name| !name.is_empty()) {
        Some(name) => format!("\"{name}\" ({thread})"),
        None => thread.to_string(),
    }
}

/// The class `window` was made of (the kind of window, never what it shows).
fn class_of(window: HWND) -> String {
    let mut class = [0u16; 128];
    let length = unsafe { GetClassNameW(window, &mut class) }.max(0) as usize;
    String::from_utf16_lossy(&class[..length])
}

/// How the PC's memory stands: the physical free, and what is committed of what may be (memory and the paging file).
fn memory() -> String {
    let mut status = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    if unsafe { GlobalMemoryStatusEx(&mut status) }.is_err() {
        return "memory not known".into();
    }
    let gb = |bytes: u64| bytes as f64 / (1u64 << 30) as f64;
    format!(
        "memory: {:.1} of {:.1} GB free, {:.1} of {:.1} GB committed",
        gb(status.ullAvailPhys),
        gb(status.ullTotalPhys),
        gb(status.ullTotalPageFile - status.ullAvailPageFile),
        gb(status.ullTotalPageFile)
    )
}

/// Where each of Glance's threads stands, written in `folder` (made if need be) as a file named by the time, those
/// written before let go of but the newest `KEPT` - 1; its name. Written as the settings are (see
/// `elevation::write_in_place`): Glance may run elevated, and follows no link in the user's folder.
fn write_down(folder: &Path) -> std::io::Result<String> {
    crate::elevation::ensure_folder(folder)?;
    let mut before: Vec<String> = std::fs::read_dir(folder)?
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("hang-") && name.ends_with(".dmp"))
        .collect();
    before.sort();
    while before.len() >= KEPT {
        crate::elevation::remove_in_place(folder, &before.remove(0));
    }
    let time = unsafe { GetLocalTime() };
    let name = format!("hang-{:04}{:02}{:02}-{:02}{:02}{:02}.dmp", time.wYear, time.wMonth, time.wDay, time.wHour, time.wMinute, time.wSecond);
    let kind = MINIDUMP_TYPE(MiniDumpWithThreadInfo.0 | MiniDumpWithUnloadedModules.0 | MiniDumpWithHandleData.0);
    crate::elevation::write_in_place_by(folder, &name, |file| {
        unsafe { MiniDumpWriteDump(GetCurrentProcess(), GetCurrentProcessId(), HANDLE(file.as_raw_handle()), kind, None, None, None) }.map_err(std::io::Error::other)
    })?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, DispatchMessageW, PeekMessageW, MSG, PM_REMOVE, WINDOW_EX_STYLE, WINDOW_STYLE};

    /// A thread of Glance's with a window, one shown at the top and one that only takes messages, that stops taking
    /// them a while: it is written down once, by its name, with how memory stood, what every thread stood at in a
    /// file of its own (a minidump); and told again as it answers. Windows calls both its windows hung meanwhile.
    #[test]
    fn a_thread_that_stops_answering_is_written_down() {
        let folder = std::env::temp_dir().join(format!("glance-hang-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let stall = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let (told, windows) = std::sync::mpsc::channel();
        let stalled = std::thread::Builder::new()
            .name("hang test: stops answering".into())
            .spawn({
                let (stall, done) = (stall.clone(), done.clone());
                move || unsafe {
                    let shown = CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WINDOW_STYLE(0), 0, 0, 10, 10, None, None, None, None).unwrap();
                    let messages = CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, None, None).unwrap();
                    told.send((shown.0 as isize, messages.0 as isize)).unwrap();
                    let mut message = MSG::default();
                    while !done.load(Ordering::Acquire) {
                        while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                            DispatchMessageW(&message);
                        }
                        if stall.swap(false, Ordering::AcqRel) {
                            std::thread::sleep(Duration::from_secs(9));
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    let _ = DestroyWindow(messages);
                    let _ = DestroyWindow(shown);
                }
            })
            .unwrap();
        let (shown, messages) = windows.recv().unwrap();
        let (shown, messages) = (HWND(shown as *mut _), HWND(messages as *mut _));
        let mut watcher = Watcher::new(folder.clone(), Duration::from_secs(1));
        let mut said = Vec::new();
        let began = Instant::now();
        stall.store(true, Ordering::Release);
        let (mut both_hung, mut answered) = (false, false);
        while began.elapsed() < Duration::from_secs(20) && !answered {
            std::thread::sleep(Duration::from_millis(250));
            both_hung |= unsafe { IsHungAppWindow(shown).as_bool() && IsHungAppWindow(messages).as_bool() };
            watcher.look(Instant::now(), &mut |line| said.push(line));
            answered = said.iter().any(|line| line.contains("hang test: stops answering") && line.contains("answers again"));
        }
        done.store(true, Ordering::Release);
        stalled.join().unwrap();
        let ours: Vec<&String> = said.iter().filter(|line| line.contains("hang test: stops answering")).collect();
        assert!(both_hung, "Windows called both its windows hung");
        assert_eq!(ours.len(), 2, "{said:?}");
        assert!(ours[0].contains("has answered nothing for") && ours[0].contains("GB free") && ours[0].contains("written as hangs\\hang-"), "{}", ours[0]);
        assert!(ours[1].contains("answers again"), "{}", ours[1]);
        let name = ours[0].rsplit('\\').next().unwrap();
        let written = std::fs::read(folder.join(name)).unwrap();
        assert_eq!(&written[..4], b"MDMP");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// No more are kept than `KEPT`, the oldest let go of first.
    #[test]
    fn keeps_the_newest_few() {
        let folder = std::env::temp_dir().join(format!("glance-hang-kept-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        for old in ["hang-20260101-000000.dmp", "hang-20260102-000000.dmp", "hang-20260103-000000.dmp", "notes.txt"] {
            std::fs::write(folder.join(old), b"").unwrap();
        }
        let name = write_down(&folder).unwrap();
        let mut left: Vec<String> = std::fs::read_dir(&folder).unwrap().flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, vec!["hang-20260102-000000.dmp".to_string(), "hang-20260103-000000.dmp".into(), name, "notes.txt".into()]);
        let _ = std::fs::remove_dir_all(&folder);
    }
}
