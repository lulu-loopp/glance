//! What is playing on this PC, and pausing, playing and skipping it.
//!
//! A program that tells Windows what it plays (the same that the
//! keyboard's media keys act on, shown in Windows' own flyout) is known by
//! its title and who it is by, and asked directly. One that does not (some
//! music players) is known only by the sound it makes: its name, and
//! whether it is sounding; it is asked as the keyboard would ask it, by a
//! media key.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::core::{Interface, Result};
use windows::Media::Control::{GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager, GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status};
use windows::Media::MediaPlaybackType;
use windows::Win32::Media::Audio::{eConsole, eRender, AudioSessionStateActive, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL};
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, QueryDosDeviceW, VerQueryValueW};
use windows::Win32::UI::Shell::{IShellItem, SHCreateItemFromParsingName, SIGDN_NORMALDISPLAY};
use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK};

pub struct Playing {
    /// What it is called; empty where the program does not say.
    pub title: String,
    pub artist: String,
    pub album: String,
    /// What kind of music the program says it is (a NetEase Cloud Music
    /// with the InfLink-rs plugin puts the track's number there, `NCM-…`).
    pub genres: Vec<String>,
    /// The program playing it, by a short name, and as Windows identifies
    /// it (empty where it does not tell Windows what it plays).
    pub app: String,
    pub app_id: String,
    pub playing: bool,
    /// How far into the track it is now and how long the track is
    /// (seconds), where the program says.
    pub timeline: Option<(f64, f64)>,
    /// Whether the program lets it be moved along its track (see `seek`).
    pub seekable: bool,
    /// How long ago the program last said how far in it is (seconds), where it says.
    pub said_ago: Option<f64>,
    /// When the program last said how far in it is, as Windows keeps it (tenths of a microsecond since 1601), where
    /// it says: the same each time it is looked at till it says again.
    pub said: Option<i64>,
    /// How far in it said it was then (seconds), where it says.
    pub said_at: Option<f64>,
    /// When it said how far in it is the time before, where it was heard saying both (see `heed`).
    pub said_before: Option<i64>,
    /// Whether what it says of how far in it is is where it was when it said so the time before, not then (Apple
    /// Music: its own window shows the whole second it says next).
    pub said_late: bool,
    /// How long before a line of the lyrics is sung the program's own window shows it (seconds), where it shows
    /// lyrics and that is known (Apple Music: against its window, 0.13 s to 0.5 s before the times the lyrics found
    /// give the same lines, 0.35 s as a rule).
    pub lines_ahead: Option<f64>,
    /// How fast it plays: track seconds to a second (1 where the program does not say).
    pub rate: f64,
}

/// How far into its track `session` is now, and how long the track is
/// (seconds): where it was when the program last said, and, while it
/// plays, on by the time since at its `rate`; how long ago it said, when
/// (see `Playing::said`), and how far in it said it was then. None where
/// the program does not say.
fn timeline(session: &Session, playing: bool, rate: f64) -> Option<(f64, f64, f64, i64, f64)> {
    let told = session.GetTimelineProperties().ok()?;
    let ticks = |span: windows::Foundation::TimeSpan| span.Duration as f64 / 1e7;
    let (position, end) = (ticks(told.Position().ok()?), ticks(told.EndTime().ok()?));
    if end <= 0.0 {
        return None;
    }
    // (Windows' times count from 1601, in tenths of a microsecond.)
    let ticks_said = told.LastUpdatedTime().ok()?.UniversalTime;
    let said = ticks_said as f64 / 1e7;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs_f64() + 11_644_473_600.0;
    let since = if playing { (now - said).max(0.0) * rate } else { 0.0 };
    Some(((position + since).min(end), end, (now - said).max(0.0), ticks_said, position))
}

/// The one manager of the programs' media sessions, got the first time
/// (what plays is looked at every second where it is shown).
fn manager() -> Result<Manager> {
    static MANAGER: Mutex<Option<Manager>> = Mutex::new(None);
    crate::actions::com();
    let mut kept = MANAGER.lock().unwrap();
    Ok(match kept.as_ref() {
        Some(manager) => manager.clone(),
        None => kept.insert(Manager::RequestAsync()?.join()?).clone(),
    })
}

/// What is followed: whatever plays (what Windows would act on were a media key pressed), or music alone (a music
/// player's, or what its program says is music: not a video, nor a call).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Follow {
    Any,
    Music,
}

/// The program chosen to be followed for each (see `follow_next` and `choose_music`), as Windows identifies it;
/// none: as `session` picks.
static CHOSEN: [Mutex<Option<String>>; 2] = [Mutex::new(None), Mutex::new(None)];
/// The music player last followed: all music paused, it is still the one to play again.
static LAST_MUSIC: Mutex<Option<String>> = Mutex::new(None);

/// The programs known to play music (by what Windows identifies them by, or their file's name), lower case: one of
/// them is music whatever it says of what it plays.
const MUSIC_PLAYERS: [&str; 18] = [
    "cloudmusic", "qqmusic", "kugou", "kwmusic", "spotify", "zunemusic", "applemusic", "itunes", "foobar2000", "musicbee", "aimp", "tidal",
    "deezer", "amazonmusic", "yesplaymusic", "lx-music", "listen1", "musicfree",
];

/// The browsers (by what Windows identifies them by, lower case), which say what they play is music whatever it is (a
/// video too): what one plays is music only where it is chosen to be followed for music.
const BROWSERS: [&str; 8] = ["msedge", "chrome", "brave", "opera", "vivaldi", "firefox", "308046b0af4a39cb", "yandex"];

/// Whether a program by `id` (as Windows identifies it, or by its file's name) is a music player.
fn music_player(id: &str) -> bool {
    let id = id.to_lowercase();
    MUSIC_PLAYERS.iter().any(|player| id.contains(player))
}

/// Whether `session` plays music: as its program says what it plays (music, not a video nor a picture), where it says
/// and is to be taken at its word (a browser is not: see `BROWSERS`); else as its program is a music player.
fn is_music(session: &Session) -> bool {
    let id = session.SourceAppUserModelId().map(|id| id.to_string()).unwrap_or_default();
    let lower = id.to_lowercase();
    if BROWSERS.iter().any(|browser| lower.contains(browser)) {
        return false;
    }
    let said = session.GetPlaybackInfo().and_then(|info| info.PlaybackType()).and_then(|kind| kind.Value()).ok();
    match said {
        Some(kind) if kind == MediaPlaybackType::Music => true,
        Some(kind) if kind == MediaPlaybackType::Video || kind == MediaPlaybackType::Image => false,
        _ => music_player(&id),
    }
}

/// The session followed for `follow`: the chosen program's, while it has one. Else, for whatever plays, what Windows
/// would act on were a media key pressed now; for music, of the sessions playing music the one playing (the one
/// followed last first), else the one followed last, else the first.
pub fn session_of(follow: Follow) -> Result<Session> {
    let manager = manager()?;
    let sessions: Vec<Session> = manager.GetSessions()?.into_iter().collect();
    let id = |session: &Session| session.SourceAppUserModelId().map(|id| id.to_string()).unwrap_or_default();
    if let Some(chosen) = CHOSEN[follow as usize].lock().unwrap().clone() {
        if let Some(session) = sessions.iter().find(|session| id(session) == chosen) {
            return Ok(session.clone());
        }
    }
    match follow {
        Follow::Any => manager.GetCurrentSession(),
        Follow::Music => {
            let music: Vec<Session> = sessions.into_iter().filter(is_music).collect();
            let last = LAST_MUSIC.lock().unwrap().clone();
            let playing = |session: &Session| session.GetPlaybackInfo().and_then(|info| info.PlaybackStatus()).is_ok_and(|status| status == Status::Playing);
            let found = music
                .iter()
                .find(|session| playing(session) && last.as_deref() == Some(id(session).as_str()))
                .or_else(|| music.iter().find(|session| playing(session)))
                .or_else(|| music.iter().find(|session| last.as_deref() == Some(id(session).as_str())))
                .or_else(|| music.first())
                .cloned()
                .ok_or_else(windows::core::Error::empty)?;
            *LAST_MUSIC.lock().unwrap() = Some(id(&found));
            Ok(found)
        }
    }
}

/// The session followed for whatever plays.
fn session() -> Result<Session> {
    session_of(Follow::Any)
}

/// Music followed from the program `id` (as Windows identifies it) while it has a session; none: as `session_of`
/// picks.
pub fn choose_music(id: Option<String>) {
    *CHOSEN[Follow::Music as usize].lock().unwrap() = id;
}

/// The programs music can be followed from now: those telling Windows what they play (music or not: a browser playing
/// music is chosen by hand), each as Windows identifies it and by the name it is shown by, the music players first.
pub fn music_players() -> Vec<(String, String)> {
    let Ok(sessions) = manager().and_then(|manager| manager.GetSessions()) else { return Vec::new() };
    let mut sessions: Vec<Session> = sessions.into_iter().collect();
    sessions.sort_by_key(|session| !is_music(session));
    sessions
        .into_iter()
        .filter_map(|session| session.SourceAppUserModelId().ok().map(|id| id.to_string()))
        .filter(|id| !id.is_empty())
        .map(|id| {
            let name = shown(&id).unwrap_or_else(|| id.trim_end_matches(".exe").to_string());
            (id, name)
        })
        .collect()
}

/// The programs that tell Windows what they play, as Windows identifies each, in its order.
pub fn players() -> Vec<String> {
    let Ok(sessions) = manager().and_then(|manager| manager.GetSessions()) else { return Vec::new() };
    sessions.into_iter().filter_map(|session| session.SourceAppUserModelId().ok().map(|id| id.to_string())).filter(|id| !id.is_empty()).collect()
}

/// Follows the next of the programs that tell Windows what they play, after the one followed now (round to the
/// first); whether there was another.
pub fn follow_next() -> bool {
    let players = players();
    if players.len() < 2 {
        return false;
    }
    let now = session().and_then(|session| session.SourceAppUserModelId()).map(|id| id.to_string()).unwrap_or_default();
    let next = players.iter().position(|id| *id == now).map_or(0, |at| (at + 1) % players.len());
    *CHOSEN[Follow::Any as usize].lock().unwrap() = Some(players[next].clone());
    true
}

/// What `find` finds for `key`, found once: a program's name does not
/// change while it runs. What is not found is looked for again only after
/// `LOOK_AGAIN` (looked for each time what is played is asked, one with
/// none touched its file, on a drive that may be slow to answer, every
/// time; one not found for a while, its file being updated, is found then).
fn once(kept: &Mutex<Vec<Kept>>, key: &str, find: impl FnOnce() -> Option<String>) -> Option<String> {
    let now = Instant::now();
    {
        let mut kept = kept.lock().unwrap();
        kept.retain(|(_, found, when)| found.is_some() || now.duration_since(*when) < LOOK_AGAIN);
        if let Some((_, found, _)) = kept.iter().find(|(of, _, _)| of == key) {
            return found.clone();
        }
    }
    let found = find();
    let mut kept = kept.lock().unwrap();
    // (No more than a session's worth of programs.)
    if kept.len() >= 64 {
        kept.clear();
    }
    kept.push((key.to_string(), found.clone(), now));
    found
}

/// A name looked for (see `once`): for what, what was found, and when.
type Kept = (String, Option<String>, Instant);

/// How long a name not found is not looked for again (see `once`).
const LOOK_AGAIN: Duration = Duration::from_secs(60);

/// The name Windows shows a program by (as in the Start menu), for one it
/// identifies as `id`; none for a program Windows has no such name for.
fn shown_as(id: &str) -> Option<String> {
    let path: Vec<u16> = format!("shell:AppsFolder\\{id}").encode_utf16().chain([0]).collect();
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(path.as_ptr()), None).ok()?;
        let name = item.GetDisplayName(SIGDN_NORMALDISPLAY).ok()?;
        let shown = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as *const _));
        shown.filter(|shown| !shown.is_empty())
    }
}

/// How Windows identifies Apple Music (its package's family, before the app in it).
const APPLE_MUSIC: &str = "AppleInc.AppleMusicWin_";

/// How long before a line is sung Apple Music shows it (see `Playing::lines_ahead`).
const APPLE_MUSIC_AHEAD: f64 = 0.35;

/// Of each program (as Windows identifies it), when it said how far into its track it is, the latest of those heard
/// as it said them (see `heed`).
static SAID: Mutex<Vec<(String, Vec<i64>)>> = Mutex::new(Vec::new());
const SAID_KEPT: usize = 4;

/// The program `id` was heard saying how far in it is at `said` (as Windows keeps when: see `Playing::said`).
fn heard_said(id: &str, said: i64) {
    let mut kept = SAID.lock().unwrap();
    let index = match kept.iter().position(|(of, _)| of == id) {
        Some(index) => index,
        None => {
            kept.push((id.to_string(), Vec::new()));
            kept.len() - 1
        }
    };
    let times = &mut kept[index].1;
    if !times.contains(&said) {
        times.push(said);
        times.sort_unstable();
        if times.len() > SAID_KEPT {
            times.remove(0);
        }
    }
}

/// When the program `id` said how far in it is the time before it said so at `said`, where it was heard saying both.
fn said_before(id: &str, said: i64) -> Option<i64> {
    let kept = SAID.lock().unwrap();
    let times = &kept.iter().find(|(of, _)| of == id)?.1;
    let at = times.iter().position(|time| *time == said)?;
    Some(times[at.checked_sub(1)?])
}

/// Who it is by and the album it is on, as told by the program `app_id` identifies: Apple Music tells both as who it is
/// by, "Artist — Album", and no album.
fn by_and_on(app_id: &str, artist: String, album: String) -> (String, String) {
    match artist.split_once(" — ") {
        Some((by, on)) if album.is_empty() && app_id.starts_with(APPLE_MUSIC) => (by.to_string(), on.to_string()),
        _ => (artist, album),
    }
}

/// What a program has told Windows it plays, of those followed for `follow`.
fn told(follow: Follow) -> Option<Playing> {
    let session = session_of(follow).ok()?;
    let about = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
    let title = about.Title().ok()?.to_string();
    if title.is_empty() {
        return None;
    }
    let info = session.GetPlaybackInfo().ok();
    let playing = info.as_ref().and_then(|info| info.PlaybackStatus().ok()).is_some_and(|status| status == Status::Playing);
    let rate = info.as_ref().and_then(|info| info.PlaybackRate().ok()).and_then(|rate| rate.Value().ok()).filter(|rate| rate.is_finite() && *rate > 0.0).unwrap_or(1.0);
    let seekable = info.and_then(|info| info.Controls().ok()).and_then(|controls| controls.IsPlaybackPositionEnabled().ok()).unwrap_or(false);
    let app_id = session.SourceAppUserModelId().map(|id| id.to_string()).unwrap_or_default();
    let timeline = timeline(&session, playing, rate);
    let (said_before, said_late) = (timeline.and_then(|(_, _, _, said, _)| said_before(&app_id, said)), app_id.starts_with(APPLE_MUSIC));
    let (artist, album) = by_and_on(&app_id, about.Artist().map(|artist| artist.to_string()).unwrap_or_default(), about.AlbumTitle().map(|album| album.to_string()).unwrap_or_default());
    Some(Playing {
        title,
        artist,
        album,
        genres: about.Genres().map(|genres| genres.into_iter().map(|genre| genre.to_string()).collect()).unwrap_or_default(),
        // (One with no name to show is not named: what plays is said
        // already, and a file's name adds nothing to it.)
        app: shown(&app_id).unwrap_or_default(),
        app_id,
        playing,
        timeline: timeline.map(|(at, end, ..)| (at, end)),
        said_ago: timeline.map(|(_, _, ago, ..)| ago),
        said: timeline.map(|(_, _, _, said, _)| said),
        said_at: timeline.map(|(.., position)| position),
        said_before,
        said_late,
        lines_ahead: said_late.then_some(APPLE_MUSIC_AHEAD),
        seekable,
        rate,
    })
}

/// Moves what plays to `seconds` into its track, where its program lets it; whether it took it.
pub fn seek(seconds: f64) -> bool {
    seek_of(Follow::Any, seconds)
}

/// As `seek`, what is followed for `follow`.
pub fn seek_of(follow: Follow, seconds: f64) -> bool {
    let Ok(session) = session_of(follow) else { return false };
    session.TryChangePlaybackPositionAsync((seconds.max(0.0) * 1e7) as i64).and_then(|asking| asking.join()).unwrap_or(false)
}

/// The programs with sound of their own on the device the sound plays
/// from, each by its file's name and whether it is sounding now. (Its name
/// out of what the sound's session is called, which has the program's path
/// in it: the program itself is not opened.)
pub fn sounding() -> Result<Vec<(String, bool)>> {
    Ok(sounds()?.into_iter().map(|(name, _, active)| (name, active)).collect())
}

/// The same, each with the path of its program's file as the session has it.
fn sounds() -> Result<Vec<(String, String, bool)>> {
    crate::actions::com();
    let mut found = Vec::new();
    unsafe {
        let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let manager: IAudioSessionManager2 = devices.GetDefaultAudioEndpoint(eRender, eConsole)?.Activate(CLSCTX_ALL, None)?;
        let sessions = manager.GetSessionEnumerator()?;
        for at in 0..sessions.GetCount()? {
            let session: IAudioSessionControl2 = sessions.GetSession(at)?.cast()?;
            // (Windows' own sounds are no program's.)
            if session.IsSystemSoundsSession().0 == 0 {
                continue;
            }
            let called = session.GetSessionInstanceIdentifier()?;
            let name = called.to_string().unwrap_or_default();
            CoTaskMemFree(Some(called.0 as *const _));
            if let Some((program, path)) = named(&name) {
                found.push((program, path, session.GetState()? == AudioSessionStateActive));
            }
        }
    }
    Ok(found)
}

/// A program's name, and the path of its file, out of what its sound's
/// session is called (`{…}|\Device\…\cloudmusic.exe%b{…}|…`); none for a
/// session with no program's path in it.
fn named(session: &str) -> Option<(String, String)> {
    let path = session.split('|').nth(1)?.split("%b").next()?;
    let file = path.strip_suffix(".exe").or_else(|| path.strip_suffix(".EXE"))?;
    let name = file.rsplit('\\').next()?;
    (!name.is_empty()).then(|| (name.to_string(), path.to_string()))
}

/// What a program is called, by its file (`path` as a sound's session has it: from the device up): as the Start menu
/// names it, where it does ("QQ音乐"); else as the file says the program is, in the first language it says it in
/// (which may be more than its name: "QQ音乐 听我想听").
fn described(path: &str) -> Option<String> {
    static DESCRIBED: Mutex<Vec<Kept>> = Mutex::new(Vec::new());
    once(&DESCRIBED, path, || on_drives(path).and_then(|path| shown_as(&path)).or_else(|| read_description(path)))
}

/// `path` from the device up (`\Device\HarddiskVolume3\App\x.exe`) as from its drive (`D:\App\x.exe`), as the Start
/// menu knows a program by its file; none where it is on no drive.
fn on_drives(path: &str) -> Option<String> {
    (b'A'..=b'Z').find_map(|letter| {
        let drive = format!("{}:", letter as char);
        let named: Vec<u16> = drive.encode_utf16().chain([0]).collect();
        let mut device = [0u16; 512];
        let length = unsafe { QueryDosDeviceW(PCWSTR(named.as_ptr()), Some(&mut device)) } as usize;
        // (The first of the names it is a link to, each ended.)
        let device = String::from_utf16_lossy(&device[..device[..length].iter().position(|unit| *unit == 0)?]);
        let rest = path.get(device.len()..).filter(|_| path[..device.len()].eq_ignore_ascii_case(&device))?;
        rest.starts_with('\\').then(|| format!("{drive}{rest}"))
    })
}

fn read_description(path: &str) -> Option<String> {
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let file = wide(&format!(r"\\?\GLOBALROOT{path}"));
    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(file.as_ptr()), None);
        let mut about = vec![0u8; size as usize];
        GetFileVersionInfoW(PCWSTR(file.as_ptr()), None, size, about.as_mut_ptr().cast()).ok()?;
        let (mut found, mut length) = (std::ptr::null_mut(), 0u32);
        // The languages it is described in (each a language and a code page), then the description in the first.
        VerQueryValueW(about.as_ptr().cast(), PCWSTR(wide(r"\VarFileInfo\Translation").as_ptr()), &mut found, &mut length).ok().ok()?;
        if length < 4 {
            return None;
        }
        let said_in = std::slice::from_raw_parts(found as *const u16, 2);
        let asked = wide(&format!(r"\StringFileInfo\{:04x}{:04x}\FileDescription", said_in[0], said_in[1]));
        VerQueryValueW(about.as_ptr().cast(), PCWSTR(asked.as_ptr()), &mut found, &mut length).ok().ok()?;
        // To its end (some programs' files say its length in bytes, twice as long: read that far, what came after it
        // was shown too, "Spotify8FileV").
        let said = std::slice::from_raw_parts(found as *const u16, length as usize);
        let text = String::from_utf16_lossy(&said[..said.iter().position(|unit| *unit == 0).unwrap_or(said.len())]);
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_string())
    }
}

/// What a program is shown as: by the name Windows has for it (as in the
/// Start menu), else as its file describes it, where its file is known by
/// the sound it has open; `id` as Windows identifies it. (Its file looked
/// for among the sounds as it is asked: one may open its sound after.)
fn shown(id: &str) -> Option<String> {
    static SHOWN: Mutex<Vec<Kept>> = Mutex::new(Vec::new());
    once(&SHOWN, id, || shown_as(id)).or_else(|| {
        let file = id.strip_suffix(".exe").or_else(|| id.strip_suffix(".EXE")).unwrap_or(id);
        let sounds = sounds().ok()?;
        sounds.iter().find(|(name, _, _)| name.eq_ignore_ascii_case(file)).and_then(|(_, path, _)| described(path))
    })
}

/// The program last heard sounding, of those that do not tell Windows what
/// they play (for each that is followed: see `Follow`): paused, it is still
/// the one to play again.
static HEARD: [Mutex<Option<String>>; 2] = [Mutex::new(None), Mutex::new(None)];

/// The program that is sounding (the one heard before, if it still is);
/// or, none sounding, the one heard before, paused, for as long as it has
/// its sound open. For music, of the music players alone.
fn heard(follow: Follow) -> Option<Playing> {
    let programs: Vec<(String, String, bool)> = sounds().ok()?.into_iter().filter(|(name, _, _)| follow == Follow::Any || music_player(name)).collect();
    let mut heard = HEARD[follow as usize].lock().unwrap();
    let is = |name: &String, sounding: bool| programs.iter().any(|(program, _, active)| program == name && *active == sounding);
    let now = heard.clone().filter(|name| is(name, true)).or_else(|| programs.iter().find(|(_, _, active)| *active).map(|(name, _, _)| name.clone()));
    let (name, playing) = match now {
        Some(name) => (name, true),
        None => (heard.clone().filter(|name| is(name, false))?, false),
    };
    *heard = Some(name.clone());
    // As its file describes it; one that does not say, by its file's name.
    let app = programs.iter().find(|(program, _, _)| *program == name).and_then(|(_, path, _)| described(path)).unwrap_or(name);
    Some(Playing { title: String::new(), artist: String::new(), album: String::new(), genres: Vec::new(), app, app_id: String::new(), playing, timeline: None, seekable: false, said_ago: None, said: None, said_at: None, said_before: None, said_late: false, lines_ahead: None, rate: 1.0 })
}

/// A program's session listened to (see `heed`), till this is dropped.
pub struct Heeding {
    session: Session,
    /// The program, as Windows identifies it.
    id: String,
    playback: i64,
    properties: i64,
    timeline: i64,
}

impl Heeding {
    /// Whether it is the session followed for `follow` now.
    pub fn stands(&self, follow: Follow) -> bool {
        session_of(follow).is_ok_and(|session| session.SourceAppUserModelId().is_ok_and(|id| id == self.id.as_str()))
    }
}

impl Drop for Heeding {
    fn drop(&mut self) {
        let _ = self.session.RemovePlaybackInfoChanged(self.playback);
        let _ = self.session.RemoveMediaPropertiesChanged(self.properties);
        let _ = self.session.RemoveTimelinePropertiesChanged(self.timeline);
    }
}

/// Listens to the session followed for `follow`: `changed` is called as its program says it plays another track, or
/// plays or pauses. When it says how far in it is is kept as it says it (see `Playing::said_before`). None where none
/// is followed.
pub fn heed(follow: Follow, changed: fn()) -> Option<Heeding> {
    use windows::Foundation::TypedEventHandler;
    let session = session_of(follow).ok()?;
    let id = session.SourceAppUserModelId().ok()?.to_string();
    let playback = session
        .PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
            changed();
            Ok(())
        }))
        .ok()?;
    let properties = session.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
        changed();
        Ok(())
    }));
    let of = id.clone();
    let timeline = session.TimelinePropertiesChanged(&TypedEventHandler::new(move |session: windows::core::Ref<Session>, _| {
        if let Some(said) = session.ok().ok().and_then(|session| session.GetTimelineProperties().ok()).and_then(|told| told.LastUpdatedTime().ok()) {
            heard_said(&of, said.UniversalTime);
        }
        Ok(())
    }));
    match (properties, timeline) {
        (Ok(properties), Ok(timeline)) => Some(Heeding { session, id, playback, properties, timeline }),
        (properties, timeline) => {
            let _ = session.RemovePlaybackInfoChanged(playback);
            if let Ok(properties) = properties {
                let _ = session.RemoveMediaPropertiesChanged(properties);
            }
            if let Ok(timeline) = timeline {
                let _ = session.RemoveTimelinePropertiesChanged(timeline);
            }
            None
        }
    }
}

/// Listens, for as long as the process runs, to the sessions as a whole: `changed` is called as one comes or goes,
/// or another becomes the one a media key acts on. Whether it does.
pub fn heed_sessions(changed: fn()) -> bool {
    use windows::Foundation::TypedEventHandler;
    let Ok(manager) = manager() else { return false };
    let sessions = manager.SessionsChanged(&TypedEventHandler::new(move |_, _| {
        changed();
        Ok(())
    }));
    let current = manager.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
        changed();
        Ok(())
    }));
    sessions.is_ok() && current.is_ok()
}

/// What is playing (or paused); none, when nothing is.
pub fn playing() -> Option<Playing> {
    playing_of(Follow::Any)
}

/// What is playing (or paused) of what is followed for `follow`; none, when nothing is.
pub fn playing_of(follow: Follow) -> Option<Playing> {
    told(follow).or_else(|| heard(follow))
}

/// Presses and lets go one of the keyboard's media keys.
fn press(key: VIRTUAL_KEY) -> bool {
    let turn = |up: bool| INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: key, dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() }, ..Default::default() } } };
    unsafe { SendInput(&[turn(false), turn(true)], size_of::<INPUT>() as i32) == 2 }
}

/// How what was asked was done.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Asked {
    /// The program playing took it.
    Taken,
    /// A media key was pressed for it (as far as Windows goes, a key
    /// pressed by hand: the screens wake at it).
    Pressed,
    /// Neither.
    Not,
}

/// Pauses or plays what is playing ("toggle"), or skips on or back
/// ("next", "previous").
pub fn ask(what: &str) -> Asked {
    ask_of(Follow::Any, what)
}

/// As `ask`, what is followed for `follow`.
pub fn ask_of(follow: Follow, what: &str) -> Asked {
    let key = match what {
        "toggle" => VK_MEDIA_PLAY_PAUSE,
        "next" => VK_MEDIA_NEXT_TRACK,
        "previous" => VK_MEDIA_PREV_TRACK,
        _ => return Asked::Not,
    };
    if let Ok(session) = session_of(follow) {
        let asked = match what {
            "toggle" => session.TryTogglePlayPauseAsync(),
            "next" => session.TrySkipNextAsync(),
            _ => session.TrySkipPreviousAsync(),
        };
        if asked.and_then(|asking| asking.join()).unwrap_or(false) {
            return Asked::Taken;
        }
        // Not taken by the program followed: the key goes to the program Windows has in front of its media, which
        // may be another (a video in a browser, while the music is followed), and is pressed only where it is that
        // one. (Pressed regardless, another program was paused or skipped.)
        let id = |session: &Session| session.SourceAppUserModelId().map(|id| id.to_string()).ok();
        let current = manager().and_then(|manager| manager.GetCurrentSession()).ok();
        if current.as_ref().and_then(id).is_none_or(|current| Some(current) != id(&session)) {
            return Asked::Not;
        }
    }
    // Music followed and no music player tells Windows it plays (one only heard, NetEase without its plug-in): the
    // key reaches such a player only where no program has Windows' media in front; where one has (a video in a
    // browser), it would go to that one, and is not pressed.
    else if follow == Follow::Music && manager().and_then(|manager| manager.GetCurrentSession()).is_ok() {
        return Asked::Not;
    }
    // (No program followed tells Windows it plays, or it is the one in front: pressed for it.)
    if press(key) {
        Asked::Pressed
    } else {
        Asked::Not
    }
}

#[cfg(test)]
mod tests {
    /// What is not found is not looked for again for a while (a name the Start menu does not have, or a file that
    /// says nothing, was looked for, its file touched, each time what plays was asked), and is after it; what is found
    /// is found once.
    #[test]
    fn what_is_not_found_is_looked_for_once() {
        let kept = std::sync::Mutex::new(Vec::new());
        let looked = std::cell::Cell::new(0);
        let find = |found: Option<&str>| {
            looked.set(looked.get() + 1);
            found.map(str::to_string)
        };
        assert_eq!(super::once(&kept, "a", || find(None)), None);
        assert_eq!(super::once(&kept, "a", || find(Some("A"))), None);
        assert_eq!(super::once(&kept, "b", || find(Some("B"))), Some("B".into()));
        assert_eq!(super::once(&kept, "b", || find(None)), Some("B".into()));
        assert_eq!(looked.get(), 2);
        // (Looked for a while ago.)
        for (_, _, when) in kept.lock().unwrap().iter_mut() {
            *when -= super::LOOK_AGAIN;
        }
        assert_eq!(super::once(&kept, "a", || find(Some("A"))), Some("A".into()));
        assert_eq!(super::once(&kept, "b", || find(None)), Some("B".into()));
        assert_eq!(looked.get(), 3);
    }

    /// Apple Music's "Artist — Album" told as who it is by is who it is by and the album; another program's, or one
    /// with an album told, as told.
    #[test]
    fn apple_music_tells_the_album_with_the_artist() {
        let apple = "AppleInc.AppleMusicWin_nzyj5cx40ttqa!App";
        let told = |app_id: &str, artist: &str, album: &str| super::by_and_on(app_id, artist.into(), album.into());
        assert_eq!(told(apple, "胡歌 — 仙剑奇侠传 (电视原声带)", ""), ("胡歌".into(), "仙剑奇侠传 (电视原声带)".into()));
        assert_eq!(told(apple, "A — B — C", ""), ("A".into(), "B — C".into()));
        assert_eq!(told(apple, "胡歌", ""), ("胡歌".into(), String::new()));
        assert_eq!(told(apple, "A — B", "C"), ("A — B".into(), "C".into()));
        assert_eq!(told("cloudmusic.exe", "A — B", ""), ("A — B".into(), String::new()));
    }

    /// When a program said how far in it is before it said so at a time is the one heard before it, where both were
    /// heard; heard twice, once; none before the first heard, or where that was not heard, or the one before it is no
    /// longer kept.
    #[test]
    fn a_report_before_is_the_one_heard_before() {
        let id = "test.a_report_before";
        for said in [100, 300, 200, 300] {
            super::heard_said(id, said);
        }
        assert_eq!(super::said_before(id, 300), Some(200));
        assert_eq!(super::said_before(id, 200), Some(100));
        assert_eq!(super::said_before(id, 100), None);
        assert_eq!(super::said_before(id, 250), None);
        assert_eq!(super::said_before("test.other", 300), None);
        for said in [400, 500] {
            super::heard_said(id, said);
        }
        assert_eq!(super::said_before(id, 200), None);
        assert_eq!(super::said_before(id, 500), Some(400));
    }

    #[test]
    fn names_a_program_from_its_sound() {
        let named = super::named(r"{0.0.0.00000000}.{6f2a}|\Device\HarddiskVolume3\App\CloudMusic\cloudmusic.exe%b{00000000-0000-0000-0000-000000000000}|1%b9044");
        assert_eq!(named, Some(("cloudmusic".to_string(), r"\Device\HarddiskVolume3\App\CloudMusic\cloudmusic.exe".to_string())));
        // A session with no program's path in it.
        assert_eq!(super::named("{0.0.0.00000000}.{6f2a}|#%b{6f2a}|1%b#"), None);
        assert_eq!(super::named("nothing of the kind"), None);
    }

    /// A program's file from the device up is found on its drive (as the Start menu knows it), its case aside; one on
    /// no drive is not.
    #[test]
    fn a_path_from_the_device_is_found_on_its_drive() {
        let windows = std::env::var("SystemRoot").unwrap();
        let (drive, rest) = windows.split_at(2);
        let named: Vec<u16> = drive.encode_utf16().chain([0]).collect();
        let mut device = [0u16; 512];
        let length = unsafe { super::QueryDosDeviceW(super::PCWSTR(named.as_ptr()), Some(&mut device)) } as usize;
        let device = String::from_utf16_lossy(&device[..device[..length].iter().position(|unit| *unit == 0).unwrap()]);
        assert_eq!(super::on_drives(&format!(r"{device}{rest}\explorer.exe")), Some(format!(r"{windows}\explorer.exe")));
        assert_eq!(super::on_drives(&format!(r"{}{rest}\explorer.exe", device.to_uppercase())), Some(format!(r"{windows}\explorer.exe")));
        assert_eq!(super::on_drives(r"\Device\NoSuchVolume\App\x.exe"), None);
        // (A volume's name with another's in front of it, "...Volume3" and "...Volume31", is not taken for it.)
        assert_eq!(super::on_drives(&format!(r"{device}1\x.exe")), None);
    }
}
