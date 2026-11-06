//! Glance's own windows (of the kinds Glance made) watched for a thread of theirs that stops answering, as the user
//! sees it: the window greyed, "not responding" (Windows' own test, `IsHungAppWindow`: no message taken in five
//! seconds). Five seconds more of it and where each of Glance's threads stands is written down where Glance keeps its settings (`FOLDER`: the threads'
//! stacks, handles and the modules loaded, not the rest of what Glance holds; never sent anywhere), the newest few
//! kept; and the journal says which thread it was and how the PC's memory stood (a PC out of memory stalls every
//! program, Glance with it), and again as it answers once more. Nothing of Windows' is set or changed: it is a thread
//! of Glance's, gone as Glance is, and its folder goes with the settings as Glance is uninstalled.
//!
//! What the threads stood at is written by another process, Glance run for it (`ASKED`, see `run_asked`): written from
//! inside, every other thread is held still meanwhile, and one hung holding what the writing waits for (the loader's
//! lock, held by a thread stuck in a graphics driver's loading: 2026-11-06) held Glance still for good.

use std::collections::HashMap;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use windows::core::{BOOL, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, LocalFree, SetHandleInformation, HANDLE, HANDLE_FLAGS, HANDLE_FLAG_INHERIT, HLOCAL, HWND, LPARAM};
use windows::Win32::System::Diagnostics::Debug::{MiniDumpWithHandleData, MiniDumpWithThreadInfo, MiniDumpWithUnloadedModules, MiniDumpWriteDump, MINIDUMP_TYPE};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::System::SystemInformation::{GetLocalTime, GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::{GetCurrentProcessId, GetThreadDescription, OpenProcess, OpenThread, CREATE_NO_WINDOW, PROCESS_DUP_HANDLE, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ, THREAD_QUERY_LIMITED_INFORMATION};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, FindWindowExW, GetClassLongPtrW, GetClassNameW, GetWindowThreadProcessId, IsHungAppWindow, GCLP_HMODULE, HWND_MESSAGE};

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
/// What Glance is run with to write where the Glance that runs it stands (see `run_asked`), with its process's number
/// and the file's handle it was given.
pub const ASKED: &str = "--write-hang";
/// How long that is waited for, at most: past it, it is told as not written (and the writer left to end).
const WRITING_WAITS: Duration = Duration::from_secs(60);
/// What is written: the threads' stacks and what they are, the handles, the modules loaded and let go of.
const KIND: MINIDUMP_TYPE = MINIDUMP_TYPE(MiniDumpWithThreadInfo.0 | MiniDumpWithUnloadedModules.0 | MiniDumpWithHandleData.0);

/// How the process that writes is run, for the process numbered `process`, into the file whose handle (inherited) is
/// `file`: this program, asked so (see `ASKED`).
type Writer = fn(process: u32, file: usize) -> Command;

fn glance_writes(process: u32, file: usize) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap_or_default());
    command.args([ASKED.to_string(), process.to_string(), file.to_string()]);
    command
}

/// Watches Glance's windows for as long as Glance runs (a thread of its own: see the top).
pub fn watch() {
    let mut watcher = Watcher::new(crate::settings::config_dir().join(FOLDER), PAST_WINDOWS, glance_writes);
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
    writer: Writer,
    hung: HashMap<u32, Hung>,
}

impl Watcher {
    fn new(folder: PathBuf, past: Duration, writer: Writer) -> Self {
        Watcher { folder, past, writer, hung: HashMap::new() }
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
            let written = write_down(&self.folder, self.writer);
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

/// `window` put in `found` by its thread, if it is this process's and of a kind Glance made (the first of its thread's to
/// come). Not one Windows' own parts make for a thread of Glance's, as COM's for a thread that uses it: a thread that
/// waits for work leaves that unanswered, keeping nobody waiting (Glance's thread that pictures files, 2026-11-06).
fn own(window: HWND, found: &mut HashMap<u32, HWND>) {
    let mut process = 0;
    let thread = unsafe { GetWindowThreadProcessId(window, Some(&mut process)) };
    if thread != 0 && process == unsafe { GetCurrentProcessId() } && made_by_glance(window) {
        found.entry(thread).or_insert(window);
    }
}

/// Whether `window` is of a kind (a class) Glance made: one made by Glance's own program, not by a library of Windows'.
fn made_by_glance(window: HWND) -> bool {
    let Ok(program) = (unsafe { GetModuleHandleW(PCWSTR::null()) }) else { return false };
    (unsafe { GetClassLongPtrW(window, GCLP_HMODULE) }) == program.0 as usize
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
/// written before let go of but the newest `KEPT` - 1; its name. The file made as the settings are (see
/// `elevation::write_in_place`: Glance may run elevated, and follows no link in the user's folder), and written by
/// another process, run by `writer` (see the top).
fn write_down(folder: &Path, writer: Writer) -> std::io::Result<String> {
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
    crate::elevation::write_in_place_by(folder, &name, |file| written_by(writer, file))?;
    Ok(name)
}

/// `file` written by a process run by `writer`, handed the file (inherited: its handle marked so while it is run, the
/// processes run meanwhile taking it as well), and waited for.
fn written_by(writer: Writer, file: &std::fs::File) -> std::io::Result<()> {
    let handle = HANDLE(file.as_raw_handle());
    unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, HANDLE_FLAG_INHERIT) }.map_err(std::io::Error::other)?;
    let run = writer(unsafe { GetCurrentProcessId() }, handle.0 as usize)
        .creation_flags(CREATE_NO_WINDOW.0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let _ = unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0)) };
    let mut writing = run?;
    let began = Instant::now();
    loop {
        if let Some(ended) = writing.try_wait()? {
            return if ended.success() { Ok(()) } else { Err(std::io::Error::other(format!("its writer ended with {ended}"))) };
        }
        if began.elapsed() > WRITING_WAITS {
            return Err(std::io::Error::other(format!("its writer did not end in {} s", WRITING_WAITS.as_secs())));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Run as `ASKED`, with the arguments after it: writes where the threads of the process numbered by the first stand,
/// into the file whose handle (inherited from it) is the second. Only the process that ran this one, and only one of
/// this same program: no other is opened. None for arguments not so.
pub fn run_asked(arguments: &[String]) -> Option<i32> {
    let [process, file] = arguments else { return None };
    let (process, file) = (process.parse::<u32>().ok()?, file.parse::<usize>().ok()?);
    if !run_by(process) {
        return None;
    }
    let opened = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_DUP_HANDLE, false, process) }.ok()?;
    let written = unsafe { MiniDumpWriteDump(opened, process, HANDLE(file as *mut _), KIND, None, None, None) };
    let _ = unsafe { CloseHandle(opened) };
    Some(if written.is_ok() { 0 } else { 1 })
}

/// Whether this process was run by the one numbered `process`, of the same program: told from the list of processes,
/// none of them opened.
fn run_by(process: u32) -> bool {
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else { return false };
    let mut each = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut listed = HashMap::new();
    let mut more = unsafe { Process32FirstW(snapshot, &mut each) }.is_ok();
    while more {
        let length = each.szExeFile.iter().position(|&c| c == 0).unwrap_or(each.szExeFile.len());
        listed.insert(each.th32ProcessID, (each.th32ParentProcessID, String::from_utf16_lossy(&each.szExeFile[..length]).to_lowercase()));
        more = unsafe { Process32NextW(snapshot, &mut each) }.is_ok();
    }
    let _ = unsafe { CloseHandle(snapshot) };
    let own = listed.get(&unsafe { GetCurrentProcessId() });
    own.is_some_and(|(parent, program)| *parent == process && listed.get(parent).is_some_and(|(_, theirs)| theirs == program))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use windows::core::w;
    use windows::Win32::Foundation::{LRESULT, WPARAM};
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, PeekMessageW, RegisterClassW, MSG, PM_REMOVE, WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW};

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
                    let class = test_class();
                    let shown = CreateWindowExW(WINDOW_EX_STYLE(0), class, w!(""), WINDOW_STYLE(0), 0, 0, 10, 10, None, None, None, None).unwrap();
                    let messages = CreateWindowExW(WINDOW_EX_STYLE(0), class, w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, None, None).unwrap();
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
        let mut watcher = Watcher::new(folder.clone(), Duration::from_secs(1), test_writes);
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

    /// A kind of window of the tests' own (as Glance makes its own), registered once.
    fn test_class() -> PCWSTR {
        unsafe extern "system" fn procedure(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
            unsafe { DefWindowProcW(window, message, w, l) }
        }
        static REGISTERED: std::sync::Once = std::sync::Once::new();
        let class = w!("GlanceHangTest");
        REGISTERED.call_once(|| unsafe {
            let program = GetModuleHandleW(PCWSTR::null()).unwrap();
            RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(procedure), hInstance: program.into(), lpszClassName: class, ..Default::default() });
        });
        class
    }

    /// A thread of Glance's that uses COM and waits for work, taking no message meanwhile: Windows calls the window COM
    /// made for it hung, but it keeps nobody waiting, and nothing is written down. (The thread that pictures files was
    /// told as hung through a night, 2026-11-06.)
    #[test]
    fn a_thread_waiting_for_work_is_not_written_down() {
        let folder = std::env::temp_dir().join(format!("glance-hang-waiting-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let (told, window) = std::sync::mpsc::channel();
        let (stop, stopped) = std::sync::mpsc::channel::<()>();
        let waiting = std::thread::Builder::new()
            .name("hang test: waits for work".into())
            .spawn(move || unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                // COM's window for this thread, among those that only take messages.
                let (mut after, mut com) = (None, None);
                while let Ok(window) = FindWindowExW(Some(HWND_MESSAGE), after, w!("OleMainThreadWndClass"), PCWSTR::null()) {
                    if GetWindowThreadProcessId(window, None) == GetCurrentThreadId() {
                        com = Some(window.0 as isize);
                        break;
                    }
                    after = Some(window);
                }
                // What came taken once, as a picture made takes it, then work waited for.
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    DispatchMessageW(&message);
                }
                told.send(com).unwrap();
                let _ = stopped.recv_timeout(Duration::from_secs(30));
                CoUninitialize();
            })
            .unwrap();
        let com = window.recv().unwrap().map(|window| HWND(window as *mut _));
        let mut watcher = Watcher::new(folder.clone(), Duration::from_secs(1), test_writes);
        let mut said = Vec::new();
        let began = Instant::now();
        let mut hung = false;
        while com.is_some() && began.elapsed() < Duration::from_secs(12) {
            std::thread::sleep(Duration::from_millis(250));
            hung |= com.is_some_and(|com| unsafe { IsHungAppWindow(com) }.as_bool());
            watcher.look(Instant::now(), &mut |line| said.push(line));
        }
        let _ = stop.send(());
        waiting.join().unwrap();
        let _ = std::fs::remove_dir_all(&folder);
        assert!(com.is_some(), "COM made its window for the thread");
        assert!(hung, "Windows called COM's window hung");
        let ours: Vec<&String> = said.iter().filter(|line| line.contains("hang test: waits for work")).collect();
        assert!(ours.is_empty(), "{ours:?}");
    }

    /// How the tests run the process that writes: this test program, at `writes_as_asked`, told what to write.
    fn test_writes(process: u32, file: usize) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["hang::tests::writes_as_asked", "--exact", "--ignored", "--test-threads=1"]).env("GLANCE_HANG_ASKED", format!("{process} {file}"));
        command
    }

    /// The process apart that writes, as the tests run it (see `test_writes`): as Glance run with `ASKED` does.
    #[test]
    #[ignore = "run by the hang tests as the process that writes"]
    fn writes_as_asked() {
        let Ok(asked) = std::env::var("GLANCE_HANG_ASKED") else { return };
        let arguments: Vec<String> = asked.split(' ').map(str::to_string).collect();
        std::process::exit(run_asked(&arguments).unwrap_or(2));
    }

    /// Written while another thread holds the loader's lock, as one stuck in a graphics driver's loading did
    /// (2026-11-06): written still, and nothing held still for it. (Written from inside, every thread was held still
    /// and the writing waited on the lock for good.) Held in a process of its own: the other tests' threads, begun and
    /// ended meanwhile, wait on the lock, holding what they hold (as what running a process waits for) all the while.
    #[test]
    fn written_while_the_loader_is_held() {
        let ran = Command::new(std::env::current_exe().unwrap())
            .args(["hang::tests::written_while_the_loader_is_held_apart", "--exact", "--ignored", "--test-threads=1", "--nocapture"])
            .env("GLANCE_HANG_HELD", "1")
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&ran.stdout).into_owned() + &String::from_utf8_lossy(&ran.stderr);
        assert!(ran.status.success() && said.contains("1 passed"), "{said}");
    }

    /// `written_while_the_loader_is_held`, in the process run for it.
    #[test]
    #[ignore = "run by written_while_the_loader_is_held in a process of its own"]
    fn written_while_the_loader_is_held_apart() {
        if std::env::var_os("GLANCE_HANG_HELD").is_none() {
            return;
        }
        type Lock = unsafe extern "system" fn(u32, *mut u32, *mut usize) -> i32;
        type Unlock = unsafe extern "system" fn(u32, usize) -> i32;
        use windows::core::s;
        use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
        let ntdll = unsafe { GetModuleHandleW(w!("ntdll.dll")) }.unwrap();
        let lock: Lock = unsafe { std::mem::transmute(GetProcAddress(ntdll, s!("LdrLockLoaderLock")).unwrap()) };
        let unlock: Unlock = unsafe { std::mem::transmute(GetProcAddress(ntdll, s!("LdrUnlockLoaderLock")).unwrap()) };
        let folder = std::env::temp_dir().join(format!("glance-hang-held-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        // Each thread made first: a thread begins and ends under the loader's lock.
        let (go, going) = std::sync::mpsc::channel::<()>();
        let (wrote, written) = std::sync::mpsc::channel();
        let writing = std::thread::spawn({
            let folder = folder.clone();
            move || {
                going.recv().unwrap();
                let _ = wrote.send(write_down(&folder, test_writes));
            }
        });
        let (held, holding) = std::sync::mpsc::channel::<()>();
        let (free, freed) = std::sync::mpsc::channel::<()>();
        let holder = std::thread::spawn(move || unsafe {
            let mut cookie = 0usize;
            assert_eq!(lock(0, std::ptr::null_mut(), &mut cookie), 0);
            held.send(()).unwrap();
            let _ = freed.recv_timeout(Duration::from_secs(60));
            unlock(0, cookie);
        });
        holding.recv().unwrap();
        go.send(()).unwrap();
        let outcome = written.recv_timeout(Duration::from_secs(30));
        let _ = free.send(());
        holder.join().unwrap();
        writing.join().unwrap();
        let name = outcome.expect("written while the loader's lock is held").unwrap();
        assert_eq!(&std::fs::read(folder.join(&name)).unwrap()[..4], b"MDMP");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// No other process than the one that ran it is written: not one of another program, not one that did not run it.
    #[test]
    fn writes_none_but_its_own() {
        assert!(!run_by(0));
        assert!(!run_by(4));
        assert_eq!(run_asked(&["4".into(), "0".into()]), None);
        assert_eq!(run_asked(&["x".into(), "0".into()]), None);
        assert_eq!(run_asked(&[]), None);
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
        let name = write_down(&folder, test_writes).unwrap();
        let mut left: Vec<String> = std::fs::read_dir(&folder).unwrap().flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, vec!["hang-20260102-000000.dmp".to_string(), "hang-20260103-000000.dmp".into(), name, "notes.txt".into()]);
        let _ = std::fs::remove_dir_all(&folder);
    }
}
