//! A document looked at as Explorer's preview pane shows it: by the previewer its kind of file has (a PDF's, Word's,
//! Excel's, PowerPoint's, a mail's, a video's that plays it), live (scrolled, its pages turned, played), in a window of
//! its own laid over the sheet's body while the sheet rests. What a previewer draws is its own, drawn in its own
//! window, not by the look: it cannot grow out of a tile or fade with the sheet, so the page's picture stands for it
//! while the sheet moves, and under it while it is made.
//!
//! Each previewer is run as Explorer runs it: in a process of its own where it says it may be (the system's host for
//! previewers, or its program's own), so that one that fails or hangs takes nothing of Glance with it.

use std::path::Path;

use windows::core::{w, Interface, GUID, HSTRING, PWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, CreateSolidBrush, DeleteObject, FillRect, SetWindowRgn, HDC};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER, STGM_READ, STGM_SHARE_DENY_NONE};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithFile, IInitializeWithStream};
use windows::Win32::UI::Shell::{AssocQueryStringW, IInitializeWithItem, IPreviewHandler, IPreviewHandlerVisuals, IShellItem, SHCreateItemFromParsingName, SHCreateStreamOnFileEx, ASSOCF_INIT_DEFAULTTOSTAR, ASSOCF_NOTRUNCATE, ASSOCSTR_SHELLEXTENSION};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindow, GetWindowLongPtrW, GetWindowRect, GW_CHILD, GW_HWNDNEXT, SWP_NOZORDER, RegisterClassW, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWLP_USERDATA, HWND_TOPMOST, MA_NOACTIVATE, SWP_NOACTIVATE,
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

/// The window a previewer draws in, and the previewer in it now (with the file it shows).
pub struct Pane {
    host: HWND,
    shown: Option<(std::path::PathBuf, IPreviewHandler)>,
    /// Where it is (physical px on the screen), as it was last laid there, and whether it shows.
    at: RECT,
    up: bool,
}

impl Pane {
    /// Its window made, kept over `over` (the look's), hidden.
    pub fn new(over: HWND) -> Option<Self> {
        let host = unsafe { make_host(over)? };
        Some(Pane { host, shown: None, at: RECT::default(), up: false })
    }

    pub fn host(&self) -> HWND {
        self.host
    }

    /// The file it shows now, where it shows one.
    pub fn shows(&self) -> Option<&Path> {
        self.shown.as_ref().map(|(path, _)| path.as_path())
    }

    /// Laid at `at` (physical px on the screen), its corners `radius` px round, and shown.
    pub fn show_at(&mut self, at: RECT, radius: i32) {
        self.lay(at, radius);
        self.cover();
        if !self.up {
            self.up = true;
            unsafe {
                let _ = ShowWindow(self.host, SW_SHOWNOACTIVATE);
            }
        }
    }

    /// Laid at `at` (physical px on the screen), its corners `radius` px round, shown or not as it was: before a
    /// previewer is made in it, where it is to show (a previewer lays out what it shows for the window it is given).
    pub fn lay(&mut self, at: RECT, radius: i32) {
        if at != self.at {
            self.at = at;
            let (w, h) = (at.right - at.left, at.bottom - at.top);
            unsafe {
                let _ = SetWindowPos(self.host, Some(HWND_TOPMOST), at.left, at.top, w, h, SWP_NOACTIVATE | SWP_NOOWNERZORDER);
                // (The region the system's from here on: not to be let go of here.)
                SetWindowRgn(self.host, Some(CreateRoundRectRgn(0, 0, w + 1, h + 1, 2 * radius, 2 * radius)), true);
                if let Some((_, handler)) = &self.shown {
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

    pub fn hide(&mut self) {
        if self.up {
            self.up = false;
            unsafe {
                let _ = ShowWindow(self.host, SW_HIDE);
            }
        }
    }

    /// `made` (a previewer made by `load` for `path`) shown in it, from now on, in place of the one there was.
    pub fn took(&mut self, path: &Path, made: IPreviewHandler) {
        self.unload();
        self.shown = Some((path.to_path_buf(), made));
    }

    /// Its previewer let go of (it ends what it shows), and it hidden.
    pub fn unload(&mut self) {
        self.hide();
        self.at = RECT::default();
        if let Some((_, handler)) = self.shown.take() {
            unsafe {
                let _ = handler.Unload();
            }
        }
    }
}

impl Drop for Pane {
    fn drop(&mut self) {
        self.unload();
        unsafe {
            let _ = DestroyWindow(self.host);
        }
    }
}

/// What it is filled with behind what a previewer draws, and what is written with, as they are asked of it.
#[derive(Clone, Copy)]
pub struct Colours {
    pub ground: Color,
    pub text: Color,
}

/// A previewer to be made: the window it is to show in (laid where it is to show, `size` px large), the file, the previewer its kind
/// has, the colours it is asked to show in.
pub struct Wanted {
    pub host: HWND,
    pub path: std::path::PathBuf,
    pub previewer: GUID,
    pub size: (i32, i32),
    pub colours: Colours,
}

/// The previewer `wanted` made, shown: in a process of its own where it says it may be run so, else in this one. Its
/// asking may take a while (a program started for it): the window's thread keeps handling what it is sent meanwhile,
/// as one waiting on another process does.
pub fn load(wanted: &Wanted) -> windows::core::Result<IPreviewHandler> {
    let Wanted { host, path, previewer, size, colours } = wanted;
    let (host, colours) = (*host, *colours);
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

    /// Colours as GDI takes them: red in the low byte.
    #[test]
    fn colours_are_given_as_gdi_takes_them() {
        assert_eq!(colorref(Color::hex(0x102030, 1.0)).0, 0x302010);
    }
}

