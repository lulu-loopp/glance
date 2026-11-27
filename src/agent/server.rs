//! The interface's pipe, served by Glance while it is on: made for this user alone, reached from this PC only, a few
//! clients at once. Each client is first checked to be a process of this user's (its token, as the pipe gives it),
//! then says which interface it speaks, then asks one call at a time, each answered on a thread of its own within its
//! time (see `Call`), and stopped as the client goes.

use super::pipe::{Came, End, Event, Line, Lines, ANSWER_MOST, ASKED_MOST};
use super::tools::{self, Args, Weight};
use super::Failure;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use windows::core::HSTRING;
use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE};

/// How many clients may be served at once.
const CLIENTS: u32 = 4;
/// How long a call may take, from when it is read, its wait for a turn among them.
const CALL_TIME: Duration = Duration::from_secs(10);
/// How long a client that came is waited for to say which interface it speaks (ms).
const HELLO_WITHIN: u32 = 10_000;

/// The pipe served, till it is stopped.
pub struct Server {
    stop: Arc<Event>,
    accepting: std::thread::JoinHandle<()>,
}

impl Server {
    /// The pipe made and served: none where it could not be made (another program had its name first).
    pub fn start() -> Option<Server> {
        let name = super::pipe_name()?;
        let sid = crate::elevation::user_sid()?;
        let first = make(&name, &sid, true)?;
        let stop = Arc::new(Event::new());
        let accepting = {
            let stop = stop.clone();
            std::thread::Builder::new().name("glance: agent pipe".into()).spawn(move || accept(first, &name, &sid, &stop)).ok()?
        };
        crate::journal::note("agent interface: on");
        Some(Server { stop, accepting })
    }

    /// Its clients let go of, their calls stopped, and the pipe gone, before this returns.
    pub fn stop(self) {
        self.stop.set();
        let _ = self.accepting.join();
        crate::journal::note("agent interface: off");
    }
}

/// An instance of the pipe named `name` made: open to this user (`sid`) and the system alone, to processes of medium
/// integrity and above, from this PC; the first of its name only where nothing else made one first.
fn make(name: &str, sid: &str, first: bool) -> Option<End> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows::Win32::Storage::FileSystem::{FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX};
    use windows::Win32::System::Pipes::{CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT};
    // The system all; the user (and whoever owns the pipe), reading and writing (FILE_GENERIC_READ |
    // FILE_GENERIC_WRITE, which holds FILE_CREATE_PIPE_INSTANCE): an instance after the first is made by them, as a
    // duplex pipe asks, and any process of the user's holds the owner's rights, so its clients cannot be given fewer.
    // No process below medium integrity reads it or writes to it, so none opens it (a sandboxed program of the user's
    // could otherwise hold its instances open, unread, and leave none for the user's own).
    let sddl = HSTRING::from(format!("D:P(A;;GA;;;SY)(A;;0x0012019F;;;OW)(A;;0x0012019F;;;{sid})S:(ML;;NWNR;;;ME)"));
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(&sddl, SDDL_REVISION_1, &mut descriptor, None) }.ok()?;
    let attributes = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: descriptor.0, bInheritHandle: false.into() };
    let flags = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { FILE_FLAGS_AND_ATTRIBUTES(0) };
    // (Two more than are served: one connected while all are, waiting, and the one there to be found meanwhile.)
    let pipe = unsafe { CreateNamedPipeW(&HSTRING::from(name), flags, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, CLIENTS + 2, 1 << 16, 1 << 16, 0, Some(&attributes)) };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
    }
    (pipe != INVALID_HANDLE_VALUE).then_some(End(pipe))
}

/// How many clients are served, told as one goes.
struct Served {
    count: Mutex<u32>,
    gone: Condvar,
}

/// Clients taken on `first` and the instances made after it, each served on a thread of its own, till `stop`; then
/// each waited for to be gone. One instance is always there to be found while the pipe is served: the next is made
/// as soon as one is connected, so that a client never finds no pipe between them (it would tell Glance not running),
/// and one connected while all are served waits, not served yet, for one of them to go.
fn accept(first: End, name: &str, sid: &str, stop: &Arc<Event>) {
    use windows::Win32::Foundation::WAIT_OBJECT_0;
    use windows::Win32::System::Pipes::{ConnectNamedPipe, DisconnectNamedPipe};
    use windows::Win32::System::Threading::{WaitForMultipleObjects, INFINITE};
    use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
    let served = Arc::new(Served { count: Mutex::new(0), gone: Condvar::new() });
    let mut serving = Vec::new();
    let mut listening = Some(first);
    while !stop.is_set() {
        let Some(end) = listening.take().or_else(|| make(name, sid, false)) else {
            // (Made again a moment later: Windows may lack what it takes now.)
            if unsafe { WaitForMultipleObjects(&[stop.0], false, 200) } == WAIT_OBJECT_0 {
                break;
            }
            continue;
        };
        let came = Event::new();
        let mut overlapped = OVERLAPPED { hEvent: came.0, ..Default::default() };
        let connected = match unsafe { ConnectNamedPipe(end.0, Some(&mut overlapped)) } {
            Ok(()) => true,
            Err(error) if error.code() == ERROR_PIPE_CONNECTED.to_hresult() => true,
            Err(error) if error.code() == ERROR_IO_PENDING.to_hresult() => {
                let waited = unsafe { WaitForMultipleObjects(&[came.0, stop.0], false, INFINITE) };
                if waited != WAIT_OBJECT_0 {
                    let _ = unsafe { CancelIoEx(end.0, Some(&overlapped)) };
                }
                let mut moved = 0;
                unsafe { GetOverlappedResult(end.0, &overlapped, &mut moved, true) }.is_ok() && waited == WAIT_OBJECT_0
            }
            Err(_) => false,
        };
        if !connected {
            if stop.is_set() {
                break;
            }
            // (A client gone before it was taken, say: the instance made ready again, never let go of, so that the
            // pipe is never without one, which another program could then make in Glance's stead.)
            if unsafe { DisconnectNamedPipe(end.0) }.is_ok() {
                listening = Some(end);
            } else if unsafe { WaitForMultipleObjects(&[stop.0], false, 200) } == WAIT_OBJECT_0 {
                break;
            }
            continue;
        }
        listening = make(name, sid, false);
        // (Served once fewer than all are.)
        {
            let mut count = served.count.lock().unwrap();
            while *count >= CLIENTS && !stop.is_set() {
                count = served.gone.wait_timeout(count, Duration::from_millis(200)).unwrap().0;
            }
            if stop.is_set() {
                break;
            }
            *count += 1;
        }
        let (counted, stop) = (served.clone(), stop.clone());
        let sid = sid.to_string();
        serving.retain(|thread: &std::thread::JoinHandle<()>| !thread.is_finished());
        if let Ok(thread) = std::thread::Builder::new().name("glance: agent client".into()).spawn(move || {
            serve(Arc::new(end), &sid, &stop);
            *counted.count.lock().unwrap() -= 1;
            counted.gone.notify_all();
        }) {
            serving.push(thread);
        } else {
            *served.count.lock().unwrap() -= 1;
        }
    }
    for thread in serving {
        let _ = thread.join();
    }
}

/// Who a client is, as its token tells: whether it is a process of this user's at medium integrity or above, and
/// whether it runs elevated.
struct Client {
    users: bool,
    elevated: bool,
}

/// The client at the other end of `end` (this user's being `sid`), as the token it opened the pipe with tells (once
/// something was read from it: Windows lets no server act as its client before).
fn client(end: &End, sid: &str) -> Client {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Security::{RevertToSelf, TOKEN_QUERY};
    use windows::Win32::System::Pipes::ImpersonateNamedPipeClient;
    use windows::Win32::System::Threading::{GetCurrentThread, OpenThreadToken};
    let mut told = Client { users: false, elevated: false };
    if unsafe { ImpersonateNamedPipeClient(end.0) }.is_err() {
        return told;
    }
    let mut token = HANDLE::default();
    let opened = unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut token) }.is_ok();
    let _ = unsafe { RevertToSelf() };
    if opened {
        told = Client { users: super::token_is_users(token, sid), elevated: crate::elevation::token_elevated(token) };
        let _ = unsafe { CloseHandle(token) };
    }
    told
}

/// How long a call that was told to stop (its time over, its client gone, the interface turned off) is waited for to
/// end, beyond which it is let run on alone: one stuck in Windows (a folder opened through a link to a network share
/// that does not answer) neither keeps its client waiting nor the interface from turning off. Its turn stays taken
/// till it does end.
const LET_GO_AFTER: u32 = 1_000;

/// Writes `answer` to `end` as a line: false where it could not be (the client gone, the interface turned off).
fn tell(end: &End, answer: &Value, stop: &Event) -> bool {
    let mut line = serde_json::to_vec(answer).unwrap_or_default();
    line.push(b'\n');
    end.write_all(&line, &[stop]) == Came::Done(())
}

/// A failure answered to call `id` (none: to no call).
fn failed(id: Option<&Value>, failure: &Failure) -> Value {
    let mut answer = json!({ "error": failure.told() });
    if let Some(id) = id {
        answer["id"] = id.clone();
    }
    answer
}

/// Serves the client at `end`, as long as it stays and the interface is on.
fn serve(end: Arc<End>, sid: &str, stop: &Arc<Event>) {
    use windows::Win32::System::Pipes::DisconnectNamedPipe;
    let mut lines = Lines::new(ASKED_MOST);
    // First, which interface it speaks, and which Glance it came with: nothing is told before.
    let hello = match lines.next(&end, &[stop], Some(HELLO_WITHIN)) {
        Line::Read(line) => serde_json::from_slice::<Value>(&line).ok(),
        _ => None,
    };
    // Then who it is (only this user's processes: the system, whom the pipe lets in as all Glance's pipes do, has
    // no need of it).
    let client = client(&end, sid);
    if !client.users {
        let _ = unsafe { DisconnectNamedPipe(end.0) };
        return;
    }
    let Some(hello) = hello.filter(|hello| hello.get("hello").is_some()) else {
        let _ = unsafe { DisconnectNamedPipe(end.0) };
        return;
    };
    let speaks = hello["hello"].as_u64();
    let version = hello["glance"].as_str().unwrap_or_default();
    if speaks != Some(super::INTERFACE) || version != super::VERSION {
        let failure = Failure::new("version_mismatch", format!("glance-cli {version} does not match Glance {} running. Use the glance-cli.exe installed with it.", super::VERSION));
        tell(&end, &failed(None, &failure), stop);
        let _ = unsafe { DisconnectNamedPipe(end.0) };
        return;
    }
    if !tell(&end, &json!({ "hello": super::INTERFACE, "glance": super::VERSION }), stop) {
        return;
    }
    // Then one call at a time, each read while the one before runs (the client gone meanwhile stops it).
    loop {
        let line = match lines.next(&end, &[stop], None) {
            Line::Read(line) => line,
            Line::TooLong => {
                tell(&end, &failed(None, &Failure::new("request_too_large", format!("A request may be at most {} KiB.", ASKED_MOST >> 10))), stop);
                break;
            }
            Line::Closed | Line::Woken(_) | Line::Late => break,
        };
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let asked: Option<Value> = serde_json::from_slice(&line).ok();
        let Some((id, tool, args)) = asked.as_ref().and_then(|asked| Some((asked.get("id")?.clone(), asked.get("tool")?.as_str()?.to_string(), asked.get("args").cloned().unwrap_or(Value::Null)))) else {
            if !tell(&end, &failed(asked.as_ref().and_then(|asked| asked.get("id")), &Failure::invalid("A request is {\"id\", \"tool\", \"args\"}.")), stop) {
                break;
            }
            continue;
        };
        let call = Call { deadline: Instant::now() + CALL_TIME, cancelled: Arc::new(AtomicBool::new(false)), end: end.clone(), client_elevated: client.elevated };
        let done = Arc::new(Event::new());
        let answered = Arc::new(Mutex::new(None));
        let working = {
            let (call, done, answered) = (call.clone(), done.clone(), answered.clone());
            std::thread::Builder::new().name("glance: agent call".into()).spawn(move || {
                let answer = run(&call, &tool, &args);
                *answered.lock().unwrap() = Some(answer);
                done.set();
            })
        };
        let Ok(working) = working else {
            tell(&end, &failed(Some(&id), &Failure::new("busy", "Glance could not take the call now. Try again.")), stop);
            continue;
        };
        // While it runs, whether the client goes, or asks another call before this one is answered (one at a time:
        // the connection then closed, which stops this one), or its time is over. Then it is told to stop, and
        // waited for a moment more.
        let within = u32::try_from(call.left().as_millis()).unwrap_or(u32::MAX).saturating_add(LET_GO_AFTER);
        let came = lines.next(&end, &[&done, stop], Some(within));
        let ended = came == Line::Woken(0) || {
            call.cancelled.store(true, Ordering::Relaxed);
            let waited = unsafe { windows::Win32::System::Threading::WaitForMultipleObjects(&[done.0], false, LET_GO_AFTER) };
            waited == windows::Win32::Foundation::WAIT_OBJECT_0
        };
        if ended {
            let _ = working.join();
        }
        // (Its client gone, or the interface off: nothing to answer.)
        if !matches!(came, Line::Woken(0) | Line::Late) {
            break;
        }
        let answer = match answered.lock().unwrap().take() {
            Some(Ok(result)) => json!({ "id": id, "result": result }),
            Some(Err(failure)) => failed(Some(&id), &failure),
            // (Still running, let run on alone: its time is over.)
            None => failed(Some(&id), &Failure::new("timeout", format!("The call took longer than {} seconds. Ask about a smaller folder, or try again once the drive's index is ready.", CALL_TIME.as_secs()))),
        };
        let line = serde_json::to_vec(&answer).unwrap_or_default();
        let answer = if line.len() + 1 > ANSWER_MOST { failed(Some(&id), &Failure::new("result_too_large", "The answer is too large. Ask for fewer results.")) } else { answer };
        if !tell(&end, &answer, stop) {
            break;
        }
    }
    let _ = unsafe { DisconnectNamedPipe(end.0) };
}

/// A call being answered: by when it must be, whether its client went, and who asked (the pipe it came by, to act as
/// its client to check a folder may be read).
#[derive(Clone)]
pub struct Call {
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
    end: Arc<End>,
    client_elevated: bool,
}

impl Call {
    /// Whether it goes on: why not, where its time is over or its client went.
    pub fn going(&self) -> Result<(), Failure> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(Failure::new("cancelled", "The call was cancelled."));
        }
        if Instant::now() >= self.deadline {
            return Err(Failure::new("timeout", format!("The call took longer than {} seconds. Ask about a smaller folder, or try again once the drive's index is ready.", CALL_TIME.as_secs())));
        }
        Ok(())
    }

    /// Why it stopped, where it did (see `going`).
    pub fn stopped(&self) -> Failure {
        self.going().err().unwrap_or_else(|| Failure::new("cancelled", "The call was cancelled."))
    }

    /// How long it has left.
    pub fn left(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    /// Whether its client may be told of files: not a client without an administrator's rights while Glance has them
    /// (its index then holds names that client may not list).
    pub fn may_read_files(&self) -> Result<(), Failure> {
        if crate::elevation::is_elevated() && !self.client_elevated {
            return Err(Failure::new("access_denied", "Glance runs as administrator and this client does not, so it is not told of files. Run the AI tool as administrator, or Glance without."));
        }
        Ok(())
    }

    /// The folder at `typed`, opened as its client, to check its user may list it: as Glance's index reads it, at
    /// the path it really is at.
    pub fn folder(&self, typed: &str) -> Result<super::files::Asked, Failure> {
        self.may_read_files()?;
        let path = super::files::absolute(typed)?;
        let real = opened_as(&self.end, &path)?;
        let (root, rest) = crate::search::rooted(&real).ok_or_else(|| Failure::new("not_local", format!("{} is not on a lettered drive of this PC.", real.display())))?;
        Ok(super::files::Asked { root, rest, path: real })
    }
}

/// The folder at `path` opened as the client at `end` (so that Windows checks its user may list it): where it really
/// is, through links on the way, or why it cannot be.
fn opened_as(end: &End, path: &Path) -> Result<std::path::PathBuf, Failure> {
    use windows::Win32::Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_INVALID_NAME, ERROR_BAD_NETPATH};
    use windows::Win32::Security::RevertToSelf;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, GetFileInformationByHandle, GetFinalPathNameByHandleW, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, VOLUME_NAME_DOS,
    };
    use windows::Win32::System::Pipes::ImpersonateNamedPipeClient;
    unsafe { ImpersonateNamedPipeClient(end.0) }.map_err(|_| Failure::new("access_denied", "Glance could not act as this client to check the folder."))?;
    let opened = unsafe { CreateFileW(&HSTRING::from(path.as_os_str()), (FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES).0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, None, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, None) };
    let _ = unsafe { RevertToSelf() };
    let handle = opened.map_err(|error| {
        let code = error.code();
        if code == ERROR_ACCESS_DENIED.to_hresult() {
            Failure::new("access_denied", format!("This Windows user may not list {}.", path.display()))
        } else if [ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_INVALID_NAME, ERROR_BAD_NETPATH].iter().any(|known| code == known.to_hresult()) {
            Failure::new("not_found", format!("{} does not exist.", path.display()))
        } else {
            Failure::new("not_found", format!("{} could not be opened: {}", path.display(), error.message()))
        }
    })?;
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    let folder = unsafe { GetFileInformationByHandle(handle, &mut information) }.is_ok() && information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0;
    let mut buffer = vec![0u16; 32_768];
    let length = unsafe { GetFinalPathNameByHandleW(handle, &mut buffer, VOLUME_NAME_DOS) } as usize;
    let _ = unsafe { CloseHandle(handle) };
    if !folder {
        return Err(Failure::new("not_a_folder", format!("{} is a file, not a folder.", path.display())));
    }
    if length == 0 || length >= buffer.len() {
        return Err(Failure::new("not_local", format!("{} is on a drive with no letter.", path.display())));
    }
    let real = String::from_utf16_lossy(&buffer[..length]);
    let real = real.strip_prefix(r"\\?\").unwrap_or(&real);
    if real.to_ascii_uppercase().starts_with(r"UNC\") {
        return Err(Failure::new("not_local", format!("{} is on the network.", path.display())));
    }
    Ok(std::path::PathBuf::from(real))
}

/// The turns calls of a weight take: a few light ones at once, one heavy one (it goes through a drive's names).
struct Turns {
    taken: Mutex<usize>,
    freed: Condvar,
    most: usize,
}

static LIGHT: Turns = Turns { taken: Mutex::new(0), freed: Condvar::new(), most: 2 };
static HEAVY: Turns = Turns { taken: Mutex::new(0), freed: Condvar::new(), most: 1 };

/// A turn taken: given back as it is let go.
struct Turn(&'static Turns);

impl Drop for Turn {
    fn drop(&mut self) {
        *self.0.taken.lock().unwrap() -= 1;
        self.0.freed.notify_one();
    }
}

impl Turns {
    /// A turn, waited for while `call` goes on.
    fn take(&'static self, call: &Call) -> Result<Turn, Failure> {
        let mut taken = self.taken.lock().unwrap();
        while *taken >= self.most {
            call.going()?;
            taken = self.freed.wait_timeout(taken, Duration::from_millis(50)).unwrap().0;
        }
        *taken += 1;
        Ok(Turn(self))
    }
}

/// Call `name` answered, given `args`, as `call` lets it go on.
fn run(call: &Call, name: &str, args: &Value) -> Result<Value, Failure> {
    let tool = tools::named(name).ok_or_else(|| Failure::new("unknown_tool", format!("There is no call named {name}.")))?;
    let args = Args::of(args, &tool)?;
    let _turn = match tool.weight {
        Weight::Light => LIGHT.take(call)?,
        Weight::Heavy => HEAVY.take(call)?,
    };
    call.going()?;
    match name {
        "glance_status" => super::readings::status(),
        "get_hardware" => super::readings::hardware(),
        "get_readings" => super::readings::readings(&args),
        "get_history" => super::readings::history(&args),
        "top_processes" => super::readings::processes(&args),
        "ai_usage" => super::readings::ai_usage(&args, call),
        "list_drives" => super::files::drives(),
        "folder_size" => super::files::folder_size(&args, call),
        "list_folder" => super::files::list_folder(&args, call),
        "largest_files" => super::files::largest_files(&args, call),
        "disk_breakdown" => super::files::disk_breakdown(&args, call),
        "search_files" => super::files::search_files(&args, call),
        _ => Err(Failure::new("unknown_tool", format!("There is no call named {name}."))),
    }
}
