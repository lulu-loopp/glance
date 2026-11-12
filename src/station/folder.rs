//! The station's own folder (`%LOCALAPPDATA%\Glance\Station`): its shelf (`station.json`), and what it keeps itself,
//! each in a file named by Glance (never by what it came with: a name given by a page or a mail could reach out of
//! the folder, or name a link).
//!
//! A kept file being dragged out is held (`hold`) till the program it was dropped on has read it: taken off the shelf
//! meanwhile, it goes as it is let go (`let_go`).

use super::shelf::Shelf;
use std::collections::{HashMap, HashSet};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub struct Folder {
    dir: PathBuf,
    /// The kept files being dragged out, and how many drags hold each.
    held: HashMap<String, usize>,
    /// Those of them taken off the shelf meanwhile: to go as they are let go.
    doomed: HashSet<String>,
}

const SHELF: &str = "station.json";
/// The station's own folder, in Glance's local folder (see `settings::local_dir`).
pub const OWN: &str = "Station";
/// The folder where a kept file is given out under the name it came with (see `named`).
const NAMED: &str = "named";
/// The stream of a file that says where it came from (its "mark of the web"), which Windows, Office and browsers heed
/// as they open it.
const ZONE: &str = ":Zone.Identifier";
/// What that stream says of a file from the Internet.
const INTERNET: &[u8] = b"[ZoneTransfer]\r\nZoneId=3\r\n";

impl Folder {
    /// The station's own folder in the user's local application data.
    pub fn own() -> Folder {
        Folder::at(crate::settings::local_dir().join(OWN))
    }

    pub fn at(dir: PathBuf) -> Folder {
        Folder { dir, held: HashMap::new(), doomed: HashSet::new() }
    }

    /// The shelf as it was last saved: none saved, an empty one; one that cannot be read, put aside (as
    /// `station.json.unread`, not written over) and an empty one.
    pub fn load(&self) -> Shelf {
        let path = self.dir.join(SHELF);
        let Ok(bytes) = std::fs::read(&path) else { return Shelf::default() };
        match serde_json::from_slice(&bytes) {
            Ok(shelf) => shelf,
            Err(error) => {
                crate::journal::note(format!("station: its shelf could not be read ({error}); put aside"));
                let _ = std::fs::rename(&path, self.dir.join(format!("{SHELF}.unread")));
                Shelf::default()
            }
        }
    }

    /// Saves `shelf`, whole or not at all (see `elevation::write_in_place`).
    pub fn save(&self, shelf: &Shelf) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        crate::elevation::write_in_place(&self.dir, SHELF, &serde_json::to_vec(shelf).expect("a shelf is written"))
    }

    /// Keeps `bytes` in a file of its own, of the kind `extension` says (letters and digits, else none), marked as from
    /// the Internet where they are `untrusted`: its name.
    pub fn keep(&self, bytes: &[u8], extension: &str, untrusted: bool) -> std::io::Result<String> {
        self.written(extension, untrusted.then_some(INTERNET), |file| std::io::Write::write_all(file, bytes))
    }

    /// Keeps a copy of the file at `path` in a file of its own, of its kind, marked where it came from as it is: its
    /// name.
    pub fn keep_file(&self, path: &Path) -> std::io::Result<String> {
        let extension = path.extension().and_then(|extension| extension.to_str()).unwrap_or_default();
        let mut from = std::fs::File::open(path)?;
        // (None where it has no mark, or its drive keeps no streams.)
        let zone = std::fs::read(stream(path)).ok();
        self.written(extension, zone.as_deref(), |file| std::io::copy(&mut from, file).map(|_| ()))
    }

    /// A new file of its own of the kind `extension` says (letters and digits, else none), as `write` writes it, with
    /// `zone` as where it came from: its name. (Not written whole: none.)
    fn written(&self, extension: &str, zone: Option<&[u8]>, write: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>) -> std::io::Result<String> {
        std::fs::create_dir_all(&self.dir)?;
        let extension = Self::extension(extension);
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        for attempt in 0u32.. {
            let name = if extension.is_empty() { format!("{stamp:x}-{attempt}") } else { format!("{stamp:x}-{attempt}.{extension}") };
            // (A new file: a name taken, the next.)
            match std::fs::OpenOptions::new().write(true).create_new(true).open(self.dir.join(&name)) {
                Ok(mut file) => {
                    let path = self.dir.join(&name);
                    let written = write(&mut file).and_then(|()| zone.map_or(Ok(()), |zone| std::fs::write(stream(&path), zone)));
                    if let Err(error) = written {
                        drop(file);
                        let _ = std::fs::remove_file(&path);
                        return Err(error);
                    }
                    return Ok(name);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        unreachable!("a name is found")
    }

    /// The name a file sent from a device nearby is kept by (see `adopt`): the `index`th of the sending `session`
    /// (letters and digits: see `nearby`), of the kind `extension` says (letters and digits, else none). Never one
    /// `written` gives (those start with a digit), and the same each time, so a sending taken in again finds what it
    /// moved here already.
    pub fn arrived_name(session: &str, index: usize, extension: &str) -> String {
        let extension = Self::extension(extension);
        if extension.is_empty() { format!("n{session}-{index}") } else { format!("n{session}-{index}.{extension}") }
    }

    /// An extension as the station's own files have one: letters and digits, at most 8, lower case.
    fn extension(extension: &str) -> String {
        extension.chars().filter(char::is_ascii_alphanumeric).take(8).collect::<String>().to_ascii_lowercase()
    }

    /// Moves the file at `source` into its own folder as `name` (see `arrived_name`): opened there and moved by its
    /// handle, only where it is a plain file (no link of any kind on the way to it, nor a second name of it elsewhere)
    /// at exactly that path. How large it is. A file that is not so is refused (`InvalidData`), and left where it is.
    pub fn adopt(&self, source: &Path, name: &str) -> std::io::Result<u64> {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows::core::HSTRING;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, FileRenameInfo, GetFileInformationByHandle, GetFinalPathNameByHandleW, SetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_NAME_NORMALIZED,
            FILE_READ_ATTRIBUTES, FILE_RENAME_INFO, FILE_SHARE_NONE, OPEN_EXISTING, SYNCHRONIZE,
        };
        let refused = |why: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, why.to_string());
        std::fs::create_dir_all(&self.dir)?;
        // (A folder opened too, to be told from a file.)
        let opened = unsafe { CreateFileW(&HSTRING::from(source.as_os_str()), (DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE).0, FILE_SHARE_NONE, None, OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS, None) }.map_err(os_error)?;
        let file = unsafe { OwnedHandle::from_raw_handle(opened.0) };
        let handle = HANDLE(file.as_raw_handle());
        let mut facts = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(handle, &mut facts) }.map_err(os_error)?;
        if facts.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY.0 | FILE_ATTRIBUTE_REPARSE_POINT.0) != 0 {
            return Err(refused("not a plain file"));
        }
        if facts.nNumberOfLinks != 1 {
            return Err(refused("a file of more than one name"));
        }
        // Where it is, every link on the way followed: the path asked for, or another reached through a link.
        let mut found = vec![0u16; 1024];
        let length = unsafe { GetFinalPathNameByHandleW(handle, &mut found, FILE_NAME_NORMALIZED) } as usize;
        if length == 0 || length >= found.len() {
            return Err(std::io::Error::last_os_error());
        }
        let found = String::from_utf16_lossy(&found[..length]);
        let found = match found.strip_prefix(r"\\?\UNC\") {
            Some(shared) => format!(r"\\{shared}"),
            None => found.strip_prefix(r"\\?\").unwrap_or(&found).to_string(),
        };
        if !found.eq_ignore_ascii_case(&source.to_string_lossy()) {
            return Err(refused("not where it was said to be"));
        }
        // Moved by the handle checked: under the name Glance gives it, in its own folder.
        let to: Vec<u16> = self.path(name).as_os_str().encode_wide().collect();
        let size = std::mem::size_of::<FILE_RENAME_INFO>() + to.len() * 2;
        let mut info = vec![0u64; size.div_ceil(8)];
        unsafe {
            let rename = info.as_mut_ptr() as *mut FILE_RENAME_INFO;
            (*rename).FileNameLength = (to.len() * 2) as u32;
            std::ptr::copy_nonoverlapping(to.as_ptr(), std::ptr::addr_of_mut!((*rename).FileName).cast::<u16>(), to.len());
            SetFileInformationByHandle(handle, FileRenameInfo, info.as_ptr().cast(), size as u32).map_err(os_error)?;
        }
        Ok(u64::from(facts.nFileSizeHigh) << 32 | u64::from(facts.nFileSizeLow))
    }

    /// Where a kept file `name` is.
    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(Path::new(name).file_name().unwrap_or_default())
    }

    /// The kept file `name` as it is given out (copied, opened, dragged off), under the name `shown` it came with (as a
    /// name may hold it: see `os::file_name_of`): the same file, linked to by that name in a folder of its own beside
    /// it, which goes with it. As Glance named it where nothing of `shown` is left; none where the link cannot be made.
    pub fn named(&self, name: &str, shown: &str) -> Option<PathBuf> {
        let own = self.path(name);
        let shown = crate::os::file_name_of(shown, 120);
        if shown.is_empty() {
            return Some(own);
        }
        let link = self.named_dir(name).join(shown);
        if !link.exists() {
            std::fs::create_dir_all(link.parent()?).ok()?;
            std::fs::hard_link(&own, &link).ok()?;
        }
        Some(link)
    }

    /// The folder holding the kept file `name` as it is given out (see `named`).
    fn named_dir(&self, name: &str) -> PathBuf {
        self.dir.join(NAMED).join(Path::new(name).file_name().unwrap_or_default())
    }

    /// The kept file `name` gone, and as it was given out (see `named`).
    fn delete(&self, name: &str) {
        let _ = std::fs::remove_dir_all(self.named_dir(name));
        let _ = std::fs::remove_file(self.path(name));
    }

    /// Removes the kept files `names` (one being dragged out, as it is let go).
    pub fn remove<'a>(&mut self, names: impl IntoIterator<Item = &'a str>) {
        for name in names {
            if self.held.contains_key(name) {
                self.doomed.insert(name.to_string());
            } else {
                self.delete(name);
            }
        }
    }

    /// Holds the kept file `name` while a drag takes it out.
    pub fn hold(&mut self, name: &str) {
        *self.held.entry(name.to_string()).or_default() += 1;
    }

    /// Lets go of the kept file `name` (the drag that held it over): removed, if it was taken off the shelf meanwhile
    /// and no other drag holds it.
    pub fn let_go(&mut self, name: &str) {
        let Some(count) = self.held.get_mut(name) else { return };
        *count -= 1;
        if *count == 0 {
            self.held.remove(name);
            if self.doomed.remove(name) {
                self.delete(name);
            }
        }
    }
}

/// A Windows call's error as the system's own error it stands for (a file not found is `NotFound`), where it stands for
/// one.
fn os_error(error: windows::core::Error) -> std::io::Error {
    let code = error.code().0 as u32;
    // (FACILITY_WIN32: the system's error in its low half.)
    if code & 0xFFFF_0000 == 0x8007_0000 {
        std::io::Error::from_raw_os_error((code & 0xFFFF) as i32)
    } else {
        std::io::Error::other(error)
    }
}

/// The stream of the file at `path` that says where it came from.
fn stream(path: &Path) -> PathBuf {
    let mut stream = path.as_os_str().to_owned();
    stream.push(ZONE);
    PathBuf::from(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::station::shelf::What;

    fn folder(test: &str) -> Folder {
        let dir = crate::settings::local_dir().join(format!("station-{test}"));
        let _ = std::fs::remove_dir_all(&dir);
        Folder::at(dir)
    }

    #[test]
    fn the_shelf_is_saved_and_read_back() {
        let folder = folder("saved");
        assert_eq!(folder.load(), Shelf::default());
        let mut shelf = Shelf::default();
        shelf.put(What::Text { text: "你好".into() }, 5, true, true, 0);
        folder.save(&shelf).unwrap();
        assert_eq!(folder.load(), shelf);
        // One that cannot be read: put aside, not written over.
        std::fs::write(folder.dir.join(SHELF), b"{ not").unwrap();
        assert_eq!(folder.load(), Shelf::default());
        assert!(folder.dir.join("station.json.unread").exists());
    }

    #[test]
    fn kept_files_are_named_by_glance() {
        let folder = folder("named");
        let a = folder.keep(b"one", "PNG", false).unwrap();
        let b = folder.keep(b"two", "../../evil\\x", false).unwrap();
        assert!(a.ends_with(".png"));
        assert_ne!(a, b);
        assert!(b.ends_with(".evilx") && !b.contains(['/', '\\']));
        assert_eq!(std::fs::read(folder.path(&b)).unwrap(), b"two");
        // (A name reaching out of the folder stays in it.)
        assert_eq!(folder.path("..\\..\\x"), folder.dir.join("x"));
    }

    #[test]
    fn a_file_dragged_out_goes_as_it_is_let_go() {
        let mut folder = folder("held");
        let name = folder.keep(b"picture", "png", false).unwrap();
        folder.hold(&name);
        folder.remove([name.as_str()]);
        assert!(folder.path(&name).exists(), "kept while the drag holds it");
        folder.let_go(&name);
        assert!(!folder.path(&name).exists());
        // Not held: removed at once.
        let other = folder.keep(b"other", "png", false).unwrap();
        folder.remove([other.as_str()]);
        assert!(!folder.path(&other).exists());
    }
}
