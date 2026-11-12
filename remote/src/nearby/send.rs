//! This PC as a sender, and as one device telling another of itself: LocalSend's client side. Every connection to a
//! device that speaks HTTPS is pinned to the fingerprint it announced, and presents this PC's certificate.

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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
    /// It asks for a PIN, which this PC does not give.
    Pin,
    Cancelled,
    Failed { why: String },
}

/// Something both read and written: a connection, over TLS or not.
pub trait Stream: Read + Write {}
impl<S: Read + Write> Stream for S {}

/// A connection to `address`:`port`, over TLS pinned to `fingerprint` where `https`.
pub fn connect(own: &tls::Own, address: IpAddr, port: u16, https: bool, fingerprint: &str, quiet: Duration) -> std::io::Result<Box<dyn Stream + Send>> {
    let socket = TcpStream::connect_timeout(&SocketAddr::new(address, port), REACH)?;
    socket.set_read_timeout(Some(quiet))?;
    socket.set_write_timeout(Some(quiet))?;
    if !https {
        return Ok(Box::new(socket));
    }
    let config = own.client(fingerprint).map_err(std::io::Error::other)?;
    let tls = rustls::ClientConnection::new(config, rustls::pki_types::ServerName::IpAddress(address.into())).map_err(std::io::Error::other)?;
    Ok(Box::new(rustls::StreamOwned::new(tls, socket)))
}

/// Asks `stream` `method` `path` (the API's) of `host`, with `body` (JSON): the answer's status and body.
fn asked(mut stream: Box<dyn Stream + Send>, method: &str, path: &str, host: &str, body: &[u8]) -> std::io::Result<(u16, Vec<u8>)> {
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

/// Sends `files` to `device` as this PC (`own`, telling of itself as `me`), Glance told how it goes as sending `job`;
/// given up where `cancelled` is set meanwhile.
pub fn send(own: &tls::Own, me: &Info, device: &Device, files: &[Outgoing], job: u64, told: &Arc<dyn Told>, cancelled: &AtomicBool) {
    let tell = |state: State| told.tell(Event::Sending { job, state });
    let state = match sending(own, me, device, files, cancelled, &|state| tell(state)) {
        Ok(state) => state,
        Err(error) => State::Failed { why: error.to_string() },
    };
    crate::journal::note(format!("nearby: sent {} files to {} ({}): {state:?}", files.len(), device.alias, device.address));
    tell(state);
}

fn sending(own: &tls::Own, me: &Info, device: &Device, outgoing: &[Outgoing], cancelled: &AtomicBool, tell: &dyn Fn(State)) -> std::io::Result<State> {
    let host = device.address.to_string();
    // What each file is: its id (its place), its name, how large it is.
    let mut files = Vec::new();
    for (at, file) in outgoing.iter().enumerate() {
        let size = std::fs::metadata(&file.path)?.len();
        files.push((format!("f{at}"), file.name.clone(), size, &file.path));
    }
    let named: serde_json::Map<String, Value> = files.iter().map(|(id, name, size, _)| (id.clone(), json!({ "id": id, "fileName": name, "size": size, "fileType": kind_of(name), "sha256": null, "preview": null }))).collect();
    tell(State::Waiting);
    let stream = connect(own, device.address, device.port, device.https, &device.fingerprint, OFFER)?;
    let (status, body) = asked(stream, "POST", "/prepare-upload", &host, json!({ "info": mine(me), "files": named }).to_string().as_bytes())?;
    match status {
        200 => {}
        204 => return Ok(State::Sent),
        401 => return Ok(State::Pin),
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
            if cancelled.load(Ordering::SeqCst) {
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
