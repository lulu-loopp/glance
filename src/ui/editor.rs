//! A box words are written in, written as Windows writes them: its RichEdit text services, windowless.
//! They take the keys, the input method's words, the selection and undo; Glance hosts them in a window it draws itself,
//! and they draw their words with Direct2D into what Glance draws (`ITextServices2::TxDrawD2D`), the caret drawn by
//! Glance where they say it is. What is chosen is copied by Glance (see `Editor::copy`).
//!
//! Everything here is in the window's physical pixels, as the text services count; the window's thread is a
//! single-threaded apartment.

use std::cell::Cell;
use std::rc::Rc;

use windows::core::{implement, Interface, BOOL, BSTR, GUID, HSTRING};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, RECTL, SIZE, WPARAM};
use windows::Win32::Graphics::Direct2D::ID2D1RenderTarget;
use windows::Win32::Graphics::Gdi::{GetDC, GetSysColor, InvalidateRect, ReleaseDC, ScreenToClient, ClientToScreen, HBITMAP, HDC, HPALETTE, HRGN, SYS_COLOR_INDEX};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Controls::RichEdit::*;
use windows::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_UNAWARE};
use windows::Win32::UI::Input::Ime::{ImmGetContext, ImmReleaseContext, HIMC};
use windows::Win32::UI::Input::KeyboardAndMouse::{SetCapture, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetCursor, SetForegroundWindow, SetTimer, HCURSOR, SCROLLBAR_CONSTANTS, SCROLL_WINDOW_FLAGS, WS_VSCROLL};

/// The edit control's message asking which character is at a point (winuser.h's `EM_CHARFROMPOS`; the bindings leave it
/// out): a rich edit box answers with the character's place.
const EM_CHARFROMPOS: u32 = 0x00D7;
/// The edit control's message putting words in place of what is chosen (winuser.h's `EM_REPLACESEL`; the bindings leave
/// it out): its WPARAM whether undo can take it back.
const EM_REPLACESEL: u32 = 0x00C2;
/// How far apart the text services set tabs, in the pixels they draw in (their default, half an inch: 720 twips at
/// 96 pixels an inch, whatever the screen's scale; see `new`).
pub const TAB_PIXELS: f32 = 48.0;

/// What the host keeps for the text services, and what they tell it.
struct Kept {
    hwnd: HWND,
    /// Where the words are written in the window.
    rect: Cell<RECT>,
    /// The caret: where, how large, whether shown.
    caret: Cell<Option<(i32, i32, i32, i32)>>,
    caret_size: Cell<(i32, i32)>,
    caret_shown: Cell<bool>,
    /// The words' look as written from here on, and their paragraphs'.
    char_format: Box<CHARFORMAT2W>,
    para_format: Box<PARAFORMAT>,
    /// What was written changed since it was last asked.
    changed: Cell<bool>,
    /// Words on one line alone, plain (a field's: see `Editor::line`), scrolled along as they are written past its end.
    one_line: bool,
}

#[implement(ITextHost2, ITextHost)]
struct Host {
    kept: Rc<Kept>,
}

/// The colours and faces a box's words are dressed in (see `Editor::dress`): its words', faint, the accent; its size
/// (points, as its words are written); its face, and code's.
pub struct Palette {
    pub text: COLORREF,
    pub faint: COLORREF,
    pub accent: COLORREF,
    pub points: f32,
    pub face: String,
    pub code_face: String,
}

/// A box words are written in.
pub struct Editor {
    /// (Let go of by `ShutdownTextServices`, as it goes: see `drop`.)
    services: std::mem::ManuallyDrop<ITextServices2>,
    _host: ITextHost2,
    kept: Rc<Kept>,
    /// What the keys last did (see `send`).
    typed: Cell<Typed>,
    /// Whether the input method is composing (its words are its till they are written), and whether a composition
    /// ended since this was last asked (see `take_composed`).
    composing: Cell<bool>,
    composed: Cell<bool>,
}

/// What the keys last did to the words: nothing yet (or the input method gave words), wrote, erased. Undo takes back one
/// kind at a time: the text services gather writing and the erasing after it into one step, as a word processor does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Typed {
    Nothing,
    Wrote,
    Erased,
}

/// Whether `message` copies what is chosen (`Some(false)`) or cuts it (`Some(true)`): asked for (`WM_COPY`, `WM_CUT`),
/// or by its keys, Ctrl+C and Ctrl+Insert, Ctrl+X and Shift+Delete. (See `Editor::copy`.)
fn copying(message: u32, wparam: WPARAM) -> Option<bool> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VIRTUAL_KEY, VK_C, VK_CONTROL, VK_DELETE, VK_INSERT, VK_MENU, VK_SHIFT, VK_X};
    use windows::Win32::UI::WindowsAndMessaging::{WM_COPY, WM_CUT, WM_KEYDOWN};
    match message {
        WM_COPY => return Some(false),
        WM_CUT => return Some(true),
        WM_KEYDOWN => {}
        _ => return None,
    }
    let held = |key: VIRTUAL_KEY| unsafe { GetKeyState(i32::from(key.0)) } < 0;
    match (VIRTUAL_KEY(wparam.0 as u16), held(VK_CONTROL), held(VK_SHIFT), held(VK_MENU)) {
        (VK_C | VK_INSERT, true, false, false) => Some(false),
        (VK_X, true, false, false) | (VK_DELETE, false, true, false) => Some(true),
        _ => None,
    }
}

/// The text services' maker, and the interface they are asked for, as msftedit.dll gives them.
fn services_maker() -> Option<(PCreateTextServices, GUID)> {
    unsafe {
        let library = LoadLibraryW(windows::core::w!("msftedit.dll")).ok()?;
        let make: PCreateTextServices = std::mem::transmute(GetProcAddress(library, windows::core::s!("CreateTextServices"))?);
        let iid = GetProcAddress(library, windows::core::s!("IID_ITextServices2"))?;
        Some((make, *(iid as *const GUID)))
    }
}

/// The ink the text services fill what is chosen with as they draw it (`ITextServices2::TxDrawD2D`): the system's
/// highlight colour two fifths over white, solid, its words drawn in their own colours on it (as measured: highlight
/// 0,120,212 filled 153,201,238, on a note's yellow and on a block of code's box alike). For what Glance draws over
/// words put away (a note's table), chosen with them.
pub fn chosen_ink() -> super::canvas::Color {
    let highlight = unsafe { GetSysColor(windows::Win32::Graphics::Gdi::COLOR_HIGHLIGHT) };
    let over_white = |channel: u32| (0.4 * (channel & 0xFF) as f32 + 0.6 * 255.0) / 255.0;
    super::canvas::Color { r: over_white(highlight), g: over_white(highlight >> 8), b: over_white(highlight >> 16), a: 1.0 }
}

impl Editor {
    /// A box in `hwnd` at `rect`, its words in `face` `pixels` high, of `color`; none where the text services cannot
    /// be had.
    pub fn new(hwnd: HWND, rect: RECT, face: &str, pixels: f32, color: COLORREF) -> Option<Editor> {
        Editor::made(hwnd, rect, face, pixels, color, false)
    }

    /// As `new`, a field's: its words plain (as typed or pasted, dressed in nothing), on one line alone, scrolled along
    /// as they are written past its end.
    pub fn line(hwnd: HWND, rect: RECT, face: &str, pixels: f32, color: COLORREF) -> Option<Editor> {
        Editor::made(hwnd, rect, face, pixels, color, true)
    }

    fn made(hwnd: HWND, rect: RECT, face: &str, pixels: f32, color: COLORREF, one_line: bool) -> Option<Editor> {
        let mut char_format = Box::new(CHARFORMAT2W::default());
        char_format.Base.cbSize = std::mem::size_of::<CHARFORMAT2W>() as u32;
        char_format.Base.dwMask = CFM_SIZE | CFM_FACE | CFM_COLOR | CFM_CHARSET;
        // (Twips: the text services' DPI is the screen's, 96 per inch of pixels as they draw here.)
        char_format.Base.yHeight = (pixels * 15.0).round() as i32;
        char_format.Base.crTextColor = color;
        char_format.Base.bCharSet = windows::Win32::Graphics::Gdi::DEFAULT_CHARSET;
        for (unit, to) in face.encode_utf16().zip(char_format.Base.szFaceName.iter_mut().take(31)) {
            *to = unit;
        }
        let mut para_format = Box::new(PARAFORMAT::default());
        para_format.cbSize = std::mem::size_of::<PARAFORMAT>() as u32;
        para_format.dwMask = PFM_ALL;
        para_format.wAlignment = PFA_LEFT;
        let kept = Rc::new(Kept { hwnd, rect: Cell::new(rect), caret: Cell::new(None), caret_size: Cell::new((1, 16)), caret_shown: Cell::new(false), char_format, para_format, changed: Cell::new(false), one_line });
        let host: ITextHost2 = Host { kept: kept.clone() }.into();
        let (make, iid) = services_maker()?;
        let make = make?;
        unsafe {
            let mut made: Option<windows::core::IUnknown> = None;
            let plain: ITextHost = host.cast().ok()?;
            // (Made as a thread unaware of its screen's scale would make them: they take the scale they count in as
            // they are made, and count in pixels at 96 per inch as the words are drawn (see `draw`). Made at the
            // window's scale, they would place the caret, and the input method's window, at its multiple of where the
            // words are.)
            let was = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_UNAWARE);
            let made_now = make(None.into(), (&plain).into(), windows::core::OutRef::from(&mut made));
            SetThreadDpiAwarenessContext(was);
            made_now.ok().ok()?;
            let made = made?;
            let mut services = std::ptr::null_mut();
            made.query(&iid, &mut services).ok().ok()?;
            let services = ITextServices2::from_raw(services);
            let mut client = rect;
            let _ = services.OnTxInPlaceActivate(&mut client);
            let _ = services.OnTxUIActivate();
            let mut result = LRESULT(0);
            let _ = services.TxSendMessage(EM_SETEVENTMASK, WPARAM(0), LPARAM(ENM_CHANGE as isize), &mut result);
            let editor = Editor { services: std::mem::ManuallyDrop::new(services), _host: host, kept, typed: Cell::new(Typed::Nothing), composing: Cell::new(false), composed: Cell::new(false) };
            // Its words all in its face, none put in another by the text services (as Chinese, in a face without it,
            // went into a taller one, its lines standing taller than the rest): what the face has not, drawn by
            // DirectWrite from the faces it falls back on, in the line as the face lays it.
            let options = editor.answer(EM_GETLANGOPTIONS, WPARAM(0), LPARAM(0)).map_or(0, |options| options.0 as u32);
            editor.answer(EM_SETLANGOPTIONS, WPARAM(0), LPARAM((options & !(IMF_AUTOFONT | IMF_DUALFONT | IMF_AUTOFONTSIZEADJUST)) as isize));
            Some(editor)
        }
    }

    /// The top of the line the character at `at` is on (in the window).
    #[cfg(test)]
    fn top_of(&self, at: i32) -> i32 {
        let mut point = windows::Win32::Foundation::POINTL::default();
        // (EM_POSFROMCHAR, which the bindings leave out.)
        self.answer(0x00D6, WPARAM(&mut point as *mut _ as usize), LPARAM(at as isize));
        point.y
    }

    /// What is written.
    pub fn text(&self) -> String {
        let mut text = BSTR::new();
        unsafe {
            let _ = self.services.TxGetText(&mut text);
        }
        // (The text services end each paragraph with a carriage return alone.)
        text.to_string().replace("\r\n", "\n").replace('\r', "\n")
    }

    /// What is written but the paragraph mark the text services end it with (see `text`): what can be written in, as
    /// an edit of it is made (see `apply`: an edit made of `text` could reach past that mark, where nothing goes).
    pub fn words(&self) -> String {
        let mut text = self.text();
        if text.ends_with('\n') {
            text.pop();
        }
        text
    }

    /// `text` written in it, in place of what was: given back as it was (see `text`).
    pub fn set_text(&self, text: &str) {
        // (Each line break one carriage return, as the text services keep them: Windows' pairs as one too. The last
        // line break is the paragraph mark they end what is written with, given back as one by `text`: written as a
        // line break too, it was a line more each time a note was opened and kept.)
        let text = text.replace("\r\n", "\n");
        let text = text.strip_suffix('\n').unwrap_or(&text);
        unsafe {
            let _ = self.services.TxSetText(&HSTRING::from(text.replace('\n', "\r")));
        }
        self.kept.changed.set(false);
    }

    /// Whether what is written changed since `take_changed` was last asked (not asking it).
    pub fn changed(&self) -> bool {
        self.kept.changed.get()
    }

    /// Whether what is written changed since this was last asked.
    pub fn take_changed(&self) -> bool {
        self.kept.changed.replace(false)
    }

    /// How far its words are scrolled down (pixels).
    pub fn scrolled(&self) -> i32 {
        let mut at = POINT::default();
        self.send(EM_GETSCROLLPOS, WPARAM(0), LPARAM(&mut at as *mut POINT as isize));
        at.y
    }

    /// Its words scrolled `down` pixels down (as far as they go).
    pub fn scroll_to(&self, down: i32) {
        // (Laid out to their end first: the text services lay out only as far as they have drawn, and scrolled before
        // that, stopped at what was laid out.)
        let _ = self.character_anywhere(self.text().encode_utf16().count().saturating_sub(1));
        let mut at = POINT { x: 0, y: down.max(0) };
        self.send(EM_SETSCROLLPOS, WPARAM(0), LPARAM(&mut at as *mut POINT as isize));
    }

    /// A window message for it (the keys, the input method's, the pointer's, its timers'): what it answers.
    /// (None: not theirs, `S_FALSE`, for the window's own handling to see to, as an input method's message may be.)
    /// Writing after erasing, erasing after writing, and what the input method gives each begin a step of their own
    /// for undo to take back.
    pub fn send(&self, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        use windows::Win32::UI::Input::Ime::GCS_RESULTSTR;
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_BACK, VK_DELETE};
        use windows::Win32::UI::WindowsAndMessaging::{WM_CHAR, WM_IME_COMPOSITION, WM_IME_ENDCOMPOSITION, WM_IME_STARTCOMPOSITION, WM_KEYDOWN};
        if let Some(cut) = copying(message, wparam) {
            self.copy(cut);
            return Some(LRESULT(0));
        }
        let now = match message {
            WM_KEYDOWN if wparam.0 == usize::from(VK_BACK.0) || wparam.0 == usize::from(VK_DELETE.0) => Some(Typed::Erased),
            WM_CHAR if wparam.0 >= 0x20 && wparam.0 != 0x7F => Some(Typed::Wrote),
            WM_IME_COMPOSITION if lparam.0 as u32 & GCS_RESULTSTR.0 != 0 => Some(Typed::Nothing),
            _ => None,
        };
        if let Some(now) = now {
            if now != self.typed.replace(now) || now == Typed::Nothing {
                self.answer(EM_STOPGROUPTYPING, WPARAM(0), LPARAM(0));
            }
        }
        let answer = self.answer(message, wparam, lparam);
        // (What the input method gave, a step of its own: what is written after it begins another.)
        if now == Some(Typed::Nothing) {
            self.answer(EM_STOPGROUPTYPING, WPARAM(0), LPARAM(0));
        }
        match message {
            WM_IME_STARTCOMPOSITION => self.composing.set(true),
            WM_IME_ENDCOMPOSITION => {
                self.composing.set(false);
                self.composed.set(true);
            }
            _ => {}
        }
        answer
    }

    /// What is chosen put on the clipboard as the text services give it out (`EM_STREAMOUT`): its words as they show (what
    /// is hidden left out), and as rich text (their look kept, for a program that reads it). Cut (`cut`): taken out of
    /// it then, a step of its own for undo to take back. Nothing chosen, nothing done.
    ///
    /// (Not left to the text services: on a thread of the process's multithreaded apartment, as the panel's is (see
    /// lib.rs), they put on the clipboard only a promise to give their words as they are asked for, kept by the box. Once
    /// the box was gone (a note on the desktop written in no longer), nothing could be pasted.)
    pub fn copy(&self, cut: bool) {
        let (from, to) = self.selection();
        if from == to {
            return;
        }
        let words: Vec<u16> = self.streamed(SF_TEXT | SF_UNICODE).chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
        // (A line break as Windows' programs read one: a carriage return and a line feed.)
        let words = String::from_utf16_lossy(&words).replace("\r\n", "\n").replace('\r', "\n").replace('\n', "\r\n");
        let mut rich = self.streamed(SF_RTF);
        rich.push(0);
        let rich_format = unsafe { windows::Win32::System::DataExchange::RegisterClipboardFormatW(windows::core::w!("Rich Text Format")) };
        crate::clipboard::put(self.kept.hwnd, &[(crate::clipboard::TEXT, crate::clipboard::utf16_z(&words)), (rich_format, rich)]);
        if cut {
            self.answer(EM_STOPGROUPTYPING, WPARAM(0), LPARAM(0));
            // (Undone as one: EM_REPLACESEL's WPARAM.)
            let none = [0u16];
            self.answer(EM_REPLACESEL, WPARAM(1), LPARAM(none.as_ptr() as isize));
            self.answer(EM_STOPGROUPTYPING, WPARAM(0), LPARAM(0));
            self.typed.set(Typed::Nothing);
            self.kept.changed.set(true);
        }
    }

    /// What is chosen as the text services give it out, as `format` says (`EM_STREAMOUT`'s, of what is chosen).
    fn streamed(&self, format: u32) -> Vec<u8> {
        unsafe extern "system" fn taken(cookie: usize, bytes: *mut u8, count: i32, done: *mut i32) -> u32 {
            unsafe {
                let into = &mut *(cookie as *mut Vec<u8>);
                into.extend_from_slice(std::slice::from_raw_parts(bytes, count.max(0) as usize));
                *done = count;
            }
            0
        }
        let mut out: Vec<u8> = Vec::new();
        let mut stream = EDITSTREAM { dwCookie: &mut out as *mut Vec<u8> as usize, dwError: 0, pfnCallback: Some(taken) };
        self.answer(EM_STREAMOUT, WPARAM((format | SFF_SELECTION) as usize), LPARAM(&mut stream as *mut EDITSTREAM as isize));
        out
    }

    /// Whether the input method is composing in it.
    pub fn composing(&self) -> bool {
        self.composing.get()
    }

    /// Whether a composition ended since this was last asked.
    pub fn take_composed(&self) -> bool {
        self.composed.replace(false)
    }

    /// The pointer's shape over it, the pointer at `point` (in the window), as the text services choose it (the I-beam
    /// over words, the arrow over what is selected): asked of them as a host of windowless text services asks
    /// (`OnTxSetCursor`). (Given `WM_SETCURSOR` as a message instead, they ended the input method's composition each
    /// time it came, and with the pointer resting on the words it comes as the input method's windows come and go:
    /// each syllable was written as it was typed, in Latin letters.)
    pub fn set_cursor(&self, point: POINT) {
        let mut rect = self.kept.rect.get();
        unsafe {
            let _ = self.services.OnTxSetCursor(windows::Win32::System::Com::DVASPECT_CONTENT, -1, std::ptr::null_mut(), std::ptr::null_mut(), HDC::default(), HDC::default(), &mut rect, point.x, point.y);
        }
    }

    /// `message` handled by the text services: what they answer.
    fn answer(&self, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        let mut result = LRESULT(0);
        // (Called through its table, its answer kept whole: the bindings' call takes S_FALSE for done.)
        let answer = unsafe { (Interface::vtable(&*self.services).base__.TxSendMessage)(Interface::as_raw(&*self.services), message, wparam, lparam, &mut result) };
        (answer == windows::Win32::Foundation::S_OK).then_some(result)
    }

    /// Its words dressed as `looks` says (see `notes::markdown`), in `palette`'s colours and faces, `hidden` hidden
    /// (Markdown's marks put away) and each of `rooms` (a space) made as wide as it says (its words' size over: a
    /// to-do's box's room, a bullet's, a quote's), the lines set otherwise than their words would set them (`drawn`: the
    /// lines of each table drawn laid out over them, as tall as it is between them, a block of code's fences, low; see
    /// `Shown::drawn_lines`) as tall as that says, each line with marks put away at its start (`leads`: see
    /// `Shown::leads`) its rows after its first as far in as its words begin on it (a list's item's, a quote's words
    /// wrapped under each other, not under its dot or its bars): what they are left as it is, the dressing kept out of
    /// what undo takes back (and drawn once, done).
    #[allow(clippy::too_many_arguments)]
    pub fn dress(&self, looks: &[(std::ops::Range<usize>, crate::notes::markdown::Look)], hidden: &[std::ops::Range<usize>], rooms: &[(usize, f32)], drawn: &[(std::ops::Range<usize>, f32)], leads: &[std::ops::Range<usize>], palette: &Palette) {
        use crate::notes::markdown::Look;
        let Ok(document) = self.services.cast::<ITextDocument>() else { return };
        let face = BSTR::from(palette.face.as_str());
        let code = BSTR::from(palette.code_face.as_str());
        let color = |c: COLORREF| c.0 as i32;
        // (A space as wide as the room, in whole pixels (as a note shown makes it, its words whole pixels high): what it
        // is wide as it is, and as much again as the room needs. Points, as the text services count, three to four
        // pixels.)
        let pixels = palette.points / 0.75;
        let space = space_width(&palette.face, pixels.round());
        // (Dressing is no writing: what was written before it, not yet asked about, still was.)
        let written = self.kept.changed.get();
        unsafe {
            let _ = document.Undo(tomSuspend.0);
            let _ = document.Freeze();
            // All as plain words first; then each stretch as it looks.
            if let Ok(font) = document.Range(0, i32::MAX).and_then(|all| all.GetFont()) {
                let _ = font.SetBold(tomFalse.0);
                let _ = font.SetItalic(tomFalse.0);
                let _ = font.SetStrikeThrough(tomFalse.0);
                let _ = font.SetHidden(tomFalse.0);
                let _ = font.SetSpacing(0.0);
                let _ = font.SetSize(palette.points);
                let _ = font.SetName(&face);
                let _ = font.SetForeColor(color(palette.text));
            }
            if let Ok(para) = document.Range(0, i32::MAX).and_then(|all| all.GetPara()) {
                let _ = para.SetIndents(0.0, 0.0, 0.0);
            }
            for (range, look) in looks {
                let Ok(font) = document.Range(range.start as i32, range.end as i32).and_then(|range| range.GetFont()) else { continue };
                let _ = match look {
                    Look::Heading(level) => {
                        let _ = font.SetBold(tomTrue.0);
                        font.SetSize(palette.points * crate::notes::markdown::heading_size(*level))
                    }
                    Look::Bold => font.SetBold(tomTrue.0),
                    Look::Italic => font.SetItalic(tomTrue.0),
                    Look::Struck => font.SetStrikeThrough(tomTrue.0),
                    Look::Code => font.SetName(&code),
                    Look::Link => font.SetForeColor(color(palette.accent)),
                    Look::Faint => font.SetForeColor(color(palette.faint)),
                    Look::Plain => font.SetForeColor(color(palette.text)),
                };
            }
            // What its faces have no glyphs for, in the faces the system falls back on.
            let units: Vec<u16> = self.text().encode_utf16().collect();
            let code: Vec<_> = looks.iter().filter(|(_, look)| matches!(look, Look::Code)).map(|(range, _)| range.clone()).collect();
            for (range, family) in super::gfx::fallen_back(&units, &code, &palette.face, &palette.code_face) {
                if let Ok(font) = document.Range(range.start as i32, range.end as i32).and_then(|range| range.GetFont()) {
                    let _ = font.SetName(&BSTR::from(family.as_str()));
                }
            }
            // Each line set exactly as tall as `markdown::line_heights` says (as a note's face sets it). (tomLineSpaceExactly;
            // points.)
            const EXACTLY: i32 = 4;
            // (Its words without the break the text services end them with: else a line after it, none, was found, and
            // set as tall as a line of words, its last line with it, which the break ends: a block of code's fence
            // there, as a note ending in one has, stood a line tall.)
            for (range, height) in crate::notes::markdown::line_heights(&self.words(), pixels.round(), drawn) {
                if let Ok(para) = document.Range(range.start as i32, range.end as i32).and_then(|range| range.GetPara()) {
                    let _ = para.SetLineSpacing(EXACTLY, height * 0.75);
                }
            }
            for range in hidden {
                if let Ok(font) = document.Range(range.start as i32, range.end as i32).and_then(|range| range.GetFont()) {
                    let _ = font.SetHidden(tomTrue.0);
                }
            }
            for (room, wide) in rooms {
                let Some(space) = space else { break };
                if let Ok(font) = document.Range(*room as i32, *room as i32 + 1).and_then(|range| range.GetFont()) {
                    let _ = font.SetSpacing(((wide * pixels.round()).round() - space) * 0.75);
                }
            }
            let _ = document.Unfreeze();
            // Each line's rows after its first as far in as its words begin on its first, found as it is laid out now
            // (points: the first row as far back out).
            let left = self.kept.rect.get().left;
            let indents: Vec<(usize, f32)> = leads.iter().filter_map(|lead| Some((lead.start, (self.character_anywhere(lead.end)?.left - left) as f32 * 0.75))).filter(|(_, by)| *by > 0.0).collect();
            if !indents.is_empty() {
                let _ = document.Freeze();
                for (start, by) in indents {
                    if let Ok(para) = document.Range(start as i32, start as i32).and_then(|range| range.GetPara()) {
                        let _ = para.SetIndents(-by, by, 0.0);
                    }
                }
                let _ = document.Unfreeze();
            }
            let _ = document.Undo(tomResume.0);
        }
        self.kept.changed.set(written);
    }

    /// All its words set in their lines as `align` says (a table's cell's, as its column is set), kept out of what undo
    /// takes back.
    pub fn align(&self, align: crate::notes::table::Align) {
        use crate::notes::table::Align;
        let Ok(document) = self.services.cast::<ITextDocument>() else { return };
        // (tomAlignLeft, tomAlignCenter, tomAlignRight.)
        let set = match align {
            Align::Left => 0,
            Align::Centre => 1,
            Align::Right => 2,
        };
        // (Setting them so is no writing: what was written before, not yet asked about, still was.)
        let written = self.kept.changed.get();
        unsafe {
            let _ = document.Undo(tomSuspend.0);
            if let Ok(para) = document.Range(0, i32::MAX).and_then(|all| all.GetPara()) {
                let _ = para.SetAlignment(set);
            }
            let _ = document.Undo(tomResume.0);
        }
        self.kept.changed.set(written);
    }

    /// Where the character at `at` (UTF-16 units) is drawn (in the window): from its left and its line's top to where
    /// the next begins and its line's foot. None where it is not drawn (scrolled out of the box).
    pub fn character(&self, at: usize) -> Option<RECT> {
        self.character_where(at, false)
    }

    /// As `character`, where it is drawn or would be (scrolled out of the box: above it or below it).
    pub fn character_anywhere(&self, at: usize) -> Option<RECT> {
        self.character_where(at, true)
    }

    fn character_where(&self, at: usize, anywhere: bool) -> Option<RECT> {
        // (tomStart, tomEnd, TA_BOTTOM.)
        const START: i32 = 32;
        const END: i32 = 0;
        const FOOT: i32 = 8;
        let (left, top) = self.point(at, START, anywhere)?;
        let (_, bottom) = self.point(at, START | FOOT, anywhere)?;
        let (right, _) = self.point(at, END, anywhere)?;
        Some(RECT { left, top, right: right.max(left), bottom })
    }

    /// Where the baseline of the character at `at` (UTF-16 units) is (in the window), where it is drawn or would be
    /// (scrolled out of the box, or only partly in it).
    pub fn baseline_anywhere(&self, at: usize) -> Option<i32> {
        // (tomStart, TA_BASELINE.)
        const START: i32 = 32;
        const BASELINE: i32 = 24;
        self.point(at, START | BASELINE, true).map(|(_, y)| y)
    }

    /// The point `kind` says of the character at `at` (UTF-16 units), in the window: where it is drawn, or where
    /// `anywhere`, where it would be out of the box too. None where the text services say it is not (`S_FALSE`, the
    /// point left unset).
    fn point(&self, at: usize, kind: i32, anywhere: bool) -> Option<(i32, i32)> {
        // (tomClientCoord: in the window, not on the screen. tomAllowOffClient: out of the box too.)
        const IN_WINDOW: i32 = 256;
        const OFF_BOX: i32 = 512;
        let document = self.services.cast::<ITextDocument>().ok()?;
        let range = unsafe { document.Range(at as i32, at as i32 + 1) }.ok()?;
        let (mut x, mut y) = (0, 0);
        let kind = tomConstants(kind | IN_WINDOW | if anywhere { OFF_BOX } else { 0 });
        // (Called through its table, its answer kept whole: the bindings' call takes S_FALSE for done.)
        let answer = unsafe { (Interface::vtable(&range).GetPoint)(Interface::as_raw(&range), kind, &mut x, &mut y) };
        (answer == windows::Win32::Foundation::S_OK).then_some((x, y))
    }

    /// Where among its words (in UTF-16 units) the point `point` (in the window) is.
    pub fn at(&self, point: POINT) -> usize {
        let mut at = windows::Win32::Foundation::POINTL { x: point.x, y: point.y };
        self.send(EM_CHARFROMPOS, WPARAM(0), LPARAM(&mut at as *mut _ as isize)).map_or(0, |found| found.0.max(0) as usize)
    }

    /// The words from `start` to `end` (UTF-16 units) written over with `text`, as if typed (undo takes it back).
    pub fn replace(&self, start: usize, end: usize, text: &str) {
        let Ok(document) = self.services.cast::<ITextDocument>() else { return };
        unsafe {
            if let Ok(range) = document.Range(start as i32, end as i32) {
                let _ = range.SetText(&BSTR::from(text));
            }
        }
        self.kept.changed.set(true);
    }

    /// `edit` made (see `notes::edit`): its words written as one step for undo to take back (not gathered with what was
    /// typed before it or is typed after), what it chooses chosen.
    pub fn apply(&self, edit: &crate::notes::edit::Edit) {
        self.answer(EM_STOPGROUPTYPING, WPARAM(0), LPARAM(0));
        // (Each line break one carriage return, as the text services keep them.)
        self.replace(edit.range.start, edit.range.end, &edit.with.replace('\n', "\r"));
        self.answer(EM_STOPGROUPTYPING, WPARAM(0), LPARAM(0));
        self.typed.set(Typed::Nothing);
        self.select((edit.chosen.0 as i32, edit.chosen.1 as i32));
    }

    /// What is chosen in it (from, to: characters).
    pub fn selection(&self) -> (i32, i32) {
        let mut range = CHARRANGE::default();
        self.answer(EM_EXGETSEL, WPARAM(0), LPARAM(&mut range as *mut _ as isize));
        (range.cpMin, range.cpMax)
    }

    /// `from`..`to` chosen in it (as far as its words go).
    pub fn select(&self, (from, to): (i32, i32)) {
        let range = CHARRANGE { cpMin: from, cpMax: to };
        self.answer(EM_EXSETSEL, WPARAM(0), LPARAM(&range as *const _ as isize));
    }

    /// Where its words are written (in the window).
    pub fn bounds(&self) -> RECT {
        self.kept.rect.get()
    }

    /// Whether `point` (in the window) is where its words are.
    pub fn holds(&self, point: POINT) -> bool {
        let rect = self.kept.rect.get();
        point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
    }

    /// Its words drawn on `target` (in pixels: see `gfx::Frame::in_pixels`).
    pub fn draw(&self, target: &ID2D1RenderTarget) {
        let rect = self.kept.rect.get();
        let mut bounds = RECTL { left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom };
        unsafe {
            let _ = self.services.TxDrawD2D(target, &mut bounds, std::ptr::null_mut(), TXTVIEW_ACTIVE.0);
        }
    }

    /// Its words written at `rect` (in the window), laid out again in it now (its size told as changed too: told only
    /// that it moved, the text services laid them out as wide as it was till they were next drawn).
    pub fn place(&self, rect: RECT) {
        self.kept.rect.set(rect);
        unsafe {
            let _ = self.services.OnTxPropertyBitsChange(TXTBIT_CLIENTRECTCHANGE | TXTBIT_EXTENTCHANGE, TXTBIT_CLIENTRECTCHANGE | TXTBIT_EXTENTCHANGE);
        }
    }

    /// The window it is written in.
    pub fn window(&self) -> HWND {
        self.kept.hwnd
    }

    /// The caret, where it is shown now (in the window): its left, top, width and height.
    pub fn caret(&self) -> Option<(i32, i32, i32, i32)> {
        self.kept.caret_shown.get().then(|| self.kept.caret.get()).flatten()
    }
}

/// How wide a space is in `face` `pixels` high (whole pixels), as the text services lay it, GDI's way (a whole number of
/// pixels), measured once for each (none where DirectWrite could not lay it).
fn space_width(face: &str, pixels: f32) -> Option<f32> {
    use windows::Win32::Graphics::DirectWrite::{DWriteCreateFactory, IDWriteFactory, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_METRICS};
    thread_local! {
        static MEASURED: std::cell::RefCell<Vec<(String, u32, f32)>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    if let Some(width) = MEASURED.with_borrow(|measured| measured.iter().find(|(kept, size, _)| kept == face && *size == pixels.to_bits()).map(|(_, _, width)| *width)) {
        return Some(width);
    }
    let measured = (|| unsafe {
        let factory: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).ok()?;
        let format = factory.CreateTextFormat(&HSTRING::from(face), None, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, pixels, windows::core::w!("zh-cn")).ok()?;
        let layout = factory.CreateGdiCompatibleTextLayout(&[u16::from(b' ')], &format, 1000.0, 1000.0, 1.0, None, false).ok()?;
        let mut metrics = DWRITE_TEXT_METRICS::default();
        layout.GetMetrics(&mut metrics).ok()?;
        Some(metrics.widthIncludingTrailingWhitespace)
    })()?;
    MEASURED.with_borrow_mut(|kept| kept.push((face.to_string(), pixels.to_bits(), measured)));
    Some(measured)
}

impl Drop for Editor {
    /// Its text services shut down (`ShutdownTextServices`) while their host and window are there: told to let go of
    /// the host, which a reference to them kept elsewhere would else call after it is gone.
    fn drop(&mut self) {
        unsafe {
            let _ = self.services.OnTxInPlaceDeactivate();
            let shutdown = LoadLibraryW(windows::core::w!("msftedit.dll")).ok().and_then(|library| GetProcAddress(library, windows::core::s!("ShutdownTextServices")));
            if let Some(shutdown) = shutdown {
                let shutdown: unsafe extern "system" fn(*mut core::ffi::c_void) -> windows::core::HRESULT = std::mem::transmute(shutdown);
                let services = std::mem::ManuallyDrop::take(&mut self.services);
                let _ = shutdown(services.into_raw());
            }
        }
    }
}

impl ITextHost_Impl for Host_Impl {
    fn TxGetDC(&self) -> HDC {
        unsafe { GetDC(Some(self.kept.hwnd)) }
    }
    fn TxReleaseDC(&self, hdc: HDC) -> i32 {
        unsafe { ReleaseDC(Some(self.kept.hwnd), hdc) }
    }
    fn TxShowScrollBar(&self, _: i32, _: BOOL) -> BOOL {
        true.into()
    }
    fn TxEnableScrollBar(&self, _: SCROLLBAR_CONSTANTS, _: i32) -> BOOL {
        true.into()
    }
    fn TxSetScrollRange(&self, _: i32, _: i32, _: i32, _: BOOL) -> BOOL {
        true.into()
    }
    fn TxSetScrollPos(&self, _: i32, _: i32, _: BOOL) -> BOOL {
        true.into()
    }
    fn TxInvalidateRect(&self, _: *mut RECT, _: BOOL) {
        unsafe {
            let _ = InvalidateRect(Some(self.kept.hwnd), None, false);
        }
    }
    fn TxViewChange(&self, _: BOOL) {
        unsafe {
            let _ = InvalidateRect(Some(self.kept.hwnd), None, false);
        }
    }
    fn TxCreateCaret(&self, _: HBITMAP, width: i32, height: i32) -> BOOL {
        self.kept.caret_size.set((width.max(1), height));
        true.into()
    }
    fn TxShowCaret(&self, show: BOOL) -> BOOL {
        self.kept.caret_shown.set(show.as_bool());
        self.TxViewChange(false.into());
        true.into()
    }
    fn TxSetCaretPos(&self, x: i32, y: i32) -> BOOL {
        let (width, height) = self.kept.caret_size.get();
        self.kept.caret.set(Some((x, y, width, height)));
        self.TxViewChange(false.into());
        true.into()
    }
    fn TxSetTimer(&self, id: u32, timeout: u32) -> BOOL {
        unsafe { (SetTimer(Some(self.kept.hwnd), id as usize, timeout, None) != 0).into() }
    }
    fn TxKillTimer(&self, id: u32) {
        unsafe {
            let _ = KillTimer(Some(self.kept.hwnd), id as usize);
        }
    }
    fn TxScrollWindowEx(&self, _: i32, _: i32, _: *mut RECT, _: *mut RECT, _: HRGN, _: *mut RECT, _: SCROLL_WINDOW_FLAGS) {
        self.TxViewChange(false.into());
    }
    fn TxSetCapture(&self, capture: BOOL) {
        if capture.as_bool() {
            unsafe { SetCapture(self.kept.hwnd) };
        } else {
            super::window::let_go_of_capture(self.kept.hwnd);
        }
    }
    fn TxSetFocus(&self) {
        unsafe {
            let _ = SetFocus(Some(self.kept.hwnd));
        }
    }
    fn TxSetCursor(&self, cursor: HCURSOR, _: BOOL) {
        unsafe {
            SetCursor(Some(cursor));
        }
    }
    fn TxScreenToClient(&self, point: *mut POINT) -> BOOL {
        unsafe { ScreenToClient(self.kept.hwnd, point) }
    }
    fn TxClientToScreen(&self, point: *mut POINT) -> BOOL {
        unsafe { ClientToScreen(self.kept.hwnd, point) }
    }
    fn TxActivate(&self, _: *mut i32) -> windows::core::Result<()> {
        Ok(())
    }
    fn TxDeactivate(&self, _: i32) -> windows::core::Result<()> {
        Ok(())
    }
    fn TxGetClientRect(&self, rect: *mut RECT) -> windows::core::Result<()> {
        unsafe { *rect = self.kept.rect.get() };
        Ok(())
    }
    fn TxGetViewInset(&self, inset: *mut RECT) -> windows::core::Result<()> {
        unsafe { *inset = RECT::default() };
        Ok(())
    }
    fn TxGetCharFormat(&self, format: *const *const CHARFORMATW) -> windows::core::Result<()> {
        unsafe { *(format as *mut *const CHARFORMATW) = &self.kept.char_format.Base };
        Ok(())
    }
    fn TxGetParaFormat(&self, format: *const *const PARAFORMAT) -> windows::core::Result<()> {
        unsafe { *(format as *mut *const PARAFORMAT) = &*self.kept.para_format };
        Ok(())
    }
    /// (What is chosen they fill from the system's highlight colour: see `chosen_ink`.)
    fn TxGetSysColor(&self, index: SYS_COLOR_INDEX) -> COLORREF {
        unsafe { COLORREF(GetSysColor(index)) }
    }
    fn TxGetBackStyle(&self, style: *mut TXTBACKSTYLE) -> windows::core::Result<()> {
        unsafe { *style = TXTBACK_TRANSPARENT };
        Ok(())
    }
    fn TxGetMaxLength(&self, length: *mut u32) -> windows::core::Result<()> {
        unsafe { *length = 1 << 24 };
        Ok(())
    }
    fn TxGetScrollBars(&self, bars: *mut u32) -> windows::core::Result<()> {
        // (Scrolled down as it is written, by the wheel, no bar drawn of theirs. On one line, along: ES_AUTOHSCROLL.)
        unsafe { *bars = if self.kept.one_line { 0x80 } else { WS_VSCROLL.0 | 0x40 } };
        Ok(())
    }
    fn TxGetPasswordChar(&self) -> windows::core::Result<i8> {
        Err(windows::Win32::Foundation::S_FALSE.into())
    }
    fn TxGetAcceleratorPos(&self, at: *mut i32) -> windows::core::Result<()> {
        unsafe { *at = -1 };
        Ok(())
    }
    fn TxGetExtent(&self, _: *mut SIZE) -> windows::core::Result<()> {
        Err(windows::Win32::Foundation::E_NOTIMPL.into())
    }
    fn OnTxCharFormatChange(&self, _: *const CHARFORMATW) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnTxParaFormatChange(&self, _: *const PARAFORMAT) -> windows::core::Result<()> {
        Ok(())
    }
    fn TxGetPropertyBits(&self, mask: u32, bits: *mut u32) -> windows::core::Result<()> {
        let drawn = TXTBIT_D2DDWRITE | TXTBIT_D2DSIMPLETYPOGRAPHY | TXTBIT_D2DPIXELSNAPPED;
        let ours = if self.kept.one_line { drawn } else { TXTBIT_RICHTEXT | TXTBIT_MULTILINE | TXTBIT_WORDWRAP | drawn };
        unsafe { *bits = ours & mask };
        Ok(())
    }
    fn TxNotify(&self, notify: u32, _: *mut core::ffi::c_void) -> windows::core::Result<()> {
        if notify == windows::Win32::UI::WindowsAndMessaging::EN_CHANGE {
            self.kept.changed.set(true);
        }
        Ok(())
    }
    fn TxImmGetContext(&self) -> HIMC {
        unsafe { ImmGetContext(self.kept.hwnd) }
    }
    fn TxImmReleaseContext(&self, context: HIMC) {
        unsafe {
            let _ = ImmReleaseContext(self.kept.hwnd, context);
        }
    }
    fn TxGetSelectionBarWidth(&self, width: *mut i32) -> windows::core::Result<()> {
        unsafe { *width = 0 };
        Ok(())
    }
}

impl ITextHost2_Impl for Host_Impl {
    fn TxIsDoubleClickPending(&self) -> BOOL {
        false.into()
    }
    fn TxGetWindow(&self, hwnd: *mut HWND) -> windows::core::Result<()> {
        unsafe { *hwnd = self.kept.hwnd };
        Ok(())
    }
    fn TxSetForegroundWindow(&self) -> windows::core::Result<()> {
        unsafe {
            let _ = SetForegroundWindow(self.kept.hwnd);
        }
        Ok(())
    }
    fn TxGetPalette(&self) -> HPALETTE {
        HPALETTE::default()
    }
    fn TxGetEastAsianFlags(&self, flags: *mut i32) -> windows::core::Result<()> {
        unsafe { *flags = 0 };
        Ok(())
    }
    fn TxSetCursor2(&self, cursor: HCURSOR, _: BOOL) -> HCURSOR {
        unsafe { SetCursor(Some(cursor)) }
    }
    fn TxFreeTextServicesNotification(&self) {}
    fn TxGetEditStyle(&self, _: u32, data: *mut u32) -> windows::core::Result<()> {
        unsafe { *data = 0 };
        Ok(())
    }
    fn TxGetWindowStyles(&self, style: *mut u32, extended: *mut u32) -> windows::core::Result<()> {
        unsafe {
            *style = 0;
            *extended = 0;
        }
        Ok(())
    }
    fn TxShowDropCaret(&self, _: BOOL, _: HDC, _: *mut RECT) -> windows::core::Result<()> {
        Ok(())
    }
    fn TxDestroyCaret(&self) -> windows::core::Result<()> {
        self.kept.caret.set(None);
        Ok(())
    }
    fn TxGetHorzExtent(&self, extent: *mut i32) -> windows::core::Result<()> {
        unsafe { *extent = 0 };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::markdown::looks;
    use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_POPUP};

    /// What `editor` draws on a white bitmap `wide` × `tall` (96 per inch, as the page draws it): each pixel's blue.
    unsafe fn drawn(editor: &Editor, wide: u32, tall: u32) -> Vec<u8> {
        use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT};
        use windows::Win32::Graphics::Direct2D::{D2D1CreateFactory, ID2D1Factory, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT};
        use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
        use windows::Win32::Graphics::Imaging::{CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory, WICBitmapCacheOnLoad};
        unsafe {
            let wic: IWICImagingFactory = windows::Win32::System::Com::CoCreateInstance(&CLSID_WICImagingFactory, None, windows::Win32::System::Com::CLSCTX_INPROC_SERVER).unwrap();
            let bitmap = wic.CreateBitmap(wide, tall, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad).unwrap();
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).unwrap();
            let properties = D2D1_RENDER_TARGET_PROPERTIES { r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT, pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED }, dpiX: 96.0, dpiY: 96.0, ..Default::default() };
            let target = factory.CreateWicBitmapRenderTarget(&bitmap, &properties).unwrap();
            target.BeginDraw();
            target.Clear(Some(&D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }));
            editor.draw(&target);
            target.EndDraw(None, None).unwrap();
            let mut pixels = vec![0u8; (wide * tall * 4) as usize];
            bitmap.CopyPixels(std::ptr::null(), wide * 4, &mut pixels).unwrap();
            pixels.chunks(4).map(|pixel| pixel[0]).collect()
        }
    }

    /// Its box made wider: its words laid out again in it, on one line where they now fit (as a table's cell written in
    /// grows as its words do).
    #[test]
    fn widened_its_words_are_laid_out_again() {
        let _device = crate::ui::gfx::held().unwrap();
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
            let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), windows::core::w!("STATIC"), windows::core::w!(""), WS_POPUP, 0, 0, 400, 300, None, None, None, None).unwrap();
            let editor = Editor::new(hwnd, RECT { left: 10, top: 10, right: 19, bottom: 34 }, "Segoe UI", 16.0, COLORREF(0)).expect("text services");
            editor.set_text("yz");
            assert_ne!(editor.character_anywhere(0).unwrap().top, editor.character_anywhere(1).unwrap().top, "narrow: broken");
            editor.place(RECT { left: 10, top: 10, right: 200, bottom: 34 });
            assert_eq!(editor.character_anywhere(0).unwrap().top, editor.character_anywhere(1).unwrap().top, "wide: one line");
            // (Just as wide as they are: on one line still.)
            let wide = editor.character_anywhere(1).unwrap().right;
            editor.place(RECT { left: 10, top: 10, right: wide, bottom: 34 });
            assert_eq!(editor.character_anywhere(0).unwrap().top, editor.character_anywhere(1).unwrap().top, "as wide: one line");
            let _ = DestroyWindow(hwnd);
        }
    }

    /// The caret where the words drawn end, the window aware of its screen's scale as Glance's are (on a screen scaled
    /// past 100%: the text services count in the window's pixels whatever the scale, as Glance draws them).
    #[test]
    #[ignore = "makes a window of this session's"]
    fn puts_the_caret_where_the_words_end() {
        unsafe {
            let _ = windows::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
            let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), windows::core::w!("STATIC"), windows::core::w!(""), WS_POPUP, 0, 0, 800, 300, None, None, None, None).unwrap();
            let (wide, tall) = (800u32, 300u32);
            let editor = Editor::new(hwnd, RECT { left: 20, top: 30, right: 780, bottom: 280 }, "Segoe UI", 22.0, COLORREF(0)).expect("text services");
            editor.set_text("MMMMMMMMMM second line");
            let _ = editor.send(windows::Win32::UI::WindowsAndMessaging::WM_SETFOCUS, WPARAM(0), LPARAM(0));
            let document: ITextDocument = editor.services.cast().unwrap();
            let _ = document.Range(i32::MAX, i32::MAX).unwrap().Select();
            let pixels = drawn(&editor, wide, tall);
            let inked = |x: u32, y: u32| pixels[(y * wide + x) as usize] < 128;
            // The words' right end, and their lines' top and bottom.
            let right = (0..wide).rev().find(|&x| (0..tall).any(|y| inked(x, y))).expect("words drawn");
            let bottom = (0..tall).rev().find(|&y| (0..wide).any(|x| inked(x, y))).unwrap();
            let caret = editor.caret().expect("a caret");
            drop(editor);
            let _ = DestroyWindow(hwnd);
            assert!((caret.0 - right as i32).abs() <= 6, "caret at x {} where the words end at {right}", caret.0);
            assert!(caret.1 <= bottom as i32 && caret.1 + caret.3 >= bottom as i32 - 4, "caret at y {}..{} where the words' last line ends at {bottom}", caret.1, caret.1 + caret.3);
        }
    }

    /// A tab: the words after it as far on as `TAB_PIXELS` says, as a note's face sets them (see `Frame::markdown`).
    #[test]
    #[ignore = "makes a window of this session's"]
    fn sets_tabs_as_far_apart_as_said() {
        unsafe {
            let _ = windows::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
            let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), windows::core::w!("STATIC"), windows::core::w!(""), WS_POPUP, 0, 0, 800, 300, None, None, None, None).unwrap();
            let editor = Editor::new(hwnd, RECT { left: 20, top: 30, right: 780, bottom: 280 }, "Segoe UI", 30.0, COLORREF(0)).expect("text services");
            let mut ends = Vec::new();
            for text in ["\tX", "\t\tX"] {
                editor.set_text(text);
                let _ = editor.send(windows::Win32::UI::WindowsAndMessaging::WM_SETFOCUS, WPARAM(0), LPARAM(0));
                let document: ITextDocument = editor.services.cast().unwrap();
                let tabs = text.matches('\t').count() as i32;
                let _ = document.Range(tabs, tabs).unwrap().Select();
                let _ = drawn(&editor, 800, 300);
                ends.push(editor.caret().expect("a caret").0 - 20);
            }
            drop(editor);
            let _ = DestroyWindow(hwnd);
            assert!((ends[0] as f32 - TAB_PIXELS).abs() <= 1.0 && (ends[1] as f32 - 2.0 * TAB_PIXELS).abs() <= 1.0, "tabs end at {ends:?}");
        }
    }

    /// Lines of Latin and lines with Chinese in them, in a face without Chinese (as on a PC in English): each line as
    /// tall as the others.
    #[test]
    #[ignore = "makes a window of this session's"]
    fn lines_with_chinese_are_as_tall_as_others() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
            let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), windows::core::w!("STATIC"), windows::core::w!(""), WS_POPUP, 0, 0, 600, 400, None, None, None, None).unwrap();
            let editor = Editor::new(hwnd, RECT { left: 0, top: 0, right: 600, bottom: 400 }, "Segoe UI", 20.0, COLORREF(0)).expect("text services");
            editor.set_text("abc\nabc 你好\nabc\nabc");
            // Each line's first character: 0, 4, 11, 15.
            let tops = [editor.top_of(0), editor.top_of(4), editor.top_of(11), editor.top_of(15)];
            // On one line, the Chinese drawn (by DirectWrite's faces for it), its foot by the Latin's (not set higher
            // or lower in a taller line).
            editor.set_text("nnn你");
            let split = {
                let mut point = windows::Win32::Foundation::POINTL::default();
                editor.answer(0x00D6, WPARAM(&mut point as *mut _ as usize), LPARAM(3));
                point.x as u32
            };
            let pixels = drawn(&editor, 600, 400);
            let foot = |xs: std::ops::Range<u32>| (0..400u32).rev().find(|&y| xs.clone().any(|x| pixels[(y * 600 + x) as usize] < 128));
            let (latin, chinese) = (foot(0..split), foot(split..600));
            drop(editor);
            let (latin, chinese) = (latin.expect("the Latin drawn"), chinese.expect("the Chinese drawn"));
            assert!(latin.abs_diff(chinese) <= 1, "the Latin's foot at {latin}, the Chinese's at {chinese}");
            let _ = DestroyWindow(hwnd);
            let heights: Vec<i32> = tops.windows(2).map(|pair| pair[1] - pair[0]).collect();
            assert!(heights.iter().all(|height| (height - heights[2]).abs() <= 1), "lines {heights:?} tall");
        }
    }

    /// The text services hosted here take keys as written and give the words back; dressed as Markdown, a heading is
    /// bold and larger, its words otherwise as typed; undo takes back what was typed, not the dressing.
    #[test]
    #[ignore = "makes a window of this session's"]
    fn writes_dresses_and_undoes() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
            let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), windows::core::w!("STATIC"), windows::core::w!(""), WS_POPUP, 0, 0, 400, 300, None, None, None, None).unwrap();
            let editor = Editor::new(hwnd, RECT { left: 0, top: 0, right: 400, bottom: 300 }, "Microsoft YaHei UI", 20.0, COLORREF(0)).expect("text services");
            let palette = Palette { text: COLORREF(0x202020), faint: COLORREF(0x909090), accent: COLORREF(0xD07000), points: 15.0, face: "Microsoft YaHei UI".into(), code_face: "Cascadia Mono".into() };
            editor.set_text("# Title\nbody");
            editor.dress(&looks(&editor.text()), &[], &[], &[], &[], &palette);
            let document: ITextDocument = editor.services.cast().unwrap();
            let font_at = |at: i32| document.Range(at, at + 1).unwrap().GetFont().unwrap();
            // "T" (2) bold and larger; "b" of body (8) neither.
            assert_eq!(font_at(2).GetBold().unwrap(), tomTrue.0);
            assert!(font_at(2).GetSize().unwrap() > 20.0);
            assert_eq!(font_at(8).GetBold().unwrap(), tomFalse.0);
            // "#" (0) faint.
            assert_eq!(font_at(0).GetForeColor().unwrap(), 0x909090);
            // Typed at its end, dressed again, then undone: what was typed goes, the dressing stays.
            let _ = editor.send(windows::Win32::UI::WindowsAndMessaging::WM_SETFOCUS, WPARAM(0), LPARAM(0));
            let _ = document.Range(i32::MAX, i32::MAX).unwrap().Select();
            for c in "!".encode_utf16() {
                let _ = editor.send(windows::Win32::UI::WindowsAndMessaging::WM_CHAR, WPARAM(usize::from(c)), LPARAM(0));
            }
            assert_eq!(editor.text().trim_end(), "# Title\nbody!");
            editor.dress(&looks(&editor.text()), &[], &[], &[], &[], &palette);
            let _ = editor.send(windows::Win32::UI::WindowsAndMessaging::WM_UNDO, WPARAM(0), LPARAM(0));
            assert_eq!(editor.text().trim_end(), "# Title\nbody");
            assert_eq!(font_at(2).GetBold().unwrap(), tomTrue.0);
            // A to-do's box ticked as if typed: undo takes it back.
            editor.set_text("- [ ] 买牛奶");
            let (range, done) = crate::notes::markdown::tasks(&editor.text()).remove(0);
            editor.replace(range.start, range.end, if done { "[ ]" } else { "[x]" });
            assert_eq!(editor.text().trim_end(), "- [x] 买牛奶");
            let _ = editor.send(windows::Win32::UI::WindowsAndMessaging::WM_UNDO, WPARAM(0), LPARAM(0));
            assert_eq!(editor.text().trim_end(), "- [ ] 买牛奶");
            // Chinese and an emoji written and given back as written.
            editor.set_text("你好 😀");
            assert_eq!(editor.text().trim_end(), "你好 😀");
            // Chinese typed, dressed, then one character taken back with Backspace (dressed again): undo brings that
            // character back, not all that was typed.
            editor.set_text("");
            for c in "你好世界".encode_utf16() {
                let _ = editor.send(windows::Win32::UI::WindowsAndMessaging::WM_CHAR, WPARAM(usize::from(c)), LPARAM(0));
            }
            editor.dress(&looks(&editor.text()), &[], &[], &[], &[], &palette);
            let back = WPARAM(usize::from(windows::Win32::UI::Input::KeyboardAndMouse::VK_BACK.0));
            let _ = editor.send(windows::Win32::UI::WindowsAndMessaging::WM_KEYDOWN, back, LPARAM(0));
            let _ = editor.send(windows::Win32::UI::WindowsAndMessaging::WM_CHAR, WPARAM(8), LPARAM(0));
            let _ = editor.send(windows::Win32::UI::WindowsAndMessaging::WM_KEYUP, back, LPARAM(0));
            editor.dress(&looks(&editor.text()), &[], &[], &[], &[], &palette);
            assert_eq!(editor.text().trim_end(), "你好世");
            let _ = editor.send(windows::Win32::UI::WindowsAndMessaging::WM_UNDO, WPARAM(0), LPARAM(0));
            assert_eq!(editor.text().trim_end(), "你好世界");
            // What is chosen, chosen again.
            editor.set_text("abcdef");
            editor.select((2, 4));
            assert_eq!(editor.selection(), (2, 4));
            // A file written on Windows: its line breaks one each, not two.
            editor.set_text("一\r\n二");
            assert_eq!(editor.text().trim_end(), "一\n二");
            // A note's file given back as it was read, its last line break no line more (opened and kept again and
            // again, it grew by one each time). One without a last line break given one, once.
            for file in ["a\nb\n", "a\n\n", "a\n\nb\n", "\n", "一\r\n二\r\n"] {
                editor.set_text(file);
                assert_eq!(editor.text(), file.replace("\r\n", "\n"), "{file:?}");
            }
            editor.set_text("a\nb");
            let once = editor.text();
            editor.set_text(&once);
            assert_eq!((once.as_str(), editor.text().as_str()), ("a\nb\n", "a\nb\n"));
            drop(editor);
            // Its text services, held here still, used after the box is gone: their host let go of, nothing faults.
            let _ = document.Range(0, 1).and_then(|range| range.GetText());
            let _ = document.Range(0, 0).and_then(|range| range.SetText(&BSTR::from("x")));
            let _ = DestroyWindow(hwnd);
            drop(document);
        }
    }

    /// Its marks put away as a note not written on shows them: a to-do's words begin where its box's room ends
    /// (`markdown::BOX_ROOM` of its words' height on), as the note's face sets them; emphasis's marks take no room;
    /// and typing then undoing leaves them put away (their dressing not what undo takes back).
    #[test]
    #[ignore = "makes a window of this session's"]
    fn puts_marks_away_and_makes_room_for_a_box() {
        use crate::notes::markdown::{Shown, BOX_ROOM};
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
            let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), windows::core::w!("STATIC"), windows::core::w!(""), WS_POPUP, 0, 0, 400, 300, None, None, None, None).unwrap();
            let editor = Editor::new(hwnd, RECT { left: 0, top: 0, right: 400, bottom: 300 }, "Microsoft YaHei UI", 20.0, COLORREF(0)).expect("text services");
            let palette = Palette { text: COLORREF(0x202020), faint: COLORREF(0x909090), accent: COLORREF(0xD07000), points: 15.0, face: "Microsoft YaHei UI".into(), code_face: "Cascadia Mono".into() };
            let pixels = palette.points / 0.75;
            let dress = |editor: &Editor| {
                let text = editor.text();
                let shown = Shown::of(&text);
                let (hidden, rooms) = shown.put_away();
                editor.dress(&looks(&text), &hidden, &rooms, &[], &shown.leads, &palette);
            };
            // "买" at 6, "粗" at 10 (after "**").
            editor.set_text("- [ ] 买牛奶\n**粗**体");
            dress(&editor);
            let left_of = |at: usize| editor.character(at).expect("drawn").left;
            let (task, bold) = (left_of(6), left_of(10));
            let _ = editor.send(windows::Win32::UI::WindowsAndMessaging::WM_SETFOCUS, WPARAM(0), LPARAM(0));
            editor.select((9, 9));
            let _ = editor.send(windows::Win32::UI::WindowsAndMessaging::WM_CHAR, WPARAM(usize::from(b'x')), LPARAM(0));
            dress(&editor);
            let _ = editor.send(windows::Win32::UI::WindowsAndMessaging::WM_UNDO, WPARAM(0), LPARAM(0));
            let undone = (left_of(6), left_of(10));
            drop(editor);
            let _ = DestroyWindow(hwnd);
            assert!((task as f32 - BOX_ROOM * pixels).abs() <= 1.5, "the to-do's words at {task}, its room {}", BOX_ROOM * pixels);
            assert!(bold <= 1, "the emphasised words at {bold}, their marks taking room");
            assert_eq!(undone, (task, bold), "typed and undone, the marks no longer put away");
        }
    }
}
