//! Shows and hides the panel, and draws it while it is up.
//!
//! One thread does all of it: it reads the pointer, owns the panel's window
//! and draws every frame while the panel is on screen. Whether the pointer is
//! "on the panel" is decided from the global cursor position against where
//! the panel rests, so nothing that moves on screen feeds back into it.

use std::collections::VecDeque;
use std::f32::consts::E;

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::rc::Rc;
use std::sync::Mutex;
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
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, RegisterHotKey, UnregisterHotKey, MOD_NOREPEAT, VK_ESCAPE, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON,
};
use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER,
    RIDEV_INPUTSINK, RID_INPUT, RIM_TYPEMOUSE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DispatchMessageW, GetCursorInfo, GetCursorPos, KillTimer, WaitMessage, MsgWaitForMultipleObjects,
    PeekMessageW, PostMessageW, SetTimer, SystemParametersInfoW, CURSORINFO, CURSOR_SHOWING, HWND_MESSAGE, MSG, PM_REMOVE,
    QS_ALLINPUT, WM_HOTKEY,
    SPI_GETCLIENTAREAANIMATION, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_INPUT, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_RBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT,
    WM_TIMER,
};

use crate::detector::{Detector, Motion};
use crate::reading::{Sample, StaticInfo};
use crate::overlay::{Choice as OverlayChoice, Overlay};
use crate::settings::{Anchor, Edge, OverFullscreen, OverlaySettings, Settings};
use crate::ui::backdrop::Capture;
use crate::ui::gfx::{self, Gfx, Surface};
use crate::ui::arrange::{self, GAP};
use crate::ui::seen::Seen;
use crate::ui::render::{self, PanelLayers};
use crate::ui::motion::{Easing, Transition, LINEAR};
use crate::ui::prefs::Prefs;
use crate::ui::skins;
use crate::ui::text::Lang;
use crate::ui::theme::{self, Entrance, Skin, Theme};
use crate::ui::view::{self, Hit, HitBox, Layout, Scene, TABLE_ROW};
use crate::ui::settings_window;
use crate::ui::window::Window;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows_numerics::{Matrix3x2, Vector2};

/// The pointer may stray this far outside the panel and still count as on it (DIPs).
const LEAVE_TOLERANCE: f32 = 10.0;
/// Strip at each end of the edge that does not trigger (logical px): the
/// height of a title bar's buttons and of the taskbar, which own the corners.
const CORNER_EXCLUSION: f64 = 48.0;
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
const MOUSE_MOVE_ABSOLUTE: u16 = 1;
/// How soon to try again when drawing failed for want of a device.
const DEVICE_RETRY: Duration = Duration::from_millis(250);
/// How often a live backdrop is captured again.
const LIVE_INTERVAL: Duration = Duration::from_millis(250);
/// Rows a wheel notch scrolls the process list by.
const WHEEL_ROWS: f32 = 3.0;
/// How quickly the process list catches up with the wheel.
const SCROLL_EASE: f32 = 0.06;

/// The panel slides in from past the edge and fades up, and goes the same way.
pub(crate) const OPEN: (Duration, Easing) = (Duration::from_millis(260), Easing(0.16, 1.0, 0.3, 1.0));
pub(crate) const OPEN_FADE: Duration = Duration::from_millis(140);
pub(crate) const CLOSE: (Duration, Easing) = (Duration::from_millis(180), Easing(0.4, 0.0, 1.0, 1.0));
/// The sizes the panel and the overlay may be drawn at, as designed's share.
pub(crate) const SIZES: (f32, f32) = (0.75, 2.0);
/// With animations turned off in Windows the panel only fades.
const PLAIN_FADE: Duration = Duration::from_millis(120);

/// Requests to the input thread from elsewhere.
const OPEN_FROM_TRAY: u32 = WM_APP + 1;
const DISMISS: u32 = WM_APP + 2;
const RESTYLE: u32 = WM_APP + 3;
const OPEN_SETTINGS: u32 = WM_APP + 4;
const TOGGLE: u32 = WM_APP + 5;
/// A new sample: the overlay is drawn again.
const OVERLAY: u32 = WM_APP + 7;
/// The panel thread's own hotkey: Escape, while the panel is open.
const ESCAPE_HOTKEY: i32 = 1;

struct Config {
    edge: Edge,
    skin: String,
    live: bool,
    anchor: Anchor,
    columns: Option<usize>,
    size: f32,
    overlay: OverlaySettings,
    /// None: pushing into the edge opens nothing.
    pressure: Option<i32>,
    over_fullscreen: OverFullscreen,
    close_delay: Duration,
    interval: Duration,
    history: usize,
    view: serde_json::Value,
}

fn config_from(settings: &Settings) -> Config {
    Config {
        edge: settings.edge,
        skin: settings.skin.clone(),
        live: settings.live_backdrop,
        anchor: settings.anchor,
        columns: settings.columns,
        // As far as the settings offer: the file may say anything.
        size: settings.panel_size.clamp(SIZES.0, SIZES.1),
        overlay: settings.overlay.clone(),
        pressure: settings.sensitivity.pressure(),
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
    info: StaticInfo,
    config: Mutex<Config>,
    playing: Mutex<Playing>,
    /// The samples of the longest chart span, oldest first.
    pub history: Mutex<VecDeque<Sample>>,
    /// What the machine has shown it can read since Glance started.
    pub seen: Mutex<Seen>,
    /// The overlay, opened by a game, closed by hand for the rest of it.
    overlay_closed: AtomicBool,
    /// The input thread's message window, once it has one.
    sink: AtomicIsize,
    shown: AtomicBool,
}

impl Controller {
    pub fn new(info: StaticInfo, settings: &Settings) -> Self {
        Controller {
            info,
            config: Mutex::new(config_from(settings)),
            playing: Mutex::new(Playing::default()),
            history: Mutex::new(VecDeque::new()),
            seen: Mutex::new(Seen::default()),
            overlay_closed: AtomicBool::new(false),
            sink: AtomicIsize::new(0),
            shown: AtomicBool::new(false),
        }
    }

    pub fn apply(&self, settings: &Settings) {
        *self.config.lock().unwrap() = config_from(settings);
        self.post(RESTYLE);
    }

    fn post(&self, message: u32) {
        let sink = self.sink.load(Ordering::Acquire);
        if sink != 0 {
            let _ = unsafe { PostMessageW(Some(HWND(sink as *mut _)), message, WPARAM(0), LPARAM(0)) };
        }
    }

    pub fn record(&self, sample: Sample) {
        self.seen.lock().unwrap().note(&sample);
        // A game that has gone a short while without frames has not ended.
        let now = Instant::now();
        let (started, ended) = {
            let mut playing = self.playing.lock().unwrap();
            if sample.game.is_some() {
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
        if started || ended {
            // The bar's overlay button is lit while the overlay is up.
            self.post(RESTYLE);
        }
        self.post(OVERLAY);
        let keep = self.config.lock().unwrap().history;
        let mut history = self.history.lock().unwrap();
        while history.len() >= keep {
            history.pop_front();
        }
        history.push_back(sample);
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

    /// Opens the panel at the pointer as the tray does, or closes it if it
    /// is open (pinned or not): the keyboard shortcut.
    pub fn toggle(&self) {
        self.post(TOGGLE);
    }

    /// Takes the panel off the screen at once, without its animation.
    pub fn dismiss(&self) {
        self.post(DISMISS);
    }

    /// Opens the settings window, which this thread carries too.
    pub fn open_settings(&self) {
        self.post(OPEN_SETTINGS);
    }

    /// The modules this machine has, in their default order.
    pub fn known_modules(&self) -> Vec<String> {
        // The game in front first, while there is one.
        let mut known = vec!["game".to_string(), "cpu".to_string()];
        known.extend(self.info.gpu_modules());
        known.extend(["memory", "network", "disk", "processes", "storage", "board", "battery", "system"].map(String::from));
        known
    }

    /// Runs the input loop, which also carries the panel. Raw input is
    /// read-only: unlike a mouse hook it cannot delay the pointer, and Windows
    /// never silently disconnects it.
    pub fn run(&self) {
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
        let mouse = RAWINPUTDEVICE { usUsagePage: 1, usUsage: 2, dwFlags: RIDEV_INPUTSINK, hwndTarget: sink };
        unsafe { RegisterRawInputDevices(&[mouse], size_of::<RAWINPUTDEVICE>() as u32) }.expect("raw mouse input");
        // In the process's multithreaded apartment (see lib.rs).
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let mut panel = Panel::new(self).expect("panel window");
        // Made when first wanted; with no graphics device just then, tried
        // again at the next sample.
        let mut overlay: Option<Overlay> = None;
        let draw_overlay = |overlay: &mut Option<Overlay>, panel: &Panel| {
            let playing = self.playing();
            let wanted = self.overlay_wanted();
            if overlay.is_none() && wanted {
                *overlay = Overlay::new().ok();
            }
            let Some(overlay) = overlay else { return };
            let settings = self.config.lock().unwrap().overlay.clone();
            let history = self.history.lock().unwrap();
            let beneath = panel.is_shown().then_some(panel.window.hwnd);
            overlay.show(history.back(), &settings, panel.lang, playing, wanted, beneath);
        };
        self.sink.store(sink.0 as isize, Ordering::Release);

        let mut detector = Detector::default();
        // Once the panel has been up, the pointer has to leave the edge before
        // it can open the panel again: pushing on after it closed is the same
        // gesture, not a new one.
        let mut armed = true;
        let mut ticking = false;
        // Escape closes the open panel: taken from every program while the
        // panel is open and not pinned (it has no keyboard focus of its own),
        // and given back as it closes or is pinned.
        let mut escape = false;
        // Raw input stops reaching this process while a window of higher
        // privilege (Task Manager, an installer) has the focus. A slow watch
        // on the cursor notices that: it moved, and no raw input came. The
        // pointer is then treated as one that cannot push, and opens the
        // panel by resting on the edge.
        let mut last_cursor = cursor_position().unwrap_or_default();
        let mut raw_since_watch = false;
        unsafe { SetTimer(Some(sink), WATCH_TIMER, WATCH_MS, None) };
        // What a frame in motion waits on: the screen's next refresh, or a
        // message, whichever comes first.
        // High resolution from Windows 10 1803 on; before, an ordinary one.
        let refresh = unsafe {
            CreateWaitableTimerExW(None, None, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0)
                .or_else(|_| CreateWaitableTimerExW(None, None, 0, TIMER_ALL_ACCESS.0))
        }
        .expect("frame timer");
        let mut msg = MSG::default();
        loop {
            // The panel, while it is up, and the settings window, while it is
            // open, are drawn whenever they are due, between the messages
            // that arrived since; with neither, the loop sleeps until a
            // message comes.
            if !unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
                let now = Instant::now();
                let mut wait = None;
                if panel.is_shown() {
                    panel.tick(now);
                    if panel.is_shown() {
                        // The pointer is followed at least this often.
                        wait = Some(panel.next_frame.saturating_duration_since(now).min(Duration::from_millis(TICK_MS as u64)));
                    }
                }
                if let Some(due) = settings_window::tick(now) {
                    wait = Some(wait.map_or(due, |wait: Duration| wait.min(due)));
                }
                match wait {
                    // In motion: the next frame comes with the screen's, and
                    // the wait for it never outlasts one (DwmFlush, which
                    // waits for the desktop to be composed again, waits as
                    // long as nothing on screen changes), nor holds back a
                    // message.
                    Some(wait) if wait.is_zero() => unsafe {
                        let due = -((until_refresh().as_nanos() / 100) as i64).max(1);
                        let _ = SetWaitableTimer(refresh, &due, 0, None, None, false);
                        MsgWaitForMultipleObjects(Some(&[refresh]), false, INFINITE, QS_ALLINPUT);
                    },
                    Some(wait) => unsafe {
                        MsgWaitForMultipleObjects(None, false, wait.as_millis() as u32, QS_ALLINPUT);
                    },
                    None => unsafe {
                        let _ = WaitMessage();
                    },
                }
                continue;
            }
            if msg.message == WM_QUIT {
                return;
            }
            let now = Instant::now();
            let (edge, pressure, over_fullscreen) = {
                let config = self.config.lock().unwrap();
                (config.edge, config.pressure, config.over_fullscreen)
            };
            let (fullscreen_hotkey, fullscreen_edge) = match over_fullscreen {
                OverFullscreen::Never => (false, false),
                OverFullscreen::Shortcut => (true, false),
                OverFullscreen::Both => (true, true),
            };
            // Over a game in exclusive fullscreen the panel sends the game to
            // the background: it opens there only as the settings allow.
            let may_open = |allowed: bool| allowed || !exclusive_fullscreen();
            // Where on the edge the pointer is, and how hard it has to push
            // there; never, with pushing turned off.
            let at_edge = |cursor: POINT| edge_contact(cursor, edge).zip(pressure);
            if panel.is_shown() {
                armed = false;
            }
            match msg.message {
                OPEN_FROM_TRAY => {
                    if let Some((cursor, contact)) = cursor_position().and_then(|cursor| Some((cursor, monitor_at(cursor)?))) {
                        panel.open(cursor, contact, now);
                    }
                }
                TOGGLE if panel.is_open() => panel.begin_close(now),
                TOGGLE if !may_open(fullscreen_hotkey) => {}
                TOGGLE => {
                    if let Some((cursor, contact)) = cursor_position().and_then(|cursor| Some((cursor, monitor_at(cursor)?))) {
                        panel.open(cursor, contact, now);
                    }
                }
                DISMISS => panel.dismiss(),
                WM_HOTKEY if msg.hwnd.is_invalid() && msg.wParam.0 == ESCAPE_HOTKEY as usize => {
                    if panel.is_open() && !panel.pinned {
                        panel.begin_close(now);
                    }
                }
                // Placed, zoomed and backed for screens that are no longer
                // so: taken down, and opened afresh.
                // The panel's own (the overlay's follows its game's screen).
                crate::ui::window::SCREENS_CHANGED if msg.hwnd == panel.window.hwnd => panel.screens_changed(msg.wParam.0 as u32, now),
                OPEN_SETTINGS => settings_window::open(),
                RESTYLE => {
                    panel.restyle();
                    draw_overlay(&mut overlay, &panel);
                    // An open settings window takes what was changed elsewhere.
                    settings_window::follow_settings();
                }
                OVERLAY => draw_overlay(&mut overlay, &panel),
                WM_LBUTTONUP if msg.hwnd == panel.window.hwnd => panel.click(lparam_point(msg.lParam)),
                // The overlay dragged into place.
                WM_LBUTTONDOWN if overlay.as_ref().is_some_and(|o| msg.hwnd == o.window.hwnd) => overlay.as_mut().unwrap().press(),
                WM_MOUSEMOVE if overlay.as_ref().is_some_and(|o| msg.hwnd == o.window.hwnd) => overlay.as_mut().unwrap().moved(),
                // Let go, or the capture taken away mid-drag (the lock
                // screen, another program): it stays where it was put.
                WM_LBUTTONUP | crate::ui::window::CAPTURE_LOST if overlay.as_ref().is_some_and(|o| msg.hwnd == o.window.hwnd) => {
                    if let Some(placed) = overlay.as_mut().unwrap().release() {
                        std::thread::spawn(move || crate::app().place_overlay(placed));
                    }
                }
                WM_RBUTTONUP if overlay.as_ref().is_some_and(|o| msg.hwnd == o.window.hwnd) => {
                    // What comes for this thread while the menu is up waits.
                    hold_messages(true);
                    let (choice, before) = overlay.as_ref().unwrap().menu();
                    hold_messages(false);
                    match choice {
                        Some(OverlayChoice::Settings) => {
                            panel.dismiss();
                            settings_window::open_at_overlay();
                        }
                        choice => {
                            // The game in front before the menu, in front again.
                            bring_back(Some(before).filter(|window| !window.is_invalid()));
                            match choice {
                                Some(OverlayChoice::Lock(locked)) => {
                                    std::thread::spawn(move || crate::app().lock_overlay(locked));
                                }
                                Some(OverlayChoice::Close) => self.set_overlay(false),
                                _ => {}
                            }
                        }
                    }
                }
                WM_MOUSEMOVE if msg.hwnd == panel.window.hwnd => panel.hover_at(lparam_point(msg.lParam)),
                WM_MOUSEWHEEL if msg.hwnd == panel.window.hwnd => {
                    let delta = (msg.wParam.0 >> 16) as u16 as i16;
                    panel.wheel(lparam_point(msg.lParam), delta);
                }
                WM_INPUT if !panel.is_open() => {
                    raw_since_watch = true;
                    let motion = read_motion(HRAWINPUT(msg.lParam.0 as *mut _), edge);
                    if let (Some(motion), Some(cursor)) = (motion, cursor_position()) {
                        match at_edge(cursor) {
                            Some((contact, pressure)) if armed && detector.motion(motion, now, pressure) => {
                                detector.reset();
                                // Not over this game: not again until the
                                // pointer has left the edge.
                                if may_open(fullscreen_edge) {
                                    panel.open(cursor, contact, now);
                                } else {
                                    armed = false;
                                }
                            }
                            Some(_) => {}
                            None => {
                                detector.reset();
                                armed = !panel.is_shown();
                            }
                        }
                    }
                }
                // While another desktop has the input (a UAC prompt, the lock
                // screen) the pointer cannot be read, and is left alone.
                WM_TIMER if msg.wParam.0 == WATCH_TIMER => {
                    let Some(cursor) = cursor_position() else { continue };
                    let moved = cursor.x != last_cursor.x || cursor.y != last_cursor.y;
                    if moved && !raw_since_watch && !panel.is_open() {
                        match at_edge(cursor) {
                            Some((_, pressure)) if armed => {
                                detector.motion(Motion::Absolute, now, pressure);
                            }
                            Some(_) => {}
                            None => {
                                detector.reset();
                                armed = !panel.is_shown();
                            }
                        }
                    }
                    last_cursor = cursor;
                    raw_since_watch = false;
                }
                WM_TIMER if msg.wParam.0 == TRACK_TIMER && !panel.is_open() => {
                    let Some(cursor) = cursor_position() else { continue };
                    match at_edge(cursor) {
                        Some((contact, _)) if armed && detector.dwell_elapsed(now) => {
                            detector.reset();
                            if may_open(fullscreen_edge) {
                                panel.open(cursor, contact, now);
                            } else {
                                armed = false;
                            }
                        }
                        Some(_) => {}
                        None => {
                            detector.reset();
                            armed = !panel.is_shown();
                        }
                    }
                }
                _ => {}
            }

            let wanted = panel.is_open() && !panel.pinned;
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
            let needed = !panel.is_open() && detector.dwelling();
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

#[derive(Clone, Copy, PartialEq)]
struct Contact {
    monitor: RECT,
    work: RECT,
    scale: f32,
}

/// Where the panel is, for the opening under way.
struct Placement {
    contact: Contact,
    /// Where along the edge the panel centres: the pointer where it was
    /// opened, or the middle of the work area.
    anchor: POINT,
    window: RECT,
    /// The panel at rest, and the pointer's leeway around it (physical px).
    panel: RECT,
    reach: RECT,
    /// Physical px per DIP: the monitor's scale, times the zoom of a panel
    /// too large for its screen.
    px: f32,
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
    taken: Instant,
}

impl Behind {
    fn new(capture: Capture, placement: &Placement, now: Instant) -> Self {
        let tone = capture.luminance(placement.panel, placement.px);
        Behind { digest: capture.digest, tone, capture, bitmap: None, taken: now }
    }
}

/// The panel and its window, owned by the input thread.
struct Panel<'a> {
    controller: &'a Controller,
    gfx: Rc<Gfx>,
    window: Window,
    /// None after the device was lost, until it is made again on the new one.
    surface: Option<Surface>,
    layers: PanelLayers,
    phase: Phase,
    /// 0 at rest on screen, 1 slid out.
    shift: Transition,
    opacity: Transition,
    placement: Option<Placement>,
    prefs: Prefs,
    skin: Skin,
    theme: Theme,
    lang: Lang,
    edge: Edge,
    /// The columns chosen in the settings; none, as few as fit.
    columns: Option<usize>,
    /// The size chosen in the settings, 1 as designed.
    size: f32,
    /// Refresh the desktop behind the glass while the panel is open.
    live: bool,
    behind: Option<Behind>,
    /// When live refraction last tried to take the desktop.
    tried_behind: Instant,
    frost: f32,
    scroll: f32,
    scroll_target: f32,
    hover: Option<Hit>,
    /// Pinned open: the pointer leaving, or a press elsewhere, does not
    /// close it; unpinning, the shortcut or the tray's settings do.
    pinned: bool,
    /// A game holding the screen in exclusive fullscreen when the panel
    /// opened over it, which sent it to the background: brought back when
    /// the panel has closed, unless a press elsewhere closed it.
    game: Option<HWND>,
    /// The panel as it opened, held while it is up (laid out afresh at the
    /// next frame when `None`), and the settings it was made for.
    opening: Option<arrange::Opening>,
    /// What a panel laid out afresh while up (the settings changed) still
    /// holds from before: a volume unplugged stays until it closes.
    held: Option<Seen>,
    style: String,
    /// Where clicks and wheel turns land, from the panel's corner, and
    /// where that corner is in the window (DIPs).
    hits: Vec<HitBox>,
    corner: (f32, f32),
    last_frame: Instant,
    /// When the panel is next drawn: at once while anything on it is in
    /// motion beyond the charts' creep.
    next_frame: Instant,
}

impl<'a> Panel<'a> {
    fn new(controller: &'a Controller) -> windows::core::Result<Self> {
        let gfx = gfx::current()?;
        let window = Window::new()?;
        let surface = Some(Surface::new(&gfx, window.hwnd)?);
        let mut panel = Panel {
            controller,
            gfx,
            window,
            surface,
            layers: PanelLayers::default(),
            phase: Phase::Hidden,
            shift: Transition::settled(1.0),
            opacity: Transition::settled(0.0),
            placement: None,
            prefs: Prefs::default(),
            skin: Skin::Paper,
            theme: Theme::new(Skin::Paper, false),
            lang: Lang::En,
            edge: Edge::Right,
            columns: None,
            size: 1.0,
            live: false,
            behind: None,
            tried_behind: Instant::now(),
            frost: 0.0,
            scroll: 0.0,
            scroll_target: 0.0,
            hover: None,
            pinned: false,
            game: None,
            opening: None,
            held: None,
            style: String::new(),
            hits: Vec::new(),
            corner: (0.0, 0.0),
            last_frame: Instant::now(),
            next_frame: Instant::now(),
        };
        panel.restyle();
        Ok(panel)
    }

    fn is_shown(&self) -> bool {
        !matches!(self.phase, Phase::Hidden)
    }

    fn is_open(&self) -> bool {
        matches!(self.phase, Phase::Open { .. })
    }

    /// The settings changed: what is shown, how, and from which edge.
    fn restyle(&mut self) {
        self.next_frame = Instant::now();
        let config = self.controller.config.lock().unwrap();
        self.prefs = Prefs::resolve(&config.view, &self.controller.known_modules());
        self.edge = config.edge;
        self.columns = config.columns;
        self.size = config.size;
        self.skin = Skin::named(&config.skin);
        self.live = config.live && self.skin.sees_backdrop();
        drop(config);
        // What is shown, and how, changed: it is laid out anew. The
        // process list's order (sorted from the panel itself) changes no
        // lane's size, and leaves it be.
        let mut sized = self.prefs.clone();
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
        // While the backdrop is live the window has to stay out of the
        // captures of what is behind it; the system offers that only as
        // "out of every capture", screenshots too. Only while it is up: a
        // window kept out of captures is "protected content" to recorders
        // (NVIDIA's Instant Replay stops for it), hidden or not.
        self.window.exclude_from_capture(self.live && self.is_shown());
        self.dress();
    }

    /// Picks the theme, and how frosted the glass is, for the skin and the
    /// desktop behind the panel.
    fn dress(&mut self) {
        let behind = self.behind.as_ref().map(|behind| behind.tone);
        let dark = theme::is_dark(self.prefs.theme, behind.map(|(mean, _)| mean));
        self.theme = Theme::new(self.skin, dark);
        self.frost = behind.map_or(0.0, |(mean, spread)| skins::frost(mean, spread, dark));
    }

    fn open(&mut self, cursor: POINT, contact: Contact, now: Instant) {
        if self.is_open() || self.controller.history.lock().unwrap().is_empty() {
            return;
        }
        // A reopening during the way out picks the panel up where it is.
        if !self.is_shown() {
            self.restyle();
            // A new opening is laid out for what the machine shows now.
            self.opening = None;
            self.held = None;
            let work = contact.work;
            let anchor = match self.controller.config.lock().unwrap().anchor {
                Anchor::Pointer => cursor,
                Anchor::Center => POINT { x: (work.left + work.right) / 2, y: (work.top + work.bottom) / 2 },
            };
            self.placement = Some(Placement {
                contact,
                anchor,
                window: RECT::default(),
                panel: RECT::default(),
                reach: RECT::default(),
                px: contact.scale,
            });
            self.scroll = 0.0;
            self.scroll_target = 0.0;
            self.hover = None;
            self.game = exclusive_fullscreen().then(|| unsafe { GetForegroundWindow() }).filter(|game| !game.is_invalid());
            self.shift.jump(1.0);
            self.opacity.jump(0.0);
            // The desktop where the panel will be, taken while the window is
            // still hidden.
            if self.skin.sees_backdrop() {
                self.arrange();
                let placement = self.placement.as_ref().unwrap();
                // Unreadable (the lock screen): the glass goes without. All
                // the screen the panel may grow into while it is up: the
                // strip along its edge (it grows along it, never across).
                let monitor = placement.contact.monitor;
                let area = match self.edge {
                    Edge::Left | Edge::Right => RECT { top: monitor.top, bottom: monitor.bottom, ..placement.window },
                    Edge::Top => RECT { left: monitor.left, right: monitor.right, bottom: monitor.bottom, ..placement.window },
                };
                self.behind = Capture::take(area).map(|capture| Behind::new(capture, placement, now));
                self.dress();
            }
        }
        self.phase = Phase::Open { entered: false, outside_since: None, dragging: false };
        self.controller.shown.store(true, Ordering::Relaxed);
        self.window.exclude_from_capture(self.live);
        if reduced_motion() {
            self.shift.jump(0.0);
            self.opacity.retarget(1.0, PLAIN_FADE, LINEAR, now);
        } else {
            self.shift.retarget(0.0, OPEN.0, OPEN.1, now);
            self.opacity.retarget(1.0, OPEN_FADE, LINEAR, now);
        }
        self.window.set_click_through(false);
        // The first frame is drawn before the window shows, so it never shows empty.
        self.last_frame = now;
        self.frame(now);
        self.window.show();
    }

    fn begin_close(&mut self, now: Instant) {
        self.phase = Phase::Closing;
        self.pinned = false;
        self.window.set_click_through(true);
        if reduced_motion() {
            self.opacity.retarget(0.0, PLAIN_FADE, LINEAR, now);
        } else {
            self.shift.retarget(1.0, CLOSE.0, CLOSE.1, now);
            self.opacity.retarget(0.0, CLOSE.0, LINEAR, now);
        }
    }

    /// The screens changed (`dpi` 0), or the window's scale did (to `dpi`):
    /// a panel placed, zoomed and backed for screens no longer so is taken
    /// down, to be opened afresh. Its own move to a monitor of another scale
    /// as it opens tells a scale it was already placed for, and is no change.
    fn screens_changed(&mut self, dpi: u32, now: Instant) {
        let Some(placement) = &self.placement else { return };
        let changed = if dpi == 0 {
            // A game leaving exclusive fullscreen sets the display mode back
            // just as the panel comes up over it, often to what it was: a
            // panel whose screen is as it was stays as it is.
            monitor_at(placement.anchor) != Some(placement.contact)
        } else {
            (dpi as f32 / 96.0 - placement.contact.scale).abs() > 0.001
        };
        if !changed {
            return;
        }
        // Open, it opens again, laid out for the screens as they are now,
        // pinned if it was, and still with the game to bring back; on its
        // way out, it is gone at once, and the game comes back as when it
        // has closed (the game's own display mode change can come as late as
        // that).
        let (open, closing, pinned, game) = (self.is_open(), matches!(self.phase, Phase::Closing), self.pinned, self.game);
        self.dismiss();
        if closing {
            bring_back(game);
        }
        if open {
            if let Some((cursor, contact)) = cursor_position().and_then(|cursor| Some((cursor, monitor_at(cursor)?))) {
                self.open(cursor, contact, now);
                self.pinned = pinned;
                self.game = game;
            }
        }
    }

    /// Takes the panel down at once (for the settings, or screens no longer
    /// as they were): a game it opened over stays where it went.
    fn dismiss(&mut self) {
        if !self.is_shown() {
            return;
        }
        self.game = None;
        self.phase = Phase::Hidden;
        self.pinned = false;
        self.window.hide();
        self.window.exclude_from_capture(false);
        // The drawing memory is given back while the panel is away.
        if let Some(surface) = &mut self.surface {
            surface.release(&self.gfx);
        }
        self.layers.release();
        self.behind = None;
        self.gfx.trim();
        self.placement = None;
        self.controller.shown.store(false, Ordering::Relaxed);
    }

    /// Follows the pointer while the panel is open: closes it once the
    /// pointer has been away long enough, or on a press elsewhere.
    fn track(&mut self, now: Instant) {
        let (Some(placement), Some(cursor)) = (&self.placement, cursor_position()) else { return };
        let on_panel = contains(&placement.panel, cursor);
        let in_reach = contains(&placement.reach, cursor);
        let close_delay = self.controller.config.lock().unwrap().close_delay;
        let held = buttons_down();
        if !on_panel && self.hover.take().is_some() {
            self.next_frame = now;
        }
        // Around the panel the window is only shadow; clicks there belong
        // to whatever is underneath.
        self.window.set_click_through(!on_panel);
        let Phase::Open { entered, outside_since, dragging } = &mut self.phase else { return };
        if self.pinned {
            *outside_since = None;
            return;
        }
        // A press that begins away from the panel dismisses it; one that
        // begins on it keeps it open wherever the pointer then goes.
        let pressed_outside = held && !*dragging && !in_reach;
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
        if self.is_open() {
            self.track(now);
        }
        if self.live && self.is_open() {
            self.refresh_behind(now);
        }
        if now >= self.next_frame {
            self.frame(now);
        }
    }

    /// With live refraction, captures the desktop behind the window again
    /// every so often; only a capture that differs from the last is used.
    fn refresh_behind(&mut self, now: Instant) {
        let Some(placement) = &self.placement else { return };
        // Tried again every so often, with or without a capture so far (the
        // first may have found the screen unreadable).
        let last = self.behind.as_ref().map_or(self.tried_behind, |behind| behind.taken);
        if now.duration_since(last) < LIVE_INTERVAL {
            return;
        }
        self.tried_behind = now;
        // The same desktop as before, or none readable just now: the last stands.
        let capture = Capture::take(placement.window);
        let Some(capture) = capture.filter(|capture| self.behind.as_ref().is_none_or(|behind| capture.digest != behind.digest)) else {
            if let Some(behind) = &mut self.behind {
                behind.taken = now;
            }
            return;
        };
        let behind = Behind::new(capture, placement, now);
        // The theme holds for the opening; only the frost follows.
        self.frost = skins::frost(behind.tone.0, behind.tone.1, self.theme.dark);
        self.behind = Some(behind);
        self.next_frame = now;
    }

    /// The panel's lanes, laid out (once each opening, and held), and the
    /// window placed for them.
    fn arrange(&mut self) -> (Vec<view::Lane>, Layout) {
        let controller = self.controller;
        let now = controller.seen.lock().unwrap().clone();
        let mut history = controller.history.lock().unwrap();
        let samples = history.make_contiguous();
        let contact = self.placement.as_ref().unwrap().contact;
        let work = (
            (contact.work.right - contact.work.left) as f32 / contact.scale,
            (contact.work.bottom - contact.work.top) as f32 / contact.scale,
        );
        let (prefs, theme, lang, scroll, hover, pinned) = (&self.prefs, &self.theme, self.lang, self.scroll, self.hover, self.pinned);
        let opening = match &mut self.opening {
            Some(opening) => {
                // What the machine has shown since joins its lane where it
                // fits (each lane measured holding what it would hold).
                let probe = scene(controller, prefs, theme, lang, scroll, hover, pinned, samples, &now);
                opening.grow(&now, &controller.info, |id, seen| view::lane_height(&probe, id, seen));
                opening
            }
            None => {
                let now = match self.held.take() {
                    Some(held) => held.join(&now, &controller.info),
                    None => now,
                };
                let scene = scene(controller, prefs, theme, lang, scroll, hover, pinned, samples, &now);
                let lanes = view::lanes(&scene);
                let heights: Vec<(&str, f32)> = lanes.iter().map(|lane| (lane.id.as_str(), lane.height(theme))).collect();
                self.opening.insert(arrange::Opening::new(theme, self.edge, &heights, work, self.columns, self.size, now.clone()))
            }
        };
        let lanes = view::lanes(&scene(controller, prefs, theme, lang, scroll, hover, pinned, samples, &opening.seen));
        let (layout, zoom) = (opening.layout.clone(), opening.zoom);
        drop(history);
        let placement = self.placement.as_mut().unwrap();
        place(placement, &self.window, self.edge, &self.theme, (layout.width(), layout.height()), zoom);
        (lanes, layout)
    }

    /// Takes up this thread's current graphics device if it is a new one (the
    /// last was lost), making the window's surface and layers again on it.
    /// False while there is no device to be had.
    fn follow_device(&mut self) -> bool {
        let Ok(current) = gfx::current() else { return false };
        if Rc::ptr_eq(&current, &self.gfx) && self.surface.is_some() {
            return true;
        }
        // A window has one composition target: the old one goes first.
        self.surface = None;
        let Ok(surface) = Surface::new(&current, self.window.hwnd) else {
            gfx::lost();
            return false;
        };
        self.surface = Some(surface);
        self.gfx = current;
        self.layers.release();
        // A desktop already made a bitmap on the lost device is gone with it.
        if let Some(behind) = &mut self.behind {
            behind.bitmap = None;
        }
        true
    }

    /// Draws the panel as it is at `now`, and finishes a closing that is done.
    fn frame(&mut self, now: Instant) {
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
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.scroll += (self.scroll_target - self.scroll) * (1.0 - E.powf(-dt / SCROLL_EASE));

        let (lanes, layout) = self.arrange();
        let placement = self.placement.as_ref().unwrap();
        let px = placement.px;
        let window = placement.window;
        let size = ((window.right - window.left) as u32, (window.bottom - window.top) as u32);
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
        let mut history = controller.history.lock().unwrap();
        let seen = &self.opening.as_ref().unwrap().seen;
        let scene = scene(controller, &self.prefs, &self.theme, self.lang, self.scroll, self.hover, self.pinned, history.make_contiguous(), seen);
        let (layers, behind, edge, frost) = (&mut self.layers, &mut self.behind, self.edge, self.frost);
        let mut drawn = None;
        let surface = self.surface.as_mut().unwrap();
        let painted = surface.draw(&self.gfx, size, px, |frame| {
            frame.origin(0.0, 0.0);
            if opacity < 1.0 {
                let everything = D2D_RECT_F { left: -f32::MAX, top: -f32::MAX, right: f32::MAX, bottom: f32::MAX };
                let layer = D2D1_LAYER_PARAMETERS1 { contentBounds: everything, opacity, ..Default::default() };
                unsafe { frame.dc.PushLayer(&layer, None) };
            }
            let backdrop = behind.as_mut().and_then(|behind| {
                if behind.bitmap.as_ref().is_none_or(|(_, made_for)| *made_for != px) {
                    behind.bitmap = behind.capture.bitmap(&frame.dc, px).ok().map(|bitmap| (bitmap, px));
                }
                // Aligned with the screen where the panel rests: the window
                // may have moved or grown since the capture was taken.
                let from = behind.capture.rect;
                let at = Vector2 {
                    X: (window.left - from.left) as f32 / px + rest.0,
                    Y: (window.top - from.top) as f32 / px + rest.1,
                };
                Some((&behind.bitmap.as_ref()?.0, at, behind.digest))
            });
            let picture = render::Picture { scene: &scene, lanes: &lanes, layout: &layout, edge, backdrop, frost };
            drawn = Some(layers.draw(frame, &picture, Matrix3x2::translation(x, y), px));
            if opacity < 1.0 {
                unsafe { frame.dc.PopLayer() };
            }
        });
        drop(history);
        let drawn = match (painted, drawn) {
            (Ok(()), Some(Ok(drawn))) => drawn,
            // The device is gone; a new one is made for the next frame.
            _ => {
                gfx::lost();
                self.next_frame = now + DEVICE_RETRY;
                return;
            }
        };
        let (hits, grounded) = (drawn.hits, drawn.grounded);
        self.gfx.sweep();
        // The effects that drew the ground are not needed again until it
        // changes; what they hold is let go.
        if grounded {
            self.gfx.clear_caches();
        }
        if let Some(hits) = hits {
            self.hits = hits;
        }
        self.corner = (x, y);

        // At rest only the charts move, a plot's width over its span: the
        // panel is drawn again when they have crept a quarter of a pixel.
        let moving = !self.shift.done(now) || !self.opacity.done(now) || (self.scroll_target - self.scroll).abs() > 0.05;
        self.next_frame = if moving {
            now
        } else {
            let speed = layout.plot_width(&self.theme) * px / self.prefs.chart_seconds as f32;
            now + Duration::from_secs_f32(0.25 / speed)
        };
    }

    /// Which hit region a point in the window's client area (physical px) is in.
    fn hit_at(&self, client: POINT) -> Option<Hit> {
        let px = self.placement.as_ref()?.px;
        let (x, y) = (client.x as f32 / px - self.corner.0, client.y as f32 / px - self.corner.1);
        self.hits.iter().find(|(hx, hy, w, h, _)| x >= *hx && x < hx + w && y >= *hy && y < hy + h).map(|hit| hit.4)
    }

    /// The pointer moved over the panel: lights what it is over.
    fn hover_at(&mut self, client: POINT) {
        let hover = self.hit_at(client).filter(|hit| hit.max_scroll().is_none());
        if hover != self.hover {
            self.hover = hover;
            self.next_frame = Instant::now();
        }
    }

    fn click(&mut self, client: POINT) {
        self.next_frame = Instant::now();
        match self.hit_at(client) {
            Some(Hit::Settings) => {
                crate::show_settings();
            }
            Some(Hit::Pin) => self.pinned ^= true,
            Some(Hit::Overlay) => {
                // As it is on screen now: shown, it closes; not, it opens.
                self.controller.set_overlay(!self.controller.overlay_wanted());
            }
            Some(Hit::Sort(sort)) if sort != self.prefs.processes.sort => {
                self.prefs.processes.sort = sort;
                self.scroll = 0.0;
                self.scroll_target = 0.0;
                // Kept for next time, as if chosen in the settings.
                // Saving tells this thread to restyle; not from here.
                std::thread::spawn(move || crate::app().set_process_sort(sort));
            }
            _ => {}
        }
    }

    /// A wheel turn with the pointer at `screen`: scrolls the process list under it.
    fn wheel(&mut self, screen: POINT, delta: i16) {
        let Some(window) = self.placement.as_ref().map(|p| p.window) else { return };
        let client = POINT { x: screen.x - window.left, y: screen.y - window.top };
        if let Some(max_scroll) = self.hit_at(client).and_then(Hit::max_scroll) {
            let rows = -(delta as f32) / 120.0 * WHEEL_ROWS;
            self.scroll_target = (self.scroll_target + rows * TABLE_ROW).clamp(0.0, max_scroll);
            self.next_frame = Instant::now();
        }
    }
}

/// What the readings are drawn with at this moment.
fn scene<'s>(
    controller: &'s Controller,
    prefs: &'s Prefs,
    theme: &'s Theme,
    lang: Lang,
    scroll: f32,
    hover: Option<Hit>,
    pinned: bool,
    history: &'s [Sample],
    seen: &'s Seen,
) -> Scene<'s> {
    let interval = controller.config.lock().unwrap().interval;
    let overlay = controller.overlay_wanted();
    let wall = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64() * 1000.0;
    Scene {
        info: &controller.info,
        prefs,
        theme,
        lang,
        history,
        seen,
        pen_ms: wall - interval.as_secs_f64() * 1000.0 - PEN_LAG_MS,
        process_scroll: scroll,
        hover,
        pinned,
        overlay,
    }
}

thread_local! {
    /// While a menu runs its own loop on this thread: what was posted to
    /// the sink meanwhile, which that loop hands to the sink's procedure.
    static HELD: std::cell::RefCell<Option<Vec<(u32, WPARAM, LPARAM)>>> = const { std::cell::RefCell::new(None) };
}

/// Starts holding the sink's messages (`true`), or posts back what was held.
fn hold_messages(hold: bool) {
    let held = HELD.with(|held| std::mem::replace(&mut *held.borrow_mut(), hold.then(Vec::new)));
    let sink = crate::app().controller.sink.load(Ordering::Acquire);
    for (message, wparam, lparam) in held.unwrap_or_default() {
        let _ = unsafe { PostMessageW(Some(HWND(sink as *mut _)), message, wparam, lparam) };
    }
}

/// The sink's procedure. The input loop takes its messages before they are
/// dispatched; only a loop of another's (a menu's) dispatches them here, and
/// Glance's own are then held for the input loop.
unsafe extern "system" fn sink_procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if (WM_APP..0xC000).contains(&message) {
        let held = HELD.with(|held| held.borrow_mut().as_mut().map(|held| held.push((message, wparam, lparam))).is_some());
        if held {
            return LRESULT(0);
        }
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}


/// Positions the panel for a layout `size` DIPs large, and moves the window
/// if that changed it.
fn place(placement: &mut Placement, panel_window: &Window, edge: Edge, theme: &Theme, size: (f32, f32), zoom: f32) {
    let Contact { monitor, work, scale } = placement.contact;
    let px = scale * zoom;
    let (width, height) = ((size.0 * px).round() as i32, (size.1 * px).round() as i32);
    let gap = (GAP * scale).round() as i32;
    let inset = (theme.inset * px).round() as i32;
    let centred = |at: i32, length: i32, low: i32, high: i32| (at - length / 2).min(high - gap - length).max(low + gap);
    let (left, top) = match edge {
        Edge::Left => (monitor.left + inset, centred(placement.anchor.y, height, work.top, work.bottom)),
        Edge::Right => (monitor.right - inset - width, centred(placement.anchor.y, height, work.top, work.bottom)),
        Edge::Top => (centred(placement.anchor.x, width, work.left, work.right), monitor.top + inset),
    };
    let panel = RECT { left, top, right: left + width, bottom: top + height };
    let margin = (theme.margin * px).ceil() as i32;
    // The shadow's room, kept on this monitor.
    let window = RECT {
        left: (left - margin).max(monitor.left),
        top: (top - margin).max(monitor.top),
        right: (panel.right + margin).min(monitor.right),
        bottom: (panel.bottom + margin).min(monitor.bottom),
    };
    let tolerance = (LEAVE_TOLERANCE * scale).round() as i32;
    let mut reach = RECT {
        left: panel.left - tolerance,
        top: panel.top - tolerance,
        right: panel.right + tolerance,
        bottom: panel.bottom + tolerance,
    };
    // The pointer arrives from the screen edge.
    match edge {
        Edge::Left => reach.left = monitor.left,
        Edge::Right => reach.right = monitor.right,
        Edge::Top => reach.top = monitor.top,
    }
    placement.panel = panel;
    placement.reach = reach;
    placement.px = px;
    if placement.window != window {
        placement.window = window;
        panel_window.place(window);
    }
}

/// Whether Windows has animations turned off ("Animation effects" in its
/// accessibility settings).
fn reduced_motion() -> bool {
    let mut animate = windows::core::BOOL(1);
    let read = unsafe {
        SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut animate as *mut _ as *mut _), Default::default())
    };
    read.is_ok() && !animate.as_bool()
}

/// The work area of the monitor under the pointer, and its scale factor.
pub fn work_area_at_cursor() -> Option<(RECT, f64)> {
    let contact = monitor_at(cursor_position()?)?;
    Some((contact.work, contact.scale as f64))
}

fn contains(rect: &RECT, point: POINT) -> bool {
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

/// Whether a program holds the screen in exclusive fullscreen (Direct3D's
/// own, not a borderless window): a window shown over it sends it to the
/// background.
fn exclusive_fullscreen() -> bool {
    unsafe { SHQueryUserNotificationState() }.is_ok_and(|state| state == QUNS_RUNNING_D3D_FULL_SCREEN)
}

fn cursor_position() -> Option<POINT> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }.ok().map(|_| point)
}

fn buttons_down() -> bool {
    [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON].iter().any(|key| unsafe { GetAsyncKeyState(key.0 as i32) } < 0)
}

fn cursor_showing() -> bool {
    let mut info = CURSORINFO { cbSize: size_of::<CURSORINFO>() as u32, ..Default::default() };
    unsafe { GetCursorInfo(&mut info) }.is_ok() && info.flags.0 & CURSOR_SHOWING.0 != 0
}

fn read_motion(handle: HRAWINPUT, edge: Edge) -> Option<Motion> {
    let mut raw = RAWINPUT::default();
    let mut size = size_of::<RAWINPUT>() as u32;
    let read = unsafe {
        GetRawInputData(handle, RID_INPUT, Some(&mut raw as *mut _ as *mut _), &mut size, size_of::<RAWINPUTHEADER>() as u32)
    };
    if read == u32::MAX || raw.header.dwType != RIM_TYPEMOUSE.0 {
        return None;
    }
    let mouse = unsafe { raw.data.mouse };
    if mouse.usFlags.0 & MOUSE_MOVE_ABSOLUTE != 0 {
        return Some(Motion::Absolute);
    }
    let outward = match edge {
        Edge::Left => -mouse.lLastX,
        Edge::Right => mouse.lLastX,
        Edge::Top => -mouse.lLastY,
    };
    let along = if edge == Edge::Top { mouse.lLastX } else { mouse.lLastY };
    Some(Motion::Relative { outward, along })
}

fn monitor_at(point: POINT) -> Option<Contact> {
    let handle = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe { GetMonitorInfoW(handle, &mut info) }.ok().ok()?;
    let (mut dpi, mut dpi_y) = (0, 0);
    unsafe { GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y) }.ok()?;
    Some(Contact { monitor: info.rcMonitor, work: info.rcWork, scale: dpi as f32 / 96.0 })
}

/// Describes the monitor under the cursor if the cursor is pressed against its
/// trigger edge with intent to point: visible, no button held, clear of the
/// corners, and with no other monitor continuing past that edge.
fn edge_contact(cursor: POINT, edge: Edge) -> Option<Contact> {
    let contact = monitor_at(cursor)?;
    let monitor = contact.monitor;
    let corner = (CORNER_EXCLUSION * contact.scale as f64).round() as i32;
    let along_height = (monitor.top + corner..monitor.bottom - corner).contains(&cursor.y);
    let (on_edge, beyond, clear_of_corners) = match edge {
        Edge::Left => (cursor.x <= monitor.left, POINT { x: monitor.left - 1, y: cursor.y }, along_height),
        Edge::Right => (cursor.x >= monitor.right - 1, POINT { x: monitor.right, y: cursor.y }, along_height),
        Edge::Top => (
            cursor.y <= monitor.top,
            POINT { x: cursor.x, y: monitor.top - 1 },
            (monitor.left + corner..monitor.right - corner).contains(&cursor.x),
        ),
    };
    if !on_edge || !clear_of_corners || buttons_down() || !cursor_showing() {
        return None;
    }
    let neighbour = unsafe { MonitorFromPoint(beyond, MONITOR_DEFAULTTONULL) };
    neighbour.is_invalid().then_some(contact)
}
