//! What the station does with what it is given: its shelf and its folder together (see `shelf`, `folder`). Each change
//! is saved as it is made.

use super::clipboard::Content;
use super::folder::Folder;
use super::shelf::{Full, Item, Rules, Shelf, What};
use std::path::PathBuf;

/// The longest text kept on the shelf itself (UTF-8 bytes); longer, it is kept as a file of the station's own (the
/// shelf is written whole at each change).
const TEXT_MOST: usize = 16 << 10;
/// The most things copied kept in Recently copied, not pinned (the oldest go first).
const COPIED_MOST: usize = 200;

/// Why what was given was not taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// No room left but what is pinned holds (see `shelf::Full`).
    Full,
    /// What it keeps of it could not be written (its disk full, a file to be copied in use or gone).
    Unwritten,
}

impl From<Full> for Refused {
    fn from(_: Full) -> Refused {
        Refused::Full
    }
}

pub struct Keep {
    pub shelf: Shelf,
    pub folder: Folder,
    pub rules: Rules,
    /// Files put on it kept as a copy too.
    pub always_copy: bool,
    /// Why what was last given could not be taken, where it could not.
    pub refused: Option<Refused>,
}

impl Keep {
    /// The station as it was last saved in `folder`.
    pub fn open(folder: Folder, rules: Rules, always_copy: bool) -> Keep {
        Keep { shelf: folder.load(), folder, rules, always_copy, refused: None }
    }

    /// Takes `content` onto the shelf at `now_ms` (`copied`: as it was copied, into Recently copied; `stays_here`: see
    /// `shelf::Item::stays_here`): its number; why not, where what it keeps of it has no room or cannot be written.
    pub fn take(&mut self, content: Content, copied: bool, stays_here: bool, now_ms: u64) -> Result<u64, Refused> {
        let made = self.made(content);
        let (what, own_bytes) = match made {
            Ok(made) => made,
            Err(refused) => {
                self.refused = Some(refused);
                return Err(refused);
            }
        };
        self.refused = None;
        // Copied again as it was before: the one before goes (the latest stands for both).
        if copied {
            let again: Vec<u64> = self.shelf.items.iter().filter(|item| item.copied && !item.pinned && item.what == what).map(|item| item.id).collect();
            for id in again {
                self.remove(id);
            }
        }
        let id = self.shelf.put(what, now_ms, copied, stays_here, own_bytes);
        if copied {
            let copies: Vec<u64> = self.shelf.items.iter().filter(|item| item.copied && !item.pinned).map(|item| item.id).collect();
            for id in &copies[..copies.len().saturating_sub(COPIED_MOST)] {
                self.remove(*id);
            }
        }
        self.save();
        Ok(id)
    }

    /// What `content` is on the shelf, what it keeps of it written to its folder (room made for it first), and how many
    /// bytes that is.
    fn made(&mut self, content: Content) -> Result<(What, u64), Refused> {
        match content {
            Content::Text(text) => {
                let trimmed = text.trim();
                if is_link(trimmed) {
                    Ok((What::Link { url: trimmed.to_string() }, 0))
                } else if text.len() <= TEXT_MOST {
                    Ok((What::Text { text }, 0))
                } else {
                    let first = trimmed.lines().next().unwrap_or_default().chars().take(40).collect::<String>();
                    let file = self.kept(text.as_bytes(), "txt", false)?;
                    Ok((What::Saved { file, name: format!("{first}.txt"), words: true }, text.len() as u64))
                }
            }
            Content::Png(png) => Ok((What::Picture { file: self.kept(&png, "png", false)? }, png.len() as u64)),
            // (Given by another program, from where it does not say: as from the Internet, as Windows takes such a
            // file, not trusted once it is on the station.)
            Content::Saved { name, bytes } => {
                let extension = std::path::Path::new(&name).extension().and_then(|extension| extension.to_str()).unwrap_or_default().to_string();
                Ok((What::Saved { file: self.kept(&bytes, &extension, true)?, name, words: false }, bytes.len() as u64))
            }
            Content::Files(paths) => {
                if !self.always_copy {
                    return Ok((What::Files { paths, kept: Vec::new() }, 0));
                }
                // A copy of each file (not of a folder: a folder is pointed to only). One not copied, none kept: they
                // are not taken (else they would seem kept, and one be lost with its file).
                let files: Vec<Option<u64>> = paths.iter().map(|path| std::fs::metadata(path).ok().filter(|meta| meta.is_file()).map(|meta| meta.len())).collect();
                self.room_for(files.iter().flatten().sum())?;
                let mut kept = Vec::new();
                for (path, file) in paths.iter().zip(&files) {
                    match file.map(|_| self.folder.keep_file(path)) {
                        Some(Ok(name)) => kept.push(name),
                        Some(Err(_)) => {
                            self.folder.remove(kept.iter().map(String::as_str).filter(|name| !name.is_empty()));
                            return Err(Refused::Unwritten);
                        }
                        None => kept.push(String::new()),
                    }
                }
                Ok((What::Files { paths, kept }, files.iter().flatten().sum()))
            }
        }
    }

    /// `bytes` kept as a file of its own of the kind `extension` says (`untrusted`: marked as from the Internet), room
    /// made for them: its name.
    fn kept(&mut self, bytes: &[u8], extension: &str, untrusted: bool) -> Result<String, Refused> {
        self.room_for(bytes.len() as u64)?;
        self.folder.keep(bytes, extension, untrusted).map_err(|_| Refused::Unwritten)
    }

    /// Room for `bytes` more of its own (see `Shelf::room_for`), what went for it removed.
    fn room_for(&mut self, bytes: u64) -> Result<(), Full> {
        let gone = self.shelf.room_for(bytes, self.rules)?;
        for item in &gone {
            self.folder.remove(item.what.own_files());
        }
        Ok(())
    }

    /// Takes `id` off the shelf, and what it kept of it.
    pub fn remove(&mut self, id: u64) {
        if let Some(item) = self.shelf.take(id) {
            self.folder.remove(item.what.own_files());
            self.save();
        }
    }

    /// All that is not pinned taken off: of what was put on it, or (`copied`) of what was copied lately.
    pub fn clear(&mut self, copied: bool) {
        let loose: Vec<u64> = self.shelf.items.iter().filter(|item| !item.pinned && item.copied == copied).map(|item| item.id).collect();
        for id in loose {
            self.remove(id);
        }
    }

    pub fn pin(&mut self, id: u64, pinned: bool) {
        self.shelf.pin(id, pinned);
        self.save();
    }

    /// Takes off what has stayed its days (see `Rules::keep_days`).
    pub fn expire(&mut self, now_ms: u64) {
        let gone = self.shelf.expire(self.rules, now_ms);
        for item in &gone {
            self.folder.remove(item.what.own_files());
        }
        if !gone.is_empty() {
            self.save();
        }
    }

    /// What `item` gives as it is copied or dragged off: its text, its files (a file gone from where it was, its copy
    /// where one is kept), or its picture. A file of its own goes by the name it came with (see `Folder::named`).
    pub fn content_of(&self, item: &Item) -> Option<Content> {
        Some(match &item.what {
            What::Text { text } => Content::Text(text.clone()),
            What::Link { url } => Content::Text(url.clone()),
            What::Files { paths, kept } => Content::Files(
                paths
                    .iter()
                    .enumerate()
                    .map(|(at, path)| match kept.get(at).filter(|name| !name.is_empty() && !path.exists()) {
                        Some(name) => self.folder.named(name, &path.file_name().unwrap_or_default().to_string_lossy()),
                        None => Some(path.clone()),
                    })
                    .collect::<Option<Vec<PathBuf>>>()?,
            ),
            What::Picture { file } => Content::Png(std::fs::read(self.folder.path(file)).ok()?),
            What::Saved { file, words: true, .. } => Content::Text(std::fs::read_to_string(self.folder.path(file)).ok()?),
            What::Saved { file, name, words: false } => Content::Files(vec![self.folder.named(file, name)?]),
        })
    }

    fn save(&self) {
        if let Err(error) = self.folder.save(&self.shelf) {
            crate::journal::note(format!("station: its shelf not saved ({error})"));
        }
    }
}

/// Whether `text` is a web address alone (one, nothing round it).
pub(super) fn is_link(text: &str) -> bool {
    (text.starts_with("https://") || text.starts_with("http://")) && text.len() > "https://".len() && !text.chars().any(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keep(test: &str, rules: Rules) -> Keep {
        let dir = crate::settings::local_dir().join(format!("keep-{test}"));
        let _ = std::fs::remove_dir_all(&dir);
        Keep::open(Folder::at(dir), rules, false)
    }

    #[test]
    fn what_is_given_is_kept_as_what_it_is() {
        let mut keep = keep("kinds", Rules::default());
        keep.take(Content::Text("  https://example.com/a?b=1 \n".into()), true, false, 0).unwrap();
        keep.take(Content::Text("hello there".into()), true, false, 0).unwrap();
        keep.take(Content::Text("x".repeat(TEXT_MOST + 1)), true, false, 0).unwrap();
        keep.take(Content::Png(vec![1, 2, 3]), false, true, 0).unwrap();
        let kinds: Vec<&What> = keep.shelf.items.iter().map(|item| &item.what).collect();
        assert_eq!(kinds[0], &What::Link { url: "https://example.com/a?b=1".into() });
        assert_eq!(kinds[1], &What::Text { text: "hello there".into() });
        assert!(matches!(kinds[2], What::Saved { name, words: true, .. } if name.ends_with(".txt")));
        // (Given back as the words it was, not as a file of them.)
        assert_eq!(keep.content_of(&keep.shelf.items[2]), Some(Content::Text("x".repeat(TEXT_MOST + 1))));
        assert!(matches!(kinds[3], What::Picture { .. }));
        assert_eq!(keep.shelf.own_bytes(), (TEXT_MOST + 1 + 3) as u64);
        assert!(keep.shelf.items[3].stays_here);
        // Saved as it changed: opened again, the same.
        let again = Keep::open(Folder::at(crate::settings::local_dir().join("keep-kinds")), Rules::default(), false);
        assert_eq!(again.shelf, keep.shelf);
    }

    #[test]
    fn copied_again_stands_once_and_the_oldest_copies_go() {
        let mut keep = keep("again", Rules::default());
        keep.take(Content::Text("a".into()), true, false, 0).unwrap();
        keep.take(Content::Text("b".into()), true, false, 1).unwrap();
        keep.take(Content::Text("a".into()), true, false, 2).unwrap();
        let texts: Vec<&What> = keep.shelf.items.iter().map(|item| &item.what).collect();
        assert_eq!(texts, [&What::Text { text: "b".into() }, &What::Text { text: "a".into() }]);
        // Put by hand, it is not a copy: both stand.
        keep.take(Content::Text("a".into()), false, false, 3).unwrap();
        assert_eq!(keep.shelf.items.len(), 3);
        for at in 0..COPIED_MOST + 5 {
            keep.take(Content::Text(format!("copy {at}")), true, false, 10 + at as u64).unwrap();
        }
        assert_eq!(keep.shelf.items.iter().filter(|item| item.copied).count(), COPIED_MOST);
        assert!(keep.shelf.items.iter().any(|item| !item.copied));
    }

    #[test]
    fn full_is_said_and_nothing_is_dropped() {
        let mut keep = keep("full", Rules { keep_days: 7, most_bytes: 10 });
        let pinned = keep.take(Content::Png(vec![0; 8]), false, false, 0).unwrap();
        keep.pin(pinned, true);
        assert_eq!(keep.take(Content::Png(vec![0; 5]), false, false, 1), Err(Refused::Full));
        assert_eq!(keep.refused, Some(Refused::Full));
        assert_eq!(keep.shelf.items.len(), 1);
        keep.take(Content::Png(vec![0; 2]), false, false, 2).unwrap();
        assert_eq!(keep.refused, None);
    }

    #[test]
    fn a_file_gone_is_given_from_its_copy() {
        let dir = crate::settings::local_dir().join("keep-gone-source");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, b"words").unwrap();
        let mut keep = keep("gone", Rules::default());
        keep.always_copy = true;
        let id = keep.take(Content::Files(vec![file.clone(), dir.clone()]), false, false, 0).unwrap();
        let item = keep.shelf.get(id).unwrap().clone();
        assert_eq!(item.own_bytes, 5);
        assert_eq!(keep.content_of(&item), Some(Content::Files(vec![file.clone(), dir.clone()])));
        std::fs::remove_file(&file).unwrap();
        let Some(Content::Files(given)) = keep.content_of(&item) else { panic!("files") };
        assert_eq!(std::fs::read(&given[0]).unwrap(), b"words");
        assert_eq!(given[0].file_name().unwrap(), "a.txt", "given by the name it had");
        // Taken off: its copy goes with it.
        keep.remove(id);
        assert!(!given[0].exists());
    }

    #[test]
    fn files_not_copied_are_not_taken() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = crate::settings::local_dir().join("keep-held-source");
        std::fs::create_dir_all(&dir).unwrap();
        let (free, held) = (dir.join("free.txt"), dir.join("held.txt"));
        std::fs::write(&free, b"free").unwrap();
        std::fs::write(&held, b"held").unwrap();
        let mut keep = keep("held", Rules::default());
        keep.always_copy = true;
        // (Held by another program that lets none read it.)
        let holding = std::fs::OpenOptions::new().read(true).share_mode(0).open(&held).unwrap();
        assert_eq!(keep.take(Content::Files(vec![free.clone(), held.clone()]), false, false, 0), Err(Refused::Unwritten));
        assert_eq!(keep.refused, Some(Refused::Unwritten));
        assert!(keep.shelf.items.is_empty());
        let left = std::fs::read_dir(crate::settings::local_dir().join("keep-held")).map_or(0, |dir| dir.filter(|entry| entry.as_ref().unwrap().file_name() != "station.json").count());
        assert_eq!(left, 0, "the copy made before is not left behind");
        drop(holding);
        keep.take(Content::Files(vec![free, held]), false, false, 1).unwrap();
        assert_eq!(keep.refused, None);
    }

    #[test]
    fn what_is_kept_says_where_it_came_from() {
        let dir = crate::settings::local_dir().join("keep-zone-source");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.docx");
        std::fs::write(&file, b"words").unwrap();
        let zone = b"[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://example.com/a.docx\r\n";
        std::fs::write(stream_of(&file), zone).unwrap();
        let plain = dir.join("b.txt");
        std::fs::write(&plain, b"mine").unwrap();
        let mut keep = keep("zone", Rules::default());
        keep.always_copy = true;
        let id = keep.take(Content::Files(vec![file, plain]), false, false, 0).unwrap();
        let What::Files { kept, .. } = &keep.shelf.get(id).unwrap().what else { panic!("files") };
        assert_eq!(std::fs::read(stream_of(&keep.folder.path(&kept[0]))).unwrap(), zone);
        assert!(std::fs::read(stream_of(&keep.folder.path(&kept[1]))).is_err(), "none where its file had none");
        // Given by a program with no path: as from the Internet.
        let id = keep.take(Content::Saved { name: "a.xlsm".into(), bytes: b"cells".to_vec() }, false, false, 1).unwrap();
        let What::Saved { file, .. } = &keep.shelf.get(id).unwrap().what else { panic!("saved") };
        assert_eq!(std::fs::read(stream_of(&keep.folder.path(file))).unwrap(), b"[ZoneTransfer]\r\nZoneId=3\r\n");
    }

    #[test]
    fn a_file_given_with_no_path_goes_by_its_name() {
        let mut keep = keep("named", Rules::default());
        let id = keep.take(Content::Saved { name: "config.json".into(), bytes: b"{}".to_vec() }, false, false, 0).unwrap();
        let item = keep.shelf.get(id).unwrap().clone();
        let Some(Content::Files(given)) = keep.content_of(&item) else { panic!("files") };
        assert_eq!(given[0].file_name().unwrap(), "config.json");
        assert_eq!(std::fs::read(&given[0]).unwrap(), b"{}");
        assert_eq!(keep.content_of(&item), Some(Content::Files(given.clone())), "given again as it was");
        // A name that names a device: as Glance named it.
        let id = keep.take(Content::Saved { name: "CON.txt".into(), bytes: b"x".to_vec() }, false, false, 1).unwrap();
        let What::Saved { file, .. } = &keep.shelf.get(id).unwrap().what else { panic!("saved") };
        assert_eq!(keep.content_of(&keep.shelf.get(id).unwrap().clone()), Some(Content::Files(vec![keep.folder.path(file)])));
        keep.remove(item.id);
        assert!(!given[0].exists() && !given[0].parent().unwrap().exists());
    }

    fn stream_of(path: &std::path::Path) -> PathBuf {
        let mut stream = path.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        PathBuf::from(stream)
    }
}
