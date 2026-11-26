//! The disk organizer's reading: on a thread of its own, the drive's index read for what the window asks to see (see
//! `Ask`), and read again as the drive changes while the window is up, what is read handed to the window (see `Told`).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::disk::{self, Detail, Filter, Light, Places, Survey, Tree, What};
use crate::search::index::{Index, ROOT};
use crate::search::Since;

/// How often the drive is looked at for changes while nothing is asked, and how long at least between two readings of
/// it as it changes (a system drive changes all the time: read anew each second, the window was laid out anew as often).
/// Asked nothing that long, a drive asked to be read ahead is read.
const LOOKED_AT: Duration = Duration::from_millis(1000);
const READ_AGAIN_AFTER: Duration = Duration::from_secs(5);

/// What the window asks to see: a folder of a drive (by its path), filtered and lit so, for a map about `area` square
/// DIPs; `serial` tells one asking from the next.
#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    pub root: String,
    pub focus: PathBuf,
    pub filter: Filter,
    pub light: Option<Light>,
    pub area: f32,
    pub serial: u64,
}

/// What a system item is, as its card tells of it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Info {
    Pagefile,
    Hiberfil,
    Restore,
    ComponentStore,
    Unlisted,
    Bin,
}

/// What was read for an asking.
pub struct Shown {
    pub serial: u64,
    pub root: String,
    /// The folder shown: the one asked for, or, gone, the nearest still there above it.
    pub focus: PathBuf,
    /// The folders from the drive's root to it: each's name and path.
    pub crumbs: Vec<(String, PathBuf)>,
    pub tree: Tree,
    /// Each node's (see `Tree`): its path (a rest's, or what is in no file, its folder's), whether it is in a OneDrive
    /// folder (what is in one may be left in the cloud alone), whether it is in the Recycle Bin or is it, and what system
    /// item it is.
    pub paths: Vec<PathBuf>,
    pub onedrive: Vec<bool>,
    pub binned: Vec<bool>,
    pub infos: Vec<Option<Info>>,
    pub detail: Detail,
    /// When the drive was surveyed, and before when a file is over a year old.
    pub at: i64,
    pub old_before: i64,
    /// The drive's size and what is free, and what the Recycle Bin holds (bytes, files).
    pub total: u64,
    pub free: u64,
    pub bin: (u64, u64),
    /// How many times its index had been put anew (its entries' places others), and whether the drive changed since
    /// it was last shown (not asked anew: read again as it changed).
    pub replaced: u64,
    pub changed: bool,
}

/// What the window asks of the reading thread: what is to be shown, or a drive's root to be read ahead of its being
/// asked (once nothing else has been asked for a while), so it is shown at once as it is.
enum Asked {
    Show(Ask),
    Ahead(Ask),
}

/// What reading an asking came to.
pub enum Told {
    Shown(Box<Shown>),
    /// A drive's root read ahead.
    Ahead(Box<Shown>),
    /// Its drive's files are being found still.
    Listing(u64),
    /// Its drive is not searched (gone).
    Nowhere(u64),
}

/// The reading thread, as the window holds it: what it is asked, and where what it reads is left (the window told by
/// `told_by` as it is).
pub struct Worker {
    asks: Sender<Asked>,
    told: Arc<Mutex<Vec<Told>>>,
}

impl Worker {
    /// Started, `window` told `message` as each reading is left in `told`.
    pub fn start(window: HWND, message: u32) -> Worker {
        let (asks, asked) = std::sync::mpsc::channel();
        let told = Arc::new(Mutex::new(Vec::new()));
        let left = told.clone();
        let window = window.0 as isize;
        let _ = std::thread::Builder::new().name("glance: disk organizer reading".into()).spawn(move || {
            crate::settle::background("disk organizer: reading a drive");
            run(asked, left, HWND(window as *mut _), message);
        });
        Worker { asks, told }
    }

    pub fn ask(&self, ask: Ask) {
        let _ = self.asks.send(Asked::Show(ask));
    }

    /// Each of `asks` (a drive's root each) read ahead, one by one as nothing else is asked.
    pub fn read_ahead(&self, asks: Vec<Ask>) {
        for ask in asks {
            let _ = self.asks.send(Asked::Ahead(ask));
        }
    }

    /// What was read since it was last taken, as it was told.
    pub fn take(&self) -> Vec<Told> {
        std::mem::take(&mut *self.told.lock().unwrap())
    }
}

/// What was read last: the drive's survey, as of which reading of its index.
struct Kept {
    root: String,
    replaced: u64,
    watch: crate::search::Watch,
    survey: Survey,
}

fn run(asked: Receiver<Asked>, told: Arc<Mutex<Vec<Told>>>, window: HWND, message: u32) {
    let places = Places::of_this_pc();
    let mut kept: Option<Kept> = None;
    let mut last: Option<Ask> = None;
    let mut ahead: VecDeque<Ask> = VecDeque::new();
    let mut listing = false;
    let mut read_at = std::time::Instant::now();
    loop {
        let first = asked.recv_timeout(LOOKED_AT);
        let idle = matches!(first, Err(RecvTimeoutError::Timeout));
        if matches!(first, Err(RecvTimeoutError::Disconnected)) {
            return;
        }
        // (The latest shown of all asked meanwhile.)
        let mut ask = None;
        for asking in first.into_iter().chain(asked.try_iter()) {
            match asking {
                Asked::Show(show) => ask = Some(show),
                Asked::Ahead(root) => ahead.push_back(root),
            }
        }
        let mut gone = false;
        let mut tell = |reading: Told| {
            told.lock().unwrap().push(reading);
            gone |= unsafe { PostMessageW(Some(window), message, WPARAM(0), LPARAM(0)) }.is_err();
        };
        let (ask, anew) = match (ask, &last) {
            (Some(ask), _) => (ask, true),
            // Asked nothing: read again as it changed, or as its files are found.
            (None, Some(last)) if listing || (read_at.elapsed() >= READ_AGAIN_AFTER && kept.as_ref().is_some_and(|kept| crate::search::read_since(kept.replaced, &kept.watch))) => (last.clone(), false),
            // Asked nothing for a while, and nothing changed: a drive read ahead (not the one shown), its survey not
            // kept.
            (None, _) => {
                if let Some(root) = ahead.pop_front().filter(|_| idle).filter(|root| last.as_ref().is_none_or(|last| !last.root.eq_ignore_ascii_case(&root.root))) {
                    if let Told::Shown(shown) = read(&root, &places, &mut None, true, &mut |_| {}) {
                        tell(Told::Ahead(shown));
                    }
                }
                if gone {
                    return;
                }
                continue;
            }
        };
        let reading = read(&ask, &places, &mut kept, anew, &mut tell);
        read_at = std::time::Instant::now();
        listing = matches!(reading, Told::Listing(_));
        last = Some(ask);
        tell(reading);
        if gone {
            return;
        }
    }
}

/// `ask` read, the survey kept (`kept`) taken again where the drive changed since, or it is of another drive. Asked
/// anew of a drive changed since its survey (its entries in their places still), what the survey kept shows is told
/// at once (by `tell`), as surveying a drive whole takes a while: what is shown moves as it is asked, and is shown as
/// the drive is now once it is surveyed again.
fn read(ask: &Ask, places: &Places, kept: &mut Option<Kept>, anew: bool, tell: &mut dyn FnMut(Told)) -> Told {
    let space = disk::act::space(&ask.root);
    let mine = kept.take().filter(|kept| kept.root.eq_ignore_ascii_case(&ask.root));
    let since = mine.as_ref().map(|kept| (kept.replaced, &kept.watch));
    let mut survey = None;
    let mut retaken = false;
    let reading = crate::search::read_drive(&ask.root, since, |index, since| {
        let had = mine.as_ref().map(|kept| &kept.survey);
        if let (Some(survey), Some(kept), Since::Changed, true) = (had, &mine, since, anew) {
            let mut shown = shown(index, survey, ask);
            shown.replaced = kept.replaced;
            tell(Told::Shown(Box::new(shown)));
        }
        let survey = match had {
            Some(survey) if since == Since::Same => survey,
            _ => {
                retaken = had.is_some();
                survey.insert(Survey::take(index, &ask.root, places, space, disk::now()))
            }
        };
        shown(index, survey, ask)
    });
    match reading {
        crate::search::Reading::Read { read: mut shown, replaced, watch } => {
            let survey = match survey {
                Some(survey) => survey,
                None => mine.map(|kept| kept.survey).expect("surveyed, or kept unchanged"),
            };
            shown.replaced = replaced;
            shown.changed = retaken && !anew;
            *kept = Some(Kept { root: ask.root.clone(), replaced, watch, survey });
            Told::Shown(Box::new(shown))
        }
        crate::search::Reading::Listing => Told::Listing(ask.serial),
        crate::search::Reading::Nowhere => Told::Nowhere(ask.serial),
    }
}

/// The folder at `path` on the drive at `root`, or the nearest above it that is (its place, and its path).
fn nearest(index: &Index, root: &str, path: &Path) -> (u32, PathBuf) {
    let text = path.to_string_lossy();
    let on_drive = text.get(..root.len()).is_some_and(|start| start.eq_ignore_ascii_case(root));
    let mut rest = if on_drive { PathBuf::from(&text[root.len()..]) } else { PathBuf::new() };
    loop {
        if rest.as_os_str().is_empty() {
            return (ROOT, PathBuf::from(root));
        }
        if let Some(place) = index.place_of(&rest) {
            if let Some(found) = index.path(place, root) {
                return (place, found);
            }
        }
        if !rest.pop() {
            return (ROOT, PathBuf::from(root));
        }
    }
}

/// What `ask` shows of the drive `survey` surveyed, its index `index`.
fn shown(index: &Index, survey: &Survey, ask: &Ask) -> Shown {
    let (focus, focus_path) = nearest(index, &ask.root, &ask.focus);
    let focus = if survey.folder(focus).is_some() { focus } else { ROOT };
    let focus_path = if focus == ROOT { PathBuf::from(&ask.root) } else { focus_path };
    let tree = Tree::take(index, survey, focus, ask.area);
    let detail = Detail::take(index, survey, focus, ask.filter, ask.light.as_ref(), &tree);
    let mut parents = vec![0; tree.nodes.len()];
    for (at, node) in tree.nodes.iter().enumerate() {
        for &child in &node.children {
            parents[child] = at;
        }
    }
    let mut paths = vec![focus_path.clone(); tree.nodes.len()];
    for at in 1..tree.nodes.len() {
        // (A node comes after the one it is in.)
        let above = paths[parents[at]].clone();
        paths[at] = match tree.nodes[at].what {
            What::Rest | What::Unlisted => above,
            _ => above.join(&tree.nodes[at].name),
        };
    }
    // The folder each node is in, as surveyed: a folder's own place for what is in it.
    let folder_of = |at: usize| -> u32 {
        let node = &tree.nodes[at];
        match node.what {
            What::Folder(_) => survey.folder(node.place).map_or(ROOT, |folder| folder.parent),
            What::Rest => node.place,
            What::Unlisted => ROOT,
            What::File { .. } => tree.nodes[parents[at]].place,
        }
    };
    let onedrive = (0..tree.nodes.len()).map(|at| at > 0 && tree.nodes[at].what != What::Unlisted && survey.onedrive.contains(&folder_of(at))).collect();
    let binned = (0..tree.nodes.len())
        .map(|at| {
            let node = &tree.nodes[at];
            match node.what {
                What::Folder(_) => survey.in_bin(node.place),
                What::Unlisted => false,
                _ => survey.in_bin(folder_of(at)),
            }
        })
        .collect();
    let infos = (0..tree.nodes.len())
        .map(|at| {
            let node = &tree.nodes[at];
            let in_root = at > 0 && folder_of(at) == ROOT;
            match node.what {
                What::Unlisted => Some(Info::Unlisted),
                What::Folder(disk::Role::Bin) => Some(Info::Bin),
                What::Folder(_) if in_root && node.folded == "system volume information" => Some(Info::Restore),
                What::Folder(_) if node.folded == "winsxs" && survey.windows.is_some_and(|windows| folder_of(at) == windows) => Some(Info::ComponentStore),
                What::File { .. } if in_root && node.folded == "pagefile.sys" => Some(Info::Pagefile),
                What::File { .. } if in_root && node.folded == "hiberfil.sys" => Some(Info::Hiberfil),
                _ => None,
            }
        })
        .collect();
    let crumbs = survey
        .chain(focus)
        .into_iter()
        .map(|place| {
            let path = if place == ROOT { PathBuf::from(&ask.root) } else { index.path(place, &ask.root).unwrap_or_default() };
            let name = if place == ROOT { ask.root.trim_end_matches('\\').to_string() } else { index.name(place).to_string_lossy().into_owned() };
            (name, path)
        })
        .collect();
    let bin = survey.bin.and_then(|bin| survey.folder(bin)).map_or((0, 0), |bin| (bin.held.bytes, bin.held.files));
    Shown {
        serial: ask.serial,
        root: ask.root.clone(),
        focus: focus_path,
        crumbs,
        tree,
        paths,
        onedrive,
        binned,
        infos,
        detail,
        at: survey.at,
        old_before: survey.old_before,
        total: survey.total,
        free: survey.free,
        bin,
        replaced: 0,
        changed: false,
    }
}
