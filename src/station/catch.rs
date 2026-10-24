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
/// (an edge's strip opens the station there), and where what is dropped on it goes.
struct Catcher {
    rect: RECT,
    parts: Vec<RECT>,
    window: HWND,
    edge: Option<Edge>,
    lands: Lands,
    _target: IDropTarget,
}

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

/// Where drops are caught now (and the parts of each): each station and quick launch shown, as much of it as no other
/// window covers, and each edge of each screen that holds the workbench while it shows a station, as a strip.
fn wanted() -> Vec<(RECT, Vec<RECT>, Option<Edge>, Lands)> {
    let mut wanted: Vec<(RECT, Vec<RECT>, Option<Edge>, Lands)> = super::run::areas()
        .into_iter()
        .filter_map(|(window, area, lands)| {
            let parts = uncovered(window, area);
            (!parts.is_empty()).then_some((area, parts, None, lands))
        })
        .collect();
    let (places, station) = {
        let settings = crate::app().settings.lock().unwrap();
        (settings.places().clone(), crate::ui::prefs::module_on_in(&settings.view, "workbench", "station"))
    };
    if !station {
        return wanted;
    }
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
            wanted.push((strip, vec![strip], Some(edge), Lands::Station));
        }
    }
    wanted
}

/// What of `area` (screen px) of `window` shows: it less each window over it that the pointer meets (shown, neither
/// minimized nor cloaked, not one the pointer passes through, not a catcher), as far as that is drawn. Where another
/// window covers a station, a drop is that window's.
fn uncovered(window: HWND, area: RECT) -> Vec<RECT> {
    let mut parts = vec![area];
    let mut above = unsafe { GetWindow(window, GW_HWNDPREV) }.ok();
    while let Some(over) = above.filter(|_| !parts.is_empty()) {
        if meets_the_pointer(over) {
            if let Some(drawn) = drawn(over) {
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

/// Where `window` is drawn on the screen (its frame as drawn, not the borders it is sized by beyond it).
fn drawn(window: HWND) -> Option<RECT> {
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    let mut rect = RECT::default();
    let framed = unsafe { DwmGetWindowAttribute(window, DWMWA_EXTENDED_FRAME_BOUNDS, &mut rect as *mut RECT as *mut _, std::mem::size_of::<RECT>() as u32) }.is_ok();
    (framed || unsafe { GetWindowRect(window, &mut rect) }.is_ok()).then_some(rect)
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

unsafe extern "system" fn procedure(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

impl Catchers {
    /// Up where drops are wanted while something is dragged; none else. What each sees goes to `run::caught`.
    pub fn follow(&mut self) {
        let wanted = if dragging() { wanted() } else { Vec::new() };
        let same = wanted.len() == self.up.len() && wanted.iter().zip(&self.up).all(|((rect, parts, edge, lands), up)| *rect == up.rect && *parts == up.parts && *edge == up.edge && *lands == up.lands);
        if same {
            return;
        }
        self.clear();
        for (rect, parts, edge, lands) in wanted {
            if let Some(catcher) = make(rect, parts, edge, lands) {
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

/// A window catching drops over `rect` (screen px; along `edge`, where it is a strip there), as far as `parts` of it,
/// above all, all but invisible (hit by the pointer: a window wholly clear is passed over), telling `run::caught` what
/// it sees.
fn make(rect: RECT, parts: Vec<RECT>, edge: Option<Edge>, lands: Lands) -> Option<Catcher> {
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
        // (Only its parts: the rest of it covered, passed through to what covers it.)
        if parts != [rect] {
            use windows::Win32::Graphics::Gdi::{CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, RGN_OR};
            let region = CreateRectRgn(0, 0, 0, 0);
            for part in &parts {
                let piece = CreateRectRgn(part.left - rect.left, part.top - rect.top, part.right - rect.left, part.bottom - rect.top);
                let _ = CombineRgn(Some(region), Some(region), Some(piece), RGN_OR);
                let _ = DeleteObject(piece.into());
            }
            // (The system holds the region from here.)
            SetWindowRgn(window, Some(region), false);
        }
        let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
        let _ = SetWindowPos(window, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE);
        let told = move |dragged: Dragged| super::run::caught(dragged, edge, lands);
        // (What is kept at hand may be given a file off the station; the station, none of its own again.)
        let target = if lands == super::run::Lands::Station { Target::not_own(told) } else { Target::new(told) };
        if RegisterDragDrop(window, &target).is_err() {
            let _ = DestroyWindow(window);
            return None;
        }
        Some(Catcher { rect, parts, window, edge, lands, _target: target })
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
