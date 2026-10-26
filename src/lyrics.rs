//! The lyrics of what plays, each line with the time it is sung at (word by
//! word, where that is known), its translation where there is one, for what
//! shows them as the track plays. Asked for on the network, only while
//! lyrics are to be shown, by the track's number on NetEase Cloud Music
//! (where its player says it, as it does with the InfLink-rs plugin; else as
//! NetEase finds it by its title and length): of the AMLL TTML collection
//! (lyrics timed word by word, public domain, kept by that number, reached
//! through a CDN) and of NetEase itself; of Kugou (its lyrics timed word by
//! word, by the track's title, artist and length); of Apple Music (timed
//! word by word, as Apple itself writes them: its catalog searched as its
//! web player does, and its lyrics asked of PaxSenix's service by the
//! song's number); and of LRCLIB (an open
//! collection of lyrics timed by line), by the track's title, artist, album
//! and length. Lyrics timed word by word are taken first, wherever they are.
//! The sources are asked all at once (see `find`): where one far away cannot
//! be reached, as may be in mainland China, NetEase's still come at once.
//! Each track's are asked for on a thread of their own, and kept on disk
//! (see `kept`): asked for again only once they may have changed. Before
//! all of these, a file of the track on this PC (see `library`): its own.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

/// One line: when it is sung (seconds into the track) and till when, where
/// that is said; what is sung, and word by word where that is known; what is
/// sung behind it (backing vocals); and its translation where one is given.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub at: f64,
    pub end: Option<f64>,
    pub text: String,
    pub words: Vec<Word>,
    pub backing: Option<String>,
    pub translation: Option<String>,
}

/// A word (or a syllable, or a character), as it is sung: from when, till when, and what it is (with the space
/// after it, where there is one).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub at: f64,
    pub end: f64,
    pub text: String,
}

/// A track's lyrics, in the order they are sung.
pub type Lyrics = Arc<Vec<Line>>;

/// The track whose lyrics are asked for: its number on NetEase Cloud Music
/// where its player says it, what it is called, by whom, on which album, and
/// how long it is (seconds); and whether it plays in QQ Music (whose own lyrics are then the ones shown, where it has
/// them, as with NetEase's number).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Track {
    pub netease: Option<u64>,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub seconds: u32,
    #[serde(default)]
    pub qq_music: bool,
}

/// What is known of a track's lyrics: still being asked for, or found (none, where no source has them).
#[derive(Clone, Debug, PartialEq)]
pub enum Known {
    Asking,
    Found(Option<Lyrics>),
}

/// The tracks asked about, and what was found; the most kept.
static KNOWN: Mutex<Option<HashMap<Track, Held>>> = Mutex::new(None);
const KEPT_MOST: usize = 200;

/// What is held of a track's lyrics while Glance runs: as `Known`, or, where a source could not be reached and
/// none of the others had them, none for now (since then), to be asked for again a while later.
#[derive(Clone)]
enum Held {
    Asking,
    Found(Option<Lyrics>),
    Unreached(Instant),
}

/// How long lyrics a source could not be reached for are taken to be none before they are asked for again.
const UNREACHED_FOR: Duration = Duration::from_secs(60);

/// Lyrics found, as they are held: none, where they have no line (a file kept on this PC may say so; shown as
/// lyrics, they were turned through from nothing, and Glance stopped).
fn found(lines: Option<Vec<Line>>) -> Held {
    Held::Found(lines.filter(|lines| !lines.is_empty()).map(Arc::new))
}

/// What is known of `track`'s lyrics; asked for (on a thread of its own) the first time, and again a while after
/// a source could not be reached.
pub fn of(track: &Track) -> Known {
    let mut known = KNOWN.lock().unwrap();
    let known = known.get_or_insert_with(HashMap::new);
    match known.get(track) {
        Some(Held::Asking) => return Known::Asking,
        Some(Held::Found(found)) => return Known::Found(found.clone()),
        Some(Held::Unreached(since)) if since.elapsed() < UNREACHED_FOR => return Known::Found(None),
        _ => {}
    }
    if known.len() >= KEPT_MOST {
        known.retain(|_, held| matches!(held, Held::Asking));
    }
    known.insert(track.clone(), Held::Asking);
    let track = track.clone();
    std::thread::spawn(move || {
        // A file of it on this PC with lyrics: those (as they are now; not kept, as the file can change).
        if let Some(lines) = crate::library::lyrics(&track) {
            KNOWN.lock().unwrap().get_or_insert_with(HashMap::new).insert(track, found(Some(lines)));
            return;
        }
        let held = match kept::read(&track) {
            Some(kept) if kept.fresh() => found(kept.lines),
            kept => {
                let said = find(&track);
                // (That none has them, only where each source said so.)
                if said.sure || said.found.is_some() {
                    kept::write(&track, said.found.as_deref(), said.sure);
                }
                match (said.found, kept) {
                    (Some(lines), _) => found(Some(lines)),
                    // Not found now, a source unreached: what was found before, where there was.
                    (None, Some(kept)) if !said.sure => found(kept.lines),
                    (None, _) if !said.sure => Held::Unreached(Instant::now()),
                    (None, _) => Held::Found(None),
                }
            }
        };
        KNOWN.lock().unwrap().get_or_insert_with(HashMap::new).insert(track, held);
    });
    Known::Asking
}

/// A source that could not be reached, or would not answer: what it has is not known.
pub(crate) struct Unreached;

/// What a source has of what it was asked for (none, where it said it has none), where it answered.
pub(crate) type Answer<T> = Result<Option<T>, Unreached>;

/// What the sources asked had of a track's lyrics, and whether each answered (else they may yet have them).
struct Said {
    found: Option<Vec<Line>>,
    sure: bool,
}

/// Asks the sources, all at once (one far away, slow or not reached, keeps none waiting for the others, and none is
/// waited for where what is found already is not to be bettered: NetEase's timed word by word): by the
/// track's NetEase number, the word-timed collection and NetEase itself (the collection's, with NetEase's
/// translation where it gives none; else NetEase's); by its name, Kugou, LRCLIB, QQ Music, and the same by the number
/// NetEase finds it by. Lyrics timed word by word first, wherever they are: the collection's or NetEase's, then
/// Apple Music's (with a translation from the others where theirs have none), then Kugou's (the same). Else, by the
/// NetEase number its player gave, NetEase's own (the lyrics it shows itself); played in QQ Music, its own; then
/// LRCLIB's (found by title, artist, album and length, more surely the track's own than a search's), then NetEase's,
/// then QQ Music's (a track QQ Music alone has the lyrics of, played there, showed none).
/// What NetEase Cloud Music has for its track `id`, and of the AMLL collection: timed word by word first (with NetEase's
/// translation where none is in the collection, and NetEase's credits); else as its player shows them.
fn netease_by_number(id: u64) -> Option<Vec<Line>> {
    let (worded, theirs) = std::thread::scope(|scope| {
        let theirs = scope.spawn(|| netease_and_credits(id));
        (amll(id).ok().flatten(), theirs.join().ok().and_then(Result::ok))
    });
    let (theirs, credits) = match theirs {
        Some((lines, credits)) => (lines, credits),
        None => (None, Vec::new()),
    };
    match (worded, theirs) {
        (Some(mut lines), theirs) => {
            if let Some(theirs) = theirs.filter(|_| lines.iter().all(|line| line.translation.is_none())) {
                translated(&mut lines, &theirs);
            }
            Some(with_credits(lines, &credits))
        }
        (None, theirs) => theirs,
    }
}

fn find(track: &Track) -> Said {
    use std::sync::atomic::{AtomicBool, Ordering};
    let sure = AtomicBool::new(true);
    // What a source has, none where it was not reached (and then what the sources have is not sure).
    let answered = |answer: Answer<Vec<Line>>| {
        answer.unwrap_or_else(|Unreached| {
            sure.store(false, Ordering::Relaxed);
            None
        })
    };
    let by_number = |id: u64| netease_by_number(id);
    // (Each on a thread of its own, left to end on its own where its answer is not waited for.)
    let asked = |source: fn(&Track) -> Answer<Vec<Line>>| {
        let (told, answer) = std::sync::mpsc::channel();
        let track = track.clone();
        std::thread::spawn(move || {
            let _ = told.send(source(&track));
        });
        move || answer.recv().unwrap_or(Err(Unreached))
    };
    let by_kugou = asked(kugou);
    let by_apple = asked(apple_music);
    // (Played by NetEase: its own, else lyrics timed word by word elsewhere; none asked of the others.)
    let (by_qq, by_name) = match track.netease {
        Some(_) => (None, None),
        None => (Some(asked(qq_music)), Some(asked(lrclib))),
    };
    let netease = match track.netease {
        Some(id) => by_number(id),
        None => match netease_search(track) {
            Ok(id) => id.and_then(by_number),
            Err(Unreached) => answered(Err(Unreached)),
        },
    };
    let found = chosen(track, netease, || answered(by_apple()), || answered(by_kugou()), || by_qq.and_then(|by_qq| answered(by_qq())), || by_name.and_then(|by_name| answered(by_name())));
    Said { found, sure: sure.into_inner() }
}

/// Of what the sources had of `track`'s lyrics (NetEase's, the collection's among them; Apple Music's; Kugou's; QQ
/// Music's; LRCLIB's, each waited for as it is needed), those taken (see `find`): timed word by word first,
/// wherever they are (the collection's or NetEase's; else Apple Music's or Kugou's, each with a translation from
/// the others where theirs have none); else as its player shows them, by its NetEase number or played in QQ Music;
/// else LRCLIB's, NetEase's, QQ Music's.
fn chosen(track: &Track, netease: Option<Vec<Line>>, apple: impl FnOnce() -> Option<Vec<Line>>, kugou: impl FnOnce() -> Option<Vec<Line>>, qq: impl FnOnce() -> Option<Vec<Line>>, named: impl FnOnce() -> Option<Vec<Line>>) -> Option<Vec<Line>> {
    let worded = |lines: &[Line]| lines.iter().any(|line| !line.words.is_empty());
    if netease.as_deref().is_some_and(worded) {
        return netease;
    }
    let (apple_worded, apple_lined) = match apple() {
        Some(lines) if worded(&lines) => (Some(lines), None),
        Some(lines) => (None, Some(lines)),
        None => (None, None),
    };
    if let Some(mut lines) = apple_worded {
        if lines.iter().all(|line| line.translation.is_none()) {
            for theirs in [&netease, &qq()].into_iter().flatten() {
                translated(&mut lines, theirs);
            }
        }
        return Some(lines);
    }
    if let Some(mut lines) = apple_lined {
        if lines.iter().all(|line| line.translation.is_none()) {
            for theirs in [&netease, &qq()].into_iter().flatten() {
                translated(&mut lines, theirs);
            }
        }
        return Some(lines);
    }
    if let Some(mut lines) = kugou().filter(|lines| worded(lines)) {
        if lines.iter().all(|line| line.translation.is_none()) {
            for theirs in [&netease, &qq()].into_iter().flatten() {
                translated(&mut lines, theirs);
            }
        }
        return Some(lines);
    }
    if track.netease.is_some() {
        return netease;
    }
    match qq() {
        Some(qq) if track.qq_music => Some(qq),
        qq => named().or(netease).or(qq),
    }
}

/// `lines` with the translation of the line of `theirs` nearest in time to each, within a second.
fn translated(lines: &mut [Line], theirs: &[Line]) {
    for line in lines {
        let nearest = theirs.iter().filter(|their| their.translation.is_some()).min_by(|a, b| (a.at - line.at).abs().total_cmp(&(b.at - line.at).abs()));
        if let Some(their) = nearest.filter(|their| (their.at - line.at).abs() <= 1.0 && line.translation.is_none()) {
            line.translation = their.translation.clone();
        }
    }
}

/// The body of `url` (fetched as JSON is): none where it is not there (404).
fn fetched(url: &str) -> Answer<Vec<u8>> {
    match crate::update::answer_json(url) {
        Some((200, body)) => Ok(Some(body)),
        Some((404, _)) => Ok(None),
        _ => Err(Unreached),
    }
}

/// The JSON `url` answers with: none where it is not there (404).
fn fetched_json(url: &str) -> Answer<serde_json::Value> {
    match fetched(url)? {
        Some(body) => serde_json::from_slice(&body).map(Some).map_err(|_| Unreached),
        None => Ok(None),
    }
}

/// The AMLL TTML collection's lyrics of NetEase's track `id`, timed word by word (through a CDN that is reached
/// from mainland China too).
pub(crate) fn amll(id: u64) -> Answer<Vec<Line>> {
    let Some(file) = fetched(&format!("https://cdn.jsdelivr.net/gh/Steve-xmh/amll-ttml-db@main/ncm-lyrics/{id}.ttml"))? else { return Ok(None) };
    let lines = ttml(&String::from_utf8(file).map_err(|_| Unreached)?);
    Ok(lines.iter().any(|line| !line.words.is_empty()).then_some(lines))
}

/// The lines of a TTML lyrics file as Apple Music and the AMLL collection write them: a `<p>` each (`begin`,
/// `end`), a `<span>` for each word in it (`begin`, `end`); spans with a role of their own hold its backing vocals
/// (`x-bg`, themselves word by word), its translation (`x-translation`) or its romanisation (`x-roman`, left out).
pub fn ttml(xml: &str) -> Vec<Line> {
    let offset = ttml_offset(xml);
    let mut lines = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<p ") {
        let after = &rest[start..];
        let Some(close) = after.find("</p>") else { break };
        // (Its head closed before its end, or it is not a line: left out, the lines after it read.)
        let Some(head_end) = after[..close].find('>') else {
            rest = &after[close + 4..];
            continue;
        };
        let head = &after[..head_end];
        let body = &after[head_end + 1..close];
        rest = &after[close + 4..];
        let (Some(at), end) = (attribute(head, "begin").and_then(clock).map(|t| (t - offset).max(0.0)), attribute(head, "end").and_then(clock).map(|t| (t - offset).max(0.0))) else { continue };
        let (mut words, mut plain, mut backing, mut translation) = (Vec::new(), String::new(), String::new(), String::new());
        // What each open span is: a word (its times), or of a role.
        let mut open: Vec<(Option<String>, Option<(f64, f64)>)> = Vec::new();
        let mut body = body;
        while !body.is_empty() {
            let Some(tag) = body.find('<') else {
                text_into(&mut words, &mut plain, &mut backing, &mut translation, &open, body);
                break;
            };
            text_into(&mut words, &mut plain, &mut backing, &mut translation, &open, &body[..tag]);
            let Some(tag_end) = body[tag..].find('>') else { break };
            let inside = &body[tag + 1..tag + tag_end];
            body = &body[tag + tag_end + 1..];
            if inside.starts_with('/') {
                open.pop();
            } else if inside.starts_with("span") {
                let role = attribute(inside, "ttm:role").map(str::to_string);
                let times = attribute(inside, "begin").and_then(clock).zip(attribute(inside, "end").and_then(clock)).map(|(b, e)| ((b - offset).max(0.0), (e - offset).max(0.0)));
                if !inside.ends_with('/') {
                    open.push((role, times));
                }
            }
        }
        let text = if words.is_empty() { plain.trim().to_string() } else { words_to_text(&words) };
        let said = |text: String| Some(text.trim().to_string()).filter(|text| !text.is_empty());
        lines.push(Line { at, end, text, words, backing: said(backing), translation: said(translation) });
    }
    lines.sort_by(|a, b| a.at.total_cmp(&b.at));
    lines
}
/// The offset Apple Music or iTunes says all times are to be moved by, in seconds (`leadingSilence` or `lyricOffset`).
fn ttml_offset(xml: &str) -> f64 {
    // (<iTunesMetadata leadingSilence="0.180" ...> or <audio lyricOffset="0.046" ...>)
    let head = xml.find("</head>").map_or(xml, |end| &xml[..end]);
    if let Some(silence) = attribute(head, "leadingSilence").and_then(|s| s.parse::<f64>().ok()) {
        return silence;
    }
    if let Some(offset) = attribute(head, "lyricOffset").and_then(|s| s.parse::<f64>().ok()) {
        return offset;
    }
    0.0
}

/// Reconstructs the complete text of a line from its timed words without duplicating adjacent punctuation or spaces.
fn words_to_text(words: &[Word]) -> String {
    let mut text = String::new();
    for word in words {
        text.push_str(&word.text);
    }
    text.trim().to_string()
}

/// Text met inside a line: a word's (or the space after one), the backing vocals', or the translation's, as the
/// spans it is in say.
fn text_into(words: &mut Vec<Word>, plain: &mut String, backing: &mut String, translation: &mut String, open: &[(Option<String>, Option<(f64, f64)>)], text: &str) {
    if text.is_empty() {
        return;
    }
    let text = unescaped(text);
    let role = open.iter().find_map(|(role, _)| role.as_deref());
    match role {
        Some("x-translation") => translation.push_str(&text),
        Some("x-bg") => backing.push_str(&text),
        Some(_) => {}
        None => match open.last().and_then(|(_, times)| *times) {
            Some((at, end)) => words.push(Word { at, end, text }),
            // (Between words: the space after the last; or the line's plain text where no words are timed.)
            None => {
                if let Some(last) = words.last_mut() {
                    last.text.push_str(&text);
                } else {
                    plain.push_str(&text);
                }
            }
        },
    }
}

/// The value of `name` in a tag's attributes.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let at = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    let value = &tag[at..];
    Some(&value[..value.find('"')?])
}

/// XML's five named characters and numbered ones, as themselves.
fn unescaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at..];
        let Some(end) = after.find(';') else { break };
        let name = &after[1..end];
        let named = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => name.strip_prefix("#x").and_then(|hex| u32::from_str_radix(hex, 16).ok()).or_else(|| name.strip_prefix('#').and_then(|n| n.parse().ok())).and_then(char::from_u32),
        };
        match named {
            Some(c) => {
                out.push(c);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A TTML clock time (`h:mm:ss.fff`, `mm:ss.fff`, `ss.fff`, with or without `s`) as seconds.
fn clock(time: &str) -> Option<f64> {
    let time = time.trim().trim_end_matches('s');
    let mut total = 0.0;
    for part in time.split(':') {
        total = total * 60.0 + part.parse::<f64>().ok()?;
    }
    seconds(total)
}

/// What a browser asks Apple's pages as (its player's page and its scripts answered so).
const BROWSER: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// Apple Music's web player's token (see `webplay_token`), held while it is good: what it is, and when it runs
/// out (seconds since 1970).
static WEBPLAY: Mutex<Option<(String, u64)>> = Mutex::new(None);

/// The path of the player's script where its token was last found, held to spare the page a re-read where
/// only the token has run out (the script is the same one, its token refreshed within it).
static SCRIPT: Mutex<Option<String>> = Mutex::new(None);

/// The storefront of Apple's catalog most likely to name `track` as its player does: where configured in
/// settings, that storefront first; else as picked from the track's script (Japan for Japanese kana, China for
/// Chinese hanzi, America for Western Latin and the like); the others asked only where that first has no match.
fn apple_storefronts(track: &Track) -> [(&'static str, &'static str); 3] {
    let chosen = crate::APP.get().and_then(|app| app.settings.lock().ok()).and_then(|s| s.apple_storefront.clone());
    match chosen.as_deref() {
        Some("cn") => [("cn", "zh-Hans"), ("us", "en-US"), ("jp", "ja")],
        Some("us") => [("us", "en-US"), ("cn", "zh-Hans"), ("jp", "ja")],
        Some("jp") => [("jp", "ja"), ("cn", "zh-Hans"), ("us", "en-US")],
        Some("hk") => [("hk", "zh-Hant-HK"), ("cn", "zh-Hans"), ("tw", "zh-Hant-TW")],
        Some("tw") => [("tw", "zh-Hant-TW"), ("cn", "zh-Hans"), ("hk", "zh-Hant-HK")],
        _ => {
            let text = format!("{} {}", track.title, track.artist);
            if text.chars().any(|c| matches!(c, '\u{3040}'..='\u{30ff}')) {
                [("jp", "ja"), ("cn", "zh-Hans"), ("us", "en-US")]
            } else if text.chars().any(|c| matches!(c, '\u{4e00}'..='\u{9fff}')) {
                [("cn", "zh-Hans"), ("us", "en-US"), ("jp", "ja")]
            } else {
                [("us", "en-US"), ("cn", "zh-Hans"), ("jp", "ja")]
            }
        }
    }
}

/// A song of Apple Music's catalog: its identifier, duration (seconds), and its title and artist as named
/// in that store's language.
#[derive(Clone, Debug)]
struct AppleSong {
    id: String,
    duration: f64,
    title: String,
    artist: String,
}

/// A title with its trailing remarks (" (feat. …)", " - Single", " (Remastered)") cut off.
fn title_bare(title: &str) -> &str {
    for mark in [" - Single", " (feat.", " [feat.", " (Bonus", " [Bonus", " (Remaster", " [Remaster"] {
        if let Some(at) = title.find(mark) {
            return title[..at].trim();
        }
    }
    title
}

/// Apple Music's lyrics of `track` (as Apple itself writes them, TTML — see `ttml`): the song of its catalog
/// found for the track — the store of its native land asked first, the other two asked together only where that
/// has no match (sparing two stores a search in the common case) — and its lyrics asked of PaxSenix's service by
/// the song's number, which asks Apple for them as the web player does (no sign-in to any of it). None where the
/// song is not Apple's.
fn apple_music(track: &Track) -> Answer<Vec<Line>> {
    let [first, second, third] = apple_storefronts(track);
    // (The store of the track's script first: most tracks match where they were written; the others spared.)
    let (song, unreached_first) = match apple_song(track, first.0, first.1) {
        Ok(Some(song)) => (Some(song), false),
        Ok(None) => (None, false),
        Err(Unreached) => (None, true),
    };
    let song = match song {
        Some(song) => song,
        None => {
            // (The other two stores together: each waits only for itself.)
            let (two, three) = std::thread::scope(|scope| {
                let three = scope.spawn(|| apple_song(track, third.0, third.1));
                let two = apple_song(track, second.0, second.1);
                (two, three.join().unwrap_or(Err(Unreached)))
            });
            let length = f64::from(track.seconds);
            let mut candidates: Vec<AppleSong> = Vec::new();
            let mut unreached = unreached_first;
            for answer in [two, three] {
                match answer {
                    Ok(Some(song)) => candidates.push(song),
                    Ok(None) => {}
                    Err(Unreached) => unreached = true,
                }
            }
            match candidates.into_iter().min_by(|a, b| (a.duration - length).abs().total_cmp(&(b.duration - length).abs())) {
                Some(song) => song,
                None => return if unreached { Err(Unreached) } else { Ok(None) },
            }
        }
    };
    // (PaxSenix keeps what was asked for before in its cache, answered quickly; else asked of its lyrics service.
    // Where its service is busy, one more attempt after a moment: a transient state, quickly past.)
    let fetch_lyrics = || fetched_json(&format!("https://lyrics.paxsenix.org/apple-music/lyrics?id={}&ttml=true", song.id));
    let said = match fetch_lyrics().ok().flatten() {
        Some(said) if said["error"].as_bool() == Some(true) => {
            std::thread::sleep(std::time::Duration::from_millis(1_500));
            fetch_lyrics().ok().flatten().filter(|said| said["error"].as_bool() != Some(true))
        }
        other => other,
    };
    let xml = said.and_then(|said| said["content"].as_str().map(str::to_string));
    if let Some(xml) = xml {
        let lines = ttml(&xml);
        if !lines.is_empty() {
            return Ok(Some(lines));
        }
    }
    // (Where PaxSenix has no lyrics of Apple's song or its service is busy: dive down to NetEase Cloud Music,
    // asked with the canonical title and artist resolved from Apple Music's catalog in the chosen region.)
    let canonical = Track {
        title: song.title,
        artist: song.artist,
        seconds: song.duration.round() as u32,
        album: track.album.clone(),
        netease: None,
        qq_music: false,
    };
    if let Ok(Some(id)) = netease_search(&canonical) {
        if let Some(lines) = netease_by_number(id) {
            return Ok(Some(lines));
        }
    }
    Ok(None)
}

/// The song of Apple's catalog that `track` is, in its store `storefront` (its pages in `language`): of those
/// its search finds by the title and artist, the nearest in length — within three hundred milliseconds first,
/// as Apple's own data is exact; within two seconds where none is so close (a live or alternate version close
/// enough). Asked as the web player asks, with the token of the player (see `webplay_token`), taken again once
/// where it no longer works.
fn apple_song(track: &Track, storefront: &str, language: &str) -> Result<Option<AppleSong>, Unreached> {
    let token = webplay_token()?;
    let searched = |token: &str| {
        let bearer = format!("Bearer {token}");
        crate::update::answer_json_with(
            &format!("https://amp-api.music.apple.com/v1/catalog/{storefront}/search?term={}&types=songs&limit=25&l={language}&platform=web&format%5Bresources%5D=map", escaped(&format!("{} {}", track.title, artists_in(&track.artist).get(1).copied().unwrap_or(&track.artist)))),
            BROWSER,
            &[("Authorization", bearer.as_str()), ("Origin", "https://music.apple.com"), ("Referer", "https://music.apple.com/")],
        )
    };
    let (status, body) = match searched(&token) {
        Some((401, _)) => {
            // (The token no longer any good: let it be taken as the page has it now.)
            *WEBPLAY.lock().unwrap() = None;
            let token = webplay_token()?;
            searched(&token).ok_or(Unreached)?
        }
        Some(said) => said,
        None => return Err(Unreached),
    };
    if status != 200 {
        return Err(Unreached);
    }
    let said: serde_json::Value = serde_json::from_slice(&body).map_err(|_| Unreached)?;
    let length = f64::from(track.seconds);
    let of = |song: &serde_json::Value| -> Option<AppleSong> {
        let said = &song["attributes"];
        let name = said["name"].as_str()?;
        if !same(name, &track.title) && !same(title_bare(name), title_bare(&track.title)) {
            return None;
        }
        let artist_name = said["artistName"].as_str()?;
        if !by_artist(artists_in(artist_name), &track.artist) {
            return None;
        }
        Some(AppleSong {
            id: song["id"].as_str()?.to_string(),
            duration: said["durationInMillis"].as_f64()? / 1000.0,
            title: name.to_string(),
            artist: artist_name.to_string(),
        })
    };
    let songs: Vec<_> = said["resources"]["songs"].as_object().into_iter().flatten().filter_map(|(_, song)| of(song)).collect();
    Ok(songs.into_iter().min_by(|a, b| (a.duration - length).abs().total_cmp(&(b.duration - length).abs())).filter(|s| (s.duration - length).abs() <= 2.0))
}

/// Apple Music's web player's token, as its page asks its catalog for songs with (a bearer token of the page
/// itself: no sign-in, good for a couple of months): what is held of it still, else a fresh one taken from the
/// page (see `webplay_taken`). Held (the token, and the taking of one: the stores' searches, each wanting it,
/// wait for the one asking) till an hour before it runs out — a clock a little behind Apple's, or one in a
/// time zone that differs, should not ask with a token just out.
fn webplay_token() -> Result<String, Unreached> {
    let mut held = WEBPLAY.lock().unwrap();
    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs();
    if let Some((token, _)) = held.as_ref().filter(|(_, till)| now + 3_600 < *till) {
        return Ok(token.clone());
    }
    let (token, till) = webplay_taken()?;
    *held = Some((token.clone(), till));
    Ok(token)
}

/// The web player's token as Apple's page has it: the last script that yielded a token tried first (the script
/// file endures well past its token; most refreshes spare the page a re-read). Where that fails, the page read
/// anew, each script it runs read (the player's own first), and of the player's tokens in what they say, the
/// one that lasts longest.
fn webplay_taken() -> Result<(String, u64), Unreached> {
    // (Try the script known to hold a token before reading the page again.)
    let cached = SCRIPT.lock().unwrap().clone();
    if let Some(path) = cached {
        if let Ok(Some(js)) = apple_script(&path) {
            let mut best: Option<(String, u64)> = None;
            for (token, till) in tokens_in(&js) {
                if best.as_ref().is_none_or(|(_, last)| till > *last) {
                    best = Some((token, till));
                }
            }
            if let Some(found) = best {
                return Ok(found);
            }
        }
        // (Script gone or token expired within it: clear, and fall through to re-read the page.)
        *SCRIPT.lock().unwrap() = None;
    }
    let page = match crate::update::answer_json_with("https://music.apple.com/", BROWSER, &[]) {
        Some((200, body)) => String::from_utf8(body).map_err(|_| Unreached)?,
        _ => return Err(Unreached),
    };
    let scripts = script_urls(&page);
    let mut best: Option<(String, u64)> = None;
    for script in scripts {
        let Some(js) = apple_script(script)? else { continue };
        for (token, till) in tokens_in(&js) {
            if best.as_ref().is_none_or(|(_, last)| till > *last) {
                best = Some((token, till));
            }
        }
        // (With one found, none of the others read: the player's own was asked for first.)
        if best.is_some() {
            *SCRIPT.lock().unwrap() = Some(script.to_string());
            break;
        }
    }
    best.ok_or(Unreached)
}

/// The paths of each `/assets/*.js` script that Apple Music's `page` loads, the player's index script first
/// (its token in it, the others' tokens spares).
fn script_urls(page: &str) -> Vec<&str> {
    let mut scripts: Vec<&str> = Vec::new();
    let mut rest = page;
    while let Some(at) = rest.find("src=\"/assets/") {
        // (From the quote that opens where the script is, to the one that closes it: "/assets/…".)
        let after = &rest[at + 5..];
        let Some(end) = after.find('"') else { break };
        let script = &after[..end];
        rest = &after[end..];
        if script.ends_with(".js") {
            // (The player's own, its token in it: read first.)
            if script.contains("index") {
                scripts.insert(0, script);
            } else {
                scripts.push(script);
            }
        }
    }
    scripts
}

/// What the script at `path` on Apple Music's site says; none where there is no such script.
fn apple_script(path: &str) -> Answer<String> {
    match crate::update::answer_json_with(&format!("https://music.apple.com{path}"), BROWSER, &[]) {
        Some((200, body)) => String::from_utf8(body).map(Some).map_err(|_| Unreached),
        Some((404, _)) => Ok(None),
        _ => Err(Unreached),
    }
}

/// The web player's tokens in what one of Apple's scripts says (what reads as a JSON web token — three runs of
/// base64's letters, dots between, head and what it says read — and says it is one of the player's): the ones
/// not yet out, each with when it runs out (seconds since 1970).
fn tokens_in(js: &str) -> Vec<(String, u64)> {
    let read = |part: &str| base64_decoded(part).and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs();
    let mut tokens = Vec::new();
    let mut rest = js;
    while let Some(at) = rest.find("eyJ") {
        let after = &rest[at..];
        let end = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')).unwrap_or(after.len());
        rest = &after[end..];
        let said = &after[..end];
        let mut parts = said.split('.');
        let (Some(head), Some(what), Some(sign), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else { continue };
        if head.is_empty() || what.is_empty() || sign.is_empty() {
            continue;
        }
        let (Some(head), Some(what)) = (read(head), read(what)) else { continue };
        if head["kid"].as_str() == Some("WebPlayKid") && head["alg"].as_str() == Some("ES256") && what["iss"].as_str() == Some("AMPWebPlay") {
            if let Some(till) = what["exp"].as_u64().filter(|till| now < *till) {
                tokens.push((said.to_string(), till));
            }
        }
    }
    tokens
}

/// NetEase Cloud Music's lyrics of its track `id`, with their translation: timed word by word where it has them
/// (its `yrc`), else line by line; with its lines of who wrote and made the song either way (see `credit`).
#[cfg(test)]
fn netease(id: u64) -> Answer<Vec<Line>> {
    netease_and_credits(id).map(|(lines, _)| lines)
}

/// As `netease`, and its lines of who wrote and made the song apart (for lyrics found elsewhere: see `find`).
fn netease_and_credits(id: u64) -> Result<(Option<Vec<Line>>, Vec<Line>), Unreached> {
    let said = netease_said(&format!("https://music.163.com/api/song/lyric/v1?id={id}&lv=1&tv=-1&yv=1"))?;
    let translation = said["tlyric"]["lyric"].as_str();
    let credits = |text: Option<&str>| text.map(|text| text.lines().filter_map(|raw| credit(raw.trim())).collect::<Vec<_>>()).unwrap_or_default();
    if let Some(lines) = said["yrc"]["lyric"].as_str().map(|yrc| word_timed(yrc, translation)).filter(|lines| lines.iter().any(|line| !line.words.is_empty())) {
        return Ok((Some(lines), credits(said["yrc"]["lyric"].as_str())));
    }
    // Line by line: its lines of who wrote and made it are written among them as JSON (see `credit`), which the
    // LRC's times do not take (left out, the lyrics showed nothing while NetEase showed those lines).
    let lrc = said["lrc"]["lyric"].as_str();
    let credits = credits(lrc);
    let lines = lrc.map(|lrc| with_credits(merged(lrc, translation), &credits));
    Ok((lines.filter(|lines| lines.iter().any(|line| !line.text.is_empty())), credits))
}

/// `lines` with `credits` (NetEase's lines of who wrote and made the song) among them by their times, each one they
/// do not hold already.
fn with_credits(mut lines: Vec<Line>, credits: &[Line]) -> Vec<Line> {
    for credit in credits {
        if !lines.iter().any(|line| line.text == credit.text) {
            lines.push(credit.clone());
        }
    }
    // (In order of time, those at one time as they were.)
    lines.sort_by(|a, b| a.at.total_cmp(&b.at));
    lines
}

/// What NetEase's API says to `url`, where it answers (its own `code` 200; another, such as when it is asked too
/// often, is no answer).
fn netease_said(url: &str) -> Result<serde_json::Value, Unreached> {
    fetched_json(url)?.filter(|said| said["code"].as_i64() == Some(200)).ok_or(Unreached)
}

/// The lines of NetEase's word-timed lyrics (`yrc`): `[start,length]` (milliseconds) before each, then each word as
/// `(start,length,0)` and what is sung; and the lines it writes otherwise, as JSON (`{"t":0,"c":[{"tx":"作词: "},…]}`:
/// who wrote it, who made it), timed by line alone, shown as NetEase shows them (left out, the lyrics showed nothing
/// while NetEase showed those lines, as if they had fallen behind it). Each with the line of `translation` (an LRC
/// text) nearest in time, within a second.
pub fn word_timed(yrc: &str, translation: Option<&str>) -> Vec<Line> {
    let mut lines = Vec::new();
    for raw in yrc.lines() {
        if let Some(line) = credit(raw.trim()) {
            lines.push(line);
            continue;
        }
        let Some((head, rest)) = raw.trim().strip_prefix('[').and_then(|after| after.split_once(']')) else { continue };
        let Some((start, length)) = head.split_once(',').and_then(|(a, b)| Some((ms(a.trim().parse().ok()?)?, ms(b.trim().parse().ok()?)?))) else { continue };
        let mut words = Vec::new();
        let mut rest = rest;
        while let Some(((at, lasts), after)) = word_timing(rest) {
            // What is sung runs to the next word's timing, or the line's end (a bracket in what is sung, "(world)",
            // is sung).
            let end = (0..after.len()).filter(|at| after.is_char_boundary(*at) && after[*at..].starts_with('(')).find(|at| word_timing(&after[*at..]).is_some()).unwrap_or(after.len());
            let text = &after[..end];
            if !text.is_empty() {
                words.push(Word { at, end: at + lasts, text: text.to_string() });
            }
            rest = &after[end..];
        }
        let text: String = words.iter().map(|word| word.text.as_str()).collect::<String>().trim().to_string();
        lines.push(Line { at: start, end: Some(start + length), text, words, backing: None, translation: None });
    }
    lines.sort_by(|a, b| a.at.total_cmp(&b.at));
    for (at, text) in translation.map(timed).unwrap_or_default() {
        // (A translation of nothing is written "//".)
        if text.is_empty() || text == "//" {
            continue;
        }
        let nearest = lines.iter_mut().min_by(|a, b| (a.at - at).abs().total_cmp(&(b.at - at).abs()));
        if let Some(line) = nearest.filter(|line| (line.at - at).abs() <= 1.0 && line.translation.is_none()) {
            line.translation = Some(text);
        }
    }
    lines
}

/// A word's timing at the start of `rest` in NetEase's word-timed lyrics, `(start,length,0)` (milliseconds): its start
/// and length (seconds), and what follows it.
fn word_timing(rest: &str) -> Option<((f64, f64), &str)> {
    let (timing, after) = rest.strip_prefix('(')?.split_once(')')?;
    let mut parts = timing.split(',').map(str::trim);
    let (at, lasts, _) = (parts.next()?, parts.next()?, parts.next()?.parse::<i64>().ok()?);
    Some(((ms(at.parse().ok()?)?, ms(lasts.parse().ok()?)?), after))
}

/// The longest time lyrics are read with (seconds): a day (a track is no longer; a time beyond it, or none, is not one).
const LONGEST: f64 = 24.0 * 3600.0;

/// `ms` milliseconds as seconds, where it is a time of a track (see `LONGEST`).
fn ms(ms: f64) -> Option<f64> {
    seconds(ms / 1000.0)
}

/// `seconds`, where it is a time of a track (see `LONGEST`).
fn seconds(seconds: f64) -> Option<f64> {
    (0.0..=LONGEST).contains(&seconds).then_some(seconds)
}

/// A line NetEase writes as JSON in its word-timed lyrics (see `word_timed`): when it begins (`t`, milliseconds) and
/// what it says (each part's `tx`, together). None where `raw` is no such line, or says nothing.
fn credit(raw: &str) -> Option<Line> {
    let said: serde_json::Value = serde_json::from_str(raw.strip_prefix('{').map(|_| raw)?).ok()?;
    let at = ms(said["t"].as_f64()?)?;
    let text: String = said["c"].as_array()?.iter().filter_map(|part| part["tx"].as_str()).collect::<String>().trim().to_string();
    (!text.is_empty()).then(|| Line { at, end: None, text, words: Vec::new(), backing: None, translation: None })
}

/// The track on NetEase Cloud Music that is `track`: of those found by its title and artist, the first of its
/// title, by its artist, and of its length (to two seconds).
fn netease_search(track: &Track) -> Answer<u64> {
    let asked = format!("{} {}", track.title, track.artist);
    let said = netease_said(&format!("https://music.163.com/api/search/get?s={}&type=1&limit=10", escaped(&asked)))?;
    let found = |artist: &str| {
        said["result"]["songs"].as_array().into_iter().flatten().find_map(|song| {
            let length = song["duration"].as_f64()? / 1000.0;
            let artists = song["artists"].as_array().into_iter().flatten().filter_map(|artist| artist["name"].as_str());
            let by = by_artist(artists, artist);
            (same(song["name"].as_str()?, &track.title) && by && (length - track.seconds as f64).abs() <= 2.0).then(|| song["id"].as_u64()).flatten()
        })
    };
    if let Some(id) = found(&track.artist) {
        return Ok(Some(id));
    }
    // The player names the artist as NetEase translates the name (YouTube Music in Chinese: "酷玩乐队", Coldplay): by
    // the name of the artist NetEase so translates, or knows by that name too.
    let known = netease_said(&format!("https://music.163.com/api/search/get?s={}&type=100&limit=5", escaped(&track.artist)))?;
    let named = |artist: &serde_json::Value| {
        let mut others = artist["alias"].as_array().into_iter().flatten().filter_map(|alias| alias.as_str()).chain(artist["trans"].as_str());
        others.any(|other| same(other, &track.artist))
    };
    Ok(known["result"]["artists"].as_array().into_iter().flatten().filter(|artist| named(artist)).filter_map(|artist| artist["name"].as_str()).find_map(found))
}

/// QQ Music's timed lyrics of `track`, with their translation: of the tracks it finds by title and artist, the first
/// of its title, by its artist, and of its length (to two seconds). (Its API answers only as from its own site.)
fn qq_music(track: &Track) -> Answer<Vec<Line>> {
    const SITE: &str = "https://y.qq.com/";
    let asked = |url: &str, ok: &str| -> Answer<serde_json::Value> {
        match crate::update::answer_json_from(url, SITE) {
            Some((200, body)) => serde_json::from_slice::<serde_json::Value>(&body).ok().filter(|said| said[ok].as_i64() == Some(0)).map(Some).ok_or(Unreached),
            Some((404, _)) => Ok(None),
            _ => Err(Unreached),
        }
    };
    let searched = format!("https://c.y.qq.com/soso/fcgi-bin/client_search_cp?w={}&p=1&n=10&format=json", escaped(&format!("{} {}", track.title, track.artist)));
    let Some(said) = asked(&searched, "code")? else { return Ok(None) };
    let found = said["data"]["song"]["list"].as_array().into_iter().flatten().find_map(|song| {
        let artists = song["singer"].as_array().into_iter().flatten().filter_map(|singer| singer["name"].as_str());
        let length = song["interval"].as_f64()?;
        (same(song["songname"].as_str()?, &track.title) && by_artist(artists, &track.artist) && (length - track.seconds as f64).abs() <= 2.0).then(|| song["songmid"].as_str()).flatten()
    });
    let Some(mid) = found else { return Ok(None) };
    let lyric = format!("https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg?songmid={}&format=json&nobase64=1&g_tk=5381", escaped(mid));
    let Some(said) = asked(&lyric, "retcode")? else { return Ok(None) };
    let translation = said["trans"].as_str().filter(|trans| !trans.is_empty());
    Ok(said["lyric"].as_str().map(|lrc| merged(lrc, translation)).filter(|lines| lines.iter().any(|line| !line.text.is_empty())))
}

/// Kugou's lyrics of `track`, timed word by word (its KRC), with their translation where they have one: of the songs
/// it finds by title and artist, the first of that title (its highlighting left out), by that artist and of that
/// length (to two seconds); of the lyrics it keeps for that song, the first of its length. None where they are not
/// timed word by word (what is chosen from them, see `find`, are those).
fn kugou(track: &Track) -> Answer<Vec<Line>> {
    // (Its mobile search: its web search answers from some of its servers with nothing found.)
    let searched = format!("https://mobiles.kugou.com/api/v3/search/song?format=json&keyword={}&page=1&pagesize=20&showtype=1", escaped(&format!("{} {}", track.title, track.artist)));
    let Some(said) = fetched_json(&searched)? else { return Ok(None) };
    let plain = |text: &str| text.replace("<em>", "").replace("</em>", "");
    let found = said["data"]["info"].as_array().into_iter().flatten().find_map(|song| {
        let singers = plain(song["singername"].as_str()?);
        let artists = std::iter::once(singers.as_str()).chain(singers.split(['、', ','])).map(str::trim);
        let length = song["duration"].as_f64()?;
        (same(&plain(song["songname"].as_str()?), &track.title) && by_artist(artists, &track.artist) && (length - track.seconds as f64).abs() <= 2.0).then(|| song["hash"].as_str().map(str::to_string)).flatten()
    });
    let Some(hash) = found else { return Ok(None) };
    let Some(kept) = fetched_json(&format!("https://krcs.kugou.com/search?ver=1&man=yes&client=mobi&keyword=&duration=&hash={}", escaped(&hash)))? else { return Ok(None) };
    let ms = f64::from(track.seconds) * 1000.0;
    let chosen = kept["candidates"].as_array().into_iter().flatten().find(|one| one["duration"].as_f64().is_some_and(|length| (length - ms).abs() <= 2000.0));
    let Some((id, key)) = chosen.and_then(|one| Some((one["id"].as_str()?, one["accesskey"].as_str()?))) else { return Ok(None) };
    let Some(lyric) = fetched_json(&format!("https://lyrics.kugou.com/download?ver=1&client=pc&id={}&accesskey={}&fmt=krc&charset=utf8", escaped(id), escaped(key)))? else { return Ok(None) };
    let Some(text) = lyric["content"].as_str().and_then(krc_opened) else { return Ok(None) };
    Ok(Some(krc(&text)).filter(|lines| lines.iter().any(|line| !line.words.is_empty())))
}

/// The words of a KRC file as Kugou gives it (`content`, in base64): past its mark ("krc1"), each byte joined
/// (exclusive or) with Kugou's key in turn, then inflated (zlib), in UTF-8; none where it is not one.
fn krc_opened(content: &str) -> Option<String> {
    const KEY: [u8; 16] = [64, 71, 97, 119, 94, 50, 116, 71, 81, 54, 49, 45, 206, 210, 110, 105];
    let bytes = base64_decoded(content)?;
    let sealed = bytes.strip_prefix(b"krc1")?;
    let unsealed: Vec<u8> = sealed.iter().zip(KEY.iter().cycle()).map(|(byte, key)| byte ^ key).collect();
    let inflated = miniz_oxide::inflate::decompress_to_vec_zlib(&unsealed).ok()?;
    let text = String::from_utf8(inflated).ok()?;
    Some(text.strip_prefix('\u{FEFF}').map(str::to_string).unwrap_or(text))
}

/// `text` in base64 (RFC 4648, its alphabet as JSON web tokens write it too — `-`, `_` for `+`, `/` — padded or
/// not, spaces and line breaks passed over) as its bytes; none where it is not.
fn base64_decoded(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    };
    let digits: Vec<u8> = text.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    let digits = digits.strip_suffix(b"==").or_else(|| digits.strip_suffix(b"=")).unwrap_or(&digits);
    let mut bytes = Vec::with_capacity(digits.len() * 3 / 4);
    for chunk in digits.chunks(4) {
        if chunk.len() == 1 {
            return None;
        }
        let mut whole = 0u32;
        for (at, digit) in chunk.iter().enumerate() {
            whole |= u32::from(value(*digit)?) << (18 - 6 * at);
        }
        bytes.extend(&whole.to_be_bytes()[1..chunk.len()]);
    }
    Some(bytes)
}

/// Lyrics in Kugou's KRC: each line `[start,length]` (ms), then its words, each `<offset,length,0>` (ms from the
/// line's start) before what is sung; its translation in `[language:…]` (base64 JSON: of its lists, the one of type
/// 1, a line of it for each line). A line with no length (the song's name over its first) left out.
pub fn krc(text: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut translation: Vec<String> = Vec::new();
    for raw in text.lines().map(str::trim) {
        if let Some(said) = raw.strip_prefix("[language:").and_then(|rest| rest.strip_suffix(']')) {
            let json = base64_decoded(said).and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
            let lists = json.as_ref().and_then(|json| json["content"].as_array().cloned()).unwrap_or_default();
            if let Some(list) = lists.iter().find(|list| list["type"].as_i64() == Some(1)) {
                translation = list["lyricContent"].as_array().into_iter().flatten().map(|line| line.as_array().into_iter().flatten().filter_map(|part| part.as_str()).collect::<String>()).collect();
            }
            continue;
        }
        let Some((head, rest)) = raw.strip_prefix('[').and_then(|after| after.split_once(']')) else { continue };
        let Some((start, length)) = head.split_once(',').and_then(|(a, b)| Some((a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?))) else { continue };
        // (Times of a track only, as every other form's, see `LONGEST`: a line at no such time left out, in its place
        // among the lines its translation's are matched to.)
        let (Some(start), Some(length)) = (ms(start), ms(length)) else {
            lines.push(None);
            continue;
        };
        let Some(end) = seconds(start + length) else {
            lines.push(None);
            continue;
        };
        let mut words = Vec::new();
        for part in rest.split('<').skip(1) {
            let Some((timing, sung)) = part.split_once('>') else { continue };
            let mut numbers = timing.split(',').map(|number| number.trim().parse::<f64>().ok().and_then(ms));
            let (Some(Some(offset)), Some(Some(lasts))) = (numbers.next(), numbers.next()) else { continue };
            let (Some(at), Some(till)) = (seconds(start + offset), seconds(start + offset + lasts)) else { continue };
            if !sung.is_empty() {
                words.push(Word { at, end: till, text: sung.to_string() });
            }
        }
        let text: String = words.iter().map(|word| word.text.as_str()).collect::<String>().trim().to_string();
        lines.push((length > 0.0 && !text.is_empty()).then(|| Line { at: start, end: Some(end), text, words, backing: None, translation: None }));
    }
    // (Its translation's lines, one for each of its lines, those left out too.)
    lines
        .into_iter()
        .enumerate()
        .filter_map(|(at, line)| line.map(|line| Line { translation: translation.get(at).filter(|text| !text.trim().is_empty()).cloned(), ..line }))
        .collect()
}

/// Whether a track by `artists` (as a collection names them) is by `artist` (as its player does): one of them is,
/// but for case and spaces. Any is, where the player names none. (By title and length alone, another artist's song
/// of that name, a cover, was taken for it, and its lyrics shown.)
fn by_artist<'a>(artists: impl IntoIterator<Item = &'a str>, artist: &str) -> bool {
    if artist.trim().is_empty() {
        return true;
    }
    let player_artists = artists_in(artist);
    let store_artists: Vec<&'a str> = artists.into_iter().collect();
    store_artists.iter().any(|theirs| player_artists.iter().any(|mine| same(theirs, mine)))
}

/// The artists of a track (as players and collections name them, all in one, e.g. "A, B & C feat. D" or
/// "明透 — ray of hope"): the whole, and each between the signs that join them.
fn artists_in(named: &str) -> Vec<&str> {
    let mut artists = vec![named];
    for between in [",", "&", "/", ";", " feat. ", " ft. ", " x ", " — ", " - "] {
        artists = artists.into_iter().flat_map(|artist| artist.split(between)).collect();
    }
    std::iter::once(named).chain(artists).map(str::trim).filter(|artist| !artist.is_empty()).collect()
}

/// LRCLIB's timed lyrics of `track`: the one it has of that title, artist, album and length; else, of those
/// it finds by title and artist, the first by that artist of that length (to two seconds) with timed lyrics.
fn lrclib(track: &Track) -> Answer<Vec<Line>> {
    let timed = |said: &serde_json::Value| said["syncedLyrics"].as_str().map(|lrc| merged(lrc, None)).filter(|lines| lines.iter().any(|line| !line.text.is_empty()));
    let exact = format!(
        "https://lrclib.net/api/get?track_name={}&artist_name={}&album_name={}&duration={}",
        escaped(&track.title),
        escaped(&track.artist),
        escaped(&track.album),
        track.seconds
    );
    if let Some(found) = fetched_json(&exact)?.and_then(|said| timed(&said)) {
        return Ok(Some(found));
    }
    let search = format!("https://lrclib.net/api/search?track_name={}&artist_name={}", escaped(&track.title), escaped(&track.artist));
    let said = fetched_json(&search)?.unwrap_or_default();
    Ok(said
        .as_array()
        .into_iter()
        .flatten()
        .filter(|one| one["duration"].as_f64().is_some_and(|length| (length - track.seconds as f64).abs() <= 2.0))
        .filter(|one| by_artist(artists_in(one["artistName"].as_str().unwrap_or_default()), &track.artist))
        .find_map(timed))
}

/// Whether two titles are the same, but for case and spaces.
fn same(a: &str, b: &str) -> bool {
    let bare = |s: &str| s.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect::<String>();
    bare(a) == bare(b)
}

/// `text` as a URL's query has it (RFC 3986: all but the unreserved, by their UTF-8 bytes).
fn escaped(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (byte as char).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// The lines of an LRC text (`[mm:ss.xx]` before each, one or more), in the order they are sung, each with the
/// line of `translation` (another LRC text) at its time. (NetEase's put some lines' breaks in as `\n`, written
/// out: they are breaks.)
pub fn merged(lrc: &str, translation: Option<&str>) -> Vec<Line> {
    let mut lines: Vec<Line> = timed(lrc).into_iter().map(|(at, text)| Line { at, end: None, text, words: Vec::new(), backing: None, translation: None }).collect();
    for (at, text) in translation.map(timed).unwrap_or_default() {
        // (A translation of nothing is written "//".)
        if text.is_empty() || text == "//" {
            continue;
        }
        if let Some(line) = lines.iter_mut().find(|line| (line.at - at).abs() < 0.05) {
            line.translation = Some(text);
        }
    }
    lines
}

/// The lines of an LRC text as players keep it beside a track or in its tags: a line at the same time as the one
/// before it is that one's translation; words timed within a line (`<mm:ss.xx>` before each, as the "enhanced"
/// LRC times them, and after the last, where its end is said) are timed word by word.
pub fn lrc(text: &str) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    let offset = offset(text);
    for (at, said) in timed(text) {
        // (The words' times moved as the lines' are.)
        let (mut words, mut closed) = worded(&said);
        for (word, _) in &mut words {
            *word = (*word - offset).max(0.0);
        }
        closed = closed.map(|end| (end - offset).max(0.0));
        let plain = if words.is_empty() { said } else { words.iter().map(|(_, word)| word.as_str()).collect::<String>().trim().to_string() };
        if let Some(line) = lines.last_mut().filter(|line| line.at == at && !line.text.is_empty() && line.translation.is_none()) {
            if !plain.is_empty() {
                line.translation = Some(plain);
            }
            continue;
        }
        // Each word ends as the next begins; the last, where its end is said, else as the next line begins.
        let words = (0..words.len()).map(|i| (words[i].0, words.get(i + 1).map(|next| next.0).or(closed), words[i].1.clone())).collect::<Vec<_>>();
        let mut line = Line { at, end: None, text: plain, words: Vec::new(), backing: None, translation: None };
        line.words = words.into_iter().map(|(at, end, text)| Word { at, end: end.unwrap_or(f64::NAN), text }).collect();
        lines.push(line);
    }
    for i in 0..lines.len() {
        let next = lines.get(i + 1).map(|line| line.at);
        if let Some(last) = lines[i].words.last_mut().filter(|last| last.end.is_nan()) {
            last.end = next.unwrap_or(last.at).max(last.at);
        }
    }
    lines
}

/// The words timed in an LRC line (`<mm:ss.xx>` before each): when each begins and what it is (with the space
/// after it); and when the last ends, where a time follows it. None where no word is timed.
fn worded(said: &str) -> (Vec<(f64, String)>, Option<f64>) {
    let mut words: Vec<(f64, String)> = Vec::new();
    let mut rest = said;
    let mut closed = None;
    while let Some(open) = rest.find('<') {
        let Some((tag, after)) = rest[open + 1..].split_once('>') else { break };
        let Some(at) = time(tag) else { break };
        if let Some(last) = words.last_mut() {
            last.1.push_str(&rest[..open]);
        }
        let end = after.find('<').unwrap_or(after.len());
        if after[..end].is_empty() && end == after.len() {
            closed = Some(at);
            rest = "";
            break;
        }
        words.push((at, after[..end].to_string()));
        rest = &after[end..];
    }
    if let Some(last) = words.last_mut() {
        last.1.push_str(rest);
    }
    words.retain(|(_, word)| !word.is_empty());
    (words, closed)
}

/// Each timed line of an LRC text: when, and what (trimmed; empty where nothing is sung from then).
fn timed(lrc: &str) -> Vec<(f64, String)> {
    let mut found = Vec::new();
    let offset = offset(lrc);
    for line in lrc.replace("\\n", "\n").lines() {
        let mut rest = line.trim();
        let mut times = Vec::new();
        // Its times, one after another; a tag that is no time ("[ar:…]") says nothing sung.
        while let Some(tag) = rest.strip_prefix('[').and_then(|after| after.split_once(']')) {
            let Some(at) = time(tag.0) else { break };
            times.push((at - offset).max(0.0));
            rest = tag.1;
        }
        found.extend(times.into_iter().map(|at| (at, rest.trim().to_string())));
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found
}

/// What an LRC text says all its times are to be moved by, in seconds (`[offset:500]`: milliseconds, a positive one
/// showing every line that much sooner); none where it says none.
fn offset(lrc: &str) -> f64 {
    lrc.lines()
        .find_map(|line| line.trim().strip_prefix("[offset:").and_then(|after| after.split_once(']')).and_then(|(ms, _)| ms.trim().parse::<f64>().ok()))
        // (No more than a track is long, either way.)
        .filter(|ms| ms.abs() / 1000.0 <= LONGEST)
        .map_or(0.0, |ms| ms / 1000.0)
}

/// `mm:ss`, `mm:ss.xx` or `mm:ss:xx` as seconds.
fn time(tag: &str) -> Option<f64> {
    let (minutes, seconds) = tag.split_once(':')?;
    let seconds = seconds.replacen(':', ".", 1);
    let (minutes, seconds) = (minutes.trim().parse::<u32>().ok()?, seconds.trim().parse::<f64>().ok()?);
    (0.0..60.0).contains(&seconds).then(|| self::seconds(minutes as f64 * 60.0 + seconds)).flatten()
}

/// The line sung at `position` (seconds), sung a little early (a line shows as it is about to be sung, not
/// after): its place among `lines`; none before the first.
pub fn sung_at(lines: &[Line], position: f64) -> Option<usize> {
    lines.iter().rposition(|line| line.at <= position + EARLY)
}

/// How early a line is shown (seconds).
pub const EARLY: f64 = 0.4;

/// The lyrics found of the tracks asked about, kept on disk (a file each, in `folder`), so that a track played again
/// shows them at once without asking: lyrics timed word by word, for good; timed only line by line, for a month (a
/// source may come to have them word by word); that none has them, for a week; and what was found while a source
/// could not be reached (which may have had better), for a day. The most kept, the files written longest ago go
/// first.
pub mod kept {
    use super::*;

    const LINED_FOR: Duration = Duration::from_secs(30 * 24 * 3600);
    const NONE_FOR: Duration = Duration::from_secs(7 * 24 * 3600);
    const UNSURE_FOR: Duration = Duration::from_secs(24 * 3600);
    const KEPT_MOST: usize = 2000;
    /// The largest file read (larger: not one Glance wrote).
    const FILE_MOST: u64 = 4 << 20;

    /// A file: the track (as the name it is kept under could be another's too), when its lyrics were asked for
    /// (seconds since 1970), what was found, and whether each source asked answered.
    #[derive(Serialize, Deserialize)]
    struct File {
        track: Track,
        asked: u64,
        lines: Option<Vec<Line>>,
        sure: bool,
        /// How they were found and read (see `FOUND_BY`); none in what earlier versions kept.
        #[serde(default)]
        found_by: u32,
    }

    /// How lyrics are found and read, counted up as that changes: what was kept by an earlier way is not taken, and
    /// they are asked for again. (2: the artist of a track found by its title checked, an LRC file's offset read;
    /// what was kept before had another artist's song's lyrics, or every line late or early, for good. 3: NetEase's
    /// lines of who wrote and made a song kept. 4: those lines kept from NetEase's lyrics timed line by line too, and
    /// with lyrics timed word by word found elsewhere. 5: Kugou's, timed word by word, asked too, and lyrics timed
    /// word by word taken first wherever they are. 6: Apple Music's, timed word by word, asked too, and taken before
    /// Kugou's. 7: Apple Music's line-timed lyrics kept, and leading silence offset taken from its player.)
    const FOUND_BY: u32 = 7;

    /// A track's lyrics as kept (none: none had them), when they were asked for, and whether each source answered.
    pub struct Kept {
        pub lines: Option<Vec<Line>>,
        asked: SystemTime,
        sure: bool,
    }

    impl Kept {
        /// Whether they are still as the sources would now give them, so far as can be told.
        pub fn fresh(&self) -> bool {
            let age = SystemTime::now().duration_since(self.asked).unwrap_or_default();
            match &self.lines {
                _ if !self.sure => age < UNSURE_FOR,
                Some(lines) if lines.iter().any(|line| !line.words.is_empty()) => true,
                Some(_) => age < LINED_FOR,
                None => age < NONE_FOR,
            }
        }
    }

    /// Where they are kept: `lyrics` in Glance's local application data.
    pub fn folder() -> PathBuf {
        crate::settings::local_dir().join("lyrics")
    }

    /// The name `track`'s are kept under: a hash (64-bit FNV-1a, the same in every build) of all it is.
    pub(super) fn name(track: &Track) -> String {
        let hash = serde_json::to_string(track).unwrap_or_default().bytes().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| (hash ^ byte as u64).wrapping_mul(0x0100_0000_01b3));
        format!("{hash:016x}.json")
    }

    pub fn read(track: &Track) -> Option<Kept> {
        read_in(&folder(), track)
    }

    /// As `read`, from `folder`.
    pub(super) fn read_in(folder: &std::path::Path, track: &Track) -> Option<Kept> {
        let text = crate::elevation::read_in_place(folder, &name(track), FILE_MOST)?;
        let file: File = serde_json::from_str(&text).ok()?;
        // (Asked at a time no clock says: not kept.)
        let asked = SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(file.asked))?;
        (file.track == *track && file.found_by == FOUND_BY).then_some(Kept { lines: file.lines, asked, sure: file.sure })
    }

    /// Keeps what was found of `track`'s lyrics (none: none had them), asked for now, and whether each source
    /// answered; and, kept the most, lets the files written longest ago go. (Not kept where it cannot be written:
    /// they are asked for again.)
    pub fn write(track: &Track, lines: Option<&[Line]>, sure: bool) {
        write_in(&folder(), track, lines, sure);
    }

    /// As `write`, into `folder`.
    pub(super) fn write_in(folder: &std::path::Path, track: &Track, lines: Option<&[Line]>, sure: bool) {
        if folder.parent().is_none_or(|own| crate::elevation::ensure_folder(own).is_err()) || crate::elevation::ensure_folder(folder).is_err() {
            return;
        }
        let asked = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs();
        let file = File { track: track.clone(), asked, lines: lines.map(<[Line]>::to_vec), sure, found_by: FOUND_BY };
        let Ok(json) = serde_json::to_vec(&file) else { return };
        if crate::elevation::write_in_place(folder, &name(track), &json).is_ok() {
            trimmed(folder);
        }
    }

    fn trimmed(folder: &std::path::Path) {
        let Ok(entries) = std::fs::read_dir(folder) else { return };
        let mut files: Vec<(SystemTime, PathBuf)> = entries
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|extension| extension == "json"))
            .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
            .collect();
        if files.len() <= KEPT_MOST {
            return;
        }
        files.sort();
        for (_, path) in &files[..files.len() - KEPT_MOST] {
            let _ = std::fs::remove_file(path);
        }
    }

    /// The names of the files kept (for the uninstaller to take away).
    pub fn names() -> Vec<String> {
        std::fs::read_dir(folder()).map(|entries| entries.flatten().filter_map(|entry| entry.file_name().into_string().ok()).collect()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_lrc_with_its_translation() {
        let lrc = "[ar:Someone]\n[00:00.00] 作词 : 汪峰\\n[00:36.09]每当我感到疼痛 就想让你抱紧我\n[01:02:50][02:10.5]Again\n[03:00.00]\n";
        let lines = merged(lrc, Some("[00:36.09]Whenever I hurt\n[03:00.00]//"));
        let said: Vec<(f64, &str, Option<&str>)> = lines.iter().map(|l| (l.at, l.text.as_str(), l.translation.as_deref())).collect();
        assert_eq!(said, vec![(0.0, "作词 : 汪峰", None), (36.09, "每当我感到疼痛 就想让你抱紧我", Some("Whenever I hurt")), (62.5, "Again", None), (130.5, "Again", None), (180.0, "", None)]);
        assert_eq!(sung_at(&lines, 35.0), Some(0));
        assert_eq!(sung_at(&lines, 35.95), Some(1));
        assert_eq!(sung_at(&lines, 500.0), Some(4));
        assert_eq!(sung_at(&merged("[00:10.00]a", None), 1.0), None);
    }

    #[test]
    fn reads_a_local_lrc() {
        let lines = lrc("[ti:x]\n[00:01.00]第一行\n[00:01.00]First line\n[00:02.50]<00:02.50>Hel<00:03.00>lo <00:03.40>you<00:04.00>\n[00:05.00]<00:05.00>end\n[00:07.00]\n");
        let said: Vec<(f64, &str, Option<&str>)> = lines.iter().map(|l| (l.at, l.text.as_str(), l.translation.as_deref())).collect();
        assert_eq!(said, vec![(1.0, "第一行", Some("First line")), (2.5, "Hello you", None), (5.0, "end", None), (7.0, "", None)]);
        let words: Vec<(f64, f64, &str)> = lines[1].words.iter().map(|w| (w.at, w.end, w.text.as_str())).collect();
        assert_eq!(words, vec![(2.5, 3.0, "Hel"), (3.0, 3.4, "lo "), (3.4, 4.0, "you")]);
        assert_eq!((lines[2].words[0].at, lines[2].words[0].end), (5.0, 7.0));
    }

    /// A line whose head is not closed before its end is left out, and the lines after it read (it was cut from
    /// past its end, and Glance stopped: what the collection serves is not checked before it is read).
    #[test]
    fn a_line_not_closed_is_left_out() {
        let file = r#"<tt><p begin="00:00.5"</p><p begin="00:01.000" end="00:02.000"><span begin="00:01.000" end="00:02.000">kept</span></p></tt>"#;
        let lines = ttml(file);
        assert_eq!(lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>(), ["kept"]);
    }

    /// Lyrics with no line are held as none found, wherever they come from.
    #[test]
    fn lyrics_with_no_line_are_none() {
        assert!(matches!(found(Some(Vec::new())), Held::Found(None)));
        assert!(matches!(found(None), Held::Found(None)));
        let line = Line { at: 1.0, end: None, text: "kept".into(), words: Vec::new(), backing: None, translation: None };
        assert!(matches!(found(Some(vec![line])), Held::Found(Some(lines)) if lines.len() == 1));
    }

    /// A track found is by the artist asked about: one of its artists, however they are joined.
    #[test]
    fn a_track_found_is_by_its_artist() {
        assert!(by_artist(artists_in("Coldplay"), "coldplay"));
        assert!(by_artist(artists_in("Daft Punk feat. Pharrell Williams, Nile Rodgers"), "Pharrell Williams"));
        assert!(by_artist(artists_in("周杰伦/费玉清"), "费玉清"));
        assert!(by_artist(artists_in("AC/DC"), "AC/DC"));
        assert!(by_artist(artists_in("Simon & Garfunkel"), "Simon & Garfunkel"));
        assert!(!by_artist(artists_in("Some Cover Band"), "Coldplay"));
        assert!(by_artist(artists_in("Anyone"), ""));
    }

    /// A file's offset moves all its times, its words' too: a positive one shows each line that much sooner.
    #[test]
    fn an_offset_moves_every_line() {
        let lines = lrc("[offset:+500]\n[00:10.00]one\n[00:12.00]<00:12.00>two<00:13.00>");
        assert_eq!(lines.iter().map(|line| line.at).collect::<Vec<_>>(), [9.5, 11.5]);
        assert_eq!((lines[1].words[0].at, lines[1].words[0].end), (11.5, 12.5));
        assert_eq!(lrc("[00:10.00]one\n[offset:-250]")[0].at, 10.25);
    }

    #[test]
    fn reads_ttml_word_by_word() {
        let file = r#"<tt><body><div><p begin="00:22.402" end="00:26.962" ttm:agent="v1"><span begin="00:22.402" end="00:23.331">Hello</span> <span begin="00:25.274" end="00:25.619">wor</span><span begin="00:25.619" end="00:26.962">ld &amp; you</span><span ttm:role="x-bg"><span begin="00:26.0" end="00:26.5">(ooh)</span></span><span ttm:role="x-translation" xml:lang="zh-CN">你好世界</span></p><p begin="1:00:01.5" end="1:00:02"><span begin="3601.5" end="3602">late</span></p></div></body></tt>"#;
        let lines = ttml(file);
        assert_eq!(lines.len(), 2);
        let line = &lines[0];
        assert_eq!((line.at, line.end, line.text.as_str()), (22.402, Some(26.962), "Hello world & you"));
        assert_eq!(line.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["Hello ", "wor", "ld & you"]);
        assert_eq!((line.words[1].at, line.words[1].end), (25.274, 25.619));
        assert_eq!((line.backing.as_deref(), line.translation.as_deref()), (Some("(ooh)"), Some("你好世界")));
        assert_eq!(lines[1].at, 3601.5);
    }

    /// Kugou's KRC read word by word: each word's time from its line's start, the song's name with no length left
    /// out, its translation a line for each line.
    #[test]
    fn reads_kugou_word_by_word() {
        let translation = r#"{"content":[{"language":0,"type":0,"lyricContent":[["x"],["y"],["z"]]},{"language":0,"type":1,"lyricContent":[[""],["You fall from the sky"],["Drowned"]]}],"version":1}"#;
        let language: String = {
            const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            translation.as_bytes().chunks(3).flat_map(|chunk| {
                let whole = chunk.iter().enumerate().fold(0u32, |whole, (at, byte)| whole | u32::from(*byte) << (16 - 8 * at));
                (0..4).map(move |at| if at <= chunk.len() { DIGITS[(whole >> (18 - 6 * at) & 63) as usize] as char } else { '=' })
            }).collect()
        };
        let text = format!("[ti:完美降落]\n[language:{language}]\n[0,0]<0,0,0>胡期皓 - 完美降落\n[12000,2000]<0,500,0>你<500,500,0>从<1000,1000,0>天上\n[14500,1000]<0,1000,0>淹没\n");
        let lines = krc(&text);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "你从天上");
        assert_eq!((lines[0].at, lines[0].end), (12.0, Some(14.0)));
        assert_eq!(lines[0].words[2], Word { at: 13.0, end: 14.0, text: "天上".into() });
        assert_eq!(lines[0].translation.as_deref(), Some("You fall from the sky"));
        assert_eq!(lines[1].translation.as_deref(), Some("Drowned"));
        assert_eq!(base64_decoded("aGk="), Some(b"hi".to_vec()));
        assert_eq!(base64_decoded("aGk"), Some(b"hi".to_vec()));
        assert_eq!(base64_decoded("a"), None);
    }

    /// KRC with times no track has (a line at 1e300 ms, a word at as many past its line): the line left out in its place
    /// (the translation's lines still each its own), the word left out; every time kept one of a track. (Taken as
    /// they were, the next line's time from now could not be told as a duration, and Glance ended.)
    #[test]
    fn reads_kugou_times_of_a_track_only() {
        let translation = r#"{"content":[{"language":0,"type":1,"lyricContent":[["One"],["Two"],["Three"]]}],"version":1}"#;
        let language: String = {
            const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            translation.as_bytes().chunks(3).flat_map(|chunk| {
                let whole = chunk.iter().enumerate().fold(0u32, |whole, (at, byte)| whole | u32::from(*byte) << (16 - 8 * at));
                (0..4).map(move |at| if at <= chunk.len() { DIGITS[(whole >> (18 - 6 * at) & 63) as usize] as char } else { '=' })
            }).collect()
        };
        let text = format!("[language:{language}]
[1000,1000]<0,500,0>一<1e300,500,0>坏
[1e300,1000]<0,1000,0>二
[3000,1000]<0,1000,0>三
");
        let lines = krc(&text);
        assert_eq!(lines.iter().map(|line| (line.text.as_str(), line.translation.as_deref())).collect::<Vec<_>>(), vec![("一", Some("One")), ("三", Some("Three"))]);
        assert!(lines.iter().all(|line| line.at <= LONGEST && line.end.is_some_and(|end| end <= LONGEST) && line.words.iter().all(|word| word.end <= LONGEST)));
    }

    /// Lyrics timed word by word taken first, wherever they are: Apple Music's or Kugou's over NetEase's timed by
    /// line (with NetEase's translation), Apple Music's over Kugou's, over QQ Music's own where it plays, over
    /// LRCLIB's; NetEase's timed word by word over all of them; with none timed word by word, the player's own.
    #[test]
    fn takes_lyrics_timed_word_by_word_first() {
        let lined = |text: &str, translation: Option<&str>| vec![Line { at: 1.0, end: None, text: text.into(), words: Vec::new(), backing: None, translation: translation.map(str::to_string) }];
        let worded = |text: &str| vec![Line { at: 1.0, end: Some(2.0), text: text.into(), words: vec![Word { at: 1.0, end: 2.0, text: text.into() }], backing: None, translation: None }];
        let track = |netease: Option<u64>, qq_music: bool| Track { netease, qq_music, title: "t".into(), artist: "a".into(), album: String::new(), seconds: 100 };
        let text = |lines: Option<Vec<Line>>| lines.map(|lines| (lines[0].text.clone(), lines[0].translation.clone()));
        let given = |lines: Option<Vec<Line>>| move || lines;
        assert_eq!(text(chosen(&track(Some(1), false), Some(lined("netease", Some("译"))), given(None), given(Some(worded("kugou"))), given(None), given(None))), Some(("kugou".into(), Some("译".into()))));
        // (Apple Music's before Kugou's, and NetEase's translation theirs where they say none.)
        assert_eq!(text(chosen(&track(None, false), None, given(Some(worded("apple"))), given(Some(worded("kugou"))), given(None), given(None))), Some(("apple".into(), None)));
        assert_eq!(text(chosen(&track(None, false), Some(lined("netease", Some("译"))), given(Some(worded("apple"))), given(None), given(None), given(None))), Some(("apple".into(), Some("译".into()))));
        assert_eq!(text(chosen(&track(None, true), None, given(None), given(Some(worded("kugou"))), given(Some(lined("qq", None))), given(Some(lined("lrclib", None))))), Some(("kugou".into(), None)));
        assert_eq!(text(chosen(&track(Some(1), false), Some(lined("netease", None)), given(None), given(Some(lined("kugou", None))), given(None), given(Some(lined("lrclib", None))))), Some(("netease".into(), None)));
        assert_eq!(text(chosen(&track(None, true), Some(lined("netease", None)), given(None), given(None), given(Some(lined("qq", None))), given(Some(lined("lrclib", None))))), Some(("qq".into(), None)));
        assert_eq!(text(chosen(&track(None, false), Some(lined("netease", None)), given(None), given(None), given(Some(lined("qq", None))), given(Some(lined("lrclib", None))))), Some(("lrclib".into(), None)));
        // NetEase's timed word by word: none of the others waited for (one slow, or not reached, holds nothing up).
        let waited = || -> Option<Vec<Line>> { panic!("waited for") };
        assert_eq!(text(chosen(&track(None, false), Some(worded("netease")), waited, waited, waited, waited)), Some(("netease".into(), None)));
    }

    /// Kugou's lyrics as it gives them, sealed: opened and read (an ignored test: it asks Kugou).
    #[test]
    #[ignore = "asks Kugou on the network"]
    fn finds_kugou_word_by_word() {
        let track = Track { netease: None, qq_music: false, title: "完美降落".into(), artist: "胡期皓、叶琼琳、董唧唧".into(), album: String::new(), seconds: 137 };
        let lines = kugou(&track).ok().flatten().expect("Kugou reached, and its lyrics");
        let sung = lines.iter().find(|line| line.text.contains("你从天上降落")).expect("the line");
        assert!(sung.words.len() > 3, "{sung:?}");
        println!("{} lines, {:?}", lines.len(), sung);
    }

    /// Apple Music's lyrics as they are given, timed word by word (an ignored test: it asks Apple's catalog, and
    /// PaxSenix's service for its lyrics).
    #[test]
    #[ignore = "asks Apple Music on the network"]
    fn finds_apple_music_word_by_word() {
        let track = Track { netease: None, qq_music: false, title: "Shape of You".into(), artist: "Ed Sheeran".into(), album: String::new(), seconds: 234 };
        let lines = apple_music(&track).ok().flatten().expect("Apple Music reached, and its lyrics");
        let sung = lines.iter().find(|line| line.text.contains("I'm in love with the shape of you")).expect("the line");
        assert!(sung.words.len() > 3, "{sung:?}");
        println!("{} lines, {:?}", lines.len(), sung);
    }
    /// Apple Music's lyrics: where PaxSenix has none or is busy, dived down to NetEase Cloud Music with the
    /// canonical title and artist resolved from Apple's catalog in the preferred region.
    #[test]
    #[ignore = "asks Apple Music and NetEase on the network"]
    fn apple_music_dives_to_netease_on_luosu() {
        let track = Track { netease: None, qq_music: false, title: "落俗".into(), artist: "李荣浩 — 李榮浩".into(), album: String::new(), seconds: 267 };
        let lines = apple_music(&track).ok().flatten().expect("Apple Music or NetEase dive yielded lyrics");
        println!("Yielded {} lines (worded: {})", lines.len(), lines.iter().any(|l| !l.words.is_empty()));
        assert!(!lines.is_empty());
    }
    #[test]
    #[ignore = "asks Apple Music on the network"]
    fn finds_spiral_on_apple_music() {
        let track = Track { netease: None, qq_music: false, title: "Spiral".into(), artist: "明透 — ray of hope".into(), album: String::new(), seconds: 166 };
        let lines = apple_music(&track).ok().flatten().expect("Apple Music reached, and its lyrics");
        assert!(!lines.is_empty(), "has lyrics");
        let sung = lines.iter().find(|line| line.text.contains("愛しくなれば")).expect("the line");
        println!("{} lines, {:?}", lines.len(), sung);
    }

    /// The web player's token as Apple's page has it (an ignored test: it asks Apple for the page, and the script
    /// of it the token is in).
    #[test]
    #[ignore = "asks Apple on the network"]
    fn scrapes_apples_webplay_token() {
        let token = webplay_token().ok().expect("the player's token, taken from its page");
        println!("{} chars, good for {} days", token.len(), (WEBPLAY.lock().unwrap().as_ref().expect("held").1 - SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs()) / 86_400);
    }

    #[test]
    fn reads_netease_word_by_word() {
        let yrc = "{\"t\":0,\"c\":[{\"tx\":\"作词: \"},{\"tx\":\"某人\"}]}
[16210,3070](16210,670,0)Hello (16880,410,0)wor(17290,1990,0)ld
[20000,1000](20000,1000,0)Again
";
        let lines = word_timed(yrc, Some("[00:16.20]你好世界
[00:20.00]//"));
        assert_eq!(lines.len(), 3);
        // Who wrote it, as NetEase shows it, from its time.
        assert_eq!((lines[0].at, lines[0].text.as_str(), lines[0].words.len()), (0.0, "作词: 某人", 0));
        let lines = &lines[1..];
        let line = &lines[0];
        assert_eq!((line.at, line.end, line.text.as_str(), line.translation.as_deref()), (16.21, Some(19.28), "Hello world", Some("你好世界")));
        assert_eq!(line.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["Hello ", "wor", "ld"]);
        assert_eq!((line.words[1].at, line.words[1].end), (16.88, 17.29));
        assert_eq!((lines[1].text.as_str(), lines[1].translation.as_deref()), ("Again", None));
    }

    /// Kept in a folder of the test's own (the user's kept lyrics are not touched: written there, the oldest of them
    /// could be let go).
    #[test]
    fn keeps_lyrics_while_they_are_fresh() {
        let folder = std::env::temp_dir().join(format!("glance-kept-lyrics-{}", std::process::id())).join("lyrics");
        let track = Track { netease: Some(7), title: "kept test".into(), artist: "nobody".into(), album: String::new(), seconds: 1, qq_music: false };
        let worded = word_timed("[1000,500](1000,500,0)la", None);
        kept::write_in(&folder, &track, Some(&worded), true);
        let read = kept::read_in(&folder, &track).expect("kept");
        assert_eq!(read.lines.as_deref(), Some(&worded[..]));
        assert!(read.fresh());
        kept::write_in(&folder, &track, None, true);
        assert!(kept::read_in(&folder, &track).is_some_and(|kept| kept.lines.is_none() && kept.fresh()));
        let other = Track { seconds: 2, ..track.clone() };
        assert!(kept::read_in(&folder, &other).is_none());
        // Asked at a time no clock says: not kept (it was added to the clock's start, past what it holds).
        let file = std::fs::read_dir(&folder).unwrap().next().unwrap().unwrap().path();
        let mut said: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        said["asked"] = serde_json::json!(u64::MAX);
        std::fs::write(&file, serde_json::to_vec(&said).unwrap()).unwrap();
        assert!(kept::read_in(&folder, &track).is_none());
        let _ = std::fs::remove_dir_all(folder.parent().unwrap());
    }

    /// Times no track has are not taken (taken, a line's time came to more than a clock holds, and Glance ended); a
    /// bracket in what is sung is sung, not taken for a word's timing.
    #[test]
    fn reads_only_times_of_a_track() {
        assert!(word_timed("[1e309,1000](0,1000,0)Hello", None).is_empty());
        assert!(word_timed("[-5,1000](0,1000,0)Hello", None).is_empty());
        assert_eq!(word_timed("[0,2000](0,1000,0)Hello(1e300,1000,0)there", None)[0].text, "Hello(1e300,1000,0)there");
        assert!(word_timed(r#"{"t":1e300,"c":[{"tx":"作词: 某人"}]}"#, None).is_empty());
        assert!(lrc("[99999:00.00]Too late").is_empty());
        assert_eq!(lrc("[offset:1e300]\n[00:01.00]On time")[0].at, 1.0);
        let lines = word_timed("[0,2000](0,1000,0)Hello (1000,1000,0)(world)", None);
        assert_eq!(lines[0].words.iter().map(|word| word.text.as_str()).collect::<Vec<_>>(), ["Hello ", "(world)"]);
    }

    /// NetEase's lines of who wrote and made a song go among lyrics found elsewhere by their times, once each.
    #[test]
    fn credits_go_among_the_lines() {
        let line = |at: f64, text: &str| Line { at, end: None, text: text.into(), words: Vec::new(), backing: None, translation: None };
        let lines = vec![line(55.9, "Upset"), line(58.2, "Cold wet")];
        let credits = vec![line(0.0, "作词: 2KM"), line(1.0, "作曲: 2KM"), line(58.2, "Cold wet")];
        let with = with_credits(lines, &credits);
        assert_eq!(with.iter().map(|line| (line.at, line.text.as_str())).collect::<Vec<_>>(), vec![(0.0, "作词: 2KM"), (1.0, "作曲: 2KM"), (55.9, "Upset"), (58.2, "Cold wet")]);
    }

    /// A track NetEase has timed line by line, its credits written as JSON among the lines (network): they are kept,
    /// and handed apart for lyrics found elsewhere. `cargo test -- --ignored keeps_netease_credits --nocapture`.
    #[test]
    #[ignore = "asks NetEase over the network"]
    fn keeps_netease_credits() {
        let Ok((lines, credits)) = netease_and_credits(1359818052) else { panic!("NetEase answers") };
        let lines = lines.expect("its lyrics");
        assert!(credits.len() >= 10, "{credits:?}");
        assert_eq!(credits[0].text, "作词: 2KM");
        assert!(lines.iter().take(3).all(|line| line.at < 3.0 && line.text.contains(':')), "{:?}", &lines[..3]);
        assert!(lines.iter().any(|line| line.text == "Cold wet"));
    }

    #[test]
    fn escapes_a_query() {
        assert_eq!(escaped("我爱你 A&B"), "%E6%88%91%E7%88%B1%E4%BD%A0%20A%26B");
        assert!(same("Yellow ", "yel low"));
    }

    /// Base64 as JSON web tokens write its alphabet (`-`, `_` for `+`, `/`, none of the padding) read.
    #[test]
    fn reads_base64_as_json_web_tokens_write_it() {
        assert_eq!(base64_decoded("aGVsbG8="), Some(b"hello".to_vec()));
        assert_eq!(base64_decoded("aGVsbG8"), Some(b"hello".to_vec()));
        assert_eq!(base64_decoded("-_8"), base64_decoded("+/8="));
    }

    /// Of what a script of Apple's says, the player's tokens known by what they say: whose they are, and till when
    /// they are good.
    #[test]
    fn knows_the_players_token_by_what_it_says() {
        const HEAD: &str = "eyJ0eXAiOiJKV1QiLCJhbGciOiJFUzI1NiIsImtpZCI6IldlYlBsYXlLaWQifQ";
        const OURS: &str = "eyJpc3MiOiJBTVBXZWJQbGF5IiwiZXhwIjo0MDAwMDAwMDAwfQ";
        const THEIRS: &str = "eyJpc3MiOiI1SUtQUDJJRUNRIiwiZXhwIjo0MDAwMDAwMDAwfQ";
        const OUT: &str = "eyJpc3MiOiJBTVBXZWJQbGF5IiwiZXhwIjoxMDAwMDAwMDAwfQ";
        // (Among what surrounds a token in the script, the one of the player's, good till 4000000000.)
        let said = tokens_in(&format!("!function(){{var t=\"{HEAD}.{OURS}.5ign_atur-e\";{HEAD}.{THEIRS}.x;{HEAD}.{OUT}.y}}()"));
        assert_eq!(said.len(), 1);
        assert_eq!(said[0], (format!("{HEAD}.{OURS}.5ign_atur-e"), 4_000_000_000));
    }

    /// Asks the sources for two tracks (network): `cargo test -- --ignored finds_lyrics --nocapture`.
    #[test]
    #[ignore = "asks the lyrics sources over the network"]
    fn finds_lyrics() {
        let worded = amll(1901371647).ok().flatten().expect("the collection's, word by word");
        println!("AMLL: {} lines; {:?}", worded.len(), worded.iter().find(|line| line.words.len() > 2).map(|line| (&line.text, line.words.len(), &line.translation)));
        let by_id = find(&Track { netease: Some(2124385847), title: String::new(), artist: String::new(), album: String::new(), seconds: 330, qq_music: false }).found.expect("NetEase's, by number");
        println!("NetEase by number: {} lines, {} timed word by word, the second {:?}", by_id.len(), by_id.iter().filter(|line| !line.words.is_empty()).count(), by_id.get(1));
        let worded = netease(1974443814).ok().flatten().expect("NetEase's, word by word");
        println!("NetEase word by word: {} lines; {:?}", worded.len(), worded.iter().find(|line| line.words.len() > 2).map(|line| (&line.text, &line.words, &line.translation)));
        assert!(worded.iter().any(|line| line.words.len() > 2));
        let by_name = find(&Track { netease: None, title: "Yellow".into(), artist: "Coldplay".into(), album: "Parachutes".into(), seconds: 267, qq_music: false }).found.expect("LRCLIB's");
        println!("by name: {} lines, the first sung {:?}", by_name.len(), by_name.iter().find(|line| !line.text.is_empty()));
        let searched = netease_search(&Track { netease: None, title: "我爱你中国 (live)".into(), artist: "孙楠".into(), album: String::new(), seconds: 330, qq_music: false });
        println!("NetEase by its title: {:?}", searched.as_ref().ok());
        assert!(matches!(searched, Ok(Some(_))));
        let translated = netease_search(&Track { netease: None, title: "Yellow".into(), artist: "酷玩乐队".into(), album: String::new(), seconds: 267, qq_music: false });
        println!("NetEase by its artist's name in Chinese: {:?}", translated.as_ref().ok());
        assert!(matches!(translated, Ok(Some(_))));
        // (NetEase has this one, but without its words; LRCLIB has it not at all.)
        let qq_only = Track { netease: None, title: "牡丹亭".into(), artist: "浅影阿".into(), album: "牡丹亭".into(), seconds: 181, qq_music: true };
        let qq = qq_music(&qq_only).ok().flatten().expect("QQ Music's");
        println!("QQ Music: {} lines, {:?}", qq.len(), qq.iter().find(|line| line.text == "悬明月照屋庭"));
        assert!(qq.iter().any(|line| line.text == "悬明月照屋庭"));
        assert!(find(&qq_only).found.is_some_and(|lines| lines.iter().any(|line| line.text == "悬明月照屋庭")));
        let none = find(&Track { netease: None, title: "zzqqxx no such song".into(), artist: "qqqww".into(), album: String::new(), seconds: 100, qq_music: false });
        assert!(none.sure && none.found.is_none(), "that no source has a track's lyrics is known");
        assert!(matches!(netease(1), Ok(None)), "NetEase says it has none");
    }
}
