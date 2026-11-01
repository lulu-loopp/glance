//! A document looked at as Explorer's preview pane shows it: by the previewer its kind of file has (a PDF's, Word's,
//! Excel's, PowerPoint's, a mail's, a video's that plays it), live (scrolled, its pages turned, played), in a window of
//! its own laid over the sheet's body while the sheet rests. What a previewer draws is its own, drawn in its own
//! window, not by the look: it cannot grow out of a tile or fade with the sheet, so the page's picture stands for it
//! while the sheet moves, and under it while it is made.
//!
//! Each previewer is run as Explorer runs it: in a process of its own where it says it may be (the system's host for
//! previewers, or its program's own), so that one that fails or hangs takes nothing of Glance with it; and made, its
//! file opened, on a thread of the pane's own, so that a drive slow to answer holds nothing of the look.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};

use windows::core::{w, Interface, GUID, HSTRING, PWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, CreateSolidBrush, DeleteObject, FillRect, SetWindowRgn, HDC};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER, STGM_READ, STGM_SHARE_DENY_NONE};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithFile, IInitializeWithStream};
use windows::Win32::UI::Shell::{AssocQueryStringW, IInitializeWithItem, IPreviewHandler, IPreviewHandlerVisuals, IShellItem, SHCreateItemFromParsingName, SHCreateStreamOnFileEx, ASSOCF_INIT_DEFAULTTOSTAR, ASSOCF_NOTRUNCATE, ASSOCSTR_SHELLEXTENSION};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, PostMessageW, TranslateMessage, MSG, WM_APP, GetWindow, GetWindowLongPtrW, GetWindowRect, GW_CHILD, GW_HWNDNEXT, SWP_NOZORDER, RegisterClassW, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWLP_USERDATA, HWND_TOPMOST, MA_NOACTIVATE, SWP_NOACTIVATE,
    SWP_NOOWNERZORDER, SW_HIDE, SW_SHOWNOACTIVATE, WM_ERASEBKGND, WM_MOUSEACTIVATE, WNDCLASSW, WS_CLIPCHILDREN, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use super::super::canvas::Color;

/// The previewer a file of `path`'s kind has, as Explorer's preview pane finds it (by its kind's shell extension for
/// `IPreviewHandler`): none for a kind that has none.
pub fn previewer(path: &Path) -> Option<GUID> {
    let extension = path.extension()?;
    let kind = HSTRING::from(format!(".{}", extension.to_string_lossy()));
    let wanted = HSTRING::from(format!("{{{:?}}}", IPreviewHandler::IID));
    let mut said = [0u16; 64];
    let mut length = said.len() as u32;
    let found = unsafe { AssocQueryStringW(ASSOCF_INIT_DEFAULTTOSTAR | ASSOCF_NOTRUNCATE, ASSOCSTR_SHELLEXTENSION, &kind, &wanted, Some(PWSTR(said.as_mut_ptr())), &mut length) };
    if found.is_err() {
        return None;
    }
    let said = String::from_utf16_lossy(&said[..said.iter().position(|&unit| unit == 0).unwrap_or(said.len())]);
    GUID::try_from(said.trim().trim_start_matches('{').trim_end_matches('}')).ok()
}

/// The window a previewer draws in, run on a thread of its own (see `run`): a previewer is made, and the file it shows
/// opened, there, never on the look's (a drive slow to answer, or a previewer slow to start, holds nothing of the
/// look); what it is asked is done there in the order asked. And what the look knows of it: the file whose previewer it
/// took, where it was last laid, whether it shows, and what it is told as a previewer it asked for is made.
pub struct Pane {
    host: HWND,
    asks: Sender<Do>,
    made: Receiver<(PathBuf, bool)>,
    shown: Option<PathBuf>,
    at: RECT,
    up: bool,
}

/// What its thread is asked (see `Hosted`).
enum Do {
    Lay(RECT, i32),
    Show,
    Hide,
    /// A previewer made as `Wanted` says (the one it had let go of first), kept aside till it is taken.
    Make(Wanted),
    /// The one made last taken, shown from now on (or let go of).
    Take,
    Leave,
    Unload,
    Quit,
}

/// Its thread's message telling it it was asked something (see `Do`).
const ASKED: u32 = WM_APP + 1;

impl Pane {
    /// Its window made, on its thread (started for it), kept over `over` (the look's), hidden; `over` told `told` as a
    /// previewer it asked for is made (see `made`).
    pub fn new(over: HWND, told: u32) -> Option<Self> {
        let (asks, asked) = std::sync::mpsc::channel();
        let (tell, made) = std::sync::mpsc::channel();
        let (tell_host, host) = std::sync::mpsc::channel();
        let over_at = over.0 as isize;
        std::thread::Builder::new().name("glance: look pane".into()).spawn(move || run(HWND(over_at as *mut _), told, asked, tell, tell_host)).ok()?;
        let host = HWND(host.recv().ok()?? as *mut _);
        Some(Pane { host, asks, made, shown: None, at: RECT::default(), up: false })
    }

    pub fn host(&self) -> HWND {
        self.host
    }

    /// The file it shows now, where it shows one.
    pub fn shows(&self) -> Option<&Path> {
        self.shown.as_deref()
    }

    fn ask(&self, what: Do) {
        if self.asks.send(what).is_ok() {
            unsafe {
                let _ = PostMessageW(Some(self.host), ASKED, WPARAM(0), LPARAM(0));
            }
        }
    }

    /// Laid at `at` (physical px on the screen), its corners `radius` px round, and shown.
    pub fn show_at(&mut self, at: RECT, radius: i32) {
        self.lay(at, radius);
        self.up = true;
        self.ask(Do::Show);
    }

    /// Laid at `at` (physical px on the screen), its corners `radius` px round, shown or not as it was: before a
    /// previewer is made in it, where it is to show (a previewer lays out what it shows for the window it is given).
    pub fn lay(&mut self, at: RECT, radius: i32) {
        if at != self.at {
            self.at = at;
            self.ask(Do::Lay(at, radius));
        }
    }

    pub fn hide(&mut self) {
        if self.up {
            self.up = false;
            self.ask(Do::Hide);
        }
    }

    /// A previewer made as `wanted` says, on its thread: told as it is made (see `made`), to be taken or left.
    pub fn make(&mut self, wanted: Wanted) {
        self.ask(Do::Make(wanted));
    }

    /// The previewer last asked for, as it is made: for which file, and whether it could be. None till then.
    pub fn made(&self) -> Option<(PathBuf, bool)> {
        self.made.try_recv().ok()
    }

    /// The previewer made for `path` (see `made`) shown in it, from now on, in place of the one there was.
    pub fn take(&mut self, path: &Path) {
        self.shown = Some(path.to_path_buf());
        self.ask(Do::Take);
    }

    /// The previewer made (see `made`) let go of: no longer wanted.
    pub fn leave(&mut self) {
        self.ask(Do::Leave);
    }

    /// Its previewer let go of (it ends what it shows), and it hidden.
    pub fn unload(&mut self) {
        self.up = false;
        self.at = RECT::default();
        self.shown = None;
        self.ask(Do::Unload);
    }
}

impl Drop for Pane {
    fn drop(&mut self) {
        self.ask(Do::Quit);
    }
}

/// What its thread holds: its window, where it was last laid, the previewer in it, the one made and not yet taken,
/// and how it tells the look of one made.
struct Hosted {
    host: HWND,
    at: RECT,
    shown: Option<IPreviewHandler>,
    made: Option<IPreviewHandler>,
    up: bool,
    asked: Receiver<Do>,
    over: HWND,
    told: u32,
    tell: Sender<(PathBuf, bool)>,
}

thread_local! {
    static HOSTED: RefCell<Option<Hosted>> = const { RefCell::new(None) };
}

/// Its thread: its window made over `over` (told to `tell_host`), and what it is asked done as it is (see `Do`), till
/// it is asked to quit.
fn run(over: HWND, told: u32, asked: Receiver<Do>, tell: Sender<(PathBuf, bool)>, tell_host: Sender<Option<isize>>) {
    // (Previewers expect a single-threaded apartment, as Explorer's preview pane has.)
    let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) };
    let Some(host) = (unsafe { make_host(over) }) else {
        let _ = tell_host.send(None);
        return;
    };
    HOSTED.with(|hosted| *hosted.borrow_mut() = Some(Hosted { host, at: RECT::default(), shown: None, made: None, up: false, asked, over, told, tell }));
    let _ = tell_host.send(Some(host.0 as isize));
    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// What its thread was asked done, in order (see `Do`). A previewer being made (which may take a while: a program
/// started for it) keeps handling what the thread is sent meanwhile, as one waiting on another process does: what is
/// asked then is done after.
fn asked() {
    loop {
        let next = HOSTED.with(|hosted| hosted.try_borrow().ok().and_then(|hosted| hosted.as_ref().and_then(|hosted| hosted.asked.try_recv().ok())));
        let Some(what) = next else { return };
        match what {
            Do::Make(wanted) => {
                // (Made with its state let go of: it may take a while, and the thread is sent things meanwhile.)
                let path = wanted.path.clone();
                let Some(host) = HOSTED.with(|hosted| hosted.borrow().as_ref().map(|hosted| hosted.host)) else { return };
                let made = load(host, &wanted);
                HOSTED.with(|hosted| {
                    let mut hosted = hosted.borrow_mut();
                    // (Asked to quit meanwhile: let go of at once.)
                    let Some(hosted) = hosted.as_mut() else {
                        if let Ok(made) = &made {
                            unsafe {
                                let _ = made.Unload();
                            }
                        }
                        return;
                    };
                    let ok = made.is_ok();
                    if let Some(old) = std::mem::replace(&mut hosted.made, made.ok()) {
                        unsafe {
                            let _ = old.Unload();
                        }
                    }
                    if hosted.tell.send((path, ok)).is_ok() {
                        unsafe {
                            let _ = PostMessageW(Some(hosted.over), hosted.told, WPARAM(0), LPARAM(0));
                        }
                    }
                });
            }
            Do::Quit => {
                let hosted = HOSTED.with(|hosted| hosted.borrow_mut().take());
                if let Some(mut hosted) = hosted {
                    hosted.unload();
                    hosted.leave();
                    unsafe {
                        let _ = DestroyWindow(hosted.host);
                        windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0);
                    }
                }
                return;
            }
            what => HOSTED.with(|hosted| {
                if let Some(hosted) = hosted.borrow_mut().as_mut() {
                    hosted.does(what);
                }
            }),
        }
    }
}

impl Hosted {
    fn does(&mut self, what: Do) {
        match what {
            Do::Lay(at, radius) => self.lay(at, radius),
            Do::Show => {
                self.cover();
                if !self.up {
                    self.up = true;
                    unsafe {
                        let _ = ShowWindow(self.host, SW_SHOWNOACTIVATE);
                    }
                }
            }
            Do::Hide => self.hide(),
            Do::Take => {
                if let Some(made) = self.made.take() {
                    self.unload();
                    self.shown = Some(made);
                }
            }
            Do::Leave => self.leave(),
            Do::Unload => self.unload(),
            Do::Make(_) | Do::Quit => {}
        }
    }

    fn lay(&mut self, at: RECT, radius: i32) {
        if at != self.at {
            self.at = at;
            let (w, h) = (at.right - at.left, at.bottom - at.top);
            unsafe {
                let _ = SetWindowPos(self.host, Some(HWND_TOPMOST), at.left, at.top, w, h, SWP_NOACTIVATE | SWP_NOOWNERZORDER);
                round(self.host, w, h, radius);
                if let Some(handler) = &self.shown {
                    let _ = handler.SetRect(&RECT { left: 0, top: 0, right: w, bottom: h });
                }
            }
        }
    }

    /// What its previewer shows laid over all of it, as the previewer was asked (`SetWindow`, `SetRect`). One run in a
    /// process of its own that knows only the main screen's scale (Edge's, for PDFs) lays its window where that scale
    /// would put it: on a screen of another scale, or away from the main one, off it, and nothing shows.
    fn cover(&self) {
        let (w, h) = (self.at.right - self.at.left, self.at.bottom - self.at.top);
        let mut child = unsafe { GetWindow(self.host, GW_CHILD) }.ok();
        while let Some(window) = child {
            let mut at = RECT::default();
            unsafe {
                if GetWindowRect(window, &mut at).is_ok() && at != self.at {
                    let _ = SetWindowPos(window, None, 0, 0, w, h, SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER);
                }
            }
            child = unsafe { GetWindow(window, GW_HWNDNEXT) }.ok();
        }
    }

    fn hide(&mut self) {
        if self.up {
            self.up = false;
            unsafe {
                let _ = ShowWindow(self.host, SW_HIDE);
            }
        }
    }

    /// The previewer made and not taken let go of.
    fn leave(&mut self) {
        if let Some(made) = self.made.take() {
            unsafe {
                let _ = made.Unload();
            }
        }
    }

    /// Its previewer let go of (it ends what it shows), and it hidden.
    fn unload(&mut self) {
        self.hide();
        self.at = RECT::default();
        if let Some(handler) = self.shown.take() {
            unsafe {
                let _ = handler.Unload();
            }
        }
    }
}

/// What it is filled with behind what a previewer draws, and what is written with, as they are asked of it.
#[derive(Clone, Copy)]
pub struct Colours {
    pub ground: Color,
    pub text: Color,
}

/// A previewer to be made (in its window, laid where it is to show, `size` px large): the file, the previewer its kind
/// has, the colours it is asked to show in.
pub struct Wanted {
    pub path: std::path::PathBuf,
    pub previewer: GUID,
    pub size: (i32, i32),
    pub colours: Colours,
}

/// The previewer `wanted` made, shown: in a process of its own where it says it may be run so, else in this one. Its
/// asking may take a while (a program started for it): the pane's thread keeps handling what it is sent meanwhile,
/// as one waiting on another process does.
fn load(host: HWND, wanted: &Wanted) -> windows::core::Result<IPreviewHandler> {
    let Wanted { path, previewer, size, colours } = wanted;
    let colours = *colours;
    unsafe {
        SetWindowLongPtrW(host, GWLP_USERDATA, colorref(colours.ground).0 as isize);
        let handler: IPreviewHandler = CoCreateInstance(previewer, None, CLSCTX_LOCAL_SERVER).or_else(|_| CoCreateInstance(previewer, None, CLSCTX_INPROC_SERVER))?;
        initialize(&handler, path)?;
        if let Ok(visuals) = handler.cast::<IPreviewHandlerVisuals>() {
            let _ = visuals.SetBackgroundColor(colorref(colours.ground));
            let _ = visuals.SetTextColor(colorref(colours.text));
        }
        let rect = RECT { left: 0, top: 0, right: size.0, bottom: size.1 };
        handler.SetWindow(host, &rect)?;
        if let Err(error) = handler.DoPreview() {
            let _ = handler.Unload();
            return Err(error);
        }
        Ok(handler)
    }
}

/// `handler` told the file at `path`, as it may be: a stream of it read (what Explorer gives one run apart from it), else
/// its path, else the shell's item for it.
unsafe fn initialize(handler: &IPreviewHandler, path: &Path) -> windows::core::Result<()> {
    let name = HSTRING::from(path.as_os_str());
    let mut failed = windows::core::Error::from_hresult(windows::Win32::Foundation::E_NOINTERFACE);
    unsafe {
        if let Ok(with) = handler.cast::<IInitializeWithStream>() {
            match SHCreateStreamOnFileEx(&name, (STGM_READ | STGM_SHARE_DENY_NONE).0, 0, false, None).and_then(|stream| with.Initialize(&stream, STGM_READ.0)) {
                Ok(()) => return Ok(()),
                Err(error) => failed = error,
            }
        }
        if let Ok(with) = handler.cast::<IInitializeWithFile>() {
            match with.Initialize(&name, STGM_READ.0) {
                Ok(()) => return Ok(()),
                Err(error) => failed = error,
            }
        }
        if let Ok(with) = handler.cast::<IInitializeWithItem>() {
            match SHCreateItemFromParsingName::<_, _, IShellItem>(&name, None).and_then(|item| with.Initialize(&item, STGM_READ.0)) {
                Ok(()) => return Ok(()),
                Err(error) => failed = error,
            }
        }
    }
    Err(failed)
}

/// `host` (`w` × `h` px) given corners `radius` px round: drawn smooth by the system where it rounds windows' corners
/// (Windows 11: its own two radii, round and small, as a look's body has them), else cut by a region.
unsafe fn round(host: HWND, w: i32, h: i32, radius: i32) {
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUND, DWMWCP_ROUNDSMALL};
    let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(host) }.max(96) as f32 / 96.0;
    let corners = match radius as f32 / dpi {
        r if r < 2.0 => DWMWCP_DONOTROUND,
        r if r < 6.0 => DWMWCP_ROUNDSMALL,
        _ => DWMWCP_ROUND,
    };
    unsafe {
        let set = DwmSetWindowAttribute(host, DWMWA_WINDOW_CORNER_PREFERENCE, std::ptr::from_ref(&corners).cast(), std::mem::size_of_val(&corners) as u32);
        if set.is_ok() {
            let none = DWMWA_COLOR_NONE;
            let _ = DwmSetWindowAttribute(host, DWMWA_BORDER_COLOR, std::ptr::from_ref(&none).cast(), std::mem::size_of_val(&none) as u32);
        } else {
            // (The region the system's from here on: not to be let go of here.)
            SetWindowRgn(host, Some(CreateRoundRectRgn(0, 0, w + 1, h + 1, 2 * radius, 2 * radius)), true);
        }
    }
}

fn colorref(color: Color) -> COLORREF {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    COLORREF(byte(color.r) | byte(color.g) << 8 | byte(color.b) << 16)
}

unsafe fn make_host(over: HWND) -> Option<HWND> {
    unsafe {
        let module = GetModuleHandleW(None).ok()?;
        let class = w!("GlanceLookPane");
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(procedure), hInstance: module.into(), lpszClassName: class, ..Default::default() });
        // (Owned by the look's window, so kept over it, and never taking the keyboard from what called the look up.)
        let style = WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        CreateWindowExW(style, class, w!("Glance"), WS_POPUP | WS_CLIPCHILDREN, 0, 0, 1, 1, Some(over), None, Some(module.into()), None).ok()
    }
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        ASKED => {
            asked();
            LRESULT(0)
        }
        // A click on what a previewer shows leaves the keyboard where it was (with what called the look up).
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => {
            let ground = COLORREF(unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as u32);
            unsafe { fill(hwnd, HDC(wparam.0 as *mut _), ground) };
            LRESULT(1)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

unsafe fn fill(hwnd: HWND, dc: HDC, ground: COLORREF) {
    unsafe {
        let mut rect = RECT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rect);
        let brush = CreateSolidBrush(ground);
        FillRect(dc, &rect, brush);
        let _ = DeleteObject(brush.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A kind of file no previewer is for has none. One is found by the kind's name whatever its case.
    #[test]
    fn previewers_are_found_by_kind() {
        assert_eq!(previewer(Path::new(r"C:\a\b.glance-no-such-kind")), None);
        assert_eq!(previewer(Path::new(r"C:\a\noname")), None);
        assert_eq!(previewer(Path::new(r"C:\a\Report.PDF")), previewer(Path::new(r"C:\a\report.pdf")));
    }

    /// A previewer is made on the pane's thread, its window there, the window it is over told as it is made, what it
    /// shows in the pane's window (left hidden here: nothing shows on the screen); the pane gone, its window and its
    /// thread go.
    #[test]
    fn a_previewer_is_made_on_the_pane_s_thread() {
        use windows::Win32::UI::WindowsAndMessaging::{IsWindow, PeekMessageW, WINDOW_EX_STYLE, PM_REMOVE};
        let folder = std::env::temp_dir().join(format!("glance-look-pane-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("a.txt");
        std::fs::write(&path, "words to be previewed").unwrap();
        let previewer = previewer(&path).expect("Windows' previewer for text");
        let over = unsafe { CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WS_POPUP, 0, 0, 400, 300, None, None, None, None) }.unwrap();
        const MADE: u32 = WM_APP + 9;
        let mut pane = Pane::new(over, MADE).expect("its thread and window");
        let host = pane.host();
        let thread = |window: HWND| unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(window, None) };
        assert_ne!(thread(host), unsafe { windows::Win32::System::Threading::GetCurrentThreadId() }, "its window on its own thread");
        let at = RECT { left: 100, top: 100, right: 500, bottom: 400 };
        pane.lay(at, 0);
        pane.make(Wanted { path: path.clone(), previewer, size: (400, 300), colours: Colours { ground: Color::hex(0xFFFFFF, 1.0), text: Color::hex(0x000000, 1.0) } });
        // (Told as it is made: this thread's messages looked at meanwhile.)
        let began = std::time::Instant::now();
        let made = loop {
            let mut msg = MSG::default();
            while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
                if msg.message == MADE {
                    assert_eq!(msg.hwnd, over);
                }
            }
            if let Some(made) = pane.made() {
                break made;
            }
            assert!(began.elapsed() < std::time::Duration::from_secs(20), "made within twenty seconds");
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert_eq!(made, (path.clone(), true));
        pane.take(&path);
        assert_eq!(pane.shows(), Some(path.as_path()));
        let shown = (0..250).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(20));
            unsafe { GetWindow(host, GW_CHILD) }.is_ok()
        });
        assert!(shown, "what it shows in its window");
        drop(pane);
        let gone = (0..250).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(20));
            !unsafe { IsWindow(Some(host)) }.as_bool()
        });
        assert!(gone, "its window gone with it");
        unsafe {
            let _ = DestroyWindow(over);
        }
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Colours as GDI takes them: red in the low byte.
    #[test]
    fn colours_are_given_as_gdi_takes_them() {
        assert_eq!(colorref(Color::hex(0x102030, 1.0)).0, 0x302010);
    }
}

