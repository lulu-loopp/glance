//! A receiver and a sender, both this module's, on this PC's loopback, over TLS: what a LocalSend device sees of
//! Glance, and Glance of it.

use super::*;
use std::sync::Condvar;

/// What Glance would be told, and how it answers when asked: at once, or after the user thought `thinks` (none, where
/// the asking was withdrawn meanwhile).
#[derive(Default)]
struct Heard {
    events: Mutex<Vec<Event>>,
    asked: Mutex<Vec<Asked>>,
    answer: Option<Answer>,
    thinks: Duration,
    withdrawn: (Mutex<bool>, Condvar),
}

impl Told for Heard {
    fn ask(&self, asked: &Asked) -> Option<Answer> {
        self.asked.lock().unwrap().push(asked.clone());
        let (withdrawn, told) = &self.withdrawn;
        let (withdrawn, _) = told.wait_timeout_while(withdrawn.lock().unwrap(), self.thinks, |withdrawn| !*withdrawn).unwrap();
        if *withdrawn { None } else { self.answer }
    }

    fn withdraw(&self) {
        *self.withdrawn.0.lock().unwrap() = true;
        self.withdrawn.1.notify_all();
    }

    fn tell(&self, event: Event) {
        self.events.lock().unwrap().push(event);
    }
}

fn folder(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("glance-nearby-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    folder
}

struct Both {
    receiver: Arc<Receiver>,
    heard: Arc<Heard>,
    kept: Arc<Mutex<Kept>>,
    sender: tls::Own,
    me: Info,
    device: Device,
}

/// A receiver answering `answer` when it asks, serving on the loopback; a sender with a certificate of its own.
fn both(name: &str, answer: Option<Answer>, kept: Kept) -> Both {
    thinking(name, answer, Duration::ZERO, kept)
}

/// The same, its user answering after thinking `thinks`.
fn thinking(name: &str, answer: Option<Answer>, thinks: Duration, kept: Kept) -> Both {
    let dir = folder(name);
    let heard = Arc::new(Heard { answer, thinks, ..Heard::default() });
    let kept = Arc::new(Mutex::new(kept));
    let (reading, keeping) = (kept.clone(), kept.clone());
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let info = Info { alias: "Glance PC".into(), device_model: Some("Windows".into()), device_type: Some("desktop".into()), port: Some(port), protocol: Some("https".into()), ..Info::default() };
    let told: Arc<dyn Told> = heard.clone();
    let receiver = Arc::new(Receiver::new(tls::Own::in_folder(&dir.join("receiver")).unwrap(), info, dir.join("inbox"), told, move || reading.lock().unwrap().clone(), move |change| change(&mut keeping.lock().unwrap())).unwrap());
    let serving = receiver.clone();
    std::thread::spawn(move || serving.serve(listener));
    let sender = tls::Own::in_folder(&dir.join("sender")).unwrap();
    let me = Info { alias: "Galaxy Z Fold6".into(), device_model: Some("Samsung".into()), device_type: Some("mobile".into()), fingerprint: sender.fingerprint.clone(), port: Some(1), ..Info::default() };
    let device = Device { fingerprint: receiver.own.fingerprint.clone(), alias: "Glance PC".into(), model: None, kind: None, address: IpAddr::from([127, 0, 0, 1]), port, https: true, heard: Instant::now() };
    Both { receiver, heard, kept, sender, me, device }
}

/// Files named `names`, each of its own bytes, in a folder of the test's.
/// Files written for sending, each named as it is told (a folder's file by its place in the folder).
fn files(name: &str, contents: &[(&str, &[u8])]) -> Vec<link::Outgoing> {
    let dir = folder(&format!("{name}-files"));
    std::fs::create_dir_all(&dir).unwrap();
    contents.iter().enumerate().map(|(at, (file, bytes))| {
        let path = dir.join(format!("{at}"));
        std::fs::write(&path, bytes).unwrap();
        link::Outgoing { path, name: file.to_string() }
    }).collect()
}

fn sent(both: &Both, files: &[link::Outgoing]) -> Vec<send::State> {
    sent_as(both, files, &send::Job::default())
}

/// What a sending of `files`, as `job`, was told to Glance as, state by state.
fn sent_as(both: &Both, files: &[link::Outgoing], job: &send::Job) -> Vec<send::State> {
    let states = Arc::new(Heard::default());
    let told: Arc<dyn Told> = states.clone();
    send::send(&both.sender, &both.me, &both.device, files, 7, &told, job);
    let events = states.events.lock().unwrap();
    events.iter().map(|event| match event {
        Event::Sending { job: 7, state } => state.clone(),
        other => panic!("not a sending's state: {other:?}"),
    }).collect()
}

/// What came, as Glance was told at the end.
fn came(heard: &Heard) -> Option<(Sender, Vec<Came>, Option<String>)> {
    heard.events.lock().unwrap().iter().find_map(|event| match event {
        Event::Came { from, files, short, .. } => Some((from.clone(), files.clone(), short.clone())),
        _ => None,
    })
}

/// A device not trusted is asked about; taken, its files come into the inbox under names of this PC's, whole and
/// marked as from the Internet, Glance told of them by the names they came with; trusted then (by the certificate it
/// presented), it is not asked about again.
#[test]
fn a_sending_asked_about_and_taken_comes_whole() {
    let both = both("taken", Some(Answer { take: true, trust: true }), Kept { on: true, ..Kept::default() });
    // (One of them sent from a folder, named by its place there: it comes by its own name.)
    let paths = files("taken", &[("IMG_20261112_143012.jpg", b"a photo's bytes"), ("trip/notes.txt", b"words")]);
    let states = sent(&both, &paths);
    assert_eq!(states.first(), Some(&send::State::Waiting));
    assert_eq!(states.last(), Some(&send::State::Sent));
    let asked = both.heard.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].from.alias, "Galaxy Z Fold6");
    assert!(asked[0].from.verified, "it presented its certificate");
    assert_eq!(asked[0].from.fingerprint, both.sender.fingerprint);
    let (from, files, short) = came(&both.heard).expect("told what came");
    assert_eq!(from.fingerprint, both.sender.fingerprint);
    assert!(short.is_none());
    let mut names: Vec<&str> = files.iter().map(|file| file.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["IMG_20261112_143012.jpg", "notes.txt"]);
    for file in &files {
        assert!(!file.file.contains(['/', '\\']) && file.file != file.name, "named by this PC");
        let path = both.receiver.inbox().join(&file.file);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes, if file.name == "notes.txt" { &b"words"[..] } else { &b"a photo's bytes"[..] });
        let zone = std::fs::read_to_string(format!("{}:Zone.Identifier", path.display())).unwrap();
        assert!(zone.contains("ZoneId=3"));
    }
    assert!(files.iter().any(|file| file.file.ends_with(".jpg")));
    assert!(std::fs::read_dir(both.receiver.inbox()).unwrap().flatten().all(|entry| !entry.file_name().to_string_lossy().ends_with(".part")));
    let kept = both.kept.lock().unwrap().clone();
    assert_eq!(kept.trusted.len(), 1);
    assert_eq!(kept.trusted[0].fingerprint, both.sender.fingerprint);
    // Written down for Glance, whatever it was told.
    let receipts: Vec<Receipt> = std::fs::read_dir(Receipt::folder(both.receiver.inbox())).unwrap().flatten().map(|entry| serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap()).collect();
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].files, files);
    // Trusted now: sent again, not asked about.
    assert_eq!(sent(&both, &paths).last(), Some(&send::State::Sent));
    assert_eq!(both.heard.asked.lock().unwrap().len(), 1);
}

/// Turned down (or not answered at all: no Glance), the sender is told it was refused, and nothing comes.
#[test]
fn a_sending_refused_comes_to_nothing() {
    for answer in [Some(Answer { take: false, trust: false }), None] {
        let both = both("refused", answer, Kept { on: true, ..Kept::default() });
        let paths = files("refused", &[("a.jpg", b"x")]);
        assert_eq!(sent(&both, &paths).last(), Some(&send::State::Refused));
        assert!(came(&both.heard).is_none());
        assert!(both.kept.lock().unwrap().trusted.is_empty());
    }
    // Others turned away as the user chose: not even asked.
    let both = both("refused-all", Some(Answer { take: true, trust: true }), Kept { on: true, others_refused: true, ..Kept::default() });
    assert_eq!(sent(&both, &files("refused-all", &[("a.jpg", b"x")])).last(), Some(&send::State::Refused));
    assert!(both.heard.asked.lock().unwrap().is_empty());
}

/// A plain request on a TLS connection that presents no certificate: what a receiver answers, by its status and body.
fn without_certificate(both: &Both, path: &str, body: &[u8]) -> (u16, Vec<u8>) {
    posted(both, path, body, false)
}

/// The same, presenting the sender's certificate where `presenting`.
fn posted(both: &Both, path: &str, body: &[u8], presenting: bool) -> (u16, Vec<u8>) {
    let config = if presenting { both.sender.client(&both.receiver.own.fingerprint).unwrap() } else { both.sender.client_anonymous(&both.receiver.own.fingerprint) };
    let socket = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, both.device.port)).unwrap();
    let tls = rustls::ClientConnection::new(config, rustls::pki_types::ServerName::IpAddress(both.device.address.into())).unwrap();
    let mut stream = rustls::StreamOwned::new(tls, socket);
    http::ask(&mut stream, "POST", &format!("{API}{path}"), "127.0.0.1", body.len() as u64, true).unwrap();
    stream.write_all(body).unwrap();
    stream.flush().unwrap();
    let mut head = http::head(&mut stream, true).unwrap().unwrap();
    let answer = http::Body::of(&mut head, &mut stream, true).unwrap().whole(1 << 16).unwrap();
    (head.status, answer)
}

/// A device is trusted only by the certificate it presents: one that only says a trusted device's fingerprint
/// (presenting none) is asked about, and, taken with "trust" asked for, is trusted no more for that (it cannot be).
#[test]
fn a_fingerprint_only_said_is_never_trusted() {
    let both = both("claimed", Some(Answer { take: true, trust: true }), Kept::default());
    let mut sender = Sender { alias: "Fold".into(), model: None, kind: None, fingerprint: both.sender.fingerprint.clone(), verified: true, address: IpAddr::from([127, 0, 0, 1]) };
    both.kept.lock().unwrap().trust(&sender);
    let offer = json!({ "info": { "alias": "Fold", "version": "2.1", "fingerprint": both.sender.fingerprint, "deviceType": "mobile" }, "files": { "a": { "id": "a", "fileName": "a.jpg", "size": 1, "fileType": "image/jpeg" } } }).to_string();
    let (status, body) = without_certificate(&both, "/prepare-upload", offer.as_bytes());
    assert_eq!(status, 200, "asked about, and taken");
    assert_eq!(both.heard.asked.lock().unwrap().len(), 1);
    assert!(!both.heard.asked.lock().unwrap()[0].from.verified);
    let answer: Value = serde_json::from_slice(&body).unwrap();
    assert!(answer["sessionId"].is_string() && answer["files"]["a"].is_string());
    // Not trusted by what it only said, though "trust" was answered.
    sender.verified = false;
    let mut kept = Kept::default();
    kept.trust(&sender);
    assert!(kept.trusted.is_empty());
    assert!(!kept.trusts(&sender));
}

/// A file not as its hash said is refused (422) and not kept; one with a token not its own, or for a receiving that is
/// not, is refused (403); a second offer while one is being received waits its turn (409).
#[test]
fn files_not_as_offered_are_refused() {
    let both = both("checked", Some(Answer { take: true, trust: false }), Kept::default());
    let wrong_hash = "0".repeat(64);
    let offer = json!({ "info": { "alias": "Fold", "fingerprint": both.sender.fingerprint }, "files": { "a": { "id": "a", "fileName": "a.jpg", "size": 5, "fileType": "image/jpeg", "sha256": wrong_hash }, "b": { "id": "b", "fileName": "b.jpg", "size": 3, "fileType": "image/jpeg" } } }).to_string();
    let (status, body) = without_certificate(&both, "/prepare-upload", offer.as_bytes());
    assert_eq!(status, 200);
    let answer: Value = serde_json::from_slice(&body).unwrap();
    let session = answer["sessionId"].as_str().unwrap();
    let token = answer["files"]["a"].as_str().unwrap();
    assert_eq!(without_certificate(&both, "/prepare-upload", offer.as_bytes()).0, 409);
    assert_eq!(without_certificate(&both, &format!("/upload?sessionId={session}&fileId=a&token=nope"), b"hello").0, 403);
    assert_eq!(without_certificate(&both, &format!("/upload?sessionId=other&fileId=a&token={token}"), b"hello").0, 403);
    assert_eq!(without_certificate(&both, &format!("/upload?sessionId={session}&fileId=a&token={token}"), b"hello").0, 422);
    assert_eq!(std::fs::read_dir(both.receiver.inbox()).unwrap().count(), 0, "nothing kept of it");
    // Cancelled: over, Glance told nothing came.
    assert_eq!(without_certificate(&both, &format!("/cancel?sessionId={session}"), b"").0, 200);
    let (_, files, short) = came(&both.heard).unwrap();
    assert!(files.is_empty() && short.is_some());
}

/// The most a sending may be is what the station keeps room for (the drive leaving room too), and what Glance's app is
/// told: an offer of that much is taken, one byte more is refused up front (403), as is a file beyond the most a file
/// may be.
#[test]
fn offers_are_taken_within_the_limits_told() {
    let both = both("limits", None, Kept { most_file: Some(6), most_sending: Some(10), ..Kept::default() });
    assert_eq!(both.receiver.limits(), (6, Some(10)));
    let offer = |sizes: &[u64]| {
        let files: serde_json::Map<String, Value> = sizes.iter().enumerate().map(|(at, size)| (at.to_string(), json!({ "id": at.to_string(), "fileName": "a.jpg", "size": size, "fileType": "image/jpeg" }))).collect();
        json!({ "info": { "alias": "Fold", "fingerprint": both.sender.fingerprint }, "files": files }).to_string()
    };
    // (Presenting no certificate, it is refused before any asking: the user answers nothing here.)
    assert_eq!(without_certificate(&both, "/prepare-upload", offer(&[6, 5]).as_bytes()).0, 403, "beyond the sending's most");
    assert_eq!(without_certificate(&both, "/prepare-upload", offer(&[7]).as_bytes()).0, 403, "beyond a file's most");
    assert!(both.heard.asked.lock().unwrap().is_empty(), "refused before its user was asked");
    // Within them, it goes on to be asked about (its user answers nothing: refused then, but asked).
    assert_eq!(without_certificate(&both, "/prepare-upload", offer(&[6, 4]).as_bytes()).0, 403);
    assert_eq!(both.heard.asked.lock().unwrap().len(), 1);
}

/// A device that registers is known (by the fingerprint it said, presenting no certificate), and told of this PC.
#[test]
fn a_device_registering_is_known_and_answered() {
    let both = both("register", None, Kept::default());
    let info = json!({ "alias": "iPhone 15", "version": "2.1", "deviceModel": "iPhone", "deviceType": "mobile", "fingerprint": "abc123", "port": 53317, "protocol": "https", "download": false }).to_string();
    let (status, body) = without_certificate(&both, "/register", info.as_bytes());
    assert_eq!(status, 200);
    let answer: Info = serde_json::from_slice(&body).unwrap();
    assert_eq!(answer.alias, "Glance PC");
    assert_eq!(answer.fingerprint, both.receiver.own.fingerprint);
    let devices = both.receiver.devices_now();
    assert_eq!(devices.len(), 1);
    assert_eq!((devices[0].alias.as_str(), devices[0].fingerprint.as_str(), devices[0].port), ("iPhone 15", "ABC123", 53317));
    // And this PC registering with another, as it does with one it hears announce itself: told of it back.
    let other = super::send::register(&both.sender, &both.me, IpAddr::from([127, 0, 0, 1]), &Info { port: Some(both.device.port), protocol: Some("https".into()), fingerprint: both.receiver.own.fingerprint.clone(), ..Info::default() }).unwrap();
    assert_eq!(other.alias, "Glance PC");
}

/// A sender talks only to the receiver of the fingerprint it was told: another certificate is not talked to.
#[test]
fn a_receiver_of_another_certificate_is_not_sent_to() {
    let both = both("pinned", Some(Answer { take: true, trust: false }), Kept::default());
    let mut device = both.device.clone();
    device.fingerprint = "00".repeat(32);
    let states = Arc::new(Heard::default());
    let told: Arc<dyn Told> = states.clone();
    send::send(&both.sender, &both.me, &device, &files("pinned", &[("a.jpg", b"x")]), 1, &told, &send::Job::default());
    let last = states.events.lock().unwrap().last().cloned();
    assert!(matches!(last, Some(Event::Sending { state: send::State::Failed { .. }, .. })), "{last:?}");
    assert!(both.heard.asked.lock().unwrap().is_empty());
}

/// Names a sending gives are shown without any folder of the sender's, nor what cannot stand in a name.
#[test]
fn names_given_are_only_names() {
    assert_eq!(shown_name("../../Windows/evil.exe"), "evil.exe");
    assert_eq!(shown_name("C:\\a\\b.txt"), "b.txt");
    assert_eq!(shown_name("con<>:|?*.jpg"), "con.jpg");
    assert_eq!(shown_name(".."), "file");
    assert_eq!(shown_name("  "), "file");
    assert_eq!(extension("a.JPG"), "jpg");
    assert_eq!(extension("a.tar.gz"), "gz");
    assert_eq!(extension("a.j p g"), "");
    assert_eq!(extension("noext"), "");
}

/// A sending cancelled while the device's user is asked about it ends at once, cancelled, and the device is told: its
/// asking is taken back from its user.
#[test]
fn a_sending_cancelled_while_its_user_is_asked_is_taken_back() {
    let both = thinking("withdrawn", Some(Answer { take: true, trust: false }), Duration::from_secs(20), Kept::default());
    let paths = files("withdrawn", &[("a.jpg", b"x")]);
    let job = Arc::new(send::Job::default());
    let cancelling = job.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        cancelling.cancel();
    });
    let began = Instant::now();
    let states = sent_as(&both, &paths, &job);
    assert_eq!(states.last(), Some(&send::State::Cancelled));
    assert!(began.elapsed() < Duration::from_secs(3), "ended as it was cancelled, not as its user answered");
    // (Told on a connection of its own: looked for a while.)
    let until = Instant::now() + Duration::from_secs(5);
    while !*both.heard.withdrawn.0.lock().unwrap() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(*both.heard.withdrawn.0.lock().unwrap(), "the asking taken back");
    assert!(came(&both.heard).is_none());
}

/// A device (on the loopback, by `own`'s certificate) that asks for the PIN "1234": an offer without it, or with
/// another, turned down at once (401); with it, nothing wanted (204).
fn asking_for_a_pin(own: &tls::Own) -> Device {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = own.server().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let config = config.clone();
            std::thread::spawn(move || {
                let mut stream = rustls::StreamOwned::new(rustls::ServerConnection::new(config).unwrap(), stream);
                let Ok(Some(mut head)) = http::head(&mut stream, false) else { return };
                let _ = http::Body::of(&mut head, &mut stream, false).and_then(|body| body.whole(1 << 20));
                let status = if head.query("pin") == Some("1234") { 204 } else { 401 };
                let _ = http::answer(&mut stream, status, None);
            });
        }
    });
    Device { fingerprint: own.fingerprint.clone(), alias: "iPhone 15".into(), model: None, kind: Some("mobile".into()), address: IpAddr::from([127, 0, 0, 1]), port, https: true, heard: Instant::now() }
}

/// Gives the PINs in `given`, one each time a PIN is asked for (cancels the sending where none is left), and keeps the
/// states told.
struct Giving {
    job: Arc<send::Job>,
    given: Mutex<Vec<&'static str>>,
    states: Mutex<Vec<send::State>>,
}

impl Told for Giving {
    fn ask(&self, _: &Asked) -> Option<Answer> {
        None
    }

    fn withdraw(&self) {}

    fn tell(&self, event: Event) {
        let Event::Sending { state, .. } = event else { return };
        if matches!(state, send::State::Pin { .. }) {
            let mut given = self.given.lock().unwrap();
            if given.is_empty() {
                self.job.cancel();
            } else {
                self.job.give_pin(given.remove(0).to_string());
            }
        }
        self.states.lock().unwrap().push(state);
    }
}

/// A device asking for a PIN is offered again with each one Glance gives: told how many it turned down, then sent
/// with the right one. Cancelled while one is waited for, it ends at once.
#[test]
fn a_pin_asked_for_is_given_and_tried_again() {
    let dir = folder("pin");
    let device = asking_for_a_pin(&tls::Own::in_folder(&dir.join("device")).unwrap());
    let own = tls::Own::in_folder(&dir.join("sender")).unwrap();
    let me = Info { alias: "Glance PC".into(), fingerprint: own.fingerprint.clone(), port: Some(1), ..Info::default() };
    let paths = files("pin", &[("a.jpg", b"x")]);
    let send_giving = |given: Vec<&'static str>| {
        let job = Arc::new(send::Job::default());
        let giving = Arc::new(Giving { job: job.clone(), given: Mutex::new(given), states: Mutex::default() });
        let told: Arc<dyn Told> = giving.clone();
        send::send(&own, &me, &device, &paths, 7, &told, &job);
        let states = giving.states.lock().unwrap().clone();
        states
    };
    assert_eq!(send_giving(vec!["0000", "1234"]), [send::State::Waiting, send::State::Pin { tried: 0 }, send::State::Pin { tried: 1 }, send::State::Sent]);
    let began = Instant::now();
    assert_eq!(send_giving(Vec::new()).last(), Some(&send::State::Cancelled));
    assert!(began.elapsed() < Duration::from_secs(3));
}

/// Words sent as a message (one text "file", its words as its preview) are taken as words: asked about as any sending
/// is (Glance shown their first words), answered 204 (nothing to upload), nothing written into the inbox, and written
/// down for Glance as a receiving of their own. From a device trusted they come while another sending is received too.
#[test]
fn words_sent_come_as_words() {
    let both = both("words", Some(Answer { take: true, trust: true }), Kept { on: true, ..Kept::default() });
    let words = "https://example.com/a?b=1";
    let offer = |words: &str| json!({ "info": { "alias": "Galaxy Z Fold6", "version": "2.1", "fingerprint": both.sender.fingerprint, "deviceType": "mobile" }, "files": { "m": { "id": "m", "fileName": "m.txt", "size": words.len(), "fileType": "text/plain", "preview": words } } }).to_string();
    let (status, _) = posted(&both, "/prepare-upload", offer(words).as_bytes(), true);
    assert_eq!(status, 204);
    let asked = both.heard.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].files[0].words.as_deref(), Some(words));
    let (from, files, short) = came(&both.heard).expect("told what came");
    assert_eq!(from.fingerprint, both.sender.fingerprint);
    assert!(files.is_empty() && short.is_none());
    assert!(std::fs::read_dir(both.receiver.inbox()).map_or(true, |mut entries| entries.next().is_none()), "nothing uploaded");
    let receipts = || -> Vec<Receipt> { std::fs::read_dir(Receipt::folder(both.receiver.inbox())).unwrap().flatten().map(|entry| serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap()).collect() };
    let written = receipts();
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].words, [words]);
    assert!(written[0].files.is_empty());
    // Trusted now: words sent while files are being received come too, unasked.
    let files_offer = json!({ "info": { "alias": "Galaxy Z Fold6", "version": "2.1", "fingerprint": both.sender.fingerprint, "deviceType": "mobile" }, "files": { "a": { "id": "a", "fileName": "a.jpg", "size": 3, "fileType": "image/jpeg" } } }).to_string();
    assert_eq!(posted(&both, "/prepare-upload", files_offer.as_bytes(), true).0, 200);
    assert_eq!(posted(&both, "/prepare-upload", offer("second").as_bytes(), true).0, 204);
    assert_eq!(both.heard.asked.lock().unwrap().len(), 1);
    assert_eq!(receipts().iter().filter(|receipt| receipt.words == ["second"]).count(), 1);
}
