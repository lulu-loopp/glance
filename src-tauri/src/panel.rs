//! Shows and hides the panel window.
//!
//! The window is a column along the trigger edge, as tall as the work area,
//! and stays put while it is on screen; the page lays the panel out inside it
//! and animates it. Whether the pointer is "on the panel" is decided here from
//! the global cursor position against the rectangle the page reports, so
//! nothing that moves on screen can feed back into that decision.

use std::collections::VecDeque;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DwmFlush, DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND};
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
    CreateWindowExW, DispatchMessageW, GetCursorInfo, GetCursorPos, GetMessageW, IsWindowVisible, KillTimer, SetTimer,
    SetWindowDisplayAffinity, SetWindowPos, CURSORINFO, CURSOR_SHOWING, HWND_MESSAGE, HWND_TOPMOST, MSG, SWP_NOACTIVATE,
    SWP_NOSIZE, SWP_NOZORDER, WINDOW_EX_STYLE, WINDOW_STYLE, WM_INPUT, WM_TIMER, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
};

use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2_19, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
};
use windows::core::Interface;

use crate::capture;
use crate::detector::{Detector, Motion};
use crate::metrics::Sample;
use crate::settings::{Edge, Settings};

/// Closest the panel gets to the top and bottom of the work area (logical px).
/// The page keeps the same gap.
const GAP: f64 = 12.0;
/// The pointer may stray this far outside the panel and still count as on it (logical px).
const LEAVE_TOLERANCE: f64 = 10.0;
/// Strip at each end of the edge that does not trigger (logical px): the
/// height of a title bar's buttons and of the taskbar, which own the corners.
const CORNER_EXCLUSION: f64 = 48.0;
/// History is kept for the longest chart span the settings offer.
const LONGEST_SPAN: Duration = Duration::from_secs(300);
const TICK_MS: u32 = 16;
/// Timers on the input loop: one that follows the pointer while the panel is
/// up or the pointer rests on the edge, and the slow watch for blocked input.
const TRACK_TIMER: usize = 1;
const WATCH_TIMER: usize = 2;
const WATCH_MS: u32 = 100;
/// How often a live backdrop is captured again.
const LIVE_INTERVAL: Duration = Duration::from_millis(250);
const MOUSE_MOVE_ABSOLUTE: u16 = 1;

/// What the page draws behind the panel.
#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backdrop {
    /// Its own opaque surface.
    None,
    /// A capture of the screen behind the window.
    Snapshot,
}

/// How the current skin wants its window, as declared by the page.
#[derive(Clone, Deserialize)]
pub struct Surface {
    /// The height of each lane, the space between lanes in a column, the
    /// height of what is not lanes (the bar), and a column's width and the
    /// space between columns, at zoom 1 (logical px). How many columns the
    /// lanes need depends on the screen, which only this side knows before
    /// the panel opens; the page splits them by the same rule.
    lane_heights: Vec<f64>,
    lane_gap: f64,
    chrome_height: f64,
    column_width: f64,
    column_gap: f64,
    /// Transparent space the page needs around the panel for its shadow.
    margin: f64,
    /// Gap between the panel and the screen edge.
    inset: f64,
    backdrop: Backdrop,
}

/// The panel's box inside the window, in page px.
#[derive(Clone, Copy, Deserialize)]
pub struct PageRect {
    left: f64,
    top: f64,
    width: f64,
    height: f64,
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
    /// The page is animating the panel away and will report when it is done.
    Closing,
}

struct Placement {
    window: RECT,
    monitor: RECT,
    work: RECT,
    scale: f64,
    zoom: f64,
}

struct Config {
    edge: Edge,
    live: bool,
    pressure: i32,
    close_delay: Duration,
    history: usize,
}

struct Inner {
    phase: Phase,
    /// Counts openings, so late reports about an earlier one are ignored.
    epoch: u64,
    surface: Option<Surface>,
    placement: Option<Placement>,
    /// Panel and tolerance rectangles in physical screen coordinates, once
    /// the page has placed the panel for the current opening.
    panel: Option<RECT>,
    reach: RECT,
    click_through: bool,
    history: VecDeque<Sample>,
    /// The rows the page last laid out (see `Sample::layout_rows`).
    rows: Option<Vec<String>>,
    /// The number the current capture is served under, if there is one, and
    /// when it was taken.
    shot: Option<u64>,
    shot_at: Instant,
    /// What the current capture shows, to tell a changed desktop from the same one.
    shot_hash: Option<u64>,
    shots: u64,
}

pub struct Controller {
    app: AppHandle,
    hwnd: isize,
    config: Mutex<Config>,
    inner: Mutex<Inner>,
    /// The current capture. Kept apart from `inner` because it is served on
    /// the main thread, which must never wait for a lock that a thread waiting
    /// on the main thread may hold.
    capture: Mutex<Option<(u64, Arc<Vec<u8>>)>>,
    /// While the settings window is open it shows a live preview, and gets
    /// every sample.
    settings_open: AtomicBool,
}

#[derive(Clone, Serialize)]
struct OpenPayload<'a> {
    epoch: u64,
    history: &'a VecDeque<Sample>,
    room: Room,
    /// Pointer position relative to the window (physical px).
    focus_x: i32,
    focus_y: i32,
    /// Where to fetch the capture of the screen behind the window.
    backdrop: Option<String>,
}

#[derive(Clone, Serialize)]
struct BackdropFrame {
    epoch: u64,
    backdrop: Option<String>,
}

#[derive(Clone, Serialize)]
struct BackdropPayload {
    epoch: u64,
    room: Room,
    backdrop: Option<String>,
}

struct Contact {
    monitor: RECT,
    work: RECT,
    scale: f64,
}

fn config_from(settings: &Settings) -> Config {
    Config {
        edge: settings.edge,
        live: settings.live_backdrop,
        pressure: settings.sensitivity.pressure(),
        close_delay: settings.close_delay(),
        history: (LONGEST_SPAN.as_millis() / settings.interval().as_millis()) as usize + 16,
    }
}

impl Controller {
    pub fn new(app: AppHandle, hwnd: HWND, settings: &Settings) -> Self {
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &DWMWCP_DONOTROUND as *const _ as *const _,
                size_of_val(&DWMWCP_DONOTROUND) as u32,
            )
        }
        .unwrap();
        // No system fade on show and hide: the page animates the panel itself,
        // and a capture taken right after hiding must not catch it fading out.
        let disabled = BOOL::from(true);
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_TRANSITIONS_FORCEDISABLED,
                &disabled as *const _ as *const _,
                size_of::<BOOL>() as u32,
            )
        }
        .unwrap();
        Controller {
            app,
            hwnd: hwnd.0 as isize,
            config: Mutex::new(config_from(settings)),
            capture: Mutex::new(None),
            settings_open: AtomicBool::new(false),
            inner: Mutex::new(Inner {
                phase: Phase::Hidden,
                epoch: 0,
                surface: None,
                placement: None,
                panel: None,
                reach: RECT::default(),
                click_through: false,
                history: VecDeque::new(),
                rows: None,
                shot: None,
                shot_at: Instant::now(),
                shot_hash: None,
                shots: 0,
            }),
        }
    }

    pub fn apply(&self, settings: &Settings) {
        *self.config.lock().unwrap() = config_from(settings);
        self.exclude_from_capture(settings.live_backdrop);
    }

    /// While the backdrop is live the window has to stay out of the captures
    /// of what is behind it; the system offers that only as "out of every
    /// capture".
    fn exclude_from_capture(&self, exclude: bool) {
        let affinity = if exclude { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
        unsafe { SetWindowDisplayAffinity(self.hwnd(), affinity) }.expect("display affinity");
    }

    fn edge(&self) -> Edge {
        self.config.lock().unwrap().edge
    }

    /// The page laid the panel out, or changed skin.
    pub fn set_surface(&self, surface: Surface) {
        let mut inner = self.inner.lock().unwrap();
        inner.surface = Some(surface);
        if !matches!(inner.phase, Phase::Hidden) {
            self.fit(&mut inner);
        }
    }

    /// The page placed the panel inside the window for opening `epoch`.
    pub fn set_panel_rect(&self, epoch: u64, rect: PageRect) {
        let mut inner = self.inner.lock().unwrap();
        let Some(placement) = &inner.placement else { return };
        if inner.epoch != epoch {
            return;
        }
        let px = placement.scale * placement.zoom;
        let left = placement.window.left + (rect.left * px).round() as i32;
        let top = placement.window.top + (rect.top * px).round() as i32;
        let panel = RECT {
            left,
            top,
            right: left + (rect.width * px).round() as i32,
            bottom: top + (rect.height * px).round() as i32,
        };
        let tolerance = (LEAVE_TOLERANCE * placement.scale).round() as i32;
        let mut reach = RECT {
            left: panel.left - tolerance,
            top: panel.top - tolerance,
            right: panel.right + tolerance,
            bottom: panel.bottom + tolerance,
        };
        // The gap between the panel and the screen edge is where the pointer arrives.
        match self.edge() {
            Edge::Left => reach.left = placement.monitor.left,
            Edge::Right => reach.right = placement.monitor.right,
            Edge::Top => reach.top = placement.monitor.top,
        }
        inner.panel = Some(panel);
        inner.reach = reach;
    }

    /// Stores a sample and forwards it to the page while the panel is on
    /// screen. A hidden page is left alone, except when a sample would change
    /// its layout: the panel has to be the right size the moment it opens.
    pub fn record(&self, sample: Sample) {
        let mut inner = self.inner.lock().unwrap();
        let keep = self.config.lock().unwrap().history;
        while inner.history.len() >= keep {
            inner.history.pop_front();
        }
        let rows = sample.layout_rows();
        let watched = !matches!(inner.phase, Phase::Hidden) || self.settings_open.load(Ordering::Relaxed);
        if watched || inner.rows.as_ref() != Some(&rows) {
            self.app.emit("sample", &sample).unwrap();
            inner.rows = Some(rows);
        }
        inner.history.push_back(sample);
    }

    pub fn set_settings_open(&self, open: bool) {
        self.settings_open.store(open, Ordering::Relaxed);
    }

    pub fn history(&self) -> Vec<Sample> {
        self.inner.lock().unwrap().history.iter().cloned().collect()
    }

    pub fn is_shown(&self) -> bool {
        !matches!(self.inner.lock().unwrap().phase, Phase::Hidden)
    }

    fn is_open(&self) -> bool {
        matches!(self.inner.lock().unwrap().phase, Phase::Open { .. })
    }

    /// Capture number `shot` of the screen behind the window.
    pub fn snapshot(&self, shot: u64) -> Option<Arc<Vec<u8>>> {
        let capture = self.capture.lock().unwrap();
        capture.as_ref().filter(|(at, _)| *at == shot).map(|(_, bytes)| bytes.clone())
    }

    /// Captures the screen inside `window` for the current surface, if it wants that.
    fn shoot(&self, inner: &mut Inner, window: RECT) {
        let capture = match &inner.surface {
            Some(surface) if surface.backdrop == Backdrop::Snapshot => {
                inner.shots += 1;
                Some((inner.shots, Arc::new(capture::screen_bmp(window))))
            }
            _ => None,
        };
        inner.shot = capture.as_ref().map(|(shot, _)| *shot);
        inner.shot_hash = capture.as_ref().map(|(_, bytes)| {
            let mut hasher = DefaultHasher::new();
            bytes.hash(&mut hasher);
            hasher.finish()
        });
        inner.shot_at = Instant::now();
        *self.capture.lock().unwrap() = capture;
    }

    fn backdrop_url(inner: &Inner) -> Option<String> {
        inner.shot.map(|shot| format!("http://backdrop.localhost/{shot}"))
    }

    /// The edge or the skin changed while the panel is up: puts the window
    /// where it now belongs and captures what is behind it afresh.
    pub fn relocate(&self, epoch: u64) {
        let is_current = |inner: &Inner| inner.epoch == epoch && matches!(inner.phase, Phase::Open { .. });
        let contact = {
            let inner = self.inner.lock().unwrap();
            let Some(placement) = inner.placement.as_ref().filter(|_| is_current(&inner)) else { return };
            Contact { monitor: placement.monitor, work: placement.work, scale: placement.scale }
        };
        // The window has to be off the screen for the capture not to contain
        // it. No lock is held meanwhile: the main thread may need one to get
        // there.
        self.hide_and_wait();

        let mut inner = self.inner.lock().unwrap();
        // The panel may have been dismissed while the window was going.
        let Some(surface) = inner.surface.clone().filter(|_| is_current(&inner)) else { return };
        let window = self.place(&mut inner, &surface, &contact);
        self.shoot(&mut inner, window);
        inner.panel = None;
        self.window().show().unwrap();
        self.app
            .emit("panel-backdrop", BackdropPayload { epoch, room: room(&surface, &contact, self.edge()), backdrop: Self::backdrop_url(&inner) })
            .unwrap();
    }

    /// The closing animation of opening `epoch` finished.
    pub fn hidden(&self, epoch: u64) {
        let mut inner = self.inner.lock().unwrap();
        if matches!(inner.phase, Phase::Closing) && inner.epoch == epoch {
            inner.phase = Phase::Hidden;
            inner.shot = None;
            *self.capture.lock().unwrap() = None;
            self.window().hide().unwrap();
            self.set_on_screen(false);
        }
    }

    /// Takes the panel off the screen at once, without its animation, and
    /// returns once it is gone from what the screen shows.
    pub fn dismiss(&self) {
        {
            let mut inner = self.inner.lock().unwrap();
            if matches!(inner.phase, Phase::Hidden) {
                return;
            }
            inner.phase = Phase::Hidden;
            inner.shot = None;
            *self.capture.lock().unwrap() = None;
            self.app.emit("panel-dismissed", inner.epoch).unwrap();
        }
        // No lock held: the main thread carries the hide out.
        self.hide_and_wait();
        self.set_on_screen(false);
    }

    /// Hides the window and waits until the screen no longer shows it.
    /// Hiding is carried out on the main thread some time after the request,
    /// and reaches the screen at the next composed frame.
    fn hide_and_wait(&self) {
        self.window().hide().unwrap();
        while unsafe { IsWindowVisible(self.hwnd()) }.as_bool() {
            std::thread::sleep(Duration::from_millis(1));
        }
        let _ = unsafe { DwmFlush() };
        let _ = unsafe { DwmFlush() };
    }

    pub fn close(&self) {
        let mut inner = self.inner.lock().unwrap();
        if matches!(inner.phase, Phase::Open { .. }) {
            self.begin_close(&mut inner);
        }
    }

    /// Tells the web view whether the panel is on screen. Off screen it gives
    /// back what it keeps for drawing (mostly the glass's textures in the GPU
    /// process, some 120 MB), which it otherwise holds on to.
    pub fn set_on_screen(&self, on_screen: bool) {
        let level = if on_screen {
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
        } else {
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
        };
        self.window()
            .with_webview(move |webview| unsafe {
                let core: ICoreWebView2_19 = webview.controller().CoreWebView2().unwrap().cast().unwrap();
                core.SetMemoryUsageTargetLevel(level).unwrap();
            })
            .unwrap();
    }

    fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut _)
    }

    fn window(&self) -> tauri::WebviewWindow {
        self.app.get_webview_window("panel").unwrap()
    }

    /// Opens the panel from the tray icon, on the monitor the pointer is on.
    pub fn open_from_tray(&self) {
        let cursor = cursor_position();
        if let Some(contact) = monitor_at(cursor) {
            self.open(cursor, &contact);
        }
    }

    fn open(&self, cursor: POINT, contact: &Contact) {
        let mut inner = self.inner.lock().unwrap();
        let Some(surface) = inner.surface.clone() else { return };
        if matches!(inner.phase, Phase::Open { .. }) {
            return;
        }
        let reopening = matches!(inner.phase, Phase::Closing);
        inner.epoch += 1;
        inner.phase = Phase::Open { entered: false, outside_since: None, dragging: false };
        inner.panel = None;
        inner.click_through = false;

        // A reopen during the slide-out keeps the window, and the capture
        // behind it, where they are.
        if !reopening {
            let window = self.place(&mut inner, &surface, contact);
            self.shoot(&mut inner, window);
        }

        self.window().set_ignore_cursor_events(false).unwrap();
        self.set_on_screen(true);
        self.window().show().unwrap();
        let window = inner.placement.as_ref().unwrap().window;
        let epoch = inner.epoch;
        self.app
            .emit(
                "panel-open",
                OpenPayload {
                    epoch,
                    history: &inner.history,
                    room: room(&surface, contact, self.edge()),
                    focus_x: cursor.x - window.left,
                    focus_y: cursor.y - window.top,
                    backdrop: Self::backdrop_url(&inner),
                },
            )
            .unwrap();
    }

    /// Sizes and positions the (hidden) window for `contact`'s monitor.
    fn place(&self, inner: &mut Inner, surface: &Surface, contact: &Contact) -> RECT {
        let (window, zoom) = geometry(surface, contact, self.edge());
        let (left, width) = (window.left, window.right - window.left);
        inner.placement = Some(Placement { window, monitor: contact.monitor, work: contact.work, scale: contact.scale, zoom });

        // Crossing to a monitor with another scale factor makes the windowing
        // layer rescale the window as it arrives. Let that happen on a move
        // alone, so the final rectangle is set on a window that already lives
        // on the target monitor.
        unsafe {
            SetWindowPos(self.hwnd(), None, left, window.top, 0, 0, SWP_NOACTIVATE | SWP_NOSIZE | SWP_NOZORDER)
        }
        .unwrap();
        unsafe {
            SetWindowPos(
                self.hwnd(),
                Some(HWND_TOPMOST),
                left,
                window.top,
                width,
                window.bottom - window.top,
                SWP_NOACTIVATE,
            )
        }
        .unwrap();
        self.window().set_zoom(zoom).unwrap();
        window
    }

    /// The panel changed size while on screen: keep it fitting the work area.
    fn fit(&self, inner: &mut Inner) {
        let (Some(surface), Some(placement)) = (inner.surface.clone(), inner.placement.as_mut()) else { return };
        let contact = Contact { monitor: placement.monitor, work: placement.work, scale: placement.scale };
        let (window, zoom) = geometry(&surface, &contact, self.edge());
        if zoom == placement.zoom && window == placement.window {
            return;
        }
        // A capture covers the window as it was; a larger window would show
        // past it. Only the zoom changes then, and only to shrink.
        let w = placement.window;
        let grows = window.right - window.left > w.right - w.left || window.bottom - window.top > w.bottom - w.top;
        if inner.shot.is_some() && grows {
            return;
        }
        placement.zoom = zoom;
        if inner.shot.is_none() {
            placement.window = window;
            let w = window;
            unsafe {
                SetWindowPos(self.hwnd(), None, w.left, w.top, w.right - w.left, w.bottom - w.top, SWP_NOACTIVATE | SWP_NOZORDER)
            }
            .unwrap();
        }
        self.window().set_zoom(zoom).unwrap();
    }

    fn begin_close(&self, inner: &mut Inner) {
        inner.phase = Phase::Closing;
        inner.click_through = true;
        self.window().set_ignore_cursor_events(true).unwrap();
        self.app.emit("panel-close", inner.epoch).unwrap();
    }

    /// Follows the pointer while the panel is open.
    fn track(&self, cursor: POINT, now: Instant) {
        let (close_delay, live) = {
            let config = self.config.lock().unwrap();
            (config.close_delay, config.live)
        };
        let mut inner = self.inner.lock().unwrap();
        // Until the page has placed the panel, the pointer counts as on it.
        let (on_panel, in_reach) = match inner.panel {
            Some(panel) => (contains(&panel, cursor), contains(&inner.reach, cursor)),
            None => (true, true),
        };
        let held = buttons_down();
        let Phase::Open { entered, outside_since, dragging } = &mut inner.phase else { return };

        // A press that begins away from the panel dismisses it; one that
        // begins on it keeps it open wherever the pointer then goes.
        let pressed_outside = held && !*dragging && !in_reach;
        *dragging = held && (*dragging || in_reach);
        *entered |= in_reach;
        if pressed_outside {
            self.begin_close(&mut inner);
            return;
        }
        if in_reach || *dragging || !*entered {
            *outside_since = None;
        } else if now.duration_since(*outside_since.get_or_insert(now)) >= close_delay {
            self.begin_close(&mut inner);
            return;
        }

        // Around the panel the window is only shadow or empty column; clicks
        // there belong to whatever is underneath.
        if inner.click_through == on_panel {
            inner.click_through = !on_panel;
            self.window().set_ignore_cursor_events(!on_panel).unwrap();
        }

        // A live backdrop is captured again every so often. The window is
        // out of captures then, so it can stay where it is.
        // Only a capture that differs from the last is passed on.
        if live && inner.shot.is_some() && now.duration_since(inner.shot_at) >= LIVE_INTERVAL {
            let window = inner.placement.as_ref().unwrap().window;
            let before = inner.shot_hash;
            self.shoot(&mut inner, window);
            if inner.shot_hash != before {
                let frame = BackdropFrame { epoch: inner.epoch, backdrop: Self::backdrop_url(&inner) };
                self.app.emit("backdrop-frame", frame).unwrap();
            }
        }
    }

    /// Runs the input loop. Raw input is read-only: unlike a mouse hook it
    /// cannot delay the pointer, and Windows never silently disconnects it.
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
        while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
            let now = Instant::now();
            let (edge, pressure) = {
                let config = self.config.lock().unwrap();
                (config.edge, config.pressure)
            };
            if self.is_shown() {
                armed = false;
            }
            match msg.message {
                WM_INPUT if !self.is_open() => {
                    raw_since_watch = true;
                    if let Some(motion) = read_motion(HRAWINPUT(msg.lParam.0 as *mut _), edge) {
                        let cursor = cursor_position();
                        match edge_contact(cursor, edge) {
                            Some(contact) if armed && detector.motion(motion, now, pressure) => {
                                detector.reset();
                                self.open(cursor, &contact);
                            }
                            Some(_) => {}
                            None => {
                                detector.reset();
                                armed = !self.is_shown();
                            }
                        }
                    }
                }
                WM_TIMER if msg.wParam.0 == WATCH_TIMER => {
                    let cursor = cursor_position();
                    let moved = cursor.x != last_cursor.x || cursor.y != last_cursor.y;
                    if moved && !raw_since_watch && !self.is_open() {
                        match edge_contact(cursor, edge) {
                            Some(_) if armed => {
                                detector.motion(Motion::Absolute, now, pressure);
                            }
                            Some(_) => {}
                            None => {
                                detector.reset();
                                armed = !self.is_shown();
                            }
                        }
                    }
                    last_cursor = cursor;
                    raw_since_watch = false;
                }
                WM_TIMER if self.is_open() => self.track(cursor_position(), now),
                WM_TIMER => {
                    let cursor = cursor_position();
                    match edge_contact(cursor, edge) {
                        Some(contact) if armed && detector.dwell_elapsed(now) => {
                            detector.reset();
                            self.open(cursor, &contact);
                        }
                        Some(_) => {}
                        None => {
                            detector.reset();
                            armed = !self.is_shown();
                        }
                    }
                }
                _ => {}
            }

            let needed = self.is_open() || detector.dwelling();
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

/// Share of the work area's height a panel opened from the top may take.
const TOP_SHARE: f64 = 0.6;

/// What a panel may take up on a work area, in logical px: its height, and
/// how many columns it may spread over. The page lays out by the same figures.
#[derive(Clone, Copy, Serialize)]
struct Room {
    height: f64,
    max_columns: f64,
}

fn room(surface: &Surface, contact: &Contact, edge: Edge) -> Room {
    let work_width = (contact.work.right - contact.work.left) as f64 / contact.scale;
    let work_height = (contact.work.bottom - contact.work.top) as f64 / contact.scale;
    match edge {
        Edge::Left | Edge::Right => Room { height: work_height, max_columns: MAX_COLUMNS },
        // Along the top edge the panel grows sideways: as many columns as the
        // width holds, and no taller than a share of the screen.
        Edge::Top => {
            let fit = ((work_width - 2.0 * GAP + surface.column_gap) / (surface.column_width + surface.column_gap)).floor();
            Room { height: work_height * TOP_SHARE, max_columns: fit.max(1.0) }
        }
    }
}

/// The window's rectangle (physical px) and the page zoom for a panel opened
/// on the monitor of `contact` from `edge`. The window is a strip along that
/// edge: as long as the work area, and as deep as the panel.
fn geometry(surface: &Surface, contact: &Contact, edge: Edge) -> (RECT, f64) {
    let (panel_width, panel_height, zoom) = fit_panel(surface, room(surface, contact, edge));
    let px = |logical: f64| (logical * contact.scale * zoom).round() as i32;
    let (monitor, work) = (contact.monitor, contact.work);
    let window = match edge {
        Edge::Left => {
            let width = px(panel_width + surface.margin + surface.inset);
            RECT { left: monitor.left, top: work.top, right: monitor.left + width, bottom: work.bottom }
        }
        Edge::Right => {
            let width = px(panel_width + surface.margin + surface.inset);
            RECT { left: monitor.right - width, top: work.top, right: monitor.right, bottom: work.bottom }
        }
        Edge::Top => {
            let height = px(panel_height + surface.margin + surface.inset);
            RECT { left: work.left, top: monitor.top, right: work.right, bottom: monitor.top + height }
        }
    };
    (window, zoom)
}

/// The work area of the monitor under the pointer, and its scale factor.
pub fn work_area_at_cursor() -> Option<(RECT, f64)> {
    let contact = monitor_at(cursor_position())?;
    Some((contact.work, contact.scale))
}

/// On a screen too short for the panel at full size, the page is zoomed out
/// until it fits, exactly as if the display had a lower scale factor.
/// Columns a panel may spread over before the window has to zoom out instead.
/// The page keeps the same limit.
const MAX_COLUMNS: f64 = 3.0;

/// The panel's size (logical px) and zoom in `room`:
/// as many columns as it takes to show the lanes at full size, and only if
/// even the most columns are not enough, zoomed out until they fit.
fn fit_panel(surface: &Surface, room: Room) -> (f64, f64, f64) {
    let space = room.height - 2.0 * GAP - surface.chrome_height;
    let most = (room.max_columns as usize).min(surface.lane_heights.len()).max(1);
    // The fewest columns whose tallest fits; failing that, the most.
    let (columns, tallest) = (1..=most)
        .map(|columns| (columns, tallest_column(&surface.lane_heights, columns, surface.lane_gap)))
        .find(|&(columns, tallest)| tallest <= space || columns == most)
        .unwrap();
    let columns = columns as f64;
    let width = columns * surface.column_width + (columns - 1.0) * surface.column_gap;
    let height = tallest + surface.chrome_height;
    let zoom = ((room.height - 2.0 * GAP) / height).min(1.0);
    (width, height, zoom)
}

/// The height of the tallest column when `heights`, in order, are split into
/// `columns` columns so that the tallest is as short as it can be. Lanes of
/// height zero are hidden and take no gap. The page splits by the same rule.
fn tallest_column(heights: &[f64], columns: usize, gap: f64) -> f64 {
    let span = |from: usize, to: usize| {
        let shown: Vec<f64> = heights[from..to].iter().copied().filter(|&h| h > 0.0).collect();
        shown.iter().sum::<f64>() + shown.len().saturating_sub(1) as f64 * gap
    };
    let n = heights.len();
    // best[k][i]: the tallest column splitting the first i lanes into k columns.
    let mut best = vec![vec![f64::INFINITY; n + 1]; columns + 1];
    best[0][0] = 0.0;
    for k in 1..=columns {
        for i in k..=n {
            for j in (k - 1)..i {
                let candidate = best[k - 1][j].max(span(j, i));
                if candidate < best[k][i] {
                    best[k][i] = candidate;
                }
            }
        }
    }
    best[columns][n]
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
    Some(Contact { monitor: info.rcMonitor, work: info.rcWork, scale: dpi as f64 / 96.0 })
}

/// Describes the monitor under the cursor if the cursor is pressed against its
/// trigger edge with intent to point: visible, no button held, clear of the
/// corners, and with no other monitor continuing past that edge.
fn edge_contact(cursor: POINT, edge: Edge) -> Option<Contact> {
    let contact = monitor_at(cursor)?;
    let monitor = contact.monitor;
    let corner = (CORNER_EXCLUSION * contact.scale).round() as i32;
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

#[cfg(test)]
mod tests {
    use super::tallest_column;

    #[test]
    fn splits_lanes_into_the_shortest_columns() {
        let heights = [100.0, 200.0, 50.0, 0.0, 150.0];
        assert_eq!(tallest_column(&heights, 1, 10.0), 100.0 + 200.0 + 50.0 + 150.0 + 3.0 * 10.0);
        // [100, 200] | [50, 0, 150]: 310 and 210.
        assert_eq!(tallest_column(&heights, 2, 10.0), 310.0);
        // [100] | [200] | [50, 0, 150] would be 210; [100, 200] stays 310; best is 210.
        assert_eq!(tallest_column(&heights, 3, 10.0), 210.0);
    }
}
