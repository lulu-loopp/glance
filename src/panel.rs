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
    CreateWindowExW, DispatchMessageW, GetCursorInfo, GetCursorPos, KillTimer, WaitMessage, MsgWaitForMultipleObjects,
    PeekMessageW, PostMessageW, SetTimer, SystemParametersInfoW, CURSORINFO, CURSOR_SHOWING, HWND_MESSAGE, MSG, PM_REMOVE,
    QS_ALLINPUT,
    SPI_GETCLIENTAREAANIMATION, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_INPUT, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT,
    WM_TIMER,
};

use crate::detector::{Detector, Motion};
use crate::metrics::{Sample, StaticInfo};
use crate::settings::{Anchor, Edge, Settings};
use crate::ui::backdrop::Capture;
use crate::ui::gfx::{self, Gfx, Surface};
use crate::ui::render::{self, PanelLayers, GAP};
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
/// With animations turned off in Windows the panel only fades.
const PLAIN_FADE: Duration = Duration::from_millis(120);

/// Requests to the input thread from elsewhere.
const OPEN_FROM_TRAY: u32 = WM_APP + 1;
const DISMISS: u32 = WM_APP + 2;
const RESTYLE: u32 = WM_APP + 3;
const OPEN_SETTINGS: u32 = WM_APP + 4;
const TOGGLE: u32 = WM_APP + 5;

struct Config {
    edge: Edge,
    skin: String,
    live: bool,
    anchor: Anchor,
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
        anchor: settings.anchor,
        pressure: settings.sensitivity.pressure(),
        close_delay: settings.close_delay(),
        interval: settings.interval(),
        history: (LONGEST_SPAN.as_millis() / settings.interval().as_millis()) as usize + 16,
        view: settings.view.clone(),
    }
}

pub struct Controller {
    info: StaticInfo,
    config: Mutex<Config>,
    /// The samples of the longest chart span, oldest first.
    pub history: Mutex<VecDeque<Sample>>,
    /// The input thread's message window, once it has one.
    sink: AtomicIsize,
    shown: AtomicBool,
}

impl Controller {
    pub fn new(info: StaticInfo, settings: &Settings) -> Self {
        Controller {
            info,
            config: Mutex::new(config_from(settings)),
            history: Mutex::new(VecDeque::new()),
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
        // In the process's multithreaded apartment (see lib.rs).
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
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
        let mut last_cursor = cursor_position().unwrap_or_default();
        let mut raw_since_watch = false;
        unsafe { SetTimer(Some(sink), WATCH_TIMER, WATCH_MS, None) };
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
                    // In motion: the next frame comes with the screen's.
                    Some(wait) if wait.is_zero() => {
                        let _ = unsafe { DwmFlush() };
                    }
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
            let (edge, pressure) = {
                let config = self.config.lock().unwrap();
                (config.edge, config.pressure)
            };
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
                TOGGLE => {
                    if let Some((cursor, contact)) = cursor_position().and_then(|cursor| Some((cursor, monitor_at(cursor)?))) {
                        panel.open(cursor, contact, now);
                    }
                }
                DISMISS => panel.dismiss(),
                // Placed, zoomed and backed for screens that are no longer
                // so: taken down, and opened afresh next time.
                crate::ui::window::SCREENS_CHANGED => panel.screens_changed(msg.wParam.0 as u32),
                OPEN_SETTINGS => settings_window::open(),
                RESTYLE => panel.restyle(),
                WM_LBUTTONUP if msg.hwnd == panel.window.hwnd => panel.click(lparam_point(msg.lParam)),
                WM_MOUSEMOVE if msg.hwnd == panel.window.hwnd => panel.hover_at(lparam_point(msg.lParam)),
                WM_MOUSEWHEEL if msg.hwnd == panel.window.hwnd => {
                    let delta = (msg.wParam.0 >> 16) as u16 as i16;
                    panel.wheel(lparam_point(msg.lParam), delta);
                }
                WM_INPUT if !panel.is_open() => {
                    raw_since_watch = true;
                    let motion = read_motion(HRAWINPUT(msg.lParam.0 as *mut _), edge);
                    if let (Some(motion), Some(cursor)) = (motion, cursor_position()) {
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
                // While another desktop has the input (a UAC prompt, the lock
                // screen) the pointer cannot be read, and is left alone.
                WM_TIMER if msg.wParam.0 == WATCH_TIMER => {
                    let Some(cursor) = cursor_position() else { continue };
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
                    let Some(cursor) = cursor_position() else { continue };
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
    /// The shape the panel opened with, held while it is up, and the
    /// settings it was made for.
    shape: render::Shape,
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
            live: false,
            behind: None,
            tried_behind: Instant::now(),
            frost: 0.0,
            scroll: 0.0,
            scroll_target: 0.0,
            hover: None,
            pinned: false,
            shape: render::Shape::default(),
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
        self.skin = Skin::named(&config.skin);
        self.live = config.live && self.skin.sees_backdrop();
        drop(config);
        // What is shown, and how, changed: the shape is made anew. The
        // process list's order (sorted from the panel itself) changes no
        // lane's size, and leaves it be.
        let mut sized = self.prefs.clone();
        sized.processes.sort = Default::default();
        let style = format!("{} {:?} {}", serde_json::to_string(&self.edge).unwrap_or_default(), self.skin, serde_json::to_string(&sized).unwrap_or_default());
        if style != self.style {
            self.style = style;
            self.shape = render::Shape::default();
        }
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
            // A new opening takes the shape the readings now give it.
            self.shape = render::Shape::default();
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
    fn screens_changed(&mut self, dpi: u32) {
        let scale = self.placement.as_ref().map(|placement| placement.contact.scale);
        if dpi == 0 || scale.is_some_and(|scale| (dpi as f32 / 96.0 - scale).abs() > 0.001) {
            self.dismiss();
        }
    }

    /// Takes the panel down at once.
    fn dismiss(&mut self) {
        if !self.is_shown() {
            return;
        }
        self.phase = Phase::Hidden;
        self.pinned = false;
        self.window.hide();
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

    /// Lays the panel out for the current readings and places the window.
    fn arrange(&mut self) -> (Vec<view::Lane>, Layout) {
        let controller = self.controller;
        let mut history = controller.history.lock().unwrap();
        let scene = scene(controller, &self.prefs, &self.theme, self.lang, self.scroll, self.hover, self.pinned, history.make_contiguous());
        let lanes = view::lanes(&scene);
        let heights: Vec<(&str, f32)> = lanes.iter().map(|lane| (lane.id.as_str(), lane.height(&self.theme))).collect();
        drop(history);

        let contact = self.placement.as_ref().unwrap().contact;
        let work = (
            (contact.work.right - contact.work.left) as f32 / contact.scale,
            (contact.work.bottom - contact.work.top) as f32 / contact.scale,
        );
        let (layout, zoom) = self.shape.arrange(&self.theme, self.edge, &heights, work);
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
            return self.dismiss();
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
        let scene = scene(controller, &self.prefs, &self.theme, self.lang, self.scroll, self.hover, self.pinned, history.make_contiguous());
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
        pinned,
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
    let contact = monitor_at(cursor_position()?)?;
    Some((contact.work, contact.scale as f64))
}

fn contains(rect: &RECT, point: POINT) -> bool {
    (rect.left..rect.right).contains(&point.x) && (rect.top..rect.bottom).contains(&point.y)
}

/// Where the pointer is; `None` while another desktop has the input (a UAC
/// prompt, the lock screen).
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
