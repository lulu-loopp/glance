//! The clipboard opened by Glance's windows. One window at a time has it open, each only a moment: another program's,
//! or another of Glance's threads (the station reads it as soon as it changes, see `station::clipboard`). Opened as
//! someone acts (pastes, copies), it is waited for while another holds it, as Windows gives no way to be told when it is
//! let go: tried again and again for a while, else given up.

use std::time::{Duration, Instant};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HWND};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

/// The clipboard's text format (UTF-16, ended by a zero).
pub const TEXT: u32 = 13;

/// How long the clipboard held by another is waited for, and how often it is tried meanwhile.
const WAIT: Duration = Duration::from_millis(500);
const AGAIN: Duration = Duration::from_millis(10);

/// The clipboard open for a window of Glance's: closed as this is let go.
pub struct Opened(());

impl Drop for Opened {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

/// The clipboard opened for `owner`, waited for while another holds it: none if it is held all that while.
pub fn open(owner: HWND) -> Option<Opened> {
    let began = Instant::now();
    loop {
        if let Some(opened) = try_open(owner) {
            return Some(opened);
        }
        if began.elapsed() >= WAIT {
            return None;
        }
        std::thread::sleep(AGAIN);
    }
}

/// The clipboard opened for `owner` if no other holds it just now (for one who comes back for it later).
pub fn try_open(owner: HWND) -> Option<Opened> {
    unsafe { OpenClipboard(Some(owner)) }.ok().map(|()| Opened(()))
}

/// What was on the clipboard replaced by `formats` (each a format's number and its bytes, handed over whole, there at
/// once: none given only as it is asked for, by a window that may be gone by then), by window `owner`. Whether all of
/// them went there (none, the clipboard held by another all the while it was waited for).
pub fn put(owner: HWND, formats: &[(u32, Vec<u8>)]) -> bool {
    let Some(_opened) = open(owner) else { return false };
    unsafe {
        let _ = EmptyClipboard();
        formats.iter().fold(true, |all, (format, bytes)| hand_over(*format, bytes) && all)
    }
}

/// `bytes` handed to the clipboard (open) as `format`: whether it took them (the clipboard owns them from then; not
/// taken, they are let go of here).
unsafe fn hand_over(format: u32, bytes: &[u8]) -> bool {
    unsafe {
        let Ok(memory) = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) else { return false };
        let at = GlobalLock(memory);
        if at.is_null() {
            let _ = GlobalFree(Some(memory));
            return false;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), at.cast::<u8>(), bytes.len());
        let _ = GlobalUnlock(memory);
        if SetClipboardData(format, Some(HANDLE(memory.0))).is_ok() {
            true
        } else {
            let _ = GlobalFree(Some(memory));
            false
        }
    }
}

/// `text` as the clipboard keeps text (`TEXT`): UTF-16, ended by a zero.
pub fn utf16_z(text: &str) -> Vec<u8> {
    text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Held by another thread, the clipboard is waited for and had once that lets it go, and not had by one who will
    /// not wait.
    #[test]
    fn waits_for_the_clipboard_held_by_another() {
        use std::sync::mpsc::channel;
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE};
        // (A window of its own on each thread, as Glance's windows open it.)
        let window = || unsafe { CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, None, None) }.unwrap();
        let (held, holding) = channel();
        let other = std::thread::spawn(move || {
            let owner = window();
            let opened = open(owner).expect("the clipboard");
            held.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(150));
            drop(opened);
            unsafe {
                let _ = DestroyWindow(owner);
            }
        });
        holding.recv().unwrap();
        let owner = window();
        assert!(try_open(owner).is_none(), "held by the other thread");
        let began = Instant::now();
        let opened = open(owner);
        assert!(opened.is_some());
        assert!(began.elapsed() >= Duration::from_millis(100), "{:?}", began.elapsed());
        drop(opened);
        other.join().unwrap();
        unsafe {
            let _ = DestroyWindow(owner);
        }
    }
}
