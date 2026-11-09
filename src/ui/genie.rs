//! The genie (macOS's "神奇效果"): a window poured out of a small place on the screen as it opens, the button it was
//! asked from, and back into it as it closes. Its picture (see `picture_of`: the window as the compositor shows it,
//! its title bar and its backdrop with it) is drawn line by line across the way to that place (see `lines`): its sides
//! bent towards the place, into a funnel, as it slides along it into the place, each line as wide as the funnel is
//! where it is, and gone past the place's far side. Drawn on a window of its own over everything, which lets the
//! pointer through, for as long as it lasts (see `Genie`). The window it pictures is hidden meanwhile.

use std::rc::Rc;
use std::time::{Duration, Instant};

use windows::core::{Interface, Result};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_RECT_U, D2D_SIZE_U};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap1, ID2D1DeviceContext3, ID2D1SpriteBatch, D2D1_ANTIALIAS_MODE_ALIASED, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_OPTIONS_NONE,
    D2D1_BITMAP_PROPERTIES1, D2D1_SPRITE_OPTIONS_NONE,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;

use super::gfx::{Frame, Gfx, Surface};
use super::window::Window;

/// How long the window takes to pour out of its place, or back into it.
pub const LASTS: Duration = Duration::from_millis(320);

/// Of the way from the whole window (0) into its place (1): how far its sides have bent into the funnel by then
/// (over the first half of the way), and how far it has slid along it (from the start, together with the bending).
/// Both start from rest at the window, so the window sets off softly and, opening, lands softly as one: its sides
/// straight as it stops, not straightening after. The slide keeps gathering speed to the place and is at half as
/// much again as its mean there: closing, the window goes into the place at speed, opening, it leaves it at once
/// (the way is the time run backwards).
fn phases(p: f32) -> (f32, f32) {
    let bend = (p / 0.5).clamp(0.0, 1.0);
    let slide = p.clamp(0.0, 1.0);
    (bend * bend * (3.0 - 2.0 * bend), slide * slide * (1.5 - 0.5 * slide))
}

/// A window's picture: its pixels (blue, green, red, alpha, its colours multiplied by its alpha), row by row from its
/// top, `size` physical pixels large.
pub struct Picture {
    pub pixels: Vec<u8>,
    pub size: (u32, u32),
}

/// The window `window` as the compositor shows it (its title bar, the backdrop the system draws behind it and what
/// its program draws, as it would show on the screen: shown or cloaked), its corners rounded as the system rounds
/// them, and where it shows on the screen (physical px, without the room round it the system keeps to size it by).
pub fn picture_of(window: HWND) -> Option<(Picture, RECT)> {
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    use windows::Win32::Graphics::Gdi::{CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS};
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsZoomed, PW_RENDERFULLCONTENT};
    let mut whole = RECT::default();
    let mut shows = RECT::default();
    unsafe {
        GetWindowRect(window, &mut whole).ok()?;
        DwmGetWindowAttribute(window, DWMWA_EXTENDED_FRAME_BOUNDS, &mut shows as *mut RECT as *mut _, size_of::<RECT>() as u32).ok()?;
    }
    let (width, height) = (whole.right - whole.left, whole.bottom - whole.top);
    let (from_x, from_y) = (shows.left - whole.left, shows.top - whole.top);
    let size = ((shows.right - shows.left) as u32, (shows.bottom - shows.top) as u32);
    if width <= 0 || height <= 0 || size.0 == 0 || size.1 == 0 || from_x < 0 || from_y < 0 || from_x + size.0 as i32 > width || from_y + size.1 as i32 > height {
        return None;
    }
    let mut pixels = vec![0u8; (size.0 * size.1 * 4) as usize];
    unsafe {
        let dc = CreateCompatibleDC(None);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: width, biHeight: -height, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(dc);
            return None;
        };
        let was = SelectObject(dc, bitmap.into());
        let printed = PrintWindow(window, dc, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool();
        let _ = GdiFlush();
        if printed {
            let all = std::slice::from_raw_parts(bits as *const u8, (width * height * 4) as usize);
            let row = (size.0 * 4) as usize;
            for y in 0..size.1 as usize {
                let at = ((from_y as usize + y) * width as usize + from_x as usize) * 4;
                pixels[y * row..(y + 1) * row].copy_from_slice(&all[at..at + row]);
            }
        }
        SelectObject(dc, was);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        if !printed {
            return None;
        }
    }
    // (What it printed is opaque: its alpha is not of the window.)
    for pixel in pixels.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    // Rounded as Windows 11 rounds a window that does not fill its screen: 8 of its pixels at 100 %.
    let rounded = crate::ui::settings_window::windows_build() >= 22000 && !unsafe { IsZoomed(window) }.as_bool();
    if rounded {
        let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(window) }.max(96) as f32;
        round_corners(&mut pixels, size, 8.0 * dpi / 96.0);
    }
    Some((Picture { pixels, size }, shows))
}

/// `pixels` (as a `Picture`'s, `size` large) cut round at their corners, `radius` pixels round: what lies outside
/// gone, the edge as much as it covers its pixels.
fn round_corners(pixels: &mut [u8], size: (u32, u32), radius: f32) {
    let (w, h) = (size.0 as f32, size.1 as f32);
    let r = radius.min(w / 2.0).min(h / 2.0);
    let reach = r.ceil() as u32;
    for y in 0..reach.min(size.1) {
        for x in 0..reach.min(size.0) {
            // How much of the pixel lies within the corner's circle (by its centre's distance from it).
            let (dx, dy) = (r - (x as f32 + 0.5), r - (y as f32 + 0.5));
            let covered = (r - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
            if dx <= 0.0 || dy <= 0.0 {
                continue;
            }
            for (cx, cy) in [(x, y), (size.0 - 1 - x, y), (x, size.1 - 1 - y), (size.0 - 1 - x, size.1 - 1 - y)] {
                let at = ((cy * size.0 + cx) * 4) as usize;
                for part in &mut pixels[at..at + 4] {
                    *part = (*part as f32 * covered).round() as u8;
                }
            }
        }
    }
}

/// The side of the window its place lies beyond, the way it pours.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Top,
    Bottom,
    Left,
    Right,
}

/// The way from a window into its place, measured along it (from the window's far side towards the place) and across
/// it, in physical pixels: the window's far and near sides along it and its sides across it, and the place's.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Way {
    side: Side,
    window_along: (f32, f32),
    window_across: (f32, f32),
    place_along: (f32, f32),
    place_across: (f32, f32),
}

impl Way {
    /// From `window` (where it shows on the screen) to `place`: beyond the side of the window its centre lies furthest
    /// beyond (or, within the window, nearest to), at the window's side where it is nearer than that (the funnel's
    /// mouth is never inside the window).
    fn between(window: RECT, place: RECT) -> Way {
        let (cx, cy) = ((place.left + place.right) as f32 / 2.0, (place.top + place.bottom) as f32 / 2.0);
        let beyond = [
            (Side::Bottom, cy - window.bottom as f32),
            (Side::Top, window.top as f32 - cy),
            (Side::Right, cx - window.right as f32),
            (Side::Left, window.left as f32 - cx),
        ];
        let side = beyond.iter().copied().fold(beyond[0], |best, each| if each.1 > best.1 { each } else { best }).0;
        let (l, t, r, b) = (window.left as f32, window.top as f32, window.right as f32, window.bottom as f32);
        let (pl, pt, pr, pb) = (place.left as f32, place.top as f32, place.right as f32, place.bottom as f32);
        // (Along runs from the far side to the near one: up the screen for the top, leftwards for the left.)
        let (window_along, window_across, place_along, place_across) = match side {
            Side::Bottom => ((t, b), (l, r), (pt, pb), (pl, pr)),
            Side::Top => ((-b, -t), (l, r), (-pb, -pt), (pl, pr)),
            Side::Right => ((l, r), (t, b), (pl, pr), (pt, pb)),
            Side::Left => ((-r, -l), (t, b), (-pr, -pl), (pt, pb)),
        };
        let mouth = place_along.0.max(window_along.1);
        let place_along = (mouth, mouth + (place_along.1 - place_along.0).max(1.0));
        Way { side, window_along, window_across, place_along, place_across }
    }

    /// How long the window is along the way (its lines), in pixels.
    fn length(&self) -> f32 {
        self.window_along.1 - self.window_along.0
    }

    /// Where the window's sides are across the way at `along`, bent `bend` of the way into the funnel (0: straight):
    /// the funnel as wide as the window up to its far side, as the place from the place's near side on, and between
    /// them eased from the one to the other. Past the middle of the place it narrows as the ellipse within the place
    /// does (Glance's buttons are round): what pours into it ends in a round tip, not cut square.
    fn across_at(&self, along: f32, bend: f32) -> (f32, f32) {
        let (from, to) = (self.window_along.0, self.place_along.0);
        let u = if to > from { ((along - from) / (to - from)).clamp(0.0, 1.0) } else { 1.0 };
        let k = u * u * (3.0 - 2.0 * u);
        let funnel = |window: f32, place: f32| window + (place - window) * k;
        let (mut low, mut high) = (funnel(self.window_across.0, self.place_across.0), funnel(self.window_across.1, self.place_across.1));
        let (middle, half_along) = ((self.place_along.0 + self.place_along.1) / 2.0, (self.place_along.1 - self.place_along.0) / 2.0);
        if along > middle && half_along > 0.0 {
            let past = ((along - middle) / half_along).min(1.0);
            let (centre, half_across) = ((self.place_across.0 + self.place_across.1) / 2.0, (self.place_across.1 - self.place_across.0) / 2.0);
            let half = half_across * (1.0 - past * past).max(0.0).sqrt();
            (low, high) = (low.max(centre - half), high.min(centre + half));
        }
        (self.window_across.0 + (low - self.window_across.0) * bend, self.window_across.1 + (high - self.window_across.1) * bend)
    }

    /// A rectangle along and across the way, on the screen.
    fn on_screen(&self, along: (f32, f32), across: (f32, f32)) -> D2D_RECT_F {
        match self.side {
            Side::Bottom => D2D_RECT_F { left: across.0, top: along.0, right: across.1, bottom: along.1 },
            Side::Top => D2D_RECT_F { left: across.0, top: -along.1, right: across.1, bottom: -along.0 },
            Side::Right => D2D_RECT_F { left: along.0, top: across.0, right: along.1, bottom: across.1 },
            Side::Left => D2D_RECT_F { left: -along.1, top: across.0, right: -along.0, bottom: across.1 },
        }
    }

    /// The `line`th line of a picture `size` large along the way (counted from its far side), all across it.
    fn line_of(&self, line: u32, size: (u32, u32)) -> D2D_RECT_U {
        match self.side {
            Side::Bottom => D2D_RECT_U { left: 0, top: line, right: size.0, bottom: line + 1 },
            Side::Top => D2D_RECT_U { left: 0, top: size.1 - 1 - line, right: size.0, bottom: size.1 - line },
            Side::Right => D2D_RECT_U { left: line, top: 0, right: line + 1, bottom: size.1 },
            Side::Left => D2D_RECT_U { left: size.0 - 1 - line, top: 0, right: size.0 - line, bottom: size.1 },
        }
    }
}

/// The lines of a picture `size` large (see `Picture`) as they are drawn `p` of the way from the window into its place
/// (0: the window whole, where it is, 1: gone into the place) along `way`: each where it is on the screen, and the
/// line of the picture drawn there. Slid along, a line comes to the place's near side and on to its far side, where
/// it is gone.
fn lines(way: &Way, size: (u32, u32), p: f32) -> Vec<(D2D_RECT_F, D2D_RECT_U)> {
    let (bend, slide) = phases(p);
    let count = way.length().round().max(0.0) as u32;
    let end = way.place_along.1;
    let slid = slide * (end - way.window_along.0);
    let mut drawn = Vec::with_capacity(count as usize);
    for line in 0..count {
        let from = way.window_along.0 + line as f32 + slid;
        if from >= end {
            break;
        }
        let to = (from + 1.0).min(end);
        let across = way.across_at((from + to) / 2.0, bend);
        drawn.push((way.on_screen((from, to), across), way.line_of(line, size)));
    }
    drawn
}

/// Whether a window asked from `place` pours out of it: there is a place, and Windows' animations are on.
pub fn wanted(place: Option<RECT>) -> Option<RECT> {
    place.filter(|place| place.right > place.left && place.bottom > place.top && !crate::panel::reduced_motion())
}

/// `place` as it is told in a message (see `of_told`): each side in 16 bits (the screens' corners lie well within
/// them), none as nothing.
pub fn told(place: Option<RECT>) -> isize {
    place.map_or(0, |at| {
        let side = |at: i32| at as i16 as u16 as u64;
        (side(at.left) | side(at.top) << 16 | side(at.right) << 32 | side(at.bottom) << 48) as isize
    })
}

/// The place told as `told` tells it.
pub fn of_told(told: isize) -> Option<RECT> {
    let told = told as u64;
    let side = |shift: u32| (told >> shift) as u16 as i16 as i32;
    let place = RECT { left: side(0), top: side(16), right: side(32), bottom: side(48) };
    (place.right > place.left && place.bottom > place.top).then_some(place)
}

/// `window` hidden as it is shown, for it to pour out of its place (see `opening`): cloaked (the compositor draws it,
/// no one sees it), and without the system's own animation of its showing.
pub fn hide_to_open(window: HWND) {
    cloak(window, true);
}

/// `window`, hidden by `hide_to_open` and shown, its first frame drawn, poured out of `place` from now on. None where
/// it could not be, or `place` is out of its reach (see `reachable`): shown at once then (see `reveal`).
pub fn opening(window: HWND, place: RECT) -> Option<Genie> {
    // (Pictured at once: the picture is the window's whole look as soon as it is shown, cloaked; waiting for the
    // compositor's next frame first gave the same picture a tenth of a second and more later. See
    // `settings_window::pictures::how_soon_it_is_pictured`.)
    let (picture, shows) = picture_of(window)?;
    if !reachable(shows, place) {
        return None;
    }
    crate::perf::mark(|| format!("genie pictured {}x{}", picture.size.0, picture.size.1));
    let genie = Genie::start(picture, shows, place, true);
    crate::perf::mark(|| "genie started".into());
    genie
}

/// Where `window`, going, pours into, out of `place` as it came: there where it is still seen (one of Glance's own
/// windows shows at its middle, a widget whose button it was, or it is the tray's icon), else into Glance's icon in the
/// tray, where Glance stays (as a window put away goes into the Dock), where that is on the window's screen, else into
/// the place still (the panel, gone from its edge as the pointer left it, comes out of there again). None where the
/// place is out of reach (see `reachable`: under the window, the window moved over it): it closes without pouring.
pub fn back_into(window: HWND, place: RECT) -> Option<RECT> {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let mut shows = RECT::default();
    unsafe { GetWindowRect(window, &mut shows) }.ok()?;
    let icon = crate::tray::icon_place();
    if icon == Some(place) || glance_shows_at(window, place) {
        return reachable(shows, place).then_some(place);
    }
    // (The tray's icon on another screen: into the place still, where what it was asked from comes back, as the panel
    // gone from its edge comes out of it again.)
    icon.filter(|icon| reachable(shows, *icon)).or_else(|| reachable(shows, place).then_some(place))
}

/// Whether a window showing at `shows` pours into the place `place` or out of it: not under it, and all the way on the
/// screens (see `shown_whole`), across as many as it goes. (A place under the window has no side of the window to pour
/// through: its mouth, kept at the window's nearest side, poured into the window's edge above the place, far from it.)
fn reachable(shows: RECT, place: RECT) -> bool {
    let (x, y) = ((place.left + place.right) / 2, (place.top + place.bottom) / 2);
    let under = shows.left <= x && x < shows.right && shows.top <= y && y < shows.bottom;
    !under && shown_whole(shows, place, &screens())
}

/// How many frames of its pouring are looked at to tell whether it shows whole (see `shown_whole`): about as many as it
/// draws on a fast screen (`LASTS` at 144 a second).
const LOOKED_AT: u32 = 48;

/// How much of what it draws, all through, may be where no screen shows it, and it still shows whole: a sliver past a
/// screen's edge (the settings a few pixels wider than the screen under them, on their way into its corner), not a
/// way through where no screen is, which loses far more.
const UNSEEN_MOST: f32 = 0.01;

/// Whether a window showing at `shows`, pouring into `place` (or out of it), shows whole: all but `UNSEEN_MOST` of every
/// line of it, all through, where one of `screens` shows it, as it passes from screen to screen where they meet.
/// (Between two screens with no screen between them, it went through where nothing shows, cut off there.)
fn shown_whole(shows: RECT, place: RECT, screens: &[RECT]) -> bool {
    let way = Way::between(shows, place);
    let size = ((shows.right - shows.left).max(1) as u32, (shows.bottom - shows.top).max(1) as u32);
    let (mut drawn, mut unseen) = (0.0, 0.0);
    for frame in 0..=LOOKED_AT {
        for (at, _) in lines(&way, size, frame as f32 / LOOKED_AT as f32) {
            let (long, off) = off_screens(at, screens);
            drawn += long;
            unseen += off;
        }
    }
    unseen <= drawn * UNSEEN_MOST
}

/// How long the line `at` (a row or a column, a pixel or so thick) is, and how much of it, along its middle, is where
/// none of `screens` shows.
fn off_screens(at: D2D_RECT_F, screens: &[RECT]) -> (f32, f32) {
    let row = at.right - at.left >= at.bottom - at.top;
    // (Along it, from and to; across it, its middle.)
    let (from, to, middle) = if row { (at.left, at.right, (at.top + at.bottom) / 2.0) } else { (at.top, at.bottom, (at.left + at.right) / 2.0) };
    let long = (to - from).max(0.0);
    // (Screens do not overlap: what each shows of it, summed.)
    let shown: f32 = screens
        .iter()
        .map(|s| if row { ((s.top as f32, s.bottom as f32), (s.left as f32, s.right as f32)) } else { ((s.left as f32, s.right as f32), (s.top as f32, s.bottom as f32)) })
        .filter(|((low, high), _)| *low <= middle && middle < *high)
        .map(|(_, (start, end))| (end.min(to) - start.max(from)).max(0.0))
        .sum();
    (long, (long - shown).max(0.0))
}

/// The screens, where each is (physical px).
fn screens() -> Vec<RECT> {
    use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO};
    unsafe extern "system" fn each(monitor: HMONITOR, _: HDC, _: *mut RECT, found: windows::Win32::Foundation::LPARAM) -> windows::core::BOOL {
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            unsafe { &mut *(found.0 as *mut Vec<RECT>) }.push(info.rcMonitor);
        }
        true.into()
    }
    let mut screens = Vec::new();
    let _ = unsafe { EnumDisplayMonitors(None, None, Some(each), windows::Win32::Foundation::LPARAM(&mut screens as *mut Vec<RECT> as isize)) };
    screens
}

/// Whether one of Glance's own windows is what shows at the middle of `place` on the screen, `window` left out: the first
/// window there from the top, shown and not cloaked, is this process's.
fn glance_shows_at(window: HWND, place: RECT) -> bool {
    use windows::Win32::Foundation::{LPARAM, POINT};
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows::Win32::Graphics::Gdi::PtInRect;
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible};
    struct Looking {
        left_out: HWND,
        middle: POINT,
        found: Option<HWND>,
    }
    unsafe extern "system" fn each(hwnd: HWND, looking: LPARAM) -> windows::core::BOOL {
        let looking = unsafe { &mut *(looking.0 as *mut Looking) };
        if hwnd == looking.left_out || !unsafe { IsWindowVisible(hwnd) }.as_bool() {
            return true.into();
        }
        let mut cloaked = 0u32;
        let _ = unsafe { DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4) };
        let mut rect = RECT::default();
        if cloaked != 0 || unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() || !unsafe { PtInRect(&rect, looking.middle) }.as_bool() {
            return true.into();
        }
        looking.found = Some(hwnd);
        false.into()
    }
    let mut looking = Looking { left_out: window, middle: POINT { x: (place.left + place.right) / 2, y: (place.top + place.bottom) / 2 }, found: None };
    // (Top-level windows from the top down: the first there is what shows.)
    let _ = unsafe { EnumWindows(Some(each), LPARAM(&mut looking as *mut Looking as isize)) };
    looking.found.is_some_and(|hwnd| {
        let mut process = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process)) };
        process == std::process::id()
    })
}

/// `window`, hidden to pour out of its place, shown as it is (it has poured out, or could not).
pub fn reveal(window: HWND) {
    cloak(window, false);
}

/// `window`, shown, poured into `place` from now on, and hidden meanwhile, the front given to what was behind it
/// (closed by its owner once it is done), `told` the window it pours on as it shows. None where it could not be (left
/// as it was).
pub fn closing(window: HWND, place: RECT, told: impl FnOnce(HWND)) -> Option<Genie> {
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED};
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
    let (picture, shows) = picture_of(window)?;
    let genie = Genie::start(picture, shows, place, false)?;
    // (What pours told of before the window is hidden: what gave way to the window gives way to it, as the window, gone
    // from the front as it is hidden, gives way no more.)
    told(genie.window.hwnd);
    // (Hidden only once what pours is on the screen in its place, without the system's animation of its hiding.)
    let _ = unsafe { windows::Win32::Graphics::Dwm::DwmFlush() };
    let off = windows::core::BOOL::from(true);
    unsafe {
        let _ = DwmSetWindowAttribute(window, DWMWA_TRANSITIONS_FORCEDISABLED, &off as *const _ as *const _, 4);
        let _ = ShowWindow(window, SW_HIDE);
    }
    Some(genie)
}

/// `window` cloaked (drawn by the compositor and kept, shown to no one) or shown again, the system's animations of its
/// showing and hiding off meanwhile.
fn cloak(window: HWND, on: bool) {
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CLOAK, DWMWA_TRANSITIONS_FORCEDISABLED};
    let on = windows::core::BOOL::from(on);
    unsafe {
        if on.as_bool() {
            let _ = DwmSetWindowAttribute(window, DWMWA_TRANSITIONS_FORCEDISABLED, &on as *const _ as *const _, 4);
        }
        let _ = DwmSetWindowAttribute(window, DWMWA_CLOAK, &on as *const _ as *const _, 4);
        if !on.as_bool() {
            let _ = DwmSetWindowAttribute(window, DWMWA_TRANSITIONS_FORCEDISABLED, &on as *const _ as *const _, 4);
        }
    }
}

/// A window poured out of its place, or into it, drawn on a window of its own till it is done.
pub struct Genie {
    /// The device it draws with: for the screen holding the most of its window (see `gfx::for_area`), whatever the
    /// window it pictures draws with.
    gfx: Rc<Gfx>,
    window: Window,
    surface: Surface,
    picture: Picture,
    bitmap: Option<(ID2D1Bitmap1, ID2D1SpriteBatch)>,
    way: Way,
    /// Where its window is on the screen (physical px).
    area: RECT,
    place: RECT,
    opening: bool,
    began: Instant,
}

impl Genie {
    /// The window pictured in `picture` (showing on the screen at `shows`) poured out of `place` (`opening`) or into
    /// it, from now on: its first frame drawn, over everything. None where it could not be drawn.
    pub fn start(picture: Picture, shows: RECT, place: RECT, opening: bool) -> Option<Genie> {
        let way = Way::between(shows, place);
        crate::perf::mark(|| format!("genie {} place {},{},{},{} window {},{},{},{} {:?}", if opening { "opening" } else { "closing" }, place.left, place.top, place.right, place.bottom, shows.left, shows.top, shows.right, shows.bottom, way.side));
        let area = RECT { left: shows.left.min(place.left), top: shows.top.min(place.top), right: shows.right.max(place.right), bottom: shows.bottom.max(place.bottom) };
        let gfx = super::gfx::for_area(area).ok()?;
        let mut window = Window::new().ok()?;
        window.set_click_through(true);
        let surface = Surface::new(&gfx, window.hwnd).ok()?;
        let mut genie = Genie { gfx, window, surface, picture, bitmap: None, way, area, place, opening, began: Instant::now() };
        genie.window.place(area);
        if !genie.draw_at(if opening { 1.0 } else { 0.0 }) {
            return None;
        }
        genie.window.show();
        Some(genie)
    }

    /// Its next frame drawn, as far as it has gone by `now`, and the compositor's next frame waited for (one frame of
    /// it for each of the screen's): whether it is still going (done, its last frame drawn, the window whole where it
    /// is, or gone into its place).
    pub fn draw(&mut self, now: Instant) -> bool {
        let t = (now.duration_since(self.began).as_secs_f32() / LASTS.as_secs_f32()).min(1.0);
        let p = if self.opening { 1.0 - t } else { t };
        let began = Instant::now();
        let going = self.draw_at(p) && t < 1.0;
        let drawn = began.elapsed();
        if going {
            let _ = unsafe { windows::Win32::Graphics::Dwm::DwmFlush() };
        }
        crate::perf::mark(|| format!("genie frame {p:.3} drawn {:.1} ms, waited {:.1} ms{}", drawn.as_secs_f32() * 1000.0, (began.elapsed() - drawn).as_secs_f32() * 1000.0, if going { "" } else { " done" }));
        going
    }

    /// Whether it is pouring out of its place (not into it).
    pub fn opening(&self) -> bool {
        self.opening
    }

    /// The place it pours out of or into (physical px).
    pub fn place(&self) -> RECT {
        self.place
    }

    fn draw_at(&mut self, p: f32) -> bool {
        let size = ((self.area.right - self.area.left).max(1) as u32, (self.area.bottom - self.area.top).max(1) as u32);
        let lines = lines(&self.way, self.picture.size, p);
        let (left, top) = (self.area.left as f32, self.area.top as f32);
        let (picture, bitmap) = (&self.picture, &mut self.bitmap);
        let mut failed = None;
        let drawn = self.surface.draw(&self.gfx, size, 1.0, |frame| {
            if let Err(error) = paint(frame, picture, bitmap, &lines, (left, top)) {
                failed = Some(error);
            }
        });
        drawn.is_ok() && failed.is_none()
    }
}

/// `lines` of `picture` drawn on `frame`, its corner at `corner` on the screen, the picture made a bitmap on the
/// frame's device the first time (kept in `bitmap`, with the batch its lines are drawn in).
fn paint(frame: &Frame, picture: &Picture, bitmap: &mut Option<(ID2D1Bitmap1, ID2D1SpriteBatch)>, lines: &[(D2D_RECT_F, D2D_RECT_U)], corner: (f32, f32)) -> Result<()> {
    let dc: ID2D1DeviceContext3 = frame.dc.cast()?;
    if bitmap.is_none() {
        let properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
            ..Default::default()
        };
        let size = D2D_SIZE_U { width: picture.size.0, height: picture.size.1 };
        let made = unsafe { dc.CreateBitmap(size, Some(picture.pixels.as_ptr().cast()), picture.size.0 * 4, &properties)? };
        *bitmap = Some((made, unsafe { dc.CreateSpriteBatch()? }));
    }
    let (made, batch) = bitmap.as_ref().unwrap();
    let places: Vec<D2D_RECT_F> = lines.iter().map(|(at, _)| D2D_RECT_F { left: at.left - corner.0, top: at.top - corner.1, right: at.right - corner.0, bottom: at.bottom - corner.1 }).collect();
    let from: Vec<D2D_RECT_U> = lines.iter().map(|(_, line)| *line).collect();
    unsafe {
        batch.Clear();
        if places.is_empty() {
            return Ok(());
        }
        batch.AddSprites(places.len() as u32, places.as_ptr(), Some(from.as_ptr()), None, None, size_of::<D2D_RECT_F>() as u32, size_of::<D2D_RECT_U>() as u32, 0, 0)?;
        // (A batch of sprites is drawn only with its edges on whole pixels.)
        let was = dc.GetAntialiasMode();
        dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
        dc.DrawSpriteBatch(batch, 0, places.len() as u32, made, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_SPRITE_OPTIONS_NONE);
        dc.SetAntialiasMode(was);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT { left, top, right, bottom }
    }

    /// The way runs to the side of the window its place lies beyond (furthest beyond, where it is beyond two), or,
    /// within the window, the side it is nearest, its mouth never inside the window.
    #[test]
    fn the_way_goes_to_the_side_its_place_lies_beyond() {
        let window = rect(400, 200, 1400, 900);
        assert_eq!(Way::between(window, rect(880, 1000, 920, 1040)).side, Side::Bottom);
        assert_eq!(Way::between(window, rect(880, 20, 920, 60)).side, Side::Top);
        assert_eq!(Way::between(window, rect(20, 500, 60, 540)).side, Side::Left);
        assert_eq!(Way::between(window, rect(1700, 500, 1740, 540)).side, Side::Right);
        // Beyond two sides: the one it is further beyond.
        assert_eq!(Way::between(window, rect(1500, 1300, 1540, 1340)).side, Side::Bottom);
        assert_eq!(Way::between(window, rect(1800, 1000, 1840, 1040)).side, Side::Right);
        // Within: the nearest side, its mouth at that side.
        let within = Way::between(window, rect(420, 500, 460, 540));
        assert_eq!(within.side, Side::Left);
        assert_eq!(within.place_along, (-400.0, -360.0));
        // Across the way as the place is.
        let top = Way::between(window, rect(880, 20, 920, 60));
        assert_eq!((top.window_along, top.place_along, top.place_across), ((-900.0, -200.0), (-60.0, -20.0), (880.0, 920.0)));
    }

    /// A window pours into a place outside it, not one under it.
    #[test]
    fn it_pours_only_into_a_place_outside_it() {
        let window = rect(9, -1290, 1941, -159);
        assert!(!super::reachable(window, rect(1431, -963, 1458, -936)));
        assert!(super::reachable(window, rect(1960, -963, 1987, -936)));
    }

    /// A window pours across screens where they meet along its way (as the owner's: a wide screen over the main one,
    /// the tray's icon at the main one's corner), not where its way passes where no screen is (down past the wide
    /// screen's left, beside the main one, to a tall screen's foot).
    #[test]
    fn it_pours_across_screens_where_they_meet() {
        let screens = [rect(-2905, -1459, -745, 2381), rect(-745, -1440, 2695, 0), rect(0, 0, 1920, 1080)];
        let window = rect(9, -1290, 1941, -159);
        assert!(super::shown_whole(window, rect(1880, 1040, 1904, 1064), &screens));
        assert!(super::shown_whole(window, rect(2628, -110, 2671, -66), &screens));
        assert!(!super::shown_whole(window, rect(-1000, 2000, -970, 2030), &screens));
        // On one screen alone, a place beyond its edge where no screen is: not.
        assert!(!super::shown_whole(window, rect(2000, 500, 2030, 530), &screens[1..2]));
    }

    /// The window bends and slides together from rest: wherever its sides are bent at all it is still on its way (it
    /// never stops first and straightens after), both at rest at the window, the slide at speed at the place.
    #[test]
    fn it_bends_and_slides_as_one() {
        let speed = |p: f32, which: fn((f32, f32)) -> f32| (which(phases(p + 1e-3)) - which(phases(p))) / 1e-3;
        for step in 1..=100 {
            let p = step as f32 / 100.0;
            let (bend, slide) = phases(p);
            assert!(bend > 0.0 && slide > 0.0 && slide < 1.0 + 1e-6, "at {p}: {bend} {slide}");
            assert!(speed(p - 1e-3, |(_, slide)| slide) > 0.0, "at {p}");
        }
        assert_eq!(phases(0.0), (0.0, 0.0));
        assert!(speed(0.0, |(bend, _)| bend) < 0.05 && speed(0.0, |(_, slide)| slide) < 0.05);
        assert!((speed(1.0 - 1e-3, |(_, slide)| slide) - 1.5).abs() < 0.01);
    }

    /// What pours into a place ends round in it: as wide as the place to its middle, then as the circle within it, a
    /// point at its far side; nowhere outside the circle past its middle.
    #[test]
    fn it_ends_round_in_its_place() {
        let way = Way::between(rect(400, 200, 1400, 900), rect(880, 1000, 920, 1040));
        let width = |along: f32| {
            let (low, high) = way.across_at(along, 1.0);
            high - low
        };
        assert!((width(1000.0) - 40.0).abs() < 1e-3 && (width(1020.0) - 40.0).abs() < 1e-3);
        assert!((width(1030.0) - 2.0 * 20.0 * (1.0f32 - 0.25).sqrt()).abs() < 1e-3);
        assert!(width(1039.5) < 9.0 && width(1040.0).abs() < 1e-3);
        for step in 0..=40 {
            let along = 1020.0 + step as f32 / 2.0;
            let (low, high) = way.across_at(along, 1.0);
            let (dx, dy) = ((high - 900.0) / 20.0, (along - 1020.0) / 20.0);
            assert!(dx * dx + dy * dy <= 1.0 + 1e-4 && (low - 900.0).abs() - (high - 900.0) < 1e-3, "{along}: {low}..{high}");
        }
    }

    /// At the start the window is drawn whole where it is, each line of its picture on its own line of the screen.
    /// At the end nothing is left of it. On the way every line drawn lies between the window's far side and the
    /// place's far side, no wider than the window, and the lines come one after another without gaps or overlaps.
    #[test]
    fn the_window_pours_into_its_place_line_by_line() {
        for (window, place) in [
            (rect(400, 200, 1400, 900), rect(880, 1000, 920, 1040)),
            (rect(400, 200, 1400, 900), rect(880, 20, 920, 60)),
            (rect(400, 200, 1400, 900), rect(20, 300, 60, 340)),
            (rect(400, 200, 1400, 900), rect(1700, 870, 1740, 910)),
            (rect(400, 200, 1400, 900), rect(1300, 820, 1340, 860)),
        ] {
            let way = Way::between(window, place);
            let size = ((window.right - window.left) as u32, (window.bottom - window.top) as u32);
            let whole = lines(&way, size, 0.0);
            assert_eq!(whole.len() as u32, if matches!(way.side, Side::Bottom | Side::Top) { size.1 } else { size.0 }, "{way:?}");
            for (at, line) in &whole {
                // Each on its own pixels of the window's, the line of the picture that shows there.
                let (x, y) = ((at.left - window.left as f32) as u32, (at.top - window.top as f32) as u32);
                assert_eq!((at.left.fract(), at.top.fract(), at.right.fract(), at.bottom.fract()), (0.0, 0.0, 0.0, 0.0), "{way:?}");
                match way.side {
                    Side::Top | Side::Bottom => assert_eq!((at.left, at.right, line.top, line.bottom - line.top), (window.left as f32, window.right as f32, y, 1), "{way:?}"),
                    Side::Left | Side::Right => assert_eq!((at.top, at.bottom, line.left, line.right - line.left), (window.top as f32, window.bottom as f32, x, 1), "{way:?}"),
                }
            }
            assert!(lines(&way, size, 1.0).is_empty(), "{way:?}");
            for step in 1..20 {
                let p = step as f32 / 20.0;
                let drawn = lines(&way, size, p);
                let along = |at: &D2D_RECT_F| match way.side {
                    Side::Bottom => (at.top, at.bottom),
                    Side::Top => (-at.bottom, -at.top),
                    Side::Right => (at.left, at.right),
                    Side::Left => (-at.right, -at.left),
                };
                let across = |at: &D2D_RECT_F| match way.side {
                    Side::Bottom | Side::Top => (at.left, at.right),
                    Side::Left | Side::Right => (at.top, at.bottom),
                };
                for pair in drawn.windows(2) {
                    assert!((along(&pair[0].0).1 - along(&pair[1].0).0).abs() < 1e-3, "{way:?} at {p}: a gap or an overlap");
                }
                for (at, _) in &drawn {
                    let (from, to) = along(at);
                    let (low, high) = across(at);
                    assert!(from >= way.window_along.0 - 1e-3 && to <= way.place_along.1 + 1e-3, "{way:?} at {p}: {from}..{to}");
                    assert!(low >= way.window_across.0.min(way.place_across.0) - 1e-3 && high <= way.window_across.1.max(way.place_across.1) + 1e-3 && high > low, "{way:?} at {p}: {low}..{high}");
                }
                // Further along, fewer lines are left.
                assert!(drawn.len() <= lines(&way, size, p - 0.05).len(), "{way:?} at {p}");
            }
        }
    }

    /// Frames of a window pouring into a place below it and to its left, one picture each (target/genie), for the eye:
    /// a window of stripes and squares, its title bar dark.
    #[test]
    #[ignore = "writes pictures to look at"]
    #[cfg(feature = "studio")]
    fn frames_to_look_at() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let gfx = Gfx::new().expect("graphics");
        let out = std::path::PathBuf::from(format!("{}/target/genie", env!("CARGO_MANIFEST_DIR")));
        std::fs::create_dir_all(&out).unwrap();
        let size = (600u32, 400u32);
        let mut pixels = vec![0u8; (size.0 * size.1 * 4) as usize];
        for y in 0..size.1 {
            for x in 0..size.0 {
                let at = ((y * size.0 + x) * 4) as usize;
                let shade = if y < 40 { 60 } else if (x / 40 + y / 40) % 2 == 0 { 235 } else { 200 };
                let blue = if x % 100 < 4 || y % 100 < 4 { 40 } else { shade };
                pixels[at..at + 4].copy_from_slice(&[blue, shade, shade, 255]);
            }
        }
        round_corners(&mut pixels, size, 12.0);
        let picture = Picture { pixels, size };
        for (name, window, place) in [("down", rect(300, 100, 900, 500), rect(140, 760, 180, 800)), ("left", rect(300, 100, 900, 500), rect(20, 600, 60, 640))] {
            let way = Way::between(window, place);
            let area = rect(0, 0, 960, 820);
            let mut bitmap = None;
            for step in 0..=10 {
                let p = step as f32 / 10.0;
                let lines = lines(&way, picture.size, p);
                let drawn = gfx
                    .draw_offscreen((area.right as u32, area.bottom as u32), 1.0, |frame| {
                        paint(frame, &picture, &mut bitmap, &lines, (0.0, 0.0)).unwrap();
                    })
                    .unwrap();
                crate::studio::save_png(&out.join(format!("{name}-{step:02}.png")), area.right as u32, area.bottom as u32, &drawn).unwrap();
            }
        }
    }

    /// Its corners cut round: a corner pixel gone, one on the edge half there, the middle of a side whole.
    #[test]
    fn its_corners_are_cut_round() {
        let size = (40, 30);
        let mut pixels = vec![255u8; 40 * 30 * 4];
        round_corners(&mut pixels, size, 8.0);
        let alpha = |x: u32, y: u32| pixels[((y * size.0 + x) * 4 + 3) as usize];
        for (x, y) in [(0, 0), (39, 0), (0, 29), (39, 29)] {
            assert_eq!(alpha(x, y), 0);
        }
        assert_eq!((alpha(20, 0), alpha(0, 15), alpha(8, 8), alpha(20, 15)), (255, 255, 255, 255));
        assert!((1..255).contains(&alpha(1, 3)), "{}", alpha(1, 3));
    }

    /// A window going pours back into its place where one of Glance's own windows still shows there (a widget whose
    /// button it was), and where none does (the panel it came out of gone), not there: into the tray's icon, none here.
    #[test]
    #[ignore = "makes a window of this session's"]
    fn it_pours_back_where_its_place_is_still_seen() {
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, ShowWindow, SW_HIDE, SW_SHOWNOACTIVATE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP};
        let widget = unsafe { CreateWindowExW(WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, w!("STATIC"), w!(""), WS_POPUP, 0, 0, 60, 60, None, None, None, None) }.unwrap();
        let button = RECT { left: 20, top: 20, right: 40, bottom: 40 };
        unsafe {
            let _ = ShowWindow(widget, SW_SHOWNOACTIVATE);
        }
        let shown = back_into(HWND::default(), button);
        unsafe {
            let _ = ShowWindow(widget, SW_HIDE);
        }
        let hidden = back_into(HWND::default(), button);
        // (Itself left out, as the window going is: what shows under it counts.)
        unsafe {
            let _ = ShowWindow(widget, SW_SHOWNOACTIVATE);
        }
        let left_out = back_into(widget, button);
        unsafe {
            let _ = DestroyWindow(widget);
        }
        assert_eq!(shown, Some(button));
        assert_eq!(hidden, None);
        assert_eq!(left_out, None);
    }
}
