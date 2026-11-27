//! A client of the interface's pipe (`glance-cli.exe`'s): each call asked on a connection of its own, made as it is
//! asked, so that Glance started, or the interface turned on, after the client was is found.

use super::pipe::{Came, End, Event, Line, Lines, ANSWER_MOST};
use super::Failure;
use serde_json::{json, Value};
use windows::core::HSTRING;
use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY};
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ, FILE_SHARE_NONE, FILE_WRITE_DATA, OPEN_EXISTING};
use windows::Win32::System::Pipes::WaitNamedPipeW;

/// How long a pipe all of whose instances are taken is waited for (ms).
const BUSY_WAIT: u32 = 5_000;
/// How long Glance is waited for to say which interface it speaks (ms).
const HELLO_WITHIN: u32 = 5_000;
/// How long a call is waited for, beyond the time Glance gives it (ms).
const ANSWER_WITHIN: u32 = 15_000;

/// Why Glance could not be reached: not running, or running with the interface off.
fn unreached() -> Failure {
    if crate::already_running() {
        Failure::new("interface_off", "Glance is running, but AI tools may not read it. Turn on \"Let AI tools read this PC's information\" in Glance's settings, under General.")
    } else {
        Failure::new("glance_not_running", "Glance is not running.")
    }
}

/// A connection to Glance's pipe, past its hello.
fn connect(stop: &Event) -> Result<(End, Lines), Failure> {
    connect_saying(&json!({ "hello": super::INTERFACE, "glance": super::VERSION, "client": format!("glance-cli {}", super::VERSION) }), stop)
}

/// A connection to Glance's pipe, past `hello`.
fn connect_saying(hello: &Value, stop: &Event) -> Result<(End, Lines), Failure> {
    let name = super::pipe_name().ok_or_else(|| Failure::new("glance_not_running", "This Windows user could not be told."))?;
    let end = loop {
        // (Reading and writing data alone: what the pipe lets this user do.)
        let opened = unsafe { CreateFileW(&HSTRING::from(&name), (FILE_GENERIC_READ | FILE_WRITE_DATA).0, FILE_SHARE_NONE, None, OPEN_EXISTING, FILE_FLAG_OVERLAPPED, None) };
        match opened {
            Ok(handle) => break End(handle),
            Err(error) if error.code() == ERROR_PIPE_BUSY.to_hresult() => {
                if !unsafe { WaitNamedPipeW(&HSTRING::from(&name), BUSY_WAIT) }.as_bool() {
                    return Err(Failure::new("busy", "Glance is answering other AI tools. Try again."));
                }
            }
            Err(error) if error.code() == ERROR_FILE_NOT_FOUND.to_hresult() => return Err(unreached()),
            Err(error) if error.code() == ERROR_ACCESS_DENIED.to_hresult() => return Err(Failure::new("access_denied", "Glance's interface is not open to this process.")),
            Err(error) => return Err(Failure::new("glance_not_running", format!("Glance could not be reached: {}", error.message()))),
        }
    };
    // Only Glance of this user's is asked anything: a pipe of that name another account made first is not Glance's.
    if !served_by_this_user(&end) {
        return Err(Failure::new("access_denied", "The pipe Glance answers on is held by a program of another Windows user."));
    }
    let mut lines = Lines::new(ANSWER_MOST);
    let mut line = serde_json::to_vec(hello).unwrap_or_default();
    line.push(b'\n');
    if end.write_all(&line, &[stop]) != Came::Done(()) {
        return Err(unreached());
    }
    let answer = match lines.next(&end, &[stop], Some(HELLO_WITHIN)) {
        Line::Read(line) => serde_json::from_slice::<Value>(&line).ok(),
        Line::Woken(_) => return Err(Failure::new("cancelled", "The call was cancelled.")),
        _ => None,
    };
    match answer {
        Some(answer) if answer.get("hello").and_then(Value::as_u64) == Some(super::INTERFACE) => Ok((end, lines)),
        Some(answer) if answer.get("error").is_some() => Err(Failure::read(&answer["error"]).unwrap_or_else(unreached)),
        _ => Err(unreached()),
    }
}

/// Whether the process serving the pipe at `end` runs as this Windows user.
fn served_by_this_user(end: &End) -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::TOKEN_QUERY;
    use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;
    use windows::Win32::System::Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};
    let mut pid = 0u32;
    if unsafe { GetNamedPipeServerProcessId(end.0, &mut pid) }.is_err() {
        return false;
    }
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else {
        return false;
    };
    let mut token = HANDLE::default();
    let opened = unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.is_ok();
    let _ = unsafe { CloseHandle(process) };
    if !opened {
        return false;
    }
    let sid = crate::elevation::token_sid(token);
    let _ = unsafe { CloseHandle(token) };
    sid.is_some() && sid == crate::elevation::user_sid()
}

/// Call `tool` asked of Glance with `args`, and its answer: given up, the connection closed (which stops it there),
/// once `stop` is set.
pub fn call(tool: &str, args: &Value, stop: &Event) -> Result<Value, Failure> {
    let (end, mut lines) = connect(stop)?;
    let mut line = serde_json::to_vec(&json!({ "id": 1, "tool": tool, "args": args })).unwrap_or_default();
    line.push(b'\n');
    match end.write_all(&line, &[stop]) {
        Came::Done(()) => {}
        Came::Woken(_) => return Err(Failure::new("cancelled", "The call was cancelled.")),
        _ => return Err(unreached()),
    }
    let answer = match lines.next(&end, &[stop], Some(ANSWER_WITHIN)) {
        Line::Read(line) => serde_json::from_slice::<Value>(&line).map_err(|_| Failure::new("unknown_error", "Glance's answer could not be read."))?,
        Line::Woken(_) => return Err(Failure::new("cancelled", "The call was cancelled.")),
        Line::Late => return Err(Failure::new("timeout", "Glance did not answer in time.")),
        Line::TooLong => return Err(Failure::new("result_too_large", "Glance's answer is too large.")),
        Line::Closed => return Err(unreached()),
    };
    if let Some(error) = answer.get("error") {
        return Err(Failure::read(error).unwrap_or_else(|| Failure::new("unknown_error", "Glance answered an error it did not say.")));
    }
    answer.get("result").cloned().ok_or_else(|| Failure::new("unknown_error", "Glance answered nothing."))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A call asked over the pipe is answered, a failure as one; a client of another Glance is refused before it asks
    /// anything; a second Glance cannot serve the pipe while one does; and once the interface is off, nothing answers.
    #[test]
    fn calls_are_asked_and_answered_over_the_pipe() {
        let server = super::super::server::Server::start().expect("the pipe made");
        let stop = Event::new();
        let failure = call("get_history", &json!({ "series": ["cpu.percnt"] }), &stop).unwrap_err();
        assert_eq!((failure.code, failure.message.as_str()), ("invalid_argument", "cpu.percnt is not a series. See the call's description for the series there are."));
        let second = call("get_history", &json!({ "series": [] }), &stop).unwrap_err();
        assert_eq!(second.code, "invalid_argument", "{}", second.message);
        assert_eq!(call("remove_files", &json!({}), &stop).unwrap_err().code, "unknown_tool");
        let refused = connect_saying(&json!({ "hello": 1, "glance": "0.0.1" }), &stop).err().unwrap();
        assert_eq!(refused.code, "version_mismatch");
        assert!(refused.message.contains("0.0.1"));
        assert!(super::super::server::Server::start().is_none(), "the name is held");
        server.stop();
        assert!(matches!(call("glance_status", &json!({}), &stop).unwrap_err().code, "glance_not_running" | "interface_off"));
        // Served again once it is on again.
        let server = super::super::server::Server::start().expect("the pipe made again");
        // Many clients at once, more than are served together, each one call after another: all answered.
        let threads: Vec<_> = (0..10)
            .map(|_| {
                std::thread::spawn(|| {
                    let stop = Event::new();
                    (0..8).map(|_| call("remove_files", &json!({}), &stop).unwrap_err().code).filter(|code| *code != "unknown_tool").count()
                })
            })
            .collect();
        assert_eq!(threads.into_iter().map(|thread| thread.join().unwrap()).sum::<usize>(), 0, "every call answered");
        assert_eq!(call("remove_files", &json!({}), &stop).unwrap_err().code, "unknown_tool");
        server.stop();
    }
}
