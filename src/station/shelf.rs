//! What the station holds: the things put on it, each as what it is (files where they are, or what it keeps a copy of
//! itself: text, a link, a picture), when it was put there, and whether it is pinned; and the rules it keeps them by.
//!
//! - What is not pinned goes after a while (`Rules::keep_days`, 7 by default); what is pinned stays.
//! - What the station keeps a copy of itself takes no more than `Rules::most_bytes` (2 GB by default): to make room,
//!   what it keeps of the oldest things not pinned goes first; pinned things never go, and what is put on it is never
//!   dropped unsaid: with no room left even then, it is not taken, and the person is told the station is full.
//!
//! Only the shelf: its files are the folder's (see `super::folder`), its windows the station's thread's.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// One thing on the station.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// Its own number, never given to another on this shelf.
    pub id: u64,
    pub what: What,
    /// When it was put there (ms since 1970).
    pub added_ms: u64,
    #[serde(default)]
    pub pinned: bool,
    /// Copied to the clipboard (in Recently copied), not put there by hand.
    #[serde(default)]
    pub copied: bool,
    /// Not to leave this PC (its copier said so: `CanUploadToCloudClipboard` 0): copied back with that said again, and
    /// never sent to the phone.
    #[serde(default)]
    pub stays_here: bool,
    /// The bytes of what the station keeps of it in its own folder (none for files only pointed to).
    #[serde(default)]
    pub own_bytes: u64,
    /// The device nearby it was sent from (see `nearby`), where it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Device>,
}

/// A device nearby files came from, as it called itself, of what kind (LocalSend's `mobile`, `desktop`...), and by the
/// fingerprint of its certificate (its own beyond doubt where it presented the certificate: see `nearby`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub fingerprint: String,
}

/// A file sent from a device nearby: its file's name in the station's own folder, and the name it was sent by.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Arrived {
    pub file: String,
    pub name: String,
}

/// What a thing on the station is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum What {
    /// Files and folders put on it together, where they are (not copied: `kept` names a copy of each kept in the
    /// station's own folder, in the same order, where one is kept).
    Files { paths: Vec<PathBuf>, #[serde(default, skip_serializing_if = "Vec::is_empty")] kept: Vec<String> },
    /// Text, kept as it is.
    Text { text: String },
    /// A web address (not fetched: fetching would tell its site of each one put here).
    Link { url: String },
    /// A picture the station keeps itself (pasted, dragged from a page, sent by the phone): its file's name in its own
    /// folder.
    Picture { file: String },
    /// A file that had no path (dragged from a browser or a mail, its contents given): saved in its own folder as it
    /// came, by its name there and the name it was given. Or text too long to keep on the shelf itself (`words`),
    /// saved so, named by its first words: words still, given back as words.
    Saved {
        file: String,
        name: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        words: bool,
    },
    /// Files sent from a device nearby (see `Item::from`), together: those of one sending, and of those after it from
    /// the same device within a minute. Each kept in its own folder, as it came.
    Came { files: Vec<Arrived> },
}

impl What {
    /// The files of its own folder it keeps (to be removed with it).
    pub fn own_files(&self) -> Vec<&str> {
        match self {
            What::Files { kept, .. } => kept.iter().map(String::as_str).collect(),
            What::Picture { file } | What::Saved { file, .. } => vec![file.as_str()],
            What::Came { files } => files.iter().map(|arrived| arrived.file.as_str()).collect(),
            What::Text { .. } | What::Link { .. } => Vec::new(),
        }
    }
}

/// Which of what was copied lately a page shows: all; words (text, and links: a link copied is words too); pictures
/// (what was copied as a picture, a screenshot among them, and picture files: files all of them pictures); files
/// (files and folders copied where they are, picture files too, and files that came with no path).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Filter {
    #[default]
    All,
    Words,
    Pictures,
    Files,
}

impl Filter {
    pub const ALL: [Filter; 4] = [Filter::All, Filter::Words, Filter::Pictures, Filter::Files];

    /// Whether `what` is one it shows.
    pub fn takes(self, what: &What) -> bool {
        let picture = |path: &std::path::Path| path.extension().is_some_and(|extension| super::recent::is_picture(&extension.to_string_lossy().to_lowercase()));
        match self {
            Filter::All => true,
            Filter::Words => matches!(what, What::Text { .. } | What::Link { .. } | What::Saved { words: true, .. }),
            Filter::Pictures => match what {
                What::Picture { .. } => true,
                What::Files { paths, .. } => !paths.is_empty() && paths.iter().all(|path| picture(path)),
                What::Saved { name, words, .. } => !words && picture(std::path::Path::new(name)),
                What::Came { files } => !files.is_empty() && files.iter().all(|arrived| picture(std::path::Path::new(&arrived.name))),
                What::Text { .. } | What::Link { .. } => false,
            },
            Filter::Files => matches!(what, What::Files { .. } | What::Saved { words: false, .. } | What::Came { .. }),
        }
    }
}

/// How long the station keeps what is not pinned, and how much of its own it keeps at most.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rules {
    pub keep_days: u32,
    pub most_bytes: u64,
}

impl Default for Rules {
    fn default() -> Self {
        Rules { keep_days: 7, most_bytes: 2 << 30 }
    }
}

/// No room for what was to be put on the station, even with all it keeps of what is not pinned gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Full;

/// The things on the station, the oldest first.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Shelf {
    pub items: Vec<Item>,
    /// The number the next thing put on it takes.
    #[serde(default)]
    next: u64,
}

const DAY_MS: u64 = 24 * 60 * 60 * 1000;
/// How soon after what came from a device what it sends next joins it (ms).
pub const TOGETHER_MS: u64 = 60 * 1000;

impl Shelf {
    /// Puts `what` on the shelf at `now_ms` (`own_bytes` of it kept in its own folder; see `room_for` first, for that
    /// much): its number.
    pub fn put(&mut self, what: What, now_ms: u64, copied: bool, stays_here: bool, own_bytes: u64) -> u64 {
        let id = self.next;
        self.next += 1;
        self.items.push(Item { id, what, added_ms: now_ms, pinned: false, copied, stays_here, own_bytes, from: None });
        id
    }

    /// Files sent from `from`, kept in its own folder (`own_bytes` of them, room made for them first), put on the shelf
    /// at `now_ms`: with what came from that device last, where that came within `TOGETHER_MS` before and is not
    /// pinned, else on their own. The number of what holds them.
    pub fn arrived(&mut self, files: Vec<Arrived>, from: Device, now_ms: u64, own_bytes: u64) -> u64 {
        let last = self.items.iter().rposition(|item| {
            matches!(item.what, What::Came { .. }) && !item.pinned && item.from.as_ref().is_some_and(|device| device.fingerprint == from.fingerprint) && now_ms.saturating_sub(item.added_ms) <= TOGETHER_MS
        });
        match last {
            // (Joined, it is as new as what joined it: the newest, last.)
            Some(at) => {
                let mut item = self.items.remove(at);
                if let What::Came { files: held } = &mut item.what {
                    held.extend(files);
                }
                item.added_ms = now_ms;
                item.own_bytes += own_bytes;
                item.from = Some(from);
                let id = item.id;
                self.items.push(item);
                id
            }
            None => {
                let id = self.put(What::Came { files }, now_ms, false, false, own_bytes);
                self.items.last_mut().expect("just put").from = Some(from);
                id
            }
        }
    }

    pub fn get(&self, id: u64) -> Option<&Item> {
        self.items.iter().find(|item| item.id == id)
    }

    /// Takes `id` off the shelf: what it was (its own files to be removed).
    pub fn take(&mut self, id: u64) -> Option<Item> {
        let at = self.items.iter().position(|item| item.id == id)?;
        Some(self.items.remove(at))
    }

    pub fn pin(&mut self, id: u64, pinned: bool) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.pinned = pinned;
        }
    }

    /// The bytes it keeps of its own, of all on it.
    pub fn own_bytes(&self) -> u64 {
        self.items.iter().map(|item| item.own_bytes).sum()
    }

    /// Takes off what has been on it longer than `rules` keep it, not pinned: what went.
    pub fn expire(&mut self, rules: Rules, now_ms: u64) -> Vec<Item> {
        let keep = u64::from(rules.keep_days) * DAY_MS;
        let (gone, kept) = std::mem::take(&mut self.items).into_iter().partition(|item| !item.pinned && now_ms.saturating_sub(item.added_ms) >= keep);
        self.items = kept;
        gone
    }

    /// Makes room for `bytes` more of its own within `rules`: the oldest things not pinned that keep something of
    /// their own taken off till there is (what went); else, with no room even with all of those gone, nothing taken
    /// off, and `Full`.
    pub fn room_for(&mut self, bytes: u64, rules: Rules) -> Result<Vec<Item>, Full> {
        let pinned: u64 = self.items.iter().filter(|item| item.pinned).map(|item| item.own_bytes).sum();
        if pinned.saturating_add(bytes) > rules.most_bytes {
            return Err(Full);
        }
        let mut over = (self.own_bytes() + bytes).saturating_sub(rules.most_bytes);
        let mut gone = Vec::new();
        while over > 0 {
            // (There is one: what is not pinned holds the rest, at least `over`.)
            let at = self.items.iter().position(|item| !item.pinned && item.own_bytes > 0).expect("something not pinned holds the rest");
            let item = self.items.remove(at);
            over = over.saturating_sub(item.own_bytes);
            gone.push(item);
        }
        Ok(gone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &str) -> What {
        What::Text { text: t.into() }
    }

    fn picture(name: &str) -> What {
        What::Picture { file: name.into() }
    }

    #[test]
    fn what_was_copied_is_sorted_into_words_pictures_and_files() {
        let link = What::Link { url: "https://glancepc.com".into() };
        let files = |paths: &[&str]| What::Files { paths: paths.iter().map(PathBuf::from).collect(), kept: Vec::new() };
        let saved = |name: &str| What::Saved { file: "1".into(), name: name.into(), words: false };
        let all = [
            text("hello"),
            link,
            picture("1.png"),
            files(&[r"D:\Photos\a.JPG", r"D:\Photos\b.png"]),
            files(&[r"D:\Photos\a.jpg", r"D:\Notes\b.txt"]),
            files(&[r"D:\Photos"]),
            saved("bill.pdf"),
            saved("cat.webp"),
            What::Saved { file: "2".into(), name: "很长的一段文字.txt".into(), words: true },
        ];
        let taken = |filter: Filter| all.iter().enumerate().filter(|(_, what)| filter.takes(what)).map(|(at, _)| at).collect::<Vec<_>>();
        assert_eq!(taken(Filter::All), [0, 1, 2, 3, 4, 5, 6, 7, 8]);
        // (Text too long for the shelf, saved as a file, is words.)
        assert_eq!(taken(Filter::Words), [0, 1, 8]);
        // Picture files are pictures and files both (files with one not a picture, a folder, files alone).
        assert_eq!(taken(Filter::Pictures), [2, 3, 7]);
        assert_eq!(taken(Filter::Files), [3, 4, 5, 6, 7]);
    }

    #[test]
    fn what_is_not_pinned_goes_after_its_days() {
        let rules = Rules::default();
        let mut shelf = Shelf::default();
        let old = shelf.put(text("old"), 0, false, false, 0);
        let pinned = shelf.put(text("pinned"), 0, false, false, 0);
        let new = shelf.put(text("new"), 6 * DAY_MS, false, false, 0);
        shelf.pin(pinned, true);
        let gone = shelf.expire(rules, 7 * DAY_MS);
        assert_eq!(gone.iter().map(|item| item.id).collect::<Vec<_>>(), [old]);
        assert_eq!(shelf.items.iter().map(|item| item.id).collect::<Vec<_>>(), [pinned, new]);
        // Kept longer, as set.
        let mut longer = shelf.clone();
        assert!(longer.expire(Rules { keep_days: 30, ..rules }, 13 * DAY_MS).is_empty());
    }

    #[test]
    fn room_is_made_from_the_oldest_not_pinned() {
        let rules = Rules { keep_days: 7, most_bytes: 100 };
        let mut shelf = Shelf::default();
        let first = shelf.put(picture("a"), 0, false, false, 40);
        let pinned = shelf.put(picture("b"), 1, false, false, 30);
        shelf.pin(pinned, true);
        let second = shelf.put(picture("c"), 2, false, false, 20);
        // (Pointing to files keeps nothing of its own: never taken off for room.)
        let files = shelf.put(What::Files { paths: vec!["C:\\a.txt".into()], kept: Vec::new() }, 0, false, false, 0);
        // 90 held: 10 more fit.
        assert_eq!(shelf.room_for(10, rules), Ok(Vec::new()));
        // 30 more: the oldest not pinned goes.
        let gone = shelf.room_for(30, rules).unwrap();
        assert_eq!(gone.iter().map(|item| item.id).collect::<Vec<_>>(), [first]);
        assert!(shelf.get(files).is_some() && shelf.get(second).is_some());
    }

    #[test]
    fn pinned_things_are_never_taken_off_for_room() {
        let rules = Rules { keep_days: 7, most_bytes: 100 };
        let mut shelf = Shelf::default();
        let pinned = shelf.put(picture("a"), 0, false, false, 80);
        shelf.pin(pinned, true);
        let loose = shelf.put(picture("b"), 1, false, false, 10);
        // More than the pinned leave: full, and nothing taken off.
        assert_eq!(shelf.room_for(30, rules), Err(Full));
        assert_eq!(shelf.items.len(), 2);
        // What fits beside the pinned: the loose one goes.
        assert_eq!(shelf.room_for(20, rules).unwrap().iter().map(|item| item.id).collect::<Vec<_>>(), [loose]);
    }

    #[test]
    fn numbers_are_never_given_again() {
        let mut shelf = Shelf::default();
        let a = shelf.put(text("a"), 0, false, false, 0);
        shelf.take(a);
        let b = shelf.put(text("b"), 0, false, false, 0);
        assert_ne!(a, b);
        // Kept as written and read back.
        let read: Shelf = serde_json::from_str(&serde_json::to_string(&shelf).unwrap()).unwrap();
        assert_eq!(read, shelf);
        let mut read = read;
        assert_ne!(read.put(text("c"), 0, false, false, 0), b);
    }
}
