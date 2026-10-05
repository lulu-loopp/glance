//! Shows and hides the panel, and draws it while it is up.
//!
//! One thread does all of it: it reads the pointer, owns the panel's window
//! and draws every frame while the panel is on screen. Whether the pointer is
//! "on the panel" is decided from the global cursor position against where
//! the panel rests, so nothing that moves on screen feeds back into it.

use std::collections::VecDeque;
use std::f32::consts::E;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap1, D2D1_LAYER_PARAMETERS1};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON};
use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER,
    RIDEV_INPUTSINK, RID_INPUT, RIM_TYPEMOUSE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DispatchMessageW, GetCursorInfo, GetCursorPos, GetMessageW, KillTimer, MsgWaitForMultipleObjects,
    PeekMessageW, PostMessageW, SetTimer, SystemParametersInfoW, CURSORINFO, CURSOR_SHOWING, HWND_MESSAGE, MSG, PM_REMOVE,
    QS_ALLINPUT,
    SPI_GETCLIENTAREAANIMATION, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_INPUT, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT,
    WM_TIMER,
};

use crate::detector::{Detector, Motion};
use crate::metrics::{Sample, StaticInfo};
use crate::settings::{Edge, Settings};
use crate::ui::backdrop::Capture;
use crate::ui::gfx::{Gfx, Layer, Surface};
use crate::ui::motion::{Easing, Transition, LINEAR};
use crate::ui::prefs::Prefs;
use crate::ui::skins;
use crate::ui::text::Lang;
use crate::ui::theme::{self, Entrance, Skin, Theme};
use crate::ui::view::{self, Hit, HitBox, Layout, Pass, Scene, COLUMN_WIDTH, TABLE_ROW};
use crate::ui::window::Window;
use windows_numerics::Vector2;

/// Closest the panel gets to the ends of the work area (DIPs).
const GAP: f32 = 12.0;
/// The pointer may stray this far outside the panel and still count as on it (DIPs).
const LEAVE_TOLERANCE: f32 = 10.0;
/// Strip at each end of the edge that does not trigger (logical px): the
/// height of a title bar's buttons and of the taskbar, which own the corners.
const CORNER_EXCLUSION: f64 = 48.0;
/// History is kept for the longest chart span the settings offer.
const LONGEST_SPAN: Duration = Duration::from_secs(300);
/// The chart's pen runs this far behind the newest sample beyond one
/// interval, so the next sample has arrived by the time it is drawn to.
const PEN_LAG_MS: f64 = 100.0;
/// Columns a panel along a side may spread over before it zooms out instead.
const MAX_COLUMNS: usize = 3;
/// Share of the work area's height a panel opened from the top may take.
const TOP_SHARE: f32 = 0.6;
const TICK_MS: u32 = 16;
/// Timers on the input loop while the panel is down: one that follows the
/// pointer resting on the edge, and the slow watch for blocked input.
const TRACK_TIMER: usize = 1;
const WATCH_TIMER: usize = 2;
const WATCH_MS: u32 = 100;
const MOUSE_MOVE_ABSOLUTE: u16 = 1;
/// How often a live backdrop is captured again.
const LIVE_INTERVAL: Duration = Duration::from_millis(250);
/// Rows a wheel notch scrolls the process list by.
const WHEEL_ROWS: f32 = 3.0;
/// How quickly the process list catches up with the wheel.
const SCROLL_EASE: f32 = 0.06;

/// The panel slides in from past the edge and fades up, and goes the same way.
const OPEN: (Duration, Easing) = (Duration::from_millis(260), Easing(0.16, 1.0, 0.3, 1.0));
const OPEN_FADE: Duration = Duration::from_millis(140);
const CLOSE: (Duration, Easing) = (Duration::from_millis(180), Easing(0.4, 0.0, 1.0, 1.0));
/// With animations turned off in Windows the panel only fades.
const PLAIN_FADE: Duration = Duration::from_millis(120);

/// Requests to the input thread from elsewhere.
const OPEN_FROM_TRAY: u32 = WM_APP + 1;
const DISMISS: u32 = WM_APP + 2;
const RESTYLE: u32 = WM_APP + 3;

struct Config {
    edge: Edge,
    skin: String,
    live: bool,
    pressure: i32,
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
        pressure: settings.sensitivity.pressure(),
        close_delay: settings.close_delay(),
        interval: settings.interval(),
        history: (LONGEST_SPAN.as_millis() / settings.interval().as_millis()) as usize + 16,
        view: settings.view.clone(),
    }
}

pub struct Controller {
    app: AppHandle,
    info: StaticInfo,
    config: Mutex<Config>,
    history: Mutex<VecDeque<Sample>>,
    /// The input thread's message window, once it has one.
    sink: AtomicIsize,
    shown: AtomicBool,
    /// While the settings window is open it shows a live preview, and gets
    /// every sample.
    settings_open: AtomicBool,
}

impl Controller {
    pub fn new(app: AppHandle, info: StaticInfo, settings: &Settings) -> Self {
        Controller {
            app,
            info,
            config: Mutex::new(config_from(settings)),
            history: Mutex::new(VecDeque::new()),
            sink: AtomicIsize::new(0),
            shown: AtomicBool::new(false),
            settings_open: AtomicBool::new(false),
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
        if self.settings_open.load(Ordering::Relaxed) {
            self.app.emit("sample", &sample).unwrap();
        }
        let keep = self.config.lock().unwrap().history;
        let mut history = self.history.lock().unwrap();
        while history.len() >= keep {
            history.pop_front();
        }
        history.push_back(sample);
    }

    pub fn set_settings_open(&self, open: bool) {
        self.settings_open.store(open, Ordering::Relaxed);
    }

    pub fn history(&self) -> Vec<Sample> {
        self.history.lock().unwrap().iter().cloned().collect()
    }

    pub fn is_shown(&self) -> bool {
        self.shown.load(Ordering::Relaxed)
    }

    /// Opens the panel from the tray icon, on the monitor the pointer is on.
    pub fn open_from_tray(&self) {
        self.post(OPEN_FROM_TRAY);
    }

    /// Takes the panel off the screen at once, without its animation.
    pub fn dismiss(&self) {
        self.post(DISMISS);
    }

    /// The modules this machine has, in their default order.
    fn known_modules(&self) -> Vec<String> {
        let mut known = vec!["cpu".to_string()];
        known.extend((0..self.info.gpus.len()).map(|i| format!("gpu:{i}")));
        known.extend(["memory", "network", "disk", "processes", "storage", "board", "battery", "system"].map(String::from));
        known
    }

    /// Runs the input loop, which also carries the panel. Raw input is
    /// read-only: unlike a mouse hook it cannot delay the pointer, and Windows
    /// never silently disconnects it.
    pub fn run(&self) {
        let sink = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
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
        let mut panel = Panel::new(self).expect("panel window");
        self.sink.store(sink.0 as isize, Ordering::Release);

        let mut detector = Detector::default();
        // Once the panel has been up, the pointer has to leave the edge before
        // it can open the panel again: pushing on after it closed is the same
        // gesture, not a new one.
        let mut armed = true;
        let mut ticking = false;
        // Raw input stops reaching this process while a window of higher
        // privilege (Task Manager, an installer) has the focus. A slow watch
        // on the cursor notices that: it moved, and no raw input came. The
        // pointer is then treated as one that cannot push, and opens the
        // panel by resting on the edge.
        let mut last_cursor = cursor_position();
        let mut raw_since_watch = false;
        unsafe { SetTimer(Some(sink), WATCH_TIMER, WATCH_MS, None) };
        let mut msg = MSG::default();
        loop {
            // While the panel is up it follows the pointer and is drawn
            // whenever it is due, between the messages that arrived since;
            // otherwise the loop sleeps until a message comes.
            if panel.is_shown() {
                if !unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
                    let now = Instant::now();
                    panel.tick(now);
                    if !panel.is_shown() {
                        continue;
                    }
                    let wait = panel.next_frame.saturating_duration_since(now);
                    if wait.is_zero() {
                        // In motion: the next frame comes with the screen's.
                        let _ = unsafe { DwmFlush() };
                    } else {
                        let timeout = wait.min(Duration::from_millis(TICK_MS as u64)).as_millis() as u32;
                        unsafe { MsgWaitForMultipleObjects(None, false, timeout, QS_ALLINPUT) };
                    }
                    continue;
                }
                if msg.message == WM_QUIT {
                    return;
                }
            } else if !unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
                return;
            }
            let now = Instant::now();
            let (edge, pressure) = {
                let config = self.config.lock().unwrap();
                (config.edge, config.pressure)
            };
            if panel.is_shown() {
                armed = false;
            }
            match msg.message {
                OPEN_FROM_TRAY => {
                    let cursor = cursor_position();
                    if let Some(contact) = monitor_at(cursor) {
                        panel.open(cursor, contact, now);
                    }
                }
                DISMISS => panel.dismiss(),
                RESTYLE => panel.restyle(),
                WM_LBUTTONUP if msg.hwnd == panel.window.hwnd => panel.click(lparam_point(msg.lParam)),
                WM_MOUSEMOVE if msg.hwnd == panel.window.hwnd => panel.hover_at(lparam_point(msg.lParam)),
                WM_MOUSEWHEEL if msg.hwnd == panel.window.hwnd => {
                    let delta = (msg.wParam.0 >> 16) as u16 as i16;
                    panel.wheel(lparam_point(msg.lParam), delta);
                }
                WM_INPUT if !panel.is_open() => {
                    raw_since_watch = true;
                    if let Some(motion) = read_motion(HRAWINPUT(msg.lParam.0 as *mut _), edge) {
                        let cursor = cursor_position();
                        match edge_contact(cursor, edge) {
                            Some(contact) if armed && detector.motion(motion, now, pressure) => {
                                detector.reset();
                                panel.open(cursor, contact, now);
                            }
                            Some(_) => {}
                            None => {
                                detector.reset();
                                armed = !panel.is_shown();
                            }
                        }
                    }
                }
                WM_TIMER if msg.wParam.0 == WATCH_TIMER => {
                    let cursor = cursor_position();
                    let moved = cursor.x != last_cursor.x || cursor.y != last_cursor.y;
                    if moved && !raw_since_watch && !panel.is_open() {
                        match edge_contact(cursor, edge) {
                            Some(_) if armed => {
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
                    let cursor = cursor_position();
                    match edge_contact(cursor, edge) {
                        Some(contact) if armed && detector.dwell_elapsed(now) => {
                            detector.reset();
                            panel.open(cursor, contact, now);
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

#[derive(Clone, Copy)]
struct Contact {
    monitor: RECT,
    work: RECT,
    scale: f32,
}

/// Where the panel is, for the opening under way.
struct Placement {
    contact: Contact,
    /// The pointer where the panel was opened; the panel centres on it.
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
    /// The capture, until it is first drawn and becomes the bitmap.
    capture: Option<Capture>,
    bitmap: Option<ID2D1Bitmap1>,
    taken: Instant,
}

impl Behind {
    fn new(capture: Capture, placement: &Placement, now: Instant) -> Self {
        let tone = capture.luminance(placement.panel, placement.px);
        Behind { digest: capture.digest, tone, capture: Some(capture), bitmap: None, taken: now }
    }
}

/// The panel and its window, owned by the input thread.
struct Panel<'a> {
    controller: &'a Controller,
    gfx: Gfx,
    window: Window,
    surface: Surface,
    /// What the skin puts behind the readings, and the readings themselves
    /// but for the charts, which alone move between samples.
    ground: Layer,
    content: Layer,
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
    /// Refresh the desktop behind the glass while the panel is open.
    live: bool,
    behind: Option<Behind>,
    frost: f32,
    scroll: f32,
    scroll_target: f32,
    hover: Option<Hit>,
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
        let gfx = Gfx::new()?;
        let window = Window::new()?;
        let surface = Surface::new(&gfx, window.hwnd)?;
        let mut panel = Panel {
            controller,
            gfx,
            window,
            surface,
            ground: Layer::default(),
            content: Layer::default(),
            phase: Phase::Hidden,
            shift: Transition::settled(1.0),
            opacity: Transition::settled(0.0),
            placement: None,
            prefs: Prefs::default(),
            skin: Skin::Paper,
            theme: Theme::new(Skin::Paper, false),
            lang: Lang::En,
            edge: Edge::Right,
            live: false,
            behind: None,
            frost: 0.0,
            scroll: 0.0,
            scroll_target: 0.0,
            hover: None,
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
        self.skin = Skin::named(&config.skin);
        self.live = config.live && self.skin.sees_backdrop();
        drop(config);
        self.lang = Lang::resolve(self.prefs.language);
        // While the backdrop is live the window has to stay out of the
        // captures of what is behind it; the system offers that only as
        // "out of every capture", screenshots too.
        self.window.exclude_from_capture(self.live);
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
            self.placement = Some(Placement {
                contact,
                anchor: cursor,
                window: RECT::default(),
                panel: RECT::default(),
                reach: RECT::default(),
                px: contact.scale,
            });
            self.scroll = 0.0;
            self.scroll_target = 0.0;
            self.hover = None;
            self.shift.jump(1.0);
            self.opacity.jump(0.0);
            // The desktop where the panel will be, taken while the window is
            // still hidden.
            if self.skin.sees_backdrop() {
                self.arrange();
                let placement = self.placement.as_ref().unwrap();
                self.behind = Some(Behind::new(Capture::take(placement.window), placement, now));
                self.dress();
            }
        }
        self.phase = Phase::Open { entered: false, outside_since: None, dragging: false };
        self.controller.shown.store(true, Ordering::Relaxed);
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
        self.window.set_click_through(true);
        if reduced_motion() {
            self.opacity.retarget(0.0, PLAIN_FADE, LINEAR, now);
        } else {
            self.shift.retarget(1.0, CLOSE.0, CLOSE.1, now);
            self.opacity.retarget(0.0, CLOSE.0, LINEAR, now);
        }
    }

    /// Takes the panel down at once.
    fn dismiss(&mut self) {
        if !self.is_shown() {
            return;
        }
        self.phase = Phase::Hidden;
        self.window.hide();
        // The drawing memory is given back while the panel is away.
        self.surface.release(&self.gfx);
        self.ground.release();
        self.content.release();
        self.behind = None;
        self.gfx.trim();
        self.placement = None;
        self.controller.shown.store(false, Ordering::Relaxed);
    }

    /// Follows the pointer while the panel is open: closes it once the
    /// pointer has been away long enough, or on a press elsewhere.
    fn track(&mut self, now: Instant) {
        let Some(placement) = &self.placement else { return };
        let cursor = cursor_position();
        let on_panel = contains(&placement.panel, cursor);
        let in_reach = contains(&placement.reach, cursor);
        let close_delay = self.controller.config.lock().unwrap().close_delay;
        let held = buttons_down();
        if !on_panel && self.hover.take().is_some() {
            self.next_frame = now;
        }
        let Phase::Open { entered, outside_since, dragging } = &mut self.phase else { return };
        // A press that begins away from the panel dismisses it; one that
        // begins on it keeps it open wherever the pointer then goes.
        let pressed_outside = held && !*dragging && !in_reach;
        *dragging = held && (*dragging || in_reach);
        *entered |= in_reach;
        if pressed_outside {
            return self.begin_close(now);
        }
        if in_reach || *dragging || !*entered {
            *outside_since = None;
        } else if now.duration_since(*outside_since.get_or_insert(now)) >= close_delay {
            return self.begin_close(now);
        }
        // Around the panel the window is only shadow; clicks there belong
        // to whatever is underneath.
        self.window.set_click_through(!on_panel);
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
        let (Some(behind), Some(placement)) = (&self.behind, &self.placement) else { return };
        if now.duration_since(behind.taken) < LIVE_INTERVAL {
            return;
        }
        let capture = Capture::take(placement.window);
        if capture.digest == behind.digest {
            self.behind.as_mut().unwrap().taken = now;
            return;
        }
        let behind = Behind::new(capture, placement, now);
        // The theme holds for the opening; only the frost follows.
        self.frost = skins::frost(behind.tone.0, behind.tone.1, self.theme.dark);
        self.behind = Some(behind);
        self.next_frame = now;
    }

    /// Lays the panel out for the current readings and places the window.
    fn arrange(&mut self) -> (Vec<view::Lane>, Layout) {
        let controller = self.controller;
        let mut history = controller.history.lock().unwrap();
        let scene = scene(controller, &self.prefs, &self.theme, self.lang, self.scroll, self.hover, history.make_contiguous());
        let lanes = view::lanes(&scene);
        let heights: Vec<f32> = lanes.iter().map(|lane| lane.height(&self.theme)).collect();
        drop(history);

        // As many columns as the lanes need to show at full size, up to what
        // the screen allows; zoomed out only if even that is too little.
        let contact = self.placement.as_ref().unwrap().contact;
        let work_width = (contact.work.right - contact.work.left) as f32 / contact.scale;
        let work_height = (contact.work.bottom - contact.work.top) as f32 / contact.scale;
        let (room, max_columns) = match self.edge {
            Edge::Left | Edge::Right => (work_height - 2.0 * GAP, MAX_COLUMNS),
            // Along the top the panel grows sideways instead, and no lower
            // than a share of the screen.
            Edge::Top => (
                work_height * TOP_SHARE - GAP - self.theme.inset,
                (((work_width - 2.0 * GAP + self.theme.column_gap) / (COLUMN_WIDTH + self.theme.column_gap)) as usize).max(1),
            ),
        };
        let layout = Layout::new(heights, room, max_columns, &self.theme);
        let zoom = (room / layout.height()).min(1.0);
        let placement = self.placement.as_mut().unwrap();
        place(placement, &self.window, self.edge, &self.theme, (layout.width(), layout.height()), zoom);
        (lanes, layout)
    }

    /// Draws the panel as it is at `now`, and finishes a closing that is done.
    fn frame(&mut self, now: Instant) {
        if matches!(self.phase, Phase::Closing) && self.shift.done(now) && self.opacity.done(now) {
            return self.dismiss();
        }
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.scroll += (self.scroll_target - self.scroll) * (1.0 - E.powf(-dt / SCROLL_EASE));

        let (lanes, layout) = self.arrange();
        let boxes = layout.lanes();
        let bar = layout.bar();
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

        // What each layer shows: when none of it changed, the one drawn for
        // an earlier frame serves again.
        let mut key = DefaultHasher::new();
        (self.skin, self.theme.dark, self.edge, px.to_bits(), self.frost.to_bits()).hash(&mut key);
        boxes.iter().chain([&bar]).for_each(|r| [r.x, r.y, r.w, r.h].map(f32::to_bits).hash(&mut key));
        self.behind.as_ref().map(|behind| behind.digest).hash(&mut key);
        let ground_key = key.finish();
        (&lanes, &layout.cuts, layout.columns, self.lang, self.scroll.to_bits(), self.hover).hash(&mut key);
        let content_key = key.finish();

        let controller = self.controller;
        let mut history = controller.history.lock().unwrap();
        let scene = scene(controller, &self.prefs, &self.theme, self.lang, self.scroll, self.hover, history.make_contiguous());
        let theme = &self.theme;
        let edge = self.edge;
        let frost = self.frost;
        let (ground, content, behind) = (&mut self.ground, &mut self.content, &mut self.behind);
        let margin = theme.margin;
        let halo = (theme.skin == Skin::Glass).then_some(theme.legibility);
        let mut hits = None;
        let mut grounded = false;
        let drawn = self.surface.draw(&self.gfx, size, px, |frame| {
            frame.origin(x, y);
            if opacity < 1.0 {
                let everything = D2D_RECT_F { left: -f32::MAX, top: -f32::MAX, right: f32::MAX, bottom: f32::MAX };
                let layer = D2D1_LAYER_PARAMETERS1 { contentBounds: everything, opacity, ..Default::default() };
                unsafe { frame.dc.PushLayer(&layer, None) };
            }
            let backdrop = behind.as_mut().map(|behind| {
                if let Some(capture) = behind.capture.take() {
                    behind.bitmap = Some(capture.bitmap(&frame.dc, px).expect("backdrop bitmap"));
                }
                // Aligned with the screen where the panel rests.
                (behind.bitmap.as_ref().unwrap(), Vector2 { X: rest.0, Y: rest.1 })
            });
            let below = skins::Ground { theme, edge, size: (width, height), lanes: &boxes, bar, backdrop, frost };
            let area = (-margin, -margin, width + 2.0 * margin, height + 2.0 * margin);
            ground
                .draw(frame, ground_key, area, px, None, |frame| {
                    skins::draw(frame, &below).expect("panel surface");
                    grounded = true;
                })
                .expect("panel ground");
            content
                .draw(frame, content_key, (0.0, 0.0, width, height), px, halo, |frame| {
                    hits = Some(view::paint(frame, &scene, &lanes, &layout, Pass::Content))
                })
                .expect("panel content");
            view::paint(frame, &scene, &lanes, &layout, Pass::Plots);
            if opacity < 1.0 {
                unsafe { frame.dc.PopLayer() };
            }
        });
        drop(history);
        drawn.expect("panel frame");
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
                let app = self.controller.app.clone();
                // Building a window waits on the main thread; not from here.
                std::thread::spawn(move || crate::show_settings(&app));
            }
            Some(Hit::Sort(sort)) if sort != self.prefs.processes.sort => {
                self.prefs.processes.sort = sort;
                self.scroll = 0.0;
                self.scroll_target = 0.0;
                // Kept for next time, as if chosen in the settings.
                let app = self.controller.app.clone();
                std::thread::spawn(move || crate::set_process_sort(&app, sort));
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
    history: &'s [Sample],
) -> Scene<'s> {
    let interval = controller.config.lock().unwrap().interval;
    let wall = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64() * 1000.0;
    Scene {
        info: &controller.info,
        prefs,
        theme,
        lang,
        history,
        pen_ms: wall - interval.as_secs_f64() * 1000.0 - PEN_LAG_MS,
        process_scroll: scroll,
        hover,
    }
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
    let contact = monitor_at(cursor_position())?;
    Some((contact.work, contact.scale as f64))
}

fn contains(rect: &RECT, point: POINT) -> bool {
    (rect.left..rect.right).contains(&point.x) && (rect.top..rect.bottom).contains(&point.y)
}

fn cursor_position() -> POINT {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }.expect("cursor position");
    point
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
