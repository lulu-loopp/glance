//! An end of the interface's pipe, Glance's or a client's: read and written with overlapped I/O, so that each wait
//! also watches other events (Glance turning the interface off, a call done, a client giving up) and ends as they come.

use windows::Win32::Foundation::{CloseHandle, ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_PIPE_NOT_CONNECTED, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForMultipleObjects, INFINITE};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

/// The most a line may hold that Glance is asked (bytes, its end included): a path as long as Windows allows, written
/// out in JSON, fits.
pub const ASKED_MOST: usize = 256 << 10;
/// The most a line Glance answers may hold (bytes).
pub const ANSWER_MOST: usize = 2 << 20;

/// An event, set by hand and let be till it is reset: closed as it is let go.
pub struct Event(pub HANDLE);

// (An event's handle may be waited on and set from any thread.)
unsafe impl Send for Event {}
unsafe impl Sync for Event {}

impl Event {
    pub fn new() -> Event {
        Event(unsafe { CreateEventW(None, true, false, windows::core::PCWSTR::null()) }.expect("an event made"))
    }

    pub fn set(&self) {
        let _ = unsafe { SetEvent(self.0) };
    }

    pub fn is_set(&self) -> bool {
        unsafe { windows::Win32::System::Threading::WaitForSingleObject(self.0, 0) == WAIT_OBJECT_0 }
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// What waiting on an end came to.
#[derive(Debug, PartialEq, Eq)]
pub enum Came<T> {
    Done(T),
    /// The other end closed it (or it failed).
    Closed,
    /// The event at this place among those also waited on was set, before it was done (what it did meanwhile
    /// undone: a read, its bytes not taken).
    Woken(usize),
    /// The time waited for was over.
    Late,
}

/// A pipe's end, opened for overlapped I/O: closed as it is let go.
pub struct End(pub HANDLE);

unsafe impl Send for End {}
unsafe impl Sync for End {}

impl Drop for End {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

impl End {
    /// Some bytes read into `buffer`, as many as came, waiting till some come, the other end closes it, one of `also`
    /// is set, or `within` (ms, none: as long as it takes) is over.
    pub fn read(&self, buffer: &mut [u8], also: &[&Event], within: Option<u32>) -> Came<usize> {
        self.overlapped(also, within, |overlapped| unsafe { ReadFile(self.0, Some(buffer), None, Some(overlapped)) })
    }

    /// All of `bytes` written, waiting as for `read`.
    pub fn write_all(&self, mut bytes: &[u8], also: &[&Event]) -> Came<()> {
        while !bytes.is_empty() {
            match self.overlapped(also, None, |overlapped| unsafe { WriteFile(self.0, Some(bytes), None, Some(overlapped)) }) {
                Came::Done(0) => return Came::Closed,
                Came::Done(wrote) => bytes = &bytes[wrote..],
                Came::Closed => return Came::Closed,
                Came::Woken(at) => return Came::Woken(at),
                Came::Late => return Came::Late,
            }
        }
        Came::Done(())
    }

    /// What `begin` begins on this end, waited for as `read` says: the bytes it moved.
    fn overlapped(&self, also: &[&Event], within: Option<u32>, begin: impl FnOnce(*mut OVERLAPPED) -> windows::core::Result<()>) -> Came<usize> {
        let done = Event::new();
        let mut overlapped = OVERLAPPED { hEvent: done.0, ..Default::default() };
        let begun = begin(&mut overlapped);
        if let Err(error) = &begun {
            if error.code() != ERROR_IO_PENDING.to_hresult() {
                return Came::Closed;
            }
        }
        let mut events = vec![done.0];
        events.extend(also.iter().map(|event| event.0));
        let waited = unsafe { WaitForMultipleObjects(&events, false, within.unwrap_or(INFINITE)) };
        let finished = waited == WAIT_OBJECT_0;
        if !finished {
            // Undone, and waited for till it is: its buffer and its OVERLAPPED are not let go of while Windows writes them.
            let _ = unsafe { CancelIoEx(self.0, Some(&overlapped)) };
        }
        let mut moved = 0u32;
        let result = unsafe { GetOverlappedResult(self.0, &overlapped, &mut moved, true) };
        match result {
            // (Done as it was undone: what it did stands.)
            Ok(()) => Came::Done(moved as usize),
            Err(error) if error.code() == ERROR_BROKEN_PIPE.to_hresult() || error.code() == ERROR_PIPE_NOT_CONNECTED.to_hresult() => Came::Closed,
            Err(_) if waited == WAIT_TIMEOUT => Came::Late,
            Err(_) => match (waited.0 as usize).checked_sub(WAIT_OBJECT_0.0 as usize + 1) {
                Some(at) if !finished && at < also.len() => Came::Woken(at),
                _ => Came::Closed,
            },
        }
    }
}

/// Lines read from an end, as they come: none longer than `most` bytes.
pub struct Lines {
    held: Vec<u8>,
    most: usize,
}

/// What reading a line came to.
#[derive(Debug, PartialEq, Eq)]
pub enum Line {
    Read(Vec<u8>),
    /// Longer than allowed.
    TooLong,
    Closed,
    Woken(usize),
    Late,
}

impl Lines {
    pub fn new(most: usize) -> Lines {
        Lines { held: Vec::new(), most }
    }

    /// The next line (without its end), read from `end` as `End::read` waits (`within` from now, for the whole line).
    pub fn next(&mut self, end: &End, also: &[&Event], within: Option<u32>) -> Line {
        let until = within.map(|ms| std::time::Instant::now() + std::time::Duration::from_millis(ms.into()));
        let mut buffer = vec![0u8; 64 << 10];
        loop {
            if let Some(at) = self.held.iter().position(|&byte| byte == b'\n') {
                // (Its end counted in: no longer than allowed, however it came in.)
                if at + 1 > self.most {
                    return Line::TooLong;
                }
                let mut line: Vec<u8> = self.held.drain(..=at).collect();
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return Line::Read(line);
            }
            if self.held.len() >= self.most {
                return Line::TooLong;
            }
            let left = until.map(|until| until.saturating_duration_since(std::time::Instant::now()).as_millis() as u32);
            if left == Some(0) {
                return Line::Late;
            }
            match end.read(&mut buffer, also, left) {
                Came::Done(0) | Came::Closed => return Line::Closed,
                Came::Done(read) => self.held.extend_from_slice(&buffer[..read]),
                Came::Woken(at) => return Line::Woken(at),
                Came::Late => return Line::Late,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_NONE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX};
    use windows::Win32::System::Pipes::{CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT};

    /// The two ends of a pipe of this test's own.
    fn ends(name: &str) -> (End, End) {
        let name = HSTRING::from(format!(r"\\.\pipe\glance.test.{}.{name}", std::process::id()));
        let server = unsafe { CreateNamedPipeW(&name, PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT, 1, 1 << 16, 1 << 16, 0, None) };
        let client = unsafe { CreateFileW(&name, (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0, FILE_SHARE_NONE, None, OPEN_EXISTING, FILE_FLAG_OVERLAPPED, None) }.unwrap();
        (End(server), End(client))
    }

    /// A line is no longer than allowed with its end, however it comes in: one byte more is too long, even where it
    /// came whole in one read.
    #[test]
    fn a_line_longer_than_allowed_is_not_read() {
        let (server, client) = ends("long");
        let mut fitting = vec![b'a'; 9];
        fitting.push(b'\n');
        let mut longer = vec![b'b'; 10];
        longer.push(b'\n');
        assert_eq!(client.write_all(&[fitting, longer].concat(), &[]), Came::Done(()));
        let mut lines = Lines::new(10);
        assert_eq!(lines.next(&server, &[], Some(1000)), Line::Read(vec![b'a'; 9]));
        assert_eq!(lines.next(&server, &[], Some(1000)), Line::TooLong);
        drop(client);
        assert_eq!(Lines::new(10).next(&server, &[], Some(1000)), Line::Closed);
    }
}
