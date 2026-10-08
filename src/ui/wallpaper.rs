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

/// The desktop of the work area `work` (physical pixels) as a picture
/// `width` × `height` pixels large. Needs COM on the calling thread.
pub fn desktop(work: RECT, width: u32, height: u32) -> Capture {
    let rect = RECT { left: 0, top: 0, right: width as i32, bottom: height as i32 };
    let pixels = shown(work, width, height)
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

/// The live wallpaper inside `work` as the desktop shows it just then,
/// scaled to `width` × `height`; it may hold the desktop's icons (Windows 11
/// 24H2 draws them into the same layer). `None` without a live wallpaper.
fn shown(work: RECT, width: u32, height: u32) -> Option<Vec<u8>> {
    let layer = live_layer()?;
    let mut at = RECT::default();
    unsafe { GetWindowRect(layer, &mut at) }.ok()?;
    let inside = work.left >= at.left && work.top >= at.top && work.right <= at.right && work.bottom <= at.bottom;
    if !inside || work.right <= work.left || work.bottom <= work.top {
        return None;
    }
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
    let (pw, ph) = (work.right - work.left, work.bottom - work.top);
    let mut pixels = vec![0u8; (pw * ph * 4) as usize];
    unsafe {
        let memory = CreateCompatibleDC(None);
        let mut bits = std::ptr::null_mut();
        let printed = CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0).ok().map(|bitmap| {
            let previous = SelectObject(memory, bitmap.into());
            // Full content: what DirectX draws in it too, as the screen shows it.
            let printed = PrintWindow(layer, memory, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool();
            if printed {
                let source = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
                let row = (pw * 4) as usize;
                for y in 0..ph {
                    let from = ((y + work.top - at.top) * w + (work.left - at.left)) as usize * 4;
                    pixels[y as usize * row..][..row].copy_from_slice(&source[from..from + row]);
                }
            }
            SelectObject(memory, previous);
            let _ = DeleteObject(bitmap.into());
            printed
        });
        let _ = DeleteDC(memory);
        if printed != Some(true) {
            return None;
        }
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        // Read as BGR: the alpha the print leaves is not the picture's.
        let bitmap = factory.CreateBitmapFromMemory(pw as u32, ph as u32, &GUID_WICPixelFormat32bppBGR, pw as u32 * 4, &pixels).ok()?;
        cover(&factory, &bitmap.cast().ok()?, width, height).ok()
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
