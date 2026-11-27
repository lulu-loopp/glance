//! A page's sending and fetching (see `page`), against a receiver of this PC's loopback.

use super::*;

/// A page's phone, as it sends (see `page`): known by its phone's id, its certificate none.
fn page_sender(phone: &str) -> Sender {
    Sender { alias: "iPhone".into(), model: None, kind: Some("mobile".into()), fingerprint: page::fingerprint(phone), verified: false, address: IpAddr::from([127, 0, 0, 1]), glance: true }
}

fn receipts(both: &Both) -> Vec<Receipt> {
    std::fs::read_dir(Receipt::folder(both.receiver.inbox())).unwrap().flatten().map(|entry| serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap()).collect()
}

/// How many times Glance was told that a sending came.
fn told_came(heard: &Heard) -> usize {
    heard.events.lock().unwrap().iter().filter(|event| matches!(event, Event::Came { .. })).count()
}

/// What a page sends comes in pieces, each asked of its own, into the inbox as any device's does: whole, marked as
/// from the Internet, told to Glance. Its phone's files taken without asking, it is not asked about. A piece out of
/// its place is refused, an empty one too (but an empty file's), one sent again (its answer lost) taken as come, the
/// last as well once the receiving ended (told once), one from another page refused.
#[test]
fn a_page_sends_in_pieces() {
    let both = both("page-pieces", None, Kept { on: true, ..Kept::default() });
    let photo: Vec<u8> = (0..(page::PIECE + 1000)).map(|at| (at % 251) as u8).collect();
    let files = [("IMG_0001.HEIC".to_string(), photo.len() as u64, "image/heic".to_string()), ("note.txt".to_string(), 5, "text/plain".to_string()), ("empty".to_string(), 0, "".to_string())];
    let (session, ids) = both.receiver.offered_by_page("o1", page_sender("p1"), &files, None, true).unwrap().expect("a receiving begun");
    assert!(both.heard.asked.lock().unwrap().is_empty(), "taken without asking");
    let from = page::fingerprint("p1");
    let (first, rest) = photo.split_at(page::PIECE);
    assert_eq!(both.receiver.piece(&session, &ids[0], &from, page::PIECE as u64, rest), Err((409, "not the piece that comes next")));
    assert_eq!(both.receiver.piece(&session, &ids[0], &from, 0, b""), Err((400, "an empty piece")));
    assert_eq!(both.receiver.piece(&session, &ids[0], &page::fingerprint("another"), 0, first).unwrap_err().0, 403);
    both.receiver.piece(&session, &ids[0], &from, 0, first).unwrap();
    both.receiver.piece(&session, &ids[0], &from, 0, first).unwrap();
    both.receiver.piece(&session, &ids[0], &from, page::PIECE as u64, rest).unwrap();
    both.receiver.piece(&session, &ids[2], &from, 0, b"").unwrap();
    assert!(came(&both.heard).is_none(), "one file still to come");
    both.receiver.piece(&session, &ids[1], &from, 0, b"words").unwrap();
    let (sender, came, short) = came(&both.heard).expect("told what came");
    assert_eq!(sender.fingerprint, from);
    assert!(short.is_none());
    assert_eq!(came.len(), 3);
    let photo_came = came.iter().find(|file| file.name == "IMG_0001.HEIC").expect("the photo came");
    let path = both.receiver.inbox().join(&photo_came.file);
    assert_eq!(std::fs::read(&path).unwrap(), photo);
    let mut zone = path.into_os_string();
    zone.push(":Zone.Identifier");
    assert!(std::fs::read_to_string(PathBuf::from(zone)).unwrap().contains("ZoneId=3"));
    both.receiver.piece(&session, &ids[1], &from, 0, b"words").expect("the last piece sent again, its answer lost");
    assert_eq!(told_came(&both.heard), 1, "told once");
    assert_eq!(both.receiver.piece(&session, &ids[1], &page::fingerprint("another"), 0, b"words").unwrap_err().0, 403, "another page's");
    assert_eq!(both.receiver.piece("other", &ids[1], &from, 0, b"words").unwrap_err().0, 403, "no such receiving");
}

/// An offer sent again (its answer lost) is answered as it was: the same receiving, not asked about twice, words not
/// given twice. Another offer is an offer of its own.
#[test]
fn an_offer_sent_again_is_answered_as_it_was() {
    let both = both("page-again", Some(Answer { take: true, trust: false }), Kept { on: true, ..Kept::default() });
    let files = [("a.jpg".to_string(), 3, "image/jpeg".to_string())];
    let first = both.receiver.offered_by_page("o1", page_sender("p5"), &files, None, false).unwrap();
    assert_eq!(both.receiver.offered_by_page("o1", page_sender("p5"), &files, None, false).unwrap(), first);
    assert_eq!(both.heard.asked.lock().unwrap().len(), 1, "asked once");
    assert_eq!(both.receiver.offered_by_page("o2", page_sender("p5"), &files, None, false).unwrap_err().0, 409, "another offer, while one is received");
    let (session, ids) = first.expect("begun");
    both.receiver.piece(&session, &ids[0], &page::fingerprint("p5"), 0, b"abc").unwrap();
    assert_eq!(both.receiver.offered_by_page("w1", page_sender("p5"), &[], Some("hello".into()), true), Ok(None));
    assert_eq!(both.receiver.offered_by_page("w1", page_sender("p5"), &[], Some("hello".into()), true), Ok(None));
    assert_eq!(receipts(&both).iter().filter(|receipt| receipt.words == ["hello"]).count(), 1, "the words given once");
}

/// A page whose phone's files are not taken without asking is asked about, never trusted by the answer (it has no
/// certificate to trust). Refused, nothing begins. What it gave up of a file goes.
#[test]
fn a_page_is_asked_about_and_what_it_gives_up_goes() {
    let both = both("page-asked", Some(Answer { take: true, trust: true }), Kept { on: true, ..Kept::default() });
    let files = [("big.mov".to_string(), 10, "video/quicktime".to_string())];
    let (session, ids) = both.receiver.offered_by_page("o1", page_sender("p2"), &files, None, false).unwrap().expect("taken");
    assert_eq!(both.heard.asked.lock().unwrap().len(), 1);
    assert!(!both.kept.lock().unwrap().trusts(&page_sender("p2")), "a page is not trusted");
    both.receiver.piece(&session, &ids[0], &page::fingerprint("p2"), 0, b"12345").unwrap();
    both.receiver.cancelled_by(&session, &page::fingerprint("p2"));
    let (_, came, short) = came(&both.heard).expect("told it ended");
    assert!(came.is_empty());
    assert_eq!(short.as_deref(), Some("cancelled by the sender"));
    assert!(std::fs::read_dir(both.receiver.inbox()).unwrap().flatten().all(|entry| entry.path() == Receipt::folder(both.receiver.inbox())), "the piece written went");
    let refusing = super::both("page-refused", Some(Answer { take: false, trust: false }), Kept { on: true, ..Kept::default() });
    assert_eq!(refusing.receiver.offered_by_page("o1", page_sender("p3"), &files, None, false), Err((403, "refused")));
}

/// A page's receiving is cancelled by the page alone, signed: not by a LocalSend device's cancelling, unsigned, from
/// the same address (two phones behind one router's).
#[test]
fn a_page_s_receiving_is_not_cancelled_by_another_device() {
    let both = both("page-not-cancelled", None, Kept { on: true, ..Kept::default() });
    let files = [("a.jpg".to_string(), 3, "image/jpeg".to_string())];
    let (session, ids) = both.receiver.offered_by_page("o1", page_sender("p6"), &files, None, true).unwrap().expect("begun");
    assert_eq!(without_certificate(&both, "/cancel", b"").0, 200);
    assert_eq!(without_certificate(&both, &format!("/cancel?sessionId={session}"), b"").0, 200);
    both.receiver.piece(&session, &ids[0], &page::fingerprint("p6"), 0, b"abc").expect("still going");
    let (_, came, short) = came(&both.heard).expect("came");
    assert!(short.is_none());
    assert_eq!(came.len(), 1);
}

/// A page that gives up its offer while the user is asked about it takes it back at once: refused, nothing begun.
/// Another page cancelling, or a LocalSend device from the same address, takes back nothing of it.
#[test]
fn a_page_takes_back_its_offer_while_its_user_is_asked() {
    let both = Arc::new(thinking("page-take-back", Some(Answer { take: true, trust: false }), Duration::from_secs(10), Kept { on: true, ..Kept::default() }));
    let asking = both.clone();
    let began = Instant::now();
    let offering = std::thread::spawn(move || asking.receiver.offered_by_page("o1", page_sender("p7"), &[("a.jpg".to_string(), 3, "image/jpeg".to_string())], None, false));
    while both.receiver.asking.lock().unwrap().is_none() {
        std::thread::sleep(Duration::from_millis(20));
    }
    both.receiver.cancelled_by("", &page::fingerprint("another"));
    assert_eq!(without_certificate(&both, "/cancel", b"").0, 200);
    std::thread::sleep(Duration::from_millis(300));
    assert!(both.receiver.asking.lock().unwrap().is_some(), "still asked about");
    both.receiver.cancelled_by("", &page::fingerprint("p7"));
    assert_eq!(offering.join().unwrap(), Err((403, "refused")));
    assert!(began.elapsed() < Duration::from_secs(5), "taken back, not answered by its user");
}

/// Words a page sends come as words: nothing begun, nothing in the inbox.
#[test]
fn words_a_page_sends_come_as_words() {
    let both = both("page-words", None, Kept { on: true, ..Kept::default() });
    assert_eq!(both.receiver.offered_by_page("o1", page_sender("p4"), &[], Some("https://example.com".into()), true), Ok(None));
    let written = receipts(&both);
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].words, ["https://example.com"]);
    assert!(written[0].files.is_empty());
}

/// What this PC sends a page waits for the page to fetch it, piece by piece, words as files are: listed with how this
/// PC is, sent once the page says it has all (and no sooner, its last piece fetched again where the answer was lost).
/// A page not open fails at once, one offered another is busy, one turning it down refuses it, one cancelled gives no
/// more of it and is offered no more.
#[test]
fn a_page_fetches_what_this_pc_sends() {
    let both = both("page-fetch", None, Kept { on: true, ..Kept::default() });
    let pages = &both.receiver.pages;
    let heard = Arc::new(Heard::default());
    let told: Arc<dyn Told> = heard.clone();
    let states = |job: u64| -> Vec<send::State> {
        heard.events.lock().unwrap().iter().filter_map(|event| match event {
            Event::Sending { job: of, state } if *of == job => Some(state.clone()),
            _ => None,
        }).collect()
    };
    let bytes = vec![7u8; page::PIECE + 10];
    let paths = files("page-fetch", &[("photo.jpg", &bytes)]);
    pages.offer("closed", 1, &paths, Arc::new(send::Job::default()), told.clone());
    assert!(matches!(states(1).as_slice(), [send::State::Failed { .. }]));
    assert!(pages.seen("f1", "iPhone", IpAddr::from([127, 0, 0, 1])));
    assert!(!pages.seen("f1", "iPhone", IpAddr::from([127, 0, 0, 1])), "open already");
    assert!(pages.open().0.iter().any(|device| device.fingerprint == page::fingerprint("f1")));
    pages.offer("f1", 2, &paths, Arc::new(send::Job::default()), told.clone());
    pages.offer("f1", 3, &paths, Arc::new(send::Job::default()), told.clone());
    assert_eq!(states(3), [send::State::Busy]);
    let offered = pages.offered_to("f1").expect("offered");
    assert_eq!(offered["job"], 2);
    assert_eq!(offered["files"][0]["name"], "photo.jpg");
    assert_eq!(offered["files"][0]["size"], page::PIECE + 10);
    assert_eq!(offered["files"][0]["words"], false);
    assert_eq!(pages.piece_of("f1", 9, 0, 0), Err((404, "no such sending")));
    assert_eq!(pages.piece_of("f1", 2, 0, 0).unwrap().len(), page::PIECE);
    assert_eq!(pages.done("f1", 2), Err((409, "not all of it fetched")));
    assert_eq!(pages.piece_of("f1", 2, 0, page::PIECE as u64).unwrap(), [7u8; 10]);
    assert_eq!(pages.piece_of("f1", 2, 0, page::PIECE as u64).unwrap(), [7u8; 10], "fetched again, its answer lost");
    assert_ne!(states(2).last(), Some(&send::State::Sent), "not sent till the page says it has all");
    pages.done("f1", 2).unwrap();
    pages.done("f1", 2).expect("said again, its answer lost");
    assert_eq!(states(2).first(), Some(&send::State::Waiting));
    assert_eq!(states(2).last(), Some(&send::State::Sent));
    assert_eq!(states(2).iter().filter(|state| **state == send::State::Sent).count(), 1);
    assert!(pages.offered_to("f1").is_none(), "nothing more offered");
    // Words are fetched as a file is: offered till the page has them.
    pages.offer("f1", 4, &[link::Outgoing::Words { words: "héllo".into(), name: "text.txt".into() }], Arc::new(send::Job::default()), told.clone());
    for _ in 0..2 {
        let offered = pages.offered_to("f1").expect("offered till the page has them");
        assert_eq!(offered["files"][0]["words"], true);
        assert_eq!(offered["files"][0]["size"], "héllo".len());
    }
    assert_eq!(pages.piece_of("f1", 4, 0, 0).unwrap(), "héllo".as_bytes());
    pages.done("f1", 4).unwrap();
    assert_eq!(states(4).last(), Some(&send::State::Sent));
    pages.offer("f1", 5, &paths, Arc::new(send::Job::default()), told.clone());
    pages.refused("f1", 5);
    assert_eq!(states(5).last(), Some(&send::State::Refused));
    let going = Arc::new(send::Job::default());
    pages.offer("f1", 6, &paths, going.clone(), told.clone());
    going.cancel();
    assert_eq!(pages.piece_of("f1", 6, 0, 0), Err((410, "the sending was cancelled")));
    pages.expire();
    assert_eq!(states(6).last(), Some(&send::State::Cancelled));
    // Cancelled before the page asks again: not offered it, cancelled then.
    let going = Arc::new(send::Job::default());
    pages.offer("f1", 7, &[link::Outgoing::Words { words: "hush".into(), name: "text.txt".into() }], going.clone(), told.clone());
    going.cancel();
    assert!(pages.offered_to("f1").is_none());
    assert_eq!(states(7).last(), Some(&send::State::Cancelled));
}

/// Callers are counted by their addresses, over IPv6 by their networks': a device holds every address of its /64.
#[test]
fn callers_over_ipv6_are_counted_by_their_network() {
    let one: IpAddr = "2601:40f:4401:1d9e:e908:2a7a:d4b9:7d04".parse().unwrap();
    let other: IpAddr = "2601:40f:4401:1d9e::1".parse().unwrap();
    let elsewhere: IpAddr = "2601:40f:4401:1d9f::1".parse().unwrap();
    assert_eq!(counted_as(one), counted_as(other));
    assert_ne!(counted_as(one), counted_as(elsewhere));
    assert_eq!(counted_as(IpAddr::from([10, 0, 0, 5])), IpAddr::from([10, 0, 0, 5]));
}
