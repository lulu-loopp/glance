//! The panel's own window: a borderless, topmost tool window that never takes
//! the focus, drawn entirely by DirectComposition.

use std::cell::Cell;

use windows::core::{w, Result, BOOL, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW, IsWindowVisible, LoadCursorW, HWND_TOP, RegisterClassW, SetCursor, SetLayeredWindowAttributes,
    SetWindowDisplayAffinity, SetWindowLongPtrW, SetWindowPos, ShowWindow, WDA_EXCLUDEFROMCAPTURE, WDA_NONE, GWL_EXSTYLE, GWLP_USERDATA, HTCLIENT, HWND_NOTOPMOST, HWND_TOPMOST, IDC_ARROW, LWA_ALPHA,
    MA_NOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_FRAMECHANGED, SW_HIDE, SW_SHOWNOACTIVATE,
    PostMessageW, WM_APP, WM_CAPTURECHANGED, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_MOUSEACTIVATE, WM_NCHITTEST, WM_SETCURSOR, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

/// Posted to the panel's thread when the screens changed (a resolution, a
/// monitor gone; WPARAM 0) or the window's scale did (WPARAM its new DPI,
/// which the panel's own move to another monitor causes too).
pub const SCREENS_CHANGED: u32 = WM_APP + 6;
/// Posted to the window's thread when the mouse capture it held was taken
/// from it (or let go).
pub const CAPTURE_LOST: u32 = WM_APP + 8;

pub struct Window {
    pub hwnd: HWND,
    click_through: bool,
    /// The window it gives way to while there is one (see `yield_to`).
    under: Cell<Option<HWND>>,
    /// Where it was last put by `place_keeping`.
    placed: Cell<RECT>,
    /// Kept on the desktop, under every other window (see `set_on_desktop`),
    /// rather than above them all.
    on_desktop: Cell<bool>,
    /// Above them all for now, kept on the desktop or not (see `lift`).
    lifted: Cell<bool>,
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        // Clicking the panel must leave the focus where it was.
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_NCHITTEST => LRESULT(HTCLIENT as isize),
        // The pointer's shape, as the window's owner last set it (see
        // `Window::point`): asked for at every move, before the move itself.
        WM_SETCURSOR => {
            let shape = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
            let shape = if shape == 0 { IDC_ARROW } else { PCWSTR(shape as *const u16) };
            unsafe { SetCursor(LoadCursorW(None, shape).ok()) };
            LRESULT(1)
        }
        WM_CAPTURECHANGED => {
            let _ = unsafe { PostMessageW(Some(hwnd), CAPTURE_LOST, WPARAM(0), LPARAM(0)) };
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            let _ = unsafe { PostMessageW(Some(hwnd), SCREENS_CHANGED, WPARAM(0), LPARAM(0)) };
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let _ = unsafe { PostMessageW(Some(hwnd), SCREENS_CHANGED, WPARAM(wparam.0 & 0xFFFF), LPARAM(0)) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

impl Window {
    /// A window that can let clicks through (see `set_click_through`).
    pub fn new() -> Result<Self> {
        Self::make(true, false)
    }

    /// A window that can come to the front and take the keyboard, as a
    /// menu does while it is up (see `menu`).
    pub fn activating() -> Result<Self> {
        Self::make(true, true)
    }

    /// A window that always takes its clicks, and that the system's
    /// compositor may draw the screen behind into, blurred (a layered
    /// window gets no host backdrop).
    pub fn see_through() -> Result<Self> {
        Self::make(false, false)
    }

    fn make(layered: bool, activating: bool) -> Result<Self> {
        let instance = unsafe { GetModuleHandleW(None)? };
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance.into(),
            lpszClassName: w!("GlancePanel"),
            // None: each window says what the pointer looks like over it
            // (`Window::point`).
            ..Default::default()
        };
        unsafe { RegisterClassW(&class) };
        // Layered so that it can let clicks through (see `set_click_through`);
        // fully opaque as a layer, its content's own alpha is what shows.
        let style = WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP;
        let style = if activating { style } else { style | WS_EX_NOACTIVATE };
        let hwnd = unsafe {
            CreateWindowExW(
                if layered { style | WS_EX_LAYERED } else { style },
                w!("GlancePanel"),
                w!("Glance"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(instance.into()),
                None,
            )?
        };
        let set_up = || -> Result<()> {
            if layered {
                unsafe { SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)? };
            }
            // The panel animates itself; no system fade on top.
            let disabled = BOOL::from(true);
            unsafe { DwmSetWindowAttribute(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, &disabled as *const _ as *const _, size_of::<BOOL>() as u32) }
        };
        // Not set up as it must be: not left behind either.
        if let Err(error) = set_up() {
            let _ = unsafe { DestroyWindow(hwnd) };
            return Err(error);
        }
        Ok(Window { hwnd, click_through: false, under: Cell::new(None), placed: Cell::new(RECT::default()), on_desktop: Cell::new(false), lifted: Cell::new(false) })
    }

    pub fn place(&self, rect: RECT) {
        self.place_under(rect, None);
    }

    /// Placed at `rect`, above all, or just below `above` (another window
    /// above all) while there is one, or the window it gives way to.
    pub fn place_under(&self, rect: RECT, above: Option<HWND>) {
        if self.low() {
            unsafe {
                let _ = SetWindowPos(self.hwnd, None, rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top, SWP_NOACTIVATE | SWP_NOZORDER);
            }
            self.sink();
            return;
        }
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(above.or(self.under.get()).unwrap_or(HWND_TOPMOST)),
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOACTIVATE,
            );
        }
    }

    pub fn show(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
        if self.low() {
            self.sink();
            return;
        }
        unsafe {
            // Above whatever took the top since it was last shown (but the
            // window it gives way to).
            let _ = SetWindowPos(self.hwnd, Some(self.under.get().unwrap_or(HWND_TOPMOST)), 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE);
        }
    }

    /// Placed at `rect`, where it is among the other windows kept; nothing
    /// done where it is there already.
    pub fn place_keeping(&self, rect: RECT) {
        if self.placed.get() == rect {
            return;
        }
        self.placed.set(rect);
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top, SWP_NOACTIVATE | SWP_NOZORDER);
        }
    }

    /// Shown if hidden, where it is among the other windows kept.
    pub fn show_in_place(&self) {
        unsafe {
            if !IsWindowVisible(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
        }
    }

    /// In front of the windows of its kind (the topmost ones, or the others),
    /// or, giving way to a window, just below it still.
    pub fn raise(&self) {
        if self.low() {
            return;
        }
        let after = self.under.get().unwrap_or(HWND_TOP);
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(after), 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE);
        }
    }

    /// Whether it is giving way to another window (see `yield_to`).
    pub fn yielding(&self) -> bool {
        self.under.get().is_some()
    }

    /// Gives way to `window` (an ordinary window in front, as the settings
    /// are), just below it, until it is given way to no more (`None`):
    /// above all again.
    pub fn yield_to(&self, window: Option<HWND>) {
        self.under.set(window);
        // On the desktop, it is under that window already.
        if self.low() {
            return;
        }
        let flags = SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE;
        unsafe {
            if let Some(window) = window {
                // Out of the topmost windows first: placed after an ordinary
                // window, it is one.
                let _ = SetWindowPos(self.hwnd, Some(HWND_NOTOPMOST), 0, 0, 0, 0, flags);
                let _ = SetWindowPos(self.hwnd, Some(window), 0, 0, 0, 0, flags);
            } else {
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, flags);
            }
        }
    }

    /// Whether it is under every other window now: kept on the desktop, not
    /// lifted.
    fn low(&self) -> bool {
        self.on_desktop.get() && !self.lifted.get()
    }

    /// Kept on the desktop (`true`): under every other window, just above
    /// the desktop's own (so that the desktop shown, its icons and all, it
    /// shows too); else above them all, as it was made.
    pub fn set_on_desktop(&self, on_desktop: bool) {
        self.on_desktop.set(on_desktop);
        if self.low() {
            self.sink();
        } else {
            self.yield_to(self.under.get());
        }
    }

    /// Above them all for now (`true`: while it is moved or sized by hand,
    /// to be seen), or back where it is kept.
    pub fn lift(&self, lifted: bool) {
        if self.lifted.replace(lifted) == lifted || !self.on_desktop.get() {
            return;
        }
        if lifted {
            self.yield_to(self.under.get());
        } else {
            self.sink();
        }
    }

    /// Kept on the desktop: put just above the desktop's own window, wherever
    /// that is among the others (shown with Win+D, it comes in front of
    /// them; see `widget::window_changed`). Nothing done where it is there.
    pub fn sink(&self) {
        use windows::Win32::UI::WindowsAndMessaging::{GetWindow, GetWindowLongPtrW, GW_HWNDPREV, HWND_BOTTOM};
        if !self.low() {
            return;
        }
        let topmost = |window: HWND| unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) } as u32 & WS_EX_TOPMOST.0 != 0;
        let flags = SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE;
        unsafe {
            // Out of the topmost windows first.
            if topmost(self.hwnd) {
                let _ = SetWindowPos(self.hwnd, Some(HWND_NOTOPMOST), 0, 0, 0, 0, flags);
            }
            let after = match desktop_window() {
                Some(desktop) => match GetWindow(desktop, GW_HWNDPREV) {
                    // There already.
                    Ok(above) if above == self.hwnd => return,
                    // Below the window just above the desktop's: between them.
                    Ok(above) if !topmost(above) => above,
                    // The desktop in front of every ordinary window: in
                    // front of them too.
                    _ => HWND_TOP,
                },
                // No desktop (Explorer not running): under all.
                None => HWND_BOTTOM,
            };
            let _ = SetWindowPos(self.hwnd, Some(after), 0, 0, 0, 0, flags);
        }
    }

    /// The pointer's shape over the window from here on: `shape`, one of
    /// the system's (IDC_*).
    pub fn point(&self, shape: PCWSTR) {
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, shape.0 as isize);
            SetCursor(LoadCursorW(None, shape).ok());
        }
    }

    /// Gone for good (one made for nothing: its content could not be).
    pub fn destroy(self) {
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    /// Keeps the window out of every screen capture, or lets it back in.
    pub fn exclude_from_capture(&self, exclude: bool) {
        let affinity = if exclude { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
        // Before Windows 10 2004 there is no excluding; the panel then shows
        // in its own captures, as without live refraction.
        let _ = unsafe { SetWindowDisplayAffinity(self.hwnd, affinity) };
    }

    /// Lets clicks through to whatever is underneath, or takes them.
    pub fn set_click_through(&mut self, through: bool) {
        if self.click_through == through {
            return;
        }
        self.click_through = through;
        unsafe {
            let style = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            let style = if through { style | WS_EX_TRANSPARENT.0 as isize } else { style & !(WS_EX_TRANSPARENT.0 as isize) };
            SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, style);
            let _ = SetWindowPos(self.hwnd, None, 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED);
        }
    }
}

/// The desktop's own window: the one holding its icons (Progman, or the
/// WorkerW Explorer puts them in behind an animated wallpaper).
pub fn desktop_window() -> Option<HWND> {
    use windows::Win32::UI::WindowsAndMessaging::FindWindowExW;
    let icons = |window: HWND| unsafe { FindWindowExW(Some(window), None, w!("SHELLDLL_DefView"), PCWSTR::null()) }.is_ok_and(|view| !view.is_invalid());
    let progman = unsafe { FindWindowExW(None, None, w!("Progman"), PCWSTR::null()) }.ok().filter(|w| !w.is_invalid());
    if let Some(progman) = progman.filter(|w| icons(*w)) {
        return Some(progman);
    }
    let mut worker = None;
    loop {
        worker = unsafe { FindWindowExW(None, worker, w!("WorkerW"), PCWSTR::null()) }.ok().filter(|w| !w.is_invalid());
        match worker {
            Some(w) if icons(w) => return Some(w),
            Some(_) => continue,
            None => return progman,
        }
    }
}
