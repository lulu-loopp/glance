//! What is dragged off the station into another program: given in the formats it is copied in (see
//! `clipboard::formats_of`), the receiving program choosing among them; files as a copy (the station never has them
//! moved away by a drag: moved, they would be gone from where it points to). Words are given as a text file too
//! (named by their first line), for what takes files only (Explorer, the desktop): a program that takes words takes
//! them as words.
//!
//! The station gives Windows no picture of what is dragged: Glance carries it on the screen itself (see `carry`), told
//! as the drag goes whether the program under the pointer would take it there and what that program says a drop does
//! ("复制到 下载": see `Going`). A program says so by setting a description on what is dragged (Explorer does, with no
//! picture given too, run elevated or not alike).

use super::clipboard::{self, Content};
use windows::core::{implement, BOOL, HRESULT};
use windows::Win32::Foundation::{GlobalFree, HGLOBAL};
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{DoDragDrop, IDropSource, IDropSource_Impl, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::SHCreateDataObject;

/// What came of a drag off the station.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dragged {
    /// Dropped on a program that took it.
    Taken,
    /// Let go where nothing took it, or given up (Esc).
    Not,
}

/// How the drag off the station asked for last goes: its number (see `announce`), what is under the pointer (see
/// `Over`), and how it ended (none: under way).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Going {
    pub drag: u64,
    pub over: Over,
    pub ended: Option<Dragged>,
}

/// What is under the pointer as something is dragged off the station: whether a drop there would be taken, and what the
/// program there says it does (as it says it, in its own language).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Over {
    pub takes: bool,
    pub says: Option<String>,
}

static GOING: std::sync::Mutex<Going> = std::sync::Mutex::new(Going { drag: 0, over: Over { takes: false, says: None }, ended: Some(Dragged::Not) });

/// A drag off the station asked for: its number, as it is followed (see `going`).
pub fn announce() -> u64 {
    let mut going = GOING.lock().unwrap();
    *going = Going { drag: going.drag + 1, over: Over::default(), ended: None };
    going.drag
}

/// How the drag off the station asked for last goes.
pub fn going() -> Going {
    GOING.lock().unwrap().clone()
}

/// Whether a drag off the station is under way: the station takes nothing of it back (see `drop::Target`).
pub fn under_way() -> bool {
    GOING.lock().unwrap().ended.is_none()
}

/// The drag off the station asked for last ended so.
pub fn ended(dragged: Dragged) {
    GOING.lock().unwrap().ended = Some(dragged);
}

/// Drags `content` off the station (`stays_here`: see `shelf::Item::stays_here`) till it is dropped or given up, on
/// this thread (which must be the station's: in a single-threaded apartment, its messages pumped meanwhile by
/// Windows), pressed on a window of the thread `pressed_on`: whose input this thread shares meanwhile (the pointer
/// is that thread's while its button is down; not shared, the drag saw it neither move nor let go). None where it
/// could not be begun.
pub fn drag(content: &Content, stays_here: bool, pressed_on: u32) -> Option<Dragged> {
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    let this = unsafe { GetCurrentThreadId() };
    let shared = pressed_on != this && unsafe { AttachThreadInput(this, pressed_on, true) }.as_bool();
    let dragged = drag_here(content, stays_here);
    if shared {
        let _ = unsafe { AttachThreadInput(this, pressed_on, false) };
    }
    dragged
}

/// As `drag`, its input this thread's.
fn drag_here(content: &Content, stays_here: bool) -> Option<Dragged> {
    let formats = clipboard::formats_of(content, stays_here)?;
    unsafe {
        // The shell's own data object, which keeps whatever it is given (and gives the receiving program what it
        // asks of it as Explorer's do).
        let data: IDataObject = SHCreateDataObject(None, None, None).ok()?;
        // (Each format with the index of the file it is of: -1 for all but a text file's contents, its first and only.)
        let mut formats: Vec<(u32, Vec<u8>, i32)> = formats.into_iter().map(|(format, bytes)| (format, bytes, -1)).collect();
        if let Content::Text(text) = content {
            formats.extend(as_text_file(text));
        }
        for (format, bytes, index) in &formats {
            let Some(memory) = global(bytes) else { return None };
            let asked = FORMATETC { cfFormat: *format as u16, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: *index, tymed: TYMED_HGLOBAL.0 as u32 };
            let medium = STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: memory }, pUnkForRelease: std::mem::ManuallyDrop::new(None) };
            // (The data object owns it from here; not taken, it is let go of here.)
            if data.SetData(&asked, &medium, true).is_err() {
                let _ = GlobalFree(Some(memory));
                return None;
            }
        }
        let source: IDropSource = Source { data: data.clone(), said: Default::default() }.into();
        let mut effect = DROPEFFECT_NONE;
        let said = DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect);
        Some(if said == windows::Win32::Foundation::DRAGDROP_S_DROP && effect != DROPEFFECT_NONE { Dragged::Taken } else { Dragged::Not })
    }
}

/// `text` as a file dropped where only files are taken: its description (named by its first line, see `text_name`)
/// and its contents (UTF-8, marked so), each format's number, bytes, and the index of the file it is of.
fn as_text_file(text: &str) -> Vec<(u32, Vec<u8>, i32)> {
    use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
    use windows::Win32::UI::Shell::{FD_FILESIZE, FD_PROGRESSUI, FD_UNICODE, FILEDESCRIPTORW, FILEGROUPDESCRIPTORW};
    let contents: Vec<u8> = [0xEF, 0xBB, 0xBF].into_iter().chain(text.bytes()).collect();
    // (Its name in a copy of its own: the descriptor is packed, its fields not to be borrowed.)
    let mut name = [0u16; 260];
    for (to, unit) in name.iter_mut().zip(text_name(text).encode_utf16().take(259)) {
        *to = unit;
    }
    let file = FILEDESCRIPTORW { dwFlags: (FD_FILESIZE.0 | FD_PROGRESSUI.0 | FD_UNICODE.0) as u32, nFileSizeLow: contents.len() as u32, cFileName: name, ..Default::default() };
    let group = FILEGROUPDESCRIPTORW { cItems: 1, fgd: [file] };
    let described = unsafe { std::slice::from_raw_parts((&group as *const FILEGROUPDESCRIPTORW).cast::<u8>(), std::mem::size_of::<FILEGROUPDESCRIPTORW>()) }.to_vec();
    let (descriptor, contents_format) = unsafe { (RegisterClipboardFormatW(windows::core::w!("FileGroupDescriptorW")), RegisterClipboardFormatW(windows::core::w!("FileContents"))) };
    vec![(descriptor, described, -1), (contents_format, contents, 0)]
}

/// The name a text file of `text` is given: its first line (as much of it as a name holds, as it may hold it: see
/// `os::file_name_of`), and ".txt"; "文本.txt" where nothing of it is left.
pub(crate) fn text_name(text: &str) -> String {
    let first = crate::os::file_name_of(text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default(), 40);
    format!("{}.txt", if first.is_empty() { "文本" } else { &first })
}

/// `bytes` in global memory of their own (to be handed over).
unsafe fn global(bytes: &[u8]) -> Option<HGLOBAL> {
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)).ok()?;
        let at = GlobalLock(memory);
        if at.is_null() {
            let _ = GlobalFree(Some(memory));
            return None;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), at.cast::<u8>(), bytes.len());
        let _ = GlobalUnlock(memory);
        Some(memory)
    }
}

#[cfg(test)]
mod tests {
    /// What Explorer says a drop does, as it sets it: its words, the place named where it says `%1`; none where it took
    /// it back (as it does by turns over one folder) or said nothing.
    #[test]
    fn says_what_a_drop_does_as_the_program_said_it() {
        use windows::Win32::UI::Shell::{DROPDESCRIPTION, DROPIMAGE_COPY, DROPIMAGE_INVALID, DROPIMAGETYPE};
        let described = |kind: DROPIMAGETYPE, message: &str, insert: &str| {
            let (mut words, mut place) = ([0u16; 260], [0u16; 260]);
            for (to, unit) in words.iter_mut().zip(message.encode_utf16()) {
                *to = unit;
            }
            for (to, unit) in place.iter_mut().zip(insert.encode_utf16()) {
                *to = unit;
            }
            super::said_by(&DROPDESCRIPTION { r#type: kind, szMessage: words, szInsert: place })
        };
        assert_eq!(described(DROPIMAGE_COPY, "复制到 %1", "下载"), Some("复制到 下载".into()));
        assert_eq!(described(DROPIMAGE_COPY, "Copy to %1", "dropdir"), Some("Copy to dropdir".into()));
        assert_eq!(described(DROPIMAGE_COPY, "Copy", ""), Some("Copy".into()));
        assert_eq!(described(DROPIMAGE_INVALID, "", ""), None);
        assert_eq!(described(DROPIMAGE_INVALID, "Copy to %1", "dropdir"), None);
        assert_eq!(described(DROPIMAGE_COPY, "", "dropdir"), None);
    }

    /// What a drop does is said on through the moments the program says nothing, over the same window taking it still;
    /// over another window, or one not taking it, not.
    #[test]
    fn what_a_drop_does_is_kept_over_the_window_that_said_it() {
        let copy = || Some("复制到 下载".to_string());
        let was = (7, copy());
        assert_eq!(super::kept_said(&was, 7, true, None), (7, copy()));
        assert_eq!(super::kept_said(&was, 7, true, Some("复制到 桌面".into())), (7, Some("复制到 桌面".into())));
        assert_eq!(super::kept_said(&was, 8, true, None), (8, None));
        assert_eq!(super::kept_said(&was, 7, false, None), (7, None));
        assert_eq!(super::kept_said(&(0, None), 7, true, copy()), (7, copy()));
    }

    /// Words dragged as a file are named by their first line that has any, what a file's name may not hold left out.
    #[test]
    fn names_a_text_file_by_its_first_line() {
        assert_eq!(super::text_name("\n  会议改到周四下午三点\n地点：二楼"), "会议改到周四下午三点.txt");
        assert_eq!(super::text_name("a/b:c*d?\"e<f>g|h"), "abcdefgh.txt");
        assert_eq!(super::text_name("..."), "文本.txt");
        assert_eq!(super::text_name(""), "文本.txt");
        assert_eq!(super::text_name(&"长".repeat(60)).chars().count(), 44);
    }
}

/// The station as the source of a drag: dropped as the left button is let go, given up by Esc; the system's cursors.
/// What is dragged, and what the program under the pointer last said a drop does, and that window (see `kept_said`).
#[implement(IDropSource)]
struct Source {
    data: IDataObject,
    said: std::cell::RefCell<(isize, Option<String>)>,
}

impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        // (The button and Esc as they are, not only as the drag's messages say: pressed on another thread's window,
        // those may never come, and the drag never ended.)
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE, VK_LBUTTON};
        let down = |key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| unsafe { GetAsyncKeyState(i32::from(key.0)) } < 0;
        if escape.as_bool() || down(VK_ESCAPE) {
            windows::Win32::Foundation::DRAGDROP_S_CANCEL
        } else if keys.0 & MK_LBUTTON.0 == 0 || !down(VK_LBUTTON) {
            windows::Win32::Foundation::DRAGDROP_S_DROP
        } else {
            HRESULT(0)
        }
    }

    /// Told as the pointer moves what the program under it would do with a drop there: told on (see `Going`).
    fn GiveFeedback(&self, effect: DROPEFFECT) -> HRESULT {
        use windows::Win32::System::Ole::{DROPEFFECT_LINK, DROPEFFECT_MOVE};
        use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, GetCursorPos, WindowFromPoint, GA_ROOT};
        let takes = effect.0 & (DROPEFFECT_COPY.0 | DROPEFFECT_MOVE.0 | DROPEFFECT_LINK.0) != 0;
        let mut cursor = windows::Win32::Foundation::POINT::default();
        let window = match unsafe { GetCursorPos(&mut cursor) } {
            Ok(()) => unsafe { GetAncestor(WindowFromPoint(cursor), GA_ROOT) }.0 as isize,
            Err(_) => 0,
        };
        let mut said = self.said.borrow_mut();
        *said = kept_said(&said, window, takes, description(&self.data));
        GOING.lock().unwrap().over = Over { takes, says: said.1.clone().filter(|_| takes) };
        windows::Win32::Foundation::DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// What a drop does as the program under the pointer last said (with the window that was under it, as `was` has it),
/// `window` under it now, taking a drop there (`takes`), and saying `said` now: what it says, else what it said, while
/// it is the same window and takes it still (Explorer, over one folder, says it and says nothing by turns as the
/// pointer moves; Windows' own picture of what is dragged said it throughout).
fn kept_said(was: &(isize, Option<String>), window: isize, takes: bool, said: Option<String>) -> (isize, Option<String>) {
    let kept = was.1.clone().filter(|_| was.0 == window && takes);
    (window, said.or(kept))
}

/// What the program under the pointer says a drop does, as it set it on what is dragged (none, or one it took back).
fn description(data: &IDataObject) -> Option<String> {
    use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
    use windows::Win32::System::Memory::GlobalSize;
    use windows::Win32::System::Ole::ReleaseStgMedium;
    use windows::Win32::UI::Shell::DROPDESCRIPTION;
    let format = unsafe { RegisterClipboardFormatW(windows::core::w!("DropDescription")) } as u16;
    let asked = FORMATETC { cfFormat: format, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
    let mut medium = unsafe { data.GetData(&asked) }.ok()?;
    let read = unsafe {
        let memory = medium.u.hGlobal;
        let at = GlobalLock(memory);
        let said = (!at.is_null() && GlobalSize(memory) >= std::mem::size_of::<DROPDESCRIPTION>()).then(|| std::ptr::read_unaligned(at.cast::<DROPDESCRIPTION>()));
        let _ = GlobalUnlock(memory);
        said
    };
    unsafe { ReleaseStgMedium(&mut medium) };
    said_by(&read?)
}

/// What a drop description says: its words, `%1` in them the place it names (none where it is taken back, or empty).
fn said_by(description: &windows::Win32::UI::Shell::DROPDESCRIPTION) -> Option<String> {
    use windows::Win32::UI::Shell::DROPIMAGE_INVALID;
    let words = |units: &[u16]| String::from_utf16_lossy(&units[..units.iter().position(|&unit| unit == 0).unwrap_or(units.len())]);
    let (kind, message, insert) = (description.r#type, description.szMessage, description.szInsert);
    let message = words(&message);
    (kind != DROPIMAGE_INVALID && !message.is_empty()).then(|| message.replace("%1", &words(&insert)))
}
