//! Folders listed again, and every folder found new in them listed in turn, by several threads at once in the
//! background, what they find put in the index by one, as each part of a folder is listed, for it to be searched
//! meanwhile.
//!
//! The machine's whole share of listing is one `Pool`: however many drives are listed at once, and however many of
//! them lie on one disk, no more folders are begun to be listed at once than its lanes (half the processors), split
//! evenly between the rounds listing (see `Using::may_take`), so that a drive whose calls hang holds no more than its
//! own share of them, and no more of what was listed waits to be put in the indexes than its bytes. A folder's names
//! are handed on part by part as they are read (see `drive::Names`), each only once there is room for it (a thread
//! waits for room before it makes the part, as the one putting names in falls behind), so that a folder larger than
//! all the room is put in part by part, all that is held of it counted.
//!
//! A round may end at a time set (`until`): the folders not listed by then are left marked to be listed
//! (`flag::STALE`, as every folder found is marked till it is listed), for a round after (see `super::sweep`).

use super::drive::{self, Drive, Folder, Listing};
use super::index::{flag, Changes, Index, Item, Relisting};
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

/// What is taken of a pool now: how many lanes each round listing holds (by its number, in the order they began),
/// how many rounds were begun, bytes listed waiting to be put in, and the most lanes held at once and bytes waiting
/// yet.
#[derive(Default, Clone, Debug)]
struct Using {
    holding: Vec<(u64, usize)>,
    begun: u64,
    queued: usize,
    most_listing: usize,
    most_queued: usize,
}

impl Using {
    /// How many lanes of `lanes` the round at `at` (in the order they began) may hold: as many as each round, split
    /// evenly, those begun first given each one more of the lanes that do not split (none for a round begun once
    /// there are more rounds than lanes, till one ends).
    fn share(&self, at: usize, lanes: usize) -> usize {
        let rounds = self.holding.len();
        lanes / rounds + usize::from(at < lanes % rounds)
    }

    /// Whether the round numbered `round` may take another lane of `lanes`: while it holds fewer than its share. Its
    /// own lanes alone are counted (a round holding more than its share, taken while there were fewer rounds, takes
    /// no more till it holds fewer, each listing it holds ending as it may): so no more folders are begun at once than
    /// the lanes, and a round whose listings do not end (a drive that answers nothing, or one taken out as it was
    /// listed, its calls not ending though cancelled) holds the others to no fewer than their own shares.
    fn may_take(&self, round: u64, lanes: usize) -> bool {
        let at = self.at(round);
        self.holding[at].1 < self.share(at, lanes)
    }

    fn at(&self, round: u64) -> usize {
        self.holding.iter().position(|&(number, _)| number == round).unwrap()
    }

    /// How many lanes are held, all rounds' together.
    fn listing(&self) -> usize {
        self.holding.iter().map(|&(_, held)| held).sum()
    }
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

    /// A round's share of the lanes (see `Using::may_take`), the others' made smaller, till it is dropped.
    fn share(&self) -> Share<'_> {
        let mut using = self.using.lock().unwrap();
        using.begun += 1;
        let round = using.begun;
        using.holding.push((round, 0));
        Share { pool: self, round }
    }

    /// Room for `bytes` more of what was listed to wait (waited for while what waits would hold more than the pool's
    /// bytes, a part larger than all of them waiting for none to be waiting, and then waiting alone).
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
        self.using.lock().unwrap().clone()
    }
}

/// A round's share of a pool's lanes, taken (see `Pool::share`), given back as it is dropped (once the round's
/// listings have ended).
struct Share<'a> {
    pool: &'a Pool,
    round: u64,
}

impl Share<'_> {
    /// A lane taken to list a folder through (waited for while the round holds its share), given back as it is
    /// dropped.
    fn lane(&self) -> Lane<'_> {
        let pool = self.pool;
        let mut using = pool.freed.wait_while(pool.using.lock().unwrap(), |using| !using.may_take(self.round, pool.lanes)).unwrap();
        let at = using.at(self.round);
        using.holding[at].1 += 1;
        using.most_listing = using.most_listing.max(using.listing());
        Lane { share: self }
    }
}

impl Drop for Share<'_> {
    fn drop(&mut self) {
        let mut using = self.pool.using.lock().unwrap();
        let at = using.at(self.round);
        using.holding.remove(at);
        // (The others' shares made larger.)
        self.pool.freed.notify_all();
    }
}

/// A lane of a pool, taken (see `Share::lane`).
struct Lane<'a> {
    share: &'a Share<'a>,
}

impl Drop for Lane<'_> {
    fn drop(&mut self) {
        let pool = self.share.pool;
        let mut using = pool.using.lock().unwrap();
        let at = using.at(self.share.round);
        using.holding[at].1 -= 1;
        pool.freed.notify_all();
    }
}

/// What a thread listing a folder hands on: a part of its names, as they were read, or how its listing ended.
enum Handed {
    Names(Vec<Item>),
    Ended(Listing),
}

/// How much memory what is handed on holds as it waits, but its names' (see `drive::Names::held`).
const HANDED: usize = size_of::<Handing>();

/// What was handed on of a folder, as it waits to be put in: the folder's id, and how much it holds.
type Handing = (u64, Handed, usize);

/// What was handed on, as it waits to be put in, and how many threads list still.
struct Waiting {
    listed: Mutex<(VecDeque<Handing>, usize)>,
    came: Condvar,
}

impl Waiting {
    /// What was handed on next, waited for (none once no thread lists any more and none waits).
    fn next(&self) -> Option<Handing> {
        let mut listed = self.came.wait_while(self.listed.lock().unwrap(), |(listed, listing)| listed.is_empty() && *listing > 0).unwrap();
        listed.0.pop_front()
    }

    /// `handed` of the folder `id`, holding `held` (room made for it in the pool), handed on.
    fn hand_on(&self, id: u64, handed: Handed, held: usize) {
        self.listed.lock().unwrap().0.push_back((id, handed, held));
        self.came.notify_one();
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
    // The folders being put in part by part, by their ids, with how many names each was found to hold yet.
    let mut relisting: HashMap<u64, (Relisting, usize)> = HashMap::new();
    let share = pool.share();
    std::thread::scope(|scope| {
        for _ in 0..pool.lanes {
            let (jobs, more, waiting, share) = (&jobs, &more, &waiting, &share);
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
                    // (Its lane held till it is listed whole, as it waits for room: what waits is put in whatever lanes
                    // are held, which makes room.)
                    let _lane = share.lane();
                    let ended = drive.list(&folder, &mut buffer, |names| {
                        let held = HANDED + names.held();
                        pool.room(held);
                        waiting.hand_on(id, Handed::Names(names.items()), held);
                    });
                    pool.room(HANDED);
                    waiting.hand_on(id, Handed::Ended(ended), HANDED);
                }
            });
        }
        while pending > 0 {
            let Some((id, handed, held)) = waiting.next() else { break };
            let mut index = index.write().unwrap();
            match handed {
                Handed::Names(items) => {
                    if let Some(place) = index.folder(id) {
                        let (folder, names) = relisting.entry(id).or_insert_with(|| (index.relisting(place, &before.remove(&id).unwrap_or_default()), 0));
                        *names += items.len();
                        index.list_part(folder, &items, &mut changes);
                    }
                }
                Handed::Ended(listing) => {
                    pending -= 1;
                    listed.folders += 1;
                    let begun = relisting.remove(&id);
                    if let Some(place) = index.folder(id) {
                        let before = before.remove(&id).unwrap_or_default();
                        match listing {
                            Listing::Listed => {
                                let (folder, names) = begun.unwrap_or_else(|| (index.relisting(place, &before), 0));
                                if names >= LARGE {
                                    listed.large.push((id, names));
                                }
                                index.listed(folder, &mut changes);
                                index.set_flag(place, flag::UNREAD | flag::STALE, false);
                            }
                            Listing::Denied => {
                                listed.denied += 1;
                                index.denied(place, &before);
                                index.set_flag(place, flag::STALE, false);
                            }
                            // (What was put in of it before it failed stays, the rest as it was, till it is listed
                            // again.)
                            Listing::Failed(_) => {
                                listed.failed.push(id);
                                index.set_flag(place, flag::UNREAD | flag::STALE, true);
                            }
                            // (Gone since it was found: taken out as the folder it was in is listed again, which its
                            // going changed.)
                            Listing::Gone => index.set_flag(place, flag::STALE, false),
                        }
                    }
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
    drop(share);
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
        let share = pool.share();
        let held = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    let _lane = share.lane();
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

    /// Two drives listed at once share the machine's lanes and its room (one lane, room for one part of a folder's
    /// names): each listed whole, never more folders at once than the lanes, never more than one part waiting.
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
        let largest = HANDED + 8 * (size_of::<Item>() + 32);
        assert!(using.most_queued <= largest, "one part waiting at a time: {}", using.most_queued);
        drop(drives);
        for root in roots {
            let _ = std::fs::remove_dir_all(root);
        }
    }

    /// The lanes are split evenly between the rounds listing (those begun first given what does not split, none for
    /// a round begun once there are more rounds than lanes), each round taking up to its own share whatever the
    /// others hold: one holding all of them, taken alone, holds another to no fewer than its own share, and takes no
    /// more itself till it holds fewer than its own, where the others waited till it gave one back.
    #[test]
    fn the_lanes_are_split_evenly_between_the_rounds() {
        let using = |holding: &[usize]| Using { holding: holding.iter().enumerate().map(|(at, &held)| (at as u64 + 1, held)).collect(), ..Using::default() };
        assert!(using(&[3]).may_take(1, 4) && !using(&[4]).may_take(1, 4), "alone, all of them");
        let hung = using(&[4, 0]);
        assert!(hung.may_take(2, 4) && !hung.may_take(1, 4), "the other its share, the one holding all no more");
        assert!(using(&[4, 1]).may_take(2, 4) && !using(&[4, 2]).may_take(2, 4), "its own share, no more");
        assert!(using(&[1, 2]).may_take(1, 4) && !using(&[1, 2]).may_take(2, 4));
        let shares = |rounds: usize, lanes: usize| (0..rounds).map(|at| using(&vec![0; rounds]).share(at, lanes)).collect::<Vec<_>>();
        assert_eq!(shares(3, 5), [2, 2, 1]);
        assert_eq!(shares(3, 2), [1, 1, 0], "more rounds than lanes: the last waits for one to end");
    }

    /// A drive whose listings hang (a USB drive that answers nothing), all the lanes taken by them as it was listed
    /// alone, holds another drive listed meanwhile to no fewer than its share: that one is listed whole while they
    /// hang, where it waited for them to end.
    #[test]
    fn a_drive_whose_listings_hang_does_not_hold_another() {
        let pool = Pool::new(2, 1 << 20);
        let root = tree("hung", 2, 2, 2);
        let drive = Drive::open(&format!("{}\\", root.display())).unwrap();
        let index = RwLock::new(Index::new(drive.root_id, drive.by_id()));
        let hung = pool.share();
        let gate = Mutex::new(());
        let shut = gate.lock().unwrap();
        let (took, taken) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            for _ in 0..2 {
                let (hung, gate, took) = (&hung, &gate, took.clone());
                scope.spawn(move || {
                    let _lane = hung.lane();
                    took.send(()).unwrap();
                    drop(gate.lock().unwrap());
                });
            }
            for _ in 0..2 {
                taken.recv().unwrap();
            }
            let (done, listed) = std::sync::mpsc::channel();
            let (index, drive, pool) = (&index, &drive, &pool);
            scope.spawn(move || {
                round(index, drive, &[super::super::index::ROOT], pool, None);
                done.send(()).unwrap();
            });
            let in_time = listed.recv_timeout(Duration::from_secs(10)).is_ok();
            let hung_holds = pool.using().holding[0].1;
            drop(shut);
            assert!(in_time, "listed while the other's listings hang");
            assert_eq!(hung_holds, 2);
        });
        assert_eq!(indexed(&index.read().unwrap()), on_disk(&root));
        drop(hung);
        drop(drive);
        let _ = std::fs::remove_dir_all(root);
    }

    /// A folder holding more than all the room is put in part by part, never more of it made and waiting than the room
    /// (a part of it at a time here), where it was held whole as it waited: listed whole all the same.
    #[test]
    fn a_folder_larger_than_all_the_room_is_put_in_part_by_part() {
        let root = tree("parts", 0, 3000, 0);
        let drive = Drive::open(&format!("{}\\", root.display())).unwrap();
        let index = RwLock::new(Index::new(drive.root_id, drive.by_id()));
        let room = 48 << 10;
        let pool = Pool::new(2, room);
        round(&index, &drive, &[super::super::index::ROOT], &pool, None);
        assert_eq!(indexed(&index.read().unwrap()), on_disk(&root));
        let whole = 3000 * (size_of::<Item>() + 2 * "file 0000.txt".len());
        let most = pool.using().most_queued;
        assert!(most <= room && most < whole, "{most} waiting at most, of {whole}");
        drop(drive);
        let _ = std::fs::remove_dir_all(root);
    }
}
