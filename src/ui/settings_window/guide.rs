//! The guide to Glance: shown once as Glance first starts with it (installed, or updated to a version with it), and
//! again from the general page. Three steps, each with what it teaches played over and over on Glance's own things
//! till the user leaves it: a panel opened from the screen's edge (in a dialog over the desktop, the user's own desktop
//! small at its top, the real panel sliding out on it); a component dragged onto an edge (on the layout page itself,
//! its own editor played on, a teaching tip beside the window); search and the preview called up by their shortcuts (in
//! the dialog again). Each step says so once the user has done it themselves. Then a word that it is done.
//!
//! It is the settings window's, drawn on its thread with its desktop and its panels (see `Ui::panel_on_stage`): the
//! window made for it is hidden but for the step on its page.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::D2D1_LAYER_PARAMETERS1;
use windows::Win32::Graphics::Dwm::{DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
use windows::Win32::Graphics::Gdi::{ClientToScreen, GetMonitorInfoW, MonitorFromWindow, ValidateRect, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VIRTUAL_KEY, VK_ESCAPE, VK_RETURN};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowRect, IsWindowVisible, LoadCursorW, LoadImageW, PostMessageW, RegisterClassExW, SetForegroundWindow, SetWindowPos,
    ShowWindow, HICON, HTCAPTION, HTCLIENT, IDC_ARROW, IMAGE_ICON, LR_SHARED, MA_NOACTIVATE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOW,
    SW_SHOWNOACTIVATE, WM_CLOSE, WM_DPICHANGED, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_NCCALCSIZE, WM_NCHITTEST, WM_PAINT,
    WNDCLASSEXW, WS_CAPTION, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_POPUP, WS_SYSMENU,
};

use super::*;
use crate::settings::Places;
use crate::ui::motion::{Easing, LINEAR};

/// The dialog: its width, the room round what is in it, the small desktop's height, its footer, a button's height and
/// the least width of one, and a shortcut's row.
const WIDE: f32 = 640.0;
const PAD: f32 = 24.0;
const DESK: f32 = 300.0;
const FOOTER: f32 = 80.0;
const BUTTON: f32 = 32.0;
const BUTTON_LEAST: f32 = 96.0;
const KEY_ROW: f32 = 52.0;
/// The teaching tip: its width, the room inside it, its beak (how far it stands out), the room round it for its shadow,
/// and how far from the window it stands.
const TIP_WIDE: f32 = 340.0;
const TIP_PAD: f32 = 16.0;
const BEAK: f32 = 10.0;
const SHADOW: f32 = 24.0;
const TIP_OFF: f32 = 6.0;
/// How soon it is drawn again while anything in it plays.
const FRAME: Duration = Duration::from_millis(15);

const TITLE: Font = Font::new(Family::Segoe, 20.0, 600.0);
const TIP_TITLE: Font = Font::new(Family::Segoe, 15.0, 600.0);
const BODY: Font = Font::new(Family::Segoe, 14.0, 400.0);
const SMALL: Font = Font::new(Family::Segoe, 12.0, 400.0);
const STATE: Font = Font::new(Family::Segoe, 13.0, 400.0);

/// How the pointer glides in the plays, and how the panel slides out and the look grows, as Glance's own do.
const GLIDE: Easing = Easing(0.45, 0.05, 0.3, 1.0);
const SLIDE_OUT: Easing = Easing(0.2, 0.9, 0.25, 1.04);
const GROW: Easing = Easing(0.2, 0.9, 0.25, 1.05);

/// The first step's play (ms): the pointer at rest, gliding to the edge, the panel sliding out as it is pushed, the
/// pointer back where it began, the panel put away as it leaves it.
const PUSH_LOOP: u64 = 5300;

/// What a step asks, done elsewhere: a panel opened by a push into an edge, search called up, something looked at
/// larger.
#[derive(Clone, Copy)]
pub(crate) enum Saw {
    Pushed,
    Searched,
    Looked,
}

impl Saw {
    fn bit(self) -> usize {
        match self {
            Saw::Pushed => 1,
            Saw::Searched => 2,
            Saw::Looked => 4,
        }
    }
}

/// Whether a guide is up (told of what is done elsewhere only then), and what it was told of since it last looked.
static UP: AtomicBool = AtomicBool::new(false);
static TOLD: AtomicUsize = AtomicUsize::new(0);

/// Tells the guide, where one is up, that `saw` was done: from any thread.
pub(crate) fn saw(saw: Saw) {
    if !UP.load(Ordering::Acquire) {
        return;
    }
    TOLD.fetch_or(saw.bit(), Ordering::AcqRel);
    if let Some(hwnd) = super::hwnd() {
        let _ = unsafe { PostMessageW(Some(hwnd), GUIDE_TOLD, WPARAM(0), LPARAM(0)) };
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    Push,
    Layout,
    Keys,
    /// The word that it is done.
    Ready,
}

impl Step {
    fn after(self) -> Option<Step> {
        match self {
            Step::Push => Some(Step::Layout),
            Step::Layout => Some(Step::Keys),
            Step::Keys => Some(Step::Ready),
            Step::Ready => None,
        }
    }

    fn before(self) -> Option<Step> {
        match self {
            Step::Push => None,
            Step::Layout => Some(Step::Push),
            Step::Keys => Some(Step::Layout),
            Step::Ready => Some(Step::Keys),
        }
    }

    /// Its place among the three (the word that it is done has none).
    fn index(self) -> Option<usize> {
        match self {
            Step::Push => Some(0),
            Step::Layout => Some(1),
            Step::Keys => Some(2),
            Step::Ready => None,
        }
    }
}

/// What may be pressed in its windows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    Skip,
    Back,
    Next,
    /// A shortcut's row on the last step: search's, the preview's.
    Search,
    Look,
}

/// One of its windows: the dialog, or the teaching tip.
struct Popup {
    hwnd: HWND,
    surface: Option<Surface>,
    scale: f32,
    /// Its size (DIPs): the tip's with the room for its shadow round it.
    size: (f32, f32),
    hits: Vec<(Rect, Hit)>,
    pointer: Option<(f32, f32)>,
    pressed: Option<Hit>,
    /// The tip: its plate in it (DIPs), and the side its beak is on and how far down that side it is; none where it
    /// stands over the window, pointing at nothing.
    plate: Rect,
    beak: Option<(Edge, f32)>,
}

impl Popup {
    fn hovered(&self) -> Option<Hit> {
        let (x, y) = self.pointer?;
        self.hits.iter().find(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h).map(|(_, hit)| *hit)
    }
}

impl Drop for Popup {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

pub(super) struct Guide {
    step: Step,
    /// Each step done by the user themselves.
    done: [bool; 3],
    /// Search and the preview called up, on the last step.
    tried: [bool; 2],
    /// Whether the window was up as it began (from the general page): shown again there as it ends, else closed.
    was_up: bool,
    /// When the step shown began to play.
    since: Instant,
    /// The edge the first step shows the panel opened from, and what that edge opens.
    edge: (Edge, Option<Part>),
    dialog: Option<Popup>,
    tip: Option<Popup>,
    /// The play on the layout page, while it plays.
    drag: Option<Drag>,
    /// The skin's ground under search and under the look, in the small desktop.
    grounds: [render::GroundLayers; 2],
    _showing: crate::ui::life::Showing,
}

impl Drop for Guide {
    /// (Gone however it goes, the window closed under it among them: told of nothing more.)
    fn drop(&mut self) {
        UP.store(false, Ordering::Release);
    }
}

/// The play on the layout page: a component carried from those beside the editor onto an edge of its screen, then
/// what the edge held carried back onto it (where it held none, what was put there carried off it), over and over.
/// The edges it changes are the window's alone, never saved, and put back as they were as it stops.
pub(super) struct Drag {
    before: Places,
    screen: String,
    moves: Vec<Move>,
    /// The move it is on, since when, and where the pointer was as it began; the pointer now (none till it shows).
    at: usize,
    since: Instant,
    from: Option<(f32, f32)>,
    pointer: Option<(f32, f32)>,
}

/// Where the pointer goes in the play on the layout page: where it first shows (the foot of the editor), a component
/// beside the editor, the edge played on, the component on it, the middle of its screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Spot {
    Rest,
    Tile(Part),
    Edge,
    Pill,
    Middle,
}

/// What a move does once the pointer is there: takes up a component (from the edge played on, or from beside the
/// editor), lets go what it carries, or nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Act {
    Take(Part, bool),
    Put,
    Wait,
}

/// The pointer gliding to `to` for `glide` ms, then `wait` ms there, then `act` done.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Move {
    to: Spot,
    glide: u64,
    wait: u64,
    act: Act,
}

/// The layout page's play, once round, for `put` carried onto the edge that holds `held`: it ends with the edge as it
/// began, and each round glides on from where the last left the pointer.
fn script(put: Part, held: Option<Part>) -> Vec<Move> {
    let mut moves = vec![
        Move { to: Spot::Tile(put), glide: 900, wait: 150, act: Act::Take(put, false) },
        Move { to: Spot::Edge, glide: 900, wait: 350, act: Act::Put },
        Move { to: Spot::Edge, glide: 0, wait: 1900, act: Act::Wait },
    ];
    match held {
        Some(held) => moves.extend([
            Move { to: Spot::Tile(held), glide: 900, wait: 150, act: Act::Take(held, false) },
            Move { to: Spot::Edge, glide: 900, wait: 350, act: Act::Put },
            Move { to: Spot::Edge, glide: 0, wait: 1900, act: Act::Wait },
        ]),
        None => moves.extend([
            Move { to: Spot::Pill, glide: 900, wait: 150, act: Act::Take(put, true) },
            Move { to: Spot::Middle, glide: 900, wait: 350, act: Act::Put },
            Move { to: Spot::Middle, glide: 0, wait: 1200, act: Act::Wait },
        ]),
    }
    moves
}

/// What the layout page's play carries onto an edge that holds `held`: another component than that.
fn played_part(held: Option<Part>) -> Part {
    [Part::Music, Part::Monitor, Part::Workbench].into_iter().find(|&part| Some(part) != held).unwrap_or(Part::Music)
}

/// How far along `ms` is of what begins at `from` and lasts `length` (0 before, 1 after), eased by `easing`.
fn eased(ms: f32, from: f32, length: f32, easing: Easing) -> f32 {
    easing.at(((ms - from) / length).clamp(0.0, 1.0))
}

/// The first step's play, `t` into it: how far along the pointer is from where it rests to the edge (0 to 1), and how
/// far the panel is slid out (0 to 1). At its end as at its start, so that it plays on round with nothing jumping.
fn pushing(t: Duration) -> (f32, f32) {
    let ms = (t.as_millis() as u64 % PUSH_LOOP) as f32;
    let along = eased(ms, 500.0, 1100.0, GLIDE) - eased(ms, 3800.0, 1000.0, GLIDE);
    let out = eased(ms, 1800.0, 420.0, SLIDE_OUT) - eased(ms, 4150.0, 420.0, SLIDE_OUT);
    (along, out)
}

/// The last step's play for shortcuts of `keys` keys (search's, the preview's), once round: when its keys go down one
/// after another and search shows, then the preview's and the look shows, then both go; how long it is (ms).
struct Keying {
    keys: (usize, usize),
    search_on: f32,
    look_on: f32,
    look_off: f32,
    search_off: f32,
    end: f32,
}

/// How long a key of the play stays down after the last of its shortcut's goes down, and how far apart they go down.
const KEY_HELD: f32 = 170.0;
const KEY_APART: f32 = 110.0;

impl Keying {
    fn new(keys: (usize, usize)) -> Self {
        let search_on = 600.0 + keys.0 as f32 * KEY_APART;
        let look_on = search_on + KEY_HELD + 1300.0 + keys.1 as f32 * KEY_APART;
        let look_off = look_on + KEY_HELD + 2100.0;
        let search_off = look_off + 250.0;
        Keying { keys, search_on, look_on, look_off, search_off, end: search_off + 300.0 }
    }

    /// `t` into it, round and round: how many of each shortcut's keys are down, and how far search and the look are up
    /// (0 to 1).
    fn at(&self, t: Duration) -> ([usize; 2], f32, f32) {
        let ms = (t.as_millis() as u64 % self.end as u64) as f32;
        let down = |first: f32, count: usize, up: f32| if ms >= first && ms < up { (((ms - first) / KEY_APART).floor() as usize + 1).min(count) } else { 0 };
        let search_first = self.search_on - self.keys.0 as f32 * KEY_APART;
        let look_first = self.look_on - self.keys.1 as f32 * KEY_APART;
        let caps = [down(search_first, self.keys.0, self.search_on + KEY_HELD), down(look_first, self.keys.1, self.look_on + KEY_HELD)];
        let search = eased(ms, self.search_on, 200.0, LINEAR) - eased(ms, self.search_off, 200.0, LINEAR);
        let look = eased(ms, self.look_on, 320.0, GROW) - eased(ms, self.look_off, 180.0, LINEAR);
        (caps, search, look)
    }
}


/// The colours of its windows besides the settings window's own: a dialog's ground and its footer's, a teaching tip's,
/// a button's (its ground, under the pointer, its outline), a tick's green, and the shadow round the tip, as Windows 11
/// has them.
struct Grounds {
    dialog: Color,
    footer: Color,
    flyout: Color,
    button: Color,
    button_hover: Color,
    button_stroke: Color,
    done: Color,
    shadow: Color,
}

impl Grounds {
    fn new(dark: bool) -> Self {
        let white = |a| Color::hex(0xFFFFFF, a);
        let black = |a| Color::hex(0x000000, a);
        if dark {
            Grounds { dialog: Color::hex(0x2B2B2B, 1.0), footer: Color::hex(0x202020, 1.0), flyout: Color::hex(0x2C2C2C, 1.0), button: white(0.06), button_hover: white(0.09), button_stroke: white(0.07), done: Color::hex(0x6CCB5F, 1.0), shadow: black(0.4) }
        } else {
            Grounds { dialog: white(1.0), footer: Color::hex(0xF3F3F3, 1.0), flyout: Color::hex(0xF9F9F9, 1.0), button: white(1.0), button_hover: Color::hex(0xF6F6F6, 1.0), button_stroke: black(0.1), done: Color::hex(0x0F7B0F, 1.0), shadow: black(0.16) }
        }
    }
}

/// What the guide says of each step: its title, what it asks, what it says till that is done and once it is.
struct Words {
    title: &'static str,
    body: String,
    wait: String,
    done: &'static str,
}

fn words(step: Step, edge: Edge, lang: Lang) -> Words {
    let p = |zh: &'static str, en: &'static str| pick(lang, zh, en);
    match step {
        Step::Push => {
            let side = match edge {
                Edge::Right => p("右", "right"),
                Edge::Left => p("左", "left"),
                Edge::Top => p("上", "top"),
            };
            let (body, wait) = match lang {
                Lang::Zh => (format!("将鼠标移到屏幕{side}边缘，再向外推动一下。"), format!("现在请在屏幕{side}边缘试一次")),
                Lang::En => (format!("Move the pointer to the {side} edge of the screen, then push a little further."), format!("Now try it at the {side} edge")),
            };
            Words { title: p("从屏幕边缘呼出面板", "Open a panel from the screen's edge"), body, wait, done: p("面板已呼出，鼠标移开后自动收起", "The panel is open. It closes as the pointer leaves it") }
        }
        Step::Layout => Words {
            title: p("将组件放到屏幕边缘", "Put components on the screen's edges"),
            body: p(
                "将左侧的组件拖到右侧屏幕的任意一条边缘，推动该边缘即可呼出它。边缘上的组件也可拖到另一条边缘，两者互换位置。",
                "Drag a component on the left onto any edge of the screen on the right, then push that edge to open it. A component on an edge can be dragged onto another, and the two change places.",
            )
            .into(),
            wait: p("现在请拖动一个组件", "Now drag a component").into(),
            done: p("已调整，推动该边缘即可呼出", "Done. Push that edge to open it"),
        },
        Step::Keys => Words { title: p("两个快捷键", "Two shortcuts"), body: String::new(), wait: p("请点击按键各试一次", "Click each to try it once").into(), done: p("两个快捷键均已试过", "Both shortcuts are tried") },
        Step::Ready => Words { title: p("准备就绪", "All set"), body: p("此引导可在通用页中再次查看。", "You can see this guide again on the General page.").into(), wait: String::new(), done: "" },
    }
}

/// A shortcut's keys, each its own cap (none set: one saying so).
fn caps(shortcut: Option<crate::settings::Shortcut>, lang: Lang) -> Vec<String> {
    match shortcut {
        Some(shortcut) => crate::tray::shortcut_name(shortcut).split('+').map(str::to_string).collect(),
        None => vec![pick(lang, "未设置", "None").to_string()],
    }
}

/// Its windows' class (the dialog's and the tip's).
const CLASS: PCWSTR = w!("GlanceGuide");

fn register() {
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else { return };
        let icon = |size| LoadImageW(Some(module.into()), PCWSTR(1 as _), IMAGE_ICON, size, size, LR_SHARED).ok().map(|h| HICON(h.0));
        // (Registered once; again is refused, which is as well.)
        RegisterClassExW(&WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(procedure),
            hInstance: module.into(),
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: icon(GetSystemMetrics(SM_CXICON)).unwrap_or_default(),
            hIconSm: icon(GetSystemMetrics(SM_CXSMICON)).unwrap_or_default(),
            ..Default::default()
        });
    }
}

/// The work area of the screen `hwnd` is on (physical px).
fn work_of(hwnd: HWND) -> RECT {
    let mut screen = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let _ = GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut screen);
    }
    screen.rcWork
}

fn point(lparam: LPARAM) -> (f32, f32) {
    ((lparam.0 & 0xFFFF) as u16 as i16 as f32, ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32)
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        // All of the dialog is its own to draw: no frame (Windows keeps its shadow and round corners).
        WM_NCCALCSIZE if wparam.0 != 0 => LRESULT(0),
        // The dialog moved by any of it that takes no press.
        WM_NCHITTEST => {
            let (x, y) = point(lparam);
            let mut at = POINT { x: x as i32, y: y as i32 };
            unsafe {
                let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut at);
            }
            let caption = with_ui(|ui| ui.guide_caption(hwnd, at.x as f32, at.y as f32)).unwrap_or(false);
            LRESULT(if caption { HTCAPTION } else { HTCLIENT } as isize)
        }
        // The tip takes presses without taking the front from the window it is beside.
        WM_MOUSEACTIVATE if with_ui(|ui| ui.guide.as_ref().and_then(|guide| guide.tip.as_ref()).is_some_and(|tip| tip.hwnd == hwnd)).unwrap_or(false) => LRESULT(MA_NOACTIVATE as isize),
        WM_PAINT => {
            unsafe {
                let _ = ValidateRect(Some(hwnd), None);
            }
            with_ui(|ui| ui.next_frame = Instant::now());
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, y) = point(lparam);
            with_ui(|ui| {
                let mut track = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                let _ = unsafe { TrackMouseEvent(&mut track) };
                ui.guide_pointer(hwnd, Some((x, y)), None);
            });
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_ui(|ui| ui.guide_pointer(hwnd, None, None));
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = point(lparam);
            unsafe { SetCapture(hwnd) };
            with_ui(|ui| ui.guide_pointer(hwnd, Some((x, y)), Some(true)));
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = point(lparam);
            unsafe {
                let _ = ReleaseCapture();
            }
            with_ui(|ui| ui.guide_pointer(hwnd, Some((x, y)), Some(false)));
            LRESULT(0)
        }
        // Enter goes on, Esc leaves it, as a dialog's keys do.
        WM_KEYDOWN => {
            match VIRTUAL_KEY(wparam.0 as u16) {
                VK_RETURN => with_ui(|ui| ui.guide_hit(Hit::Next)),
                VK_ESCAPE => with_ui(|ui| ui.guide_hit(Hit::Skip)),
                _ => None,
            };
            LRESULT(0)
        }
        WM_CLOSE => {
            with_ui(|ui| ui.guide_hit(Hit::Skip));
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let suggested = unsafe { *(lparam.0 as *const RECT) };
            with_ui(|ui| ui.guide_scaled(hwnd, (wparam.0 & 0xFFFF) as f32 / 96.0));
            unsafe {
                let _ = SetWindowPos(hwnd, None, suggested.left, suggested.top, suggested.right - suggested.left, suggested.bottom - suggested.top, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// What search and the look show in the small desktop of the last step, as large as on the screen (DIPs): search's
/// sheet, and the look beside it.
const SEARCH: (f32, f32) = (640.0, 236.0);
const LOOK: (f32, f32) = (380.0, 470.0);

/// What the last step's search finds, and the look shows of the first: the words typed, the files (their name, their
/// folder, their kind and its colour), the look's heading and its lines.
struct Example {
    typed: &'static str,
    group: &'static str,
    files: [(&'static str, &'static str, &'static str, u32); 2],
    size: &'static str,
    heading: &'static str,
    lines: [(&'static str, &'static str); 5],
}

fn example(lang: Lang) -> Example {
    match lang {
        Lang::Zh => Example {
            typed: "公式",
            group: "文件",
            files: [("期中公式纸.pdf", r"D:\Documents\课程\力学", "PDF", 0xD6282E), ("公式推导笔记.docx", r"D:\Documents\课程\力学", "DOCX", 0x2B62C4)],
            size: "741 KB",
            heading: "力学 期中公式",
            lines: [("牛顿第二定律", "F = m a"), ("动能", "E = ½ m v²"), ("胡克定律", "F = −k x"), ("角动量", "L = I ω"), ("万有引力", "F = G m₁m₂ / r²")],
        },
        Lang::En => Example {
            typed: "formula",
            group: "Files",
            files: [("Midterm formula sheet.pdf", r"D:\Documents\Courses\Mechanics", "PDF", 0xD6282E), ("Formula notes.docx", r"D:\Documents\Courses\Mechanics", "DOCX", 0x2B62C4)],
            size: "741 KB",
            heading: "Mechanics midterm formulas",
            lines: [("Newton's second law", "F = m a"), ("Kinetic energy", "E = ½ m v²"), ("Hooke's law", "F = −k x"), ("Angular momentum", "L = I ω"), ("Gravitation", "F = G m₁m₂ / r²")],
        },
    }
}

/// The pointer the plays move, its tip at `at`, `size` across, as Windows draws its own: white, outlined, a little
/// shadow under it.
fn paint_pointer(frame: &Frame, at: (f32, f32), size: f32) {
    // (Its icon's tip is near the corner of its box: the box's middle is that far from it.)
    let off = (12.0 - 4.2) * size / 24.0;
    let centre = Point { x: at.0 + off, y: at.1 + off };
    frame.icon_solid(Icon::Pointer, Point { x: centre.x, y: centre.y + 1.0 }, size, Color::hex(0x000000, 0.3));
    frame.icon_solid(Icon::Pointer, centre, size, Color::hex(0xFFFFFF, 1.0));
    frame.icon(Icon::Pointer, centre, size, Color::hex(0x111111, 1.0));
}

/// `paint` drawn `opacity` opaque, within `bounds`.
fn faded(frame: &Frame, bounds: Rect, opacity: f32, paint: impl FnOnce()) {
    let layer = D2D1_LAYER_PARAMETERS1 { contentBounds: rect(bounds.x, bounds.y, bounds.w, bounds.h), maskTransform: Matrix3x2::identity(), opacity, ..Default::default() };
    unsafe { frame.dc.PushLayer(&layer, None) };
    paint();
    unsafe { frame.dc.PopLayer() };
}

/// One line of `text` in the middle of `r`.
fn centred(frame: &Frame, text: &str, font: Font, color: Color, r: Rect) {
    let wide = frame.gfx.measure(text, font).min(r.w);
    text_centred(frame, text, font, color, r.x + (r.w - wide) / 2.0, r.y + r.h / 2.0, wide + 1.0, Align::Start);
}

/// A button of its windows in `r`, as Windows 11 draws one: outlined, or the one that goes on in the accent; lit under
/// the pointer, darker held down.
#[allow(clippy::too_many_arguments)]
fn button(frame: &Frame, palette: &Palette, grounds: &Grounds, popup: &mut Popup, r: Rect, text: &str, hit: Hit, accent: bool) {
    let hover = popup.hovered() == Some(hit);
    let down = hover && popup.pressed == Some(hit);
    if accent {
        fill(frame, palette.switch_on.alpha(if down { 0.8 } else if hover { 0.9 } else { 1.0 }), r.x, r.y, r.w, r.h, 4.0);
        centred(frame, text, BODY, Color::hex(0xFFFFFF, 1.0), r);
    } else {
        fill(frame, if hover { grounds.button_hover } else { grounds.button }, r.x, r.y, r.w, r.h, 4.0);
        stroke_inside(frame, r, 4.0, grounds.button_stroke);
        centred(frame, text, BODY, if down { palette.text2 } else { palette.text }, r);
    }
    popup.hits.push((r, hit));
}

/// The buttons a step has, from the left: leaving it (but at its end), going back (but at its first step), going on
/// (the last step's and its end's say it is done).
fn buttons(step: Step, lang: Lang) -> Vec<(&'static str, Hit, bool)> {
    let p = |zh: &'static str, en: &'static str| pick(lang, zh, en);
    let mut buttons = Vec::new();
    if step != Step::Ready {
        buttons.push((p("跳过引导", "Skip guide"), Hit::Skip, false));
    }
    if step.before().is_some() {
        buttons.push((p("上一步", "Back"), Hit::Back, false));
    }
    let next = if matches!(step, Step::Keys | Step::Ready) { p("完成", "Done") } else { p("下一步", "Next") };
    buttons.push((next, Hit::Next, true));
    buttons
}

/// `buttons` side by side, the last at `right`, their middles on `cy`.
fn paint_buttons(frame: &Frame, palette: &Palette, grounds: &Grounds, popup: &mut Popup, buttons: &[(&str, Hit, bool)], right: f32, cy: f32, least: f32) {
    let mut x = right;
    for &(text, hit, accent) in buttons.iter().rev() {
        let w = (frame.gfx.measure(text, BODY) + 40.0).max(least);
        x -= w;
        button(frame, palette, grounds, popup, Rect { x, y: cy - BUTTON / 2.0, w, h: BUTTON }, text, hit, accent);
        x -= 8.0;
    }
}

/// Whether a step is done, in a line: a tick and what it says done, else what it asks.
fn paint_state(frame: &Frame, palette: &Palette, grounds: &Grounds, words: &Words, done: bool, x: f32, cy: f32, width: f32) {
    if done {
        frame.icon(Icon::Check, Point { x: x + 8.0, y: cy }, 16.0, grounds.done);
        text_centred(frame, words.done, STATE, grounds.done, x + 22.0, cy, width - 22.0, Align::Start);
    } else {
        text_centred(frame, &words.wait, STATE, palette.text2, x, cy, width, Align::Start);
    }
}

impl Ui {
    /// The guide begun, at its first step; up already, brought to the front.
    pub(super) fn begin_guide(&mut self) {
        if let Some(guide) = &self.guide {
            let shown = [guide.dialog.as_ref(), guide.tip.as_ref()].into_iter().flatten().any(|popup| unsafe { IsWindowVisible(popup.hwnd) }.as_bool());
            match guide.dialog.as_ref().filter(|_| shown && guide.step != Step::Layout) {
                Some(dialog) => unsafe {
                    let _ = SetForegroundWindow(dialog.hwnd);
                },
                None => self.show_for_guide(),
            }
            return;
        }
        register();
        let was_up = unsafe { IsWindowVisible(self.hwnd) }.as_bool();
        self.guide = Some(Guide {
            step: Step::Push,
            done: [false; 3],
            tried: [false; 2],
            was_up,
            since: Instant::now(),
            edge: (Edge::Right, None),
            dialog: None,
            tip: None,
            drag: None,
            grounds: [render::GroundLayers::default(), render::GroundLayers::default()],
            _showing: crate::ui::life::Showing::of(&self.gfx),
        });
        TOLD.store(0, Ordering::Release);
        UP.store(true, Ordering::Release);
        // Shown: not again unasked.
        if !self.settings.guided {
            self.settings.guided = true;
            self.save();
        }
        self.guide_to(Step::Push);
    }

    /// The guide turned to `step`: the layout's on the window's own page with the tip beside it, the others in the
    /// dialog with the window hidden.
    fn guide_to(&mut self, step: Step) {
        self.stop_drag();
        let now = Instant::now();
        // (Its windows made as they are first wanted.)
        let (want_tip, want_dialog) = match &self.guide {
            Some(guide) => (step == Step::Layout && guide.tip.is_none(), step != Step::Layout && guide.dialog.is_none()),
            None => return,
        };
        let tip = if want_tip { self.make_tip() } else { None };
        let dialog = if want_dialog { self.make_dialog() } else { None };
        let Some(guide) = self.guide.as_mut() else { return };
        if tip.is_some() {
            guide.tip = tip;
        }
        if dialog.is_some() {
            guide.dialog = dialog;
        }
        guide.step = step;
        guide.since = now;
        // (What was done before the step is shown is not its doing.)
        TOLD.store(0, Ordering::Release);
        if step == Step::Layout {
            if let Some(dialog) = &guide.dialog {
                unsafe {
                    let _ = ShowWindow(dialog.hwnd, SW_HIDE);
                }
            }
            self.turn_to(Page::Layout);
            self.scroll = 0.0;
            self.scroll_target = 0.0;
            // One screen large: its desktop, and the real panel slid out as a component is put on its edge.
            if self.screens.len() > 1 {
                self.view = None;
                self.view = self.stage_screen();
            }
            self.shown_edge = None;
            self.show_for_guide();
            let screen = self.editor_screen().or_else(|| self.stage.screen.clone()).unwrap_or_default();
            let before = self.settings.places().clone();
            let held = before.part(&screen, Edge::Left);
            let moves = script(played_part(held), held);
            if let Some(guide) = self.guide.as_mut() {
                guide.drag = Some(Drag { before, screen, moves, at: 0, since: now, from: None, pointer: None });
            }
        } else {
            if let Some(tip) = &guide.tip {
                unsafe {
                    let _ = ShowWindow(tip.hwnd, SW_HIDE);
                }
            }
            // The edge the panel is shown opened from: the right, where it opens anything, else the first that does.
            let screen = self.stage.screen.clone().unwrap_or_default();
            let places = self.settings.places();
            let edge = if places.part(&screen, Edge::Right).is_some() { Edge::Right } else { places.lit(&screen).each().next().unwrap_or(Edge::Right) };
            let part = places.part(&screen, edge);
            guide.edge = (edge, part);
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.fit_dialog();
            if let Some(dialog) = self.guide.as_ref().and_then(|guide| guide.dialog.as_ref()) {
                unsafe {
                    // (Glance started hidden has the system take the first window it shows as hidden: shown again.)
                    let _ = ShowWindow(dialog.hwnd, SW_SHOW);
                    if !IsWindowVisible(dialog.hwnd).as_bool() {
                        let _ = ShowWindow(dialog.hwnd, SW_SHOW);
                    }
                    let _ = SetForegroundWindow(dialog.hwnd);
                }
            }
        }
        self.next_frame = now;
    }

    /// The window shown and in front, for the guide's step on its page.
    fn show_for_guide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            if !IsWindowVisible(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_SHOW);
            }
            let _ = SetForegroundWindow(self.hwnd);
        }
    }

    /// The guide ended (left, or done): its windows gone, and the window shown again on the general page if it was up
    /// as it began, else closed.
    fn end_guide(&mut self) {
        self.stop_drag();
        let Some(guide) = self.guide.take() else { return };
        let was_up = guide.was_up;
        drop(guide);
        if was_up {
            self.turn_to(Page::General);
            self.show_for_guide();
        } else {
            unsafe {
                let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }

    /// The dialog's window, made hidden, in the middle of the screen the window is on.
    fn make_dialog(&self) -> Option<Popup> {
        let work = work_of(self.hwnd);
        let (wide, tall) = ((WIDE * self.scale) as i32, (DESK * self.scale) as i32);
        let (x, y) = ((work.left + work.right - wide) / 2, (work.top + work.bottom - tall) / 2);
        unsafe {
            let module = GetModuleHandleW(None).ok()?;
            let title = HSTRING::from(pick(self.lang, "Glance 使用引导", "Glance guide"));
            let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP, CLASS, &title, WS_POPUP | WS_CAPTION | WS_SYSMENU, x, y, wide, tall, None, None, Some(module.into()), None).ok()?;
            // Its shadow and its round corners, as Windows gives a window with a frame.
            let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS { cxLeftWidth: 1, cxRightWidth: 1, cyTopHeight: 1, cyBottomHeight: 1 });
            let round = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &round as *const _ as *const _, size_of_val(&round) as u32);
            let dark = BOOL::from(self.dark);
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as *const _, 4);
            // (Its frame gone at once: the size it was made at is the size it is.)
            let _ = SetWindowPos(hwnd, None, x, y, wide, tall, SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
            let surface = Surface::new(&self.gfx, hwnd).ok();
            let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
            Some(Popup { hwnd, surface, scale, size: (WIDE, DESK), hits: Vec::new(), pointer: None, pressed: None, plate: Rect { x: 0.0, y: 0.0, w: WIDE, h: DESK }, beak: None })
        }
    }

    /// The tip's window, made hidden (placed beside the window as it is drawn: see `place_tip`), kept over the window.
    fn make_tip(&self) -> Option<Popup> {
        unsafe {
            let module = GetModuleHandleW(None).ok()?;
            let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW, CLASS, w!(""), WS_POPUP, 0, 0, 1, 1, Some(self.hwnd), None, Some(module.into()), None).ok()?;
            let surface = Surface::new(&self.gfx, hwnd).ok();
            Some(Popup { hwnd, surface, scale: self.scale, size: (0.0, 0.0), hits: Vec::new(), pointer: None, pressed: None, plate: Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }, beak: None })
        }
    }

    /// The dialog as tall as its step asks, its middle where it was.
    fn fit_dialog(&mut self) {
        let tall = self.dialog_height();
        let Some(dialog) = self.guide.as_mut().and_then(|guide| guide.dialog.as_mut()) else { return };
        if dialog.size.1 == tall {
            return;
        }
        dialog.size = (WIDE, tall);
        dialog.plate = Rect { x: 0.0, y: 0.0, w: WIDE, h: tall };
        let mut was = RECT::default();
        unsafe {
            let _ = GetWindowRect(dialog.hwnd, &mut was);
        }
        let work = work_of(dialog.hwnd);
        let (wide, high) = ((WIDE * dialog.scale) as i32, ((tall * dialog.scale) as i32).min(work.bottom - work.top));
        let x = ((was.left + was.right - wide) / 2).clamp(work.left, (work.right - wide).max(work.left));
        let y = ((was.top + was.bottom - high) / 2).clamp(work.top, (work.bottom - high).max(work.top));
        unsafe {
            let _ = SetWindowPos(dialog.hwnd, None, x, y, wide, high, SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    /// How tall the dialog is for the step it shows (DIPs).
    fn dialog_height(&self) -> f32 {
        let Some(guide) = &self.guide else { return DESK };
        let words = words(guide.step, guide.edge.0, self.lang);
        let inner = WIDE - 2.0 * PAD;
        let desk = if guide.step == Step::Ready { 0.0 } else { DESK + 20.0 };
        let body = if words.body.is_empty() { 0.0 } else { 6.0 + self.gfx.wrapped_height(&words.body, BODY, inner) };
        let keys = if guide.step == Step::Keys { 12.0 + 2.0 * KEY_ROW + 8.0 } else { 0.0 };
        let state = if guide.step == Step::Ready { 0.0 } else { 12.0 + 20.0 };
        PAD + desk + 28.0 + body + keys + state + PAD + FOOTER
    }

    /// How tall the tip's plate is (DIPs).
    fn tip_height(&self) -> f32 {
        let words = words(Step::Layout, Edge::Left, self.lang);
        TIP_PAD + 24.0 + self.gfx.wrapped_height(&words.body, BODY, TIP_WIDE - 2.0 * TIP_PAD) + 10.0 + 20.0 + 14.0 + BUTTON + TIP_PAD
    }

    /// Where the tip goes, `plate_h` DIPs tall at `scale`: beside the window by its editor, its beak at it, on the
    /// window's right where there is room, else on its left; else over the window at the editor's foot, pointing at
    /// nothing. The side its beak is on and how far down it (DIPs), and its window, the room for its shadow round its
    /// plate (physical px).
    fn place_tip(&self, scale: f32, plate_h: f32) -> (Option<(Edge, f32)>, RECT) {
        let mut window = RECT::default();
        let mut corner = POINT::default();
        unsafe {
            let _ = GetWindowRect(self.hwnd, &mut window);
            let _ = ClientToScreen(self.hwnd, &mut corner);
        }
        let work = work_of(self.hwnd);
        let px = |dips: f32| (dips * scale).round() as i32;
        // The editor on the screen (the window's DIPs are of its own scale).
        let editor = self.editor_at.unwrap_or(Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 });
        let on_screen = |x: f32, y: f32| (corner.x + (x * self.scale).round() as i32, corner.y + (y * self.scale).round() as i32);
        let (_, middle) = on_screen(0.0, editor.y + editor.h / 2.0);
        let (right, foot) = on_screen(editor.x + editor.w, editor.y + editor.h);
        let (w, h) = (px(TIP_WIDE), px(plate_h));
        let room = px(BEAK + TIP_OFF);
        let beside = (middle - h / 2).clamp(work.top, (work.bottom - h).max(work.top));
        // With no room beside the window: in it, left of the editor and under the components (what is dragged and
        // where it goes both left in sight), where that holds it.
        let (editor_left, _) = on_screen(editor.x, 0.0);
        let (_, under_library) = self.library_at.map_or((0, corner.y), |r| on_screen(0.0, r.y + r.h + 12.0));
        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut client);
        }
        let client_bottom = corner.y + client.bottom;
        let inside = (editor_left - room - w, (middle - h / 2).clamp(under_library, (client_bottom - h - px(12.0)).max(under_library)));
        let ((left, top), side) = if window.right + room + w <= work.right {
            ((window.right + room, beside), Some(Edge::Left))
        } else if window.left - room - w >= work.left {
            ((window.left - room - w, beside), Some(Edge::Right))
        } else if inside.0 >= corner.x && inside.1 + h <= client_bottom {
            (inside, Some(Edge::Right))
        } else {
            (((right - w - px(16.0)).clamp(work.left, (work.right - w).max(work.left)), (foot - h - px(16.0)).clamp(work.top, (work.bottom - h).max(work.top))), None)
        };
        let beak = side.map(|side| (side, ((middle - top) as f32 / scale).clamp(20.0, plate_h - 20.0)));
        let pad = px(SHADOW);
        (beak, RECT { left: left - pad, top: top - pad, right: left + w + pad, bottom: top + h + pad })
    }

    /// The guide's windows drawn, what was done elsewhere taken in first: the dialog, or the tip beside the window,
    /// placed by it as it is now.
    pub(super) fn draw_guide(&mut self, now: Instant) {
        let Some(mut guide) = self.guide.take() else { return };
        let told = TOLD.swap(0, Ordering::AcqRel);
        match guide.step {
            Step::Push if told & Saw::Pushed.bit() != 0 => guide.done[0] = true,
            Step::Keys => {
                for (at, saw) in [Saw::Searched, Saw::Looked].into_iter().enumerate() {
                    if told & saw.bit() != 0 {
                        guide.tried[at] = true;
                    }
                }
                if guide.tried == [true, true] {
                    guide.done[2] = true;
                }
            }
            _ => {}
        }
        let gfx = self.gfx.clone();
        if let Some(mut dialog) = guide.dialog.take() {
            if guide.step != Step::Layout && unsafe { IsWindowVisible(dialog.hwnd) }.as_bool() {
                self.draw_popup(&gfx, &mut dialog, |ui, frame, popup| ui.paint_dialog(frame, &mut guide, popup, now));
            }
            guide.dialog = Some(dialog);
        }
        if let Some(mut tip) = guide.tip.take().filter(|_| guide.step == Step::Layout) {
            // Beside the window, as it is now (moved, sized, on another screen).
            tip.scale = unsafe { GetDpiForWindow(self.hwnd) }.max(96) as f32 / 96.0;
            let plate_h = self.tip_height();
            let (beak, at) = self.place_tip(tip.scale, plate_h);
            let mut was = RECT::default();
            unsafe {
                let _ = GetWindowRect(tip.hwnd, &mut was);
                if was != at {
                    let _ = SetWindowPos(tip.hwnd, None, at.left, at.top, at.right - at.left, at.bottom - at.top, SWP_NOZORDER | SWP_NOACTIVATE);
                }
            }
            tip.size = ((at.right - at.left) as f32 / tip.scale, (at.bottom - at.top) as f32 / tip.scale);
            tip.plate = Rect { x: SHADOW, y: SHADOW, w: TIP_WIDE, h: plate_h };
            tip.beak = beak;
            self.draw_popup(&gfx, &mut tip, |ui, frame, popup| ui.paint_tip(frame, &guide, popup));
            unsafe {
                if !IsWindowVisible(tip.hwnd).as_bool() {
                    let _ = ShowWindow(tip.hwnd, SW_SHOWNOACTIVATE);
                }
            }
            guide.tip = Some(tip);
        }
        self.guide = Some(guide);
        self.next_frame = self.next_frame.min(now + FRAME);
    }

    /// `popup` drawn by `paint` (at its own scale: what is drawn as the window's is drawn at the popup's meanwhile).
    fn draw_popup(&mut self, gfx: &Rc<Gfx>, popup: &mut Popup, paint: impl FnOnce(&mut Self, &Frame, &mut Popup)) {
        let Some(mut surface) = popup.surface.take() else { return };
        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(popup.hwnd, &mut client);
        }
        let size = ((client.right - client.left).max(1) as u32, (client.bottom - client.top).max(1) as u32);
        let scale = popup.scale;
        let window_scale = std::mem::replace(&mut self.scale, scale);
        let drawn = surface.draw(gfx, size, scale, |frame| {
            frame.origin(0.0, 0.0);
            paint(self, frame, popup);
        });
        self.scale = window_scale;
        if let Err(error) = &drawn {
            gfx::failed("guide", Some(error));
        }
        popup.surface = Some(surface);
    }

    /// The dialog: the small desktop with the step's play at its top, its place among the steps, its title and what it
    /// asks (the shortcuts' rows, on the last step), whether it is done; the buttons in the footer.
    fn paint_dialog(&mut self, frame: &Frame, guide: &mut Guide, dialog: &mut Popup, now: Instant) {
        let palette = self.palette.clone();
        let grounds = Grounds::new(self.dark);
        let lang = self.lang;
        let (width, height) = dialog.size;
        let inner = width - 2.0 * PAD;
        let footer = height - FOOTER;
        dialog.hits.clear();
        frame.fill(grounds.dialog, 0.0, 0.0, width, height);
        frame.fill(grounds.footer, 0.0, footer, width, FOOTER);
        frame.fill(palette.rule, 0.0, footer, width, 1.0);
        let step = guide.step;
        let words = words(step, guide.edge.0, lang);
        let mut y = PAD;
        let mut down = [0, 0];
        if step != Step::Ready {
            let area = Rect { x: PAD, y, w: inner, h: DESK };
            fill(frame, palette.card, area.x, area.y, area.w, area.h, 8.0);
            match step {
                Step::Push => self.play_push(frame, guide, area, now),
                Step::Keys => down = self.play_keys(frame, guide, area, now),
                _ => {}
            }
            stroke_inside(frame, area, 8.0, palette.card_stroke);
            y += DESK + 20.0;
        }
        if let Some(index) = step.index() {
            let count = format!("{} / 3", index + 1);
            frame.text(&count, SMALL, palette.text3, PAD, y + 6.0, inner, Align::End);
        }
        frame.text(words.title, TITLE, palette.text, PAD, y, inner - 60.0, Align::Start);
        y += 28.0;
        if !words.body.is_empty() {
            y += 6.0;
            frame.text_wrapped(&words.body, BODY, palette.text2, PAD, y, inner);
            y += frame.gfx.wrapped_height(&words.body, BODY, inner);
        }
        if step == Step::Keys {
            y += 12.0;
            self.paint_keys(frame, &palette, &grounds, guide, dialog, Rect { x: PAD, y, w: inner, h: 2.0 * KEY_ROW + 8.0 }, down);
            y += 2.0 * KEY_ROW + 8.0;
        }
        if let Some(index) = step.index() {
            paint_state(frame, &palette, &grounds, &words, guide.done[index], PAD, y + 12.0 + 10.0, inner);
        }
        paint_buttons(frame, &palette, &grounds, dialog, &buttons(step, lang), width - PAD, footer + FOOTER / 2.0, BUTTON_LEAST);
    }

    /// The shortcuts' rows of the last step in `r`: what each calls up, its keys (as many down as `down` says, as the
    /// play presses them), and a tick once it was tried. Each pressed calls up what it does.
    #[allow(clippy::too_many_arguments)]
    fn paint_keys(&self, frame: &Frame, palette: &Palette, grounds: &Grounds, guide: &Guide, popup: &mut Popup, r: Rect, down: [usize; 2]) {
        let p = |zh: &'static str, en: &'static str| pick(self.lang, zh, en);
        let rows = [
            (Hit::Search, p("搜索", "Search"), p("应用、文件与便签", "Apps, files and notes"), self.settings.search_shortcut),
            (Hit::Look, p("预览", "Preview"), p("选中的文件或卡片", "The file or card picked"), self.settings.preview_shortcut),
        ];
        let hovered = popup.hovered();
        for (at, (hit, name, about, shortcut)) in rows.into_iter().enumerate() {
            let row = Rect { x: r.x, y: r.y + at as f32 * (KEY_ROW + 8.0), w: r.w, h: KEY_ROW };
            fill(frame, if hovered == Some(hit) { palette.control_on } else { palette.control }, row.x, row.y, row.w, row.h, 4.0);
            stroke_inside(frame, row, 4.0, palette.card_stroke);
            frame.text(name, BODY, palette.text, row.x + 14.0, row.y + 8.0, row.w / 2.0, Align::Start);
            frame.text(about, SMALL, palette.text2, row.x + 14.0, row.y + 28.0, row.w / 2.0, Align::Start);
            let cy = row.y + row.h / 2.0;
            let mut x = row.x + row.w - 14.0;
            if guide.tried[at] {
                frame.icon(Icon::Check, Point { x: x - 8.0, y: cy }, 16.0, grounds.done);
            }
            x -= 28.0;
            let keys = caps(shortcut, self.lang);
            let widths: Vec<f32> = keys.iter().map(|key| (frame.gfx.measure(key, SMALL) + 16.0).max(30.0)).collect();
            x -= widths.iter().sum::<f32>() + 6.0 * (keys.len() - 1) as f32;
            for (index, (key, w)) in keys.iter().zip(widths).enumerate() {
                let pressed = index < down[at];
                let cap = Rect { x, y: cy - 12.0 + if pressed { 1.0 } else { 0.0 }, w, h: 24.0 };
                if pressed {
                    fill(frame, palette.switch_on, cap.x, cap.y, cap.w, cap.h, 4.0);
                    centred(frame, key, SMALL, Color::hex(0xFFFFFF, 1.0), cap);
                } else {
                    // (A key's foot: its edge heavier below.)
                    fill(frame, palette.rule.alpha(palette.rule.a * 2.0), cap.x, cap.y + 2.0, cap.w, cap.h, 4.0);
                    fill(frame, grounds.button, cap.x, cap.y, cap.w, cap.h, 4.0);
                    stroke_inside(frame, cap, 4.0, grounds.button_stroke);
                    centred(frame, key, SMALL, palette.text, cap);
                }
                x += cap.w + 6.0;
            }
            popup.hits.push((row, hit));
        }
    }

    /// The first step's play in `area`: the desktop, the pointer gliding to the edge and pushing it, the real panel
    /// sliding out, the pointer back and the panel put away, round and round.
    fn play_push(&mut self, frame: &Frame, guide: &Guide, area: Rect, now: Instant) {
        let (along, out) = pushing(now.saturating_duration_since(guide.since));
        let (edge, part) = guide.edge;
        let screen = match part {
            Some(Part::Music) => self.music_on_stage(frame, area, edge, out).1,
            Some(part) => {
                let prefs = self.prefs.clone().holding(part);
                self.panel_on_stage(frame, area, Some(edge), Some(&prefs), false, out).1
            }
            None => self.panel_on_stage(frame, area, None, None, true, 1.0).1,
        };
        // (The screen as much as shows of it.)
        let (left, top) = (screen.x.max(area.x), screen.y.max(area.y));
        let (right, bottom) = ((screen.x + screen.w).min(area.x + area.w), (screen.y + screen.h).min(area.y + area.h));
        let rest = (area.x + area.w * 0.42, area.y + area.h * 0.62);
        let pushed = match edge {
            Edge::Right => (right - 3.0, (top + bottom) / 2.0),
            Edge::Left => (left + 3.0, (top + bottom) / 2.0),
            Edge::Top => ((left + right) / 2.0, top + 3.0),
        };
        paint_pointer(frame, (rest.0 + (pushed.0 - rest.0) * along, rest.1 + (pushed.1 - rest.1) * along), 20.0);
    }

    /// The last step's play in `area`: the desktop by the middle of its top, search coming up on it as its shortcut is
    /// pressed, the first file found looked at larger beside it as the preview's is, both gone again, round and round.
    /// How many of each shortcut's keys are down.
    fn play_keys(&mut self, frame: &Frame, guide: &mut Guide, area: Rect, now: Instant) -> [usize; 2] {
        let keys = (caps(self.settings.search_shortcut, self.lang).len(), caps(self.settings.preview_shortcut, self.lang).len());
        let (down, search, look) = Keying::new(keys).at(now.saturating_duration_since(guide.since));
        // Search where it comes up, a little way down the middle of the screen, the look beside it; the part of the
        // screen they are in, and some room round them, shown.
        let (sw, sh) = self.stage.size;
        let s = Rect { x: ((sw - SEARCH.0 - 16.0 - LOOK.0) / 2.0).round(), y: (sh * 0.16).round(), w: SEARCH.0, h: SEARCH.1 };
        let l = Rect { x: s.x + s.w + 16.0, y: s.y, w: LOOK.0, h: LOOK.1 };
        let room = 40.0;
        let shown = Rect { x: s.x - room, y: s.y - room, w: l.x + l.w - s.x + 2.0 * room, h: l.h.max(s.h) + 2.0 * room };
        let k = (area.w / shown.w).min(area.h / shown.h);
        let (ox, oy) = (area.x + area.w / 2.0 - (shown.x + shown.w / 2.0) * k, area.y + area.h / 2.0 - (shown.y + shown.h / 2.0) * k);
        self.stage_picture(frame, area, (ox, oy), k, 1.0);
        let skin = Skin::named(&self.settings.skin);
        let example = example(self.lang);
        unsafe { frame.dc.PushAxisAlignedClip(&rect(area.x, area.y, area.w, area.h), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
        if search > 0.0 {
            // (Coming down a little as it shows.)
            let drop = (1.0 - search) * -8.0;
            let local = Matrix3x2::scale(k, k) * Matrix3x2::translation(ox + s.x * k, oy + (s.y + drop) * k);
            let behind = RECT { left: s.x as i32, top: s.y as i32, right: (s.x + s.w) as i32, bottom: (s.y + s.h) as i32 };
            let (theme, frost) = self.stage.look(skin, self.prefs.theme, behind);
            let scale = self.scale * k;
            faded(frame, area, search, || {
                self.stage.music_ground(frame, &mut guide.grounds[0], &theme, frost, (s.w, s.h), Some(((s.x, s.y), 1.0)), local, scale);
                frame.place(local);
                paint_search(frame, &theme, &example, (s.w, s.h));
                frame.origin(0.0, 0.0);
            });
        }
        if look > 0.0 {
            // (Growing out of the search's first row, beside which it stands.)
            let grow = 0.25 + 0.75 * look;
            let from = l.w * 0.2;
            let local = Matrix3x2::scale(grow * k, grow * k) * Matrix3x2::translation(ox + (l.x + from - from * grow) * k, oy + l.y * k);
            let behind = RECT { left: l.x as i32, top: l.y as i32, right: (l.x + l.w) as i32, bottom: (l.y + l.h) as i32 };
            let (theme, frost) = self.stage.look(skin, self.prefs.theme, behind);
            let scale = self.scale * k * grow;
            faded(frame, area, look.min(1.0), || {
                self.stage.music_ground(frame, &mut guide.grounds[1], &theme, frost, (l.w, l.h), Some(((l.x, l.y), 1.0)), local, scale);
                frame.place(local);
                paint_look(frame, &theme, &example, (l.w, l.h));
                frame.origin(0.0, 0.0);
            });
        }
        unsafe { frame.dc.PopAxisAlignedClip() };
        down
    }

    /// The tip: its plate, its beak at the editor, its place among the steps, its title and what it asks, whether it is
    /// done, its buttons.
    fn paint_tip(&mut self, frame: &Frame, guide: &Guide, tip: &mut Popup) {
        let palette = self.palette.clone();
        let grounds = Grounds::new(self.dark);
        let lang = self.lang;
        tip.hits.clear();
        let p = tip.plate;
        frame.shadow(grounds.shadow, p.x, p.y, p.w, p.h, 8.0, 16.0, 6.0);
        fill(frame, grounds.flyout, p.x, p.y, p.w, p.h, 8.0);
        stroke_inside(frame, p, 8.0, palette.rule);
        if let Some((side, down)) = tip.beak {
            // Over the plate's edge where it stands out, outlined but at its foot.
            let (foot, point) = match side {
                Edge::Right => (p.x + p.w - 1.0, p.x + p.w + BEAK),
                _ => (p.x + 1.0, p.x - BEAK),
            };
            let (top, bottom) = (p.y + down - BEAK, p.y + down + BEAK);
            frame.fill_shape(&[Point { x: foot, y: top }, Point { x: point, y: p.y + down }, Point { x: foot, y: bottom }], crate::ui::canvas::Fill::Solid(grounds.flyout));
            frame.stroke(&[Point { x: foot, y: top }, Point { x: point, y: p.y + down }, Point { x: foot, y: bottom }], palette.rule, 1.0);
        }
        let words = words(Step::Layout, Edge::Left, lang);
        let (x, inner) = (p.x + TIP_PAD, p.w - 2.0 * TIP_PAD);
        let mut y = p.y + TIP_PAD;
        frame.text("2 / 3", SMALL, palette.text3, x, y + 3.0, inner, Align::End);
        frame.text(words.title, TIP_TITLE, palette.text, x, y, inner - 40.0, Align::Start);
        y += 24.0;
        frame.text_wrapped(&words.body, BODY, palette.text2, x, y, inner);
        y += frame.gfx.wrapped_height(&words.body, BODY, inner) + 10.0;
        paint_state(frame, &palette, &grounds, &words, guide.done[1], x, y + 10.0, inner);
        y += 20.0 + 14.0;
        paint_buttons(frame, &palette, &grounds, tip, &buttons(Step::Layout, lang), x + inner, y + BUTTON / 2.0, 72.0);
    }

    /// Whether a press at `x`, `y` (physical px) on the guide's window `hwnd` moves it: on the dialog, where it takes no
    /// press.
    fn guide_caption(&self, hwnd: HWND, x: f32, y: f32) -> bool {
        let Some(dialog) = self.guide.as_ref().and_then(|guide| guide.dialog.as_ref()).filter(|dialog| dialog.hwnd == hwnd) else { return false };
        let (x, y) = (x / dialog.scale, y / dialog.scale);
        !dialog.hits.iter().any(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
    }

    /// The pointer on the guide's window `hwnd` at `at` (physical px; none: gone from it), pressed (true) or let go
    /// (false) there: let go on what it was pressed on, that done.
    fn guide_pointer(&mut self, hwnd: HWND, at: Option<(f32, f32)>, press: Option<bool>) {
        let Some(guide) = self.guide.as_mut() else { return };
        let Some(popup) = [guide.dialog.as_mut(), guide.tip.as_mut()].into_iter().flatten().find(|popup| popup.hwnd == hwnd) else { return };
        popup.pointer = at.map(|(x, y)| (x / popup.scale, y / popup.scale));
        let hit = match press {
            Some(true) => {
                popup.pressed = popup.hovered();
                None
            }
            Some(false) => popup.pressed.take().filter(|&pressed| popup.hovered() == Some(pressed)),
            None => None,
        };
        self.next_frame = Instant::now();
        if let Some(hit) = hit {
            self.guide_hit(hit);
        }
    }

    /// Does what `hit` does.
    fn guide_hit(&mut self, hit: Hit) {
        let Some(step) = self.guide.as_ref().map(|guide| guide.step) else { return };
        match hit {
            Hit::Skip => self.end_guide(),
            Hit::Back => {
                if let Some(before) = step.before() {
                    self.guide_to(before);
                }
            }
            Hit::Next => match step.after() {
                Some(after) => self.guide_to(after),
                None => self.end_guide(),
            },
            Hit::Search => crate::ui::find::toggle(),
            Hit::Look => crate::ui::find::preview(),
        }
    }

    /// The dialog `hwnd` moved to a screen of another scale.
    fn guide_scaled(&mut self, hwnd: HWND, scale: f32) {
        if let Some(dialog) = self.guide.as_mut().and_then(|guide| guide.dialog.as_mut()).filter(|dialog| dialog.hwnd == hwnd) {
            dialog.scale = scale;
        }
    }

    /// A component put on an edge by hand: the layout's step done, where it shows.
    pub(super) fn guide_saw_layout(&mut self) {
        if let Some(guide) = self.guide.as_mut().filter(|guide| guide.step == Step::Layout) {
            guide.done[1] = true;
        }
    }

    /// Where the guide's play on the layout page has the pointer, while it plays.
    pub(super) fn played_hand(&self) -> Option<(f32, f32)> {
        self.guide.as_ref()?.drag.as_ref()?.pointer
    }

    /// The pointer of the guide's play on the layout page, over all the window shows.
    pub(super) fn paint_played_pointer(&self, frame: &Frame) {
        if let Some(at) = self.played_hand() {
            paint_pointer(frame, at, 20.0);
        }
    }

    /// Where `spot` is in the window (DIPs), as last drawn: none, not drawn (yet).
    fn spot_at(&self, drag: &Drag, spot: Spot) -> Option<(f32, f32)> {
        let zone = |edge: Edge| self.zones.iter().find(|zone| zone.screen == drag.screen && zone.edge == edge).map(|zone| zone.rect);
        match spot {
            Spot::Rest => self.editor_at.map(|r| (r.x + r.w * 0.6, r.y + r.h - 30.0)),
            Spot::Tile(part) => self.targets.iter().find(|(_, target)| *target == Target::Library(part)).map(|(r, _)| (r.x + r.w / 2.0, r.y + r.h * 0.4)),
            Spot::Edge => zone(Edge::Left).map(|r| (r.x + r.w / 2.0, r.y + r.h / 2.0)),
            Spot::Pill => self.targets.iter().find(|(_, target)| matches!(target, Target::Chip(screen, Edge::Left, _) if *screen == drag.screen)).map(|(r, _)| (r.x + r.w * 0.35, r.y + r.h / 2.0)),
            Spot::Middle => {
                let (left, right) = (zone(Edge::Left)?, zone(Edge::Right)?);
                Some(((left.x + right.x + right.w) / 2.0, left.y + left.h / 2.0))
            }
        }
    }

    /// The guide's play on the layout page moved on to `now`: the pointer where its move has it, what the moves passed
    /// do done (a component taken up, or let go on an edge or off it), as the window was last drawn.
    pub(super) fn play_drag(&mut self, now: Instant) {
        let Some(mut drag) = self.guide.as_mut().and_then(|guide| guide.drag.take()) else { return };
        // First shown at the foot of the editor, a moment before it plays.
        if drag.pointer.is_none() {
            if let Some(rest) = self.spot_at(&drag, Spot::Rest) {
                drag.pointer = Some(rest);
                drag.from = Some(rest);
                drag.since = now + Duration::from_millis(500);
            }
        }
        while drag.pointer.is_some() {
            let made = drag.moves[drag.at];
            // (Bound for the pill: the panel its edge shows put away, the pill there in its place.)
            if made.to == Spot::Pill {
                self.shown_edge = None;
            }
            let Some(to) = self.spot_at(&drag, made.to) else { break };
            let ms = now.saturating_duration_since(drag.since).as_millis() as f32;
            let from = drag.from.unwrap_or(to);
            let along = if made.glide == 0 { 1.0 } else { GLIDE.at((ms / made.glide as f32).clamp(0.0, 1.0)) };
            drag.pointer = Some((from.0 + (to.0 - from.0) * along, from.1 + (to.1 - from.1) * along));
            if ms < (made.glide + made.wait) as f32 {
                break;
            }
            match made.act {
                Act::Take(part, off_edge) => {
                    self.carry = Some(Carry { part, from: off_edge.then(|| (drag.screen.clone(), Edge::Left)), start: to, moved: true });
                    self.shown_edge = None;
                }
                Act::Put => {
                    if let Some(carry) = self.carry.take() {
                        self.put(carry, Some(to));
                    }
                }
                Act::Wait => {}
            }
            drag.from = Some(to);
            drag.since += Duration::from_millis(made.glide + made.wait);
            drag.at = (drag.at + 1) % drag.moves.len();
        }
        if let Some(guide) = self.guide.as_mut() {
            guide.drag = Some(drag);
        }
    }

    /// The guide's play on the layout page stopped, where it plays: what it carried put away, the edges as they were
    /// before it.
    pub(super) fn stop_drag(&mut self) {
        let Some(drag) = self.guide.as_mut().and_then(|guide| guide.drag.take()) else { return };
        *self.settings.places_mut() = drag.before;
        self.carry = None;
        self.shown_edge = None;
        self.next_frame = Instant::now();
    }
}

/// Search's sheet as the last step's play shows it, `size` large, its corner at the frame's origin: what is typed, and
/// the files found, the first picked.
fn paint_search(frame: &Frame, theme: &Theme, example: &Example, size: (f32, f32)) {
    let family = theme.body.family;
    let typed = Font::new(family, 22.0, 400.0);
    frame.icon(Icon::Search, Point { x: 30.0, y: 34.0 }, 20.0, theme.text2);
    text_centred(frame, example.typed, typed, theme.text, 54.0, 34.0, size.0 - 80.0, Align::Start);
    // (Its caret blinking, as a caret does.)
    if SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() % 1060 < 530 {
        let end = 54.0 + frame.gfx.measure(example.typed, typed) + 2.0;
        frame.fill(theme.text, end, 22.0, 1.5, 24.0);
    }
    frame.fill(theme.rule, 0.0, 66.0, size.0, 1.0);
    frame.text(example.group, Font::new(family, 12.0, 400.0), theme.text2, 22.0, 78.0, size.0 - 44.0, Align::Start);
    for (at, (name, folder, kind, colour)) in example.files.iter().enumerate() {
        let y = 100.0 + at as f32 * 60.0;
        if at == 0 {
            frame.fill_rounded(theme.text.alpha(0.1), 8.0, y, size.0 - 16.0, 56.0, 8.0);
        }
        // The file's icon: a page, its kind on a band of its colour.
        let page = Rect { x: 24.0, y: y + 10.0, w: 28.0, h: 36.0 };
        frame.fill_rounded(Color::hex(0xFFFFFF, 1.0), page.x, page.y, page.w, page.h, 3.0);
        frame.fill(Color::hex(*colour, 1.0), page.x, page.y + 20.0, page.w, 11.0);
        centred(frame, kind, Font::new(family, 7.0, 800.0), Color::hex(0xFFFFFF, 1.0), Rect { x: page.x, y: page.y + 20.0, w: page.w, h: 11.0 });
        frame.text(name, Font::new(family, 14.0, 400.0), theme.text, 66.0, y + 9.0, size.0 - 90.0, Align::Start);
        frame.text(folder, Font::new(family, 12.0, 400.0), theme.text2, 66.0, y + 30.0, size.0 - 90.0, Align::Start);
    }
}

/// The look at the first file found as the last step's play shows it, `size` large, its corner at the frame's origin:
/// its name and size, and the page it shows.
fn paint_look(frame: &Frame, theme: &Theme, example: &Example, size: (f32, f32)) {
    let family = theme.body.family;
    let name = example.files[0].0;
    let bold = Font::new(family, 14.0, 600.0);
    frame.text(name, bold, theme.text, 16.0, 14.0, size.0 - 32.0, Align::Start);
    let after = 16.0 + frame.gfx.measure(name, bold) + 10.0;
    frame.text(example.size, Font::new(family, 12.0, 400.0), theme.text2, after, 16.0, (size.0 - after - 16.0).max(0.0), Align::Start);
    let page = Rect { x: 12.0, y: 44.0, w: size.0 - 24.0, h: size.1 - 56.0 };
    frame.fill_rounded(Color::hex(0xFBFBF9, 1.0), page.x, page.y, page.w, page.h, 6.0);
    let (ink, rule) = (Color::hex(0x22262C, 1.0), Color::hex(0xE5E5E2, 1.0));
    frame.text(example.heading, Font::new(family, 17.0, 700.0), Color::hex(0x173A6B, 1.0), page.x + 22.0, page.y + 22.0, page.w - 44.0, Align::Start);
    let line = Font::new(family, 13.0, 400.0);
    for (at, (what, formula)) in example.lines.iter().enumerate() {
        let y = page.y + 66.0 + at as f32 * 40.0;
        frame.text(what, line, ink, page.x + 22.0, y, page.w / 2.0, Align::Start);
        frame.text(formula, line, ink, page.x + 22.0, y, page.w - 44.0, Align::End);
        frame.fill(rule, page.x + 22.0, y + 28.0, page.w - 44.0, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first step's play runs on round with nothing jumping: the pointer and the panel move a little from one
    /// frame to the next, across where it begins again too; the pointer reaches the edge, the panel slides all the way
    /// out, and both are back where they began.
    #[test]
    fn the_push_s_play_never_jumps() {
        let (mut along_most, mut out_most) = (0.0f32, 0.0f32);
        let mut was = pushing(Duration::ZERO);
        assert_eq!(was, (0.0, 0.0));
        for ms in (8..=2 * PUSH_LOOP).step_by(8) {
            let now = pushing(Duration::from_millis(ms));
            assert!((now.0 - was.0).abs() < 0.05 && (now.1 - was.1).abs() < 0.12, "{ms} ms: {was:?} to {now:?}");
            (along_most, out_most) = (along_most.max(now.0), out_most.max(now.1));
            was = now;
        }
        assert!(along_most > 0.999 && out_most >= 1.0, "{along_most} {out_most}");
    }

    /// The last step's play presses each shortcut's keys one after another and lets them go, search shows as the first
    /// is pressed and the look only while search does; it ends as it begins, nothing down and nothing up.
    #[test]
    fn the_keys_play_presses_each_shortcut_then_lets_go() {
        for keys in [(3, 2), (2, 1), (1, 3)] {
            let keying = Keying::new(keys);
            let (mut most, mut searched, mut looked) = ([0, 0], false, false);
            for ms in (0..keying.end as u64 + 200).step_by(5) {
                let (down, search, look) = keying.at(Duration::from_millis(ms));
                most = [most[0].max(down[0]), most[1].max(down[1])];
                assert!(down[0] <= keys.0 && down[1] <= keys.1);
                assert!(look <= 0.0 || search >= 1.0, "{ms} ms: the look without search ({search})");
                searched |= search >= 1.0;
                looked |= look >= 1.0;
            }
            assert_eq!(most, [keys.0, keys.1]);
            assert!(searched && looked);
            assert_eq!(keying.at(Duration::ZERO), ([0, 0], 0.0, 0.0));
            assert_eq!(keying.at(Duration::from_millis(keying.end as u64 - 1)), ([0, 0], 0.0, 0.0));
        }
    }

    /// The layout page's play, played round as the window would play it, leaves every edge as it found it, whatever
    /// the edge it plays on held; and each round begins with the pointer gliding on from where the last left it.
    #[test]
    fn the_layout_s_play_leaves_the_edges_as_they_were() {
        let screen = "screen";
        for held in [Some(Part::Workbench), Some(Part::Music), None] {
            let mut places = Places::default();
            match held {
                Some(part) => places.put(screen, Edge::Left, part, None),
                None => places.take(screen, Edge::Left),
            }
            let before: Vec<Option<Part>> = EDGES.iter().map(|&edge| places.part(screen, edge)).collect();
            let put = played_part(held);
            assert_ne!(Some(put), held);
            let moves = script(put, held);
            assert!(moves[0].glide > 0, "a round begins gliding on");
            let mut carried: Option<(Part, bool)> = None;
            let mut put_down = false;
            for made in &moves {
                match made.act {
                    Act::Take(part, off_edge) => carried = Some((part, off_edge)),
                    Act::Put => {
                        let (part, off_edge) = carried.take().expect("carrying");
                        let from = off_edge.then_some((screen, Edge::Left));
                        match made.to {
                            Spot::Edge => places.put(screen, Edge::Left, part, from),
                            _ => places.take(screen, Edge::Left),
                        }
                        put_down |= places.part(screen, Edge::Left) == Some(put);
                    }
                    Act::Wait => {}
                }
            }
            assert!(put_down, "it shows the component put on the edge");
            assert!(carried.is_none());
            assert_eq!(EDGES.iter().map(|&edge| places.part(screen, edge)).collect::<Vec<_>>(), before, "held {held:?}");
        }
    }
}
