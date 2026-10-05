//! Captures what is on screen behind the panel, for skins that draw the
//! desktop through glass.

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
};

/// The screen inside `rect` (physical coordinates) as a top-down 32-bit BMP file.
pub fn screen_bmp(rect: RECT) -> Vec<u8> {
    let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
    let header = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width,
        // Negative: rows run top to bottom, as the file wants them too.
        biHeight: -height,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };
    let info = BITMAPINFO { bmiHeader: header, ..Default::default() };
    let pixels = (width * height * 4) as usize;
    const FILE_HEADER: usize = 14;
    let offset = FILE_HEADER + size_of::<BITMAPINFOHEADER>();

    let mut file = Vec::with_capacity(offset + pixels);
    file.extend_from_slice(b"BM");
    file.extend_from_slice(&((offset + pixels) as u32).to_le_bytes());
    file.extend_from_slice(&[0; 4]);
    file.extend_from_slice(&(offset as u32).to_le_bytes());
    file.extend_from_slice(unsafe {
        std::slice::from_raw_parts(&header as *const _ as *const u8, size_of::<BITMAPINFOHEADER>())
    });

    unsafe {
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0)
            .expect("capture bitmap");
        let previous = SelectObject(memory, bitmap.into());
        BitBlt(memory, 0, 0, width, height, Some(screen), rect.left, rect.top, SRCCOPY).expect("screen copy");
        let data = std::slice::from_raw_parts_mut(bits as *mut u8, pixels);
        // The copy leaves alpha at zero; the image is opaque.
        for pixel in data.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        file.extend_from_slice(data);
        SelectObject(memory, previous);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
    }
    file
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_a_column_quickly() {
        let rect = RECT { left: 1400, top: 0, right: 1920, bottom: 1080 };
        let start = std::time::Instant::now();
        let file = screen_bmp(rect);
        println!("capture took {:?}", start.elapsed());
        assert_eq!(&file[..2], b"BM");
        assert_eq!(file.len(), 54 + 520 * 1080 * 4);
    }
}
