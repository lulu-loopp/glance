//! Files to and from a paired phone's browser: the remote's page (an iPhone's, say, which has no Glance app). It asks
//! everything over the phone channel, signed with the key it paired by (see `Helper::serve` in `main`), and holds no
//! certificate: it is known by its phone's id (see `fingerprint`). What it sends comes in pieces, each asked of its own,
//! into a receiving as any device's (see `Receiver::offered_by_page`). What this PC sends it waits for it to fetch,
//! piece by piece, while it is open (see `Pages::offer`). The receiver keeps which pages are open and what they are
//! offered (see `Receiver::pages`).

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::link::Outgoing;
use super::send::{kind_of, Job, State};
use super::{extension, shown_path, Device, Event, Offered, Receiver, Refused, Sender, Told, Writing, MOST_FILES, PROGRESS_EVERY, WORDS_SHOWN};

/// The most a piece may hold, either way.
pub const PIECE: usize = 512 << 10;
/// A page asks how this PC is every two seconds while it shows: one that has not asked for this long is not open.
const OPEN_FOR: Duration = Duration::from_secs(8);
/// What this PC offers a page is given up where the page fetches nothing of it for this long.
const WAITS: Duration = Duration::from_secs(120);

/// The fingerprint a page is known by among the devices sent to (it has no certificate): its phone's id, after `web:`.
pub fn fingerprint(phone: &str) -> String {
    format!("web:{phone}")
}

/// The phone of the page known by `fingerprint` (see `fingerprint`), where it is a page's.
pub fn phone_of(fingerprint: &str) -> Option<&str> {
    fingerprint.strip_prefix("web:")
}

/// A receiving a page began: its id, and each file's id. None for words, or nothing.
pub type Begun = Option<(String, Vec<String>)>;

/// A page open: its phone's name, where it asks from, when it last asked.
struct Open {
    name: String,
    address: IpAddr,
    seen: Instant,
}

/// The pages open, and what this PC offers each (see `Offer`), by their phones' ids.
#[derive(Default)]
pub struct Pages {
    open: Mutex<HashMap<String, Open>>,
    offered: Mutex<HashMap<String, Offer>>,
}

impl Pages {
    /// The page of phone `phone` asked how this PC is (as `name`, from `address`): open. Whether it was not before (the
    /// devices are then told anew: it may be sent to).
    pub fn seen(&self, phone: &str, name: &str, address: IpAddr) -> bool {
        let mut open = self.open.lock().unwrap();
        let was = open.get(phone).is_some_and(|page| page.seen.elapsed() < OPEN_FOR);
        open.insert(phone.to_string(), Open { name: name.to_string(), address, seen: Instant::now() });
        !was
    }

    /// The pages open now, as devices to send to; and whether any that was is closed since this was last asked (the
    /// devices are then told anew).
    pub fn open(&self) -> (Vec<Device>, bool) {
        let mut open = self.open.lock().unwrap();
        let before = open.len();
        open.retain(|_, page| page.seen.elapsed() < OPEN_FOR);
        let closed = open.len() != before;
        let mut pages: Vec<(&String, &Open)> = open.iter().collect();
        pages.sort_by_key(|(_, page)| std::cmp::Reverse(page.seen));
        let devices = pages
            .into_iter()
            .map(|(phone, page)| Device { fingerprint: fingerprint(phone), alias: page.name.clone(), model: None, kind: Some("mobile".into()), address: page.address, port: 0, https: false, glance: true, heard: page.seen, proven: true })
            .collect();
        (devices, closed)
    }
}

impl Receiver {
    /// A sending offered by a page as `sender` (its phone's name, its fingerprint as `fingerprint` makes it, where it
    /// asks from): its files (`files`, each one's name, size and type), or words alone (`words`). Taken without asking
    /// where `direct` (the phone's files are, see `Phone::files`), else as the user answers (never trusted by that: a
    /// page has no certificate to trust). The receiving's id and each file's id (its place among `files`); none for
    /// words, or nothing.
    pub fn offered_by_page(&self, sender: Sender, files: &[(String, u64, String)], words: Option<String>, direct: bool) -> Result<Begun, Refused> {
        let kept = (self.kept)();
        if !kept.on {
            return Err((403, "not receiving"));
        }
        if files.len() > MOST_FILES {
            return Err((400, "too many files"));
        }
        let offered: Vec<Offered> = match &words {
            Some(words) => vec![Offered { id: "0".into(), name: "text.txt".into(), size: words.len() as u64, kind: "text/plain".into(), words: Some(words.chars().take(WORDS_SHOWN).collect()) }],
            None => files.iter().enumerate().map(|(at, (name, size, kind))| Offered { id: at.to_string(), name: shown_path(name), size: *size, kind: kind.chars().take(100).collect(), words: None }).collect(),
        };
        if offered.is_empty() {
            return Ok(None);
        }
        if words.is_none() {
            let (most_file, most_sending) = self.limits();
            if offered.iter().any(|file| file.size > most_file) {
                return Err((403, "a file too large"));
            }
            let total = offered.iter().try_fold(0u64, |total, file| total.checked_add(file.size)).ok_or((400, "sizes beyond counting"))?;
            if most_sending.is_some_and(|most| total > most) {
                return Err((403, "no room for them"));
            }
            self.room_for_one()?;
        }
        if !direct {
            self.asked_about(&sender, &offered, &kept)?;
        }
        if let Some(words) = words {
            self.message(sender, words);
            return Ok(None);
        }
        let ids = offered.iter().map(|file| file.id.clone()).collect();
        let (id, _) = self.begin(sender, offered.into_iter().map(|file| (file, None)).collect())?;
        Ok(Some((id, ids)))
    }

    /// A piece of file `file` of the receiving `session`, from the page of `from` (its fingerprint) that sends it: its
    /// bytes from `at`, the one after the last (one sent again that came already, its answer lost, is taken as come).
    /// The file kept as its last piece comes, and the receiving ended as all its files have.
    pub fn piece(&self, session: &str, file: &str, from: &str, at: u64, bytes: &[u8]) -> Result<(), Refused> {
        let mut going = self.session.lock().unwrap();
        let receiving = going.as_mut().filter(|going| going.id == session).ok_or((403, "no such receiving"))?;
        if receiving.from.fingerprint != from {
            return Err((403, "not from where the sending was offered"));
        }
        let incoming = receiving.files.get_mut(file).ok_or((403, "no such file"))?;
        let size = incoming.offered.size;
        if incoming.came.is_some() || (at < incoming.done && at + bytes.len() as u64 == incoming.done) {
            return Ok(());
        }
        if at != incoming.done {
            return Err((409, "not the piece that comes next"));
        }
        let writing = match incoming.pieces.as_mut() {
            Some(writing) => writing,
            None => incoming.pieces.insert(Writing::begin(&self.inbox)?),
        };
        if let Err(refused) = writing.add(bytes, size) {
            incoming.pieces.take().expect("written in pieces").gone();
            incoming.done = 0;
            return Err(refused);
        }
        incoming.done = writing.done;
        if incoming.done == size {
            let writing = incoming.pieces.take().expect("written in pieces");
            match writing.whole(size, None, &extension(&incoming.offered.name), &self.inbox) {
                Ok(name) => incoming.came = Some(name),
                Err(refused) => {
                    incoming.done = 0;
                    return Err(refused);
                }
            }
        }
        receiving.touched = Instant::now();
        let whole = receiving.whole();
        if whole || receiving.told.elapsed() >= PROGRESS_EVERY {
            receiving.told = Instant::now();
            self.told.tell(Event::Progress { session: session.to_string(), done: receiving.done(), of: receiving.total() });
        }
        if whole {
            let ended = going.take().expect("the receiving going on");
            drop(going);
            self.end(ended, None);
        }
        Ok(())
    }

    /// The receiving `session` given up by the page of `from` (its fingerprint) that sends it.
    pub fn cancelled_by(&self, session: &str, from: &str) {
        let mut going = self.session.lock().unwrap();
        if going.as_ref().is_some_and(|going| going.id == session && going.from.fingerprint == from) {
            let ended = going.take().expect("the receiving going on");
            drop(going);
            self.end(ended, Some("cancelled by the sender".into()));
        }
    }
}

/// A file or words this PC sends a page: where the file is, or the words; the name it is told; how large; its type.
struct Sent {
    path: Option<PathBuf>,
    words: Option<String>,
    name: String,
    size: u64,
    kind: &'static str,
}

/// What this PC sends a page, waiting for the page to fetch it: the sending's number, what it sends, how much of each
/// the page has fetched, the sending as Glance may cancel it, whom to tell how it goes, when the page last fetched
/// any of it (or it was offered), and when it was last told how far it is.
struct Offer {
    job: u64,
    sent: Vec<Sent>,
    fetched: Vec<u64>,
    going: Arc<Job>,
    told: Arc<dyn Told>,
    moved: Instant,
    progress: Instant,
}

impl Offer {
    fn tell(&self, state: State) {
        self.told.tell(Event::Sending { job: self.job, state });
    }

    /// All the files fetched (the words go with the offer itself).
    fn fetched_all(&self) -> bool {
        self.sent.iter().zip(&self.fetched).all(|(sent, fetched)| sent.path.is_none() || *fetched >= sent.size)
    }

    fn of(&self) -> u64 {
        self.sent.iter().filter(|sent| sent.path.is_some()).map(|sent| sent.size).sum()
    }
}

impl Pages {
    /// `files` offered to the page of `phone` as sending `job` (told so: `Waiting`; `Sending` as the page fetches them;
    /// `Sent` once it has all). The page not open, or offered another meanwhile, it fails or is busy at once.
    pub fn offer(&self, phone: &str, job: u64, files: &[Outgoing], going: Arc<Job>, told: Arc<dyn Told>) {
        let tell = |state: State| told.tell(Event::Sending { job, state });
        if self.open.lock().unwrap().get(phone).is_none_or(|page| page.seen.elapsed() >= OPEN_FOR) {
            tell(State::Failed { why: "the page is not open".into() });
            return;
        }
        let mut sent = Vec::new();
        for file in files {
            match file {
                Outgoing::File { path, name } => match std::fs::metadata(path) {
                    Ok(metadata) if metadata.is_file() => sent.push(Sent { path: Some(path.clone()), words: None, name: name.clone(), size: metadata.len(), kind: kind_of(name) }),
                    _ => {
                        tell(State::Failed { why: format!("{} cannot be read", path.display()) });
                        return;
                    }
                },
                Outgoing::Words { words, name } => sent.push(Sent { path: None, words: Some(words.clone()), name: name.clone(), size: words.len() as u64, kind: "text/plain" }),
            }
        }
        let mut offered = self.offered.lock().unwrap();
        if offered.contains_key(phone) {
            tell(State::Busy);
            return;
        }
        let fetched = vec![0; sent.len()];
        crate::journal::note(format!("nearby: {} things offered to the page of {phone}", sent.len()));
        offered.insert(phone.to_string(), Offer { job, sent, fetched, going, told: told.clone(), moved: Instant::now(), progress: Instant::now() });
        tell(State::Waiting);
    }

    /// What the page of `phone` is offered, as it is told with how this PC is: the sending's number and each thing's id,
    /// name, size and type, words given whole. None where nothing is. Words alone are sent once the page is told them.
    pub fn offered_to(&self, phone: &str) -> Option<Value> {
        let mut offered = self.offered.lock().unwrap();
        let offer = offered.get(phone)?;
        let told = json!({
            "job": offer.job,
            "files": offer.sent.iter().enumerate().map(|(at, sent)| json!({ "id": at, "name": sent.name, "size": sent.size, "type": sent.kind, "words": sent.words })).collect::<Vec<_>>(),
        });
        if offer.fetched_all() {
            let offer = offered.remove(phone).expect("offered");
            offer.tell(State::Sent);
        }
        Some(told)
    }

    /// A piece of thing `at` of sending `job` offered to the page of `phone`: its bytes from `from`, `PIECE` at most. All
    /// of the files fetched, the sending is sent.
    pub fn piece_of(&self, phone: &str, job: u64, at: usize, from: u64) -> Result<Vec<u8>, Refused> {
        use std::io::{Read, Seek, SeekFrom};
        let mut offered = self.offered.lock().unwrap();
        let offer = offered.get_mut(phone).filter(|offer| offer.job == job).ok_or((404, "no such sending"))?;
        if offer.going.cancelled() {
            return Err((410, "the sending was cancelled"));
        }
        let sent = offer.sent.get(at).ok_or((404, "no such file"))?;
        let path = sent.path.clone().ok_or((404, "no such file"))?;
        if from > sent.size {
            return Err((416, "beyond the file"));
        }
        let wanted = (sent.size - from).min(PIECE as u64) as usize;
        let mut bytes = vec![0u8; wanted];
        let mut file = std::fs::File::open(&path).map_err(|_| (500, "the file cannot be read"))?;
        file.seek(SeekFrom::Start(from)).and_then(|_| file.read_exact(&mut bytes)).map_err(|_| (500, "the file cannot be read"))?;
        let end = from + wanted as u64;
        offer.fetched[at] = offer.fetched[at].max(end);
        offer.moved = Instant::now();
        if offer.fetched_all() {
            let offer = offered.remove(phone).expect("offered");
            offer.tell(State::Sent);
        } else if offer.progress.elapsed() >= PROGRESS_EVERY {
            offer.progress = Instant::now();
            offer.tell(State::Sending { done: offer.fetched.iter().sum(), of: offer.of() });
        }
        Ok(bytes)
    }

    /// The page of `phone` turned sending `job` down.
    pub fn refused(&self, phone: &str, job: u64) {
        let mut offered = self.offered.lock().unwrap();
        if offered.get(phone).is_some_and(|offer| offer.job == job) {
            offered.remove(phone).expect("offered").tell(State::Refused);
        }
    }

    /// The sendings to pages given up: cancelled (told so), or not fetched from for `WAITS` (failed: the page was
    /// closed, or its user did not take them).
    pub fn expire(&self) {
        let mut offered = self.offered.lock().unwrap();
        let over: Vec<String> = offered.iter().filter(|(_, offer)| offer.going.cancelled() || offer.moved.elapsed() > WAITS).map(|(phone, _)| phone.clone()).collect();
        for phone in over {
            let offer = offered.remove(&phone).expect("offered");
            offer.tell(if offer.going.cancelled() { State::Cancelled } else { State::Failed { why: "the page did not take it".into() } });
        }
    }
}
