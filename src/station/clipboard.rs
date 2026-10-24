//! What is copied: read from the clipboard for the station's Recently copied (as it changes), or as it is pasted on
//! the station (Ctrl+V).
//!
//! Read as one: the formats that keep it out are looked at and what it is read in the one opening of the clipboard,
//! and what is read copied into Glance's own before the clipboard is closed (the clipboard's own is the copier's, and
//! goes as the next copy comes). Its sequence number, read while it is open, tells which copy it was: told of a
//! change, Glance may find a later copy there (the one told of is then not had: copied very quickly one after
//! another, a copy between may be missed).
//!
//! What a copier asks of it is kept to (`Marks`, `judged`):
//! - `ExcludeClipboardContentFromMonitorProcessing` (a password manager's): not read at all as it changes.
//! - `CanIncludeInClipboardHistory` 0: not in Recently copied.
//! - `CanUploadToCloudClipboard` 0: kept, never to leave this PC (see `shelf::Item::stays_here`).

use std::path::PathBuf;
use windows::core::{w, Interface, PCWSTR};
use windows::Win32::Foundation::{HGLOBAL, HWND};
use windows::Win32::Graphics::Gdi::HBITMAP;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA, IWICBitmapSource, IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapEncoderNoCache, WICBitmapPaletteTypeCustom,
    WICBitmapUseAlpha,
};
use windows::Win32::System::Com::StructuredStorage::{CreateStreamOnHGlobal, GetHGlobalFromStream};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER, STREAM_SEEK_END};
use std::os::windows::ffi::OsStrExt;
use windows::Win32::Graphics::Imaging::WICDecodeMetadataCacheOnDemand;
use windows::Win32::System::DataExchange::{GetClipboardData, GetClipboardSequenceNumber, IsClipboardFormatAvailable, RegisterClipboardFormatW};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

/// The clipboard's own formats: a picture, text, files.
const BITMAP: u32 = 2;
const TEXT: u32 = crate::clipboard::TEXT;
const FILES: u32 = 15;
/// The most of a picture's or text's bytes read (more is not what the station is for: a file is).
const BYTES_MOST: usize = 256 << 20;

/// What was copied.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    Files(Vec<PathBuf>),
    Text(String),
    /// A picture, as a PNG's bytes.
    Png(Vec<u8>),
    /// A file with no path of its own, as it was given (dragged from a mail or a page: see `drop`): its name, and
    /// what it holds.
    Saved { name: String, bytes: Vec<u8> },
}

/// What was copied, as Glance read it.
#[derive(Debug)]
pub struct Copied {
    pub content: Content,
    /// Never to leave this PC (see `Marks::upload`).
    pub stays_here: bool,
    /// The clipboard's sequence number as it was read (which copy it was).
    pub sequence: u32,
}

/// What its copier asked of a copy, by the formats it put with it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Marks {
    /// `ExcludeClipboardContentFromMonitorProcessing` there (whatever it holds).
    pub excluded: bool,
    /// `CanIncludeInClipboardHistory`'s value, there.
    pub history: Option<u32>,
    /// `CanUploadToCloudClipboard`'s value, there.
    pub upload: Option<u32>,
}

/// Why the clipboard is read: it changed (for Recently copied), or it was pasted on the station by hand.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Why {
    Changed,
    Pasted,
}

/// Whether a copy marked `marks` is taken, read for `why`: if so, whether it never leaves this PC. (Pasted by hand,
/// it is taken whatever it says of monitors and history: the person put it there.)
pub fn judged(marks: Marks, why: Why) -> Option<bool> {
    let stays_here = marks.upload == Some(0);
    match why {
        Why::Changed if marks.excluded || marks.history == Some(0) => None,
        _ => Some(stays_here),
    }
}

/// The clipboard's sequence number now.
pub fn sequence() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

/// The clipboard held by another program just now (to be read again in a moment).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Busy;

/// What is on the clipboard, read for `why` by window `owner`, as `judged` takes it: none if it is not taken or holds
/// nothing the station keeps; `Busy` if another program holds it just now.
pub fn read(owner: HWND, why: Why) -> Result<Option<Copied>, Busy> {
    unsafe {
        // (Not waited for: read again in a moment, see `run`.)
        let opened = crate::clipboard::try_open(owner).ok_or(Busy)?;
        let read = read_open(why);
        drop(opened);
        let Some((taken, stays_here, sequence)) = read else { return Ok(None) };
        // (A picture read as the clipboard's was copied into a bitmap of Glance's; made a PNG now it is closed.)
        let content = match taken {
            Taken::Content(content) => content,
            Taken::Bitmap(bitmap) => match png(&bitmap) {
                Ok(png) => Content::Png(png),
                Err(_) => return Ok(None),
            },
        };
        Ok(Some(Copied { content, stays_here, sequence }))
    }
}

/// What was read with the clipboard open: what it is, or a picture copied out of it to be made a PNG.
enum Taken {
    Content(Content),
    Bitmap(IWICBitmapSource),
}

/// As `read`, the clipboard open.
unsafe fn read_open(why: Why) -> Option<(Taken, bool, u32)> {
    unsafe {
        let sequence = GetClipboardSequenceNumber();
        let format = |name: PCWSTR| RegisterClipboardFormatW(name);
        let value = |name: PCWSTR| -> Option<u32> {
            let bytes = bytes_of(format(name))?;
            Some(u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?))
        };
        let marks = Marks {
            excluded: IsClipboardFormatAvailable(format(w!("ExcludeClipboardContentFromMonitorProcessing"))).is_ok(),
            history: value(w!("CanIncludeInClipboardHistory")),
            upload: value(w!("CanUploadToCloudClipboard")),
        };
        let stays_here = judged(marks, why)?;
        // Files before text (a file copied in Explorer is also its name), text before a picture (cells copied in a
        // sheet are also their picture).
        let taken = if let Ok(files) = GetClipboardData(FILES) {
            let files = HDROP(files.0);
            let paths: Vec<PathBuf> = (0..DragQueryFileW(files, u32::MAX, None))
                .map(|index| {
                    let mut path = vec![0u16; DragQueryFileW(files, index, None) as usize + 1];
                    let length = DragQueryFileW(files, index, Some(&mut path)) as usize;
                    PathBuf::from(String::from_utf16_lossy(&path[..length]))
                })
                .collect();
            Taken::Content(Content::Files(paths))
        } else if let Some(text) = bytes_of(TEXT) {
            let units: Vec<u16> = text.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).take_while(|unit| *unit != 0).collect();
            Taken::Content(Content::Text(String::from_utf16_lossy(&units)))
        } else if let Some(png) = bytes_of(format(w!("PNG"))) {
            Taken::Content(Content::Png(png))
        } else if let Ok(bitmap) = GetClipboardData(BITMAP) {
            let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
            // (Its pixels copied into the bitmap made: the clipboard's goes as it is closed.)
            Taken::Bitmap(factory.CreateBitmapFromHBITMAP(HBITMAP(bitmap.0), Default::default(), WICBitmapUseAlpha).ok()?.cast().ok()?)
        } else {
            return None;
        };
        Some((taken, stays_here, sequence))
    }
}

/// The bytes of the clipboard's `format` (open), copied: none if it has none, or more than `BYTES_MOST`.
unsafe fn bytes_of(format: u32) -> Option<Vec<u8>> {
    unsafe {
        if format == 0 || IsClipboardFormatAvailable(format).is_err() {
            return None;
        }
        let memory = HGLOBAL(GetClipboardData(format).ok()?.0);
        let size = GlobalSize(memory);
        if size > BYTES_MOST {
            return None;
        }
        let at = GlobalLock(memory);
        if at.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(at.cast::<u8>(), size).to_vec();
        let _ = GlobalUnlock(memory);
        Some(bytes)
    }
}

/// `image` as a PNG's bytes.
unsafe fn png(image: &IWICBitmapSource) -> windows::core::Result<Vec<u8>> {
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let (mut width, mut height) = (0, 0);
        image.GetSize(&mut width, &mut height)?;
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(image, &GUID_WICPixelFormat32bppBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
        let stream = CreateStreamOnHGlobal(HGLOBAL::default(), true)?;
        let encoder = factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
        let frame = frame.ok_or_else(windows::core::Error::empty)?;
        frame.Initialize(None)?;
        frame.SetSize(width, height)?;
        let mut format = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut format)?;
        frame.WriteSource(&converter, std::ptr::null())?;
        frame.Commit()?;
        encoder.Commit()?;
        let mut length = 0u64;
        stream.Seek(0, STREAM_SEEK_END, Some(&mut length))?;
        let memory = GetHGlobalFromStream(&stream)?;
        let bytes = GlobalLock(memory);
        let png = std::slice::from_raw_parts(bytes.cast::<u8>(), length as usize).to_vec();
        let _ = GlobalUnlock(memory);
        Ok(png)
    }
}

/// Puts `content` on the clipboard, by window `owner` (`stays_here`: marked not to leave this PC, as its copier marked
/// it: see `Marks::upload`). A picture goes as a PNG and as a bitmap (programs that read no PNG take that). Its
/// sequence number once it is there (what Glance itself put there, not taken for a copy: see `read`); none if the
/// clipboard was held by another, or what was to go there could not be made.
pub fn put(owner: HWND, content: &Content, stays_here: bool) -> Option<u32> {
    // Made before the clipboard is opened (a picture decoded): it is held no longer than it takes to hand it over.
    let formats = formats_of(content, stays_here)?;
    crate::clipboard::put(owner, &formats).then(|| unsafe { GetClipboardSequenceNumber() })
}

/// `content` in the formats it is given in, copied or dragged (see `put`), each format's number and bytes: none where
/// it cannot be made.
pub(super) fn formats_of(content: &Content, stays_here: bool) -> Option<Vec<(u32, Vec<u8>)>> {
    let registered = |name: PCWSTR| unsafe { RegisterClipboardFormatW(name) };
    let mut formats: Vec<(u32, Vec<u8>)> = match content {
        Content::Text(text) => vec![(TEXT, crate::clipboard::utf16_z(text))],
        // (Copied where they are dropped: see `drop`, and moved only by hand.)
        Content::Files(paths) => vec![(FILES, dropped(paths)), (registered(w!("Preferred DropEffect")), 1u32.to_le_bytes().to_vec())],
        Content::Png(png) => {
            let dib = unsafe { dib_of(png).ok()? };
            vec![(registered(w!("PNG")), png.clone()), (DIB, dib)]
        }
        // (Given back as the file the station keeps of it: see `keep::Keep::content_of`.)
        Content::Saved { .. } => return None,
    };
    if stays_here {
        formats.push((registered(w!("CanUploadToCloudClipboard")), 0u32.to_le_bytes().to_vec()));
    }
    Some(formats)
}

/// The clipboard's device-independent bitmap format.
const DIB: u32 = 8;


/// `paths` as files are put on the clipboard (`DROPFILES`, then each path in UTF-16 ended by a zero, and a zero more).
pub(super) fn dropped(paths: &[PathBuf]) -> Vec<u8> {
    // pFiles (where the paths begin), pt (x, y), fNC, fWide.
    let mut bytes: Vec<u8> = [20u32, 0, 0, 0, 1].iter().flat_map(|v| v.to_le_bytes()).collect();
    for path in paths {
        bytes.extend(path.as_os_str().encode_wide().chain([0]).flat_map(u16::to_le_bytes));
    }
    bytes.extend([0, 0]);
    bytes
}

/// A PNG's picture as a device-independent bitmap (`BITMAPINFOHEADER`, 32 bits a pixel, its rows bottom up).
unsafe fn dib_of(png: &[u8]) -> windows::core::Result<Vec<u8>> {
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = factory.CreateStream()?;
        stream.InitializeFromMemory(png)?;
        let decoder = factory.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
        let frame = decoder.GetFrame(0)?;
        let (mut width, mut height) = (0, 0);
        frame.GetSize(&mut width, &mut height)?;
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(&frame, &GUID_WICPixelFormat32bppBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
        let row = width as usize * 4;
        let mut pixels = vec![0u8; row * height as usize];
        converter.CopyPixels(std::ptr::null(), row as u32, &mut pixels)?;
        // biSize, biWidth, biHeight (bottom up), biPlanes and biBitCount, biCompression (BI_RGB), biSizeImage, the
        // rest none.
        let mut dib: Vec<u8> = Vec::with_capacity(40 + pixels.len());
        dib.extend(40u32.to_le_bytes());
        dib.extend((width as i32).to_le_bytes());
        dib.extend((height as i32).to_le_bytes());
        dib.extend(1u16.to_le_bytes());
        dib.extend(32u16.to_le_bytes());
        dib.extend(0u32.to_le_bytes());
        dib.extend((pixels.len() as u32).to_le_bytes());
        dib.extend([0u8; 16]);
        for line in pixels.chunks_exact(row).rev() {
            dib.extend_from_slice(line);
        }
        Ok(dib)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copier_is_kept_to() {
        let plain = Marks::default();
        assert_eq!(judged(plain, Why::Changed), Some(false));
        // A password manager's: not read as it changes; pasted by hand, the person's.
        let excluded = Marks { excluded: true, ..plain };
        assert_eq!(judged(excluded, Why::Changed), None);
        assert_eq!(judged(excluded, Why::Pasted), Some(false));
        // Not for the history: not in Recently copied (1 is as none).
        assert_eq!(judged(Marks { history: Some(0), ..plain }, Why::Changed), None);
        assert_eq!(judged(Marks { history: Some(1), ..plain }, Why::Changed), Some(false));
        // Not to be uploaded: kept, never to leave this PC.
        assert_eq!(judged(Marks { upload: Some(0), ..plain }, Why::Changed), Some(true));
        assert_eq!(judged(Marks { upload: Some(0), excluded: true, ..plain }, Why::Pasted), Some(true));
    }

    /// Files go on the clipboard as Explorer puts them: where their paths begin, wide, each ended, and the list too.
    #[test]
    fn files_are_put_as_explorer_puts_them() {
        let bytes = dropped(&[PathBuf::from(r"C:\a"), PathBuf::from(r"D:\b")]);
        assert_eq!(&bytes[..4], &20u32.to_le_bytes());
        assert_eq!(&bytes[16..20], &1u32.to_le_bytes());
        let units: Vec<u16> = bytes[20..].chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
        assert_eq!(String::from_utf16(&units).unwrap(), "C:\\a\0D:\\b\0\0");
    }
}
