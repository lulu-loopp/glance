//! Folders listed again, and every folder found new in them listed in turn, by several threads at once in the
//! background, what they find put in the index by one, as each folder is listed, for it to be searched meanwhile.
//!
//! The machine's whole share of listing is one `Pool`: however many drives are listed at once, and however many of
//! them lie on one disk, no more folders are listed at once than its lanes (half the processors), and no more of what
//! was listed waits to be put in the indexes than its bytes (a thread that listed a folder waits for room before it
//! hands it on, as the one putting names in falls behind).
//!
//! A round may end at a time set (`until`): the folders not listed by then are left marked to be listed
//! (`flag::STALE`, as every folder found is marked till it is listed), for a round after (see `super::sweep`).

use super::drive::{self, Drive, Folder, Listing};
use super::index::{flag, Changes, Index, Item};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Condvar, Mutex, OnceLock, RwLock};
use std::time::Instant;

/// What a round of listing did: how many folders it listed, how many the user may not, those that could not be
/// listed (by their ids), and the large ones listed (by their ids, with how many names each holds).
#[derive(Default, Debug, Clone)]
pub struct Listed {
    pub folders: usize,
    pub denied: usize,
    pub failed: Vec<u64>,
    pub large: Vec<(u64, usize)>,
}

/// How many names a folder holds to be counted large.
pub const LARGE: usize = 10_000;

/// How much of what was listed may wait to be put in the indexes, all drives' together (bytes).
const QUEUED_MOST: usize = 16 << 20;

/// The folders to list (by their ids, and their paths where the drive keeps no ids), and whether the round is over.
struct Jobs {
    ids: Vec<(u64, Option<PathBuf>)>,
    over: bool,
}

/// How a folder is listed: by its id where the drive keeps them, else by its path.
fn job(index: &Index, drive: &Drive, place: u32, id: u64) -> (u64, Option<PathBuf>) {
    let path = (!index.by_id()).then(|| index.path(place, &format!("\\\\?\\{}", drive.path))).flatten();
    (id, path)
}

/// How many folders the machine lists at once: half its processors (2 to 16).
pub fn lanes() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get() / 2).clamp(2, 16)
}

/// The machine's share of listing (see the module's note), all drives' rounds taking from it.
pub struct Pool {
    lanes: usize,
    queued_most: usize,
    using: Mutex<Using>,
    freed: Condvar,
}

/// What is taken of a pool now: folders being listed, bytes listed waiting to be put in, and the most of each yet.
#[derive(Default, Clone, Copy, Debug)]
struct Using {
    listing: usize,
    queued: usize,
    most_listing: usize,
    most_queued: usize,
}

/// The machine's pool.
pub fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| Pool::new(lanes(), QUEUED_MOST))
}

impl Pool {
    pub fn new(lanes: usize, queued_most: usize) -> Pool {
        Pool { lanes, queued_most, using: Mutex::new(Using::default()), freed: Condvar::new() }
    }

    /// A lane taken to list a folder through (waited for while all are taken), given back as it is dropped.
    fn lane(&self) -> Lane<'_> {
        let mut using = self.freed.wait_while(self.using.lock().unwrap(), |using| using.listing >= self.lanes).unwrap();
        using.listing += 1;
        using.most_listing = using.most_listing.max(using.listing);
        Lane { pool: self }
    }

    /// Room for `bytes` more of what was listed to wait (waited for while what waits would hold more than the pool's
    /// bytes, one larger than all of them waiting for none to be waiting).
    fn room(&self, bytes: usize) {
        let mut using = self.freed.wait_while(self.using.lock().unwrap(), |using| using.queued > 0 && using.queued + bytes > self.queued_most).unwrap();
        using.queued += bytes;
        using.most_queued = using.most_queued.max(using.queued);
    }

    /// `bytes` of what waited put in (and let go of).
    fn put_in(&self, bytes: usize) {
        self.using.lock().unwrap().queued -= bytes;
        self.freed.notify_all();
    }

    #[cfg(test)]
    fn using(&self) -> Using {
        *self.using.lock().unwrap()
    }
}

/// A lane of a pool, taken (see `Pool::lane`).
struct Lane<'a> {
    pool: &'a Pool,
}

impl Drop for Lane<'_> {
    fn drop(&mut self) {
        self.pool.using.lock().unwrap().listing -= 1;
        self.pool.freed.notify_all();
    }
}

/// How much memory a folder's listing holds, as it waits.
fn bytes(listing: &Listing) -> usize {
    size_of::<(u64, Listing)>()
        + match listing {
            Listing::Items(items) => items.capacity() * size_of::<Item>() + items.iter().map(|item| item.name.capacity() * 2).sum::<usize>(),
            _ => 0,
        }
}

/// The folders listed, as they wait to be put in (each with how much it holds), and how many threads list still.
struct Waiting {
    listed: Mutex<(VecDeque<(u64, Listing, usize)>, usize)>,
    came: Condvar,
}

impl Waiting {
    /// The next listed, waited for (none once no thread lists any more and none waits).
    fn next(&self) -> Option<(u64, Listing, usize)> {
        let mut listed = self.came.wait_while(self.listed.lock().unwrap(), |(listed, listing)| listed.is_empty() && *listing > 0).unwrap();
        listed.0.pop_front()
    }
}

/// A thread listing, counted among those that list still till it ends (whatever ends it).
struct Lister<'a> {
    waiting: &'a Waiting,
}

impl Drop for Lister<'_> {
    fn drop(&mut self) {
        self.waiting.listed.lock().unwrap().1 -= 1;
        self.waiting.came.notify_all();
    }
}

/// `folders` (live ones, by their places) listed again, and what was found new in them in turn, through `pool`,
/// till `until` where there is one (what is not listed by then left marked to be listed).
pub fn round(index: &RwLock<Index>, drive: &Drive, folders: &[u32], pool: &Pool, until: Option<Instant>) -> Listed {
    // What each folder held before, by its id (a folder moved meanwhile is found by it where it went).
    let (ids, mut before) = {
        let index = index.read().unwrap();
        let ids: Vec<(u32, u64)> = folders.iter().filter_map(|&place| index.id(place).map(|id| (place, id))).filter(|&(place, _)| index.entry(place).alive()).collect();
        let children = index.children(&ids.iter().map(|&(place, _)| place).collect::<HashSet<_>>());
        let before: HashMap<u64, Vec<u32>> = ids.iter().map(|&(place, id)| (id, children.get(&place).cloned().unwrap_or_default())).collect();
        (ids.into_iter().map(|(place, id)| job(&index, drive, place, id)).collect::<Vec<_>>(), before)
    };
    let mut listed = Listed::default();
    if ids.is_empty() {
        return listed;
    }
    let mut pending = ids.len();
    let jobs = Mutex::new(Jobs { ids, over: false });
    let more = Condvar::new();
    let waiting = Waiting { listed: Mutex::new((VecDeque::new(), pool.lanes)), came: Condvar::new() };
    let mut changes = Changes::default();
    std::thread::scope(|scope| {
        for _ in 0..pool.lanes {
            let (jobs, more, waiting) = (&jobs, &more, &waiting);
            scope.spawn(move || {
                let _lister = Lister { waiting };
                crate::settle::background("search: listing");
                let mut buffer = drive::buffer();
                loop {
                    let job = {
                        let mut jobs = jobs.lock().unwrap();
                        loop {
                            if let Some(job) = jobs.ids.pop() {
                                break Some(job);
                            }
                            if jobs.over {
                                break None;
                            }
                            jobs = more.wait(jobs).unwrap();
                        }
                    };
                    let Some((id, path)) = job else { break };
                    let folder = path.map_or(Folder::Id(id), Folder::Path);
                    let listing = {
                        let _lane = pool.lane();
                        drive.list(&folder, &mut buffer)
                    };
                    let held = bytes(&listing);
                    pool.room(held);
                    waiting.listed.lock().unwrap().0.push_back((id, listing, held));
                    waiting.came.notify_one();
                }
            });
        }
        while pending > 0 {
            let Some((id, listing, held)) = waiting.next() else { break };
            pending -= 1;
            listed.folders += 1;
            let mut index = index.write().unwrap();
            if let Some(place) = index.folder(id) {
                let before = before.remove(&id).unwrap_or_default();
                match listing {
                    Listing::Items(items) => {
                        if items.len() >= LARGE {
                            listed.large.push((id, items.len()));
                        }
                        index.list(place, &items, &before, &mut changes);
                        index.set_flag(place, flag::UNREAD | flag::STALE, false);
                    }
                    Listing::Denied => {
                        listed.denied += 1;
                        index.denied(place, &before);
                        index.set_flag(place, flag::STALE, false);
                    }
                    Listing::Failed(_) => {
                        listed.failed.push(id);
                        index.set_flag(place, flag::UNREAD | flag::STALE, true);
                    }
                    // (Gone since it was found: taken out as the folder it was in is listed again, which its going
                    // changed.)
                    Listing::Gone => index.set_flag(place, flag::STALE, false),
                }
            }
            pool.put_in(held);
            // The folders found listed in turn, while there is time, those not (and those not yet begun as it ends)
            // left marked to be listed.
            let open = until.is_none_or(|until| Instant::now() < until);
            if open && !changes.to_list.is_empty() {
                pending += changes.to_list.len();
                let found: Vec<_> = changes.to_list.drain(..).map(|(place, id)| job(&index, drive, place, id)).collect();
                jobs.lock().unwrap().ids.extend(found);
                more.notify_all();
            } else if !open {
                changes.to_list.clear();
                let mut jobs = jobs.lock().unwrap();
                pending -= jobs.ids.len();
                jobs.ids.clear();
            }
            drop(index);
        }
        jobs.lock().unwrap().over = true;
        more.notify_all();
    });
    index.write().unwrap().finish(changes);
    listed
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// A folder of this test's own, as a drive (its folders listed by their ids), holding `files` files and `folders`
    /// folders, each holding as many again, `depth` levels down.
    pub(crate) fn tree(name: &str, folders: usize, files: usize, depth: usize) -> PathBuf {
        let root = std::env::temp_dir().join(format!("glance-walk-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        fn make(at: &Path, folders: usize, files: usize, depth: usize) {
            std::fs::create_dir_all(at).unwrap();
            for file in 0..files {
                std::fs::write(at.join(format!("file {file}.txt")), "").unwrap();
            }
            if depth > 0 {
                for folder in 0..folders {
                    make(&at.join(format!("folder {folder}")), folders, files, depth - 1);
                }
            }
        }
        make(&root, folders, files, depth);
        root
    }

    /// Every path below `root`, from it, as Rust lists them.
    pub(crate) fn on_disk(root: &Path) -> Vec<String> {
        let mut paths = Vec::new();
        let mut folders = vec![root.to_path_buf()];
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(&folder).unwrap().flatten() {
                let path = entry.path();
                paths.push(path.strip_prefix(root).unwrap().display().to_string());
                if entry.file_type().unwrap().is_dir() {
                    folders.push(path);
                }
            }
        }
        paths.sort();
        paths
    }

    /// Every live path in `index`, from its root.
    pub(crate) fn indexed(index: &Index) -> Vec<String> {
        let mut paths: Vec<String> = (1..index.len()).filter(|&place| index.entry(place).alive()).filter_map(|place| index.path(place, "")).map(|path| path.display().to_string()).collect();
        paths.sort();
        paths
    }

    /// A pool's lanes are the machine's: threads asking for more than there are wait till one is given back.
    #[test]
    fn no_more_folders_are_listed_at_once_than_the_pool_has_lanes() {
        let pool = Pool::new(2, 1 << 20);
        let held = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    let _lane = pool.lane();
                    held.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(400));
                });
            }
            std::thread::sleep(Duration::from_millis(200));
            assert_eq!(held.load(Ordering::SeqCst), 2, "two lanes taken, two threads waiting");
        });
        assert_eq!((held.load(Ordering::SeqCst), pool.using().most_listing), (4, 2));
    }

    /// What was listed waits to be put in only as far as the pool's bytes: a thread with more waits till what waits is
    /// put in (one larger than all of them, till none waits).
    #[test]
    fn what_was_listed_waits_only_as_far_as_the_pools_bytes() {
        let pool = Pool::new(2, 1000);
        pool.room(600);
        let more = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                pool.room(600);
                more.store(true, Ordering::SeqCst);
            });
            std::thread::sleep(Duration::from_millis(300));
            assert!(!more.load(Ordering::SeqCst), "it waits for room");
            pool.put_in(600);
        });
        assert!(more.load(Ordering::SeqCst));
        assert_eq!(pool.using().most_queued, 600);
        pool.put_in(600);
        pool.room(5000);
        assert_eq!(pool.using().queued, 5000, "one larger than all, with none waiting");
    }

    /// Two drives listed at once share the machine's lanes and its room (one lane, room for one listing): each listed
    /// whole, never more folders at once than the lanes, never more than one listing waiting.
    #[test]
    fn two_drives_listed_at_once_share_the_machines_pool() {
        let pool = Pool::new(1, 1);
        let roots = [tree("pool-a", 3, 4, 3), tree("pool-b", 3, 4, 3)];
        let drives: Vec<Drive> = roots.iter().map(|root| Drive::open(&format!("{}\\", root.display())).unwrap()).collect();
        let indexes: Vec<RwLock<Index>> = drives.iter().map(|drive| RwLock::new(Index::new(drive.root_id, drive.by_id()))).collect();
        std::thread::scope(|scope| {
            for (drive, index) in drives.iter().zip(&indexes) {
                let pool = &pool;
                scope.spawn(move || round(index, drive, &[super::super::index::ROOT], pool, None));
            }
        });
        for (root, index) in roots.iter().zip(&indexes) {
            assert_eq!(indexed(&index.read().unwrap()), on_disk(root));
        }
        let using = pool.using();
        assert_eq!(using.most_listing, 1, "the lane shared");
        let largest = size_of::<(u64, Listing)>() + 8 * (size_of::<Item>() + 32);
        assert!(using.most_queued <= largest, "one listing waiting at a time: {}", using.most_queued);
        drop(drives);
        for root in roots {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}
