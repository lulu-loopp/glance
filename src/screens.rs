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
/// DISPLAY_DEVICE_ACTIVE and DISPLAY_DEVICE_ATTACHED: of a display listed on an output, that the output shows it.
const SHOWN: u32 = 0x1 | 0x2;

#[derive(Clone, PartialEq, Debug)]
pub struct Screen {
    /// The name that stays its own, no other screen there is named so (see `named`).
    pub id: String,
    /// The output it is shown on (`\\.\DISPLAY1` and its like), and the displays Windows lists on that output, in
    /// its order: each one's device interface path and state (`DISPLAY_DEVICEW::StateFlags`).
    pub output: String,
    pub listed: Vec<(String, u32)>,
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
            // The displays Windows lists on that output (none behind an output of a remote session's).
            let mut listed = Vec::new();
            for at in 0.. {
                let mut device = DISPLAY_DEVICEW { cb: size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
                if !unsafe { EnumDisplayDevicesW(PCWSTR(info.szDevice.as_ptr()), at, &mut device, INTERFACE_NAME) }.as_bool() {
                    break;
                }
                let path = text(&device.DeviceID);
                if !path.is_empty() {
                    listed.push((path, device.StateFlags.0));
                }
            }
            let output = text(&info.szDevice);
            found.push(Screen { id: named(&output, &listed), output, listed, monitor: info.monitorInfo.rcMonitor, primary: info.monitorInfo.dwFlags & PRIMARY != 0 });
        }
        true.into()
    }
    let mut found: Vec<Screen> = Vec::new();
    let _ = unsafe { EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut found as *mut _ as isize)) };
    found
}

/// The name that stays the screen on `output` its own, from the displays Windows lists there (see `Screen::listed`):
/// the device interface path of the one it says the output shows (active and attached). A device interface path is the
/// display device's instance in the system, which the system gives no other device (a display's holds the connector
/// it is on, so two alike on two connectors differ), joined to the interface, and it stays the same for that display on
/// that connector from one start to the next. An output shows one display and a display is shown by one output, so no
/// two screens are named alike, whichever others there are. Windows lists every display it has known on an output, the
/// one shown there not always first (the first was taken before, and two outputs were seen to name one display). Where
/// none listed is said to be shown, the first is taken with the output's name, which no other screen has, and an output
/// with none listed (a remote session's) is known by its own name.
fn named(output: &str, listed: &[(String, u32)]) -> String {
    match listed.iter().find(|(_, state)| state & SHOWN == SHOWN) {
        Some((path, _)) => path.clone(),
        None => match listed.first() {
            Some((path, _)) => format!("{path} on {output}"),
            None => output.to_string(),
        },
    }
}

/// Each screen as Windows has it, for a report: the output it is on, the name it goes by, where it is (physical px),
/// whether it is the main screen, and each display listed on its output with its state.
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
            let listed: Vec<String> = screen.listed.iter().map(|(path, state)| format!("{path} (state {state:#x})")).collect();
            format!("Screen on {}: {} at ({}, {}) to ({}, {}){main}, listed: {}", screen.output, screen.id, m.left, m.top, m.right, m.bottom, listed.join(", "))
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

    /// A screen is named by the display its output is said to show, whichever is listed first and whichever other
    /// screens there are: two alike screens whose outputs both list one display first (as a report seemed to have
    /// them) are two names, each its own shown display's, the same with the other screen gone. One whose output lists
    /// the display it shows first keeps the name it had. One whose output says it shows none of those it lists is named
    /// by the first and its output, and one with none listed by its output.
    #[test]
    fn a_screen_is_named_by_the_display_its_output_shows() {
        let path = |model: &str, connector: u32| format!(r"\?\DISPLAY#{model}#5&1a2b3c4d&0&UID{connector}#{{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}}");
        let (wide, tall) = (path("XMI3444", 4352), path("XMI3444", 4353));
        let (known, shown) = (0x0, SHOWN);
        // (Each output's list as Windows has it, the one shown on the second listed after the first's.)
        let first = vec![(wide.clone(), shown)];
        let second = vec![(wide.clone(), known), (tall.clone(), shown)];
        assert_eq!(named(r"\.\DISPLAY1", &first), wide);
        assert_eq!(named(r"\.\DISPLAY2", &second), tall);
        assert_eq!(named(r"\.\DISPLAY1", &[(path("GSM77E8", 4355), shown), (wide.clone(), known)]), path("GSM77E8", 4355));
        assert_eq!(named(r"\.\DISPLAY3", &[(wide.clone(), known)]), format!(r"{wide} on \.\DISPLAY3"));
        assert_eq!(named(r"\.\DISPLAY4", &[]), r"\.\DISPLAY4");
        // (A report says what each output lists, and the name its screen goes by.)
        let screen = Screen { id: named(r"\.\DISPLAY2", &second), output: r"\.\DISPLAY2".into(), listed: second, monitor: RECT { left: -2160, top: -1071, right: 0, bottom: 2769 }, primary: false };
        assert_eq!(described(&[screen]), [format!(r"Screen on \.\DISPLAY2: {tall} at (-2160, -1071) to (0, 2769), listed: {wide} (state 0x0), {tall} (state 0x3)")]);
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
