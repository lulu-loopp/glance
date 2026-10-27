//! The lyrics of what plays, each line with the time it is sung at (word by
//! word, where that is known), its translation where there is one, for what
//! shows them as the track plays. Asked for on the network, only while
//! lyrics are to be shown, by the track's number on NetEase Cloud Music
//! (where its player says it, as it does with the InfLink-rs plugin; else as
//! NetEase finds it by its title and length): of the AMLL TTML collection
//! (lyrics timed word by word, public domain, kept by that number, reached
//! through a CDN) and of NetEase itself; of Kugou (its lyrics timed word by
//! word, by the track's title, artist and length); and of LRCLIB (an open
//! collection of lyrics timed by line), by the track's title, artist, album
//! and length. The track's own lyrics are taken first, as surely as it can be
//! told which they are, timed word by word where another source's are of the
//! same words; none where it is played without words (see `chosen`).
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

/// What is known of a track's lyrics: still being asked for, found (none, where no source has them), or that it is
/// played without words.
#[derive(Clone, Debug, PartialEq)]
pub enum Known {
    Asking,
    Found(Option<Lyrics>),
    Instrumental,
}

/// What a source has of a track: its lyrics, or that it is played without words (NetEase's `pureMusic`, LRCLIB's
/// `instrumental`).
#[derive(Clone, Debug, PartialEq)]
pub enum Has {
    Sung(Vec<Line>),
    Instrumental,
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
    Instrumental,
    Unreached(Instant),
}

/// How long lyrics a source could not be reached for are taken to be none before they are asked for again.
const UNREACHED_FOR: Duration = Duration::from_secs(60);

/// Lyrics found, as they are held: none, where they have no line (a file kept on this PC may say so; shown as
/// lyrics, they were turned through from nothing, and Glance stopped).
fn found(lines: Option<Vec<Line>>) -> Held {
    Held::Found(lines.filter(|lines| !lines.is_empty()).map(Arc::new))
}

/// What a source has, as it is held (see `found`).
fn held(has: Option<Has>) -> Held {
    match has {
        Some(Has::Instrumental) => Held::Instrumental,
        Some(Has::Sung(lines)) => found(Some(lines)),
        None => found(None),
    }
}

/// What is known of `track`'s lyrics; asked for (on a thread of its own) the first time, and again a while after
/// a source could not be reached.
pub fn of(track: &Track) -> Known {
    let mut known = KNOWN.lock().unwrap();
    let known = known.get_or_insert_with(HashMap::new);
    match known.get(track) {
        Some(Held::Asking) => return Known::Asking,
        Some(Held::Found(found)) => return Known::Found(found.clone()),
        Some(Held::Instrumental) => return Known::Instrumental,
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
            Some(kept) if kept.fresh() => held(kept.found),
            kept => {
                let said = find(&track);
                // (That none has them, only where each source said so.)
                if said.sure || said.found.is_some() {
                    kept::write(&track, said.found.as_ref(), said.sure, said.doubted);
                }
                match (said.found, kept) {
                    (Some(has), _) => held(Some(has)),
                    // Not found now, a source unreached: what was found before, where there was.
                    (None, Some(kept)) if !said.sure => held(kept.found),
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

/// What the sources asked had of a track's lyrics, whether each answered (else they may yet have them), and whether
/// what was found rests on Kugou's alone (see `chosen`).
struct Said {
    found: Option<Has>,
    sure: bool,
    doubted: bool,
}

/// Asks the sources, all at once (one far away, slow or not reached, keeps none waiting for the others, and none is
/// waited for where what it has would not change what is taken): by the track's NetEase number, the word-timed
/// collection and NetEase itself (the collection's, with NetEase's translation where it gives none; else NetEase's;
/// none where NetEase says it is played without words); by its name, Kugou, LRCLIB, QQ Music (where its player gives
/// no NetEase number), and the same by the number NetEase finds it by. Which are taken, see `chosen`.
fn find(track: &Track) -> Said {
    use std::sync::atomic::{AtomicBool, Ordering};
    let sure = AtomicBool::new(true);
    // What a source has, none where it was not reached (and then what the sources have is not sure).
    let unreached = || sure.store(false, Ordering::Relaxed);
    let answered = |answer: Answer<Vec<Line>>| {
        answer.unwrap_or_else(|Unreached| {
            unreached();
            None
        })
    };
    let by_number = |id: u64| {
        let (worded, theirs) = std::thread::scope(|scope| {
            let theirs = scope.spawn(|| netease_and_credits(id));
            (answered(amll(id)), theirs.join().expect("NetEase asked"))
        });
        let (theirs, credits) = match theirs {
            Ok((has, credits)) => (has, credits),
            Err(Unreached) => (answered(Err(Unreached)).map(Has::Sung), Vec::new()),
        };
        match (worded, theirs) {
            // Played without words, as NetEase says: none (the collection keeps what it is given by NetEase's number).
            (_, Some(Has::Instrumental)) => Some(Has::Instrumental),
            // Timed word by word elsewhere: with NetEase's translation where they have none, and NetEase's lines of
            // who wrote and made the song (they have none: NetEase showed those lines, the lyrics nothing).
            (Some(mut lines), theirs) => {
                if let Some(Has::Sung(theirs)) = theirs.filter(|_| lines.iter().all(|line| line.translation.is_none())) {
                    translated(&mut lines, &theirs);
                }
                Some(Has::Sung(with_credits(lines, &credits)))
            }
            (None, theirs) => theirs,
        }
    };
    // (Each on a thread of its own, left to end on its own where its answer is not waited for.)
    fn asked<T: Send + 'static>(track: &Track, source: fn(&Track) -> Answer<T>) -> impl FnOnce() -> Answer<T> {
        let (told, answer) = std::sync::mpsc::channel();
        let track = track.clone();
        std::thread::spawn(move || {
            let _ = told.send(source(&track));
        });
        move || answer.recv().unwrap_or(Err(Unreached))
    }
    let by_kugou = asked(track, kugou);
    let by_name = asked(track, lrclib);
    // (Played by NetEase: its own first; QQ Music not asked.)
    let by_qq = track.netease.is_none().then(|| asked(track, qq_music));
    let netease = match track.netease {
        Some(id) => by_number(id),
        None => match netease_search(track) {
            Ok(id) => id.and_then(by_number),
            Err(Unreached) => answered(Err(Unreached)).map(Has::Sung),
        },
    };
    let (found, doubted) = chosen(track, netease, || answered(by_kugou()), || by_qq.and_then(|by_qq| answered(by_qq())), || {
        by_name().unwrap_or_else(|Unreached| {
            unreached();
            None
        })
    });
    Said { found, sure: sure.into_inner(), doubted }
}

/// Of what the sources had of `track`'s lyrics (NetEase's, the collection's among them; Kugou's; QQ Music's; LRCLIB's,
/// each waited for only once it is needed), what is taken, and whether it rests on Kugou's alone. The track's own
/// first, as surely as it can be told which they are: by the NetEase number its player gave, NetEase's (that it is
/// played without words, too); played in QQ Music, QQ Music's; then NetEase's timed word by word (of the track it
/// finds of that title, artist and length); then LRCLIB's (of that title, artist and length, its album too where it
/// has it), NetEase's, QQ Music's. Those timed line by line are timed word by word where Kugou's are of the same words
/// (Kugou's are its users', any song's put to any track: taken for the lines they agree with alone). Where none of
/// these has them, none where NetEase or LRCLIB says the track is played without words (where its player names its
/// artist: else what they found by name may be any artist's song of that title); else Kugou's, doubted.
fn chosen(track: &Track, netease: Option<Has>, kugou: impl FnOnce() -> Option<Vec<Line>>, qq: impl FnOnce() -> Option<Vec<Line>>, named: impl FnOnce() -> Option<Has>) -> (Option<Has>, bool) {
    use std::cell::LazyCell;
    let (kugou, qq, named) = (LazyCell::new(kugou), LazyCell::new(qq), LazyCell::new(named));
    let worded = |lines: &[Line]| lines.iter().any(|line| !line.words.is_empty());
    let sung = |has: &Option<Has>| match has {
        Some(Has::Sung(lines)) => Some(lines.clone()),
        _ => None,
    };
    let own = 'own: {
        if track.netease.is_some() {
            match &netease {
                Some(Has::Instrumental) => return (Some(Has::Instrumental), false),
                Some(Has::Sung(lines)) => break 'own Some(lines.clone()),
                None => {}
            }
        }
        if let Some(lines) = track.qq_music.then(|| (*qq).clone()).flatten() {
            break 'own Some(lines);
        }
        if let Some(lines) = sung(&netease).filter(|lines| worded(lines)) {
            break 'own Some(lines);
        }
        sung(&named).or_else(|| sung(&netease)).or_else(|| track.netease.is_none().then(|| (*qq).clone()).flatten())
    };
    match own {
        Some(lines) if worded(&lines) => (Some(Has::Sung(lines)), false),
        Some(lines) => match (*kugou).clone().filter(|theirs| worded(theirs) && agree(theirs, &lines)) {
            Some(mut theirs) => {
                if theirs.iter().all(|line| line.translation.is_none()) {
                    for other in std::iter::once(lines).chain(sung(&netease).filter(|other| agree(other, &theirs))) {
                        translated(&mut theirs, &other);
                    }
                }
                (Some(Has::Sung(theirs)), false)
            }
            None => (Some(Has::Sung(lines)), false),
        },
        None if !track.artist.trim().is_empty() && [&netease, &*named].iter().any(|has| matches!(has, Some(Has::Instrumental))) => (Some(Has::Instrumental), false),
        None => match (*kugou).clone().filter(|lines| worded(lines)) {
            Some(lines) => (Some(Has::Sung(lines)), true),
            None => (None, false),
        },
    }
}

/// Whether two lyrics are of the same words, so far as can be told: of the letters and characters each sings (case,
/// spaces and signs left out), most of the pairs one after another are the other's too (Sørensen–Dice, of their
/// bigrams; another song's, in the same language, shares few). Lines of who wrote and made it, one has and the other
/// not, leave them agreeing.
fn agree(a: &[Line], b: &[Line]) -> bool {
    let pairs = |lines: &[Line]| {
        let sung: Vec<char> = lines.iter().flat_map(|line| line.text.chars()).filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect();
        let mut pairs: HashMap<(char, char), usize> = HashMap::new();
        for pair in sung.windows(2) {
            *pairs.entry((pair[0], pair[1])).or_default() += 1;
        }
        pairs
    };
    let (a, b) = (pairs(a), pairs(b));
    let (count_a, count_b) = (a.values().sum::<usize>(), b.values().sum::<usize>());
    let shared: usize = a.iter().map(|(pair, n)| (*n).min(b.get(pair).copied().unwrap_or(0))).sum();
    count_a + count_b > 0 && 2 * shared * 10 >= AGREE_TENTHS * (count_a + count_b)
}

/// How much of two lyrics' pairs of characters must be shared for them to agree (tenths; see `agree`).
const AGREE_TENTHS: usize = 6;

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
        let (Some(at), end) = (attribute(head, "begin").and_then(clock), attribute(head, "end").and_then(clock)) else { continue };
        let (mut words, mut backing, mut translation) = (Vec::new(), String::new(), String::new());
        // What each open span is: a word (its times), or of a role.
        let mut open: Vec<(Option<String>, Option<(f64, f64)>)> = Vec::new();
        let mut body = body;
        while !body.is_empty() {
            let Some(tag) = body.find('<') else {
                text_into(&mut words, &mut backing, &mut translation, &open, body);
                break;
            };
            text_into(&mut words, &mut backing, &mut translation, &open, &body[..tag]);
            let Some(tag_end) = body[tag..].find('>') else { break };
            let inside = &body[tag + 1..tag + tag_end];
            body = &body[tag + tag_end + 1..];
            if inside.starts_with('/') {
                open.pop();
            } else if inside.starts_with("span") {
                let role = attribute(inside, "ttm:role").map(str::to_string);
                let times = attribute(inside, "begin").and_then(clock).zip(attribute(inside, "end").and_then(clock));
                if !inside.ends_with('/') {
                    open.push((role, times));
                }
            }
        }
        let text: String = words.iter().map(|word: &Word| word.text.as_str()).collect::<String>().trim().to_string();
        let said = |text: String| Some(text.trim().to_string()).filter(|text| !text.is_empty());
        lines.push(Line { at, end, text, words, backing: said(backing), translation: said(translation) });
    }
    lines.sort_by(|a, b| a.at.total_cmp(&b.at));
    lines
}

/// Text met inside a line: a word's (or the space after one), the backing vocals', or the translation's, as the
/// spans it is in say.
fn text_into(words: &mut Vec<Word>, backing: &mut String, translation: &mut String, open: &[(Option<String>, Option<(f64, f64)>)], text: &str) {
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
            // (Between words: the space after the last.)
            None => {
                if let Some(last) = words.last_mut() {
                    last.text.push_str(&text);
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

/// NetEase Cloud Music's lyrics of its track `id`, with their translation: timed word by word where it has them
/// (its `yrc`), else line by line; with its lines of who wrote and made the song either way (see `credit`). Or that
/// it is played without words, where NetEase says so (`pureMusic`: its lyrics then are none, or a line saying so).
#[cfg(test)]
fn netease(id: u64) -> Answer<Has> {
    netease_and_credits(id).map(|(has, _)| has)
}

/// As `netease`, and its lines of who wrote and made the song apart (for lyrics found elsewhere: see `find`).
fn netease_and_credits(id: u64) -> Result<(Option<Has>, Vec<Line>), Unreached> {
    let said = netease_said(&format!("https://music.163.com/api/song/lyric/v1?id={id}&lv=1&tv=-1&yv=1"))?;
    Ok(netease_read(&said))
}

/// What NetEase says of a track's lyrics (see `netease`), read.
fn netease_read(said: &serde_json::Value) -> (Option<Has>, Vec<Line>) {
    if said["pureMusic"].as_bool() == Some(true) {
        return (Some(Has::Instrumental), Vec::new());
    }
    let translation = said["tlyric"]["lyric"].as_str();
    let credits = |text: Option<&str>| text.map(|text| text.lines().filter_map(|raw| credit(raw.trim())).collect::<Vec<_>>()).unwrap_or_default();
    if let Some(lines) = said["yrc"]["lyric"].as_str().map(|yrc| word_timed(yrc, translation)).filter(|lines| lines.iter().any(|line| !line.words.is_empty())) {
        return (Some(Has::Sung(lines)), credits(said["yrc"]["lyric"].as_str()));
    }
    // Line by line: its lines of who wrote and made it are written among them as JSON (see `credit`), which the
    // LRC's times do not take (left out, the lyrics showed nothing while NetEase showed those lines).
    let lrc = said["lrc"]["lyric"].as_str();
    let credits = credits(lrc);
    let lines = lrc.map(|lrc| with_credits(merged(lrc, translation), &credits));
    (lines.filter(|lines| lines.iter().any(|line| !line.text.is_empty())).map(Has::Sung), credits)
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

/// The track on NetEase Cloud Music that is `track`: of those found by its title and artist, by its artist and of its
/// length (to two seconds), the first of its title (see `titled`).
fn netease_search(track: &Track) -> Answer<u64> {
    let asked = format!("{} {}", track.title, track.artist);
    let said = netease_said(&format!("https://music.163.com/api/search/get?s={}&type=1&limit=10", escaped(&asked)))?;
    let found = |artist: &str| {
        let songs = said["result"]["songs"].as_array().into_iter().flatten().filter_map(|song| {
            let length = song["duration"].as_f64()? / 1000.0;
            let artists = song["artists"].as_array().into_iter().flatten().filter_map(|artist| artist["name"].as_str());
            let by = by_artist(artists, artist);
            (by && (length - track.seconds as f64).abs() <= 2.0).then_some(())?;
            Some((titled(song["name"].as_str()?, &track.title)?, song["id"].as_u64()?))
        });
        songs.min_by_key(|(title, _)| *title).map(|(_, id)| id)
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

/// QQ Music's timed lyrics of `track`, with their translation: of the tracks it finds by title and artist, by its
/// artist and of its length (to two seconds), the first of its title (see `titled`). (Its API answers only as from
/// its own site.)
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
    let songs = said["data"]["song"]["list"].as_array().into_iter().flatten().filter_map(|song| {
        let artists = song["singer"].as_array().into_iter().flatten().filter_map(|singer| singer["name"].as_str());
        let length = song["interval"].as_f64()?;
        (by_artist(artists, &track.artist) && (length - track.seconds as f64).abs() <= 2.0).then_some(())?;
        Some((titled(song["songname"].as_str()?, &track.title)?, song["songmid"].as_str()?))
    });
    let found = songs.min_by_key(|(title, _)| *title).map(|(_, mid)| mid);
    let Some(mid) = found else { return Ok(None) };
    let lyric = format!("https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg?songmid={}&format=json&nobase64=1&g_tk=5381", escaped(mid));
    let Some(said) = asked(&lyric, "retcode")? else { return Ok(None) };
    let translation = said["trans"].as_str().filter(|trans| !trans.is_empty());
    Ok(said["lyric"].as_str().map(|lrc| merged(lrc, translation)).filter(|lines| lines.iter().any(|line| !line.text.is_empty())))
}

/// Kugou's lyrics of `track`, timed word by word (its KRC), with their translation where they have one: of the songs
/// it finds by title and artist, by that artist and of that length (to two seconds), the first of that title (its
/// highlighting left out, see `titled`); of the lyrics its users have given it for that song, the first of its length, its name and
/// one of its artists (that song's or the track's), whose file names no other (see `krc_of`). None where they are not
/// timed word by word (what is chosen from them, see `chosen`, are those). (Kugou gives lyrics of any song for a
/// track, those of another first, where its users have put them to it; its scores of them tell nothing.)
fn kugou(track: &Track) -> Answer<Vec<Line>> {
    // (Its mobile search: its web search answers from some of its servers with nothing found.)
    let searched = format!("https://mobiles.kugou.com/api/v3/search/song?format=json&keyword={}&page=1&pagesize=20&showtype=1", escaped(&format!("{} {}", track.title, track.artist)));
    let Some(said) = fetched_json(&searched)? else { return Ok(None) };
    let plain = |text: &str| text.replace("<em>", "").replace("</em>", "");
    let songs = said["data"]["info"].as_array().into_iter().flatten().filter_map(|song| {
        let singers = plain(song["singername"].as_str()?);
        let length = song["duration"].as_f64()?;
        (by_artist(artists_in(&singers), &track.artist) && (length - track.seconds as f64).abs() <= 2.0).then_some(())?;
        Some((titled(&plain(song["songname"].as_str()?), &track.title)?, song["hash"].as_str()?.to_string(), singers))
    });
    let Some((_, hash, singers)) = songs.min_by_key(|(title, ..)| *title) else { return Ok(None) };
    let artists: Vec<&str> = artists_in(&singers).into_iter().chain(Some(track.artist.as_str()).filter(|artist| !artist.trim().is_empty())).collect();
    let Some(kept) = fetched_json(&format!("https://krcs.kugou.com/search?ver=1&man=yes&client=mobi&keyword=&duration=&hash={}", escaped(&hash)))? else { return Ok(None) };
    let ms = f64::from(track.seconds) * 1000.0;
    for one in kept["candidates"].as_array().into_iter().flatten() {
        let of_it = one["duration"].as_f64().is_some_and(|length| (length - ms).abs() <= 2000.0)
            && one["song"].as_str().is_some_and(|song| titled(song, &track.title).is_some())
            && one["singer"].as_str().is_some_and(|singer| by_any(singer, &artists));
        let Some((id, key)) = Some(one).filter(|_| of_it).and_then(|one| Some((one["id"].as_str()?, one["accesskey"].as_str()?))) else { continue };
        let Some(lyric) = fetched_json(&format!("https://lyrics.kugou.com/download?ver=1&client=pc&id={}&accesskey={}&fmt=krc&charset=utf8", escaped(id), escaped(key)))? else { continue };
        if let Some(lines) = lyric["content"].as_str().and_then(krc_opened).and_then(|text| krc_of(&text, &track.title, &artists)) {
            return Ok(Some(lines));
        }
    }
    Ok(None)
}

/// Whether the artists `named` all in one (see `artists_in`) are, one of them, one of `artists`, but for case and
/// spaces.
fn by_any(named: &str, artists: &[&str]) -> bool {
    artists_in(named).into_iter().any(|theirs| artists.iter().any(|artist| same(theirs, artist)))
}

/// The lines of a KRC file (see `krc`), where it is of the song `title` by one of `artists`, so far as it says (its
/// `[ti:]` and `[ar:]`: Kugou's users have given it, under a song's name, files that name another) and they are timed
/// word by word.
fn krc_of(text: &str, title: &str, artists: &[&str]) -> Option<Vec<Line>> {
    let tag = |name: &str| text.lines().find_map(|line| line.trim().strip_prefix(name)?.strip_suffix(']').map(str::trim)).filter(|said| !said.is_empty());
    let titled = tag("[ti:").is_none_or(|said| titled(said, title).is_some());
    let by = tag("[ar:").is_none_or(|said| by_any(said, artists));
    Some(krc(text)).filter(|lines| titled && by && lines.iter().any(|line| !line.words.is_empty()))
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

/// `text` in base64 (RFC 4648, padded or not, spaces and line breaks passed over) as its bytes; none where it is not.
fn base64_decoded(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
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
    artist.trim().is_empty() || artists.into_iter().any(|theirs| same(theirs, artist))
}

/// The artists of a track as LRCLIB and Kugou name them, all in one ("A, B & C feat. D", "A、B"): the whole, and each
/// between the signs that join them (the whole kept: "AC/DC" and "Simon & Garfunkel" are one each).
fn artists_in(named: &str) -> Vec<&str> {
    let mut artists = vec![named];
    for between in [",", "、", "&", "/", ";", " feat. ", " ft. ", " x "] {
        artists = artists.into_iter().flat_map(|artist| artist.split(between)).collect();
    }
    std::iter::once(named).chain(artists).map(str::trim).filter(|artist| !artist.is_empty()).collect()
}

/// LRCLIB's timed lyrics of `track`, or that it is played without words: of the one it has of that title, artist,
/// album and length, and of those it finds by title and artist (see `lrclib_chosen`).
fn lrclib(track: &Track) -> Answer<Has> {
    let exact = format!(
        "https://lrclib.net/api/get?track_name={}&artist_name={}&album_name={}&duration={}",
        escaped(&track.title),
        escaped(&track.artist),
        escaped(&track.album),
        track.seconds
    );
    let exact = fetched_json(&exact)?;
    if let Some(Has::Sung(lines)) = lrclib_chosen(track, exact.iter()) {
        return Ok(Some(Has::Sung(lines)));
    }
    let search = format!("https://lrclib.net/api/search?track_name={}&artist_name={}", escaped(&track.title), escaped(&track.artist));
    let said = fetched_json(&search)?.unwrap_or_default();
    Ok(lrclib_chosen(track, exact.iter().chain(said.as_array().into_iter().flatten())))
}

/// Of LRCLIB's `records`, those of `track` (of its title, see `titled`; by its artist; of its length, to two seconds;
/// those of its whole title first, then those of its album): the first one's timed lyrics; else, where one says it is
/// played without words and none has words even untimed, that. (Anyone may give LRCLIB a record: one of them alone
/// is not taken over another of the track.)
fn lrclib_chosen<'a>(track: &Track, records: impl Iterator<Item = &'a serde_json::Value>) -> Option<Has> {
    let text = |one: &serde_json::Value, key: &str| one[key].as_str().unwrap_or_default().to_string();
    let mut records: Vec<(Titled, &serde_json::Value)> = records
        .filter(|one| by_artist(artists_in(&text(one, "artistName")), &track.artist))
        .filter(|one| one["duration"].as_f64().is_some_and(|length| (length - track.seconds as f64).abs() <= 2.0))
        .filter_map(|one| Some((titled(&text(one, "trackName"), &track.title)?, one)))
        .collect();
    records.sort_by_key(|(title, one)| (*title, !same(&text(one, "albumName"), &track.album)));
    let records: Vec<&serde_json::Value> = records.into_iter().map(|(_, one)| one).collect();
    let synced = |one: &&serde_json::Value| one["syncedLyrics"].as_str().map(|lrc| merged(lrc, None)).filter(|lines| lines.iter().any(|line| !line.text.is_empty()));
    if let Some(lines) = records.iter().find_map(synced) {
        return Some(Has::Sung(lines));
    }
    let worded = records.iter().any(|one| !text(one, "plainLyrics").trim().is_empty());
    (!worded && records.iter().any(|one| one["instrumental"].as_bool() == Some(true))).then_some(Has::Instrumental)
}

/// How a song a source names `theirs` is of the title `ours`: the same title, but for case and spaces; or the same as
/// `ours` without what it has in brackets at its end (a player's "后来的雨天(Always)" and "岁岁 (广播剧《…》主题曲)" are a
/// source's "后来的雨天" and "岁岁"); none, of another title. The first is taken over the second ("Song (Live)" is
/// another recording than "Song", told apart by its length where both are found). What the source's has in brackets
/// is kept: its "岁岁 (伴奏)", as long as the song, is the song without its words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Titled {
    Whole,
    Bare,
}

fn titled(theirs: &str, ours: &str) -> Option<Titled> {
    if same(theirs, ours) {
        Some(Titled::Whole)
    } else if same(theirs, bare_title(ours)) {
        Some(Titled::Bare)
    } else {
        None
    }
}

/// `title` without what it has in brackets at its end, each pair of brackets with all inside it (pairs in pairs too):
/// "A (B (C)) [D]" is "A". The whole, where nothing is left without them.
fn bare_title(title: &str) -> &str {
    const PAIRS: [(char, char); 4] = [('(', ')'), ('（', '）'), ('[', ']'), ('【', '】')];
    let mut bare = title.trim_end();
    while let Some((open, close)) = bare.chars().next_back().and_then(|last| PAIRS.into_iter().find(|(_, close)| *close == last)) {
        let mut depth = 0;
        let Some(start) = bare.char_indices().rev().find_map(|(at, c)| {
            if c == close {
                depth += 1;
            } else if c == open {
                depth -= 1;
            }
            (depth == 0).then_some(at)
        }) else {
            break;
        };
        bare = bare[..start].trim_end();
    }
    if bare.is_empty() { title } else { bare }
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

/// The line sung at `position` (seconds), shown `early` (a line shows as it is about to be sung, not after: see
/// `EARLY`): its place among `lines`; none before the first.
pub fn sung_at(lines: &[Line], position: f64, early: f64) -> Option<usize> {
    lines.iter().rposition(|line| line.at <= position + early)
}

/// How early a line is shown (seconds), but where its player shows lyrics itself, as early as it shows them (see
/// `media::Playing::lines_ahead`): beside them, a line shown after the player's was late.
pub const EARLY: f64 = 0.2;

/// The lyrics found of the tracks asked about, kept on disk (a file each, in `folder`), so that a track played again
/// shows them at once without asking: lyrics timed word by word, for good; timed only line by line, for a month (a
/// source may come to have them word by word); that none has them, or that it is played without words, and Kugou's
/// taken where none else had them (its users' to change), for a week; and what was found while a source could not
/// be reached (which may have had better), for a day. The most kept, the files written longest ago go first.
pub mod kept {
    use super::*;

    const LINED_FOR: Duration = Duration::from_secs(30 * 24 * 3600);
    const NONE_FOR: Duration = Duration::from_secs(7 * 24 * 3600);
    const UNSURE_FOR: Duration = Duration::from_secs(24 * 3600);
    const KEPT_MOST: usize = 2000;
    /// The largest file read (larger: not one Glance wrote).
    const FILE_MOST: u64 = 4 << 20;

    /// A file: the track (as the name it is kept under could be another's too), when its lyrics were asked for
    /// (seconds since 1970), what was found (none, and whether that is as it is played without words), whether each
    /// source asked answered, and whether what was found rests on Kugou's alone.
    #[derive(Serialize, Deserialize)]
    struct File {
        track: Track,
        asked: u64,
        lines: Option<Vec<Line>>,
        #[serde(default)]
        instrumental: bool,
        sure: bool,
        #[serde(default)]
        doubted: bool,
        /// How they were found and read (see `FOUND_BY`); none in what earlier versions kept.
        #[serde(default)]
        found_by: u32,
    }

    /// How lyrics are found and read, counted up as that changes: what was kept by an earlier way is not taken, and
    /// they are asked for again. (2: the artist of a track found by its title checked, an LRC file's offset read;
    /// what was kept before had another artist's song's lyrics, or every line late or early, for good. 3: NetEase's
    /// lines of who wrote and made a song kept. 4: those lines kept from NetEase's lyrics timed line by line too, and
    /// with lyrics timed word by word found elsewhere. 5: Kugou's, timed word by word, asked too, and lyrics timed
    /// word by word taken first wherever they are. 6: the track's own taken first, Kugou's only of its words or where
    /// none has them, and that a track is played without words kept; what was kept before had another song's
    /// lyrics, Kugou's, for good. 7: a track's title found without what it has in brackets at its end, where no
    /// song has it whole; what was kept before had none for it, for a while.)
    const FOUND_BY: u32 = 7;

    /// A track's lyrics as kept (none: none had them), when they were asked for, whether each source answered, and
    /// whether they rest on Kugou's alone.
    pub struct Kept {
        pub found: Option<Has>,
        asked: SystemTime,
        sure: bool,
        doubted: bool,
    }

    impl Kept {
        /// Whether they are still as the sources would now give them, so far as can be told.
        pub fn fresh(&self) -> bool {
            let age = SystemTime::now().duration_since(self.asked).unwrap_or_default();
            match &self.found {
                _ if !self.sure => age < UNSURE_FOR,
                _ if self.doubted => age < NONE_FOR,
                Some(Has::Sung(lines)) if lines.iter().any(|line| !line.words.is_empty()) => true,
                Some(Has::Sung(_)) => age < LINED_FOR,
                Some(Has::Instrumental) | None => age < NONE_FOR,
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
        let found = match file.lines {
            _ if file.instrumental => Some(Has::Instrumental),
            lines => lines.map(Has::Sung),
        };
        (file.track == *track && file.found_by == FOUND_BY).then_some(Kept { found, asked, sure: file.sure, doubted: file.doubted })
    }

    /// Keeps what was found of `track`'s lyrics (none: none had them), asked for now, whether each source answered,
    /// and whether it rests on Kugou's alone; and, kept the most, lets the files written longest ago go. (Not kept
    /// where it cannot be written: they are asked for again.)
    pub fn write(track: &Track, found: Option<&Has>, sure: bool, doubted: bool) {
        write_in(&folder(), track, found, sure, doubted);
    }

    /// As `write`, into `folder`.
    pub(super) fn write_in(folder: &std::path::Path, track: &Track, found: Option<&Has>, sure: bool, doubted: bool) {
        if folder.parent().is_none_or(|own| crate::elevation::ensure_folder(own).is_err()) || crate::elevation::ensure_folder(folder).is_err() {
            return;
        }
        let asked = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs();
        let lines = match found {
            Some(Has::Sung(lines)) => Some(lines.clone()),
            _ => None,
        };
        let file = File { track: track.clone(), asked, lines, instrumental: found == Some(&Has::Instrumental), sure, doubted, found_by: FOUND_BY };
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
        assert_eq!(sung_at(&lines, 35.0, EARLY), Some(0));
        assert_eq!(sung_at(&lines, 35.95, EARLY), Some(1));
        // (Shown as early as its player shows it, where that is earlier: Apple Music's, 0.35 s.)
        assert_eq!(sung_at(&lines, 35.8, EARLY), Some(0));
        assert_eq!(sung_at(&lines, 35.8, 0.35), Some(1));
        assert_eq!(sung_at(&lines, 500.0, EARLY), Some(4));
        assert_eq!(sung_at(&merged("[00:10.00]a", None), 1.0, EARLY), None);
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

    /// Lines of `text`, one a second, timed by line (with `translation` of each, where given) or word by word.
    fn lines_of(text: &[&str], translation: Option<&str>, worded: bool) -> Vec<Line> {
        let line = |at: usize, text: &str| {
            let at = at as f64;
            let words = if worded { vec![Word { at, end: at + 1.0, text: text.into() }] } else { Vec::new() };
            Line { at, end: worded.then_some(at + 1.0), text: text.into(), words, backing: None, translation: translation.map(str::to_string) }
        };
        text.iter().enumerate().map(|(at, text)| line(at, text)).collect()
    }

    const STARS: &[&str] = &["Look at the stars", "Look how they shine for you", "And everything you do"];
    const DRUGS: &[&str] = &["The drugs begin to take effect", "A smile of joy is on my face", "I cannot feel my hands"];

    fn a_track(netease: Option<u64>, qq_music: bool, artist: &str) -> Track {
        Track { netease, qq_music, title: "t".into(), artist: artist.into(), album: String::new(), seconds: 100 }
    }

    /// What `chosen` took: the first line's text and translation, whether it is timed word by word, and whether it was
    /// doubted; or that it is played without words.
    fn taken(chosen: (Option<Has>, bool)) -> Option<(String, Option<String>, bool, bool)> {
        match chosen {
            (Some(Has::Sung(lines)), doubted) => Some((lines[0].text.clone(), lines[0].translation.clone(), !lines[0].words.is_empty(), doubted)),
            (Some(Has::Instrumental), _) => Some(("instrumental".into(), None, false, false)),
            (None, _) => None,
        }
    }

    fn given<T>(has: T) -> impl FnOnce() -> T {
        move || has
    }

    fn waited<T>() -> T {
        panic!("waited for")
    }

    /// That a track is played without words is taken where NetEase says so of the number its player gave, the
    /// collection's and Kugou's lyrics not waited for; and where NetEase (found by name) or LRCLIB says so and none of
    /// the track's own has words: not Kugou's. (Kugou gave another song's lyrics for a piano piece: shown, for good.)
    #[test]
    fn a_track_played_without_words_shows_none() {
        let instrumental = Some(("instrumental".to_string(), None, false, false));
        let artist = "Pianoboy高至豪";
        assert_eq!(taken(chosen(&a_track(Some(139774), false, artist), Some(Has::Instrumental), waited, waited, waited)), instrumental);
        let junk = || Some(lines_of(DRUGS, None, true));
        assert_eq!(taken(chosen(&a_track(None, false, artist), Some(Has::Instrumental), junk, given(None), given(Some(Has::Instrumental)))), instrumental);
        assert_eq!(taken(chosen(&a_track(None, false, artist), None, junk, given(None), given(Some(Has::Instrumental)))), instrumental);
        assert_eq!(taken(chosen(&a_track(Some(1), false, artist), None, junk, waited, given(Some(Has::Instrumental)))), instrumental);
        // Words found by name of the track elsewhere: those (one source's word against another's, the words shown).
        let lrclib = Some(Has::Sung(lines_of(STARS, None, false)));
        assert_eq!(taken(chosen(&a_track(None, false, artist), Some(Has::Instrumental), given(None), given(None), given(lrclib))).map(|taken| taken.0), Some("Look at the stars".into()));
        // A player that names no artist: what was found by name may be any artist's song of that title.
        assert_eq!(taken(chosen(&a_track(None, false, ""), None, junk, given(None), given(Some(Has::Instrumental)))), Some(("The drugs begin to take effect".into(), None, true, true)));
    }

    /// The track's own lyrics first, as surely as it can be told which they are: those timed only line by line are
    /// timed word by word where Kugou's are of the same words (with their translation), and not otherwise.
    #[test]
    fn the_tracks_own_lyrics_come_first() {
        let lined = |text: &'static [&'static str], translation: Option<&str>| Some(lines_of(text, translation, false));
        let worded = |text: &'static [&'static str]| Some(lines_of(text, None, true));
        let sung = |lines: Option<Vec<Line>>| lines.map(Has::Sung);
        let stars = |translation: Option<&str>, worded: bool, doubted: bool| Some(("Look at the stars".to_string(), translation.map(str::to_string), worded, doubted));
        // By the player's number: NetEase's, timed word by word with Kugou's where they agree.
        assert_eq!(taken(chosen(&a_track(Some(1), false, "a"), sung(lined(STARS, Some("看星星"))), given(worded(STARS)), waited, given(None))), stars(Some("看星星"), true, false));
        assert_eq!(taken(chosen(&a_track(Some(1), false, "a"), sung(lined(STARS, None)), given(worded(DRUGS)), waited, given(None))), stars(None, false, false));
        // NetEase's timed word by word: the others not waited for.
        assert_eq!(taken(chosen(&a_track(Some(1), false, "a"), sung(worded(STARS)), waited, waited, waited)), stars(None, true, false));
        assert_eq!(taken(chosen(&a_track(None, false, "a"), sung(worded(STARS)), waited, waited, waited)), stars(None, true, false));
        // Played in QQ Music: its own, over another song's words from Kugou.
        assert_eq!(taken(chosen(&a_track(None, true, "a"), None, given(worded(DRUGS)), given(lined(STARS, None)), given(None))), stars(None, false, false));
        assert_eq!(taken(chosen(&a_track(None, true, "a"), sung(lined(DRUGS, None)), given(None), given(lined(STARS, None)), given(sung(lined(DRUGS, None))))), stars(None, false, false));
        // LRCLIB's, over NetEase's found by name, over QQ Music's; Kugou's of other words not taken over them.
        let drugs_lined = sung(lined(DRUGS, None));
        assert_eq!(taken(chosen(&a_track(None, false, "a"), drugs_lined.clone(), given(worded(DRUGS)), given(None), given(sung(lined(STARS, None))))), stars(None, false, false));
        assert_eq!(taken(chosen(&a_track(None, false, "a"), sung(lined(STARS, None)), given(None), given(lined(DRUGS, None)), given(None))), stars(None, false, false));
        assert_eq!(taken(chosen(&a_track(None, false, "a"), None, given(None), given(lined(STARS, None)), given(None))), stars(None, false, false));
        // Kugou's taken by its words with the translation of a source of the same words alone.
        assert_eq!(taken(chosen(&a_track(None, false, "a"), sung(lined(DRUGS, Some("药"))), given(worded(STARS)), given(None), given(sung(lined(STARS, None))))), stars(None, true, false));
        // None of the track's own: Kugou's, doubted.
        assert_eq!(taken(chosen(&a_track(None, false, "a"), None, given(worded(STARS)), given(None), given(None))), stars(None, true, true));
        assert_eq!(taken(chosen(&a_track(None, false, "a"), None, given(None), given(None), given(None))), None);
    }

    /// Lyrics agree where they are of the same words: lines of who wrote it one has alone, case, spaces and signs
    /// left aside; another song's words do not.
    #[test]
    fn lyrics_of_the_same_words_agree() {
        let with_credits: Vec<&str> = ["作词: Chris Martin", "作曲: Coldplay"].into_iter().chain(STARS.iter().copied()).collect();
        let shouted: Vec<String> = STARS.iter().map(|line| format!("{}!", line.to_uppercase().replace(' ', "  "))).collect();
        let shouted: Vec<&str> = shouted.iter().map(String::as_str).collect();
        assert!(agree(&lines_of(STARS, None, true), &lines_of(&with_credits, None, false)));
        assert!(agree(&lines_of(&shouted, None, true), &lines_of(STARS, None, false)));
        assert!(!agree(&lines_of(DRUGS, None, true), &lines_of(STARS, None, false)));
        assert!(!agree(&lines_of(&["你从天上降落", "淹没我"], None, true), &lines_of(&["豆花之歌", "甜甜的豆花"], None, false)));
        assert!(!agree(&[], &[]));
    }

    /// NetEase saying a track is played without words (its `pureMusic`, as it answers for 139774) is read as that,
    /// whatever line it gives for lyrics.
    #[test]
    fn netease_says_a_track_is_played_without_words() {
        let said = serde_json::json!({"pureMusic": true, "lrc": {"version": 1, "lyric": "[00:00.000] 纯音乐，请欣赏\n"}, "code": 200});
        assert_eq!(netease_read(&said).0, Some(Has::Instrumental));
        let sung = serde_json::json!({"lrc": {"lyric": "[00:01.00]Look at the stars"}, "code": 200});
        assert!(matches!(netease_read(&sung).0, Some(Has::Sung(lines)) if lines[0].text == "Look at the stars"));
    }

    /// LRCLIB's records of the track alone, by its title (another version's is another's), its artist and its length:
    /// the timed lyrics of one, those of its album first; that it is played without words only where none has words.
    #[test]
    fn lrclib_records_of_the_track_alone() {
        let track = Track { netease: None, qq_music: false, title: "Song".into(), artist: "Band".into(), album: "Second".into(), seconds: 200 };
        let record = |title: &str, artist: &str, album: &str, seconds: f64, synced: Option<&str>, plain: Option<&str>, instrumental: bool| {
            serde_json::json!({"trackName": title, "artistName": artist, "albumName": album, "duration": seconds, "syncedLyrics": synced, "plainLyrics": plain, "instrumental": instrumental})
        };
        let first = |has: Option<Has>| match has {
            Some(Has::Sung(lines)) => Some(lines[0].text.clone()),
            Some(Has::Instrumental) => Some("instrumental".into()),
            None => None,
        };
        let pick = |records: Vec<serde_json::Value>| first(lrclib_chosen(&track, records.iter()));
        // Another version's, another artist's, another length's: none.
        assert_eq!(pick(vec![record("Song (Instrumental)", "Band", "", 200.0, None, None, true)]), None);
        assert_eq!(pick(vec![record("Song", "Other", "", 200.0, Some("[00:01.00]theirs"), None, false)]), None);
        assert_eq!(pick(vec![record("Song", "Band", "", 140.0, Some("[00:01.00]short"), None, false)]), None);
        // Timed lyrics over a record saying it is played without words; of its album first.
        assert_eq!(pick(vec![record("Song", "Band", "", 200.0, None, None, true), record("song", "Band & Friend", "", 201.0, Some("[00:01.00]sung"), None, false)]), Some("sung".into()));
        assert_eq!(pick(vec![record("Song", "Band", "First", 200.0, Some("[00:01.00]first"), None, false), record("Song", "Band", "Second", 200.0, Some("[00:01.00]second"), None, false)]), Some("second".into()));
        // Played without words: only where none has words, even untimed.
        assert_eq!(pick(vec![record("Song", "Band", "", 200.0, None, None, true)]), Some("instrumental".into()));
        assert_eq!(pick(vec![record("Song", "Band", "", 200.0, None, None, true), record("Song", "Band", "", 200.0, None, Some("words"), false)]), None);
    }

    /// A KRC file is taken only where it names the song and one of its artists, where it names them (Kugou's users put
    /// "豆花之歌" under "The Truth That You Leave").
    #[test]
    fn a_kugou_file_naming_another_song_is_not_taken() {
        let body = "[1000,1000]<0,1000,0>sung\n";
        let artists = ["Pianoboy高至豪", "Pianoboy"];
        assert!(krc_of(&format!("[ti:The Truth That You Leave]\n[ar:pianoboy]\n{body}"), "The Truth That You Leave", &artists).is_some());
        assert!(krc_of(body, "The Truth That You Leave", &artists).is_some());
        assert!(krc_of(&format!("[ti:豆花之歌]\n[ar:锦零]\n{body}"), "The Truth That You Leave", &artists).is_none());
        assert!(krc_of(&format!("[ti:The Truth That You Leave]\n[ar:arkady sevidov]\n{body}"), "The Truth That You Leave", &artists).is_none());
        assert!(krc_of("[ti:The Truth That You Leave]\n[1000,1000]sung\n", "The Truth That You Leave", &artists).is_none());
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
        // Found by name (its first artist, as its player names it): timed word by word, by Kugou's of the same words.
        let Some(Has::Sung(found)) = find(&Track { artist: "胡期皓".into(), ..track }).found else { panic!("its lyrics") };
        assert!(found.iter().any(|line| line.text.contains("你从天上降落") && line.words.len() > 3), "{:?}", &found[..3]);
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
        let worded = Has::Sung(worded);
        kept::write_in(&folder, &track, Some(&worded), true, false);
        let read = kept::read_in(&folder, &track).expect("kept");
        assert_eq!(read.found.as_ref(), Some(&worded));
        assert!(read.fresh());
        kept::write_in(&folder, &track, None, true, false);
        assert!(kept::read_in(&folder, &track).is_some_and(|kept| kept.found.is_none() && kept.fresh()));
        kept::write_in(&folder, &track, Some(&Has::Instrumental), true, false);
        assert!(kept::read_in(&folder, &track).is_some_and(|kept| kept.found == Some(Has::Instrumental) && kept.fresh()));
        // Kugou's alone, timed word by word: for a week, not for good.
        let file = std::fs::read_dir(&folder).unwrap().next().unwrap().unwrap().path();
        let asked_days_ago = |days: u64| {
            let mut said: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
            said["asked"] = serde_json::json!(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs() - days * 24 * 3600);
            std::fs::write(&file, serde_json::to_vec(&said).unwrap()).unwrap();
        };
        kept::write_in(&folder, &track, Some(&worded), true, true);
        assert!(kept::read_in(&folder, &track).is_some_and(|kept| kept.found.as_ref() == Some(&worded) && kept.fresh()));
        asked_days_ago(8);
        assert!(kept::read_in(&folder, &track).is_some_and(|kept| !kept.fresh()));
        kept::write_in(&folder, &track, Some(&worded), true, false);
        asked_days_ago(400);
        assert!(kept::read_in(&folder, &track).is_some_and(|kept| kept.fresh()));
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
        let Ok((Some(Has::Sung(lines)), credits)) = netease_and_credits(1359818052) else { panic!("NetEase answers, with its lyrics") };
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

    /// A player's title with a name or note in brackets at its end is a source's of the title before them; one the
    /// same whole is taken first.
    #[test]
    fn a_title_is_known_without_its_brackets() {
        assert_eq!(bare_title("后来的雨天(Always)"), "后来的雨天");
        assert_eq!(bare_title("岁岁 (广播剧《在古代上学的日子》主题曲)"), "岁岁");
        assert_eq!(bare_title("A (B (C)) [D]"), "A");
        assert_eq!(bare_title("A（B）【C】"), "A");
        assert_eq!(bare_title("(Intro)"), "(Intro)");
        assert_eq!(bare_title("A) B"), "A) B");
        assert_eq!(bare_title("Mr. (B"), "Mr. (B");
        assert_eq!(titled("后来的雨天", "后来的雨天(Always)"), Some(Titled::Bare));
        assert_eq!(titled("岁岁", "岁岁 (广播剧《在古代上学的日子》主题曲)"), Some(Titled::Bare));
        assert_eq!(titled("我爱你中国 (Live)", "我爱你中国 (live)"), Some(Titled::Whole));
        assert_eq!(titled("岁岁 (伴奏)", "岁岁 (广播剧《在古代上学的日子》主题曲)"), None);
        assert_eq!(titled("岁岁 (伴奏)", "岁岁"), None);
        assert_eq!(titled("Yellow", "Yellow"), Some(Titled::Whole));
        assert_eq!(titled("Yellow Submarine", "Yellow"), None);
        assert!(Titled::Whole < Titled::Bare);
        // Of LRCLIB's records of the length: the whole title's first, though another's album is the track's.
        let track = Track { netease: None, title: "Song (Live)".into(), artist: "A".into(), album: "Album".into(), seconds: 200, qq_music: false };
        let record = |name: &str, album: &str, line: &str| serde_json::json!({ "trackName": name, "artistName": "A", "albumName": album, "duration": 200.0, "syncedLyrics": format!("[00:01.00]{line}") });
        let records = [record("Song", "Album", "bare"), record("Song (Live)", "Other", "whole")];
        let Some(Has::Sung(lines)) = lrclib_chosen(&track, records.iter()) else { panic!("none chosen") };
        assert_eq!(lines[0].text, "whole");
        let Some(Has::Sung(lines)) = lrclib_chosen(&Track { title: "Song (Remastered)".into(), ..track }, records[..1].iter()) else { panic!("none chosen") };
        assert_eq!(lines[0].text, "bare");
    }

    /// Asks the sources for two tracks (network): `cargo test -- --ignored finds_lyrics --nocapture`.
    #[test]
    #[ignore = "asks the lyrics sources over the network"]
    fn finds_lyrics() {
        let worded = amll(1901371647).ok().flatten().expect("the collection's, word by word");
        println!("AMLL: {} lines; {:?}", worded.len(), worded.iter().find(|line| line.words.len() > 2).map(|line| (&line.text, line.words.len(), &line.translation)));
        let by_id = find(&Track { netease: Some(2124385847), title: String::new(), artist: String::new(), album: String::new(), seconds: 330, qq_music: false }).found.and_then(sung_lines).expect("NetEase's, by number");
        println!("NetEase by number: {} lines, {} timed word by word, the second {:?}", by_id.len(), by_id.iter().filter(|line| !line.words.is_empty()).count(), by_id.get(1));
        let worded = netease(1974443814).ok().flatten().and_then(sung_lines).expect("NetEase's, word by word");
        println!("NetEase word by word: {} lines; {:?}", worded.len(), worded.iter().find(|line| line.words.len() > 2).map(|line| (&line.text, &line.words, &line.translation)));
        assert!(worded.iter().any(|line| line.words.len() > 2));
        let by_name = find(&Track { netease: None, title: "Yellow".into(), artist: "Coldplay".into(), album: "Parachutes".into(), seconds: 267, qq_music: false }).found.and_then(sung_lines).expect("LRCLIB's");
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
        assert!(find(&qq_only).found.and_then(sung_lines).is_some_and(|lines| lines.iter().any(|line| line.text == "悬明月照屋庭")));
        let none = find(&Track { netease: None, title: "zzqqxx no such song".into(), artist: "qqqww".into(), album: String::new(), seconds: 100, qq_music: false });
        assert!(none.sure && none.found.is_none(), "that no source has a track's lyrics is known");
        assert!(matches!(netease(1), Ok(None)), "NetEase says it has none");
    }

    fn sung_lines(has: Has) -> Option<Vec<Line>> {
        match has {
            Has::Sung(lines) => Some(lines),
            Has::Instrumental => None,
        }
    }

    /// A piano piece Kugou gave another song's lyrics for (network): that it is played without words, by NetEase's
    /// number and by its name alike. `cargo test -- --ignored a_piano_piece --nocapture`.
    #[test]
    #[ignore = "asks the lyrics sources over the network"]
    fn a_piano_piece_has_no_words() {
        let by_name = Track { netease: None, qq_music: false, title: "The Truth That You Leave".into(), artist: "Pianoboy高至豪".into(), album: String::new(), seconds: 223 };
        for track in [Track { netease: Some(139774), ..by_name.clone() }, by_name.clone(), Track { qq_music: true, ..by_name.clone() }] {
            let said = find(&track);
            println!("{:?}: {:?} sure {}", track.netease, said.found.as_ref().map(|has| matches!(has, Has::Instrumental)), said.sure);
            assert_eq!(said.found, Some(Has::Instrumental));
        }
        // Kugou's of its name alone: none of the files given it under that name is of it.
        println!("Kugou: {:?}", kugou(&by_name).ok().flatten().map(|lines| lines.into_iter().take(2).map(|line| line.text).collect::<Vec<_>>()));
    }
}
