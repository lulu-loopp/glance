//! A window that asks before something is done, or leaves this PC, drawn as
//! Glance's own windows are (Windows 11's dialog, light or dark): its title
//! and what it is about, and, for something sent, everything that is sent,
//! each part under its name: the sender's own words (typed here), their
//! pictures, and what Glance adds, whole, in a box that scrolls. Two buttons
//! answer it; the window it is over takes nothing till it is answered. A
//! report's thread is shown in the same window, as a conversation, kept up
//! beside the window it is opened from (see `thread`).

pub mod thread;

use std::cell::RefCell;

use super::typing::{at_point, caret_at, height_of, line_of, Typing};
use std::rc::Rc;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT, D2D_SIZE_U};
use windows::Win32::Graphics::Direct2D::{D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_PROPERTIES1, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_LAYER_PARAMETERS1, D2D1_ROUNDED_RECT};
use windows::Win32::Graphics::DirectWrite::{IDWriteTextLayout, DWRITE_HIT_TEST_METRICS};
use windows::Win32::Graphics::Dwm::{DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, InvalidateRect, ScreenToClient, ValidateRect, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::DataExchange::{AddClipboardFormatListener, EmptyClipboard, RemoveClipboardFormatListener, SetClipboardData};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::Controls::{MARGINS, WM_MOUSELEAVE};
use windows::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::Ime::{
    ImmAssociateContextEx, HIMC, IACE_DEFAULT,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetKeyState, ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VIRTUAL_KEY, VK_A, VK_BACK, VK_C, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_NEXT,
    VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP, VK_V, VK_X,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCaretBlinkTime, GetClientRect, GetMessageW, GetWindowRect, IsWindow, KillTimer, LoadCursorW, PostQuitMessage, RegisterClassExW, SetCursor,
    SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, TranslateMessage, HTCAPTION, HTCLIENT, IDC_ARROW, IDC_IBEAM, MSG, SWP_NOACTIVATE, SWP_NOZORDER, SW_SHOW, WM_CHAR, WM_CLIPBOARDUPDATE, WM_CLOSE,
    WM_DPICHANGED, WM_IME_STARTCOMPOSITION, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCALCSIZE, WM_NCHITTEST, WM_PAINT, WM_SETCURSOR, WM_TIMER, WNDCLASSEXW,
    WS_CAPTION, WS_EX_NOREDIRECTIONBITMAP, WS_POPUP, WS_SYSMENU, ChangeWindowMessageFilterEx, MSGFLT_ALLOW, PostMessageW, WM_APP, WM_COPYDATA, WM_DESTROY, WM_DROPFILES,
};

use super::canvas::{Align, Canvas, Color, Family, Fill, Font, Point};
use super::gfx::{self, rect, Frame, Gfx, Surface};
use super::icons::Icon;
use super::picture::{self, Picture};
use super::view::Rect;

/// What the window asks: its title, what it is about (in as many lines as
/// that takes), the reader's own part if they may add one, a choice they may
/// tick under it (what it says), what is shown besides (under its name, and
/// whether it is shown from its end, as a thread is, the newest last), the
/// two answers, and whether it is dark.
pub struct Asked<'a> {
    pub title: &'a str,
    pub intro: &'a str,
    pub own: Option<Own<'a>>,
    pub option: Option<&'a str>,
    pub sent: Option<(&'a str, &'a str)>,
    pub from_end: bool,
    pub yes: &'a str,
    pub no: &'a str,
    pub dark: bool,
}

/// What the reader's own part is called: the heading over the box for
/// their words and what the box says while it is empty; the heading over
/// their pictures; the place that adds one (what it is, and the ways it is
/// done), its button that pastes the clipboard's picture and what it says
/// of that picture where there is no room for the button; and pictures, as
/// the file chooser lists them.
pub struct Own<'a> {
    pub words: &'a str,
    pub hint: &'a str,
    pub pictures: &'a str,
    pub add: &'a str,
    pub ways: &'a str,
    pub paste: &'a str,
    pub waiting: &'a str,
    pub kinds: &'a str,
}

/// The same, kept by the window while it is up.
#[derive(Clone)]
struct OwnText {
    words: String,
    hint: String,
    pictures: String,
    add: String,
    ways: String,
    paste: String,
    waiting: String,
    kinds: String,
}

/// `picture`'s likeness covering `tile`, its middle kept, the corners rounded by `radius`.
fn covering(frame: &Frame, picture: &Picture, tile: Rect, radius: f32, scale: f32) {
    let properties = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
        dpiX: 96.0 * scale,
        dpiY: 96.0 * scale,
        bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
        ..Default::default()
    };
    let (width, height) = picture.size;
    let Ok(bitmap) = (unsafe { frame.dc.CreateBitmap(D2D_SIZE_U { width, height }, Some(picture.small.as_ptr().cast()), width * 4, &properties) }) else { return };
    // The part of it (in DIPs, as the bitmap is laid out at this scale) of the tile's shape, about its middle.
    let (w, h) = (width as f32 / scale, height as f32 / scale);
    let fit = (w / tile.w).min(h / tile.h);
    let (cut_w, cut_h) = (tile.w * fit, tile.h * fit);
    let source = rect((w - cut_w) / 2.0, (h - cut_h) / 2.0, cut_w, cut_h);
    let Ok(mask) = (unsafe { frame.gfx.factory.CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT { rect: rect(tile.x, tile.y, tile.w, tile.h), radiusX: radius, radiusY: radius }) }) else { return };
    let layer = D2D1_LAYER_PARAMETERS1 {
        contentBounds: rect(tile.x, tile.y, tile.w, tile.h),
        geometricMask: std::mem::ManuallyDrop::new(windows::core::Interface::cast(&mask).ok()),
        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
        maskTransform: windows_numerics::Matrix3x2::identity(),
        opacity: 1.0,
        ..Default::default()
    };
    unsafe {
        frame.dc.PushLayer(&layer, None);
        frame.dc.DrawBitmap(&bitmap, Some(&rect(tile.x, tile.y, tile.w, tile.h)), 1.0, D2D1_INTERPOLATION_MODE_LINEAR, Some(&source), None);
        frame.dc.PopLayer();
    }
    drop(std::mem::ManuallyDrop::into_inner(layer.geometricMask));
}

/// What the reader gave of their own: their words (none, perhaps), their
/// pictures, each a JPEG, and whether they ticked the choice.
pub struct Said {
    pub words: String,
    pub pictures: Vec<Vec<u8>>,
    pub option: bool,
}

/// How many characters (UTF-16 units) may be written, and how many pictures added (the site takes no more).
const WORDS_MOST: usize = 500;
pub const PICTURES_MOST: usize = 3;

/// Its measures (DIPs): its width, and its height where it has a part that
/// scrolls; the room round its content; the space between parts and under a
/// heading; the footer and its buttons; the box for words; a picture's
/// tile; the room inside a box round its text; and a turn of the wheel.
const WIDTH: f32 = 600.0;
const TALL: f32 = 760.0;
const PAD: f32 = 24.0;
const GAP: f32 = 20.0;
const HEAD: f32 = 28.0;
const FOOTER: f32 = 80.0;
const BUTTON: f32 = 32.0;
const WORDS_HEIGHT: f32 = 88.0;
const TILE: (f32, f32) = (120.0, 76.0);
const TILE_GAP: f32 = 8.0;
const INSET: (f32, f32) = (11.0, 8.0);
const WHEEL: f32 = 48.0;
/// The choice's line, and its box.
const OPTION: f32 = 24.0;
const OPTION_BOX: f32 = 18.0;
/// How far down the window's top may be held to move it.
const CAPTION: f32 = 56.0;

const TITLE_FONT: Font = Font::new(Family::Segoe, 20.0, 600.0);
const BODY: Font = Font::new(Family::Segoe, 14.0, 400.0);
const HEADING: Font = Font::new(Family::Segoe, 14.0, 600.0);
const SMALL: Font = Font::new(Family::Segoe, 12.0, 400.0);

/// What Windows sends a window that files are dropped on, besides WM_DROPFILES (WM_COPYGLOBALDATA).
const COPY_GLOBAL_DATA: u32 = 0x0049;

/// What the window tells itself to show the file chooser: from its own turn at the messages, with nothing of it held.
const CHOOSE: u32 = WM_APP + 1;

/// The clipboard's name for text (CF_UNICODETEXT).
const TEXT: u32 = 13;

/// What a point in the window is on, and what the keyboard may be on.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    Words,
    Add,
    Paste,
    Picture(usize),
    Remove(usize),
    Option,
    Sent,
    Thumb,
    /// A thread's report closed, or opened again (see `thread`).
    Turn,
    Yes,
    No,
}

/// What the window is drawn in, light or dark, as Windows 11's dialogs are: and, for a thread, the accent as text
/// is written in it, what the user's own messages are tinted with, a light ground under what is pointed at, and the
/// colours of a fix and of a failure.
struct Colours {
    content: Color,
    footer: Color,
    text: Color,
    text2: Color,
    text3: Color,
    field: Color,
    field_stroke: Color,
    field_bottom: Color,
    sent: Color,
    button: Color,
    button_hover: Color,
    button_stroke: Color,
    accent: Color,
    accent_text: Color,
    selection: Color,
    scroll: Color,
    ring: Color,
    shade: Color,
    mine: Color,
    subtle: Color,
    success: Color,
    danger: Color,
}

impl Colours {
    fn new(dark: bool) -> Self {
        let white = |a| Color::hex(0xFFFFFF, a);
        let black = |a| Color::hex(0x000000, a);
        let (accent, accent_text, selection) = (super::theme::accent(false), super::theme::accent(dark), super::theme::accent(dark).alpha(0.4));
        if dark {
            Colours {
                content: Color::hex(0x2B2B2B, 1.0),
                footer: Color::hex(0x202020, 1.0),
                text: white(1.0),
                text2: white(0.76),
                text3: white(0.5),
                field: white(0.06),
                field_stroke: white(0.08),
                field_bottom: white(0.55),
                sent: black(0.18),
                button: white(0.06),
                button_hover: white(0.09),
                button_stroke: white(0.07),
                accent,
                accent_text,
                selection,
                scroll: white(0.5),
                ring: white(1.0),
                shade: black(0.2),
                mine: accent_text.alpha(0.2),
                subtle: white(0.07),
                success: Color::hex(0x6CCB5F, 1.0),
                danger: Color::hex(0xFF99A4, 1.0),
            }
        } else {
            Colours {
                content: Color::hex(0xFFFFFF, 1.0),
                footer: Color::hex(0xF3F3F3, 1.0),
                text: black(0.9),
                text2: black(0.62),
                text3: black(0.45),
                field: white(1.0),
                field_stroke: black(0.1),
                field_bottom: black(0.45),
                sent: black(0.025),
                button: white(1.0),
                button_hover: Color::hex(0xF6F6F6, 1.0),
                button_stroke: black(0.1),
                accent,
                accent_text,
                selection,
                scroll: black(0.45),
                ring: black(0.9),
                shade: black(0.12),
                mine: accent.alpha(0.1),
                subtle: black(0.05),
                success: Color::hex(0x0F7B0F, 1.0),
                danger: Color::hex(0xC42B1C, 1.0),
            }
        }
    }
}

/// The text on the clipboard, its lines broken by `\n` alone, its tabs as spaces and nothing else unprintable.
fn clipboard_text(owner: HWND) -> Vec<u16> {
    let text = crate::clipboard::text(owner).unwrap_or_default().replace('\t', " ");
    text.chars().filter(|&c| c == '\n' || !c.is_control()).collect::<String>().encode_utf16().collect()
}

/// `text` put on the clipboard.
fn copy_text(owner: HWND, text: &[u16]) {
    let Some(_opened) = crate::clipboard::open(owner) else { return };
    unsafe {
        let _ = EmptyClipboard();
        if let Ok(memory) = GlobalAlloc(GMEM_MOVEABLE, (text.len() + 1) * 2) {
            let units = GlobalLock(memory) as *mut u16;
            if !units.is_null() {
                std::ptr::copy_nonoverlapping(text.as_ptr(), units, text.len());
                *units.add(text.len()) = 0;
                let _ = GlobalUnlock(memory);
                // (The clipboard owns it from here.)
                let _ = SetClipboardData(TEXT, Some(HANDLE(memory.0)));
            }
        }
    }
}

/// The window while it is up.
struct Dialog {
    hwnd: HWND,
    gfx: Rc<Gfx>,
    surface: Option<Surface>,
    scale: f32,
    dark: bool,
    title: String,
    intro: String,
    own: Option<OwnText>,
    /// Whether the button that pastes the clipboard's picture was drawn (there was room for it).
    chip_shown: bool,
    sent: Option<(String, String)>,
    /// The choice, what it says and whether it is ticked.
    option: Option<(String, bool)>,
    yes: String,
    no: String,
    typing: Typing,
    pictures: Vec<Picture>,
    /// How far what is sent is scrolled, and the most it can be.
    scroll: f32,
    scroll_most: f32,
    /// Its scroll bar: where its track starts, how long it is, and its thumb's length.
    track: (f32, f32, f32),
    pointer: Option<(f32, f32)>,
    pressed: Option<Hit>,
    /// Words being selected with the pointer, or the scroll bar held (this far down its thumb).
    selecting: bool,
    held: Option<f32>,
    focus: Hit,
    /// The focus shown, as it is once the keyboard has moved it.
    keyboard: bool,
    caret_on: bool,
    hits: Vec<(Rect, Hit)>,
    /// Those of the frame being drawn: they become `hits` once it is drawn whole (what is pointed at, while it is
    /// drawn, is what was under the pointer in the frame before).
    drawn: Vec<(Rect, Hit)>,
    /// The box the words are written in (inside its border), and where the caret was drawn.
    words_inside: Rect,
    caret: Option<Rect>,
    answer: Option<bool>,
    /// The report's thread it shows, kept up beside its owner (see `thread`), if it shows one.
    conversation: Option<thread::Conversation>,
}

thread_local! {
    /// The windows up on this thread: one asking, perhaps over a thread, the window that asked over that.
    static DIALOGS: RefCell<Vec<Dialog>> = const { RefCell::new(Vec::new()) };
}

#[cfg(test)]
thread_local! {
    /// Pictures a test has the window hold as it comes up (their files).
    static GIVEN_FOR_TEST: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// Where a test has the pointer be, whatever the real one does (DIPs).
    static POINTER_FOR_TEST: std::cell::Cell<Option<(f32, f32)>> = const { std::cell::Cell::new(None) };
}

/// Runs `act` on the window `hwnd`'s state, unless it is already in use further up this thread's stack (a message
/// sent while handling another), or the window is not one of these.
fn with<R>(hwnd: HWND, act: impl FnOnce(&mut Dialog) -> R) -> Option<R> {
    DIALOGS.with(|cell| cell.try_borrow_mut().ok().and_then(|mut dialogs| dialogs.iter_mut().find(|dialog| dialog.hwnd == hwnd).map(act)))
}

fn contains(r: Rect, x: f32, y: f32) -> bool {
    x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
}

impl Dialog {
    /// A window asking `title` and `intro`, the reader's own part `own` if they may add one, nothing chosen, typed
    /// or added yet, drawn with `gfx`.
    fn new(gfx: Rc<Gfx>, dark: bool, title: &str, intro: &str, own: Option<OwnText>) -> Self {
        Dialog {
            hwnd: HWND::default(),
            gfx,
            surface: None,
            scale: 1.0,
            dark,
            title: title.into(),
            intro: intro.into(),
            focus: if own.is_some() { Hit::Words } else { Hit::Yes },
            own,
            chip_shown: false,
            sent: None,
            option: None,
            yes: String::new(),
            no: String::new(),
            typing: Typing::most(WORDS_MOST),
            pictures: Vec::new(),
            scroll: 0.0,
            scroll_most: 0.0,
            track: (0.0, 0.0, 0.0),
            pointer: None,
            pressed: None,
            selecting: false,
            held: None,
            keyboard: false,
            caret_on: true,
            hits: Vec::new(),
            drawn: Vec::new(),
            words_inside: Rect { x: 0.0, y: 0.0, w: 200.0, h: 0.0 },
            caret: None,
            answer: None,
            conversation: None,
        }
    }

    fn redraw(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// How wide it is (DIPs).
    fn width(&self) -> f32 {
        if self.conversation.is_some() {
            thread::WIDTH
        } else {
            WIDTH
        }
    }

    /// How tall it is, laid out `width` wide (DIPs): as tall as what it says, or, with a part that scrolls, `TALL`.
    fn height(&self) -> f32 {
        if self.own.is_some() || self.sent.is_some() || self.conversation.is_some() {
            return TALL;
        }
        let inner = WIDTH - 2.0 * PAD;
        PAD + self.gfx.wrapped_height(&self.title, TITLE_FONT, inner) + 12.0 + self.gfx.wrapped_height(&self.intro, BODY, inner) + PAD + FOOTER
    }

    fn hit_at(&self, x: f32, y: f32) -> Option<Hit> {
        self.hits.iter().rev().find(|(r, _)| contains(*r, x, y)).map(|(_, hit)| *hit)
    }

    fn hovered(&self) -> Option<Hit> {
        #[cfg(test)]
        if let Some((x, y)) = POINTER_FOR_TEST.get() {
            return self.hit_at(x, y);
        }
        self.pointer.and_then(|(x, y)| self.hit_at(x, y))
    }

    /// Whether the window may be moved from `(x, y)`: its top (a thread's whole heading), where nothing is to be pressed.
    fn caption(&self, x: f32, y: f32) -> bool {
        let top = self.conversation.as_ref().map_or(CAPTION, |conversation| conversation.header);
        y < top && self.hit_at(x, y).is_none()
    }

    fn over_words(&self) -> bool {
        self.pointer.is_some_and(|(x, y)| self.hit_at(x, y) == Some(Hit::Words))
    }

    /// What the keyboard can go to, in order.
    fn stops(&self) -> Vec<Hit> {
        if self.conversation.is_some() {
            return self.thread_stops();
        }
        let mut stops = Vec::new();
        if self.own.is_some() {
            stops.push(Hit::Words);
            stops.extend((0..self.pictures.len()).map(Hit::Remove));
            if self.pictures.len() < PICTURES_MOST {
                stops.push(Hit::Add);
                if self.chip_shown {
                    stops.push(Hit::Paste);
                }
            }
        }
        if self.option.is_some() {
            stops.push(Hit::Option);
        }
        if self.sent.is_some() {
            stops.push(Hit::Sent);
        }
        stops.extend([Hit::Yes, Hit::No]);
        stops
    }

    fn focus_on(&mut self, hit: Hit) {
        self.focus = hit;
        // An input method only while words are written.
        unsafe {
            if hit == Hit::Words {
                let _ = ImmAssociateContextEx(self.hwnd, HIMC::default(), IACE_DEFAULT);
            } else {
                let _ = ImmAssociateContextEx(self.hwnd, HIMC::default(), 0);
            }
        }
        self.caret_on = true;
        self.redraw();
    }

    /// The input method's words and choices put at the caret, in the box's font.
    fn place_ime(&self) {
        let Some(caret) = self.caret else { return };
        let px = |dips: f32| (dips * self.scale).round() as i32;
        super::typing::place_ime(self.hwnd, POINT { x: px(caret.x), y: px(caret.y) }, px(caret.y + caret.h) - px(caret.y), "Segoe UI", px(BODY.size));
    }

    fn words_layout(&self) -> IDWriteTextLayout {
        self.gfx.wrapped(&self.typing.string(), BODY, self.words_inside.w)
    }

    fn add(&mut self, more: Vec<Picture>) {
        if self.written_fixed() {
            return;
        }
        self.pictures.extend(more);
        self.pictures.truncate(PICTURES_MOST);
        self.redraw();
    }

    fn likeness(&self) -> (u32, u32) {
        ((TILE.0 * self.scale).ceil() as u32, (TILE.1 * self.scale).ceil() as u32)
    }

    fn paste_pictures(&mut self) {
        let taken = picture::from_clipboard(self.hwnd, self.likeness());
        self.add(taken);
    }

    fn activate(&mut self, hit: Hit) {
        match hit {
            Hit::Yes | Hit::No if self.conversation.is_some() => self.tell(hit == Hit::Yes),
            Hit::Turn => self.tell_turn(),
            Hit::Yes => self.answer = Some(true),
            Hit::No => self.answer = Some(false),
            Hit::Add => unsafe {
                let _ = PostMessageW(Some(self.hwnd), CHOOSE, WPARAM(0), LPARAM(0));
            },
            Hit::Paste => self.paste_pictures(),
            Hit::Option | Hit::Remove(_) if self.written_fixed() => {}
            Hit::Option => {
                if let Some((_, ticked)) = &mut self.option {
                    *ticked = !*ticked;
                }
                self.redraw();
            }
            Hit::Remove(index) if index < self.pictures.len() => {
                self.pictures.remove(index);
                // The keyboard on what took its place, or what comes after the pictures.
                if self.focus == Hit::Remove(index) && index >= self.pictures.len() {
                    self.focus = if self.pictures.is_empty() { Hit::Add } else { Hit::Remove(self.pictures.len() - 1) };
                }
                self.redraw();
            }
            _ => {}
        }
    }

    fn moved(&mut self, x: f32, y: f32) {
        let before = self.hovered();
        self.pointer = Some((x, y));
        if self.selecting {
            let inside = self.words_inside;
            let at = at_point(&self.words_layout(), x - inside.x, y - inside.y + self.typing.top);
            self.typing.go(at, true);
            self.redraw();
        } else if let Some(grab) = self.held {
            let (start, length, thumb) = self.track;
            if length > thumb {
                self.scroll = ((y - grab - start) / (length - thumb) * self.scroll_most).clamp(0.0, self.scroll_most);
            }
            self.redraw();
        } else if self.hovered() != before {
            self.redraw();
        }
    }

    fn press(&mut self, x: f32, y: f32, shift: bool) {
        let hit = self.hit_at(x, y);
        self.pressed = hit;
        self.keyboard = false;
        match hit {
            Some(Hit::Words) => {
                self.focus_on(Hit::Words);
                let inside = self.words_inside;
                let at = at_point(&self.words_layout(), x - inside.x, y - inside.y + self.typing.top);
                self.typing.go(at, shift);
                self.selecting = true;
            }
            Some(Hit::Thumb) => {
                let (start, length, thumb) = self.track;
                let at = start + (length - thumb) * if self.scroll_most > 0.0 { self.scroll / self.scroll_most } else { 0.0 };
                self.held = Some(y - at);
            }
            Some(hit) if hit != Hit::Thumb => self.focus_on(hit),
            _ => {}
        }
        self.redraw();
    }

    fn release(&mut self, x: f32, y: f32) {
        let pressed = self.pressed.take();
        self.selecting = false;
        self.held = None;
        if let Some(pressed) = pressed.filter(|pressed| self.hit_at(x, y) == Some(*pressed)) {
            self.activate(pressed);
        }
        self.redraw();
    }

    fn wheel(&mut self, x: f32, y: f32, turns: f32) {
        if self.hit_at(x, y) == Some(Hit::Words) {
            self.typing.top -= turns * WHEEL;
        } else {
            self.scroll = (self.scroll - turns * WHEEL).clamp(0.0, self.scroll_most);
        }
        self.redraw();
    }

    fn key(&mut self, key: VIRTUAL_KEY, shift: bool, ctrl: bool) {
        match key {
            VK_ESCAPE => self.activate(Hit::No),
            VK_RETURN if ctrl => self.activate(Hit::Yes),
            VK_TAB => {
                let stops = self.stops();
                let at = stops.iter().position(|&stop| stop == self.focus);
                let next = match at {
                    Some(at) if shift => stops[(at + stops.len() - 1) % stops.len()],
                    Some(at) => stops[(at + 1) % stops.len()],
                    None => stops[0],
                };
                self.keyboard = true;
                self.focus_on(next);
            }
            // A picture on the clipboard is taken wherever the keyboard is; text, by the box for words.
            VK_V if ctrl && self.own.is_some() && self.pictures.len() < PICTURES_MOST && picture::on_clipboard() => self.paste_pictures(),
            _ if self.focus == Hit::Words => self.type_key(key, shift, ctrl),
            VK_RETURN | VK_SPACE if !matches!(self.focus, Hit::Sent) => {
                let focus = self.focus;
                self.activate(focus);
            }
            VK_UP | VK_DOWN | VK_PRIOR | VK_NEXT | VK_HOME | VK_END if self.focus == Hit::Sent => {
                let page = self.track.1;
                self.scroll = match key {
                    VK_UP => self.scroll - WHEEL,
                    VK_DOWN => self.scroll + WHEEL,
                    VK_PRIOR => self.scroll - page,
                    VK_NEXT => self.scroll + page,
                    VK_HOME => 0.0,
                    _ => self.scroll_most,
                }
                .clamp(0.0, self.scroll_most);
                self.redraw();
            }
            _ => {}
        }
    }

    /// A key pressed while words are written: the caret moved, words taken away, copied or pasted.
    fn type_key(&mut self, key: VIRTUAL_KEY, shift: bool, ctrl: bool) {
        let fixed = self.written_fixed();
        let typing = &mut self.typing;
        match key {
            VK_LEFT | VK_RIGHT => {
                let forward = key == VK_RIGHT;
                let selection = typing.selection();
                let to = if !shift && !selection.is_empty() { if forward { selection.end } else { selection.start } } else { typing.step(typing.caret, forward) };
                typing.go(to, shift);
            }
            VK_UP | VK_DOWN => {
                let layout = self.words_layout();
                let typing = &mut self.typing;
                let (x, top, height) = caret_at(&layout, typing.caret);
                let y = if key == VK_UP { top - height / 2.0 } else { top + height * 1.5 };
                let to = if y < 0.0 { 0 } else if y > height_of(&layout) { typing.text.len() } else { at_point(&layout, x, y) };
                typing.go(to, shift);
            }
            VK_HOME | VK_END if ctrl => typing.go(if key == VK_HOME { 0 } else { typing.text.len() }, shift),
            VK_HOME | VK_END => {
                let (start, end) = line_of(&self.words_layout(), self.typing.caret);
                self.typing.go(if key == VK_HOME { start } else { end }, shift);
            }
            VK_BACK | VK_DELETE | VK_RETURN if fixed => return,
            VK_BACK | VK_DELETE => typing.erase(key == VK_DELETE),
            VK_RETURN => typing.put(&[u16::from(b'\n')]),
            VK_A if ctrl => {
                typing.anchor = 0;
                typing.caret = typing.text.len();
            }
            VK_C | VK_X if ctrl => {
                let selection = typing.selection();
                if !selection.is_empty() {
                    copy_text(self.hwnd, &self.typing.text[selection]);
                    if key == VK_X && !fixed {
                        self.typing.put(&[]);
                    }
                }
            }
            VK_V if ctrl && fixed => return,
            VK_V if ctrl => {
                let pasted = clipboard_text(self.hwnd);
                self.typing.put(&pasted);
            }
            _ => return,
        }
        self.caret_on = true;
        self.redraw();
    }

    /// A character typed (or given by the input method), one UTF-16 unit at a time.
    fn char(&mut self, unit: u16) {
        // (Control characters come of keys taken as keys: Enter, Backspace, Ctrl with a letter.)
        if self.focus != Hit::Words || unit < 0x20 || unit == 0x7F || self.written_fixed() {
            return;
        }
        self.typing.put(&[unit]);
        self.caret_on = true;
        self.redraw();
    }

    fn draw(&mut self) {
        let mut inside = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut inside);
        }
        let size = (inside.right.max(0) as u32, inside.bottom.max(0) as u32);
        if size.0 == 0 || size.1 == 0 {
            return;
        }
        // On the device for its screen (see `gfx::on_screen`): its surface made anew on it as it is moved onto a
        // screen another adapter drives, or its device was lost.
        if let Ok(gfx) = gfx::for_window(self.hwnd) {
            if !Rc::ptr_eq(&gfx, &self.gfx) {
                self.surface = None;
                self.gfx = gfx;
            }
        }
        if self.surface.is_none() {
            self.surface = Surface::new(&self.gfx, self.hwnd).ok();
        }
        let Some(mut surface) = self.surface.take() else { return };
        let gfx = self.gfx.clone();
        let (scale, area) = (self.scale, (size.0 as f32 / self.scale, size.1 as f32 / self.scale));
        // (Not drawn, it keeps its surface, and draws again on it; the device gone, it makes one anew on the next.)
        match surface.draw(&gfx, size, scale, |frame| self.paint(frame, area)) {
            Err(error) if gfx::failed("dialog", &gfx, Some(&error)) => {}
            _ => self.surface = Some(surface),
        }
        if self.focus == Hit::Words {
            self.place_ime();
        }
    }

    fn paint(&mut self, frame: &Frame, (width, height): (f32, f32)) {
        self.drawn.clear();
        if self.conversation.is_some() {
            self.paint_thread(frame, (width, height));
            return self.drawn_whole();
        }
        let colours = Colours::new(self.dark);
        let c = &colours;
        let inner = width - 2.0 * PAD;
        let footer = height - FOOTER;
        frame.fill(c.content, 0.0, 0.0, width, height);
        frame.fill(c.footer, 0.0, footer, width, FOOTER);
        let mut y = PAD;
        frame.text_wrapped(&self.title, TITLE_FONT, c.text, PAD, y, inner);
        y += frame.gfx.wrapped_height(&self.title, TITLE_FONT, inner) + 12.0;
        frame.text_wrapped(&self.intro, BODY, c.text, PAD, y, inner);
        y += frame.gfx.wrapped_height(&self.intro, BODY, inner) + GAP;
        if let Some(own) = self.own.clone() {
            frame.text(&own.words, HEADING, c.text, PAD, y, inner, Align::Start);
            y += HEAD;
            self.paint_words(frame, c, Rect { x: PAD, y, w: inner, h: WORDS_HEIGHT }, &own.hint);
            y += WORDS_HEIGHT + GAP;
            frame.text(&own.pictures, HEADING, c.text, PAD, y, inner, Align::Start);
            y += HEAD;
            self.paint_pictures(frame, c, PAD, y, inner, &own);
            y += TILE.1 + GAP;
        }
        if let Some((words, ticked)) = self.option.clone() {
            self.paint_option(frame, c, Rect { x: PAD, y, w: inner, h: OPTION }, &words, ticked);
            y += OPTION + GAP;
        }
        if let Some((name, sent)) = self.sent.clone() {
            frame.text(&name, HEADING, c.text, PAD, y, inner, Align::Start);
            y += HEAD;
            self.paint_sent(frame, c, Rect { x: PAD, y, w: inner, h: (footer - GAP - y).max(60.0) }, &sent);
        }
        // The two answers, sharing the footer, as Windows 11's dialogs have them.
        let across = (inner - 8.0) / 2.0;
        let top = footer + (FOOTER - BUTTON) / 2.0;
        let (yes, no) = (self.yes.clone(), self.no.clone());
        self.button(frame, c, Rect { x: PAD, y: top, w: across, h: BUTTON }, &yes, Hit::Yes, true, true);
        self.button(frame, c, Rect { x: PAD + across + 8.0, y: top, w: across, h: BUTTON }, &no, Hit::No, false, true);
        self.drawn_whole();
    }

    /// The frame's own places from now, it drawn whole, and drawn again where what is pointed at is another among them
    /// (a picture's button, shown as the picture was pointed at, now under the pointer).
    fn drawn_whole(&mut self) {
        let before = self.hovered();
        self.hits = std::mem::take(&mut self.drawn);
        if self.hovered() != before {
            self.redraw();
        }
    }

    /// The ring round what the keyboard is on, once the keyboard has moved it.
    fn ring(&self, frame: &Frame, c: &Colours, r: Rect, radius: f32, hit: Hit) {
        if self.keyboard && self.focus == hit {
            frame.stroke_rounded(Fill::Solid(c.ring), r.x - 4.0, r.y - 4.0, r.w + 8.0, r.h + 8.0, radius + 4.0, 2.0);
        }
    }

    /// One line of `text`, centred in `r`.
    fn centred(frame: &Frame, text: &str, font: Font, colour: Color, r: Rect) {
        let (ascent, descent) = frame.gfx.baseline(font);
        let wide = frame.gfx.measure(text, font).min(r.w);
        frame.text(text, font, colour, r.x + (r.w - wide) / 2.0, r.y + (r.h - ascent - descent) / 2.0, wide + 1.0, Align::Start);
    }

    #[allow(clippy::too_many_arguments)]
    fn button(&mut self, frame: &Frame, c: &Colours, r: Rect, text: &str, hit: Hit, accent: bool, enabled: bool) {
        let hover = enabled && self.hovered() == Some(hit);
        let down = hover && self.pressed == Some(hit);
        if accent && !enabled {
            frame.fill_rounded(c.subtle, r.x, r.y, r.w, r.h, 6.0);
            Self::centred(frame, text, BODY, c.text3, r);
        } else if accent {
            frame.fill_rounded(c.accent.alpha(if down { 0.8 } else if hover { 0.9 } else { 1.0 }), r.x, r.y, r.w, r.h, 6.0);
            Self::centred(frame, text, BODY, Color::hex(0xFFFFFF, 1.0), r);
        } else {
            frame.fill_rounded(if hover { c.button_hover } else { c.button }, r.x, r.y, r.w, r.h, 6.0);
            frame.stroke_rounded(Fill::Solid(c.button_stroke), r.x, r.y, r.w, r.h, 6.0, 1.0);
            Self::centred(frame, text, BODY, if !enabled { c.text3 } else if down { c.text2 } else { c.text }, r);
        }
        self.ring(frame, c, r, 6.0, hit);
        if enabled {
            self.drawn.push((r, hit));
        }
    }

    fn paint_words(&mut self, frame: &Frame, c: &Colours, r: Rect, hint: &str) {
        let focused = self.focus == Hit::Words;
        frame.fill_rounded(c.field, r.x, r.y, r.w, r.h, 4.0);
        frame.stroke_rounded(Fill::Solid(c.field_stroke), r.x, r.y, r.w, r.h, 4.0, 1.0);
        // Its lower edge, in the accent and thicker while words are written in it.
        if focused {
            frame.fill(c.accent, r.x + 2.0, r.y + r.h - 2.0, r.w - 4.0, 2.0);
        } else {
            frame.fill(c.field_bottom, r.x + 2.0, r.y + r.h - 1.0, r.w - 4.0, 1.0);
        }
        let inside = Rect { x: r.x + INSET.0, y: r.y + INSET.1, w: r.w - 2.0 * INSET.0 - 6.0, h: r.h - 2.0 * INSET.1 };
        self.words_inside = inside;
        let text = self.typing.string();
        let layout = frame.gfx.wrapped(&text, BODY, inside.w);
        let (x, top, tall) = caret_at(&layout, self.typing.caret);
        // Scrolled to the caret after it moved; never past the words.
        if self.typing.follow {
            self.typing.top = self.typing.top.min(top).max(top + tall - inside.h);
            self.typing.follow = false;
        }
        let whole = height_of(&layout);
        self.typing.top = self.typing.top.clamp(0.0, (whole - inside.h).max(0.0));
        let shift = inside.y - self.typing.top;
        frame.clip(inside.x - 1.0, inside.y, inside.w + 2.0, inside.h);
        let selection = self.typing.selection();
        if !selection.is_empty() {
            let mut count = 0;
            let _ = unsafe { layout.HitTestTextRange(selection.start as u32, selection.len() as u32, 0.0, 0.0, None, &mut count) };
            let mut spans = vec![DWRITE_HIT_TEST_METRICS::default(); count as usize];
            let _ = unsafe { layout.HitTestTextRange(selection.start as u32, selection.len() as u32, 0.0, 0.0, Some(&mut spans), &mut count) };
            for span in &spans {
                frame.fill(c.selection, inside.x + span.left, shift + span.top, span.width.max(4.0), span.height);
            }
        }
        if text.is_empty() {
            frame.text_wrapped(hint, BODY, c.text3, inside.x, inside.y, inside.w);
        } else {
            frame.text_wrapped(&text, BODY, c.text, inside.x, shift, inside.w);
        }
        self.caret = focused.then_some(Rect { x: inside.x + x, y: shift + top, w: 1.0, h: tall });
        if focused && self.caret_on && selection.is_empty() {
            frame.fill(c.text, inside.x + x, shift + top, 1.0, tall);
        }
        frame.unclip();
        if whole > inside.h {
            let thumb = (inside.h * inside.h / whole).max(16.0);
            let at = inside.y + (inside.h - thumb) * self.typing.top / (whole - inside.h);
            frame.fill_rounded(c.scroll, r.x + r.w - 6.0, at, 3.0, thumb, 1.5);
        }
        self.drawn.push((r, Hit::Words));
    }

    /// The pictures given (pointed at, a button on each takes it away again), and the place to add more, as
    /// wide as is left of `width`.
    fn paint_pictures(&mut self, frame: &Frame, c: &Colours, left: f32, top: f32, width: f32, own: &OwnText) {
        let hovered = self.hovered();
        let x = self.paint_tiles(frame, c, left, top, TILE);
        self.chip_shown = false;
        if self.pictures.len() >= PICTURES_MOST {
            return;
        }
        // The place to add one: pressed, the file chooser; it says the other ways (dropping a file on the window,
        // Ctrl+V), and, with a picture on the clipboard, offers it.
        let zone = Rect { x, y: top, w: left + width - x, h: TILE.1 };
        let over = hovered == Some(Hit::Add);
        frame.fill_rounded(if over { c.button_hover } else { c.field }, zone.x, zone.y, zone.w, zone.h, 6.0);
        frame.stroke_rounded_dashed(if over { c.accent } else { c.field_bottom.alpha(0.6) }, zone.x, zone.y, zone.w, zone.h, 6.0, 1.0);
        self.ring(frame, c, zone, 6.0, Hit::Add);
        self.drawn.push((zone, Hit::Add));
        let middle = zone.y + zone.h / 2.0;
        frame.icon(Icon::ImagePlus, Point { x: zone.x + 26.0, y: middle }, 22.0, if over { c.accent } else { c.text2 });
        let words = zone.x + 50.0;
        let pasting = picture::on_clipboard();
        let chip = frame.gfx.measure(&own.paste, BODY) + 28.0;
        let said = frame.gfx.measure(&own.add, BODY).max(frame.gfx.measure(&own.ways, SMALL));
        self.chip_shown = pasting && words + said + 16.0 + chip + 12.0 <= zone.x + zone.w;
        let room = zone.x + zone.w - words - if self.chip_shown { chip + 24.0 } else { 12.0 };
        frame.text(&own.add, BODY, c.text, words, middle - 20.0, room, Align::Start);
        frame.text(if pasting && !self.chip_shown { &own.waiting } else { &own.ways }, SMALL, c.text3, words, middle + 2.0, room, Align::Start);
        if self.chip_shown {
            let paste = own.paste.clone();
            self.button(frame, c, Rect { x: zone.x + zone.w - 12.0 - chip, y: middle - 15.0, w: chip, h: 30.0 }, &paste, Hit::Paste, false, true);
        }
    }

    /// The pictures given, each in a `tile` (w, h) from `left` along `top` (pointed at, a button on each takes it away
    /// again): where the next would go.
    fn paint_tiles(&mut self, frame: &Frame, c: &Colours, left: f32, top: f32, tile: (f32, f32)) -> f32 {
        let hovered = self.hovered();
        let white = Color::hex(0xFFFFFF, 1.0);
        let mut x = left;
        for (index, picture) in self.pictures.iter().enumerate() {
            let tile = Rect { x, y: top, w: tile.0, h: tile.1 };
            frame.fill_rounded(c.sent, tile.x, tile.y, tile.w, tile.h, 6.0);
            covering(frame, picture, tile, 6.0, self.scale);
            frame.stroke_rounded(Fill::Solid(c.field_stroke), tile.x, tile.y, tile.w, tile.h, 6.0, 1.0);
            self.drawn.push((tile, Hit::Picture(index)));
            let keyed = self.keyboard && self.focus == Hit::Remove(index);
            if keyed || matches!(hovered, Some(Hit::Picture(at) | Hit::Remove(at)) if at == index) {
                frame.fill_rounded(c.shade, tile.x, tile.y, tile.w, tile.h, 6.0);
                // Its button to take it away, in the corner.
                let centre = Point { x: tile.x + tile.w - 15.0, y: tile.y + 15.0 };
                let on = hovered == Some(Hit::Remove(index));
                frame.fill_circle(Color::hex(0x000000, if on { 0.82 } else { 0.6 }), centre, 11.0);
                frame.icon(Icon::Close, centre, 13.0, white);
                let badge = Rect { x: centre.x - 13.0, y: centre.y - 13.0, w: 26.0, h: 26.0 };
                self.ring(frame, c, badge, 13.0, Hit::Remove(index));
                self.drawn.push((badge, Hit::Remove(index)));
            }
            x += tile.w + TILE_GAP;
        }
        x
    }

    /// The choice, its box ticked in the accent or ruled round, and what it says beside it.
    fn paint_option(&mut self, frame: &Frame, c: &Colours, r: Rect, words: &str, ticked: bool) {
        let top = r.y + (r.h - OPTION_BOX) / 2.0;
        let over = self.hovered() == Some(Hit::Option);
        if ticked {
            frame.fill_rounded(c.accent.alpha(if over { 0.9 } else { 1.0 }), r.x, top, OPTION_BOX, OPTION_BOX, 4.0);
            frame.icon(Icon::Check, Point { x: r.x + OPTION_BOX / 2.0, y: top + OPTION_BOX / 2.0 }, 14.0, Color::hex(0xFFFFFF, 1.0));
        } else {
            frame.fill_rounded(if over { c.button_hover } else { c.field }, r.x, top, OPTION_BOX, OPTION_BOX, 4.0);
            frame.stroke_rounded(Fill::Solid(c.field_bottom), r.x + 0.5, top + 0.5, OPTION_BOX - 1.0, OPTION_BOX - 1.0, 4.0, 1.0);
        }
        let (ascent, descent) = frame.gfx.baseline(BODY);
        let wide = frame.gfx.measure(words, BODY).min(r.w - OPTION_BOX - 10.0);
        frame.text(words, BODY, c.text, r.x + OPTION_BOX + 10.0, r.y + (r.h - ascent - descent) / 2.0, wide + 1.0, Align::Start);
        let hit = Rect { x: r.x, y: r.y, w: OPTION_BOX + 10.0 + wide, h: r.h };
        self.ring(frame, c, Rect { x: r.x, y: top, w: OPTION_BOX, h: OPTION_BOX }, 4.0, Hit::Option);
        self.drawn.push((hit, Hit::Option));
    }

    fn paint_sent(&mut self, frame: &Frame, c: &Colours, r: Rect, sent: &str) {
        frame.fill_rounded(c.sent, r.x, r.y, r.w, r.h, 4.0);
        frame.stroke_rounded(Fill::Solid(c.field_stroke), r.x, r.y, r.w, r.h, 4.0, 1.0);
        let inside = Rect { x: r.x + INSET.0, y: r.y + INSET.1, w: r.w - 2.0 * INSET.0 - 8.0, h: r.h - 2.0 * INSET.1 };
        let whole = frame.gfx.wrapped_height(sent, SMALL, inside.w);
        self.scroll_most = (whole - inside.h).max(0.0);
        self.scroll = self.scroll.clamp(0.0, self.scroll_most);
        frame.clip(inside.x, inside.y, inside.w, inside.h);
        frame.text_wrapped(sent, SMALL, c.text2, inside.x, inside.y - self.scroll, inside.w);
        frame.unclip();
        self.ring(frame, c, r, 4.0, Hit::Sent);
        self.drawn.push((r, Hit::Sent));
        if self.scroll_most > 0.0 {
            let thumb = (inside.h * inside.h / whole).max(24.0);
            let at = inside.y + (inside.h - thumb) * self.scroll / self.scroll_most;
            self.track = (inside.y, inside.h, thumb);
            let wide = matches!(self.hovered(), Some(Hit::Thumb)) || self.held.is_some();
            let across = if wide { 6.0 } else { 3.0 };
            frame.fill_rounded(c.scroll, r.x + r.w - 5.0 - across, at, across, thumb, across / 2.0);
            self.drawn.push((Rect { x: r.x + r.w - 16.0, y: at, w: 16.0, h: thumb }, Hit::Thumb));
        } else {
            self.track = (inside.y, inside.h, inside.h);
        }
    }
}

/// The picture files the reader chooses (in Windows' own chooser, over
/// `owner`; `kinds` is what it calls them): their paths.
fn chosen(owner: HWND, kinds: &str) -> Vec<String> {
    use windows::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OFN_ALLOWMULTISELECT, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OFN_NOCHANGEDIR, OPENFILENAMEW};
    let filter: Vec<u16> = format!("{kinds}\0*.png;*.jpg;*.jpeg;*.bmp;*.gif;*.webp\0\0").encode_utf16().collect();
    let mut paths = vec![0u16; 16 << 10];
    let mut asked = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: windows::core::PWSTR(paths.as_mut_ptr()),
        nMaxFile: paths.len() as u32,
        Flags: OFN_EXPLORER | OFN_ALLOWMULTISELECT | OFN_FILEMUSTEXIST | OFN_HIDEREADONLY | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    if !unsafe { GetOpenFileNameW(&mut asked) }.as_bool() {
        return Vec::new();
    }
    // One file chosen is its whole path; several are their folder, then each one's name.
    let named: Vec<String> = paths.split(|&c| c == 0).take_while(|part| !part.is_empty()).map(String::from_utf16_lossy).collect();
    match named.split_first() {
        Some((folder, names)) if !names.is_empty() => names.iter().map(|name| format!("{}\\{name}", folder.trim_end_matches('\\'))).collect(),
        _ => named,
    }
}

fn point(lparam: LPARAM) -> (f32, f32) {
    ((lparam.0 & 0xFFFF) as u16 as i16 as f32, ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32)
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let held = |key: VIRTUAL_KEY| unsafe { GetKeyState(key.0 as i32) } < 0;
    match message {
        // All of it is the window's own to draw: no frame (Windows keeps its shadow and round corners).
        WM_NCCALCSIZE if wparam.0 != 0 => LRESULT(0),
        WM_NCHITTEST => {
            let (x, y) = point(lparam);
            let mut at = POINT { x: x as i32, y: y as i32 };
            unsafe {
                let _ = ScreenToClient(hwnd, &mut at);
            }
            let caption = with(hwnd, |dialog| dialog.caption(at.x as f32 / dialog.scale, at.y as f32 / dialog.scale)).unwrap_or(false);
            LRESULT(if caption { HTCAPTION } else { HTCLIENT } as isize)
        }
        WM_PAINT => {
            unsafe {
                let _ = ValidateRect(Some(hwnd), None);
            }
            with(hwnd, Dialog::draw);
            LRESULT(0)
        }
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
            let typing = with(hwnd, |dialog| dialog.over_words()).unwrap_or(false);
            unsafe { SetCursor(LoadCursorW(None, if typing { IDC_IBEAM } else { IDC_ARROW }).ok()) };
            LRESULT(1)
        }
        WM_MOUSEMOVE => {
            let (x, y) = point(lparam);
            with(hwnd, |dialog| {
                if dialog.pointer.is_none() {
                    let mut track = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    let _ = unsafe { TrackMouseEvent(&mut track) };
                }
                dialog.moved(x / dialog.scale, y / dialog.scale);
            });
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with(hwnd, |dialog| {
                dialog.pointer = None;
                dialog.redraw();
            });
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = point(lparam);
            unsafe { SetCapture(hwnd) };
            with(hwnd, |dialog| dialog.press(x / dialog.scale, y / dialog.scale, held(VK_SHIFT)));
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = point(lparam);
            unsafe {
                let _ = ReleaseCapture();
            }
            with(hwnd, |dialog| dialog.release(x / dialog.scale, y / dialog.scale));
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let (x, y) = point(lparam);
            let mut at = POINT { x: x as i32, y: y as i32 };
            unsafe {
                let _ = ScreenToClient(hwnd, &mut at);
            }
            let turns = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / 120.0;
            with(hwnd, |dialog| dialog.wheel(at.x as f32 / dialog.scale, at.y as f32 / dialog.scale, turns));
            LRESULT(0)
        }
        WM_KEYDOWN => {
            with(hwnd, |dialog| dialog.key(VIRTUAL_KEY(wparam.0 as u16), held(VK_SHIFT), held(VK_CONTROL)));
            LRESULT(0)
        }
        WM_CHAR => {
            with(hwnd, |dialog| dialog.char(wparam.0 as u16));
            LRESULT(0)
        }
        // The caret blinks.
        WM_TIMER => {
            with(hwnd, |dialog| {
                dialog.caret_on = !dialog.caret_on;
                if dialog.focus == Hit::Words {
                    dialog.redraw();
                }
            });
            LRESULT(0)
        }
        // Files dropped on the window: those that are pictures are added.
        WM_DROPFILES => {
            let files = HDROP(wparam.0 as *mut _);
            let paths: Vec<String> = (0..unsafe { DragQueryFileW(files, u32::MAX, None) })
                .map(|index| {
                    let mut path = vec![0u16; unsafe { DragQueryFileW(files, index, None) } as usize + 1];
                    let length = unsafe { DragQueryFileW(files, index, Some(&mut path)) } as usize;
                    String::from_utf16_lossy(&path[..length])
                })
                .collect();
            unsafe { DragFinish(files) };
            with(hwnd, |dialog| {
                if dialog.own.is_some() {
                    let small = dialog.likeness();
                    let more = paths.iter().filter_map(|path| picture::from_file(path, small)).collect();
                    dialog.add(more);
                }
            });
            LRESULT(0)
        }
        // The file chooser, which has its own turn at the messages: shown with nothing of the window's held.
        CHOOSE => {
            let asked = with(hwnd, |dialog| (dialog.own.as_ref().map(|own| own.kinds.clone()).unwrap_or_default(), dialog.likeness()));
            if let Some((kinds, small)) = asked {
                let more = chosen(hwnd, &kinds).iter().filter_map(|path| picture::from_file(path, small)).collect();
                with(hwnd, |dialog| dialog.add(more));
            }
            LRESULT(0)
        }
        // (Whether a picture may be pasted.)
        WM_CLIPBOARDUPDATE => {
            with(hwnd, |dialog| dialog.redraw());
            LRESULT(0)
        }
        WM_IME_STARTCOMPOSITION => {
            with(hwnd, |dialog| dialog.place_ime());
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_DPICHANGED => {
            let suggested = unsafe { *(lparam.0 as *const RECT) };
            with(hwnd, |dialog| dialog.scale = (wparam.0 & 0xFFFF) as f32 / 96.0);
            unsafe {
                let _ = SetWindowPos(hwnd, None, suggested.left, suggested.top, suggested.right - suggested.left, suggested.bottom - suggested.top, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            with(hwnd, |dialog| dialog.redraw());
            LRESULT(0)
        }
        WM_CLOSE => {
            with(hwnd, |dialog| dialog.activate(Hit::No));
            LRESULT(0)
        }
        // Gone with the window it is over (a thread's, kept up beside it): what it held goes with it.
        WM_DESTROY => {
            DIALOGS.with(|cell| {
                if let Ok(mut dialogs) = cell.try_borrow_mut() {
                    dialogs.retain(|dialog| dialog.hwnd != hwnd);
                }
            });
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// The device for a dialog over `owner` (see `gfx::on_screen`): on its screen, or with none, the pointer's (where it
/// opens then).
pub(super) fn device_over(owner: HWND) -> windows::core::Result<Rc<Gfx>> {
    if owner.is_invalid() {
        gfx::at_pointer()
    } else {
        gfx::for_window(owner)
    }
}

/// Asks `asked` in a window over `owner`: what the reader gave of their
/// own (nothing, where they were not asked for it), if the answer was its
/// `yes`; none, if not. With no `owner` (asked from a thread of its own, for
/// what was pressed on the panel or a widget), it is at the pointer, over the
/// windows kept above others as the panel is.
pub fn show(owner: HWND, asked: &Asked) -> Option<Said> {
    let gfx = device_over(owner).ok()?;
    // Shown till it returns (see `life`).
    let _showing = super::life::Showing::of(&gfx);
    let own = asked.own.as_ref().map(|own| OwnText {
        words: own.words.into(),
        hint: own.hint.into(),
        pictures: own.pictures.into(),
        add: own.add.into(),
        ways: own.ways.into(),
        paste: own.paste.into(),
        waiting: own.waiting.into(),
        kinds: own.kinds.into(),
    });
    let mut dialog = Dialog::new(gfx, asked.dark, asked.title, asked.intro, own);
    dialog.sent = asked.sent.map(|(name, text)| (name.into(), text.into()));
    dialog.option = asked.option.map(|words| (words.into(), false));
    dialog.yes = asked.yes.into();
    dialog.no = asked.no.into();
    // (From its end: as far down as it goes, once it is laid out.)
    dialog.scroll = if asked.from_end { f32::MAX } else { 0.0 };
    let window = unsafe {
        put_up(owner, dialog, |_dialog| {
            // (Pictures a test has it hold as it comes up.)
            #[cfg(test)]
            for path in GIVEN_FOR_TEST.take() {
                _dialog.pictures.extend(picture::from_file(&path, _dialog.likeness()));
            }
        })
    }?;
    // Its own turn at the messages, till it is answered (or gone).
    let mut message = MSG::default();
    let mut quit = None;
    unsafe {
        while with(window, |dialog| dialog.answer.is_none()).unwrap_or(false) && IsWindow(Some(window)).as_bool() {
            if !GetMessageW(&mut message, None, 0, 0).as_bool() {
                quit = Some(message.wParam.0 as i32);
                break;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    let dialog = take_down(window, owner)?;
    if let Some(code) = quit {
        unsafe { PostQuitMessage(code) };
    }
    let words = dialog.typing.string().trim().to_string();
    let option = dialog.option.as_ref().is_some_and(|(_, ticked)| *ticked);
    (dialog.answer == Some(true)).then(|| Said { words, pictures: dialog.pictures.into_iter().map(|picture| picture.jpeg).collect(), option })
}

/// `dialog`'s window made and put up over `owner`, which takes nothing meanwhile: in the middle of it, within its
/// screen, no taller than the screen (what scrolls in it scrolls the more), the size `dialog` asks for at its scale.
/// With no `owner` (asked from a thread of its own, for what was pressed on the panel or a widget), at the pointer,
/// over the windows kept above others as the panel is. `ready` is done with it once its scale is known, before it is
/// first drawn. The window, if it was made.
unsafe fn put_up(owner: HWND, mut dialog: Dialog, ready: impl FnOnce(&mut Dialog)) -> Option<HWND> {
    unsafe {
        let module = GetModuleHandleW(None).ok()?;
        let class = w!("GlanceAsk");
        // (Registered once; again is refused, which is as well.)
        RegisterClassExW(&WNDCLASSEXW { cbSize: size_of::<WNDCLASSEXW>() as u32, lpfnWndProc: Some(procedure), hInstance: module.into(), lpszClassName: class, ..Default::default() });
        // Where it goes: over its owner, or (none) at the pointer.
        let owned = !owner.is_invalid();
        let mut over = RECT::default();
        if !owned || GetWindowRect(owner, &mut over).is_err() {
            let mut pointer = POINT::default();
            let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pointer);
            over = RECT { left: pointer.x, top: pointer.y, right: pointer.x, bottom: pointer.y };
        }
        let monitor = windows::Win32::Graphics::Gdi::MonitorFromRect(&over, MONITOR_DEFAULTTONEAREST);
        let scale = if owned {
            GetDpiForWindow(owner).max(96) as f32 / 96.0
        } else {
            let (mut dpi, mut unused) = (96u32, 96u32);
            let _ = windows::Win32::UI::HiDpi::GetDpiForMonitor(monitor, windows::Win32::UI::HiDpi::MDT_EFFECTIVE_DPI, &mut dpi, &mut unused);
            dpi.max(96) as f32 / 96.0
        };
        dialog.scale = scale;
        let mut screen = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(monitor, &mut screen);
        let room = screen.rcWork;
        let wide = ((dialog.width() * scale) as i32).min(room.right - room.left);
        let tall = ((dialog.height() * scale) as i32).min((room.bottom - room.top) * 19 / 20);
        let (x, y) = (((over.left + over.right - wide) / 2).clamp(room.left, room.right - wide), ((over.top + over.bottom - tall) / 2).clamp(room.top, room.bottom - tall));
        let above = if owned { WS_EX_NOREDIRECTIONBITMAP } else { WS_EX_NOREDIRECTIONBITMAP | windows::Win32::UI::WindowsAndMessaging::WS_EX_TOPMOST };
        let window = CreateWindowExW(above, class, &HSTRING::from(dialog.title.as_str()), WS_POPUP | WS_CAPTION | WS_SYSMENU, x, y, wide, tall, owned.then_some(owner), None, Some(module.into()), None).ok()?;
        dialog.hwnd = window;
        // Its shadow and (on Windows 11) its round corners, as Windows gives a window with a frame.
        let _ = DwmExtendFrameIntoClientArea(window, &MARGINS { cxLeftWidth: 1, cxRightWidth: 1, cyTopHeight: 1, cyBottomHeight: 1 });
        let round = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(window, DWMWA_WINDOW_CORNER_PREFERENCE, &round as *const _ as *const _, size_of_val(&round) as u32);
        // (Its frame gone at once: the size it was made at is the size it is.)
        let _ = SetWindowPos(window, None, x, y, wide, tall, SWP_NOZORDER | SWP_NOACTIVATE | windows::Win32::UI::WindowsAndMessaging::SWP_FRAMECHANGED);
        ready(&mut dialog);
        let (focus, takes_pictures) = (dialog.focus, dialog.own.is_some());
        DIALOGS.with(|cell| cell.borrow_mut().push(dialog));
        with(window, |dialog| {
            dialog.focus_on(focus);
            dialog.draw();
        });
        let _ = AddClipboardFormatListener(window);
        // Files dropped on it, from Explorer too (which is not elevated, as Glance is: what it sends is let through).
        if takes_pictures {
            DragAcceptFiles(window, true);
            for message in [WM_DROPFILES, WM_COPYDATA, COPY_GLOBAL_DATA] {
                let _ = ChangeWindowMessageFilterEx(window, message, MSGFLT_ALLOW, None);
            }
        }
        let _ = SetTimer(Some(window), 1, GetCaretBlinkTime(), None);

        let _ = EnableWindow(owner, false);
        let _ = ShowWindow(window, SW_SHOW);
        let _ = SetForegroundWindow(window);
        let _ = SetFocus(Some(window));
        Some(window)
    }
}

/// The window `window` taken down, and `owner` given back what it takes: its state, if it was still up.
fn take_down(window: HWND, owner: HWND) -> Option<Dialog> {
    let dialog = DIALOGS.with(|cell| {
        let mut dialogs = cell.borrow_mut();
        let at = dialogs.iter().position(|dialog| dialog.hwnd == window)?;
        Some(dialogs.remove(at))
    });
    unsafe {
        let _ = KillTimer(Some(window), 1);
        let _ = RemoveClipboardFormatListener(window);
        let _ = EnableWindow(owner, true);
        let _ = DestroyWindow(window);
        let _ = SetForegroundWindow(owner);
    }
    dialog
}

#[cfg(test)]
mod tests {
    use super::Typing;

    fn typed(text: &str) -> Typing {
        let mut typing = Typing::most(super::WORDS_MOST);
        typing.put(&text.encode_utf16().collect::<Vec<_>>());
        typing
    }

    #[test]
    fn words_are_written_and_taken_away_a_character_at_a_time() {
        // (One character outside the basic plane: two units, taken as one.)
        let mut typing = typed("主板😀");
        assert_eq!(typing.caret, 4);
        typing.erase(false);
        assert_eq!(typing.string(), "主板");
        typing.go(0, false);
        typing.go(typing.step(0, true), true);
        typing.put(&"CPU".encode_utf16().collect::<Vec<_>>());
        assert_eq!(typing.string(), "CPU板");
        assert_eq!((typing.caret, typing.anchor), (3, 3));
        typing.erase(true);
        assert_eq!(typing.string(), "CPU");
    }

    #[test]
    fn no_more_is_written_than_there_is_room_for() {
        let mut typing = typed(&"字".repeat(super::WORDS_MOST - 1));
        // The last unit free: a character of two units does not fit, nor is it cut.
        typing.put(&"😀".encode_utf16().collect::<Vec<_>>());
        assert_eq!(typing.text.len(), super::WORDS_MOST - 1);
        typing.put(&"ab".encode_utf16().collect::<Vec<_>>());
        assert_eq!(typing.text.len(), super::WORDS_MOST);
        assert!(typing.string().ends_with('a'));
    }

    /// Puts the window up for a few seconds, to be looked at: `cargo test -- --ignored shows_the_window`.
    #[test]
    #[ignore = "puts a window up on this PC's desktop"]
    fn shows_the_window() {
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, FindWindowW, PostMessageW, WM_CLOSE, WS_POPUP, WS_VISIBLE};
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
            let owner = CreateWindowExW(Default::default(), w!("STATIC"), w!(""), WS_POPUP | WS_VISIBLE, 300, 200, 900, 700, None, None, None, None).unwrap();
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_secs(std::env::var("FOR").ok().and_then(|s| s.parse().ok()).unwrap_or(8)));
                let _ = PostMessageW(FindWindowW(w!("GlanceAsk"), None).ok(), WM_CLOSE, Default::default(), Default::default());
            });
            let dark = std::env::var("DARK").is_ok();
            // Pictures to hold, by their files, as `PICTURES` names them (split by `;`).
            super::POINTER_FOR_TEST.set(std::env::var("POINTER").ok().and_then(|at| at.split_once(',').and_then(|(x, y)| Some((x.parse().ok()?, y.parse().ok()?)))));
            super::GIVEN_FOR_TEST.set(std::env::var("PICTURES").map(|paths| paths.split(';').map(str::to_string).collect()).unwrap_or_default());
            let text = (0..60).map(|line| format!("Line {line} of what is sent")).collect::<Vec<_>>().join("\n");
            let own = super::Own { words: "你想说的话（可不填）", hint: "比如：主板温度没有显示；风扇转速和别的软件里看到的不一样", pictures: "截图（可不添，最多 3 张）", add: "添加截图", ways: "点击选择、拖进窗口，或按 Ctrl+V 粘贴", paste: "粘贴剪贴板里的截图", waiting: "剪贴板里有截图，按 Ctrl+V 即可粘贴", kinds: "图片" };
            let asked = super::Asked { title: "发送诊断信息", intro: "下面是将要发送给 Glance（glancepc.com）的全部内容：你写的话、你添加的截图，和最下面的诊断信息。不包含你的文件、账号或网络地址。", own: Some(own), option: std::env::var("OPTION").is_ok().then_some("附上当前的诊断信息"), sent: Some(("诊断信息", &text)), from_end: std::env::var("FROM_END").is_ok(), yes: "发送", no: "取消", dark };
            assert!(super::show(owner, &asked).is_none());
        }
    }
}
