//! Nearby transfer, Glance's side (see `docs/nearby.md`): files sent from devices on the local network by the
//! LocalSend protocol, into the station and recent files, and the station's sent to them.
//!
//! The phone remote's helper (`glance-remote.exe`, running as the user, the one process of Glance's that opens a port)
//! receives and sends. Glance serves the two pipes it tells and is asked by (for this user alone, and only to that
//! helper beside it), asks the user what the helper asks, says what goes on, and takes in what came: each receiving
//! that ended is written down by the helper (a receipt beside its inbox) before Glance is told, and Glance takes in
//! each receipt once, what it took kept before the receipt goes (what the pipe says is only a nudge).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::Deserialize;
use serde_json::json;

/// A device sending, as the helper tells of it: what it said of itself, from where, and whether its fingerprint is its
/// own beyond doubt (it presented the certificate) or only said.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Sender {
    pub alias: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    pub fingerprint: String,
    pub verified: bool,
    pub address: IpAddr,
}

/// A file as a sending offers it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Offered {
    pub name: String,
    pub size: u64,
    #[serde(rename = "type", default)]
    pub kind: String,
}

/// A device seen nearby, to be sent to.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Device {
    pub fingerprint: String,
    pub alias: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
}

/// How a sending of this PC's goes.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Sending {
    /// Offered: the device's user is to take it.
    Waiting,
    Sending { done: u64, of: u64 },
    Sent,
    Refused,
    /// The device is receiving another sending.
    Busy,
    /// It asks for a PIN.
    Pin,
    Cancelled,
    Failed { why: String },
}

/// What the helper tells.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Told {
    /// Whether to take what a device not trusted sends (answered with `answer`).
    Ask { id: u64, from: Sender, files: Vec<Offered> },
    /// That asking given up (not answered in time).
    Unasked { id: u64 },
    Coming { session: String, from: Sender, files: Vec<Offered> },
    Progress { session: String, done: u64, of: u64 },
    /// A receiving over (its receipt written: see `receipts`), and why it ended short, where it did.
    Came { session: String, short: Option<String> },
    Devices { devices: Vec<Device> },
    Sending { job: u64, state: Sending },
}

/// A device's sending the user is asked about.
#[derive(Clone, Debug, PartialEq)]
pub struct Asking {
    pub id: u64,
    pub from: Sender,
    pub files: Vec<Offered>,
}

/// A receiving under way: from whom, what, and how much of it has come.
#[derive(Clone, Debug, PartialEq)]
pub struct Receiving {
    pub session: String,
    pub from: Sender,
    pub files: Vec<Offered>,
    pub done: u64,
    pub of: u64,
}

/// What nearby transfer shows: whether the helper is there to tell, what the user is asked, what comes, the devices
/// seen, and this PC's sendings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shown {
    pub there: bool,
    pub asks: Vec<Asking>,
    pub receiving: Option<Receiving>,
    pub devices: Vec<Device>,
    pub sendings: HashMap<u64, Sending>,
}

static SHOWN: Mutex<Option<Shown>> = Mutex::new(None);
static TOLD: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
/// Where the helper is told, while it is there.
static DOWN: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// The most one line the helper tells may be (bytes): an offer of many files.
const MOST: u64 = 8 << 20;
/// The most files one receipt may hold, and the longest name or alias kept of what it says.
const MOST_FILES: usize = 10_000;
const MOST_NAME: usize = 1_000;

/// What nearby transfer shows now.
pub fn shown() -> Shown {
    SHOWN.lock().unwrap().clone().unwrap_or_default()
}

fn change(act: impl FnOnce(&mut Shown)) {
    act(SHOWN.lock().unwrap().get_or_insert_with(Shown::default));
    if let Some(told) = TOLD.get() {
        told();
    }
}

/// Serves the helper's pipes on a thread of its own, and takes in what came while Glance was not there; `told` is
/// called whenever what it shows changed.
pub fn start(told: impl Fn() + Send + Sync + 'static) {
    if TOLD.set(Box::new(told)).is_err() {
        return;
    }
    crate::station::run::ask(crate::station::run::Ask::Arrivals);
    std::thread::Builder::new().name("glance: nearby".into()).spawn(serve).expect("a thread started");
}

/// The answer to what the helper asked (`id`): whether to take it, and whether to take what that device sends from
/// now on without asking.
pub fn answer(id: u64, take: bool, trust: bool) {
    change(|shown| shown.asks.retain(|asking| asking.id != id));
    command(&json!({ "kind": "answer", "id": id, "take": take, "trust": trust }));
}

/// The files at `paths` sent to the device `to` (its fingerprint), as sending `job`: whether the helper was there to
/// be asked.
pub fn send(job: u64, to: &str, paths: &[PathBuf]) -> bool {
    change(|shown| {
        shown.sendings.insert(job, Sending::Waiting);
    });
    command(&json!({ "kind": "send", "job": job, "to": to, "paths": paths }))
}

pub fn cancel(job: u64) -> bool {
    command(&json!({ "kind": "cancel", "job": job }))
}

/// This PC announced to the devices nearby, and which they are said again (the devices are about to be shown).
pub fn announce() -> bool {
    command(&json!({ "kind": "announce" }))
}

/// Writes `line` to the helper, if it is there.
fn command(line: &serde_json::Value) -> bool {
    let mut down = DOWN.lock().unwrap();
    let Some(pipe) = down.as_mut() else { return false };
    let written = writeln!(pipe, "{line}").and_then(|()| pipe.flush());
    if written.is_err() {
        *down = None;
    }
    written.is_ok()
}

/// What the helper told, as it is shown or done.
fn heard(told: Told) {
    match told {
        Told::Ask { id, from, files } => change(|shown| shown.asks.push(Asking { id, from, files })),
        Told::Unasked { id } => change(|shown| shown.asks.retain(|asking| asking.id != id)),
        Told::Coming { session, from, files } => {
            let of = files.iter().map(|file| file.size).sum();
            change(|shown| shown.receiving = Some(Receiving { session, from, files, done: 0, of }));
        }
        Told::Progress { session, done, of } => change(|shown| {
            if let Some(receiving) = shown.receiving.as_mut().filter(|receiving| receiving.session == session) {
                (receiving.done, receiving.of) = (done, of);
            }
        }),
        Told::Came { session, short } => {
            if let Some(why) = short {
                crate::journal::note(format!("nearby: a receiving ended short ({why})"));
            }
            change(|shown| {
                if shown.receiving.as_ref().is_some_and(|receiving| receiving.session == session) {
                    shown.receiving = None;
                }
            });
            crate::station::run::ask(crate::station::run::Ask::Arrivals);
        }
        Told::Devices { devices } => change(|shown| shown.devices = devices),
        Told::Sending { job, state } => change(|shown| {
            shown.sendings.insert(job, state);
        }),
    }
}

// ---- The pipes ----

/// The pipes' names, this user's: the one the helper tells by, and the one it is told by (as it names them).
fn names() -> Option<(String, String)> {
    let sid = crate::elevation::user_sid()?;
    Some((format!(r"\\.\pipe\dev.weiyi.glance.nearby.{sid}.up"), format!(r"\\.\pipe\dev.weiyi.glance.nearby.{sid}.down")))
}

/// Glance's ends of the two pipes: made for this user alone (and the system), the first of their names (nothing else
/// made them first), open to ordinary processes, reached from this PC only.
fn made() -> Option<(windows::Win32::Foundation::HANDLE, windows::Win32::Foundation::HANDLE)> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{CloseHandle, LocalFree, HLOCAL, INVALID_HANDLE_VALUE};
    use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND, PIPE_ACCESS_OUTBOUND};
    use windows::Win32::System::Pipes::{CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT};
    let (up, down) = names()?;
    let sid = crate::elevation::user_sid()?;
    // The system all; the user (and whoever owns the pipes, which may not change who may use them), ordinary (medium
    // integrity) processes too, only reading and writing data (FILE_GENERIC_READ | FILE_WRITE_DATA: no further
    // instance of a pipe made).
    let sddl = HSTRING::from(format!("D:P(A;;GA;;;SY)(A;;0x0012008B;;;OW)(A;;0x0012008B;;;{sid})S:(ML;;NW;;;ME)"));
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(&sddl, SDDL_REVISION_1, &mut descriptor, None) }.ok()?;
    let attributes = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: descriptor.0, bInheritHandle: false.into() };
    let make = |name: &str, access| unsafe {
        CreateNamedPipeW(&HSTRING::from(name), access | FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, 1, 1 << 16, 1 << 20, 0, Some(&attributes))
    };
    let (up, down) = (make(&up, PIPE_ACCESS_INBOUND), make(&down, PIPE_ACCESS_OUTBOUND));
    unsafe {
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
    }
    if up == INVALID_HANDLE_VALUE || down == INVALID_HANDLE_VALUE {
        unsafe {
            let _ = CloseHandle(up);
            let _ = CloseHandle(down);
        }
        return None;
    }
    Some((up, down))
}

/// Serves the helper, as it comes and goes: what it tells heard, what it is told written.
fn serve() {
    use std::os::windows::io::FromRawHandle;
    use windows::Win32::Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS, ERROR_PIPE_CONNECTED, HANDLE};
    use windows::Win32::System::Pipes::{ConnectNamedPipe, DisconnectNamedPipe, GetNamedPipeClientProcessId};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let Some((up, down)) = made() else {
        crate::journal::note("nearby: its pipes could not be made (another Glance has them?); nearby transfer is not shown");
        return;
    };
    // (The same reason not noted again each time the helper comes.)
    let mut noted = None;
    loop {
        let connected = [up, down].into_iter().all(|pipe| match unsafe { ConnectNamedPipe(pipe, None) } {
            Ok(()) => true,
            Err(error) => error.code() == ERROR_PIPE_CONNECTED.to_hresult(),
        });
        // Believed only from the helper beside this Glance, one process at both ends.
        let (mut one, mut other) = (0u32, 0u32);
        let why = if !connected {
            Some("it did not come whole".to_string())
        } else if unsafe { GetNamedPipeClientProcessId(up, &mut one).and(GetNamedPipeClientProcessId(down, &mut other)) }.is_err() || one != other {
            Some(format!("two processes, {one} and {other}"))
        } else if !crate::link::runs(one, HELPER) {
            Some(format!("process {one} is not the helper beside Glance"))
        } else {
            None
        };
        if let Some(why) = why {
            if noted.as_ref() != Some(&why) {
                crate::journal::note(format!("nearby: what came to its pipes was turned away: {why}"));
            }
            noted = Some(why);
            unsafe {
                let _ = DisconnectNamedPipe(up);
                let _ = DisconnectNamedPipe(down);
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
            continue;
        }
        noted = None;
        let copy = |pipe: HANDLE| unsafe {
            let process = GetCurrentProcess();
            let mut copied = HANDLE::default();
            DuplicateHandle(process, pipe, process, &mut copied, 0, false, DUPLICATE_SAME_ACCESS).ok().map(|()| std::fs::File::from_raw_handle(copied.0))
        };
        if let (Some(told), Some(telling)) = (copy(up), copy(down)) {
            *DOWN.lock().unwrap() = Some(telling);
            change(|shown| shown.there = true);
            // (What came while it was not linked, taken in now.)
            crate::station::run::ask(crate::station::run::Ask::Arrivals);
            listen(told);
        }
        // Gone: nothing to tell it, nothing it asked to answer, nothing coming.
        *DOWN.lock().unwrap() = None;
        change(|shown| *shown = Shown { devices: Vec::new(), ..Shown::default() });
        unsafe {
            let _ = DisconnectNamedPipe(up);
            let _ = DisconnectNamedPipe(down);
        }
    }
}

/// The helper's program, beside Glance's.
const HELPER: &str = "glance-remote.exe";

/// Hears what the helper tells on `told`, a line each, till it goes (or tells a line too long to be one).
fn listen(told: std::fs::File) {
    let mut lines = BufReader::new(told);
    let mut line = Vec::new();
    loop {
        line.clear();
        match (&mut lines).take(MOST).read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) if line.last() != Some(&b'\n') => {
                crate::journal::note("nearby: the helper told a line too long; let go of");
                return;
            }
            Ok(_) => match serde_json::from_slice::<Told>(&line) {
                Ok(told) => heard(told),
                Err(error) => crate::journal::note(format!("nearby: the helper told what is not understood ({error})")),
            },
        }
    }
}

// ---- What came ----

/// A receiving the helper wrote down as it ended: the sending, from whom, the files that came whole (each by its name
/// in the inbox, the name it was sent by, how large it is), and when it ended (seconds since 1970).
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Receipt {
    pub session: String,
    pub from: Sender,
    pub files: Vec<CameFile>,
    pub at: u64,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct CameFile {
    pub file: String,
    pub name: String,
    pub size: u64,
}

impl Receipt {
    /// Whether it holds only what the helper writes: a sending's number and its files' names in the inbox of letters
    /// and digits (with an extension of them), so none names anything but a file of the inbox; within bounds.
    fn sound(&self) -> bool {
        let plain = |text: &str, most: usize| !text.is_empty() && text.len() <= most && text.bytes().all(|byte| byte.is_ascii_alphanumeric());
        let file = |name: &str| match name.split_once('.') {
            Some((stem, extension)) => plain(stem, 64) && plain(extension, 8),
            None => plain(name, 64),
        };
        plain(&self.session, 64) && self.files.len() <= MOST_FILES && self.files.iter().all(|came| file(&came.file))
    }

    /// The device it came from, as the station keeps it.
    pub fn device(&self) -> crate::station::shelf::Device {
        crate::station::shelf::Device { alias: self.from.alias.chars().take(MOST_NAME).collect(), kind: self.from.kind.clone(), fingerprint: self.from.fingerprint.clone() }
    }

    /// Its files as the station takes them in (see `Keep::arrived`): each where it is in `inbox`, as it was sent, how
    /// large it is.
    pub fn files_in(&self, inbox: &Path) -> Vec<(PathBuf, String, u64)> {
        self.files.iter().map(|came| (inbox.join(&came.file), came.name.chars().take(MOST_NAME).collect(), came.size)).collect()
    }
}

/// The helper's folder for nearby transfer (where its inbox and receipts are), reached through no link of the user's
/// making: the user's local application data, as it really is, and the helper's folders in it by their names.
pub fn helper_folder() -> Option<PathBuf> {
    Some(real(crate::settings::local_dir().parent()?)?.join("dev.weiyi.glance.remote").join("nearby"))
}

/// Where the folder at `path` really is (every link on the way followed), as a plain path.
pub(crate) fn real(path: &Path) -> Option<PathBuf> {
    let real = std::fs::canonicalize(path).ok()?;
    let text = real.to_string_lossy();
    Some(match text.strip_prefix(r"\\?\UNC\") {
        Some(shared) => PathBuf::from(format!(r"\\{shared}")),
        None => PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text)),
    })
}

/// The receipts written in `folder` (the helper's: see `helper_folder`), the oldest first, each with where it is. One
/// that is not sound is noted and taken away (nothing would ever take it in).
pub fn receipts(folder: &Path) -> Vec<(PathBuf, Receipt)> {
    let Ok(entries) = std::fs::read_dir(folder.join("receipts")) else { return Vec::new() };
    let mut found: Vec<(PathBuf, Receipt)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "json"))
        .filter_map(|path| {
            let read = std::fs::read(&path).ok()?;
            match serde_json::from_slice::<Receipt>(&read) {
                Ok(receipt) if receipt.sound() => Some((path, receipt)),
                _ => {
                    crate::journal::note(format!("nearby: a receipt that is not as the helper writes them was taken away ({})", path.display()));
                    let _ = std::fs::remove_file(&path);
                    None
                }
            }
        })
        .collect();
    found.sort_by_key(|(_, receipt)| receipt.at);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt(session: &str, files: &[&str]) -> Receipt {
        Receipt {
            session: session.into(),
            from: Sender { alias: "Z Fold".into(), model: None, kind: Some("mobile".into()), fingerprint: "AB".into(), verified: true, address: "10.0.0.5".parse().unwrap() },
            files: files.iter().map(|file| CameFile { file: (*file).into(), name: "照片.jpg".into(), size: 3 }).collect(),
            at: 1,
        }
    }

    /// A receipt names only files of the inbox, by names the helper gives: one that names anything else is not sound.
    #[test]
    fn a_receipt_names_only_files_of_the_inbox() {
        assert!(receipt("a1b2", &["0f3a9c.jpg", "77e1"]).sound());
        for file in ["..\\station.json", "a/b.jpg", "a.b.c", "", "x.jpeg2000long", "C:x", "a..jpg", ".jpg"] {
            assert!(!receipt("a1b2", &[file]).sound(), "{file:?}");
        }
        assert!(!receipt("..", &["a.jpg"]).sound());
        assert!(!receipt("", &["a.jpg"]).sound());
    }

    /// The pipes are made once (none made again while they are), and the helper opens them as it does (writing the
    /// one, reading the other, asking no more), what it writes read whole.
    #[test]
    fn the_helper_opens_the_pipes_as_it_does() {
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::io::FromRawHandle;
        let (up, down) = made().expect("made");
        assert!(made().is_none(), "made once");
        let (up_name, down_name) = names().unwrap();
        let mut telling = std::fs::OpenOptions::new().access_mode(0x0010_0082).open(&up_name).expect("opened to write");
        let _told = std::fs::OpenOptions::new().access_mode(0x0012_0089).open(&down_name).expect("opened to read");
        // (More than that is not given: a further instance of a pipe made by the user.)
        assert!(std::fs::OpenOptions::new().write(true).open(&up_name).is_err());
        writeln!(telling, r#"{{"kind":"unasked","id":4}}"#).unwrap();
        let mut heard = String::new();
        BufReader::new(unsafe { std::fs::File::from_raw_handle(up.0) }).read_line(&mut heard).unwrap();
        assert!(matches!(serde_json::from_str::<Told>(&heard).unwrap(), Told::Unasked { id: 4 }));
        drop(unsafe { std::fs::File::from_raw_handle(down.0) });
    }

    /// What the helper tells is read as it writes it, the receipts too.
    #[test]
    fn what_the_helper_tells_is_read() {
        let from = r#"{"alias":"Z Fold","model":"SM-F956","kind":"mobile","fingerprint":"AB","verified":true,"address":"10.0.0.5"}"#;
        let ask: Told = serde_json::from_str(&format!(r#"{{"kind":"ask","id":3,"from":{from},"files":[{{"id":"f","name":"a.jpg","size":5,"type":"image/jpeg"}}]}}"#)).unwrap();
        assert!(matches!(ask, Told::Ask { id: 3, ref files, .. } if files[0].size == 5));
        let sending: Told = serde_json::from_str(r#"{"kind":"sending","job":7,"state":{"state":"sending","done":1,"of":2}}"#).unwrap();
        assert!(matches!(sending, Told::Sending { job: 7, state: Sending::Sending { done: 1, of: 2 } }));
        let came: Told = serde_json::from_str(&format!(r#"{{"kind":"came","session":"s1","from":{from},"files":[],"short":null}}"#)).unwrap();
        assert!(matches!(came, Told::Came { ref session, short: None } if session == "s1"));
        let receipt: Receipt = serde_json::from_str(&format!(r#"{{"session":"s1","from":{from},"files":[{{"file":"9a.jpg","name":"a.jpg","size":5,"type":"image/jpeg"}}],"at":9}}"#)).unwrap();
        assert!(receipt.sound());
    }
}
