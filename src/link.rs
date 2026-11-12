//! Glance in two processes: the sensors, read with administrator rights by a
//! program of their own (`glance-sensors.exe`, started elevated by Glance's
//! task), and all the
//! rest, running as the user. They speak over two named pipes, one each way,
//! that only the user (and the system) may open, and that the sensors make:
//! the sensors tell what they read and what is presented, and are asked only
//! a few things that need their rights, each of a fixed kind with its fields
//! checked. The rest checks, before it believes a word, that what answers is
//! the sensors' program beside it, elevated; the sensors, that what asks is
//! Glance beside them.

use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL, INVALID_HANDLE_VALUE};
use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_SHARE_NONE, OPEN_EXISTING, PIPE_ACCESS_INBOUND, PIPE_ACCESS_OUTBOUND};
use windows::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeServerProcessId, WaitNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT};
use windows::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW, WaitForSingleObject, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE};

use crate::presents::GameMode;
use crate::reading::{Sample, StaticInfo};

/// What the sensors tell.
#[derive(Serialize, Deserialize)]
pub enum Told {
    /// As they start: their version, what the machine is, and what they found to read it with (for a report).
    Hello { version: String, info: StaticInfo, found: Vec<String> },
    /// What they found to read the machine with, as it changes after hello (the GPUs' makers' libraries are looked up
    /// apart from their first look at the hardware, and what came of it is told once it is known).
    Found(Vec<String>),
    /// What they read, each time they read.
    Sample(Box<Sample>),
    /// The games presenting frames, as they are seen now, and whether a game shows (as set by hand, or as it
    /// came back to by itself).
    Presenting { games: Vec<crate::presents::Presenting>, mode: GameModeSent },
    /// The pointer pushed into `edge` (an edge that opens the panel, as asked: see `Asked::Edges`) at `at`
    /// (physical px). They hear the mouse whatever window is in front; the rest, running as the user, does not
    /// over an elevated one.
    Pushed { edge: crate::settings::Edge, at: (i32, i32) },
    /// What they have to say for a report, asked for.
    Report(Vec<String>),
    /// What became of a switch asked for (or how it is, as they start): whether it is on now.
    Switched { switch: Switch, on: bool },
    /// Where an update asked for stands.
    Updating(UpdateState),
    /// A line for Glance's log (the sensors' process writes none of its own).
    Note(String),
    /// The mouse's buttons, as they hear them whatever window is in front (the rest, running as the user, reads
    /// them as up throughout over an elevated one): whether any is down now, and whether one was pressed since
    /// they last said (a press let go again before they said is told so). Said as the rest comes, and as they
    /// change.
    Buttons { down: bool, pressed: bool },
    /// What touches the PC, while the rest asks (see `Asked::Activity`): the last touch, and the last report that
    /// said nothing was moved or pressed, each by the device it came from (at most `DEVICE_MOST` characters) and
    /// when (`GetTickCount`); none, nothing heard yet.
    Activity { touched: Option<(String, u32)>, nothing: Option<(String, u32)> },
}

/// The most of a device's name told in `Told::Activity`.
pub const DEVICE_MOST: usize = 256;

/// The switches that need the sensors' rights to be turned: Glance starting at sign-in (its task was made
/// elevated), the phone remote (its task, and its way through the firewall), and OpenRGB run with those rights.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Switch {
    Autostart,
    Remote,
    RaisedLights,
    /// Nearby transfer (the same task as the phone remote's, and a way of its own through the firewall).
    Nearby,
}

/// Where taking an update stands, as it travels.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum UpdateState {
    Ready,
    Downloading,
    Failed,
    Rejected,
}

/// A release to update to: its version, and each site's version and installer (the sensors take an installer
/// only from Glance's own site, GitHub or Gitee, and run it only if it is signed as Glance is and is that newer
/// version).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReleaseSent {
    pub version: String,
    pub installers: Vec<(String, String)>,
}

/// What the sensors are asked.
#[derive(Serialize, Deserialize)]
pub enum Asked {
    /// The first thing the rest says, as it comes: its version (the sensors serve only their own version's rest:
    /// another's makes them go, for the rest to start its own).
    Hello { version: String },
    /// Whether to hear what touches the PC (for the phone remote, while it has the screens out), and tell it.
    Activity(bool),
    /// How often to read, and what besides the plain readings.
    Want(Want),
    /// Whether a game shows, as the user set it.
    GameMode(GameModeSent),
    /// What they have to say for a report.
    Report,
    /// A switch turned on or off.
    Switch { switch: Switch, on: bool },
    /// The release found newer, downloaded and installed.
    Install(ReleaseSent),
    /// Which edges open the panel to a push, and how hard (as the settings have them; told again as they change).
    Edges(crate::push::Edges),
}

/// How often the sensors read (milliseconds), whether what changes slowly is read afresh each time (something
/// shows it), and whether WSL's and Docker's are read.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Want {
    pub interval_ms: u32,
    pub refresh_slow: bool,
    pub wsl: bool,
    pub docker: bool,
}

impl Want {
    /// Within what the settings offer (a sample from a quarter of a second to ten seconds apart).
    pub fn checked(self) -> Self {
        Want { interval_ms: self.interval_ms.clamp(250, 10_000), ..self }
    }
}

/// A game mode as it travels.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum GameModeSent {
    Auto,
    On,
    Off,
}

impl From<GameMode> for GameModeSent {
    fn from(mode: GameMode) -> Self {
        match mode {
            GameMode::Auto => GameModeSent::Auto,
            GameMode::On => GameModeSent::On,
            GameMode::Off => GameModeSent::Off,
        }
    }
}

impl From<GameModeSent> for GameMode {
    fn from(mode: GameModeSent) -> Self {
        match mode {
            GameModeSent::Auto => GameMode::Auto,
            GameModeSent::On => GameMode::On,
            GameModeSent::Off => GameMode::Off,
        }
    }
}

/// The most one message may be (bytes): a sample of a busy machine is some tens of kilobytes.
const MOST: usize = 8 << 20;

/// The pipes' names, this user's: what the sensors tell, and what they are asked. A debug build's are its own, as
/// its settings and its single instance are (see `lib::SINGLE_INSTANCE`): run beside an installed Glance, it neither
/// links to that Glance's sensors nor tells them, as another version, to go.
fn names() -> Option<(String, String)> {
    let sid = crate::elevation::user_sid()?;
    let kind = if cfg!(debug_assertions) { "-debug" } else { "" };
    Some((format!(r"\\.\pipe\glance-told{kind}-{sid}"), format!(r"\\.\pipe\glance-asked{kind}-{sid}")))
}

/// Writes `message`, its length first.
fn send<T: Serialize>(to: &mut File, message: &T) -> bool {
    let Ok(body) = serde_json::to_vec(message) else { return false };
    to.write_all(&(body.len() as u32).to_le_bytes()).and_then(|_| to.write_all(&body)).is_ok()
}

/// Reads a message, its length first; none when the other end has gone (or sent what is no message).
fn receive<T: for<'a> Deserialize<'a>>(from: &mut File) -> Option<T> {
    let mut length = [0u8; 4];
    from.read_exact(&mut length).ok()?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MOST {
        return None;
    }
    let mut body = vec![0u8; length];
    from.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

// ---- The sensors' side ----

/// The sensors' ends of the two pipes, made for this user alone (and the system), the first of their names (so
/// that nothing else made them first), and open to the rest running unelevated.
pub struct Sensors {
    told: HANDLE,
    asked: HANDLE,
}

// (The pipes are the kernel's: their handles may be used from any thread.)
unsafe impl Send for Sensors {}
unsafe impl Sync for Sensors {}

impl Sensors {
    pub fn open() -> Option<Self> {
        let (told, asked) = names()?;
        let sid = crate::elevation::user_sid()?;
        // Owned by the Administrators (an owner may always change who may do what: the user may not); the system
        // and the Administrators all; the user, ordinary (medium integrity) processes too, only reading and
        // writing data (FILE_GENERIC_READ | FILE_WRITE_DATA: no further instance of the pipe made, no change to
        // who may use it).
        let sddl = HSTRING::from(format!("O:BAG:BAD:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x0012008B;;;{sid})S:(ML;;NW;;;ME)"));
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(&sddl, SDDL_REVISION_1, &mut descriptor, None) }.ok()?;
        let attributes = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: descriptor.0, bInheritHandle: false.into() };
        let make = |name: &str, access| unsafe {
            CreateNamedPipeW(&HSTRING::from(name), access | FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, 1, 1 << 20, 1 << 16, 0, Some(&attributes))
        };
        let (told, asked) = (make(&told, PIPE_ACCESS_OUTBOUND), make(&asked, PIPE_ACCESS_INBOUND));
        unsafe {
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        }
        if told == INVALID_HANDLE_VALUE || asked == INVALID_HANDLE_VALUE {
            unsafe {
                let _ = CloseHandle(told);
                let _ = CloseHandle(asked);
            }
            return None;
        }
        Some(Sensors { told, asked })
    }

    /// Waits for the rest to come (both its ends); its ends to write to and read from, once it has (copies of
    /// the pipes' handles, closed as they are let go: the pipes stay).
    pub fn accept(&self) -> Option<(File, File)> {
        unsafe {
            for pipe in [self.told, self.asked] {
                // (One that came between the pipe's making and this wait is there already: ERROR_PIPE_CONNECTED.)
                if let Err(error) = ConnectNamedPipe(pipe, None) {
                    if error.code() != windows::Win32::Foundation::ERROR_PIPE_CONNECTED.to_hresult() {
                        return None;
                    }
                }
            }
            let copy = |pipe: HANDLE| {
                let process = windows::Win32::System::Threading::GetCurrentProcess();
                let mut copied = HANDLE::default();
                windows::Win32::Foundation::DuplicateHandle(process, pipe, process, &mut copied, 0, false, windows::Win32::Foundation::DUPLICATE_SAME_ACCESS).ok().map(|_| File::from_raw_handle(copied.0))
            };
            Some((copy(self.told)?, copy(self.asked)?))
        }
    }

    /// The rest gone: the pipes free for it to come again.
    pub fn let_go(&self) {
        unsafe {
            let _ = DisconnectNamedPipe(self.told);
            let _ = DisconnectNamedPipe(self.asked);
        }
    }
}

pub fn tell(to: &mut File, told: &Told) -> bool {
    send(to, told)
}

pub fn heard(from: &mut File) -> Option<Asked> {
    receive(from)
}

/// Why nothing whole came on a pipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unheard {
    /// Nothing by the time it was waited for.
    InTime,
    /// The other end has gone, or what it sent is no message.
    Broke,
}

/// What comes on `from` within `within`, read on a thread of its own, or why nothing did. The read given up on is
/// cancelled, again and again till that thread has ended: a read may be issued just after one is cancelled (a message
/// comes in parts), and that one is not cancelled with it.
pub fn receive_within<T: for<'de> Deserialize<'de> + Send + 'static>(from: &File, within: Duration) -> Result<T, Unheard> {
    use std::os::windows::io::AsRawHandle;
    let mut reading = from.try_clone().map_err(|_| Unheard::Broke)?;
    let (said, came) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let _ = said.send(receive::<T>(&mut reading));
    });
    let got = match came.recv_timeout(within) {
        Ok(Some(message)) => Ok(message),
        Ok(None) => Err(Unheard::Broke),
        Err(_) => Err(Unheard::InTime),
    };
    while !reader.is_finished() {
        unsafe {
            let _ = windows::Win32::System::IO::CancelSynchronousIo(HANDLE(reader.as_raw_handle()));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let _ = reader.join();
    got
}

/// `asked` as it goes on the pipe: its length, then it (see `send`); none where it cannot be written.
pub fn framed(asked: &Asked) -> Option<Vec<u8>> {
    let body = serde_json::to_vec(asked).ok()?;
    let mut framed = (body.len() as u32).to_le_bytes().to_vec();
    framed.extend(body);
    Some(framed)
}

impl Sensors {
    /// Whether what came is the rest of Glance: one process at the other end of both pipes, of this user and this
    /// session, running as the user (medium integrity), Glance beside these sensors. Who is at the asking end is read
    /// through the pipe itself (at identification level: who, never with the rights of), once something has been
    /// read from it. Why not, if not.
    pub fn check_client(&self) -> Result<u32, String> {
        use windows::Win32::Security::RevertToSelf;
        use windows::Win32::System::Pipes::ImpersonateNamedPipeClient;
        use windows::Win32::System::Pipes::{GetNamedPipeClientProcessId, GetNamedPipeClientSessionId};
        use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
        use windows::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThread, OpenThreadToken};
        let (mut told, mut asked, mut session) = (0u32, 0u32, u32::MAX);
        unsafe {
            GetNamedPipeClientProcessId(self.told, &mut told).map_err(|e| format!("its process on one pipe: {e}"))?;
            GetNamedPipeClientProcessId(self.asked, &mut asked).map_err(|e| format!("its process on the other: {e}"))?;
            GetNamedPipeClientSessionId(self.asked, &mut session).map_err(|e| format!("its session: {e}"))?;
        }
        if told != asked {
            return Err(format!("two processes, {told} and {asked}"));
        }
        let mut ours = u32::MAX;
        unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut ours) }.map_err(|e| format!("our session: {e}"))?;
        if session != ours {
            return Err(format!("session {session}, not {ours}"));
        }
        // Who it is, through the pipe; back to ourselves whatever happens.
        unsafe { ImpersonateNamedPipeClient(self.asked) }.map_err(|e| format!("identifying it: {e}"))?;
        let mut token = HANDLE::default();
        let opened = unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut token) };
        let reverted = unsafe { RevertToSelf() };
        if reverted.is_err() {
            // Still someone else: nothing more is done by this process.
            std::process::exit(1);
        }
        opened.map_err(|e| format!("its token: {e}"))?;
        let facts = token_facts(token);
        unsafe {
            let _ = CloseHandle(token);
        }
        let (user, integrity) = facts.ok_or("its token's facts")?;
        if Some(user) != crate::elevation::user_sid() {
            return Err("another user".into());
        }
        // SECURITY_MANDATORY_MEDIUM_RID: the user's ordinary processes.
        if integrity != 0x2000 {
            return Err(format!("integrity {integrity:#x}, not the user's ordinary one"));
        }
        if !runs(told, crate::elevation::APP) {
            return Err("another program".into());
        }
        Ok(told)
    }
}

/// A token's user (as a SID string) and integrity level (its RID).
fn token_facts(token: HANDLE) -> Option<(String, u32)> {
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::Security::{GetSidSubAuthority, GetSidSubAuthorityCount, TokenIntegrityLevel, TokenUser, TOKEN_MANDATORY_LABEL, TOKEN_USER};
    unsafe {
        let mut buffer = vec![0u8; 512];
        let mut size = 0u32;
        GetTokenInformation(token, TokenUser, Some(buffer.as_mut_ptr().cast()), buffer.len() as u32, &mut size).ok()?;
        let mut text = PWSTR::null();
        ConvertSidToStringSidW((*(buffer.as_ptr() as *const TOKEN_USER)).User.Sid, &mut text).ok()?;
        let user = text.to_string().ok();
        let _ = LocalFree(Some(HLOCAL(text.0.cast())));
        let mut label = vec![0u8; 512];
        GetTokenInformation(token, TokenIntegrityLevel, Some(label.as_mut_ptr().cast()), label.len() as u32, &mut size).ok()?;
        let sid = (*(label.as_ptr() as *const TOKEN_MANDATORY_LABEL)).Label.Sid;
        let count = *GetSidSubAuthorityCount(sid) as u32;
        let integrity = *GetSidSubAuthority(sid, count.checked_sub(1)?);
        Some((user?, integrity))
    }
}

/// Whether the process `pid` runs Glance's program `name` (`elevation::APP` or `elevation::SENSORS`) beside this one.
pub(crate) fn runs(pid: u32, name: &str) -> bool {
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let ran = runs_program(process, name);
        let _ = CloseHandle(process);
        ran
    }
}

/// Whether `process` runs Glance's program `name` beside this one: both paths resolved, or no match.
fn runs_program(process: HANDLE, name: &str) -> bool {
    let mut path = vec![0u16; 1024];
    let mut length = path.len() as u32;
    if unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(path.as_mut_ptr()), &mut length) }.is_err() {
        return false;
    }
    let theirs = std::path::PathBuf::from(String::from_utf16_lossy(&path[..length as usize]));
    let Some(ours) = crate::elevation::beside(name) else { return false };
    matches!((std::fs::canonicalize(&ours), std::fs::canonicalize(&theirs)), (Ok(a), Ok(b)) if a == b)
}

// ---- The rest's side ----

/// The rest's ends: what it is told, and what it asks with; and the process at the other end, held from the moment it
/// was found to be the sensors' (see `gone_within`).
pub struct Link {
    pub told: File,
    pub asked: File,
    process: Option<OwnedHandle>,
}

/// What became of the sensors' process at the other end of a link let go of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gone {
    /// It ended, with this code.
    Ended(u32),
    /// It had not ended by the time it was waited for.
    Running,
    /// Not known: its process could not be held, or looked at.
    Unknown,
}

impl Link {
    /// Connects to the sensors, waiting up to `patience` for them; none if they are not there, or what answers is
    /// not the sensors' program beside this one, elevated.
    pub fn connect(patience: Duration) -> Result<Link, String> {
        let (told, asked) = names().ok_or("this user's identity could not be read")?;
        let until = Instant::now() + patience;
        // Only as much as each end needs (the pipes give the user no more), and the sensors allowed to learn who
        // this is, never to act as it (SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION).
        let identification_only = windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0x0011_0000);
        let open = |name: &str, access: u32| -> Result<HANDLE, String> {
            loop {
                let opened = unsafe { CreateFileW(&HSTRING::from(name), access, FILE_SHARE_NONE, None, OPEN_EXISTING, identification_only, None) };
                let error = match opened {
                    Ok(handle) => return Ok(handle),
                    Err(error) => error,
                };
                if Instant::now() >= until {
                    return Err(format!("their pipe was not opened in {} s ({:#x})", patience.as_secs(), error.code().0));
                }
                // (Not made yet, or busy with another: waited for a moment.)
                unsafe {
                    let _ = WaitNamedPipeW(&HSTRING::from(name), 200);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        };
        // FILE_GENERIC_READ; FILE_WRITE_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE.
        let told = open(&told, 0x0012_0089)?;
        let asked = match open(&asked, 0x0010_0082) {
            Ok(asked) => asked,
            Err(why) => {
                unsafe {
                    let _ = CloseHandle(told);
                }
                return Err(why);
            }
        };
        let mut link = unsafe { Link { told: File::from_raw_handle(told.0), asked: File::from_raw_handle(asked.0), process: None } };
        // Believed only from the sensors' program, elevated, at both ends.
        let (mut server, mut other) = (0u32, 0u32);
        unsafe { GetNamedPipeServerProcessId(told, &mut server) }.map_err(|e| format!("who made their pipe was not told ({:#x})", e.code().0))?;
        unsafe { GetNamedPipeServerProcessId(asked, &mut other) }.map_err(|e| format!("who made their pipe was not told ({:#x})", e.code().0))?;
        if server != other {
            return Err(format!("two processes made their pipes, {server} and {other}"));
        }
        if !is_sensors_elevated(server) {
            return Err(format!("process {server}, at their pipes, is not their program elevated"));
        }
        // (Held now, while the link is had: that process and no other, whenever it is looked at.)
        link.process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, false, server) }.ok().map(|process| unsafe { OwnedHandle::from_raw_handle(process.0) });
        if send(&mut link.asked, &Asked::Hello { version: env!("CARGO_PKG_VERSION").into() }) {
            Ok(link)
        } else {
            Err("they could not be told who this is".into())
        }
    }

    /// Lets the link go, and waits up to `ceiling` for the sensors' process at its other end to end. (Another version's
    /// goes as it hears this one; its pipes are free only then.)
    pub fn gone_within(self, ceiling: Duration) -> Gone {
        use std::os::windows::io::AsRawHandle;
        let Link { told, asked, process } = self;
        drop((told, asked));
        let Some(process) = process else { return Gone::Unknown };
        let process = HANDLE(process.as_raw_handle());
        if unsafe { WaitForSingleObject(process, ceiling.as_millis().min(u32::MAX as u128) as u32) } != windows::Win32::Foundation::WAIT_OBJECT_0 {
            return Gone::Running;
        }
        let mut code = 0u32;
        match unsafe { GetExitCodeProcess(process, &mut code) } {
            Ok(()) => Gone::Ended(code),
            Err(_) => Gone::Unknown,
        }
    }

    pub fn receive(&mut self) -> Option<Told> {
        receive(&mut self.told)
    }
}

/// Whether the process `pid` runs Glance's sensors beside this program, elevated.
fn is_sensors_elevated(pid: u32) -> bool {
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let named = runs_program(process, crate::elevation::SENSORS);
        let mut token = HANDLE::default();
        let mut elevated = TOKEN_ELEVATION::default();
        let mut size = 0u32;
        let raised = OpenProcessToken(process, TOKEN_QUERY, &mut token).is_ok()
            && GetTokenInformation(token, TokenElevation, Some((&mut elevated as *mut TOKEN_ELEVATION).cast()), size_of::<TOKEN_ELEVATION>() as u32, &mut size).is_ok()
            && elevated.TokenIsElevated != 0;
        if !token.is_invalid() {
            let _ = CloseHandle(token);
        }
        let _ = CloseHandle(process);
        named && raised
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_want_is_kept_within_what_is_offered() {
        let want = Want { interval_ms: 5, refresh_slow: true, wsl: false, docker: true }.checked();
        assert_eq!(want.interval_ms, 250);
        assert_eq!(Want { interval_ms: 60_000, ..want }.checked().interval_ms, 10_000);
    }

    #[test]
    fn messages_travel_whole() {
        // Through a file standing for the pipe: what is sent is what is received.
        let path = std::env::temp_dir().join(format!("glance-link-{}", std::process::id()));
        {
            let mut out = File::create(&path).unwrap();
            assert!(send(&mut out, &Asked::Want(Want { interval_ms: 1000, refresh_slow: false, wsl: true, docker: false })));
            assert!(send(&mut out, &Asked::GameMode(GameModeSent::On)));
        }
        let mut back = File::open(&path).unwrap();
        assert!(matches!(receive::<Asked>(&mut back), Some(Asked::Want(Want { interval_ms: 1000, wsl: true, .. }))));
        assert!(matches!(receive::<Asked>(&mut back), Some(Asked::GameMode(GameModeSent::On))));
        assert!(receive::<Asked>(&mut back).is_none());
        let _ = std::fs::remove_file(path);
    }
}
