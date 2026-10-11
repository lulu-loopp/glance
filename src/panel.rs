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
use crate::settings::{Anchor, Edge, OverFullscreen, OverlaySettings, PanelAt, Rest, ScreenEdges, Settings, EDGES};
use crate::ui::backdrop::Capture;
use crate::ui::gfx::{self, Gfx, Surface};
use crate::ui::arrange::{self, Form, GAP};
use crate::ui::seen::Seen;
use crate::ui::render::{self, PanelLayers};
use crate::ui::motion::{Easing, Transition, LINEAR};
use crate::ui::prefs::Prefs;
use crate::ui::skins;
use crate::ui::text::Lang;
use crate::ui::theme::{self, Entrance, Skin, Theme};
use crate::ui::view::{self, Detail, Hit, HitBox, Layout, Scene, TABLE_ROW};
use crate::ui::settings_window;
use crate::ui::window::Window;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows_numerics::{Matrix3x2, Vector2};

/// The pointer may stray this far outside the panel and still count as on it (DIPs).
const LEAVE_TOLERANCE: f32 = 10.0;
/// Strip at each end of the edge that does not trigger (logical px): the
/// height of a title bar's buttons and of the taskbar, which own the corners.
const CORNER_EXCLUSION: f64 = 48.0;
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
/// While the pinned panel is dragged, the pointer is followed this often (ms).
const FOLLOW_MS: u32 = 8;

/// Requests to the input thread from elsewhere.
const OPEN_FROM_TRAY: u32 = WM_APP + 1;
const DISMISS: u32 = WM_APP + 2;
const RESTYLE: u32 = WM_APP + 3;
const OPEN_SETTINGS: u32 = WM_APP + 4;
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
/// The panel thread's own hotkey: Escape, while the panel is open.
const ESCAPE_HOTKEY: i32 = 1;

struct Config {
    /// How the edges of a screen `screens` does not name open the panel;
    /// and those it names, with theirs.
    rest: Rest,
    screens: Vec<ScreenEdges>,
    skin: String,
    live: bool,
    anchor: Anchor,
    columns: Option<usize>,
    size: f32,
    overlay: OverlaySettings,
    panel_at: Option<PanelAt>,
    panel_pinned: Option<(i32, i32)>,
    widgets: Vec<crate::settings::WidgetAt>,
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
        rest: settings.rest(),
        screens: settings.screens.clone(),
        skin: settings.skin.clone(),
        live: settings.live_backdrop,
        anchor: settings.anchor,
        columns: settings.columns,
        // As far as the settings offer: the file may say anything.
        size: settings.panel_size.clamp(SIZES.0, SIZES.1),
        overlay: settings.overlay.clone(),
        panel_at: settings.panel_at,
        panel_pinned: settings.panel_pinned,
        widgets: settings.widgets.clone(),
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
    /// A desktop widget was put away this session (the tray offers it back).
    widget_put_away: AtomicBool,
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
            widget_put_away: AtomicBool::new(false),
        }
    }

    pub fn apply(&self, settings: &Settings) {
        *self.config.lock().unwrap() = config_from(settings);
        self.post(RESTYLE);
    }

    /// Shows the tray icon's menu from `(x, y)`: on this thread, which draws
    /// Glance's menus.
    pub fn tray_menu(&self, x: i32, y: i32) {
        let sink = self.sink.load(Ordering::Acquire);
        if sink != 0 {
            let at = ((y as u16 as u32) << 16 | x as u16 as u32) as isize;
            let _ = unsafe { PostMessageW(Some(HWND(sink as *mut _)), TRAY_MENU, WPARAM(0), LPARAM(at)) };
        }
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
            let mut history = self.history.lock().unwrap();
            while history.len() >= keep {
                history.pop_front();
            }
            history.push_back(sample);
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

    /// The settings window came to the front or left it: a panel that
    /// stays up gives way to it meanwhile, so that it never covers it. (Which
    /// it is, is read as the message is taken: one taken late, after a menu,
    /// does not undo a later one.)
    pub fn settings_front(&self) {
        self.post(SETTINGS_FRONT);
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
        known.extend(["memory", "network", "disk", "processes", "storage", "board", "battery", "system", "wsl", "docker"].map(String::from));
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
        let mut widgets = crate::widget::Widgets::default();
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
            overlay.show(&history, &settings, panel.lang, playing, wanted, beneath);
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
        // A rest on the seam between two screens: since when, and where;
        // and, as on the edge, armed once the pointer has left it.
        let mut seam_rest: Option<(Instant, POINT)> = None;
        let mut seam_armed = true;
        // The screen the pointer was last on, and the one it last came
        // across from, and when.
        let mut on_screen: Option<RECT> = None;
        let mut crossed: Option<(RECT, Instant)> = None;
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
            widgets.adopt(crate::settings::WidgetAt { at: corner, layout, pinned: at.locked, click_through: false, game_only: false, on_desktop: false, stuck: None, before: None });
            std::thread::spawn(|| crate::app().change(|settings| settings.panel_at = None));
        }
        let mut restore = false;
        // Pinned open at its edge as Glance last ran: opened so again once
        // there are readings to show.
        let mut pinned_at = self.config.lock().unwrap().panel_pinned;
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
            if !unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
                let now = Instant::now();
                if frame_at.is_some_and(|at| now < at) {
                    // The refresh timer is set for it.
                    unsafe { MsgWaitForMultipleObjects(Some(&[refresh]), false, INFINITE, QS_ALLINPUT) };
                    continue;
                }
                frame_at = None;
                let mut wait = None;
                // A capture of the desktop behind the panel come back, up or
                // not (see `collect_behind`).
                panel.collect_behind(now);
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
                {
                    let kept = self.config.lock().unwrap().widgets.clone();
                    let seen = self.seen.lock().unwrap().clone();
                    let mut history = self.history.lock().unwrap();
                    let samples = history.make_contiguous();
                    if !samples.is_empty() {
                        let look = crate::widget::Look { prefs: &panel.prefs, skin: panel.skin, lang: panel.lang, live: panel.live };
                        if let Some(due) = widgets.tick(now, &look, &kept, samples, &seen, self) {
                            wait = Some(wait.map_or(due, |wait: Duration| wait.min(due)));
                        }
                    }
                    self.widget_put_away.store(widgets.any_put_away(), Ordering::Relaxed);
                }
                if let Some(point) = pinned_at.filter(|_| !self.history.lock().unwrap().is_empty()) {
                    pinned_at = None;
                    let point = POINT { x: point.0, y: point.1 };
                    // On its screen, or the nearest, if it is gone.
                    if let Some(contact) = monitor_at(point) {
                        panel.open(point, contact, panel.edge_for(point, contact), None, now);
                        panel.pinned = panel.is_open();
                    }
                }
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
            let (pressure, over_fullscreen) = {
                let config = self.config.lock().unwrap();
                (config.pressure, config.over_fullscreen)
            };
            // How the edges of a screen open the panel.
            let lit = |monitor: RECT| {
                let config = self.config.lock().unwrap();
                crate::screens::lit(&config.screens, config.rest, monitor)
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
            // Where the pointer is on an edge of its screen that opens the
            // panel, which edge, and how hard it has to push there; never,
            // with pushing turned off. (The ends of an edge open nothing: the
            // pointer is on one edge at a time, and off them all between two.)
            let at_edge = |cursor: POINT| {
                let pressure = pressure?;
                let here = monitor_at(cursor)?;
                lit(here.monitor).edges.each().find_map(|edge| edge_contact(cursor, edge).map(|contact| (contact, edge, pressure)))
            };
            if panel.is_shown() {
                armed = false;
                seam_armed = false;
            }
            // The pointer resting on the seam, where the panel opens.
            let mut on_seam = |cursor: POINT, panel: &mut Panel| {
                let here = monitor_at(cursor).map(|contact| contact.monitor);
                if here != on_screen {
                    crossed = on_screen.map(|from| (from, now));
                    on_screen = here;
                }
                let came_from = crossed.filter(|(_, at)| now.duration_since(*at) < SEAM_CROSSED).map(|(from, _)| from);
                // On a seam that is an edge that opens the panel, of the
                // screen the pointer is on before the one it came from.
                let found = EDGES
                    .into_iter()
                    .filter_map(|edge| seam_contact(cursor, edge, came_from).filter(|contact| lit(contact.monitor).seams.has(edge)).map(|contact| (contact, edge)))
                    .min_by_key(|(contact, _)| Some(contact.monitor) != here);
                let Some((contact, edge)) = found else {
                    seam_rest = None;
                    seam_armed = !panel.is_shown();
                    return;
                };
                if !seam_armed || panel.is_open() {
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
                        panel.open(cursor, contact, edge, None, now);
                    } else {
                        seam_armed = false;
                    }
                }
            };
            match msg.message {
                OPEN_FROM_TRAY => panel.open_by_hand(now),
                TOGGLE if panel.is_open() => panel.begin_close(now),
                TOGGLE if !may_open(fullscreen_hotkey) => {}
                TOGGLE => panel.open_by_hand(now),
                // A panel that stays (pinned, or moved away from the edge)
                // stays, the settings window coming up or not.
                DISMISS if panel.stays() => {}
                DISMISS => panel.dismiss(),
                WM_HOTKEY if msg.hwnd.is_invalid() && msg.wParam.0 == ESCAPE_HOTKEY as usize => {
                    if panel.is_open() && !panel.stays() {
                        panel.begin_close(now);
                    }
                }
                // Placed, zoomed and backed for screens that are no longer
                // so: taken down, and opened afresh.
                // The panel's own (the overlay's follows its game's screen).
                crate::ui::window::SCREENS_CHANGED if msg.hwnd == panel.window.hwnd => {
                    crate::screens::changed();
                    panel.screens_changed(msg.wParam.0 as u32, now);
                    widgets.screens_changed();
                }
                crate::ui::window::SCREENS_CHANGED if widgets.owns(msg.hwnd) => widgets.screens_changed(),
                crate::widget::CAPTURED if widgets.owns(msg.hwnd) => widgets.captured(msg.hwnd),
                OPEN_SETTINGS => settings_window::open(),
                TRAY_MENU => {
                    let (x, y) = ((msg.lParam.0 & 0xFFFF) as u16 as i16 as i32, ((msg.lParam.0 >> 16) & 0xFFFF) as u16 as i16 as i32);
                    // (What comes for this thread while the menu is up waits:
                    // see `menu::show`.)
                    crate::tray::show_menu(x, y);
                }
                RECALL_WIDGET => {
                    widgets.recall();
                }
                SETTINGS_FRONT => {
                    let front = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
                    let settings = settings_window::hwnd().filter(|settings| *settings == front);
                    panel.settings_front(settings);
                    widgets.settings_front(settings);
                }
                RESTYLE => {
                    panel.restyle();
                    draw_overlay(&mut overlay, &panel);
                    // An open settings window takes what was changed elsewhere.
                    settings_window::follow_settings();
                }
                OVERLAY => {
                    // A panel pinned away from the edge when Glance last
                    // closed comes back with the first sample.
                    if restore {
                        restore = false;
                        if !panel.is_shown() && may_open(fullscreen_hotkey) {
                            panel.open_by_hand(now);
                        }
                    }
                    draw_overlay(&mut overlay, &panel)
                }
                // The pinned panel dragged by its bar.
                WM_LBUTTONDOWN if msg.hwnd == panel.window.hwnd => {
                    panel.release_owed = false;
                    panel.press(lparam_point(msg.lParam))
                }
                WM_LBUTTONUP if msg.hwnd == panel.window.hwnd && std::mem::take(&mut panel.release_owed) => {}
                WM_TIMER if msg.hwnd == panel.window.hwnd && panel.held() => {
                    panel.dragged(now);
                    if let Some((form, scale, at, grab)) = panel.tear.take() {
                        // The widget takes the desktop behind it before the
                        // panel goes: without the panel in it (it is put back
                        // in captures as it goes).
                        panel.exclude();
                        // The panel's desktop and its look, lent to the widget
                        // till it has taken its own.
                        let lent = panel.behind.as_ref().and_then(|behind| {
                            Some(crate::widget::Lent { capture: behind.capture.without_pixels(), bitmap: behind.bitmap.as_ref()?.clone(), tone: behind.tone })
                        });
                        if !widgets.tear(form, scale, at, grab, lent) {
                            panel.torn = false;
                            panel.exclude();
                        }
                    }
                }
                // The desktop widgets: pressed, let go, their menu.
                WM_LBUTTONDOWN if widgets.owns(msg.hwnd) => widgets.press(msg.hwnd, lparam_point(msg.lParam)),
                WM_LBUTTONUP | crate::ui::window::CAPTURE_LOST if widgets.owns(msg.hwnd) => widgets.release(msg.hwnd, now),
                WM_MOUSEMOVE if widgets.owns(msg.hwnd) => widgets.hover(msg.hwnd, lparam_point(msg.lParam)),
                WM_RBUTTONUP if widgets.owns(msg.hwnd) => {
                    // (What comes for this thread while the menu is up waits:
                    // see `menu::show`.)
                    let choice = widgets.menu(msg.hwnd, panel.lang);
                    if let Some(crate::widget::WidgetChoice::Settings) = choice {
                        crate::show_settings();
                    }
                }
                WM_LBUTTONUP | WM_TIMER | crate::ui::window::CAPTURE_LOST if msg.hwnd == panel.window.hwnd && panel.drag.is_some() => {
                    // Kept here, in the order things happened (an unpin just
                    // after stands).
                    panel.release_owed = msg.message == WM_TIMER;
                    let kept = panel.let_go(now);
                    if std::mem::take(&mut panel.screens_missed) {
                        panel.screens_changed(0, now);
                    }
                    if let Some((at, size)) = kept.filter(|(at, size)| !matches!(at, Kept::Unchanged) || size.is_some()) {
                        crate::app().change(|settings| {
                            match at {
                                Kept::At(at) => settings.panel_at = Some(at),
                                Kept::Docked => settings.panel_at = None,
                                Kept::Unchanged => {}
                            }
                            if let Some(size) = size {
                                settings.panel_size = size;
                            }
                        });
                    }
                }
                WM_LBUTTONUP if msg.hwnd == panel.window.hwnd => panel.click(lparam_point(msg.lParam)),
                // The overlay dragged into place.
                WM_LBUTTONDOWN if overlay.as_ref().is_some_and(|o| msg.hwnd == o.window.hwnd) => overlay.as_mut().unwrap().press(),
                WM_MOUSEMOVE if overlay.as_ref().is_some_and(|o| msg.hwnd == o.window.hwnd) => overlay.as_mut().unwrap().moved(),
                WM_TIMER if msg.wParam.0 == crate::overlay::SAID_TIMER && overlay.as_ref().is_some_and(|o| msg.hwnd == o.window.hwnd) => overlay.as_mut().unwrap().hush(),
                // Followed while dragged, wherever the pointer is.
                WM_TIMER if msg.wParam.0 == crate::overlay::FOLLOW_TIMER && overlay.as_ref().is_some_and(|o| msg.hwnd == o.window.hwnd && o.held()) => {
                    overlay.as_mut().unwrap().moved()
                }
                // Let go (where the overlay may not hear of it), or the
                // capture taken away mid-drag (the lock screen, another
                // program): it stays where it was put.
                WM_LBUTTONUP | WM_TIMER | crate::ui::window::CAPTURE_LOST if overlay.as_ref().is_some_and(|o| msg.hwnd == o.window.hwnd) => {
                    if let Some(placed) = overlay.as_mut().unwrap().release() {
                        std::thread::spawn(move || crate::app().place_overlay(placed));
                    }
                }
                WM_RBUTTONUP if overlay.as_ref().is_some_and(|o| msg.hwnd == o.window.hwnd) => {
                    // (What comes for this thread while the menu is up waits:
                    // see `menu::show`.)
                    let choice = overlay.as_ref().unwrap().menu();
                    match choice {
                        Some(OverlayChoice::Settings) => {
                            if !panel.stays() {
                                panel.dismiss();
                            }
                            settings_window::open_at_overlay();
                        }
                        Some(OverlayChoice::Lock(locked)) => {
                            std::thread::spawn(move || crate::app().lock_overlay(locked));
                        }
                        Some(OverlayChoice::Through(through)) => {
                            std::thread::spawn(move || crate::app().overlay_through(through));
                        }
                        Some(OverlayChoice::Close) => self.set_overlay(false),
                        None => {}
                    }
                }
                WM_MOUSEMOVE if msg.hwnd == panel.window.hwnd => panel.hover_at(lparam_point(msg.lParam)),
                // The wheel over the overlay: its glass clearer (up) or
                // less clear, a step of the settings' slider a notch.
                WM_MOUSEWHEEL if overlay.as_ref().is_some_and(|o| msg.hwnd == o.window.hwnd) => {
                    let notches = (msg.wParam.0 >> 16) as u16 as i16 as f32 / 120.0;
                    std::thread::spawn(move || crate::app().clear_overlay(notches * 0.05));
                }
                WM_MOUSEWHEEL if msg.hwnd == panel.window.hwnd => {
                    let delta = (msg.wParam.0 >> 16) as u16 as i16;
                    panel.wheel(lparam_point(msg.lParam), delta);
                }
                WM_INPUT if !panel.is_open() => {
                    raw_since_watch = true;
                    if let Some(cursor) = cursor_position() {
                        on_seam(cursor, &mut panel);
                    }
                    let moved = read_motion(HRAWINPUT(msg.lParam.0 as *mut _));
                    if let (Some(moved), Some(cursor)) = (moved, cursor_position()) {
                        match at_edge(cursor) {
                            Some((contact, edge, pressure)) if armed && detector.motion(moved.toward(edge), now, pressure) => {
                                detector.reset();
                                // Not over this game: not again until the
                                // pointer has left the edge.
                                if may_open(fullscreen_edge) {
                                    panel.open(cursor, contact, edge, None, now);
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
                    if let Some(overlay) = &mut overlay {
                        overlay.take_clicks();
                    }
                    let Some(cursor) = cursor_position() else { continue };
                    if !panel.is_open() {
                        on_seam(cursor, &mut panel);
                    }
                    let moved = cursor.x != last_cursor.x || cursor.y != last_cursor.y;
                    if moved && !raw_since_watch && !panel.is_open() {
                        match at_edge(cursor) {
                            Some((_, _, pressure)) if armed => {
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
                    on_seam(cursor, &mut panel);
                    match at_edge(cursor) {
                        Some((contact, edge, _)) if armed && detector.dwell_elapsed(now) => {
                            detector.reset();
                            if may_open(fullscreen_edge) {
                                panel.open(cursor, contact, edge, None, now);
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

            let wanted = panel.is_open() && !panel.stays();
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
            let needed = !panel.is_open() && (detector.dwelling() || seam_rest.is_some());
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

/// A screen: its whole, its work area (physical px), and its scale (physical
/// px per DIP).
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct Contact {
    pub(crate) monitor: RECT,
    pub(crate) work: RECT,
    pub(crate) scale: f32,
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
    /// Moved away from the edge: how far across and down the room on its
    /// screen's work area it is (see `PanelAt`).
    floating: Option<(f32, f32)>,
    /// The corner held while it is dragged, wherever that is.
    held: Option<Held>,
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
    edges: Edges,
    sized: bool,
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

/// The pointer's shape over edges `edges` of what is dragged by them (none:
/// the whole of it, moved).
pub(crate) fn sizing(left: bool, right: bool, top: bool, bottom: bool) -> windows::core::PCWSTR {
    use windows::Win32::UI::WindowsAndMessaging::{IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE};
    match (left || right, top || bottom) {
        (true, true) if left == top => IDC_SIZENWSE,
        (true, true) => IDC_SIZENESW,
        (true, false) => IDC_SIZEWE,
        (false, true) => IDC_SIZENS,
        (false, false) => IDC_SIZEALL,
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
    taken: Instant,
}

impl Behind {
    fn new(capture: Capture, placement: &Placement, now: Instant) -> Self {
        let tone = capture.luminance(placement.panel, placement.px);
        Behind { digest: capture.digest, tone, capture, bitmap: None, taken: now }
    }
}

/// A capture of the desktop behind the panel, asked for: for which
/// `Panel::generation`, and whether it is taken again where the panel now is
/// (the panel kept out of it; its theme following), else refreshed in place.
struct PanelRequest {
    receiver: std::sync::mpsc::Receiver<Option<Capture>>,
    generation: u64,
    /// Menus shown and gone as it was asked for (see `menu::SHOWN`).
    menus: u64,
    afresh: bool,
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
    /// The bar, from the panel's corner (DIPs): a pinned panel is dragged by it.
    bar: view::Rect,
    /// Torn off as it was dragged: the widget to make (its form, scale and
    /// corner on the screen, and where the hand holds it from that corner).
    tear: Option<(Form, f32, POINT, POINT)>,
    /// Torn off: to go once the widget is drawn.
    torn: bool,
    /// A drag of the pinned panel under way.
    drag: Option<PanelDrag>,
    /// The largest size the panel still grows to on its screen (see
    /// `arrange::largest`), as it was last laid out.
    largest: f32,
    /// While it is sized by its edges: the size it is at, and the limit
    /// the pointer is past, if it is (shown on it as it is dragged).
    sizing: Option<(f32, Option<Limit>)>,
    /// The desktop behind the glass is to be taken again (the panel moved
    /// or grew past what was taken).
    recapture: bool,
    /// A capture of the desktop behind under way on another thread (see
    /// `take_behind`).
    capturing: Option<PanelRequest>,
    /// Counts what makes a capture under way of no use: the panel closing,
    /// the settings coming in front of it.
    generation: u64,
    /// The last capture asked for came back with nothing: when.
    failed_behind: Option<Instant>,
    /// Kept out of captures, as last set (see `exclude`).
    excluded: bool,
    /// The screens changed while the panel was dragged: seen to as it is
    /// let go.
    screens_missed: bool,
    /// A drag ended with the button already up (seen by the timer): the
    /// button's release, still to come, is the drag's, not a click.
    release_owed: bool,
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
            bar: view::Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
            tear: None,
            torn: false,
            drag: None,
            largest: SIZES.1,
            sizing: None,
            recapture: false,
            capturing: None,
            generation: 0,
            failed_behind: None,
            excluded: false,
            screens_missed: false,
            release_owed: false,
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

    /// The settings changed: what is shown, and how.
    fn restyle(&mut self) {
        self.next_frame = Instant::now();
        let config = self.controller.config.lock().unwrap();
        self.prefs = Prefs::resolve(&config.view, &self.controller.known_modules());
        self.columns = config.columns;
        // Mid-drag, the size is the drag's.
        if self.drag.is_none() {
            self.size = config.size;
        }
        let skin = Skin::named(&config.skin);
        // Up, and now a skin that sees the desktop: it is taken behind the
        // panel as it is now (one taken for another skin, or none, would not
        // fit it).
        if skin != self.skin && skin.sees_backdrop() && self.is_shown() {
            self.recapture = true;
        }
        self.skin = skin;
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
        // (And while the desktop behind is about to be taken again, after a
        // drag.)
        self.exclude();
        self.dress();
    }

    /// Kept out of captures while it is up with live refraction (the
    /// desktop behind it is taken without hiding it), while a widget torn
    /// off it takes the desktop (it is not to be in it), and while the
    /// desktop behind it is being taken; else in them.
    fn exclude(&mut self) {
        // (A capture under way, live or not, holds it out till it is done.)
        let excluded = (self.is_shown() && (self.live || self.torn)) || self.capturing.is_some();
        if excluded != self.excluded {
            self.window.exclude_from_capture(excluded);
            self.excluded = excluded;
        }
    }

    /// Picks the theme, and how frosted the glass is, for the skin and the
    /// desktop behind the panel.
    fn dress(&mut self) {
        let behind = self.behind.as_ref().map(|behind| behind.tone);
        let dark = theme::is_dark(self.prefs.theme, behind.map(|(mean, _)| mean));
        self.theme = Theme::new(self.skin, dark);
        self.frost = behind.map_or(0.0, |(mean, spread)| skins::frost(mean, spread, dark));
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
                self.open(point, contact, self.edge_for(point, contact), Some(at), now);
                // Opened there, pinned there if it was; one picked up on its
                // way out (opened from the edge) stays as it was.
                if !was_shown && self.is_open() {
                    self.pinned = locked;
                }
            }
        } else if let Some((cursor, contact)) = cursor_position().and_then(|cursor| Some((cursor, monitor_at(cursor)?))) {
            self.open(cursor, contact, self.edge_for(cursor, contact), None, now);
        }
    }

    /// The edge the panel opens from on `contact`'s screen when nothing
    /// pushed into one (the shortcut, the tray): of those that open it
    /// there, the nearest to `point`; where none does, the one of a screen
    /// not set otherwise.
    fn edge_for(&self, point: POINT, contact: Contact) -> Edge {
        let config = self.controller.config.lock().unwrap();
        let m = contact.monitor;
        crate::screens::lit(&config.screens, config.rest, m).any().nearest((m.left, m.top, m.right, m.bottom), (point.x, point.y)).unwrap_or(config.rest.edge)
    }

    /// Opens the panel on `contact`'s screen: along its `edge` at `cursor`,
    /// or `floating` where it was moved to (see `Placement::floating`).
    fn open(&mut self, cursor: POINT, contact: Contact, edge: Edge, floating: Option<(f32, f32)>, now: Instant) {
        if self.is_open() || self.controller.history.lock().unwrap().is_empty() {
            return;
        }
        // A reopening during the way out picks the panel up where it is.
        if !self.is_shown() {
            self.edge = edge;
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
                floating,
                held: None,
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
                let area = match (placement.floating, self.edge) {
                    (Some(_), _) => placement.window,
                    (None, Edge::Left | Edge::Right) => RECT { top: monitor.top, bottom: monitor.bottom, ..placement.window },
                    (None, Edge::Top) => RECT { left: monitor.left, right: monitor.right, ..placement.window },
                };
                self.behind = Capture::take(area).map(|capture| Behind::new(capture, placement, now));
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
        self.controller.shown.store(true, Ordering::Relaxed);
        self.exclude();
        // Away from the edge it has nowhere to slide from: it fades in.
        if reduced_motion() || self.floating() {
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
        if let Some(placement) = &self.placement {
            crate::widget::own_window_changed(placement.window);
        }
    }

    fn begin_close(&mut self, now: Instant) {
        self.phase = Phase::Closing;
        // Pinned and closed by hand: no longer opened as Glance starts.
        if std::mem::take(&mut self.pinned) && !self.floating() {
            std::thread::spawn(|| crate::app().pin_panel(None));
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
    /// as it opens tells a scale it was already placed for, and is no change.
    fn screens_changed(&mut self, dpi: u32, now: Instant) {
        // Dragged onto another screen, it is laid out for it as it is let
        // go; the screens themselves changing, seen to then.
        if self.drag.is_some() {
            self.screens_missed |= dpi == 0;
            return;
        }
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
        let (open, closing, pinned, game, floating) = (self.is_open(), matches!(self.phase, Phase::Closing), self.pinned, self.game, self.floating());
        self.dismiss();
        if closing {
            bring_back(game);
        }
        if open && floating {
            self.open_by_hand(now);
            self.pinned = pinned;
        } else if open {
            if let Some((cursor, contact)) = cursor_position().and_then(|cursor| Some((cursor, monitor_at(cursor)?))) {
                self.open(cursor, contact, self.edge_for(cursor, contact), None, now);
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
        self.end_drag();
        self.recapture = false;
        self.generation += 1;
        self.torn = false;
        self.game = None;
        self.phase = Phase::Hidden;
        self.pinned = false;
        self.window.hide();
        if let Some(placement) = &self.placement {
            crate::widget::own_window_changed(placement.window);
        }
        self.exclude();
        // The drawing memory is given back while the panel is away.
        if let Some(surface) = &mut self.surface {
            surface.release(&self.gfx);
        }
        self.layers.release();
        self.behind = None;
        if !crate::widget::any_shown() {
            self.gfx.trim();
        }
        self.placement = None;
        self.controller.shown.store(false, Ordering::Relaxed);
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
        let stays = self.stays();
        let Phase::Open { entered, outside_since, dragging } = &mut self.phase else { return };
        if stays {
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
        // Not while it gives way to the settings: they are in front of it,
        // and would be taken as what is behind it.
        if self.live && self.is_open() && self.capturing.is_none() && !self.window.yielding() {
            self.refresh_behind(now);
        }
        // A panel the desktop was taken behind, grown or moved past it
        // (dragged, sized, laid out afresh), takes it again; with live
        // refraction it is taken again anyway.
        if !self.live && self.drag.is_none() && self.skin.sees_backdrop() {
            let window = self.placement.as_ref().map(|placement| placement.window);
            let taken = self.behind.as_ref().map(|behind| behind.capture.rect);
            if let (Some(window), Some(taken)) = (window, taken) {
                self.recapture |= !covers(taken, window);
            }
        }
        // Not while it gives way to the settings: they are in front of it,
        // and would be taken as what is behind it.
        let rested = self.failed_behind.is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(1));
        if self.recapture && rested && self.capturing.is_none() && !self.window.yielding() {
            self.recapture_behind();
        }
        if now >= self.next_frame {
            self.frame(now);
        }
    }

    /// With live refraction, captures the desktop behind the window again
    /// every so often (on another thread: see `collect_behind`).
    fn refresh_behind(&mut self, now: Instant) {
        let Some(placement) = &self.placement else { return };
        // Tried again every so often, with or without a capture so far (the
        // first may have found the screen unreadable).
        // From the last try (a capture that came back with nothing too).
        if now.duration_since(self.tried_behind) < LIVE_INTERVAL {
            return;
        }
        self.tried_behind = now;
        let area = placement.window;
        self.take_behind(area, false);
    }

    /// Takes the desktop behind the glass again where the panel now is,
    /// kept out of the capture itself for the while: once the desktop has
    /// been composed again without it (on another thread: see
    /// `collect_behind`).
    fn recapture_behind(&mut self) {
        let Some(placement) = &self.placement else { return };
        let area = placement.window;
        self.recapture = false;
        self.take_behind(area, true);
    }

    /// Takes the desktop within `area` on another thread, once the desktop
    /// has been composed again for a capture `afresh` (the panel kept out of
    /// it meanwhile); the panel goes on with what it had.
    fn take_behind(&mut self, area: RECT, afresh: bool) {
        let (send, receiver) = std::sync::mpsc::channel();
        let menus = crate::ui::menu::SHOWN.load(Ordering::Relaxed);
        self.capturing = Some(PanelRequest { receiver, generation: self.generation, menus, afresh });
        self.exclude();
        std::thread::spawn(move || {
            if afresh {
                let _ = unsafe { windows::Win32::Graphics::Dwm::DwmFlush() };
            }
            let _ = send.send(Capture::take(area));
        });
    }

    /// A capture of the desktop behind come back, if it has (whether the
    /// panel is up or not: what it holds is let go of either way). Asked for
    /// what is still so: taken again where the panel now is, its theme
    /// follows it (live refraction, refreshing over the same place, leaves
    /// the theme be, and only the frost follows); the same desktop as
    /// before, the last stands; none readable just now, it is asked for
    /// again once a second has passed.
    fn collect_behind(&mut self, now: Instant) {
        let Some(request) = &self.capturing else { return };
        let capture = match request.receiver.try_recv() {
            Ok(capture) => capture,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
        };
        let request = self.capturing.take().unwrap();
        self.exclude();
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
        let behind = Behind::new(capture, placement, now);
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
        self.window.yield_to(settings);
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

    /// Whether it is moved away from the edge.
    fn floating(&self) -> bool {
        self.placement.as_ref().is_some_and(|placement| placement.floating.is_some() || placement.held.is_some())
    }

    /// Whether it stays up wherever the pointer goes: pinned, or moved away
    /// from the edge (it lives there until it is closed by hand, or moved
    /// back to the edge).
    fn stays(&self) -> bool {
        self.pinned || self.floating()
    }

    /// The edges of the panel a point `(x, y)` from its corner (DIPs) is
    /// on, if it is near them.
    fn edges_at(&self, x: f32, y: f32) -> Edges {
        let Some(opening) = &self.opening else { return Edges::default() };
        let (width, height) = (opening.layout.width(), opening.layout.height());
        Edges { left: x < EDGE, right: x >= width - EDGE, top: y < EDGE, bottom: y >= height - EDGE }
    }

    /// What a press at `client` (physical px in the window) would do to the
    /// open panel: size it by the edges it is on, or move it, from anywhere
    /// on it that takes no click or wheel of its own; nothing, pinned (it
    /// stays as it is).
    fn grip_at(&self, client: POINT) -> Option<Edges> {
        let px = match &self.placement {
            Some(placement) if !self.pinned && self.is_open() => placement.px,
            _ => return None,
        };
        let (x, y) = (client.x as f32 / px - self.corner.0, client.y as f32 / px - self.corner.1);
        let edges = self.edges_at(x, y);
        if edges.any() {
            return Some(edges);
        }
        self.hit_at(client).is_none().then_some(edges)
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
        let Some(edges) = self.grip_at(client) else { return };
        let (Some(from), Some(placement)) = (cursor_position(), &self.placement) else { return };
        self.drag = Some(PanelDrag { from, rect: placement.panel, size: self.size, edges, sized: false });
        unsafe {
            windows::Win32::UI::Input::KeyboardAndMouse::SetCapture(self.window.hwnd);
            SetTimer(Some(self.window.hwnd), crate::overlay::FOLLOW_TIMER, FOLLOW_MS, None);
        }
    }

    /// Whether it is being dragged, the button still down.
    fn held(&self) -> bool {
        self.drag.is_some() && crate::overlay::primary_down()
    }

    /// Follows the pointer while it is dragged: the panel moves with it,
    /// onto another screen too; or, held by its edges, grows and shrinks as
    /// a whole from the corner opposite them (its shape is its lanes').
    fn dragged(&mut self, now: Instant) {
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
            let _ = held;
            let Some(opening) = &self.opening else { return };
            let form = Form::Lanes { detail: Detail::Full, columns: opening.layout.columns };
            let at = POINT { x: placement.panel.left + dx, y: placement.panel.top + dy };
            self.tear = Some((form, opening.zoom, at, POINT { x: cursor.x - at.x, y: cursor.y - at.y }));
            self.end_drag();
            self.torn = true;
            return;
        }
        let (width, height) = ((r.right - r.left) as f32, (r.bottom - r.top) as f32);
        let across = (e.left || e.right).then(|| (width + if e.right { dx } else { -dx } as f32) / width);
        let down = (e.top || e.bottom).then(|| (height + if e.bottom { dy } else { -dy } as f32) / height);
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

    /// Ends a drag under way.
    fn end_drag(&mut self) -> Option<PanelDrag> {
        self.sizing = None;
        let drag = self.drag.take()?;
        unsafe {
            let _ = KillTimer(Some(self.window.hwnd), crate::overlay::FOLLOW_TIMER);
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
        }
        Some(drag)
    }

    /// Let go: it stays where it was put and as large as it was made, laid
    /// out for that screen; what to keep, if it moved (where it is) or was
    /// sized (its size).
    fn let_go(&mut self, now: Instant) -> Option<(Kept, Option<f32>)> {
        let drag = self.end_drag()?;
        let size = drag.sized.then_some(self.size);
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
            crate::screens::lit(&config.screens, config.rest, contact.monitor).any()
        };
        if let Some(edge) = lit.each().find(|edge| docks(*edge, contact, panel)) {
            self.edge = edge;
            placement.contact = contact;
            placement.px = contact.scale;
            placement.floating = None;
            placement.held = None;
            placement.anchor = centre;
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
                opening.grow(&now, &controller.info, |id, seen, detail| view::lane_height(&probe, id, seen, detail));
                opening
            }
            None => {
                let now = match self.held.take() {
                    Some(held) => held.join(&now, &controller.info),
                    None => now,
                };
                let scene = scene(controller, prefs, theme, lang, scroll, hover, pinned, samples, &now);
                let heights = view::heights(&scene);
                let tall: Vec<f32> = heights.iter().map(|(_, height)| height[0]).collect();
                self.largest = arrange::largest(theme, self.edge, &tall, work, self.columns, SIZES.1);
                let lanes: Vec<(&str, arrange::Heights)> = heights.iter().map(|(id, height)| (id.as_str(), *height)).collect();
                self.opening.insert(arrange::Opening::new(theme, self.edge, &lanes, 0, work, self.columns, self.size, arrange::Want::Itself, now.clone()))
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
        let edge = (!self.floating()).then_some(self.edge);
        let (layers, behind, frost) = (&mut self.layers, &mut self.behind, self.frost);
        let (sizing, lang, theme) = (self.sizing, self.lang, &self.theme);
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
                Some((&behind.bitmap.as_ref()?.0, at, behind.digest, px))
            });
            let picture = render::Picture { scene: &scene, lanes: &lanes, layout: &layout, small: None, size: (width, height), at: (0.0, 0.0), edge, backdrop, frost, rough: false };
            drawn = Some(layers.draw(frame, &picture, Matrix3x2::translation(x, y), px));
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
        self.bar = layout.bar();

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
        // Unpinned, its edges and its bar show what a drag there does (the
        // rest moves it too, under the reader's arrow).
        match self.drag.map(|drag| drag.edges).or_else(|| self.grip_at(client).filter(|edges| edges.any() || self.on_bar(client))) {
            Some(edges) => point_at(&self.window, edges),
            None => self.window.point(windows::Win32::UI::WindowsAndMessaging::IDC_ARROW),
        }
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
            Some(Hit::Pin) => {
                // Pinned, it stays as it is: not moved, not sized, and not
                // closed as the pointer leaves. Away from the edge, kept so
                // across restarts.
                self.pinned ^= true;
                if self.floating() {
                    crate::app().place_panel(self.kept_at());
                } else {
                    // At its edge, kept so across restarts too.
                    let at = self.placement.as_ref().filter(|_| self.pinned).map(|p| (p.anchor.x, p.anchor.y));
                    crate::app().pin_panel(at);
                }
            }
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
        buttons: true,
    }
}

/// What a desktop widget's readings are drawn with at this moment.
pub(crate) fn scene_for<'s>(controller: &'s Controller, prefs: &'s Prefs, theme: &'s Theme, lang: Lang, history: &'s [Sample], seen: &'s Seen) -> Scene<'s> {
    Scene { buttons: false, ..scene(controller, prefs, theme, lang, 0.0, None, false, history, seen) }
}

/// The sink's procedure: the input loop takes its messages before they are
/// dispatched (a menu's loop sets them aside for it: see `menu::show`).
unsafe extern "system" fn sink_procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
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
    let margin = (theme.margin * px).ceil() as i32;
    // The shadow's room, kept on this monitor (while it is dragged, onto
    // another too).
    let mut window = RECT { left: left - margin, top: top - margin, right: panel.right + margin, bottom: panel.bottom + margin };
    if placement.held.is_none() {
        window = RECT {
            left: window.left.max(monitor.left),
            top: window.top.max(monitor.top),
            right: window.right.min(monitor.right),
            bottom: window.bottom.min(monitor.bottom),
        };
    }
    let tolerance = (LEAVE_TOLERANCE * scale).round() as i32;
    let mut reach = RECT {
        left: panel.left - tolerance,
        top: panel.top - tolerance,
        right: panel.right + tolerance,
        bottom: panel.bottom + tolerance,
    };
    // The pointer arrives from the screen edge, if it is against it.
    if placement.floating.is_none() && placement.held.is_none() {
        match edge {
            Edge::Left => reach.left = monitor.left,
            Edge::Right => reach.right = monitor.right,
            Edge::Top => reach.top = monitor.top,
        }
    }
    placement.panel = panel;
    placement.reach = reach;
    placement.px = px;
    if placement.window != window {
        // Where it was and where it is: a widget that took the desktop with
        // it there takes it again.
        crate::widget::own_window_changed(placement.window);
        crate::widget::own_window_changed(window);
        placement.window = window;
        panel_window.place(window);
    }
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
    frame.stroke_rounded(Fill::Solid(edge), x + line / 2.0, y + line / 2.0, width - line, height - line, radius, line);
    Canvas::text(frame, text, font, ink, x + pad_x, y + pad_y, width, Align::Start);
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

pub(crate) fn cursor_position() -> Option<POINT> {
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

/// How the mouse moved, as it says itself: to a place, or by so many
/// counts across and down.
#[derive(Clone, Copy)]
enum Moved {
    To,
    By(i32, i32),
}

impl Moved {
    /// The move as a push into `edge`.
    fn toward(self, edge: Edge) -> Motion {
        let Moved::By(x, y) = self else { return Motion::Absolute };
        let (outward, along) = match edge {
            Edge::Left => (-x, y),
            Edge::Right => (x, y),
            Edge::Top => (-y, x),
        };
        Motion::Relative { outward, along }
    }
}

fn read_motion(handle: HRAWINPUT) -> Option<Moved> {
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
        return Some(Moved::To);
    }
    Some(Moved::By(mouse.lLastX, mouse.lLastY))
}

pub(crate) fn monitor_at(point: POINT) -> Option<Contact> {
    let handle = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe { GetMonitorInfoW(handle, &mut info) }.ok().ok()?;
    let (mut dpi, mut dpi_y) = (0, 0);
    unsafe { GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y) }.ok()?;
    Some(Contact { monitor: info.rcMonitor, work: info.rcWork, scale: dpi as f32 / 96.0 })
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

/// Describes the monitor under the cursor if the cursor is pressed against its
/// trigger edge with intent to point: visible, no button held, clear of the
/// corners, and where the edge stops it (it does not pass on to another
/// screen there).
fn edge_contact(cursor: POINT, edge: Edge) -> Option<Contact> {
    let contact = monitor_at(cursor)?;
    let monitor = contact.monitor;
    let corner = (CORNER_EXCLUSION * contact.scale as f64).round() as i32;
    let along_height = (monitor.top + corner..monitor.bottom - corner).contains(&cursor.y);
    let (on_edge, along, clear_of_corners) = match edge {
        Edge::Left => (cursor.x <= monitor.left, cursor.y, along_height),
        Edge::Right => (cursor.x >= monitor.right - 1, cursor.y, along_height),
        Edge::Top => (cursor.y <= monitor.top, cursor.x, (monitor.left + corner..monitor.right - corner).contains(&cursor.x)),
    };
    if !on_edge || !clear_of_corners || buttons_down() || !cursor_showing() {
        return None;
    }
    (!crate::screens::passes(monitor, edge, along)).then_some(contact)
}
