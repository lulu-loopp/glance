//! What plays on this PC and how loud the sound is, for the module that
//! shows them and lets them be changed: the track paused, played and
//! skipped, the sound turned up and down and muted. Read and done as the
//! phone remote reads and does them (the same code: see the remote's
//! `media` and `actions`), by Glance itself, whether the remote is on or
//! not; looked at every second while the module is on, and at once after
//! anything is asked. With them, while lyrics are shown, the line sung now
//! (looked at again as the next is due).
//!
//! Two are followed: whatever plays (the time and media show it, and let it be paused, whatever it is), and music
//! alone (the music component shows it, with its lyrics: not a video playing meanwhile). See `media::Follow`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

#[cfg(windows)]
pub use crate::media::Follow;
/// What is followed (see the media module's).
#[cfg(not(windows))]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Follow {
    Any,
    Music,
}

/// How it is now.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Now {
    /// What plays, by whom and in which program: each empty where it is not said (all, with nothing playing).
    pub title: String,
    pub artist: String,
    /// The album it is from, where its player says (a song on two albums has two covers).
    pub album: String,
    pub app: String,
    pub playing: bool,
    /// How loud the sound is (0 to 100), where it could be read, and whether it is muted.
    pub volume: Option<u8>,
    pub muted: bool,
    /// How far into the track it is and how long it is, where its player says.
    pub timeline: Option<Timeline>,
    /// How many players tell Windows what they play (one of them followed: see `Key::Player`).
    pub players: u8,
    /// What is sung, while lyrics are shown and a track plays.
    pub sung: Option<Sung>,
}

/// How far into its track what plays is (whole seconds, as last looked at, or as it is being dragged to), how
/// long the track is (seconds), and whether its player lets it be moved along it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Timeline {
    pub at: f64,
    pub length: f64,
    pub seekable: bool,
}

/// What is sung of the track that plays.
#[derive(Clone, PartialEq, Debug)]
pub enum Sung {
    /// Its player does not say how far into it it is, so no line can be told from another; `netease`, NetEase
    /// Cloud Music (which says, with the InfLink-rs plugin).
    Untimed { netease: bool },
    /// Its lyrics are being asked for.
    Asking,
    /// None were found.
    Unknown,
    /// Its lyrics, the line sung now (none before the first), and since when it is the one (what is drawn of
    /// them glides to it for `GLIDE` after).
    Lines { lines: Lyrics, now: Option<usize>, since: std::time::Instant },
}

/// How long the lyrics take to glide from one line to the next.
pub const GLIDE: std::time::Duration = std::time::Duration::from_millis(420);


/// What is told that the line sung has changed (the windows that show it are drawn at once then).
#[cfg(windows)]
static TOLD: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

#[cfg(windows)]
use crate::lyrics::Lyrics;
#[cfg(not(windows))]
pub type Lyrics = std::sync::Arc<Vec<()>>;

/// One of the things pressed.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Key {
    Previous,
    Toggle,
    Next,
    Mute,
    /// The next of the players followed (where several tell Windows what they play).
    Player,
}

/// What is asked for.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Act {
    Key(Key),
    /// The sound as loud as this (0 to 100).
    Volume(u8),
    /// The track played from this far into it (seconds).
    Seek(f64),
}

/// How it is, for each followed (see `Follow`).
static NOW: [Mutex<Option<Now>>; 2] = [Mutex::new(None), Mutex::new(None)];
/// Counted up each time what `NOW` holds changes: what is drawn of it is drawn again then.
static STAMP: AtomicU64 = AtomicU64::new(0);

/// Music shown by the settings (their music page, and the layout's): it is looked at meanwhile, shown anywhere else
/// or not.
static SHOWN_IN_SETTINGS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the settings show music now (see `SHOWN_IN_SETTINGS`), and the settings saying so.
pub fn shown_in_settings() -> bool {
    SHOWN_IN_SETTINGS.load(Ordering::Relaxed)
}

pub fn show_in_settings(shown: bool) {
    SHOWN_IN_SETTINGS.store(shown, Ordering::Relaxed);
}

/// The players telling Windows they play music, as last looked at (each as Windows identifies it, and its name).
static MUSIC_PLAYERS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// The players telling Windows they play music, as last looked at while music is shown.
pub fn music_players() -> Vec<(String, String)> {
    MUSIC_PLAYERS.lock().unwrap().clone()
}

/// How whatever plays is, as last looked at; none while nothing shows it (or before the first look).
pub fn now() -> Option<Now> {
    NOW[Follow::Any as usize].lock().unwrap().clone()
}

/// How the music is, as last looked at (with its lyrics, while they are shown); none while nothing shows it.
pub fn music() -> Option<Now> {
    NOW[Follow::Music as usize].lock().unwrap().clone()
}

/// A number that changes whenever `now` does.
pub fn stamp() -> u64 {
    STAMP.load(Ordering::Relaxed)
}

fn keep(follow: Follow, now: Option<Now>) {
    let mut kept = NOW[follow as usize].lock().unwrap();
    if *kept != now {
        *kept = now;
        STAMP.fetch_add(1, Ordering::Relaxed);
    }
}

/// Just asked to pause or to play: whether it is taken to be playing till
/// when. (A program says that it has paused a little after it was asked
/// to, and some say the other thing once in between: what was asked for
/// shows meanwhile, and what the program says after.)
#[cfg(windows)]
static ASKED: [Mutex<Option<(std::time::Instant, bool)>>; 2] = [Mutex::new(None), Mutex::new(None)];
/// How long what was asked for stands for what is said.
#[cfg(windows)]
const SETTLES: std::time::Duration = std::time::Duration::from_millis(1500);

/// Where along its track what plays is being dragged to, while it is (shown so, whatever its player says); then,
/// asked to go there, till when what was asked for stands for what is said.
#[cfg(windows)]
static SEEKING: [Mutex<Option<(f64, Option<std::time::Instant>)>>; 2] = [Mutex::new(None), Mutex::new(None)];

/// Whether lyrics are shown (and so asked for), as `watch` was last told.
#[cfg(windows)]
static LYRICS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Where along its track what is followed for each is reckoned to be (see `Reckoning`); none while it is not looked at.
#[cfg(windows)]
static RECKONING: [Mutex<Option<Reckoning>>; 2] = [Mutex::new(None), Mutex::new(None)];

/// One look at a time, whoever asks (the watcher, or an action's own thread): two at once, one's older report of the
/// player was taken after the other's newer one.
#[cfg(windows)]
static LOOKING: Mutex<()> = Mutex::new(());

/// What is followed for `follow` moved `at` that far along its track at `now`, and what is followed for the other where
/// it is the same player's same track (moved along too: left as it was, it was reckoned on from where it had been, its
/// lines with it, till the player said where it is, which some never do): each shown there at once and till its
/// player says it is, reckoned from there on from now (what its player said before stands for where it was then).
/// Each moved, and how it was reckoned before (to be put back, the player not moved).
#[cfg(windows)]
fn moved_along(follow: Follow, at: f64, now: std::time::Instant) -> Vec<(usize, Option<Reckoning>)> {
    let of = |reckoning: &Option<Reckoning>| reckoning.as_ref().map(|reckoning| (reckoning.app_id.clone(), reckoning.title.clone()));
    let this = of(&RECKONING[follow as usize].lock().unwrap());
    let mut moved = Vec::new();
    for g in [Follow::Any as usize, Follow::Music as usize] {
        let before = {
            let mut reckoning = RECKONING[g].lock().unwrap();
            if g != follow as usize && (this.is_none() || of(&reckoning) != this) {
                continue;
            }
            let before = reckoning.clone();
            if let Some(reckoning) = reckoning.as_mut() {
                let said = Some(reckoning.said.unwrap_or(i64::MIN).max(said_now()));
                *reckoning = Reckoning { at, when: now, said, ..reckoning.clone() };
            }
            before
        };
        *SEEKING[g].lock().unwrap() = Some((at, Some(now + SETTLES)));
        moved.push((g, before));
    }
    moved
}

/// Where along its track something is reckoned to be: `at` (seconds) as of `when`, playing or not, at `rate`, of
/// its program and track; and the player's report it rests on (see `media::Playing::said`).
///
/// Players say how far in they are now and then, and not always as they pause, play again or are moved along:
/// NetEase's says nothing more as it pauses, QQ Music's says it plays again before it says where, Media Player says
/// nothing after it is moved along. So what a player says is taken only when it is newer than what was taken before;
/// else it goes on from what was reckoned at the last look, at the rate and in the state then, to now. A pause, a
/// play again, a move along or another rate so counts from when it is seen, not from the player's last report.
#[derive(Clone, Debug, PartialEq)]
struct Reckoning {
    at: f64,
    when: std::time::Instant,
    playing: bool,
    rate: f64,
    app_id: String,
    title: String,
    said: Option<i64>,
}

/// What a player says of where along its track it is, as a look sees it (see `media::Playing`).
#[derive(Clone, Copy)]
struct Report<'a> {
    app_id: &'a str,
    title: &'a str,
    playing: bool,
    rate: f64,
    /// How far in it is now, reckoned by the player's report, and how long the track is (seconds).
    at: f64,
    length: f64,
    said: Option<i64>,
}

impl Reckoning {
    /// How far in it is at `now` (seconds).
    fn at(&self, now: std::time::Instant) -> f64 {
        if self.playing { self.at + now.saturating_duration_since(self.when).as_secs_f64() * self.rate } else { self.at }
    }

    /// Where `report` puts it at `now`, after `was` (`settling`: just moved along, what the player says meanwhile not
    /// taken).
    fn after(was: Option<&Reckoning>, report: Report, settling: bool, now: std::time::Instant) -> Reckoning {
        let was = was.filter(|was| was.app_id == report.app_id && was.title == report.title);
        let newer = report.said.is_some_and(|said| was.is_none_or(|was| was.said.is_none_or(|taken| said > taken)));
        let (at, said) = match was {
            Some(was) if settling || !newer => (was.at(now).clamp(0.0, report.length), was.said),
            // (While it plays on, a report slightly behind where it was reckoned to is jitter of its player's clock
            // or coarse second updates, as Apple Music does: held to move forward, not drawn back.)
            Some(was) if was.playing && report.playing && report.at < was.at(now) && was.at(now) - report.at < 1.0 => {
                (was.at(now).clamp(0.0, report.length), report.said)
            }
            _ => (report.at, report.said),
        };
        Reckoning { at, when: now, playing: report.playing, rate: report.rate, app_id: report.app_id.to_string(), title: report.title.to_string(), said }
    }
}

/// Windows' time now, as players' reports say it (tenths of a microsecond since 1601).
#[cfg(windows)]
fn said_now() -> i64 {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    (since.as_secs() as i64 + 11_644_473_600) * 10_000_000 + since.subsec_nanos() as i64 / 100
}

/// How far into the track of what is followed for `follow` it is now (seconds): where it is dragged, while it is;
/// else as reckoned (see `Reckoning`); none where its player does not say.
#[cfg(windows)]
pub fn position(follow: Follow) -> Option<f64> {
    dragged_to(follow).or_else(|| RECKONING[follow as usize].lock().unwrap().as_ref().map(|reckoning| reckoning.at(std::time::Instant::now())))
}

/// Looks at how what is followed for `follow` is; how long till the next line of the lyrics is due, where one is
/// (music's, while lyrics are shown).
#[cfg(windows)]
fn look(follow: Follow) -> Option<std::time::Duration> {
    let _one = LOOKING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let f = follow as usize;
    let mut playing = crate::media::playing_of(follow);
    if follow == Follow::Music {
        *MUSIC_PLAYERS.lock().unwrap() = crate::media::music_players();
    }
    // Just asked to go somewhere along it: what its player says meanwhile not taken (see `SEEKING`).
    let now = std::time::Instant::now();
    let settling = matches!(*SEEKING[f].lock().unwrap(), Some((_, Some(till))) if now < till);
    {
        let mut reckoning = RECKONING[f].lock().unwrap();
        *reckoning = playing.as_ref().and_then(|playing| {
            let (at, length) = playing.timeline?;
            let report = Report { app_id: &playing.app_id, title: &playing.title, playing: playing.playing, rate: playing.rate, at, length, said: playing.said };
            Some(Reckoning::after(reckoning.as_ref(), report, settling, now))
        });
        if let (Some(reckoning), Some((position, _))) = (reckoning.as_ref(), playing.as_mut().and_then(|playing| playing.timeline.as_mut())) {
            *position = reckoning.at;
        }
    }
    // Dragged along: shown as far as it is dragged.
    if let (Some(at), Some((position, _))) = (dragged_to(follow), playing.as_mut().and_then(|playing| playing.timeline.as_mut())) {
        *position = at;
    }
    let timeline = playing.as_ref().and_then(|playing| playing.timeline.map(|(at, length)| Timeline { at: at.floor(), length, seekable: playing.seekable }));
    let sound = crate::actions::volume();
    // (The slider held, where it is.)
    let sound = match *SLID_TO.lock().unwrap() {
        Some(level) => Some((level, sound.is_some_and(|(_, muted)| muted))),
        None => sound,
    };
    let (sung, next) = match playing.as_ref().filter(|playing| follow == Follow::Music && LYRICS.load(Ordering::Relaxed) && !playing.title.is_empty()) {
        Some(playing) => sung(playing),
        None => (None, None),
    };
    let (title, artist, album, app, is_playing) = playing.map_or_else(Default::default, |playing| (playing.title, playing.artist, playing.album, playing.app, playing.playing));
    let is_playing = match *ASKED[f].lock().unwrap() {
        Some((till, asked)) if std::time::Instant::now() < till => asked,
        _ => is_playing,
    };
    let players = crate::media::players().len().min(u8::MAX as usize) as u8;
    keep(follow, Some(Now { title, artist, album, app, playing: is_playing, volume: sound.map(|(level, _)| level), muted: sound.is_some_and(|(_, muted)| muted), timeline, players, sung }));
    next
}

/// What is sung of `playing`, and how long till its next line is due (while it plays).
#[cfg(windows)]
fn sung(playing: &crate::media::Playing) -> (Option<Sung>, Option<std::time::Duration>) {
    let Some((position, length)) = playing.timeline else {
        return (Some(Sung::Untimed { netease: playing.app_id.eq_ignore_ascii_case("cloudmusic.exe") }), None);
    };
    // Its number on NetEase Cloud Music, where its player says it; else its first artist (InfLink-rs and others
    // join them with " / "), as the collections name a track by.
    let netease = playing.genres.iter().find_map(|genre| genre.strip_prefix("NCM-")?.parse().ok());
    let artist = playing.artist.split(" / ").next().unwrap_or_default().to_string();
    let qq_music = playing.app_id.eq_ignore_ascii_case("QQMusic.exe");
    let track = crate::lyrics::Track { netease, title: playing.title.clone(), artist, album: playing.album.clone(), seconds: length.round() as u32, qq_music };
    match crate::lyrics::of(&track) {
        crate::lyrics::Known::Asking => (Some(Sung::Asking), None),
        crate::lyrics::Known::Found(None) => (Some(Sung::Unknown), None),
        crate::lyrics::Known::Found(Some(lines)) => {
            let now = crate::lyrics::sung_at(&lines, position);
            let next = now.map_or(0, |now| now + 1);
            // (Track seconds, at the rate it plays.)
            let due = lines.get(next).filter(|_| playing.playing).map(|line| std::time::Duration::from_secs_f64(((line.at - crate::lyrics::EARLY - position) / playing.rate).max(0.0)));
            // The same line as before keeps its time; another is the one from when it became due (from now, if
            // that was longer ago than a glide: come to by a jump in the track, it glides all the same).
            let before = NOW[Follow::Music as usize].lock().unwrap().as_ref().and_then(|kept| match &kept.sung {
                Some(Sung::Lines { lines: had, now: was, since }) if std::sync::Arc::ptr_eq(had, &lines) && *was == now => Some(*since),
                _ => None,
            });
            let since = before.unwrap_or_else(|| {
                let late = now.map_or(0.0, |now| ((position + crate::lyrics::EARLY - lines[now].at) / playing.rate).clamp(0.0, GLIDE.as_secs_f64()));
                // (So far back only where the PC has been up that long: just started, from now.)
                std::time::Instant::now().checked_sub(std::time::Duration::from_secs_f64(late)).unwrap_or_else(std::time::Instant::now)
            });
            if before.is_none() {
                if let Some(told) = TOLD.get() {
                    told();
                }
            }
            (Some(Sung::Lines { lines, now, since }), due)
        }
    }
}

/// Looks every second at what `wanted` says is shown (whatever plays, and music with its lyrics), while what can show
/// it is on the screen (its third), as each line of the lyrics is due, and as its player says it changed (another
/// track, played or paused, another player); forgets what it saw of one while it is not shown. (Looked at with
/// nothing on the screen to show it, Windows was asked what plays, and how loud, every second for nothing.) With
/// nothing on the screen to show it, the thread sleeps till something that can comes on it (see `look_now`).
#[cfg(windows)]
pub fn watch(wanted: impl Fn() -> (bool, bool, bool) + Send + 'static, changed: impl Fn() + Send + Sync + 'static) {
    let _ = TOLD.set(Box::new(changed));
    std::thread::Builder::new().name("glance: what plays".into()).spawn(move || {
        crate::actions::com();
        crate::media::heed_sessions(look_now);
        // The player followed for each, listened to while it is looked at.
        let mut heeding: [Option<crate::media::Heeding>; 2] = [None, None];
        // Looked at once as Glance starts, whether or not anything shows it yet: the panel is laid out for what it
        // holds as it first opens, and what plays was not known then (laid out without it, it was drawn over the lane
        // under the time's once it was).
        let (any, music, _) = wanted();
        if any {
            let _ = look(Follow::Any);
        }
        if music {
            let _ = look(Follow::Music);
        }
        loop {
            let (any, music, seen) = wanted();
            for (follow, on) in [(Follow::Any, any), (Follow::Music, music)] {
                let heeded = &mut heeding[follow as usize];
                if !(on && seen) {
                    *heeded = None;
                    // Where it was, and whether it played, as last looked at, no longer known (not looked at
                    // meanwhile, a pause then was taken for one just now, and the whole time since added to where it
                    // was held).
                    *RECKONING[follow as usize].lock().unwrap() = None;
                } else if !heeded.as_ref().is_some_and(|heeded| heeded.stands(follow)) {
                    *heeded = None;
                    *heeded = crate::media::heed(follow, look_now);
                }
            }
            LYRICS.store(music, Ordering::Relaxed);
            // (Off the screen, what was seen last kept: shown again, it shows that till it is looked at, at once.)
            match (any, seen) {
                (true, true) => drop(look(Follow::Any)),
                (true, false) => {}
                (false, _) => keep(Follow::Any, None),
            }
            let next = match (music, seen) {
                (true, true) => look(Follow::Music),
                (true, false) => None,
                (false, _) => {
                    keep(Follow::Music, None);
                    None
                }
            };
            // (Not sooner than a twentieth of a second: a line due already is looked at as the next comes.)
            let wait = next.map_or(std::time::Duration::from_secs(1), |next| next.clamp(std::time::Duration::from_millis(50), std::time::Duration::from_secs(1)));
            let (asked, woken) = &LOOK_NOW;
            let asked = asked.lock().unwrap();
            // (With nothing on the screen to show it, looked at again only as something comes on it: see `look_now`.)
            let mut asked = if seen { woken.wait_timeout_while(asked, wait, |asked| !*asked).unwrap().0 } else { woken.wait_while(asked, |asked| !*asked).unwrap() };
            *asked = false;
        }
    }).expect("a thread started");
}

/// Asked to look at what plays at once (something that shows it came on the screen, or its player said it changed),
/// and the watch woken for it.
#[cfg(windows)]
static LOOK_NOW: (Mutex<bool>, std::sync::Condvar) = (Mutex::new(false), std::sync::Condvar::new());

/// What plays looked at at once, not at the next second (see `watch`).
#[cfg(windows)]
pub fn look_now() {
    *LOOK_NOW.0.lock().unwrap() = true;
    LOOK_NOW.1.notify_all();
}

/// What is followed for `follow`, shown at once as paused if it played and as playing if it was paused, till its
/// program says how it is (see `ASKED`).
#[cfg(windows)]
fn shown_toggled(follow: Follow) {
    let f = follow as usize;
    // (Taken out before it is kept again: the lock held on would be locked again by `keep`.)
    let now = NOW[f].lock().unwrap().clone();
    if let Some(now) = now {
        *ASKED[f].lock().unwrap() = Some((std::time::Instant::now() + SETTLES, !now.playing));
        keep(follow, Some(Now { playing: !now.playing, ..now }));
    }
}

/// Does `act` on whatever plays (see `act_on`).
#[cfg(windows)]
pub fn act(act: Act) {
    act_on(Follow::Any, act);
}

/// Does `act` on what is followed for `follow`, on a thread of its own, and looks at how it is then (and again as its
/// player says it has changed: a program says that it has paused a little after it was asked to; see `watch`).
#[cfg(windows)]
pub fn act_on(follow: Follow, act: Act) {
    std::thread::spawn(move || {
        match act {
            Act::Key(Key::Previous) => {
                let _ = crate::media::ask_of(follow, "previous");
            }
            Act::Key(Key::Toggle) => {
                shown_toggled(follow);
                let _ = crate::media::ask_of(follow, "toggle");
            }
            Act::Key(Key::Next) => {
                let _ = crate::media::ask_of(follow, "next");
            }
            Act::Key(Key::Player) => {
                crate::media::follow_next();
            }
            Act::Key(Key::Mute) => {
                let muted = crate::actions::volume().is_some_and(|(_, muted)| muted);
                sound_to(None, Some(!muted));
            }
            Act::Volume(level) => sound_to(Some(level), None),
            Act::Seek(at) => {
                let moved = moved_along(follow, at, std::time::Instant::now());
                if !crate::media::seek_of(follow, at) {
                    for (g, before) in moved {
                        *SEEKING[g].lock().unwrap() = None;
                        *RECKONING[g].lock().unwrap() = before;
                    }
                }
            }
        }
        let _ = look(follow);
    });
}

/// The sound as loud as `level` while its track is dragged along: shown so
/// at once, and set (it is looked at again when the track is let go: see
/// `act`).
#[cfg(windows)]
pub fn slide(level: u8) {
    *SLID_TO.lock().unwrap() = Some(level);
    set_sound(Some(level), None);
}

/// Where the volume's slider is, while it is held: shown, whatever the sound read meanwhile says, and set as it is
/// let go. (Taken from what was shown as it was let go, a reading of the sound from before its last move, shown
/// meanwhile, was set, and the sound went back.)
#[cfg(windows)]
static SLID_TO: Mutex<Option<u8>> = Mutex::new(None);

/// Whether the volume's slider is held now (dragged, not yet let go).
#[cfg(windows)]
pub fn volume_held() -> bool {
    SLID_TO.lock().unwrap().is_some()
}

/// The sound made as loud as `level` and muted or not as `muted` (each, where it is said): shown so at once in what
/// is followed (the sound is the whole system's, the same for both), and set on a thread of its own (it waits on the
/// sound's service, which may take long; this is done on the thread that draws). Nothing done where nothing would
/// change.
#[cfg(windows)]
pub fn set_sound(level: Option<u8>, muted: Option<bool>) {
    if shown_sound(level, muted) {
        sound_to(level, muted);
    }
}

/// The sound set to `level` and `muted` (each, where it is said), on the one thread that sets it, in the order asked;
/// what was asked meanwhile taken together, the latest of each standing. (Set each on a thread of its own, as a
/// slider was dragged, an earlier value could come after a later one and stand: the sound left louder than it was
/// turned down to.)
#[cfg(windows)]
fn sound_to(level: Option<u8>, muted: Option<bool>) {
    static SETTING: Mutex<Option<std::sync::mpsc::Sender<(Option<u8>, Option<bool>)>>> = Mutex::new(None);
    let mut setting = SETTING.lock().unwrap();
    let sender = setting.get_or_insert_with(|| {
        let (asked, asking) = std::sync::mpsc::channel::<(Option<u8>, Option<bool>)>();
        std::thread::Builder::new()
            .name("glance: the sound set".into())
            .spawn(move || {
                while let Ok(mut wanted) = asking.recv() {
                    while let Ok(later) = asking.try_recv() {
                        wanted = (later.0.or(wanted.0), later.1.or(wanted.1));
                    }
                    let _ = crate::actions::set_volume(wanted.0, wanted.1);
                }
            })
            .expect("a thread started");
        asked
    });
    let _ = sender.send((level, muted));
}

/// The sound shown as `set_sound` sets it, at once; whether it is to be set (something shown changed, or nothing is
/// shown to tell).
#[cfg(windows)]
fn shown_sound(level: Option<u8>, muted: Option<bool>) -> bool {
    let mut changed = false;
    let mut seen = false;
    for follow in [Follow::Any, Follow::Music] {
        // (Taken out before it is kept again: the lock held on would be locked again by `keep`.)
        let now = NOW[follow as usize].lock().unwrap().clone();
        if let Some(now) = now {
            seen = true;
            let shown = Now { volume: level.or(now.volume), muted: muted.unwrap_or(now.muted), ..now.clone() };
            if shown != now {
                changed = true;
                keep(follow, Some(shown));
            }
        }
    }
    changed || !seen
}

/// How loud the sound is (0 to 100) and whether it is muted, as last seen; none where it could not be read.
#[cfg(windows)]
pub fn sound() -> Option<(u8, bool)> {
    [Follow::Music, Follow::Any].into_iter().find_map(|follow| NOW[follow as usize].lock().unwrap().as_ref().and_then(|now| now.volume.map(|level| (level, now.muted))))
}

/// A track dragged along: the sound's, or that of what is followed for its `Follow` (see `Follow`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slider {
    Volume,
    Seek(Follow),
}

/// The wheel's `delta` (120 a notch) added to what it turned before (`kept`, in hundred and twentieths of a step): the
/// whole steps of the sound it makes (`per_notch` a notch), what is left of a step kept toward the next (a touchpad
/// turns it by parts of a notch, none lost).
pub fn wheel_steps(kept: &mut i32, delta: i16, per_notch: i32) -> i32 {
    *kept += i32::from(delta) * per_notch;
    let steps = *kept / 120;
    *kept -= steps * 120;
    steps
}

/// `slider` dragged to `fraction` of the way along it: the sound as loud, set at once; what plays shown as that far
/// into its track, and played from there once let go (see `slid`).
#[cfg(windows)]
pub fn slide_along(slider: Slider, fraction: f32) {
    let fraction = fraction.clamp(0.0, 1.0);
    match slider {
        Slider::Volume => slide((fraction * 100.0).round() as u8),
        Slider::Seek(follow) => {
            let Some(length) = NOW[follow as usize].lock().unwrap().as_ref().and_then(|now| now.timeline).map(|timeline| timeline.length) else { return };
            let at = fraction as f64 * length;
            *SEEKING[follow as usize].lock().unwrap() = Some((at, None));
            shown_dragged(follow, at);
        }
    }
}

/// What is followed for `follow` shown `at` that far into its track (seconds) as it is dragged there, from what was
/// last seen of it (as `look` shows it while it is dragged), its lines following. Its player is not asked: this is
/// done at each move of the pointer, on the thread that draws, and a player may take long to answer.
#[cfg(windows)]
fn shown_dragged(follow: Follow, at: f64) {
    let f = follow as usize;
    // (Taken out before it is kept again: the lock held on would be locked again by `keep`.)
    let now = NOW[f].lock().unwrap().clone();
    let Some(now) = now else { return };
    let Some(timeline) = now.timeline else { return };
    keep(follow, Some(Now { timeline: Some(Timeline { at: at.floor(), ..timeline }), ..now }));
}

/// Where the track of what is followed for `follow` is being dragged to (seconds), while it is (see `slide_along`).
#[cfg(windows)]
pub fn dragged_to(follow: Follow) -> Option<f64> {
    SEEKING[follow as usize].lock().unwrap().and_then(|(at, settling)| settling.is_none().then_some(at))
}

#[cfg(not(windows))]
pub fn dragged_to(_: Follow) -> Option<f64> {
    None
}

/// `slider` let go (`kept`: where it was left; else, the drag given up, as it was before).
#[cfg(windows)]
pub fn slid(slider: Slider, kept: bool) {
    match slider {
        Slider::Volume => {
            if let Some(level) = SLID_TO.lock().unwrap().take().filter(|_| kept) {
                act(Act::Volume(level));
            }
        }
        Slider::Seek(follow) => {
            let left_at = SEEKING[follow as usize].lock().unwrap().and_then(|(at, settling)| settling.is_none().then_some(at));
            match left_at.filter(|_| kept) {
                Some(at) => act_on(follow, Act::Seek(at)),
                // (Looked at again on a thread of its own: see `shown_dragged`.)
                None => {
                    *SEEKING[follow as usize].lock().unwrap() = None;
                    std::thread::spawn(move || {
                        let _ = look(follow);
                    });
                }
            }
        }
    }
}

#[cfg(not(windows))]
pub fn act(_: Act) {}

#[cfg(not(windows))]
pub fn act_on(_: Follow, _: Act) {}

/// For the studio's pictures: as if this played (and were music).
#[cfg(feature = "studio")]
pub fn pretend(now: Now) {
    keep(Follow::Music, Some(now.clone()));
    keep(Follow::Any, Some(now));
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// A notch of the wheel turns the sound by its step; a touchpad's parts of a notch add up to as much, none lost;
    /// back the other way, back.
    #[test]
    fn the_wheel_turns_the_sound_by_its_parts_too() {
        let mut kept = 0;
        assert_eq!(wheel_steps(&mut kept, 120, 5), 5);
        assert_eq!(wheel_steps(&mut kept, -240, 5), -10);
        assert_eq!((0..8).map(|_| wheel_steps(&mut kept, 30, 5)).sum::<i32>(), 10);
        assert_eq!(wheel_steps(&mut kept, 10, 5), 0);
        assert_eq!(wheel_steps(&mut kept, 14, 5), 1);
        assert_eq!((0..60).map(|_| wheel_steps(&mut kept, 1, 2)).sum::<i32>(), 1);
    }

    /// What is followed is kept in statics shared by these tests: one at a time.
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// The sound turned shows at once as turned, in what is followed for either, without waiting on the sound's
    /// service (it was read and set at every frame and every move on the thread that draws the full screen); turned
    /// to where it is, it is not set again. (Only what is shown: this PC's sound is not touched.)
    #[test]
    fn the_sound_turned_shows_at_once() {
        let _one = ONE_AT_A_TIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        keep(Follow::Any, Some(Now { title: "Sound".into(), volume: Some(30), ..Default::default() }));
        keep(Follow::Music, Some(Now { title: "Sound".into(), volume: Some(30), ..Default::default() }));
        assert!(shown_sound(Some(45), Some(false)));
        assert_eq!(sound(), Some((45, false)));
        assert_eq!(now().and_then(|now| now.volume), Some(45));
        assert!(!shown_sound(Some(45), None));
        assert!(shown_sound(None, Some(true)));
        assert_eq!(sound(), Some((45, true)));
    }

    /// Dragged along its track, what plays is shown where it is dragged, from what was seen of it, its player not
    /// asked (that was asked at each move of the pointer, on the thread that draws: a player slow to answer held up
    /// the panel, the widgets and the full screen; and what was seen went for what the players said instead).
    #[test]
    fn dragging_along_shows_where_it_is_dragged_without_asking() {
        let _one = ONE_AT_A_TIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let length = 200.0;
        keep(Follow::Any, Some(Now { title: "A song".into(), playing: true, timeline: Some(Timeline { at: 10.0, length, seekable: true }), ..Default::default() }));
        slide_along(Slider::Seek(Follow::Any), 0.25);
        let shown = now().expect("still what was seen");
        assert_eq!(shown.title, "A song");
        assert_eq!(shown.timeline.map(|timeline| timeline.at), Some(50.0));
        assert!(position(Follow::Any).is_some_and(|at| (50.0..51.0).contains(&at)));
        *SEEKING[Follow::Any as usize].lock().unwrap() = None;
    }

    /// Moved along by what is followed for either, what is followed for the other is moved along too where it is the
    /// same player's same track (left, it was reckoned on from where it had been: its lines and its time stayed
    /// behind the music, with a player that does not say where it is after it is moved); not another's.
    #[test]
    fn moved_along_moves_what_follows_the_same_track() {
        let _one = ONE_AT_A_TIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = std::time::Instant::now();
        let reckoned = |app_id: &str, title: &str| Some(Reckoning { at: 10.0, when: now, playing: false, rate: 1.0, app_id: app_id.into(), title: title.into(), said: None });
        let at = |follow: Follow| RECKONING[follow as usize].lock().unwrap().as_ref().map(|reckoning| reckoning.at);
        *RECKONING[Follow::Any as usize].lock().unwrap() = reckoned("player", "Song");
        *RECKONING[Follow::Music as usize].lock().unwrap() = reckoned("player", "Song");
        assert_eq!(moved_along(Follow::Any, 50.0, now).len(), 2);
        assert_eq!((at(Follow::Any), at(Follow::Music)), (Some(50.0), Some(50.0)));
        // Another track: only its own.
        *RECKONING[Follow::Music as usize].lock().unwrap() = reckoned("player", "Other");
        assert_eq!(moved_along(Follow::Any, 80.0, now).len(), 1);
        assert_eq!((at(Follow::Any), at(Follow::Music)), (Some(80.0), Some(10.0)));
        for f in [Follow::Any as usize, Follow::Music as usize] {
            *RECKONING[f].lock().unwrap() = None;
            *SEEKING[f].lock().unwrap() = None;
        }
    }

    /// A player's reports, one look after another, `seconds` after the first: where each puts it.
    struct Player {
        start: std::time::Instant,
        was: Option<Reckoning>,
    }

    /// Tenths of a microsecond, as reports say when: `seconds` after the first look.
    fn ticks(seconds: f64) -> i64 {
        1_000_000_000 + (seconds * 1e7) as i64
    }

    impl Player {
        fn new() -> Player {
            Player { start: std::time::Instant::now(), was: None }
        }

        /// Looked at `seconds` in: `playing` at `rate`, its last report said at `said` (seconds in) that it was `at`
        /// then (reckoned on to now while it plays, as `media::timeline` does).
        fn look(&mut self, seconds: f64, title: &str, playing: bool, rate: f64, said: f64, at: f64) -> f64 {
            self.look_settling(seconds, title, playing, rate, said, at, false)
        }

        #[allow(clippy::too_many_arguments)]
        fn look_settling(&mut self, seconds: f64, title: &str, playing: bool, rate: f64, said: f64, at: f64, settling: bool) -> f64 {
            let now = self.start + std::time::Duration::from_secs_f64(seconds);
            let at = if playing { at + (seconds - said) * rate } else { at };
            let report = Report { app_id: "player", title, playing, rate, at, length: 600.0, said: Some(ticks(said)) };
            let reckoning = Reckoning::after(self.was.as_ref(), report, settling, now);
            let at = reckoning.at;
            self.was = Some(reckoning);
            at
        }

        /// Moved along to `at`, `seconds` in (as `act_on` does).
        fn sought(&mut self, seconds: f64, at: f64) {
            let now = self.start + std::time::Duration::from_secs_f64(seconds);
            let was = self.was.clone().unwrap();
            self.was = Some(Reckoning { at, when: now, said: Some(was.said.unwrap().max(ticks(seconds))), ..was });
        }
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    /// Played again after a pause, its player not saying where yet (QQ Music): on from where it was paused, not
    /// ahead by the pause; then as it says.
    #[test]
    fn played_again_goes_on_from_where_it_was_paused() {
        let mut player = Player::new();
        assert!(near(player.look(0.0, "A", true, 1.0, 0.0, 10.0), 10.0));
        // Paused at 15 s, said so then.
        assert!(near(player.look(5.0, "A", false, 1.0, 5.0, 15.0), 15.0));
        // Played again 60 s later: says it plays, its report still the pause's.
        assert!(near(player.look(65.0, "A", true, 1.0, 5.0, 15.0), 15.0));
        assert!(near(player.look(70.0, "A", true, 1.0, 5.0, 15.0), 20.0));
        // Then says where it is.
        assert!(near(player.look(71.0, "A", true, 1.0, 70.5, 20.5), 21.0));
    }

    /// Paused, its player not saying so (NetEase): on from its last report till the pause is seen, then held there;
    /// paused again after playing again, the first pause not counted again.
    #[test]
    fn a_pause_holds_where_it_was_seen() {
        let mut player = Player::new();
        player.look(0.0, "A", true, 1.0, 0.0, 10.0);
        assert!(near(player.look(5.0, "A", false, 1.0, 0.0, 10.0), 15.0));
        assert!(near(player.look(65.0, "A", false, 1.0, 0.0, 10.0), 15.0));
        assert!(near(player.look(65.0, "A", true, 1.0, 0.0, 10.0), 15.0));
        assert!(near(player.look(70.0, "A", false, 1.0, 0.0, 10.0), 20.0));
        assert!(near(player.look(200.0, "A", false, 1.0, 0.0, 10.0), 20.0));
    }

    /// Moved along while paused, then played a minute later, its player not saying: from where it was moved to, the
    /// pause not counted; what it says from before the move not taken, what it says after taken.
    #[test]
    fn moved_along_while_paused_holds_there() {
        let mut player = Player::new();
        player.look(0.0, "A", false, 1.0, 0.0, 10.0);
        player.sought(1.0, 100.0);
        assert!(near(player.look(2.0, "A", false, 1.0, 0.0, 10.0), 100.0));
        assert!(near(player.look(61.0, "A", true, 1.0, 0.0, 10.0), 100.0));
        assert!(near(player.look(66.0, "A", true, 1.0, 0.0, 10.0), 105.0));
        // Paused after: held where it was, the report's age not added.
        assert!(near(player.look(70.0, "A", false, 1.0, 0.0, 10.0), 109.0));
        assert!(near(player.look(80.0, "A", false, 1.0, 80.0, 109.5), 109.5));
    }

    /// While it settles after a move, what its player says is not taken, even if newer.
    #[test]
    fn settling_after_a_move_takes_nothing_said() {
        let mut player = Player::new();
        player.look(0.0, "A", true, 1.0, 0.0, 10.0);
        player.sought(1.0, 100.0);
        assert!(near(player.look_settling(1.5, "A", true, 1.0, 1.2, 11.2, true), 100.5));
        assert!(near(player.look(3.0, "A", true, 1.0, 2.5, 101.5), 102.0));
    }

    /// Another rate counts from when it is seen, not over what played before.
    #[test]
    fn another_rate_counts_from_when_it_is_seen() {
        let mut player = Player::new();
        player.look(0.0, "A", true, 1.0, 0.0, 50.0);
        assert!(near(player.look(10.0, "A", true, 2.0, 0.0, 50.0), 60.0));
        assert!(near(player.look(15.0, "A", true, 2.0, 0.0, 50.0), 70.0));
    }

    /// A report newer than what was taken is taken, whether it came before or after its playing again was seen.
    #[test]
    fn a_newer_report_is_taken() {
        let mut player = Player::new();
        player.look(0.0, "A", false, 1.0, 0.0, 60.0);
        // Moved along elsewhere and played, its player saying so before Glance looked.
        assert!(near(player.look(10.0, "A", true, 1.0, 9.0, 120.0), 121.0));
    }

    /// Another track, or another player's: as its own player says, nothing of what was reckoned before.
    #[test]
    fn another_track_is_as_said() {
        let mut player = Player::new();
        player.look(0.0, "A", false, 1.0, 0.0, 60.0);
        player.sought(1.0, 100.0);
        assert!(near(player.look(2.0, "B", false, 1.0, 0.0, 30.0), 30.0));
        let now = player.start + std::time::Duration::from_secs(3);
        let other = Report { app_id: "other", title: "B", playing: false, rate: 1.0, at: 40.0, length: 600.0, said: Some(ticks(0.0)) };
        assert!(near(Reckoning::after(player.was.as_ref(), other, false, now).at, 40.0));
    }

    /// Jitter from coarse updates (e.g. Apple Music reporting same integer second multiple times with fresh timestamps)
    /// does not draw position backward while playing continuously.
    #[test]
    fn small_backward_jitter_while_playing_does_not_move_backward() {
        let mut player = Player::new();
        assert!(near(player.look(0.0, "A", true, 1.0, 0.0, 10.0), 10.0));
        assert!(near(player.look(0.25, "A", true, 1.0, 0.0, 10.0), 10.25));
        // Jitter report: new timestamp (0.25), but stale position (10.0).
        assert!(near(player.look(0.25, "A", true, 1.0, 0.25, 10.0), 10.25));
        assert!(near(player.look(0.5, "A", true, 1.0, 0.25, 10.0), 10.5));
    }

    /// Pausing shows it paused at once, and returns (it once locked what plays twice, and Glance hung).
    #[test]
    #[cfg(windows)]
    fn pausing_shows_it_paused() {
        let _one = ONE_AT_A_TIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        keep(Follow::Music, Some(Now { title: "A song".into(), playing: true, ..Default::default() }));
        let (done, told) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            shown_toggled(Follow::Music);
            let _ = done.send(());
        });
        assert!(told.recv_timeout(std::time::Duration::from_secs(2)).is_ok(), "pausing did not return");
        assert_eq!(NOW[Follow::Music as usize].lock().unwrap().as_ref().map(|now| now.playing), Some(false));
    }

    /// On a desktop, something playing: where its player says it is (reckoned on to now from when it said so), ten
    /// times a second for ten seconds, and each step back.
    #[test]
    #[ignore = "needs something playing on this PC"]
    fn follows_what_plays() {
        let start = std::time::Instant::now();
        let mut last: Option<f64> = None;
        while start.elapsed() < std::time::Duration::from_secs(10) {
            super::look(super::Follow::Music);
            if let Some(pos) = super::position(super::Follow::Music) {
                let back = last.map_or(0.0, |last| last - pos);
                println!("{:6.2} s  reckoned pos: {pos:8.3}{}", start.elapsed().as_secs_f64(), if back > 0.0 { format!("  BACK {back:.3}") } else { String::new() });
                last = Some(pos);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    /// On a desktop, music paused: played again, where it is shown to be goes on from where it was paused, neither
    /// ahead (by the pause) nor back; paused again after.
    #[test]
    #[ignore = "needs music paused on this PC, and plays it four seconds"]
    fn resumes_where_paused() {
        let follow = super::Follow::Music;
        for _ in 0..10 {
            super::look(follow);
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let paused = super::position(follow).expect("music paused, its player saying where it is");
        crate::media::ask_of(follow, "toggle");
        let start = std::time::Instant::now();
        let mut last = paused;
        let mut worst: f64 = 0.0;
        while start.elapsed() < std::time::Duration::from_secs(4) {
            super::look(follow);
            let at = super::position(follow).unwrap_or(last);
            let off = at - (paused + start.elapsed().as_secs_f64());
            println!("{:5.2} s  at {at:8.3}  off {off:+.3}{}", start.elapsed().as_secs_f64(), if at < last - 0.05 { "  BACK" } else { "" });
            worst = worst.max(off.abs());
            last = at;
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        crate::media::ask_of(follow, "toggle");
        assert!(worst < 0.6, "shown as far off as {worst:.3} s from where it was paused, on by the time since");
    }

    /// On a desktop: how long one look takes, and what it sees.
    #[test]
    #[ignore = "needs something playing on this PC"]
    fn times_a_look() {
        super::look(super::Follow::Any);
        let from = std::time::Instant::now();
        for _ in 0..20 {
            super::look(super::Follow::Any);
        }
        println!("a look takes {:.2} ms; it sees {:?}", from.elapsed().as_secs_f64() * 1000.0 / 20.0, super::now());
    }
}

#[cfg(all(test, windows))]
mod seek_tests {
    /// Moves what plays to a minute in and back to where it was: `cargo test --lib -- --ignored seeks_and_back --nocapture`.
    #[test]
    #[ignore = "moves this PC's player"]
    fn seeks_and_back() {
        let was = crate::media::playing().and_then(|playing| playing.timeline).expect("something playing, with a timeline").0;
        println!("was at {was:.1}; asked 60: {}", crate::media::seek(60.0));
        std::thread::sleep(std::time::Duration::from_millis(800));
        println!("now at {:?}", crate::media::playing().and_then(|playing| playing.timeline).map(|t| t.0));
        println!("asked back: {}", crate::media::seek(was));
        std::thread::sleep(std::time::Duration::from_millis(800));
        println!("back at {:?}", crate::media::playing().and_then(|playing| playing.timeline).map(|t| t.0));
    }
}
