//! This PC as a sender, and as one device telling another of itself: LocalSend's client side. Every connection to a
//! device that speaks HTTPS is pinned to the fingerprint it announced, and presents this PC's certificate.

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};

use super::link::Outgoing;
use super::{http, tls, Device, Event, Info, Told, API};

/// How long a device is given to be reached, and to answer what it is asked (but an offer, which waits on its user).
const REACH: Duration = Duration::from_secs(3);
const ANSWER: Duration = Duration::from_secs(10);
/// How long a device's user is given to take what is offered.
const OFFER: Duration = Duration::from_secs(120);
/// Glance is told of a sending's progress at most this often.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);
/// How often an offer waiting on the device's user looks whether it was cancelled meanwhile.
const LOOK_EVERY: Duration = Duration::from_millis(100);
/// An offer the device has not answered within this is held for its user to answer (a device answers at once what it
/// turns away by itself: a PIN not right, another sending going on).
const HELD: Duration = Duration::from_millis(1000);

/// How a sending of this PC's goes.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum State {
    /// Offered: the device's user is to take it.
    Waiting,
    Sending { done: u64, of: u64 },
    Sent,
    /// Its user turned it down.
    Refused,
    /// It is receiving another sending.
    Busy,
    /// It asks for a PIN (see `Job::give_pin`), and how many given it turned down so far.
    Pin { tried: u32 },
    Cancelled,
    Failed { why: String },
}

/// A sending going on, as Glance has a say in it: whether it is cancelled, and the PIN the device asked for, given.
#[derive(Default)]
pub struct Job {
    cancelled: AtomicBool,
    pin: Mutex<Option<String>>,
    given: Condvar,
}

impl Job {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        // (Held while told: one waiting for a PIN either saw it cancelled or is woken.)
        let _pin = self.pin.lock().unwrap();
        self.given.notify_all();
    }

    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// The PIN the device asked for, given (used once).
    pub fn give_pin(&self, pin: String) {
        *self.pin.lock().unwrap() = Some(pin);
        self.given.notify_all();
    }

    /// The PIN given, waited for up to `most`: none where it was cancelled meanwhile, or none was given.
    fn pin_given(&self, most: Duration) -> Option<String> {
        let until = Instant::now() + most;
        let mut pin = self.pin.lock().unwrap();
        loop {
            if self.cancelled() {
                return None;
            }
            if let Some(pin) = pin.take() {
                return Some(pin);
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            pin = self.given.wait_timeout(pin, left).unwrap().0;
        }
    }
}

/// Something both read and written: a connection, over TLS or not.
pub trait Stream: Read + Write {}
impl<S: Read + Write> Stream for S {}

/// A connection to `address`:`port`, over TLS pinned to `fingerprint` where `https`.
pub fn connect(own: &tls::Own, address: IpAddr, port: u16, https: bool, fingerprint: &str, quiet: Duration) -> std::io::Result<Box<dyn Stream + Send>> {
    connected(own, address, port, https, fingerprint, quiet).map(|(stream, _)| stream)
}

/// A connection as `connect` makes it, and its socket's handle, to end what it waits on from elsewhere (valid while
/// the connection is).
fn connected(own: &tls::Own, address: IpAddr, port: u16, https: bool, fingerprint: &str, quiet: Duration) -> std::io::Result<(Box<dyn Stream + Send>, RawSocket)> {
    use std::os::windows::io::AsRawSocket;
    let socket = TcpStream::connect_timeout(&SocketAddr::new(address, port), REACH)?;
    socket.set_read_timeout(Some(quiet))?;
    socket.set_write_timeout(Some(quiet))?;
    let raw = socket.as_raw_socket();
    if !https {
        return Ok((Box::new(socket), raw));
    }
    let config = own.client(fingerprint).map_err(std::io::Error::other)?;
    let tls = rustls::ClientConnection::new(config, rustls::pki_types::ServerName::IpAddress(address.into())).map_err(std::io::Error::other)?;
    Ok((Box::new(rustls::StreamOwned::new(tls, socket)), raw))
}

type RawSocket = std::os::windows::io::RawSocket;

/// Offers `body` to `device` at `path` (`prepare-upload`), waiting on its user up to `OFFER`: its answer. The
/// connection is ended where `job` is cancelled meanwhile (what the answer then is, it was cancelled); `held` told once
/// the device has held it `HELD` unanswered (its user is asked).
fn offered(own: &tls::Own, device: &Device, path: &str, body: &[u8], job: &Job, held: Option<&(dyn Fn() + Sync)>) -> std::io::Result<(u16, Vec<u8>)> {
    let (stream, socket) = connected(own, device.address, device.port, device.https, &device.fingerprint, OFFER)?;
    // (Whether it was answered, held while `held` is told: told first, or not at all.)
    let answered = Mutex::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let (since, mut held) = (Instant::now(), held);
            loop {
                {
                    let answered = answered.lock().unwrap();
                    if *answered {
                        return;
                    }
                    if job.cancelled() {
                        // (What its own thread waits on in it given up: shutting it down wakes no wait on Windows.)
                        unsafe {
                            let _ = windows::Win32::System::IO::CancelIoEx(windows::Win32::Foundation::HANDLE(socket as *mut std::ffi::c_void), None);
                        }
                        return;
                    }
                    if since.elapsed() >= HELD {
                        if let Some(held) = held.take() {
                            held();
                        }
                    }
                }
                std::thread::sleep(LOOK_EVERY);
            }
        });
        let answer = asked(stream, "POST", path, &device.address.to_string(), body);
        *answered.lock().unwrap() = true;
        answer
    })
}

/// Asks `stream` `method` `path` (the API's) of `host`, with `body` (JSON): the answer's status and body.
fn asked(mut stream: impl Read + Write, method: &str, path: &str, host: &str, body: &[u8]) -> std::io::Result<(u16, Vec<u8>)> {
    http::ask(&mut stream, method, &format!("{API}{path}"), host, body.len() as u64, true)?;
    stream.write_all(body)?;
    stream.flush()?;
    let mut head = http::head(&mut stream, true)?.ok_or_else(|| std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "no answer"))?;
    let answer = http::Body::of(&mut head, &mut stream, true)?.whole(1 << 20)?;
    Ok((head.status, answer))
}

/// This PC as it tells of itself to another (with its port and protocol, which an answer leaves out).
fn mine(me: &Info) -> Value {
    json!({ "alias": me.alias, "version": super::VERSION, "deviceModel": me.device_model, "deviceType": me.device_type, "fingerprint": me.fingerprint, "port": me.port, "protocol": "https", "download": false })
}

/// Tells the device at `address` (which announced `info`) of this PC (`register`): what it says of itself back.
pub fn register(own: &tls::Own, me: &Info, address: IpAddr, info: &Info) -> std::io::Result<Info> {
    let https = info.protocol.as_deref() != Some("http");
    let stream = connect(own, address, info.port.unwrap_or(super::PORT), https, &info.fingerprint, ANSWER)?;
    let (status, body) = asked(stream, "POST", "/register", &address.to_string(), mine(me).to_string().as_bytes())?;
    if status != 200 {
        return Err(std::io::Error::other(format!("register answered {status}")));
    }
    serde_json::from_slice(&body).map_err(std::io::Error::other)
}

/// The kind a file is, by its name, as LocalSend's devices are told (they sort and open by it).
pub fn kind_of(name: &str) -> &'static str {
    let extension = name.rsplit_once('.').map_or("", |(_, after)| after).to_ascii_lowercase();
    match extension.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "pdf" => "application/pdf",
        "txt" | "md" => "text/plain",
        "zip" => "application/zip",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/vnd.rar",
        "apk" => "application/vnd.android.package-archive",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
}

/// Sends `files` to `device` as this PC (`own`, telling of itself as `me`), Glance told how it goes as sending
/// `number`; given up where `job` is cancelled meanwhile, a PIN the device asks for taken from it (see `Job`).
pub fn send(own: &tls::Own, me: &Info, device: &Device, files: &[Outgoing], number: u64, told: &Arc<dyn Told>, job: &Job) {
    let tell = |state: State| told.tell(Event::Sending { job: number, state });
    let state = match sending(own, me, device, files, job, &tell) {
        Ok(state) => state,
        // (Its connection ended as it was cancelled: cancelled, however that ended it.)
        Err(_) if job.cancelled() => State::Cancelled,
        Err(error) => State::Failed { why: error.to_string() },
    };
    crate::journal::note(format!("nearby: sent {} files to {} ({}): {state:?}", files.len(), device.alias, device.address));
    tell(state);
}

fn sending(own: &tls::Own, me: &Info, device: &Device, outgoing: &[Outgoing], job: &Job, tell: &(dyn Fn(State) + Sync)) -> std::io::Result<State> {
    let host = device.address.to_string();
    // What each file is: its id (its place), its name, how large it is.
    let mut files = Vec::new();
    for (at, file) in outgoing.iter().enumerate() {
        let size = std::fs::metadata(&file.path)?.len();
        files.push((format!("f{at}"), file.name.clone(), size, &file.path));
    }
    let named: serde_json::Map<String, Value> = files.iter().map(|(id, name, size, _)| (id.clone(), json!({ "id": id, "fileName": name, "size": size, "fileType": kind_of(name), "sha256": null, "preview": null }))).collect();
    tell(State::Waiting);
    let offer = json!({ "info": mine(me), "files": named }).to_string();
    // Offered, and again with each PIN given while the device asks for one (it turns a PIN not right down at once:
    // offered with one, it is waiting on its user only once the device holds it).
    let (mut pin, mut tried) = (None::<String>, 0u32);
    let waiting = || tell(State::Waiting);
    let (status, body) = loop {
        let path = match &pin {
            Some(pin) => format!("/prepare-upload?pin={}", http::escaped(pin)),
            None => "/prepare-upload".to_string(),
        };
        let answer = offered(own, device, &path, offer.as_bytes(), job, pin.is_some().then_some(&waiting as &(dyn Fn() + Sync)));
        if job.cancelled() {
            // (The device told, so that it stops asking its user: no session of it is known yet.)
            let _ = connect(own, device.address, device.port, device.https, &device.fingerprint, ANSWER).and_then(|stream| asked(stream, "POST", "/cancel", &host, b""));
            return Ok(State::Cancelled);
        }
        let (status, body) = answer?;
        if status != 401 {
            break (status, body);
        }
        if pin.is_some() {
            tried += 1;
        }
        tell(State::Pin { tried });
        match job.pin_given(OFFER) {
            Some(given) => pin = Some(given),
            None if job.cancelled() => return Ok(State::Cancelled),
            None => return Ok(State::Failed { why: "no PIN was given".into() }),
        }
    };
    match status {
        200 => {}
        204 => return Ok(State::Sent),
        403 => return Ok(State::Refused),
        409 | 429 => return Ok(State::Busy),
        other => return Ok(State::Failed { why: format!("the offer was answered {other}") }),
    }
    let answer: Value = serde_json::from_slice(&body).map_err(std::io::Error::other)?;
    let session = answer.get("sessionId").and_then(Value::as_str).ok_or_else(|| std::io::Error::other("no session"))?.to_string();
    let tokens = answer.get("files").and_then(Value::as_object).cloned().unwrap_or_default();
    let of: u64 = files.iter().filter(|(id, ..)| tokens.contains_key(id)).map(|(_, _, size, _)| size).sum();
    let mut done = 0u64;
    let mut told = Instant::now();
    tell(State::Sending { done, of });
    for (id, _, size, path) in &files {
        // (A file the device did not want has no token: left out.)
        let Some(token) = tokens.get(id).and_then(Value::as_str) else { continue };
        let mut stream = connect(own, device.address, device.port, device.https, &device.fingerprint, ANSWER)?;
        let path_query = format!("{API}/upload?sessionId={}&fileId={}&token={}", http::escaped(&session), http::escaped(id), http::escaped(token));
        http::ask(&mut stream, "POST", &path_query, &host, *size, false)?;
        let mut from = std::fs::File::open(path)?;
        let mut chunk = vec![0u8; 256 << 10];
        loop {
            if job.cancelled() {
                drop(stream);
                let _ = connect(own, device.address, device.port, device.https, &device.fingerprint, ANSWER).and_then(|stream| asked(stream, "POST", &format!("/cancel?sessionId={}", http::escaped(&session)), &host, b""));
                return Ok(State::Cancelled);
            }
            let read = from.read(&mut chunk)?;
            if read == 0 {
                break;
            }
            stream.write_all(&chunk[..read])?;
            done += read as u64;
            if told.elapsed() >= PROGRESS_EVERY {
                told = Instant::now();
                tell(State::Sending { done, of });
            }
        }
        stream.flush()?;
        let mut head = http::head(&mut stream, true)?.ok_or_else(|| std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "no answer to a file"))?;
        let _ = http::Body::of(&mut head, &mut stream, true).and_then(|body| body.whole(1 << 16));
        if head.status != 200 {
            return Ok(State::Failed { why: format!("a file was answered {}", head.status) });
        }
    }
    Ok(State::Sent)
}
