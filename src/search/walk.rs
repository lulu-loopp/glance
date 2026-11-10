//! Folders listed again, and every folder found new in them listed in turn, by several threads at once in the
//! background, what they find put in the index by one, as each part of a folder is listed, for it to be searched
//! meanwhile.
//!
//! The machine's whole share of listing is one `Pool`: however many drives are listed at once, and however many of
//! them lie on one disk, no more folders are begun to be listed at once than its lanes (half the processors), or one
//! for each round listing where there are more of those, split evenly between the rounds (see `Using::share`), so
//! that a drive whose calls hang holds no more than its own share of them and never all of another's, and no more of
//! what was listed waits to be put in the indexes than its bytes. A folder's names are handed on part by part as they
//! are read (see `drive::Names`), each only once there is room for it (a thread waits for room before it makes the
//! part, as the one putting names in falls behind), so that a folder larger than all the room is put in part by part,
//! all that is held of it counted.
//!
//! A call on a drive cannot be taken back: cancelled, a drive that answers nothing may hold it for good, and the
//! thread making it with it. What waits for it can be: a round whose drive is let go of (its calls cancelled, see
//! `Drive::tell_let_go`) ends at once, waiting for none of its listings, and those that have not ended hold no lane
//! of the pool from then on (their drive read no more), their threads left to end as their calls do.
//!
//! A round may end at a time set (`until`): the folders not listed by then are left marked to be listed
//! (`flag::STALE`, as every folder found is marked till it is listed), for a round after (see `super::sweep`).

use super::drive::{self, Drive, Folder, Listing, Names};
use super::index::{flag, Changes, Index, Item, Relisting};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, OnceLock, RwLock, Weak};
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

/// What a round lists folders through: a drive (see `Drive`, whose own calls these are).
pub trait Source: Send + Sync + 'static {
    /// Its root's path ("C:\"), where folders listed by their paths are.
    fn root(&self) -> &str;
    fn list(&self, folder: &Folder, buffer: &mut [u64], names: &mut dyn FnMut(Names<'_>)) -> Listing;
    fn left(&self) -> bool;
    fn tell_let_go(&self, tell: Weak<dyn Fn() + Send + Sync>);
}

impl Source for Drive {
    fn root(&self) -> &str {
        &self.path
    }

    fn list(&self, folder: &Folder, buffer: &mut [u64], names: &mut dyn FnMut(Names<'_>)) -> Listing {
        Drive::list(self, folder, buffer, names)
    }

    fn left(&self) -> bool {
        Drive::left(self)
    }

    fn tell_let_go(&self, tell: Weak<dyn Fn() + Send + Sync>) {
        Drive::tell_let_go(self, tell)
    }
}

/// The folders to list (by their ids, and their paths where the drive keeps no ids), and whether the round is over.
struct Jobs {
    ids: Vec<(u64, Option<PathBuf>)>,
    over: bool,
}

/// How a folder is listed: by its id where the drive keeps them, else by its path (under `root`, the drive's).
fn job(index: &Index, root: &str, place: u32, id: u64) -> (u64, Option<PathBuf>) {
    let path = (!index.by_id()).then(|| index.path(place, &format!("\\\\?\\{root}"))).flatten();
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
    /// evenly, those begun first given each one more of the lanes that do not split, and one each where there are
    /// more rounds than lanes (a round never waits for another to end, which one whose calls hang never may).
    fn share(&self, at: usize, lanes: usize) -> usize {
        let rounds = self.holding.len();
        (lanes / rounds + usize::from(at < lanes % rounds)).max(1)
    }

    /// Whether the round at `at` may take another lane of `lanes`: while it holds fewer than its share. Its own lanes
    /// alone are counted (a round holding more than its share, taken while there were fewer rounds, takes no more till
    /// it holds fewer, each listing it holds ending as it may): so no more folders are begun at once than the lanes,
    /// or the rounds, and a round whose listings do not end (a drive that answers nothing, or one taken out as it was
    /// listed, its calls not ending though cancelled) holds the others to no fewer than their own shares.
    fn may_take(&self, at: usize, lanes: usize) -> bool {
        self.holding[at].1 < self.share(at, lanes)
    }

    /// Where the round numbered `round` is among those listing (none once it has ended).
    fn at(&self, round: u64) -> Option<usize> {
        self.holding.iter().position(|&(number, _)| number == round)
    }

    /// How many lanes are held, all rounds' together.
    fn listing(&self) -> usize {
        self.holding.iter().map(|&(_, held)| held).sum()
    }
}

/// The machine's pool.
pub fn pool() -> &'static Arc<Pool> {
    static POOL: OnceLock<Arc<Pool>> = OnceLock::new();
    POOL.get_or_init(|| Arc::new(Pool::new(lanes(), QUEUED_MOST)))
}

impl Pool {
    pub fn new(lanes: usize, queued_most: usize) -> Pool {
        Pool { lanes, queued_most, using: Mutex::new(Using::default()), freed: Condvar::new() }
    }

    /// A round's share of the lanes (see `Using::share`), the others' made smaller, till it is dropped.
    fn share(&self) -> Share<'_> {
        let mut using = self.using.lock().unwrap();
        using.begun += 1;
        let round = using.begun;
        using.holding.push((round, 0));
        Share { pool: self, round }
    }

    /// A lane taken for the round numbered `round` to list a folder through (waited for while the round holds its
    /// share), given back as it is dropped: none once the round has ended.
    fn lane(self: &Arc<Pool>, round: u64) -> Option<Lane> {
        let mut using = self.freed.wait_while(self.using.lock().unwrap(), |using| using.at(round).is_some_and(|at| !using.may_take(at, self.lanes))).unwrap();
        let at = using.at(round)?;
        using.holding[at].1 += 1;
        using.most_listing = using.most_listing.max(using.listing());
        Some(Lane { pool: self.clone(), round })
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

/// A round's share of a pool's lanes, taken (see `Pool::share`), given back as it is dropped (as the round ends): the
/// lanes its listings hold then, those that have not ended, held no more.
struct Share<'a> {
    pool: &'a Pool,
    round: u64,
}

impl Drop for Share<'_> {
    fn drop(&mut self) {
        let mut using = self.pool.using.lock().unwrap();
        let at = using.at(self.round).unwrap();
        using.holding.remove(at);
        // (The others' shares made larger, and its threads waiting for a lane told it has ended.)
        self.pool.freed.notify_all();
    }
}

/// A lane of a pool, taken for a round (see `Pool::lane`).
struct Lane {
    pool: Arc<Pool>,
    round: u64,
}

impl Drop for Lane {
    fn drop(&mut self) {
        let mut using = self.pool.using.lock().unwrap();
        // (Held no more once its round has ended.)
        if let Some(at) = using.at(self.round) {
            using.holding[at].1 -= 1;
            self.pool.freed.notify_all();
        }
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

/// What was handed on, as it waits to be put in, how many threads list still, and whether the round has ended.
struct Queue {
    handed: VecDeque<Handing>,
    listing: usize,
    over: bool,
}

/// What a round's threads hand on (see `Queue`).
struct Waiting {
    queue: Mutex<Queue>,
    came: Condvar,
}

impl Waiting {
    /// What was handed on next, waited for (none once no thread lists any more and none waits, or once `left`: the
    /// drive let go of, those of its listings that do not end not waited for).
    fn next(&self, left: impl Fn() -> bool) -> Option<Handing> {
        let mut queue = self.came.wait_while(self.queue.lock().unwrap(), |queue| queue.handed.is_empty() && queue.listing > 0 && !left()).unwrap();
        queue.handed.pop_front()
    }

    /// `handed` of the folder `id`, holding `held` (room made for it in `pool`), handed on: let go of at once, its room
    /// given back, once the round has ended.
    fn hand_on(&self, pool: &Pool, id: u64, handed: Handed, held: usize) {
        let mut queue = self.queue.lock().unwrap();
        if queue.over {
            drop(queue);
            pool.put_in(held);
            return;
        }
        queue.handed.push_back((id, handed, held));
        self.came.notify_one();
    }

    /// The one waiting for what is handed on woken (its drive let go of).
    fn wake(&self) {
        let _queue = self.queue.lock().unwrap();
        self.came.notify_all();
    }

    /// The round ended: what waits let go of, its room given back, as is what is handed on after.
    fn end(&self, pool: &Pool) {
        let mut queue = self.queue.lock().unwrap();
        queue.over = true;
        let held: usize = queue.handed.drain(..).map(|(_, _, held)| held).sum();
        drop(queue);
        pool.put_in(held);
    }
}

/// A thread listing, counted among those that list still till it ends (whatever ends it).
struct Lister<'a> {
    waiting: &'a Waiting,
}

impl Drop for Lister<'_> {
    fn drop(&mut self) {
        self.waiting.queue.lock().unwrap().listing -= 1;
        self.waiting.came.notify_all();
    }
}

/// What a round's threads share with it: the folders to list (and their being told of more), and what they hand on.
struct Shared {
    jobs: Mutex<Jobs>,
    more: Condvar,
    waiting: Waiting,
}

impl Shared {
    /// The next folder to list, waited for (none once the round is over).
    fn job(&self) -> Option<(u64, Option<PathBuf>)> {
        let mut jobs = self.jobs.lock().unwrap();
        loop {
            if let Some(job) = jobs.ids.pop() {
                return Some(job);
            }
            if jobs.over {
                return None;
            }
            jobs = self.more.wait(jobs).unwrap();
        }
    }
}

/// `folders` (live ones, by their places) listed again through `source`, and what was found new in them in turn,
/// through `pool`, till `until` where there is one (what is not listed by then left marked to be listed), or till the
/// drive is let go of (see the module's note).
pub fn round<S: Source>(index: &RwLock<Index>, source: &Arc<S>, folders: &[u32], pool: &Arc<Pool>, until: Option<Instant>) -> Listed {
    // What each folder held before, by its id (a folder moved meanwhile is found by it where it went).
    let (ids, mut before) = {
        let index = index.read().unwrap();
        let ids: Vec<(u32, u64)> = folders.iter().filter_map(|&place| index.id(place).map(|id| (place, id))).filter(|&(place, _)| index.entry(place).alive()).collect();
        let children = index.children(&ids.iter().map(|&(place, _)| place).collect::<HashSet<_>>());
        let before: HashMap<u64, Vec<u32>> = ids.iter().map(|&(place, id)| (id, children.get(&place).cloned().unwrap_or_default())).collect();
        (ids.into_iter().map(|(place, id)| job(&index, source.root(), place, id)).collect::<Vec<_>>(), before)
    };
    let mut listed = Listed::default();
    if ids.is_empty() {
        return listed;
    }
    let mut pending = ids.len();
    let shared = Arc::new(Shared {
        jobs: Mutex::new(Jobs { ids, over: false }),
        more: Condvar::new(),
        waiting: Waiting { queue: Mutex::new(Queue { handed: VecDeque::new(), listing: pool.lanes, over: false }), came: Condvar::new() },
    });
    // (Woken as the drive is let go of, while the round lasts.)
    let woken: Arc<dyn Fn() + Send + Sync> = {
        let shared = shared.clone();
        Arc::new(move || shared.waiting.wake())
    };
    source.tell_let_go(Arc::downgrade(&woken));
    let mut changes = Changes::default();
    // The folders being put in part by part, by their ids, with how many names each was found to hold yet.
    let mut relisting: HashMap<u64, (Relisting, usize)> = HashMap::new();
    let share = pool.share();
    // (Threads of their own, not the round's: one whose call does not end is not waited for as the round ends.)
    for _ in 0..pool.lanes {
        let (shared, source, pool, round) = (shared.clone(), source.clone(), pool.clone(), share.round);
        std::thread::spawn(move || {
            let _lister = Lister { waiting: &shared.waiting };
            crate::settle::background("search: listing");
            let mut buffer = drive::buffer();
            while let Some((id, path)) = shared.job() {
                let folder = path.map_or(Folder::Id(id), Folder::Path);
                // (Its lane held till it is listed whole, as it waits for room: what waits is put in whatever lanes
                // are held, which makes room.)
                let Some(_lane) = pool.lane(round) else { break };
                let ended = source.list(&folder, &mut buffer, &mut |names| {
                    let held = HANDED + names.held();
                    pool.room(held);
                    shared.waiting.hand_on(&pool, id, Handed::Names(names.items()), held);
                });
                pool.room(HANDED);
                shared.waiting.hand_on(&pool, id, Handed::Ended(ended), HANDED);
            }
        });
    }
    while pending > 0 {
        let Some((id, handed, held)) = shared.waiting.next(|| source.left()) else { break };
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
            let found: Vec<_> = changes.to_list.drain(..).map(|(place, id)| job(&index, source.root(), place, id)).collect();
            shared.jobs.lock().unwrap().ids.extend(found);
            shared.more.notify_all();
        } else if !open {
            changes.to_list.clear();
            let mut jobs = shared.jobs.lock().unwrap();
            pending -= jobs.ids.len();
            jobs.ids.clear();
        }
        drop(index);
    }
    // (Listed whole, or its drive let go of: none of its folders begun after, what is handed on after let go of. The
    // drive's index, let go of, is not kept: see `super::Keeping::keep`.)
    shared.waiting.end(pool);
    {
        let mut jobs = shared.jobs.lock().unwrap();
        jobs.ids.clear();
        jobs.over = true;
    }
    shared.more.notify_all();
    drop(share);
    index.write().unwrap().finish(changes);
    listed
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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

    /// A drive whose listings hang (one that answers nothing): each call waits till it is let through, then fails.
    /// Let go of (as Windows asks to take a drive out), those told are told, and its calls hang on all the same, as
    /// cancelled ones may.
    struct Hanging {
        left: AtomicBool,
        told: Mutex<Vec<Weak<dyn Fn() + Send + Sync>>>,
        /// How many calls were begun on it, and whether they are let through.
        calls: Mutex<(usize, bool)>,
        changed: Condvar,
    }

    impl Hanging {
        fn new() -> Arc<Hanging> {
            Arc::new(Hanging { left: AtomicBool::new(false), told: Mutex::new(Vec::new()), calls: Mutex::new((0, false)), changed: Condvar::new() })
        }

        /// Waited for till `calls` calls hang on it (10 seconds at most).
        fn hanging(&self, calls: usize) {
            let (begun, timeout) = self.changed.wait_timeout_while(self.calls.lock().unwrap(), Duration::from_secs(10), |(begun, _)| *begun < calls).unwrap();
            assert!(!timeout.timed_out(), "{} of {calls} calls begun", begun.0);
        }

        fn let_through(&self) {
            self.calls.lock().unwrap().1 = true;
            self.changed.notify_all();
        }

        fn let_go(&self) {
            self.left.store(true, Ordering::SeqCst);
            for told in std::mem::take(&mut *self.told.lock().unwrap()) {
                if let Some(tell) = told.upgrade() {
                    tell();
                }
            }
        }

        /// Its index, with `folders` folders in its root to list, and their places.
        fn index(folders: usize) -> (RwLock<Index>, Vec<u32>) {
            let mut index = Index::new(1, true);
            let items: Vec<Item> = (0..folders).map(|at| Item { name: format!("folder {at}").encode_utf16().collect(), id: 100 + at as u64, attributes: 0x10, tag: 0 }).collect();
            let mut changes = Changes::default();
            index.list(super::super::index::ROOT, &items, &[], &mut changes);
            let places = changes.to_list.iter().map(|&(place, _)| place).collect();
            index.finish(changes);
            (RwLock::new(index), places)
        }
    }

    impl Source for Hanging {
        fn root(&self) -> &str {
            "X:\\"
        }

        fn list(&self, _: &Folder, _: &mut [u64], _: &mut dyn FnMut(Names<'_>)) -> Listing {
            let mut calls = self.calls.lock().unwrap();
            calls.0 += 1;
            self.changed.notify_all();
            drop(self.changed.wait_while(calls, |(_, through)| !*through).unwrap());
            Listing::Failed(windows::core::Error::from(windows::Win32::Foundation::ERROR_NOT_READY))
        }

        fn left(&self) -> bool {
            self.left.load(Ordering::SeqCst)
        }

        fn tell_let_go(&self, tell: Weak<dyn Fn() + Send + Sync>) {
            self.told.lock().unwrap().push(tell);
        }
    }

    /// A drive of this test's own (see `tree`) and its index, its root to be listed.
    fn opened(root: &Path) -> (Arc<Drive>, RwLock<Index>) {
        let drive = Arc::new(Drive::open(&format!("{}\\", root.display())).unwrap());
        let index = RwLock::new(Index::new(drive.root_id, drive.by_id()));
        (drive, index)
    }

    /// A pool's lanes are the machine's: threads asking for more than there are wait till one is given back.
    #[test]
    fn no_more_folders_are_listed_at_once_than_the_pool_has_lanes() {
        let pool = Arc::new(Pool::new(2, 1 << 20));
        let share = pool.share();
        let held = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    let _lane = pool.lane(share.round).unwrap();
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
    /// names): each listed whole, never more folders at once than one each, never more than one part waiting.
    #[test]
    fn two_drives_listed_at_once_share_the_machines_pool() {
        let pool = Arc::new(Pool::new(1, 1));
        let roots = [tree("pool-a", 3, 4, 3), tree("pool-b", 3, 4, 3)];
        let opened: Vec<(Arc<Drive>, RwLock<Index>)> = roots.iter().map(|root| opened(root)).collect();
        std::thread::scope(|scope| {
            for (drive, index) in &opened {
                let pool = &pool;
                scope.spawn(move || round(index, drive, &[super::super::index::ROOT], pool, None));
            }
        });
        for (root, (_, index)) in roots.iter().zip(&opened) {
            assert_eq!(indexed(&index.read().unwrap()), on_disk(root));
        }
        let using = pool.using();
        assert!(using.most_listing <= 2, "a lane each at most: {}", using.most_listing);
        let largest = HANDED + 8 * (size_of::<Item>() + 32);
        assert!(using.most_queued <= largest, "one part waiting at a time: {}", using.most_queued);
        drop(opened);
        for root in roots {
            let _ = std::fs::remove_dir_all(root);
        }
    }

    /// The lanes are split evenly between the rounds listing (those begun first given what does not split, one each
    /// where there are more rounds than lanes), each round taking up to its own share whatever the others hold: one
    /// holding all of them, taken alone, holds another to no fewer than its own share, and takes no more itself till it
    /// holds fewer than its own, where the others waited till it gave one back.
    #[test]
    fn the_lanes_are_split_evenly_between_the_rounds() {
        let using = |holding: &[usize]| Using { holding: holding.iter().enumerate().map(|(at, &held)| (at as u64 + 1, held)).collect(), ..Using::default() };
        assert!(using(&[3]).may_take(0, 4) && !using(&[4]).may_take(0, 4), "alone, all of them");
        let hung = using(&[4, 0]);
        assert!(hung.may_take(1, 4) && !hung.may_take(0, 4), "the other its share, the one holding all no more");
        assert!(using(&[4, 1]).may_take(1, 4) && !using(&[4, 2]).may_take(1, 4), "its own share, no more");
        assert!(using(&[1, 2]).may_take(0, 4) && !using(&[1, 2]).may_take(1, 4));
        let shares = |rounds: usize, lanes: usize| (0..rounds).map(|at| using(&vec![0; rounds]).share(at, lanes)).collect::<Vec<_>>();
        assert_eq!(shares(3, 5), [2, 2, 1]);
        assert_eq!(shares(3, 2), [1, 1, 1], "more rounds than lanes: one each");
        assert!(using(&[1, 1, 0]).may_take(2, 2) && !using(&[1, 1, 1]).may_take(2, 2));
    }

    /// A drive whose listings hang (a USB drive that answers nothing), all the lanes taken by them as it was listed
    /// alone, holds another drive listed meanwhile to no fewer than its share: that one is listed whole while they
    /// hang, where it waited for them to end.
    #[test]
    fn a_drive_whose_listings_hang_does_not_hold_another() {
        let pool = Arc::new(Pool::new(2, 1 << 20));
        let root = tree("hung", 2, 2, 2);
        let (drive, index) = opened(&root);
        let hung = Hanging::new();
        let (hung_index, folders) = Hanging::index(2);
        std::thread::scope(|scope| {
            scope.spawn(|| round(&hung_index, &hung, &folders, &pool, None));
            hung.hanging(2);
            let (done, listed) = std::sync::mpsc::channel();
            let (index, drive, pool) = (&index, &drive, &pool);
            scope.spawn(move || {
                round(index, drive, &[super::super::index::ROOT], pool, None);
                done.send(()).unwrap();
            });
            let in_time = listed.recv_timeout(Duration::from_secs(10)).is_ok();
            let hung_holds = pool.using().holding[0].1;
            hung.let_through();
            assert!(in_time, "listed while the other's listings hang");
            assert_eq!(hung_holds, 2);
        });
        assert_eq!(indexed(&index.read().unwrap()), on_disk(&root));
        drop(drive);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Where more drives are listed than there are lanes, those begun last are listed too while the others' listings
    /// hang (two volumes of a disk that answers nothing holding both lanes of a laptop's two): each round lists one
    /// folder at least at once, where one begun once there were more rounds than lanes waited for one to end, and none
    /// of those whose listings hang did.
    #[test]
    fn more_drives_than_lanes_are_listed_while_the_others_hang() {
        let pool = Arc::new(Pool::new(2, 1 << 20));
        let root = tree("more-than-lanes", 2, 2, 2);
        let (drive, index) = opened(&root);
        let hung = [Hanging::new(), Hanging::new()];
        let indexes = [Hanging::index(1), Hanging::index(1)];
        std::thread::scope(|scope| {
            for (hung, (index, folders)) in hung.iter().zip(&indexes) {
                let pool = &pool;
                scope.spawn(move || round(index, hung, folders, pool, None));
                hung.hanging(1);
            }
            let (done, listed) = std::sync::mpsc::channel();
            let (index, drive, pool) = (&index, &drive, &pool);
            scope.spawn(move || {
                round(index, drive, &[super::super::index::ROOT], pool, None);
                done.send(()).unwrap();
            });
            let in_time = listed.recv_timeout(Duration::from_secs(10)).is_ok();
            let held = pool.using().holding.iter().take(2).map(|&(_, held)| held).collect::<Vec<_>>();
            for hung in &hung {
                hung.let_through();
            }
            assert!(in_time, "listed while both lanes hang");
            assert_eq!(held, [1, 1]);
        });
        assert_eq!(indexed(&index.read().unwrap()), on_disk(&root));
        drop(drive);
        let _ = std::fs::remove_dir_all(root);
    }

    /// A round whose drive is let go of (taken out as it was listed) as its listings hang, their calls not ending
    /// though cancelled, ends at once: its share of the lanes and the lanes its listings hold given back to the
    /// others, where it waited for them for good, the lanes held with it. Its threads end as their calls do, what they
    /// hand on then let go of, its room given back.
    #[test]
    fn a_round_whose_drive_is_let_go_of_ends_though_its_listings_hang() {
        let pool = Arc::new(Pool::new(2, 1 << 20));
        let hung = Hanging::new();
        let (index, folders) = Hanging::index(3);
        let (done, ended) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let (index, folders, hung, pool) = (&index, &folders, &hung, &pool);
            scope.spawn(move || done.send(round(index, hung, folders, pool, None)).unwrap());
            hung.hanging(2);
            hung.let_go();
            let listed = ended.recv_timeout(Duration::from_secs(10));
            let using = pool.using();
            hung.let_through();
            let listed = listed.expect("ended as its drive was let go of");
            assert!(using.holding.is_empty() && using.listing() == 0, "nothing held of the lanes: {using:?}");
            assert_eq!(listed.folders, 0, "none of its listings waited for");
        });
        // (Its threads end as their calls do, holding the drive till then.)
        let until = Instant::now() + Duration::from_secs(10);
        while Arc::strong_count(&hung) > 1 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(Arc::strong_count(&hung), 1, "its threads ended");
        assert_eq!(pool.using().queued, 0, "what they handed on after let go of");
    }

    /// A folder holding more than all the room is put in part by part, never more of it made and waiting than the room
    /// (a part of it at a time here), where it was held whole as it waited: listed whole all the same.
    #[test]
    fn a_folder_larger_than_all_the_room_is_put_in_part_by_part() {
        let root = tree("parts", 0, 3000, 0);
        let (drive, index) = opened(&root);
        let room = 48 << 10;
        let pool = Arc::new(Pool::new(2, room));
        round(&index, &drive, &[super::super::index::ROOT], &pool, None);
        assert_eq!(indexed(&index.read().unwrap()), on_disk(&root));
        let whole = 3000 * (size_of::<Item>() + 2 * "file 0000.txt".len());
        let most = pool.using().most_queued;
        assert!(most <= room && most < whole, "{most} waiting at most, of {whole}");
        drop(drive);
        let _ = std::fs::remove_dir_all(root);
    }
}
