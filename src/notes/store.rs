//! The notes' folder: a note a Markdown file each, readable without
//! Glance and synced as the user syncs the folder. What Glance keeps of them besides their words (each one's colour)
//! is in a file of its own beside them (`.glance-notes.json`), so the notes stay plain Markdown.
//!
//! - Read and written whole; written to a file beside it first, which then takes its place (a power cut leaves the old
//!   or the new, never half of one).
//! - Each note as it was last read or written (its length, when it was written, a digest of its bytes): one changed
//!   outside since is never written over. Its words from outside stay where they are, and what was written here goes
//!   to a copy beside it, named as a conflict's copy, so nothing is lost. The file a write takes the place of is kept
//!   till it is seen to be the one read: one changed outside in the moment between is put back.
//! - Read as their bytes say they are written (UTF-8, UTF-16 by its mark, or the system's code page); written in
//!   UTF-8. One whose words could not all be read is never written over (what was written here goes apart).
//! - Taken away to the Recycle Bin, not deleted (where a folder has none, Windows asks first).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// A note's colour, as its card shows it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Colour {
    #[default]
    Blue,
    Yellow,
    Green,
    Pink,
    Purple,
    Grey,
}

impl Colour {
    pub const ALL: [Colour; 6] = [Colour::Blue, Colour::Yellow, Colour::Green, Colour::Pink, Colour::Purple, Colour::Grey];
}

/// A note's file as it was last read or written: how long, when written (ms since 1970), and a digest of its bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Stamp {
    pub bytes: u64,
    pub written_ms: u64,
    pub digest: u64,
}

impl Stamp {
    /// Whether it is of the same bytes as `other` (whenever written).
    pub fn same(&self, other: &Stamp) -> bool {
        self.bytes == other.bytes && self.digest == other.digest
    }

    fn of(bytes: &[u8], written_ms: u64) -> Stamp {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut hasher);
        Stamp { bytes: bytes.len() as u64, written_ms, digest: hasher.finish() }
    }
}

/// A note: its file's name in the folder, its words (Markdown, as the file holds them), its colour, and its file as
/// last read or written.
#[derive(Clone, PartialEq, Debug)]
pub struct Note {
    pub name: String,
    pub text: String,
    pub colour: Colour,
    pub stamp: Stamp,
}

impl Note {
    /// Its title: its first line with words, as it reads (its Markdown's marks off: see `markdown::plain_lines`).
    pub fn title(&self) -> String {
        super::markdown::plain_lines(&self.text).into_iter().next().unwrap_or_default()
    }
}

/// What Glance keeps of the notes besides their words: each one's colour, by its file's name.
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
struct Kept {
    #[serde(default)]
    colours: BTreeMap<String, Colour>,
}

const KEPT: &str = ".glance-notes.json";

/// What became of a note written.
#[derive(Clone, PartialEq, Debug)]
pub enum Written {
    /// Written in its place (its file as it is now).
    Done(Stamp),
    /// Its file had been changed outside since it was read (or deleted): left as it was, and what was written here
    /// put in a copy beside it, by this name (and that copy's file as written).
    Apart(String, Stamp),
}

/// The notes' folder, and what a conflict's copy of a note is called after its name (see `Written::Apart`).
pub struct Folder {
    pub path: PathBuf,
    pub apart: &'static str,
}

impl Folder {
    /// The notes in it, the last written first.
    pub fn notes(&self) -> Vec<Note> {
        let kept = self.kept();
        let mut notes: Vec<Note> = std::fs::read_dir(&self.path)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|extension| extension.eq_ignore_ascii_case("md")) && entry.file_type().is_ok_and(|kind| kind.is_file()))
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let (text, stamp) = self.read(&name)?;
                let colour = kept.colours.get(&name).copied().unwrap_or_default();
                Some(Note { name, text, colour, stamp })
            })
            .collect();
        notes.sort_by(|a, b| b.stamp.written_ms.cmp(&a.stamp.written_ms).then_with(|| a.name.cmp(&b.name)));
        notes
    }

    /// The note `name`'s words and its file as it is now; none where it is gone (or not to be read).
    pub fn read(&self, name: &str) -> Option<(String, Stamp)> {
        let path = self.path.join(name);
        let bytes = std::fs::read(&path).ok()?;
        let written = written_ms(&path)?;
        Some((decoded(&bytes).0, Stamp::of(&bytes, written)))
    }

    /// The note `name`'s file as it is now, if it is there.
    #[cfg(test)]
    pub fn stamp(&self, name: &str) -> Option<Stamp> {
        let path = self.path.join(name);
        Some(Stamp::of(&std::fs::read(&path).ok()?, written_ms(&path)?))
    }

    /// `text` written as the note `name`, its file having been `was` as last read or written here (none: a note new
    /// here, whose file must not be there yet): in its place, unless it was changed outside since (see `Written`).
    pub fn write(&self, name: &str, text: &str, was: Option<Stamp>) -> std::io::Result<Written> {
        std::fs::create_dir_all(&self.path)?;
        let now = std::fs::read(self.path.join(name)).ok();
        let changed_outside = match (was, &now) {
            // (One whose words were not all read: never written over.)
            (Some(was), Some(now)) => !was.same(&Stamp::of(now, 0)) || !decoded(now).1,
            // A note new here whose name was taken meanwhile; or one deleted outside since (not made again where it
            // was, as if it had never been deleted: what was written here kept apart).
            (None, Some(_)) | (Some(_), None) => true,
            (None, None) => false,
        };
        if changed_outside {
            return self.apart(name, text);
        }
        self.put(name, text, was)
    }

    /// `text` put in the place of the note `name`, its file having been `was` as last read or written (none: a note
    /// new here): written beside it first, then taking its place, the file it took the place of kept till it is seen
    /// to be `was` (a file changed outside in the moment between put back, and what was written here put apart; a new
    /// note never put over a file that took its name).
    fn put(&self, name: &str, text: &str, was: Option<Stamp>) -> std::io::Result<Written> {
        let path = self.path.join(name);
        let beside = self.path.join(format!(".{name}.glance-writing"));
        let bytes = text.as_bytes();
        std::fs::write(&beside, bytes)?;
        let placed = match was {
            None => crate::os::move_new(&beside, &path),
            Some(was) => {
                // (Kept under a name of its own each time: one left by a save cut short, never looked at, is never
                // written over or taken away by the next.)
                let displaced = self.free_hidden(&format!(".{name}.glance-was"));
                match put_over(&beside, &path, &displaced) {
                    Ok(()) => {
                        let as_read = std::fs::read(&displaced).is_ok_and(|displaced| was.same(&Stamp::of(&displaced, 0)));
                        if !as_read {
                            return self.put_back(name, text, &displaced);
                        }
                        let _ = std::fs::remove_file(&displaced);
                        Ok(())
                    }
                    // (Its file moved to where it is kept, and this not put in its place: put back where it was.)
                    Err(error) if error.raw_os_error() == Some(ERROR_UNABLE_TO_MOVE_REPLACEMENT_2) => {
                        let _ = crate::os::move_new(&displaced, &path);
                        Err(error)
                    }
                    Err(error) => Err(error),
                }
            }
        };
        match placed {
            // (As written here: its digest of the bytes written, not of a file read again after, which another may
            // have written meanwhile.)
            Ok(()) => Ok(Written::Done(Stamp::of(bytes, written_ms(&path).unwrap_or_default()))),
            Err(error) => {
                let _ = std::fs::remove_file(&beside);
                match error.kind() {
                    std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::NotFound => self.apart(name, text),
                    _ => Err(error),
                }
            }
        }
    }

    /// `text` put in the place of the note `name` just now, and the file it took the place of (kept at `displaced`)
    /// found changed outside meanwhile: what was written here put apart, and that file put back in its place, never
    /// over another (one written there meanwhile stays, and it is kept apart too).
    fn put_back(&self, name: &str, text: &str, displaced: &Path) -> std::io::Result<Written> {
        let path = self.path.join(name);
        let written = self.apart(name, text)?;
        let stem = Path::new(name).file_stem().map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
        // In its place still what was written here, kept apart now: taken away; anything else there is another's, and
        // stays. Moved aside first, in one move, and looked at where no one else writes: looked at in its place and
        // then taken away by its name, a file written there in between was taken away in its stead.
        let taken = self.free_hidden(&format!(".{name}.glance-taken"));
        if crate::os::move_new(&path, &taken).is_ok() {
            if std::fs::read(&taken).is_ok_and(|now| now == text.as_bytes()) {
                std::fs::remove_file(&taken)?;
            } else if crate::os::move_new(&taken, &path).is_err() {
                // (Another's, and yet another written in its place meanwhile: kept beside it, never over it.)
                self.put_new_named(&format!("{stem}{}.md", self.apart), &taken)?;
            }
        }
        if crate::os::move_new(displaced, &path).is_err() {
            self.put_new_named(&format!("{stem}{}.md", self.apart), displaced)?;
        }
        Ok(written)
    }

    /// What was written here as the note `name` put in a conflict's copy beside it (never over another file).
    fn apart(&self, name: &str, text: &str) -> std::io::Result<Written> {
        let stem = Path::new(name).file_stem().map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
        let beside = self.path.join(format!(".{name}.glance-apart"));
        std::fs::write(&beside, text.as_bytes())?;
        let apart = self.put_new_named(&format!("{stem}{}.md", self.apart), &beside).inspect_err(|_| {
            let _ = std::fs::remove_file(&beside);
        })?;
        let written = written_ms(&self.path.join(&apart)).unwrap_or_default();
        // (In the colour of the note it is kept apart from.)
        let mut kept = self.kept();
        if let Some(colour) = kept.colours.get(name).copied() {
            kept.colours.insert(apart.clone(), colour);
            let _ = self.keep(&kept);
        }
        Ok(Written::Apart(apart, Stamp::of(text.as_bytes(), written)))
    }

    /// The file at `from` given `name`, or that with a number after its stem, a name no file has (taken by another
    /// meanwhile: the next); the name it was given.
    fn put_new_named(&self, name: &str, from: &Path) -> std::io::Result<String> {
        loop {
            let free = self.free_name(name);
            match crate::os::move_new(from, &self.path.join(&free)) {
                Ok(()) => return Ok(free),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
    }

    /// `name` (a file of Glance's own, hidden by its dot), or with a number after it, a name no file has.
    fn free_hidden(&self, name: &str) -> PathBuf {
        let mut candidate = self.path.join(name);
        let mut more = 2;
        while candidate.exists() {
            candidate = self.path.join(format!("{name}.{more}"));
            more += 1;
        }
        candidate
    }

    /// `text` in the file `name`: written beside it first, then put in its place.
    fn replace(&self, name: &str, text: &str) -> std::io::Result<()> {
        let path = self.path.join(name);
        let beside = self.path.join(format!(".{name}.glance-writing"));
        std::fs::write(&beside, text.as_bytes())?;
        std::fs::rename(&beside, &path).inspect_err(|_| {
            let _ = std::fs::remove_file(&beside);
        })
    }

    /// A name for a new note, from its title (`title`, as a name may hold it: see `os::file_name_of`), or else from
    /// today's date (`fallback`), not taken by another file.
    pub fn name_for(&self, title: &str, fallback: &str) -> String {
        let cleaned = crate::os::file_name_of(title, 60);
        let stem = if cleaned.is_empty() { fallback.to_string() } else { cleaned };
        self.free_name(&format!("{stem}.md"))
    }

    /// `name`, or with a number after its stem where a file has it already.
    fn free_name(&self, name: &str) -> String {
        let path = Path::new(name);
        let stem = path.file_stem().map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
        let extension = path.extension().map_or_else(String::new, |extension| format!(".{}", extension.to_string_lossy()));
        let mut candidate = name.to_string();
        let mut more = 2;
        while self.path.join(&candidate).exists() {
            candidate = format!("{stem} {more}{extension}");
            more += 1;
        }
        candidate
    }

    /// The note `name`'s colour kept as `colour`.
    pub fn colour(&self, name: &str, colour: Colour) -> std::io::Result<()> {
        let mut kept = self.kept();
        kept.colours.insert(name.to_string(), colour);
        self.keep(&kept)
    }

    /// The note `name` taken to the Recycle Bin (its colour forgotten); whether it was.
    pub fn take_away(&self, name: &str) -> bool {
        let taken = recycle(&self.path.join(name));
        if taken {
            let mut kept = self.kept();
            if kept.colours.remove(name).is_some() {
                let _ = self.keep(&kept);
            }
        }
        taken
    }

    fn kept(&self) -> Kept {
        std::fs::read(self.path.join(KEPT)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
    }

    fn keep(&self, kept: &Kept) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.path)?;
        self.replace(KEPT, &serde_json::to_string_pretty(kept).unwrap_or_default())
    }
}

/// A note's words from its file's `bytes`, as they say they are written: UTF-8 (its mark left out), UTF-32 or UTF-16
/// by its mark, else the system's code page (another editor's, saving as programs did before Unicode); and whether
/// they were read faithfully (none of the bytes lost, and no NUL in them, which the box they are written in would end
/// them at: read as near as they go, they are never written over).
pub fn decoded(bytes: &[u8]) -> (String, bool) {
    let (text, faithful) = read_as_written(bytes);
    let faithful = faithful && !text.contains('\0');
    (text, faithful)
}

fn read_as_written(bytes: &[u8]) -> (String, bool) {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return match std::str::from_utf8(rest) {
            Ok(text) => (text.to_string(), true),
            Err(_) => (String::from_utf8_lossy(rest).into_owned(), false),
        };
    }
    let utf16 = |rest: &[u8], unit: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = rest.chunks_exact(2).map(|pair| unit([pair[0], pair[1]])).collect();
        match String::from_utf16(&units) {
            Ok(text) => (text, rest.len() % 2 == 0),
            Err(_) => (String::from_utf16_lossy(&units), false),
        }
    };
    // (UTF-32's marks first: little-endian, its begins as UTF-16's does.)
    let utf32 = |rest: &[u8], unit: fn([u8; 4]) -> u32| {
        let read: Option<String> = rest.chunks_exact(4).map(|four| char::from_u32(unit([four[0], four[1], four[2], four[3]]))).collect();
        match read {
            Some(text) => (text, rest.len() % 4 == 0),
            None => (rest.chunks_exact(4).map(|four| char::from_u32(unit([four[0], four[1], four[2], four[3]])).unwrap_or('\u{FFFD}')).collect(), false),
        }
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE, 0x00, 0x00]) {
        return utf32(rest, u32::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0x00, 0x00, 0xFE, 0xFF]) {
        return utf32(rest, u32::from_be_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, u16::from_be_bytes);
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return (text.to_string(), true);
    }
    match system_code_page(bytes) {
        Some(text) => (text, true),
        None => (String::from_utf8_lossy(bytes).into_owned(), false),
    }
}

/// `bytes` read in the system's code page for programs that are not Unicode; none where they are not written in it.
#[cfg(windows)]
fn system_code_page(bytes: &[u8]) -> Option<String> {
    use windows::Win32::Globalization::{MultiByteToWideChar, CP_ACP, MB_ERR_INVALID_CHARS};
    let length = unsafe { MultiByteToWideChar(CP_ACP, MB_ERR_INVALID_CHARS, bytes, None) };
    if length <= 0 {
        return None;
    }
    let mut units = vec![0u16; length as usize];
    let written = unsafe { MultiByteToWideChar(CP_ACP, MB_ERR_INVALID_CHARS, bytes, Some(&mut units)) };
    (written == length).then(|| String::from_utf16_lossy(&units))
}

#[cfg(not(windows))]
fn system_code_page(_: &[u8]) -> Option<String> {
    None
}

/// What `ReplaceFileW` says when the file replaced was moved to where it is kept and the new one could not be put in
/// its place (winerror.h's `ERROR_UNABLE_TO_MOVE_REPLACEMENT_2`): the file is then only where it is kept.
const ERROR_UNABLE_TO_MOVE_REPLACEMENT_2: i32 = 1177;

/// The file at `from` put in the place of the one at `to`, which is kept as `kept` (gone: `NotFound`).
#[cfg(windows)]
fn put_over(from: &Path, to: &Path, kept: &Path) -> std::io::Result<()> {
    use windows::Win32::Storage::FileSystem::{ReplaceFileW, REPLACE_FILE_FLAGS};
    let name = |path: &Path| windows::core::HSTRING::from(path.as_os_str());
    unsafe { ReplaceFileW(&name(to), &name(from), &name(kept), REPLACE_FILE_FLAGS(0), None, None) }.map_err(|error| std::io::Error::from_raw_os_error(error.code().0 & 0xFFFF))
}

#[cfg(not(windows))]
fn put_over(from: &Path, to: &Path, kept: &Path) -> std::io::Result<()> {
    std::fs::hard_link(to, kept)?;
    std::fs::rename(from, to)
}

/// When the file at `path` was last written (ms since 1970).
fn written_ms(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(modified.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64)
}

/// The file at `path` moved to the Recycle Bin; whether it was. (Where it would be deleted for good, its folder having
/// no Recycle Bin, Windows asks first: never deleted for good unasked.)
#[cfg(windows)]
fn recycle(path: &Path) -> bool {
    use windows::Win32::UI::Shell::{SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING, FO_DELETE, SHFILEOPSTRUCTW};
    // (Its path ended by two nulls, as the call reads a list of them.)
    use std::os::windows::ffi::OsStrExt;
    let mut from: Vec<u16> = path.as_os_str().encode_wide().collect();
    from.extend([0, 0]);
    let mut operation = SHFILEOPSTRUCTW { wFunc: FO_DELETE, pFrom: windows::core::PCWSTR(from.as_ptr()), fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_WANTNUKEWARNING | FOF_NOERRORUI | FOF_SILENT).0 as u16, ..Default::default() };
    unsafe { SHFileOperationW(&mut operation) == 0 && !operation.fAnyOperationsAborted.as_bool() }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A note's title is its first line as it reads: a to-do's, a list's, a heading's marks off.
    #[test]
    fn a_title_reads_without_marks() {
        let title = |text: &str| Note { name: String::new(), text: text.into(), colour: Colour::Blue, stamp: Stamp::default() }.title();
        assert_eq!(title("- [ ] nihao
年后"), "nihao");
        assert_eq!(title("
## **本周**
"), "本周");
        assert_eq!(title("> 引的话"), "引的话");
    }

    fn folder(test: &str) -> Folder {
        let path = std::env::temp_dir().join(format!("glance-notes-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        Folder { path, apart: "（冲突副本）" }
    }

    /// A note written, read back as written, its title its first line with words; a new note named by its title, a
    /// second one of that title numbered.
    #[test]
    fn writes_and_reads_notes() {
        let folder = folder("write");
        let name = folder.name_for("本周: 要做的事?", "便签");
        assert_eq!(name, "本周 要做的事.md");
        assert_eq!(folder.write(&name, "# 本周: 要做的事?\n\n- [ ] 发布\n", None).unwrap(), Written::Done(folder.stamp(&name).unwrap()));
        assert_eq!(folder.name_for("本周: 要做的事?", "便签"), "本周 要做的事 2.md");
        let notes = folder.notes();
        assert_eq!((notes.len(), notes[0].title().as_str(), notes[0].colour), (1, "本周: 要做的事?", Colour::Blue));
        folder.colour(&name, Colour::Green).unwrap();
        assert_eq!(folder.notes()[0].colour, Colour::Green);
        // (No file of its own writing left beside it.)
        assert_eq!(std::fs::read_dir(&folder.path).unwrap().count(), 2);
        let _ = std::fs::remove_dir_all(&folder.path);
    }

    /// A note changed outside since it was read is not written over: its words from outside stay, and what was
    /// written here goes to a conflict's copy beside it.
    #[test]
    fn never_writes_over_what_changed_outside() {
        let folder = folder("conflict");
        let Written::Done(stamp) = folder.write("a.md", "here", None).unwrap() else { panic!("written") };
        folder.colour("a.md", Colour::Yellow).unwrap();
        std::fs::write(folder.path.join("a.md"), "outside, longer").unwrap();
        let Written::Apart(apart, _) = folder.write("a.md", "here again", Some(stamp)).unwrap() else { panic!("put apart") };
        assert_eq!(folder.read("a.md").unwrap().0, "outside, longer");
        assert_eq!(folder.read(&apart).unwrap().0, "here again");
        // (The copy in the note's colour.)
        assert_eq!(folder.notes().iter().find(|note| note.name == apart).map(|note| note.colour), Some(Colour::Yellow));
        // A note new here whose name was taken meanwhile: put apart too.
        assert!(matches!(folder.write("a.md", "new", None).unwrap(), Written::Apart(..)));
        // One deleted outside since it was read: not made again in its place, what was written kept apart.
        let Written::Done(stamp) = folder.write("b.md", "b", None).unwrap() else { panic!("written") };
        std::fs::remove_file(folder.path.join("b.md")).unwrap();
        assert!(matches!(folder.write("b.md", "b again", Some(stamp)).unwrap(), Written::Apart(..)));
        assert!(!folder.path.join("b.md").exists());
        let _ = std::fs::remove_dir_all(&folder.path);
    }

    /// Changed outside in the moment between a note being looked at and written: the file it took the place of seen
    /// not to be the one read, and put back; what was written here put apart. Its stamp from the bytes written.
    #[test]
    fn puts_back_what_changed_outside_meanwhile() {
        let folder = folder("meanwhile");
        let Written::Done(stamp) = folder.write("a.md", "here", None).unwrap() else { panic!("written") };
        assert!(stamp.same(&Stamp::of(b"here", 0)));
        std::fs::write(folder.path.join("a.md"), "outside").unwrap();
        // (Past the look before writing, as if it came just after it.)
        let Written::Apart(apart, apart_stamp) = folder.put("a.md", "here again", Some(stamp)).unwrap() else { panic!("put apart") };
        assert_eq!(folder.read("a.md").unwrap().0, "outside");
        assert_eq!(folder.read(&apart).unwrap().0, "here again");
        // (The copy's stamp of the bytes written here.)
        assert!(apart_stamp.same(&Stamp::of(b"here again", 0)));
        // A new note whose name another took meanwhile: not put over it.
        assert!(matches!(folder.put("a.md", "new", None).unwrap(), Written::Apart(..)));
        assert_eq!(folder.read("a.md").unwrap().0, "outside");
        // Nothing of its writing left beside them.
        assert!(std::fs::read_dir(&folder.path).unwrap().flatten().all(|entry| !entry.file_name().to_string_lossy().starts_with(".a.md")));
        let _ = std::fs::remove_dir_all(&folder.path);
    }

    /// A save cut short before (the file it took the place of left where it is kept, never looked at): not written over
    /// or taken away by the next, which keeps its own under another name, and takes that away.
    #[test]
    fn keeps_what_a_save_cut_short_left() {
        let folder = folder("cut-short");
        let Written::Done(stamp) = folder.write("a.md", "one", None).unwrap() else { panic!("written") };
        std::fs::write(folder.path.join(".a.md.glance-was"), "left by a save cut short").unwrap();
        assert!(matches!(folder.write("a.md", "two", Some(stamp)).unwrap(), Written::Done(_)));
        assert_eq!(std::fs::read_to_string(folder.path.join(".a.md.glance-was")).unwrap(), "left by a save cut short");
        assert!(!folder.path.join(".a.md.glance-was.2").exists());
        assert_eq!(folder.read("a.md").unwrap().0, "two");
        let _ = std::fs::remove_dir_all(&folder.path);
    }

    /// Words saved by other editors read as written: UTF-8 with its mark, UTF-16 either way round by its mark; and
    /// one in UTF-16 written here again, faithfully, in UTF-8.
    #[test]
    fn reads_what_other_editors_saved() {
        assert_eq!(decoded("\u{FEFF}便签".as_bytes()), ("便签".to_string(), true));
        let le: Vec<u8> = [0xFF, 0xFE].into_iter().chain("便签 a".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        let be: Vec<u8> = [0xFE, 0xFF].into_iter().chain("便签 a".encode_utf16().flat_map(u16::to_be_bytes)).collect();
        assert_eq!(decoded(&le), ("便签 a".to_string(), true));
        assert_eq!(decoded(&be), ("便签 a".to_string(), true));
        // Half a unit at its end: read as near as it goes, never written over.
        assert!(!decoded(&le[..le.len() - 1]).1);
        // UTF-32 by its mark (its little-endian mark begins as UTF-16's does), either way round.
        let le32: Vec<u8> = [0xFF, 0xFE, 0, 0].into_iter().chain("便签 a".chars().flat_map(|c| u32::from(c).to_le_bytes())).collect();
        let be32: Vec<u8> = [0, 0, 0xFE, 0xFF].into_iter().chain("便签 a".chars().flat_map(|c| u32::from(c).to_be_bytes())).collect();
        assert_eq!(decoded(&le32), ("便签 a".to_string(), true));
        assert_eq!(decoded(&be32), ("便签 a".to_string(), true));
        // A NUL in what is written: never read faithfully (the box would end the words at it).
        assert!(!decoded(b"a\0b").1);
        let folder = folder("utf16");
        std::fs::create_dir_all(&folder.path).unwrap();
        std::fs::write(folder.path.join("u.md"), &le).unwrap();
        let (text, stamp) = folder.read("u.md").unwrap();
        assert_eq!(text, "便签 a");
        assert!(matches!(folder.write("u.md", &format!("{text}!"), Some(stamp)).unwrap(), Written::Done(_)));
        assert_eq!(std::fs::read(folder.path.join("u.md")).unwrap(), "便签 a!".as_bytes());
        // One not read faithfully: what is written here goes apart.
        std::fs::write(folder.path.join("half.md"), &le[..le.len() - 1]).unwrap();
        let (text, stamp) = folder.read("half.md").unwrap();
        assert!(matches!(folder.write("half.md", &text, Some(stamp)).unwrap(), Written::Apart(..)));
        assert_eq!(std::fs::read(folder.path.join("half.md")).unwrap(), &le[..le.len() - 1]);
        let _ = std::fs::remove_dir_all(&folder.path);
    }
}
