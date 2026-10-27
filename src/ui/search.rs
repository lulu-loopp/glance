//! Global search: a PowerToys Run-like launcher that opens on its own
//! shortcut, in the middle of the screen. Its box searches the Start menu's
//! programs (by name or pinyin initials), what is kept at hand, the files
//! come lately, and Glance's own settings; a sum typed in is worked out, a
//! web address opens, and what is not found anywhere is searched for on the
//! web. Up and down move through the results, Enter opens the chosen one,
//! Esc or a click outside closes it. Drawn as Glance draws its other windows,
//! in the skin chosen.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Mutex, OnceLock};

use windows::core::{w, HSTRING};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, InvalidateRect, MonitorFromPoint, ValidateRect, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::Ime::{
    ImmGetContext, ImmReleaseContext, ImmSetCandidateWindow, ImmSetCompositionFontW, ImmSetCompositionWindow, CANDIDATEFORM, CFS_EXCLUDE, CFS_POINT, COMPOSITIONFORM,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, SetFocus, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_UP};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetCaretBlinkTime, GetClientRect, GetCursorPos, GetMessageW, KillTimer, LoadCursorW, PostMessageW, RegisterClassW,
    SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, TranslateMessage, HWND_MESSAGE, IDC_ARROW, MSG, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOW, WA_INACTIVE,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_ACTIVATE, WM_APP, WM_CHAR, WM_CLOSE, WM_DESTROY, WM_DPICHANGED, WM_IME_STARTCOMPOSITION, WM_KEYDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use super::canvas::{Align, Canvas, Color, Fill, Picture, Point};
use super::gfx::{self, Frame, Gfx, Surface};
use super::icons::Icon;
use super::text::Lang;
use super::theme::{self, Skin, Theme};
use super::typing::Typing;
use super::view::Rect;
use crate::station::apps;
use crate::station::found;
use crate::station::recent;
use crate::station::run::{self, Ask, FilesAct};

/// Its width and its parts (DIPs): the search row, a result's row, how many
/// rows show, and the foot with what the keys do.
const WIDTH: f32 = 640.0;
const SEARCH_ROW: f32 = 64.0;
const ROW: f32 = 48.0;
const ROWS: usize = 7;
const FOOTER: f32 = 36.0;
const PAD: f32 = 16.0;
const HEIGHT: f32 = SEARCH_ROW + 1.0 + ROWS as f32 * ROW + FOOTER;
const SEARCH_MOST: usize = 128;
/// How often the pictures of what is listed are looked for again while it is
/// up, and the caret blinks.
const AGAIN_MS: u32 = 350;

/// What is asked of its thread.
enum Asked {
    Toggle,
}

static ASKS: OnceLock<Mutex<Sender<Asked>>> = OnceLock::new();
static POST: AtomicIsize = AtomicIsize::new(0);
static WINDOW: AtomicIsize = AtomicIsize::new(0);

/// Its messages: something asked.
const ASKED: u32 = WM_APP + 11;
/// Its timers: the caret's blink, and looking again for pictures and for the
/// Start menu read.
const BLINK: usize = 1;
const AGAIN: usize = 2;

thread_local! {
    static SEARCH: RefCell<Option<Search>> = const { RefCell::new(None) };
}

fn with<R>(act: impl FnOnce(&mut Search) -> R) -> Option<R> {
    SEARCH.with(|cell| cell.try_borrow_mut().ok().and_then(|mut search| search.as_mut().map(act)))
}

/// Opens global search, or closes it where it is up.
pub fn toggle() {
    ask(Asked::Toggle);
}

fn ask(asked: Asked) {
    let sender = ASKS.get_or_init(|| {
        let (sender, asks) = std::sync::mpsc::channel();
        std::thread::Builder::new().name("glance: search".into()).spawn(move || run(asks)).expect("a thread started");
        Mutex::new(sender)
    });
    if sender.lock().unwrap().send(asked).is_ok() {
        let post = POST.load(Ordering::Acquire);
        if post != 0 {
            let _ = unsafe { PostMessageW(Some(HWND(post as *mut _)), ASKED, WPARAM(0), LPARAM(0)) };
        }
    }
}

fn run(asks: Receiver<Asked>) {
    unsafe {
        let post_class = WNDCLASSW { lpfnWndProc: Some(procedure), lpszClassName: w!("GlanceSearchAsks"), ..Default::default() };
        RegisterClassW(&post_class);
        let Ok(post) = CreateWindowExW(WINDOW_EX_STYLE(0), w!("GlanceSearchAsks"), w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, None, None) else { return };
        POST.store(post.0 as isize, Ordering::Release);
        let page = WNDCLASSW { lpfnWndProc: Some(procedure), lpszClassName: w!("GlanceSearch"), hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(), ..Default::default() };
        RegisterClassW(&page);
        // (Asks sent before its window was there.)
        let _ = PostMessageW(Some(post), ASKED, WPARAM(0), LPARAM(0));
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            if message.hwnd == post && message.message == ASKED {
                while let Ok(asked) = asks.try_recv() {
                    match asked {
                        Asked::Toggle => toggle_now(),
                    }
                }
                continue;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

/// Opens it where it is closed, or closes it where it is up.
fn toggle_now() {
    let exists = with(|search| {
        if search.shown {
            search.hide_now();
        } else {
            search.show_now();
        }
    })
    .is_some();
    if exists {
        return;
    }
    let Some(mut search) = Search::make() else { return };
    search.show_now();
    let hwnd = search.hwnd;
    SEARCH.with(|cell| *cell.borrow_mut() = Some(search));
    WINDOW.store(hwnd.0 as isize, Ordering::Release);
}

/// What is listed: a program, a file, one of Glance's own places, a sum's
/// answer, or a place on the web.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    App,
    File,
    Settings,
    Calculator,
    Web,
}

#[derive(Clone, Debug)]
struct Item {
    kind: Kind,
    name: String,
    subtitle: String,
    /// What opens it: a shell name or a path for a program or a file, an
    /// address for what is on the web; for a sum, the answer itself.
    target: String,
}

impl Item {
    fn fallback_icon(&self) -> Icon {
        match self.kind {
            Kind::App => Icon::LayoutGrid,
            Kind::File => Icon::File,
            Kind::Settings => Icon::Settings,
            Kind::Calculator => Icon::SquareCode,
            Kind::Web => Icon::Link,
        }
    }

    fn tag(&self, lang: Lang) -> &'static str {
        match self.kind {
            Kind::App => lang.pick("程序", "App"),
            Kind::File => lang.pick("文件", "File"),
            Kind::Settings => lang.pick("设置", "Settings"),
            Kind::Calculator => lang.pick("计算", "Calculator"),
            Kind::Web => lang.pick("网页", "Web"),
        }
    }
}

/// What it is drawn with and shows.
struct Search {
    hwnd: HWND,
    gfx: Rc<Gfx>,
    surface: Option<Surface>,
    _showing: super::life::Showing,
    scale: f32,
    inset: f32,
    theme: Rc<Theme>,
    lang: Lang,
    search: Typing,
    results: Vec<Item>,
    /// The result chosen, and the first of those shown.
    chosen: usize,
    first: usize,
    /// Pictures of what is listed, by what they are and how large.
    icons: HashMap<(String, u32), Picture>,
    pointer: Option<(f32, f32)>,
    hits: Vec<(f32, f32, f32, f32, usize)>,
    caret: Option<(f32, f32, f32)>,
    caret_on: bool,
    shown: bool,
}

impl Search {
    fn make() -> Option<Self> {
        let gfx = gfx::current().ok()?;
        let (theme, lang) = look_now();
        let scale = scale_at_cursor();
        let inset = theme.margin;
        let room = (inset * scale).round() as i32;
        let (wide, tall) = ((WIDTH * scale).round() as i32, (HEIGHT * scale).round() as i32);
        let (x, y) = place_at_cursor(wide + 2 * room, tall + 2 * room);
        let hwnd = unsafe { CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOREDIRECTIONBITMAP, w!("GlanceSearch"), &HSTRING::from("Glance"), WS_POPUP, x - room, y - room, wide + 2 * room, tall + 2 * room, None, None, None, None) }.ok()?;
        unsafe {
            let square = DWMWCP_DONOTROUND;
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as *const _, size_of_val(&square) as u32);
            let off = windows::core::BOOL(1);
            let _ = DwmSetWindowAttribute(hwnd, windows::Win32::Graphics::Dwm::DWMWA_TRANSITIONS_FORCEDISABLED, &off as *const _ as *const _, size_of_val(&off) as u32);
        }
        let showing = super::life::Showing::of(&gfx);
        let surface = Surface::new(&gfx, hwnd).ok();
        Some(Search {
            hwnd,
            gfx,
            surface,
            _showing: showing,
            scale,
            inset,
            theme,
            lang,
            search: Typing::most(SEARCH_MOST),
            results: Vec::new(),
            chosen: 0,
            first: 0,
            icons: HashMap::new(),
            pointer: None,
            hits: Vec::new(),
            caret: None,
            caret_on: true,
            shown: false,
        })
    }

    fn show_now(&mut self) {
        self.reload_look();
        self.scale = scale_at_cursor();
        self.search = Typing::most(SEARCH_MOST);
        self.chosen = 0;
        self.first = 0;
        self.pointer = None;
        self.caret_on = true;
        self.refresh();
        self.place();
        self.shown = true;
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(self.hwnd);
            let _ = SetFocus(Some(self.hwnd));
            let _ = InvalidateRect(Some(self.hwnd), None, false);
            let _ = SetTimer(Some(self.hwnd), BLINK, GetCaretBlinkTime(), None);
            let _ = SetTimer(Some(self.hwnd), AGAIN, AGAIN_MS, None);
        }
        let told = self.hwnd.0 as isize;
        found::look(move || {
            let _ = unsafe { PostMessageW(Some(HWND(told as *mut _)), WM_TIMER, WPARAM(AGAIN), LPARAM(0)) };
        });
    }

    fn hide_now(&mut self) {
        if !self.shown {
            return;
        }
        self.shown = false;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), BLINK);
            let _ = KillTimer(Some(self.hwnd), AGAIN);
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    /// The skin and language as they are now.
    fn reload_look(&mut self) {
        let (theme, lang) = look_now();
        self.theme = theme;
        self.lang = lang;
        self.inset = self.theme.margin;
        self.surface = None;
        self.icons.clear();
    }

    /// Puts it in the middle of the screen the pointer is on, a little above
    /// its middle as PowerToys Run is.
    fn place(&self) {
        let room = (self.inset * self.scale).round() as i32;
        let (wide, tall) = ((WIDTH * self.scale).round() as i32, (HEIGHT * self.scale).round() as i32);
        let (x, y) = place_at_cursor(wide + 2 * room, tall + 2 * room);
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, x - room, y - room, wide + 2 * room, tall + 2 * room, SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    fn redraw(&self) {
        let _ = unsafe { InvalidateRect(Some(self.hwnd), None, false) };
    }

    fn refresh(&mut self) {
        self.results = results(&self.search.string(), self.lang);
        self.chosen = self.chosen.min(self.results.len().saturating_sub(1));
        self.keep_shown();
    }

    fn keep_shown(&mut self) {
        if self.chosen < self.first {
            self.first = self.chosen;
        } else if self.chosen >= self.first + ROWS {
            self.first = self.chosen + 1 - ROWS;
        }
    }

    fn on_page(&self, x: f32, y: f32) -> (f32, f32) {
        (x / self.scale - self.inset, y / self.scale - self.inset)
    }

    /// The picture of what `item` is, if one is had.
    fn icon_for(&mut self, item: &Item, side: u32) -> Option<Picture> {
        if !matches!(item.kind, Kind::App | Kind::File) {
            return None;
        }
        let key = (item.target.clone(), side);
        if let Some(picture) = self.icons.get(&key) {
            return Some(picture.clone());
        }
        let picture = crate::station::thumbs::icon(std::path::Path::new(&item.target), side)?;
        if self.icons.len() >= 512 {
            self.icons.clear();
        }
        self.icons.insert(key, picture.clone());
        Some(picture)
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
        if self.surface.is_none() {
            if let Ok(gfx) = gfx::current() {
                self.gfx = gfx;
            }
            self.surface = Surface::new(&self.gfx, self.hwnd).ok();
        }
        let Some(mut surface) = self.surface.take() else { return };
        let gfx = self.gfx.clone();
        let inset = self.inset;
        let area = (size.0 as f32 / self.scale - 2.0 * inset, size.1 as f32 / self.scale - 2.0 * inset);
        let drawn = surface.draw(&gfx, size, self.scale, |frame| {
            let mut was = windows_numerics::Matrix3x2::default();
            unsafe {
                frame.dc.GetTransform(&mut was);
                frame.dc.SetTransform(&(windows_numerics::Matrix3x2::translation(inset, inset) * was));
            }
            self.paint(frame, area);
            unsafe { frame.dc.SetTransform(&was) };
        });
        if drawn.is_ok() {
            self.surface = Some(surface);
        }
    }

    fn paint(&mut self, frame: &Frame, (width, height): (f32, f32)) {
        let theme = self.theme.clone();
        let lang = self.lang;
        paint_ground(frame, &theme, (width, height));
        let mark = match theme.skin {
            Skin::Paper => theme.text,
            _ => theme::accent(theme.dark),
        };
        let round = |tall: f32| if theme.skin == Skin::Glass { tall / 2.0 } else { theme.control_radius.max(4.0).min(tall / 2.0) };
        // The search box, as quick launch's is: a faint well in chart paper,
        // Windows 11's field with the accent along its foot, a shallow well
        // of glass.
        let field = (PAD, 13.0, width - 2.0 * PAD, 38.0);
        match theme.skin {
            Skin::Glass => {
                frame.fill_rounded(if theme.dark { Color::hex(0xFFFFFF, 0.08) } else { Color::hex(0, 0.05) }, field.0, field.1, field.2, field.3, round(field.3));
                frame.stroke_rounded(Fill::Down { top: field.1, from: Color::hex(0xFFFFFF, 0.0), bottom: field.1 + field.3, to: Color::hex(0xFFFFFF, if theme.dark { 0.18 } else { 0.6 }) }, field.0, field.1, field.2, field.3, round(field.3), 1.0);
            }
            Skin::Fluent => {
                frame.fill_rounded(theme.track, field.0, field.1, field.2, field.3, round(field.3));
                frame.clip(field.0, field.1 + field.3 - 2.0, field.2, 2.0);
                frame.fill_rounded(mark, field.0, field.1, field.2, field.3, round(field.3));
                frame.unclip();
            }
            Skin::Paper => frame.fill_rounded(theme.track.alpha(theme.track.a * 0.6), field.0, field.1, field.2, field.3, round(field.3)),
        }
        frame.icon(Icon::Search, Point { x: field.0 + 18.0, y: field.1 + field.3 / 2.0 }, 15.0, theme.text3);
        let text_x = field.0 + 38.0;
        let text_y = field.1 + (field.3 - theme.body.size * 1.35) / 2.0;
        let words = self.search.string();
        if words.is_empty() {
            frame.text(lang.pick("搜索应用、文件和设置", "Search apps, files and settings"), theme.body, theme.text3, text_x, text_y, field.2 - 52.0, Align::Start);
        } else {
            frame.text(&words, theme.body, theme.text, text_x, text_y, field.2 - 52.0, Align::Start);
        }
        let before = String::from_utf16_lossy(&self.search.text[..self.search.caret]);
        let caret_x = text_x + frame.measure(&before, theme.body).min(field.2 - 56.0);
        let caret_h = theme.body.size * 1.35;
        self.caret = Some((caret_x, text_y, caret_h));
        if self.caret_on {
            frame.fill(theme.text, caret_x, text_y + 1.0, 1.2, caret_h - 2.0);
        }
        // A rule between the box and what it lists.
        frame.fill(theme.rule, PAD, SEARCH_ROW, width - 2.0 * PAD, 1.0);
        // The results, the chosen one marked as the skin marks what is chosen.
        let top = SEARCH_ROW + 1.0;
        let bottom = height - FOOTER;
        let row_x = PAD - 6.0;
        let row_w = width - 2.0 * (PAD - 6.0);
        let side = (24.0 * self.scale).round().max(16.0) as u32;
        let mut hits = Vec::new();
        frame.clip(0.0, top, width, bottom - top);
        if self.results.is_empty() {
            let none = if words.trim().is_empty() { lang.pick("输入以搜索应用、文件和设置", "Type to search apps, files and settings") } else { lang.pick("没有找到匹配的项目", "Nothing matches that") };
            frame.text(none, theme.body, theme.text3, PAD, top + 18.0, width - 2.0 * PAD, Align::Start);
        }
        for at in 0..ROWS {
            let index = self.first + at;
            let Some(item) = self.results.get(index).cloned() else { break };
            let y = top + at as f32 * ROW;
            let hovered = self.pointer.is_some_and(|(x, py)| x >= row_x && x < row_x + row_w && py >= y && py < y + ROW);
            if index == self.chosen {
                frame.fill_rounded(theme.hover, row_x, y + 2.0, row_w, ROW - 4.0, round(10.0));
                frame.fill_rounded(mark, row_x + 2.0, y + 10.0, 3.0, ROW - 20.0, 1.5);
            } else if hovered {
                frame.fill_rounded(theme.hover.alpha(0.6), row_x, y + 2.0, row_w, ROW - 4.0, round(10.0));
            }
            let tile = (PAD + 4.0, y + 10.0, 28.0, 28.0);
            if let Some(picture) = self.icon_for(&item, side) {
                frame.picture(&picture, tile.0 + 2.0, tile.1 + 2.0, 24.0, 24.0, 6.0, 1.0);
            } else {
                frame.fill_rounded(theme.hover, tile.0, tile.1, tile.2, tile.3, 7.0);
                frame.icon(item.fallback_icon(), Point { x: tile.0 + 14.0, y: tile.1 + 14.0 }, 15.0, theme.text2);
            }
            let tag = item.tag(lang);
            let tag_w = frame.measure(tag, theme.small);
            let text_x = PAD + 44.0;
            let text_room = (width - PAD - tag_w - 16.0 - text_x).max(40.0);
            frame.text(&item.name, theme.body, theme.text, text_x, y + 7.0, text_room, Align::Start);
            if !item.subtitle.is_empty() {
                frame.text(&item.subtitle, theme.small, theme.text3, text_x, y + 25.0, text_room, Align::Start);
            }
            frame.text(tag, theme.small, theme.text3, width - PAD - tag_w, y + 16.0, tag_w + 2.0, Align::End);
            hits.push((row_x, y, row_w, ROW, index));
        }
        frame.unclip();
        self.hits = hits;
        // Its foot: how many were found, and what the keys do.
        let foot = height - FOOTER + 9.0;
        let found = match lang {
            Lang::Zh => format!("{} 个结果", self.results.len()),
            Lang::En => format!("{} results", self.results.len()),
        };
        frame.text(&found, theme.small, theme.text3, PAD, foot, width / 2.0, Align::Start);
        frame.text(lang.pick("↑↓ 选择    Enter 打开    Esc 关闭", "↑↓ Select    Enter Open    Esc Close"), theme.small, theme.text3, width / 2.0, foot, width / 2.0 - PAD, Align::End);
    }

    /// The result `index` opened, with Shift held to show a file where it is.
    fn open(&mut self, index: usize, reveal: bool) {
        let Some(item) = self.results.get(index).cloned() else { return };
        self.hide_now();
        match item.kind {
            Kind::App | Kind::Web => run::ask(Ask::Launch(item.target)),
            Kind::File => run::ask(Ask::Files(vec![std::path::PathBuf::from(&item.target)], if reveal { FilesAct::Show } else { FilesAct::Open })),
            Kind::Settings => crate::show_settings(None),
            Kind::Calculator => run::ask(Ask::CopyText(item.target, None)),
        }
    }

    fn key(&mut self, key: VIRTUAL_KEY) {
        let shift = unsafe { GetKeyState(i32::from(VK_SHIFT.0)) } < 0;
        let ctrl = unsafe { GetKeyState(i32::from(VK_CONTROL.0)) } < 0;
        match key {
            VK_ESCAPE => self.hide_now(),
            VK_UP => {
                if !self.results.is_empty() {
                    self.chosen = self.chosen.saturating_sub(1);
                    self.keep_shown();
                }
            }
            VK_DOWN => {
                if !self.results.is_empty() {
                    self.chosen = (self.chosen + 1).min(self.results.len() - 1);
                    self.keep_shown();
                }
            }
            VK_RETURN => self.open(self.chosen, shift),
            VK_LEFT => self.search.go(self.search.step(self.search.caret, false), shift),
            VK_RIGHT => self.search.go(self.search.step(self.search.caret, true), shift),
            VK_HOME => self.search.go(0, shift),
            VK_END => self.search.go(self.search.text.len(), shift),
            VK_BACK => {
                self.search.erase(false);
                self.typed();
            }
            VK_DELETE => {
                self.search.erase(true);
                self.typed();
            }
            _ if ctrl && key.0 == u16::from(b'A') => {
                self.search.go(0, false);
                self.search.go(self.search.text.len(), true);
            }
            _ => return,
        }
        self.caret_on = true;
        self.redraw();
    }

    fn typed(&mut self) {
        self.chosen = 0;
        self.first = 0;
        self.refresh();
        self.caret_on = true;
        self.redraw();
    }

    /// The input method's words and choices put at the caret.
    fn place_ime(&self) {
        let Some((x, y, h)) = self.caret else { return };
        let px = |dips: f32| (dips * self.scale).round() as i32;
        unsafe {
            let context = ImmGetContext(self.hwnd);
            if context.is_invalid() {
                return;
            }
            let (x, y) = (x + self.inset, y + self.inset);
            let _ = ImmSetCompositionWindow(context, &COMPOSITIONFORM { dwStyle: CFS_POINT, ptCurrentPos: POINT { x: px(x), y: px(y) }, ..Default::default() });
            let around = RECT { left: px(x), top: px(y), right: px(x) + 1, bottom: px(y + h) };
            let _ = ImmSetCandidateWindow(context, &CANDIDATEFORM { dwIndex: 0, dwStyle: CFS_EXCLUDE, ptCurrentPos: POINT { x: around.left, y: around.bottom }, rcArea: around });
            let mut face = windows::Win32::Graphics::Gdi::LOGFONTW { lfHeight: -px(self.theme.body.size), lfWeight: 400, lfCharSet: windows::Win32::Graphics::Gdi::DEFAULT_CHARSET, ..Default::default() };
            for (unit, to) in "Microsoft YaHei UI".encode_utf16().zip(face.lfFaceName.iter_mut()) {
                *to = unit;
            }
            let _ = ImmSetCompositionFontW(context, &face);
            let _ = ImmReleaseContext(self.hwnd, context);
        }
    }
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let point = || ((lparam.0 & 0xFFFF) as i16 as f32, ((lparam.0 >> 16) & 0xFFFF) as i16 as f32);
    match message {
        WM_PAINT => {
            with(Search::draw);
            let _ = unsafe { ValidateRect(Some(hwnd), None) };
        }
        WM_MOUSEMOVE => {
            let (x, y) = point();
            with(|search| {
                let (x, y) = search.on_page(x, y);
                if search.pointer != Some((x, y)) {
                    search.pointer = Some((x, y));
                    search.redraw();
                }
            });
        }
        WM_LBUTTONUP => {
            let (x, y) = point();
            let chosen = with(|search| {
                let (x, y) = search.on_page(x, y);
                search.hits.iter().find(|(hx, hy, w, h, _)| x >= *hx && x < hx + w && y >= *hy && y < hy + h).map(|(.., index)| *index)
            })
            .flatten();
            if let Some(index) = chosen {
                with(|search| search.open(index, false));
            }
        }
        WM_MOUSEWHEEL => {
            let turns = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / 120.0;
            with(|search| {
                if search.results.is_empty() {
                    return;
                }
                let chosen = (search.chosen as i32 - turns.round() as i32).clamp(0, search.results.len() as i32 - 1) as usize;
                search.chosen = chosen;
                search.keep_shown();
                search.redraw();
            });
        }
        WM_KEYDOWN => {
            with(|search| search.key(VIRTUAL_KEY(wparam.0 as u16)));
        }
        WM_CHAR => {
            with(|search| {
                let unit = wparam.0 as u16;
                if unit >= 0x20 && unit != 0x7F {
                    search.search.put(&[unit]);
                    search.typed();
                }
            });
        }
        WM_TIMER => {
            with(|search| match wparam.0 {
                BLINK => {
                    search.caret_on = !search.caret_on;
                    search.redraw();
                }
                AGAIN => {
                    search.refresh();
                    search.redraw();
                }
                _ => {}
            });
        }
        WM_IME_STARTCOMPOSITION => {
            with(|search| search.place_ime());
            return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
        }
        // Another window come in front: it goes, as PowerToys Run does.
        WM_ACTIVATE if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE => {
            with(Search::hide_now);
        }
        // Another screen's scale: drawn again at it, where Windows says.
        WM_DPICHANGED => {
            with(|search| {
                search.scale = ((wparam.0 & 0xFFFF) as u32).max(96) as f32 / 96.0;
                search.reload_look();
                search.surface = None;
                let place = unsafe { &*(lparam.0 as *const RECT) };
                let _ = unsafe { SetWindowPos(search.hwnd, None, place.left, place.top, place.right - place.left, place.bottom - place.top, SWP_NOZORDER | SWP_NOACTIVATE) };
                search.redraw();
            });
        }
        WM_CLOSE => {
            with(Search::hide_now);
        }
        WM_DESTROY => {
            if WINDOW.load(Ordering::Acquire) == hwnd.0 as isize {
                WINDOW.store(0, Ordering::Release);
            }
            unsafe { windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0) };
        }
        _ => return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
    LRESULT(0)
}

/// The skin and language as the settings have them.
fn look_now() -> (Rc<Theme>, Lang) {
    let settings = crate::app().settings.lock().unwrap();
    let prefs = super::prefs::Prefs::resolve(&settings.view, &[]);
    let skin = Skin::named(&settings.skin);
    let dark = theme::is_dark(prefs.theme, None);
    (Rc::new(Theme::new(skin, dark)), Lang::resolve(prefs.language))
}

/// Its sheet, drawn as a panel's is, over nothing but itself (the launcher
/// is its own place; it does not take the desktop behind it).
fn paint_ground(frame: &Frame, theme: &Theme, (width, height): (f32, f32)) {
    let piece = [Rect { x: 0.0, y: 0.0, w: width, h: height }];
    let ground = super::skins::Ground {
        theme,
        edge: None,
        size: (width, height),
        lanes: &piece,
        bar: Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
        backdrop: None,
        frost: 0.0,
    };
    for part in super::skins::parts(theme.skin) {
        let _ = super::skins::draw(frame, &ground, *part);
    }
}

/// How many pixels a DIP is on the screen the pointer is on.
fn scale_at_cursor() -> f32 {
    let mut pointer = POINT::default();
    let _ = unsafe { GetCursorPos(&mut pointer) };
    let monitor = unsafe { MonitorFromPoint(pointer, MONITOR_DEFAULTTONEAREST) };
    let (mut dpi, mut unused) = (96u32, 96u32);
    let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut unused) };
    dpi.max(96) as f32 / 96.0
}

/// Where a box `wide` × `tall` (physical px) goes on the screen the pointer
/// is on: across its middle, a little above its middle.
fn place_at_cursor(wide: i32, tall: i32) -> (i32, i32) {
    let mut pointer = POINT::default();
    let _ = unsafe { GetCursorPos(&mut pointer) };
    let monitor = unsafe { MonitorFromPoint(pointer, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = unsafe { GetMonitorInfoW(monitor, &mut info) };
    let room = info.rcWork;
    let x = ((room.left + room.right - wide) / 2).clamp(room.left, (room.right - wide).max(room.left));
    let above = ((room.bottom - room.top - tall) as f32 * 0.24) as i32;
    let y = (room.top + above).clamp(room.top, (room.bottom - tall).max(room.top));
    (x, y)
}

fn put(scored: &mut Vec<(i32, usize, Item)>, order: &mut usize, score: i32, item: Item) {
    scored.push((score, *order, item));
    *order += 1;
}

/// What is listed for `query`, best first.
fn results(query: &str, lang: Lang) -> Vec<Item> {
    let query = query.trim();
    let lower = query.to_lowercase();
    let mut scored: Vec<(i32, usize, Item)> = Vec::new();
    let mut order = 0usize;
    if query.is_empty() {
        // Nothing typed: the files come lately, or else the Start menu's
        // first programs.
        let mut seen = HashSet::new();
        for came in recent::shown().iter() {
            let Some(path) = Some(came.path.clone()).filter(|path| seen.insert(path.clone())) else { continue };
            let Some(name) = path.file_name().map(|name| name.to_string_lossy().into_owned()) else { continue };
            let place = path.parent().map(|parent| parent.display().to_string()).unwrap_or_default();
            put(&mut scored, &mut order, 0, Item { kind: Kind::File, name, subtitle: place, target: path.display().to_string() });
            if scored.len() >= 8 {
                break;
            }
        }
        if scored.is_empty() {
            if let Some(all) = found::all() {
                for found in all.iter().take(8) {
                    put(&mut scored, &mut order, 0, Item { kind: Kind::App, name: found.name.clone(), subtitle: String::new(), target: found.target.clone() });
                }
            }
        }
        return scored.into_iter().map(|(_, _, item)| item).collect();
    }
    // A sum worked out.
    if let Some(answer) = calculate(query) {
        put(&mut scored, &mut order, -2, Item { kind: Kind::Calculator, name: answer.clone(), subtitle: format!("= {query}"), target: answer });
    }
    // A web address as it is.
    if let Some(address) = as_url(query) {
        put(&mut scored, &mut order, -1, Item { kind: Kind::Web, name: address.clone(), subtitle: lang.pick("在浏览器中打开", "Open in the browser").to_string(), target: address });
    }
    // What is kept at hand first, then the Start menu's programs.
    for app in apps::shown().iter() {
        let initials = found::initials(&app.name);
        if let Some(score) = score(&app.name, &initials, "", &lower) {
            put(&mut scored, &mut order, score - 1, Item { kind: Kind::App, name: app.name.clone(), subtitle: lang.pick("快捷 App", "Quick launch").to_string(), target: app.target.clone() });
        }
    }
    if let Some(all) = found::all() {
        for found in all.iter() {
            if let Some(score) = score(&found.name, &found.initials, &found.target, &lower) {
                put(&mut scored, &mut order, score, Item { kind: Kind::App, name: found.name.clone(), subtitle: String::new(), target: found.target.clone() });
            }
        }
    }
    // The files come lately.
    let mut seen = HashSet::new();
    let mut came = recent::shown().iter().cloned().collect::<Vec<_>>();
    came.sort_by_key(|came| std::cmp::Reverse(came.came_ms));
    for came in came {
        if !seen.insert(came.path.clone()) {
            continue;
        }
        let Some(name) = came.path.file_name().map(|name| name.to_string_lossy().into_owned()) else { continue };
        let place = came.path.parent().map(|parent| parent.display().to_string()).unwrap_or_default();
        let initials = found::initials(&name);
        if let Some(score) = score(&name, &initials, &place, &lower) {
            put(&mut scored, &mut order, score + 4, Item { kind: Kind::File, name, subtitle: place, target: came.path.display().to_string() });
        }
    }
    // Glance's own settings.
    let settings_words = vec![lang.pick("设置", "settings").to_lowercase(), lang.pick("Glance", "Glance").to_lowercase()];
    if settings_words.iter().any(|words| words.contains(lower.as_str()) || lower.contains(words.as_str())) {
        put(&mut scored, &mut order, 8, Item { kind: Kind::Settings, name: lang.pick("Glance 设置", "Glance settings").to_string(), subtitle: lang.pick("打开 Glance 的设置", "Open Glance's settings").to_string(), target: String::new() });
    }
    // What the web may know, last.
    put(&mut scored, &mut order, 1000, Item { kind: Kind::Web, name: format!("{} “{query}”", lang.pick("在浏览器中搜索", "Search the web for")), subtitle: "Bing".to_string(), target: format!("https://www.bing.com/search?q={}", quoted(query)) });
    scored.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    let mut seen = HashSet::new();
    scored
        .into_iter()
        .filter_map(|(_, _, item)| seen.insert(item.target.clone()).then_some(item))
        .take(50)
        .collect()
}

/// How well `name` answers `query` (lower case): its whole name, then its
/// beginning, then a word in it, then its initials, then where it is; none
/// if it does not.
fn score(name: &str, initials: &[u64], extra: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let lower = name.to_lowercase();
    if lower == query {
        return Some(0);
    }
    if lower.starts_with(query) {
        return Some(1);
    }
    if let Some(at) = lower.find(query) {
        return Some(3 + (at.min(20) as i32));
    }
    if initials_match(initials, query) {
        return Some(2);
    }
    if !extra.is_empty() && extra.to_lowercase().contains(query) {
        return Some(30);
    }
    None
}

/// Whether the words of `initials` may begin with the letters of `query`,
/// each an initial's (as `found::matches` has them).
fn initials_match(initials: &[u64], query: &str) -> bool {
    let letters: Vec<u64> = query.chars().filter(|c| !c.is_whitespace()).map(bit).collect();
    !letters.is_empty() && letters.iter().all(|letter| *letter != 0) && letters.len() <= initials.len() && letters.iter().zip(initials).all(|(letter, may)| letter & may != 0)
}

fn bit(c: char) -> u64 {
    match c.to_ascii_lowercase() {
        c @ 'a'..='z' => 1 << (c as u32 - 'a' as u32),
        c @ '0'..='9' => 1 << (26 + c as u32 - '0' as u32),
        _ => 0,
    }
}

/// `query` as an address to open, where it is one: an address as it is, or
/// a name with a dot in it and no room, taken as https.
fn as_url(query: &str) -> Option<String> {
    let query = query.trim();
    if query.is_empty() || query.contains(char::is_whitespace) {
        return None;
    }
    if query.starts_with("https://") || query.starts_with("http://") {
        return Some(query.to_string());
    }
    let looks = query.contains('.') && query.chars().any(|c| c.is_ascii_alphabetic()) && !query.starts_with('.') && !query.ends_with('.') && !query.contains('\\') && !query.contains('/');
    looks.then(|| format!("https://{query}"))
}

/// `query` worked out where it is a sum: numbers with +, -, *, /, %, powers
/// and brackets. None for anything else, and for what is not a number.
fn calculate(query: &str) -> Option<String> {
    if !query.chars().any(|c| c.is_ascii_digit()) || !query.chars().any(|c| matches!(c, '+' | '-' | '*' | '/' | '%' | '^' | '(')) {
        return None;
    }
    if query.chars().any(|c| c.is_ascii_alphabetic() || c == '_' || c == '=') {
        return None;
    }
    let chars: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).collect();
    let mut at = 0usize;
    let value = expression(&chars, &mut at)?;
    if at != chars.len() || !value.is_finite() {
        return None;
    }
    Some(format_value(value))
}

fn expression(chars: &[char], at: &mut usize) -> Option<f64> {
    let mut value = term(chars, at)?;
    loop {
        match chars.get(*at) {
            Some('+') => {
                *at += 1;
                value += term(chars, at)?;
            }
            Some('-') => {
                *at += 1;
                value -= term(chars, at)?;
            }
            _ => return Some(value),
        }
    }
}

fn term(chars: &[char], at: &mut usize) -> Option<f64> {
    let mut value = power(chars, at)?;
    loop {
        match chars.get(*at) {
            Some('*') => {
                *at += 1;
                value *= power(chars, at)?;
            }
            Some('/') => {
                *at += 1;
                let divisor = power(chars, at)?;
                if divisor == 0.0 {
                    return None;
                }
                value /= divisor;
            }
            Some('%') => {
                *at += 1;
                let divisor = power(chars, at)?;
                if divisor == 0.0 {
                    return None;
                }
                value %= divisor;
            }
            _ => return Some(value),
        }
    }
}

fn power(chars: &[char], at: &mut usize) -> Option<f64> {
    let value = unary(chars, at)?;
    if chars.get(*at) == Some(&'^') {
        *at += 1;
        return Some(value.powf(power(chars, at)?));
    }
    Some(value)
}

fn unary(chars: &[char], at: &mut usize) -> Option<f64> {
    match chars.get(*at) {
        Some('-') => {
            *at += 1;
            Some(-unary(chars, at)?)
        }
        Some('+') => {
            *at += 1;
            unary(chars, at)
        }
        _ => atom(chars, at),
    }
}

fn atom(chars: &[char], at: &mut usize) -> Option<f64> {
    match chars.get(*at) {
        Some('(') => {
            *at += 1;
            let value = expression(chars, at)?;
            (chars.get(*at) == Some(&')')).then_some(())?;
            *at += 1;
            Some(value)
        }
        Some(c) if c.is_ascii_digit() || *c == '.' => {
            let start = *at;
            while chars.get(*at).is_some_and(|c| c.is_ascii_digit() || *c == '.') {
                *at += 1;
            }
            chars[start..*at].iter().collect::<String>().parse().ok()
        }
        _ => None,
    }
}

/// A sum's answer as it is written: no trailing zeros, and no -0.
fn format_value(value: f64) -> String {
    let mut text = format!("{value:.10}");
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    if text == "-0" {
        text = "0".into();
    }
    text
}

/// `text` as a web search takes it in an address.
fn quoted(text: &str) -> String {
    let mut quoted = String::new();
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => quoted.push(char::from(*byte)),
            b' ' => quoted.push('+'),
            _ => quoted.push_str(&format!("%{byte:02X}")),
        }
    }
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_are_worked_out() {
        assert_eq!(calculate("1+2*3").as_deref(), Some("7"));
        assert_eq!(calculate("(1+2)*3").as_deref(), Some("9"));
        assert_eq!(calculate("2^3^2").as_deref(), Some("512"));
        assert_eq!(calculate("-4/2").as_deref(), Some("-2"));
        assert_eq!(calculate("5 % 2").as_deref(), Some("1"));
        assert_eq!(calculate("1/0"), None);
        assert_eq!(calculate("v1.2"), None);
        assert_eq!(calculate("42"), None);
    }

    #[test]
    fn addresses_are_told_apart() {
        assert_eq!(as_url("example.com").as_deref(), Some("https://example.com"));
        assert_eq!(as_url("https://a.b/c"), Some("https://a.b/c".to_string()));
        assert_eq!(as_url("two words.com"), None);
        assert_eq!(as_url("readme"), None);
        assert_eq!(as_url("1.2"), None);
    }

    #[test]
    fn what_begins_a_name_is_found_first() {
        let initials = found::initials("Visual Studio Code");
        let starts = score("Visual Studio Code", &initials, "", "vis").unwrap();
        let contains = score("Visual Studio Code", &initials, "", "studio").unwrap();
        let pinyin = score("微信", &found::initials("微信"), "", "wx").unwrap();
        assert!(starts < contains);
        assert!(pinyin < 3);
        assert_eq!(score("Visual Studio Code", &initials, "", "code"), Some(3 + 14));
        assert_eq!(score("Visual Studio Code", &initials, "", "zzz"), None);
    }

    #[test]
    fn the_web_search_is_quoted() {
        assert_eq!(quoted("主板 温度"), "%E4%B8%BB%E6%9D%BF+%E6%B8%A9%E5%BA%A6");
    }
}
