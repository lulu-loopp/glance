//! Where what is dragged from another program is caught: windows of the station's own thread (OLE gives a drop to the
//! thread a window is of, and the panel's and the widgets' thread has
//! none), put over each station shown (see `run::shows_at`) and along the screen edges that hold one, only while
//! something is dragged. Else there are none: they would take presses meant for what is under them (a maximized
//! window's scroll bar along the screen's edge).
//!
//! A drag is told from outside by the window that holds the pointer on the dragging program's thread: OLE's own
//! (`CLIPBRDWNDCLASS`), not a window moved or text selected.

use super::drop::{Dragged, Target};
use super::run::Lands;
use crate::settings::{Edge, Part};
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::Ole::{IDropTarget, RegisterDragDrop, RevokeDragDrop};
use windows::Win32::UI::WindowsAndMessaging::*;

/// How thick a strip along a screen's edge is (physical px).
const STRIP: i32 = 6;

/// A window catching drops: where it is (and the parts of that it takes: see `uncovered`), its window, what it is for
/// (an edge's strip opens the station there), where what is dropped on it goes, and the window it catches for (none:
/// an edge's strip).
struct Catcher {
    rect: RECT,
    parts: Vec<RECT>,
    window: HWND,
    edge: Option<Edge>,
    lands: Lands,
    over: Option<HWND>,
    _target: IDropTarget,
}

/// Where a catcher is wanted: as `Catcher` says, less its own window.
type Wanted = (RECT, Vec<RECT>, Option<Edge>, Lands, Option<HWND>);

/// The catchers up, while something is dragged.
#[derive(Default)]
pub struct Catchers {
    up: Vec<Catcher>,
}

/// Whether a program is dragging something with OLE now: its thread (the one in front's) holding the pointer with
/// OLE's window.
pub fn dragging() -> bool {
    unsafe {
        let front = GetForegroundWindow();
        let thread = GetWindowThreadProcessId(front, None);
        let mut info = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        if thread == 0 || GetGUIThreadInfo(thread, &mut info).is_err() || info.hwndCapture.is_invalid() {
            return false;
        }
        let mut class = [0u16; 32];
        let length = GetClassNameW(info.hwndCapture, &mut class) as usize;
        String::from_utf16_lossy(&class[..length]) == "CLIPBRDWNDCLASS"
    }
}

/// Where drops are caught now (and the parts of each): each station and quick launch shown, and where a station's page
/// is chosen, as much of it as no other window covers, and each edge of each screen that holds the workbench while it
/// shows a station, as a strip. The devices nearby on a station's strip last (see `send`): made last, they are over
/// the station they are on, and over a screen's edge where they reach it.
fn wanted() -> Vec<Wanted> {
    let mut wanted: Vec<Wanted> = super::run::areas()
        .into_iter()
        .filter_map(|(window, area, lands)| {
            let parts = uncovered(window, area);
            (!parts.is_empty()).then_some((area, parts, None, lands, Some(window)))
        })
        .collect();
    let (places, station) = {
        let settings = crate::app().settings.lock().unwrap();
        (settings.places().clone(), crate::ui::prefs::module_on_in(&settings.view, "workbench", "station"))
    };
    if station {
        wanted.extend(edges(&places));
    }
    devices_last(&mut wanted);
    wanted
}

/// What is wanted in the order the catchers are made, the devices nearby last (the rest as they were).
fn devices_last(wanted: &mut [Wanted]) {
    wanted.sort_by_key(|(.., lands, _)| matches!(lands, Lands::Device(_)));
}

/// A strip along each edge of each screen that holds the workbench (as `places` says).
fn edges(places: &crate::settings::Places) -> Vec<Wanted> {
    let mut strips = Vec::new();
    for monitor in crate::screens::monitors() {
        let Some(id) = crate::screens::id_of(monitor) else { continue };
        for edge in [Edge::Left, Edge::Right, Edge::Top] {
            if places.part(&id, edge) != Some(Part::Workbench) {
                continue;
            }
            let strip = match edge {
                Edge::Left => RECT { right: monitor.left + STRIP, ..monitor },
                Edge::Right => RECT { left: monitor.right - STRIP, ..monitor },
                Edge::Top => RECT { bottom: monitor.top + STRIP, ..monitor },
            };
            strips.push((strip, vec![strip], Some(edge), Lands::Station, None));
        }
    }
    strips
}

/// What of `area` (screen px) of `window` shows: it less each window over it that the pointer meets (shown, neither
/// minimized nor cloaked, not one the pointer passes through, not a catcher), as far as that is drawn. Where another
/// window covers a station, a drop is that window's.
fn uncovered(window: HWND, area: RECT) -> Vec<RECT> {
    let mut parts = vec![area];
    let mut above = unsafe { GetWindow(window, GW_HWNDPREV) }.ok();
    while let Some(over) = above.filter(|_| !parts.is_empty()) {
        if meets_the_pointer(over) {
            for drawn in drawn(over) {
                parts = without(&parts, drawn);
            }
        }
        above = unsafe { GetWindow(over, GW_HWNDPREV) }.ok();
    }
    parts
}

/// Whether the pointer meets `window` where it is: it shown, neither minimized nor cloaked, not one it passes through
/// (layered and see-through to it), and not a catcher.
fn meets_the_pointer(window: HWND) -> bool {
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    unsafe {
        if !IsWindowVisible(window).as_bool() || IsIconic(window).as_bool() {
            return false;
        }
        let mut cloaked = 0u32;
        if DwmGetWindowAttribute(window, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, 4).is_ok() && cloaked != 0 {
            return false;
        }
        let style = GetWindowLongPtrW(window, GWL_EXSTYLE) as u32;
        if style & WS_EX_LAYERED.0 != 0 && style & WS_EX_TRANSPARENT.0 != 0 {
            return false;
        }
        let mut class = [0u16; 32];
        let length = GetClassNameW(window, &mut class) as usize;
        String::from_utf16_lossy(&class[..length]) != "GlanceStationCatcher"
    }
}

/// Where `window` is drawn on the screen (its frame as drawn, not the borders it is sized by beyond it), in parts:
/// where it has a region of its own, what of its region lies within that alone (the pointer meets it nowhere else, and
/// the system draws it nowhere else: a look over a whole screen takes presses only where its sheet is).
fn drawn(window: HWND) -> Vec<RECT> {
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    use windows::Win32::Graphics::Gdi::{CreateRectRgn, DeleteObject, GetRegionData, GetWindowRgn, RGNDATA, RGNDATAHEADER, RGN_ERROR};
    let mut whole = RECT::default();
    if unsafe { GetWindowRect(window, &mut whole) }.is_err() {
        return Vec::new();
    }
    let mut frame = whole;
    if unsafe { DwmGetWindowAttribute(window, DWMWA_EXTENDED_FRAME_BOUNDS, &mut frame as *mut RECT as *mut _, std::mem::size_of::<RECT>() as u32) }.is_err() {
        frame = whole;
    }
    let region = unsafe { CreateRectRgn(0, 0, 0, 0) };
    if region.is_invalid() {
        return vec![frame];
    }
    // (None of its own: the whole of it. Its region is told from its window's corner, its borders and all.)
    let parts = if unsafe { GetWindowRgn(window, region) } == RGN_ERROR {
        vec![frame]
    } else {
        let size = unsafe { GetRegionData(region, 0, None) } as usize;
        let mut data = vec![0u64; size.div_ceil(8)];
        let read = size > 0 && unsafe { GetRegionData(region, size as u32, Some(data.as_mut_ptr().cast::<RGNDATA>())) } as usize == size;
        if read {
            let header = unsafe { &*data.as_ptr().cast::<RGNDATAHEADER>() };
            let rects = unsafe { std::slice::from_raw_parts(data.as_ptr().cast::<u8>().add(header.dwSize as usize).cast::<RECT>(), header.nCount as usize) };
            rects
                .iter()
                .map(|part| RECT { left: (part.left + whole.left).max(frame.left), top: (part.top + whole.top).max(frame.top), right: (part.right + whole.left).min(frame.right), bottom: (part.bottom + whole.top).min(frame.bottom) })
                .filter(|part| part.left < part.right && part.top < part.bottom)
                .collect()
        } else {
            vec![frame]
        }
    };
    let _ = unsafe { DeleteObject(region.into()) };
    parts
}

/// `parts` less `cut`: each part met by it as what of it is above, below, left and right of it.
fn without(parts: &[RECT], cut: RECT) -> Vec<RECT> {
    let mut left = Vec::new();
    for part in parts {
        let met = RECT { left: part.left.max(cut.left), top: part.top.max(cut.top), right: part.right.min(cut.right), bottom: part.bottom.min(cut.bottom) };
        if met.left >= met.right || met.top >= met.bottom {
            left.push(*part);
            continue;
        }
        let pieces = [
            RECT { bottom: met.top, ..*part },
            RECT { top: met.bottom, ..*part },
            RECT { top: met.top, bottom: met.bottom, right: met.left, ..*part },
            RECT { top: met.top, bottom: met.bottom, left: met.right, ..*part },
        ];
        left.extend(pieces.into_iter().filter(|piece| piece.left < piece.right && piece.top < piece.bottom));
    }
    left
}

/// A region of `parts` (screen px) in a window at `rect`: none where the system could not make it whole (out of room for
/// GDI's objects).
fn region_of(parts: &[RECT], rect: RECT) -> Option<windows::Win32::Graphics::Gdi::HRGN> {
    use windows::Win32::Graphics::Gdi::{CombineRgn, CreateRectRgn, DeleteObject, RGN_ERROR, RGN_OR};
    let region = unsafe { CreateRectRgn(0, 0, 0, 0) };
    if region.is_invalid() {
        return None;
    }
    for part in parts {
        let piece = unsafe { CreateRectRgn(part.left - rect.left, part.top - rect.top, part.right - rect.left, part.bottom - rect.top) };
        let joined = !piece.is_invalid() && unsafe { CombineRgn(Some(region), Some(region), Some(piece), RGN_OR) } != RGN_ERROR;
        if !piece.is_invalid() {
            let _ = unsafe { DeleteObject(piece.into()) };
        }
        if !joined {
            let _ = unsafe { DeleteObject(region.into()) };
            return None;
        }
    }
    Some(region)
}

unsafe extern "system" fn procedure(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

impl Catchers {
    /// Up where drops are wanted while something is dragged (by another program, or off the station: see
    /// `drag::under_way`); none else. What each sees goes to `run::caught`, or (a device nearby's) is sent there.
    pub fn follow(&mut self) {
        let dragged = dragging() || super::drag::under_way();
        // (No drag at all: none from another program over a station either, see `send::foreign`.)
        if !dragged {
            super::send::foreign(false);
        }
        let wanted = if dragged { wanted() } else { Vec::new() };
        let same = wanted.len() == self.up.len() && wanted.iter().zip(&self.up).all(|((rect, parts, edge, lands, over), up)| *rect == up.rect && *parts == up.parts && *edge == up.edge && *lands == up.lands && *over == up.over);
        if same {
            keep_over(&self.up.iter().map(|catcher| (catcher.window, catcher.over)).collect::<Vec<_>>());
            return;
        }
        self.clear();
        for (rect, parts, edge, lands, over) in wanted {
            if let Some(catcher) = make(rect, parts, edge, lands, over) {
                self.up.push(catcher);
            }
        }
    }

    /// None up.
    pub fn clear(&mut self) {
        for catcher in self.up.drain(..) {
            unsafe {
                let _ = RevokeDragDrop(catcher.window);
                let _ = DestroyWindow(catcher.window);
            }
        }
    }
}

/// Catchers (each its window, and the one it catches for) kept over those they catch for: where one is not, the window
/// it catches for raised over it since it was made (as the panel is raised when it is shown again), they are all raised
/// again in their order, the last on top. (What of the panel other windows cover is left out of them already: see
/// `uncovered`.)
fn keep_over(catchers: &[(HWND, Option<HWND>)]) {
    if catchers.iter().all(|(window, over)| over.is_none_or(|over| above(*window, over))) {
        return;
    }
    for (window, _) in catchers {
        let _ = unsafe { SetWindowPos(*window, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE) };
    }
}

/// Whether `window` is over `other` on the screen (of the windows up front, nearer the front).
fn above(window: HWND, other: HWND) -> bool {
    let mut at = other;
    while let Ok(before) = unsafe { GetWindow(at, GW_HWNDPREV) } {
        if before == window {
            return true;
        }
        at = before;
    }
    false
}

/// A window catching drops over `rect` (screen px; along `edge`, where it is a strip there; for the window `over`), as
/// far as `parts` of it, above all, all but invisible (hit by the pointer: a window wholly clear is passed over),
/// telling `run::caught` what it sees.
fn make(rect: RECT, parts: Vec<RECT>, edge: Option<Edge>, lands: Lands, over: Option<HWND>) -> Option<Catcher> {
    unsafe {
        let class = WNDCLASSW { lpfnWndProc: Some(procedure), lpszClassName: w!("GlanceStationCatcher"), ..Default::default() };
        RegisterClassW(&class);
        let window = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_LAYERED,
            class.lpszClassName,
            w!(""),
            WS_POPUP,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            None,
            None,
            None,
            None,
        )
        .ok()?;
        let _ = SetLayeredWindowAttributes(window, COLORREF(0), 1, LWA_ALPHA);
        // (Only its parts: the rest of it covered, passed through to what covers it. Where it cannot be cut to them, it
        // is not up, and is made again as the drops wanted are next followed.)
        if parts != [rect] && !region_of(&parts, rect).is_some_and(|region| crate::ui::gfx::give_region(window, region, false)) {
            let _ = DestroyWindow(window);
            return None;
        }
        let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
        let _ = SetWindowPos(window, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE);
        let told = move |dragged: Dragged| super::run::caught(dragged, edge, lands, over);
        // (What is kept at hand may be given a file off the station; the station, none of its own again; a page other
        // than the station's, nothing, only told it comes; a device nearby, what is dragged off the station, sent.)
        let target = match lands {
            Lands::Apps | Lands::Send => Target::new(told),
            Lands::Folders => Target::folders(told),
            Lands::Pages(page) if page != crate::ui::view::Tab::default() => Target::refusing(told),
            Lands::Device(key) => super::send::ToDevice::over(key, rect),
            _ => Target::not_own(told),
        };
        if RegisterDragDrop(window, &target).is_err() {
            let _ = DestroyWindow(window);
            return None;
        }
        Some(Catcher { rect, parts, window, edge, lands, over, _target: target })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT { left, top, right, bottom }
    }

    fn area(parts: &[RECT]) -> i32 {
        parts.iter().map(|part| (part.right - part.left) * (part.bottom - part.top)).sum()
    }

    /// A window with a region of its own covers what of its region is on the screen, no more: the look is a window as
    /// large as its screen taking presses only where its sheet is, and taken as all of it, it covered a station's
    /// devices beside it, which then caught nothing dragged off the look. One with none covers all of it.
    #[test]
    fn a_window_drawn_in_part_covers_that_part_alone() {
        use windows::Win32::Graphics::Gdi::{CombineRgn, CreateRectRgn, SetWindowRgn, RGN_OR};
        let window = unsafe { CreateWindowExW(WS_EX_TOOLWINDOW, w!("STATIC"), w!(""), WS_POPUP, 100, 200, 800, 600, None, None, None, None) }.expect("a window");
        let sorted = |mut parts: Vec<RECT>| {
            parts.sort_by_key(|part| (part.top, part.left));
            parts
        };
        assert_eq!(sorted(drawn(window)), [rect(100, 200, 900, 800)]);
        unsafe {
            let _ = SetWindowRgn(window, Some(CreateRectRgn(10, 20, 110, 70)), false);
        }
        assert_eq!(sorted(drawn(window)), [rect(110, 220, 210, 270)]);
        // (In two pieces, apart: each, and nothing between them.)
        unsafe {
            let two = CreateRectRgn(0, 0, 50, 50);
            let _ = CombineRgn(Some(two), Some(two), Some(CreateRectRgn(300, 400, 350, 450)), RGN_OR);
            let _ = SetWindowRgn(window, Some(two), false);
        }
        assert_eq!(sorted(drawn(window)), [rect(100, 200, 150, 250), rect(400, 600, 450, 650)]);
        // (One drawn nowhere: nothing.)
        unsafe {
            let _ = SetWindowRgn(window, Some(CreateRectRgn(0, 0, 0, 0)), false);
        }
        assert_eq!(drawn(window), []);
        let _ = unsafe { DestroyWindow(window) };
    }

    /// Catchers over a panel raised over them since (shown again, put in front) are put back over it, in their order,
    /// the last on top; over it still, they are left where they are.
    #[test]
    fn catchers_are_kept_over_the_panel_they_catch_for() {
        let made = || unsafe { CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE, w!("STATIC"), w!(""), WS_POPUP, 0, 0, 10, 10, None, None, None, None) }.expect("a window");
        let raise = |window: HWND| unsafe { SetWindowPos(window, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE).unwrap() };
        let (panel, station, device) = (made(), made(), made());
        for window in [panel, station, device] {
            raise(window);
        }
        assert!(above(station, panel) && above(device, station) && !above(panel, station));
        let catchers = [(station, Some(panel)), (device, Some(panel))];
        keep_over(&catchers);
        assert!(above(station, panel) && above(device, station));
        raise(panel);
        assert!(!above(station, panel) && !above(device, panel));
        keep_over(&catchers);
        assert!(above(station, panel) && above(device, panel) && above(device, station));
        for window in [panel, station, device] {
            let _ = unsafe { DestroyWindow(window) };
        }
    }

    /// The devices nearby are made last, over a screen's edge where they reach it as over the station they are on.
    #[test]
    fn devices_are_made_after_the_screens_edges() {
        let strip = rect(0, 0, 6, 100);
        let mut wanted: Vec<Wanted> = vec![
            (rect(0, 0, 50, 50), vec![rect(0, 0, 50, 50)], None, Lands::Station, None),
            (rect(0, 40, 30, 60), vec![rect(0, 40, 30, 60)], None, Lands::Device(7), None),
            (strip, vec![strip], Some(Edge::Left), Lands::Station, None),
        ];
        devices_last(&mut wanted);
        assert_eq!(wanted.iter().map(|(.., lands, _)| *lands).collect::<Vec<_>>(), [Lands::Station, Lands::Station, Lands::Device(7)]);
        assert_eq!(wanted[1].2, Some(Edge::Left));
    }

    /// A station less the windows over it: what is left of it, no more (none of it twice) and no less.
    #[test]
    fn a_station_is_caught_on_only_as_far_as_it_shows() {
        let station = rect(0, 0, 100, 100);
        // Nothing over it, or one beside it: all of it.
        assert_eq!(without(&[station], rect(200, 0, 300, 100)), [station]);
        // Covered whole: none.
        assert!(without(&[station], rect(-10, -10, 110, 110)).is_empty());
        // Its right half covered: its left half.
        assert_eq!(without(&[station], rect(50, -10, 200, 200)), [rect(0, 0, 50, 100)]);
        // A window in its middle: a ring round it, of all but that.
        let ring = without(&[station], rect(25, 25, 75, 75));
        assert_eq!(area(&ring), 100 * 100 - 50 * 50);
        for x in (0..100).step_by(5) {
            for y in (0..100).step_by(5) {
                let held = ring.iter().filter(|part| x >= part.left && x < part.right && y >= part.top && y < part.bottom).count();
                assert_eq!(held, usize::from(!((25..75).contains(&x) && (25..75).contains(&y))), "({x}, {y})");
            }
        }
        // Two windows over it, one after the other.
        let left = without(&without(&[station], rect(0, 0, 100, 30)), rect(0, 60, 40, 100));
        assert_eq!(area(&left), 100 * 100 - 100 * 30 - 40 * 40);
    }
}
