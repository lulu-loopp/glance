//! What is dragged onto the station's windows from other programs (Explorer, a browser, a mail): told as it comes,
//! moves over them, goes, and is dropped, and read when it is.
//!
//! A drop is always taken as a copy (`DROPEFFECT_COPY`, whatever the dragging program offers besides): taken as a
//! move, the program would delete what it dragged, and the station only points to files where they are.
//!
//! Files with no path of their own (an attachment dragged from a mail, a picture from a page: their names and
//! contents given, see `FileGroupDescriptorW`) are read as they are dropped, and kept as the station's own: once the
//! program that gave them is closed, they are to be had nowhere else.

use super::clipboard::Content;
use std::path::PathBuf;
use windows::core::{implement, w, Ref, Result, PCWSTR};
use windows::Win32::Foundation::{HGLOBAL, POINT, POINTL};
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, TYMED_HGLOBAL, TYMED_ISTREAM};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{IDropTarget, IDropTarget_Impl, ReleaseStgMedium, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::FILEDESCRIPTORW;

/// What happens to a drag over a station window, as it is told.
#[derive(Debug)]
pub enum Dragged {
    /// Something the station takes has come over it, at a point on the screen (physical px).
    Over(POINT),
    /// It went, or was dropped elsewhere.
    Gone,
    /// It was dropped there: what it gave.
    Dropped(Vec<Content>),
}

/// The most of a file with no path of its own read as it is dropped (bytes): more is not what the station is for.
const NAMELESS_MOST: u64 = 256 << 20;

/// A station window's drop target: tells `told` what happens to a drag over it. One the station's own drags are
/// dragged over (`own` false: the station's) takes none of them back: one put back where it was came on it twice. One
/// that `refuses` takes nothing, and is told all the same as something comes over it.
#[implement(IDropTarget)]
pub struct Target {
    told: Box<dyn Fn(Dragged)>,
    own: bool,
    refuses: bool,
    /// What came over it is something it takes.
    takes: std::cell::Cell<bool>,
}

impl Target {
    pub fn new(told: impl Fn(Dragged) + 'static) -> IDropTarget {
        Target { told: Box::new(told), own: true, refuses: false, takes: std::cell::Cell::new(false) }.into()
    }

    /// One that takes nothing dragged off the station (see `Target`).
    pub fn not_own(told: impl Fn(Dragged) + 'static) -> IDropTarget {
        Target { told: Box::new(told), own: false, refuses: false, takes: std::cell::Cell::new(false) }.into()
    }

    /// One that takes nothing, only told what comes over it (see `Target`).
    pub fn refusing(told: impl Fn(Dragged) + 'static) -> IDropTarget {
        Target { told: Box::new(told), own: true, refuses: true, takes: std::cell::Cell::new(false) }.into()
    }
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, data: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, at: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let takes = !self.refuses && data.as_ref().is_some_and(takes) && (self.own || !super::drag::under_way());
        self.takes.set(takes);
        self.answer(at, effect);
        Ok(())
    }

    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, at: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        self.answer(at, effect);
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        (self.told)(Dragged::Gone);
        Ok(())
    }

    fn Drop(&self, data: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, at: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let offered = unsafe { *effect };
        let given = data.as_ref().filter(|_| self.takes.get() && offered.0 & DROPEFFECT_COPY.0 != 0).map(read).unwrap_or_default();
        unsafe { *effect = if given.is_empty() { DROPEFFECT_NONE } else { DROPEFFECT_COPY } };
        let _ = at;
        (self.told)(if given.is_empty() { Dragged::Gone } else { Dragged::Dropped(given) });
        Ok(())
    }
}

impl Target_Impl {
    /// Answers a drag at `at`: a copy where it is something the station takes and the dragging program lets it be
    /// copied; else nothing. (Told it is there where so, or where it refuses all.)
    fn answer(&self, at: &POINTL, effect: *mut DROPEFFECT) {
        let offered = unsafe { *effect };
        let copies = self.takes.get() && offered.0 & DROPEFFECT_COPY.0 != 0;
        unsafe { *effect = if copies { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
        if copies || self.refuses {
            (self.told)(Dragged::Over(POINT { x: at.x, y: at.y }));
        }
    }
}

/// The formats the station takes, by the clipboard's numbers (registered ones looked up).
struct Formats {
    files: u16,
    text: u16,
    nameless: u16,
    contents: u16,
    png: u16,
    link: u16,
}

fn formats() -> Formats {
    let registered = |name: PCWSTR| unsafe { RegisterClipboardFormatW(name) } as u16;
    Formats {
        files: 15,
        text: 13,
        nameless: registered(w!("FileGroupDescriptorW")),
        contents: registered(w!("FileContents")),
        png: registered(w!("PNG")),
        link: registered(w!("UniformResourceLocatorW")),
    }
}

/// What it is asked for in `format` (by `tymed`, the `index`th of its files where it gives several).
fn asked(format: u16, index: i32, tymed: i32) -> FORMATETC {
    FORMATETC { cfFormat: format, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: index, tymed: tymed as u32 }
}

/// Whether `data` holds something the station takes.
fn takes(data: &IDataObject) -> bool {
    let formats = formats();
    [formats.files, formats.nameless, formats.png, formats.link, formats.text]
        .into_iter()
        .any(|format| unsafe { data.QueryGetData(&asked(format, -1, TYMED_HGLOBAL.0)) }.is_ok())
}

/// What `data` gives: its files where it has paths; else its files with none (read now); else its picture, its web
/// address, or its text.
fn read(data: &IDataObject) -> Vec<Content> {
    let formats = formats();
    if let Some(paths) = memory(data, formats.files, -1).map(|bytes| paths_of(&bytes)) {
        return vec![Content::Files(paths)];
    }
    if let Some(described) = memory(data, formats.nameless, -1) {
        let nameless: Vec<Content> = names_of(&described)
            .into_iter()
            .enumerate()
            .filter_map(|(index, name)| Some(Content::Saved { bytes: contents(data, formats.contents, index as i32)?, name }))
            .collect();
        if !nameless.is_empty() {
            return nameless;
        }
    }
    if let Some(png) = memory(data, formats.png, -1) {
        return vec![Content::Png(png)];
    }
    let text = |format: u16| memory(data, format, -1).map(|bytes| utf16_text(&bytes)).filter(|text| !text.trim().is_empty());
    match text(formats.link).or_else(|| text(formats.text)) {
        Some(text) => vec![Content::Text(text)],
        None => Vec::new(),
    }
}

/// What `data` holds as `format` in memory (the `index`th of its files), copied.
fn memory(data: &IDataObject, format: u16, index: i32) -> Option<Vec<u8>> {
    unsafe {
        let mut medium: STGMEDIUM = data.GetData(&asked(format, index, TYMED_HGLOBAL.0)).ok()?;
        let read = (|| {
            let global = HGLOBAL(medium.u.hGlobal.0);
            let size = GlobalSize(global);
            let at = GlobalLock(global);
            if at.is_null() {
                return None;
            }
            let bytes = std::slice::from_raw_parts(at.cast::<u8>(), size).to_vec();
            let _ = GlobalUnlock(global);
            Some(bytes)
        })();
        ReleaseStgMedium(&mut medium);
        read
    }
}

/// The contents of the `index`th of the files with no path `data` gives, read whole (in memory or as a stream): none
/// past `NAMELESS_MOST`.
fn contents(data: &IDataObject, format: u16, index: i32) -> Option<Vec<u8>> {
    unsafe {
        let mut medium: STGMEDIUM = data.GetData(&asked(format, index, TYMED_HGLOBAL.0 | TYMED_ISTREAM.0)).ok()?;
        let read = if medium.tymed == TYMED_ISTREAM.0 as u32 {
            (*medium.u.pstm).as_ref().and_then(|stream| {
                let mut bytes = Vec::new();
                let mut chunk = vec![0u8; 1 << 16];
                loop {
                    let mut got = 0u32;
                    let said = stream.Read(chunk.as_mut_ptr().cast(), chunk.len() as u32, Some(&mut got));
                    if said.is_err() {
                        return None;
                    }
                    if got == 0 {
                        break Some(bytes);
                    }
                    bytes.extend_from_slice(&chunk[..got as usize]);
                    if bytes.len() as u64 > NAMELESS_MOST {
                        return None;
                    }
                }
            })
        } else {
            let global = HGLOBAL(medium.u.hGlobal.0);
            let size = GlobalSize(global);
            let at = GlobalLock(global);
            if at.is_null() || size as u64 > NAMELESS_MOST {
                None
            } else {
                let bytes = std::slice::from_raw_parts(at.cast::<u8>(), size).to_vec();
                let _ = GlobalUnlock(global);
                Some(bytes)
            }
        };
        ReleaseStgMedium(&mut medium);
        read
    }
}

/// The paths of files as they are dragged (`DROPFILES`: where the paths begin, whether they are wide, then each ended
/// by a zero, and the list by another).
fn paths_of(bytes: &[u8]) -> Vec<PathBuf> {
    let word = |at: usize| bytes.get(at..at + 4).map(|word| u32::from_le_bytes(word.try_into().unwrap()));
    let (Some(from), Some(wide)) = (word(0), word(16)) else { return Vec::new() };
    let Some(listed) = bytes.get(from as usize..) else { return Vec::new() };
    let names: Vec<String> = if wide != 0 {
        let units: Vec<u16> = listed.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
        units.split(|unit| *unit == 0).take_while(|name| !name.is_empty()).map(String::from_utf16_lossy).collect()
    } else {
        listed.split(|byte| *byte == 0).take_while(|name| !name.is_empty()).map(|name| String::from_utf8_lossy(name).into_owned()).collect()
    };
    names.into_iter().map(PathBuf::from).collect()
}

/// The names of the files with no path a `FILEGROUPDESCRIPTORW` gives (only their last part: a name given is never
/// where it is kept, see `folder`).
fn names_of(bytes: &[u8]) -> Vec<String> {
    let Some(count) = bytes.get(..4).map(|count| u32::from_le_bytes(count.try_into().unwrap()) as usize) else { return Vec::new() };
    let size = std::mem::size_of::<FILEDESCRIPTORW>();
    (0..count)
        .map_while(|index| {
            let at = 4 + index * size;
            let described = bytes.get(at..at + size)?;
            // (Read where it lies, not where a FILEDESCRIPTORW would be aligned.)
            let described: FILEDESCRIPTORW = unsafe { std::ptr::read_unaligned(described.as_ptr().cast()) };
            // (Copied out of the packed struct before it is looked at.)
            let units = described.cFileName;
            let name = String::from_utf16_lossy(&units[..units.iter().position(|unit| *unit == 0).unwrap_or(units.len())]);
            Some(name.rsplit(['\\', '/']).next().unwrap_or_default().to_string())
        })
        .collect()
}

/// Text as it is dragged: UTF-16, ended by a zero.
fn utf16_text(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).take_while(|unit| *unit != 0).collect();
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Files dragged are read as they are put on the clipboard (the same `DROPFILES`).
    #[test]
    fn dragged_files_are_read() {
        let paths = vec![PathBuf::from("C:/a b/c.txt"), PathBuf::from("D:/d")];
        assert_eq!(paths_of(&crate::station::clipboard::dropped(&paths)), paths);
        assert!(paths_of(&[1, 2]).is_empty());
    }

    /// The names of files with no path are their last part alone (a name given cannot reach out of the station's
    /// folder: it is only shown).
    #[test]
    fn nameless_files_are_named_by_their_last_part() {
        let mut descriptor = FILEDESCRIPTORW::default();
        for (at, unit) in "..\\..\\photo.jpg".encode_utf16().enumerate() {
            descriptor.cFileName[at] = unit;
        }
        let mut bytes = 1u32.to_le_bytes().to_vec();
        bytes.extend_from_slice(unsafe { std::slice::from_raw_parts((&descriptor as *const FILEDESCRIPTORW).cast::<u8>(), std::mem::size_of::<FILEDESCRIPTORW>()) });
        assert_eq!(names_of(&bytes), ["photo.jpg"]);
        // Said to be more than it holds: those it holds.
        bytes[..4].copy_from_slice(&3u32.to_le_bytes());
        assert_eq!(names_of(&bytes).len(), 1);
    }
}
