//! The settings window: the choices on the left, and on the right the panel
//! as it will look, over the desktop the window was opened on, running live.
//!
//! The window lives on the panel's thread and draws with its device. Its rows
//! are laid out and drawn from the settings on every frame; what moves (a
//! choice's thumb, a switch's knob, rows making way for a dragged one) is
//! kept from frame to frame as transitions.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::f32::consts::E;
use std::rc::Rc;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::core::{w, BOOL, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::D2D1_GRADIENT_STOP;
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap1, ID2D1RenderTarget, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_ELLIPSE, D2D1_EXTEND_MODE_CLAMP,
    D2D1_GAMMA_2_2, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_LAYER_PARAMETERS1, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES,
    D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::Dwm::{
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMSBT_MAINWINDOW, DWMWA_SYSTEMBACKDROP_TYPE,
    DWMWA_USE_IMMERSIVE_DARK_MODE,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{MARGINS, WM_MOUSELEAVE};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VIRTUAL_KEY, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_MENU,
    VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, GetSystemMetrics, LoadCursorW, LoadImageW,
    MessageBoxW, PostMessageW, RegisterClassExW, SetCursor, SetForegroundWindow, MB_ICONINFORMATION, MB_OK, WM_CLOSE,
    WM_KEYDOWN, WM_SYSKEYDOWN,
    SetWindowPos, SetWindowTextW, ShowWindow, HICON, IDC_ARROW, IDC_HAND, IMAGE_ICON, LR_SHARED, MINMAXINFO,
    SM_CXICON, SM_CXSMICON, SW_RESTORE, SW_SHOW, SW_SHOWNORMAL, SWP_NOACTIVATE, SWP_NOZORDER, WM_DESTROY,
    WM_DPICHANGED, WM_GETMINMAXINFO, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_SETCURSOR, WM_SIZE, WNDCLASSEXW, WS_EX_NOREDIRECTIONBITMAP, WS_OVERLAPPEDWINDOW,
};
use windows_numerics::{Matrix3x2, Vector2};

use super::backdrop::Capture;
use super::canvas::{Align, Color, Family, Font};
use super::gfx::{self, rect, Frame, Gfx, Surface};
use super::motion::{Easing, Transition};
use super::prefs::{self, LanguagePref, Prefs, ProcessSort, ThemePref};
use super::arrange::{self, GAP};
use super::render::{self, PanelLayers};
use super::skins;
use super::text::Lang;
use super::theme::{self, Shadow, Skin, Theme};
use super::view::{self, Rect, Scene};
use super::wallpaper;
use crate::elevation;
use crate::metrics;
use crate::settings::{Anchor, Edge, Sensitivity, Settings};
use crate::update;

/// The window's size, and the least it can be resized to (DIPs).
const SIZE: (f32, f32) = (1120.0, 760.0);
const MIN_SIZE: (f32, f32) = (900.0, 560.0);
/// The first Windows 11 build that lets a window ask for Mica.
const FIRST_MICA_BUILD: u32 = 22621;

/// The list of choices: its width, its padding (top, sides, bottom), and
/// the thin scroll bar's gutter.
const PANE: f32 = 480.0;
const PAD_TOP: f32 = 28.0;
const PAD_SIDE: f32 = 36.0;
const PAD_BOTTOM: f32 = 48.0;
const GUTTER: f32 = 8.0;
/// Rows: their least height, sides, the space between rows of a group, and
/// before a group's heading.
const ROW: f32 = 56.0;
const ROW_SIDE: f32 = 20.0;
const ROW_GAP: f32 = 4.0;
/// A row inside a module's open card, and how far in from the card's edge
/// its label starts: under the module's name, past the grip.
const ITEM_ROW: f32 = 48.0;
const ITEM_INSET: f32 = 12.0 + 20.0 + 12.0;
/// Room at the right of a module's row for its chevron, after its switch.
const CHEVRON_ROOM: f32 = 28.0;
const GROUP_GAP: f32 = 32.0;
const SKINS_ROW: f32 = 16.0 + 72.0 + 10.0 + 18.0 + 16.0;
/// Desktop shown beside the panel in the preview (DIPs of screen).
const PREVIEW_MARGIN: f32 = 160.0;
/// How far a wheel notch scrolls the choices, and how quickly they follow.
const WHEEL: f32 = 100.0;
const SCROLL_EASE: f32 = 0.06;
/// How the controls move: a thumb sliding to the chosen option, a switch's
/// knob, rows making way for a dragged one, and the dragged row settling.
const SLIDE: (Duration, Easing) = (Duration::from_millis(240), Easing(0.16, 1.0, 0.3, 1.0));
const FLIP: (Duration, Easing) = (Duration::from_millis(160), Easing(0.16, 1.0, 0.3, 1.0));
const GLIDE: (Duration, Easing) = (Duration::from_millis(200), Easing(0.16, 1.0, 0.3, 1.0));
const SETTLE: (Duration, Easing) = (Duration::from_millis(180), Easing(0.16, 1.0, 0.3, 1.0));
/// How soon to try again when drawing failed for want of a device.
const DEVICE_RETRY: Duration = Duration::from_millis(250);
/// The pen of the preview's charts runs this far behind the newest sample
/// beyond one interval, as the panel's does.
const PEN_LAG_MS: f64 = 100.0;

/// How long the diagnostics' button says they were copied.
const COPIED_FOR: Duration = Duration::from_secs(2);

/// The window, while it is open.
static WINDOW: AtomicIsize = AtomicIsize::new(0);

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

pub fn is_open() -> bool {
    WINDOW.load(Ordering::Relaxed) != 0
}

/// Brings the settings window up, from any thread: raised if it is open,
/// else made, by the panel's thread.
pub fn show() {
    let hwnd = WINDOW.load(Ordering::Acquire);
    if hwnd == 0 {
        crate::app().controller.open_settings();
        return;
    }
    let hwnd = HWND(hwnd as *mut _);
    unsafe {
        let _ = ShowWindow(hwnd, SW_RESTORE);
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Makes the window, centred on the monitor the pointer is on, on the
/// calling thread, which then carries it (see `tick`).
pub fn open() {
    if WINDOW.load(Ordering::Acquire) != 0 {
        return show();
    }
    let Ok(gfx) = gfx::current() else { return };
    if let Some(hwnd) = make(gfx) {
        WINDOW.store(hwnd.0 as isize, Ordering::Release);
    }
}

/// Draws the window if a frame is due, and says when the next one is; `None`
/// while there is no window.
pub fn tick(now: Instant) -> Option<Duration> {
    with_ui(|ui| {
        if ui.relabel_at.is_some_and(|at| now >= at) {
            ui.relabel();
        }
        if now >= ui.next_frame {
            ui.draw(now);
        }
        ui.next_frame.saturating_duration_since(now)
    })
}

fn windows_build() -> u32 {
    metrics::reg_string(w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"), w!("CurrentBuild")).parse().unwrap_or(0)
}

/// The window's own colours: Windows 11's, whatever skin the panel wears.
#[derive(Clone)]
struct Palette {
    text: Color,
    text2: Color,
    text3: Color,
    rule: Color,
    signal: Color,
    hover: Color,
    control: Color,
    control_on: Color,
    /// The chosen option's ring, and its drop shadow (none in dark).
    control_ring: Color,
    control_drop: Option<Color>,
    switch_off: Color,
    switch_stroke: Color,
    switch_knob: Color,
    window: Color,
    card: Color,
    card_stroke: Color,
    /// A switch that does nothing for now: its outline or track, and its
    /// knob when on.
    disabled: Color,
    disabled_knob: Color,
    /// The accent in the shade for this theme, and the deeper one switches
    /// take in either (the pale dark-mode shade cannot carry a white knob).
    selection: Color,
    switch_on: Color,
}

impl Palette {
    fn new(dark: bool) -> Self {
        let (on, off) = (theme::accent(false), theme::accent(dark));
        let white = |a| Color::hex(0xFFFFFF, a);
        let black = |a| Color::hex(0x000000, a);
        if dark {
            Palette {
                text: white(1.0),
                text2: white(0.76),
                text3: white(0.5),
                rule: white(0.08),
                signal: Color::hex(0xFF99A4, 1.0),
                hover: white(0.06),
                control: white(0.06),
                control_on: white(0.16),
                control_ring: white(0.06),
                control_drop: None,
                switch_off: black(0.1),
                switch_stroke: white(0.6),
                switch_knob: white(0.8),
                window: Color::hex(0x202020, 1.0),
                card: white(0.05),
                card_stroke: black(0.2),
                disabled: white(0.16),
                disabled_knob: white(0.53),
                selection: off,
                switch_on: on,
            }
        } else {
            Palette {
                text: black(0.9),
                text2: black(0.6),
                text3: black(0.44),
                rule: black(0.08),
                signal: Color::hex(0xC42B1C, 1.0),
                hover: black(0.04),
                control: black(0.05),
                control_on: white(0.95),
                control_ring: black(0.06),
                control_drop: Some(black(0.08)),
                switch_off: black(0.03),
                switch_stroke: black(0.6),
                switch_knob: black(0.6),
                window: Color::hex(0xF3F3F3, 1.0),
                card: white(0.7),
                card_stroke: black(0.06),
                disabled: black(0.22),
                disabled_knob: white(1.0),
                selection: on,
                switch_on: on,
            }
        }
    }
}

/// The choices of module `id`'s own, shown in its card.
fn module_fields(id: &str) -> &'static [Field] {
    match id {
        "network" => &[Field::RateUnit],
        "processes" => &[Field::ProcessCount, Field::ProcessSort],
        _ => &[],
    }
}

/// A row of choices, one of which is picked.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Field {
    Theme,
    Language,
    Edge,
    Anchor,
    Columns,
    Push,
    CloseDelay,
    RateUnit,
    ProcessCount,
    ProcessSort,
    Interval,
    Span,
    LoadAlert,
    TempAlert,
}

/// A row that is on or off.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Switch {
    Live,
    Startup,
    Updates,
    Hotkey,
    HeatAlert,
}

/// What a press lands on.
#[derive(Clone, PartialEq, Debug)]
enum Target {
    Skin(Skin),
    Choice(Field, usize),
    Switch(Switch),
    /// A module's switch, and the rest of its row, which opens and closes
    /// its card.
    Module(String),
    Expand(String),
    /// One of a module's items.
    Item(String, &'static str),
    Grip(String),
    Diagnostics,
    Update,
    Uninstall,
    Quit,
}

enum Row {
    Title,
    Heading(&'static str, &'static str),
    Skins,
    Choice(Field),
    Switch(Switch),
    Module(String),
    /// In a module's open card: one of its items, or a choice of its own.
    Item(String, &'static str),
    ModuleChoice(String, Field),
    Diagnostics,
    Update,
    Uninstall,
    Quit,
}

/// A module row being dragged: by how far below the row's top it was
/// taken, and where the pointer is (document DIPs).
struct Drag {
    id: String,
    grab: f32,
    pointer: f32,
}

/// The desktop the preview shows, at the size of the work area (DIPs).
struct Stage {
    size: (f32, f32),
    desktop: Capture,
    /// The desktop as a bitmap, for the zoom it was made at.
    bitmap: Option<(u32, ID2D1Bitmap1)>,
}

struct Ui {
    hwnd: HWND,
    gfx: Rc<Gfx>,
    /// Out of the struct while a frame is drawn on it.
    surface: Option<Surface>,
    mica: bool,
    /// Physical pixels per DIP.
    scale: f32,
    settings: Settings,
    prefs: Prefs,
    lang: Lang,
    dark: bool,
    palette: Palette,
    autostart: bool,
    /// Whether this copy may start with Windows (see elevation.rs).
    may_autostart: bool,
    /// The installer's uninstaller beside this executable, if it was
    /// installed somewhere ordinary programs cannot change.
    uninstaller: Option<std::path::PathBuf>,
    /// When the diagnostics were last copied.
    copied_at: Option<Instant>,
    scroll: f32,
    scroll_target: f32,
    motion: HashMap<String, Transition>,
    /// Thumbs land where they belong without sliding (after relabelling).
    settle_thumbs: bool,
    pointer: Option<(f32, f32)>,
    pressed: Option<Target>,
    /// The control the keyboard acts on, and whether the keyboard has been
    /// used since the last click (the focus is shown only then).
    focus: Option<Target>,
    keyboard: bool,
    drag: Option<Drag>,
    /// The modules whose cards are open (all closed as the window opens).
    expanded: HashSet<String>,
    targets: Vec<(Rect, Target)>,
    relabel_at: Option<Instant>,
    stage: Stage,
    layers: PanelLayers,
    last_frame: Instant,
    next_frame: Instant,
    /// Drawing the preview failed this frame (the device was lost).
    failed: bool,
}

/// Makes the window, draws its first frame, and shows it.
fn make(gfx: Rc<Gfx>) -> Option<HWND> {
    let (work, scale) = crate::panel::work_area_at_cursor()?;
    let scale = scale as f32;
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let icon = |size| LoadImageW(Some(instance.into()), PCWSTR(1 as _), IMAGE_ICON, size, size, LR_SHARED).ok().map(|h| HICON(h.0));
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(procedure),
            hInstance: instance.into(),
            lpszClassName: w!("GlanceSettings"),
            hCursor: LoadCursorW(None, IDC_ARROW).ok()?,
            hIcon: icon(GetSystemMetrics(SM_CXICON)).unwrap_or_default(),
            hIconSm: icon(GetSystemMetrics(SM_CXSMICON)).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassExW(&class);
        // Sized and centred in physical pixels of the monitor it opens on.
        let (work_w, work_h) = (work.right - work.left, work.bottom - work.top);
        let width = ((SIZE.0 * scale) as i32).min(work_w * 9 / 10);
        let height = ((SIZE.1 * scale) as i32).min(work_h * 9 / 10);
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP,
            w!("GlanceSettings"),
            w!("Glance"),
            WS_OVERLAPPEDWINDOW,
            work.left + (work_w - width) / 2,
            work.top + (work_h - height) / 2,
            width,
            height,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .ok()?;
        // Mica, the Windows 11 window material, where the system has it: the
        // window's content leaves it showing through.
        let mica = windows_build() >= FIRST_MICA_BUILD;
        if mica {
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &DWMSBT_MAINWINDOW as *const _ as *const _, 4);
            let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 });
        }
        let surface = Some(Surface::new(&gfx, hwnd).ok()?);
        let app = crate::app();
        let settings = app.settings.lock().unwrap().clone();
        let prefs = Prefs::resolve(&settings.view, &app.controller.known_modules());
        let size = ((work_w as f32 / scale).round(), (work_h as f32 / scale).round());
        let stage = Stage { size, desktop: wallpaper::desktop(size.0 as u32, size.1 as u32), bitmap: None };
        let window_scale = GetDpiForWindow(hwnd) as f32 / 96.0;
        let mut ui = Ui {
            hwnd,
            gfx,
            surface,
            mica,
            scale: window_scale,
            lang: Lang::resolve(prefs.language),
            dark: false,
            palette: Palette::new(false),
            settings,
            prefs,
            autostart: elevation::autostart_enabled(),
            may_autostart: elevation::may_start_unasked(),
            // Run elevated, so only where no ordinary program can change it.
            copied_at: None,
            uninstaller: std::env::current_exe().ok().and_then(|exe| elevation::trusted(&exe.with_file_name("uninstall.exe"))),
            scroll: 0.0,
            scroll_target: 0.0,
            motion: HashMap::new(),
            settle_thumbs: true,
            pointer: None,
            pressed: None,
            focus: None,
            keyboard: false,
            drag: None,
            expanded: HashSet::new(),
            targets: Vec::new(),
            relabel_at: None,
            stage,
            layers: PanelLayers::default(),
            last_frame: Instant::now(),
            next_frame: Instant::now(),
            failed: false,
        };
        ui.restyle();
        ui.draw(Instant::now());
        UI.with(|cell| *cell.borrow_mut() = Some(ui));
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        Some(hwnd)
    }
}

/// Runs `act` on the window's state, unless it is already in use further up
/// this thread's stack (a message sent while handling another).
fn with_ui<R>(act: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    UI.with(|cell| cell.try_borrow_mut().ok().and_then(|mut ui| ui.as_mut().map(act)))
}

fn point(lparam: LPARAM) -> (i32, i32) {
    ((lparam.0 & 0xFFFF) as u16 as i16 as i32, ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32)
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_GETMINMAXINFO => {
            let scale = unsafe { GetDpiForWindow(hwnd) } as f32 / 96.0;
            let info = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
            info.ptMinTrackSize = POINT { x: (MIN_SIZE.0 * scale) as i32, y: (MIN_SIZE.1 * scale) as i32 };
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let suggested = unsafe { &*(lparam.0 as *const RECT) };
            with_ui(|ui| ui.scale = ((wparam.0 & 0xFFFF) as f32) / 96.0);
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            LRESULT(0)
        }
        // Drawn at once, so the content keeps up while the window is resized.
        WM_SIZE => {
            with_ui(|ui| ui.draw(Instant::now()));
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, y) = point(lparam);
            with_ui(|ui| {
                if ui.pointer.is_none() {
                    let mut track = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    let _ = unsafe { TrackMouseEvent(&mut track) };
                }
                ui.moved(x as f32 / ui.scale, y as f32 / ui.scale);
            });
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_ui(|ui| {
                ui.pointer = None;
                ui.next_frame = Instant::now();
            });
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = point(lparam);
            with_ui(|ui| ui.press(x as f32 / ui.scale, y as f32 / ui.scale));
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = point(lparam);
            with_ui(|ui| ui.release(x as f32 / ui.scale, y as f32 / ui.scale));
            LRESULT(0)
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
            let alt = unsafe { GetKeyState(VK_MENU.0 as i32) } < 0;
            // Bit 30: the key was already down (an auto-repeat).
            let repeat = lparam.0 & (1 << 30) != 0;
            match with_ui(|ui| ui.key(VIRTUAL_KEY(wparam.0 as u16), shift, alt, repeat)) {
                Some(true) => LRESULT(0),
                _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
            }
        }
        WM_MOUSEWHEEL => {
            let delta = (wparam.0 >> 16) as u16 as i16;
            with_ui(|ui| ui.wheel(delta));
            LRESULT(0)
        }
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == 1 => {
            // Over the client area: a hand on what can be dragged.
            let grip = with_ui(|ui| matches!(ui.hovered(), Some(Target::Grip(_))) || ui.drag.is_some()).unwrap_or(false);
            if grip {
                unsafe { SetCursor(LoadCursorW(None, IDC_HAND).ok()) };
                return LRESULT(1);
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        // Closed: everything it drew with is let go; the device stays with
        // the panel.
        WM_DESTROY => {
            // Its surfaces and bitmaps go first; then the device gives back
            // what they held.
            let ui = UI.with(|cell| cell.borrow_mut().take());
            if let Some(ui) = ui {
                let gfx = ui.gfx.clone();
                drop(ui);
                gfx.trim();
            }
            WINDOW.store(0, Ordering::Release);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// The text of the window in both languages.
fn pick(lang: Lang, zh: &'static str, en: &'static str) -> &'static str {
    lang.pick(zh, en)
}

impl Ui {
    /// Follows the theme and the language now chosen.
    fn restyle(&mut self) {
        // Following the backdrop is the panel's; the window follows the system then.
        self.dark = match self.prefs.theme {
            ThemePref::Light => false,
            ThemePref::Dark => true,
            ThemePref::System | ThemePref::Backdrop => crate::os::apps_dark(),
        };
        self.palette = Palette::new(self.dark);
        unsafe {
            let dark = BOOL::from(self.dark);
            let _ = DwmSetWindowAttribute(self.hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as *const _, 4);
            let _ = SetWindowTextW(self.hwnd, &HSTRING::from(pick(self.lang, "Glance 设置", "Glance Settings")));
        }
        self.next_frame = Instant::now();
    }

    /// Takes the language now chosen, once its thumb has slid there.
    fn relabel(&mut self) {
        self.relabel_at = None;
        self.lang = Lang::resolve(self.prefs.language);
        self.settle_thumbs = true;
        self.restyle();
    }

    /// Keeps the settings and hands them to the panel.
    fn save(&mut self) {
        self.settings.view = serde_json::to_value(&self.prefs).unwrap();
        crate::app().save(self.settings.clone());
        self.next_frame = Instant::now();
    }

    fn rows(&self) -> Vec<Row> {
        let mut rows = vec![
            Row::Title,
            Row::Heading("外观", "Appearance"),
            Row::Skins,
            Row::Choice(Field::Theme),
            Row::Switch(Switch::Live),
            Row::Choice(Field::Language),
            Row::Heading("呼出", "Opening"),
            Row::Choice(Field::Edge),
            Row::Choice(Field::Anchor),
            Row::Choice(Field::Columns),
            Row::Choice(Field::Push),
            Row::Choice(Field::CloseDelay),
            Row::Switch(Switch::Hotkey),
            Row::Heading("显示内容", "Shown"),
        ];
        for entry in &self.prefs.modules {
            rows.push(Row::Module(entry.id.clone()));
            // A card's rows, while it is open or still closing.
            if self.opened(&entry.id) > 0.0 {
                rows.extend(self.items_here(&entry.id).into_iter().map(|name| Row::Item(entry.id.clone(), name)));
                rows.extend(module_fields(&entry.id).iter().map(|field| Row::ModuleChoice(entry.id.clone(), *field)));
            }
        }
        rows.extend([
            Row::Heading("数据", "Data"),
            Row::Choice(Field::Interval),
            Row::Choice(Field::Span),
            Row::Choice(Field::LoadAlert),
            Row::Choice(Field::TempAlert),
            Row::Switch(Switch::HeatAlert),
            Row::Heading("系统", "System"),
            Row::Switch(Switch::Startup),
            Row::Switch(Switch::Updates),
        ]);
        if update::available().is_some() {
            rows.push(Row::Update);
        }
        rows.push(Row::Diagnostics);
        if self.uninstaller.is_some() {
            rows.push(Row::Uninstall);
        }
        rows.push(Row::Quit);
        rows
    }

    /// Each row with its top and height, from the top of the list (DIPs).
    fn layout(&self) -> Vec<(Row, f32, f32)> {
        let mut y = PAD_TOP;
        let mut after_heading = false;
        let mut placed = Vec::new();
        // The card whose rows are being placed: its module, where its rows
        // start, and how open it is. Its rows keep their places inside it;
        // what follows it moves up by the part of them still hidden.
        let mut card: Option<(String, f32, f32)> = None;
        for row in self.rows() {
            let inside = match &row {
                Row::Item(id, _) | Row::ModuleChoice(id, _) => Some(id.clone()),
                _ => None,
            };
            if card.as_ref().is_some_and(|(id, ..)| Some(id) != inside.as_ref()) {
                let (_, start, open) = card.take().unwrap();
                y -= (1.0 - open) * (y - start);
            }
            if let Some(id) = inside.filter(|_| card.is_none()) {
                card = Some((id.clone(), y, self.opened(&id)));
            }
            // A heading's space above it takes in the title's below; the
            // first row of a group sits 8 under its heading, the rest 4 apart.
            let (before, height) = match &row {
                Row::Title => (0.0, 36.0),
                Row::Heading(..) => (GROUP_GAP, 20.0),
                Row::Skins => (if after_heading { 8.0 } else { ROW_GAP }, SKINS_ROW),
                // Inside its module's card, right under the row above.
                Row::Item(..) | Row::ModuleChoice(..) => (0.0, ITEM_ROW),
                _ => (if after_heading { 8.0 } else { ROW_GAP }, ROW),
            };
            y += before;
            after_heading = matches!(row, Row::Heading(..));
            placed.push((row, y, height));
            y += height;
        }
        if let Some((_, start, open)) = card {
            y -= (1.0 - open) * (y - start);
        }
        let _ = y;
        placed
    }

    /// How open module `id`'s card is: 0 closed, 1 open, between while it
    /// slides.
    fn opened(&self, id: &str) -> f32 {
        let open = if self.expanded.contains(id) { 1.0 } else { 0.0 };
        self.motion.get(&format!("open:{id}")).map_or(open, |t| t.value(Instant::now()).clamp(0.0, 1.0))
    }

    /// Opens or closes module `id`'s card, sliding unless `at_once`.
    fn set_open(&mut self, id: &str, open: bool, at_once: bool) {
        if open {
            self.expanded.insert(id.to_string());
        } else {
            self.expanded.remove(id);
        }
        self.animate(format!("open:{id}"), if open { 1.0 } else { 0.0 }, SLIDE, Instant::now(), at_once);
        self.next_frame = Instant::now();
    }

    fn content_height(&self) -> f32 {
        self.layout().last().map_or(0.0, |(_, y, h)| y + h) + PAD_BOTTOM
    }

    /// The labels of a row of choices, and which is chosen.
    fn choices(&self, field: Field) -> (&'static str, Vec<String>, Option<usize>) {
        let lang = self.lang;
        let s = |zh: &'static str, en: &'static str| lang.pick(zh, en).to_string();
        let seconds = |n: &str| if lang == Lang::Zh { format!("{n} 秒") } else { format!("{n} s") };
        let minutes = |n: &str| if lang == Lang::Zh { format!("{n} 分") } else { format!("{n} min") };
        let (settings, prefs) = (&self.settings, &self.prefs);
        let at = |values: &[u64], value: u64| values.iter().position(|&v| v == value);
        match field {
            Field::Theme => (
                pick(lang, "明暗", "Theme"),
                vec![s("跟随系统", "System"), s("浅色", "Light"), s("深色", "Dark"), s("跟随背景", "Backdrop")],
                [ThemePref::System, ThemePref::Light, ThemePref::Dark, ThemePref::Backdrop].iter().position(|&t| t == prefs.theme),
            ),
            Field::Language => (
                pick(lang, "语言", "Language"),
                vec![s("跟随系统", "System"), "中文".into(), "English".into()],
                [LanguagePref::System, LanguagePref::Zh, LanguagePref::En].iter().position(|&l| l == prefs.language),
            ),
            Field::Edge => (
                pick(lang, "屏幕边缘", "Screen edge"),
                vec![s("左侧", "Left"), s("顶部", "Top"), s("右侧", "Right")],
                [Edge::Left, Edge::Top, Edge::Right].iter().position(|&e| e == settings.edge),
            ),
            Field::Anchor => (
                pick(lang, "面板位置", "Position"),
                vec![s("跟随指针", "At pointer"), s("居中", "Centred")],
                [Anchor::Pointer, Anchor::Center].iter().position(|&a| a == settings.anchor),
            ),
            Field::Columns => (
                pick(lang, "面板栏数", "Columns"),
                vec![s("自动", "Auto"), "1".into(), "2".into(), "3".into(), "4".into()],
                [None, Some(1), Some(2), Some(3), Some(4)].iter().position(|&c| c == settings.columns),
            ),
            Field::Push => (
                pick(lang, "推入力度", "Push"),
                vec![s("轻", "Light"), s("中", "Medium"), s("重", "Firm")],
                [Sensitivity::Light, Sensitivity::Medium, Sensitivity::Firm].iter().position(|&p| p == settings.sensitivity),
            ),
            Field::CloseDelay => (
                pick(lang, "离开后收起", "Close after"),
                vec![s("立即", "At once"), seconds("0.5"), seconds("1")],
                at(&[200, 500, 1000], settings.close_delay_ms),
            ),
            Field::RateUnit => (pick(lang, "网速单位", "Network unit"), vec!["MB/s".into(), "Mbps".into()], Some(prefs.network.bits as usize)),
            Field::ProcessCount => (
                pick(lang, "进程显示行数", "Rows of processes"),
                vec!["5".into(), "8".into(), "12".into()],
                at(&[5, 8, 12], prefs.processes.count as u64),
            ),
            Field::ProcessSort => (
                pick(lang, "进程排序", "Sort processes by"),
                vec!["CPU".into(), s("内存", "Memory"), s("读写", "I/O"), "GPU".into()],
                [ProcessSort::Cpu, ProcessSort::Memory, ProcessSort::Io, ProcessSort::Gpu].iter().position(|&p| p == prefs.processes.sort),
            ),
            Field::Interval => (
                pick(lang, "刷新间隔", "Refresh every"),
                vec![seconds("0.5"), seconds("1"), seconds("2")],
                at(&[500, 1000, 2000], settings.interval_ms),
            ),
            Field::Span => (
                pick(lang, "曲线时长", "Graph spans"),
                vec![seconds("30"), minutes("1"), minutes("2"), minutes("5")],
                at(&[30, 60, 120, 300], prefs.chart_seconds as u64),
            ),
            Field::LoadAlert => (pick(lang, "负载警示", "Load alert"), vec!["70%".into(), "85%".into(), "95%".into()], at(&[70, 85, 95], prefs.hot_load as u64)),
            Field::TempAlert => (
                pick(lang, "温度警示", "Temperature alert"),
                vec!["75 °C".into(), "85 °C".into(), "95 °C".into()],
                at(&[75, 85, 95], prefs.hot_temp as u64),
            ),
        }
    }

    fn choose(&mut self, field: Field, index: usize) {
        let (settings, prefs) = (&mut self.settings, &mut self.prefs);
        match field {
            Field::Theme => prefs.theme = [ThemePref::System, ThemePref::Light, ThemePref::Dark, ThemePref::Backdrop][index],
            Field::Language => prefs.language = [LanguagePref::System, LanguagePref::Zh, LanguagePref::En][index],
            Field::Edge => settings.edge = [Edge::Left, Edge::Top, Edge::Right][index],
            Field::Anchor => settings.anchor = [Anchor::Pointer, Anchor::Center][index],
            Field::Columns => settings.columns = [None, Some(1), Some(2), Some(3), Some(4)][index],
            Field::Push => settings.sensitivity = [Sensitivity::Light, Sensitivity::Medium, Sensitivity::Firm][index],
            Field::CloseDelay => settings.close_delay_ms = [200, 500, 1000][index],
            Field::RateUnit => prefs.network.bits = index == 1,
            Field::ProcessCount => prefs.processes.count = [5, 8, 12][index],
            Field::ProcessSort => prefs.processes.sort = [ProcessSort::Cpu, ProcessSort::Memory, ProcessSort::Io, ProcessSort::Gpu][index],
            Field::Interval => settings.interval_ms = [500, 1000, 2000][index],
            Field::Span => prefs.chart_seconds = [30.0, 60.0, 120.0, 300.0][index],
            Field::LoadAlert => prefs.hot_load = [70.0, 85.0, 95.0][index],
            Field::TempAlert => prefs.hot_temp = [75.0, 85.0, 95.0][index],
        }
        self.save();
        match field {
            Field::Theme => self.restyle(),
            // Everything is relabelled in the new language once the thumb
            // has slid; mid-way it would jump.
            Field::Language => self.relabel_at = Some(Instant::now() + SLIDE.0),
            _ => {}
        }
    }

    /// A switch's label, its hint, and whether it is on.
    fn switch(&self, switch: Switch) -> (&'static str, Option<Cow<'static, str>>, bool) {
        let lang = self.lang;
        let p = |zh, en| pick(lang, zh, en);
        let (name, hint, on) = match switch {
            Switch::Live => (p("实时折射", "Live refraction"), Some(p("开启后截图里不会出现面板", "The panel then stays out of screenshots")), self.settings.live_backdrop),
            // Only a copy no ordinary program can replace (one the installer
            // put in Program Files) may start elevated unasked.
            Switch::Startup => (
                p("开机时启动", "Start with Windows"),
                (!self.may_autostart).then(|| {
                    p("当前安装位置不受保护，无法启用；点击查看原因", "Unavailable in this location; click for details")
                }),
                self.autostart,
            ),
            Switch::Hotkey => (
                p("快捷键 Ctrl+Alt+G", "Shortcut Ctrl+Alt+G"),
                Some(if crate::tray::hotkey_taken() {
                    p("已被其他程序占用，暂时无法使用", "Another program is using it")
                } else {
                    p("打开或收起面板，也可收起已固定的面板", "Opens and closes the panel, pinned or not")
                }),
                self.settings.hotkey,
            ),
            Switch::HeatAlert => (
                p("过热提醒", "Heat alert"),
                Some(p("达到温度警示值 30 秒后从托盘提醒", "A tray warning after 30 s at the alert")),
                self.settings.heat_alert,
            ),
            // The version this is, where the user looks for a newer one.
            Switch::Updates => {
                let version = env!("CARGO_PKG_VERSION");
                let hint = match lang {
                    Lang::Zh => format!("当前版本 {version}，每天检查一次"),
                    Lang::En => format!("Version {version}; checks once a day"),
                };
                return (p("检查更新", "Check for updates"), Some(Cow::Owned(hint)), self.settings.check_updates);
            }
        };
        (name, hint.map(Cow::Borrowed), on)
    }

    /// Why Start with Windows cannot be turned on for this copy, and how it can.
    fn explain_no_autostart(&self) {
        let place = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.display().to_string())).unwrap_or_default();
        let (title, text) = match self.lang {
            Lang::Zh => (
                "无法启用开机时启动".to_string(),
                format!(
                    "Glance 当前位于：\n{place}\n\n这个位置不受保护：普通程序可以修改它或它的上级文件夹，因此可能把 Glance 替换掉。开机时启动会让 Glance 不经确认就以管理员身份运行，只有在受保护的位置才能安全地这样做。\n\n如需启用，请使用安装程序将 Glance 安装到 Program Files，或磁盘根目录下的新文件夹（例如 D:\\Glance）。"
                ),
            ),
            Lang::En => (
                "Start with Windows is unavailable".to_string(),
                format!(
                    "Glance is currently located in:\n{place}\n\nThis location is not protected: ordinary programs can change it or a folder above it, and so could replace Glance. Starting with Windows runs Glance with administrator rights without asking, which is safe only from a protected location.\n\nTo turn it on, use the installer to install Glance to Program Files, or to a new folder at the root of a drive (for example D:\\Glance)."
                ),
            ),
        };
        unsafe { MessageBoxW(Some(self.hwnd), &HSTRING::from(text), &HSTRING::from(title), MB_OK | MB_ICONINFORMATION) };
    }

    fn flip(&mut self, switch: Switch) {
        let settings = &mut self.settings;
        match switch {
            Switch::Live => settings.live_backdrop ^= true,
            Switch::Updates => settings.check_updates ^= true,
            Switch::Hotkey => settings.hotkey ^= true,
            Switch::HeatAlert => settings.heat_alert ^= true,
            Switch::Startup if !self.may_autostart && !self.autostart => {
                self.explain_no_autostart();
                return;
            }
            Switch::Startup => {
                // What the system then reports, not what was asked.
                self.autostart = elevation::set_autostart(!self.autostart);
                self.next_frame = Instant::now();
                return;
            }
        }
        self.save();
    }

    /// A module's title and what it describes on this machine.
    fn module(&self, id: &str) -> (String, Option<String>) {
        let (lang, info) = (self.lang, &crate::app().info);
        let p = |zh, en| pick(lang, zh, en).to_string();
        match id {
            "cpu" => ("CPU".into(), Some(info.cpu_name.clone())),
            _ if id.starts_with("gpu:") => ("GPU".into(), info.gpu_of(id).map(|i| info.gpus[i].name.clone())),
            "memory" => (p("内存", "Memory"), info.memory_modules.clone()),
            "network" => (p("网络", "Network"), Some(info.network_adapter.clone().unwrap_or_else(|| p("所有物理网卡的上下行速度", "Traffic over every physical adapter")))),
            "disk" => (p("磁盘", "Disk"), (!info.drives.is_empty()).then(|| info.drives.join(", "))),
            "processes" => (p("进程", "Processes"), Some(p("占用最多的程序，点表头排序", "The busiest programs; click a heading to sort"))),
            "storage" => (p("存储", "Storage"), Some(p("各分区的空间", "Space on each drive"))),
            "board" => (p("主板", "Motherboard"), (!info.board.is_empty()).then(|| info.board.clone())),
            "battery" => (p("电池", "Battery"), Some(p("笔记本电脑", "Laptops"))),
            _ => (p("系统", "System"), Some(p("开机时长、进程和句柄数", "Uptime, processes, handles"))),
        }
    }

    /// Whether module `id` is on.
    fn module_on(&self, id: &str) -> bool {
        self.prefs.modules.iter().any(|entry| entry.id == id && entry.on)
    }

    /// The items of module `id` this machine has shown it can read: only
    /// those are listed.
    fn items_here(&self, id: &str) -> Vec<&'static str> {
        let app = crate::app();
        let seen = app.controller.seen.lock().unwrap();
        let info = &app.info;
        let gpu = info.gpu_of(id).map(|index| (&info.gpus[index], seen.gpus.get(index).cloned().unwrap_or_default()));
        // A module this machine has shown nothing of (no battery, no board
        // sensors, a GPU not read) has no lane, and nothing to set.
        let present = match id.split(':').next().unwrap_or(id) {
            "battery" => seen.battery,
            "board" => seen.board.is_some(),
            "gpu" => gpu.as_ref().is_some_and(|(_, seen)| seen.present),
            _ => true,
        };
        if !present {
            return Vec::new();
        }
        prefs::items(id)
            .iter()
            .map(|item| item.name)
            .filter(|name| match (id.split(':').next().unwrap_or(id), *name) {
                (_, "chart") => true,
                ("cpu", "temp") => seen.cpu_temp,
                ("cpu", "clock") => seen.cpu_clock,
                ("cpu", "power") => seen.cpu_power,
                ("cpu", "ccds") => seen.ccds.len() > 1,
                ("cpu", "threads") => seen.threads > 0,
                ("gpu", name) => gpu.as_ref().is_some_and(|(info, seen)| match name {
                    "temp" => seen.temp,
                    "vram" => info.mem_total > 0,
                    "clock" => seen.clock,
                    "power" => seen.power,
                    "fan" => seen.fan,
                    "shared" => info.shared_total > 0,
                    _ => !seen.engines.is_empty(),
                }),
                ("memory", "dimms") => seen.dimms > 0,
                ("network", "address") => seen.address,
                ("network", "link") => seen.link,
                ("disk", "drives") => !seen.drives.is_empty(),
                ("disk", "active") => seen.disk_active,
                ("board", "temps") => seen.board.as_ref().is_some_and(|board| !board.temps.is_empty()),
                ("board", "fans") => seen.board.as_ref().is_some_and(|board| !board.fans.is_empty()),
                _ => true,
            })
            .collect()
    }

    /// Whether module `id` has a card to open: items, or choices of its own.
    fn opens(&self, id: &str) -> bool {
        !module_fields(id).is_empty() || !self.items_here(id).is_empty()
    }

    /// What an item of module `id` is called, and a word on it.
    fn item_label(&self, id: &str, name: &str) -> (&'static str, Option<&'static str>) {
        let p = |zh, en| pick(self.lang, zh, en);
        match (id.split(':').next().unwrap_or(id), name) {
            ("network" | "disk", "chart") => (p("速率图表", "Rate chart"), None),
            ("battery", "chart") => (p("电量图表", "Charge chart"), None),
            (_, "chart") => (p("占用图表", "Usage chart"), Some(p("关闭后只显示数字，面板更紧凑", "Off, the figures alone: a more compact panel"))),
            ("board", "temps") => (p("温度传感器", "Temperature sensors"), None),
            ("board", "fans") => (p("风扇", "Fans"), None),
            ("system", "uptime") => (p("开机时长", "Uptime"), None),
            ("system", "processes") => (p("进程数", "Processes"), None),
            ("system", "threads") => (p("线程数", "Threads"), None),
            ("system", "handles") => (p("句柄数", "Handles"), None),
            (_, "temp") => (p("温度", "Temperature"), None),
            (_, "clock") => (p("频率", "Clock"), None),
            (_, "power") => (p("功耗", "Power"), None),
            (_, "ccds") => (p("各 CCD 温度", "Each CCD's temperature"), None),
            (_, "threads") => (p("线程", "Threads"), Some(p("每个线程的占用", "Each thread's load"))),
            (_, "vram") => (p("显存", "Video memory"), None),
            (_, "fan") => (p("风扇", "Fan"), None),
            (_, "shared") => (p("共享显存", "Shared memory"), Some(p("从内存借用的部分", "Borrowed from system memory"))),
            (_, "engines") => (p("各引擎", "Engines"), Some(p("3D、复制、视频编解码", "3D, copy, video"))),
            (_, "dimms") => (p("内存条温度", "Module temperatures"), None),
            (_, "committed") => (p("已提交", "Committed"), None),
            (_, "cached") => (p("缓存", "Cached"), None),
            (_, "adapter") => (p("网卡名称", "Adapter"), None),
            (_, "address") => (p("地址", "Address"), None),
            (_, "link") => (p("链路速度", "Link speed"), None),
            (_, "totals") => (p("开机以来的流量", "Traffic since boot"), None),
            (_, "drives") => (p("各硬盘温度", "Drive temperatures"), None),
            (_, "active") => (p("活动时间", "Active time"), None),
            _ => (p("其他", "Other"), None),
        }
    }

    // ---- Input ----

    fn hovered(&self) -> Option<Target> {
        let (x, y) = self.pointer?;
        self.targets.iter().find(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h).map(|(_, t)| t.clone())
    }

    fn moved(&mut self, x: f32, y: f32) {
        self.pointer = Some((x, y));
        if let Some(drag) = &mut self.drag {
            drag.pointer = y + self.scroll;
            self.reorder();
        }
        self.next_frame = Instant::now();
    }

    fn press(&mut self, x: f32, y: f32) {
        self.pointer = Some((x, y));
        self.pressed = self.hovered();
        // A press moves the focus there too, without showing it.
        if let Some(target) = &self.pressed {
            self.keyboard = false;
            self.focus = Some(self.stop_for(target));
        }
        if let Some(Target::Grip(id)) = &self.pressed {
            // A card travels closed.
            let id = id.clone();
            self.set_open(&id, false, true);
            let id = &id;
            let top = self.row_top(id);
            self.drag = Some(Drag { id: id.clone(), grab: y + self.scroll - top, pointer: y + self.scroll });
            unsafe { SetCapture(self.hwnd) };
        }
        self.next_frame = Instant::now();
    }

    fn release(&mut self, x: f32, y: f32) {
        self.pointer = Some((x, y));
        if let Some(drag) = self.drag.take() {
            unsafe {
                let _ = ReleaseCapture();
            }
            // The held row settles into its place.
            let slot = self.row_top(&drag.id);
            let now = Instant::now();
            let mut settle = Transition::settled(drag.pointer - drag.grab - slot);
            settle.retarget(0.0, SETTLE.0, SETTLE.1, now);
            self.motion.insert(format!("row:{}", drag.id), settle);
            self.pressed = None;
            self.save();
            return;
        }
        let pressed = self.pressed.take();
        if pressed.is_none() || pressed != self.hovered() {
            self.next_frame = Instant::now();
            return;
        }
        // The focus follows the choice a click made.
        let target = pressed.unwrap();
        self.activate(target.clone());
        self.focus = Some(self.stop_for(&target));
    }

    /// Does what pressing `target` does.
    fn activate(&mut self, target: Target) {
        match target {
            Target::Skin(skin) => {
                self.settings.skin = match skin {
                    Skin::Paper => "paper",
                    Skin::Glass => "glass",
                    Skin::Fluent => "fluent",
                }
                .into();
                self.save();
            }
            Target::Choice(field, index) => self.choose(field, index),
            Target::Switch(switch) => self.flip(switch),
            Target::Module(id) => {
                if let Some(entry) = self.prefs.modules.iter_mut().find(|entry| entry.id == id) {
                    entry.on ^= true;
                }
                self.save();
            }
            Target::Expand(id) => {
                let open = !self.expanded.contains(&id);
                self.set_open(&id, open, false);
            }
            // An item is changed only while its module is on.
            Target::Item(id, name) if self.module_on(&id) => {
                let on = self.prefs.shows(&id, name);
                self.prefs.set_item(&id, name, !on);
                self.save();
            }
            Target::Item(..) => {}
            // The uninstaller asks first and closes Glance itself.
            Target::Uninstall => {
                // Checked again as it is run: the window may have been open a while.
                if let Some(uninstaller) = self.uninstaller.as_deref().and_then(elevation::trusted) {
                    let path = HSTRING::from(uninstaller.as_os_str());
                    unsafe { ShellExecuteW(Some(self.hwnd), w!("open"), &path, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
                }
            }
            Target::Update => update::install(),
            Target::Diagnostics => {
                if crate::diagnostics::copy(self.hwnd, &crate::diagnostics::report()) {
                    self.copied_at = Some(Instant::now());
                }
            }
            Target::Quit => crate::quit(),
            Target::Grip(_) => {}
        }
        self.next_frame = Instant::now();
    }

    // ---- Keyboard ----

    /// The focus stop a target belongs to: a row of choices, or the skin
    /// cards, is one stop, at the option chosen.
    fn stop_for(&self, target: &Target) -> Target {
        match target {
            Target::Choice(field, _) => Target::Choice(*field, self.choices(*field).2.unwrap_or(0)),
            Target::Skin(_) => Target::Skin(Skin::named(&self.settings.skin)),
            Target::Grip(id) => Target::Module(id.clone()),
            other => other.clone(),
        }
    }

    /// Every focus stop, in the order the list shows them (scrolled out of
    /// view or not).
    fn stops(&self) -> Vec<Target> {
        self.layout()
            .into_iter()
            .flat_map(|(row, ..)| match row {
                Row::Skins => vec![Target::Skin(Skin::named(&self.settings.skin))],
                Row::Choice(field) => vec![Target::Choice(field, self.choices(field).2.unwrap_or(0))],
                Row::Switch(switch) => vec![Target::Switch(switch)],
                // A module's row, which opens its card if it has one, then its switch.
                Row::Module(id) if self.opens(&id) => vec![Target::Expand(id.clone()), Target::Module(id)],
                Row::Module(id) => vec![Target::Module(id)],
                Row::Item(id, name) if self.module_on(&id) && self.expanded.contains(&id) => vec![Target::Item(id, name)],
                Row::ModuleChoice(id, field) if self.module_on(&id) && self.expanded.contains(&id) => vec![Target::Choice(field, self.choices(field).2.unwrap_or(0))],
                Row::Item(..) | Row::ModuleChoice(..) => vec![],
                Row::Update => vec![Target::Update],
                Row::Diagnostics => vec![Target::Diagnostics],
                Row::Uninstall => vec![Target::Uninstall],
                Row::Quit => vec![Target::Quit],
                Row::Title | Row::Heading(..) => vec![],
            })
            .collect()
    }

    /// A key pressed (`repeat` when it is held down and auto-repeating): Tab
    /// and Shift+Tab move between controls, the arrows change a row's
    /// choice, Space and Enter press, Alt with the up and down arrows moves a
    /// module, Escape closes the window. Returns whether the key was used;
    /// any other combination with Alt is left to the system.
    fn key(&mut self, key: VIRTUAL_KEY, shift: bool, alt: bool, repeat: bool) -> bool {
        let moves_module = alt && (key == VK_UP || key == VK_DOWN) && matches!(self.focus, Some(Target::Module(_) | Target::Expand(_)));
        if alt && !moves_module {
            return false;
        }
        self.keyboard = true;
        self.next_frame = Instant::now();
        let stops = self.stops();
        let at = self.focus.as_ref().and_then(|focus| stops.iter().position(|stop| stop == focus));
        match key {
            VK_TAB if !stops.is_empty() => {
                let next = match (at, shift) {
                    (None, false) => 0,
                    (None, true) => stops.len() - 1,
                    (Some(i), false) => (i + 1) % stops.len(),
                    (Some(i), true) => (i + stops.len() - 1) % stops.len(),
                };
                self.focus = Some(stops[next].clone());
                self.reveal_focus();
            }
            VK_ESCAPE => unsafe {
                let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            },
            // Holding the key presses once.
            VK_SPACE | VK_RETURN if repeat => {}
            VK_SPACE | VK_RETURN => {
                if let Some(focus) = self.focus.clone() {
                    self.activate(focus);
                }
            }
            VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN => {
                let forward = key == VK_RIGHT || key == VK_DOWN;
                match self.focus.clone() {
                    Some(Target::Module(id) | Target::Expand(id)) if moves_module => self.shift_module(&id, forward),
                    Some(Target::Choice(field, index)) => {
                        let count = self.choices(field).1.len();
                        let next = if forward { (index + 1).min(count - 1) } else { index.saturating_sub(1) };
                        if next != index {
                            self.activate(Target::Choice(field, next));
                            self.focus = Some(Target::Choice(field, next));
                        }
                    }
                    Some(Target::Skin(skin)) => {
                        let order = [Skin::Paper, Skin::Glass, Skin::Fluent];
                        let index = order.iter().position(|s| *s == skin).unwrap_or(0);
                        let next = if forward { (index + 1).min(order.len() - 1) } else { index.saturating_sub(1) };
                        if next != index {
                            self.activate(Target::Skin(order[next]));
                            self.focus = Some(Target::Skin(order[next]));
                        }
                    }
                    // Elsewhere the up and down arrows scroll the list.
                    _ if key == VK_UP || key == VK_DOWN => self.scroll_by(if forward { WHEEL / 2.0 } else { -WHEEL / 2.0 }),
                    _ => return false,
                }
            }
            VK_PRIOR | VK_NEXT => {
                let page = self.client().1 * 0.8;
                self.scroll_by(if key == VK_NEXT { page } else { -page });
            }
            _ => return false,
        }
        true
    }

    /// Moves module `id` one place up or down the list.
    fn shift_module(&mut self, id: &str, down: bool) {
        let Some(at) = self.prefs.modules.iter().position(|entry| entry.id == id) else { return };
        let to = if down { at + 1 } else { at.wrapping_sub(1) };
        if to >= self.prefs.modules.len() {
            return;
        }
        let before = self.row_tops();
        self.prefs.modules.swap(at, to);
        self.glide_rows(before, "");
        self.save();
        self.reveal_focus();
    }

    fn scroll_by(&mut self, distance: f32) {
        let most = (self.content_height() - self.client().1).max(0.0);
        self.scroll_target = (self.scroll_target + distance).clamp(0.0, most);
    }

    /// Scrolls the list so the focused control is in view.
    fn reveal_focus(&mut self) {
        let Some(focus) = self.focus.clone() else { return };
        let row = self.layout().into_iter().find(|(row, ..)| match (row, &focus) {
            (Row::Skins, Target::Skin(_)) | (Row::Update, Target::Update) | (Row::Diagnostics, Target::Diagnostics) | (Row::Uninstall, Target::Uninstall) | (Row::Quit, Target::Quit) => true,
            (Row::Choice(f), Target::Choice(g, _)) => f == g,
            (Row::Switch(s), Target::Switch(t)) => s == t,
            (Row::Module(m), Target::Module(n) | Target::Expand(n)) => m == n,
            (Row::Item(m, a), Target::Item(n, b)) => m == n && a == b,
            (Row::ModuleChoice(_, f), Target::Choice(g, _)) => f == g,
            _ => false,
        });
        let Some((_, top, height)) = row else { return };
        let view = self.client().1;
        let margin = 24.0;
        let most = (self.content_height() - view).max(0.0);
        if top - margin < self.scroll_target {
            self.scroll_target = (top - margin).clamp(0.0, most);
        } else if top + height + margin > self.scroll_target + view {
            self.scroll_target = (top + height + margin - view).clamp(0.0, most);
        }
    }

    fn wheel(&mut self, delta: i16) {
        let Some((x, _)) = self.pointer else { return };
        if x >= PANE {
            return;
        }
        let height = self.client().1;
        let most = (self.content_height() - height).max(0.0);
        self.scroll_target = (self.scroll_target - delta as f32 / 120.0 * WHEEL).clamp(0.0, most);
        self.next_frame = Instant::now();
    }

    /// Where a module's row is laid out (document DIPs).
    fn row_top(&self, id: &str) -> f32 {
        self.layout().iter().find(|(row, ..)| matches!(row, Row::Module(m) if m == id)).map_or(0.0, |(_, y, _)| *y)
    }

    /// Moves the dragged row to where its middle now is; the rows it passes
    /// glide from where they were to where they now belong.
    fn reorder(&mut self) {
        let Some(drag) = &self.drag else { return };
        let middle = drag.pointer - drag.grab + ROW / 2.0;
        let before = self.row_tops();
        let held = self.prefs.modules.iter().position(|entry| entry.id == drag.id).unwrap();
        let entry = self.prefs.modules.remove(held);
        let next = self.prefs.modules.iter().position(|other| middle < before[&other.id] + ROW / 2.0).unwrap_or(self.prefs.modules.len());
        let id = entry.id.clone();
        self.prefs.modules.insert(next, entry);
        if next == held {
            return;
        }
        self.glide_rows(before, &id);
    }

    /// Where each module's row is laid out (document DIPs).
    fn row_tops(&self) -> HashMap<String, f32> {
        self.layout().into_iter().filter_map(|(row, y, _)| if let Row::Module(id) = row { Some((id, y)) } else { None }).collect()
    }

    /// After the modules' order changed: every row but `moved` glides from
    /// where it was (`before`) to where it now belongs.
    fn glide_rows(&mut self, before: HashMap<String, f32>, moved: &str) {
        let now = Instant::now();
        let after = self.row_tops();
        let id = moved.to_string();
        for (other, was) in before {
            if other == id || after[&other] == was {
                continue;
            }
            let key = format!("row:{other}");
            let current = self.motion.get(&key).map_or(0.0, |t| t.value(now));
            let mut glide = Transition::settled(was - after[&other] + current);
            glide.retarget(0.0, GLIDE.0, GLIDE.1, now);
            self.motion.insert(key, glide);
        }
    }

    fn client(&self) -> (f32, f32) {
        let mut rect = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rect);
        }
        ((rect.right - rect.left) as f32 / self.scale, (rect.bottom - rect.top) as f32 / self.scale)
    }

    /// A transition's value, made at `start` if it is new, and sent toward
    /// `to` if that changed.
    fn animate(&mut self, key: String, to: f32, how: (Duration, Easing), now: Instant, jump: bool) -> f32 {
        let transition = self.motion.entry(key).or_insert_with(|| Transition::settled(to));
        if jump {
            transition.jump(to);
        } else if transition.target() != to {
            transition.retarget(to, how.0, how.1, now);
        }
        transition.value(now)
    }

    // ---- Drawing ----

    /// Takes up the thread's current graphics device if it is a new one,
    /// making the window's surface and the preview's bitmaps again on it.
    fn follow_device(&mut self) -> bool {
        let Ok(current) = gfx::current() else { return false };
        if Rc::ptr_eq(&current, &self.gfx) && self.surface.is_some() {
            return true;
        }
        // A window has one composition target: the old one goes first.
        self.surface = None;
        let Ok(surface) = Surface::new(&current, self.hwnd) else {
            gfx::lost();
            return false;
        };
        self.surface = Some(surface);
        self.gfx = current;
        self.layers.release();
        self.stage.bitmap = None;
        true
    }

    fn draw(&mut self, now: Instant) {
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.scroll += (self.scroll_target - self.scroll) * (1.0 - E.powf(-dt / SCROLL_EASE));
        if (self.scroll_target - self.scroll).abs() < 0.05 {
            self.scroll = self.scroll_target;
        }
        let mut rect = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rect);
        }
        let size = ((rect.right - rect.left).max(1) as u32, (rect.bottom - rect.top).max(1) as u32);
        if !self.follow_device() {
            self.next_frame = now + DEVICE_RETRY;
            return;
        }
        let gfx = self.gfx.clone();
        let mut surface = self.surface.take().unwrap();
        let mut creep = None;
        self.failed = false;
        let drawn = surface.draw(&gfx, size, self.scale, |frame| {
            frame.origin(0.0, 0.0);
            creep = self.paint(frame, now);
        });
        self.surface = Some(surface);
        // The device is gone; a new one is made for the next frame.
        if drawn.is_err() || self.failed {
            gfx::lost();
            self.next_frame = now + DEVICE_RETRY;
            return;
        }
        self.gfx.sweep();
        let moving = self.motion.values().any(|t| !t.done(now)) || self.scroll != self.scroll_target || self.drag.is_some();
        self.settle_thumbs = false;
        self.next_frame = if moving {
            now
        } else {
            // Otherwise as often as the preview's charts creep a quarter pixel.
            now + creep.map_or(Duration::from_millis(250), |speed: f32| Duration::from_secs_f32((0.25 / speed).min(0.25)))
        };
    }

    /// Draws everything, and returns how fast the preview's charts move
    /// (physical pixels a second).
    fn paint(&mut self, frame: &Frame, now: Instant) -> Option<f32> {
        let palette = self.palette.clone();
        let (width, height) = self.client();
        if !self.mica {
            fill(frame, palette.window, 0.0, 0.0, width, height, 0.0);
        }
        self.targets.clear();
        unsafe { frame.dc.PushAxisAlignedClip(&rect(0.0, 0.0, PANE, height), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
        self.paint_list(frame, &palette, now, height);
        unsafe { frame.dc.PopAxisAlignedClip() };
        self.paint_preview(frame, &palette, width, height)
    }

    fn paint_list(&mut self, frame: &Frame, palette: &Palette, now: Instant, height: f32) {
        let lang = self.lang;
        let left = PAD_SIDE;
        let width = PANE - 2.0 * PAD_SIDE - GUTTER;
        let scroll = self.scroll;
        let hovered = self.hovered();
        let label = Font::new(Family::Segoe, 14.0, 400.0);
        let hint = Font::new(Family::Segoe, 12.0, 400.0);
        let mut dragged = None;
        let placed = self.layout();
        // How far each module's card reaches down: its row, and as it opens,
        // that share of the rows inside it; and where the card's top is.
        // A card's rows follow its module's.
        let mut reach: HashMap<String, f32> = HashMap::new();
        let mut card_top: HashMap<String, f32> = HashMap::new();
        for (row, top, row_height) in &placed {
            match row {
                Row::Module(id) => {
                    card_top.insert(id.clone(), *top);
                    reach.insert(id.clone(), *row_height);
                }
                Row::Item(id, _) | Row::ModuleChoice(id, _) => {
                    reach.insert(id.clone(), top + row_height - card_top[id]);
                }
                _ => {}
            }
        }
        let open: HashMap<String, f32> = card_top.keys().map(|id| (id.clone(), self.opened(id))).collect();
        for (id, extent) in reach.iter_mut() {
            *extent = ROW + (*extent - ROW) * open[id];
        }
        for (row, top, row_height) in placed {
            let y = top - scroll;
            let extent = if let Row::Module(id) = &row { reach[id] } else { row_height };
            if y > height || y + extent + 60.0 < 0.0 {
                continue;
            }
            // A row inside a card shows only within the card as far as it
            // has opened, and takes clicks once it is open.
            let inside = match &row {
                Row::Item(id, _) | Row::ModuleChoice(id, _) => {
                    let glide = self.motion.get(&format!("row:{id}")).map_or(0.0, |t| t.value(now));
                    let top = card_top[id] - scroll + glide;
                    unsafe { frame.dc.PushAxisAlignedClip(&rect(left, top, width, reach[id]), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
                    Some(open[id] >= 1.0)
                }
                _ => None,
            };
            let usable = inside.unwrap_or(true);
            match row {
                Row::Title => {
                    let title = Font::new(Family::SegoeDisplay, 28.0, 600.0);
                    text_centred(frame, pick(lang, "设置", "Settings"), title, palette.text, left, y + 18.0, width, Align::Start);
                }
                Row::Heading(zh, en) => {
                    text_centred(frame, pick(lang, zh, en), Font::new(Family::Segoe, 14.0, 600.0), palette.text, left, y + 10.0, width, Align::Start);
                }
                Row::Skins => {
                    card(frame, palette, left, y, width, row_height, palette.card);
                    let gap = 12.0;
                    let cell = (width - 2.0 * ROW_SIDE - 2.0 * gap) / 3.0;
                    let names = [
                        (Skin::Paper, pick(lang, "记录纸", "Chart paper")),
                        (Skin::Glass, pick(lang, "磨砂玻璃", "Frosted glass")),
                        (Skin::Fluent, "Windows 11"),
                    ];
                    let current = theme::Skin::named(&self.settings.skin);
                    for (i, (skin, name)) in names.into_iter().enumerate() {
                        let x = left + ROW_SIDE + i as f32 * (cell + gap);
                        let box_ = Rect { x, y: y + 16.0, w: cell, h: 72.0 };
                        let chosen = skin == current;
                        let hover = hovered == Some(Target::Skin(skin));
                        skin_miniature(frame, skin, box_);
                        let ring = if chosen { (palette.selection, 2.0) } else { (palette.card_stroke, 1.0) };
                        if hover && !chosen {
                            let _ = skins::shadows(frame, &[(box_, 6.0)], &[Shadow { y: 2.0, blur: 8.0, spread: 0.0, color: Color::hex(0, 0.08) }]);
                        }
                        stroke_outside(frame, box_, 6.0, ring.0, ring.1);
                        let font = if chosen { Font::new(Family::Segoe, 13.0, 600.0) } else { Font::new(Family::Segoe, 13.0, 400.0) };
                        text_centred(frame, name, font, if chosen { palette.text } else { palette.text2 }, x, y + 16.0 + 72.0 + 10.0 + 9.0, cell, Align::Start);
                        self.targets.push((Rect { x, y: y + 16.0, w: cell, h: 72.0 + 10.0 + 18.0 }, Target::Skin(skin)));
                    }
                }
                Row::Choice(field) => {
                    card(frame, palette, left, y, width, row_height, palette.card);
                    let (name, options, chosen) = self.choices(field);
                    text_centred(frame, name, label, palette.text, left + ROW_SIDE, y + row_height / 2.0, width / 2.0, Align::Start);
                    self.segmented(frame, palette, field, &options, chosen, true, left + width - ROW_SIDE, y + row_height / 2.0, now, &hovered);
                }
                Row::Switch(switch) => {
                    card(frame, palette, left, y, width, row_height, palette.card);
                    let (name, detail, on) = self.switch(switch);
                    field_label(frame, palette, name, detail.as_deref(), label, hint, left + ROW_SIDE, y + row_height / 2.0, width - 2.0 * ROW_SIDE - 64.0);
                    let key = format!("switch:{switch:?}");
                    let pressed = self.pressed == Some(Target::Switch(switch));
                    self.toggle(frame, palette, key, on, true, left + width - ROW_SIDE - 40.0, y + row_height / 2.0, now, pressed);
                    self.targets.push((Rect { x: left, y, w: width, h: row_height }, Target::Switch(switch)));
                }
                Row::Module(id) => {
                    let offset = self.motion.get(&format!("row:{id}")).map_or(0.0, |t| t.value(now));
                    if self.drag.as_ref().is_some_and(|drag| drag.id == id) {
                        dragged = Some((id, row_height));
                        continue;
                    }
                    self.module_row(frame, palette, &id, left, y + offset, width, row_height, reach[&id], now, &hovered, false);
                }
                // The rows inside a module's card (drawn with its row) glide
                // with it, each under a hairline; while the module is off they
                // are dimmed and do nothing, and keep their state.
                Row::Item(id, name) => {
                    let y = y + self.motion.get(&format!("row:{id}")).map_or(0.0, |t| t.value(now));
                    fill(frame, palette.rule, left + 1.0, y, width - 2.0, 1.0, 0.0);
                    let active = self.module_on(&id);
                    let (name_text, detail) = self.item_label(&id, name);
                    let target = Target::Item(id.clone(), name);
                    let colors = if active { (palette.text, palette.text2) } else { (palette.text3, palette.text3) };
                    let switch_left = left + width - ROW_SIDE - CHEVRON_ROOM - 40.0;
                    field_label_in(frame, colors, name_text, detail, label, hint, left + ITEM_INSET, y + row_height / 2.0, switch_left - 12.0 - left - ITEM_INSET);
                    let pressed = self.pressed == Some(target.clone());
                    let on = self.prefs.shows(&id, name);
                    self.toggle(frame, palette, format!("item:{id}:{name}"), on, active, switch_left, y + row_height / 2.0, now, pressed);
                    if active && usable {
                        self.targets.push((Rect { x: left, y, w: width, h: row_height }, target));
                    }
                }
                Row::ModuleChoice(id, field) => {
                    let y = y + self.motion.get(&format!("row:{id}")).map_or(0.0, |t| t.value(now));
                    fill(frame, palette.rule, left + 1.0, y, width - 2.0, 1.0, 0.0);
                    let active = self.module_on(&id);
                    let (name, options, chosen) = self.choices(field);
                    let color = if active { palette.text } else { palette.text3 };
                    text_centred(frame, name, label, color, left + ITEM_INSET, y + row_height / 2.0, width / 2.0 - ITEM_INSET, Align::Start);
                    let right = left + width - ROW_SIDE - CHEVRON_ROOM;
                    let targets = self.targets.len();
                    self.segmented(frame, palette, field, &options, chosen, active, right, y + row_height / 2.0, now, &hovered);
                    if !usable {
                        self.targets.truncate(targets);
                    }
                }
                Row::Update => {
                    let Some((version, state)) = update::available() else { continue };
                    let name = match lang {
                        Lang::Zh => format!("Glance {version} 可用"),
                        Lang::En => format!("Glance {version} is available"),
                    };
                    let (detail, action) = match state {
                        update::State::Ready => (
                            pick(lang, "下载并运行已签名的安装程序", "Downloads and runs the signed installer"),
                            pick(lang, "更新", "Update"),
                        ),
                        update::State::Downloading => (pick(lang, "正在下载安装程序", "Downloading the installer"), pick(lang, "下载中", "Downloading")),
                        update::State::Failed => (pick(lang, "下载失败，请稍后重试", "Download failed; try again later"), pick(lang, "重试", "Retry")),
                        update::State::Rejected => (
                            pick(lang, "签名或版本不符，未运行；请手动下载", "Signature or version did not match; not run. Download it yourself"),
                            pick(lang, "重试", "Retry"),
                        ),
                    };
                    card(frame, palette, left, y, width, row_height, palette.card);
                    field_label(frame, palette, &name, Some(detail), label, hint, left + ROW_SIDE, y + row_height / 2.0, width - 160.0);
                    let button_w = frame.gfx.measure(action, label) + 32.0;
                    let button = Rect { x: left + width - ROW_SIDE - button_w, y: y + row_height / 2.0 - 16.0, w: button_w, h: 32.0 };
                    let busy = state == update::State::Downloading;
                    if hovered == Some(Target::Update) && !busy {
                        fill(frame, palette.hover, button.x, button.y, button.w, button.h, 6.0);
                    }
                    stroke_inside(frame, button, 6.0, palette.rule);
                    text_centred(frame, action, label, if busy { palette.text2 } else { palette.signal }, button.x + 16.0, button.y + 16.0, button_w, Align::Start);
                    self.targets.push((button, Target::Update));
                }
                Row::Diagnostics => {
                    // "Copied" for a moment after the press.
                    let copied = self.copied_at.is_some_and(|at| now.duration_since(at) < COPIED_FOR);
                    let action = if copied { pick(lang, "已复制", "Copied") } else { pick(lang, "复制", "Copy") };
                    card(frame, palette, left, y, width, row_height, palette.card);
                    let name = pick(lang, "诊断信息", "Diagnostics");
                    let detail = pick(lang, "反馈问题时复制附上", "To paste into a problem report");
                    field_label(frame, palette, name, Some(detail), label, hint, left + ROW_SIDE, y + row_height / 2.0, width - 160.0);
                    let button_w = frame.gfx.measure(action, label) + 32.0;
                    let button = Rect { x: left + width - ROW_SIDE - button_w, y: y + row_height / 2.0 - 16.0, w: button_w, h: 32.0 };
                    if hovered == Some(Target::Diagnostics) {
                        fill(frame, palette.hover, button.x, button.y, button.w, button.h, 6.0);
                    }
                    stroke_inside(frame, button, 6.0, palette.rule);
                    text_centred(frame, action, label, palette.signal, button.x + 16.0, button.y + 16.0, button_w, Align::Start);
                    self.targets.push((button, Target::Diagnostics));
                }
                Row::Uninstall | Row::Quit => {
                    let (target, name, detail, action) = if matches!(row, Row::Uninstall) {
                        (
                            Target::Uninstall,
                            pick(lang, "卸载 Glance", "Uninstall Glance"),
                            pick(lang, "连同设置一起删除，会先确认", "Removes it and its settings; asks first"),
                            pick(lang, "卸载", "Uninstall"),
                        )
                    } else {
                        (
                            Target::Quit,
                            pick(lang, "退出 Glance", "Quit Glance"),
                            pick(lang, "面板和托盘图标都会关闭", "Closes the panel and the tray icon"),
                            pick(lang, "退出", "Quit"),
                        )
                    };
                    card(frame, palette, left, y, width, row_height, palette.card);
                    field_label(frame, palette, name, Some(detail), label, hint, left + ROW_SIDE, y + row_height / 2.0, width - 160.0);
                    let button_w = frame.gfx.measure(action, label) + 32.0;
                    let button = Rect { x: left + width - ROW_SIDE - button_w, y: y + row_height / 2.0 - 16.0, w: button_w, h: 32.0 };
                    if hovered.as_ref() == Some(&target) {
                        fill(frame, palette.hover, button.x, button.y, button.w, button.h, 6.0);
                    }
                    stroke_inside(frame, button, 6.0, palette.rule);
                    text_centred(frame, action, label, palette.signal, button.x + 16.0, button.y + 16.0, button_w, Align::Start);
                    self.targets.push((button, target));
                }
            }
            if inside.is_some() {
                unsafe { frame.dc.PopAxisAlignedClip() };
            }
        }
        // The dragged row passes over the others.
        if let Some((id, row_height)) = dragged {
            let drag = self.drag.as_ref().unwrap();
            let y = drag.pointer - drag.grab - scroll;
            self.module_row(frame, palette, &id, left, y, width, row_height, row_height, now, &hovered, true);
        }
        // The focus, shown once the keyboard is in use.
        if self.keyboard {
            if let Some(focus) = &self.focus {
                if let Some((r, target)) = self.targets.iter().find(|(_, target)| target == focus) {
                    // A skin's target holds its label flush with the card's
                    // edge, so its ring keeps some room; the rest hug theirs.
                    let room = if matches!(target, Target::Skin(_)) { 8.0 } else { -1.0 };
                    let ring = Rect { x: r.x - room, y: r.y - room, w: r.w + 2.0 * room, h: r.h + 2.0 * room };
                    stroke_outside(frame, ring, 5.0 + room.max(0.0), palette.text, 2.0);
                }
            }
        }
        // A thin thumb shows where in the list the view is.
        let content = self.content_height();
        if content > height {
            let thumb = (height * height / content).max(24.0);
            let at = (height - thumb) * self.scroll / (content - height);
            fill(frame, palette.text3.alpha(0.6), PANE - GUTTER + 2.0, at + 2.0, 3.0, thumb - 4.0, 1.5);
        }
    }

    /// A module's row, and when its card is open (`reach` below the row's
    /// top), the card around the rows inside it: one rounded box.
    #[allow(clippy::too_many_arguments)]
    fn module_row(&mut self, frame: &Frame, palette: &Palette, id: &str, left: f32, y: f32, width: f32, height: f32, reach: f32, now: Instant, hovered: &Option<Target>, held: bool) {
        let label = Font::new(Family::Segoe, 14.0, 400.0);
        let hint = Font::new(Family::Segoe, 12.0, 400.0);
        if held {
            let shape = Rect { x: left, y, w: width, h: height };
            let _ = skins::shadows(frame, &[(shape, 6.0)], &[Shadow { y: 4.0, blur: 16.0, spread: 0.0, color: Color::hex(0, 0.12) }]);
            // Opaque while it passes over the other rows.
            fill(frame, palette.window, left, y, width, height, 6.0);
            card(frame, palette, left, y, width, height, palette.card);
            stroke_inside(frame, shape, 6.0, palette.selection);
        } else {
            // The whole card, its rows inside it in the same shade, as
            // Windows' own settings draw an open one.
            card(frame, palette, left, y, width, reach, palette.card);
        }
        let on = self.prefs.modules.iter().find(|entry| entry.id == id).is_some_and(|entry| entry.on);
        let (title, detail) = self.module(id);
        // The grip: six dots, to drag the row by.
        let grip = Rect { x: left + 12.0, y: y + height / 2.0 - 16.0, w: 20.0, h: 32.0 };
        if *hovered == Some(Target::Grip(id.to_string())) || held {
            fill(frame, palette.hover, grip.x, grip.y, grip.w, grip.h, 4.0);
        }
        for (dx, dy) in [(-2.0, -5.0), (2.0, -5.0), (-2.0, 0.0), (2.0, 0.0), (-2.0, 5.0), (2.0, 5.0)] {
            let dot = D2D1_ELLIPSE { point: Vector2 { X: grip.x + 10.0 + dx, Y: grip.y + 16.0 + dy }, radiusX: 1.2, radiusY: 1.2 };
            unsafe { frame.dc.FillEllipse(&dot, frame.brush(palette.text3)) };
        }
        let text_left = left + ITEM_INSET;
        // The switch, and right of it, if the module has a card to open, a
        // chevron that turns over as it opens.
        let opens = self.opens(id);
        let open = self.expanded.contains(id);
        let cy = y + height / 2.0;
        let switch_left = left + width - ROW_SIDE - CHEVRON_ROOM - 40.0;
        if opens {
            let turn = self.animate(format!("chevron:{id}"), if open { 180.0 } else { 0.0 }, SLIDE, now, false);
            let chevron = Font::new(Family::Icons, 12.0, 400.0);
            let glyph = "\u{E70D}";
            let glyph_w = frame.gfx.measure(glyph, chevron);
            let cx = left + width - ROW_SIDE - 6.0;
            frame.place(Matrix3x2::rotation_around(turn, Vector2 { X: cx, Y: cy }));
            text_centred(frame, glyph, chevron, palette.text2, cx - glyph_w / 2.0, cy, glyph_w + 4.0, Align::Start);
            frame.origin(0.0, 0.0);
        }
        field_label(frame, palette, &title, detail.as_deref(), label, hint, text_left, cy, switch_left - 12.0 - text_left);
        let pressed = self.pressed == Some(Target::Module(id.to_string()));
        self.toggle(frame, palette, format!("module:{id}"), on, true, switch_left, cy, now, pressed);
        if !held {
            // The grip drags, the switch switches, and the rest of the row
            // opens and closes the card (or, with none, switches too).
            self.targets.push((grip, Target::Grip(id.to_string())));
            let switch_area = Rect { x: switch_left - 12.0, y, w: 40.0 + 24.0, h: height };
            self.targets.push((switch_area, Target::Module(id.to_string())));
            let row = Rect { x: left, y, w: width, h: height };
            self.targets.push((row, if opens { Target::Expand(id.to_string()) } else { Target::Module(id.to_string()) }));
        }
    }

    /// A row of options with one thumb that slides under the chosen one,
    /// right-aligned at `right` and centred on `cy`; unless `enabled`, dimmed
    /// and not to be pressed.
    #[allow(clippy::too_many_arguments)]
    fn segmented(&mut self, frame: &Frame, palette: &Palette, field: Field, options: &[String], chosen: Option<usize>, enabled: bool, right: f32, cy: f32, now: Instant, hovered: &Option<Target>) {
        let font = Font::new(Family::Segoe, 13.0, 400.0);
        let widths: Vec<f32> = options.iter().map(|o| (frame.gfx.measure(o, font) + 24.0).max(44.0)).collect();
        let total = widths.iter().sum::<f32>() + 4.0;
        let (x0, y0) = (right - total, cy - 16.0);
        fill(frame, palette.control, x0, y0, total, 32.0, 6.0);
        let mut x = x0 + 2.0;
        let mut spans = Vec::new();
        for w in &widths {
            spans.push((x, *w));
            x += w;
        }
        if let Some(index) = chosen {
            let (to_x, to_w) = (spans[index].0 - x0, spans[index].1);
            let jump = self.settle_thumbs;
            let tx = self.animate(format!("thumb-x:{field:?}"), to_x, SLIDE, now, jump);
            let tw = self.animate(format!("thumb-w:{field:?}"), to_w, SLIDE, now, jump);
            let thumb = Rect { x: x0 + tx, y: y0 + 2.0, w: tw, h: 28.0 };
            if let Some(drop) = palette.control_drop {
                fill(frame, drop, thumb.x, thumb.y + 1.0, thumb.w, thumb.h, 4.0);
            }
            stroke_outside(frame, thumb, 4.0, palette.control_ring, 1.0);
            fill(frame, palette.control_on, thumb.x, thumb.y, thumb.w, thumb.h, 4.0);
        }
        for (i, ((sx, sw), option)) in spans.iter().zip(options).enumerate() {
            let target = Target::Choice(field, i);
            let lit = chosen == Some(i) || *hovered == Some(target.clone());
            let color = match (enabled, lit) {
                (false, _) => palette.text3,
                (true, true) => palette.text,
                (true, false) => palette.text2,
            };
            let text_w = frame.gfx.measure(option, font);
            text_centred(frame, option, font, color, sx + (sw - text_w) / 2.0, cy, *sw, Align::Start);
            if enabled {
                self.targets.push((Rect { x: *sx, y: y0, w: *sw, h: 32.0 }, target));
            }
        }
    }

    /// A switch, 40 × 22, with its left edge at `x` and centred on `cy`;
    /// unless `enabled`, in the greys of one that does nothing for now.
    #[allow(clippy::too_many_arguments)]
    fn toggle(&mut self, frame: &Frame, palette: &Palette, key: String, on: bool, enabled: bool, x: f32, cy: f32, now: Instant, pressed: bool) {
        let dim = 1.0 - self.animate(format!("{key}:enabled"), if enabled { 1.0 } else { 0.0 }, FLIP, now, false).clamp(0.0, 1.0);
        let t = self.animate(key, if on { 1.0 } else { 0.0 }, FLIP, now, false).clamp(0.0, 1.0);
        let track = Rect { x, y: cy - 11.0, w: 40.0, h: 22.0 };
        // Off: an outlined track; on: a filled one. Between, the two
        // cross-fade. Disabled, the off track is outline alone, and both
        // take the disabled grey.
        if t < 1.0 {
            fill(frame, palette.switch_off.alpha((1.0 - t) * (1.0 - dim)), track.x, track.y, track.w, track.h, 11.0);
            stroke_inside(frame, track, 11.0, mix(palette.switch_stroke, palette.disabled, dim).alpha(1.0 - t));
        }
        if t > 0.0 {
            fill(frame, mix(palette.switch_on, palette.disabled, dim).alpha(t), track.x, track.y, track.w, track.h, 11.0);
        }
        // The knob grows as it crosses, and stretches while pressed.
        let size = 12.0 + 2.0 * t;
        let stretch = if pressed && enabled { 16.0 - size } else { 0.0 };
        let left = x + 4.0 + 17.0 * t - if on { stretch } else { 0.0 };
        let knob_off = mix(palette.switch_knob, palette.disabled, dim);
        let knob_on = mix(Color::hex(0xFFFFFF, 1.0), palette.disabled_knob, dim);
        fill(frame, mix(knob_off, knob_on, t), left, cy - size / 2.0, size + stretch, size, size / 2.0);
    }

    /// The preview: the panel as it will look over the desktop, running live.
    fn paint_preview(&mut self, frame: &Frame, palette: &Palette, width: f32, height: f32) -> Option<f32> {
        let lang = self.lang;
        let x0 = PANE + PAD_SIDE;
        let head = Font::new(Family::SegoeDisplay, 20.0, 600.0);
        let title = pick(lang, "预览", "Preview");
        text_centred(frame, title, head, palette.text, x0, PAD_TOP + 18.0, 200.0, Align::Start);
        let detail = pick(lang, "实时数据，桌面为当前壁纸", "Live readings over your wallpaper");
        let title_w = frame.gfx.measure(title, head);
        text_centred(frame, detail, Font::new(Family::Segoe, 12.0, 400.0), palette.text2, x0 + title_w + 12.0, PAD_TOP + 20.0, 400.0, Align::Start);
        let area = Rect { x: x0, y: PAD_TOP + 40.0, w: width - PANE - 2.0 * PAD_SIDE, h: height - PAD_TOP - 40.0 - PAD_TOP };
        if area.w <= 0.0 || area.h <= 0.0 {
            return None;
        }
        fill(frame, palette.card, area.x, area.y, area.w, area.h, 8.0);
        stroke_inside(frame, area, 8.0, palette.card_stroke);

        // The panel laid out for the screen the window opened on, centred
        // along its edge.
        let app = crate::app();
        let skin = Skin::named(&self.settings.skin);
        let edge = self.settings.edge;
        let interval = self.settings.interval_ms as f64;
        // Held while the preview is drawn; the sampler waits that long.
        let mut history = app.controller.history.lock().unwrap();
        let samples = history.make_contiguous();
        if samples.is_empty() {
            return None;
        }
        let wall = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64() * 1000.0;
        let pen = wall - interval - PEN_LAG_MS;
        let measure = Theme::new(skin, false);
        // The preview shows what the panel would hold if it opened now.
        let seen = app.controller.seen.lock().unwrap().clone();
        let probe = Scene { info: &app.info, prefs: &self.prefs, theme: &measure, lang, history: samples, seen: &seen, pen_ms: pen, process_scroll: 0.0, hover: None, pinned: false };
        let heights = view::lanes(&probe).iter().map(|lane| lane.height(&measure)).collect();
        let (sw, sh) = self.stage.size;
        let (layout, zoom) = arrange::arrange(&measure, edge, heights, (sw, sh), self.settings.columns);
        let (pw, ph) = (layout.width() * zoom, layout.height() * zoom);
        let inset = measure.inset * zoom;
        let along = |at: f32, length: f32, extent: f32| (at - length / 2.0).min(extent - GAP - length).max(GAP);
        let pos = match edge {
            Edge::Left => (inset, along(sh / 2.0, ph, sh)),
            Edge::Right => (sw - inset - pw, along(sh / 2.0, ph, sh)),
            Edge::Top => (along(sw / 2.0, pw, sw), inset),
        };
        // The desktop behind the panel decides its theme when it follows the
        // backdrop, and how frosted glass is.
        let behind = RECT { left: pos.0 as i32, top: pos.1 as i32, right: (pos.0 + pw) as i32, bottom: (pos.1 + ph) as i32 };
        let tone = self.stage.desktop.luminance(behind, 1.0);
        let dark = theme::is_dark(self.prefs.theme, Some(tone.0).filter(|_| skin.sees_backdrop()));
        let theme = Theme::new(skin, dark);
        let frost = if skin == Skin::Glass { skins::frost(tone.0, tone.1, dark) } else { 0.0 };
        let scene = Scene { info: &app.info, prefs: &self.prefs, theme: &theme, lang, history: samples, seen: &seen, pen_ms: pen, process_scroll: 0.0, hover: None, pinned: false };
        let lanes = view::lanes(&scene);

        // The whole height of the screen, and the whole panel with a strip of
        // desktop beside it; along the top, the panel and the screen below.
        let (k, stage_x, stage_y) = match edge {
            Edge::Left | Edge::Right => {
                let k = (area.h / sh).min(area.w / (pw + PREVIEW_MARGIN).min(sw)).min(1.0);
                let x = if edge == Edge::Left { 0.0 } else { area.w - sw * k };
                (k, x, (area.h - sh * k) / 2.0)
            }
            Edge::Top => {
                let shown = ph + PREVIEW_MARGIN;
                let k = (area.w / sw).min(area.h / shown).min(1.0);
                (k, (area.w - sw * k) / 2.0, ((area.h - shown * k) / 2.0).max(0.0))
            }
        };
        let (ox, oy) = (area.x + stage_x, area.y + stage_y);
        let mask = unsafe {
            frame.gfx.factory.CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT { rect: rect(area.x, area.y, area.w, area.h), radiusX: 8.0, radiusY: 8.0 })
        };
        let layer = D2D1_LAYER_PARAMETERS1 {
            contentBounds: rect(area.x, area.y, area.w, area.h),
            geometricMask: std::mem::ManuallyDrop::new(mask.ok().map(|m| windows::core::Interface::cast(&m).unwrap())),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: Matrix3x2::identity(),
            opacity: 1.0,
            ..Default::default()
        };
        unsafe { frame.dc.PushLayer(&layer, None) };
        let zoom_key = zoom.to_bits();
        if self.stage.bitmap.as_ref().is_none_or(|(key, _)| *key != zoom_key) {
            // The desktop in the panel's units, which a zoomed panel shrinks.
            self.stage.bitmap = self.stage.desktop.bitmap(&frame.dc, zoom).ok().map(|b| (zoom_key, b));
        }
        if let Some((_, bitmap)) = &self.stage.bitmap {
            unsafe {
                frame.dc.DrawBitmap(bitmap, Some(&rect(ox, oy, sw * k, sh * k)), 1.0, D2D1_INTERPOLATION_MODE_LINEAR, None, None);
            }
        }
        let local = Matrix3x2::scale(zoom, zoom) * Matrix3x2::translation(pos.0, pos.1) * Matrix3x2::scale(k, k) * Matrix3x2::translation(ox, oy);
        let backdrop = self.stage.bitmap.as_ref().map(|(_, bitmap)| (bitmap, Vector2 { X: pos.0 / zoom, Y: pos.1 / zoom }, self.stage.desktop.digest));
        let picture = render::Picture { scene: &scene, lanes: &lanes, layout: &layout, edge, backdrop, frost };
        let drawn = self.layers.draw(frame, &picture, local, self.scale * zoom * k);
        unsafe { frame.dc.PopLayer() };
        drop(std::mem::ManuallyDrop::into_inner(layer.geometricMask));
        match drawn {
            Ok(drawn) if drawn.grounded => self.gfx.clear_caches(),
            Ok(_) => {}
            Err(_) => self.failed = true,
        }
        Some(layout.plot_width(&theme) * self.scale * zoom * k / self.prefs.chart_seconds as f32)
    }
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color { r: a.r + (b.r - a.r) * t, g: a.g + (b.g - a.g) * t, b: a.b + (b.b - a.b) * t, a: a.a + (b.a - a.a) * t }
}

fn rounded(r: Rect, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT { rect: rect(r.x, r.y, r.w, r.h), radiusX: radius, radiusY: radius }
}

fn fill(frame: &Frame, color: Color, x: f32, y: f32, w: f32, h: f32, radius: f32) {
    let radius = radius.min(w / 2.0).min(h / 2.0);
    unsafe { frame.dc.FillRoundedRectangle(&rounded(Rect { x, y, w, h }, radius), frame.brush(color)) };
}

/// A one-DIP border drawn inside the box, as CSS's `border`.
fn stroke_inside(frame: &Frame, r: Rect, radius: f32, color: Color) {
    let inner = Rect { x: r.x + 0.5, y: r.y + 0.5, w: r.w - 1.0, h: r.h - 1.0 };
    unsafe { frame.dc.DrawRoundedRectangle(&rounded(inner, (radius - 0.5).max(0.0)), frame.brush(color), 1.0, None) };
}

/// A ring `width` DIPs wide just outside the box, as CSS's `box-shadow: 0 0 0 width`.
fn stroke_outside(frame: &Frame, r: Rect, radius: f32, color: Color, width: f32) {
    let ring = Rect { x: r.x - width / 2.0, y: r.y - width / 2.0, w: r.w + width, h: r.h + width };
    unsafe { frame.dc.DrawRoundedRectangle(&rounded(ring, radius + width / 2.0), frame.brush(color), width, None) };
}

/// A row's card: filled, with a hairline border.
fn card(frame: &Frame, palette: &Palette, x: f32, y: f32, w: f32, h: f32, color: Color) {
    fill(frame, color, x, y, w, h, 6.0);
    stroke_inside(frame, Rect { x, y, w, h }, 6.0, palette.card_stroke);
}

/// One line of text centred on `cy`.
#[allow(clippy::too_many_arguments)]
fn text_centred(frame: &Frame, text: &str, font: Font, color: Color, x: f32, cy: f32, width: f32, align: Align) {
    let (ascent, descent) = frame.gfx.baseline(font);
    frame.text(text, font, color, x, cy - (ascent + descent) / 2.0, width, align);
}

/// A row's label, and below it, if there is one, its hint, centred together on `cy`.
#[allow(clippy::too_many_arguments)]
fn field_label(frame: &Frame, palette: &Palette, label: &str, hint: Option<&str>, label_font: Font, hint_font: Font, x: f32, cy: f32, width: f32) {
    field_label_in(frame, (palette.text, palette.text2), label, hint, label_font, hint_font, x, cy, width);
}

/// A row's label and hint in the colours given (label, hint).
#[allow(clippy::too_many_arguments)]
fn field_label_in(frame: &Frame, colors: (Color, Color), label: &str, hint: Option<&str>, label_font: Font, hint_font: Font, x: f32, cy: f32, width: f32) {
    match hint {
        None => text_centred(frame, label, label_font, colors.0, x, cy, width, Align::Start),
        Some(hint) => {
            // A 20 DIP line, 2 apart, then a 16 DIP one.
            let top = cy - 19.0;
            text_centred(frame, label, label_font, colors.0, x, top + 10.0, width, Align::Start);
            text_centred(frame, hint, hint_font, colors.1, x, top + 22.0 + 8.0, width, Align::Start);
        }
    }
}

/// A miniature of each skin, drawn the same whichever is active.
fn skin_miniature(frame: &Frame, skin: Skin, r: Rect) {
    let dc = &frame.dc;
    let inner = (r.x + 10.0, r.w - 20.0);
    let cy = r.y + r.h / 2.0;
    match skin {
        Skin::Paper => {
            fill(frame, Color::hex(0xEEF2EC, 1.0), r.x, r.y, r.w, r.h, 6.0);
            // Three ruled lines, the middle one in the signal colour.
            for (i, color) in [0x16221E, 0xD23F1C, 0x16221E].into_iter().enumerate() {
                let y = cy + (i as f32 - 1.0) * 13.5;
                fill(frame, Color::hex(color, 1.0), inner.0, y - 0.75, inner.1, 1.5, 0.0);
            }
        }
        Skin::Glass => {
            let stops = [
                D2D1_GRADIENT_STOP { position: 0.0, color: Color::hex(0x7AA7FF, 1.0).d2d() },
                D2D1_GRADIENT_STOP { position: 0.55, color: Color::hex(0xC79BFF, 1.0).d2d() },
                D2D1_GRADIENT_STOP { position: 1.0, color: Color::hex(0xFFB38A, 1.0).d2d() },
            ];
            let span = (r.w + r.h) * std::f32::consts::FRAC_1_SQRT_2 / 2.0 * std::f32::consts::FRAC_1_SQRT_2;
            let (cx, my) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            unsafe {
                if let Ok(stops) = ID2D1RenderTarget::CreateGradientStopCollection(dc, &stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) {
                    let line = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: Vector2 { X: cx - span, Y: my - span },
                        endPoint: Vector2 { X: cx + span, Y: my + span },
                    };
                    if let Ok(brush) = dc.CreateLinearGradientBrush(&line, None, &stops) {
                        dc.FillRoundedRectangle(&rounded(r, 6.0), &brush);
                    }
                }
            }
            for i in 0..3 {
                let y = cy - 19.0 + i as f32 * 14.0;
                let pill = Rect { x: inner.0, y, w: inner.1, h: 10.0 };
                fill(frame, Color::hex(0xFFFFFF, 0.35), pill.x, pill.y, pill.w, pill.h, 5.0);
                stroke_inside(frame, pill, 5.0, Color::hex(0xFFFFFF, 0.4));
                fill(frame, Color::hex(0xFFFFFF, 0.9), pill.x + 4.0, pill.y, pill.w - 8.0, 1.0, 0.0);
            }
        }
        Skin::Fluent => {
            fill(frame, Color::hex(0xECEEF2, 1.0), r.x, r.y, r.w, r.h, 6.0);
            for i in 0..3 {
                let y = cy - 8.5 + i as f32 * 7.0;
                let (w, color) = if i == 0 { (inner.1 * 0.6, theme::accent(false)) } else { (inner.1, Color::hex(0xC8CCD2, 1.0)) };
                fill(frame, color, inner.0, y, w, 3.0, 1.5);
            }
        }
    }
}
