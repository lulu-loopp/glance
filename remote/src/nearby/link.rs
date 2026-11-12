//! What tells Glance and asks it: two pipes of Glance's (it serves them, for this user alone), one each way, a JSON
//! line to each message (one way each, as a pipe opened so is read or written by one thread at a time). With no
//! Glance running, nothing is told, and nobody is asked: no answer.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use super::{Answer, Asked, Event, Told, ASKED_FOR};

/// How often Glance's pipes are looked for while it is not there.
const LOOKED_FOR_EVERY: Duration = Duration::from_secs(2);

/// What Glance asks of this helper.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Command {
    /// The user's answer to `ask` number `id`.
    Answer { id: u64, take: bool, #[serde(default)] trust: bool },
    /// Send `files` to the device `to` (its fingerprint), as sending `job`.
    Send { job: u64, to: String, files: Vec<Outgoing> },
    Cancel { job: u64 },
    /// Announce this PC (Glance is about to show the devices nearby), and say which they are now.
    Announce,
}

/// A file to be sent: where it is, and the name the device is told (a folder's files named by their places in it,
/// "folder/inner/file", as LocalSend names them).
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Outgoing {
    pub path: PathBuf,
    pub name: String,
}

pub struct Link {
    /// Where what Glance is told is written, while it is there.
    up: Mutex<Option<std::fs::File>>,
    /// The asks not answered yet.
    pending: Mutex<HashMap<u64, mpsc::Sender<Answer>>>,
    next: AtomicU64,
}

/// The pipe of Glance's this user's helper is told by (`down`) or tells by: named for this user, as Glance names it.
pub fn pipe(down: bool) -> PathBuf {
    PathBuf::from(format!(r"\\.\pipe\dev.weiyi.glance.nearby.{}.{}", user_sid().unwrap_or_default(), if down { "down" } else { "up" }))
}

/// This process's user, as a SID's text.
fn user_sid() -> Option<String> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
        let mut size = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut size);
        let mut buffer = vec![0u64; size as usize / 8 + 1];
        let got = GetTokenInformation(token, TokenUser, Some(buffer.as_mut_ptr().cast()), size, &mut size);
        let _ = CloseHandle(token);
        got.ok()?;
        let user = &*(buffer.as_ptr() as *const TOKEN_USER);
        let mut text = windows::core::PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut text).ok()?;
        let sid = text.to_string().ok();
        let _ = LocalFree(Some(HLOCAL(text.0.cast())));
        sid
    }
}

/// Glance's pipes, opened: the one it is told by, and the one it tells by. Not there, no reason; another than
/// Glance's program beside this one serving them, why they are not kept to.
fn glances() -> Result<(std::fs::File, std::fs::File), Option<String>> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;
    // Glance allowed to learn who this is, never to act as it (SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION).
    const IDENTIFICATION_ONLY: u32 = 0x0011_0000;
    // Only as much as each end needs, and Glance gives no more: writing data (FILE_WRITE_DATA | FILE_READ_ATTRIBUTES |
    // SYNCHRONIZE), and reading (FILE_GENERIC_READ).
    let up = std::fs::OpenOptions::new().access_mode(0x0010_0082).security_qos_flags(IDENTIFICATION_ONLY).open(pipe(false)).map_err(|_| None)?;
    let down = std::fs::OpenOptions::new().access_mode(0x0012_0089).security_qos_flags(IDENTIFICATION_ONLY).open(pipe(true)).map_err(|_| None)?;
    let server = |file: &std::fs::File| {
        use std::os::windows::io::AsRawHandle;
        let mut process = 0u32;
        unsafe { GetNamedPipeServerProcessId(HANDLE(file.as_raw_handle()), &mut process) }.ok().map(|()| process)
    };
    let (Some(one), Some(other)) = (server(&up), server(&down)) else { return Err(Some("who serves them was not told".into())) };
    if one != other {
        return Err(Some(format!("two processes serve them, {one} and {other}")));
    }
    if !runs_glance(one) {
        return Err(Some(format!("process {one}, serving them, is not Glance beside this helper")));
    }
    Ok((up, down))
}

/// Whether process `number` runs the Glance beside this helper (both paths resolved, or not).
fn runs_glance(number: u32) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, number) }) else { return false };
    let mut path = vec![0u16; 1024];
    let mut length = path.len() as u32;
    let read = unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, windows::core::PWSTR(path.as_mut_ptr()), &mut length) };
    let _ = unsafe { CloseHandle(process) };
    if read.is_err() {
        return false;
    }
    let theirs = PathBuf::from(String::from_utf16_lossy(&path[..length as usize]));
    let Some(ours) = std::env::current_exe().ok().and_then(|helper| Some(helper.parent()?.join("glance.exe"))) else { return false };
    matches!((std::fs::canonicalize(ours), std::fs::canonicalize(theirs)), (Ok(a), Ok(b)) if a == b)
}

impl Link {
    /// Looks for Glance's pipes (a thread), keeps to them while they are there, and hands what Glance asks to `asked`.
    pub fn start(asked: impl Fn(Command) + Send + Sync + 'static) -> Arc<Link> {
        let link = Arc::new(Link { up: Mutex::new(None), pending: Mutex::default(), next: AtomicU64::new(1) });
        let keeping = link.clone();
        // (Why the pipes were last not kept to: noted once, not each time they are looked for.)
        let mut noted = None;
        std::thread::spawn(move || loop {
            let (up, down) = match glances() {
                Ok(both) => both,
                Err(why) => {
                    // (Not there is how it is most of the time: nothing to note.)
                    if why.is_some() && why != noted {
                        crate::journal::note(format!("nearby: Glance's pipes not kept to: {}", why.as_deref().unwrap_or_default()));
                    }
                    noted = why;
                    std::thread::sleep(LOOKED_FOR_EVERY);
                    continue;
                }
            };
            noted = None;
            crate::journal::note("nearby: Glance is there to tell");
            *keeping.up.lock().unwrap() = Some(up);
            for line in BufReader::new(down).lines() {
                let Ok(line) = line else { break };
                match serde_json::from_str::<Command>(&line) {
                    Ok(Command::Answer { id, take, trust }) => {
                        if let Some(waiting) = keeping.pending.lock().unwrap().remove(&id) {
                            let _ = waiting.send(Answer { take, trust });
                        }
                    }
                    Ok(command) => asked(command),
                    Err(error) => crate::journal::note(format!("nearby: Glance said what is not understood ({error})")),
                }
            }
            // Gone: nothing is told till it is there again, and what was being asked has no answer.
            *keeping.up.lock().unwrap() = None;
            keeping.pending.lock().unwrap().clear();
            crate::journal::note("nearby: Glance is gone");
            std::thread::sleep(LOOKED_FOR_EVERY);
        });
        link
    }

    /// Writes `line` to Glance, if it is there.
    fn write(&self, line: &serde_json::Value) -> bool {
        let mut up = self.up.lock().unwrap();
        let Some(pipe) = up.as_mut() else { return false };
        let written = writeln!(pipe, "{line}").and_then(|()| pipe.flush());
        if written.is_err() {
            *up = None;
        }
        written.is_ok()
    }
}

impl Told for Link {
    fn ask(&self, asked: &Asked) -> Option<Answer> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (answered, answer) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, answered);
        if !self.write(&json!({ "kind": "ask", "id": id, "from": asked.from, "files": asked.files })) {
            self.pending.lock().unwrap().remove(&id);
            return None;
        }
        let answer = answer.recv_timeout(ASKED_FOR).ok();
        if answer.is_none() {
            self.pending.lock().unwrap().remove(&id);
            // (Glance's notice of it is taken back.)
            self.write(&json!({ "kind": "unasked", "id": id }));
        }
        answer
    }

    fn tell(&self, event: Event) {
        self.write(&serde_json::to_value(event).expect("an event is written"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What Glance tells it to send (as Glance's `nearby::send_line` writes it) is read: each file where it is, and
    /// the name the device is told.
    #[test]
    fn a_sending_glance_asks_for_is_read() {
        let line = r#"{"files":[{"name":"trip/a.jpg","path":"C:\\Users\\me\\trip\\a.jpg"}],"job":3,"kind":"send","to":"AB12"}"#;
        match serde_json::from_str::<Command>(line).unwrap() {
            Command::Send { job, to, files } => {
                assert_eq!((job, to.as_str()), (3, "AB12"));
                assert_eq!(files, [Outgoing { path: PathBuf::from(r"C:\Users\me\trip\a.jpg"), name: "trip/a.jpg".into() }]);
            }
            other => panic!("read as {other:?}"),
        }
    }
}
