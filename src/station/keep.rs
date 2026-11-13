//! What the station does with what it is given: its shelf and its folder together (see `shelf`, `folder`). Each change
//! is saved as it is made.

use super::clipboard::Content;
use super::folder::Folder;
use super::shelf::{Arrived, Entry, Full, Item, Rules, Shelf, What};
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

    /// The files of the sending `session` from `from` (each where it is, as it was sent, how large it said it is)
    /// moved into its own folder and put on the shelf at `now_ms` (see `Shelf::arrived`), room made for them first:
    /// those taken in (each with how large it is). One moved already (taken in before,
    /// and the shelf not kept then) is taken as it is. One that is not a plain file where it was said to be is left
    /// out (see `Folder::adopt`). One that cannot be moved now leaves the rest as they are, to be taken again.
    pub fn arrived(&mut self, session: &str, files: &[(PathBuf, String, u64)], from: super::shelf::Device, now_ms: u64) -> Result<Vec<(super::shelf::Arrived, u64)>, Refused> {
        let made = (|| {
            self.room_for(files.iter().map(|(_, _, size)| size).sum())?;
            let mut arrived = Vec::new();
            let mut own_bytes = 0;
            for (index, (source, name, _)) in files.iter().enumerate() {
                let extension = std::path::Path::new(name).extension().and_then(|extension| extension.to_str()).unwrap_or_default();
                let file = Folder::arrived_name(session, index, extension);
                let size = match std::fs::metadata(self.folder.path(&file)) {
                    Ok(moved) => moved.len(),
                    Err(_) => match self.folder.adopt(source, &file) {
                        Ok(size) => size,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                            crate::journal::note(format!("nearby: a file sent was left out ({error})"));
                            continue;
                        }
                        Err(_) => return Err(Refused::Unwritten),
                    },
                };
                own_bytes += size;
                arrived.push((super::shelf::Arrived { file, name: name.clone() }, size));
            }
            Ok((arrived, own_bytes))
        })();
        let (arrived, own_bytes) = match made {
            Ok(made) => made,
            Err(refused) => {
                self.refused = Some(refused);
                return Err(refused);
            }
        };
        self.refused = None;
        if !arrived.is_empty() {
            let id = self.shelf.arrived(arrived.iter().map(|(arrived, _)| arrived.clone()).collect(), from, now_ms, own_bytes);
            self.save();
            // (The folders sent made as they are given out now: their tile shows them as folders.)
            if let Some(What::Came { files }) = self.shelf.get(id).map(|item| &item.what) {
                if self.given(files).is_none() {
                    crate::journal::note("nearby: a folder sent could not be made as it is given out");
                }
            }
        }
        Ok(arrived)
    }

    /// The files sent from devices nearby `files` as they are given out (see `shelf::entries`): each sent by itself
    /// under the name it came with, each folder sent as that folder. None where one cannot be.
    pub fn given(&self, files: &[Arrived]) -> Option<Vec<PathBuf>> {
        super::shelf::entries(files)
            .into_iter()
            .map(|entry| match entry {
                Entry::File(arrived) => self.folder.named(&arrived.file, &arrived.name),
                Entry::Folder { name, files } => self.folder.sent_folder(name, &files),
            })
            .collect()
    }

    /// Each of the files sent from devices nearby `files` where it is given out: by itself, or in its folder (see
    /// `given`), in their order. None for one that cannot be.
    pub fn given_each(&self, files: &[Arrived]) -> Vec<Option<PathBuf>> {
        let entries = super::shelf::entries(files);
        files
            .iter()
            .map(|arrived| match arrived.folder() {
                None => self.folder.named(&arrived.file, &arrived.name),
                Some(top) => {
                    let first = entries.iter().find_map(|entry| match entry {
                        Entry::Folder { name, files } if *name == top => files.first(),
                        _ => None,
                    })?;
                    self.folder.sent_file(&self.folder.sent_folder_path(&first.file, top), arrived)
                }
            })
            .collect()
    }

    /// The words sent as messages from `from` (see `nearby::Receipt::words`) put on the shelf at `now_ms`, each as words
    /// given are (a link, text, or text too long for the shelf saved as words), from that device: their numbers. All or
    /// none (what was kept of those made before one that cannot be, removed).
    pub fn words_arrived(&mut self, words: &[String], from: &super::shelf::Device, now_ms: u64) -> Result<Vec<u64>, Refused> {
        let mut made = Vec::new();
        for one in words {
            match self.made(Content::Text(one.clone())) {
                Ok(what) => made.push(what),
                Err(refused) => {
                    for (what, _) in &made {
                        self.folder.remove(what.own_files());
                    }
                    self.refused = Some(refused);
                    return Err(refused);
                }
            }
        }
        self.refused = None;
        let ids: Vec<u64> = made
            .into_iter()
            .map(|(what, own_bytes)| {
                let id = self.shelf.put(what, now_ms, false, false, own_bytes);
                self.shelf.items.last_mut().expect("just put").from = Some(from.clone());
                id
            })
            .collect();
        if !ids.is_empty() {
            self.save();
        }
        Ok(ids)
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
            What::Came { files } => Content::Files(self.given(files)?),
        })
    }

    fn save(&self) {
        if let Err(error) = self.folder.save(&self.shelf) {
            crate::journal::note(format!("station: its shelf not saved ({error})"));
        }
    }
}

/// Whether `text` is a web address alone (one, nothing round it).
pub(crate) fn is_link(text: &str) -> bool {
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

    /// A folder for files sent in a test, where it really is (as the helper's inbox is reached: see
    /// `nearby::helper_folder`).
    fn inbox(test: &str) -> PathBuf {
        std::fs::create_dir_all(crate::settings::local_dir()).unwrap();
        let dir = crate::nearby::real(&crate::settings::local_dir()).unwrap();
        let inbox = dir.join(format!("inbox-{test}"));
        let _ = std::fs::remove_dir_all(&inbox);
        std::fs::create_dir_all(&inbox).unwrap();
        inbox
    }

    fn device(fingerprint: &str) -> super::super::shelf::Device {
        super::super::shelf::Device { alias: "Z Fold".into(), kind: Some("mobile".into()), fingerprint: fingerprint.into() }
    }

    /// Files sent are moved in under Glance's names, given by the names they were sent by, and what one device sends
    /// within a minute joins what it sent before.
    #[test]
    fn files_sent_are_moved_in_and_join_within_a_minute() {
        let inbox = inbox("join");
        std::fs::write(inbox.join("a1.jpg"), b"one").unwrap();
        std::fs::write(inbox.join("b2.png"), b"two!").unwrap();
        let mut keep = keep("arrived", Rules::default());
        let taken = keep.arrived("s1", &[(inbox.join("a1.jpg"), "照片.jpg".into(), 3)], device("AB"), 1_000).unwrap();
        assert_eq!(taken.len(), 1);
        assert!(!inbox.join("a1.jpg").exists(), "moved, not copied");
        keep.arrived("s2", &[(inbox.join("b2.png"), "截图.png".into(), 4)], device("AB"), 30_000).unwrap();
        assert_eq!(keep.shelf.items.len(), 1);
        let item = keep.shelf.items[0].clone();
        assert_eq!((item.own_bytes, item.added_ms, item.from.as_ref().map(|from| from.alias.as_str())), (7, 30_000, Some("Z Fold")));
        let Some(Content::Files(given)) = keep.content_of(&item) else { panic!("files") };
        assert_eq!(given.iter().map(|path| path.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>(), ["照片.jpg", "截图.png"]);
        assert_eq!(std::fs::read(&given[1]).unwrap(), b"two!");
        // Later, or from another device: on its own.
        std::fs::write(inbox.join("c3.jpg"), b"3").unwrap();
        keep.arrived("s3", &[(inbox.join("c3.jpg"), "c.jpg".into(), 1)], device("AB"), 30_000 + super::super::shelf::TOGETHER_MS + 1).unwrap();
        std::fs::write(inbox.join("d4.jpg"), b"4").unwrap();
        keep.arrived("s4", &[(inbox.join("d4.jpg"), "d.jpg".into(), 1)], device("CD"), 30_000 + super::super::shelf::TOGETHER_MS + 2).unwrap();
        assert_eq!(keep.shelf.items.len(), 3);
        // Taken off: its files go.
        keep.remove(item.id);
        assert!(!given[1].exists());
    }

    /// A folder sent is one thing on the station, given out as that folder, its files each in its place in it (the
    /// same files, not copies), beside a file sent by itself; taken off, the folder goes with its files.
    #[test]
    fn a_folder_sent_is_given_out_as_that_folder() {
        let inbox = inbox("folder");
        for (name, bytes) in [("a.pdf", &b"quote"[..]), ("b.dwg", b"front"), ("c.dwg", b"side"), ("d.txt", b"alone")] {
            std::fs::write(inbox.join(name), bytes).unwrap();
        }
        let mut keep = keep("arrived-folder", Rules::default());
        let sent = [
            (inbox.join("a.pdf"), "报价资料/报价单 v3.pdf".to_string(), 5),
            (inbox.join("b.dwg"), "报价资料/图纸/外壳正视图.dwg".to_string(), 5),
            (inbox.join("d.txt"), "说明.txt".to_string(), 5),
            (inbox.join("c.dwg"), "报价资料/图纸/外壳侧视图.dwg".to_string(), 4),
        ];
        keep.arrived("s1", &sent, device("AB"), 1_000).unwrap();
        let item = keep.shelf.items[0].clone();
        let What::Came { files } = &item.what else { panic!("came") };
        assert_eq!(super::super::shelf::entries(files).len(), 2, "the folder, and the file by itself");
        let Some(Content::Files(given)) = keep.content_of(&item) else { panic!("files") };
        assert_eq!(given.len(), 2);
        assert!(given[0].is_dir() && given[0].ends_with("报价资料"));
        assert_eq!(std::fs::read(given[0].join("报价单 v3.pdf")).unwrap(), b"quote");
        assert_eq!(std::fs::read(given[0].join("图纸").join("外壳正视图.dwg")).unwrap(), b"front");
        assert_eq!(std::fs::read(given[0].join("图纸").join("外壳侧视图.dwg")).unwrap(), b"side");
        assert_eq!(given[1].file_name().unwrap(), "说明.txt");
        // Each file where it is given out: in its folder, or by itself.
        let each = keep.given_each(files);
        assert_eq!(each[1].as_deref(), Some(given[0].join("图纸").join("外壳正视图.dwg").as_path()));
        assert_eq!(each[2].as_deref(), Some(given[1].as_path()));
        // Given out again: the same.
        assert_eq!(keep.content_of(&item), Some(Content::Files(given.clone())));
        keep.remove(item.id);
        assert!(!given[0].exists() && !given[1].exists());
    }

    /// A sending taken in again (its receipt not taken away before Glance ended) finds what it moved in already, and
    /// puts it on the shelf; a file not there is passed over.
    #[test]
    fn a_sending_taken_in_again_finds_what_was_moved() {
        let inbox = inbox("again");
        std::fs::write(inbox.join("a1.jpg"), b"one").unwrap();
        let mut keep = keep("arrived-again", Rules::default());
        let name = Folder::arrived_name("s1", 0, "jpg");
        keep.folder.adopt(&inbox.join("a1.jpg"), &name).unwrap();
        let files = [(inbox.join("a1.jpg"), "a.jpg".to_string(), 3), (inbox.join("gone.jpg"), "gone.jpg".to_string(), 9)];
        let taken = keep.arrived("s1", &files, device("AB"), 0).unwrap();
        assert_eq!(taken.iter().map(|(arrived, size)| (arrived.file.as_str(), *size)).collect::<Vec<_>>(), [(name.as_str(), 3)]);
        assert_eq!(keep.shelf.own_bytes(), 3);
    }

    /// A file is moved in only as it is, where it was said to be: not one of two names, nor one reached through a
    /// link to another folder.
    #[test]
    fn only_a_plain_file_where_it_was_said_to_be_is_moved_in() {
        let inbox = inbox("plain");
        let folder = Folder::at(crate::settings::local_dir().join("station-plain"));
        std::fs::write(inbox.join("two.jpg"), b"x").unwrap();
        std::fs::hard_link(inbox.join("two.jpg"), inbox.join("other.jpg")).unwrap();
        let refused = folder.adopt(&inbox.join("two.jpg"), "n1-0.jpg").unwrap_err();
        assert_eq!(refused.kind(), std::io::ErrorKind::InvalidData);
        assert!(inbox.join("two.jpg").exists(), "left where it was");
        // Through a junction to a folder elsewhere.
        let elsewhere = inbox.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("c.jpg"), b"c").unwrap();
        let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(inbox.join("through")).arg(&elsewhere).output().unwrap();
        assert!(made.status.success());
        let refused = folder.adopt(&inbox.join("through").join("c.jpg"), "n1-1.jpg").unwrap_err();
        assert_eq!(refused.kind(), std::io::ErrorKind::InvalidData);
        assert!(elsewhere.join("c.jpg").exists());
        // A folder is no file.
        assert_eq!(folder.adopt(&elsewhere, "n1-2").unwrap_err().kind(), std::io::ErrorKind::InvalidData);
        // As it is: moved.
        std::fs::write(inbox.join("plain.jpg"), b"p").unwrap();
        assert_eq!(folder.adopt(&inbox.join("plain.jpg"), "n1-3.jpg").unwrap(), 1);
        assert_eq!(std::fs::read(folder.path("n1-3.jpg")).unwrap(), b"p");
    }

    fn stream_of(path: &std::path::Path) -> PathBuf {
        let mut stream = path.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        PathBuf::from(stream)
    }
}
