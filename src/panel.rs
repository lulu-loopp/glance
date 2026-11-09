//! Shows and hides the panels, and draws them while they are up.
//!
//! Each edge that holds something (of each screen) has a panel of its own:
//! pushed into, or rested on where screens meet, it opens there whatever
//! the others do, and is pinned there or not on its own; several may be up
//! at once. A panel made is kept, hidden, for the next edge opened (made
//! anew, what it captures of the desktop keeps some of the card's memory).
//!
//! One thread does all of it: it reads the pointer, owns the panels' windows
//! and draws every frame while a panel is on screen. Whether the pointer is
//! "on the panel" is decided from the global cursor position against where
//! the panel rests, so nothing that moves on screen feeds back into it.

use std::collections::VecDeque;

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::rc::Rc;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsIconic, SetForegroundWindow, SC_RESTORE, WM_SYSCOMMAND};
use windows::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_RUNNING_D3D_FULL_SCREEN};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{DefWindowProcW, RegisterClassW, WNDCLASSW};
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap1, D2D1_LAYER_PARAMETERS1};
use windows::Win32::Graphics::Dwm::{DwmGetCompositionTimingInfo, DWM_TIMING_INFO};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::{CreateWaitableTimerExW, SetWaitableTimer, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, INFINITE, TIMER_ALL_ACCESS};
use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONULL};
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, MOD_NOREPEAT, VK_ESCAPE};
use windows::Win32::UI::Input::{RegisterRawInputDevices, HRAWINPUT, RAWINPUTDEVICE, RIDEV_INPUTSINK, RIDEV_REMOVE};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DispatchMessageW, KillTimer, WaitMessage, MsgWaitForMultipleObjects,
    PeekMessageW, PostMessageW, SetTimer, SystemParametersInfoW, HWND_MESSAGE, MSG, PM_REMOVE,
    QS_ALLINPUT, WM_HOTKEY,
    SPI_GETCLIENTAREAANIMATION, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_INPUT, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_RBUTTONUP, WM_MOUSEMOVE, WM_MOUSEHWHEEL, WM_MOUSEWHEEL, WM_QUIT,
    WM_TIMER,
};

use crate::mouse::{cursor_position, cursor_showing, monitor_at, read_mouse, Contact, CORNER_EXCLUSION};
use crate::reading::{Sample, StaticInfo};
use crate::overlay::{Choice as OverlayChoice, Overlay};
use crate::settings::{Anchor, Edge, OverFullscreen, OverlaySettings, PanelAt, PanelSizes, Part, Settings, EDGES};
use crate::ui::backdrop::Capture;
use crate::ui::gfx::{self, Gfx, Surface};
use crate::ui::arrange::{self, Form, GAP};
use crate::ui::seen::Seen;
use crate::ui::life::Lives;
use crate::ui::render::{self, PanelLayers};
use crate::ui::motion::{Easing, Transition, LINEAR};
use crate::ui::prefs::Prefs;
use crate::ui::skins;
use crate::ui::text::Lang;
use crate::ui::theme::{self, Entrance, Skin, Theme};
use crate::ui::view::{self, Detail, Hit, HitBox, Layout, Scene};
use crate::ui::settings_window;
use crate::ui::window::Window;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows_numerics::{Matrix3x2, Vector2};

/// The pointer may stray this far outside the panel and still count as on it (DIPs).
const LEAVE_TOLERANCE: f32 = 10.0;
/// The seam between two screens (see `seam_contact`): how far either side
/// of the line the pointer may be (DIPs; there is no wall to stop it on the
/// line); how far past it, on the other screen, a pointer that has just
/// come across from the edge's screen (it overshot) may be, and for how
/// long after; how still it has to rest there (DIPs), and how long. Longer
/// than on an edge: a scroll bar may be beside the seam.
const SEAM_ZONE: f32 = 8.0;
const SEAM_OVERSHOOT: f32 = 64.0;
const SEAM_CROSSED: Duration = Duration::from_millis(1500);
const SEAM_STILL: f32 = 8.0;
const SEAM_DWELL: Duration = Duration::from_millis(500);
/// History is kept for the longest chart span the settings offer.
const LONGEST_SPAN: Duration = Duration::from_secs(300);
/// The chart's pen runs this far behind the newest sample beyond one
/// interval, so the next sample has arrived by the time it is drawn to.
pub(crate) const PEN_LAG_MS: f64 = 100.0;
const TICK_MS: u32 = 16;
/// Timers on the input loop while the panel is down: one that follows the
/// pointer resting on the edge, and the slow watch for blocked input.
const TRACK_TIMER: usize = 1;
const WATCH_TIMER: usize = 2;
const WATCH_MS: u32 = 100;
/// How soon to try again when drawing failed for want of a device.
const DEVICE_RETRY: Duration = Duration::from_millis(250);
/// How often a live backdrop is taken again as it changes (see `widget::LIVE_INTERVAL`).
const LIVE_INTERVAL: Duration = crate::ui::scene::LIVE;
/// Rows a wheel notch scrolls the process list by.
pub(crate) const WHEEL_ROWS: f32 = 3.0;
/// How quickly the process list catches up with the wheel.
pub(crate) const SCROLL_EASE: f32 = 0.06;

/// The panel slides in from past the edge and fades up, and goes the same way.
pub(crate) const OPEN: (Duration, Easing) = (Duration::from_millis(260), Easing(0.16, 1.0, 0.3, 1.0));
pub(crate) const OPEN_FADE: Duration = Duration::from_millis(140);
pub(crate) const CLOSE: (Duration, Easing) = (Duration::from_millis(180), Easing(0.4, 0.0, 1.0, 1.0));
/// The sizes the panel and the overlay may be drawn at, as designed's share.
pub(crate) const SIZES: (f32, f32) = (0.75, 2.0);
/// With animations turned off in Windows the panel only fades.
const PLAIN_FADE: Duration = Duration::from_millis(120);
/// While the pinned panel is dragged, the pointer is followed this often (ms).
const FOLLOW_MS: u32 = 8;

/// Requests to the input thread from elsewhere.
const OPEN_FROM_TRAY: u32 = WM_APP + 1;
const DISMISS: u32 = WM_APP + 2;
const RESTYLE: u32 = WM_APP + 3;
const OPEN_SETTINGS: u32 = WM_APP + 4;
/// Notes asked to be stuck on the desktop (see `Controller::stick_note`).
const STICK_NOTES: u32 = WM_APP + 30;
/// Posted as Glance goes, for what is written in notes where they are to be kept (see `Controller::keep_writing`).
const KEEP_WRITING: u32 = WM_APP + 18;
const TOGGLE: u32 = WM_APP + 5;
/// A new sample: the overlay is drawn again.
const OVERLAY: u32 = WM_APP + 7;
/// The settings window came to the front, or left it.
const SETTINGS_FRONT: u32 = WM_APP + 9;
/// Brings back the desktop widget put away last.
const RECALL_WIDGET: u32 = WM_APP + 10;
/// The tray icon's menu, opening up from the point in LPARAM (x low, y
/// high, physical px).
const TRAY_MENU: u32 = WM_APP + 11;
/// The line of the lyrics sung has changed: what shows them is drawn at once (it glides to the next).
const LYRIC: u32 = WM_APP + 13;
/// The sensors' process saw a push into an edge (WPARAM the edge, LPARAM where: x low, y high, physical px).
const PUSHED: u32 = WM_APP + 14;
/// The sensors' process went: the mouse is heard here, and the edges watched, from now on.
const HEARING_CHANGED: u32 = WM_APP + 15;
/// Something this thread looks at changed elsewhere (it takes it up as it wakes: see `Controller::wake`).
const WAKE: u32 = WM_APP + 16;

/// The panel thread's own hotkey: Escape, while the panel is open.
const ESCAPE_HOTKEY: i32 = 1;
/// Ctrl+V, while the pointer is over a station: what is on the clipboard put on it.
const PASTE_HOTKEY: i32 = 2;
/// The preview's shortcut, while the pointer is over what may be looked at larger, or a look is up from here (see
/// `ui::look`).
const PREVIEW_HOTKEY: i32 = 3;
/// The look's own keys, while it is up from here: the arrows, Enter, Ctrl+C (Esc is the panel's own, as ever). Each with
/// whether it is held with Ctrl, and its virtual key.
const LOOK_KEYS: [(i32, crate::ui::look::Key, bool, u16); 6] = [
    (4, crate::ui::look::Key::Left, false, 0x25),
    (5, crate::ui::look::Key::Up, false, 0x26),
    (6, crate::ui::look::Key::Right, false, 0x27),
    (7, crate::ui::look::Key::Down, false, 0x28),
    (8, crate::ui::look::Key::Open, false, 0x0D),
    (9, crate::ui::look::Key::Copy, true, 0x43),
];
/// Backspace, while a look up from here shows a file of a group by itself: back to the group (see `ui::look::inside`).
const LOOK_BACK_HOTKEY: i32 = 10;
/// Space, while a look up from here shows what plays: played or paused (see `ui::look::plays`).
const LOOK_PLAY_HOTKEY: i32 = 11;

struct Config {
    /// Which edges of which screens open the panel, to a push (and how hard)
    /// or to a rest on a seam.
    edges: crate::push::Edges,
    skin: String,
    live: bool,
    tear_off: bool,
    anchor: Anchor,
    /// The readings' panel's columns and the workbench's.
    columns: Option<usize>,
    workbench_columns: Option<usize>,
    /// How large a panel is drawn where it was not sized, and each component's on each screen it was (see
    /// `Settings::part_sizes`).
    size: f32,
    sizes: PanelSizes,
    overlay: OverlaySettings,
    panel_at: Option<PanelAt>,
    /// The panels pinned open at their edges (see `Settings::panels_pinned`).
    panels_pinned: Vec<((i32, i32), Option<Edge>)>,
    music_rooms: crate::settings::MusicRooms,
    /// (Shared: taken at every frame of the widgets, as they are, not copied.)
    widgets: Arc<[crate::settings::WidgetAt]>,
    over_fullscreen: OverFullscreen,
    close_delay: Duration,
    interval: Duration,
    history: usize,
    view: serde_json::Value,
}

impl Config {
    /// How large `part`'s panel is drawn on the screen named `screen` (see `Settings::panel_size_on`): as far as the
    /// settings offer (the file may say anything).
    fn size_on(&self, part: Part, screen: Option<&str>) -> f32 {
        self.sizes.on(part, screen, self.size).clamp(SIZES.0, SIZES.1)
    }
}

fn config_from(settings: &Settings) -> Config {
    Config {
        edges: crate::push::Edges { places: settings.places().clone(), pressure: settings.sensitivity.pressure() },
        skin: settings.skin.clone(),
        live: settings.live_backdrop,
        tear_off: settings.tear_off,
        anchor: settings.anchor,
        columns: settings.columns_of(Part::Monitor),
        workbench_columns: settings.columns_of(Part::Workbench),
        size: settings.panel_size,
        sizes: settings.part_sizes.clone(),
        overlay: settings.overlay.clone(),
        panel_at: settings.panel_at,
        panels_pinned: settings.panels_pinned.clone(),
        music_rooms: settings.music_rooms,
        widgets: settings.widgets.as_slice().into(),
        over_fullscreen: settings.over_fullscreen,
        close_delay: settings.close_delay(),
        interval: settings.interval(),
        history: (LONGEST_SPAN.as_millis() / settings.interval().as_millis()) as usize + 16,
        view: settings.view.clone(),
    }
}

/// Whether a game is being played: one presented frames, and has not been
/// without them for `GAME_GAP` since.
#[derive(Default)]
struct Playing {
    now: bool,
    /// When a game last presented frames.
    last: Option<Instant>,
}

/// How long a game can go without frames (a loading screen, a stall, a
/// look at another window) and still be played.
const GAME_GAP: Duration = Duration::from_secs(3);

pub struct Controller {
    /// What the machine is, as Glance started, its drives as what the readings come from last listed them (see
    /// `drives_listed`).
    info: RwLock<Arc<StaticInfo>>,
    config: Mutex<Config>,
    playing: Mutex<Playing>,
    /// The samples of the longest chart span, oldest first.
    /// Read by the threads that draw at once (the panel's and the settings'), each for as long as it draws; written
    /// by the sampler alone, a sample at a time.
    pub history: RwLock<VecDeque<Sample>>,
    /// What the machine has shown it can read since Glance started.
    pub seen: Mutex<Seen>,
    /// The overlay, opened by a game, closed by hand for the rest of it.
    overlay_closed: AtomicBool,
    /// The input thread's message window, once it has one.
    sink: AtomicIsize,
    shown: AtomicBool,
    /// A desktop widget was put away this session (the tray offers it back).
    widget_put_away: AtomicBool,
    /// Whether a widget on the screen shows the process list (read afresh each sample then, as while the panel is).
    widget_processes: AtomicBool,
    /// Whether a widget is on the screen, or the music full screen is up.
    widgets_seen: AtomicBool,
}

impl Controller {
    pub fn new(info: StaticInfo, settings: &Settings) -> Self {
        crate::media::choose_music(settings.music_source.as_ref().map(|source| source.id.clone()));
        Controller {
            info: RwLock::new(Arc::new(info)),
            config: Mutex::new(config_from(settings)),
            playing: Mutex::new(Playing::default()),
            history: RwLock::new(VecDeque::new()),
            seen: Mutex::new(Seen::default()),
            overlay_closed: AtomicBool::new(false),
            sink: AtomicIsize::new(0),
            shown: AtomicBool::new(false),
            widget_put_away: AtomicBool::new(false),
            widget_processes: AtomicBool::new(false),
            widgets_seen: AtomicBool::new(false),
        }
    }

    /// What the machine is (see `info`).
    pub fn info(&self) -> Arc<StaticInfo> {
        self.info.read().unwrap().clone()
    }

    /// The drives as `listed` lists them, from now: what the readings come from has looked at the hardware. (Glance
    /// may have started with a sampler of the meanwhile, which lists none: see `Sampler::meanwhile`.)
    pub fn drives_listed(&self, listed: &StaticInfo) {
        let mut info = self.info.write().unwrap();
        *info = Arc::new(info.with_drives_of(listed));
    }

    pub fn apply(&self, settings: &Settings) {
        crate::media::choose_music(settings.music_source.as_ref().map(|source| source.id.clone()));
        *self.config.lock().unwrap() = config_from(settings);
        self.post(RESTYLE);
    }

    /// Shows the tray icon's menu from `(x, y)`: on this thread, which draws
    /// Glance's menus.
    pub fn tray_menu(&self, x: i32, y: i32) {
        let sink = self.sink.load(Ordering::Acquire);
        if sink != 0 {
            let _ = unsafe { PostMessageW(Some(HWND(sink as *mut _)), TRAY_MENU, WPARAM(0), LPARAM(point_param(x, y))) };
        }
    }

    /// The sensors' process saw a push into `edge` with the pointer at `at` (from any thread).
    pub fn pushed(&self, edge: Edge, at: (i32, i32)) {
        let sink = self.sink.load(Ordering::Acquire);
        if sink != 0 {
            let edge = EDGES.iter().position(|e| *e == edge).unwrap_or(0);
            let _ = unsafe { PostMessageW(Some(HWND(sink as *mut _)), PUSHED, WPARAM(edge), LPARAM(point_param(at.0, at.1))) };
        }
    }

    /// The sensors' process went, or came: the edges are watched on the panel's thread while there is none, by it while
    /// there is one (from any thread).
    pub fn hearing_changed(&self) {
        self.post(HEARING_CHANGED);
    }

    /// Which edges open the panel, as the settings have them now.
    pub fn edges(&self) -> crate::push::Edges {
        self.config.lock().unwrap().edges.clone()
    }

    /// Wakes the panel's thread, for what it looks at as it wakes (from any thread).
    pub fn wake(&self) {
        self.post(WAKE);
    }

    /// The line of the lyrics sung has changed (from any thread).
    pub fn lyric_changed(&self) {
        self.post(LYRIC);
    }

    fn post(&self, message: u32) {
        self.post_with(message, 0);
    }

    fn post_with(&self, message: u32, lparam: isize) {
        let sink = self.sink.load(Ordering::Acquire);
        if sink != 0 {
            let _ = unsafe { PostMessageW(Some(HWND(sink as *mut _)), message, WPARAM(0), LPARAM(lparam)) };
        }
    }

    pub fn record(&self, sample: Sample) {
        {
            let mut seen = self.seen.lock().unwrap();
            seen.note(&sample);
            seen.note_playing(crate::ui::view::playing_rows());
        }
        // A game that has gone a short while without frames has not ended.
        let now = Instant::now();
        let (started, ended) = {
            let mut playing = self.playing.lock().unwrap();
            // A game taken by hand is the asking widget's alone.
            if sample.game.as_ref().is_some_and(|game| !game.by_hand) {
                playing.last = Some(now);
            }
            let was = playing.now;
            playing.now = playing.last.is_some_and(|last| now.duration_since(last) < GAME_GAP);
            // An overlay closed by hand stays closed for the rest of the
            // game (cleared under the lock `set_overlay` closes it under).
            if was && !playing.now {
                self.overlay_closed.store(false, Ordering::Relaxed);
            }
            (playing.now && !was, was && !playing.now)
        };
        if started {
            // The first game ever, with the overlay off: it is offered.
            let (on, in_game, offered) = {
                let config = self.config.lock().unwrap();
                (config.overlay.on, config.overlay.in_game, config.overlay.offered)
            };
            if !on && !in_game && !offered {
                let name = sample.game.as_ref().map(|game| game.name.clone()).unwrap_or_default();
                std::thread::spawn(move || crate::app().offer_overlay(&name));
            }
        }
        let keep = self.config.lock().unwrap().history;
        {
            let mut history = self.history.write().unwrap();
            while history.len() >= keep {
                history.pop_front();
            }
            history.push_back(sample);
            // (Kept in one piece, for those that read it: see `contiguous`.)
            history.make_contiguous();
        }
        // Told once the sample is there to be drawn.
        if started || ended {
            // The bar's overlay button is lit while the overlay is up.
            self.post(RESTYLE);
        }
        self.post(OVERLAY);
    }

    pub fn is_shown(&self) -> bool {
        self.shown.load(Ordering::Relaxed)
    }

    /// Whether a game is being played.
    pub fn playing(&self) -> bool {
        self.playing.lock().unwrap().now
    }

    /// Whether the overlay is wanted on screen: on all the time, or opened by
    /// a game (and not closed for the rest of it).
    pub fn overlay_wanted(&self) -> bool {
        let (on, in_game) = {
            let config = self.config.lock().unwrap();
            (config.overlay.on, config.overlay.in_game)
        };
        on || in_game && self.playing() && !self.overlay_closed.load(Ordering::Relaxed)
    }

    /// The overlay opened or closed by hand (its button, its menu): closed,
    /// it is off, and if a game opened it, closed for the rest of that game;
    /// opened, it is on, or during a game that opens it, only open again
    /// for the rest of it.
    pub fn set_overlay(&self, open: bool) {
        let (on, in_game) = {
            let config = self.config.lock().unwrap();
            (config.overlay.on, config.overlay.in_game)
        };
        let by_game = {
            let playing = self.playing.lock().unwrap();
            self.overlay_closed.store(!open && playing.now, Ordering::Relaxed);
            in_game && playing.now
        };
        if on != open && !(open && by_game) {
            std::thread::spawn(move || crate::app().set_overlay(open));
        } else {
            self.post(RESTYLE);
        }
    }

    /// Opens the panel from the tray icon, on the monitor the pointer is on.
    pub fn open_from_tray(&self) {
        self.post(OPEN_FROM_TRAY);
    }

    /// Whether a desktop widget was put away that can be brought back.
    pub fn widget_processes(&self) -> bool {
        self.widget_processes.load(Ordering::Relaxed)
    }

    /// Whether anything of the panel's thread that can show what plays is on the screen: the panel, a widget, the
    /// music full screen.
    pub fn plays_seen(&self) -> bool {
        self.is_shown() || self.widgets_seen.load(Ordering::Relaxed)
    }

    /// Whether `seen`, as `plays_seen` is now: what plays looked at again at once, as it comes on the screen.
    fn set_seen(&self, flag: &AtomicBool, seen: bool) {
        if !flag.swap(seen, Ordering::Relaxed) && seen {
            crate::playing::look_now();
        }
    }

    pub fn widget_put_away(&self) -> bool {
        self.widget_put_away.load(Ordering::Relaxed)
    }

    /// Brings back the desktop widget put away last.
    pub fn recall_widget(&self) {
        self.post(RECALL_WIDGET);
    }

    /// Opens the panel at the pointer as the tray does, or closes it if it
    /// is open (pinned or not): the keyboard shortcut.
    pub fn toggle(&self) {
        self.post(TOGGLE);
    }

    /// Takes the panel off the screen at once, without its animation.
    pub fn dismiss(&self) {
        self.post(DISMISS);
    }

    /// The settings window is in front (`Some`: it), or no longer (gone to another window, or closed): the panel and
    /// the widgets give way to it meanwhile, so that they never cover it (see `Window::yield_to`). Told by the
    /// settings' own thread as it happens, in the message: found out here as the message is taken, the window in
    /// front may not yet be the one that comes, nor the settings gone.
    pub fn settings_front(&self, front: Option<HWND>) {
        self.in_front(front, 0);
    }

    /// The page beside the panel or a widget is up (`Some`: it), or no longer: the widgets give way to it meanwhile
    /// (one under it, a note stuck on the desktop written in over its card, never comes over it as it is drawn
    /// again). Told by the page's thread.
    pub fn page_front(&self, front: Option<HWND>) {
        self.in_front(front, 1);
    }

    /// The window `front` in front, or none (`which`: 0 the settings, 1 the page beside), told to the panel's thread.
    fn in_front(&self, front: Option<HWND>, which: isize) {
        let sink = self.sink.load(Ordering::Acquire);
        if sink != 0 {
            let front = front.map_or(0, |hwnd| hwnd.0 as usize);
            let _ = unsafe { PostMessageW(Some(HWND(sink as *mut _)), SETTINGS_FRONT, WPARAM(front), LPARAM(which)) };
        }
    }

    /// Opens the settings window (on a thread of its own: see `settings_window::open`), poured out of `place`, asked of
    /// this thread, which takes the tray's and the overlay's asking.
    pub fn open_settings(&self, place: Option<RECT>) {
        self.post_with(OPEN_SETTINGS, crate::ui::genie::told(place));
    }

    /// The note `name` stuck on the desktop with its corner at `at`, `size` large (DIPs: the page's it is stuck from;
    /// none, as large as a note is stuck) (asked from any thread: done on the panel's).
    pub fn stick_note(&self, name: String, at: POINT, size: Option<(f32, f32)>) {
        STUCK.lock().unwrap().push((name, OnDesk::Stick(at, size)));
        self.post(STICK_NOTES);
    }

    /// The note `name`'s card on the desktop taken away, where it is stuck (it was deleted).
    pub fn unstick_note(&self, name: String) {
        STUCK.lock().unwrap().push((name, OnDesk::Unstick));
        self.post(STICK_NOTES);
    }

    /// Glance going: what is written in notes stuck on the desktop, where they are (see `widget::InPlace`), kept on the
    /// panel's thread (or put aside), waited for a while.
    pub fn keep_writing(&self) {
        let sink = self.sink.load(Ordering::Acquire);
        if sink == 0 {
            return;
        }
        let (told, kept) = std::sync::mpsc::channel();
        *KEPT.lock().unwrap() = Some(told);
        self.post(KEEP_WRITING);
        let _ = kept.recv_timeout(Duration::from_secs(5));
    }

    /// The modules this machine has, in their default order.
    pub fn known_modules(&self) -> Vec<String> {
        // The game in front first, while there is one.
        let mut known = vec!["game".to_string(), "cpu".to_string()];
        known.extend(self.info().gpu_modules());
        known.extend(["memory", "network", "disk", "processes", "storage", "board", "battery", "system", "time", "lyrics", "wsl", "docker"].map(String::from));
        known
    }

    /// Runs the input loop, which also carries the panel. Raw input is
    /// read-only: unlike a mouse hook it cannot delay the pointer, and Windows
    /// never silently disconnects it.
    pub fn run<'s>(&'s self) {
        let sink = unsafe {
            let class = WNDCLASSW {
                lpfnWndProc: Some(sink_procedure),
                hInstance: GetModuleHandleW(None).expect("own module").into(),
                lpszClassName: w!("GlanceSink"),
                ..Default::default()
            };
            RegisterClassW(&class);
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("GlanceSink"),
                PCWSTR::null(),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
        }
        .expect("message window");
        // The mouse heard here only while there is no sensors' process to watch the edges (see `push`).
        let mut own_input = !crate::source::linked() && hear_mouse(sink);
        // In the process's multithreaded apartment (see lib.rs).
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        // The panels, each up at an edge or hidden to serve the next (see the top): the first made at once.
        let mut panels: Vec<Panel<'s>> = vec![Panel::new(self).expect("panel window")];
        // The edge a panel was last opened at, by where along it (on its screen): what the shortcut and the tray open
        // again (see `hand_target`).
        let mut last_place: Option<(POINT, Edge)> = None;
        // The panel a menu up was opened on, where it was opened on one.
        let mut menu_of: Option<usize> = None;
        // Made when first wanted; with no graphics device just then, tried
        // again at the next sample.
        let mut overlay: Option<Overlay> = None;
        let mut widgets = crate::widget::Widgets::default();
        let draw_overlay = |overlay: &mut Option<Overlay>, panels: &[Panel]| {
            let playing = self.playing();
            let wanted = self.overlay_wanted();
            if overlay.is_none() && wanted {
                *overlay = Overlay::new().ok();
            }
            let Some(overlay) = overlay else { return };
            let settings = self.config.lock().unwrap().overlay.clone();
            let history = self.history.read().unwrap();
            let beneath = panels.iter().find(|panel| panel.is_shown()).map(|panel| panel.window.hwnd);
            overlay.show(&history, &settings, panels[0].lang, playing, wanted, beneath);
        };
        self.sink.store(sink.0 as isize, Ordering::Release);
        // (A link made or lost as this thread began had no one to tell yet.)
        own_input = hear_as_linked(sink, own_input);

        // Once the panel has been up, the pointer has to leave the edge before
        // it can open the panel again: pushing on after it closed is the same
        // gesture, not a new one.
        let mut pusher = crate::push::Pusher::default();
        let mut ticking = false;
        // Escape closes the open panel: taken from every program while the
        // panel is open and not pinned (it has no keyboard focus of its own),
        // and given back as it closes or is pinned.
        let mut escape = false;
        // Ctrl+V taken (see `PASTE_HOTKEY`).
        let mut paste = false;
        // The preview's shortcut taken, as it was (see `PREVIEW_HOTKEY`), and the look's keys (see `LOOK_KEYS`); what a
        // look up from here shows, and from what, as that was last drawn.
        let mut preview: Option<crate::settings::Shortcut> = None;
        let mut look_keys = false;
        let mut look_back = false;
        let mut look_play = false;
        let mut look_shown: Option<(isize, u64)> = None;
        // A rest on the seam between two screens: since when, and where;
        // and, as on the edge, armed once the pointer has left it.
        let mut seam_rest: Option<(Instant, POINT)> = None;
        let mut seam_armed = true;
        // The screen the pointer was last on, and the one it last came
        // across from (see `Approach`).
        let mut approach = Approach::default();
        unsafe { SetTimer(Some(sink), WATCH_TIMER, WATCH_MS, None) };
        // What a frame in motion waits on: the screen's next refresh, or a
        // message, whichever comes first.
        // High resolution from Windows 10 1803 on; before, an ordinary one.
        let refresh = unsafe {
            CreateWaitableTimerExW(None, None, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0)
                .or_else(|_| CreateWaitableTimerExW(None, None, 0, TIMER_ALL_ACCESS.0))
        }
        .expect("frame timer");
        let left_out = self.config.lock().unwrap().panel_at;
        if let Some(at) = left_out {
            // A widget among the others as they are restored (and saved with
            // them); the panel's place forgotten.
            let layout = arrange::Choice { form: Form::Lanes { detail: Detail::Full, columns: 2 }, scale: 1.0, extra: (0.0, 0.0) }.kept();
            // Its corner, as near as can be told from its middle.
            let corner = (at.screen.0 - 356, at.screen.1 - 270);
            widgets.adopt(crate::settings::WidgetAt { at: corner, layout, pinned: at.locked, click_through: false, game_only: false, on_desktop: false, stuck: None, corner: None, size: None, parts: None, beside: false, before: None });
            std::thread::spawn(|| crate::app().change(|settings| settings.panel_at = None));
        }
        let mut restore = false;
        // How many times a widget had come to the top, as last seen (see `window::on_top`).
        let mut widgets_on_top = crate::ui::window::on_top();
        // How a thing on a station's lane was held, as the panels and widgets were last told to draw it.
        let mut tiles_drawn = crate::ui::view::tile_changes();
        // Pinned open at their edges as Glance last ran: opened so again once
        // there are readings to show.
        let mut pinned_at = self.config.lock().unwrap().panels_pinned.clone();
        // A menu up, and what it is for: run by this loop, as every other window of this thread is (see `menu::Menu`).
        let mut menu: Option<(crate::ui::menu::Menu, MenuFor)> = None;
        let mut closing_press = ClosingPress::default();
        // The windows in front the panel and the widgets give way to: the settings, the page beside.
        let mut in_front: (Option<HWND>, Option<HWND>) = (None, None);
        let mut msg = MSG::default();
        // In motion: when the next frame is due, with the screen's. Messages
        // that come before are taken as they come, not each followed by a
        // frame of its own.
        let mut frame_at: Option<Instant> = None;
        loop {
            // The panel, while it is up, and the settings window, while it is
            // open, are drawn whenever they are due, between the messages
            // that arrived since; with neither, the loop sleeps until a
            // message comes.
            let got = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool();
            // What notes written in where they are took (the messages sent to their windows as this looked, or handed
            // on before it), seen to.
            for hwnd in crate::ui::keys::took() {
                widgets.written(hwnd);
                for panel in panels.iter_mut().filter(|panel| panel.window.hwnd == hwnd) {
                    panel.next_frame = Instant::now();
                }
            }
            // A note's writing menu asked for on its words (see `Widgets::take_writing_menu_asked`).
            if widgets.writing_menu_was_asked() {
                self.end_menu(&mut menu, None, &mut panels, &mut menu_of, &mut widgets, &mut overlay);
                menu = widgets.take_writing_menu_asked(panels[0].lang, panels.iter().any(Panel::is_shown)).map(|(open, name, choices)| (open, MenuFor::Note(name, choices)));
                keep_menu_above(&menu, &mut overlay);
            }
            if !got {
                let now = Instant::now();
                if frame_at.is_some_and(|at| now < at) {
                    // The refresh timer is set for it.
                    unsafe { MsgWaitForMultipleObjects(Some(&[refresh]), false, INFINITE, QS_ALLINPUT) };
                    continue;
                }
                frame_at = None;
                // A menu left for another window come to the front, or for the panel come since it opened (by its
                // shortcut, an edge).
                if menu.as_ref().is_some_and(|(open, _)| open.left() || (panels.iter().any(Panel::is_shown) && !open.over_the_panel())) {
                    self.end_menu(&mut menu, None, &mut panels, &mut menu_of, &mut widgets, &mut overlay);
                }
                // The picker of a to-do's time left as well.
                if crate::ui::todo::pick::left() {
                    end_pick(false, &mut panels, &mut widgets);
                }
                // The panels under what is in front of them: a menu up; the settings, but for one pressed since they came
                // in front (in use). Not under the page beside: over it, always (its window's room for its shadow lay
                // over the panel's edge, taking presses meant for the panel's buttons there; placed again above all as
                // they are drawn, a panel came over it and went under it by turns).
                let menu_up = menu.as_ref().map(|(open, _)| open.hwnd());
                for panel in panels.iter_mut() {
                    let give_way = menu_up.or(in_front.0.filter(|_| !panel.forward));
                    if panel.window.gives_way_to() != give_way {
                        panel.window.yield_to(give_way);
                    }
                }
                let mut wait: Option<Duration> = None;
                for (at, panel) in panels.iter_mut().enumerate() {
                    // A capture of the desktop behind the panel come back, up or
                    // not (see `collect_behind`).
                    panel.collect_behind(now);
                    panel.menu_up = menu.is_some() && menu_of == Some(at);
                    if panel.is_shown() {
                        panel.tick(now);
                        if panel.is_shown() {
                            // The pointer is followed at least this often.
                            let due = panel.next_frame.saturating_duration_since(now).min(Duration::from_millis(TICK_MS as u64));
                            wait = Some(wait.map_or(due, |wait| wait.min(due)));
                        }
                    }
                }
                if !panels.iter().any(Panel::is_shown) {
                    self.shown.store(false, Ordering::Relaxed);
                }
                {
                    let kept = self.config.lock().unwrap().widgets.clone();
                    let seen = self.seen.lock().unwrap().clone();
                    let history = self.history.read().unwrap();
                    let samples = crate::panel::contiguous(&history);
                    if !samples.is_empty() {
                        let look = crate::widget::Look { prefs: &panels[0].prefs, skin: panels[0].skin, lang: panels[0].lang, live: panels[0].live };
                        if let Some(due) = widgets.tick(now, &look, &kept, samples, &seen, self) {
                            wait = Some(wait.map_or(due, |wait: Duration| wait.min(due)));
                        }
                    }
                    // A note pressed on a panel to be pulled off it, notes carried off: drawn so on the panels at once.
                    let pressed = panels.iter().find_map(|panel| panel.note_press.as_ref()).map(|(name, ..)| crate::notes::number_of(name));
                    if crate::ui::view::press_note(pressed) | crate::ui::view::carry_notes(widgets.carried()) {
                        for panel in panels.iter_mut().filter(|panel| panel.is_shown()) {
                            panel.next_frame = now;
                        }
                        wait = Some(Duration::ZERO);
                    }
                    // A thing dragged off a station's lane, carried (see `carry`); a thing on one pressed, carried off
                    // it, put back: drawn so on the panels and widgets at once, and as it springs back to its size.
                    if let Some(due) = crate::carry::tick(now) {
                        wait = Some(wait.map_or(due, |wait: Duration| wait.min(due)));
                    }
                    let pressed = panels.iter().find_map(|panel| panel.shelf_press.map(|(hit, _)| hit)).or_else(|| widgets.shelf_pressed());
                    crate::ui::view::press_tile(pressed);
                    let tiles = crate::ui::view::tile_changes();
                    if std::mem::replace(&mut tiles_drawn, tiles) != tiles || crate::ui::view::tile_landing() {
                        for panel in panels.iter_mut().filter(|panel| panel.is_shown()) {
                            panel.next_frame = now;
                        }
                        widgets.draw_soon();
                        wait = Some(Duration::ZERO);
                    }
                    // A widget come to the top over the open panel (giving way to the settings and back, shown, torn
                    // off): the panel over it again.
                    let on_top = crate::ui::window::on_top();
                    if on_top != widgets_on_top {
                        widgets_on_top = on_top;
                        for panel in panels.iter().filter(|panel| panel.is_shown()) {
                            panel.window.stand_again();
                        }
                    }
                    self.widget_put_away.store(widgets.any_put_away(), Ordering::Relaxed);
                    self.widget_processes.store(widgets.any_showing_processes(), Ordering::Relaxed);
                    self.set_seen(&self.widgets_seen, widgets.any_seen());
                }
                if !pinned_at.is_empty() && !self.history.read().unwrap().is_empty() {
                    for (point, edge) in std::mem::take(&mut pinned_at) {
                        let point = POINT { x: point.0, y: point.1 };
                        // On its screen, or the nearest, if it is gone; at its edge, each its own panel (one up there
                        // already, as two were kept for one edge, left as it is).
                        if let Some(contact) = monitor_at(point) {
                            let edge = panels[0].pinned_edge(point, contact, edge);
                            if panels.iter().any(|panel| panel.at(contact.monitor, edge)) {
                                continue;
                            }
                            let at = panel_for(&mut panels, self, contact.monitor, edge);
                            panels[at].open(point, contact, edge, None, false, now);
                            panels[at].pinned = panels[at].is_open();
                            last_place = Some((point, edge));
                        }
                    }
                }
                for panel in panels.iter_mut() {
                    // Torn off, the widget now on the screen in its place (it may
                    // wait on the desktop behind it first): the panel goes.
                    if panel.torn && widgets.torn_shown() {
                        panel.torn = false;
                        panel.dismiss();
                    } else if panel.torn && !widgets.torn_waiting() {
                        // The widget gone before it showed: the panel is itself again.
                        panel.torn = false;
                        panel.exclude();
                    }
                }
                match wait {
                    // In motion: the next frame comes with the screen's, and
                    // the wait for it never outlasts one (DwmFlush, which
                    // waits for the desktop to be composed again, waits as
                    // long as nothing on screen changes), nor holds back a
                    // message.
                    Some(wait) if wait.is_zero() => unsafe {
                        // From now, the frame drawn: the deadline and the
                        // timer for the same moment.
                        let until = until_refresh();
                        frame_at = Some(Instant::now() + until);
                        let due = -((until.as_nanos() / 100) as i64).max(1);
                        let _ = SetWaitableTimer(refresh, &due, 0, None, None, false);
                        MsgWaitForMultipleObjects(Some(&[refresh]), false, INFINITE, QS_ALLINPUT);
                    },
                    Some(wait) => unsafe {
                        // (In whole milliseconds, rounded up: what is less than one would be none, and the loop would
                        // go round without waiting till it is due.)
                        MsgWaitForMultipleObjects(None, false, wait.as_micros().div_ceil(1000) as u32, QS_ALLINPUT);
                    },
                    None => unsafe {
                        let _ = WaitMessage();
                    },
                }
                continue;
            }
            if msg.message == WM_QUIT {
                if let Some((open, _)) = menu.take() {
                    open.close();
                }
                return;
            }
            // A note written in where it is: its keys, the characters they make, the input method's, its timers, its
            // (see `ui::keys`).
            if crate::ui::keys::queued(&msg) {
                for hwnd in crate::ui::keys::took() {
                    widgets.written(hwnd);
                    for panel in panels.iter_mut().filter(|panel| panel.window.hwnd == hwnd) {
                        panel.next_frame = Instant::now();
                    }
                }
                continue;
            }
            // A press on the panel, or while its menu is up (which takes every press: one away from it only closes
            // it): one of its own, not one elsewhere (see `Panel::track`).
            // The panel whose window the message is for, where it is one's.
            let owner = panels.iter().position(|panel| panel.window.hwnd == msg.hwnd);
            if matches!(msg.message, windows::Win32::UI::WindowsAndMessaging::WM_LBUTTONDOWN | windows::Win32::UI::WindowsAndMessaging::WM_RBUTTONDOWN | windows::Win32::UI::WindowsAndMessaging::WM_MBUTTONDOWN)
                && (owner.is_some() || menu.as_ref().is_some_and(|(open, _)| open.owns(msg.hwnd)) || crate::ui::todo::pick::owns(msg.hwnd))
            {
                OURS.fetch_add(1, Ordering::Relaxed);
            }
            if closing_press.takes(msg.message) {
                continue;
            }
            // The picker of a to-do's time, as a menu: its own messages, a shortcut leaving it (Esc only that, putting it
            // away as its own Esc would).
            if crate::ui::todo::pick::is_up() {
                use crate::ui::todo::pick::{self, Handled as Picked};
                let esc = msg.hwnd.is_invalid() && msg.message == WM_HOTKEY && msg.wParam.0 == ESCAPE_HOTKEY as usize;
                let handled = if pick::owns(msg.hwnd) {
                    pick::handle(&msg)
                } else if matches!(msg.message, WM_HOTKEY | TOGGLE | OPEN_FROM_TRAY) {
                    Picked::Done { asked: esc }
                } else {
                    Picked::Not
                };
                match handled {
                    Picked::Done { asked } => {
                        closing_press.closed_by(msg.message);
                        end_pick(asked, &mut panels, &mut widgets);
                        if !matches!(msg.message, WM_HOTKEY | TOGGLE | OPEN_FROM_TRAY) || esc {
                            continue;
                        }
                    }
                    Picked::Chosen => {
                        for panel in panels.iter_mut().filter(|panel| panel.is_shown()) {
                            panel.next_frame = Instant::now();
                        }
                        widgets.draw_soon();
                        continue;
                    }
                    Picked::Taken => continue,
                    Picked::Not => {}
                }
            }
            // A menu's own messages; a shortcut leaves it (Esc, the panel's while it is open, is the menu's own: the
            // panel stays), the rest are taken up as ever. (The panel's own shortcut comes as `TOGGLE`.)
            if let Some((open, _)) = menu.as_mut() {
                use crate::ui::menu::Handled;
                let handled = if open.owns(msg.hwnd) {
                    open.handle(&msg)
                } else if matches!(msg.message, WM_HOTKEY | TOGGLE | OPEN_FROM_TRAY) {
                    Handled::Done(None)
                } else {
                    Handled::Not
                };
                match handled {
                    Handled::Done(choice) => {
                        closing_press.closed_by(msg.message);
                        self.end_menu(&mut menu, choice, &mut panels, &mut menu_of, &mut widgets, &mut overlay);
                        // (A shortcut, the panel's too, does what it is for as well; Esc only leaves the menu.)
                        let shortcut = matches!(msg.message, WM_HOTKEY | TOGGLE | OPEN_FROM_TRAY);
                        if !shortcut || (msg.hwnd.is_invalid() && msg.wParam.0 == ESCAPE_HOTKEY as usize) {
                            continue;
                        }
                    }
                    Handled::Taken => continue,
                    Handled::Not => {}
                }
            }
            let now = Instant::now();
            let over_fullscreen = self.config.lock().unwrap().over_fullscreen;
            // How the edges of a screen open the panel.
            let lit = |monitor: RECT| {
                let config = self.config.lock().unwrap();
                crate::screens::lit(&config.edges.places, monitor)
            };
            let (fullscreen_hotkey, fullscreen_edge) = match over_fullscreen {
                OverFullscreen::Never => (false, false),
                OverFullscreen::Shortcut => (true, false),
                OverFullscreen::Both => (true, true),
            };
            // Over a fullscreen game (one in exclusive fullscreen the panel
            // sends to the background; a push into the edge may be an
            // accident in either) it opens only as the settings allow.
            let may_open = |allowed: bool| allowed || !fullscreen_game();
            // A push into the edge, seen here or by the sensors' process: the
            // panel opens, unless over a game the settings keep it from (and
            // then not again till the pointer has left the edge).
            // (That edge's panel: the one up there already, which stays as it is, else one opened there.)
            let open_pushed = |(cursor, contact, edge): crate::push::Pushed, panels: &mut Vec<Panel<'s>>, last_place: &mut Option<(POINT, Edge)>| {
                if may_open(fullscreen_edge) {
                    let at = panel_for(panels, self, contact.monitor, edge);
                    if !panels[at].is_open() {
                        panels[at].open(cursor, contact, edge, None, true, now);
                        *last_place = Some((cursor, edge));
                    }
                }
            };
            // The pointer resting on the seam, where that edge's panel opens.
            let mut on_seam = |cursor: POINT, panels: &mut Vec<Panel<'s>>, last_place: &mut Option<(POINT, Edge)>| {
                let here = monitor_at(cursor);
                approach.moved(cursor, here.map(|contact| (contact.monitor, contact.scale)), now);
                let here = here.map(|contact| contact.monitor);
                let came_from = approach.came_from(now);
                // On a seam that is an edge that opens the panel, of the
                // screen it came from before the one it is on (it was
                // making for that screen's edge and went past it), before
                // the others.
                // (No seam is looked for along an edge that opens the panel
                // on no screen: this is asked at every move of the mouse.)
                let sought = {
                    let config = self.config.lock().unwrap();
                    EDGES.map(|edge| config.edges.places.anywhere(edge))
                };
                let found = EDGES
                    .into_iter()
                    .zip(sought)
                    .filter_map(|(edge, sought)| sought.then_some(edge))
                    .filter_map(|edge| seam_contact(cursor, edge, came_from).filter(|contact| lit(contact.monitor).has(edge)).map(|contact| (contact, edge)))
                    .min_by_key(|(contact, _)| (Some(contact.monitor) != came_from, Some(contact.monitor) != here));
                let Some((contact, edge)) = found else {
                    seam_rest = None;
                    seam_armed = true;
                    return;
                };
                // (Its panel up already: it stays as it is.)
                if !seam_armed || panels.iter().any(|panel| panel.at(contact.monitor, edge)) {
                    return;
                }
                let still = (SEAM_STILL * contact.scale).round() as i32;
                let since = match seam_rest {
                    Some((since, at)) if (at.x - cursor.x).abs() <= still && (at.y - cursor.y).abs() <= still => since,
                    _ => seam_rest.insert((now, cursor)).0,
                };
                if now.duration_since(since) >= SEAM_DWELL {
                    seam_rest = None;
                    if may_open(fullscreen_edge) {
                        let at = panel_for(panels, self, contact.monitor, edge);
                        panels[at].open(cursor, contact, edge, None, true, now);
                        *last_place = Some((cursor, edge));
                    }
                    seam_armed = false;
                }
            };
            match msg.message {
                // The shortcut and the tray: the panel of the edge last opened (see `hand_target`), opened, or (the
                // shortcut) closed where it is up.
                OPEN_FROM_TRAY | TOGGLE => {
                    crate::perf::mark(|| "panel asked".into());
                    let at = self.hand_target(&mut panels, last_place);
                    if panels[at].is_open() {
                        if msg.message == TOGGLE {
                            panels[at].begin_close(now);
                        }
                    } else if msg.message == OPEN_FROM_TRAY || may_open(fullscreen_hotkey) {
                        self.open_by_hand(&mut panels[at], last_place, now);
                        last_place = panels[at].place().or(last_place);
                    }
                }
                // A panel that stays (pinned, or moved away from the edge)
                // stays, the settings window coming up or not.
                DISMISS => {
                    for panel in panels.iter_mut().filter(|panel| !panel.stays()) {
                        panel.dismiss();
                    }
                }
                WM_HOTKEY if msg.hwnd.is_invalid() && msg.wParam.0 == PASTE_HOTKEY as usize => crate::station::run::ask(crate::station::run::Ask::Paste),
                // The preview's shortcut: the look up from here gone back into its tile, else what is under the pointer
                // looked at larger.
                WM_HOTKEY if msg.hwnd.is_invalid() && msg.wParam.0 == PREVIEW_HOTKEY as usize => {
                    if looking_here(&panels, &widgets) {
                        crate::ui::look::key(crate::ui::look::Key::Close);
                    } else if let Some((window, hit)) = looked_under(&panels, &widgets) {
                        look_at(&panels, &widgets, window, hit);
                    }
                }
                WM_HOTKEY if msg.hwnd.is_invalid() && LOOK_KEYS.iter().any(|(id, ..)| msg.wParam.0 == *id as usize) => {
                    if let Some((_, key, ..)) = LOOK_KEYS.iter().find(|(id, ..)| msg.wParam.0 == *id as usize) {
                        crate::ui::look::key(*key);
                    }
                }
                WM_HOTKEY if msg.hwnd.is_invalid() && msg.wParam.0 == LOOK_BACK_HOTKEY as usize => crate::ui::look::key(crate::ui::look::Key::Back),
                WM_HOTKEY if msg.hwnd.is_invalid() && msg.wParam.0 == LOOK_PLAY_HOTKEY as usize => crate::ui::look::key(crate::ui::look::Key::Toggle),
                // Esc: the music full screen left, over all else (and nothing more); a thing kept at hand dragged put
                // back where it was (and nothing more); else the panels that go, gone.
                // (A look up from here: back into its tile, and nothing more.)
                WM_HOTKEY if msg.hwnd.is_invalid() && msg.wParam.0 == ESCAPE_HOTKEY as usize => {
                    if looking_here(&panels, &widgets) {
                        crate::ui::look::key(crate::ui::look::Key::Close);
                    } else if panels.iter().any(|panel| crate::ui::todo::writing(panel.window.hwnd)) {
                        // A to-do being written on it: the keys given back, and nothing more.
                        for panel in panels.iter_mut() {
                            panel.stop_todo(true);
                        }
                    } else if widgets.full().is_some() {
                        widgets.leave_full();
                    } else if panels.iter().any(Panel::dragging_app) {
                        for panel in panels.iter_mut().filter(|panel| panel.dragging_app()) {
                            panel.app_press = None;
                            panel.app_over = None;
                            panel.next_frame = now;
                            crate::ui::window::let_go_of_capture(panel.window.hwnd);
                        }
                    } else {
                        for panel in panels.iter_mut().filter(|panel| panel.is_open() && !panel.stays()) {
                            panel.begin_close(now);
                        }
                    }
                }
                // Placed, zoomed and backed for screens that are no longer
                // so: taken down, and opened afresh.
                // The panels' own (the overlay's follows its game's screen); what is shared seen to once, as the first
                // panel's window is told.
                crate::ui::window::SCREENS_CHANGED if owner.is_some() => {
                    let at = owner.unwrap();
                    if at == 0 {
                        crate::screens::changed();
                        // (What is hidden on a card no screen needs any longer let go of, so that none keeps it awake:
                        // the overlay and the tip made again as they are next shown, the widgets' and the panels' on
                        // their screens' devices, the beside page's thread told.)
                        if msg.wParam.0 == 0 {
                            if overlay.as_ref().is_some_and(Overlay::hidden_on_a_card_not_needed) {
                                overlay = None;
                            }
                            crate::ui::tip::screens_changed();
                            crate::ui::beside::screens_changed();
                        }
                        widgets.screens_changed();
                    }
                    if let Some(again) = panels[at].screens_changed(msg.wParam.0 as u32, now) {
                        again.open(&mut panels, at, now);
                    }
                }
                crate::ui::window::SCREENS_CHANGED if widgets.owns(msg.hwnd) || widgets.full_owns(msg.hwnd) => widgets.screens_changed(),
                crate::widget::CAPTURED if widgets.owns(msg.hwnd) => widgets.captured(msg.hwnd),
                // Something dragged over where a page of the workbench is chosen: that page shown.
                crate::station::run::SHOW_PAGE if owner.is_some() => {
                    let panel = &mut panels[owner.unwrap()];
                    if let Some(page) = view::Tab::numbered(msg.wParam.0).filter(|page| *page != panel.tab.page()) {
                        panel.tab = page;
                        panel.next_frame = now;
                    }
                }
                crate::station::run::SHOW_PAGE if widgets.owns(msg.hwnd) => {
                    if let Some(page) = view::Tab::numbered(msg.wParam.0) {
                        widgets.show_page(msg.hwnd, page);
                    }
                }
                OPEN_SETTINGS => settings_window::open(crate::ui::genie::of_told(msg.lParam.0)),
                STICK_NOTES => {
                    for (name, asked) in std::mem::take(&mut *STUCK.lock().unwrap()) {
                        match asked {
                            OnDesk::Stick(at, size) => widgets.stick_note(&name, at, size),
                            OnDesk::Unstick => widgets.unstick_note(&name),
                        }
                    }
                }
                // A note written in where it is giving back the keys (Esc, another window in front).
                crate::ui::keys::LEFT if widgets.owns(msg.hwnd) => widgets.keys_left(msg.hwnd, msg.wParam.0 == 1),
                // A to-do written on the panel giving back the keys (Esc, another window in front: the picker of its
                // time, which writes it in again as it goes).
                crate::ui::keys::LEFT if owner.is_some() => panels[owner.unwrap()].stop_todo(msg.wParam.0 == 1),
                crate::ui::todo::RESUME if owner.is_some() => panels[owner.unwrap()].write_todo(),
                crate::ui::todo::RESUME if widgets.owns(msg.hwnd) => widgets.write_todo(msg.hwnd),
                // (Alt+F4 on a note written in where it is: as Esc. Other windows of Glance's are not closed so.)
                crate::ui::window::CLOSE_ASKED if widgets.owns(msg.hwnd) => widgets.keys_left(msg.hwnd, true),
                KEEP_WRITING => {
                    widgets.keep_as_glance_goes();
                    if let Some(told) = KEPT.lock().unwrap().take() {
                        let _ = told.send(());
                    }
                }
                TRAY_MENU => {
                    let (x, y) = ((msg.lParam.0 & 0xFFFF) as u16 as i16 as i32, ((msg.lParam.0 >> 16) & 0xFFFF) as u16 as i16 as i32);
                    self.end_menu(&mut menu, None, &mut panels, &mut menu_of, &mut widgets, &mut overlay);
                    menu = crate::tray::open_menu(x, y, panels.iter().any(Panel::is_shown)).map(|(open, choices)| (open, MenuFor::Tray(choices)));
                    keep_menu_above(&menu, &mut overlay);
                }
                RECALL_WIDGET => {
                    widgets.recall();
                }
                LYRIC => {
                    for panel in panels.iter_mut() {
                        panel.next_frame = now;
                    }
                    widgets.draw_soon();
                }
                // The settings, or the page beside, in front or no longer: the panel gives way to the settings, the
                // widgets to the page while it is up, else to the settings.
                SETTINGS_FRONT => {
                    let front = (msg.wParam.0 != 0).then(|| HWND(msg.wParam.0 as *mut _));
                    if msg.lParam.0 == 0 {
                        in_front.0 = front;
                        for panel in panels.iter_mut() {
                            // (Come in front again: given way to again, as it was pressed before.)
                            if front.is_some() {
                                panel.forward = false;
                            }
                            panel.settings_front(front);
                        }
                    } else {
                        in_front.1 = front;
                        // (A page come up: the panels over it, as they stay.)
                        if front.is_some() {
                            for panel in panels.iter().filter(|panel| panel.is_shown()) {
                                panel.window.stand_again();
                            }
                        }
                    }
                    widgets.settings_front(in_front.1.or(in_front.0));
                }
                RESTYLE => {
                    for panel in panels.iter_mut() {
                        panel.restyle();
                    }
                    if let Some(panel) = panels.first() {
                        crate::ui::beside::follow_skin(panel.skin);
                    }
                    draw_overlay(&mut overlay, &panels);
                    // An open settings window takes what was changed elsewhere.
                    settings_window::follow_settings();
                }
                OVERLAY => {
                    // A panel pinned away from the edge when Glance last
                    // closed comes back with the first sample.
                    if restore {
                        restore = false;
                        let at = self.hand_target(&mut panels, last_place);
                        if !panels[at].is_shown() && may_open(fullscreen_hotkey) {
                            self.open_by_hand(&mut panels[at], last_place, now);
                        }
                    }
                    draw_overlay(&mut overlay, &panels);
                }
                // The pinned panel dragged by its bar.
                // (Pressed with the settings in front, it comes over them: in use, it was under them till a page it opened
                // took the front.)
                // (A look up from it, or a widget: a press on a thing on it that may be looked at shows that one, and does
                // nothing more, its button's coming up too. One elsewhere on it closes the look up from it, and is a press
                // as ever.)
                WM_LBUTTONDOWN if owner.is_some() => {
                    let panel = &panels[owner.unwrap()];
                    let window = panel.window.hwnd;
                    if looking_here(&panels, &widgets) {
                        match panel.hit_at(lparam_point(msg.lParam)).filter(|hit| hit.looks()) {
                            Some(hit) => {
                                look_at(&panels, &widgets, window, hit);
                                closing_press.closed_by(msg.message);
                                continue;
                            }
                            None if crate::ui::look::up_for(window) => crate::ui::look::close(),
                            None => {}
                        }
                    }
                    let panel = &mut panels[owner.unwrap()];
                    panel.forward = true;
                    panel.press(lparam_point(msg.lParam));
                }
                // A thing on its station, or a note, pressed: followed as the pointer moves over the panel, and wherever it
                // goes as it is followed (see `Panel::follow_press`).
                WM_MOUSEMOVE if owner.is_some_and(|at| panels[at].pressed_to_pull()) => {
                    let panel = &mut panels[owner.unwrap()];
                    if let Some(pulled) = panel.follow_press() {
                        widgets.stick_note_held(pulled);
                    }
                    panel.hover_at(lparam_point(msg.lParam));
                }
                // (After a press ended otherwise, let go over the panel: no longer followed.)
                WM_TIMER if msg.wParam.0 == crate::overlay::FOLLOW_TIMER && owner.is_some_and(|at| !panels[at].held()) => {
                    if let Some(pulled) = panels[owner.unwrap()].follow_press() {
                        widgets.stick_note_held(pulled);
                    }
                }
                crate::ui::window::CAPTURE_LOST if owner.is_some_and(|at| panels[at].note_press.is_some()) => panels[owner.unwrap()].note_press = None,
                // A thing kept at hand pressed and moved: put elsewhere among them as it is let go, over another.
                WM_MOUSEMOVE if owner.is_some_and(|at| panels[at].app_press.is_some()) => {
                    let panel = &mut panels[owner.unwrap()];
                    if let (Some((_, from, moved @ false)), Some(now)) = (panel.app_press.as_mut(), cursor_position()) {
                        if crate::station::run::drag_begins(*from, now) {
                            *moved = true;
                        }
                    }
                    // Moved: where the pointer has it, and the place it would go (the one it is over; between them,
                    // the last).
                    if panel.app_press.as_ref().is_some_and(|(_, _, moved)| *moved) {
                        panel.held_at(lparam_point(msg.lParam));
                    }
                    panel.hover_at(lparam_point(msg.lParam));
                }
                WM_LBUTTONUP | crate::ui::window::CAPTURE_LOST if owner.is_some_and(|at| panels[at].app_press.is_some()) => {
                    let panel = &mut panels[owner.unwrap()];
                    let (moving, _, moved) = panel.app_press.take().unwrap();
                    crate::ui::window::let_go_of_capture(panel.window.hwnd);
                    if moved {
                        panel.held_at(lparam_point(msg.lParam));
                    }
                    let over = panel.hit_at(lparam_point(msg.lParam));
                    // (Where it would go as it was let go: the place the others made room at, not only another's.)
                    let went = panel.app_over.take().filter(|_| moved);
                    // (By what opens it: where it is now, if it is kept still.)
                    let shown = crate::station::apps::shown();
                    let from = shown.iter().position(|app| app.target == moving);
                    match (msg.message == WM_LBUTTONUP, moved, went, from) {
                        (true, true, Some(to), Some(from)) if usize::from(to) != from => {
                            let to = usize::from(to).min(shown.len() - 1);
                            // (Dropped on a later one: after it; on an earlier one: before it.)
                            let before = shown.get(if to > from { to + 1 } else { to }).map(|app| app.target.clone());
                            panel.app_dropped = Some((moving.clone(), to, Instant::now()));
                            crate::station::apps::change(move |apps| crate::station::apps::put_before(apps, &moving, before.as_deref()));
                        }
                        (true, false, _, Some(_)) if over == Some(Hit::App(crate::station::apps::number_of(&moving))) => {
                            crate::station::pressed(Hit::App(crate::station::apps::number_of(&moving)));
                        }
                        _ => {}
                    }
                    panel.next_frame = now;
                }
                // (The pointer taken from it as a thing on its station was pressed: nothing pressed.)
                crate::ui::window::CAPTURE_LOST if owner.is_some_and(|at| panels[at].shelf_press.is_some()) => panels[owner.unwrap()].shelf_press = None,
                WM_TIMER if owner.is_some_and(|at| panels[at].held()) => {
                    let panel = &mut panels[owner.unwrap()];
                    panel.dragged(now);
                    if let Some((form, scale, at, grab, size, only)) = panel.tear.take() {
                        // The widget takes the desktop behind it before the
                        // panel goes: without the panel in it (it is put back
                        // in captures as it goes).
                        panel.exclude();
                        // The panel's desktop and its look, lent to the widget
                        // till it has taken its own.
                        // (Given, not shared: the panel writes no later capture into it.)
                        let lent = panel.behind.as_mut().and_then(|behind| {
                            Some(crate::widget::Lent { capture: behind.capture.without_pixels(), bitmap: behind.bitmap.take()?, tone: behind.tone })
                        });
                        // The edge it was at, unless it had been moved away.
                        let from = panel.placement.as_ref().filter(|placement| placement.floating.is_none()).map(|_| panel.edge);
                        if !widgets.tear(form, scale, at, grab, size, lent, from, panel.part, only) {
                            panel.torn = false;
                            panel.exclude();
                        }
                    }
                    if let Some((size, at, grab)) = panel.tear_music.take() {
                        widgets.tear_music(size, at, grab);
                    }
                }
                // The music full screen: its keys (Esc leaves it), presses (a double click leaves it), the pointer
                // (bringing its controls) and the wheel (the volume).
                WM_KEYDOWN if widgets.full_owns(msg.hwnd) => {
                    if widgets.full().is_some_and(|full| full.key(msg.wParam.0 as u16) == crate::fullscreen::Asked::Leave) {
                        widgets.leave_full();
                    }
                }
                WM_LBUTTONDOWN if widgets.full_owns(msg.hwnd) => {
                    let at = lparam_point(msg.lParam);
                    if widgets.full().is_some_and(|full| full.press((at.x, at.y)) == crate::fullscreen::Asked::Leave) {
                        widgets.leave_full();
                    }
                }
                WM_LBUTTONUP if widgets.full_owns(msg.hwnd) => {
                    if let Some(full) = widgets.full() {
                        full.release();
                    }
                }
                crate::ui::window::CAPTURE_LOST if widgets.full_owns(msg.hwnd) => {
                    if let Some(full) = widgets.full() {
                        full.lost();
                    }
                }
                WM_MOUSEMOVE if widgets.full_owns(msg.hwnd) => {
                    let at = lparam_point(msg.lParam);
                    if let Some(full) = widgets.full() {
                        full.moved((at.x, at.y));
                    }
                }
                WM_MOUSEWHEEL if widgets.full_owns(msg.hwnd) => {
                    if let Some(full) = widgets.full() {
                        full.wheel((msg.wParam.0 >> 16) as u16 as i16);
                    }
                }
                // A look up from a widget, or the panel: a press on a thing on a widget that may be looked at shows that
                // one, and does nothing more, its button's coming up too. One elsewhere on it goes on as a press (below),
                // the look up from it closed.
                WM_LBUTTONDOWN if widgets.owns(msg.hwnd) && looking_here(&panels, &widgets) && widgets.hit_at(msg.hwnd, lparam_point(msg.lParam)).is_some_and(|hit| hit.looks()) => {
                    if let Some(hit) = widgets.hit_at(msg.hwnd, lparam_point(msg.lParam)) {
                        look_at(&panels, &widgets, msg.hwnd, hit);
                        closing_press.closed_by(msg.message);
                    }
                }
                // A note stuck on the desktop written in where it is, or to be: the pointer's on its words, and on its
                // menu's button (see `Widgets::write`).
                WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MOUSEMOVE | WM_MOUSEWHEEL | windows::Win32::UI::WindowsAndMessaging::WM_RBUTTONDOWN | WM_RBUTTONUP if widgets.owns(msg.hwnd) && widgets.write(&msg) => {
                    if let Some(number) = widgets.take_note_menu_asked() {
                        self.end_menu(&mut menu, None, &mut panels, &mut menu_of, &mut widgets, &mut overlay);
                        menu = widgets.note_menu(msg.hwnd, number, panels[0].lang, panels.iter().any(Panel::is_shown)).map(|(open, name, choices)| (open, MenuFor::Note(name, choices)));
                        keep_menu_above(&menu, &mut overlay);
                    }
                }
                // The desktop widgets: pressed, let go (or found let go as the pointer moves: the button up unheard,
                // let go before it was torn off), their menu.
                WM_LBUTTONDOWN if widgets.owns(msg.hwnd) => widgets.press(msg.hwnd, lparam_point(msg.lParam)),
                WM_LBUTTONUP if widgets.owns(msg.hwnd) => {
                    widgets.release(msg.hwnd, now, false);
                    // A button of its station's head: its menu.
                    if let Some((hit, tab)) = widgets.take_menu_asked() {
                        self.end_menu(&mut menu, None, &mut panels, &mut menu_of, &mut widgets, &mut overlay);
                        menu = cursor_position().and_then(|at| crate::station::head_menu(hit, tab, at, panels.iter().any(Panel::is_shown), widgets.look_of(msg.hwnd))).map(|(open, choices)| (open, MenuFor::StationHead(choices, Some(msg.hwnd))));
                        keep_menu_above(&menu, &mut overlay);
                    }
                }
                // Its capture taken (a UAC prompt, the lock screen, another program): no press completed, nothing
                // pressed is done.
                crate::ui::window::CAPTURE_LOST if widgets.owns(msg.hwnd) => widgets.release(msg.hwnd, now, true),
                WM_MOUSEMOVE if widgets.owns(msg.hwnd) && widgets.held(msg.hwnd) && let_go(&msg) => widgets.release(msg.hwnd, now, false),
                WM_MOUSEMOVE if widgets.owns(msg.hwnd) => widgets.hover(msg.hwnd, lparam_point(msg.lParam), panels[0].lang),
                WM_RBUTTONUP if widgets.owns(msg.hwnd) => {
                    self.end_menu(&mut menu, None, &mut panels, &mut menu_of, &mut widgets, &mut overlay);
                    // On a thing on the station, its menu; else the widget's.
                    let panel_shown = panels.iter().any(Panel::is_shown);
                    menu = match widgets.hit_at(msg.hwnd, lparam_point(msg.lParam)) {
                        Some(Hit::Shelf(id) | Hit::Copied(id)) => cursor_position().and_then(|at| crate::station::menu(id, at, panel_shown, widgets.look_of(msg.hwnd))).map(|(open, choices)| (open, MenuFor::Station(id, choices, Some(msg.hwnd)))),
                        Some(Hit::Recent(file)) => cursor_position().and_then(|at| crate::station::recent_menu(file, at, panel_shown, widgets.look_of(msg.hwnd))).map(|(open, choices)| (open, MenuFor::Recent(file, choices))),
                        Some(Hit::App(at)) => cursor_position().and_then(|point| crate::station::app_menu(at, point, panel_shown, widgets.look_of(msg.hwnd))).map(|(open, choices, app)| (open, MenuFor::App(app, choices))),
                        // A note stuck on the desktop, its card: the note's menu (its colours, taken off, taken away).
                        Some(Hit::Note(number) | Hit::Words(number)) => widgets.note_menu(msg.hwnd, number, panels[0].lang, panel_shown).map(|(open, name, choices)| (open, MenuFor::Note(name, choices))).or_else(|| widgets.menu(msg.hwnd, panels[0].lang, panel_shown).map(|open| (open, MenuFor::Widget(msg.hwnd)))),
                        _ => widgets.menu(msg.hwnd, panels[0].lang, panel_shown).map(|open| (open, MenuFor::Widget(msg.hwnd))),
                    };
                    keep_menu_above(&menu, &mut overlay);
                }
                // On a thing on the station in the panel, its menu.
                WM_RBUTTONUP if owner.is_some() => {
                    let hit = panels[owner.unwrap()].hit_at(lparam_point(msg.lParam));
                    let look = panels[owner.unwrap()].menu_look();
                    if matches!(hit, Some(Hit::Shelf(_) | Hit::Copied(_) | Hit::App(_) | Hit::Recent(_) | Hit::Note(_))) {
                        // (Before `end_menu`, which forgets whose the menu up was.)
                        self.end_menu(&mut menu, None, &mut panels, &mut menu_of, &mut widgets, &mut overlay);
                        menu_of = owner;
                    }
                    match hit {
                        Some(Hit::Shelf(id) | Hit::Copied(id)) => {
                            menu = cursor_position().and_then(|at| crate::station::menu(id, at, true, look)).map(|(open, choices)| (open, MenuFor::Station(id, choices, None)));
                            keep_menu_above(&menu, &mut overlay);
                        }
                        Some(Hit::App(at)) => {
                            menu = cursor_position().and_then(|point| crate::station::app_menu(at, point, true, look)).map(|(open, choices, app)| (open, MenuFor::App(app, choices)));
                            keep_menu_above(&menu, &mut overlay);
                        }
                        Some(Hit::Recent(file)) => {
                            menu = cursor_position().and_then(|at| crate::station::recent_menu(file, at, true, look)).map(|(open, choices)| (open, MenuFor::Recent(file, choices)));
                            keep_menu_above(&menu, &mut overlay);
                        }
                        // A note's card: the note's menu, as its card on the desktop has it.
                        Some(Hit::Note(number)) => {
                            menu = widgets.menu_of_note(number, None, panels[0].lang, look, true).map(|(open, name, choices)| (open, MenuFor::Note(name, choices)));
                            keep_menu_above(&menu, &mut overlay);
                        }
                        _ => {}
                    }
                }
                // Let go: the button up, the capture lost, or the pointer moved with the button up (its going up
                // told to another window).
                WM_LBUTTONUP | crate::ui::window::CAPTURE_LOST | WM_MOUSEMOVE
                    if owner.is_some_and(|at| panels[at].drag.is_some()) && (msg.message != WM_MOUSEMOVE || let_go(&msg)) =>
                {
                    let panel = &mut panels[owner.unwrap()];
                    // Kept here, in the order things happened (an unpin just
                    // after stands).
                    let tapped = msg.message == WM_LBUTTONUP && panel.tapped();
                    let held_drag = panel.drag;
                    let kept = panel.let_go(now);
                    // Let go where it was pressed: a tap on what was pressed (a line of what plays).
                    if tapped {
                        panel.click(lparam_point(msg.lParam));
                    }
                    // What plays, sized: kept as large as it comes to rest, for the edge it is at (a side or the top),
                    // settling there from as large as the hand made it (see `music_settle`).
                    if let Some(room) = panel.sized_room {
                        let shows = crate::ui::music::Shows::from(&panel.controller.config.lock().unwrap().view);
                        let top = panel.edge == Edge::Top;
                        let to = crate::ui::music::rests_at(room, &shows);
                        crate::app().change(|settings| settings.music_rooms.set(top, to));
                        match held_drag {
                            Some(drag) if to != room => {
                                panel.music_settle = Some((now, drag, room, to));
                                panel.next_frame = now;
                            }
                            _ => {
                                panel.sized_room = None;
                                panel.arrange();
                            }
                        }
                    }
                    if std::mem::take(&mut panel.screens_missed) {
                        panel.screens_changed(0, now);
                    }
                    if let Some((at, size)) = kept.filter(|(at, size)| !matches!(at, Kept::Unchanged) || size.is_some()) {
                        // (Sized: for what it shows, on the screen it is on.)
                        let (part, screen) = (panel.part, panel.screen.clone());
                        crate::app().change(|settings| {
                            match at {
                                Kept::At(at) => settings.panel_at = Some(at),
                                Kept::Docked => settings.panel_at = None,
                                Kept::Unchanged => {}
                            }
                            if let Some(size) = size {
                                settings.size_panel(part, screen.as_deref(), size);
                            }
                        });
                    }
                }
                // The sound's track let go (or the button found up as the pointer moves, or the capture lost).
                WM_LBUTTONUP | crate::ui::window::CAPTURE_LOST | WM_MOUSEMOVE
                    if owner.is_some_and(|at| panels[at].sliding.is_some()) && (msg.message != WM_MOUSEMOVE || let_go(&msg)) =>
                {
                    // (The pointer taken from it mid-drag: given up, nowhere sought.)
                    panels[owner.unwrap()].slid(msg.message != crate::ui::window::CAPTURE_LOST)
                }
                // A list's bar's thumb let go, as the sound's track is.
                WM_LBUTTONUP | crate::ui::window::CAPTURE_LOST | WM_MOUSEMOVE
                    if owner.is_some_and(|at| panels[at].scrolling.is_some()) && (msg.message != WM_MOUSEMOVE || let_go(&msg)) =>
                {
                    panels[owner.unwrap()].let_go_of_thumb()
                }
                // Let go of the panel after a widget was torn off it (the button went up before the widget took
                // the pointer): the widget is let go.
                WM_LBUTTONUP if owner.is_some() && widgets.torn_in_hand().is_some() => {
                    let torn = widgets.torn_in_hand().unwrap();
                    widgets.release(torn, now, false)
                }
                // A note stuck on the desktop pressed in the panel: the one stuck shown, not opened a second time.
                WM_LBUTTONUP
                    if owner.is_some_and(|at| panels[at].hit_at(lparam_point(msg.lParam)).and_then(|hit| if let Hit::Note(number) = hit { crate::notes::named(number) } else { None }).is_some_and(|name| widgets.raise_note(&name))) =>
                {
                    panels[owner.unwrap()].dismiss();
                }
                // The station's head's buttons: their menus.
                WM_LBUTTONUP if owner.is_some_and(|at| panels[at].hit_at(lparam_point(msg.lParam)).is_some_and(Hit::opens_menu)) => {
                    let at = owner.unwrap();
                    let hit = panels[at].hit_at(lparam_point(msg.lParam)).unwrap();
                    panels[at].click(lparam_point(msg.lParam));
                    self.end_menu(&mut menu, None, &mut panels, &mut menu_of, &mut widgets, &mut overlay);
                    menu = cursor_position().and_then(|point| crate::station::head_menu(hit, panels[at].tab, point, true, panels[at].menu_look())).map(|(open, choices)| (open, MenuFor::StationHead(choices, None)));
                    menu_of = Some(at);
                    keep_menu_above(&menu, &mut overlay);
                }
                WM_LBUTTONUP if owner.is_some() => {
                    let panel = &mut panels[owner.unwrap()];
                    // (A note pressed and let go on the panel: opened, as a press on it is.)
                    if panel.note_press.take().is_some() {
                        crate::ui::window::let_go_of_capture(panel.window.hwnd);
                    }
                    panel.click(lparam_point(msg.lParam));
                    // Its music asked to be shown over the whole of its screen.
                    if let Some(contact) = panel.full_asked.take() {
                        widgets.open_full(contact.monitor, contact.scale);
                    }
                }
                // The overlay dragged into place.
                WM_LBUTTONDOWN if overlay.as_ref().is_some_and(|o| msg.hwnd == o.window.hwnd) => overlay.as_mut().unwrap().press(),
                // Let go: the button up, the capture lost, or the pointer moved with the button up.
                WM_LBUTTONUP | crate::ui::window::CAPTURE_LOST | WM_MOUSEMOVE
                    if overlay.as_ref().is_some_and(|o| msg.hwnd == o.window.hwnd && o.held()) && (msg.message != WM_MOUSEMOVE || let_go(&msg)) =>
                {
                    if let Some(placed) = overlay.as_mut().unwrap().release() {
                        std::thread::spawn(move || crate::app().place_overlay(placed));
                    }
                }
                WM_MOUSEMOVE if overlay.as_ref().is_some_and(|o| msg.hwnd == o.window.hwnd) => overlay.as_mut().unwrap().moved(),
                WM_TIMER if msg.wParam.0 == crate::overlay::SAID_TIMER && overlay.as_ref().is_some_and(|o| msg.hwnd == o.window.hwnd) => overlay.as_mut().unwrap().hush(),
                // Followed while dragged, wherever the pointer is.
                WM_TIMER if msg.wParam.0 == crate::overlay::FOLLOW_TIMER && overlay.as_ref().is_some_and(|o| msg.hwnd == o.window.hwnd && o.held()) => {
                    overlay.as_mut().unwrap().moved()
                }
                WM_MOUSEMOVE if overlay.as_ref().is_some_and(|o| o.owns_bar(msg.hwnd)) => overlay.as_mut().unwrap().bar_hover(),
                // Its menu, or one of its buttons: the same is done (see `overlay_chosen`).
                WM_RBUTTONUP if overlay.as_ref().is_some_and(|o| msg.hwnd == o.window.hwnd) => {
                    self.end_menu(&mut menu, None, &mut panels, &mut menu_of, &mut widgets, &mut overlay);
                    menu = overlay.as_ref().unwrap().menu(panels.iter().any(Panel::is_shown)).map(|open| (open, MenuFor::Overlay));
                    keep_menu_above(&menu, &mut overlay);
                }
                WM_LBUTTONUP if overlay.as_ref().is_some_and(|o| o.owns_bar(msg.hwnd)) => {
                    if let Some((choice, place)) = overlay.as_mut().unwrap().bar_press() {
                        self.overlay_chosen(choice, Some(place), &mut panels);
                    }
                }
                WM_MOUSEMOVE if owner.is_some() => panels[owner.unwrap()].hover_at(lparam_point(msg.lParam)),
                // The wheel over the overlay: its glass clearer (up) or
                // less clear, a step of the settings' slider a notch.
                WM_MOUSEWHEEL if overlay.as_ref().is_some_and(|o| msg.hwnd == o.window.hwnd) => {
                    let notches = (msg.wParam.0 >> 16) as u16 as i16 as f32 / 120.0;
                    std::thread::spawn(move || crate::app().clear_overlay(notches * 0.05));
                }
                WM_MOUSEWHEEL if widgets.owns(msg.hwnd) => widgets.wheel(msg.hwnd, lparam_point(msg.lParam), (msg.wParam.0 >> 16) as u16 as i16, false),
                WM_MOUSEHWHEEL if widgets.owns(msg.hwnd) => widgets.wheel(msg.hwnd, lparam_point(msg.lParam), (msg.wParam.0 >> 16) as u16 as i16, true),
                WM_MOUSEWHEEL if owner.is_some() => {
                    let delta = (msg.wParam.0 >> 16) as u16 as i16;
                    panels[owner.unwrap()].wheel(lparam_point(msg.lParam), delta);
                }
                // A push the sensors' process saw (it watches the edges while
                // there is one).
                PUSHED => {
                    let (edge, cursor) = pushed_from(msg.wParam, msg.lParam);
                    if let Some(contact) = monitor_at(cursor) {
                        open_pushed((cursor, contact, edge), &mut panels, &mut last_place);
                    }
                }
                // Their process gone, or come: the edges are watched here while there is none.
                HEARING_CHANGED => {
                    own_input = hear_as_linked(sink, own_input);
                }
                // The mouse's own motion, heard here while there is no sensors' process to watch the edges (and
                // passed on: what touches the PC is listened for here meanwhile too).
                WM_INPUT => {
                    crate::touches::mouse_input(msg.lParam);
                    if let Some(cursor) = cursor_position() {
                        on_seam(cursor, &mut panels, &mut last_place);
                    }
                    if let (Some((moved, _)), Some(cursor)) = (read_mouse(HRAWINPUT(msg.lParam.0 as *mut _)), cursor_position()) {
                        let pushed = pusher.heard(&self.config.lock().unwrap().edges, moved, cursor, now);
                        if let Some(pushed) = pushed {
                            open_pushed(pushed, &mut panels, &mut last_place);
                        }
                    }
                }
                // While another desktop has the input (a UAC prompt, the lock
                // screen) the pointer cannot be read, and is left alone.
                WM_TIMER if msg.wParam.0 == WATCH_TIMER => {
                    if let Some(overlay) = &mut overlay {
                        overlay.watch();
                    }
                    let Some(cursor) = cursor_position() else { continue };
                    on_seam(cursor, &mut panels, &mut last_place);
                    if own_input {
                        pusher.looked(&self.config.lock().unwrap().edges, cursor, now);
                    }
                }
                WM_TIMER if msg.wParam.0 == TRACK_TIMER => {
                    let Some(cursor) = cursor_position() else { continue };
                    on_seam(cursor, &mut panels, &mut last_place);
                    if own_input {
                        let pushed = pusher.rested(&self.config.lock().unwrap().edges, cursor, now);
                        if let Some(pushed) = pushed {
                            open_pushed(pushed, &mut panels, &mut last_place);
                        }
                    }
                }
                _ => {}
            }

            // (A look up from here takes Esc too: see `ui::look`. None while another program drags something: Esc is
            // its, to call the drag off, over the panel too.)
            let looking = looking_here(&panels, &widgets);
            let wanted = !crate::station::catch::dragging() && (looking || panels.iter().any(|panel| (panel.is_open() && !panel.stays()) || panel.dragging_app()));
            if wanted != escape {
                escape = wanted;
                unsafe {
                    if wanted {
                        let _ = RegisterHotKey(None, ESCAPE_HOTKEY, MOD_NOREPEAT, u32::from(VK_ESCAPE.0));
                    } else {
                        let _ = UnregisterHotKey(None, ESCAPE_HOTKEY);
                    }
                }
            }
            // Ctrl+V taken while the pointer is over a station (the panel's or a widget's) where it shows, and only
            // then: else it is every program's (the one over a widget covered by it among them).
            let over_station = cursor_position().is_some_and(|at| {
                let under = unsafe { windows::Win32::UI::WindowsAndMessaging::GetAncestor(windows::Win32::UI::WindowsAndMessaging::WindowFromPoint(at), windows::Win32::UI::WindowsAndMessaging::GA_ROOT) };
                crate::station::run::areas().iter().filter(|(window, _, lands)| *lands == crate::station::run::Lands::Station && *window == under).any(|(_, area, _)| at.x >= area.left && at.x < area.right && at.y >= area.top && at.y < area.bottom)
            });
            // (Not while a note is written in where it is: Ctrl+V is its own.)
            let over_station = over_station && !widgets.writing();
            if over_station != paste {
                paste = over_station;
                unsafe {
                    if over_station {
                        let _ = RegisterHotKey(None, PASTE_HOTKEY, windows::Win32::UI::Input::KeyboardAndMouse::MOD_CONTROL | MOD_NOREPEAT, u32::from(b'V'));
                    } else {
                        let _ = UnregisterHotKey(None, PASTE_HOTKEY);
                    }
                }
            }
            // The preview's shortcut taken while the pointer is over what may be looked at larger (not while a note is
            // written in where it is: its keys are its own), or a look is up from here: else it is every program's. The
            // look's own keys taken while it is up.
            let shortcut = crate::app().settings.lock().unwrap().preview_shortcut;
            // (Nor while search is in front: the shortcut there is its own, for what is picked in it.)
            let wanted = shortcut.filter(|_| looking || (!widgets.writing() && !crate::ui::find::in_front() && looked_under(&panels, &widgets).is_some()));
            if wanted != preview {
                preview = wanted;
                unsafe {
                    let _ = UnregisterHotKey(None, PREVIEW_HOTKEY);
                    if let Some(shortcut) = wanted {
                        crate::ui::look::key_was_taken(RegisterHotKey(None, PREVIEW_HOTKEY, crate::tray::modifiers_of(shortcut), u32::from(shortcut.key)).is_err());
                    }
                }
            }
            if looking != look_keys {
                look_keys = looking;
                for (id, _, ctrl, key) in LOOK_KEYS {
                    unsafe {
                        if looking {
                            let ctrl = if ctrl { windows::Win32::UI::Input::KeyboardAndMouse::MOD_CONTROL } else { Default::default() };
                            let _ = RegisterHotKey(None, id, ctrl | MOD_NOREPEAT, u32::from(key));
                        } else {
                            let _ = UnregisterHotKey(None, id);
                        }
                    }
                }
            }
            let back = looking && crate::ui::look::inside();
            if back != look_back {
                look_back = back;
                unsafe {
                    if back {
                        let _ = RegisterHotKey(None, LOOK_BACK_HOTKEY, MOD_NOREPEAT, 0x08);
                    } else {
                        let _ = UnregisterHotKey(None, LOOK_BACK_HOTKEY);
                    }
                }
            }
            let play = looking && crate::ui::look::plays();
            if play != look_play {
                look_play = play;
                unsafe {
                    if play {
                        let _ = RegisterHotKey(None, LOOK_PLAY_HOTKEY, MOD_NOREPEAT, 0x20);
                    } else {
                        let _ = UnregisterHotKey(None, LOOK_PLAY_HOTKEY);
                    }
                }
            }
            // What the look shows lit on what it was called up from, and no longer on what it showed before: drawn at once.
            let shown = crate::ui::look::owner().and_then(|owner| Some((owner.0 as isize, crate::ui::look::shown_for(owner)?)));
            if shown != look_shown {
                for (owner, _) in [look_shown, shown].into_iter().flatten() {
                    match panels.iter_mut().find(|panel| panel.window.hwnd.0 as isize == owner) {
                        Some(panel) => panel.next_frame = now,
                        None => widgets.draw_soon(),
                    }
                }
                look_shown = shown;
            }
            let needed = pusher.resting() || seam_rest.is_some();
            if needed != ticking {
                ticking = needed;
                unsafe {
                    if needed {
                        SetTimer(Some(sink), TRACK_TIMER, TICK_MS, None);
                    } else {
                        let _ = KillTimer(Some(sink), TRACK_TIMER);
                    }
                }
            }
            unsafe { DispatchMessageW(&msg) };
        }
    }
}

/// Notes asked to be stuck on the desktop, and where (see `Controller::stick_note`), or taken off it (none).
static STUCK: std::sync::Mutex<Vec<(String, OnDesk)>> = std::sync::Mutex::new(Vec::new());

/// What is asked of a note on the desktop: stuck there with its corner at a point (as large as the page it is stuck
/// from, DIPs), or taken away.
enum OnDesk {
    Stick(POINT, Option<(f32, f32)>),
    Unstick,
}

/// Told as what is written in notes where they are has been kept, Glance going (see `Controller::keep_writing`).
static KEPT: std::sync::Mutex<Option<std::sync::mpsc::Sender<()>>> = std::sync::Mutex::new(None);

/// The picker of a to-do's time put away (`asked`: by its own buttons or Esc), what it set drawn so, the pointer taken
/// to have not yet come onto the panel (as a menu's going leaves it: see `Controller::end_menu`).
fn end_pick(asked: bool, panels: &mut [Panel], widgets: &mut crate::widget::Widgets) {
    crate::ui::todo::pick::close(asked);
    for panel in panels.iter_mut() {
        panel.menu_closed();
        if panel.is_shown() {
            panel.next_frame = Instant::now();
        }
    }
    widgets.draw_soon();
}

/// The overlay kept under `menu`, while one is up (see `Overlay::under_menu`).
fn keep_menu_above(menu: &Option<(crate::ui::menu::Menu, MenuFor)>, overlay: &mut Option<Overlay>) {
    if let (Some((open, _)), Some(overlay)) = (menu.as_ref(), overlay.as_mut()) {
        overlay.under_menu(Some(open.hwnd()));
    }
}

/// What a menu up is for: the tray icon's (with what each of its choices is), the widget's at a window, or the
/// overlay's.
enum MenuFor {
    Tray(Vec<usize>),
    Widget(HWND),
    Overlay,
    /// A thing on the station, by its number, what each row does, and the widget it was opened on (none: the panel).
    Station(u64, Vec<crate::station::Choice>, Option<HWND>),
    /// The station's head's (see `station::head_menu`).
    /// (And the widget it was opened on: none, the panel.)
    StationHead(Vec<crate::station::HeadChoice>, Option<HWND>),
    /// A file come lately (or files come together), as its row names it, and what each row does (see
    /// `station::recent_menu`).
    Recent(crate::station::recent::Entry, Vec<crate::station::RecentChoice>),
    /// A thing kept at hand, as its menu was opened on it, and what each row does (see `station::app_menu`).
    App(crate::station::apps::App, Vec<crate::station::AppChoice>),
    /// A note stuck on the desktop, by its name, and what each row does (see `Widgets::note_menu`).
    Note(String, Vec<crate::ui::note::Choice>),
}

impl Controller {
    /// The panel the shortcut and the tray are for: the one moved away from its edge, where the panel is kept so (see
    /// `PanelAt`); else that of the edge a panel was last opened at (`last`), on its screen. Up already, or (none up
    /// there) one to open: hidden, or made.
    fn hand_target<'a>(&'a self, panels: &mut Vec<Panel<'a>>, last: Option<(POINT, Edge)>) -> usize {
        let floating = self.config.lock().unwrap().panel_at.is_some();
        let found = if floating {
            panels.iter().position(|panel| panel.is_shown() && panel.floating())
        } else {
            last.and_then(|(point, edge)| {
                let monitor = monitor_at(point)?.monitor;
                panels.iter().position(|panel| panel.at(monitor, edge))
            })
        };
        found.unwrap_or_else(|| match panels.iter().position(|panel| !panel.is_shown()) {
            Some(at) => at,
            None => {
                panels.push(Panel::new(self).expect("panel window"));
                panels.len() - 1
            }
        })
    }

    /// `panel` opened by hand (the shortcut, the tray): where the panel is kept moved away from its edge, or at the
    /// edge a panel was last opened at (`last`), where it was; else as by hand ever (see `Panel::open_by_hand`).
    fn open_by_hand(&self, panel: &mut Panel, last: Option<(POINT, Edge)>, now: Instant) {
        let floating = self.config.lock().unwrap().panel_at.is_some();
        match last.filter(|_| !floating).and_then(|(point, edge)| Some((point, edge, monitor_at(point)?))) {
            Some((point, edge, contact)) => panel.open(point, contact, edge, None, false, now),
            None => panel.open_by_hand(now),
        }
    }

    /// The menu up, if one is, gone, `choice` chosen in it (the index of one, or none, left): done.
    fn end_menu(&self, menu: &mut Option<(crate::ui::menu::Menu, MenuFor)>, choice: Option<usize>, panels: &mut [Panel], menu_of: &mut Option<usize>, widgets: &mut crate::widget::Widgets, overlay: &mut Option<Overlay>) {
        let Some((open, of)) = menu.take() else { return };
        let menu_panel = menu_of.take();
        // (Where the choice was on the screen: what it opens comes out of there, but for the tray icon's menu.)
        let place = choice.and_then(|row| open.row_on_screen(row));
        open.close();
        if let Some(overlay) = overlay.as_mut() {
            overlay.under_menu(None);
        }
        // The pointer, where the menu left it (past the panel's edge, as a menu longer than it reaches), taken to
        // have not yet come onto the panel: it stays till the pointer comes and goes again, or a press elsewhere, as
        // opened by its shortcut. (Else gone from under it a moment after its choice, before what was chosen showed.)
        for panel in panels.iter_mut() {
            panel.menu_closed();
        }
        match of {
            MenuFor::Tray(choices) => crate::tray::chosen(choice, &choices),
            MenuFor::Widget(hwnd) => {
                if let Some(crate::widget::WidgetChoice::Settings) = choice.and_then(|row| widgets.chose(hwnd, row)) {
                    crate::show_settings(place);
                }
            }
            MenuFor::Overlay => {
                if let (Some(row), Some(overlay)) = (choice, overlay.as_ref()) {
                    self.overlay_chosen(overlay.chose(row), place, panels);
                }
            }
            MenuFor::Station(id, choices, on) => match (choice.and_then(|row| choices.get(row)), on) {
                // Looked at larger where it was chosen: on the widget, or on the panel.
                (Some(crate::station::Choice::Preview), on) => {
                    let window = on.or_else(|| menu_panel.and_then(|at| panels.get(at)).map(|panel| panel.window.hwnd));
                    if let Some(window) = window {
                        let looks = match panels.iter().find(|panel| panel.window.hwnd == window) {
                            Some(panel) => panel.looks(),
                            None => widgets.looks(window),
                        };
                        if let Some(&(hit, _)) = looks.iter().find(|(hit, _)| matches!(hit, Hit::Shelf(of) | Hit::Copied(of) if *of == id)) {
                            look_at(panels, widgets, window, hit);
                        }
                    }
                }
                (Some(choice), _) => crate::station::chose(id, *choice),
                (None, _) => {}
            },
            MenuFor::App(app, choices) => {
                if let Some(choice) = choice.and_then(|row| choices.get(row)) {
                    crate::station::app_chose(app, *choice);
                }
            }
            MenuFor::Note(name, choices) => match choice.and_then(|row| choices.get(row)) {
                // (Offered where it is not stuck yet: its corner where its row was chosen.)
                Some(crate::ui::note::Choice::Stick) => {
                    if let Some(at) = place.map(|row| POINT { x: row.left, y: row.top }).or_else(cursor_position) {
                        widgets.stick_note(&name, at, None);
                    }
                }
                Some(choice) => widgets.note_chose(&name, *choice),
                None => {}
            },
            MenuFor::Recent(file, choices) => {
                if let Some(choice) = choice.and_then(|row| choices.get(row)) {
                    crate::station::recent_chose(file, *choice);
                }
            }
            MenuFor::StationHead(choices, widget) => match choice.and_then(|row| choices.get(row)) {
                // A page chosen: shown on what the menu was opened on.
                Some(crate::station::HeadChoice::Page(page)) => match (widget, menu_panel) {
                    (Some(hwnd), _) => widgets.show_page(hwnd, *page),
                    (None, Some(at)) => {
                        if let Some(panel) = panels.get_mut(at) {
                            panel.tab = *page;
                            panel.next_frame = Instant::now();
                        }
                    }
                    (None, None) => {}
                },
                Some(choice) => crate::station::head_chose(*choice, place),
                None => {}
            },
        }
    }

    /// What was chosen of the overlay, in its menu or by one of its buttons (at `place` on the screen): done.
    fn overlay_chosen(&self, choice: OverlayChoice, place: Option<RECT>, panels: &mut [Panel]) {
        match choice {
            OverlayChoice::Settings => {
                for panel in panels.iter_mut().filter(|panel| !panel.stays()) {
                    panel.dismiss();
                }
                settings_window::open_at_overlay(place);
            }
            OverlayChoice::Lock(locked) => {
                std::thread::spawn(move || crate::app().lock_overlay(locked));
            }
            OverlayChoice::Through(through) => {
                std::thread::spawn(move || crate::app().overlay_through(through));
            }
            OverlayChoice::Close => self.set_overlay(false),
        }
    }
}

/// The panel to open at `edge` of the screen `monitor`: the one up there already, else one hidden (kept from before),
/// else one made (see the top).
fn panel_for<'a>(panels: &mut Vec<Panel<'a>>, controller: &'a Controller, monitor: RECT, edge: Edge) -> usize {
    if let Some(at) = panels.iter().position(|panel| panel.at(monitor, edge)) {
        return at;
    }
    if let Some(at) = panels.iter().position(|panel| !panel.is_shown()) {
        return at;
    }
    panels.push(Panel::new(controller).expect("panel window"));
    panels.len() - 1
}

/// A panel open at an edge as the screens changed, to open again where it was: along its edge where it centred, pinned
/// if it was, with the game to bring back.
struct Reopen {
    anchor: POINT,
    edge: Edge,
    pinned: bool,
    game: Option<HWND>,
}

impl Reopen {
    /// The panel `at` open again: at its edge of its screen (the nearest, that one gone; another edge, that one holding
    /// nothing now), as a panel pinned opens as Glance starts; not where the pointer is, which put every panel at one
    /// edge. Another up at that edge of that screen already (two screens become one): that one stays, this one not.
    fn open(self, panels: &mut [Panel], at: usize, now: Instant) {
        let Some(contact) = monitor_at(self.anchor) else { return };
        let edge = panels[at].pinned_edge(self.anchor, contact, Some(self.edge));
        if panels.iter().any(|panel| panel.at(contact.monitor, edge)) {
            return;
        }
        panels[at].open(self.anchor, contact, edge, None, false, now);
        panels[at].pinned = self.pinned && panels[at].is_open();
        panels[at].game = self.game;
    }
}

/// A point as a message carries it (x low, y high: physical px, each within what 16 bits hold).
fn point_param(x: i32, y: i32) -> isize {
    ((y as u16 as u32) << 16 | x as u16 as u32) as isize
}

/// A push as `Controller::pushed` sent it: the edge, and where the pointer was.
fn pushed_from(wparam: WPARAM, lparam: LPARAM) -> (Edge, POINT) {
    (EDGES.get(wparam.0).copied().unwrap_or_default(), lparam_point(lparam))
}

/// Registers this thread's message window `sink` for the mouse's raw input (heard whatever window has the focus):
/// whether it is.
fn hear_mouse(sink: HWND) -> bool {
    let mouse = RAWINPUTDEVICE { usUsagePage: 1, usUsage: 2, dwFlags: RIDEV_INPUTSINK, hwndTarget: sink };
    unsafe { RegisterRawInputDevices(&[mouse], size_of::<RAWINPUTDEVICE>() as u32) }.is_ok()
}

/// Has this thread's message window `sink` hear the mouse while there is no sensors' process to, and no longer once
/// there is (`heard`: whether it does now): whether it does then.
fn hear_as_linked(sink: HWND, heard: bool) -> bool {
    let wanted = !crate::source::linked();
    if wanted == heard {
        return heard;
    }
    if wanted {
        return hear_mouse(sink);
    }
    let mouse = RAWINPUTDEVICE { usUsagePage: 1, usUsage: 2, dwFlags: RIDEV_REMOVE, hwndTarget: HWND::default() };
    // (Not let go of, it is still heard, and pushes it sees are seen twice.)
    unsafe { RegisterRawInputDevices(&[mouse], size_of::<RAWINPUTDEVICE>() as u32) }.is_err()
}

/// Whether a look is up from the panel or a widget (see `ui::look`), not one from search.
fn looking_here(panels: &[Panel], widgets: &crate::widget::Widgets) -> bool {
    crate::ui::look::owner().is_some_and(|owner| panels.iter().any(|panel| panel.window.hwnd == owner) || widgets.owns(owner))
}

/// What may be looked at larger under the pointer, where it is that (see `ui::look`): the window it is on (the panel's,
/// a widget's), and what it is there.
fn looked_under(panels: &[Panel], widgets: &crate::widget::Widgets) -> Option<(HWND, Hit)> {
    use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, WindowFromPoint, GA_ROOT};
    let at = cursor_position()?;
    let under = unsafe { GetAncestor(WindowFromPoint(at), GA_ROOT) };
    let mut client = at;
    let _ = unsafe { windows::Win32::Graphics::Gdi::ScreenToClient(under, &mut client) };
    let hit = match panels.iter().find(|panel| panel.window.hwnd == under) {
        Some(panel) => panel.is_shown().then(|| panel.hit_at(client)).flatten(),
        None if widgets.owns(under) => widgets.hit_at(under, client),
        None => None,
    }?;
    hit.looks().then_some((under, hit))
}

/// `hit` on the window `window` (the panel's, a widget's) looked at larger (see `ui::look`), beside it and in its skin,
/// with the others there that may be, had by the arrows: a look up from it shows it, those others had by the arrows as
/// they show now (scrolled to, of another list).
fn look_at(panels: &[Panel], widgets: &crate::widget::Widgets, window: HWND, hit: Hit) {
    let (looks, beside, look) = match panels.iter().find(|panel| panel.window.hwnd == window) {
        Some(panel) => (panel.looks(), panel.placement.as_ref().map(|placement| placement.panel), panel.menu_look()),
        None => (widgets.looks(window), widgets.rect_of(window), widgets.look_of(window)),
    };
    let lang = panels[0].lang;
    let items: Vec<crate::ui::look::Item> = looks.iter().filter_map(|&(hit, rect)| view::looked_at(hit, Some(rect), lang)).collect();
    let Some(at) = items.iter().position(|item| item.key == hit.key()) else { return };
    if crate::ui::look::up_for(window) {
        return crate::ui::look::show_among(items, hit.key());
    }
    let skin = look.skin.unwrap_or(crate::ui::theme::Skin::Fluent);
    crate::ui::look::open(crate::ui::look::Opening { items, at, owner: window.0 as isize, beside, skin, dark: look.dark, frost: look.frost, tell: None });
}

fn lparam_point(lparam: LPARAM) -> POINT {
    POINT { x: (lparam.0 & 0xFFFF) as u16 as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32 }
}

enum Phase {
    Hidden,
    Open {
        /// The pointer has been on the panel since it opened. Until then
        /// moving elsewhere does not close it (it may have been opened from
        /// the tray, far from it).
        entered: bool,
        outside_since: Option<Instant>,
        dragging: bool,
    },
    Closing,
}


struct Placement {
    contact: Contact,
    /// Where along the edge the panel centres: the pointer where it was
    /// opened, or the middle of the work area.
    anchor: POINT,
    /// Its window: the screen it opens on (the whole desktop, away from the edge), all the while it is up. The panel
    /// is drawn in it where it is, laid out, sized and let go, and the window is not moved or sized: made smaller or
    /// larger where it shows, its corner moved, and what is drawn moved within it, the two come to the screen a frame
    /// of the compositor's apart, whichever is done first (the panel flashed out where it was, or showed where it was
    /// not, as it was sized and let go). Only the panel takes the pointer (see `track`).
    window: RECT,
    /// What is drawn in the window: the panel and its shadow's room.
    painted: RECT,
    /// The panel at rest, and the pointer's leeway around it (physical px).
    panel: RECT,
    reach: RECT,
    /// Its buttons' row outside it, where it has one (what plays as a strip, as low as its keys: see `Rack`), on the
    /// screen (physical px), and whether under it.
    rack: Option<(RECT, bool)>,
    /// Physical px per DIP: the monitor's scale, times the zoom of a panel
    /// too large for its screen.
    px: f32,
    /// Moved away from the edge: how far across and down the room on its
    /// screen's work area it is (see `PanelAt`).
    floating: Option<(f32, f32)>,
    /// The corner held while it is dragged, wherever that is.
    held: Option<Held>,
    /// Where the pointer opened it (a push into the edge, a rest on the seam): within its reach till it is moved
    /// away from the edge (see `place`); none, opened by hand (the shortcut, the tray), from anywhere.
    came_from: Option<POINT>,
}

/// A corner of the panel held where it is (physical px): its left or
/// right, its top or bottom.
#[derive(Clone, Copy, PartialEq)]
struct Held {
    at: POINT,
    right: bool,
    bottom: bool,
}

/// A drag of the pinned panel: where the pointer was and the panel was as
/// it began (physical px), and its size then; which edges it holds (none:
/// the bar, to move it); and whether it has been sized since.
#[derive(Clone, Copy)]
struct PanelDrag {
    from: POINT,
    rect: RECT,
    size: f32,
    /// What plays: the room it was given as the drag began (DIPs, at the panel's size 1).
    room: (f32, f32),
    edges: Edges,
    sized: bool,
    /// The lane whose head it was taken by (its place among the opening's lanes): pulled off, that lane alone is torn
    /// off (see `head_at`).
    lane: Option<usize>,
}

/// A limit a panel sized by its edges stops at.
#[derive(Clone, Copy, PartialEq)]
enum Limit {
    /// The smallest size there is.
    Smallest,
    /// The largest size there is.
    Largest,
    /// As large as its screen holds.
    Screen,
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Edges {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}

impl Edges {
    fn any(self) -> bool {
        self.left || self.right || self.top || self.bottom
    }
}

/// How near its edge the pointer sizes the pinned panel rather than
/// pressing what is there (DIPs).
const EDGE: f32 = 6.0;
/// How far along an edge from a corner the pointer sizes the panel by both
/// of the corner's edges at once (DIPs): a corner is easier to take than a
/// square of `EDGE` each way.
const CORNER: f32 = 20.0;

/// How far the pointer moves a pressed panel before the panel moves with it
/// (DIPs): a press that wanders less is a press.
const MOVE_FROM: f32 = 8.0;

/// How near its screen's edge a panel let go is taken back to the edge, to
/// open from it again (DIPs).
const DOCK: f32 = 24.0;

/// What a drag let go leaves to keep of where the panel is.
enum Kept {
    /// Where it was: pressed and let go, or sized against the edge.
    Unchanged,
    /// Moved away from the edge, to stay there.
    At(PanelAt),
    /// Moved back against the edge: it opens from there again.
    Docked,
}

/// The pointer's shape over edges `edges` of what is dragged by them (none: the plain arrow, as over a window,
/// which is moved by its body all the same: the shape says what a press does where it is, and over the readings,
/// a list among them, a press does what it does over any window).
pub(crate) fn sizing(left: bool, right: bool, top: bool, bottom: bool) -> windows::core::PCWSTR {
    use windows::Win32::UI::WindowsAndMessaging::{IDC_ARROW, IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE};
    match (left || right, top || bottom) {
        (true, true) if left == top => IDC_SIZENWSE,
        (true, true) => IDC_SIZENESW,
        (true, false) => IDC_SIZEWE,
        (false, true) => IDC_SIZENS,
        (false, false) => IDC_ARROW,
    }
}

fn point_at(window: &Window, edges: Edges) {
    window.point(sizing(edges.left, edges.right, edges.top, edges.bottom));
}

/// The desktop behind the window, captured as the panel opened (or again
/// and again, with live refraction), and what it means for the glass.
struct Behind {
    digest: u64,
    /// The luminance behind the panel: its mean and spread.
    tone: (f32, f32),
    /// The capture, kept: where it was taken on screen places it under the
    /// panel wherever the panel has since grown or moved to.
    capture: Capture,
    /// The capture as drawn, and the physical pixels per DIP it was made
    /// for: made again when the zoom changes, or the device was lost.
    bitmap: Option<(ID2D1Bitmap1, f32)>,
    /// The bitmap the capture before was drawn into: its memory written
    /// again for this one (see `Capture::bitmap_into`).
    former: Option<ID2D1Bitmap1>,
    taken: Instant,
}

impl Behind {
    fn new(capture: Capture, placement: &Placement, now: Instant, former: Option<ID2D1Bitmap1>) -> Self {
        let tone = capture.luminance(placement.panel, placement.px);
        Behind { digest: capture.digest, tone, capture, bitmap: None, former, taken: now }
    }

    /// The capture as drawn at `px` physical pixels a DIP (made again at another zoom, from the same pixels; else made
    /// into what the capture before was), placed so that the panel's corner, `corner` from the corner of what it is
    /// painted in (`window`), is where the panel rests on screen (the window may have moved or grown since the capture
    /// was taken); as `render::Picture::backdrop` takes it.
    fn backdrop(&mut self, dc: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext, px: f32, window: RECT, corner: (f32, f32)) -> Option<(&ID2D1Bitmap1, Vector2, u64, f32)> {
        if self.bitmap.as_ref().is_none_or(|(_, made_for)| *made_for != px) {
            self.bitmap = match self.bitmap.take() {
                Some((made, _)) => self.capture.at_scale(dc, &made, px),
                None => self.capture.bitmap_into(dc, px, self.former.take().as_ref()),
            }
            .ok()
            .map(|bitmap| (bitmap, px));
            // (Copied: what it was taken in is the next capture's to be taken in.)
            if self.bitmap.is_some() {
                self.capture.forget_texture();
            }
        }
        let from = self.capture.rect;
        let at = Vector2 { X: (window.left - from.left) as f32 / px + corner.0, Y: (window.top - from.top) as f32 / px + corner.1 };
        Some((&self.bitmap.as_ref()?.0, at, self.digest, px))
    }
}

/// What a turn between layouts starts from: a layout shown (at its zoom), or what a turn under way drew last.
enum Before {
    Shown(Layout, f32),
    Drawn(Vec<crate::ui::morph::Part>),
}

/// A capture of the desktop behind the panel, asked for: for which
/// `Panel::generation`, and whether it is taken again where the panel now is
/// (the panel kept out of it; its theme following), else refreshed in place.
struct PanelRequest {
    generation: u64,
    /// Menus shown and gone as it was asked for (see `menu::SHOWN`).
    menus: u64,
    afresh: bool,
}

impl crate::ui::life::Lives for Panel<'_> {
    fn device(&self) -> &Rc<Gfx> {
        &self.gfx
    }

    fn showing(&mut self) -> &mut Option<crate::ui::life::Showing> {
        &mut self.showing
    }

    /// Its surface, its layers (and the turn from one layout into another: opened again, it is laid out afresh), the
    /// desktop behind it; its scene rests.
    fn let_go_of_shown(&mut self) {
        if let Some(surface) = &mut self.surface {
            surface.release(&self.gfx);
        }
        self.layers.release();
        self.music.release();
        self.music_layer.release();
        self.music_ground.release();
        self.last_layout = None;
        self.turning = None;
        self.turned.clear();
        self.turn_layer.release();
        self.behind = None;
        if let Some(scene) = &mut self.backdrop {
            scene.rest();
        }
    }

    /// Its scene of what is behind it, and its surface.
    fn let_go_of_kept(&mut self) {
        self.backdrop = None;
        self.surface = None;
    }
}

/// The panel and its window, owned by the input thread.
struct Panel<'a> {
    controller: &'a Controller,
    gfx: Rc<Gfx>,
    /// Counted as shown on its device while it is (see `life`).
    showing: Option<crate::ui::life::Showing>,
    window: Window,
    /// None after the device was lost, until it is made again on the new one.
    surface: Option<Surface>,
    layers: PanelLayers,
    /// What plays, drawn whole where the edge it opened from holds it.
    music: crate::ui::music::Music,
    /// The layer what plays is drawn in, anew at each frame, and how many frames it has drawn. (Not straight onto the
    /// surface: text drawn straight onto it comes out a little differently in each place of the compositor's atlas
    /// the frames go to, and shook by a pixel frame by frame; a layer is drawn in its own place and put on whole
    /// pixels, as a widget's is.)
    music_layer: crate::ui::gfx::Layer,
    music_frames: u64,
    /// The skin's ground under what plays, where it is drawn on the skin (see `prefs::MusicGround`).
    music_ground: render::GroundLayers,
    phase: Phase,
    /// 0 at rest on screen, 1 slid out.
    shift: Transition,
    opacity: Transition,
    placement: Option<Placement>,
    /// What is shown and how, as the settings have it (the widgets draw by it too); and as the panel draws what it
    /// shows (see `Prefs::holding`).
    prefs: Prefs,
    drawn: Prefs,
    skin: Skin,
    theme: Theme,
    lang: Lang,
    edge: Edge,
    /// What it shows: the component the edge it opened from holds.
    part: Part,
    /// The columns chosen in the settings; none, as few as fit.
    columns: Option<usize>,
    /// The size chosen for its screen, 1 as designed.
    size: f32,
    /// The screen it is on, by the name that stays its own (see `screens`): its size is that screen's.
    screen: Option<String>,
    /// Refresh the desktop behind the glass while the panel is open.
    live: bool,
    behind: Option<Behind>,
    /// When live refraction last tried to take the desktop.
    tried_behind: Instant,
    frost: f32,
    /// How far its lists are scrolled (see `view::Scrolls`).
    scrolls: view::Scrolls,
    /// Its station's page shown (see `view::Tab`).
    tab: view::Tab,
    hover: Option<Hit>,
    /// The preview's shortcut said on what the pointer rests on (see `view::Hint`).
    hint: view::Hint,
    /// What is under the pointer said under it (see `hover_at`): taken away as the pointer leaves it, the panel goes,
    /// or what is under the pointer changes without it moving (see `hover_again`).
    saying: bool,
    /// Pinned open: the pointer leaving, or a press elsewhere, does not
    /// close it; unpinning, the shortcut or the tray's settings do.
    pinned: bool,
    /// Its music asked to be shown over the whole of its screen (see `Widgets::open_full`): on which screen.
    full_asked: Option<Contact>,
    /// A game holding the screen in exclusive fullscreen when the panel
    /// opened over it, which sent it to the background: brought back when
    /// the panel has closed, unless a press elsewhere closed it.
    game: Option<HWND>,
    /// The panel as it opened, held while it is up (laid out afresh at the
    /// next frame when `None`), and the settings it was made for.
    opening: Option<arrange::Opening>,
    /// Whether the game was asked for, and turned off, by hand, as `opening` was laid out: its lane comes or goes
    /// with them (see `arrange`).
    game_at: (bool, bool),
    /// What a panel laid out afresh while up (the settings changed) still
    /// holds from before: a volume unplugged stays until it closes.
    held: Option<Seen>,
    style: String,
    /// Where clicks and wheel turns land, from the panel's corner, and
    /// where that corner is in the window (DIPs).
    hits: Vec<HitBox>,
    /// Where its settings' button was as the settings were asked from it, while they are (see `settings_here`).
    settings_from: Option<RECT>,
    corner: (f32, f32),
    /// Sized by hand in a frame since it was last drawn at rest (see `Gfx::settle`).
    was_sized: bool,
    /// The layout drawn last, at its zoom: laid out anew in other columns, it turns into the new one (see `morph`).
    last_layout: Option<(Layout, f32)>,
    /// Turning from one layout into the next: since when, and from what; what the turn drew last (where a turn
    /// that turns again starts from); and the layer it is drawn in.
    turning: Option<(Instant, Before)>,
    turned: Vec<crate::ui::morph::Part>,
    /// What it drew as a thing was carried off its station (see `render::Away`).
    away: render::Away,
    turn_layer: gfx::Layer,
    /// The bar, from the panel's corner (DIPs): a pinned panel is dragged by it.
    bar: view::Rect,
    /// Torn off as it was dragged: the widget to make (its form, scale and
    /// corner on the screen, where the hand holds it from that corner, its size, and the one lane it shows, where it
    /// was taken by that lane's head).
    #[allow(clippy::type_complexity)]
    tear: Option<(Form, f32, POINT, POINT, (f32, f32), Option<String>)>,
    /// What plays torn off its lane as it was dragged: the music widget to make (its size, DIPs, its corner on the
    /// screen, and where the hand holds it from that corner). The panel stays.
    tear_music: Option<((f32, f32), POINT, POINT)>,
    /// Torn off: to go once the widget is drawn.
    torn: bool,
    /// The presses the sensors' process had told when the panel last looked (one told since is a press, however
    /// short, as the panel is open).
    presses_seen: u64,
    /// The presses on it (or its menu) seen when it last looked, those of them not yet matched by one the sensors'
    /// process told, and whether a button was down then (where that process is not there to tell presses).
    ours_seen: u64,
    ours_owed: u64,
    was_down: bool,
    /// Whether a menu of its own is up (see `stays`).
    menu_up: bool,
    /// A drag of the pinned panel under way.
    drag: Option<PanelDrag>,
    /// The largest size the panel still grows to on its screen (see
    /// `arrange::largest`), as it was last laid out.
    largest: f32,
    /// While it is sized by its edges: the size it is at, and the limit
    /// the pointer is past, if it is (shown on it as it is dragged).
    sizing: Option<(f32, Option<Limit>)>,
    /// What plays, sized by its edges: the room the hand gives it (DIPs, at the panel's size 1), till it is kept.
    sized_room: Option<(f32, f32)>,
    /// What plays let go after it was sized: settling from the room the hand gave it to the room its layout is right
    /// in, its edges held as the drag held them (see `music_sized_to`): since when, the drag, from, to.
    music_settle: Option<(Instant, PanelDrag, (f32, f32), (f32, f32))>,
    /// A thing on its station pressed, and where the pointer was: dragged off it once moved far enough (see
    /// `shelf_moved`), else pressed as it is let go.
    shelf_press: Option<(Hit, POINT)>,
    /// A thing on its station dragged off it, to be lifted off its room as the panel is next drawn (see `carry`): it,
    /// the drag's number, and where it was pressed.
    lifting: Option<(Hit, u64, POINT)>,
    /// A thing kept at hand pressed (its place), where the pointer was, and whether it has been moved far enough to
    /// be put elsewhere among them: let go on another, it goes there; else it is opened as it is let go.
    /// (By what opens it, as shown as it was pressed: what is kept may change meanwhile, and its place with it.)
    app_press: Option<(String, POINT, bool)>,
    /// Where the pointer last was over it (physical px in its window): what is under it found again as what is drawn
    /// moves under a pointer that stays.
    pointer: Option<POINT>,
    /// Pressed since the settings last came in front: over them, in use (see the frame loop).
    forward: bool,
    /// A note pressed (by its file's name), where the pointer was, and where its card is (physical px on the screen):
    /// pulled, it is stuck on the desktop under the hand, lifted off its card; let go on it, it is opened beside.
    note_press: Option<(String, POINT, RECT)>,
    /// One held so and moved: the place among them it would go to (the one the pointer was last over), and where
    /// the pointer has it (DIPs from the panel's corner).
    app_over: Option<u16>,
    app_point: (f32, f32),
    /// One let go among them: what opens it and its place, kept so (see `thumb::Dots::apps`) till what is kept comes
    /// to that order, or a while after.
    app_dropped: Option<(String, usize, Instant)>,
    /// The sound's track is being slid along.
    sliding: Option<crate::playing::Slider>,
    /// A list's bar's thumb is being dragged along it (see `view::Thumb`).
    scrolling: Option<view::Thumb>,
    /// Its sliders' knobs' dots, easing (see `thumb::Dots`).
    thumbs: crate::ui::thumb::Dots<Hit>,
    /// The desktop behind the glass is to be taken again (the panel moved
    /// or grew past what was taken).
    recapture: bool,
    /// A capture of the desktop behind under way on another thread (see
    /// `take_behind`).
    capturing: Option<PanelRequest>,
    /// Without live refraction, a frame taken: till when another is looked for before the scene rests (see
    /// `collect_behind`).
    look_again: Option<Instant>,
    /// Counts what makes a capture under way of no use: the panel closing,
    /// the settings coming in front of it.
    generation: u64,
    /// The last capture asked for came back with nothing: when.
    failed_behind: Option<Instant>,
    /// What is behind it, put together live where no one sees it (see
    /// `scene`), once it has been taken again while the panel is up.
    backdrop: Option<crate::ui::scene::Scene>,
    /// The screens changed while the panel was dragged: seen to as it is
    /// let go.
    screens_missed: bool,
    /// A drag ended with the button already up (seen by the timer): the
    /// button's release, still to come, is the drag's, not a click.
    last_frame: Instant,
    /// When the panel is next drawn: at once while anything on it is in
    /// motion beyond the charts' creep.
    next_frame: Instant,
}

impl<'a> Panel<'a> {
    fn new(controller: &'a Controller) -> windows::core::Result<Self> {
        // (Drawn first on the main screen's device: it takes up its own screen's as it opens, see `follow_device`.)
        let gfx = gfx::for_main_screen()?;
        let window = Window::new()?;
        // Its window holds its screen (see `Placement::window`): not a program in full screen there, which the shell
        // would make way for (the taskbar hidden).
        unsafe { windows::Win32::UI::WindowsAndMessaging::SetPropW(window.hwnd, windows::core::w!("NonRudeHWND"), Some(windows::Win32::Foundation::HANDLE(1 as _)))? };
        let surface = Some(Surface::new(&gfx, window.hwnd)?.clipped());
        let mut panel = Panel {
            controller,
            gfx,
            window,
            surface,
            layers: PanelLayers::default(),
            music: crate::ui::music::Music::default(),
            music_layer: crate::ui::gfx::Layer::default(),
            music_ground: render::GroundLayers::default(),
            music_frames: 0,
            showing: None,
            phase: Phase::Hidden,
            shift: Transition::settled(1.0),
            opacity: Transition::settled(0.0),
            placement: None,
            prefs: Prefs::default(),
            drawn: Prefs::default(),
            skin: Skin::Paper,
            theme: Theme::new(Skin::Paper, false),
            lang: Lang::En,
            edge: Edge::Right,
            part: Part::Monitor,
            columns: None,
            size: 1.0,
            screen: None,
            live: false,
            behind: None,
            tried_behind: Instant::now(),
            frost: 0.0,
            scrolls: view::Scrolls::default(),
            tab: view::Tab::default(),
            hover: None,
            hint: view::Hint::default(),
            saying: false,
            pinned: false,
            full_asked: None,
            game: None,
            opening: None,
            game_at: (false, false),
            held: None,
            style: String::new(),
            hits: Vec::new(),
            settings_from: None,
            corner: (0.0, 0.0),
            was_sized: false,
            last_layout: None,
            turning: None,
            turned: Vec::new(),
            away: render::Away::default(),
            turn_layer: gfx::Layer::default(),
            bar: view::Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
            tear: None,
            tear_music: None,
            torn: false,
            presses_seen: 0,
            ours_seen: 0,
            ours_owed: 0,
            was_down: false,
            menu_up: false,
            drag: None,
            largest: SIZES.1,
            sizing: None,
            sized_room: None,
            music_settle: None,
            shelf_press: None,
            lifting: None,
            app_press: None,
            pointer: None,
            forward: false,
            note_press: None,
            app_over: None,
            app_point: (0.0, 0.0),
            app_dropped: None,
            sliding: None,
            scrolling: None,
            thumbs: crate::ui::thumb::Dots::new(),
            recapture: false,
            capturing: None,
            look_again: None,
            generation: 0,
            failed_behind: None,
            backdrop: None,
            screens_missed: false,
            last_frame: Instant::now(),
            next_frame: Instant::now(),
        };
        panel.restyle();
        Ok(panel)
    }

    fn is_shown(&self) -> bool {
        !matches!(self.phase, Phase::Hidden)
    }

    /// Whether it is up at `edge` of the screen `monitor` (not moved away from it): the panel of that edge.
    fn at(&self, monitor: RECT, edge: Edge) -> bool {
        self.is_shown() && self.edge == edge && !self.floating() && self.placement.as_ref().is_some_and(|placement| placement.contact.monitor == monitor)
    }

    fn is_open(&self) -> bool {
        matches!(self.phase, Phase::Open { .. })
    }

    /// Where it is up at its edge: along it where it centres, and the edge (none, moved away from it, or hidden).
    fn place(&self) -> Option<(POINT, Edge)> {
        self.placement.as_ref().filter(|_| self.is_shown() && !self.floating()).map(|placement| (placement.anchor, self.edge))
    }

    /// The settings changed: what is shown, and how.
    fn restyle(&mut self) {
        self.next_frame = Instant::now();
        let config = self.controller.config.lock().unwrap();
        self.prefs = Prefs::resolve(&config.view, &self.controller.known_modules());
        self.drawn = self.prefs.clone().holding(self.part);
        self.columns = if self.part == Part::Workbench { config.workbench_columns } else { config.columns };
        // Mid-drag, the size is the drag's.
        if self.drag.is_none() {
            self.size = config.size_on(self.part, self.screen.as_deref());
        }
        let skin = Skin::named(&config.skin);
        // Up, and now a skin that sees the desktop: it is taken behind the
        // panel as it is now (one taken for another skin, or none, would not
        // fit it).
        if skin != self.skin && skin.sees_backdrop() && self.is_shown() {
            self.recapture = true;
        }
        // A skin that does not see the desktop: its scene, what was taken with it, and a taking awaited let go of (made
        // again as one that does is chosen, as a panel that never saw it has none).
        if !skin.sees_backdrop() {
            self.backdrop = None;
            self.behind = None;
            self.capturing = None;
        }
        self.skin = skin;
        let live = config.live && self.skin.sees_backdrop();
        // No longer live: its scene let go of, and the desktop's capture with it where no other holds it (kept, it held
        // the desktop's whole size of the card's memory with nothing shown live; opened, the panel takes what is behind
        // it once, as one never live does).
        if self.live && !live {
            self.backdrop = None;
            self.capturing = None;
        }
        self.live = live;
        drop(config);
        // What is shown, and how, changed: it is laid out anew. The
        // process list's order (sorted from the panel itself) changes no
        // lane's size, and leaves it be.
        let mut sized = self.drawn.clone();
        sized.processes.sort = Default::default();
        let style = format!("{} {:?} {} {:?} {}", serde_json::to_string(&self.edge).unwrap_or_default(), self.columns, self.size, self.skin, serde_json::to_string(&sized).unwrap_or_default());
        if style != self.style {
            self.style = style;
            // A second change before a frame laid it out keeps what the
            // first took from the opening.
            if let Some(opening) = self.opening.take().filter(|_| self.is_shown()) {
                self.held = Some(opening.seen);
            }
        }
        self.lang = Lang::resolve(self.prefs.language);
        self.exclude();
        self.dress();
    }

    /// Left out of what widgets see behind them while a widget torn off it
    /// takes its place (it is going, and the widget is above it meanwhile).
    fn exclude(&mut self) {
        crate::ui::scene::leave_out(self.window.hwnd, self.is_shown() && self.torn);
    }

    /// Picks the theme, and how frosted the glass is, for the skin and the
    /// desktop behind the panel.
    fn dress(&mut self) {
        let behind = self.behind.as_ref().map(|behind| behind.tone);
        let dark = theme::is_dark(self.prefs.theme, behind.map(|(mean, _)| mean));
        self.theme = Theme::new(self.skin, dark);
        self.frost = behind.map_or(0.0, |(mean, spread)| skins::frost(mean, spread, dark));
        // (The page beside it, where one is up, follows as the panel is next placed in its skin: see `follow_beside`.)
    }

    /// Opens the panel as the shortcut and the tray do: where it was moved
    /// away from the edge, if it was (pinned again if it was), or at the
    /// pointer.
    fn open_by_hand(&mut self, now: Instant) {
        let panel_at = self.controller.config.lock().unwrap().panel_at;
        if let Some(PanelAt { at, screen: (x, y), locked }) = panel_at {
            let point = POINT { x, y };
            if let Some(contact) = monitor_at(point) {
                let was_shown = self.is_shown();
                self.open(point, contact, self.edge_for(point, contact), Some(at), false, now);
                // Opened there, pinned there if it was; one picked up on its
                // way out (opened from the edge) stays as it was.
                if !was_shown && self.is_open() {
                    self.pinned = locked;
                }
            }
        } else if let Some((cursor, contact)) = cursor_position().and_then(|cursor| Some((cursor, monitor_at(cursor)?))) {
            self.open(cursor, contact, self.edge_for(cursor, contact), None, false, now);
        }
    }

    /// The edge the panel opens from on `contact`'s screen when nothing
    /// pushed into one (the shortcut, the tray): of those that open one
    /// there, the nearest to `point`; where none does, the one a panel
    /// opened so shows (see `Places::by_hand`).
    /// The edge a panel pinned at `point` opens at again: the one it was pinned at (`kept`), where that one still
    /// opens a panel on its screen; else the nearest that does. (By the point alone, a panel opened in the middle of
    /// its edge, the screen's middle, came back at whichever edge was nearest it: the top, on a wide screen.)
    fn pinned_edge(&self, point: POINT, contact: Contact, kept: Option<Edge>) -> Edge {
        let lit = {
            let config = self.controller.config.lock().unwrap();
            crate::screens::lit(&config.edges.places, contact.monitor)
        };
        kept.filter(|edge| lit.has(*edge)).unwrap_or_else(|| self.edge_for(point, contact))
    }

    fn edge_for(&self, point: POINT, contact: Contact) -> Edge {
        let config = self.controller.config.lock().unwrap();
        let m = contact.monitor;
        crate::screens::lit(&config.edges.places, m).nearest((m.left, m.top, m.right, m.bottom), (point.x, point.y)).unwrap_or(config.edges.places.by_hand().0)
    }

    /// Opens the panel on `contact`'s screen: along its `edge` at `cursor`,
    /// or `floating` where it was moved to (see `Placement::floating`).
    /// (`pushed`: opened by the pointer at the edge, a push into it or a rest on its seam; else by hand, the pointer
    /// anywhere.)
    fn open(&mut self, cursor: POINT, contact: Contact, edge: Edge, floating: Option<(f32, f32)>, pushed: bool, now: Instant) {
        if self.controller.history.read().unwrap().is_empty() {
            return;
        }
        // (Up already: it stays as it is. Each edge has a panel of its own: another edge's opens beside it.)
        if self.is_open() {
            return;
        }
        if pushed {
            crate::ui::settings_window::guide::saw(crate::ui::settings_window::guide::Saw::Pushed);
        }
        let began = Instant::now();
        let mut taking = Duration::ZERO;
        // A reopening during the way out picks the panel up where it is.
        if !self.is_shown() {
            self.edge = edge;
            self.screen = crate::screens::id_of(contact.monitor);
            // What that edge holds (where it holds nothing, opened by hand: what a panel opened so shows).
            self.part = {
                let places = &self.controller.config.lock().unwrap().edges.places;
                self.screen.as_deref().and_then(|id| places.part(id, edge)).unwrap_or(places.by_hand().1)
            };
            self.restyle();
            // A new opening is laid out for what the machine shows now.
            self.opening = None;
            self.held = None;
            let work = contact.work;
            let anchor = match self.controller.config.lock().unwrap().anchor {
                Anchor::Pointer => cursor,
                Anchor::Center => POINT { x: (work.left + work.right) / 2, y: (work.top + work.bottom) / 2 },
            };
            // (Away from the edge, it may be sized past its screen, and let go on another: see `Placement::window`.)
            let window = if floating.is_some() { crate::mouse::desktop() } else { contact.monitor };
            self.placement = Some(Placement {
                contact,
                anchor,
                window,
                painted: RECT::default(),
                panel: RECT::default(),
                reach: RECT::default(),
                rack: None,
                px: contact.scale,
                floating,
                held: None,
                came_from: pushed.then_some(cursor),
            });
            self.window.place(window);
            self.scrolls.reset();
            self.hover = None;
            self.game = exclusive_fullscreen().then(|| unsafe { GetForegroundWindow() }).filter(|game| !game.is_invalid());
            self.shift.jump(1.0);
            self.opacity.jump(0.0);
            self.arrange();
            // The desktop where the panel will be, taken while the window is
            // still hidden.
            if self.skin.sees_backdrop() {
                let placement = self.placement.as_ref().unwrap();
                // Unreadable (the lock screen): the glass goes without. All
                // the screen the panel may grow into while it is up: the
                // strip along its edge (it grows along it, never across).
                let (monitor, painted) = (placement.contact.monitor, placement.painted);
                let area = match (placement.floating, self.edge) {
                    (Some(_), _) => painted,
                    (None, Edge::Left | Edge::Right) => RECT { top: monitor.top, bottom: monitor.bottom, ..painted },
                    (None, Edge::Top) => RECT { left: monitor.left, right: monitor.right, ..painted },
                };
                let taken = Instant::now();
                self.behind = Capture::take(area).map(|capture| Behind::new(capture, placement, now, None));
                taking = taken.elapsed();
                // None readable (the lock screen): taken again once a second
                // has passed.
                if self.behind.is_none() {
                    self.failed_behind = Some(now);
                    self.recapture = true;
                }
                self.dress();
            }
        }
        self.phase = Phase::Open { entered: false, outside_since: None, dragging: false };
        // Presses from before it opened are no press elsewhere (what was seen of them is otherwise as old as the
        // last look while it was away, which may be long ago: opened by the shortcut, it closed at once).
        self.forget_presses();
        self.controller.set_seen(&self.controller.shown, true);
        self.exclude();
        // Away from the edge it has nowhere to slide from: it fades in.
        if reduced_motion() || self.floating() {
            self.shift.jump(0.0);
            self.opacity.retarget(1.0, PLAIN_FADE, LINEAR, now);
        } else {
            self.shift.retarget(0.0, OPEN.0, OPEN.1, now);
            self.opacity.retarget(1.0, OPEN_FADE, LINEAR, now);
        }
        // (Its window holds its screen: a click off the panel, as it shows, is for what is under it.)
        let off = self.placement.as_ref().zip(cursor_position()).is_none_or(|(placement, cursor)| !contains(&placement.panel, cursor));
        self.window.set_click_through(off);
        // The first frame is drawn before the window shows, so it never shows empty.
        self.last_frame = now;
        self.frame(now);
        self.window.show();
        crate::perf::mark(|| format!("panel shown {:.3} ms", began.elapsed().as_secs_f64() * 1000.0));
        // (Where it stands among the windows as it shows, for the journal with its opening: see below.)
        let standing = self.window.standing();
        if let Some(placement) = &self.placement {
            crate::widget::own_window_changed(placement.painted);
        }
        // An opening slow enough to be seen, and why, for the journal.
        let took = began.elapsed();
        if took > Duration::from_millis(50) {
            crate::journal::note(format!("panel: opened in {} ms (the desktop behind taken in {} ms)", took.as_millis(), taking.as_millis()));
        }
        // Not above all as it shows: said each time (it hides under the windows over its edge).
        if !standing.starts_with("above all") {
            crate::journal::note(format!("panel: shown {standing}"));
        }
    }

    fn begin_close(&mut self, now: Instant) {
        self.phase = Phase::Closing;
        self.let_go_todo();
        // Pinned and closed by hand: no longer opened as Glance starts.
        if std::mem::take(&mut self.pinned) && !self.floating() {
            if let Some(placement) = &self.placement {
                let (point, edge) = ((placement.anchor.x, placement.anchor.y), self.edge);
                std::thread::spawn(move || crate::app().pin_panel(point, edge, false));
            }
        }
        self.window.set_click_through(true);
        if reduced_motion() || self.floating() {
            self.opacity.retarget(0.0, PLAIN_FADE, LINEAR, now);
        } else {
            self.shift.retarget(1.0, CLOSE.0, CLOSE.1, now);
            self.opacity.retarget(0.0, CLOSE.0, LINEAR, now);
        }
    }

    /// The screens changed (`dpi` 0), or the window's scale did (to `dpi`):
    /// a panel placed, zoomed and backed for screens no longer so is taken
    /// down, to be opened afresh. Its own move to a monitor of another scale
    /// as it opens tells a scale it was already placed for, and is no change;
    /// nor is the scale of a window over the whole desktop (a panel away from
    /// the edge: see `Placement::window`), its largest screen's: only its own
    /// screen's scale counts.
    ///
    /// Open at an edge, it is to open again where it was (see `Reopen`): what is told back.
    fn screens_changed(&mut self, dpi: u32, now: Instant) -> Option<Reopen> {
        // Hidden, on the device of the screen its window is on now: one on a card that no longer drives it (a monitor
        // on the discrete card's port unplugged) is let go of, which would keep that card awake till it next opened.
        if !self.is_shown() && dpi == 0 {
            self.follow_device();
            gfx::let_go_of_strays();
        }
        // Dragged onto another screen, it is laid out for it as it is let
        // go; the screens themselves changing, seen to then.
        if self.drag.is_some() {
            self.screens_missed |= dpi == 0;
            return None;
        }
        let placement = self.placement.as_ref()?;
        let changed = if dpi == 0 {
            // A game leaving exclusive fullscreen sets the display mode back
            // just as the panel comes up over it, often to what it was: a
            // panel whose screen is as it was stays as it is.
            monitor_at(placement.anchor) != Some(placement.contact)
        } else {
            let monitor = placement.contact.monitor;
            let middle = POINT { x: (monitor.left + monitor.right) / 2, y: (monitor.top + monitor.bottom) / 2 };
            monitor_at(middle).is_none_or(|now| (now.scale - placement.contact.scale).abs() > 0.001)
        };
        if !changed {
            return None;
        }
        // Open, it opens again, laid out for the screens as they are now,
        // pinned if it was, and still with the game to bring back: where it
        // was, at its own edge of its own screen (see `Reopen`); on its way
        // out, it is gone at once, and the game comes back as when it has
        // closed (the game's own display mode change can come as late as
        // that).
        let (open, closing, pinned, game, floating) = (self.is_open(), matches!(self.phase, Phase::Closing), self.pinned, self.game, self.floating());
        let (anchor, edge) = (placement.anchor, self.edge);
        self.dismiss();
        if closing {
            bring_back(game);
        }
        if open && floating {
            self.open_by_hand(now);
            self.pinned = pinned;
        }
        (open && !floating).then_some(Reopen { anchor, edge, pinned, game })
    }

    /// A to-do written on it: its field written in, it taking the keys (see `ui::todo::begin`).
    fn write_todo(&mut self) {
        if self.is_open() {
            crate::ui::todo::begin(&self.window);
            self.next_frame = Instant::now();
        }
    }

    /// A to-do written on it no more: the keys given back (the front too, where `escaped`).
    fn stop_todo(&mut self, escaped: bool) {
        if crate::ui::todo::writing(self.window.hwnd) {
            crate::ui::todo::end(&self.window, escaped);
            self.next_frame = Instant::now();
        }
    }

    /// Going: a to-do written on it given back the keys, the picker of a time opened from it put away.
    fn let_go_todo(&mut self) {
        self.stop_todo(false);
        if crate::ui::todo::pick::up_for(self.window.hwnd) {
            crate::ui::todo::pick::close(false);
        }
    }

    /// Takes the panel down at once (for the settings, or screens no longer
    /// as they were): a game it opened over stays where it went.
    fn dismiss(&mut self) {
        if !self.is_shown() {
            return;
        }
        // (Its own page beside it goes with it; another's stays.)
        if crate::ui::beside::up_for(self.window.hwnd) {
            crate::ui::beside::close();
        }
        self.end_drag();
        self.let_go_todo();
        // (What it said under what the pointer was on goes with it: a drive's row, the line that switches players.)
        self.hover = None;
        self.unsay();
        self.recapture = false;
        self.generation += 1;
        self.torn = false;
        self.game = None;
        self.phase = Phase::Hidden;
        self.pinned = false;
        self.window.hide();
        crate::perf::mark(|| "panel hidden".into());
        if let Some(placement) = &self.placement {
            crate::widget::own_window_changed(placement.painted);
        }
        self.exclude();
        self.capturing = None;
        self.hidden();
        self.placement = None;
    }

    /// What was pressed since it last looked (`down`: a button is down now): whether anything was, and whether any of
    /// it was elsewhere, not on it nor its menu.
    fn presses_since(&mut self, down: bool) -> (bool, bool) {
        let (told, ours) = (PRESSES.load(Ordering::Relaxed), OURS.load(Ordering::Relaxed));
        let new_ours = ours - std::mem::replace(&mut self.ours_seen, ours);
        let new = if crate::source::linked() { told - std::mem::replace(&mut self.presses_seen, told) } else { u64::from(down && !self.was_down).max(new_ours) };
        self.was_down = down;
        let elsewhere;
        (self.ours_owed, elsewhere) = matched(new, new_ours, self.ours_owed);
        (new > 0, elsewhere)
    }

    /// Its menu closed (see `Controller::end_menu`): the pointer not yet come onto it.
    fn menu_closed(&mut self) {
        if let Phase::Open { entered, outside_since, .. } = &mut self.phase {
            *entered = false;
            *outside_since = None;
        }
    }

    /// Presses so far taken as seen: none of them a press elsewhere.
    fn forget_presses(&mut self) {
        self.presses_seen = PRESSES.load(Ordering::Relaxed);
        self.ours_seen = OURS.load(Ordering::Relaxed);
        self.ours_owed = 0;
        self.was_down = buttons_down();
    }

    /// Follows the pointer while the panel is open: closes it once the
    /// pointer has been away long enough, or on a press elsewhere.
    fn track(&mut self, now: Instant) {
        // Torn off, the widget not yet on the screen: the panel stays till
        // it is (see the input loop), wherever the pointer goes.
        if self.torn {
            return;
        }
        let (Some(placement), Some(cursor)) = (&self.placement, cursor_position()) else { return };
        // (Its buttons' row outside it is on it: see `Placement::rack`.)
        let on_panel = contains(&placement.panel, cursor) || placement.rack.is_some_and(|(rack, _)| contains(&rack, cursor));
        let in_reach = contains(&placement.reach, cursor);
        let close_delay = self.controller.config.lock().unwrap().close_delay;
        // Down now, or pressed (and perhaps let go) since it last looked; and of those presses, any not on it nor its
        // menu: a press elsewhere. (Told by the sensors' process, a press may come after the panel's menu took it and
        // closed: matched with those seen on its own windows, it is not taken for one elsewhere. Where that process
        // is not there, a press is a button come down since it last looked.)
        let down = buttons_down();
        let (pressed, elsewhere) = self.presses_since(down);
        let held = down || pressed;
        if !on_panel {
            // (What it said under it goes with it: a drive's row, the line that switches players, a name cut short.)
            self.unsay();
            if self.hover.take().is_some() {
                self.next_frame = now;
            }
        }
        // Around the panel the window is only shadow; clicks there belong
        // to whatever is underneath.
        self.window.set_click_through(!on_panel);
        let stays = self.stays();
        let Phase::Open { entered, outside_since, dragging } = &mut self.phase else { return };
        if stays {
            *outside_since = None;
            return;
        }
        // A press that begins away from the panel dismisses it; one that
        // begins on it keeps it open wherever the pointer then goes.
        let pressed_outside = elsewhere && !*dragging && !in_reach;
        *dragging = held && (*dragging || in_reach);
        *entered |= in_reach;
        if pressed_outside {
            // Something else was chosen: the game stays where it went.
            self.game = None;
            return self.begin_close(now);
        }
        if in_reach || *dragging || !*entered {
            *outside_since = None;
        } else if now.duration_since(*outside_since.get_or_insert(now)) >= close_delay {
            self.begin_close(now);
        }
    }

    /// Follows the pointer, takes the desktop behind the glass again when
    /// it is live, and draws the panel if a frame is due.
    fn tick(&mut self, now: Instant) {
        // (What it draws and takes from behind it is sized as it is: see `gfx::room`.)
        let _sizing = gfx::sizing(self.drag.is_some_and(|drag| drag.edges.any()));
        if self.is_open() {
            self.track(now);
        } else {
            // (What was pressed before it opened is not a press elsewhere.)
            self.forget_presses();
        }
        // (Giving way to the settings, it is taken all the same: what is behind it is put together from the windows
        // under it, and the settings are in front of it, never among them; see `scene`.)
        if self.live && self.is_open() && self.capturing.is_none() {
            self.refresh_behind(now);
        }
        // A panel the desktop was taken behind, grown or moved past it
        // (dragged, sized, laid out afresh), takes it again; with live
        // refraction it is taken again anyway.
        if !self.live && self.drag.is_none() && self.skin.sees_backdrop() {
            let window = self.placement.as_ref().map(|placement| placement.painted);
            let taken = self.behind.as_ref().map(|behind| behind.capture.rect);
            if let (Some(window), Some(taken)) = (window, taken) {
                self.recapture |= !covers(taken, window);
            }
        }
        // (Taken again while it gives way to the settings too, as after the device was lost: what was taken on the
        // lost one cannot be drawn on the new one, and till it is taken again the glass has nothing behind it.)
        let rested = self.failed_behind.is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(1));
        if self.recapture && rested && self.capturing.is_none() {
            self.recapture_behind();
        }
        if now >= self.next_frame {
            self.frame(now);
        }
    }

    /// With live refraction, looks at the windows behind again every so
    /// often (what they show comes as it changes: see `collect_behind`).
    fn refresh_behind(&mut self, now: Instant) {
        let Some(placement) = &self.placement else { return };
        if now.duration_since(self.tried_behind) < LIVE_INTERVAL {
            return;
        }
        self.tried_behind = now;
        let area = placement.painted;
        self.take_behind(area, false);
    }

    /// Takes the desktop behind the glass again where the panel now is
    /// (see `collect_behind`).
    fn recapture_behind(&mut self) {
        let Some(placement) = &self.placement else { return };
        let area = placement.painted;
        self.recapture = false;
        self.take_behind(area, true);
    }

    /// Puts together what is behind the panel within `area`, where no one
    /// sees it (see `scene`), and awaits a frame of it if anything there
    /// changed; `afresh`, the theme follows what comes. The panel goes on
    /// with what it had meanwhile; nothing changed, that stands.
    fn take_behind(&mut self, area: RECT, afresh: bool) {
        // (Without the system's capture, Windows 10 before 1903: what was taken as it opened stands.)
        if !crate::ui::scene::supported() {
            return;
        }
        if self.backdrop.is_none() {
            let wake = std::sync::Arc::new(|| crate::app().controller.wake());
            self.backdrop = crate::ui::scene::Scene::new(area, wake, &self.gfx);
        }
        let Some(scene) = &mut self.backdrop else {
            // (It could not be put together: as when the screen cannot be read, tried again in a while.)
            self.failed_behind = Some(Instant::now());
            self.recapture |= afresh;
            return;
        };
        let changed = scene.arrange(area, self.window.hwnd);
        if changed || self.behind.is_none() {
            let menus = crate::ui::menu::SHOWN.load(Ordering::Relaxed);
            self.capturing = Some(PanelRequest { generation: self.generation, menus, afresh });
            // (Its first frame looked past as the last were.)
            self.look_again = None;
        }
    }

    /// A frame of what is behind come, if one has: the one awaited, or with
    /// live refraction one that came as what is behind changed. Of what is
    /// still so: taken again where the panel now is, its theme follows it
    /// (live refraction, refreshing over the same place, leaves the theme
    /// be, and only the frost follows); the same desktop as before, the
    /// last stands.
    fn collect_behind(&mut self, now: Instant) {
        let Some(scene) = &mut self.backdrop else { return };
        let Some(capture) = scene.ready() else {
            // Looked for again a while, and none came: what was taken stands, and the scene rests.
            if self.look_again.is_some_and(|till| now >= till) && !scene.reading() {
                self.look_again = None;
                scene.rest();
                return;
            }
            // (Live, as often as it is looked at: see `refresh_behind`; each as it comes where they come no oftener.)
            if self.capturing.is_some() || (self.live && self.behind.as_ref().is_none_or(|behind| scene.paced() || now.duration_since(behind.taken) >= LIVE_INTERVAL)) {
                scene.want();
            }
            return;
        };
        // Without live refraction, what is behind is as it was when taken, till it is taken again: the scene rests
        // meanwhile. Not at once: a frame put together while the compositor had put in only some of the windows
        // behind is followed by one with all of them, which is looked for a while (see `widget::LOOK_AGAIN`), and
        // taken in its place if it comes; then it rests.
        if !self.live {
            if self.look_again.take().is_some() {
                scene.rest();
            } else {
                self.look_again = Some(now + crate::widget::LOOK_AGAIN);
                scene.want();
            }
        }
        let capture = Some(capture);
        let request = self.capturing.take().unwrap_or(PanelRequest { generation: self.generation, menus: crate::ui::menu::SHOWN.load(Ordering::Relaxed), afresh: false });
        if request.generation != self.generation {
            return;
        }
        // A menu came or went meanwhile: it may be in it; taken again.
        if request.menus != crate::ui::menu::SHOWN.load(Ordering::Relaxed) {
            self.recapture |= request.afresh;
            return;
        }
        let Some(placement) = &self.placement else { return };
        let Some(capture) = capture else {
            self.failed_behind = Some(now);
            self.recapture |= request.afresh;
            return;
        };
        self.failed_behind = None;
        if !request.afresh && self.behind.as_ref().is_some_and(|behind| capture.digest == behind.digest) {
            if let Some(behind) = &mut self.behind {
                behind.taken = now;
            }
            return;
        }
        // (The one before's, or what it had from its own before, if it was not drawn.)
        let former = self.behind.take().and_then(|behind| behind.bitmap.map(|(bitmap, _)| bitmap).or(behind.former));
        let behind = Behind::new(capture, placement, now, former);
        if request.afresh {
            self.behind = Some(behind);
            self.dress();
        } else {
            self.frost = skins::frost(behind.tone.0, behind.tone.1, self.theme.dark);
            self.behind = Some(behind);
        }
        self.next_frame = now;
    }

    /// The settings window came to the front (`Some`), or left it: the
    /// panel gives way to it meanwhile. Up over a desktop it sees, the
    /// desktop is taken again once the settings have gone, without them.
    fn settings_front(&mut self, settings: Option<HWND>) {
        let was = self.window.yielding();
        // (Drawn again: its settings' button lit while the settings asked from it are open.)
        self.next_frame = Instant::now();
        self.window.yield_to(settings);
        // (Said as it changes: a panel left under the settings is under every window over its edge.)
        if was != settings.is_some() {
            crate::journal::note(format!("panel: {} the settings ({})", if settings.is_some() { "gives way to" } else { "no longer gives way to" }, self.window.standing()));
        }
        // A capture under way may take the settings in: of no use when it
        // comes back, and taken again once they have gone.
        if settings.is_some() && self.capturing.is_some() {
            self.generation += 1;
            self.recapture = self.skin.sees_backdrop() && !self.live;
        }
        if was && settings.is_none() && self.is_shown() && self.skin.sees_backdrop() && !self.live {
            self.recapture = true;
            self.next_frame = Instant::now();
        }
    }

    /// How a menu opened on it looks: in its skin, light or dark, as frosted as it is drawn and as large.
    fn menu_look(&self) -> crate::ui::menu::Look {
        let zoom = self.placement.as_ref().map_or(1.0, |placement| placement.px / placement.contact.scale);
        crate::ui::menu::Look::of(self.theme.skin, self.theme.dark).frosted(self.frost, self.behind.is_some()).zoomed(zoom)
    }

    /// Whether it is moved away from the edge.
    fn floating(&self) -> bool {
        self.placement.as_ref().is_some_and(|placement| placement.floating.is_some() || placement.held.is_some())
    }

    /// Whether it stays up wherever the pointer goes: pinned, or moved away
    /// from the edge (it lives there until it is closed by hand, or moved
    /// back to the edge).
    fn stays(&self) -> bool {
        // (With the page beside it up: the two are one place, left by the page's own going. With a menu of its own up,
        // as large as it may be past its edge: a press on the menu is on the panel's behalf, and one away from both
        // closes the menu first.)
        self.pinned || self.floating() || crate::ui::beside::up_for(self.window.hwnd) || crate::ui::look::up_for(self.window.hwnd) || self.menu_up || crate::ui::todo::holds(self.window.hwnd)
    }

    /// What may be looked at larger on it, as it was just drawn (see `ui::look`): each such thing, and where its tile is
    /// on the screen (physical px).
    fn looks(&self) -> Vec<(Hit, RECT)> {
        looks_in(&self.hits_on_screen())
    }

    /// What the pointer is taken to be over: while a look at a thing on it is up, that thing (another is had by the
    /// arrows or a click, never by the pointer passing over it).
    fn hover_shown(&self) -> Option<Hit> {
        match crate::ui::look::shown_for(self.window.hwnd) {
            Some(key) => self.hits.iter().map(|hit| hit.4).find(|hit| hit.looks() && hit.key() == key),
            None => self.hover,
        }
    }

    /// `page` opened beside it on its inner side (under it at the top edge): quick launch's choosing, a note.
    fn open_beside(&self, page: crate::ui::beside::Page) {
        use crate::ui::beside::{Opening, Side};
        let Some(placement) = &self.placement else { return };
        let side = match self.edge {
            Edge::Left => Side::Right,
            Edge::Right => Side::Left,
            Edge::Top => Side::Below,
        };
        crate::ui::beside::open(Opening { page, by: placement.panel, side, skin: self.theme.skin, dark: self.theme.dark, folder: None, owner: self.window.hwnd.0 as isize });
    }

    /// The edges of the panel a point `(x, y)` from its corner (DIPs) is
    /// on, if it is near them.
    fn edges_at(&self, x: f32, y: f32) -> Edges {
        let (width, height) = match (self.part, &self.opening, &self.placement) {
            (Part::Music, _, Some(placement)) => ((placement.panel.right - placement.panel.left) as f32 / placement.px, (placement.panel.bottom - placement.panel.top) as f32 / placement.px),
            (_, Some(opening), _) => (opening.layout.width(), opening.layout.height()),
            _ => return Edges::default(),
        };
        let mut edges = Edges { left: x < EDGE, right: x >= width - EDGE, top: y < EDGE, bottom: y >= height - EDGE };
        // What plays, against its edge, is sized from its edges out on the screen (it stays against that one).
        if self.part == Part::Music && !self.floating() {
            match self.edge {
                Edge::Left => edges.left = false,
                Edge::Right => edges.right = false,
                Edge::Top => edges.top = false,
            }
            if !edges.any() {
                return edges;
            }
        }
        // On an edge near a corner: the corner's other edge too.
        let across = edges.left || edges.right;
        let down = edges.top || edges.bottom;
        Edges {
            left: edges.left || (down && x < CORNER),
            right: edges.right || (down && x >= width - CORNER),
            top: edges.top || (across && y < CORNER),
            bottom: edges.bottom || (across && y >= height - CORNER),
        }
    }

    /// What a press at `client` (physical px in the window) would do to the
    /// open panel: size it by the edges it is on, or move it, from anywhere
    /// on it that takes no press of its own (the process list too, which
    /// takes only the wheel); nothing, pinned (it stays as it is).
    fn grip_at(&self, client: POINT) -> Option<Edges> {
        let px = match &self.placement {
            Some(placement) if !self.pinned && self.is_open() => placement.px,
            _ => return None,
        };
        // (Its buttons' row outside it is no edge of it: a press there is the buttons'.)
        if self.on_rack(client) {
            return None;
        }
        let (x, y) = (client.x as f32 / px - self.corner.0, client.y as f32 / px - self.corner.1);
        let edges = self.edges_at(x, y);
        if edges.any() {
            return Some(edges);
        }
        // (A line of what plays takes a press too, as a tap: pulled, it moves the panel, or tears off what plays, as
        // a music widget's lines do. Most of a card is its lines.)
        self.hit_at(client).is_none_or(|hit| !hit.takes_press() || matches!(hit, Hit::Line(_))).then_some(edges)
    }

    /// The lane at `client` (physical px in the window), anywhere on a workbench's (its head, its free room, the clock's
    /// figures), by its head on the readings':
    /// its place among the opening's lanes, pulled, that lane alone torn off; none where the panel shows one lane only,
    /// or the point is on none (between them, round them: pulled there, the panel is torn off whole). (By its head
    /// alone, a lane was torn off only where its head was free, and a pull anywhere else on it took the whole panel.)
    fn head_at(&self, client: POINT) -> Option<usize> {
        let (placement, opening) = (self.placement.as_ref()?, self.opening.as_ref()?);
        let (x, y) = (client.x as f32 / placement.px - self.corner.0, client.y as f32 / placement.px - self.corner.1);
        let lanes = opening.layout.lanes();
        if lanes.len() < 2 {
            return None;
        }
        // (The readings' lanes by their heads, as ever: pulled by its readings, the panel comes off whole.)
        let reach = |lane: &view::Rect| if self.part == Part::Workbench { lane.h } else { (self.theme.pad_top + view::HEAD_BAND).min(lane.h) };
        lanes.iter().position(|lane| x >= lane.x && x < lane.x + lane.w && y >= lane.y && y < lane.y + reach(lane))
    }

    /// Whether the press in hand is a tap so far: neither moved nor sized since.
    fn tapped(&self) -> bool {
        self.drag.is_some_and(|drag| !drag.edges.any() && !drag.sized) && self.placement.as_ref().is_some_and(|placement| placement.held.is_none())
    }

    /// Whether `client` (physical px in the window) is on the panel, not in its shadow's room or past it.
    fn on_panel(&self, client: POINT) -> bool {
        self.placement.as_ref().is_some_and(|placement| {
            let at = POINT { x: client.x + placement.window.left, y: client.y + placement.window.top };
            contains(&placement.panel, at)
        }) || self.on_rack(client)
    }

    /// Whether `client` (physical px in the window) is on the panel's buttons' row outside it (see `Placement::rack`).
    fn on_rack(&self, client: POINT) -> bool {
        self.placement.as_ref().is_some_and(|placement| {
            let at = POINT { x: client.x + placement.window.left, y: client.y + placement.window.top };
            placement.rack.is_some_and(|(rack, _)| contains(&rack, at))
        })
    }

    /// Whether `client` (physical px in the window) is on the panel's bar.
    fn on_bar(&self, client: POINT) -> bool {
        let Some(placement) = &self.placement else { return false };
        let (x, y) = (client.x as f32 / placement.px - self.corner.0, client.y as f32 / placement.px - self.corner.1);
        let bar = self.bar;
        x >= bar.x && x < bar.x + bar.w && y >= bar.y && y < bar.y + bar.h
    }

    /// A press on the panel at `client` (physical px in the window): unless
    /// it is pinned, on its edges it begins to size it, elsewhere (see
    /// `grip_at`) to move it.
    fn press(&mut self, client: POINT) {
        // (Off the panel, in its window before it was told to let clicks through there: nothing of the panel's.)
        if !self.on_panel(client) {
            return;
        }
        // On a note: pulled off, stuck on the desktop; else opened as it is let go (see `note_press`).
        if let (Some(Hit::Note(number)), Some(from)) = (self.hit_at(client), cursor_position()) {
            if let (Some(name), Some(card)) = (crate::notes::named(number), self.hit_on_screen(Hit::Note(number))) {
                self.note_press = Some((name, from, card));
                self.follow_pointer();
            }
            return;
        }
        // On a thing kept at hand: moved among them, or opened (see `app_press`).
        if let (Some(Hit::App(number)), Some(from)) = (self.hit_at(client), cursor_position()) {
            let Some(target) = crate::station::apps::numbered(number).map(|app| app.target) else { return };
            self.app_press = Some((target, from, false));
            self.app_over = None;
            unsafe { windows::Win32::UI::Input::KeyboardAndMouse::SetCapture(self.window.hwnd) };
            return;
        }
        // On a thing on its station: dragged off it, or pressed (see `shelf_press`).
        if let (Some(hit), Some(from)) = (self.hit_at(client).filter(|hit| hit.drags()), cursor_position()) {
            self.shelf_press = Some((hit, from));
            self.follow_pointer();
            return;
        }
        // On a list's bar: its thumb dragged along it till let go, the list scrolled as it goes.
        let at = self.dips(client);
        let thumb = view::under(&self.hits, at).next().and_then(|bar| view::Thumb::pressed(*bar, at.1, &self.scrolls));
        if let Some(thumb) = thumb {
            self.scrolling = Some(thumb);
            self.drag_thumb(client);
            self.follow_pointer();
            return;
        }
        // On the sound's track: it is slid along till let go.
        if let Some(slider) = self.hit_at(client).and_then(Hit::slider) {
            self.sliding = Some(slider);
            self.slide(client);
            unsafe {
                windows::Win32::UI::Input::KeyboardAndMouse::SetCapture(self.window.hwnd);
                SetTimer(Some(self.window.hwnd), crate::overlay::FOLLOW_TIMER, FOLLOW_MS, None);
            }
            return;
        }
        let Some(edges) = self.grip_at(client) else { return };
        let (Some(from), Some(placement)) = (cursor_position(), &self.placement) else { return };
        let rect = placement.panel;
        let room = self.music_room();
        // (Settling after a drag before: taken in hand where it is.)
        self.music_settle = None;
        let lane = (!edges.any()).then(|| self.head_at(client)).flatten();
        self.drag = Some(PanelDrag { from, rect, size: self.size, room, edges, sized: false, lane });
        unsafe {
            windows::Win32::UI::Input::KeyboardAndMouse::SetCapture(self.window.hwnd);
            SetTimer(Some(self.window.hwnd), crate::overlay::FOLLOW_TIMER, FOLLOW_MS, None);
        }
    }

    /// The pointer taken, and followed wherever it goes while it is: the panel is never in front, and a window not in
    /// front is told of the pointer only over itself, its capture or not. (A note pressed could not be pulled off it:
    /// the pointer, past the panel's edge, was no longer heard of, but where something of Glance's in front had
    /// been clicked first.)
    fn follow_pointer(&self) {
        unsafe {
            windows::Win32::UI::Input::KeyboardAndMouse::SetCapture(self.window.hwnd);
            SetTimer(Some(self.window.hwnd), crate::overlay::FOLLOW_TIMER, FOLLOW_MS, None);
        }
    }

    /// Whether a thing on its station or a note is pressed, to be pulled off as the pointer goes far enough.
    fn pressed_to_pull(&self) -> bool {
        self.shelf_press.is_some() || self.note_press.is_some()
    }

    /// The pointer followed with a thing on its station or a note pressed (see `follow_pointer`): let go where the
    /// panel was not told (past its edge), nothing pressed; far enough, the thing dragged off, or the note pulled off
    /// the panel (see `note_pulled`).
    fn follow_press(&mut self) -> Option<crate::widget::NotePulled> {
        if !buttons_down() {
            self.shelf_press = None;
            self.note_press = None;
        }
        self.shelf_moved();
        let pulled = self.note_pulled();
        if !self.pressed_to_pull() && !self.held() {
            unsafe {
                let _ = KillTimer(Some(self.window.hwnd), crate::overlay::FOLLOW_TIMER);
            }
            crate::ui::window::let_go_of_capture(self.window.hwnd);
        }
        pulled
    }

    /// A note pressed and pulled, far enough (over the panel or past it): lifted off its card, to be stuck on the
    /// desktop under the hand (the panel stays): from where its card is, held where the card was pressed, gone as far
    /// as the hand went.
    fn note_pulled(&mut self) -> Option<crate::widget::NotePulled> {
        let (cursor, placement) = (cursor_position()?, self.placement.as_ref()?);
        let pulled = self.note_press.as_ref().is_some_and(|(_, from, _)| crate::station::run::drag_begins(*from, cursor));
        if !pulled {
            return None;
        }
        let (name, from, card) = self.note_press.take()?;
        Some(crate::widget::NotePulled { name, card, from, over: placement.panel, px: placement.px })
    }

    /// Where `hit` shows on the screen (physical px), as the panel was just drawn.
    fn hit_on_screen(&self, hit: Hit) -> Option<RECT> {
        let placement = self.placement.as_ref()?;
        let (px, window) = (placement.px, placement.window);
        let screen = |at: f32, from: f32, origin: i32| origin + ((at + from) * px).round() as i32;
        let (x, y, w, h, _) = self.hits.iter().find(|(.., at)| *at == hit)?;
        Some(RECT { left: screen(*x, self.corner.0, window.left), top: screen(*y, self.corner.1, window.top), right: screen(x + w, self.corner.0, window.left), bottom: screen(y + h, self.corner.1, window.top) })
    }

    /// The pointer moved with a thing on its station pressed: far enough, it is dragged off (by the station, which
    /// takes the pointer from here).
    fn shelf_moved(&mut self) {
        let (Some((hit, from)), Some(now)) = (self.shelf_press, cursor_position()) else { return };
        if crate::station::run::drag_begins(from, now) {
            self.shelf_press = None;
            crate::ui::window::let_go_of_capture(self.window.hwnd);
            if let Some(drag) = crate::station::drag_off(hit, unsafe { windows::Win32::System::Threading::GetCurrentThreadId() }) {
                self.lifting = Some((hit, drag, from));
                self.next_frame = Instant::now();
            }
        }
    }

    /// Whether it is being dragged (or the sound's track is, or a list's bar's thumb): till its window is told the
    /// button went up.
    fn held(&self) -> bool {
        self.drag.is_some() || self.sliding.is_some() || self.scrolling.is_some()
    }

    /// The list whose bar's thumb is in hand scrolled as far as the thumb goes with the hand at `client` (physical px in
    /// the window, off the bar too, as far as it goes).
    fn drag_thumb(&mut self, client: POINT) {
        let Some(thumb) = self.scrolling else { return };
        let (_, y) = self.dips(client);
        self.scrolls.hold(thumb.list(), thumb.scroll_at(y), thumb.most());
        self.next_frame = Instant::now();
    }

    /// A list's bar's thumb let go: the list left where it was taken.
    fn let_go_of_thumb(&mut self) {
        self.scrolling = None;
        unsafe {
            let _ = KillTimer(Some(self.window.hwnd), crate::overlay::FOLLOW_TIMER);
        }
        crate::ui::window::let_go_of_capture(self.window.hwnd);
    }

    /// The track slid along as far as where `client` is along it (the pointer may have left the track: as far as it
    /// goes).
    fn slide(&mut self, client: POINT) -> Option<()> {
        let slider = self.sliding?;
        let px = self.placement.as_ref()?.px;
        let (x, y) = (client.x as f32 / px - self.corner.0, client.y as f32 / px - self.corner.1);
        let (hx, hy, w, h, hit) = *self.hits.iter().find(|hit| hit.4.slider() == Some(slider))?;
        // (A track standing up, along from its bottom.)
        let fraction = if hit.upright() { (hy + h - y) / h } else { (x - hx) / w };
        crate::playing::slide_along(slider, fraction);
        self.next_frame = Instant::now();
        Some(())
    }

    /// The track let go: as it was left.
    /// The slider in hand let go: where it is taken (`taken`), or given up.
    fn slid(&mut self, taken: bool) {
        let Some(slider) = self.sliding.take() else { return };
        unsafe {
            let _ = KillTimer(Some(self.window.hwnd), crate::overlay::FOLLOW_TIMER);
        }
        crate::ui::window::let_go_of_capture(self.window.hwnd);
        crate::playing::slid(slider, taken);
    }

    /// Follows the pointer while it is dragged: the panel moves with it,
    /// onto another screen too; or, held by its edges, grows and shrinks as
    /// a whole from the corner opposite them (its shape is its lanes').
    fn dragged(&mut self, now: Instant) {
        if self.sliding.is_some() || self.scrolling.is_some() {
            if let (Some(cursor), Some(window)) = (cursor_position(), self.placement.as_ref().map(|p| p.window)) {
                let client = POINT { x: cursor.x - window.left, y: cursor.y - window.top };
                self.slide(client);
                self.drag_thumb(client);
            }
            return;
        }
        let (Some(drag), Some(cursor)) = (self.drag, cursor_position()) else { return };
        point_at(&self.window, drag.edges);
        let (dx, dy) = (cursor.x - drag.from.x, cursor.y - drag.from.y);
        let r = drag.rect;
        let e = drag.edges;
        if !e.any() {
            let held = Held { at: POINT { x: r.left + dx, y: r.top + dy }, right: false, bottom: false };
            let Some(placement) = &mut self.placement else { return };
            if placement.held == Some(held) {
                return;
            }
            // Not moved yet: only once the pointer has gone far enough.
            let from = MOVE_FROM * placement.contact.scale;
            if placement.held.is_none() && ((dx * dx + dy * dy) as f32) < from * from {
                return;
            }
            // Pulled far enough: a widget torn off, the panel as it shows,
            // under the hand where it was taken; the panel goes once the
            // widget is drawn, so that one is on the screen throughout.
            // (Unless that is turned off: then pulling it does nothing.)
            let _ = held;
            if !self.controller.config.lock().unwrap().tear_off {
                return;
            }
            // What plays, torn off whole: a music widget as large as the panel, under the hand where it was taken.
            if self.part == Part::Music {
                let scale = placement.contact.scale;
                let size = ((placement.panel.right - placement.panel.left) as f32 / scale, (placement.panel.bottom - placement.panel.top) as f32 / scale);
                let at = POINT { x: placement.panel.left + dx, y: placement.panel.top + dy };
                self.tear_music = Some((size, at, POINT { x: cursor.x - at.x, y: cursor.y - at.y }));
                self.end_drag();
                self.torn = true;
                return;
            }
            let Some(opening) = &self.opening else { return };
            // Taken by a lane's head: that lane alone, a widget of its own (one column of it), its corner where the
            // lane's was; else the panel as it shows.
            let lane = drag.lane.and_then(|at| Some((opening.lane_ids().get(at)?.clone(), *opening.layout.lanes().get(at)?)));
            let (form, at, only) = match lane {
                Some((id, lane)) => {
                    let corner = |along: f32, corner: f32, window: i32| window + ((along + corner) * placement.px).round() as i32;
                    let at = POINT { x: corner(lane.x, self.corner.0, placement.window.left) + dx, y: corner(lane.y, self.corner.1, placement.window.top) + dy };
                    (Form::Lanes { detail: Detail::Full, columns: 1 }, at, Some(id))
                }
                None => (Form::Lanes { detail: Detail::Full, columns: opening.layout.columns }, POINT { x: placement.panel.left + dx, y: placement.panel.top + dy }, None),
            };
            let size = ((placement.panel.right - placement.panel.left) as f32 / placement.contact.scale, (placement.panel.bottom - placement.panel.top) as f32 / placement.contact.scale);
            self.tear = Some((form, opening.zoom, at, POINT { x: cursor.x - at.x, y: cursor.y - at.y }, size, only));
            self.end_drag();
            self.torn = true;
            return;
        }
        if self.part == Part::Music {
            self.size_music(drag, (dx, dy), now);
            return;
        }
        let (width, height) = ((r.right - r.left) as f32, (r.bottom - r.top) as f32);
        // (Centred on its edge, it grows on both sides along it: the side pulled follows the hand, so twice the pull.)
        let centred = !self.floating() && self.controller.config.lock().unwrap().anchor == crate::settings::Anchor::Center;
        let (twice_x, twice_y) = match (centred, self.edge) {
            (true, Edge::Top) => (2, 1),
            (true, _) => (1, 2),
            (false, _) => (1, 1),
        };
        let across = (e.left || e.right).then(|| (width + (if e.right { dx } else { -dx } * twice_x) as f32) / width);
        let down = (e.top || e.bottom).then(|| (height + (if e.bottom { dy } else { -dy } * twice_y) as f32) / height);
        let ratio = match (across, down) {
            (Some(a), Some(d)) => a.max(d),
            (Some(a), None) => a,
            (None, Some(d)) => d,
            (None, None) => 1.0,
        };
        // In hundredths: laid out afresh only as often as that changes. From
        // as large as it shows (a size past what its screen holds shows no
        // larger), and no larger than that: past either end, it says so.
        let wanted = (drag.size.min(self.largest) * ratio * 100.0).round() / 100.0;
        let limit = if wanted < SIZES.0 {
            Some(Limit::Smallest)
        } else if wanted > self.largest {
            Some(if self.largest < SIZES.1 { Limit::Screen } else { Limit::Largest })
        } else {
            None
        };
        let size = wanted.clamp(SIZES.0, self.largest.max(SIZES.0));
        if self.sizing != Some((size, limit)) {
            self.sizing = Some((size, limit));
            self.next_frame = now;
        }
        if size == self.size {
            return;
        }
        self.size = size;
        if let Some(drag) = &mut self.drag {
            drag.sized = true;
        }
        // Away from the edge, the corner opposite the edges held stays;
        // against it, the panel stays against it.
        let floating = self.floating();
        if let Some(placement) = &mut self.placement {
            placement.held = floating.then_some(Held {
                at: POINT { x: if e.left { r.right } else { r.left }, y: if e.top { r.bottom } else { r.top } },
                right: e.left,
                bottom: e.top,
            });
        }
        if let Some(opening) = self.opening.take() {
            self.held = Some(opening.seen);
        }
        self.arrange();
        self.next_frame = now;
    }

    /// What plays, held by its edges `drag.edges` and pulled `(dx, dy)` (physical px) since: given as much more room
    /// or less as that, as a music widget is (its layout following: a strip, a player, a card, a stage), within its
    /// screen. The edges across from those held stay where they were.
    fn size_music(&mut self, drag: PanelDrag, (dx, dy): (i32, i32), now: Instant) {
        let e = drag.edges;
        let Some(placement) = &self.placement else { return };
        let (contact, px) = (placement.contact, placement.px);
        // Centred on its edge (as the settings place it), it grows and shrinks along the edge on both sides at once,
        // its middle where it was: the side pulled follows the hand, so twice as much is given or taken.
        let centred = !self.floating() && self.controller.config.lock().unwrap().anchor == crate::settings::Anchor::Center;
        let (along_x, along_y) = if self.edge == Edge::Top { (2.0, 1.0) } else { (1.0, 2.0) };
        let (twice_x, twice_y) = if centred { (along_x, along_y) } else { (1.0, 1.0) };
        let grown = (
            if e.right { dx } else if e.left { -dx } else { 0 } as f32 / px * twice_x,
            if e.bottom { dy } else if e.top { -dy } else { 0 } as f32 / px * twice_y,
        );
        let most = self.music_most(contact, px);
        let least = crate::ui::music::LEAST;
        let room = (
            (drag.room.0 + grown.0).clamp(least.0, most.0.max(least.0)),
            (drag.room.1 + grown.1).clamp(least.1, most.1.max(least.1)),
        );
        if self.sized_room == Some(room) {
            return;
        }
        if let Some(drag) = &mut self.drag {
            drag.sized = true;
        }
        self.music_sized_to(drag, room, now);
    }

    /// What plays given `room` by `drag` (as the hand sizes it, or as it settles after: see `music_settle`): as large
    /// as that, as a widget of what plays follows the hand (held to the range its layout is right in, it followed the
    /// hand only part of the way, and held back between one layout's range and the next's); placed with the edges the
    /// drag holds where they were.
    fn music_sized_to(&mut self, drag: PanelDrag, room: (f32, f32), now: Instant) {
        let (e, r) = (drag.edges, drag.rect);
        let Some(placement) = &self.placement else { return };
        let px = placement.px;
        let centred = !self.floating() && self.controller.config.lock().unwrap().anchor == crate::settings::Anchor::Center;
        self.sized_room = Some(room);
        let size = room;
        let (width, height) = ((size.0 * px).round() as i32, (size.1 * px).round() as i32);
        let (floating, edge) = (self.floating(), self.edge);
        let Some(placement) = &mut self.placement else { return };
        if floating {
            // Away from the edge: the corner across from the edges held stays.
            placement.held = Some(Held {
                at: POINT { x: if e.left { r.right } else { r.left }, y: if e.top { r.bottom } else { r.top } },
                right: e.left,
                bottom: e.top,
            });
        } else if !centred {
            // Against it, centred along it where it is held: the edge across from the one pulled stays.
            match edge {
                Edge::Left | Edge::Right if e.top => placement.anchor.y = r.bottom - height / 2,
                Edge::Left | Edge::Right if e.bottom => placement.anchor.y = r.top + height / 2,
                Edge::Top if e.left => placement.anchor.x = r.right - width / 2,
                Edge::Top if e.right => placement.anchor.x = r.left + width / 2,
                _ => {}
            }
        }
        self.arrange();
        self.next_frame = now;
    }

    /// The most room what plays has on the screen `contact` at `px` physical pixels a DIP: its work area, less the gaps
    /// round it and its shadow's room.
    fn music_most(&self, contact: Contact, px: f32) -> (f32, f32) {
        let round = 2.0 * (GAP * contact.scale + self.theme.margin * px) / px;
        ((contact.work.right - contact.work.left) as f32 / px - round, (contact.work.bottom - contact.work.top) as f32 / px - round)
    }

    /// The room what plays is given from the edge the panel is at (DIPs, at the panel's size 1): as the hand gives it
    /// now, or as it was kept.
    fn music_room(&self) -> (f32, f32) {
        self.sized_room.unwrap_or_else(|| self.controller.config.lock().unwrap().music_rooms.at(self.edge == Edge::Top))
    }

    /// Ends a drag under way.
    fn end_drag(&mut self) -> Option<PanelDrag> {
        self.sizing = None;
        let drag = self.drag.take()?;
        unsafe {
            let _ = KillTimer(Some(self.window.hwnd), crate::overlay::FOLLOW_TIMER);
        }
        crate::ui::window::let_go_of_capture(self.window.hwnd);
        Some(drag)
    }

    /// Let go: it stays where it was put and as large as it was made, laid
    /// out for that screen; what to keep, if it moved (where it is) or was
    /// sized (its size).
    fn let_go(&mut self, now: Instant) -> Option<(Kept, Option<f32>)> {
        let drag = self.end_drag()?;
        // (What plays keeps the room it was given instead: see `sized_room`.)
        let size = (drag.sized && self.part != Part::Music).then_some(self.size);
        // Moved or sized, the desktop behind is taken again for what it now covers.
        let moved = self.placement.as_ref().is_some_and(|placement| placement.held.is_some());
        self.recapture |= (drag.sized || moved) && self.skin.sees_backdrop();
        self.next_frame = now;
        let placement = self.placement.as_mut()?;
        if placement.held.is_none() {
            // Pressed and let go where it was, or sized against the edge.
            return Some((Kept::Unchanged, size));
        }
        let panel = placement.panel;
        let centre = POINT { x: (panel.left + panel.right) / 2, y: (panel.top + panel.bottom) / 2 };
        let contact = monitor_at(centre)?;
        // Let go against an edge of its screen that opens it: back to that
        // edge, centred where it was let go, to close as the pointer leaves
        // and open from the edge again.
        let lit = {
            let config = self.controller.config.lock().unwrap();
            crate::screens::lit(&config.edges.places, contact.monitor)
        };
        let sized = drag.sized;
        if let Some(edge) = lit.each().find(|edge| docks(*edge, contact, panel)) {
            self.edge = edge;
            let other = contact != placement.contact;
            placement.contact = contact;
            placement.px = contact.scale;
            placement.floating = None;
            placement.held = None;
            placement.anchor = centre;
            if other {
                self.moved_to(contact.monitor, sized);
            }
            if let Some(opening) = self.opening.take() {
                self.held = Some(opening.seen);
            }
            self.arrange();
            return Some((Kept::Docked, size));
        }
        if contact != placement.contact {
            // Another screen: laid out afresh for it, its held corner where
            // it was let go.
            placement.contact = contact;
            placement.px = contact.scale;
            self.moved_to(contact.monitor, sized);
            if let Some(opening) = self.opening.take() {
                self.held = Some(opening.seen);
            }
            self.arrange();
        }
        // Where it now is, as far into the room on the screen's work area
        // as that (wholly on it, as it is placed from now on).
        let PanelAt { at, .. } = self.kept_at()?;
        let placement = self.placement.as_mut()?;
        placement.floating = Some(at);
        placement.held = None;
        self.arrange();
        let kept = self.kept_at()?;
        self.placement.as_mut()?.anchor = POINT { x: kept.screen.0, y: kept.screen.1 };
        Some((Kept::At(kept), size))
    }

    /// Let go on another screen, at `monitor`: that screen's, and as large as the panel is drawn there; as large as
    /// the hand made it, `sized`, to be kept for that screen.
    fn moved_to(&mut self, monitor: RECT, sized: bool) {
        self.screen = crate::screens::id_of(monitor);
        if !sized {
            self.size = self.controller.config.lock().unwrap().size_on(self.part, self.screen.as_deref());
        }
    }

    /// Where the panel is, as it is kept away from the edge: how far into
    /// the room on its screen's work area, and its middle.
    fn kept_at(&self) -> Option<PanelAt> {
        let placement = self.placement.as_ref()?;
        let panel = placement.panel;
        let contact = placement.contact;
        let (width, height) = (panel.right - panel.left, panel.bottom - panel.top);
        let gap = (GAP * contact.scale).round() as i32;
        let work = contact.work;
        let share = |at: i32, low: i32, room: i32| if room > 0 { ((at - low - gap) as f32 / room as f32).clamp(0.0, 1.0) } else { 0.0 };
        let at = (
            share(panel.left, work.left, work.right - work.left - 2 * gap - width),
            share(panel.top, work.top, work.bottom - work.top - 2 * gap - height),
        );
        Some(PanelAt { at, screen: (panel.left + width / 2, panel.top + height / 2), locked: self.pinned })
    }

    /// The window placed for what it shows: its lanes laid out (see `arrange_lanes`), or what plays at its size.
    fn arrange(&mut self) {
        match self.part {
            Part::Music => {
                self.music_size();
            }
            Part::Monitor | Part::Workbench => {
                self.arrange_lanes();
            }
        }
    }

    /// What plays, as large as it is shown from the edge the panel is at (as a card from a side, as a player across
    /// from the top), the window placed for it: drawn as large as a widget of what plays is (the panel's size is the
    /// readings'; what plays is made larger or smaller by its edges, as a widget is), so that the two look alike.
    fn music_size(&mut self) -> (f32, f32) {
        let shows = crate::ui::music::Shows::from(&self.controller.config.lock().unwrap().view);
        // (No larger than the screen it opens on holds: kept as it was made on a larger one, it reached past this one.)
        let room = self.music_room();
        let contact = self.placement.as_ref().unwrap().contact;
        let most = self.music_most(contact, contact.scale);
        let least = crate::ui::music::LEAST;
        let room = (room.0.min(most.0).max(least.0), room.1.min(most.1).max(least.1));
        // (As the hand gives it while it is sized or settles after; else within the range its layout is right in.)
        let size = if self.sized_room.is_some() { room } else { crate::ui::music::rests_at(room, &shows) };
        // As a strip, its buttons in a row outside it, as a widget's are (in its corner they lay over its keys).
        let rack = (self.music.layout_at(size, &shows) == crate::ui::music::Layout::Strip).then_some(RACK);
        let placement = self.placement.as_mut().unwrap();
        place(placement, self.edge, &self.theme, size, 1.0, rack);
        self.follow_beside();
        size
    }

    /// The page beside it, where one is up from it, kept beside it as it is now: where it is (larger in another skin,
    /// its lanes grown), in its skin.
    fn follow_beside(&self) {
        if let Some(placement) = &self.placement {
            if crate::ui::beside::up_for(self.window.hwnd) {
                crate::ui::beside::follow(placement.panel, self.theme.skin, self.theme.dark);
            }
        }
    }

    /// The panel's lanes, laid out (once each opening, and held), and the
    /// window placed for them.
    fn arrange_lanes(&mut self) -> (Vec<view::Lane>, Layout) {
        let controller = self.controller;
        let info = controller.info();
        let now = controller.seen.lock().unwrap().clone();
        let history = controller.history.read().unwrap();
        let samples = crate::panel::contiguous(&history);
        let contact = self.placement.as_ref().unwrap().contact;
        let work = (
            (contact.work.right - contact.work.left) as f32 / contact.scale,
            (contact.work.bottom - contact.work.top) as f32 / contact.scale,
        );
        // The game asked for or turned off by hand since it was laid out (from here, a widget, or the game gone):
        // its lane comes or goes, and it is laid out anew, holding what it held.
        let game = (view::front_is_game(), view::game_hidden());
        if game != self.game_at {
            if let Some(opening) = self.opening.take() {
                self.held = Some(opening.seen);
            }
            self.game_at = game;
        }
        let (prefs, theme, lang, scroll, tab, hover, pinned) = (&self.drawn, &self.theme, self.lang, self.scrolls, self.tab, self.hover, self.pinned);
        let opening = match &mut self.opening {
            Some(opening) => {
                // What the machine has shown since joins its lane where it
                // fits (each lane measured holding what it would hold).
                let probe = scene(controller, &info, prefs, theme, lang, scroll, tab, hover, pinned, samples, &now);
                opening.grow(&now, &info, |id, seen, detail| view::lane_height(&probe, id, seen, detail));
                opening
            }
            None => {
                let now = match self.held.take() {
                    Some(held) => held.join(&now, &info),
                    None => now,
                };
                let scene = scene(controller, &info, prefs, theme, lang, scroll, tab, hover, pinned, samples, &now);
                let heights = view::heights(&scene);
                let tall: Vec<f32> = heights.iter().map(|(_, height)| height[0]).collect();
                let rail = heights.iter().position(|(id, _)| id == "apps");
                self.largest = arrange::largest(theme, self.edge, &tall, work, self.columns, SIZES.1, rail);
                let lanes: Vec<(&str, arrange::Heights)> = heights.iter().map(|(id, height)| (id.as_str(), *height)).collect();
                self.opening.insert(arrange::Opening::new(theme, self.edge, &lanes, 0, work, self.columns, self.size, arrange::Want::Itself, now.clone()))
            }
        };
        let mut lanes = view::lanes(&scene(controller, &info, prefs, theme, lang, scroll, tab, hover, pinned, samples, &opening.seen));
        view::todo::write_in(&mut lanes, &crate::ui::todo::field(self.window.hwnd, Instant::now()).0);
        let (layout, zoom) = (opening.layout.clone(), opening.zoom);
        drop(history);
        let placement = self.placement.as_mut().unwrap();
        place(placement, self.edge, &self.theme, (layout.width(), layout.height()), zoom, None);
        self.follow_beside();
        (lanes, layout)
    }

    /// Takes up this thread's graphics device for the screen it is drawn on if it is another (the last was lost, or the
    /// panel is on a screen another adapter drives: see `gfx::on_screen`), making the window's surface and layers
    /// again on it. False while there is no device to be had.
    fn follow_device(&mut self) -> bool {
        // (Where it is drawn: its window may hold the whole desktop, see `Placement::window`. Hidden, where its window
        // is.)
        let current = match &self.placement {
            Some(placement) => gfx::for_area(placement.painted),
            None => gfx::for_window(self.window.hwnd),
        };
        let Ok(current) = current else { return false };
        if Rc::ptr_eq(&current, &self.gfx) && self.surface.is_some() {
            return true;
        }
        // All it drew with, and kept, was the lost device's (see `life`); what was read of the desktop behind (how
        // light it is, which the theme follows) stands without its bitmap, and what is behind is put together on the
        // new one as it is next taken.
        let behind = self.behind.take();
        let had_scene = self.backdrop.is_some();
        // (A window has one composition target: the old one goes first.)
        self.gone();
        self.behind = behind.map(|behind| Behind { bitmap: None, former: None, ..behind });
        let surface = match Surface::new(&current, self.window.hwnd) {
            Ok(surface) => surface.clipped(),
            Err(error) => {
                gfx::failed("panel", &current, Some(&error));
                return false;
            }
        };
        self.surface = Some(surface);
        self.gfx = current;
        if had_scene {
            self.capturing = None;
            self.recapture = true;
        }
        true
    }

    /// Draws the panel as it is at `now` where it is laid out in its window, and finishes a closing that is done.
    fn frame(&mut self, now: Instant) {
        let _timed = crate::perf::Timed::start("panel frame", || format!("{} moving={}", match self.phase { Phase::Hidden => "hidden", Phase::Closing => "closing", _ => "open" }, !(self.shift.done(now) && self.opacity.done(now))));
        let by_hand = self.drag.is_some_and(|drag| drag.edges.any());
        let _sizing = gfx::sizing(by_hand);
        // (What another window drew on this thread before forgotten: see `view::gliding_drawn`.)
        crate::ui::view::gliding_drawn();
        if matches!(self.phase, Phase::Closing) && self.shift.done(now) && self.opacity.done(now) {
            // Closed: a game the panel sent to the background comes back.
            let game = self.game.take();
            self.dismiss();
            bring_back(game);
            return;
        }
        if !self.follow_device() {
            self.next_frame = now + DEVICE_RETRY;
            return;
        }
        if self.is_shown() {
            self.shown();
        }
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        // (Each list scrolled as far as it was for what it shows: another view, or filter, of it starts at its top.)
        self.scrolls.follow(view::lists_show(self.tab));
        let lists_moving = self.scrolls.ease(dt, SCROLL_EASE);

        if self.part == Part::Music {
            return self.frame_music(now);
        }
        let (lanes, layout) = self.arrange_lanes();
        let zoom = self.opening.as_ref().map_or(1.0, |opening| opening.zoom);
        // Laid out in other columns (sized, or the readings grown): it turns into the new layout, each reading moved
        // from where it was to where it goes, as a widget turns between its forms. One turning already turns on from
        // where it got to.
        if let Some((last, last_zoom)) = self.last_layout.take() {
            if last.columns != layout.columns && !reduced_motion() {
                let before = match self.turning.take() {
                    Some(_) => Before::Drawn(std::mem::take(&mut self.turned)),
                    None => Before::Shown(last, last_zoom),
                };
                self.turning = Some((now, before));
            }
        }
        self.last_layout = Some((layout.clone(), zoom));
        if self.turning.as_ref().is_some_and(|(since, _)| now.duration_since(*since) >= crate::widget::MORPH) {
            self.turning = None;
            self.turned.clear();
            self.turn_layer.release();
        }
        let turn = self.turning.as_ref().map(|(since, _)| crate::ui::morph::ease(now.duration_since(*since).as_secs_f32() / crate::widget::MORPH.as_secs_f32()));
        let placement = self.placement.as_ref().unwrap();
        let px = placement.px;
        // Drawn in what it is painted in (the window, but while it is sized), placed in the window there.
        let window = placement.painted;
        let size = ((window.right - window.left) as u32, (window.bottom - window.top) as u32);
        let offset = (window.left - placement.window.left, window.top - placement.window.top);
        let (width, height) = (layout.width(), layout.height());
        // Where the panel rests in the window, and where it is now, slid
        // out by `shift` of the way it comes in.
        let rest = ((placement.panel.left - window.left) as f32 / px, (placement.panel.top - window.top) as f32 / px);
        let shift = self.shift.value(now);
        let opacity = self.opacity.value(now);
        let travel = match self.theme.entrance {
            Entrance::Beyond(extra) => (if self.edge == Edge::Top { height } else { width }) + extra,
            Entrance::Slide(distance) => distance,
        };
        let (mut x, mut y) = rest;
        match self.edge {
            Edge::Left => x -= shift * travel,
            Edge::Right => x += shift * travel,
            Edge::Top => y -= shift * travel,
        }

        let controller = self.controller;
        let info = controller.info();
        let history = controller.history.read().unwrap();
        let seen = &self.opening.as_ref().unwrap().seen;
        // The knob under the pointer, and the one held (the slider in hand's).
        let held = self.sliding.and_then(|sliding| self.hits.iter().map(|hit| hit.4).find(|hit| hit.slider() == Some(sliding)));
        let thumbs_moving = self.thumbs.step(now, self.hover.filter(|hit| hit.eases()), held);
        // (What is chosen of several, its mark sliding to it as it changes: the station's page, the notes' view.)
        let thumbs_moving = self.thumbs.choose(now, &view::choices(self.tab)) | thumbs_moving;
        // (What was just copied from it, marked so a moment.)
        let thumbs_moving = self.thumbs.mark_copied(now, view::copied(), crate::station::run::COPIED_FOR) | thumbs_moving;
        // What is kept at hand: the one held where it would go, the others making room; one let go, kept where it went
        // till the order comes so.
        let order: Vec<String> = crate::station::apps::shown().iter().map(|app| app.target.clone()).collect();
        if self.app_dropped.as_ref().is_some_and(|(target, to, at)| order.get(*to) == Some(target) || at.elapsed() > Duration::from_secs(1)) {
            self.app_dropped = None;
        }
        let held = self.app_press.as_ref().filter(|(_, _, moved)| *moved).and_then(|(target, ..)| order.iter().position(|one| one == target));
        let hold = match (held, &self.app_dropped) {
            (Some(from), _) => Some((from, self.app_over.map_or(from, usize::from), Some(self.app_point))),
            (_, Some((target, to, _))) => order.iter().position(|one| one == target).map(|from| (from, *to, None)),
            _ => None,
        };
        let thumbs_moving = self.thumbs.apps(now, &order, hold) | thumbs_moving;
        // (The preview's shortcut said on what the pointer rests on that may be looked at larger: not while a look is
        // up from it.)
        let says = crate::ui::look::hint_says(self.lang).filter(|_| !crate::ui::look::up_for(self.window.hwnd));
        let hint = self.hint.step(self.hover.filter(|_| says.is_some()), now).zip(says.as_deref()).map(|((hit, shown), says)| (hit, shown, says));
        let scene = Scene { thumbs: &self.thumbs, settings_lit: self.settings_lit(), hint, ..scene(controller, &info, &self.drawn, &self.theme, self.lang, self.scrolls, self.tab, self.hover_shown(), self.pinned, crate::panel::contiguous(&history), seen) };
        let edge = (!self.floating()).then_some(self.edge);
        // What it turns from keeps to the side the panel keeps to: the corner held as it is sized, else its edge.
        let (keeps_right, keeps_bottom) = match placement.held {
            Some(held) => (held.right, held.bottom),
            None => (placement.floating.is_none() && self.edge == Edge::Right, false),
        };
        let centred = placement.held.is_none() && placement.floating.is_none() && self.edge == Edge::Top;
        let halo = (self.theme.skin == Skin::Glass).then_some(self.theme.legibility);
        let (before, turned, turn_layer) = (self.turning.as_mut().map(|(_, before)| before), &mut self.turned, &mut self.turn_layer);
        let (layers, behind, frost) = (&mut self.layers, &mut self.behind, self.frost);
        let (sizing, lang, theme) = (self.sizing, self.lang, &self.theme);
        let mut drawn = None;
        // A thing on its station dragged off it: lifted off its room, as it was drawn there (see `carry`).
        let lifting = self.lifting.take().and_then(|(hit, drag, from)| self.hits.iter().find(|box_| box_.4 == hit).map(|&(hx, hy, hw, hh, _)| (hit, drag, from, (hx, hy, hw, hh))));
        let mut lifted = None;
        let (away, mut taken_off) = (&mut self.away, None);
        let surface = self.surface.as_mut().unwrap();
        surface.offset(offset);
        let painted = surface.draw(&self.gfx, size, px, |frame| {
            frame.origin(0.0, 0.0);
            if let Some((hit, _, _, (hx, hy, ..))) = lifting {
                lifted = Some(render::record_tile(frame, &scene, &lanes, &layout, None, 1.0, (-hx, -hy), hit)).filter(|parts| !parts.is_empty());
                if lifted.is_some() {
                    crate::ui::view::carry_tile(Some(hit));
                }
            }
            if opacity < 1.0 {
                let everything = D2D_RECT_F { left: -f32::MAX, top: -f32::MAX, right: f32::MAX, bottom: f32::MAX };
                let layer = D2D1_LAYER_PARAMETERS1 { contentBounds: everything, opacity, ..Default::default() };
                unsafe { frame.dc.PushLayer(&layer, None) };
            }
            let backdrop = behind.as_mut().and_then(|behind| behind.backdrop(&frame.dc, px, window, rest));
            let picture = render::Picture { scene: &scene, lanes: &lanes, layout: &layout, small: None, size: (width, height), at: (0.0, 0.0), edge, backdrop, frost, rough: false };
            let local = Matrix3x2::translation(x, y);
            // A thing carried off its station put down and taken off it: turning from how it was drawn as it was
            // carried (its room outlined) into how it is drawn now.
            let mut gone_from = away.follow(|| render::record(frame, &scene, &lanes, &layout, None, 1.0, (0.0, 0.0))).filter(|_| turn.is_none()).map(Before::Drawn);
            let (turn, before) = match gone_from.as_mut() {
                Some(from) => (Some(0.0), Some(from)),
                None => (turn, before),
            };
            drawn = Some(match (turn, before) {
                (Some(t), Some(before)) => (|| {
                    // Turning: the skin's ground as it is now, and over it each reading between where it was and
                    // where it goes.
                    let grounded = layers.draw_ground(frame, &picture, local, px)?;
                    if let Before::Shown(old, old_zoom) = before {
                        let k = *old_zoom / zoom;
                        let (old_w, old_h) = (old.width() * k, old.height() * k);
                        let old_at = (
                            if keeps_right { width - old_w } else if centred { (width - old_w) / 2.0 } else { 0.0 },
                            if keeps_bottom { height - old_h } else { 0.0 },
                        );
                        *before = Before::Drawn(render::record(frame, &scene, &lanes, old, None, k, old_at));
                    }
                    let now_drawn = render::record(frame, &scene, &lanes, &layout, None, 1.0, (0.0, 0.0));
                    let Before::Drawn(from) = before else { unreachable!() };
                    let parts = crate::ui::morph::between(from, &now_drawn, t);
                    turn_layer.draw(frame, t.to_bits() as u64 + 1, local, (0.0, 0.0, width, height), px, halo, |frame| {
                        crate::ui::morph::draw(frame, &parts);
                        Ok(())
                    })?;
                    *turned = parts;
                    // (Nothing to press mid-turn: where each thing is comes as it ends.)
                    Ok(render::Drawn { hits: None, grounded })
                })(),
                _ => layers.draw(frame, &picture, local, px),
            });
            taken_off = gone_from;
            // (Not shown, a frame part of which failed: see `Frame::fail`.)
            if let Some(Err(error)) = &drawn {
                frame.fail(error.clone());
            }
            if let Some((size, limit)) = sizing {
                frame.place(Matrix3x2::translation(x, y));
                size_badge(frame, theme, size, limit, lang, (width, height));
                frame.origin(0.0, 0.0);
            }
            if opacity < 1.0 {
                unsafe { frame.dc.PopLayer() };
            }
        });
        drop(history);
        let drawn = match (painted, drawn) {
            (Ok(()), Some(Ok(drawn))) => drawn,
            // Not drawn: drawn again shortly (the device gone, on a new one).
            (painted, drawn) => {
                gfx::failed("panel", &self.gfx, painted.as_ref().err().or(drawn.as_ref().and_then(|drawn| drawn.as_ref().err())));
                self.next_frame = now + DEVICE_RETRY;
                return;
            }
        };
        let (hits, grounded) = (drawn.hits, drawn.grounded);
        if let Some(from) = taken_off {
            self.turning = Some((now, from));
        }
        if let (Some((hit, drag, from, (_, _, hw, hh))), Some(parts), Some(at)) = (lifting, lifted, lifting.and_then(|(hit, ..)| self.hit_on_screen(hit))) {
            let lifted = crate::carry::Lifted { hit, drag, parts, size: (hw, hh), at: POINT { x: at.left, y: at.top }, from, px, over: placement.panel, window: self.window.hwnd, look: self.menu_look() };
            crate::carry::begin(&self.gfx, lifted);
        }
        self.gfx.sweep();
        // Sized by hand, and now drawn at rest: what was made for it as it was sized is let go (see `Gfx::settle`).
        if by_hand {
            self.was_sized = true;
        } else if std::mem::take(&mut self.was_sized) {
            self.gfx.settle();
        }
        // The effects that drew the ground are not needed again until it
        // changes; what they hold is let go.
        if grounded {
            self.gfx.clear_caches();
        }
        if let Some(hits) = hits {
            self.hits = hits;
            self.follow_settings();
            self.hover_again();
        }
        // (In the window: where it is painted in it, and where in that.)
        self.corner = (x + offset.0 as f32 / px, y + offset.1 as f32 / px);
        self.show_stations();
        self.bar = layout.bar();

        // At rest only the charts move, a plot's width over its span: the
        // panel is drawn again when they have crept a quarter of a pixel (or, with none, four times a second).
        let moving = thumbs_moving || settings_window::bounce(self.settings_from).is_some() || !self.shift.done(now) || !self.opacity.done(now) || lists_moving || crate::ui::view::gliding_drawn() || self.turning.is_some();
        self.next_frame = if moving {
            now
        } else {
            let speed = layout.plot_width(&self.theme, &lanes).map(|width| width * px / self.prefs.chart_seconds as f32);
            now + settings_window::creeping(speed)
        };
        // (Sooner, the preview's shortcut due to be said, or coming in.)
        if let Some(due) = self.hint.due(now) {
            self.next_frame = self.next_frame.min(due);
        }
        // A to-do written on it: its caret blinking, the input method's words put at it.
        let hwnd = self.window.hwnd;
        if let (_, Some(due)) = crate::ui::todo::field(hwnd, now) {
            self.next_frame = self.next_frame.min(due);
        }
        if crate::ui::todo::writing(hwnd) {
            if let Some(field) = self.on_screen(Hit::Todo(view::TodoHit::Field)) {
                crate::ui::todo::placed(hwnd, field, px);
            }
        }
    }

    /// Draws what plays as it is at `now`, slid out by as much of the way as it has come (its frame's start is
    /// `frame`'s: a closing done is finished there, and the device followed).
    fn frame_music(&mut self, now: Instant) {
        // Settling after it was sized (see `music_settle`): as a widget settles, from the hand's room to its layout's.
        if let Some((since, drag, from, to)) = self.music_settle {
            let t = (now.saturating_duration_since(since).as_secs_f32() / crate::widget::SETTLE.as_secs_f32()).min(1.0);
            let eased = crate::ui::morph::ease(t);
            let room = (from.0 + (to.0 - from.0) * eased, from.1 + (to.1 - from.1) * eased);
            self.music_sized_to(drag, room, now);
            if t >= 1.0 {
                self.music_settle = None;
                self.sized_room = None;
                // Away from the edge: kept where it came to rest, as it is kept where it is let go (see `let_go`; kept
                // where it was let go, larger, it was placed there as its corner was let go, and jumped).
                let rested = if self.floating() { self.kept_at() } else { None };
                if let Some(placement) = &mut self.placement {
                    placement.held = None;
                    if let Some(rested) = &rested {
                        placement.floating = Some(rested.at);
                    }
                }
                self.arrange();
                if let Some(rested) = rested {
                    if let Some(placement) = &mut self.placement {
                        placement.anchor = POINT { x: rested.screen.0, y: rested.screen.1 };
                    }
                    crate::app().change(|settings| settings.panel_at = Some(rested));
                }
            }
        }
        let size = self.music_size();
        let placement = self.placement.as_ref().unwrap();
        let px = placement.px;
        let window = placement.painted;
        let surface_size = ((window.right - window.left) as u32, (window.bottom - window.top) as u32);
        let offset = (window.left - placement.window.left, window.top - placement.window.top);
        let rest = ((placement.panel.left - window.left) as f32 / px, (placement.panel.top - window.top) as f32 / px);
        let (shift, opacity) = (self.shift.value(now), self.opacity.value(now));
        // In from beyond the edge, its shadow's room too.
        let travel = if self.edge == Edge::Top { size.1 } else { size.0 } + self.theme.margin;
        let (mut x, mut y) = rest;
        match self.edge {
            Edge::Left => x -= shift * travel,
            Edge::Right => x += shift * travel,
            Edge::Top => y -= shift * travel,
        }
        // Its buttons: in its top right corner, or in their row outside it (see `Placement::rack`), from its corner.
        let rack = placement.rack.map(|(_, under)| {
            let (w, h) = RACK;
            (size.0 - w, if under { size.1 } else { -h }, w, h)
        });
        let at = cursor_position().map(|cursor| ((cursor.x - window.left) as f32 / px - x, (cursor.y - window.top) as f32 / px - y));
        let within = |(px_x, px_y): (f32, f32), (rx, ry, rw, rh): (f32, f32, f32, f32)| px_x >= rx && px_y >= ry && px_x < rx + rw && px_y < ry + rh;
        let pointer = at.filter(|at| within(*at, (0.0, 0.0, size.0, size.1)));
        let at_buttons = at.filter(|at| pointer.is_some() || rack.is_some_and(|rack| within(*at, rack)));
        let settings_lit = self.settings_lit();
        let shows = crate::ui::music::Shows::from(&self.controller.config.lock().unwrap().view);
        let family = self.theme.body.family;
        let ground = crate::ui::music::Ground::of(&shows, self.theme.dark);
        let (theme, frost, behind, skin_ground) = (&self.theme, self.frost, &mut self.behind, &mut self.music_ground);
        let music = &mut self.music;
        let layer = &mut self.music_layer;
        self.music_frames += 1;
        let key = self.music_frames;
        let mut painted = None;
        let pinned = self.pinned;
        // Its settings and its pin (as the readings' bar has them) and the full screen, at its top right while the
        // pointer is over it.
        let mut buttons = Vec::new();
        let surface = self.surface.as_mut().unwrap();
        surface.offset(offset);
        let drawn = surface.draw(&self.gfx, surface_size, px, |frame| {
            if opacity < 1.0 {
                let everything = D2D_RECT_F { left: -f32::MAX, top: -f32::MAX, right: f32::MAX, bottom: f32::MAX };
                let layer = D2D1_LAYER_PARAMETERS1 { contentBounds: everything, opacity, ..Default::default() };
                unsafe { frame.dc.PushLayer(&layer, None) };
            }
            // On the skin: its ground under it, as the readings' is under them.
            if let crate::ui::music::Ground::Skin { .. } = ground {
                frame.origin(0.0, 0.0);
                let backdrop = behind.as_mut().and_then(|behind| behind.backdrop(&frame.dc, px, window, rest));
                if let Err(error) = skin_ground.draw_whole(frame, theme, size, backdrop, frost, false, Matrix3x2::translation(x, y), px) {
                    crate::journal::note(format!("panel: what plays' ground not drawn: {error}"));
                }
            }
            frame.origin(x, y);
            // (Round it, the room for its shadow.)
            let area = (-24.0, -24.0, size.0 + 48.0, size.1 + 48.0);
            let drew = layer.draw(frame, key, windows_numerics::Matrix3x2::identity(), area, px, None, |frame| {
                painted = Some(music.paint(frame, size, &shows, ground, family, px, pointer, now));
                Ok(())
            });
            if drew.is_err() {
                painted = None;
            }
            if at_buttons.is_some() || settings_lit.is_some() {
                // (The pointer away: over none of them.)
                let (px_x, px_y) = at_buttons.unwrap_or((f32::NEG_INFINITY, f32::NEG_INFINITY));
                let colour = music.ground_colour(size);
                let (side, gap) = (crate::ui::buttons::SIZE, crate::ui::buttons::GAP);
                // (In its corner, or in their row.)
                let (top, right) = match rack {
                    Some((rx, ry, rw, _)) => (ry + RACK_INSET, rx + rw),
                    None => music.top_right(size),
                };
                for (index, (button, hit)) in [(crate::ui::buttons::Button::Pin, Hit::Pin), (crate::ui::buttons::Button::Settings, Hit::Settings), (crate::ui::buttons::Button::Full, Hit::Full)].into_iter().enumerate() {
                    let (bx, by) = (right - side - index as f32 * (side + gap), top);
                    let over = px_x >= bx && px_x < bx + side && px_y >= by && px_y < by + side;
                    let settings = button == crate::ui::buttons::Button::Settings;
                    let lit = (button == crate::ui::buttons::Button::Pin && pinned) || (settings && settings_lit.is_some()) || over;
                    let grown = if settings { settings_lit.unwrap_or(1.0) } else { 1.0 };
                    let centre = crate::ui::canvas::Point { x: bx + side / 2.0, y: by + side / 2.0 };
                    // (On the skin, as the readings' are; else in the colour of its own ground there.)
                    match ground {
                        crate::ui::music::Ground::Skin { .. } => crate::ui::buttons::paint(frame, centre, button, lit, theme.dark, theme.text3.alpha(0.5), grown),
                        _ => crate::ui::buttons::paint_on(frame, centre, button, lit, colour, grown),
                    }
                    buttons.push((bx, by, side, side, hit));
                }
            }
            frame.origin(0.0, 0.0);
            if opacity < 1.0 {
                unsafe { frame.dc.PopLayer() };
            }
        });
        let Some(painted) = painted.filter(|_| drawn.is_ok()) else {
            // Not drawn: drawn again shortly (the device gone, on a new one).
            gfx::failed("panel", &self.gfx, drawn.as_ref().err());
            self.next_frame = now + DEVICE_RETRY;
            return;
        };
        self.gfx.sweep();
        // (Its buttons over what is under them.)
        self.hits = buttons.into_iter().chain(painted.hits).collect();
        self.follow_settings();
        self.corner = (x + offset.0 as f32 / px, y + offset.1 as f32 / px);
        self.show_stations();
        self.bar = view::Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
        // (Settling after it was sized, too: see `music_settle`.)
        let moving = !self.shift.done(now) || !self.opacity.done(now) || painted.busy || self.music_settle.is_some() || settings_window::bounce(self.settings_from).is_some();
        self.next_frame = if moving { now } else { now + Duration::from_millis(250) };
    }

    /// Whether the settings, open, were asked from its settings' button (it shows so, lit, and they pour back into it).
    fn settings_here(&self) -> bool {
        let from = settings_window::asked_from();
        from.is_some() && from == self.settings_from
    }

    /// Its settings' button lit, and how large (see `view::Scene::settings_lit`): the settings asked from it open, or
    /// just poured back into it.
    fn settings_lit(&self) -> Option<f32> {
        if self.settings_here() { Some(1.0) } else { settings_window::bounce(self.settings_from) }
    }

    /// The settings asked from its settings' button, open: told where it is now, drawn somewhere else (the panel laid
    /// out anew), so they pour back into it there. Asked from elsewhere, or closed and done bouncing: no longer its.
    fn follow_settings(&mut self) {
        if !self.settings_here() {
            if settings_window::bounce(self.settings_from).is_none() {
                self.settings_from = None;
            }
            return;
        }
        let Some(now) = self.on_screen(Hit::Settings) else { return };
        if let Some(from) = self.settings_from.filter(|from| *from != now) {
            settings_window::asked_from_moved(from, Some(now));
            self.settings_from = Some(now);
        }
    }

    /// Where on the screen the region `hit` was last drawn (physical px), if it was.
    fn on_screen(&self, hit: Hit) -> Option<RECT> {
        let placement = self.placement.as_ref()?;
        let (px, window) = (placement.px, placement.window);
        let screen = |at: f32, from: f32, origin: i32| origin + ((at + from) * px).round() as i32;
        let (x, y, w, h, _) = self.hits.iter().find(|region| region.4 == hit)?;
        Some(RECT { left: screen(*x, self.corner.0, window.left), top: screen(*y, self.corner.1, window.top), right: screen(x + w, self.corner.0, window.left), bottom: screen(y + h, self.corner.1, window.top) })
    }

    /// Which hit region a point in the window's client area (physical px) is in.
    fn hit_at(&self, client: POINT) -> Option<Hit> {
        self.hits_at(client).next()
    }

    /// Where each thing drawn is on the screen (physical px), as it was just drawn.
    fn hits_on_screen(&self) -> Vec<(RECT, Hit)> {
        let Some(placement) = &self.placement else { return Vec::new() };
        let (px, window) = (placement.px, placement.window);
        let screen = |at: f32, from: f32, origin: i32| origin + ((at + from) * px).round() as i32;
        self.hits
            .iter()
            .map(|(x, y, w, h, hit)| (RECT { left: screen(*x, self.corner.0, window.left), top: screen(*y, self.corner.1, window.top), right: screen(x + w, self.corner.0, window.left), bottom: screen(y + h, self.corner.1, window.top) }, *hit))
            .collect()
    }

    /// Where it shows a station, as it was just drawn, told to the station (see `station::run::shows_at`); where what
    /// may be looked at larger is, told to the look at one of them up (see `ui::look::tiles`).
    fn show_stations(&self) {
        let on_screen = self.hits_on_screen();
        crate::station::run::shows_at(self.window.hwnd, on_screen.iter().filter_map(|(rect, hit)| Some((*rect, hit.lands()?))).collect());
        crate::ui::look::tiles(self.window.hwnd, looks_in(&on_screen).into_iter().map(|(hit, rect)| (hit.key(), rect)).collect());
    }

    /// Each hit region a point in the window's client area (physical px) is in, in the order they were drawn.
    fn hits_at(&self, client: POINT) -> impl Iterator<Item = Hit> + '_ {
        view::under(&self.hits, self.dips(client)).map(|hit| hit.4)
    }

    /// A point in the window's client area (physical px) where the panel draws it (its DIPs, from its corner).
    fn dips(&self, client: POINT) -> (f32, f32) {
        let px = self.placement.as_ref().map_or(1.0, |placement| placement.px);
        (client.x as f32 / px - self.corner.0, client.y as f32 / px - self.corner.1)
    }

    /// Whether a thing kept at hand is being dragged among the others on it.
    fn dragging_app(&self) -> bool {
        self.app_press.as_ref().is_some_and(|(_, _, moved)| *moved)
    }

    /// What is under the pointer found again where it stays as what is drawn moves under it (a list scrolled, things
    /// moving to new places): lit as it is.
    fn hover_again(&mut self) {
        let Some(at) = self.pointer else { return };
        let hover = self.hit_at(at).filter(|hit| hit.list().is_none());
        if hover != self.hover {
            // (What was said of what was there is not of what is there now.)
            self.unsay();
            self.hover = hover;
            self.next_frame = Instant::now();
        }
    }

    /// What it said under the pointer taken away.
    fn unsay(&mut self) {
        if std::mem::take(&mut self.saying) {
            crate::ui::tip::hide();
        }
    }

    /// A thing kept at hand held and moved, the pointer at `client` (physical px in the window): where it has it, and
    /// the place it would go (the one it is over, where it is over one), drawn so as soon as can be.
    fn held_at(&mut self, client: POINT) {
        let px = self.placement.as_ref().map_or(1.0, |placement| placement.px);
        self.app_point = (client.x as f32 / px - self.corner.0, client.y as f32 / px - self.corner.1);
        // (The place it is over, which stands still as the others make room: whatever else is drawn there, the "+"
        // after them among it.)
        let slot = self.hits_at(client).find_map(|hit| if let Hit::AppSlot(slot) = hit { Some(slot) } else { None });
        if let Some(slot) = slot {
            self.app_over = Some(slot);
        }
        self.next_frame = Instant::now();
    }

    /// The pointer moved over the panel: lights what it is over.
    fn hover_at(&mut self, client: POINT) {
        self.pointer = Some(client);
        // Off the panel (its window holds its screen): clicks there are let through at once, not as the pointer is
        // next followed (see `track`), which a quick click comes before.
        if !self.held() && !self.on_panel(client) {
            self.window.set_click_through(true);
            return;
        }
        // Unpinned, its edges and its bar show what a drag there does (the
        // rest moves it too, under the reader's arrow).
        match self.drag.map(|drag| drag.edges).or_else(|| self.grip_at(client).filter(|edges| edges.any() || self.on_bar(client))) {
            Some(edges) => point_at(&self.window, edges),
            // Over what a press does something with (a button, a drive's row): a hand.
            None if self.hit_at(client).is_some_and(|hit| hit.takes_press()) => self.window.point(windows::Win32::UI::WindowsAndMessaging::IDC_HAND),
            None => self.window.point(windows::Win32::UI::WindowsAndMessaging::IDC_ARROW),
        }
        let hover = self.hit_at(client).filter(|hit| hit.list().is_none());
        if hover != self.hover {
            // What a press does, said under what says so (a drive's row, the line that switches players).
            match (hover.and_then(|hit| Some((hit, says(hit, self.lang, &self.hits)?))), self.placement.as_ref()) {
                (Some((hit, said)), Some(placement)) => {
                    if let Some(&(x, y, w, h, _)) = self.hits.iter().find(|one| one.4 == hit) {
                        let on_screen = |dx: f32, dy: f32| ((placement.window.left as f32 + (self.corner.0 + dx) * placement.px).round() as i32, (placement.window.top as f32 + (self.corner.1 + dy) * placement.px).round() as i32);
                        let ((left, top), (right, bottom)) = (on_screen(x, y), on_screen(x + w, y + h));
                        crate::ui::tip::show(self.window.hwnd, &said, RECT { left, top, right, bottom }, crate::os::apps_dark());
                        self.saying = true;
                    }
                }
                _ => self.unsay(),
            }
            // (The gear under the pointer: the settings made ready, to come up at once as it is pressed.)
            if hover == Some(Hit::Settings) {
                crate::ui::settings_window::prepare();
            }
            self.hover = hover;
            self.next_frame = Instant::now();
        }
    }

    fn click(&mut self, client: POINT) {
        self.next_frame = Instant::now();
        if self.shelf_press.take().is_some() {
            crate::ui::window::let_go_of_capture(self.window.hwnd);
        }
        let hit = self.hit_at(client);
        // A to-do being written: a press away from its field gives back the keys.
        if !matches!(hit, Some(Hit::Todo(view::TodoHit::Field | view::TodoHit::Add | view::TodoHit::FieldDue))) {
            self.stop_todo(false);
        }
        match hit {
            Some(Hit::Todo(hit)) => match crate::ui::todo::pressed(hit) {
                crate::ui::todo::Act::Nothing => {}
                crate::ui::todo::Act::Write => self.write_todo(),
                crate::ui::todo::Act::Pick(of) => {
                    if let Some(anchor) = self.on_screen(Hit::Todo(hit)) {
                        crate::ui::todo::pick::open(self.window.hwnd, of, anchor, self.menu_look(), self.lang);
                    }
                }
            },
            Some(Hit::Settings) => {
                self.settings_from = self.on_screen(Hit::Settings);
                crate::show_settings(self.settings_from);
            }
            Some(Hit::Full) => {
                self.full_asked = self.placement.as_ref().map(|placement| placement.contact);
            }
            Some(Hit::Pin) => {
                // Pinned, it stays as it is: not moved, not sized, and not
                // closed as the pointer leaves. Away from the edge, kept so
                // across restarts.
                self.pinned ^= true;
                if self.floating() {
                    crate::app().place_panel(self.kept_at());
                } else {
                    // At its edge, kept so across restarts too (that edge's alone).
                    if let Some(placement) = &self.placement {
                        crate::app().pin_panel((placement.anchor.x, placement.anchor.y), self.edge, self.pinned);
                    }
                }
            }
            Some(Hit::Game) => {
                // (Its lane comes or goes at the next frame: see `arrange`.)
                crate::widget::press_game();
                self.next_frame = Instant::now();
            }
            Some(Hit::Overlay) => {
                // As it is on screen now: shown, it closes; not, it opens.
                self.controller.set_overlay(!self.controller.overlay_wanted());
            }
            Some(Hit::Media(key, follow)) => crate::playing::act_on(follow, crate::playing::Act::Key(key)),
            Some(Hit::Line(index)) => {
                crate::ui::music::play_line(index);
                self.music.back_to_now();
            }
            Some(Hit::Drive(letter)) => open_drive(letter),
            Some(hit) if crate::station::pressed(hit) => {}
            Some(Hit::Tab(tab)) => self.tab = tab,
            Some(Hit::AddApp) => self.open_beside(crate::ui::beside::Page::Picker),
            Some(Hit::NewNote) => self.open_beside(crate::ui::beside::Page::Note(None)),
            Some(Hit::Search) => crate::ui::find::toggle(),
            Some(Hit::Note(number)) => {
                if let Some(name) = crate::notes::named(number) {
                    self.open_beside(crate::ui::beside::Page::Note(Some(name)));
                }
            }
            Some(Hit::Sort(sort)) if sort != self.drawn.processes.sort => {
                self.prefs.processes.sort = sort;
                self.drawn.processes.sort = sort;
                self.scrolls.reset();
                // Kept for next time, as if chosen in the settings.
                // Saving tells this thread to restyle; not from here.
                std::thread::spawn(move || crate::app().set_process_sort(sort));
            }
            _ => {}
        }
    }

    /// A wheel turn with the pointer at `screen`: scrolls the list under it (under what is on it: the wheel over a thing
    /// on the station scrolls the station, and over its bar too).
    fn wheel(&mut self, screen: POINT, delta: i16) {
        let Some(window) = self.placement.as_ref().map(|p| p.window) else { return };
        // What plays: its lines turned.
        if self.part == Part::Music {
            let on_sound = self.hit_at(POINT { x: screen.x - window.left, y: screen.y - window.top }).is_some_and(Hit::of_sound);
            self.music.wheel(delta, on_sound);
            self.next_frame = Instant::now();
            return;
        }
        let client = POINT { x: screen.x - window.left, y: screen.y - window.top };
        let under: Vec<Hit> = self.hits_at(client).collect();
        if view::wheel(&mut self.scrolls, &under, delta, false) {
            self.next_frame = Instant::now();
        }
    }
}

/// The samples read, oldest first, in one piece as the sampler keeps them (see `Controller::record`): read under
/// the history's read lock, never made so there (that would need it written).
pub fn contiguous(history: &VecDeque<Sample>) -> &[Sample] {
    let (samples, rest) = history.as_slices();
    debug_assert!(rest.is_empty(), "the history is kept in one piece");
    samples
}

/// What the readings are drawn with at this moment.
fn scene<'s>(
    controller: &'s Controller,
    info: &'s StaticInfo,
    prefs: &'s Prefs,
    theme: &'s Theme,
    lang: Lang,
    scrolls: view::Scrolls,
    tab: view::Tab,
    hover: Option<Hit>,
    pinned: bool,
    history: &'s [Sample],
    seen: &'s Seen,
) -> Scene<'s> {
    let interval = controller.config.lock().unwrap().interval;
    let overlay = controller.overlay_wanted();
    let wall = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64() * 1000.0;
    Scene {
        thumbs: &crate::ui::view::RESTING,
        info,
        prefs,
        theme,
        lang,
        history,
        seen,
        pen_ms: wall - interval.as_secs_f64() * 1000.0 - PEN_LAG_MS,
        scrolls,
        tab,
        hover,
        pinned,
        overlay,
        buttons: true,
        newer: crate::update::available().is_some(),
        settings_lit: None,
        music_apart: false,
        hint: None,
    }
}

/// What a desktop widget's readings are drawn with at this moment, of the machine `info` says (`Controller::info`).
pub(crate) fn scene_for<'s>(controller: &'s Controller, info: &'s StaticInfo, prefs: &'s Prefs, theme: &'s Theme, lang: Lang, history: &'s [Sample], seen: &'s Seen) -> Scene<'s> {
    Scene { buttons: false, ..scene(controller, info, prefs, theme, lang, view::Scrolls::default(), view::Tab::default(), None, false, history, seen) }
}

/// The sink's procedure: the input loop takes its messages before they are dispatched.
unsafe extern "system" fn sink_procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// A row of buttons outside the panel (see `Placement::rack`): its width and height (DIPs), at its right end.
const RACK: (f32, f32) = (3.0 * crate::ui::buttons::SIZE + 2.0 * crate::ui::buttons::GAP, crate::ui::buttons::SIZE + 2.0 * RACK_INSET);
/// The room between such a row and the panel, and round it.
const RACK_INSET: f32 = 6.0;

/// Positions the panel for a layout `size` DIPs large, in its window (see `Placement::window`); with a row of buttons
/// `rack` large outside it at its right end (above it, or under it where its screen has no room above), where it has
/// one.
fn place(placement: &mut Placement, edge: Edge, theme: &Theme, size: (f32, f32), zoom: f32, rack: Option<(f32, f32)>) {
    let Contact { monitor, work, scale } = placement.contact;
    let px = scale * zoom;
    let (width, height) = ((size.0 * px).round() as i32, (size.1 * px).round() as i32);
    let gap = (GAP * scale).round() as i32;
    let inset = (theme.inset * px).round() as i32;
    let centred = |at: i32, length: i32, low: i32, high: i32| (at - length / 2).min(high - gap - length).max(low + gap);
    // Moved away from the edge: where the pointer holds it, or as far into
    // the room on its screen's work area as it was put, wholly on it.
    let into = |share: f32, low: i32, high: i32, length: i32| low + gap + (share.clamp(0.0, 1.0) * (high - low - 2 * gap - length).max(0) as f32).round() as i32;
    let (left, top) = match (placement.held, placement.floating, edge) {
        (Some(held), ..) => (if held.right { held.at.x - width } else { held.at.x }, if held.bottom { held.at.y - height } else { held.at.y }),
        (None, Some((x, y)), _) => (into(x, work.left, work.right, width), into(y, work.top, work.bottom, height)),
        (None, None, Edge::Left) => (monitor.left + inset, centred(placement.anchor.y, height, work.top, work.bottom)),
        (None, None, Edge::Right) => (monitor.right - inset - width, centred(placement.anchor.y, height, work.top, work.bottom)),
        (None, None, Edge::Top) => (centred(placement.anchor.x, width, work.left, work.right), monitor.top + inset),
    };
    let panel = RECT { left, top, right: left + width, bottom: top + height };
    let rack = rack.map(|(w, h)| {
        let (w, h) = ((w * px).ceil() as i32, (h * px).ceil() as i32);
        let under = panel.top - h < work.top;
        let top = if under { panel.bottom } else { panel.top - h };
        (RECT { left: panel.right - w, top, right: panel.right, bottom: top + h }, under)
    });
    let margin = (theme.margin * px).ceil() as i32;
    // The shadow's room, kept on this monitor (while it is sized away from
    // the edge, past it too); and its buttons' row's.
    let mut painted = RECT { left: left - margin, top: top - margin, right: panel.right + margin, bottom: panel.bottom + margin };
    if let Some((r, _)) = rack {
        painted = RECT { left: painted.left.min(r.left), top: painted.top.min(r.top), right: painted.right.max(r.right), bottom: painted.bottom.max(r.bottom) };
    }
    if placement.held.is_none() {
        painted = RECT {
            left: painted.left.max(monitor.left),
            top: painted.top.max(monitor.top),
            right: painted.right.min(monitor.right),
            bottom: painted.bottom.min(monitor.bottom),
        };
    }
    let tolerance = (LEAVE_TOLERANCE * scale).round() as i32;
    let mut reach = RECT {
        left: panel.left - tolerance,
        top: panel.top - tolerance,
        right: panel.right + tolerance,
        bottom: panel.bottom + tolerance,
    };
    // The pointer arrives from the screen edge, if it is against it: from where it pushed into it too, wherever
    // along it that is. (Opened in the middle of the edge from a push near its end, the pointer was never within
    // its reach, and the panel took it for one opened from afar, which stays till the pointer comes: moved away from
    // it, it stayed.)
    if placement.floating.is_none() && placement.held.is_none() {
        match edge {
            Edge::Left => reach.left = monitor.left,
            Edge::Right => reach.right = monitor.right,
            Edge::Top => reach.top = monitor.top,
        }
        if let Some(from) = placement.came_from {
            reach = RECT {
                left: reach.left.min(from.x - tolerance),
                top: reach.top.min(from.y - tolerance),
                right: reach.right.max(from.x + tolerance),
                bottom: reach.bottom.max(from.y + tolerance),
            };
        }
    }
    // (The pointer going to its buttons' row is still at it.)
    if let Some((r, _)) = rack {
        reach = RECT { left: reach.left.min(r.left), top: reach.top.min(r.top), right: reach.right.max(r.right), bottom: reach.bottom.max(r.bottom) };
    }
    placement.rack = rack;
    if placement.painted != painted {
        // Where it was and where it is: a widget that took the desktop with
        // it there takes it again.
        crate::widget::own_window_changed(placement.painted);
        crate::widget::own_window_changed(painted);
    }
    placement.panel = panel;
    placement.painted = painted;
    placement.reach = reach;
    placement.px = px;
}

/// Over the middle of a panel `area` DIPs large while it is sized by its
/// edges: the size it is at, or the limit it has reached (see `tag`).
fn size_badge(frame: &gfx::Frame, theme: &Theme, size: f32, limit: Option<Limit>, lang: Lang, area: (f32, f32)) {
    let percent = format!("{:.0}%", size * 100.0);
    let text = match limit {
        None => percent,
        Some(Limit::Smallest) => format!("{} {percent}", lang.pick("最小", "Smallest:")),
        Some(Limit::Largest) => format!("{} {percent}", lang.pick("最大", "Largest:")),
        Some(Limit::Screen) => lang.pick("已占满屏幕", "As large as the screen holds").to_string(),
    };
    tag(frame, theme, &text, (area.0 / 2.0, area.1 / 2.0), limit.is_some());
}

/// Whether `outer` holds all of `inner`.
fn covers(outer: RECT, inner: RECT) -> bool {
    inner.left >= outer.left && inner.top >= outer.top && inner.right <= outer.right && inner.bottom <= outer.bottom
}

/// Whether a panel at `panel` (physical px) on `contact`'s screen is near
/// enough to the edge it opens from to be taken back to it (see `DOCK`).
fn docks(edge: Edge, contact: Contact, panel: RECT) -> bool {
    let (monitor, near) = (contact.monitor, (DOCK * contact.scale).round() as i32);
    match edge {
        Edge::Left => panel.left - monitor.left <= near,
        Edge::Right => monitor.right - panel.right <= near,
        Edge::Top => panel.top - monitor.top <= near,
    }
}

/// A word on what a drag does, centred at `centre` (DIPs), in the look of
/// the panel's skin (its sheet, ink and type); `heed`, its edge and words in
/// the skin's signal colour.
fn tag(frame: &gfx::Frame, theme: &Theme, text: &str, centre: (f32, f32), heed: bool) {
    use crate::ui::canvas::{Align, Canvas, Color, Fill};
    let font = theme.title;
    let (ascent, descent) = frame.baseline(font);
    let (pad_x, pad_y) = (14.0, 7.0);
    let (width, height) = (frame.measure(text, font) + 2.0 * pad_x, ascent + descent + 2.0 * pad_y);
    let (x, y) = (centre.0 - width / 2.0, centre.1 - height / 2.0);
    // Each skin's own sheet: paper, Windows 11's tint under its stroke, or
    // glass under its rim.
    let (ground, edge, radius) = match theme.skin {
        Skin::Paper => (theme.paper, theme.text, theme.control_radius),
        Skin::Fluent => (Color { a: 0.96, ..theme.tint }, theme.stroke, theme.control_radius.max(6.0)),
        Skin::Glass => (Color { a: 0.9, ..theme.glass }, theme.legibility, theme.radius.min(height / 2.0)),
    };
    let (edge, ink) = if heed { (theme.signal, theme.signal) } else { (edge, theme.text) };
    frame.fill_rounded(ground, x, y, width, height, radius);
    let line = 1.0;
    frame.stroke_rounded(Fill::Solid(edge), x, y, width, height, radius, line);
    Canvas::text(frame, text, font, ink, x + pad_x, y + pad_y, width, Align::Start);
}

/// Whether Windows has animations turned off ("Animation effects" in its
/// accessibility settings).
pub(crate) fn reduced_motion() -> bool {
    let mut animate = windows::core::BOOL(1);
    let read = unsafe {
        SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut animate as *mut _ as *mut _), Default::default())
    };
    read.is_ok() && !animate.as_bool()
}

/// A press away from a menu that closed it: all the menu's, its button's coming up too. (Else, let go on the button that
/// had opened the menu, pressed again where it was, it opened it again: a menu could not be closed by its own button.)
/// So too a press on a thing that a look shows (see `ui::look`): the look's, not a click on that thing (which, let go,
/// copied it or opened it).
#[derive(Default)]
struct ClosingPress {
    /// The message its button comes up with, till it has.
    up: Option<u32>,
}

impl ClosingPress {
    /// The menu closed by `message`: by a press, that press is the menu's.
    fn closed_by(&mut self, message: u32) {
        use windows::Win32::UI::WindowsAndMessaging::{WM_MBUTTONDOWN, WM_MBUTTONUP, WM_RBUTTONDOWN, WM_RBUTTONUP};
        self.up = match message {
            WM_LBUTTONDOWN => Some(WM_LBUTTONUP),
            WM_RBUTTONDOWN => Some(WM_RBUTTONUP),
            WM_MBUTTONDOWN => Some(WM_MBUTTONUP),
            _ => None,
        };
    }

    /// Whether `message` is its button coming up, nothing more. (Another press first: that one is not the menu's, nor
    /// what comes up after it.)
    fn takes(&mut self, message: u32) -> bool {
        use windows::Win32::UI::WindowsAndMessaging::{WM_MBUTTONDOWN, WM_RBUTTONDOWN};
        let Some(up) = self.up else { return false };
        if matches!(message, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN) {
            self.up = None;
            return false;
        }
        if message == up {
            self.up = None;
            return true;
        }
        false
    }
}

/// The work area of the monitor under the pointer, and its scale factor.
pub fn work_area_at_cursor() -> Option<(RECT, f64)> {
    let contact = monitor_at(cursor_position()?)?;
    Some((contact.work, contact.scale as f64))
}

pub(crate) fn contains(rect: &RECT, point: POINT) -> bool {
    (rect.left..rect.right).contains(&point.x) && (rect.top..rect.bottom).contains(&point.y)
}

/// Where the pointer is; `None` while another desktop has the input (a UAC
/// prompt, the lock screen).
/// Brings a game the panel sent to the background back to the front, if it
/// is not there already. One that went down (minimized) is restored as a
/// click on its taskbar button does it: it restores itself (a protected
/// game's window refuses ShowWindow from another process) and takes the
/// screen back.
fn bring_back(game: Option<HWND>) {
    let Some(game) = game.filter(|&game| unsafe { GetForegroundWindow() } != game) else { return };
    unsafe {
        let _ = SetForegroundWindow(game);
        if IsIconic(game).as_bool() {
            let _ = PostMessageW(Some(game), WM_SYSCOMMAND, WPARAM(SC_RESTORE as usize), LPARAM(0));
        }
    }
}

/// The time to the screen's next refresh, as the desktop compositor keeps
/// it; one tick of the pointer's watch when it cannot tell.
fn until_refresh() -> Duration {
    let mut timing = DWM_TIMING_INFO { cbSize: size_of::<DWM_TIMING_INFO>() as u32, ..Default::default() };
    let (mut now, mut frequency) = (0i64, 0i64);
    let known = unsafe { DwmGetCompositionTimingInfo(HWND::default(), &mut timing).is_ok() && QueryPerformanceCounter(&mut now).is_ok() && QueryPerformanceFrequency(&mut frequency).is_ok() };
    let (period, last) = (timing.qpcRefreshPeriod as i64, timing.qpcVBlank as i64);
    if !known || period <= 0 || frequency <= 0 {
        return Duration::from_millis(TICK_MS as u64);
    }
    // The first refresh after now.
    let next = last + ((now - last).div_euclid(period) + 1) * period;
    Duration::from_nanos(((next - now) as i128 * 1_000_000_000 / frequency as i128) as u64)
}

/// Whether a game holds the screen: in exclusive fullscreen, or in a
/// borderless window in front covering its screen as it presents frames.
/// Over one, the settings say what may open the panel (see
/// `OverFullscreen`): a push into the edge may be an accident mid-game.
fn fullscreen_game() -> bool {
    exclusive_fullscreen() || crate::presents::presenting().iter().any(|game| game.in_front && game.fills_screen)
}

/// Whether a program holds the screen in exclusive fullscreen (Direct3D's
/// own, not a borderless window): a window shown over it sends it to the
/// background.
fn exclusive_fullscreen() -> bool {
    unsafe { SHQueryUserNotificationState() }.is_ok_and(|state| state == QUNS_RUNNING_D3D_FULL_SCREEN)
}

/// What a press on the row of the drive `name` does, said.
pub(crate) fn opens_drive(name: &str, lang: Lang) -> String {
    match lang {
        Lang::Zh => format!("打开 {name}"),
        Lang::En => format!("Open {name}"),
    }
}

/// Of what is drawn where (`on_screen`, as it was just drawn), what may be looked at larger (see `ui::look`), each once,
/// in the order it is drawn, and where its tile is.
pub(crate) fn looks_in(on_screen: &[(RECT, Hit)]) -> Vec<(Hit, RECT)> {
    let mut looks: Vec<(Hit, RECT)> = Vec::new();
    for &(rect, hit) in on_screen {
        if hit.looks() && !looks.iter().any(|(seen, _)| *seen == hit) {
            looks.push((hit, rect));
        }
    }
    looks
}

/// What is said under `hit` as the pointer comes over it (`hits` as last drawn): what a press on it does, for what
/// says so (a drive's row; the line that switches players); a thing's whole name, where it shows shortened.
pub(crate) fn says(hit: Hit, lang: Lang, hits: &[crate::ui::view::HitBox]) -> Option<String> {
    if let Some(name) = crate::ui::view::named(hits, hit, lang) {
        return Some(name);
    }
    match hit {
        Hit::Drive(letter) => Some(opens_drive(&format!("{letter}:"), lang)),
        Hit::Media(crate::playing::Key::Player, _) => {
            let players = crate::playing::now().map_or(0, |now| now.players);
            Some(match lang {
                Lang::Zh => format!("切换到下一个播放器（共 {players} 个）"),
                Lang::En => format!("Switch to the next player ({players} in all)"),
            })
        }
        _ => None,
    }
}

/// Opens the drive seen at `place` (see `Hit::Drive`) where the user browses files, on another thread (the shell may
/// take a while).
pub(crate) fn open_drive(letter: char) {
    std::thread::spawn(move || {
        let root = windows::core::HSTRING::from(format!("{letter}:\\"));
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
            windows::Win32::UI::Shell::ShellExecuteW(None, windows::core::w!("open"), &root, windows::core::PCWSTR::null(), windows::core::PCWSTR::null(), windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL);
        }
    });
}

/// Whether a mouse button is down: as the sensors' process says while there is one (it reads them truly whatever
/// window is in front), else as read here.
pub(crate) fn buttons_down() -> bool {
    if crate::source::linked() {
        return BUTTONS_DOWN.load(Ordering::Relaxed);
    }
    crate::mouse::buttons_held()
}

/// The buttons as the sensors' process last said: whether one is down, and how many presses it has told.
static BUTTONS_DOWN: AtomicBool = AtomicBool::new(false);
static PRESSES: AtomicU64 = AtomicU64::new(0);
/// Presses told (`new`) matched with those seen on the panel's own windows (`ours`, and `owed`: seen before and not
/// yet matched, the telling coming after): those of its own not yet matched now, and whether any told was elsewhere.
fn matched(new: u64, ours: u64, owed: u64) -> (u64, bool) {
    let ours = ours + owed;
    (ours.saturating_sub(new), new > ours)
}

/// The presses seen on the panel's own windows (it, its menu): see `Panel::presses_since`.
static OURS: AtomicU64 = AtomicU64::new(0);

/// What the sensors' process says of the buttons (see `Told::Buttons`).
pub(crate) fn heard_buttons(down: bool, pressed: bool) {
    BUTTONS_DOWN.store(down, Ordering::Relaxed);
    if pressed {
        PRESSES.fetch_add(1, Ordering::Relaxed);
    }
}

/// Where the pointer has come from: the screen it is on (and its scale), how far it has been from each of that
/// screen's sides (left, top, right, bottom; physical px) since it came on, and the screen it last came across from,
/// and when. A screen it was on only by its side, no further in than it may overshoot (`SEAM_OVERSHOOT`), it is not
/// taken to have come across from: it was not making for that side from inside it, it had gone past the seam the
/// other way and come back.
#[derive(Default)]
struct Approach {
    on: Option<(RECT, f32)>,
    reach: [i32; 4],
    crossed: Option<(RECT, Instant)>,
}

impl Approach {
    /// The pointer at `cursor`, on the screen `here` (and its scale), at `now`.
    fn moved(&mut self, cursor: POINT, here: Option<(RECT, f32)>, now: Instant) {
        let away = |m: RECT| [cursor.x - m.left, cursor.y - m.top, m.right - 1 - cursor.x, m.bottom - 1 - cursor.y];
        if here.map(|(m, _)| m) == self.on.map(|(m, _)| m) {
            if let Some((m, _)) = here {
                self.reach = std::array::from_fn(|side| self.reach[side].max(away(m)[side]));
            }
            return;
        }
        if let Some((from, scale)) = self.on {
            // The side of it the pointer has gone past, and whether it had been well in from that side.
            let past = away(from);
            let deep = (0..4).filter(|&side| past[side] < 0).any(|side| self.reach[side] > (SEAM_OVERSHOOT * scale).round() as i32);
            if deep {
                self.crossed = Some((from, now));
            }
        }
        self.on = here;
        self.reach = here.map_or([0; 4], |(m, _)| away(m));
    }

    /// The screen the pointer came across from lately (see `SEAM_CROSSED`).
    fn came_from(&self, now: Instant) -> Option<RECT> {
        self.crossed.filter(|(_, at)| now.duration_since(*at) < SEAM_CROSSED).map(|(from, _)| from)
    }
}

/// The screen to open on when the pointer is on the seam where a screen's
/// `edge` meets another screen: within a few pixels of that line, on
/// either side of it (further past it, on the other screen, when it has
/// just come across from `came_from`, the edge's screen: it overshot),
/// clear of the line's ends, visible, no button held. The screen whose
/// edge it is.
fn seam_contact(cursor: POINT, edge: Edge, came_from: Option<RECT>) -> Option<Contact> {
    let here = monitor_at(cursor)?;
    let m = here.monitor;
    let near = (SEAM_ZONE * here.scale).round() as i32;
    // How far from its own side the pointer may be on this screen, when the
    // seam is on that side and the screen behind it is the edge's.
    let behind_point = match edge {
        Edge::Right => POINT { x: m.left - 1, y: cursor.y },
        Edge::Left => POINT { x: m.right, y: cursor.y },
        Edge::Top => POINT { x: cursor.x, y: m.bottom },
    };
    let overshot = came_from.is_some_and(|from| monitor_at(behind_point).is_some_and(|behind| behind.monitor == from));
    let far = if overshot { (SEAM_OVERSHOOT * here.scale).round() as i32 } else { near };
    let exists = |point: POINT| !unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONULL) }.is_invalid();
    // Near this screen's own edge where the pointer passes on from it, or
    // near the opposite side of it with a screen behind it whose edge that
    // is.
    let (own, along, other, behind) = match edge {
        Edge::Right => (cursor.x >= m.right - near, cursor.y, cursor.x < m.left + far, behind_point),
        Edge::Left => (cursor.x < m.left + near, cursor.y, cursor.x >= m.right - far, behind_point),
        Edge::Top => (cursor.y < m.top + near, cursor.x, cursor.y >= m.bottom - far, behind_point),
    };
    let target = if own && crate::screens::passes(m, edge, along) {
        here
    } else if other && exists(behind) {
        monitor_at(behind)?
    } else {
        return None;
    };
    let t = target.monitor;
    let corner = (CORNER_EXCLUSION * target.scale as f64).round() as i32;
    let clear = match edge {
        Edge::Left | Edge::Right => (t.top + corner..t.bottom - corner).contains(&cursor.y),
        Edge::Top => (t.left + corner..t.right - corner).contains(&cursor.x),
    };
    (clear && !buttons_down() && cursor_showing()).then_some(target)
}

/// Whether a mouse move told to a window being dragged says the button it is dragged by is up: what the window is
/// told, which it is told under its capture whatever window is in front (an elevated one too, where reading the
/// button's state would say "up" all along).
fn let_go(message: &MSG) -> bool {
    message.message == WM_MOUSEMOVE && message.wParam.0 & windows::Win32::System::SystemServices::MK_LBUTTON.0 as usize == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A panel opens as large as what it shows was made on its screen, and dealt into its own columns, however the other
    /// component's are: as far as the settings offer, what the file says aside. What plays, drawn as designed (made larger
    /// or smaller by its edges).
    #[test]
    fn a_panel_is_as_large_as_its_component_was_made() {
        let mut settings = Settings::default();
        settings.panel_size = 1.25;
        settings.size_panel(Part::Workbench, Some("a"), 0.8);
        settings.size_panel(Part::Monitor, Some("a"), 9.0);
        settings.set_columns(Part::Workbench, Some(3));
        let config = config_from(&settings);
        assert_eq!(config.size_on(Part::Workbench, Some("a")), 0.8);
        assert_eq!(config.size_on(Part::Monitor, Some("a")), SIZES.1);
        assert_eq!(config.size_on(Part::Workbench, Some("b")), 1.25);
        assert_eq!(config.size_on(Part::Music, Some("a")), 1.0);
        assert_eq!((config.columns, config.workbench_columns), (None, Some(3)));
    }

    /// The pointer that went past the seam a little and came back has not come across from the screen it went
    /// onto: it was making for its own screen's edge (the owner's portrait screen at 200 % left of the wide one at
    /// 150 %, the panel of the portrait one's right edge opened with the pointer back on the wide one). Come across
    /// from well inside a screen, it has, the other way as this.
    #[test]
    fn a_pointer_back_from_past_the_seam_has_not_come_from_there() {
        let tall = (RECT { left: -2905, top: -1459, right: -745, bottom: 2381 }, 2.0);
        let wide = (RECT { left: -745, top: -1440, right: 2695, bottom: 0 }, 1.5);
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let point = |x: i32| POINT { x, y: -700 };
        // From the middle of the wide screen to its left side, 30 px onto the tall one, and back.
        let mut approach = Approach::default();
        for (ms, x, on) in [(0, 900, wide), (100, -600, wide), (200, -775, tall), (300, -740, wide)] {
            approach.moved(point(x), Some(on), at(ms));
        }
        assert_ne!(approach.came_from(at(400)), Some(tall.0));
        // From well inside the wide screen onto the tall one: come across from the wide one, lately.
        let mut approach = Approach::default();
        for (ms, x, on) in [(0, 900, wide), (100, -760, tall)] {
            approach.moved(point(x), Some(on), at(ms));
        }
        assert_eq!(approach.came_from(at(200)), Some(wide.0));
        assert_eq!(approach.came_from(at(100) + SEAM_CROSSED), None);
        // From well inside the tall screen onto the wide one.
        let mut approach = Approach::default();
        for (ms, x, on) in [(0, -2000, tall), (100, -800, tall), (200, -730, wide)] {
            approach.moved(point(x), Some(on), at(ms));
        }
        assert_eq!(approach.came_from(at(300)), Some(tall.0));
    }

    /// A menu closed by a press away from it, on the button that opened it: that press is the menu's, its button let
    /// go there too (not taken by the button, as a press of it, to open the menu again); the next press is not.
    #[test]
    fn a_press_that_closes_a_menu_is_the_menus_as_it_is_let_go() {
        use windows::Win32::UI::WindowsAndMessaging::{WM_KEYDOWN, WM_RBUTTONDOWN, WM_RBUTTONUP};
        let mut press = ClosingPress::default();
        press.closed_by(WM_LBUTTONDOWN);
        assert!(!press.takes(WM_MOUSEMOVE));
        assert!(!press.takes(WM_RBUTTONUP));
        assert!(press.takes(WM_LBUTTONUP));
        assert!(!press.takes(WM_LBUTTONUP));
        // Closed by a key: no press of it.
        press.closed_by(WM_KEYDOWN);
        assert!(!press.takes(WM_LBUTTONUP));
        // Another press before it is let go: that press, and its letting go, are not the menu's.
        press.closed_by(WM_RBUTTONDOWN);
        assert!(!press.takes(WM_LBUTTONDOWN));
        assert!(!press.takes(WM_RBUTTONUP));
    }

    /// A press told by the sensors' process is one elsewhere only where no press on the panel or its menu accounts
    /// for it, whichever came first: the menu's press seen before it is told (the menu closed meanwhile), or told
    /// before it is seen.
    #[test]
    fn a_press_on_the_panels_own_menu_is_no_press_elsewhere() {
        // Seen on the menu, told after: nothing elsewhere then, nor as it is told.
        assert_eq!(matched(0, 1, 0), (1, false));
        assert_eq!(matched(1, 0, 1), (0, false));
        // Seen and told at once.
        assert_eq!(matched(1, 1, 0), (0, false));
        // Told, none seen: elsewhere.
        assert_eq!(matched(1, 0, 0), (0, true));
        // Two told, one of them the menu's: one elsewhere.
        assert_eq!(matched(2, 1, 0), (0, true));
    }
}
