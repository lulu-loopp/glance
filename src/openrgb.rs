//! OpenRGB with an administrator's rights, for the lights reached only over the board's SMBus (the memory's): run by
//! the sensors, as the user asked it to be. The phone remote, which runs as the user, then asks that one as it would
//! its own.
//!
//! What runs is the release Glance names, file by file: each is read from the user's folder (where the remote fetched
//! it), checked, and written to a folder of administrators' alone, made new each time. It keeps its configuration
//! there too (where it would look for plugins), hears this machine alone, and takes nothing from the user's
//! environment that would have Qt load another's code.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::Duration;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Registry::{RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegGetValueW, HKEY, HKEY_LOCAL_MACHINE, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE, RRF_RT_REG_DWORD};

/// OpenRGB 1.0 for 64-bit Windows, as the remote unpacks it (see `fetch` there): each file and its SHA-256.
const RELEASE: [(&str, &str); 22] = [
    ("LedsValve.bin", "1c1809281ae6f3e3c5b362f1536c0e6706013e6e33bfba66cad74636e61436d6"),
    ("LpcIO.bin", "b3896a1cab0d808fca31fe2ebcae045d59dac690da87b17c858bb8da357eb45e"),
    ("OpenRGB.exe", "86a88b99f60a086e13e6f6ecfb8260a94dbcd598ddeabffda7070d78c97d7e95"),
    ("PawnIOLib.dll", "c6e7c2f54643ef9cb51b9a3ec7b0efc9e204803c206f33014e39f281546ea9b4"),
    ("Qt6Core.dll", "8632f4a9f57355680f1b0de5013eb4657b1038b0cedc7c0534db827f73162014"),
    ("Qt6Core5Compat.dll", "0d949a077144cfe2fecd4a030a3a6fbf0d651501e1b39bbdaaa11f56c74c25a2"),
    ("Qt6Gui.dll", "1c6ab7a785ade7a8964f67cce21e6099b1209301498726019189ea0023e6f24c"),
    ("Qt6OpenGL.dll", "bfb1be764c0e9d75682319b14d8ba0493d89a52d1680e1cc914525ea1ad944fb"),
    ("Qt6Widgets.dll", "baf97dfdb7fe72a14e1675f37b21cc8634c7ec26cc9082f4883aa7492292a1db"),
    ("SmbusI801.bin", "a0f7d066e7efda28c0e754c1f52dbd8dc280d388ba8575b514a80cb67490530c"),
    ("SmbusIntelSkylakeIMC.bin", "2e2d6c1e541fdfe598aa8df4d334e11c491c8a84f23e8256dd18119dd420dbd2"),
    ("SmbusNCT6793.bin", "db068c6a87c0066ebfb7f691d88422fee8af350898e7a7f0db100fd6632bab17"),
    ("SmbusPIIX4.bin", "91f9b4b1c39e3d399ce48477a89d8f6bd3e58a2241064a4daec3a1513dff56e5"),
    ("dxcompiler.dll", "6ec587d1952778a41a5a90647b39d0c86958e7dabf87fa1cf2b14408bfe1e8de"),
    ("dxil.dll", "a0cfaf3e930894e9b5d48efcacaed6ace5c645bad3c4e99b4a23c9831a078120"),
    ("hidapi-hotplug.dll", "263a662b014a7082871bb3e46f4d3c4790d7ed253768dcd585264fd598892dfb"),
    ("imageformats/qgif.dll", "9cb2a7645576e72db54f957e9431b6145daf55bb83f4879cc01f864d4af96621"),
    ("imageformats/qico.dll", "41038802daf3a01d4701f88e2c7730c41a9e7e63fa8c985e2ef4a9a9370b2d6e"),
    ("imageformats/qjpeg.dll", "c6d100f40b051a1750cda26c46b66694b8854eae01de45a0d4510b779b5ea88d"),
    ("libusb-1.0.dll", "a8c91f0ff68fb7802a9f4416728f0eeb4d99af4ceaa4ef7dfe9374e76e375018"),
    ("platforms/qwindows.dll", "d858a41e1617052f113442ea755696bc6dbe8030e64266e3ff3dc27fddea6bef"),
    ("styles/qmodernwindowsstyle.dll", "518118bbe1eba484b431a5eff1a212a1ffe8b92305db18a3476a6a6217a78c9f"),
];
/// The folders it runs from, in ProgramData, each named after this and made new.
const FOLDERS: &str = "Glance-OpenRGB-";
/// The folder in those it keeps its configuration in.
const CONFIGURATION: &str = "config";
/// Where it is kept that the user wants it (which only administrators can write).
const KEY: PCWSTR = w!(r"SOFTWARE\Glance");
const WANTED: PCWSTR = w!("OpenRGBRaised");
/// The makers' programs that take the lights for themselves as they start (the remote's `lights::RIVALS`): one
/// started or gone, OpenRGB is started afresh, as the remote does its own.
const RIVALS: [&str; 1] = ["GCC.exe"];
/// How often they are looked for.
const WATCH: Duration = Duration::from_secs(2);
/// How long after it is started it is seen still to be running.
const STAYS_WITHIN: Duration = Duration::from_secs(1);
/// How often it is seen whether it ended by itself, to be started again: seldom, for one that cannot run.
const ENDED_LOOKED_FOR: Duration = Duration::from_secs(60);
/// How long it is waited for as it is ended.
const ENDED_WITHIN: Duration = Duration::from_secs(5);

struct Running {
    program: Child,
    folder: crate::elevation::Private,
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);

/// A job its program runs in, so that it goes with the sensors however they go (as a number, the handle being the
/// process's own: it is closed only as the process ends, ending what is in it).
static JOB: std::sync::OnceLock<isize> = std::sync::OnceLock::new();

/// Whether it is running.
pub fn on() -> bool {
    RUNNING.lock().unwrap().as_mut().is_some_and(|running| matches!(running.program.try_wait(), Ok(None)))
}

/// Runs it or ends it, as the user asked, and keeps that for the sensors' next start: whether it is running then.
pub fn turn(on: bool) -> bool {
    if on {
        let started = start();
        keep_wanted(started);
        started
    } else {
        stop();
        keep_wanted(false);
        false
    }
}

/// For the sensors, as they start: runs it if that is wanted, and from then on starts it afresh after a maker's
/// program has taken the lights. On a thread of its own.
pub fn watch() {
    std::thread::Builder::new()
        .name("glance sensors: lights' program".into())
        .spawn(|| {
            if wanted() && !start() {
                crate::journal::note("OpenRGB, wanted with an administrator's rights, did not start");
            }
            let mut rivals = rivals_running();
            let mut looked_for_it = std::time::Instant::now();
            loop {
                std::thread::sleep(WATCH);
                let now = rivals_running();
                if now != rivals {
                    rivals = now;
                    afresh();
                } else if looked_for_it.elapsed() >= ENDED_LOOKED_FOR {
                    looked_for_it = std::time::Instant::now();
                    if ended_by_itself() {
                        afresh();
                    }
                }
            }
        })
        .expect("a thread started");
}

/// Whether it was run and has ended, not having been asked to.
fn ended_by_itself() -> bool {
    RUNNING.lock().unwrap().as_mut().is_some_and(|running| !matches!(running.program.try_wait(), Ok(None)))
}

/// Ended, and started again from the folder it ran from, if it was run.
fn afresh() {
    let mut running = RUNNING.lock().unwrap();
    let Some(mut was) = running.take() else { return };
    end(&mut was.program);
    match run_from(&was.folder) {
        Ok(program) => *running = Some(Running { program, folder: was.folder }),
        Err(error) => crate::journal::note(format!("OpenRGB did not start again: {error}")),
    }
}

fn stop() {
    if let Some(mut running) = RUNNING.lock().unwrap().take() {
        end(&mut running.program);
    }
    forget_folders();
}

/// Takes all that is kept of it away: for the uninstaller (the sensors, gone by then, took it with them).
pub fn forget() {
    keep_wanted(false);
    forget_folders();
}

fn forget_folders() {
    if let Some(program_data) = crate::update::program_data() {
        crate::elevation::remove_private_folders(&program_data, FOLDERS);
    }
}

fn start() -> bool {
    let mut running = RUNNING.lock().unwrap();
    if running.as_mut().is_some_and(|running| matches!(running.program.try_wait(), Ok(None))) {
        return true;
    }
    *running = None;
    match started() {
        Ok(now) => {
            *running = Some(now);
            true
        }
        Err(error) => {
            crate::journal::note(format!("OpenRGB with an administrator's rights: {error}"));
            false
        }
    }
}

fn started() -> Result<Running, String> {
    let program_data = crate::update::program_data().ok_or("no ProgramData")?;
    // (What an earlier run left, gone: it ran from a folder of its own.)
    crate::elevation::remove_private_folders(&program_data, FOLDERS);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_nanos());
    let name = format!("{FOLDERS}{}-{stamp}", std::process::id());
    let folder = crate::elevation::private_folder(&program_data, &name).ok_or("no folder of administrators' alone")?;
    put_in(&RELEASE, &fetched().ok_or("no folder of the user's")?, folder.path())?;
    let program = run_from(&folder)?;
    Ok(Running { program, folder })
}

/// Where the remote unpacked the release, for this user.
fn fetched() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("dev.weiyi.glance.remote").join("OpenRGB").join("OpenRGB Windows 64-bit"))
}

/// All it needs put in `to`.
fn put_in(files: &[(&str, &str)], from: &Path, to: &Path) -> Result<(), String> {
    copy_checked(files, from, to)?;
    std::fs::create_dir_all(to.join(CONFIGURATION)).map_err(|error| error.to_string())
}

/// Each of `files` read from `from` and written under `to`, once it is known to be what it should be: what is checked
/// is what is written, and nothing runs till all of them are there.
fn copy_checked(files: &[(&str, &str)], from: &Path, to: &Path) -> Result<(), String> {
    for (name, digest) in files {
        let bytes = std::fs::read(from.join(name)).map_err(|error| format!("{name}: {error}"))?;
        if !is(&bytes, digest) {
            return Err(format!("{name} is not the release's"));
        }
        let path = to.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| format!("{name}: {error}"))?;
        }
        std::fs::write(&path, &bytes).map_err(|error| format!("{name}: {error}"))?;
    }
    Ok(())
}

fn is(bytes: &[u8], digest: &str) -> bool {
    use windows::Win32::Security::Cryptography::{BCryptHash, BCRYPT_SHA256_ALG_HANDLE};
    let mut made = [0u8; 32];
    unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, bytes, &mut made) }.is_ok() && made.iter().map(|byte| format!("{byte:02x}")).collect::<String>() == digest
}

/// What it is started with: as a server alone, hearing this machine alone, its configuration in `folder`.
fn arguments(folder: &Path) -> Vec<std::ffi::OsString> {
    ["--server", "--server-host", "127.0.0.1", "--noautoconnect", "--config"].iter().map(Into::into).chain([folder.join(CONFIGURATION).into_os_string()]).collect()
}

/// Whether Qt reads a variable of this name (the user's environment is the sensors' too, and anything of the user's
/// may set it: `QT_PLUGIN_PATH` would have it load what is there).
fn for_qt(name: &std::ffi::OsStr) -> bool {
    name.to_string_lossy().to_ascii_uppercase().starts_with("QT")
}

fn run_from(folder: &crate::elevation::Private) -> Result<Child, String> {
    use std::os::windows::process::CommandExt;
    /// CREATE_NO_WINDOW: no console for a program started without one.
    const NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new(folder.path().join("OpenRGB.exe"));
    command.args(arguments(folder.path())).current_dir(folder.path()).creation_flags(NO_WINDOW);
    for (name, _) in std::env::vars_os().filter(|(name, _)| for_qt(name)) {
        command.env_remove(name);
    }
    let mut program = command.spawn().map_err(|error| error.to_string())?;
    in_the_job(&program);
    // (What it does not like it says by ending at once, and by nothing else.)
    std::thread::sleep(STAYS_WITHIN);
    match program.try_wait() {
        Ok(None) => Ok(program),
        Ok(Some(status)) => Err(format!("it ended as it started ({status})")),
        Err(error) => Err(error.to_string()),
    }
}

fn in_the_job(program: &Child) {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE};
    let job = JOB.get_or_init(|| unsafe {
        let Ok(job) = CreateJobObjectW(None, PCWSTR::null()) else { return 0 };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(job, JobObjectExtendedLimitInformation, (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(), size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32).is_err() {
            let _ = CloseHandle(job);
            return 0;
        }
        job.0 as isize
    });
    if *job != 0 {
        let _ = unsafe { AssignProcessToJobObject(HANDLE(*job as *mut _), HANDLE(program.as_raw_handle())) };
    }
}

fn end(program: &mut Child) {
    let _ = program.kill();
    let since = std::time::Instant::now();
    while since.elapsed() < ENDED_WITHIN && matches!(program.try_wait(), Ok(None)) {
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The makers' programs running, by their numbers.
fn rivals_running() -> Vec<u32> {
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    let mut found = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return found };
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
        while more {
            let length = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..length]);
            if RIVALS.iter().any(|rival| rival.eq_ignore_ascii_case(&name)) {
                found.push(entry.th32ProcessID);
            }
            more = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
    }
    found.sort_unstable();
    found
}

fn wanted() -> bool {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, KEY, WANTED, RRF_RT_REG_DWORD, None, Some((&mut value as *mut u32).cast()), Some(&mut size)) }.is_ok() && value != 0
}

fn keep_wanted(on: bool) {
    let mut key = HKEY::default();
    if unsafe { RegCreateKeyExW(HKEY_LOCAL_MACHINE, KEY, None, None, REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None) }.is_err() {
        return;
    }
    unsafe {
        if on {
            let _ = windows::Win32::System::Registry::RegSetValueExW(key, WANTED, None, REG_DWORD, Some(&1u32.to_le_bytes()));
        } else {
            let _ = RegDeleteValueW(key, WANTED);
        }
        let _ = RegCloseKey(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(test: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("glance-openrgb-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    // SHA-256 of "first" and of "second".
    const FIRST: &str = "a7937b64b8caa58f03721bb6bacf5c78cb235febe0e70b1b84cd99541461a08e";
    const SECOND: &str = "16367aacb67a4a017c8da8ab95682ccb390863780f7114dda0a0e0c55644c7c4";

    #[test]
    fn writes_only_what_is_the_release() {
        let from = folder("from");
        let to = folder("to");
        std::fs::create_dir_all(from.join("inner")).unwrap();
        std::fs::write(from.join("a.bin"), "first").unwrap();
        std::fs::write(from.join("inner/b.bin"), "second").unwrap();
        copy_checked(&[("a.bin", FIRST), ("inner/b.bin", SECOND)], &from, &to).unwrap();
        assert_eq!(std::fs::read_to_string(to.join("inner/b.bin")).unwrap(), "second");
        // Changed in the user's folder, it is refused, and what comes after it is not written.
        let other = folder("other");
        std::fs::write(from.join("a.bin"), "changed").unwrap();
        assert!(copy_checked(&[("a.bin", FIRST), ("inner/b.bin", SECOND)], &from, &other).is_err());
        assert!(!other.join("a.bin").exists() && !other.join("inner").exists());
        // One that is not there, the same.
        assert!(copy_checked(&[("gone.bin", FIRST)], &from, &other).is_err());
        for path in [from, to, other] {
            let _ = std::fs::remove_dir_all(path);
        }
    }

    /// Told of a folder for its configuration that is not there, the program ends at once, saying nothing.
    #[test]
    fn its_configuration_has_a_folder_before_it_runs() {
        let from = folder("put-from");
        let to = folder("put-to");
        std::fs::write(from.join("a.bin"), "first").unwrap();
        put_in(&[("a.bin", FIRST)], &from, &to).unwrap();
        assert!(to.join("config").is_dir());
        for path in [from, to] {
            let _ = std::fs::remove_dir_all(path);
        }
    }

    #[test]
    fn hears_this_machine_alone_and_keeps_its_configuration_with_it() {
        let said: Vec<String> = arguments(Path::new(r"C:\Kept")).iter().map(|argument| argument.to_string_lossy().into_owned()).collect();
        let host = said.iter().position(|argument| argument == "--server-host").unwrap();
        assert_eq!(said[host + 1], "127.0.0.1");
        let config = said.iter().position(|argument| argument == "--config").unwrap();
        assert_eq!(said[config + 1], r"C:\Kept\config");
    }

    #[test]
    fn takes_nothing_of_qt_from_the_users_environment() {
        for name in ["QT_PLUGIN_PATH", "QT_QPA_PLATFORM_PLUGIN_PATH", "qt_debug_plugins", "QML2_IMPORT_PATH"] {
            assert_eq!(for_qt(std::ffi::OsStr::new(name)), name != "QML2_IMPORT_PATH", "{name}");
        }
        assert!(!for_qt(std::ffi::OsStr::new("PATH")));
    }
}
