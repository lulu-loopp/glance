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

use crate::settings::{Edge, Opens, Rest, ScreenEdges};

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
    pub id: String,
    /// Where it is on the desktop (physical px).
    pub monitor: RECT,
    pub primary: bool,
}

fn text(wide: &[u16]) -> String {
    String::from_utf16_lossy(&wide[..wide.iter().position(|c| *c == 0).unwrap_or(wide.len())])
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
            let id = Some(text(&device.DeviceID)).filter(|id| named && !id.is_empty()).unwrap_or_else(|| text(&info.szDevice));
            found.push(Screen { id, monitor: info.monitorInfo.rcMonitor, primary: info.monitorInfo.dwFlags & PRIMARY != 0 });
        }
        true.into()
    }
    let mut found: Vec<Screen> = Vec::new();
    let _ = unsafe { EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut found as *mut _ as isize)) };
    found
}

thread_local! {
    /// The screens as they were last asked for on this thread.
    static KNOWN: RefCell<Vec<Screen>> = const { RefCell::new(Vec::new()) };
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
        if !known.iter().any(|screen| screen.monitor == monitor) {
            *known = all();
        }
        let monitors: Vec<RECT> = known.iter().map(|screen| screen.monitor).collect();
        let Some(index) = monitors.iter().position(|m| *m == monitor) else { return false };
        stretches(&monitors, index, edge, eased).into_iter().any(|(from, to, seam)| seam && (from..to).contains(&along))
    })
}

/// The screens changed: they are asked for anew.
pub fn changed() {
    KNOWN.with(|known| known.borrow_mut().clear());
}

/// How the edges of the screen at `monitor` open the panel (see
/// `settings::lit`); none does, for no screen of the desktop.
pub fn lit(kept: &[ScreenEdges], rest: Rest, monitor: RECT) -> Opens {
    KNOWN.with(|known| {
        let mut known = known.borrow_mut();
        // One not known: the screens are not as they were.
        if !known.iter().any(|screen| screen.monitor == monitor) {
            *known = all();
        }
        known.iter().find(|screen| screen.monitor == monitor).map(|screen| crate::settings::lit(kept, rest, &screen.id)).unwrap_or_default()
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
