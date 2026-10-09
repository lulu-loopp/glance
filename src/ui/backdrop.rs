//! What is on screen behind the panel, for skins that draw the desktop
//! through glass or acrylic.

use windows::core::Result;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_IGNORE, D2D1_PIXEL_FORMAT, D2D_SIZE_U};
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap1, ID2D1DeviceContext, D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_PROPERTIES1};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush, GetDC, ReleaseDC, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
};

/// The screen inside a rectangle, as it was when captured.
pub struct Capture {
    /// Physical screen coordinates.
    pub rect: RECT,
    /// Rows top to bottom, BGRA, opaque; none once let go of (see
    /// `forget_pixels`).
    pixels: Option<Vec<u8>>,
    /// How light it is, a value (0–255) for each `LUMA_STEP` square of
    /// pixels: what is read of it once its pixels are let go of.
    luma: Vec<u8>,
    /// Tells one capture's content from another's.
    pub digest: u64,
}

/// The side of the squares of pixels the luminance is kept for.
const LUMA_STEP: i32 = 2;

/// What tells one screenful from another: every pixel, eight bytes at a
/// time, mixed as FxHash mixes (a screen's worth in a few milliseconds,
/// where a general hasher takes tens). Alpha, which the copy leaves as it
/// is, is left out.
fn digest(pixels: &[u8]) -> u64 {
    const K: u64 = 0x517C_C1B7_2722_0A95;
    const COLOUR: u64 = 0x00FF_FFFF_00FF_FFFF;
    let mut hash = pixels.len() as u64;
    let mut words = pixels.chunks_exact(8);
    for word in &mut words {
        let v = u64::from_le_bytes(word.try_into().unwrap()) & COLOUR;
        hash = (hash.rotate_left(5) ^ v).wrapping_mul(K);
    }
    for &byte in words.remainder() {
        hash = (hash.rotate_left(5) ^ byte as u64).wrapping_mul(K);
    }
    hash
}

/// How many `LUMA_STEP` squares span `pixels`.
fn squares(pixels: i32) -> i32 {
    (pixels + LUMA_STEP - 1) / LUMA_STEP
}

/// The relative luminance (0–1) of a BGRA pixel at `i`.
fn pixel_luminance(pixels: &[u8], i: usize) -> f64 {
    (0.2126 * pixels[i + 2] as f64 + 0.7152 * pixels[i + 1] as f64 + 0.0722 * pixels[i] as f64) / 255.0
}

impl Capture {
    /// Copies the screen inside `rect`; `None` while it cannot be read (the
    /// lock screen or a UAC prompt has the display), or for an empty `rect`.
    pub fn take(rect: RECT) -> Option<Self> {
        let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
        if width <= 0 || height <= 0 {
            return None;
        }
        let header = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            // Negative: rows run top to bottom.
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let info = BITMAPINFO { bmiHeader: header, ..Default::default() };
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        unsafe {
            let screen = GetDC(None);
            let memory = CreateCompatibleDC(Some(screen));
            let mut bits = std::ptr::null_mut();
            let copied = CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0).ok().map(|bitmap| {
                let previous = SelectObject(memory, bitmap.into());
                // Read once GDI has done the copy, not while it may still be
                // queued.
                let copied = BitBlt(memory, 0, 0, width, height, Some(screen), rect.left, rect.top, SRCCOPY).is_ok() && GdiFlush().as_bool();
                if copied {
                    let length = pixels.len();
                    pixels.copy_from_slice(std::slice::from_raw_parts(bits as *const u8, length));
                }
                SelectObject(memory, previous);
                let _ = DeleteObject(bitmap.into());
                copied
            });
            let _ = DeleteDC(memory);
            ReleaseDC(None, screen);
            if copied != Some(true) {
                return None;
            }
        }
        // The copy leaves alpha at zero: its bitmap ignores it (the screen
        // is opaque), rather than every pixel being set.
        Some(Capture::from_pixels(rect, pixels))
    }

    /// A picture already in memory (BGRA, opaque, rows top to bottom),
    /// standing for the screen inside `rect`.
    pub fn from_pixels(rect: RECT, pixels: Vec<u8>) -> Self {
        let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
        let (columns, rows) = (squares(width), squares(height));
        let mut luma = Vec::with_capacity((columns * rows) as usize);
        for row in 0..rows {
            for column in 0..columns {
                let i = (((row * LUMA_STEP) * width + column * LUMA_STEP) * 4) as usize;
                luma.push((pixel_luminance(&pixels, i) * 255.0).round() as u8);
            }
        }
        let digest = digest(&pixels);
        Capture { rect, pixels: Some(pixels), luma, digest }
    }

    /// The same capture without its pixels (what is read of it is from its
    /// luminance): to be lent along with a bitmap already made of it.
    pub fn without_pixels(&self) -> Self {
        Capture { rect: self.rect, pixels: None, luma: self.luma.clone(), digest: self.digest }
    }

    fn width(&self) -> i32 {
        self.rect.right - self.rect.left
    }

    /// Lets go of its pixels once they are a bitmap: a screen's worth of
    /// memory. What is read of it after is from its luminance (see `luma`);
    /// a bitmap is not made of it again.
    pub fn forget_pixels(&mut self) {
        self.pixels = None;
    }

    /// The relative luminance (0–1) at (`x`, `y`) from its corner: of the
    /// pixel while it has its pixels, else of the square it is in.
    fn luminance_at(&self, x: i32, y: i32) -> f64 {
        match &self.pixels {
            Some(pixels) => pixel_luminance(pixels, ((y * self.width() + x) * 4) as usize),
            None => self.luma[((y / LUMA_STEP) * squares(self.width()) + x / LUMA_STEP) as usize] as f64 / 255.0,
        }
    }

    /// The capture as a bitmap at `px` physical pixels per DIP, so that it
    /// covers its rectangle's size in DIPs.
    pub fn bitmap(&self, dc: &ID2D1DeviceContext, px: f32) -> Result<ID2D1Bitmap1> {
        let Some(pixels) = &self.pixels else { return Err(windows::Win32::Foundation::E_UNEXPECTED.into()) };
        let size = D2D_SIZE_U { width: self.width() as u32, height: (self.rect.bottom - self.rect.top) as u32 };
        let properties = D2D1_BITMAP_PROPERTIES1 {
            // Opaque whatever its alpha bytes say (see `take`).
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_IGNORE },
            dpiX: 96.0 * px,
            dpiY: 96.0 * px,
            bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
            ..Default::default()
        };
        unsafe { dc.CreateBitmap(size, Some(pixels.as_ptr().cast()), (self.width() * 4) as u32, &properties) }
    }

    /// The relative luminance, as WCAG measures it (its channels made
    /// linear first), at (`x`, `y`) from its corner, while it has its
    /// pixels; else its luminance as kept.
    fn wcag_at(&self, x: i32, y: i32) -> f32 {
        let channel = |v: u8| v as f32 / 255.0;
        match &self.pixels {
            Some(pixels) => {
                let i = ((y * self.width() + x) * 4) as usize;
                crate::ui::overlay::luminance(channel(pixels[i + 2]), channel(pixels[i + 1]), channel(pixels[i]))
            }
            None => self.luminance_at(x, y) as f32,
        }
    }

    /// The relative luminance (0–1, as WCAG measures it) of every `step`th
    /// pixel each way inside `area` but outside `hole` (both in the
    /// capture's coordinates: physical screen ones for the screen's).
    pub fn luminances(&self, area: RECT, hole: RECT, step: i32) -> Vec<f32> {
        let r = self.rect;
        let (left, right) = (area.left.max(r.left), area.right.min(r.right));
        let (top, bottom) = (area.top.max(r.top), area.bottom.min(r.bottom));
        let mut luminances = Vec::new();
        for y in (top..bottom).step_by(step.max(1) as usize) {
            for x in (left..right).step_by(step.max(1) as usize) {
                if x >= hole.left && x < hole.right && y >= hole.top && y < hole.bottom {
                    continue;
                }
                luminances.push(self.wcag_at(x - r.left, y - r.top));
            }
        }
        luminances
    }

    /// The mean and standard deviation (0–1) of the luminance behind `area`
    /// (physical screen coordinates), sampled once per DIP: fine enough to
    /// see text strokes, and independent of the display's scale.
    pub fn luminance(&self, area: RECT, px: f32) -> (f32, f32) {
        let clip = |v: i32, low: i32, high: i32| v.clamp(low, high);
        let left = clip(area.left, self.rect.left, self.rect.right) - self.rect.left;
        let right = clip(area.right, self.rect.left, self.rect.right) - self.rect.left;
        let top = clip(area.top, self.rect.top, self.rect.bottom) - self.rect.top;
        let bottom = clip(area.bottom, self.rect.top, self.rect.bottom) - self.rect.top;
        let (columns, rows) = (((right - left) as f32 / px) as i32, ((bottom - top) as f32 / px) as i32);
        if columns <= 0 || rows <= 0 {
            return (0.5, 0.0);
        }
        let (mut sum, mut squares) = (0.0f64, 0.0f64);
        for row in 0..rows {
            let (y0, y1) = (top + (row as f32 * px) as i32, top + ((row + 1) as f32 * px) as i32);
            for column in 0..columns {
                let (x0, x1) = (left + (column as f32 * px) as i32, left + ((column + 1) as f32 * px) as i32);
                // The average of the pixels in this DIP.
                let mut cell = 0.0f64;
                let mut count = 0;
                for y in y0..y1.max(y0 + 1) {
                    for x in x0..x1.max(x0 + 1) {
                        cell += self.luminance_at(x, y);
                        count += 1;
                    }
                }
                let value = cell / count as f64;
                sum += value;
                squares += value * value;
            }
        }
        let count = (columns * rows) as f64;
        let mean = sum / count;
        (mean as f32, (squares / count - mean * mean).max(0.0).sqrt() as f32)
    }
}
