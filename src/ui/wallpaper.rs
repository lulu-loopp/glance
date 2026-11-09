//! The desktop the settings' preview stands in for: the wallpaper as the
//! desktop shows it (a live one, a video or a scene, as it is just then),
//! else its picture scaled to fill the work area as Windows does by default,
//! or the plain desktop colour.

use windows::core::{w, Interface, Result, HSTRING};
use windows::Win32::Foundation::{GENERIC_READ, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetSysColor, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    COLOR_DESKTOP, DIB_RGB_COLORS,
};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppBGR, GUID_WICPixelFormat32bppPBGRA, IWICBitmapSource, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand, WICRect,
};
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetWindowRect, SystemParametersInfoW, PW_RENDERFULLCONTENT, SPI_GETDESKWALLPAPER,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
};

use super::backdrop::Capture;

/// The desktop of each of `asked` (an area of it in physical pixels, and the picture's width and height) as a picture
/// that large, a live wallpaper printed once for them all. Needs COM on the calling thread.
pub fn desktops(asked: &[(RECT, u32, u32)]) -> Vec<Capture> {
    let printed = print(&asked.iter().map(|&(area, ..)| area).collect::<Vec<_>>());
    asked.iter().map(|&(work, width, height)| desktop_from(printed.as_ref(), work, width, height)).collect()
}

/// The desktop of `work` as a picture `width` × `height` pixels large, from the live wallpaper as `printed` holds it,
/// if there is one.
fn desktop_from(printed: Option<&Printed>, work: RECT, width: u32, height: u32) -> Capture {
    let rect = RECT { left: 0, top: 0, right: width as i32, bottom: height as i32 };
    let pixels = printed
        .and_then(|printed| shown_in(printed, work, width, height))
        .or_else(|| wallpaper_path().and_then(|path| file(&path, width, height).ok()))
        .unwrap_or_else(|| {
            // GetSysColor answers 0x00BBGGRR; the picture is BGRA.
            let rgb = unsafe { GetSysColor(COLOR_DESKTOP) };
            let pixel = [((rgb >> 16) & 0xFF) as u8, ((rgb >> 8) & 0xFF) as u8, (rgb & 0xFF) as u8, 255];
            pixel.repeat((width * height) as usize)
        });
    Capture::from_pixels(rect, pixels)
}

/// The picture at `path`, scaled to cover `width` × `height` pixels, as the
/// desktop. For the studio, which films the panel over a chosen picture.
#[cfg(feature = "studio")]
pub fn picture(path: &str, width: u32, height: u32) -> Result<Capture> {
    let rect = RECT { left: 0, top: 0, right: width as i32, bottom: height as i32 };
    Ok(Capture::from_pixels(rect, file(path, width, height)?))
}

/// The window a live wallpaper (Wallpaper Engine, Lively, …) is drawn in,
/// behind the desktop's icons: a WorkerW inside Progman since Windows 11
/// 24H2, before that the WorkerW after the one holding the icons. `None`
/// without a live wallpaper, where the desktop shows the wallpaper's picture.
fn live_layer() -> Option<HWND> {
    unsafe {
        let progman = FindWindowW(w!("Progman"), None).ok()?;
        if let Ok(layer) = FindWindowExW(Some(progman), None, w!("WorkerW"), None) {
            return Some(layer);
        }
        let mut after = None;
        while let Ok(worker) = FindWindowExW(None, after, w!("WorkerW"), None) {
            if FindWindowExW(Some(worker), None, w!("SHELLDLL_DefView"), None).is_ok() {
                return FindWindowExW(None, Some(worker), w!("WorkerW"), None).ok();
            }
            after = Some(worker);
        }
        None
    }
}

/// The live wallpaper layer as the desktop shows it just then (`print`), printed once for every screen read from it:
/// its pixels (rows top to bottom, BGRX), where what was printed of it is on the desktop, and the GDI objects that
/// hold them.
struct Printed {
    memory: windows::Win32::Graphics::Gdi::HDC,
    bitmap: windows::Win32::Graphics::Gdi::HBITMAP,
    previous: windows::Win32::Graphics::Gdi::HGDIOBJ,
    bits: *const u8,
    at: RECT,
}

impl Drop for Printed {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.memory, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.memory);
        }
    }
}

/// The live wallpaper's layer printed as far as the areas `asked` of the desktop take (see `printed_extent`), with what
/// DirectX draws in it as the screen shows it. `None` without a live wallpaper (see `live_layer`), where none of them
/// is in it, or if it could not be printed.
fn print(asked: &[RECT]) -> Option<Printed> {
    let layer = live_layer()?;
    let mut whole = RECT::default();
    unsafe { GetWindowRect(layer, &mut whole) }.ok()?;
    let at = printed_extent(whole, asked)?;
    let (w, h) = (at.right - at.left, at.bottom - at.top);
    let header = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: w,
        // Negative: rows run top to bottom.
        biHeight: -h,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };
    let info = BITMAPINFO { bmiHeader: header, ..Default::default() };
    unsafe {
        let memory = CreateCompatibleDC(None);
        let mut bits = std::ptr::null_mut();
        let Ok(bitmap) = CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(memory);
            return None;
        };
        let previous = SelectObject(memory, bitmap.into());
        let printed = Printed { memory, bitmap, previous, bits: bits as *const u8, at };
        // Full content: what DirectX draws in it too, as the screen shows it.
        let done = PrintWindow(layer, memory, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool();
        // (What GDI has yet to do in the bitmap, done before its bits are read.)
        let _ = windows::Win32::Graphics::Gdi::GdiFlush();
        done.then_some(printed)
    }
}

/// What of a live wallpaper's layer at `layer` (physical px) is printed for the areas `asked` of the desktop: from its
/// corner as far right and down as the farthest of those in it reaches. PrintWindow draws a window from its own corner
/// at the corner of what it draws into, whatever origin that is given, so less of the layer is printed only by stopping
/// short of its far edges. None, where none of them is in it (none would be read from it: see `shown_in`).
fn printed_extent(layer: RECT, asked: &[RECT]) -> Option<RECT> {
    let inside = |area: &&RECT| area.left >= layer.left && area.top >= layer.top && area.right <= layer.right && area.bottom <= layer.bottom && area.right > area.left && area.bottom > area.top;
    let right = asked.iter().filter(inside).map(|area| area.right).max()?;
    let bottom = asked.iter().filter(inside).map(|area| area.bottom).max()?;
    Some(RECT { right, bottom, ..layer })
}

/// The part of a printed layer (see `Printed`) inside the work area it was asked for, as a picture WIC reads from
/// where the print holds it: nothing copied out of the print (a desktop's worth for each screen) before it is scaled.
#[windows_core::implement(IWICBitmapSource)]
struct Part {
    bits: usize,
    /// The print's width (pixels), and the part's corner in it and size.
    stride: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl windows::Win32::Graphics::Imaging::IWICBitmapSource_Impl for Part_Impl {
    fn GetSize(&self, width: *mut u32, height: *mut u32) -> Result<()> {
        unsafe {
            *width = self.width;
            *height = self.height;
        }
        Ok(())
    }

    fn GetPixelFormat(&self) -> Result<windows::core::GUID> {
        // (Read as BGR: the alpha the print leaves is not the picture's.)
        Ok(GUID_WICPixelFormat32bppBGR)
    }

    fn GetResolution(&self, x: *mut f64, y: *mut f64) -> Result<()> {
        unsafe {
            *x = 96.0;
            *y = 96.0;
        }
        Ok(())
    }

    fn CopyPalette(&self, _: windows_core::Ref<'_, windows::Win32::Graphics::Imaging::IWICPalette>) -> Result<()> {
        Err(windows::Win32::Foundation::WINCODEC_ERR_PALETTEUNAVAILABLE.into())
    }

    fn CopyPixels(&self, rect: *const WICRect, stride: u32, size: u32, buffer: *mut u8) -> Result<()> {
        let rect = unsafe { rect.as_ref() }.copied().unwrap_or(WICRect { X: 0, Y: 0, Width: self.width as i32, Height: self.height as i32 });
        let fits = rect.X >= 0 && rect.Y >= 0 && rect.Width >= 0 && rect.Height >= 0 && (rect.X + rect.Width) as u32 <= self.width && (rect.Y + rect.Height) as u32 <= self.height;
        let row = rect.Width as usize * 4;
        if !fits || (stride as usize) < row || (size as usize) < stride as usize * (rect.Height.max(1) as usize - 1) + row {
            return Err(windows::Win32::Foundation::E_INVALIDARG.into());
        }
        for line in 0..rect.Height as usize {
            let from = ((self.y as usize + rect.Y as usize + line) * self.stride as usize + self.x as usize + rect.X as usize) * 4;
            unsafe { std::ptr::copy_nonoverlapping((self.bits as *const u8).add(from), buffer.add(line * stride as usize), row) };
        }
        Ok(())
    }
}

/// The live wallpaper inside `work` as `printed` holds it, scaled to `width` × `height`; it may hold the desktop's
/// icons (Windows 11 24H2 draws them into the same layer). `None` if the work area is not all in it.
fn shown_in(printed: &Printed, work: RECT, width: u32, height: u32) -> Option<Vec<u8>> {
    let at = printed.at;
    let inside = work.left >= at.left && work.top >= at.top && work.right <= at.right && work.bottom <= at.bottom;
    if !inside || work.right <= work.left || work.bottom <= work.top {
        return None;
    }
    let part: IWICBitmapSource = Part {
        bits: printed.bits as usize,
        stride: (at.right - at.left) as u32,
        x: (work.left - at.left) as u32,
        y: (work.top - at.top) as u32,
        width: (work.right - work.left) as u32,
        height: (work.bottom - work.top) as u32,
    }
    .into();
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        cover(&factory, &part, width, height).ok()
    }
}

/// The wallpaper's image file; `None` for a plain colour.
fn wallpaper_path() -> Option<String> {
    let mut path = [0u16; 260];
    unsafe { SystemParametersInfoW(SPI_GETDESKWALLPAPER, path.len() as u32, Some(path.as_mut_ptr().cast()), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) }
        .ok()?;
    let length = path.iter().position(|&c| c == 0)?;
    (length > 0).then(|| String::from_utf16_lossy(&path[..length]))
}

/// The image at `path`, scaled to cover `width` × `height` (see `cover`).
fn file(path: &str, width: u32, height: u32) -> Result<Vec<u8>> {
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let decoder = factory.CreateDecoderFromFilename(&HSTRING::from(path), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)?;
        cover(&factory, &decoder.GetFrame(0)?.cast()?, width, height)
    }
}

/// `image` scaled to cover `width` × `height` and cropped to it about its
/// centre, as premultiplied BGRA.
fn cover(factory: &IWICImagingFactory, image: &IWICBitmapSource, width: u32, height: u32) -> Result<Vec<u8>> {
    unsafe {
        let (mut w, mut h) = (0, 0);
        image.GetSize(&mut w, &mut h)?;
        let cover = (width as f64 / w as f64).max(height as f64 / h as f64);
        let (scaled_w, scaled_h) = (((w as f64 * cover).round() as u32).max(width), ((h as f64 * cover).round() as u32).max(height));
        let scaler = factory.CreateBitmapScaler()?;
        scaler.Initialize(image, scaled_w, scaled_h, WICBitmapInterpolationModeFant)?;
        let clipper = factory.CreateBitmapClipper()?;
        let crop = WICRect {
            X: ((scaled_w - width) / 2) as i32,
            Y: ((scaled_h - height) / 2) as i32,
            Width: width as i32,
            Height: height as i32,
        };
        clipper.Initialize(&scaler, &crop)?;
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(&clipper, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        converter.CopyPixels(std::ptr::null(), width * 4, &mut pixels)?;
        Ok(pixels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A live wallpaper's layer is printed from its corner as far as the areas asked of it reach, and not at all for
    /// areas outside it: two screens, a tall one left of a wide one (as a report had them), the layer over both.
    #[test]
    fn a_live_wallpaper_is_printed_as_far_as_it_is_asked() {
        let layer = RECT { left: -2160, top: -1071, right: 3840, bottom: 2769 };
        let tall = RECT { left: -2160, top: -1071, right: 0, bottom: 2729 };
        let wide = RECT { left: 0, top: 0, right: 3840, bottom: 2120 };
        // (Both work areas stop short of the tall one's taskbar.)
        assert_eq!(printed_extent(layer, &[tall, wide]), Some(RECT { bottom: 2729, ..layer }));
        assert_eq!(printed_extent(layer, &[tall, layer]), Some(layer));
        assert_eq!(printed_extent(layer, &[tall]), Some(RECT { right: 0, bottom: 2729, ..layer }));
        assert_eq!(printed_extent(layer, &[RECT { left: 100, top: 100, right: 500, bottom: 300 }]), Some(RECT { right: 500, bottom: 300, ..layer }));
        // (One not wholly in it is not read from it.)
        let beyond = RECT { left: 3840, top: 0, right: 5760, bottom: 1080 };
        assert_eq!(printed_extent(layer, &[beyond]), None);
        assert_eq!(printed_extent(layer, &[beyond, tall]), Some(RECT { right: 0, bottom: 2729, ..layer }));
        assert_eq!(printed_extent(layer, &[]), None);
    }
}
