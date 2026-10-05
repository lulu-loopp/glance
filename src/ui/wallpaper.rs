//! The desktop the settings' preview stands in for: the wallpaper, scaled to
//! fill the work area as Windows does by default, or the plain desktop colour.

use windows::core::{Result, HSTRING};
use windows::Win32::Foundation::{GENERIC_READ, RECT};
use windows::Win32::Graphics::Gdi::{GetSysColor, COLOR_DESKTOP};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory, WICBitmapDitherTypeNone,
    WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand, WICRect,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETDESKWALLPAPER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS};

use super::backdrop::Capture;

/// The desktop as a picture `width` × `height` pixels large. Needs COM on
/// the calling thread.
pub fn desktop(width: u32, height: u32) -> Capture {
    let rect = RECT { left: 0, top: 0, right: width as i32, bottom: height as i32 };
    let pixels = wallpaper_path().and_then(|path| cover(&path, width, height).ok()).unwrap_or_else(|| {
        // GetSysColor answers 0x00BBGGRR; the picture is BGRA.
        let rgb = unsafe { GetSysColor(COLOR_DESKTOP) };
        let pixel = [((rgb >> 16) & 0xFF) as u8, ((rgb >> 8) & 0xFF) as u8, (rgb & 0xFF) as u8, 255];
        pixel.repeat((width * height) as usize)
    });
    Capture::from_pixels(rect, pixels)
}

/// The wallpaper's image file; `None` for a plain colour.
fn wallpaper_path() -> Option<String> {
    let mut path = [0u16; 260];
    unsafe { SystemParametersInfoW(SPI_GETDESKWALLPAPER, path.len() as u32, Some(path.as_mut_ptr().cast()), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) }
        .ok()?;
    let length = path.iter().position(|&c| c == 0)?;
    (length > 0).then(|| String::from_utf16_lossy(&path[..length]))
}

/// The image at `path`, scaled to cover `width` × `height` and cropped to it
/// about its centre, as premultiplied BGRA.
fn cover(path: &str, width: u32, height: u32) -> Result<Vec<u8>> {
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let decoder = factory.CreateDecoderFromFilename(&HSTRING::from(path), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)?;
        let frame = decoder.GetFrame(0)?;
        let (mut w, mut h) = (0, 0);
        frame.GetSize(&mut w, &mut h)?;
        let cover = (width as f64 / w as f64).max(height as f64 / h as f64);
        let (scaled_w, scaled_h) = (((w as f64 * cover).round() as u32).max(width), ((h as f64 * cover).round() as u32).max(height));
        let scaler = factory.CreateBitmapScaler()?;
        scaler.Initialize(&frame, scaled_w, scaled_h, WICBitmapInterpolationModeFant)?;
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
