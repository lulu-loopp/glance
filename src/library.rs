//! The music kept on this PC, for the lyrics of a track played from a file: the folders of the user's Music
//! library (as Explorer keeps them), each file in them told by its tags as Windows reads them (title, artist,
//! length), kept on disk (`index`) and read again only where a file has changed; and the lyrics of a file,
//! from an LRC file beside it of the same name, else from its own tags (ID3's USLT and SYLT, Vorbis comments in
//! FLAC and Ogg, MP4's ©lyr), else as Windows reads them. A player says only what plays, never from which file:
//! the track is found among them by its title, artist and length.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use windows::core::HSTRING;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::System::Com::StructuredStorage::{PropVariantToStringAlloc, PropVariantToUInt64};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, PSGetPropertyKeyFromName, SHGetPropertyStoreFromParsingName, GPS_DEFAULT};
use windows::Win32::UI::Shell::{IShellItemArray, IShellLibrary, ShellLibrary, FOLDERID_MusicLibrary, LFF_FORCEFILESYSTEM, SIGDN_FILESYSPATH};

use crate::lyrics::{Line, Track, Word};

/// A file of music: where, as it was when its tags were read (its size and when it was last written), and what
/// they say (each empty where not said; its length in seconds, 0 where not said).
#[derive(Clone, Serialize, Deserialize)]
struct Song {
    size: u64,
    written: u64,
    title: String,
    artist: String,
    seconds: u32,
}

/// The files of music, by path, as last read; loaded from disk the first time.
static INDEX: Mutex<Option<HashMap<PathBuf, Song>>> = Mutex::new(None);
pub const INDEX_FILE: &str = "music.json";
const INDEX_MOST: u64 = 64 << 20;
/// The most files looked at in the folders (a library larger is looked at so far).
const FILES_MOST: usize = 200_000;
/// What a file of music is called by.
const AUDIO: &[&str] = &["mp3", "flac", "m4a", "mp4", "aac", "alac", "ogg", "oga", "opus", "wma", "wav", "ape", "wv", "aiff", "aif", "dsf", "dff"];

/// The lyrics of `track` from a file of music on this PC that is it (its title the same, but for case and spaces;
/// its length the same to two seconds; its artist the same, or one of its artists, where both say), where one has
/// timed lyrics. The library is looked at again first (only files changed since being read).
pub fn lyrics(track: &Track) -> Option<Vec<Line>> {
    if track.title.trim().is_empty() {
        return None;
    }
    matching(&looked_at(&folders()), track).into_iter().find_map(|path| of_file(path))
}

/// The file of music on this PC that is `track`, as `lyrics` finds it (the first, by its path).
pub fn file_of(track: &Track) -> Option<PathBuf> {
    if track.title.trim().is_empty() {
        return None;
    }
    matching(&looked_at(&folders()), track).into_iter().next().cloned()
}

/// The files in `index` that are `track`, as `lyrics` says; in the order of their paths (the same each time).
fn matching<'a>(index: &'a HashMap<PathBuf, Song>, track: &Track) -> Vec<&'a PathBuf> {
    let bare = |text: &str| text.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect::<String>();
    let (title, artist) = (bare(&track.title), bare(&track.artist));
    let mut found: Vec<&PathBuf> = index
        .iter()
        .filter(|(_, song)| bare(&song.title) == title)
        .filter(|(_, song)| song.seconds == 0 || track.seconds == 0 || song.seconds.abs_diff(track.seconds) <= 2)
        .filter(|(_, song)| song.artist.is_empty() || artist.is_empty() || song.artist.split(';').any(|theirs| bare(theirs) == artist))
        .map(|(path, _)| path)
        .collect();
    found.sort();
    found
}

/// How long the folders, once looked at, are taken as they were (a track with no cover is asked for one every few
/// seconds while it plays: the folders, which may be on a USB drive or a network's, were walked whole each time).
const LOOK_AGAIN: std::time::Duration = std::time::Duration::from_secs(60);

/// The index of the files in `folders`, looked at again (no more often than `LOOK_AGAIN`): those gone left out, those
/// new or changed read.
fn looked_at(folders: &[PathBuf]) -> HashMap<PathBuf, Song> {
    static LOOKED: Mutex<Option<(Vec<PathBuf>, std::time::Instant)>> = Mutex::new(None);
    let mut held = INDEX.lock().unwrap();
    let dir = crate::settings::local_dir();
    let index = held.get_or_insert_with(|| crate::elevation::read_in_place(&dir, INDEX_FILE, INDEX_MOST).and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default());
    let mut looked = LOOKED.lock().unwrap();
    if looked.as_ref().is_some_and(|(those, at)| those == folders && at.elapsed() < LOOK_AGAIN) {
        return index.clone();
    }
    *looked = Some((folders.to_vec(), std::time::Instant::now()));
    drop(looked);
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let mut seen = HashMap::new();
    let mut changed = false;
    let mut files = Vec::new();
    for folder in folders {
        walk(folder, &mut files);
    }
    for (path, size, written) in files {
        let song = match index.get(&path) {
            Some(song) if song.size == size && song.written == written => song.clone(),
            _ => {
                changed = true;
                read_tags(&path, size, written)
            }
        };
        seen.insert(path, song);
    }
    changed |= seen.len() != index.len();
    *index = seen;
    if changed {
        if let Ok(json) = serde_json::to_vec(&*index) {
            if dir.parent().is_some_and(|parent| crate::elevation::ensure_folder(parent).is_ok()) && crate::elevation::ensure_folder(&dir).is_ok() {
                let _ = crate::elevation::write_in_place(&dir, INDEX_FILE, &json);
            }
        }
    }
    index.clone()
}

/// The folders of the user's Music library, as Explorer keeps them (the user's Music folder, and those added).
fn folders() -> Vec<PathBuf> {
    unsafe {
        let Ok(library) = CoCreateInstance::<_, IShellLibrary>(&ShellLibrary, None, CLSCTX_INPROC_SERVER) else { return Vec::new() };
        // (Read only: STGM_READ.)
        if library.LoadLibraryFromKnownFolder(&FOLDERID_MusicLibrary, 0).is_err() {
            return Vec::new();
        }
        let Ok(items) = library.GetFolders::<IShellItemArray>(LFF_FORCEFILESYSTEM) else { return Vec::new() };
        let count = items.GetCount().unwrap_or(0);
        (0..count)
            .filter_map(|i| {
                let name = items.GetItemAt(i).ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
                let path = PathBuf::from(name.to_string().ok()?);
                CoTaskMemFree(Some(name.0 as *const _));
                Some(path)
            })
            .collect()
    }
}

/// The files of music in `folder` and the folders in it (not through links, which could lead round again):
/// each with its size and when it was last written.
fn walk(folder: &Path, files: &mut Vec<(PathBuf, u64, u64)>) {
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    for entry in entries.flatten() {
        if files.len() >= FILES_MOST {
            return;
        }
        let Ok(kind) = entry.file_type() else { continue };
        let path = entry.path();
        if kind.is_dir() {
            walk(&path, files);
        } else if kind.is_file() && path.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| AUDIO.iter().any(|audio| audio.eq_ignore_ascii_case(extension))) {
            if let Ok(metadata) = entry.metadata() {
                let written = metadata.modified().ok().and_then(|time| time.duration_since(std::time::SystemTime::UNIX_EPOCH).ok()).map_or(0, |since| since.as_nanos() as u64);
                files.push((path, metadata.len(), written));
            }
        }
    }
}

/// What `path`'s tags say, as Windows reads them.
fn read_tags(path: &Path, size: u64, written: u64) -> Song {
    let mut song = Song { size, written, title: String::new(), artist: String::new(), seconds: 0 };
    let Some(store) = unsafe { SHGetPropertyStoreFromParsingName::<_, _, IPropertyStore>(&HSTRING::from(path.as_os_str()), None, GPS_DEFAULT) }.ok() else { return song };
    song.title = property(&store, "System.Title").unwrap_or_default();
    // (Artists, as Windows joins them: "; ".)
    song.artist = property(&store, "System.Music.Artist").unwrap_or_default();
    song.seconds = key("System.Media.Duration").and_then(|key| unsafe { PropVariantToUInt64(&store.GetValue(&key).ok()?).ok() }).map_or(0, |ticks| (ticks as f64 / 1e7).round() as u32);
    song
}

/// A property's key by its name ("System.Title").
fn key(name: &str) -> Option<PROPERTYKEY> {
    let mut key = PROPERTYKEY::default();
    unsafe { PSGetPropertyKeyFromName(&HSTRING::from(name), &mut key) }.ok()?;
    Some(key)
}

/// A property of `store` as text (several, joined "; "), where it has it.
fn property(store: &IPropertyStore, name: &str) -> Option<String> {
    let key = key(name)?;
    unsafe {
        let value = store.GetValue(&key).ok()?;
        let text = PropVariantToStringAlloc(&value).ok()?;
        let said = text.to_string().ok();
        CoTaskMemFree(Some(text.0 as *const _));
        said.filter(|said| !said.is_empty())
    }
}

/// The timed lyrics of the file at `path`: from an LRC file beside it of the same name, else its own tags, else
/// as Windows reads them.
pub fn of_file(path: &Path) -> Option<Vec<Line>> {
    let timed = |lines: Vec<Line>| (!lines.is_empty() && lines.iter().any(|line| !line.text.is_empty())).then_some(lines);
    let beside = std::fs::read(path.with_extension("lrc")).ok().map(|bytes| decoded(&bytes));
    if let Some(lines) = beside.map(|text| crate::lyrics::lrc(&text)).and_then(timed) {
        return Some(lines);
    }
    let tagged = std::fs::File::open(path).ok().and_then(|mut file| tags(&mut file));
    for lyric in tagged.map(|tagged| tagged.lyrics).unwrap_or_default() {
        let lines = match lyric {
            Lyric::Text(text) => crate::lyrics::lrc(&text),
            Lyric::Synced(timed) => synced(&timed),
        };
        if let Some(lines) = timed(lines) {
            return Some(lines);
        }
    }
    let store = unsafe { SHGetPropertyStoreFromParsingName::<_, _, IPropertyStore>(&HSTRING::from(path.as_os_str()), None, GPS_DEFAULT) }.ok()?;
    property(&store, "System.Music.Lyrics").map(|text| crate::lyrics::lrc(&text)).and_then(timed)
}

/// The cover in the tags of the file at `path` (ID3's APIC, FLAC's and Ogg's pictures, MP4's covr): the picture's
/// bytes as kept there (a JPEG or a PNG), the front cover's where it says which is.
pub fn cover_of(path: &Path) -> Option<Vec<u8>> {
    let tagged = tags(&mut std::fs::File::open(path).ok()?)?;
    tagged.picture.map(|(_, bytes)| bytes)
}

/// What a file's tags hold that Glance shows: its lyrics, and its cover (whether it is the front cover, and its
/// bytes).
#[derive(Default)]
struct Tagged {
    lyrics: Vec<Lyric>,
    picture: Option<(bool, Vec<u8>)>,
}

impl Tagged {
    /// A picture in the tags (`front`: the front cover): kept where none is yet, or it is the front cover and the
    /// one kept is not.
    fn picture(&mut self, (front, bytes): (bool, Vec<u8>)) {
        if !bytes.is_empty() && self.picture.as_ref().is_none_or(|(kept, _)| front && !kept) {
            self.picture = Some((front, bytes));
        }
    }
}

/// Lyrics as a file's tags keep them: as text (LRC, where timed), or timed by the tag itself (ID3's SYLT: each
/// piece of text with the moment it is sung from, in seconds).
enum Lyric {
    Text(String),
    Synced(Vec<(f64, String)>),
}

/// The lines of lyrics timed by the tag (SYLT): a piece of text beginning with a line break begins a line, the
/// others go on the one before as its words; where none begins with one, each is a line.
fn synced(timed: &[(f64, String)]) -> Vec<Line> {
    let breaks = |text: &str| text.starts_with(['\n', '\r']);
    let worded = timed.iter().any(|(_, text)| breaks(text));
    let mut lines: Vec<Line> = Vec::new();
    for (i, (at, text)) in timed.iter().enumerate() {
        let end = timed.get(i + 1).map_or(*at, |next| next.0);
        let word = Word { at: *at, end, text: text.trim_start_matches(['\n', '\r']).to_string() };
        match lines.last_mut() {
            Some(line) if worded && !breaks(text) => {
                line.text.push_str(&word.text);
                line.words.push(word);
            }
            _ => lines.push(Line { at: *at, end: None, text: word.text.clone(), words: if worded { vec![word] } else { Vec::new() }, backing: None, translation: None }),
        }
    }
    for line in &mut lines {
        line.text = line.text.trim().to_string();
    }
    lines
}

/// The lyrics and the cover in a file's tags, as its kind keeps them (told by how it begins).
fn tags(file: &mut std::fs::File) -> Option<Tagged> {
    let mut start = [0u8; 12];
    file.read_exact(&mut start).ok()?;
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut tagged = Tagged::default();
    match &start {
        [b'I', b'D', b'3', ..] => {
            id3(file, &mut tagged)?;
            // (A FLAC file can begin with an ID3 tag, before its own.)
            let mut magic = [0u8; 4];
            if file.read_exact(&mut magic).is_ok() && &magic == b"fLaC" {
                flac(file, &mut tagged);
            }
        }
        [b'f', b'L', b'a', b'C', ..] => {
            file.seek(SeekFrom::Start(4)).ok()?;
            flac(file, &mut tagged)?;
        }
        [b'O', b'g', b'g', b'S', ..] => ogg(file, &mut tagged)?,
        [_, _, _, _, b'f', b't', b'y', b'p', ..] => mp4(file, &mut tagged)?,
        _ => return None,
    }
    Some(tagged)
}

/// The lyrics and pictures in an ID3v2 tag (versions 2.2 to 2.4), the file read from its start: left just after the
/// tag.
fn id3(file: &mut std::fs::File, tagged: &mut Tagged) -> Option<()> {
    let mut header = [0u8; 10];
    file.read_exact(&mut header).ok()?;
    let (version, flags) = (header[3], header[5]);
    let size = syncsafe(&header[6..10]);
    let mut tag = vec![0u8; size as usize];
    file.read_exact(&mut tag).ok()?;
    // Unsynchronised throughout (2.2, 2.3): each 0xFF 0x00 was 0xFF.
    if version < 4 && flags & 0x80 != 0 {
        tag = resynchronised(&tag);
    }
    let mut at = 0;
    // An extended header (2.3, 2.4) is passed over.
    if version >= 3 && flags & 0x40 != 0 && tag.len() >= 4 {
        at = if version == 4 { syncsafe(&tag[..4]) as usize } else { u32::from_be_bytes(tag[..4].try_into().ok()?) as usize + 4 };
    }
    let (id_length, header_length) = if version == 2 { (3, 6) } else { (4, 10) };
    while at + header_length <= tag.len() && tag[at] != 0 {
        let id = &tag[at..at + id_length];
        let length = match version {
            2 => u32::from_be_bytes([0, tag[at + 3], tag[at + 4], tag[at + 5]]) as usize,
            3 => u32::from_be_bytes(tag[at + 4..at + 8].try_into().ok()?) as usize,
            _ => syncsafe(&tag[at + 4..at + 8]) as usize,
        };
        let frame_flags = if version == 2 { 0 } else { u16::from_be_bytes([tag[at + 8], tag[at + 9]]) };
        let body_start = at + header_length;
        let Some(body) = tag.get(body_start..body_start + length) else { break };
        at = body_start + length;
        let mut body = body.to_vec();
        match version {
            // Compressed or encrypted: passed over; grouped: its group's byte first.
            3 if frame_flags & 0x00C0 != 0 => continue,
            3 if frame_flags & 0x0020 != 0 => body = body.split_off(1.min(body.len())),
            4 if frame_flags & 0x000C != 0 => continue,
            4 => {
                if frame_flags & 0x0040 != 0 {
                    body = body.split_off(1.min(body.len()));
                }
                if frame_flags & 0x0002 != 0 {
                    body = resynchronised(&body);
                }
                if frame_flags & 0x0001 != 0 {
                    body = body.split_off(4.min(body.len()));
                }
            }
            _ => {}
        }
        match id {
            b"USLT" | b"ULT" => tagged.lyrics.extend(unsynced(&body).map(Lyric::Text)),
            b"SYLT" | b"SLT" => tagged.lyrics.extend(synced_frame(&body).map(Lyric::Synced)),
            b"APIC" | b"PIC" => {
                if let Some(picture) = attached(&body, version == 2) {
                    tagged.picture(picture);
                }
            }
            _ => {}
        }
    }
    file.seek(SeekFrom::Start(10 + size as u64 + if version == 4 && flags & 0x10 != 0 { 10 } else { 0 })).ok()?;
    Some(())
}

/// APIC (PIC in 2.2): its encoding, its kind (a MIME type ended by a zero, in 2.2 three letters), what it shows (3:
/// the front cover), a description, and the picture.
fn attached(body: &[u8], short: bool) -> Option<(bool, Vec<u8>)> {
    let encoding = *body.first()?;
    let rest = if short { body.get(4..)? } else { body.get(1..).and_then(|after| after.get(after.iter().position(|byte| *byte == 0)? + 1..))? };
    let shows = *rest.first()?;
    let (_, picture) = terminated(rest.get(1..)?, encoding);
    Some((shows == 3, picture.to_vec()))
}

/// A 28-bit number written 7 bits to a byte.
fn syncsafe(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0, |number, byte| number << 7 | (*byte & 0x7F) as u32)
}

/// Bytes unsynchronised (each 0xFF followed by 0x00 written for 0xFF) as they were.
fn resynchronised(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    for (i, byte) in bytes.iter().enumerate() {
        if *byte == 0 && i > 0 && bytes[i - 1] == 0xFF {
            continue;
        }
        out.push(*byte);
    }
    out
}

/// USLT: its encoding, language, a description and the lyrics.
fn unsynced(body: &[u8]) -> Option<String> {
    let encoding = *body.first()?;
    let rest = body.get(4..)?;
    let (_, after) = terminated(rest, encoding);
    Some(id3_text(after, encoding))
}

/// SYLT: its encoding, language, how its times are said (2: milliseconds; 1, MPEG frames, cannot be told in time
/// without the stream), what it holds, a description, then each piece of text (ended) and the time it begins.
fn synced_frame(body: &[u8]) -> Option<Vec<(f64, String)>> {
    let encoding = *body.first()?;
    if *body.get(4)? != 2 {
        return None;
    }
    let (_, mut rest) = terminated(body.get(6..)?, encoding);
    let mut timed = Vec::new();
    while !rest.is_empty() {
        let (text, after) = terminated(rest, encoding);
        let at = u32::from_be_bytes(after.get(..4)?.try_into().ok()?);
        timed.push((at as f64 / 1000.0, id3_text(text, encoding)));
        rest = &after[4..];
    }
    Some(timed)
}

/// Text in `bytes` up to its end (one zero byte, two in UTF-16), and what follows it.
fn terminated(bytes: &[u8], encoding: u8) -> (&[u8], &[u8]) {
    if encoding == 1 || encoding == 2 {
        let end = (0..bytes.len() / 2).map(|i| i * 2).find(|&i| bytes[i] == 0 && bytes[i + 1] == 0);
        end.map_or((bytes, &[][..]), |end| (&bytes[..end], &bytes[end + 2..]))
    } else {
        bytes.iter().position(|byte| *byte == 0).map_or((bytes, &[][..]), |end| (&bytes[..end], &bytes[end + 1..]))
    }
}

/// ID3 text as its encoding says: 1, UTF-16 with a byte-order mark; 2, UTF-16 big-endian; 3, UTF-8; 0, Latin-1 by
/// the standard, read in the system's code page as Windows reads it (in which tags written in China are).
fn id3_text(bytes: &[u8], encoding: u8) -> String {
    let text = match encoding {
        1 => match bytes {
            [0xFE, 0xFF, rest @ ..] => utf16(rest, true),
            [0xFF, 0xFE, rest @ ..] => utf16(rest, false),
            _ => utf16(bytes, false),
        },
        2 => utf16(bytes, true),
        3 => String::from_utf8_lossy(bytes).into_owned(),
        _ => ansi(bytes),
    };
    text.trim_end_matches('\0').to_string()
}

fn utf16(bytes: &[u8], big: bool) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|pair| if big { u16::from_be_bytes([pair[0], pair[1]]) } else { u16::from_le_bytes([pair[0], pair[1]]) }).collect();
    String::from_utf16_lossy(&units)
}

/// Bytes in the system's code page (ANSI) as text.
fn ansi(bytes: &[u8]) -> String {
    use windows::Win32::Globalization::{MultiByteToWideChar, CP_ACP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS};
    if bytes.is_empty() {
        return String::new();
    }
    let length = unsafe { MultiByteToWideChar(CP_ACP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, None) };
    let mut wide = vec![0u16; length.max(0) as usize];
    unsafe { MultiByteToWideChar(CP_ACP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, Some(&mut wide)) };
    String::from_utf16_lossy(&wide)
}

/// A text file as players write it: by its byte-order mark where it has one; else UTF-8 where it is that, else
/// in the system's code page (as LRC files made in China are, GBK).
pub fn decoded(bytes: &[u8]) -> String {
    match bytes {
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        [0xFF, 0xFE, rest @ ..] => utf16(rest, false),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, true),
        _ => std::str::from_utf8(bytes).map_or_else(|_| ansi(bytes), str::to_string),
    }
}

/// The lyrics and pictures in a FLAC file's metadata (its Vorbis comment, its pictures), the file read from just
/// after "fLaC".
fn flac(file: &mut std::fs::File, tagged: &mut Tagged) -> Option<()> {
    loop {
        let mut header = [0u8; 4];
        file.read_exact(&mut header).ok()?;
        let (last, kind) = (header[0] & 0x80 != 0, header[0] & 0x7F);
        let length = u32::from_be_bytes([0, header[1], header[2], header[3]]) as u64;
        if kind == 4 || kind == 6 {
            let mut block = vec![0u8; length as usize];
            file.read_exact(&mut block).ok()?;
            if kind == 4 {
                vorbis_comment(&block, tagged);
            } else if let Some(picture) = flac_picture(&block) {
                tagged.picture(picture);
            }
        } else {
            file.seek(SeekFrom::Current(length as i64)).ok()?;
        }
        if last {
            return Some(());
        }
    }
}

/// A FLAC picture block (as Vorbis comments keep one too, in base64): what it shows (3: the front cover), its MIME
/// type and description (each its length first), its size and colours, and the picture (its length first); the
/// numbers big-endian.
fn flac_picture(block: &[u8]) -> Option<(bool, Vec<u8>)> {
    let number = |at: usize| block.get(at..at + 4).map(|bytes| u32::from_be_bytes(bytes.try_into().unwrap()) as usize);
    let shows = number(0)?;
    let described = 8 + number(4)?;
    let size_at = described + 4 + number(described)? + 16;
    let length = number(size_at)?;
    Some((shows == 3, block.get(size_at + 4..size_at + 4 + length)?.to_vec()))
}

/// The lyrics among Vorbis comments (`LYRICS=`, `UNSYNCEDLYRICS=`), and its pictures (`METADATA_BLOCK_PICTURE=`): a
/// vendor string, then each comment, each with its length before it (little-endian).
fn vorbis_comment(block: &[u8], tagged: &mut Tagged) {
    let read = |at: usize| block.get(at..at + 4).map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()) as usize);
    let Some(vendor) = read(0) else { return };
    let mut at = 4 + vendor;
    let Some(count) = read(at) else { return };
    at += 4;
    for _ in 0..count {
        let Some(length) = read(at) else { break };
        let Some(comment) = block.get(at + 4..at + 4 + length) else { break };
        at += 4 + length;
        let Some(equals) = comment.iter().position(|byte| *byte == b'=') else { continue };
        let (name, value) = (String::from_utf8_lossy(&comment[..equals]), &comment[equals + 1..]);
        if name.eq_ignore_ascii_case("LYRICS") || name.eq_ignore_ascii_case("UNSYNCEDLYRICS") {
            tagged.lyrics.push(Lyric::Text(String::from_utf8_lossy(value).into_owned()));
        } else if name.eq_ignore_ascii_case("METADATA_BLOCK_PICTURE") {
            if let Some(picture) = unbased(value).as_deref().and_then(flac_picture) {
                tagged.picture(picture);
            }
        }
    }
}

/// Base64 (RFC 4648, the standard alphabet) as the bytes it says; none where it is not that.
fn unbased(text: &[u8]) -> Option<Vec<u8>> {
    let value = |byte: u8| match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let digits: Vec<u8> = text.iter().copied().filter(|byte| !byte.is_ascii_whitespace() && *byte != b'=').map(value).collect::<Option<_>>()?;
    let mut bytes = Vec::with_capacity(digits.len() * 3 / 4);
    for chunk in digits.chunks(4) {
        let bits = chunk.iter().enumerate().fold(0u32, |bits, (i, digit)| bits | (*digit as u32) << (18 - 6 * i));
        bytes.extend_from_slice(&bits.to_be_bytes()[1..chunk.len()]);
    }
    Some(bytes)
}

/// The lyrics and pictures in an Ogg file's comment header (Vorbis or Opus): its second packet, put together from its
/// pages.
fn ogg(file: &mut std::fs::File, tagged: &mut Tagged) -> Option<()> {
    let mut packets: Vec<Vec<u8>> = vec![Vec::new()];
    // (A comment header with pictures in it can run to megabytes; none goes past this.)
    let mut read = 0usize;
    while packets.len() < 3 && read < 32 << 20 {
        let mut header = [0u8; 27];
        file.read_exact(&mut header).ok()?;
        if &header[..4] != b"OggS" {
            return None;
        }
        let mut lacing = vec![0u8; header[26] as usize];
        file.read_exact(&mut lacing).ok()?;
        for segment in lacing {
            let mut bytes = vec![0u8; segment as usize];
            file.read_exact(&mut bytes).ok()?;
            read += bytes.len();
            packets.last_mut()?.extend_from_slice(&bytes);
            // A segment shorter than 255 ends its packet.
            if segment < 255 {
                packets.push(Vec::new());
            }
        }
    }
    let comments = packets.get(1)?;
    let block = comments.strip_prefix(b"\x03vorbis").or_else(|| comments.strip_prefix(b"OpusTags"))?;
    vorbis_comment(block, tagged);
    Some(())
}

/// The lyrics and the cover in an MP4 file's metadata: moov / udta / meta / ilst / ©lyr (and covr) / data.
fn mp4(file: &mut std::fs::File, tagged: &mut Tagged) -> Option<()> {
    let end = file.seek(SeekFrom::End(0)).ok()?;
    let moov = boxed(file, 0, end, b"moov")?;
    let udta = boxed(file, moov.0, moov.1, b"udta")?;
    let meta = boxed(file, udta.0, udta.1, b"meta")?;
    // `meta` is a full box (a version and flags before what it holds), but as QuickTime writes it.
    let mut peek = [0u8; 8];
    file.seek(SeekFrom::Start(meta.0)).ok()?;
    file.read_exact(&mut peek).ok()?;
    let inner = if &peek[4..8] == b"hdlr" { meta.0 } else { meta.0 + 4 };
    let ilst = boxed(file, inner, meta.1, b"ilst")?;
    // What the `data` box in an item of it holds: its type and locale, then that.
    let mut held = |kind: &[u8; 4], most: u64| -> Option<Vec<u8>> {
        let item = boxed(file, ilst.0, ilst.1, kind)?;
        let data = boxed(file, item.0, item.1, b"data")?;
        let length = (data.1 - data.0).checked_sub(8)?;
        let mut bytes = vec![0u8; length.min(most) as usize];
        file.seek(SeekFrom::Start(data.0 + 8)).ok()?;
        file.read_exact(&mut bytes).ok()?;
        Some(bytes)
    };
    if let Some(text) = held(b"\xa9lyr", 4 << 20) {
        tagged.lyrics.push(Lyric::Text(String::from_utf8_lossy(&text).into_owned()));
    }
    // (The cover: the first of its pictures, which is the front.)
    if let Some(picture) = held(b"covr", 32 << 20) {
        tagged.picture((true, picture));
    }
    Some(())
}

/// Where what the box `kind` holds begins and ends, the first of that kind between `start` and `end`.
fn boxed(file: &mut std::fs::File, start: u64, end: u64, kind: &[u8; 4]) -> Option<(u64, u64)> {
    let mut at = start;
    while at + 8 <= end {
        let mut header = [0u8; 8];
        file.seek(SeekFrom::Start(at)).ok()?;
        file.read_exact(&mut header).ok()?;
        let (mut length, mut header_length) = (u32::from_be_bytes(header[..4].try_into().ok()?) as u64, 8);
        if length == 1 {
            let mut large = [0u8; 8];
            file.read_exact(&mut large).ok()?;
            length = u64::from_be_bytes(large);
            header_length = 16;
        } else if length == 0 {
            length = end - at;
        }
        if length < header_length {
            return None;
        }
        if &header[4..8] == kind {
            return Some((at + header_length, (at + length).min(end)));
        }
        at += length;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads the test music made by `mkaudio.py` (in the folder `GLANCE_TEST_MUSIC` names):
    /// `cargo test --lib -- --ignored reads_local_lyrics --nocapture`.
    #[test]
    #[ignore = "reads the test music GLANCE_TEST_MUSIC names"]
    fn reads_local_lyrics() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let folder = PathBuf::from(std::env::var("GLANCE_TEST_MUSIC").expect("GLANCE_TEST_MUSIC"));
        let mut files = Vec::new();
        walk(&folder, &mut files);
        assert_eq!(files.len(), 5);
        for (path, size, written) in files {
            let song = read_tags(&path, size, written);
            let lines = of_file(&path);
            println!("{}: [{}] [{}] {} s: {:?}", path.display(), song.title, song.artist, song.seconds, lines.as_ref().map(|lines| lines.iter().map(|line| (line.at, line.text.clone(), line.words.len())).collect::<Vec<_>>()));
            let lines = lines.expect("timed lyrics");
            assert_eq!(lines.len(), 3);
            assert_eq!(lines[1].at, 5.5);
            assert_eq!(song.seconds, 20);
        }
        // Found by what plays: its title (but for case and spaces), artist and length.
        let index = looked_at(&[folder]);
        let track = |title: &str, artist: &str, seconds| Track { netease: None, title: title.into(), artist: artist.into(), album: String::new(), seconds, qq_music: false };
        let found = |track: &Track| matching(&index, track).len();
        assert_eq!(found(&track("旁边的歌词", "测试歌手", 21)), 1);
        assert_eq!(found(&track("oggsong", "Some Artist", 20)), 1);
        assert_eq!(found(&track("Ogg Song", "Someone Else", 20)), 0);
        assert_eq!(found(&track("Ogg Song", "Some Artist", 30)), 0);
    }

    #[test]
    fn reads_mp4_lyrics() {
        fn boxed(kind: &[u8], inner: &[u8]) -> Vec<u8> {
            [&((inner.len() + 8) as u32).to_be_bytes()[..], kind, inner].concat()
        }
        let data = boxed(b"data", &[&[0, 0, 0, 1, 0, 0, 0, 0][..], "[00:01.00]一\n[00:02.00]二".as_bytes()].concat());
        let ilst = boxed(b"ilst", &boxed(b"\xa9lyr", &data));
        let meta = boxed(b"meta", &[&[0u8, 0, 0, 0][..], &boxed(b"hdlr", &[0; 25]), &ilst].concat());
        let file = [boxed(b"ftyp", b"M4A \0\0\0\0"), boxed(b"moov", &[boxed(b"mvhd", &[0; 100]), boxed(b"udta", &meta)].concat()), boxed(b"mdat", &[0; 64])].concat();
        let path = std::env::temp_dir().join(format!("glance-test-{}.m4a", std::process::id()));
        std::fs::write(&path, &file).unwrap();
        let lines = of_file(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(lines.unwrap().iter().map(|line| (line.at, line.text.as_str())).collect::<Vec<_>>(), vec![(1.0, "一"), (2.0, "二")]);
    }

    /// The cover in each kind of file's tags: ID3 2.4's APIC (the front cover over another picture), FLAC's picture
    /// block, an Ogg comment's picture in base64, MP4's covr.
    #[test]
    fn reads_covers_in_tags() {
        let picture = b"\x89PNG not really";
        let read = |name: &str, bytes: &[u8]| {
            let path = std::env::temp_dir().join(format!("glance-test-{}-{name}", std::process::id()));
            std::fs::write(&path, bytes).unwrap();
            let cover = cover_of(&path);
            std::fs::remove_file(&path).unwrap();
            cover
        };
        // ID3 2.4: a picture of the artist (8), then the front cover (3) with a UTF-16 description.
        let frame = |body: &[u8]| {
            let length = body.len() as u32;
            let size = [(length >> 21) as u8 & 0x7F, (length >> 14) as u8 & 0x7F, (length >> 7) as u8 & 0x7F, length as u8 & 0x7F];
            [&b"APIC"[..], &size, &[0, 0], body].concat()
        };
        let artist = frame(&[&[0u8][..], b"image/jpeg\0", &[8], b"artist\0", b"other"].concat());
        let front = frame(&[&[1u8][..], b"image/png\0", &[3], &[0xFF, 0xFE, b'c', 0, 0, 0], picture].concat());
        let frames = [artist, front].concat();
        let length = frames.len() as u32;
        let size = [(length >> 21) as u8 & 0x7F, (length >> 14) as u8 & 0x7F, (length >> 7) as u8 & 0x7F, length as u8 & 0x7F];
        let id3 = [&b"ID3\x04\x00\x00"[..], &size, &frames, &[0xFF, 0xFB, 0, 0]].concat();
        assert_eq!(read("cover.mp3", &id3).as_deref(), Some(&picture[..]));
        // FLAC: its stream's block, then a picture block, the last.
        let block = [&3u32.to_be_bytes()[..], &9u32.to_be_bytes(), b"image/png", &0u32.to_be_bytes(), &[0; 16], &(picture.len() as u32).to_be_bytes(), picture].concat();
        let flac = [&b"fLaC"[..], &[0, 0, 0, 34], &[0; 34], &[0x86], &(block.len() as u32).to_be_bytes()[1..], &block].concat();
        assert_eq!(read("cover.flac", &flac).as_deref(), Some(&picture[..]));
        // The same block as an Ogg comment keeps it.
        let based = crate::diagnostics::base64(&block);
        assert_eq!(unbased(based.as_bytes()).as_deref(), Some(&block[..]));
        let comment = format!("METADATA_BLOCK_PICTURE={based}");
        let mut tagged = Tagged::default();
        vorbis_comment(&[&0u32.to_le_bytes()[..], &1u32.to_le_bytes(), &(comment.len() as u32).to_le_bytes(), comment.as_bytes()].concat(), &mut tagged);
        assert_eq!(tagged.picture.map(|(front, bytes)| (front, bytes)), Some((true, picture.to_vec())));
        // MP4.
        fn boxed(kind: &[u8], inner: &[u8]) -> Vec<u8> {
            [&((inner.len() + 8) as u32).to_be_bytes()[..], kind, inner].concat()
        }
        let ilst = boxed(b"ilst", &boxed(b"covr", &boxed(b"data", &[&[0, 0, 0, 14, 0, 0, 0, 0][..], picture].concat())));
        let meta = boxed(b"meta", &[&[0u8, 0, 0, 0][..], &boxed(b"hdlr", &[0; 25]), &ilst].concat());
        let mp4 = [boxed(b"ftyp", b"M4A \0\0\0\0"), boxed(b"moov", &boxed(b"udta", &meta)), boxed(b"mdat", &[0; 64])].concat();
        assert_eq!(read("cover.m4a", &mp4).as_deref(), Some(&picture[..]));
        // Base64 as the standard has it.
        assert_eq!(unbased(b"TWFu").as_deref(), Some(&b"Man"[..]));
        assert_eq!(unbased(b"TWE=").as_deref(), Some(&b"Ma"[..]));
        assert_eq!(unbased(b"TQ==").as_deref(), Some(&b"M"[..]));
        assert!(unbased(b"T*Q=").is_none());
    }
}
