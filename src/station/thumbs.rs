//! Each thing on the station's picture: a file's as Explorer shows it (its thumbnail: a picture's, a video's, a
//! PDF's; else its icon), through the shell, on a thread of its own (a thumbnail may take a while to be made: never
//! on a thread that draws), kept once made. And what a path is (a folder, a file and when it was last written, or
//! nothing), looked at on a thread of its own too (a drive may be slow to answer, one on the network gone: never on a
//! thread that draws), and looked at again a while after.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use crate::ui::canvas::Picture;
use std::sync::{Arc, Mutex, OnceLock};
use windows::core::HSTRING;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HBITMAP, HGDIOBJ};
use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY, SIIGBF_RESIZETOFIT};

/// A picture: its pixels (BGRA, premultiplied, top row first) and how many across and down.
#[derive(Clone, Debug)]
struct Thumb {
    pixels: Vec<u8>,
    size: (u32, u32),
}

/// What a picture is asked for: the file, when it was last written (a file written again is pictured again), the
/// side (physical px) it is to fill, and how (see `Made`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    path: PathBuf,
    written: Option<std::time::SystemTime>,
    side: u32,
    made: Made,
}

/// How a picture is made: as Explorer shows the file (its thumbnail, else its icon), of its own shape, as large as
/// fits the square; its icon alone, as Explorer shows a file of its kind (not what it holds), in the square; its
/// thumbnail alone (none where it has none, not its icon), as `Fit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Made {
    Fit,
    Icon,
    Thumbnail,
}

/// The most pictures kept (those asked for longest ago go first: never one drawn now, as a list turned through many
/// others pictured, else the first made, quick launch's, went and showed none till made again). Past it, those asked
/// for within `DRAWN_STILL` are kept too.
const KEPT_MOST: usize = 256;

struct Kept {
    made: Used<Key, Picture>,
    /// Those being made.
    asked: HashSet<Key>,
    /// Those the shell could not make, made again a while after (see `Failed`).
    failed: HashMap<Key, Failed>,
    /// What each path was found to be, and when; those being looked at.
    looked: Used<PathBuf, (std::time::Instant, Looked)>,
    looking: HashSet<PathBuf>,
}

/// A picture the shell could not make (a thumbnail maker busy or failing, the shell slow to answer while the PC is
/// loaded: none of it says the file has no picture): when it was last tried, how long till it is tried again, and
/// whether it was asked for since (one no longer drawn is not tried again, but made anew when next asked for).
struct Failed {
    at: std::time::Instant,
    wait: std::time::Duration,
    asked: bool,
}

/// How long after a picture first could not be made it is tried again; each time it fails again, twice as long, up to
/// `TRY_AGAIN_MOST`. (Kept as failed for good, quick launch's pictures, failed once, showed letters till Glance
/// was started again.)
const TRY_AGAIN: std::time::Duration = std::time::Duration::from_secs(5);
const TRY_AGAIN_MOST: std::time::Duration = std::time::Duration::from_secs(300);

/// How long after it was last asked for a picture counts as drawn still: kept past `KEPT_MOST`. (More drawn at once
/// than the most kept, each let go of as the next was made, then made again: every picture shown came and went, over
/// and over.)
const DRAWN_STILL: std::time::Duration = std::time::Duration::from_secs(3);

/// What is kept, as many as `most`: those asked for longest ago let go of first, as more come, but never one asked for
/// within `fresh` (more than `most` kept while more are asked for at once).
struct Used<K, V> {
    kept: HashMap<K, (V, u64, std::time::Instant)>,
    /// How many times one was asked for or kept, each so numbered.
    asked: u64,
    most: usize,
    fresh: std::time::Duration,
}

impl<K: std::hash::Hash + Eq + Clone, V> Used<K, V> {
    fn new(most: usize, fresh: std::time::Duration) -> Used<K, V> {
        Used { kept: HashMap::new(), asked: 0, most, fresh }
    }

    /// What is kept for `key`, asked for now.
    fn get<Q: std::hash::Hash + Eq + ?Sized>(&mut self, key: &Q) -> Option<&V>
    where
        K: std::borrow::Borrow<Q>,
    {
        self.asked += 1;
        let asked = self.asked;
        self.kept.get_mut(key).map(|(value, when, at)| {
            (*when, *at) = (asked, std::time::Instant::now());
            &*value
        })
    }

    /// `value` kept for `key`, asked for now, and past `most`, the one asked for longest ago let go of, where it was asked
    /// for longer ago than `fresh`.
    fn insert(&mut self, key: K, value: V) {
        self.asked += 1;
        self.kept.insert(key, (value, self.asked, std::time::Instant::now()));
        while self.kept.len() > self.most {
            let Some((oldest, at)) = self.kept.iter().min_by_key(|(_, (_, when, _))| *when).map(|(key, (_, _, at))| (key.clone(), *at)) else { break };
            if at.elapsed() < self.fresh {
                break;
            }
            self.kept.remove(&oldest);
        }
    }
}

/// What a path is: a folder; a file, when it was last written (as far as that is told); nothing (gone, or on a drive
/// that is not there).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Looked {
    Folder,
    File(Option<std::time::SystemTime>),
    Gone,
}

/// How long what a path was found to be stands before it is looked at again.
const LOOK_AGAIN: std::time::Duration = std::time::Duration::from_secs(3);
/// The most paths kept looked at (those asked about longest ago let go of past it: looked at again as asked).
const LOOKED_MOST: usize = 4096;

static KEPT: Mutex<Option<Kept>> = Mutex::new(None);
static ASKS: OnceLock<Mutex<Sender<Key>>> = OnceLock::new();
static LOOKS: OnceLock<Mutex<Sender<PathBuf>>> = OnceLock::new();
/// Called (on the pictures' thread) as one is made.
static TOLD: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
/// How many times a picture was made, or a path found to be other than it was (see `changes`).
static CHANGES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A picture made, or a path found to be other than it was: counted, and told.
fn tell() {
    CHANGES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if let Some(told) = TOLD.get() {
        told();
    }
}

/// How many times a picture was made, or a path found to be other than it was, so far: for a drawing to tell that
/// what it shows may have changed since, without asking for each picture it may show (asked for, those not shown
/// were made too, and as many as there are files listed kept, those shown let go of to make room).
pub fn changes() -> u64 {
    CHANGES.load(std::sync::atomic::Ordering::Relaxed)
}

/// Starts the pictures' thread; `told` is called as each is made.
pub fn start(told: impl Fn() + Send + Sync + 'static) {
    let _ = TOLD.set(Box::new(told));
    let (sender, asks) = std::sync::mpsc::channel();
    if ASKS.set(Mutex::new(sender)).is_err() {
        return;
    }
    *KEPT.lock().unwrap() = Some(Kept { made: Used::new(KEPT_MOST, DRAWN_STILL), asked: HashSet::new(), failed: HashMap::new(), looked: Used::new(LOOKED_MOST, LOOK_AGAIN), looking: HashSet::new() });
    std::thread::Builder::new().name("glance: station pictures".into()).spawn(move || make(asks)).expect("a thread started");
    let (sender, looks) = std::sync::mpsc::channel();
    let _ = LOOKS.set(Mutex::new(sender));
    std::thread::Builder::new().name("glance: station files".into()).spawn(move || look(looks)).expect("a thread started");
}

/// What the path `path` is, as last looked at (none yet: asked, and told when it is); looked at again where that was
/// a while ago (what it was standing meanwhile).
pub fn looked(path: &Path) -> Option<Looked> {
    let mut kept = KEPT.lock().unwrap();
    let kept = kept.as_mut()?;
    let found = kept.looked.get(path).copied();
    if found.is_none_or(|(at, _)| at.elapsed() >= LOOK_AGAIN) && kept.looking.insert(path.to_path_buf()) {
        if let Some(looks) = LOOKS.get() {
            let _ = looks.lock().unwrap().send(path.to_path_buf());
        }
    }
    found.map(|(_, looked)| looked)
}

/// Paths looked at as they are asked for (see `looked`); told as what one is changed, or is first found.
fn look(looks: Receiver<PathBuf>) {
    while let Ok(path) = looks.recv() {
        let found = match std::fs::metadata(&path) {
            Ok(meta) if meta.is_dir() => Looked::Folder,
            Ok(meta) => Looked::File(meta.modified().ok()),
            Err(_) => Looked::Gone,
        };
        let changed = {
            let mut kept = KEPT.lock().unwrap();
            let Some(kept) = kept.as_mut() else { continue };
            kept.looking.remove(&path);
            let changed = kept.looked.get(&path).is_none_or(|(_, was)| *was != found);
            kept.looked.insert(path, (std::time::Instant::now(), found));
            changed
        };
        if changed {
            tell();
        }
    }
}

/// The picture of the file at `path`, of its own shape, as large as fits `side` physical px square: as made; none yet
/// (asked for, and told when it is), or none to be had.
pub fn of(path: &Path, side: u32) -> Option<Picture> {
    asked(path, side, Made::Fit)
}

/// The icon of the file or folder at `path`, `side` physical px square: Windows' own for it (its program's for a
/// file of a kind one opens, the folder's), as Explorer shows it.
pub fn icon(path: &Path, side: u32) -> Option<Picture> {
    asked(path, side, Made::Icon)
}

fn asked(path: &Path, side: u32, made: Made) -> Option<Picture> {
    // (Pictured as it was last written: a file written again pictured again. Not yet looked at, none yet. A name
    // that is not a path, a Store app's, is pictured as it is.)
    let written = if path.is_absolute() {
        match looked(path)? {
            Looked::File(written) => written,
            Looked::Folder | Looked::Gone => None,
        }
    } else {
        None
    };
    let key = Key { path: path.to_path_buf(), written, side, made };
    let mut kept = KEPT.lock().unwrap();
    let kept = kept.as_mut()?;
    if let Some(made) = kept.made.get(&key) {
        return Some(made.clone());
    }
    // (One that could not be made is tried again in its time, as it is still asked for.)
    if let Some(failed) = kept.failed.get_mut(&key) {
        failed.asked = true;
        return None;
    }
    if kept.asked.insert(key.clone()) {
        if let Some(asks) = ASKS.get() {
            let _ = asks.lock().unwrap().send(key);
        }
    }
    None
}

fn make(asks: Receiver<Key>) {
    // (The shell's thumbnail makers expect a single-threaded apartment.)
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
    }
    loop {
        // (Waiting for the next asked for, or till one that could not be made is to be tried again.)
        let next = {
            let kept = KEPT.lock().unwrap();
            kept.as_ref().and_then(|kept| kept.failed.values().map(|failed| failed.at + failed.wait).min())
        };
        let asked = match next {
            Some(next) => asks.recv_timeout(next.saturating_duration_since(std::time::Instant::now())),
            None => asks.recv().map_err(|_| std::sync::mpsc::RecvTimeoutError::Disconnected),
        };
        match asked {
            Ok(key) => make_one(key),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let due = {
                    let mut kept = KEPT.lock().unwrap();
                    let Some(kept) = kept.as_mut() else { continue };
                    due(&mut kept.failed, std::time::Instant::now())
                };
                due.into_iter().for_each(make_one);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// Those of `failed` due to be tried again by `now`: those asked for since, each kept failed till it is made; those
/// not asked for since let go of (made anew when next asked for).
fn due(failed: &mut HashMap<Key, Failed>, now: std::time::Instant) -> Vec<Key> {
    let mut due = Vec::new();
    failed.retain(|key, failed| {
        if failed.at + failed.wait > now {
            return true;
        }
        if failed.asked {
            due.push(key.clone());
        }
        failed.asked
    });
    due
}

/// How `key` stands having failed at `now`, having failed before as `before`: tried again after twice as long as
/// before (up to `TRY_AGAIN_MOST`), or after `TRY_AGAIN` the first time; asked for since, as it was just tried.
fn failed_again(before: Option<&Failed>, now: std::time::Instant) -> Failed {
    let wait = before.map_or(TRY_AGAIN, |before| (before.wait * 2).min(TRY_AGAIN_MOST));
    Failed { at: now, wait, asked: false }
}

/// The picture asked for as `key` made, kept, and told; one the shell could not make, kept as failed (see `Failed`),
/// told too: what drew it draws again, asking for it, which keeps it tried.
fn make_one(key: Key) {
    let number = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        hasher.finish()
    };
    let made = picture(&key.path, key.side, key.made).map(|thumb| framed(&thumb, key.side, key.made, number));
    {
        let mut kept = KEPT.lock().unwrap();
        let Some(kept) = kept.as_mut() else { return };
        kept.asked.remove(&key);
        match made {
            Ok(made) => {
                kept.failed.remove(&key);
                kept.made.insert(key, made);
            }
            Err(error) => {
                let before = kept.failed.get(&key);
                if before.is_none() {
                    crate::journal::note(format!("station: no picture of {} ({error}), tried again in a while", key.path.display()));
                }
                let failed = failed_again(before, std::time::Instant::now());
                kept.failed.insert(key, failed);
            }
        }
    }
    tell();
}

/// `thumb` made no larger than `side` square, its shape kept: each pixel of the smaller the mean of those of the larger
/// in its box (the shell, asked for one size, may give a larger); one less than half as large made as large as fits
/// (a Store app's icon comes small), each pixel between the four nearest.
fn fitted(thumb: &Thumb, side: u32) -> std::borrow::Cow<'_, Thumb> {
    let (w, h) = thumb.size;
    if w.max(h) * 2 < side && w > 0 && h > 0 {
        return std::borrow::Cow::Owned(enlarged(thumb, side));
    }
    if w <= side && h <= side {
        return std::borrow::Cow::Borrowed(thumb);
    }
    let k = side as f64 / w.max(h) as f64;
    let (to_w, to_h) = (((w as f64 * k).round() as u32).clamp(1, side), ((h as f64 * k).round() as u32).clamp(1, side));
    let mut pixels = vec![0u8; (to_w * to_h * 4) as usize];
    for ty in 0..to_h {
        let (y0, y1) = (ty * h / to_h, ((ty + 1) * h / to_h).max(ty * h / to_h + 1));
        for tx in 0..to_w {
            let (x0, x1) = (tx * w / to_w, ((tx + 1) * w / to_w).max(tx * w / to_w + 1));
            let mut sum = [0u32; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let at = ((y * w + x) * 4) as usize;
                    for (channel, total) in sum.iter_mut().enumerate() {
                        *total += u32::from(thumb.pixels[at + channel]);
                    }
                }
            }
            let count = (y1 - y0) * (x1 - x0);
            let to = ((ty * to_w + tx) * 4) as usize;
            for (channel, total) in sum.into_iter().enumerate() {
                pixels[to + channel] = ((total + count / 2) / count) as u8;
            }
        }
    }
    std::borrow::Cow::Owned(Thumb { pixels, size: (to_w, to_h) })
}

/// `thumb` made as large as fits `side` square, its shape kept, each pixel blended from the four nearest of it.
fn enlarged(thumb: &Thumb, side: u32) -> Thumb {
    let (w, h) = thumb.size;
    let k = side as f64 / w.max(h) as f64;
    let (to_w, to_h) = (((w as f64 * k).round() as u32).clamp(1, side), ((h as f64 * k).round() as u32).clamp(1, side));
    let mut pixels = vec![0u8; (to_w * to_h * 4) as usize];
    let at = |x: u32, y: u32, channel: usize| f64::from(thumb.pixels[((y * w + x) * 4) as usize + channel]);
    for ty in 0..to_h {
        let sy = ((ty as f64 + 0.5) / k - 0.5).clamp(0.0, (h - 1) as f64);
        let (y0, fy) = (sy.floor() as u32, sy.fract());
        let y1 = (y0 + 1).min(h - 1);
        for tx in 0..to_w {
            let sx = ((tx as f64 + 0.5) / k - 0.5).clamp(0.0, (w - 1) as f64);
            let (x0, fx) = (sx.floor() as u32, sx.fract());
            let x1 = (x0 + 1).min(w - 1);
            let to = ((ty * to_w + tx) * 4) as usize;
            for channel in 0..4 {
                let top = at(x0, y0, channel) * (1.0 - fx) + at(x1, y0, channel) * fx;
                let bottom = at(x0, y1, channel) * (1.0 - fx) + at(x1, y1, channel) * fx;
                pixels[to + channel] = (top * (1.0 - fy) + bottom * fy).round() as u8;
            }
        }
    }
    Thumb { pixels, size: (to_w, to_h) }
}

/// `thumb` without the clear rows and columns round what it shows (an icon is drawn within room round it, a Store app's
/// in much room): what it shows made as large as the tile, as every picture's is.
fn trimmed(thumb: &Thumb) -> std::borrow::Cow<'_, Thumb> {
    let (w, h) = thumb.size;
    let seen = |x: u32, y: u32| thumb.pixels[((y * w + x) * 4 + 3) as usize] > 0;
    let rows: Vec<u32> = (0..h).filter(|y| (0..w).any(|x| seen(x, *y))).collect();
    let columns: Vec<u32> = (0..w).filter(|x| (0..h).any(|y| seen(*x, y))).collect();
    let (Some(&top), Some(&bottom), Some(&left), Some(&right)) = (rows.first(), rows.last(), columns.first(), columns.last()) else {
        return std::borrow::Cow::Borrowed(thumb);
    };
    if (top, left, bottom, right) == (0, 0, h - 1, w - 1) {
        return std::borrow::Cow::Borrowed(thumb);
    }
    let (to_w, to_h) = (right - left + 1, bottom - top + 1);
    let pixels = (top..=bottom).flat_map(|y| thumb.pixels[((y * w + left) * 4) as usize..((y * w + right + 1) * 4) as usize].iter().copied()).collect();
    std::borrow::Cow::Owned(Thumb { pixels, size: (to_w, to_h) })
}

/// `thumb` in the middle of a square `side` across (made smaller to fit where it is larger), clear round it, as a
/// picture known by `key`.
fn squared(thumb: &Thumb, side: u32, key: u64) -> Picture {
    let thumb = trimmed(thumb);
    let thumb = fitted(&thumb, side);
    let (w, h) = thumb.size;
    let (left, top) = ((side - w) / 2, (side - h) / 2);
    let mut pixels = vec![0u8; (side * side * 4) as usize];
    for row in 0..h {
        let from = (row * thumb.size.0 * 4) as usize;
        let to = (((top + row) * side + left) * 4) as usize;
        pixels[to..to + (w * 4) as usize].copy_from_slice(&thumb.pixels[from..from + (w * 4) as usize]);
    }
    Picture { key, size: (side, side), pixels: Arc::new(pixels) }
}

/// `thumb` as a picture known by `key`, made `made` for a square `side` across: of its own shape (see `shaped`), or in
/// the square (`squared`).
fn framed(thumb: &Thumb, side: u32, made: Made, key: u64) -> Picture {
    match made {
        Made::Fit | Made::Thumbnail => shaped(thumb, side, key),
        Made::Icon => squared(thumb, side, key),
    }
}

/// The picture of the file at `path`, made `made` for a square `side` physical px across, made now on this thread (a
/// single-threaded apartment: the shell's makers expect one), known by `key`, and not kept: one looked at larger than
/// any card shows it (see `ui::look`), on a thread of its own that may wait for it.
pub fn made_now(path: &Path, side: u32, made: Made, key: u64) -> Option<Picture> {
    picture(path, side, made).ok().map(|thumb| framed(&thumb, side, made, key))
}

/// `thumb` as large as fits a square `side` across (made smaller where it is larger), of its own shape, as a picture
/// known by `key`: drawn whole where it is shown (see `Picture::fitted_in`).
fn shaped(thumb: &Thumb, side: u32, key: u64) -> Picture {
    let thumb = trimmed(thumb);
    let thumb = fitted(&thumb, side);
    Picture { key, size: thumb.size, pixels: Arc::new(thumb.pixels.clone()) }
}

/// The picture of the file at `path` as Explorer shows it, `made` so: as large as fits `side` square, its shape kept
/// (`Fit`), or its icon alone so (`Icon`); else what went wrong.
fn picture(path: &Path, side: u32, made: Made) -> Result<Thumb, String> {
    unsafe {
        // (A program the Start menu names by its file's path, "shell:AppsFolder\D:\App\a.exe", which the shell
        // does not read by that name: by the file itself.)
        let named = path.to_string_lossy();
        let path = match named.strip_prefix("shell:AppsFolder\\").map(Path::new) {
            Some(file) if file.is_absolute() => file.to_path_buf(),
            // (Its parts joined as Windows writes paths: the shell does not read one written with '/'.)
            _ => path.components().collect(),
        };
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None).map_err(|error| format!("the shell found no item: {error}"))?;
        let side = side.max(16) as i32;
        let how = match made {
            Made::Fit => SIIGBF_RESIZETOFIT,
            Made::Icon => SIIGBF_ICONONLY,
            Made::Thumbnail => SIIGBF_RESIZETOFIT | windows::Win32::UI::Shell::SIIGBF_THUMBNAILONLY,
        } | SIIGBF_BIGGERSIZEOK;
        let bitmap = factory.GetImage(SIZE { cx: side, cy: side }, how).map_err(|error| format!("the shell gave no picture: {error}"))?;
        let thumb = pixels_of(bitmap);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        thumb
    }
}

/// A bitmap's pixels (32 bits, BGRA), top row first, premultiplied as they are drawn: the shell makes them straight, each
/// colour whole whatever its alpha (taken as premultiplied, an icon's soft edge came out light, a white rim round it).
unsafe fn pixels_of(bitmap: HBITMAP) -> Result<Thumb, String> {
    unsafe {
        let mut described = BITMAP::default();
        if GetObjectW(HGDIOBJ(bitmap.0), std::mem::size_of::<BITMAP>() as i32, Some((&mut described as *mut BITMAP).cast())) == 0 {
            return Err("the shell's picture could not be described".into());
        }
        let (w, h) = (described.bmWidth, described.bmHeight.abs());
        let mut info = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, ..Default::default() }, ..Default::default() };
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        let screen = windows::Win32::Graphics::Gdi::GetDC(None);
        let lines = GetDIBits(screen, bitmap, 0, h as u32, Some(pixels.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS);
        windows::Win32::Graphics::Gdi::ReleaseDC(None, screen);
        if lines != h {
            return Err(format!("{lines} of the shell's picture's {h} rows were read: {}", windows::core::Error::from_thread()));
        }
        // (A picture with no alpha of its own, as an icon of old may be, wholly opaque.)
        if described.bmBitsPixel != 32 || pixels.chunks_exact(4).all(|pixel| pixel[3] == 0) {
            pixels.chunks_exact_mut(4).for_each(|pixel| pixel[3] = 255);
        }
        for pixel in pixels.chunks_exact_mut(4) {
            let alpha = u32::from(pixel[3]);
            for colour in &mut pixel[..3] {
                *colour = ((u32::from(*colour) * alpha + 127) / 255) as u8;
            }
        }
        Ok(Thumb { pixels, size: (w as u32, h as u32) })
    }
}

#[cfg(test)]
mod made {
    use super::*;

    /// A picture the shell could not make is tried again in a while, then twice as long after each time it fails again,
    /// up to the most: never kept as failed for good (a moment the shell failed once, a letter shown till Glance was
    /// started again). One no longer asked for is let go of, not tried.
    #[test]
    fn a_picture_that_could_not_be_made_is_tried_again() {
        let key = |name: &str| Key { path: PathBuf::from(name), written: None, side: 64, made: Made::Icon };
        let start = std::time::Instant::now();
        let first = failed_again(None, start);
        assert_eq!(first.wait, TRY_AGAIN);
        let mut wait = first.wait;
        for _ in 0..10 {
            wait = failed_again(Some(&Failed { at: start, wait, asked: true }), start).wait;
        }
        assert_eq!(wait, TRY_AGAIN_MOST);
        assert_eq!(failed_again(Some(&Failed { at: start, wait: TRY_AGAIN, asked: true }), start).wait, TRY_AGAIN * 2);

        let mut failed = HashMap::new();
        failed.insert(key("shown"), Failed { asked: true, ..failed_again(None, start) });
        failed.insert(key("gone from view"), failed_again(None, start));
        failed.insert(key("later"), Failed { at: start, wait: TRY_AGAIN * 4, asked: true });
        assert!(due(&mut failed, start).is_empty());
        assert_eq!(failed.len(), 3);
        assert_eq!(due(&mut failed, start + TRY_AGAIN), vec![key("shown")]);
        assert!(failed.contains_key(&key("shown")) && failed.contains_key(&key("later")));
        assert!(!failed.contains_key(&key("gone from view")));
    }

    /// Those kept are let go of as they were last asked for, not as they were made: one asked for all along stays
    /// as many more come and go.
    #[test]
    fn what_is_asked_for_all_along_is_kept() {
        let mut used = Used::new(3, std::time::Duration::ZERO);
        used.insert("quick launch", 0);
        for file in 1..10 {
            assert_eq!(used.get(&"quick launch"), Some(&0), "after {file}");
            used.insert(["a", "b", "c", "d", "e", "f", "g", "h", "i"][file - 1], file);
        }
        assert_eq!(used.get(&"quick launch"), Some(&0));
        assert_eq!(used.get(&"h"), Some(&8));
        assert_eq!(used.get(&"i"), Some(&9));
        assert_eq!(used.get(&"g"), None);
        assert_eq!(used.kept.len(), 3);
    }

    /// More asked for at once than the most kept: none let go of while it was asked for lately (each let go of as the
    /// next came, every picture drawn went and was made again, over and over), those asked for long ago let go of then.
    #[test]
    fn what_is_drawn_now_is_kept_past_the_most() {
        let mut used = Used::new(3, std::time::Duration::from_millis(200));
        for file in 0..5 {
            used.insert(file, file);
        }
        assert_eq!(used.kept.len(), 5);
        assert!((0..5).all(|file| used.get(&file) == Some(&file)));
        std::thread::sleep(std::time::Duration::from_millis(250));
        used.get(&4);
        used.insert(5, 5);
        assert_eq!(used.kept.len(), 3);
        assert_eq!(used.get(&4), Some(&4));
        assert_eq!(used.get(&5), Some(&5));
        assert_eq!(used.get(&0), None);
    }

    /// A thumbnail larger than the side asked for is made smaller to fit, its shape and colours kept, and centred:
    /// never cut down to its corner.
    #[test]
    fn a_larger_thumbnail_is_fitted_whole() {
        // 256 × 128: its left half red, its right half blue (BGRA).
        let pixels = (0..128).flat_map(|_| (0..256).flat_map(|x| if x < 128 { [0, 0, 255, 255] } else { [255, 0, 0, 255] })).collect();
        let picture = squared(&Thumb { pixels, size: (256, 128) }, 128, 7);
        let at = |x: u32, y: u32| &picture.pixels[((y * 128 + x) * 4) as usize..((y * 128 + x) * 4 + 4) as usize];
        // 128 × 64 in the middle: clear above and below, red on its left, blue on its right.
        assert_eq!(at(10, 10), [0, 0, 0, 0]);
        assert_eq!(at(10, 64), [0, 0, 255, 255]);
        assert_eq!(at(120, 64), [255, 0, 0, 255]);
        assert_eq!(at(120, 120), [0, 0, 0, 0]);
    }

    /// A picture to be drawn whole keeps its shape: made smaller to fit, with no clear room round it.
    #[test]
    fn a_picture_keeps_its_shape() {
        let pixels = (0..128).flat_map(|_| (0..256).flat_map(|x| if x < 128 { [0, 0, 255, 255] } else { [255, 0, 0, 255] })).collect();
        let picture = shaped(&Thumb { pixels, size: (256, 128) }, 128, 7);
        assert_eq!(picture.size, (128, 64));
        assert_eq!(&picture.pixels[..4], [0, 0, 255, 255]);
        assert_eq!(&picture.pixels[(127 * 4) as usize..(128 * 4) as usize], [255, 0, 0, 255]);
    }

    /// An icon drawn small in much clear room is made as large as the tile: the clear room taken off first.
    #[test]
    fn an_icon_in_much_room_fills_the_tile() {
        // 128 × 128, clear but for 16 × 16 red in its middle.
        let pixels = (0..128u32).flat_map(|y| (0..128u32).flat_map(move |x| if (56..72).contains(&x) && (56..72).contains(&y) { [0, 0, 255, 255] } else { [0, 0, 0, 0] })).collect();
        let picture = squared(&Thumb { pixels, size: (128, 128) }, 128, 7);
        assert_eq!(&picture.pixels[((4 * 128 + 4) * 4) as usize..((4 * 128 + 4) * 4 + 4) as usize], [0, 0, 255, 255]);
    }

    /// A thumbnail less than half as large as the side asked for is made as large as fits, its colours kept.
    #[test]
    fn a_small_thumbnail_is_made_larger() {
        let pixels = (0..16).flat_map(|_| (0..32).flat_map(|x| if x < 16 { [0, 0, 255, 255] } else { [255, 0, 0, 255] })).collect();
        let picture = squared(&Thumb { pixels, size: (32, 16) }, 128, 7);
        let at = |x: u32, y: u32| &picture.pixels[((y * 128 + x) * 4) as usize..((y * 128 + x) * 4 + 4) as usize];
        // 128 × 64 in the middle.
        assert_eq!(at(10, 10), [0, 0, 0, 0]);
        assert_eq!(at(10, 64), [0, 0, 255, 255]);
        assert_eq!(at(120, 64), [255, 0, 0, 255]);
    }
}

#[cfg(test)]
mod tests {
    /// A picture's file gives its picture, another file its icon; each within the side asked for.
    #[test]
    #[ignore = "asks this PC's shell"]
    fn files_are_pictured_as_explorer_shows_them() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        let picture = super::picture(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/icons/128x128@2x.png")), 96, super::Made::Fit).expect("a picture's");
        assert!(picture.size.0 <= 96 && picture.size.1 <= 96 && picture.size.0.max(picture.size.1) >= 48, "{:?}", picture.size);
        let icon = super::picture(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")), 48, super::Made::Fit).expect("an icon");
        assert!(icon.pixels.chunks_exact(4).any(|pixel| pixel[3] > 0));
        println!("picture {:?}, icon {:?}", picture.size, icon.size);
    }

    /// A wide picture is as many pixels across as it is asked for, of its own shape: drawn whole, as sharp as its face.
    #[test]
    #[ignore = "asks this PC's shell"]
    fn a_wide_picture_is_made_whole_at_the_pixels_asked_for() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        // (A picture three times as wide as tall, as a screenshot of a wide window is, written as a plain bitmap.)
        let (w, h) = (1200u32, 400u32);
        let mut file = Vec::new();
        let size = 54 + w * h * 3;
        file.extend_from_slice(b"BM");
        for value in [size, 0, 54, 40, w, h] {
            file.extend_from_slice(&value.to_le_bytes());
        }
        file.extend_from_slice(&1u16.to_le_bytes());
        file.extend_from_slice(&24u16.to_le_bytes());
        for value in [0u32, w * h * 3, 2835, 2835, 0, 0] {
            file.extend_from_slice(&value.to_le_bytes());
        }
        for y in 0..h {
            for x in 0..w {
                file.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, ((x / 7 + y / 5) % 256) as u8]);
            }
        }
        let path = std::env::temp_dir().join(format!("glance-wide-{}.bmp", std::process::id()));
        std::fs::write(&path, file).unwrap();
        let side = 320;
        let picture = super::picture(&path, side, super::Made::Fit);
        let _ = std::fs::remove_file(&path);
        let picture = super::shaped(&picture.expect("a picture's"), side, 7);
        assert_eq!(picture.size, (side, (side as f32 / 3.0).round() as u32));
    }

    /// A program's icon, its edges soft, as it is drawn: premultiplied, no colour past its alpha (where one was, it was
    /// drawn lighter than it is, a white rim round the icon).
    #[test]
    #[ignore = "asks this PC's shell"]
    fn an_icon_s_soft_edges_are_drawn_as_soft() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        let windows = std::env::var("SystemRoot").unwrap();
        let icon = super::picture(&std::path::Path::new(&windows).join("explorer.exe"), 96, super::Made::Icon).expect("its icon");
        let soft = icon.pixels.chunks_exact(4).filter(|pixel| pixel[3] > 0 && pixel[3] < 255).count();
        let past = icon.pixels.chunks_exact(4).filter(|pixel| pixel[..3].iter().any(|colour| *colour > pixel[3])).count();
        assert!(soft > 0, "no soft edge to look at");
        assert_eq!(past, 0, "of {soft} pixels at its soft edges, {past} have a colour past their alpha");
    }
}
