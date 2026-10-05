//! Shows and hides the panel window.
//!
//! The window is a column along the trigger edge, as tall as the work area,
//! and stays put while it is on screen; the page lays the panel out inside it
//! and animates it. Whether the pointer is "on the panel" is decided here from
//! the global cursor position against the rectangle the page reports, so
//! nothing that moves on screen can feed back into that decision.

use std::collections::VecDeque;
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
    SetWindowPos, CURSORINFO, CURSOR_SHOWING, HWND_MESSAGE, HWND_TOPMOST, MSG, SWP_NOACTIVATE,
    SWP_NOSIZE, SWP_NOZORDER, WINDOW_EX_STYLE, WINDOW_STYLE, WM_INPUT, WM_TIMER,
};

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
#[derive(Clone, Copy, Deserialize)]
pub struct Surface {
    /// The lanes' height laid out in one column, and the height of what is
    /// not lanes (the bar), at zoom 1 (logical px). How many columns the
    /// lanes need depends on the screen, which only this side knows before
    /// the panel opens; the page splits them by the same rule.
    lanes_height: f64,
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
    /// The number the current capture is served under, if there is one.
    shot: Option<u64>,
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
    /// Height of the work area the panel has to fit in (logical px).
    room: f64,
    /// Pointer height relative to the window (physical px).
    focus_y: i32,
    /// Where to fetch the capture of the screen behind the window.
    backdrop: Option<String>,
}

#[derive(Clone, Serialize)]
struct BackdropPayload {
    epoch: u64,
    room: f64,
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
                shots: 0,
            }),
        }
    }

    pub fn apply(&self, settings: &Settings) {
        *self.config.lock().unwrap() = config_from(settings);
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
        let capture = match inner.surface {
            Some(surface) if surface.backdrop == Backdrop::Snapshot => {
                inner.shots += 1;
                Some((inner.shots, Arc::new(capture::screen_bmp(window))))
            }
            _ => None,
        };
        inner.shot = capture.as_ref().map(|(shot, _)| *shot);
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
        let Some(surface) = inner.surface.filter(|_| is_current(&inner)) else { return };
        let window = self.place(&mut inner, surface, &contact);
        self.shoot(&mut inner, window);
        inner.panel = None;
        self.window().show().unwrap();
        self.app
            .emit("panel-backdrop", BackdropPayload { epoch, room: room(&contact), backdrop: Self::backdrop_url(&inner) })
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
        let Some(surface) = inner.surface else { return };
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
            let window = self.place(&mut inner, surface, contact);
            self.shoot(&mut inner, window);
        }

        self.window().set_ignore_cursor_events(false).unwrap();
        self.window().show().unwrap();
        let window = inner.placement.as_ref().unwrap().window;
        let epoch = inner.epoch;
        self.app
            .emit(
                "panel-open",
                OpenPayload {
                    epoch,
                    history: &inner.history,
                    room: room(contact),
                    focus_y: cursor.y - window.top,
                    backdrop: Self::backdrop_url(&inner),
                },
            )
            .unwrap();
    }

    /// Sizes and positions the (hidden) window for `contact`'s monitor.
    fn place(&self, inner: &mut Inner, surface: Surface, contact: &Contact) -> RECT {
        let work_height = (contact.work.bottom - contact.work.top) as f64 / contact.scale;
        let (panel_width, _, zoom) = fit_panel(surface, work_height);
        let width = window_width(surface, panel_width, contact.scale, zoom);
        let left = match self.edge() {
            Edge::Left => contact.monitor.left,
            Edge::Right => contact.monitor.right - width,
        };
        let window = RECT { left, top: contact.work.top, right: left + width, bottom: contact.work.bottom };
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
        let (Some(surface), Some(placement)) = (inner.surface, inner.placement.as_mut()) else { return };
        let work_height = (placement.window.bottom - placement.window.top) as f64 / placement.scale;
        let (panel_width, _, zoom) = fit_panel(surface, work_height);
        let width = window_width(surface, panel_width, placement.scale, zoom);
        if zoom == placement.zoom && width == placement.window.right - placement.window.left {
            return;
        }
        // A capture covers the window as it was; a wider window would show
        // past it. Only the zoom changes then, and only to shrink.
        if inner.shot.is_some() && width > placement.window.right - placement.window.left {
            return;
        }
        placement.zoom = zoom;
        if inner.shot.is_none() {
            let left = match self.edge() {
                Edge::Left => placement.monitor.left,
                Edge::Right => placement.monitor.right - width,
            };
            placement.window.left = left;
            placement.window.right = left + width;
            let w = placement.window;
            unsafe {
                SetWindowPos(self.hwnd(), None, w.left, w.top, width, w.bottom - w.top, SWP_NOACTIVATE | SWP_NOZORDER)
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
        let close_delay = self.config.lock().unwrap().close_delay;
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
        let mut ticking = false;
        let mut msg = MSG::default();
        while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
            let now = Instant::now();
            let (edge, pressure) = {
                let config = self.config.lock().unwrap();
                (config.edge, config.pressure)
            };
            match msg.message {
                WM_INPUT if !self.is_open() => {
                    if let Some(motion) = read_motion(HRAWINPUT(msg.lParam.0 as *mut _), edge) {
                        let cursor = cursor_position();
                        match edge_contact(cursor, edge) {
                            Some(contact) if detector.motion(motion, now, pressure) => {
                                detector.reset();
                                self.open(cursor, &contact);
                            }
                            Some(_) => {}
                            None => detector.reset(),
                        }
                    }
                }
                WM_TIMER if self.is_open() => self.track(cursor_position(), now),
                WM_TIMER => {
                    let cursor = cursor_position();
                    match edge_contact(cursor, edge) {
                        Some(contact) if detector.dwell_elapsed(now) => {
                            detector.reset();
                            self.open(cursor, &contact);
                        }
                        Some(_) => {}
                        None => detector.reset(),
                    }
                }
                _ => {}
            }

            let needed = self.is_open() || detector.dwelling();
            if needed != ticking {
                ticking = needed;
                unsafe {
                    if needed {
                        SetTimer(Some(sink), 1, TICK_MS, None);
                    } else {
                        let _ = KillTimer(Some(sink), 1);
                    }
                }
            }
            unsafe { DispatchMessageW(&msg) };
        }
    }
}

fn room(contact: &Contact) -> f64 {
    (contact.work.bottom - contact.work.top) as f64 / contact.scale
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

/// The panel's size (logical px) and zoom on a work area `work_height` tall:
/// as many columns as it takes to show the lanes at full size, and only if
/// even the most columns are not enough, zoomed out until they fit.
fn fit_panel(surface: Surface, work_height: f64) -> (f64, f64, f64) {
    let space = work_height - 2.0 * GAP - surface.chrome_height;
    let columns = (surface.lanes_height / space).ceil().clamp(1.0, MAX_COLUMNS);
    let width = columns * surface.column_width + (columns - 1.0) * surface.column_gap;
    let height = surface.lanes_height / columns + surface.chrome_height;
    let zoom = ((work_height - 2.0 * GAP) / height).min(1.0);
    (width, height, zoom)
}

/// Window width (physical px) for a panel `width` wide.
fn window_width(surface: Surface, width: f64, scale: f64, zoom: f64) -> i32 {
    ((width + surface.margin + surface.inset) * scale * zoom).round() as i32
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
    };
    Some(Motion::Relative { outward, along: mouse.lLastY })
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
    let (on_edge, beyond) = match edge {
        Edge::Left => (cursor.x <= monitor.left, monitor.left - 1),
        Edge::Right => (cursor.x >= monitor.right - 1, monitor.right),
    };
    let corner = (CORNER_EXCLUSION * contact.scale).round() as i32;
    let clear_of_corners = (monitor.top + corner..monitor.bottom - corner).contains(&cursor.y);
    if !on_edge || !clear_of_corners || buttons_down() || !cursor_showing() {
        return None;
    }
    let neighbour = unsafe { MonitorFromPoint(POINT { x: beyond, y: cursor.y }, MONITOR_DEFAULTTONULL) };
    neighbour.is_invalid().then_some(contact)
}
