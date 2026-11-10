//! The panel's own window: a borderless, topmost tool window that never takes
//! the focus, drawn entirely by DirectComposition.

use std::cell::Cell;

use windows::core::{w, Result, BOOL, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW, IsWindowVisible, LoadCursorW, HWND_TOP, RegisterClassW, SetCursor, SetLayeredWindowAttributes,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE, GWLP_USERDATA, HTCLIENT, HWND_NOTOPMOST, HWND_TOPMOST, IDC_ARROW, LWA_ALPHA,
    MA_NOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_FRAMECHANGED, SW_HIDE, SW_SHOWNOACTIVATE,
    PostMessageW, WM_APP, WM_CAPTURECHANGED, WM_CLOSE, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_MOUSEACTIVATE, WM_NCHITTEST, WM_SETCURSOR, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

/// Posted to the panel's thread when the screens changed (a resolution, a
/// monitor gone; WPARAM 0) or the window's scale did (WPARAM its new DPI,
/// which the panel's own move to another monitor causes too).
pub const SCREENS_CHANGED: u32 = WM_APP + 6;
/// What a window keeps for its pointer's shape where it shows none (no shape's resource number is so low).
const NO_POINTER: isize = 1;

/// Posted to the window's thread when the mouse capture it held was taken
/// from it (or let go).
pub const CAPTURE_LOST: u32 = WM_APP + 8;

/// The capture `hwnd` took let go of, where it still holds it. One another window of its thread has taken since (a
/// menu opened as a button is held, a part torn out of a widget) is that window's, and stays so: the system lets go of
/// whichever window of the thread holds it, and that window, told it lost it, went (a menu closed as it opened).
pub fn let_go_of_capture(hwnd: HWND) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetCapture, ReleaseCapture};
    if unsafe { GetCapture() } == hwnd {
        let _ = unsafe { ReleaseCapture() };
    }
}

/// Posted to the window's thread when it is asked to close as a window is (Alt+F4 while it has the keys): not
/// destroyed (each of Glance's windows goes as what owns it lets it go), what owns it told instead.
pub const CLOSE_ASKED: u32 = WM_APP + 19;

/// The window goes with it: one let go of, however (with what owned it, put away, merged), is never left on the
/// screen (as where a widget would stick was, the widget put away as it was dragged).
impl Drop for Window {
    fn drop(&mut self) {
        LIFTED.lock().unwrap().retain(|&lifted| lifted != self.hwnd.0 as isize);
        // (Whatever it took the keys for, and the to-dos' picker opened from it, let go with it.)
        super::keys::give_back(self.hwnd);
        super::todo::went(self.hwnd);
        let _ = unsafe { DestroyWindow(self.hwnd) };
        super::tip::went(self.hwnd);
    }
}

pub struct Window {
    pub hwnd: HWND,
    click_through: bool,
    /// Made layered, and so always: one that is not is while it lets
    /// clicks through, and only then (see `set_click_through`).
    layered: bool,
    /// The window it gives way to while there is one (see `yield_to`).
    under: Cell<Option<HWND>>,
    /// Where it was last put by `place_keeping`.
    placed: Cell<RECT>,
    /// Kept on the desktop, under every other window (see `set_on_desktop`),
    /// rather than above them all.
    on_desktop: Cell<bool>,
    /// Above them all for now, kept on the desktop or not (see `lift`).
    lifted: Cell<bool>,
    /// Taking the keys (see `take_keys`), and so above them all too.
    keyed: Cell<bool>,
    /// Kept under the panel while it is open (a widget's: see `under_the_panel`).
    under_panel: Cell<bool>,
}

/// How many times a window kept under the panel has come to the top of the windows above all, or been let go from
/// the hand there (see `Window::under_the_panel`): the panel, open, comes over it again as this changes (see `panel`).
static ON_TOP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The windows lifted now, in hand (see `Window::lift`): over every other window of Glance's, the panel's too, however
/// that comes to the top meanwhile (see `came_to_top`).
static LIFTED: std::sync::Mutex<Vec<isize>> = std::sync::Mutex::new(Vec::new());

/// How many times a window kept under the panel has come to the top (see `ON_TOP`).
pub fn on_top() -> u64 {
    ON_TOP.load(std::sync::atomic::Ordering::Relaxed)
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // (Taking the keys, what writes in it answers first: see `keys`.)
    if let Some(answered) = super::keys::answer(hwnd, message, wparam, lparam) {
        return answered;
    }
    match message {
        // Clicking the panel must leave the focus where it was; a window that comes to the front (a menu, the music
        // full screen) comes there as it is clicked, with the keyboard.
        WM_MOUSEACTIVATE => {
            let never = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32 & WS_EX_NOACTIVATE.0 != 0;
            LRESULT(if never { MA_NOACTIVATE } else { windows::Win32::UI::WindowsAndMessaging::MA_ACTIVATE } as isize)
        }
        WM_NCHITTEST => LRESULT(HTCLIENT as isize),
        // The pointer's shape, as the window's owner last set it (see
        // `Window::point`): asked for at every move, before the move itself.
        WM_SETCURSOR => {
            let shape = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
            // (None at all, where it hides the pointer: see `point_none`.)
            if shape == NO_POINTER {
                unsafe { SetCursor(None) };
                return LRESULT(1);
            }
            let shape = if shape == 0 { IDC_ARROW } else { PCWSTR(shape as *const u16) };
            unsafe { SetCursor(LoadCursorW(None, shape).ok()) };
            LRESULT(1)
        }
        WM_CAPTURECHANGED => {
            let _ = unsafe { PostMessageW(Some(hwnd), CAPTURE_LOST, WPARAM(0), LPARAM(0)) };
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            // (Which adapter drives which screen seen again as a device is next asked for: see `gfx::on_screen`.)
            super::gfx::screens_changed();
            let _ = unsafe { PostMessageW(Some(hwnd), SCREENS_CHANGED, WPARAM(0), LPARAM(0)) };
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let _ = unsafe { PostMessageW(Some(hwnd), SCREENS_CHANGED, WPARAM(wparam.0 & 0xFFFF), LPARAM(0)) };
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = unsafe { PostMessageW(Some(hwnd), CLOSE_ASKED, WPARAM(0), LPARAM(0)) };
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
        Ok(Window { hwnd, click_through: false, layered, under: Cell::new(None), placed: Cell::new(RECT::default()), on_desktop: Cell::new(false), lifted: Cell::new(false), keyed: Cell::new(false), under_panel: Cell::new(false) })
    }

    pub fn place(&self, rect: RECT) {
        self.place_under(rect, None);
    }

    /// Kept under the panel while it is open (a widget: one stuck to the edge the panel opens at was over it,
    /// taking its presses, after it came to the top giving way to the settings and back).
    pub fn under_the_panel(&self) {
        self.under_panel.set(true);
    }

    /// Back at the top of the windows above all (but under the window it gives way to): over a window kept under
    /// it that came there since (see `under_the_panel`).
    pub fn stand_again(&self) {
        if !self.low() {
            self.stand(HWND_TOPMOST);
        }
    }

    /// Come to the top: told, where it is kept under the panel (not while it is lifted, in hand: over the panel then).
    /// Another (the panel), the windows in hand put over it again.
    fn came_to_top(&self) {
        if self.under_panel.get() {
            if !self.lifted.get() {
                ON_TOP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            return;
        }
        // (A window of another thread is placed by that thread: asked, not waited for, and not with the list held, which
        // that thread may be waiting on.)
        use windows::Win32::System::Threading::GetCurrentThreadId;
        use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, SWP_ASYNCWINDOWPOS};
        let lifted = LIFTED.lock().unwrap().clone();
        for lifted in lifted.into_iter().filter(|&lifted| lifted != self.hwnd.0 as isize).map(|lifted| HWND(lifted as *mut _)) {
            let elsewhere = unsafe { GetWindowThreadProcessId(lifted, None) } != unsafe { GetCurrentThreadId() };
            let flags = SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | if elsewhere { SWP_ASYNCWINDOWPOS } else { Default::default() };
            unsafe {
                let _ = SetWindowPos(lifted, Some(HWND_TOP), 0, 0, 0, 0, flags);
            }
        }
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
        self.came_to_top();
    }

    pub fn show(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
        if self.low() {
            self.sink();
            return;
        }
        // Above whatever took the top since it was last shown (but the window it gives way to).
        self.stand(HWND_TOPMOST);
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
        self.stand(HWND_TOP);
    }

    /// Placed among the windows above all: just below the window it gives way to, while that one is above all
    /// too (it is, while it is in front: see `settings_window`), else after `otherwise`. It never leaves the windows
    /// above all to give way (placed after an ordinary window, it would be one): with a window held over the whole
    /// screen in front, Windows lets no other window become one of them again, and it stayed under that window, its
    /// edge opening it unseen.
    fn stand(&self, otherwise: HWND) {
        use windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW;
        let above_all = |window: HWND| unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) } as u32 & WS_EX_TOPMOST.0 != 0;
        let after = self.under.get().filter(|under| above_all(*under)).unwrap_or(otherwise);
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(after), 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE);
        }
        self.came_to_top();
    }

    /// The window it gives way to, if it does (see `yield_to`).
    pub fn gives_way_to(&self) -> Option<HWND> {
        self.under.get()
    }

    /// Whether it is giving way to another window (see `yield_to`).
    pub fn yielding(&self) -> bool {
        self.under.get().is_some()
    }

    /// Where it is among the windows, said for the journal: whether it is above all (topmost), and the window it
    /// gives way to, if it does.
    pub fn standing(&self) -> String {
        use windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW;
        let topmost = unsafe { GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) } as u32 & WS_EX_TOPMOST.0 != 0;
        format!("{}{}", if topmost { "above all" } else { "not above all" }, self.under.get().map_or(String::new(), |under| format!(", giving way to {:?}", under.0)))
    }

    /// Gives way to `window` (the settings, above all while they are in front), just below it, until it is given
    /// way to no more (`None`): at the top of the windows above all again. (Above all throughout: see `stand`.)
    pub fn yield_to(&self, window: Option<HWND>) {
        self.under.set(window);
        // On the desktop, it is under that window already.
        if self.low() {
            return;
        }
        self.stand(HWND_TOPMOST);
    }

    /// Whether it is under every other window now: kept on the desktop, not
    /// lifted, not taking the keys.
    fn low(&self) -> bool {
        self.on_desktop.get() && !self.lifted.get() && !self.keyed.get()
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
    /// to be seen), the panel too, or back where it is kept (under the panel again, kept under it).
    pub fn lift(&self, lifted: bool) {
        if self.lifted.replace(lifted) != lifted {
            let mut held = LIFTED.lock().unwrap();
            held.retain(|&window| window != self.hwnd.0 as isize);
            if lifted {
                held.push(self.hwnd.0 as isize);
            }
            drop(held);
            self.stand_as_kept();
            if !lifted && !self.low() {
                self.came_to_top();
            }
        }
    }

    /// Where it is kept now, it kept on the desktop: under the others where it is low (see `low`), else above them.
    fn stand_as_kept(&self) {
        if !self.on_desktop.get() {
            return;
        }
        if self.low() {
            self.sink();
        } else {
            self.yield_to(self.under.get());
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
    /// Shows no pointer over it (`none`), or the arrow, from the pointer's next move over it on, and at once where
    /// it is over it (`over`): told elsewhere, the pointer over another window would come and go.
    pub fn point_none(&self, none: bool, over: bool) {
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, if none { NO_POINTER } else { 0 });
            if over {
                SetCursor(if none { None } else { LoadCursorW(None, IDC_ARROW).ok() });
            }
        }
    }

    /// Brings it to the front with the keyboard's focus (one made `activating`, as the user just asked for it).
    pub fn focus(&self) {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow(self.hwnd);
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(self.hwnd));
        }
    }

    /// Taking the keys (`on`: brought to the front with the keyboard's focus, clicks on it leaving them there), or
    /// never taking the focus again, as it was made (see `keys`). Kept on the desktop, it is above the others while it
    /// takes them (as while it is moved: see `lift`), back under them after.
    pub fn take_keys(&self, on: bool) {
        self.let_take_keys(on);
        if on {
            self.focus();
        }
    }

    /// Let take the focus or not (see `take_keys`), and standing so, but not brought to the front.
    fn let_take_keys(&self, on: bool) {
        unsafe {
            let style = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) as u32;
            let style = if on { style & !WS_EX_NOACTIVATE.0 } else { style | WS_EX_NOACTIVATE.0 };
            SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, style as isize);
        }
        if self.keyed.replace(on) != on {
            self.stand_as_kept();
        }
    }

    pub fn point(&self, shape: PCWSTR) {
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, shape.0 as isize);
            SetCursor(LoadCursorW(None, shape).ok());
        }
    }

    /// Gone for good (one made for nothing: its content could not be).
    /// Destroys it (as letting go of it does: see `Drop`).
    pub fn destroy(self) {}

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        super::tip::went(self.hwnd);
    }

    /// Lets clicks through to whatever is underneath, or takes them. Only
    /// a layered window lets them through to another program's: one made
    /// without the style has it for as long as it does (given no layer
    /// attributes, it keeps the backdrop the compositor draws into it).
    pub fn set_click_through(&mut self, through: bool) {
        if self.click_through == through {
            return;
        }
        self.click_through = through;
        unsafe {
            let style = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            let styles = if self.layered { WS_EX_TRANSPARENT.0 } else { WS_EX_TRANSPARENT.0 | WS_EX_LAYERED.0 } as isize;
            let style = if through { style | styles } else { style & !styles };
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Asked to close as a window is (Alt+F4), it is not destroyed: its thread is told instead.
    #[test]
    fn a_window_asked_to_close_is_not_destroyed_but_told() {
        use windows::Win32::UI::WindowsAndMessaging::{IsWindow, PeekMessageW, SendMessageW, MSG, PM_REMOVE};
        let window = Window::new().unwrap();
        unsafe { SendMessageW(window.hwnd, WM_CLOSE, None, None) };
        assert!(unsafe { IsWindow(Some(window.hwnd)) }.as_bool());
        let mut message = MSG::default();
        assert!(unsafe { PeekMessageW(&mut message, Some(window.hwnd), CLOSE_ASKED, CLOSE_ASKED, PM_REMOVE) }.as_bool());
    }

    /// A window letting go of the capture it took leaves it with another window of its thread that has taken it since
    /// (a menu opened over the panel as the bar's thumb was held): let go of all the same, the menu, told it lost it,
    /// closed as it opened. Its own it lets go of.
    #[test]
    fn a_window_lets_go_only_of_the_capture_it_holds() {
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetCapture, SetCapture};
        let (panel, menu) = (Window::new().unwrap(), Window::new().unwrap());
        unsafe { SetCapture(panel.hwnd) };
        assert_eq!(unsafe { GetCapture() }, panel.hwnd, "the capture not taken");
        unsafe { SetCapture(menu.hwnd) };
        let_go_of_capture(panel.hwnd);
        let kept = unsafe { GetCapture() } == menu.hwnd;
        let_go_of_capture(menu.hwnd);
        let gone = unsafe { GetCapture() }.is_invalid();
        menu.destroy();
        panel.destroy();
        assert!(kept, "the capture another window took since let go of");
        assert!(gone, "a window's own capture kept");
    }

    /// A window taking the keys takes the focus as it is clicked (its style no longer asks never to), and once it gives
    /// them back never again. Kept on the desktop, it stands above the others meanwhile, under them again after. (Not
    /// brought to the front here: that would take the front from whatever has it as the tests run.)
    #[test]
    fn a_window_taking_the_keys_takes_the_focus_while_it_does() {
        let window = Window::new().unwrap();
        let never = || unsafe { GetWindowLongPtrW(window.hwnd, GWL_EXSTYLE) } as u32 & WS_EX_NOACTIVATE.0 != 0;
        let clicked = || unsafe { procedure(window.hwnd, WM_MOUSEACTIVATE, WPARAM(0), LPARAM(0)) }.0 as u32;
        assert!(never());
        assert_eq!(clicked(), MA_NOACTIVATE);
        window.set_on_desktop(true);
        assert!(window.low());
        window.let_take_keys(true);
        assert!(!never());
        assert_eq!(clicked(), windows::Win32::UI::WindowsAndMessaging::MA_ACTIVATE);
        assert!(!window.low());
        window.let_take_keys(false);
        assert!(never());
        assert_eq!(clicked(), MA_NOACTIVATE);
        assert!(window.low());
    }

    /// A window kept under the panel, lifted into the hand, leaves the panel where it is (the panel is not told to come
    /// over it), and is over the panel come to the top even so: as one pulled off the panel is, which came to the top
    /// before it was lifted, made and given way to first, and the panel came over it as told. Let go, the panel is told
    /// and comes over it again. (One test: what the panel is told is counted for the whole process.)
    #[test]
    fn a_window_in_hand_is_over_the_panel_till_it_is_let_go() {
        use windows::Win32::UI::WindowsAndMessaging::{GetWindow, GW_HWNDPREV};
        // Whether `upper` is above `lower` among the windows.
        let above = |upper: HWND, lower: HWND| {
            let mut at = lower;
            while let Ok(prev) = unsafe { GetWindow(at, GW_HWNDPREV) }.map_err(|_| ()).and_then(|prev| if prev.is_invalid() { Err(()) } else { Ok(prev) }) {
                if prev == upper {
                    return true;
                }
                at = prev;
            }
            false
        };
        let (panel, widget) = (Window::new().unwrap(), Window::new().unwrap());
        widget.under_the_panel();
        widget.yield_to(None);
        let before = on_top();
        widget.lift(true);
        widget.raise();
        widget.show();
        let told = on_top();
        panel.stand_again();
        let held = above(widget.hwnd, panel.hwnd);
        widget.lift(false);
        let let_go = on_top();
        panel.stand_again();
        let under = above(panel.hwnd, widget.hwnd);
        widget.destroy();
        panel.destroy();
        assert_eq!(told, before);
        assert!(held);
        assert_eq!(let_go, before + 1);
        assert!(under);
    }
}
