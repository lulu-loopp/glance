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
pub mod page;
pub mod run;
pub mod send;
pub mod tls;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
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
/// The ports this PC answers on, the first free of them (`OWN_PORT` first): the firewall lets devices on other
/// networks reach these alone (see `phone::open_firewall` in Glance), not the phones' port.
pub const OWN_PORTS: std::ops::RangeInclusive<u16> = OWN_PORT..=OWN_PORT + 9;
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
    /// It is Glance's (a PC's or a phone's), as it says beside what LocalSend's devices say (which they pass over).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub glance: bool,
    /// Where else it is reached, as Glance's devices say beside the rest (each network's address and the port): a
    /// device added by its address is reached at another once that one no longer answers (see `Added`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub addresses: Vec<SocketAddr>,
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
    /// It said it is Glance's (see `Info::glance`).
    pub glance: bool,
    #[serde(skip)]
    pub heard: Instant,
    /// It proved it holds the certificate of its fingerprint where it was heard (it presented it, or was reached
    /// pinned to it): what anyone only says of it then moves it nowhere (see `Receiver::heard`).
    #[serde(skip)]
    pub proven: bool,
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
    /// It said it is Glance's (see `Info::glance`).
    #[serde(default)]
    pub glance: bool,
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
    /// The devices seen lately, as they are now, and those added by their addresses (found or not).
    Devices { devices: Vec<Device>, added: Vec<Added> },
    /// Where other devices reach this PC: its port, and each network's address.
    Me { port: u16, addresses: Vec<discover::Address> },
    /// A device asked to be added by its address (see `link::Command::Add`): how that went.
    Added { job: u64, outcome: Adding },
    /// A sending of this PC's: how it goes.
    Sending { job: u64, state: send::State },
    /// A device found nearby that Glance asked to trust (see `link::Command::Trust`): trusted, or why not.
    Trusted { fingerprint: String, why_not: Option<String> },
}

/// How a device asked to be added by its address went (see `run::Running::add`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Adding {
    /// Added, as the device of `fingerprint`: `known` where it was already, found nearby or added before.
    Added { fingerprint: String, known: bool },
    /// What was given is no address.
    NotAnAddress,
    /// Nothing answered there.
    Unreachable,
    /// Something answered, but not as a LocalSend device does.
    NotLocalSend,
    /// It is this PC.
    Itself,
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
    /// Sending is wanted while receiving is not on: the helper runs nearby transfer for it, and every sending offered
    /// to this PC is refused.
    pub send: bool,
    /// What this PC was called to others before it had one name (`Config::name`): read once, to become that name, and
    /// not written again.
    #[serde(skip_serializing)]
    pub alias: Option<String>,
    /// What this PC is called, to others as to phones (`Config::name`, put here by `run::kept_of`: not kept here).
    #[serde(skip)]
    pub name: String,
    pub trusted: Vec<Trusted>,
    /// Devices not trusted are turned away, not asked about.
    pub others_refused: bool,
    /// The devices added by their addresses (see `Added`).
    pub added: Vec<Added>,
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
    /// It said it is Glance's (see `Info::glance`), as it was trusted.
    pub glance: bool,
}

/// A device added by its address, where it is not found nearby (on another network, as across a campus): known by its
/// certificate (as it proved holding it when it was added, or as it said, where it speaks plain HTTP), told of this PC
/// now and then (see `run::Running::reach_added`), and reached at the addresses it was added by or told, the one that
/// last answered first.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Added {
    pub fingerprint: String,
    pub alias: String,
    pub model: Option<String>,
    pub kind: Option<String>,
    pub glance: bool,
    pub https: bool,
    pub addresses: Vec<SocketAddr>,
    /// When it was added (seconds since 1970).
    pub since: u64,
}

/// The most addresses a device added is kept with.
const MOST_ADDRESSES: usize = 8;

impl Added {
    /// Reached at `first` (it answered there), and told at `others` too: `first` first, then the others, then those
    /// known before, each once, `MOST_ADDRESSES` at most. Whether that changed what is kept.
    pub fn reached_at(&mut self, first: SocketAddr, others: &[SocketAddr]) -> bool {
        let mut addresses = vec![first];
        for address in others.iter().chain(&self.addresses) {
            if !addresses.contains(address) {
                addresses.push(*address);
            }
        }
        addresses.truncate(MOST_ADDRESSES);
        let changed = addresses != self.addresses;
        self.addresses = addresses;
        changed
    }
}

impl Kept {
    /// Whether what `sender` sends is taken without asking: it presented the certificate of a device trusted.
    pub fn trusts(&self, sender: &Sender) -> bool {
        sender.verified && self.trusted.iter().any(|trusted| trusted.fingerprint.eq_ignore_ascii_case(&sender.fingerprint))
    }

    /// `sender` trusted from now on, where it presented its certificate (else it cannot be: nothing changes); trusted
    /// already, its name and when it last sent kept.
    pub fn trust(&mut self, sender: &Sender) {
        if sender.verified {
            self.trust_proven(&sender.fingerprint, &sender.alias, sender.kind.clone(), sender.glance, true);
        }
    }

    /// The device that proved it holds the certificate of `fingerprint` (it presented it as it sent, or as this PC
    /// reached it pinned to it), called `alias`, of `kind`, Glance's or not (`glance`), trusted from now on. Trusted
    /// already, its name kept, and, where it `sent`, when it last did.
    pub fn trust_proven(&mut self, fingerprint: &str, alias: &str, kind: Option<String>, glance: bool, sent: bool) {
        let now = seconds();
        match self.trusted.iter_mut().find(|trusted| trusted.fingerprint.eq_ignore_ascii_case(fingerprint)) {
            Some(trusted) => {
                trusted.alias = alias.to_string();
                trusted.glance |= glance;
                if sent {
                    trusted.last = now;
                }
            }
            None => self.trusted.push(Trusted { fingerprint: fingerprint.to_ascii_uppercase(), alias: alias.to_string(), kind, last: if sent { now } else { 0 }, since: now, phone: None, glance }),
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

/// Who a caller is counted as, of those answered at once (see `AT_ONCE_EACH`): its address, or over IPv6 its network's
/// (the first 64 bits), for a device is given all the addresses of one.
pub fn counted_as(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(v6) if v6.to_ipv4_mapped().is_none() => IpAddr::V6(std::net::Ipv6Addr::from(u128::from(v6) & !((1u128 << 64) - 1))),
        other => other,
    }
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
    /// Where it is written as it comes in pieces, each asked of its own (a phone's browser sends so: see `page`).
    pieces: Option<Writing>,
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
    /// The device whose sending the user is being asked about (one at a time), by its address and its fingerprint.
    asking: Mutex<Option<(IpAddr, String)>>,
    /// The devices heard lately (see `discover`).
    pub devices: Mutex<HashMap<String, Device>>,
    /// Where other devices reach this PC, network by network (see `discover::addresses`), as last looked at.
    addresses: Mutex<Vec<discover::Address>>,
    callers: Mutex<HashMap<IpAddr, usize>>,
    /// The pages of phones paired by the browser that are open, and what is offered them (see `page`).
    pub pages: page::Pages,
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
        Ok(Receiver { own, info, tls, inbox, receipts, told, kept: Box::new(kept), keep: Box::new(keep), session: Mutex::new(None), asking: Mutex::new(None), devices: Mutex::default(), addresses: Mutex::default(), callers: Mutex::default(), pages: page::Pages::default() })
    }

    /// Serves what comes to `listener`, a thread to each, for as long as it listens.
    pub fn serve(self: &Arc<Self>, listener: TcpListener) {
        for stream in listener.incoming().flatten() {
            // (Who calls, as its address is: one over IPv4 reaches a listener of both kinds as an IPv6 address that maps
            // it, and is the same device as one heard over IPv4.)
            let Ok(from) = stream.peer_addr().map(|peer| peer.ip().to_canonical()) else { continue };
            let counted = counted_as(from);
            {
                let mut callers = self.callers.lock().unwrap();
                if callers.values().sum::<usize>() >= AT_ONCE || callers.get(&counted).copied().unwrap_or(0) >= AT_ONCE_EACH {
                    continue;
                }
                *callers.entry(counted).or_default() += 1;
            }
            let receiver = self.clone();
            std::thread::spawn(move || {
                if let Err(error) = receiver.connection(stream, from) {
                    if error.kind() != std::io::ErrorKind::UnexpectedEof && error.kind() != std::io::ErrorKind::ConnectionReset {
                        crate::journal::note(format!("nearby: {from}: {error}"));
                    }
                }
                let mut callers = receiver.callers.lock().unwrap();
                if let Some(count) = callers.get_mut(&counted) {
                    *count -= 1;
                    if *count == 0 {
                        callers.remove(&counted);
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

    /// What this PC says of itself now: called by its name (`Kept::name`), where that is one (it may be renamed as it
    /// runs: told so from then on), else as it was started.
    pub fn me(&self) -> Info {
        let name = (self.kept)().name;
        let port = self.info.port.unwrap_or(PORT);
        let addresses = self.addresses.lock().unwrap().iter().map(|address| SocketAddr::new(address.address, port)).collect();
        Info { alias: if name.trim().is_empty() { self.info.alias.clone() } else { name }, addresses, ..self.info.clone() }
    }

    /// What this PC says of itself, as an answer to `register` and `info`.
    fn about(&self) -> String {
        let info = &self.me();
        json!({ "alias": info.alias, "version": VERSION, "deviceModel": info.device_model, "deviceType": info.device_type, "fingerprint": info.fingerprint, "download": false, "glance": true, "addresses": info.addresses }).to_string()
    }

    /// Where other devices reach this PC, as looked at now (`discover::addresses`): kept, and Glance told where it
    /// changed.
    pub fn addressed(&self, addresses: Vec<discover::Address>) {
        let changed = {
            let mut kept = self.addresses.lock().unwrap();
            let changed = *kept != addresses;
            *kept = addresses;
            changed
        };
        if changed {
            self.tell_me();
        }
    }

    /// Glance told where other devices reach this PC.
    pub fn tell_me(&self) {
        let addresses = self.addresses.lock().unwrap().clone();
        self.told.tell(Event::Me { port: self.info.port.unwrap_or(PORT), addresses });
    }

    /// Glance told the devices seen lately, and those added by their addresses.
    pub fn tell_devices(&self) {
        // (And the pages open, which may be sent to: see `page`.)
        let devices = [self.devices_now(), self.pages.open().0].concat();
        self.told.tell(Event::Devices { devices, added: self.added() });
    }

    /// The devices added by their addresses (see `Added`), as kept now.
    pub fn added(&self) -> Vec<Added> {
        (self.kept)().added
    }

    /// What is kept of the devices added by their addresses, changed by `change` (Glance told where it changed).
    pub fn change_added(&self, change: &(dyn Fn(&mut Vec<Added>) -> bool + Sync)) {
        let changed = std::sync::atomic::AtomicBool::new(false);
        (self.keep)(&|kept| {
            if change(&mut kept.added) {
                changed.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        });
        if changed.load(std::sync::atomic::Ordering::SeqCst) {
            self.tell_devices();
        }
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
            // earlier way), or, before any is, the asking about it. (Never a page's, which is known by no address: it
            // cancels its own, signed, see `Receiver::cancelled_by`.)
            ("POST", "/cancel") => {
                let id = head.query("sessionId");
                let ours = |address: IpAddr, fingerprint: &str| address == from && page::phone_of(fingerprint).is_none();
                let mut session = self.session.lock().unwrap();
                if session.as_ref().is_some_and(|session| id.is_none_or(|id| session.id == id) && ours(session.from.address, &session.from.fingerprint)) {
                    let ended = session.take().unwrap();
                    drop(session);
                    self.end(ended, Some("cancelled by the sender".into()));
                } else if id.is_none() && self.asking.lock().unwrap().as_ref().is_some_and(|(address, fingerprint)| ours(*address, fingerprint)) {
                    drop(session);
                    self.told.withdraw();
                }
                Ok(None)
            }
            _ => Err((404, "no such thing")),
        }
    }

    /// A device heard of (it registered, or announced itself): known as it says, at the address it spoke from (where
    /// it presented a certificate, by that one's fingerprint, whatever it said). A device known by its certificate (one
    /// added by its address over HTTPS, or one heard proven lately) is moved only by what proves it: what anyone only
    /// says of it, from elsewhere or as speaking otherwise, is not taken (files sent where it was said to be would go to
    /// whoever said so), and said from where it was proven, as it spoke there, keeps it heard.
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
            glance: info.glance,
            heard: Instant::now(),
            proven: verified.is_some(),
        };
        let held_added = || self.added().iter().any(|added| added.https && added.fingerprint.eq_ignore_ascii_case(&fingerprint));
        let changed = {
            let mut devices = self.devices.lock().unwrap();
            let was = devices.get(&fingerprint).filter(|was| was.heard.elapsed() < discover::FRESH);
            let mut device = device.clone();
            if !device.proven {
                let there = was.is_some_and(|was| was.proven && was.address == device.address && was.port == device.port && was.https == device.https);
                if !there && (was.is_some_and(|was| was.proven) || held_added()) {
                    return;
                }
                device.proven = there;
            }
            let changed = was.is_none_or(|was| was.alias != device.alias || was.address != device.address || was.port != device.port);
            devices.insert(fingerprint.clone(), device);
            changed
        };
        // A device added by its address that proved itself (a certificate presented, or reached pinned to it): kept as
        // reached where it spoke from, and at the addresses it tells of, its name as it says now.
        if verified.is_some() && self.added().iter().any(|added| added.fingerprint.eq_ignore_ascii_case(&fingerprint)) {
            let at = SocketAddr::new(address, device.port);
            self.change_added(&|added| {
                let Some(added) = added.iter_mut().find(|added| added.fingerprint.eq_ignore_ascii_case(&fingerprint)) else { return false };
                let named = added.alias != device.alias || added.model != device.model;
                added.alias = device.alias.clone();
                added.model = device.model.clone();
                added.reached_at(at, &info.addresses) || named
            });
        }
        if changed {
            self.tell_devices();
        }
    }

    /// The devices not heard within `discover::FRESH` let go, Glance told where any were (they are listed no longer,
    /// a device added shown as not answering): looked at now and then.
    pub fn let_go_quiet(&self) {
        let gone = {
            let mut devices = self.devices.lock().unwrap();
            let before = devices.len();
            devices.retain(|_, device| device.heard.elapsed() < discover::FRESH);
            devices.len() != before
        };
        // (A page closed is gone too.)
        let closed = self.pages.open().1;
        if gone || closed {
            self.tell_devices();
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
        // (Running to send alone, this PC takes nothing.)
        if !kept.on {
            return Err((403, "not receiving"));
        }
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
            glance: info.glance,
        };
        if sender.fingerprint.is_empty() {
            return Err((400, "no fingerprint"));
        }
        if message.is_none() {
            self.room_for_one()?;
        }
        if !kept.trusts(&sender) {
            let answer = self.asked_about(&sender, &files.iter().map(|(file, _)| file.clone()).collect::<Vec<_>>(), &kept)?;
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
        let (id, tokens) = self.begin(sender, files)?;
        Ok((200, json!({ "sessionId": id, "files": tokens }).to_string()))
    }

    /// Room for a receiving: one at a time (another, or one being asked about, turns this away: the sender may try
    /// again), one nothing came for in `IDLE` given up first. (A message is no receiving: it comes whole with its offer.)
    fn room_for_one(&self) -> Result<(), Refused> {
        let mut session = self.session.lock().unwrap();
        if session.as_ref().is_some_and(|going| going.touched.elapsed() > IDLE) {
            let stale = session.take().unwrap();
            drop(session);
            self.end(stale, Some("nothing came for too long".into()));
        } else if session.is_some() {
            return Err((409, "another sending is being received"));
        }
        Ok(())
    }

    /// The user asked whether to take what `sender` offers (`files`), one device at a time, where devices not trusted
    /// are asked about at all (`kept`): taken, with whether to trust it; else why not.
    fn asked_about(&self, sender: &Sender, files: &[Offered], kept: &Kept) -> Result<Answer, Refused> {
        if kept.others_refused {
            return Err((403, "not taken from devices not trusted"));
        }
        {
            let mut asking = self.asking.lock().unwrap();
            if asking.is_some() {
                return Err((409, "another device is being asked about"));
            }
            *asking = Some((sender.address, sender.fingerprint.clone()));
        }
        let answer = self.told.ask(&Asked { from: sender.clone(), files: files.to_vec() });
        *self.asking.lock().unwrap() = None;
        let answer = answer.unwrap_or_default();
        crate::journal::note(format!("nearby: {} ({}) offered {} files: {}", sender.alias, sender.address, files.len(), if answer.take { "taken" } else { "refused" }));
        if !answer.take {
            return Err((403, "refused"));
        }
        Ok(answer)
    }

    /// A receiving begun of `files` from `sender` (Glance told it is coming): its id, and each file's token by its id.
    fn begin(&self, sender: Sender, files: Vec<(Offered, Option<String>)>) -> Result<(String, serde_json::Map<String, Value>), Refused> {
        let id = random(16);
        let mut tokens = serde_json::Map::new();
        let mut incoming = HashMap::new();
        for (offered, sha256) in files {
            let token = random(16);
            tokens.insert(offered.id.clone(), Value::String(token.clone()));
            incoming.insert(offered.id.clone(), Incoming { offered, sha256, token, done: 0, came: None, writing: false, pieces: None });
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
        Ok((id, tokens))
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
        let mut writing = Writing::begin(&self.inbox)?;
        let mut chunk = vec![0u8; 256 << 10];
        loop {
            let read = match body.read(&mut chunk) {
                Ok(read) => read,
                Err(_) => {
                    writing.gone();
                    return Err((400, "the file did not come whole"));
                }
            };
            if read == 0 {
                break;
            }
            if let Err(refused) = writing.add(&chunk[..read], size) {
                writing.gone();
                return Err(refused);
            }
            self.progressed(session, file_id, writing.done);
        }
        writing.whole(size, sha256, extension, &self.inbox)
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
        let mut came = Vec::new();
        for file in session.files.into_values() {
            // (What came of a file in pieces before it ended goes.)
            if let Some(pieces) = file.pieces {
                pieces.gone();
            }
            if let Some(name) = file.came {
                came.push(Came { file: name, name: file.offered.name, size: file.offered.size, kind: file.offered.kind });
            }
        }
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

/// A file being written into the inbox as it comes: as `<random>.part`, marked as from the Internet before anything of
/// it is written (the mark goes with it as it is renamed: not marked, not kept), its bytes counted and hashed.
struct Writing {
    part: PathBuf,
    stem: String,
    out: std::fs::File,
    done: u64,
    hash: Sha256,
}

impl Writing {
    fn begin(inbox: &Path) -> Result<Writing, Refused> {
        std::fs::create_dir_all(inbox).map_err(|_| (500, "the inbox cannot be made"))?;
        let stem = random(12);
        let part = inbox.join(format!("{stem}.part"));
        let out = std::fs::OpenOptions::new().write(true).create_new(true).open(&part).map_err(|_| (500, "the file cannot be written"))?;
        let mut zone = part.as_os_str().to_owned();
        zone.push(":Zone.Identifier");
        if std::fs::write(PathBuf::from(zone), b"[ZoneTransfer]\r\nZoneId=3\r\n").is_err() {
            drop(out);
            let _ = std::fs::remove_file(&part);
            return Err((500, "the file cannot be marked"));
        }
        Ok(Writing { part, stem, out, done: 0, hash: Sha256::new() })
    }

    /// `bytes` written after what came, of a file said to be `size` long.
    fn add(&mut self, bytes: &[u8], size: u64) -> Result<(), Refused> {
        if self.done + bytes.len() as u64 > size {
            return Err((400, "more came than was said"));
        }
        self.hash.update(bytes);
        self.out.write_all(bytes).map_err(|_| (500, "the file cannot be written"))?;
        self.done += bytes.len() as u64;
        Ok(())
    }

    /// Kept, where all `size` bytes came (and, where its hash was said, as `sha256` says), as `<random>.<extension>`:
    /// that name. Else gone.
    fn whole(self, size: u64, sha256: Option<&str>, extension: &str, inbox: &Path) -> Result<String, Refused> {
        let Writing { part, stem, out, done, hash } = self;
        drop(out);
        let gone = || {
            let _ = std::fs::remove_file(&part);
        };
        if done != size {
            gone();
            return Err((400, "less came than was said"));
        }
        if let Some(said) = sha256 {
            let made: String = hash.finalize().iter().map(|byte| format!("{byte:02x}")).collect();
            if made != said {
                gone();
                return Err((422, "not what its hash says"));
            }
        }
        let name = if extension.is_empty() { stem } else { format!("{stem}.{extension}") };
        if std::fs::rename(&part, inbox.join(&name)).is_err() {
            gone();
            return Err((500, "the file cannot be kept"));
        }
        Ok(name)
    }

    /// Given up: what was written goes.
    fn gone(self) {
        drop(self.out);
        let _ = std::fs::remove_file(&self.part);
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
