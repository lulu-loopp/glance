//! What the station holds: the things put on it, each as what it is (files where they are, or what it keeps a copy of
//! itself: text, a link, a picture), when it was put there, and whether it is pinned; and the rules it keeps them by.
//!
//! - What is not pinned goes after a while (`Rules::keep_days`, 7 by default); what is pinned stays.
//! - What the station keeps a copy of itself takes no more than `Rules::most_bytes` (2 GB by default): to make room,
//!   what it keeps of the oldest things not pinned goes first; pinned things never go, and what is put on it is never
//!   dropped unsaid: with no room left even then, it is not taken, and the person is told the station is full.
//!
//! What is pinned of what was copied may be tagged: tags are told apart by their own numbers, never given to another, and
//! what is pinned no longer has none.
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
    /// When it was last put on the clipboard from here, or copied again (ms since 1970; 0: not since it came).
    #[serde(default, skip_serializing_if = "never")]
    pub used_ms: u64,
    /// Its tags, by their numbers (see `Tag`), in the order they were put on it (pinned things copied have them alone).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<u32>,
}

/// Whether a time is none (see `Item::used_ms`).
fn never(ms: &u64) -> bool {
    *ms == 0
}

impl Item {
    /// When it was last used: put on the clipboard, or come (ms since 1970).
    pub fn last_ms(&self) -> u64 {
        self.used_ms.max(self.added_ms)
    }
}

/// A tag what is pinned is sorted by: its own number, and its name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Tag {
    pub id: u32,
    pub name: String,
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

/// A file sent from a device nearby: its file's name in the station's own folder, and the name it was sent by (its
/// place in the folder it was sent in, where it was sent in one: "folder/inner/name").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Arrived {
    pub file: String,
    pub name: String,
}

impl Arrived {
    /// The folder it was sent in, where it was sent in one: the first part of the name it was sent by.
    pub fn folder(&self) -> Option<&str> {
        self.name.split_once('/').map(|(top, _)| top)
    }
}

/// One of the files sent from devices nearby as they are given out (see `entries`): a file sent by itself, or a folder
/// sent, by its name, with all of its files.
#[derive(Debug, PartialEq)]
pub enum Entry<'a> {
    File(&'a Arrived),
    Folder { name: &'a str, files: Vec<&'a Arrived> },
}

/// `files` as they are given out: each sent by itself as it is, and those sent in a folder as that folder, all of a
/// folder's together where the first of them came.
///
/// ```text
/// a.jpg, 资料/x.pdf, b.jpg, 资料/图/y.png  ->  a.jpg, 资料 (x.pdf, 图/y.png), b.jpg
/// ```
pub fn entries(files: &[Arrived]) -> Vec<Entry<'_>> {
    let mut entries: Vec<Entry> = Vec::new();
    for arrived in files {
        let Some(top) = arrived.folder() else {
            entries.push(Entry::File(arrived));
            continue;
        };
        let held = entries.iter_mut().find_map(|entry| match entry {
            Entry::Folder { name, files } if *name == top => Some(files),
            _ => None,
        });
        match held {
            Some(files) => files.push(arrived),
            None => entries.push(Entry::Folder { name: top, files: vec![arrived] }),
        }
    }
    entries
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

/// Which of what was copied lately a page shows: all; what is pinned (all of it, or that of one tag, by its number); words (text, and links: a link copied is words too); pictures
/// (what was copied as a picture, a screenshot among them, and picture files: files all of them pictures); files
/// (files and folders copied where they are, picture files too, and files that came with no path).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Filter {
    #[default]
    All,
    Pinned(Option<u32>),
    Words,
    Pictures,
    Files,
}

/// How many of what is pinned are listed first where all that was copied is (see `copies`).
pub const PINNED_FIRST: usize = 3;

impl Filter {
    pub const ALL: [Filter; 5] = [Filter::All, Filter::Pinned(None), Filter::Words, Filter::Pictures, Filter::Files];

    /// The choice among `ALL` it is of (what is pinned, whatever tag of it).
    pub fn choice(self) -> Filter {
        match self {
            Filter::Pinned(_) => Filter::Pinned(None),
            filter => filter,
        }
    }

    /// The same, as `tags` are now: what is pinned of a tag that is gone, all that is pinned.
    pub fn among(self, tags: &[Tag]) -> Filter {
        match self {
            Filter::Pinned(Some(id)) if !tags.iter().any(|tag| tag.id == id) => Filter::Pinned(None),
            filter => filter,
        }
    }

    /// It as a number, each its own.
    pub fn number(self) -> u32 {
        match self {
            Filter::All => 0,
            Filter::Words => 1,
            Filter::Pictures => 2,
            Filter::Files => 3,
            Filter::Pinned(None) => 4,
            Filter::Pinned(Some(tag)) => 5 + tag,
        }
    }

    /// Whether `item` is one it shows.
    pub fn takes(self, item: &Item) -> bool {
        match self {
            Filter::Pinned(tag) => item.pinned && tag.is_none_or(|tag| item.tags.contains(&tag)),
            filter => filter.takes_what(&item.what),
        }
    }

    /// Whether `what` is of its kind (what is pinned: of any).
    fn takes_what(self, what: &What) -> bool {
        let picture = |path: &std::path::Path| path.extension().is_some_and(|extension| super::recent::is_picture(&extension.to_string_lossy().to_lowercase()));
        match self {
            Filter::All | Filter::Pinned(_) => true,
            Filter::Words => matches!(what, What::Text { .. } | What::Link { .. } | What::Saved { words: true, .. }),
            Filter::Pictures => match what {
                What::Picture { .. } => true,
                What::Files { paths, .. } => !paths.is_empty() && paths.iter().all(|path| picture(path)),
                What::Saved { name, words, .. } => !words && picture(std::path::Path::new(name)),
                What::Came { files } => !files.is_empty() && entries(files).iter().all(|entry| matches!(entry, Entry::File(arrived) if picture(std::path::Path::new(&arrived.name)))),
                What::Text { .. } | What::Link { .. } => false,
            },
            Filter::Files => matches!(what, What::Files { .. } | What::Saved { words: false, .. } | What::Came { .. }),
        }
    }
}

/// What a page of what was copied lately lists, in its order (see `copies`): the pinned things listed first, under a
/// heading of their own, how many are pinned in all (none: no such heading), and the rest.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Copies<'a> {
    pub first: Vec<&'a Item>,
    pub pinned: usize,
    pub rest: Vec<&'a Item>,
}

/// What a page of what was copied lately lists of `items` (the station's) under `filter`: with all of it, what is
/// pinned first (the `PINNED_FIRST` last used; `folded`: none of them), what is not after; what is pinned by when it
/// was last used; of a kind, by when it came, what is pinned among the rest.
pub fn copies(items: &[Item], filter: Filter, folded: bool) -> Copies<'_> {
    let newest = |filter: Filter| items.iter().rev().filter(move |item| item.copied && filter.takes(item));
    let pinned = |tag: Option<u32>| {
        let mut pinned: Vec<&Item> = newest(Filter::Pinned(tag)).collect();
        pinned.sort_by_key(|item| std::cmp::Reverse(item.last_ms()));
        pinned
    };
    match filter {
        Filter::All => {
            let pinned = pinned(None);
            Copies { first: pinned.iter().copied().take(if folded { 0 } else { PINNED_FIRST }).collect(), pinned: pinned.len(), rest: newest(Filter::All).filter(|item| !item.pinned).collect() }
        }
        Filter::Pinned(tag) => Copies { rest: pinned(tag), ..Copies::default() },
        filter => Copies { rest: newest(filter).collect(), ..Copies::default() },
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
    /// The tags there are, in their order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
    /// The number the next tag takes.
    #[serde(default)]
    next_tag: u32,
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
        self.items.push(Item { id, what, added_ms: now_ms, pinned: false, copied, stays_here, own_bytes, from: None, used_ms: 0, tags: Vec::new() });
        id
    }

    /// Files sent from `from`, kept in its own folder (`own_bytes` of them, room made for them first), put on the shelf
    /// at `now_ms`: with what came from that device last, where that came within `TOGETHER_MS` before and is not
    /// pinned, else on their own. The number of what holds them.
    pub fn arrived(&mut self, files: Vec<Arrived>, from: Device, now_ms: u64, own_bytes: u64) -> u64 {
        self.came(What::Came { files }, from, now_ms, own_bytes)
    }

    /// Files sent from `from` that went to the folder chosen for them (see `Keep::arrived_into`), put on the shelf at
    /// `now_ms` where they are (each file, and each folder sent: `paths`), none kept of its own: joined as `arrived`
    /// joins them. The number of what holds them.
    pub fn arrived_at(&mut self, paths: Vec<PathBuf>, from: Device, now_ms: u64) -> u64 {
        self.came(What::Files { paths, kept: Vec::new() }, from, now_ms, 0)
    }

    /// What came from `from` (`what`: files kept, or files where they went) put on the shelf at `now_ms`: with what came
    /// from that device last as the same, where that came within `TOGETHER_MS` before and is not pinned, else on its
    /// own.
    fn came(&mut self, what: What, from: Device, now_ms: u64, own_bytes: u64) -> u64 {
        let last = self.items.iter().rposition(|item| {
            std::mem::discriminant(&item.what) == std::mem::discriminant(&what) && !item.pinned && item.from.as_ref().is_some_and(|device| device.fingerprint == from.fingerprint) && now_ms.saturating_sub(item.added_ms) <= TOGETHER_MS
        });
        match last {
            // (Joined, it is as new as what joined it: the newest, last.)
            Some(at) => {
                let mut item = self.items.remove(at);
                match (&mut item.what, what) {
                    (What::Came { files: held }, What::Came { files }) => held.extend(files),
                    (What::Files { paths: held, .. }, What::Files { paths, .. }) => {
                        for path in paths {
                            if !held.contains(&path) {
                                held.push(path);
                            }
                        }
                    }
                    _ => unreachable!("joined only with what is of its kind"),
                }
                item.added_ms = now_ms;
                item.own_bytes += own_bytes;
                item.from = Some(from);
                let id = item.id;
                self.items.push(item);
                id
            }
            None => {
                let id = self.put(what, now_ms, false, false, own_bytes);
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

    /// The thing `id` pinned, or not (and of no tag any more).
    pub fn pin(&mut self, id: u64, pinned: bool) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.pinned = pinned;
            if !pinned {
                item.tags.clear();
            }
        }
    }

    /// The thing `id` used at `now_ms`: put on the clipboard from here.
    pub fn used(&mut self, id: u64, now_ms: u64) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.used_ms = now_ms;
        }
    }

    /// The thing `id` copied again at `now_ms`: the newest now, staying here as its copier said this time (see
    /// `Item::stays_here`).
    pub fn copied_again(&mut self, id: u64, stays_here: bool, now_ms: u64) {
        if let Some(at) = self.items.iter().position(|item| item.id == id) {
            let mut item = self.items.remove(at);
            (item.added_ms, item.used_ms, item.stays_here) = (now_ms, now_ms, stays_here);
            self.items.push(item);
        }
    }

    /// The things `ids` of the tag `tag` (pinned with it), or not of it any more.
    pub fn tag(&mut self, ids: &[u64], tag: u32, on: bool) {
        if !self.tags.iter().any(|kept| kept.id == tag) {
            return;
        }
        for item in self.items.iter_mut().filter(|item| item.copied && ids.contains(&item.id)) {
            item.tags.retain(|of| *of != tag);
            if on {
                item.pinned = true;
                item.tags.push(tag);
            }
        }
    }

    /// A tag named `name` (the one there is of that name, where there is one): its number. None for a name of nothing
    /// but spaces.
    pub fn new_tag(&mut self, name: &str) -> Option<u32> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        if let Some(tag) = self.tags.iter().find(|tag| tag.name == name) {
            return Some(tag.id);
        }
        let id = self.next_tag;
        self.next_tag += 1;
        self.tags.push(Tag { id, name: name.to_string() });
        Some(id)
    }

    /// The tag `id` named `name` from now on: left as it is for a name of nothing but spaces, or one another has.
    pub fn rename_tag(&mut self, id: u32, name: &str) {
        let name = name.trim();
        if name.is_empty() || self.tags.iter().any(|tag| tag.name == name) {
            return;
        }
        if let Some(tag) = self.tags.iter_mut().find(|tag| tag.id == id) {
            tag.name = name.to_string();
        }
    }

    /// The tag `id` gone, what was of it pinned still.
    pub fn remove_tag(&mut self, id: u32) {
        self.tags.retain(|tag| tag.id != id);
        for item in &mut self.items {
            item.tags.retain(|of| *of != id);
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

    /// The most it can make room for within `rules` (see `room_for`): all but what pinned things keep of their own.
    pub fn room(&self, rules: Rules) -> u64 {
        rules.most_bytes.saturating_sub(self.pinned_bytes())
    }

    fn pinned_bytes(&self) -> u64 {
        self.items.iter().filter(|item| item.pinned).map(|item| item.own_bytes).sum()
    }

    /// Makes room for `bytes` more of its own within `rules`: the oldest things not pinned that keep something of
    /// their own taken off till there is (what went); else, with no room even with all of those gone, nothing taken
    /// off, and `Full`.
    pub fn room_for(&mut self, bytes: u64, rules: Rules) -> Result<Vec<Item>, Full> {
        if bytes > self.room(rules) {
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
        let taken = |filter: Filter| all.iter().enumerate().filter(|(_, what)| filter.takes_what(what)).map(|(at, _)| at).collect::<Vec<_>>();
        assert_eq!(taken(Filter::All), [0, 1, 2, 3, 4, 5, 6, 7, 8]);
        // (Text too long for the shelf, saved as a file, is words.)
        assert_eq!(taken(Filter::Words), [0, 1, 8]);
        // Picture files are pictures and files both (files with one not a picture, a folder, files alone).
        assert_eq!(taken(Filter::Pictures), [2, 3, 7]);
        assert_eq!(taken(Filter::Files), [3, 4, 5, 6, 7]);
    }

    /// With all that was copied, what is pinned is listed first, the three last used, and only there. Folded, none of
    /// them. What is pinned is listed by when it was last used, that of a tag alone under it. Of a kind, all of it by
    /// when it came.
    #[test]
    fn what_is_pinned_is_listed_first() {
        let mut shelf = Shelf::default();
        let ids: Vec<u64> = (0..7).map(|at| shelf.put(text(&at.to_string()), at, true, false, 0)).collect();
        shelf.put(text("put there"), 8, false, false, 0);
        for at in [0, 1, 2, 4] {
            shelf.pin(ids[at], true);
        }
        let of = |items: &[&Item]| items.iter().map(|item| item.id).collect::<Vec<_>>();
        let listed = |shelf: &Shelf, filter: Filter, folded: bool| {
            let copies = copies(&shelf.items, filter, folded);
            (of(&copies.first), copies.pinned, of(&copies.rest))
        };
        assert_eq!(listed(&shelf, Filter::All, false), (vec![4, 2, 1], 4, vec![6, 5, 3]));
        assert_eq!(listed(&shelf, Filter::All, true), (vec![], 4, vec![6, 5, 3]));
        // The oldest of them used: first of them.
        shelf.used(0, 20);
        assert_eq!(listed(&shelf, Filter::All, false), (vec![0, 4, 2], 4, vec![6, 5, 3]));
        assert_eq!(listed(&shelf, Filter::Pinned(None), false), (vec![], 0, vec![0, 4, 2, 1]));
        assert_eq!(listed(&shelf, Filter::Words, false), (vec![], 0, vec![6, 5, 4, 3, 2, 1, 0]));
        let work = shelf.new_tag("work").unwrap();
        shelf.tag(&[1, 3], work, true);
        assert_eq!(listed(&shelf, Filter::Pinned(Some(work)), false), (vec![], 0, vec![3, 1]));
        // With nothing pinned, no heading of it.
        (0..7).for_each(|at| shelf.pin(ids[at], false));
        assert_eq!(listed(&shelf, Filter::All, false), (vec![], 0, vec![6, 5, 4, 3, 2, 1, 0]));
    }

    /// Tagged, a thing copied is pinned with it (one put on the station is not tagged). Pinned no longer, it is of no
    /// tag. A tag gone leaves what was of it pinned. A name there is already is that tag's, and no other may take it.
    #[test]
    fn a_tag_pins_and_goes_without_what_it_is_on() {
        let mut shelf = Shelf::default();
        let copied = shelf.put(text("a"), 0, true, false, 0);
        let other = shelf.put(text("b"), 1, true, false, 0);
        let put = shelf.put(text("c"), 2, false, false, 0);
        let one = shelf.new_tag(" one ").unwrap();
        assert_eq!((shelf.new_tag("one"), shelf.new_tag("  ")), (Some(one), None));
        let two = shelf.new_tag("two").unwrap();
        shelf.tag(&[copied, other, put], one, true);
        shelf.tag(&[copied], two, true);
        let of = |shelf: &Shelf, id: u64| shelf.get(id).map(|item| (item.pinned, item.tags.clone())).unwrap();
        assert_eq!((of(&shelf, copied), of(&shelf, other), of(&shelf, put)), ((true, vec![one, two]), (true, vec![one]), (false, vec![])));
        shelf.pin(other, false);
        assert_eq!(of(&shelf, other), (false, vec![]));
        shelf.rename_tag(two, "one");
        shelf.rename_tag(one, " first ");
        assert_eq!(shelf.tags.iter().map(|tag| tag.name.as_str()).collect::<Vec<_>>(), ["first", "two"]);
        shelf.remove_tag(one);
        assert_eq!(of(&shelf, copied), (true, vec![two]));
        assert_eq!(Filter::Pinned(Some(one)).among(&shelf.tags), Filter::Pinned(None));
        assert_eq!(Filter::Pinned(Some(two)).among(&shelf.tags), Filter::Pinned(Some(two)));
        // (Its number never another's.)
        assert_ne!(shelf.new_tag("one"), Some(one));
        let read: Shelf = serde_json::from_str(&serde_json::to_string(&shelf).unwrap()).unwrap();
        assert_eq!(read, shelf);
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
        assert_eq!(shelf.room(rules), 100);
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
