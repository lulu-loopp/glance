//! The screens as Windows has them arranged: where each is, a name that
//! stays its own (the display's and the output it is plugged into;
//! `\\.\DISPLAY1` and its like are dealt anew as displays come and go),
//! and where the pointer passes from one to another.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW, DISPLAY_DEVICEW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW};

use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

use crate::settings::{Edge, Lit, Places, EDGES};

/// The first Windows that eases the pointer between displays (see `eased`).
const FIRST_EASING_BUILD: u32 = 22557;
/// How long what the system says of it is taken to stand.
const EASED_FOR: Duration = Duration::from_secs(1);

/// EDD_GET_DEVICE_INTERFACE_NAME: the display's device interface path,
/// rather than its hardware identifiers alone.
const INTERFACE_NAME: u32 = 1;
/// MONITORINFOF_PRIMARY.
const PRIMARY: u32 = 1;

#[derive(Clone, PartialEq, Debug)]
pub struct Screen {
    /// The name that stays its own, no other screen there is named so (see `told_apart`).
    pub id: String,
    /// The output it is shown on (`\\.\DISPLAY1` and its like), and the display Windows names on that output: its
    /// device interface path, or the output's own name for an output with no display device behind it (a remote
    /// session's).
    pub output: String,
    pub display: String,
    /// Where it is on the desktop (physical px).
    pub monitor: RECT,
    pub primary: bool,
}

fn text(wide: &[u16]) -> String {
    String::from_utf16_lossy(&wide[..wide.iter().position(|c| *c == 0).unwrap_or(wide.len())])
}

/// Where each screen of the desktop is (physical px), and no more: quick, where `all`, which names each by its
/// display, takes a millisecond or two.
pub fn monitors() -> Vec<RECT> {
    unsafe extern "system" fn each(_: HMONITOR, _: HDC, at: *mut RECT, found: LPARAM) -> BOOL {
        unsafe { (*(found.0 as *mut Vec<RECT>)).push(*at) };
        true.into()
    }
    let mut found: Vec<RECT> = Vec::new();
    let _ = unsafe { EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut found as *mut _ as isize)) };
    found
}

/// Every screen of the desktop.
pub fn all() -> Vec<Screen> {
    unsafe extern "system" fn each(monitor: HMONITOR, _: HDC, _: *mut RECT, found: LPARAM) -> BOOL {
        let found = unsafe { &mut *(found.0 as *mut Vec<Screen>) };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if unsafe { GetMonitorInfoW(monitor, &mut info as *mut _ as *mut MONITORINFO) }.as_bool() {
            // The display on that output; an output with no display device
            // behind it (a remote session's) is known by its own name.
            let mut device = DISPLAY_DEVICEW { cb: size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
            let named = unsafe { EnumDisplayDevicesW(PCWSTR(info.szDevice.as_ptr()), 0, &mut device, INTERFACE_NAME) }.as_bool();
            let output = text(&info.szDevice);
            let display = Some(text(&device.DeviceID)).filter(|id| named && !id.is_empty()).unwrap_or_else(|| output.clone());
            found.push(Screen { id: display.clone(), output, display, monitor: info.monitorInfo.rcMonitor, primary: info.monitorInfo.dwFlags & PRIMARY != 0 });
        }
        true.into()
    }
    let mut found: Vec<Screen> = Vec::new();
    let _ = unsafe { EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut found as *mut _ as isize)) };
    told_apart(found)
}

/// `found`, each named by its display (see `Screen::display`), each given a name no other of them has. A device
/// interface path is the display device's instance in the system (which the system gives no other device: a display's
/// holds the connector it is on, so two alike on two connectors differ) joined to the interface, and stays the same
/// for that display on that connector from one start to the next. Windows lists the displays it has known on an
/// output, though, without promising that the first it lists is the one shown there: two outputs said to show the
/// same display are told apart by their outputs, the first of them by its output's name keeping the display's name
/// alone (what was kept under it stays that screen's) and each other named by it and its output.
fn told_apart(mut found: Vec<Screen>) -> Vec<Screen> {
    let ids: Vec<String> = found
        .iter()
        .map(|screen| {
            let first = found.iter().filter(|other| other.display == screen.display).map(|other| &other.output).min();
            if first == Some(&screen.output) { screen.display.clone() } else { format!("{} on {}", screen.display, screen.output) }
        })
        .collect();
    for (screen, id) in found.iter_mut().zip(ids) {
        screen.id = id;
    }
    found
}

/// Each screen as Windows has it, for a report: the output it is on, the display named there, where it is (physical
/// px), whether it is the main screen, and the name it goes by where that is not the display's alone.
pub fn describe() -> Vec<String> {
    described(&all())
}

/// `describe`, of `screens`.
fn described(screens: &[Screen]) -> Vec<String> {
    screens
        .iter()
        .map(|screen| {
            let m = screen.monitor;
            let main = if screen.primary { ", main" } else { "" };
            let named = if screen.id == screen.display { String::new() } else { format!(", named {}", screen.id) };
            format!("Screen on {}: {} at ({}, {}) to ({}, {}){main}{named}", screen.output, screen.display, m.left, m.top, m.right, m.bottom)
        })
        .collect()
}

/// The screens as they were last asked for, and each one's edges (left,
/// top, right: see `stretches`) for the pointer eased between displays or
/// not, as it was then.
struct Known {
    screens: Vec<Screen>,
    eased: bool,
    edges: Vec<[Vec<(i32, i32, bool)>; 3]>,
}

impl Known {
    const fn none() -> Known {
        Known { screens: Vec::new(), eased: false, edges: Vec::new() }
    }

    /// Takes `screens`, and works their edges out.
    fn take(&mut self, screens: Vec<Screen>, eased: bool) {
        let monitors: Vec<RECT> = screens.iter().map(|screen| screen.monitor).collect();
        self.edges = (0..monitors.len()).map(|index| EDGES.map(|edge| stretches(&monitors, index, edge, eased))).collect();
        self.screens = screens;
        self.eased = eased;
    }

    /// Where the screen at `monitor` is among them: asked for anew if it
    /// is not known (the screens are not as they were), or if the pointer
    /// is no longer eased as it was.
    fn find(&mut self, monitor: RECT, eased: bool) -> Option<usize> {
        let known = self.screens.iter().position(|screen| screen.monitor == monitor);
        if known.is_none() {
            self.take(all(), eased);
        } else if self.eased != eased {
            let screens = std::mem::take(&mut self.screens);
            self.take(screens, eased);
        }
        self.screens.iter().position(|screen| screen.monitor == monitor)
    }
}

thread_local! {
    static KNOWN: RefCell<Known> = const { RefCell::new(Known::none()) };
    /// Whether the pointer is eased between displays, and when that was
    /// last asked.
    static EASED: RefCell<Option<(Instant, bool)>> = const { RefCell::new(None) };
}

/// Whether Windows eases the pointer between displays ("Ease cursor
/// movement between displays", on unless turned off, from Windows 11
/// 22H2): pushed past an edge another screen lies against, anywhere along
/// that edge, the pointer goes on to that screen, and stops nowhere on it.
pub fn eased() -> bool {
    EASED.with(|eased| {
        let mut eased = eased.borrow_mut();
        if let Some((_, is)) = eased.filter(|(at, _)| at.elapsed() < EASED_FOR) {
            return is;
        }
        let build: u32 = crate::metrics::reg_string(w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"), w!("CurrentBuild")).parse().unwrap_or(0);
        let (mut set, mut size) = (1u32, size_of::<u32>() as u32);
        // Not there: as the system has it, on.
        let _ = unsafe { RegGetValueW(HKEY_CURRENT_USER, w!(r"Control Panel\Cursors"), w!("CursorDeadzoneJumpingSetting"), RRF_RT_REG_DWORD, None, Some(&mut set as *mut _ as *mut _), Some(&mut size)) };
        let is = build >= FIRST_EASING_BUILD && set != 0;
        *eased = Some((Instant::now(), is));
        is
    })
}

/// Whether the pointer passes on to another screen from `edge` of the
/// screen at `monitor`, `along` it (physical px): not, where it stops (and
/// for no screen of the desktop).
pub fn passes(monitor: RECT, edge: Edge, along: i32) -> bool {
    let eased = eased();
    KNOWN.with(|known| {
        let mut known = known.borrow_mut();
        let Some(index) = known.find(monitor, eased) else { return false };
        let at = EDGES.iter().position(|e| *e == edge).unwrap();
        known.edges[index][at].iter().any(|(from, to, seam)| *seam && (*from..*to).contains(&along))
    })
}

/// The screens changed: they are asked for anew.
pub fn changed() {
    KNOWN.with(|known| *known.borrow_mut() = Known::none());
}

/// The edges of the screen at `monitor` that open a panel (see `Places::lit`); none does, for no screen of the
/// desktop.
pub fn lit(places: &Places, monitor: RECT) -> Lit {
    id_of(monitor).map(|id| places.lit(&id)).unwrap_or_default()
}

/// The name that stays the screen's own (see `Screen::id`) of the screen at `monitor`; none, for no screen of the
/// desktop.
pub fn id_of(monitor: RECT) -> Option<String> {
    let eased = eased();
    KNOWN.with(|known| {
        let mut known = known.borrow_mut();
        known.find(monitor, eased).map(|index| known.screens[index].id.clone())
    })
}

/// `edge` of the screen `monitors[index]` from one end to the other, in
/// stretches (from, to: physical px along it): each with whether the
/// pointer passes on to another screen there (a seam). It does where
/// another screen lies against the edge; and, `eased` (see `eased`), all
/// along an edge another screen lies against anywhere.
pub fn stretches(monitors: &[RECT], index: usize, edge: Edge, eased: bool) -> Vec<(i32, i32, bool)> {
    let own = monitors[index];
    let (line, from, to) = match edge {
        Edge::Left => (own.left, own.top, own.bottom),
        Edge::Right => (own.right, own.top, own.bottom),
        Edge::Top => (own.top, own.left, own.right),
    };
    let mut seams: Vec<(i32, i32)> = monitors
        .iter()
        .enumerate()
        .filter(|(other, _)| *other != index)
        .filter_map(|(_, other)| {
            let (facing, a, b) = match edge {
                Edge::Left => (other.right, other.top, other.bottom),
                Edge::Right => (other.left, other.top, other.bottom),
                Edge::Top => (other.bottom, other.left, other.right),
            };
            let (a, b) = (a.max(from), b.min(to));
            (facing == line && b > a).then_some((a, b))
        })
        .collect();
    seams.sort();
    if eased && !seams.is_empty() {
        return vec![(from, to, true)];
    }
    let mut parts = Vec::new();
    let mut at = from;
    for (a, b) in seams {
        if a > at {
            parts.push((at, a, false));
        }
        parts.push((a, b, true));
        at = b;
    }
    if at < to {
        parts.push((at, to, false));
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two outputs said to show one display (two alike screens, as a report had them) are two screens of their own
    /// names: the first by its output's name keeps the display's (what was kept under it stays its), the other is named
    /// by its output as well, whichever order Windows lists them in. A screen whose display is its own alone keeps its
    /// name as it was.
    #[test]
    fn screens_said_to_show_one_display_are_told_apart() {
        let shared = r"\\?\DISPLAY#XMI3444#5&1a2b3c4d&0&UID4352#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}";
        let own = r"\\?\DISPLAY#GSM77E8#5&1a2b3c4d&0&UID4353#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}";
        let screen = |output: &str, display: &str, monitor: RECT, primary: bool| Screen { id: display.into(), output: output.into(), display: display.into(), monitor, primary };
        let wide = screen(r"\\.\DISPLAY1", shared, RECT { left: 0, top: 0, right: 3840, bottom: 2160 }, true);
        let tall = screen(r"\\.\DISPLAY2", shared, RECT { left: -2160, top: -1071, right: 0, bottom: 2769 }, false);
        let third = screen(r"\\.\DISPLAY3", own, RECT { left: 3840, top: 0, right: 5760, bottom: 1080 }, false);
        let names = |found: Vec<Screen>| told_apart(found).into_iter().map(|screen| (screen.output, screen.id)).collect::<Vec<_>>();
        let told = names(vec![wide.clone(), tall.clone(), third.clone()]);
        assert_eq!(told, [(r"\\.\DISPLAY1".to_string(), shared.to_string()), (r"\\.\DISPLAY2".into(), format!(r"{shared} on \\.\DISPLAY2")), (r"\\.\DISPLAY3".into(), own.into())]);
        let mut listed = names(vec![third, tall, wide]);
        listed.reverse();
        assert_eq!(listed, told);
        // (A report says what Windows named on each output, and the name each screen goes by where that differs.)
        let shown = described(&told_apart(vec![screen(r"\\.\DISPLAY1", shared, RECT { left: 0, top: 0, right: 3840, bottom: 2160 }, true), screen(r"\\.\DISPLAY2", shared, RECT { left: -2160, top: -1071, right: 0, bottom: 2769 }, false)]));
        assert_eq!(shown, [format!(r"Screen on \\.\DISPLAY1: {shared} at (0, 0) to (3840, 2160), main"), format!(r"Screen on \\.\DISPLAY2: {shared} at (-2160, -1071) to (0, 2769), named {shared} on \\.\DISPLAY2")]);
    }

    #[test]
    fn an_edge_is_a_seam_where_another_screen_lies_against_it() {
        // A tall screen on the left, a wide one above a small one beside it.
        let tall = RECT { left: -2889, top: -1459, right: -729, bottom: 2381 };
        let wide = RECT { left: -729, top: -1440, right: 2711, bottom: 0 };
        let small = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
        let screens = [tall, wide, small];
        assert_eq!(stretches(&screens, 0, Edge::Right, false), [(-1459, -1440, false), (-1440, 0, true), (0, 2381, false)]);
        assert_eq!(stretches(&screens, 0, Edge::Left, false), [(-1459, 2381, false)]);
        assert_eq!(stretches(&screens, 1, Edge::Left, false), [(-1440, 0, true)]);
        assert_eq!(stretches(&screens, 1, Edge::Right, false), [(-1440, 0, false)]);
        assert_eq!(stretches(&screens, 2, Edge::Top, false), [(0, 1920, true)]);
        assert_eq!(stretches(&screens, 2, Edge::Left, false), [(0, 1080, false)]);
        // Eased between displays, the pointer passes all along an edge
        // another screen lies against (as measured on these three: pushed
        // right off the tall one beside or below the small one, it goes on
        // to the wide one); an edge none lies against stops it as before
        // (the small one's left, the tall one beyond a gap).
        assert_eq!(stretches(&screens, 0, Edge::Right, true), [(-1459, 2381, true)]);
        assert_eq!(stretches(&screens, 0, Edge::Left, true), [(-1459, 2381, false)]);
        assert_eq!(stretches(&screens, 2, Edge::Left, true), [(0, 1080, false)]);
    }
}
