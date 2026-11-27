//! A page's sending and fetching (see `page`), against a receiver of this PC's loopback.

use super::*;

/// A page's phone, as it sends (see `page`): known by its phone's id, its certificate none.
fn page_sender(phone: &str) -> Sender {
    Sender { alias: "iPhone".into(), model: None, kind: Some("mobile".into()), fingerprint: page::fingerprint(phone), verified: false, address: IpAddr::from([127, 0, 0, 1]), glance: true }
}

fn receipts(both: &Both) -> Vec<Receipt> {
    std::fs::read_dir(Receipt::folder(both.receiver.inbox())).unwrap().flatten().map(|entry| serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap()).collect()
}

/// What a page sends comes in pieces, each asked of its own, into the inbox as any device's does: whole, marked as
/// from the Internet, told to Glance. Its phone's files taken without asking, it is not asked about. A piece out of
/// its place is refused, one sent again (its answer lost) taken as come, one from another page refused.
#[test]
fn a_page_sends_in_pieces() {
    let both = both("page-pieces", None, Kept { on: true, ..Kept::default() });
    let photo: Vec<u8> = (0..(page::PIECE + 1000)).map(|at| (at % 251) as u8).collect();
    let files = [("IMG_0001.HEIC".to_string(), photo.len() as u64, "image/heic".to_string()), ("note.txt".to_string(), 5, "text/plain".to_string())];
    let (session, ids) = both.receiver.offered_by_page(page_sender("p1"), &files, None, true).unwrap().expect("a receiving begun");
    assert!(both.heard.asked.lock().unwrap().is_empty(), "taken without asking");
    let from = page::fingerprint("p1");
    let (first, rest) = photo.split_at(page::PIECE);
    assert_eq!(both.receiver.piece(&session, &ids[0], &from, page::PIECE as u64, rest), Err((409, "not the piece that comes next")));
    assert_eq!(both.receiver.piece(&session, &ids[0], &page::fingerprint("another"), 0, first).unwrap_err().0, 403);
    both.receiver.piece(&session, &ids[0], &from, 0, first).unwrap();
    both.receiver.piece(&session, &ids[0], &from, 0, first).unwrap();
    both.receiver.piece(&session, &ids[0], &from, page::PIECE as u64, rest).unwrap();
    assert!(came(&both.heard).is_none(), "one file still to come");
    both.receiver.piece(&session, &ids[1], &from, 0, b"words").unwrap();
    let (sender, came, short) = came(&both.heard).expect("told what came");
    assert_eq!(sender.fingerprint, from);
    assert!(short.is_none());
    let photo_came = came.iter().find(|file| file.name == "IMG_0001.HEIC").expect("the photo came");
    let path = both.receiver.inbox().join(&photo_came.file);
    assert_eq!(std::fs::read(&path).unwrap(), photo);
    let mut zone = path.into_os_string();
    zone.push(":Zone.Identifier");
    assert!(std::fs::read_to_string(PathBuf::from(zone)).unwrap().contains("ZoneId=3"));
    assert_eq!(both.receiver.piece(&session, &ids[1], &from, 0, b"words").unwrap_err().0, 403, "the receiving is over");
}

/// A page whose phone's files are not taken without asking is asked about, never trusted by the answer (it has no
/// certificate to trust). Refused, nothing begins. What it gave up of a file goes.
#[test]
fn a_page_is_asked_about_and_what_it_gives_up_goes() {
    let both = both("page-asked", Some(Answer { take: true, trust: true }), Kept { on: true, ..Kept::default() });
    let files = [("big.mov".to_string(), 10, "video/quicktime".to_string())];
    let (session, ids) = both.receiver.offered_by_page(page_sender("p2"), &files, None, false).unwrap().expect("taken");
    assert_eq!(both.heard.asked.lock().unwrap().len(), 1);
    assert!(!both.kept.lock().unwrap().trusts(&page_sender("p2")), "a page is not trusted");
    both.receiver.piece(&session, &ids[0], &page::fingerprint("p2"), 0, b"12345").unwrap();
    both.receiver.cancelled_by(&session, &page::fingerprint("p2"));
    let (_, came, short) = came(&both.heard).expect("told it ended");
    assert!(came.is_empty());
    assert_eq!(short.as_deref(), Some("cancelled by the sender"));
    assert!(std::fs::read_dir(both.receiver.inbox()).unwrap().flatten().all(|entry| entry.path() == Receipt::folder(both.receiver.inbox())), "the piece written went");
    let refusing = super::both("page-refused", Some(Answer { take: false, trust: false }), Kept { on: true, ..Kept::default() });
    assert_eq!(refusing.receiver.offered_by_page(page_sender("p3"), &files, None, false), Err((403, "refused")));
}

/// Words a page sends come as words: nothing begun, nothing in the inbox.
#[test]
fn words_a_page_sends_come_as_words() {
    let both = both("page-words", None, Kept { on: true, ..Kept::default() });
    assert_eq!(both.receiver.offered_by_page(page_sender("p4"), &[], Some("https://example.com".into()), true), Ok(None));
    let written = receipts(&both);
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].words, ["https://example.com"]);
    assert!(written[0].files.is_empty());
}

/// What this PC sends a page waits for the page to fetch it, piece by piece: listed with how this PC is, sent once all
/// is fetched (words alone once the page is told them). A page not open fails at once, one offered another is busy,
/// one turning it down refuses it, one cancelled gives no more of it.
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
    assert_eq!(pages.piece_of("f1", 9, 0, 0), Err((404, "no such sending")));
    assert_eq!(pages.piece_of("f1", 2, 0, 0).unwrap().len(), page::PIECE);
    assert_eq!(pages.piece_of("f1", 2, 0, page::PIECE as u64).unwrap(), [7u8; 10]);
    assert_eq!(states(2).first(), Some(&send::State::Waiting));
    assert_eq!(states(2).last(), Some(&send::State::Sent));
    assert!(pages.offered_to("f1").is_none(), "nothing more offered");
    // Words alone are sent as the page is told them.
    pages.offer("f1", 4, &[link::Outgoing::Words { words: "hello".into(), name: "text.txt".into() }], Arc::new(send::Job::default()), told.clone());
    assert_eq!(pages.offered_to("f1").expect("offered")["files"][0]["words"], "hello");
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
}
