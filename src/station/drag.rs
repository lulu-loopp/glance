//! What is dragged off the station into another program: given in the formats it is copied in (see
//! `clipboard::formats_of`), the receiving program choosing among them; files as a copy (the station never has them
//! moved away by a drag: moved, they would be gone from where it points to). Words are given as a text file too
//! (named by their first line), for what takes files only (Explorer, the desktop): a program that takes words takes
//! them as words.
//!
//! Several things dragged together (chosen together, see `select`) are given as one (see `given`): words and links
//! alone as their words one after another, and as a file each besides (a text file, an Internet shortcut) that only
//! the program they are dropped on makes, with files among them, as files alone, each word and link written as its
//! file first. (Files and words given as files the program makes, Explorer takes the files alone and the rest is lost,
//! all given as files the program makes, a file's contents would be read whole into memory, a folder's listed file by
//! file, and the many programs that take files but not files they make, a chat's or a browser's, would take none.)
//!
//! The station gives Windows no picture of what is dragged: Glance carries it on the screen itself (see `carry`), told
//! as the drag goes whether the program under the pointer would take it there and what that program says a drop does
//! ("复制到 下载": see `Going`). A program says so by setting a description on what is dragged (Explorer does, with no
//! picture given too, run elevated or not alike).

use super::clipboard::{self, Content};
use windows::core::{implement, BOOL, HRESULT};
use windows::Win32::Foundation::{GlobalFree, HGLOBAL};
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{DoDragDrop, IDropSource, IDropSource_Impl, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::SHCreateDataObject;

/// What came of a drag off the station.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dragged {
    /// Dropped on a program that took it.
    Taken,
    /// Let go where nothing took it, or given up (Esc).
    Not,
}

/// How the drag off the station asked for last goes: its number (see `announce`), what is under the pointer (see
/// `Over`), and how it ended (none: under way).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Going {
    pub drag: u64,
    pub over: Over,
    pub ended: Option<Dragged>,
}

/// What is under the pointer as something is dragged off the station: whether a drop there would be taken, and what the
/// program there says it does (as it says it, in its own language).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Over {
    pub takes: bool,
    pub says: Option<String>,
}

static GOING: std::sync::Mutex<Going> = std::sync::Mutex::new(Going { drag: 0, over: Over { takes: false, says: None }, ended: Some(Dragged::Not) });

/// A drag off the station asked for: its number, as it is followed (see `going`).
pub fn announce() -> u64 {
    let mut going = GOING.lock().unwrap();
    *going = Going { drag: going.drag + 1, over: Over::default(), ended: None };
    going.drag
}

/// How the drag off the station asked for last goes.
pub fn going() -> Going {
    GOING.lock().unwrap().clone()
}

/// Whether a drag off the station is under way: the station takes nothing of it back (see `drop::Target`).
pub fn under_way() -> bool {
    GOING.lock().unwrap().ended.is_none()
}

/// The drag off the station asked for last ended so.
pub fn ended(dragged: Dragged) {
    GOING.lock().unwrap().ended = Some(dragged);
}

/// Drags `given` off the station (as one: see `given`, `stays_here`, any of it: see `shelf::Item::stays_here`) till it
/// is dropped or given up, on this thread (which must be the station's: in a single-threaded apartment, its messages
/// pumped meanwhile by Windows), pressed on a window of the thread `pressed_on`: whose input this thread shares
/// meanwhile (the pointer is that thread's while its button is down: not shared, the drag saw it neither move nor let
/// go). None where it could not be begun.
pub fn drag(given: &[Content], stays_here: bool, pressed_on: u32) -> Option<Dragged> {
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    let this = unsafe { GetCurrentThreadId() };
    let shared = pressed_on != this && unsafe { AttachThreadInput(this, pressed_on, true) }.as_bool();
    let dragged = drag_here(given, stays_here);
    if shared {
        let _ = unsafe { AttachThreadInput(this, pressed_on, false) };
    }
    dragged
}

/// As `drag`, its input this thread's.
fn drag_here(given: &[Content], stays_here: bool) -> Option<Dragged> {
    let data = data_of(&self::given(given, stays_here, &dragged_folder())?)?;
    unsafe {
        let source: IDropSource = Source { data: data.clone(), said: Default::default() }.into();
        let mut effect = DROPEFFECT_NONE;
        let said = DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect);
        Some(if said == windows::Win32::Foundation::DRAGDROP_S_DROP && effect != DROPEFFECT_NONE { Dragged::Taken } else { Dragged::Not })
    }
}

/// The shell's own data object (which keeps whatever it is given, and gives the receiving program what it asks of it
/// as Explorer's do), holding `formats` (see `given`). None where one could not be made. With files' contents among
/// them, ours over it, giving each by its index (see `Contents`): the shell's keeps one of a format, whatever its index.
fn data_of(formats: &[(u32, Vec<u8>, i32)]) -> Option<IDataObject> {
    let shell = shell_data(formats)?;
    let files: Vec<Vec<u8>> = formats.iter().filter(|(_, _, index)| *index >= 0).map(|(_, bytes, _)| bytes.clone()).collect();
    if files.is_empty() {
        return Some(shell);
    }
    let format = formats.iter().find(|(_, _, index)| *index >= 0).map(|(format, ..)| *format)?;
    Some(Contents { shell, format, files }.into())
}

/// The shell's own data object holding `formats` (see `data_of`). None where one could not be made.
fn shell_data(formats: &[(u32, Vec<u8>, i32)]) -> Option<IDataObject> {
    unsafe {
        let data: IDataObject = SHCreateDataObject(None, None, None).ok()?;
        for (format, bytes, index) in formats {
            let memory = global(bytes)?;
            let asked = FORMATETC { cfFormat: *format as u16, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: *index, tymed: TYMED_HGLOBAL.0 as u32 };
            let medium = STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: memory }, pUnkForRelease: std::mem::ManuallyDrop::new(None) };
            // (The data object owns it from here; not taken, it is let go of here.)
            if data.SetData(&asked, &medium, true).is_err() {
                let _ = GlobalFree(Some(memory));
                return None;
            }
        }
        Some(data)
    }
}

/// What `given` is dragged off as (`stays_here`: any of it, see `shelf::Item::stays_here`): each format's number, its
/// bytes, and the index of the file it is of (-1 for all but a file's contents, see `files_made`). One thing: as it is
/// copied, and words as a text file too. Several, words and links alone: their words one after another, an empty line
/// between each, and each a file besides (see `word_file`). Several, files among them: all as files (see `as_paths`),
/// each word and link written as its file in `folder` first. None where something of it cannot be made.
fn given(given: &[Content], stays_here: bool, folder: &std::path::Path) -> Option<Vec<(u32, Vec<u8>, i32)>> {
    let formats = |content: &Content| Some(clipboard::formats_of(content, stays_here)?.into_iter().map(|(format, bytes)| (format, bytes, -1)).collect::<Vec<_>>());
    match given {
        [Content::Text(text)] => {
            let mut formats = formats(&Content::Text(text.clone()))?;
            formats.extend(files_made(&[(text_name(text), text_contents(text))]));
            Some(formats)
        }
        [one] => formats(one),
        _ => match words_of(given) {
            Some(words) => {
                let mut formats = formats(&Content::Text(words.join(BETWEEN)))?;
                let names = unlike(words.iter().map(|words| word_file(words).0).collect(), &[]);
                let made: Vec<(String, Vec<u8>)> = names.into_iter().zip(&words).map(|(name, words)| (name, word_file(words).1)).collect();
                formats.extend(files_made(&made));
                Some(formats)
            }
            None => formats(&Content::Files(as_paths(given, folder)?)),
        },
    }
}

/// What is between the words of things given together as words: an empty line.
const BETWEEN: &str = "\r\n\r\n";

/// `given` as one, as it is put on the clipboard: one thing as it is, words and links alone as their words one after
/// another, an empty line between each, with files among them, all as files (see `as_paths`, the words and links
/// written in Glance's folder for them). None where something of it cannot be made.
pub(super) fn as_one(given: &[Content]) -> Option<Content> {
    match given {
        [one] => Some(one.clone()),
        _ => match words_of(given) {
            Some(words) => Some(Content::Text(words.join(BETWEEN))),
            None => Some(Content::Files(as_paths(given, &dragged_folder())?)),
        },
    }
}

/// The words of each of `given`, where all of it is words (text, links), none where anything else is among it.
fn words_of(given: &[Content]) -> Option<Vec<&str>> {
    given.iter().map(|content| if let Content::Text(text) = content { Some(text.as_str()) } else { None }).collect()
}

/// The file words are given as: a link (a web address alone, see `keep::is_link`) as an Internet shortcut to it, named
/// by its site (see `link_name`), other words as a text file (see `text_name`). Its name and contents.
fn word_file(words: &str) -> (String, Vec<u8>) {
    if super::keep::is_link(words) {
        (link_name(words), format!("[InternetShortcut]\r\nURL={words}\r\n").into_bytes())
    } else {
        (text_name(words), text_contents(words))
    }
}

/// Words as a text file holds them: UTF-8, marked so (as Notepad reads Chinese right).
fn text_contents(text: &str) -> Vec<u8> {
    [0xEF, 0xBB, 0xBF].into_iter().chain(text.bytes()).collect()
}

/// The name an Internet shortcut to `url` is given: its site (as much of it as a name holds, see `os::file_name_of`),
/// and ".url", "链接.url" where nothing of it is left.
fn link_name(url: &str) -> String {
    let site = url.split_once("://").map_or(url, |(_, rest)| rest).split(['/', '?', '#']).next().unwrap_or_default();
    let site = crate::os::file_name_of(site, 40);
    format!("{}.url", if site.is_empty() { "链接" } else { &site })
}

/// `names` made each unlike the others before it and those `taken` (told without case, as Windows tells names): one
/// like one before numbered on, as Explorer numbers a copy ("会议.txt", "会议 (2).txt"). Dropped into one folder, none
/// is asked about.
fn unlike(names: Vec<String>, taken: &[String]) -> Vec<String> {
    let mut held: Vec<String> = taken.iter().map(|name| name.to_lowercase()).collect();
    names
        .into_iter()
        .map(|name| {
            let path = std::path::Path::new(&name);
            let (stem, extension) = (path.file_stem().unwrap_or_default().to_string_lossy().into_owned(), path.extension().map(|extension| format!(".{}", extension.to_string_lossy())).unwrap_or_default());
            let named = (1u32..).map(|again| if again == 1 { name.clone() } else { format!("{stem} ({again}){extension}") }).find(|named| !held.contains(&named.to_lowercase())).expect("a name is found");
            held.push(named.to_lowercase());
            named
        })
        .collect()
}

/// What of `given` is not a file yet given as one: words and links as `word_file` says, a picture as a PNG named
/// "图片.png", what came with no path of its own by its own name. Its name and contents (none for files).
fn as_file(content: &Content) -> Option<(String, Vec<u8>)> {
    match content {
        Content::Files(_) => None,
        Content::Text(words) => Some(word_file(words)),
        Content::Png(bytes) => Some(("图片.png".into(), bytes.clone())),
        Content::Saved { name, bytes } => Some((name.clone(), bytes.clone())),
    }
}

/// `given` all as files: files where they are, the rest written as its file (see `as_file`) in `folder`, in a folder
/// of its own there (named by what it holds: the same, the same file), its name unlike those of the others dragged
/// (see `unlike`). None where one could not be written.
fn as_paths(given: &[Content], folder: &std::path::Path) -> Option<Vec<std::path::PathBuf>> {
    let taken: Vec<String> = given.iter().flat_map(|content| if let Content::Files(paths) = content { paths.iter().filter_map(|path| Some(path.file_name()?.to_string_lossy().into_owned())).collect() } else { Vec::new() }).collect();
    let made: Vec<(String, Vec<u8>)> = given.iter().filter_map(as_file).collect();
    let names = unlike(made.iter().map(|(name, _)| name.clone()).collect(), &taken);
    let mut written = names.into_iter().zip(&made).map(|(name, (_, contents))| write_in(folder, &name, contents));
    let mut paths = Vec::new();
    for content in given {
        match content {
            Content::Files(files) => paths.extend(files.iter().cloned()),
            _ => paths.push(written.next()??),
        }
    }
    Some(paths)
}

/// `contents` written as a file named `name` in a folder of its own in `folder`, named by what it holds (the same
/// name and contents, the same file): where it is.
fn write_in(folder: &std::path::Path, name: &str, contents: &[u8]) -> Option<std::path::PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut digest = std::collections::hash_map::DefaultHasher::new();
    (name, contents).hash(&mut digest);
    let own = folder.join(format!("{:016x}", digest.finish()));
    std::fs::create_dir_all(&own).ok()?;
    let file = own.join(name);
    std::fs::write(&file, contents).ok()?;
    Some(file)
}

/// Where words and links dragged off with files are written as files: a folder of Glance's among Windows' temporary
/// ones.
fn dragged_folder() -> std::path::PathBuf {
    std::env::temp_dir().join("Glance").join("dragged")
}

/// `files` (each its name and contents) as files dropped where only files are taken, made by the program they are
/// dropped on: their description (`FILEGROUPDESCRIPTORW`, as many as there are, one after another) and each one's
/// contents, by its index among them, each format's number, bytes, and the index of the file it is of.
fn files_made(files: &[(String, Vec<u8>)]) -> Vec<(u32, Vec<u8>, i32)> {
    use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
    use windows::Win32::UI::Shell::{FD_FILESIZE, FD_PROGRESSUI, FD_UNICODE, FILEDESCRIPTORW};
    let mut described: Vec<u8> = (files.len() as u32).to_le_bytes().to_vec();
    for (name, contents) in files {
        // (Its name in a copy of its own: the descriptor is packed, its fields not to be borrowed.)
        let mut units = [0u16; 260];
        for (to, unit) in units.iter_mut().zip(name.encode_utf16().take(259)) {
            *to = unit;
        }
        let file = FILEDESCRIPTORW { dwFlags: (FD_FILESIZE.0 | FD_PROGRESSUI.0 | FD_UNICODE.0) as u32, nFileSizeLow: contents.len() as u32, cFileName: units, ..Default::default() };
        described.extend_from_slice(unsafe { std::slice::from_raw_parts((&file as *const FILEDESCRIPTORW).cast::<u8>(), std::mem::size_of::<FILEDESCRIPTORW>()) });
    }
    let (descriptor, contents_format) = unsafe { (RegisterClipboardFormatW(windows::core::w!("FileGroupDescriptorW")), RegisterClipboardFormatW(windows::core::w!("FileContents"))) };
    std::iter::once((descriptor, described, -1)).chain(files.iter().enumerate().map(|(at, (_, contents))| (contents_format, contents.clone(), at as i32))).collect()
}

/// The name a text file of `text` is given: its first line (as much of it as a name holds, as it may hold it: see
/// `os::file_name_of`), and ".txt"; "文本.txt" where nothing of it is left.
pub(crate) fn text_name(text: &str) -> String {
    let first = crate::os::file_name_of(text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default(), 40);
    format!("{}.txt", if first.is_empty() { "文本" } else { &first })
}

/// `bytes` in global memory of their own (to be handed over).
unsafe fn global(bytes: &[u8]) -> Option<HGLOBAL> {
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)).ok()?;
        let at = GlobalLock(memory);
        if at.is_null() {
            let _ = GlobalFree(Some(memory));
            return None;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), at.cast::<u8>(), bytes.len());
        let _ = GlobalUnlock(memory);
        Some(memory)
    }
}

#[cfg(test)]
mod tests {
    /// What Explorer says a drop does, as it sets it: its words, the place named where it says `%1`; none where it took
    /// it back (as it does by turns over one folder) or said nothing.
    #[test]
    fn says_what_a_drop_does_as_the_program_said_it() {
        use windows::Win32::UI::Shell::{DROPDESCRIPTION, DROPIMAGE_COPY, DROPIMAGE_INVALID, DROPIMAGETYPE};
        let described = |kind: DROPIMAGETYPE, message: &str, insert: &str| {
            let (mut words, mut place) = ([0u16; 260], [0u16; 260]);
            for (to, unit) in words.iter_mut().zip(message.encode_utf16()) {
                *to = unit;
            }
            for (to, unit) in place.iter_mut().zip(insert.encode_utf16()) {
                *to = unit;
            }
            super::said_by(&DROPDESCRIPTION { r#type: kind, szMessage: words, szInsert: place })
        };
        assert_eq!(described(DROPIMAGE_COPY, "复制到 %1", "下载"), Some("复制到 下载".into()));
        assert_eq!(described(DROPIMAGE_COPY, "Copy to %1", "dropdir"), Some("Copy to dropdir".into()));
        assert_eq!(described(DROPIMAGE_COPY, "Copy", ""), Some("Copy".into()));
        assert_eq!(described(DROPIMAGE_INVALID, "", ""), None);
        assert_eq!(described(DROPIMAGE_INVALID, "Copy to %1", "dropdir"), None);
        assert_eq!(described(DROPIMAGE_COPY, "", "dropdir"), None);
    }

    /// What a drop does is said on through the moments the program says nothing, over the same window taking it still;
    /// over another window, or one not taking it, not.
    #[test]
    fn what_a_drop_does_is_kept_over_the_window_that_said_it() {
        let copy = || Some("复制到 下载".to_string());
        let was = (7, copy());
        assert_eq!(super::kept_said(&was, 7, true, None), (7, copy()));
        assert_eq!(super::kept_said(&was, 7, true, Some("复制到 桌面".into())), (7, Some("复制到 桌面".into())));
        assert_eq!(super::kept_said(&was, 8, true, None), (8, None));
        assert_eq!(super::kept_said(&was, 7, false, None), (7, None));
        assert_eq!(super::kept_said(&(0, None), 7, true, copy()), (7, copy()));
    }

    use super::super::clipboard::Content;

    /// The formats given, as the data object is given them: the number of each, and the index of the file it is of.
    fn kinds(given: &[(u32, Vec<u8>, i32)]) -> Vec<(u32, i32)> {
        given.iter().map(|(format, _, index)| (*format, *index)).collect()
    }

    fn format(name: windows::core::PCWSTR) -> u32 {
        unsafe { windows::Win32::System::DataExchange::RegisterClipboardFormatW(name) }
    }

    /// A folder of a test's own among the temporary ones, not there yet.
    fn folder(name: &str) -> std::path::PathBuf {
        let folder = std::env::temp_dir().join(format!("glance-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        folder
    }

    /// Names given together are made unlike each other and those already taken, told without case, numbered on as
    /// Explorer numbers a copy.
    #[test]
    fn names_given_together_are_made_unlike() {
        let names = |given: &[&str]| given.iter().map(|name| name.to_string()).collect::<Vec<_>>();
        assert_eq!(super::unlike(names(&["会议.txt", "会议.txt", "会议.txt"]), &[]), names(&["会议.txt", "会议 (2).txt", "会议 (3).txt"]));
        assert_eq!(super::unlike(names(&["A.url", "a.URL"]), &names(&["a.url"])), names(&["A (2).url", "a (3).URL"]));
        assert_eq!(super::unlike(names(&["readme"]), &names(&["README"])), names(&["readme (2)"]));
    }

    /// A link is given as an Internet shortcut named by its site, other words as a text file named by its first line,
    /// marked UTF-8.
    #[test]
    fn words_and_links_are_given_as_their_files() {
        assert_eq!(super::link_name("https://www.example.com/a/b?c#d"), "www.example.com.url");
        let (name, contents) = super::word_file("https://glancepc.com/zh/");
        assert_eq!((name.as_str(), contents.as_slice()), ("glancepc.com.url", b"[InternetShortcut]\r\nURL=https://glancepc.com/zh/\r\n".as_slice()));
        let (name, contents) = super::word_file("周四开会\n二楼");
        assert_eq!(name, "周四开会.txt");
        assert_eq!(contents, [&[0xEF, 0xBB, 0xBF][..], "周四开会\n二楼".as_bytes()].concat());
    }

    /// Files made by the program dropped on are described as Windows reads them: their count, then each one's
    /// description, as long as the structure, each one's contents by its index.
    #[test]
    fn files_made_are_described_as_windows_reads_them() {
        use windows::Win32::UI::Shell::{FILEDESCRIPTORW, FILEGROUPDESCRIPTORW};
        let made = super::files_made(&[("a.txt".into(), b"one".to_vec()), ("b.url".into(), b"three".to_vec())]);
        let (descriptor, contents) = (format(windows::core::w!("FileGroupDescriptorW")), format(windows::core::w!("FileContents")));
        assert_eq!(kinds(&made), [(descriptor, -1), (contents, 0), (contents, 1)]);
        assert_eq!(made[0].1.len(), std::mem::size_of::<FILEGROUPDESCRIPTORW>() + std::mem::size_of::<FILEDESCRIPTORW>());
        let group = unsafe { std::ptr::read_unaligned(made[0].1.as_ptr().cast::<FILEGROUPDESCRIPTORW>()) };
        assert_eq!({ group.cItems }, 2);
        let second = unsafe { std::ptr::read_unaligned(made[0].1.as_ptr().add(4 + std::mem::size_of::<FILEDESCRIPTORW>()).cast::<FILEDESCRIPTORW>()) };
        let named = { second.cFileName };
        assert_eq!(String::from_utf16_lossy(&named[..named.iter().position(|unit| *unit == 0).unwrap()]), "b.url");
        assert_eq!({ second.nFileSizeLow }, 5);
        assert_eq!((made[1].1.as_slice(), made[2].1.as_slice()), (b"one".as_slice(), b"three".as_slice()));
    }

    /// One thing is given as it is copied (words as a text file too), several words and links as their words, an empty
    /// line between each, and each as its file, nothing written, several with files among them, all as files, the
    /// rest written as files first, named unlike the files given.
    #[test]
    fn things_given_together_are_given_as_one() {
        use windows::Win32::System::Ole::{CF_HDROP, CF_UNICODETEXT};
        let (text, drop) = (u32::from(CF_UNICODETEXT.0), u32::from(CF_HDROP.0));
        let (descriptor, contents) = (format(windows::core::w!("FileGroupDescriptorW")), format(windows::core::w!("FileContents")));
        let folder = folder("given");
        let words = Content::Text("周四开会".into());
        let one = kinds(&super::given(std::slice::from_ref(&words), false, &folder).unwrap());
        assert!(one.contains(&(text, -1)) && one.contains(&(descriptor, -1)) && one.contains(&(contents, 0)));
        let link = Content::Text("https://glancepc.com".into());
        let both = super::given(&[words.clone(), link.clone()], false, &folder).unwrap();
        let joined: Vec<u8> = "周四开会\r\n\r\nhttps://glancepc.com\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert!(both.iter().any(|(format, bytes, _)| *format == text && *bytes == joined));
        let kinds_both = kinds(&both);
        assert!(kinds_both.contains(&(contents, 0)) && kinds_both.contains(&(contents, 1)) && !kinds_both.contains(&(drop, -1)));
        assert!(!folder.exists());
        // With a file: all as files, the rest written in the folder given, named unlike the file.
        let file = folder.join("given").join("周四开会.txt");
        let mixed = [Content::Files(vec![file.clone()]), words, link, Content::Png(vec![1, 2, 3])];
        let paths = super::as_paths(&mixed, &folder).unwrap();
        assert_eq!(paths[0], file);
        let names: Vec<String> = paths[1..].iter().map(|path| path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["周四开会 (2).txt", "glancepc.com.url", "图片.png"]);
        assert!(paths[1..].iter().all(|path| path.starts_with(&folder)));
        assert_eq!(std::fs::read(&paths[3]).unwrap(), [1, 2, 3]);
        let given = kinds(&super::given(&mixed, false, &folder).unwrap());
        assert!(given.contains(&(drop, -1)) && !given.contains(&(descriptor, -1)));
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Things put on the clipboard together are put there as one: one as it is, words and links as their words with an
    /// empty line between each, with files among them all as files.
    #[test]
    fn things_copied_together_are_copied_as_one() {
        let words = Content::Text("周四开会".into());
        assert_eq!(super::as_one(std::slice::from_ref(&words)), Some(words.clone()));
        assert_eq!(super::as_one(&[words.clone(), Content::Text("https://a.b".into())]), Some(Content::Text("周四开会\r\n\r\nhttps://a.b".into())));
        let file = std::env::temp_dir().join("周四.pdf");
        let Some(Content::Files(paths)) = super::as_one(&[Content::Files(vec![file.clone()]), words]) else { panic!("as files") };
        assert_eq!(paths[0], file);
        assert_eq!(paths[1].file_name().unwrap(), "周四开会.txt");
        assert!(paths[1].starts_with(super::dragged_folder()));
    }

    /// The shell's data object keeps the contents of each file made apart by their index: each asked by its own, its
    /// own given.
    #[test]
    fn the_data_object_keeps_each_files_contents_apart() {
        use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
        use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
        let _com = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) };
        let made = super::files_made(&[("a.txt".into(), b"one".to_vec()), ("b.txt".into(), b"three".to_vec())]);
        let data = super::data_of(&made).unwrap();
        let contents = format(windows::core::w!("FileContents"));
        for (index, want) in [(0, b"one".as_slice()), (1, b"three".as_slice())] {
            let asked = FORMATETC { cfFormat: contents as u16, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: index, tymed: TYMED_HGLOBAL.0 as u32 };
            let mut medium = unsafe { data.GetData(&asked) }.unwrap();
            let got = unsafe {
                let memory = medium.u.hGlobal;
                let at = GlobalLock(memory).cast::<u8>();
                let got = std::slice::from_raw_parts(at, GlobalSize(memory).min(want.len())).to_vec();
                let _ = GlobalUnlock(memory);
                windows::Win32::System::Ole::ReleaseStgMedium(&mut medium);
                got
            };
            assert_eq!(got, want);
        }
        // (The format listed, as the shell lists it.)
        let listed = unsafe { data.EnumFormatEtc(windows::Win32::System::Com::DATADIR_GET.0 as u32) }.unwrap();
        let mut each = [FORMATETC::default(); 8];
        let mut got = 0;
        let _ = unsafe { listed.Next(&mut each, Some(&mut got)) };
        assert!(each[..got as usize].iter().any(|one| u32::from(one.cfFormat) == contents));
    }

    /// Words dragged as a file are named by their first line that has any, what a file's name may not hold left out.
    #[test]
    fn names_a_text_file_by_its_first_line() {
        assert_eq!(super::text_name("\n  会议改到周四下午三点\n地点：二楼"), "会议改到周四下午三点.txt");
        assert_eq!(super::text_name("a/b:c*d?\"e<f>g|h"), "abcdefgh.txt");
        assert_eq!(super::text_name("..."), "文本.txt");
        assert_eq!(super::text_name(""), "文本.txt");
        assert_eq!(super::text_name(&"长".repeat(60)).chars().count(), 44);
    }
}

/// What is dragged where files made by the program dropped on are among it (see `files_made`): each one's contents
/// given by its index, as that program asks them one by one, all else the shell's data object's (`shell`, which lists
/// the format of the contents too, and keeps what the program dropped on says, as Explorer says what a drop does).
#[implement(IDataObject)]
struct Contents {
    shell: IDataObject,
    format: u32,
    files: Vec<Vec<u8>>,
}

impl Contents {
    /// The contents `asked` is for, where it asks one of the files by its index in global memory.
    fn asked(&self, asked: &FORMATETC) -> Option<&Vec<u8>> {
        let ours = u32::from(asked.cfFormat) == self.format && asked.tymed & TYMED_HGLOBAL.0 as u32 != 0;
        ours.then(|| usize::try_from(asked.lindex).ok().and_then(|index| self.files.get(index))).flatten()
    }
}

impl windows::Win32::System::Com::IDataObject_Impl for Contents_Impl {
    fn GetData(&self, asked: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
        let Some(bytes) = (unsafe { asked.as_ref() }).and_then(|asked| self.asked(asked)) else { return unsafe { self.shell.GetData(asked) } };
        let memory = unsafe { global(bytes) }.ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_OUTOFMEMORY))?;
        Ok(STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: memory }, pUnkForRelease: std::mem::ManuallyDrop::new(None) })
    }

    fn GetDataHere(&self, asked: *const FORMATETC, medium: *mut STGMEDIUM) -> windows::core::Result<()> {
        unsafe { self.shell.GetDataHere(asked, medium) }
    }

    fn QueryGetData(&self, asked: *const FORMATETC) -> HRESULT {
        match (unsafe { asked.as_ref() }).and_then(|asked| self.asked(asked)) {
            Some(_) => HRESULT(0),
            None => unsafe { self.shell.QueryGetData(asked) },
        }
    }

    fn GetCanonicalFormatEtc(&self, asked: *const FORMATETC, canonical: *mut FORMATETC) -> HRESULT {
        unsafe { self.shell.GetCanonicalFormatEtc(asked, canonical) }
    }

    fn SetData(&self, given: *const FORMATETC, medium: *const STGMEDIUM, release: BOOL) -> windows::core::Result<()> {
        unsafe { self.shell.SetData(given, medium, release.as_bool()) }
    }

    fn EnumFormatEtc(&self, direction: u32) -> windows::core::Result<windows::Win32::System::Com::IEnumFORMATETC> {
        unsafe { self.shell.EnumFormatEtc(direction) }
    }

    fn DAdvise(&self, asked: *const FORMATETC, advise: u32, sink: windows::core::Ref<windows::Win32::System::Com::IAdviseSink>) -> windows::core::Result<u32> {
        unsafe { self.shell.DAdvise(asked, advise, sink.as_ref()) }
    }

    fn DUnadvise(&self, connection: u32) -> windows::core::Result<()> {
        unsafe { self.shell.DUnadvise(connection) }
    }

    fn EnumDAdvise(&self) -> windows::core::Result<windows::Win32::System::Com::IEnumSTATDATA> {
        unsafe { self.shell.EnumDAdvise() }
    }
}

/// The station as the source of a drag: dropped as the left button is let go, given up by Esc; the system's cursors.
/// What is dragged, and what the program under the pointer last said a drop does, and that window (see `kept_said`).
#[implement(IDropSource)]
struct Source {
    data: IDataObject,
    said: std::cell::RefCell<(isize, Option<String>)>,
}

impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, _keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        // (The button and Esc as they are, not as the drag's messages say: pressed on another thread's window, those
        // may never come, and the drag never ended; or come late, the button said up as the drag began, and it ended
        // at once.)
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE, VK_LBUTTON, VK_RBUTTON};
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SWAPBUTTON};
        let down = |key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| unsafe { GetAsyncKeyState(i32::from(key.0)) } < 0;
        // (The button the mouse's buttons swapped make the left: its state as it is is the one under the finger.)
        let button = if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 { VK_RBUTTON } else { VK_LBUTTON };
        if escape.as_bool() || down(VK_ESCAPE) {
            windows::Win32::Foundation::DRAGDROP_S_CANCEL
        } else if !down(button) {
            windows::Win32::Foundation::DRAGDROP_S_DROP
        } else {
            HRESULT(0)
        }
    }

    /// Told as the pointer moves what the program under it would do with a drop there: told on (see `Going`).
    fn GiveFeedback(&self, effect: DROPEFFECT) -> HRESULT {
        use windows::Win32::System::Ole::{DROPEFFECT_LINK, DROPEFFECT_MOVE};
        use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, GetCursorPos, WindowFromPoint, GA_ROOT};
        let takes = effect.0 & (DROPEFFECT_COPY.0 | DROPEFFECT_MOVE.0 | DROPEFFECT_LINK.0) != 0;
        let mut cursor = windows::Win32::Foundation::POINT::default();
        let window = match unsafe { GetCursorPos(&mut cursor) } {
            Ok(()) => unsafe { GetAncestor(WindowFromPoint(cursor), GA_ROOT) }.0 as isize,
            Err(_) => 0,
        };
        let mut said = self.said.borrow_mut();
        *said = kept_said(&said, window, takes, description(&self.data));
        GOING.lock().unwrap().over = Over { takes, says: said.1.clone().filter(|_| takes) };
        windows::Win32::Foundation::DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// What a drop does as the program under the pointer last said (with the window that was under it, as `was` has it),
/// `window` under it now, taking a drop there (`takes`), and saying `said` now: what it says, else what it said, while
/// it is the same window and takes it still (Explorer, over one folder, says it and says nothing by turns as the
/// pointer moves; Windows' own picture of what is dragged said it throughout).
fn kept_said(was: &(isize, Option<String>), window: isize, takes: bool, said: Option<String>) -> (isize, Option<String>) {
    let kept = was.1.clone().filter(|_| was.0 == window && takes);
    (window, said.or(kept))
}

/// What the program under the pointer says a drop does, as it set it on what is dragged (none, or one it took back).
fn description(data: &IDataObject) -> Option<String> {
    use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
    use windows::Win32::System::Memory::GlobalSize;
    use windows::Win32::System::Ole::ReleaseStgMedium;
    use windows::Win32::UI::Shell::DROPDESCRIPTION;
    let format = unsafe { RegisterClipboardFormatW(windows::core::w!("DropDescription")) } as u16;
    let asked = FORMATETC { cfFormat: format, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
    let mut medium = unsafe { data.GetData(&asked) }.ok()?;
    let read = unsafe {
        let memory = medium.u.hGlobal;
        let at = GlobalLock(memory);
        let said = (!at.is_null() && GlobalSize(memory) >= std::mem::size_of::<DROPDESCRIPTION>()).then(|| std::ptr::read_unaligned(at.cast::<DROPDESCRIPTION>()));
        let _ = GlobalUnlock(memory);
        said
    };
    unsafe { ReleaseStgMedium(&mut medium) };
    said_by(&read?)
}

/// What a drop description says: its words, `%1` in them the place it names (none where it is taken back, or empty).
fn said_by(description: &windows::Win32::UI::Shell::DROPDESCRIPTION) -> Option<String> {
    use windows::Win32::UI::Shell::DROPIMAGE_INVALID;
    let words = |units: &[u16]| String::from_utf16_lossy(&units[..units.iter().position(|&unit| unit == 0).unwrap_or(units.len())]);
    let (kind, message, insert) = (description.r#type, description.szMessage, description.szInsert);
    let message = words(&message);
    (kind != DROPIMAGE_INVALID && !message.is_empty()).then(|| message.replace("%1", &words(&insert)))
}
