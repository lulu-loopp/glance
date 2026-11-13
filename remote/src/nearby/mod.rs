//! Nearby transfer: files to and from the devices on this network, as LocalSend's open protocol (v2) has it, so that
//! any phone or computer with LocalSend sends to this PC and takes from it (see `docs/nearby.md`).
//!
//! This helper is the only one of Glance's programs to open a port, and it runs as the user: what comes is written
//! into a folder of its own (`inbox`, in the user's local application data), named by this helper (the name it came
//! with is kept only to be shown), and marked as from the Internet; Glance takes it from there onto the station.
//! What Glance is told and asked goes by `Told` (a pipe of Glance's: see `link`); with no Glance to ask, a device not
//! trusted is turned away and a trusted one's files are taken, and left for Glance to take in when it runs.

pub mod discover;
pub mod http;
pub mod link;
pub mod run;
pub mod send;
pub mod tls;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// LocalSend's port: its multicast group's, and the one a device answers on unless it says otherwise.
pub const PORT: u16 = 53317;
/// The port this PC answers on, where it is free: one of its own, so that a LocalSend app on this PC, started after
/// Glance's helper, still has its own.
pub const OWN_PORT: u16 = 53320;
/// The protocol's version this PC speaks.
pub const VERSION: &str = "2.1";
const API: &str = "/api/localsend/v2";
/// The most a file may be, and the room left on the drive after all of them.
const MOST_FILE: u64 = 4 << 30;
const ROOM_LEFT: u64 = 1 << 30;
/// The most files one sending may have.
const MOST_FILES: usize = 10_000;
/// The most a request's JSON may be (a sending of many files names them all).
const MOST_JSON: usize = 8 << 20;
/// How long the user is given to answer a device not trusted.
pub const ASKED_FOR: Duration = Duration::from_secs(60);
/// A receiving nothing has come for in this long is given up (what came of it kept, what had not, gone).
const IDLE: Duration = Duration::from_secs(60);
/// How long a connection may wait between one piece and the next.
const QUIET: Duration = Duration::from_secs(30);
/// Glance is told of a receiving's progress at most this often.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);
/// How many connections are served at once, and how many of them one address may have.
const AT_ONCE: usize = 64;
const AT_ONCE_EACH: usize = 16;

/// A device on the network, as it tells of itself (LocalSend's `info`, and its announcement).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Info {
    pub alias: String,
    pub version: String,
    pub device_model: Option<String>,
    pub device_type: Option<String>,
    pub fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    pub download: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub announce: Option<bool>,
}

/// A device known on the network: what it said of itself, where it was, when it was last heard.
#[derive(Clone, Debug, Serialize)]
pub struct Device {
    pub fingerprint: String,
    pub alias: String,
    pub model: Option<String>,
    pub kind: Option<String>,
    pub address: IpAddr,
    pub port: u16,
    pub https: bool,
    #[serde(skip)]
    pub heard: Instant,
}

/// Who sends: what they said of themselves, from where, and whether their fingerprint is theirs beyond doubt (they
/// presented its certificate) or only said.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sender {
    pub alias: String,
    pub model: Option<String>,
    pub kind: Option<String>,
    pub fingerprint: String,
    pub verified: bool,
    pub address: IpAddr,
}

/// A file as a sending names it: where the sending is words (a message: see `Receiver::prepare`), its first words
/// too, as they may be shown (at most `WORDS_SHOWN` characters).
#[derive(Clone, Debug, Serialize)]
pub struct Offered {
    pub id: String,
    pub name: String,
    pub size: u64,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub words: Option<String>,
}

/// How much of the words a message sends Glance is given to show as it asks.
const WORDS_SHOWN: usize = 200;

/// What Glance is asked: whether to take what a device not trusted sends.
#[derive(Clone, Debug, Serialize)]
pub struct Asked {
    pub from: Sender,
    pub files: Vec<Offered>,
}

/// Glance's answer: whether to take it, and whether to take what that device sends from now on without asking.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct Answer {
    pub take: bool,
    #[serde(default)]
    pub trust: bool,
}

/// A file come whole: the name it is kept by in the inbox, the name it came with (to be shown), how large it is.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Came {
    pub file: String,
    pub name: String,
    pub size: u64,
    #[serde(rename = "type")]
    pub kind: String,
}

/// What Glance is told.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// A receiving begun: from whom, what.
    Coming { session: String, from: Sender, files: Vec<Offered> },
    /// How much of it has come.
    Progress { session: String, done: u64, of: u64 },
    /// It is over: the files that came whole (none, where it was cancelled or failed before any did), and why it
    /// ended short, where it did.
    Came { session: String, from: Sender, files: Vec<Came>, short: Option<String> },
    /// The devices seen lately, as they are now.
    Devices { devices: Vec<Device> },
    /// A sending of this PC's: how it goes.
    Sending { job: u64, state: send::State },
}

/// What tells Glance and asks it.
pub trait Told: Send + Sync {
    /// Asks the user whether to take what `asked` offers: no answer within `ASKED_FOR` (or no Glance to ask), none.
    fn ask(&self, asked: &Asked) -> Option<Answer>;
    /// The asking going on given up (its device cancelled it): answered as not taken, and taken back from the user.
    fn withdraw(&self);
    fn tell(&self, event: Event);
}

/// What this PC keeps for nearby transfer (see `Config::nearby`): what it is called, the devices whose files are
/// taken without asking, and whether others' are asked about or turned away.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Kept {
    /// Receiving is on.
    pub on: bool,
    /// What this PC is called to others (none: its computer name).
    pub alias: Option<String>,
    pub trusted: Vec<Trusted>,
    /// Devices not trusted are turned away, not asked about.
    pub others_refused: bool,
    /// The most one file may be, and all of one sending, as the station can hold them (Glance keeps them: none
    /// kept, `MOST_FILE` and no more than the drive holds).
    pub most_file: Option<u64>,
    pub most_sending: Option<u64>,
}

/// A device whose files are taken without asking: known by the certificate it presented (a device is only ever
/// trusted so: a fingerprint only said may be anyone's who heard it announced).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Trusted {
    pub fingerprint: String,
    pub alias: String,
    pub kind: Option<String>,
    /// When it last sent, and since when it is trusted (seconds since 1970; none known: 0).
    pub last: u64,
    pub since: u64,
    /// The paired phone that told its certificate, over what it is paired by (see `/v1/nearby/trust` in main): it is
    /// trusted while that phone is paired, and no longer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
}

impl Kept {
    /// Whether what `sender` sends is taken without asking: it presented the certificate of a device trusted.
    pub fn trusts(&self, sender: &Sender) -> bool {
        sender.verified && self.trusted.iter().any(|trusted| trusted.fingerprint.eq_ignore_ascii_case(&sender.fingerprint))
    }

    /// `sender` trusted from now on, where it presented its certificate (else it cannot be: nothing changes); trusted
    /// already, its name and when it last sent kept.
    pub fn trust(&mut self, sender: &Sender) {
        if !sender.verified {
            return;
        }
        let now = seconds();
        match self.trusted.iter_mut().find(|trusted| trusted.fingerprint.eq_ignore_ascii_case(&sender.fingerprint)) {
            Some(trusted) => {
                trusted.alias = sender.alias.clone();
                trusted.last = now;
            }
            None => self.trusted.push(Trusted { fingerprint: sender.fingerprint.to_ascii_uppercase(), alias: sender.alias.clone(), kind: sender.kind.clone(), last: now, since: now, phone: None }),
        }
    }
}

fn seconds() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// Random hex, `bytes` of it.
fn random(bytes: usize) -> String {
    let mut drawn = vec![0u8; bytes];
    getrandom::getrandom(&mut drawn).expect("random bytes");
    drawn.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A name a sending gave a file, as it may be shown: its last part (no folder of the sender's), without what cannot
/// stand in a name, at most 200 characters; nothing left, "file".
pub fn shown_name(given: &str) -> String {
    let kept = shown_part(given.rsplit(['/', '\\']).next().unwrap_or_default());
    if kept.is_empty() { "file".to_string() } else { kept }
}

/// A name a sending gave a file, as it is kept: its place in the folder it was sent in, where it was sent in one
/// ("folder/inner/name", as LocalSend names the files of a folder), each part as a name may be (see `shown_part`), a
/// part that names no folder of its own ("", ".", "..") left out, so that it never reaches out of the folder it is put
/// in. Nothing left, "file".
pub fn shown_path(given: &str) -> String {
    let parts: Vec<String> = given.split(['/', '\\']).map(shown_part).filter(|part| !part.is_empty()).collect();
    if parts.is_empty() { "file".to_string() } else { parts.join("/") }
}

/// One part of a name a sending gave (a file's, or a folder's it is in), without what cannot stand in a name, at most
/// 200 characters: nothing where nothing is left, or it names no folder of its own ("." or "..").
fn shown_part(part: &str) -> String {
    let kept: String = part.chars().filter(|c| !c.is_control() && !"<>:\"|?*".contains(*c)).take(200).collect();
    let kept = kept.trim().trim_end_matches('.').trim().to_string();
    if kept == "." || kept == ".." { String::new() } else { kept }
}

/// The kind of file a name says (the extension of its last part: letters and digits, at most 8, small).
fn extension(name: &str) -> String {
    let last = name.rsplit('/').next().unwrap_or_default();
    let after = last.rsplit_once('.').map_or("", |(_, after)| after);
    if after.chars().all(|c| c.is_ascii_alphanumeric()) && after.len() <= 8 {
        after.to_ascii_lowercase()
    } else {
        String::new()
    }
}

/// What a file of a sending is as it comes.
struct Incoming {
    offered: Offered,
    sha256: Option<String>,
    token: String,
    /// Its bytes come so far (while it is being written), or come whole: its name in the inbox.
    done: u64,
    came: Option<String>,
    writing: bool,
}

/// A receiving: from whom, its files, when it last moved.
struct Session {
    id: String,
    from: Sender,
    files: HashMap<String, Incoming>,
    touched: Instant,
    told: Instant,
}

impl Session {
    fn total(&self) -> u64 {
        self.files.values().map(|file| file.offered.size).sum()
    }

    fn done(&self) -> u64 {
        self.files.values().map(|file| if file.came.is_some() { file.offered.size } else { file.done }).sum()
    }

    fn whole(&self) -> bool {
        self.files.values().all(|file| file.came.is_some())
    }
}

/// How what is kept for nearby transfer is changed (written to the helper's configuration).
type Keeping = Box<dyn Fn(&dyn Fn(&mut Kept)) + Send + Sync>;

/// This PC as a receiver: its certificate, what it says of itself (called as its computer is, but where what it keeps
/// names it otherwise: see `me`), what it keeps, where what comes is written, whom it tells.
pub struct Receiver {
    pub own: tls::Own,
    pub info: Info,
    tls: Arc<rustls::ServerConfig>,
    inbox: PathBuf,
    /// Where a receiving that ended is written down (see `Receipt`), for Glance to take in.
    receipts: PathBuf,
    told: Arc<dyn Told>,
    kept: Box<dyn Fn() -> Kept + Send + Sync>,
    keep: Keeping,
    /// The receiving going on, and whether a device is being asked about (another meanwhile is turned away).
    session: Mutex<Option<Session>>,
    /// The device whose sending the user is being asked about (one at a time), by its address.
    asking: Mutex<Option<IpAddr>>,
    /// The devices heard lately (see `discover`).
    pub devices: Mutex<HashMap<String, Device>>,
    callers: Mutex<HashMap<IpAddr, usize>>,
}

/// What goes wrong with a request: its status, and what is said of it.
type Refused = (u16, &'static str);

impl Receiver {
    /// A receiver as `own` (its certificate) and `info` (what it says of itself), writing into `inbox`, telling
    /// `told`, reading what it keeps by `kept` and changing it by `keep`.
    pub fn new(own: tls::Own, mut info: Info, inbox: PathBuf, told: Arc<dyn Told>, kept: impl Fn() -> Kept + Send + Sync + 'static, keep: impl Fn(&dyn Fn(&mut Kept)) + Send + Sync + 'static) -> std::io::Result<Receiver> {
        let tls = own.server().map_err(std::io::Error::other)?;
        info.fingerprint = own.fingerprint.clone();
        info.version = VERSION.into();
        // What a helper that ended was writing will not come whole: gone.
        if let Ok(entries) = std::fs::read_dir(&inbox) {
            for entry in entries.flatten() {
                if entry.path().extension().is_some_and(|extension| extension == "part") {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        let receipts = Receipt::folder(&inbox);
        Ok(Receiver { own, info, tls, inbox, receipts, told, kept: Box::new(kept), keep: Box::new(keep), session: Mutex::new(None), asking: Mutex::new(None), devices: Mutex::default(), callers: Mutex::default() })
    }

    /// Serves what comes to `listener`, a thread to each, for as long as it listens.
    pub fn serve(self: &Arc<Self>, listener: TcpListener) {
        for stream in listener.incoming().flatten() {
            let Ok(peer) = stream.peer_addr() else { continue };
            {
                let mut callers = self.callers.lock().unwrap();
                if callers.values().sum::<usize>() >= AT_ONCE || callers.get(&peer.ip()).copied().unwrap_or(0) >= AT_ONCE_EACH {
                    continue;
                }
                *callers.entry(peer.ip()).or_default() += 1;
            }
            let receiver = self.clone();
            std::thread::spawn(move || {
                if let Err(error) = receiver.connection(stream, peer.ip()) {
                    if error.kind() != std::io::ErrorKind::UnexpectedEof && error.kind() != std::io::ErrorKind::ConnectionReset {
                        crate::journal::note(format!("nearby: {}: {error}", peer.ip()));
                    }
                }
                let mut callers = receiver.callers.lock().unwrap();
                if let Some(count) = callers.get_mut(&peer.ip()) {
                    *count -= 1;
                    if *count == 0 {
                        callers.remove(&peer.ip());
                    }
                }
            });
        }
    }

    /// One connection: its TLS, then its one request.
    fn connection(&self, stream: TcpStream, from: IpAddr) -> std::io::Result<()> {
        stream.set_read_timeout(Some(QUIET))?;
        stream.set_write_timeout(Some(QUIET))?;
        let tls = rustls::ServerConnection::new(self.tls.clone()).map_err(std::io::Error::other)?;
        let mut stream = rustls::StreamOwned::new(tls, stream);
        // (The handshake first: what certificate the sender presented is known only after it.)
        while stream.conn.is_handshaking() {
            stream.conn.complete_io(&mut stream.sock)?;
        }
        let verified = stream.conn.peer_certificates().and_then(|certs| certs.first()).map(|cert| tls::fingerprint(cert));
        self.request(&mut stream, from, verified)?;
        stream.conn.send_close_notify();
        let _ = stream.flush();
        Ok(())
    }

    /// Reads one request from `stream` (from `from`, which presented the certificate of `verified`, if any), does it
    /// and answers.
    pub fn request<S: Read + Write>(&self, stream: &mut S, from: IpAddr, verified: Option<String>) -> std::io::Result<()> {
        let Some(mut head) = http::head(stream, false)? else { return Ok(()) };
        let path = head.path.strip_prefix(API).map(str::to_string);
        let method = head.method.clone();
        let answered = match (method.as_str(), path.as_deref()) {
            ("POST", Some("/upload")) => {
                let mut body = http::Body::of(&mut head, stream, false)?;
                let done = self.upload(&head, &mut body, from);
                // (What of the body was not read is not waited for: the connection ends with the answer.)
                drop(body);
                done.map(|()| None)
            }
            (method, Some(path)) => {
                let body = http::Body::of(&mut head, stream, false)?.whole(MOST_JSON)?;
                self.asked(method, path, &head, &body, from, verified)
            }
            _ => Err((404, "no such thing")),
        };
        match answered {
            Ok(Some((status, json))) => http::answer(stream, status, Some(&json)),
            Ok(None) => http::answer(stream, 200, None),
            Err((status, said)) => http::answer(stream, status, Some(&json!({ "message": said }).to_string())),
        }
    }

    /// The most one file offered may be, and all of one sending (none: no more than any): as the station can hold
    /// them (`Kept::most_file`, `Kept::most_sending`), and leaving `ROOM_LEFT` on the drive. An offer beyond either is
    /// refused (403); Glance's app sends no more than they say (see `run::told_to_phones`).
    pub fn limits(&self) -> (u64, Option<u64>) {
        let kept = (self.kept)();
        let file = kept.most_file.map_or(MOST_FILE, |most| most.min(MOST_FILE));
        let drive = free_space(&self.inbox).map(|free| free.saturating_sub(ROOM_LEFT));
        let sending = match (kept.most_sending, drive) {
            (Some(kept), Some(drive)) => Some(kept.min(drive)),
            (kept, drive) => kept.or(drive),
        };
        (file, sending)
    }

    /// What this PC says of itself now: called as what it keeps names it, where that is a name (it may be renamed as it
    /// runs: told so from then on), else as its computer is.
    pub fn me(&self) -> Info {
        let alias = (self.kept)().alias.filter(|alias| !alias.trim().is_empty());
        Info { alias: alias.unwrap_or_else(|| self.info.alias.clone()), ..self.info.clone() }
    }

    /// What this PC says of itself, as an answer to `register` and `info`.
    fn about(&self) -> String {
        let info = &self.me();
        json!({ "alias": info.alias, "version": VERSION, "deviceModel": info.device_model, "deviceType": info.device_type, "fingerprint": info.fingerprint, "download": false }).to_string()
    }

    fn asked(&self, method: &str, path: &str, head: &http::Head, body: &[u8], from: IpAddr, verified: Option<String>) -> Result<Option<(u16, String)>, Refused> {
        match (method, path) {
            ("GET", "/info") => Ok(Some((200, self.about()))),
            ("POST", "/register") => {
                let info: Info = serde_json::from_slice(body).map_err(|_| (400, "not a device's info"))?;
                self.heard(&info, from, verified.as_deref());
                Ok(Some((200, self.about())))
            }
            ("POST", "/prepare-upload") => self.prepare(body, from, verified).map(Some),
            // A sending cancelled by its device: the one going on (named, or the device's, unnamed: LocalSend's
            // earlier way), or, before any is, the asking about it.
            ("POST", "/cancel") => {
                let id = head.query("sessionId");
                let mut session = self.session.lock().unwrap();
                if session.as_ref().is_some_and(|session| id.is_none_or(|id| session.id == id) && session.from.address == from) {
                    let ended = session.take().unwrap();
                    drop(session);
                    self.end(ended, Some("cancelled by the sender".into()));
                } else if id.is_none() && *self.asking.lock().unwrap() == Some(from) {
                    drop(session);
                    self.told.withdraw();
                }
                Ok(None)
            }
            _ => Err((404, "no such thing")),
        }
    }

    /// A device heard of (it registered, or announced itself): known as it says, at the address it spoke from (where
    /// it presented a certificate, by that one's fingerprint, whatever it said).
    pub fn heard(&self, info: &Info, address: IpAddr, verified: Option<&str>) {
        let fingerprint = verified.map_or_else(|| info.fingerprint.to_ascii_uppercase(), str::to_string);
        if fingerprint.is_empty() || fingerprint.eq_ignore_ascii_case(&self.info.fingerprint) {
            return;
        }
        let device = Device {
            fingerprint: fingerprint.clone(),
            alias: shown_name(&info.alias),
            model: info.device_model.clone(),
            kind: info.device_type.clone(),
            address,
            port: info.port.unwrap_or(PORT),
            https: info.protocol.as_deref() != Some("http"),
            heard: Instant::now(),
        };
        let changed = {
            let mut devices = self.devices.lock().unwrap();
            let changed = devices.get(&fingerprint).is_none_or(|was| was.alias != device.alias || was.address != device.address || was.port != device.port);
            devices.insert(fingerprint, device);
            changed
        };
        if changed {
            self.told.tell(Event::Devices { devices: self.devices_now() });
        }
    }

    /// The devices heard within `discover::FRESH`, newest first.
    pub fn devices_now(&self) -> Vec<Device> {
        let mut devices = self.devices.lock().unwrap();
        devices.retain(|_, device| device.heard.elapsed() < discover::FRESH);
        let mut now: Vec<Device> = devices.values().cloned().collect();
        now.sort_by_key(|device| std::cmp::Reverse(device.heard));
        now
    }

    /// A sending offered (`prepare-upload`): taken (trusted, or the user says so), a receiving begun and each file's
    /// token given; else why not. A message (LocalSend's way of sending words: one text "file" whose words are given
    /// whole as its preview) is taken as words: nothing of it is uploaded (answered 204), and it is written down for
    /// Glance at once (see `message`).
    fn prepare(&self, body: &[u8], from: IpAddr, verified: Option<String>) -> Result<(u16, String), Refused> {
        let asked: Value = serde_json::from_slice(body).map_err(|_| (400, "not a sending"))?;
        let info: Info = asked.get("info").cloned().and_then(|info| serde_json::from_value(info).ok()).ok_or((400, "no info of the sender"))?;
        let offered = asked.get("files").and_then(Value::as_object).ok_or((400, "no files"))?;
        if offered.len() > MOST_FILES {
            return Err((400, "too many files"));
        }
        let kept = (self.kept)();
        let (most_file, most_sending) = self.limits();
        let message = match (offered.len(), offered.values().next()) {
            (1, Some(file)) if file.get("fileType").and_then(Value::as_str).is_some_and(|kind| kind.starts_with("text/")) => file.get("preview").and_then(Value::as_str).filter(|words| !words.trim().is_empty()).map(str::to_string),
            _ => None,
        };
        let mut files = Vec::new();
        for (id, file) in offered {
            let size = file.get("size").and_then(Value::as_u64).ok_or((400, "a file with no size"))?;
            if size > most_file {
                return Err((403, "a file too large"));
            }
            let name = shown_path(file.get("fileName").and_then(Value::as_str).unwrap_or_default());
            let kind = file.get("fileType").and_then(Value::as_str).unwrap_or("application/octet-stream").chars().take(100).collect();
            let sha256 = file.get("sha256").and_then(Value::as_str).filter(|hash| hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit())).map(str::to_ascii_lowercase);
            let words = message.as_ref().map(|words| words.chars().take(WORDS_SHOWN).collect());
            files.push((Offered { id: id.chars().take(200).collect(), name, size, kind, words }, sha256));
        }
        if files.is_empty() {
            return Ok((204, String::new()));
        }
        let total = files.iter().try_fold(0u64, |total, (file, _)| total.checked_add(file.size)).ok_or((400, "sizes beyond counting"))?;
        if most_sending.is_some_and(|most| total > most) {
            return Err((403, "no room for them"));
        }
        let sender = Sender {
            alias: shown_name(&info.alias),
            model: info.device_model.clone(),
            kind: info.device_type.clone(),
            fingerprint: verified.clone().unwrap_or_else(|| info.fingerprint.to_ascii_uppercase()),
            verified: verified.is_some(),
            address: from,
        };
        if sender.fingerprint.is_empty() {
            return Err((400, "no fingerprint"));
        }
        // One receiving at a time (another, or one being asked about, turns this away): the sender may try again. (A
        // message is no receiving: it comes whole with its offer.)
        if message.is_none() {
            let mut session = self.session.lock().unwrap();
            if session.as_ref().is_some_and(|going| going.touched.elapsed() > IDLE) {
                let stale = session.take().unwrap();
                drop(session);
                self.end(stale, Some("nothing came for too long".into()));
            } else if session.is_some() {
                return Err((409, "another sending is being received"));
            }
        }
        if !kept.trusts(&sender) {
            if kept.others_refused {
                return Err((403, "not taken from devices not trusted"));
            }
            {
                let mut asking = self.asking.lock().unwrap();
                if asking.is_some() {
                    return Err((409, "another device is being asked about"));
                }
                *asking = Some(from);
            }
            let answer = self.told.ask(&Asked { from: sender.clone(), files: files.iter().map(|(file, _)| file.clone()).collect() });
            *self.asking.lock().unwrap() = None;
            let answer = answer.unwrap_or_default();
            crate::journal::note(format!("nearby: {} ({}) offered {} files: {}", sender.alias, sender.address, files.len(), if answer.take { "taken" } else { "refused" }));
            if !answer.take {
                return Err((403, "refused"));
            }
            if answer.trust && sender.verified {
                (self.keep)(&|kept: &mut Kept| kept.trust(&sender));
            }
        } else {
            (self.keep)(&|kept: &mut Kept| kept.trust(&sender));
        }
        if let Some(words) = message {
            self.message(sender, words);
            return Ok((204, String::new()));
        }
        let id = random(16);
        let mut tokens = serde_json::Map::new();
        let mut incoming = HashMap::new();
        for (offered, sha256) in files {
            let token = random(16);
            tokens.insert(offered.id.clone(), Value::String(token.clone()));
            incoming.insert(offered.id.clone(), Incoming { offered, sha256, token, done: 0, came: None, writing: false });
        }
        let session = Session { id: id.clone(), from: sender.clone(), files: incoming, touched: Instant::now(), told: Instant::now() };
        let offered: Vec<Offered> = session.files.values().map(|file| file.offered.clone()).collect();
        {
            let mut going = self.session.lock().unwrap();
            if going.is_some() {
                return Err((409, "another sending is being received"));
            }
            *going = Some(session);
        }
        self.told.tell(Event::Coming { session: id.clone(), from: sender, files: offered });
        Ok((200, json!({ "sessionId": id, "files": tokens }).to_string()))
    }

    /// One file of the receiving going on (`upload`), as its body comes: written into the inbox, checked to be as
    /// long as it was said to be (and, where its hash was said, to be that), and kept by a name of this PC's.
    fn upload(&self, head: &http::Head, body: &mut impl Read, from: IpAddr) -> Result<(), Refused> {
        let (id, file_id, token) = (head.query("sessionId").unwrap_or_default(), head.query("fileId").unwrap_or_default(), head.query("token").unwrap_or_default());
        let (size, sha256, extension_of) = {
            let mut session = self.session.lock().unwrap();
            let session = session.as_mut().filter(|session| session.id == id).ok_or((403, "no such receiving"))?;
            if session.from.address != from {
                return Err((403, "not from where the sending was offered"));
            }
            let file = session.files.get_mut(file_id).filter(|file| file.token == token).ok_or((403, "no such file"))?;
            if file.writing || file.came.is_some() {
                return Err((409, "that file is coming already"));
            }
            file.writing = true;
            session.touched = Instant::now();
            (file.offered.size, file.sha256.clone(), extension(&file.offered.name))
        };
        let written = self.written(body, size, sha256.as_deref(), &extension_of, id, file_id);
        let mut session = self.session.lock().unwrap();
        let Some(going) = session.as_mut().filter(|session| session.id == id) else {
            // (Cancelled meanwhile: what came goes.)
            if let Ok(name) = written {
                let _ = std::fs::remove_file(self.inbox.join(name));
            }
            return Err((403, "the receiving was cancelled"));
        };
        let file = going.files.get_mut(file_id).expect("the file is the session's");
        file.writing = false;
        match written {
            Ok(name) => {
                file.came = Some(name);
                going.touched = Instant::now();
                if going.whole() {
                    let ended = session.take().unwrap();
                    drop(session);
                    self.end(ended, None);
                }
                Ok(())
            }
            Err(refused) => {
                file.done = 0;
                Err(refused)
            }
        }
    }

    /// A file's body written into the inbox as `<random>.part`, and, whole, named `<random>.<extension>` and marked as
    /// from the Internet: that name. (Not whole, or not as its hash said, it is gone.)
    fn written(&self, body: &mut impl Read, size: u64, sha256: Option<&str>, extension: &str, session: &str, file_id: &str) -> Result<String, Refused> {
        std::fs::create_dir_all(&self.inbox).map_err(|_| (500, "the inbox cannot be made"))?;
        let stem = random(12);
        let part = self.inbox.join(format!("{stem}.part"));
        let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(&part).map_err(|_| (500, "the file cannot be written"))?;
        let gone = |part: &Path| {
            let _ = std::fs::remove_file(part);
        };
        // Marked as from the Internet before anything of it is written (the mark goes with it as it is renamed): not
        // marked, not kept.
        let mut zone = part.as_os_str().to_owned();
        zone.push(":Zone.Identifier");
        if std::fs::write(PathBuf::from(zone), b"[ZoneTransfer]\r\nZoneId=3\r\n").is_err() {
            drop(out);
            gone(&part);
            return Err((500, "the file cannot be marked"));
        }
        let mut hash = Sha256::new();
        let mut chunk = vec![0u8; 256 << 10];
        let mut done = 0u64;
        loop {
            let read = match body.read(&mut chunk) {
                Ok(read) => read,
                Err(_) => {
                    drop(out);
                    gone(&part);
                    return Err((400, "the file did not come whole"));
                }
            };
            if read == 0 {
                break;
            }
            done += read as u64;
            if done > size {
                drop(out);
                gone(&part);
                return Err((400, "more came than was said"));
            }
            hash.update(&chunk[..read]);
            if out.write_all(&chunk[..read]).is_err() {
                drop(out);
                gone(&part);
                return Err((500, "the file cannot be written"));
            }
            self.progressed(session, file_id, done);
        }
        drop(out);
        if done != size {
            gone(&part);
            return Err((400, "less came than was said"));
        }
        if let Some(said) = sha256 {
            let made: String = hash.finalize().iter().map(|byte| format!("{byte:02x}")).collect();
            if made != said {
                gone(&part);
                return Err((422, "not what its hash says"));
            }
        }
        let name = if extension.is_empty() { stem } else { format!("{stem}.{extension}") };
        let whole = self.inbox.join(&name);
        if std::fs::rename(&part, &whole).is_err() {
            gone(&part);
            return Err((500, "the file cannot be kept"));
        }
        Ok(name)
    }

    /// How far a file has come: kept, and Glance told of the receiving's progress (at most every `PROGRESS_EVERY`).
    fn progressed(&self, session: &str, file_id: &str, done: u64) {
        let mut going = self.session.lock().unwrap();
        let Some(going) = going.as_mut().filter(|going| going.id == session) else { return };
        if let Some(file) = going.files.get_mut(file_id) {
            file.done = done;
        }
        going.touched = Instant::now();
        if going.told.elapsed() >= PROGRESS_EVERY {
            going.told = Instant::now();
            self.told.tell(Event::Progress { session: session.to_string(), done: going.done(), of: going.total() });
        }
    }

    /// A receiving over (`short`: why before all of it came): Glance told what came whole.
    fn end(&self, session: Session, short: Option<String>) {
        let mut came: Vec<Came> = session.files.into_values().filter_map(|file| file.came.map(|name| Came { file: name, name: file.offered.name, size: file.offered.size, kind: file.offered.kind })).collect();
        came.sort_by(|a, b| a.name.cmp(&b.name));
        crate::journal::note(format!("nearby: from {} ({}): {} files came{}", session.from.alias, session.from.address, came.len(), short.as_deref().map(|why| format!(", then {why}")).unwrap_or_default()));
        // Written down before Glance is told: what came is Glance's to take in, told or not (it may not be there, or
        // go before it has kept it).
        if !came.is_empty() {
            let receipt = Receipt { session: session.id.clone(), from: session.from.clone(), files: came.clone(), words: Vec::new(), at: seconds() };
            if let Err(error) = receipt.write(&self.receipts) {
                crate::journal::note(format!("nearby: the receipt of {} cannot be written ({error})", session.id));
            }
        }
        self.told.tell(Event::Came { session: session.id, from: session.from, files: came, short });
    }

    /// The words `from` sent as a message, taken: written down for Glance as a receiving of their own (with no files),
    /// and Glance told so.
    fn message(&self, from: Sender, words: String) {
        let id = random(16);
        crate::journal::note(format!("nearby: from {} ({}): words came", from.alias, from.address));
        let receipt = Receipt { session: id.clone(), from: from.clone(), files: Vec::new(), words: vec![words], at: seconds() };
        if let Err(error) = receipt.write(&self.receipts) {
            crate::journal::note(format!("nearby: the receipt of {id} cannot be written ({error})"));
        }
        self.told.tell(Event::Came { session: id, from, files: Vec::new(), short: None });
    }

    /// A receiving nothing has come for in `IDLE`, given up (what came of it is told): looked at now and then.
    pub fn expire(&self) {
        let mut session = self.session.lock().unwrap();
        if session.as_ref().is_some_and(|going| going.touched.elapsed() > IDLE && going.files.values().all(|file| !file.writing)) {
            let stale = session.take().unwrap();
            drop(session);
            self.end(stale, Some("nothing came for too long".into()));
        }
    }

    /// The folder what comes is written into.
    #[cfg(test)]
    pub fn inbox(&self) -> &Path {
        &self.inbox
    }
}

/// A receiving that ended with files come, or words (a message: see `Receiver::message`), written down for Glance
/// (see `Receiver::end`): Glance takes each file in once (by the session and the file), and deletes the receipt once
/// what it took in is kept.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub session: String,
    pub from: Sender,
    pub files: Vec<Came>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<String>,
    pub at: u64,
}

impl Receipt {
    /// Written whole into `folder` (as `<session>.json`), or not at all.
    fn write(&self, folder: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(folder)?;
        let part = folder.join(format!("{}.json.part", self.session));
        std::fs::write(&part, serde_json::to_vec(self).expect("a receipt is written"))?;
        std::fs::rename(&part, folder.join(format!("{}.json", self.session)))
    }

    /// The folder receipts are written into, beside `inbox`.
    pub fn folder(inbox: &Path) -> PathBuf {
        inbox.with_file_name("receipts")
    }
}

/// The bytes free to the user on the drive of `folder` (its nearest folder that is there).
fn free_space(folder: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    let mut at = folder;
    while !at.exists() {
        at = at.parent()?;
    }
    let wide: Vec<u16> = at.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free = 0u64;
    unsafe { windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(windows::core::PCWSTR(wide.as_ptr()), Some(&mut free), None, None) }.ok()?;
    Some(free)
}

#[cfg(test)]
mod tests;
