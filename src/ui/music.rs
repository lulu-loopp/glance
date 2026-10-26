//! What plays, as a music widget shows it: laid out by the room it is given (a strip, a player, a card of lines, a
//! card stood on end), each of what it can show switched on or off (the cover, what plays and by whom, the line under
//! the one sung now, the lines round it, the spectrum, the controls kept showing), on the cover blurred behind it or
//! on nothing (its words edged to be read on whatever is behind). The line sung now is filled as it is sung; the wheel
//! turns through the lines, and the one turned to is played from where it is pressed. The widget's shell (see
//! `widget`) moves, sizes and keeps it; this draws it and says where in it a press does something.

use std::sync::Arc;
use std::time::{Duration, Instant};

use windows::core::Interface;
use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT, D2D_SIZE_U};
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap1, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_PROPERTIES1, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_LAYER_PARAMETERS1, D2D1_ROUNDED_RECT};
use windows::Win32::Graphics::DirectWrite::{IDWriteTextLayout, DWRITE_HIT_TEST_METRICS, DWRITE_LINE_METRICS};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;

use super::canvas::{Align, Canvas, Color, Family, Fill, Font, Point};
use super::gfx::{rect, Frame, Layer};
use super::icons::Icon;
use super::view::{Hit, HitBox};
use crate::cover::Cover;
use crate::lyrics::Line;
use crate::playing::{Key, Sung};

/// How it is laid out, by the room it has (see `Layout::for_room`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Layout {
    /// One row: the cover small at its start, the line sung now, and at its end the spectrum or the controls. With
    /// no ground and no cover, the line large in its middle and what is under it below (floating lyrics).
    Strip,
    /// The cover at the start as tall as the room; beside it what plays, the line sung now and what is under it; the
    /// spectrum or the controls along the bottom.
    Player,
    /// What plays along the top (the cover small beside it), the lines round the one sung now gliding past it, the
    /// spectrum or the controls along the bottom.
    Card,
    /// The card stood on end: the cover above, what plays under it, then the lines.
    Tall,
    /// Wide and tall: the cover on a record turning as it plays at the start, what plays and the lines beside it, and
    /// along the bottom the spectrum and the controls (kept showing, the spectrum over them; else coming over it).
    Stage,
    /// A strip down, narrow: the cover at its top, the line sung now under it written down (and what is under it beside
    /// it, where there is room for two), what plays or pauses at its foot.
    Column,
}

impl Layout {
    /// The layout for a room `w` × `h` (DIPs) showing `shows`: the one it had (`had`) while the room is within
    /// `HOLD` of where the next begins, so that a room sized about a threshold does not turn back and forth.
    pub fn for_room((w, h): (f32, f32), shows: &Shows, had: Option<Layout>) -> Layout {
        // Controls kept showing over the spectrum take a row more.
        let more = if shows.controls && shows.spectrum { SPECTRUM_ROW + ROW_GAP } else { 0.0 };
        let pick = |slack: f32| {
            if w < COLUMN_UNDER + slack && h >= w {
                Layout::Column
            } else if h < strip_under(more) + slack || w < STRIP_UNDER + slack {
                Layout::Strip
            } else if h < CARD_FROM + more + slack {
                Layout::Player
            } else if shows.cover && w >= STAGE_FROM + slack && h >= CARD_FROM + slack {
                Layout::Stage
            } else if w >= TALL_UNDER + slack {
                Layout::Card
            } else {
                Layout::Tall
            }
        };
        let plain = pick(0.0);
        match had {
            Some(had) if had != plain && (pick(-HOLD) == had || pick(HOLD) == had) => had,
            _ => plain,
        }
    }

    /// As the journal and the studio name it.
    pub fn key(self) -> &'static str {
        match self {
            Layout::Strip => "strip",
            Layout::Player => "player",
            Layout::Card => "card",
            Layout::Tall => "tall",
            Layout::Stage => "stage",
            Layout::Column => "column",
        }
    }
}

/// Where each layout begins (DIPs): the player from this tall (and the strip below this wide), the card from this
/// tall, stood on end below this wide.
/// (As tall as its edges, what plays, a row of the line sung now and the rows along the bottom: no less, so that the
/// line sung now is never shown cut.)
const PLAYER_FROM: f32 = 146.0;
const STRIP_UNDER: f32 = 230.0;
const CARD_FROM: f32 = 250.0;
const TALL_UNDER: f32 = 360.0;
/// The stage from this wide (and as tall as a card).
const STAGE_FROM: f32 = 520.0;
/// A strip down below this wide, as tall as it is wide or taller (narrower and short, a strip across).
const COLUMN_UNDER: f32 = 200.0;
/// How far past where the next layout begins a room keeps the one it has (DIPs).
const HOLD: f32 = 12.0;
/// Below how tall it is a strip (DIPs; `more` as in `Layout::for_room`): half way from the tallest the strip is at
/// rest to the least the player is, so that sized between them it is the one it is nearer, and as it is let go it
/// comes to that one (the player that tall drawn smaller as a whole: see `player_zoom`).
fn strip_under(more: f32) -> f32 {
    (STRIP_MOST + PLAYER_FROM + more) / 2.0
}

/// How much smaller than its own size a player sized `h` tall (DIPs; `more` as in `Layout::for_room`) is drawn: as
/// the least it is, made smaller as a whole, below that (between it and the strip: see `strip_under`); else 1.
fn player_zoom(h: f32, more: f32) -> f32 {
    (h / (PLAYER_FROM + more)).min(1.0)
}

/// How large a layout is drawn in a room `size` large (DIPs) showing `shows`, of its own size (see `player_zoom`).
fn zoom_at(layout: Layout, size: (f32, f32), shows: &Shows) -> f32 {
    let more = if shows.controls && shows.spectrum { SPECTRUM_ROW + ROW_GAP } else { 0.0 };
    if layout == Layout::Player { player_zoom(size.1, more) } else { 1.0 }
}

/// The least a music widget is (DIPs): as small as its cover and its key to play or pause (see `Layout::Column`).
pub const LEAST: (f32, f32) = (56.0, 56.0);
/// The least what plays is, merged with another widget (DIPs): a strip across.
pub const MERGED_LEAST: (f32, f32) = (240.0, 56.0);
/// What plays as a panel slides it out (DIPs, at the panel's size 1): from a side edge as a card, from the top as a
/// player across.
pub const PANEL_SIDE: (f32, f32) = (380.0, 400.0);
pub const PANEL_TOP: (f32, f32) = (640.0, 200.0);
/// The tallest the strip and the player are at rest (DIPs; the player a row more with its controls kept): sized
/// taller, short of where the next layout begins, they come back to it as they are let go.
const STRIP_MOST: f32 = 96.0;
const PLAYER_MOST: f32 = 236.0;

/// The size a music widget sized to `room` (DIPs) comes to rest at: the room, within the range its layout is right
/// in (as a widget of readings settles onto the form it shows, the hand having pulled it past it).
pub fn rests_at(room: (f32, f32), shows: &Shows) -> (f32, f32) {
    let room = (room.0.max(LEAST.0), room.1.max(LEAST.1));
    let more = if shows.controls && shows.spectrum { SPECTRUM_ROW + ROW_GAP } else { 0.0 };
    match Layout::for_room(room, shows, None) {
        Layout::Strip => (room.0, room.1.min(STRIP_MOST)),
        Layout::Player => (room.0, room.1.clamp(PLAYER_FROM + more, PLAYER_MOST + more)),
        Layout::Card | Layout::Tall | Layout::Stage | Layout::Column => room,
    }
}

/// What a music widget shows, as the lyrics module's items say (the same for every one).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Shows {
    pub cover: bool,
    pub title: bool,
    /// Under the line sung now: its translation where it has one (and translations are on), else the next line.
    /// (How many lines show is the room's: one in a strip and a player, those round it in a card and taller.)
    pub translation: bool,
    pub spectrum: bool,
    /// The controls kept showing, in a row of their own; off, they come over the spectrum as the pointer does.
    pub controls: bool,
    /// No ground: its words edged, on whatever is behind it.
    pub clear: bool,
    /// How far in from its edge what it shows begins (DIPs), where not its own (merged into another widget).
    pub inset: Option<u32>,
}

impl Shows {
    pub fn from(view: &serde_json::Value) -> Shows {
        let on = |name: &str| super::prefs::item_on(view, "lyrics", name);
        Shows {
            cover: on("cover"),
            title: on("title"),
            translation: on("translation"),
            spectrum: on("spectrum"),
            controls: on("controls"),
            clear: on("clear"),
            inset: None,
        }
    }
}

/// What it is drawn on: its own ground (the cover blurred), none (its words edged, to be read on whatever is behind),
/// or the ground of the widget it is merged into (that one's look: its words in that ground's ink, dark or light).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Ground {
    Own,
    /// Its own look, its ground drawn under it already (it is the one other widgets are merged into).
    Under,
    Clear,
    /// Its words in the host's type (`family`), as far in from its edge as the host's are (`inset`).
    Host { dark: bool, family: Family, inset: f32 },
}

impl Ground {
    /// As a standalone music widget is drawn, its switches say.
    pub fn of(shows: &Shows) -> Ground {
        if shows.clear { Ground::Clear } else { Ground::Own }
    }
}

/// The words: the line sung now in the card's lines, the others, the translation under it, what plays.
const NOW_FONT: Font = Font::new(Family::Segoe, 25.0, 700.0);
const UNDER_FONT: Font = Font::new(Family::Segoe, 14.0, 500.0);
const TITLE: Font = Font::new(Family::Segoe, 15.0, 700.0);
const ABOUT: Font = Font::new(Family::Segoe, 12.0, 500.0);
/// The player's line sung now and what is under it; how long is left.
const PLAYER_LINE: Font = Font::new(Family::Segoe, 18.0, 700.0);
const PLAYER_UNDER: Font = Font::new(Family::Segoe, 12.5, 500.0);
const TIME: Font = Font::new(Family::Segoe, 11.0, 500.0);
/// The strip's line sung now, and the least it is made to fit its room; large, in the middle (floating lyrics), at
/// most, and what is under it there, of its size.
const STRIP_LINE: Font = Font::new(Family::Segoe, 16.0, 700.0);
const STRIP_LEAST: f32 = 12.5;
const FLOAT_LINE: Font = Font::new(Family::Segoe, 36.0, 800.0);
const FLOAT_UNDER: f32 = 0.5;
/// Its type, each font as it is drawn (in the host's family, merged into another widget).
#[derive(Clone, Copy)]
struct Fonts {
    now: Font,
    under_font: Font,
    title: Font,
    about: Font,
    line: Font,
    under: Font,
    time: Font,
    strip: Font,
    float: Font,
}

impl Fonts {
    fn in_family(family: Option<Family>) -> Fonts {
        let of = |font: Font| match family {
            Some(family) => Font { family, ..font },
            None => font,
        };
        Fonts { now: of(NOW_FONT), under_font: of(UNDER_FONT), title: of(TITLE), about: of(ABOUT), line: of(PLAYER_LINE), under: of(PLAYER_UNDER), time: of(TIME), strip: of(STRIP_LINE), float: of(FLOAT_LINE) }
    }
}

/// The three that are pressed (what goes back, pauses or plays, and skips), and the sound's: the side of each, of its
/// drawing, and of the one that pauses or plays; and the room between them in a row.
const CONTROL: f32 = 32.0;
const KEY_ICON: f32 = 18.0;
const TOGGLE_ICON: f32 = 22.0;
const KEY_GAP: f32 = 2.0;
/// How far inside a key's box its drawing begins (its outer stroke, as the icons are drawn on a 24 grid): a row of
/// them is lined up with what is above it by their drawings, not their boxes.
const KEY_LEAD: f32 = (CONTROL - KEY_ICON) / 2.0 + KEY_ICON * 4.0 / 24.0;
/// The sound's drawing reaches nearer its box's right side (to 23 of the 24).
const SOUND_TRAIL: f32 = (CONTROL - KEY_ICON) / 2.0 + KEY_ICON * 1.0 / 24.0;
/// How long a track takes to grow thicker under the pointer, or thinner after it (seconds).
const LIGHT_UP: f32 = 0.12;
/// How long the pointer is off the sound's key and slider before they close (a frame that saw it elsewhere for a
/// moment does not start them closing).
const SOUND_LINGER: Duration = Duration::from_millis(150);
/// How far up from the stage's foot its rows along the bottom are (DIPs): as a card's, less than its sides' room.
const STAGE_FOOT: f32 = 22.0;
/// How far in from its top and right the buttons set in its corner are (DIPs).
const CORNER: f32 = 12.0;
/// How wide the sound's slider opens beside its key (DIPs), and how long it takes to open (seconds).
const SOUND_WIDE: f32 = 72.0;
const SOUND_OPEN: f32 = 0.18;
/// The spectrum's row of its own, under kept controls, and the room between rows (DIPs).
const SPECTRUM_ROW: f32 = 24.0;
const ROW_GAP: f32 = 12.0;
/// How fast a band of the spectrum falls, at most (of its whole height, a second): from full, in about 0.6 s.
const SPECTRUM_FALL: f32 = 1.7;
/// How long the controls take to come in over the spectrum (or go), seconds.
const CONTROLS_FADE: f32 = 0.18;
/// How long it takes to turn from one layout into the next (as long as a widget of readings takes).
const TURN: Duration = Duration::from_millis(260);
/// How long the record takes to turn once, and its arm to swing onto it or off (seconds).
const RECORD_TURN: f32 = 24.0;
const ARM_SWING: f32 = 0.6;
/// The record's arm, from its pivot (degrees, clockwise from the right): its stylus on the record, and off it.
const ARM_ON: f32 = 100.0;
const ARM_OFF: f32 = 78.0;
/// The stage's rows along the bottom: the spectrum's (DIPs).
const STAGE_SPECTRUM: f32 = 40.0;
/// How long a line turned to by the wheel is held before the lines go back to the one sung now.
const TURNED_HOLD: Duration = Duration::from_secs(3);
/// How far the wheel turns for a line (its notch is 120).
const NOTCH: f32 = 120.0;
/// The dots of the music between lines: their radius, of the size of the words in their place (lit, a quarter
/// larger), how far apart their centres are, and how wide the three are, of it.
const INTERLUDE_DOT: f32 = 0.08;
const INTERLUDE_STEP: f32 = 0.43;
const INTERLUDE_WIDE: f32 = INTERLUDE_DOT * 2.5 + INTERLUDE_STEP * 2.0;

/// What a music widget keeps from frame to frame.
pub struct Music {
    /// The layout it was last drawn in.
    layout: Option<Layout>,
    /// The rows the card's lines have glided to (the line aimed at, fractional while gliding), and how fast.
    rail: f32,
    rail_speed: f32,
    /// The lyrics it last showed (to tell another track's).
    lyrics: Option<usize>,
    last_frame: Instant,
    /// The cover behind it, blurred, and the one it is fading from (with when that began).
    backdrop: Option<(u64, ID2D1Bitmap1)>,
    fading: Option<(ID2D1Bitmap1, Instant)>,
    /// The colours the lyrics are drawn in, easing toward the cover's.
    tint: [f32; 3],
    accent: [f32; 3],
    /// The cover blurred behind it, as its pixels (BGRA, `cover::SIDE` square; see `ground_colour`).
    ground_pixels: Option<Arc<Vec<u8>>>,
    /// Its ground (its shadow, the cover blurred and washed, the cover, what plays), kept and drawn again only as it
    /// changes: what moves at every frame is drawn over it.
    ground: Layer,
    /// How far the controls have come in over the spectrum (0 to 1: in while the pointer is over it).
    controls_in: f32,
    /// Whether the sound's slider is open beside its key (the pointer came onto the key, and has stayed on it or the
    /// slider since), and how far it has opened (0 to 1, eased).
    sound_open: bool,
    sound_in: f32,
    /// How lit the track and the sound's slider are (0 to 1, eased: thicker, their knobs larger, without a jump as
    /// they move under a pointer that stands still).
    track_lit: f32,
    sound_lit: f32,
    /// Whether one of those was drawn short of where it is going, this frame (more frames wanted). Not where they
    /// stand: one not drawn (the controls gone, another layout) is not eased on, and stands short of it for good.
    easing: bool,
    /// Since when the pointer has been off the sound's key and slider (they close only after `SOUND_LINGER`), and the
    /// sound as last read (a frame that read none shows it, and does not close the slider).
    sound_left: Option<Instant>,
    sound_seen: Option<(u8, bool)>,
    /// Its spectrum as shown: up at once with the sound, down no faster than `SPECTRUM_FALL`.
    bands: [f32; crate::sound::BANDS],
    /// Where in the rail of lines the line aimed at is drawn (DIPs, from its top), easing toward where it is to be (it
    /// moves as the lines next to the one aimed at change, and is never seen to jump); how long the last frame was
    /// after the one before (seconds), and whether it is still on its way.
    anchor: Option<Anchor>,
    step: f32,
    anchoring: bool,
    /// (For the tests: where each line of the rail was drawn in the last frame, by its index.)
    #[cfg(test)]
    drawn_lines: Vec<(usize, f32)>,
    /// How far the record has turned (degrees), and how far its arm has swung onto it (0 off it, 1 on it: on as
    /// something plays).
    turned_by: f32,
    arm: f32,
    /// Laid out anew (another layout, or what it shows switched): since when, and where its parts were (they move
    /// from there to where they go, what else it shows coming in, over `TURN`); what it showed last.
    /// (And the layout it was, with whether it showed the lines round the one sung now, whose lines and spectrum fade
    /// out as this one's come in.)
    turning: Option<(Instant, Placed, (Layout, bool))>,
    shown: Option<(Layout, Shows, bool)>,
    /// Lines turned by the wheel from the one sung now, when last turned, and the turn not yet a line.
    turned: i32,
    turned_at: Instant,
    wheeled: f32,
    /// The wheel turned over the sound, short of a step of it (see `wheel`).
    sound_wheeled: i32,
}

/// Where the line aimed at in the rail of lines is drawn (from its top, DIPs): moving from where it was (`from`, the
/// rail then at `from_rail`) to where it is to be (`to`) as the rail glides to the line aimed at (`aimed`), as far
/// along as the rail is: the two arrive together, the lines moving one way.
#[derive(Clone, Copy)]
struct Anchor {
    from: f32,
    from_rail: f32,
    aimed: isize,
    to: f32,
    shown: f32,
}

/// What a frame drew: where a press does something (from its corner, DIPs), and whether it is to be drawn again soon
/// (something plays, or it still moves).
pub struct Painted {
    pub hits: Vec<HitBox>,
    pub busy: bool,
}

/// Its colours with no cover (as it begins, and for a track with none).
const TINT: [f32; 3] = [1.0, 0.9, 0.8];
const ACCENT: [f32; 3] = [1.0, 0.75, 0.6];

impl Default for Music {
    fn default() -> Self {
        Music {
            layout: None,
            rail: -1.0,
            rail_speed: 0.0,
            lyrics: None,
            last_frame: Instant::now(),
            backdrop: None,
            fading: None,
            tint: TINT,
            accent: ACCENT,
            ground_pixels: None,
            ground: Layer::default(),
            controls_in: 0.0,
            sound_open: false,
            sound_in: 0.0,
            track_lit: 0.0,
            sound_lit: 0.0,
            easing: false,
            sound_left: None,
            sound_seen: None,
            bands: [0.0; crate::sound::BANDS],
            anchor: None,
            step: 0.0,
            anchoring: false,
            #[cfg(test)]
            drawn_lines: Vec::new(),
            turned_by: 0.0,
            arm: 0.0,
            turning: None,
            shown: None,
            turned: 0,
            turned_at: Instant::now(),
            wheeled: 0.0,
            sound_wheeled: 0,
        }
    }
}

/// What plays at one moment, as it is drawn.
#[derive(Clone)]
struct Moment {
    lines: Option<Arc<Vec<Line>>>,
    now: Option<usize>,
    since: Option<Instant>,
    position: f64,
    /// How long the track is (seconds), and whether it can be played from elsewhere along it; none where its player
    /// does not say.
    length: Option<(f64, bool)>,
    /// Where its track is being dragged to, while it is (seconds).
    dragged_to: Option<f64>,
    title: String,
    artist: String,
    /// Why there are no lines, where there are none.
    said: Option<String>,
    playing: bool,
    cover: Option<Arc<Cover>>,
    /// Known to have no cover (see `cover::has_none`): the last track's goes, and its colours.
    no_cover: bool,
    sound: crate::sound::Heard,
}

fn moment() -> Moment {
    let heard = crate::playing::music();
    let lang = crate::update::language();
    let (title, artist, playing) = heard.as_ref().map_or_else(Default::default, |heard| (heard.title.clone(), heard.artist.clone(), heard.playing));
    let mut moment = Moment {
        lines: None,
        now: None,
        since: None,
        position: crate::playing::position(crate::playing::Follow::Music).unwrap_or(0.0),
        length: heard.as_ref().and_then(|heard| heard.timeline).map(|timeline| (timeline.length, timeline.seekable)),
        dragged_to: crate::playing::dragged_to(crate::playing::Follow::Music),
        cover: heard.as_ref().filter(|heard| !heard.title.is_empty()).and_then(crate::cover::of),
        no_cover: heard.as_ref().filter(|heard| !heard.title.is_empty()).is_some_and(crate::cover::has_none),
        sound: if playing { crate::sound::heard() } else { crate::sound::Heard::default() },
        title,
        artist,
        said: None,
        playing,
    };
    match heard.as_ref().and_then(|heard| heard.sung.clone()) {
        Some(Sung::Lines { lines, now, since }) => {
            moment.lines = Some(lines);
            moment.now = now;
            moment.since = Some(since);
        }
        Some(Sung::Untimed { netease: true }) => moment.said = Some(lang.pick("网易云音乐没有提供播放进度，可在设置的“音乐”页安装 InfLink-rs 插件以显示歌词", "NetEase Cloud Music does not say how far in it is. Install the InfLink-rs plugin on the Music page of Settings to show lyrics").into()),
        Some(Sung::Untimed { netease: false }) => moment.said = Some(lang.pick("此播放器没有提供播放进度", "This player does not say how far into the track it is").into()),
        Some(Sung::Asking) => moment.said = Some(lang.pick("正在查找歌词…", "Finding the lyrics…").into()),
        Some(Sung::Unknown) => moment.said = Some(lang.pick("没有找到这首歌的歌词", "No lyrics were found for this track").into()),
        None => moment.said = Some(unsaid(heard.as_ref(), lang)),
    }
    moment
}

/// What is said with no lyrics to show and no word of how far in a track is: that nothing plays; or, a player heard
/// that does not tell Windows what it plays (its sound alone heard: an old version of one, or one whose telling is
/// turned off), that it does not (it is heard to play, and can be paused and played, but no more is known of it).
fn unsaid(heard: Option<&crate::playing::Now>, lang: crate::ui::text::Lang) -> String {
    match heard {
        Some(heard) if heard.title.is_empty() => {
            let app = if heard.app.is_empty() { lang.pick("此播放器", "This player") } else { heard.app.as_str() };
            lang.pick(&format!("{app}未向系统提供歌曲信息"), &format!("{app} does not tell Windows what it plays")).to_string()
        }
        _ => lang.pick("当前没有正在播放的歌曲", "Nothing is playing").into(),
    }
}

/// The example the settings show while no music plays: its lines (each with its translation), a line every
/// `EXAMPLE_LINE` seconds, sung word by word, over and over.
const EXAMPLE: [(&str, &str); 8] = [
    ("晨光落在窗台", "Morning light falls on the sill"),
    ("城市还没醒来", "The city has not woken yet"),
    ("我把昨天折好", "I fold yesterday away"),
    ("放进口袋", "And keep it in my pocket"),
    ("风从街角吹过", "A wind comes round the corner"),
    ("带走一点寂寞", "And takes a little loneliness"),
    ("路灯一盏盏熄灭", "The street lamps go out one by one"),
    ("像在说再见", "As if they said goodbye"),
];
const EXAMPLE_LINE: f64 = 4.0;

/// The example's lines, made once.
fn example_lines() -> Arc<Vec<Line>> {
    static LINES: std::sync::OnceLock<Arc<Vec<Line>>> = std::sync::OnceLock::new();
    LINES
        .get_or_init(|| {
            let lines = EXAMPLE.iter().enumerate().map(|(index, (text, translation))| {
                let at = index as f64 * EXAMPLE_LINE;
                let count = text.chars().count() as f64;
                let step = (EXAMPLE_LINE * 0.8) / count;
                let words = text.chars().enumerate().map(|(i, c)| crate::lyrics::Word { at: at + i as f64 * step, end: at + (i + 1) as f64 * step, text: c.to_string() }).collect();
                Line { at, end: Some(at + EXAMPLE_LINE * 0.8), text: text.to_string(), words, backing: None, translation: Some(translation.to_string()) }
            });
            Arc::new(lines.collect())
        })
        .clone()
}

/// The example's cover: a warm sky over a dark sea, the sun low in it; made once.
fn example_cover() -> Arc<Cover> {
    static COVER: std::sync::OnceLock<Arc<Cover>> = std::sync::OnceLock::new();
    COVER
        .get_or_init(|| {
            let side = crate::cover::SIDE as usize;
            let mut pixels = vec![0u8; side * side * 4];
            for y in 0..side {
                for x in 0..side {
                    let (fx, fy) = (x as f32 / side as f32, y as f32 / side as f32);
                    let sun = (((fx - 0.62).powi(2) + (fy - 0.42).powi(2)).sqrt() < 0.11) as u8 as f32;
                    let [r, g, b] = if fy < 0.6 { [0.98 - 0.25 * fy, 0.55 + 0.15 * fy, 0.42 + 0.12 * fx] } else { [0.12, 0.2 + 0.1 * fx, 0.36] };
                    let at = (y * side + x) * 4;
                    pixels[at] = ((b * (1.0 - sun) + 0.85 * sun) * 255.0) as u8;
                    pixels[at + 1] = ((g * (1.0 - sun) + 0.95 * sun) * 255.0) as u8;
                    pixels[at + 2] = ((r * (1.0 - sun) + sun) * 255.0) as u8;
                    pixels[at + 3] = 255;
                }
            }
            Arc::new(Cover::new(pixels, [0.85, 0.52, 0.42], [0.98, 0.62, 0.38]))
        })
        .clone()
}

/// The example as it is `t` seconds in: its line sung now, from when, and its spectrum moving with it.
fn example(t: f64, now: Instant) -> Moment {
    let length = EXAMPLE.len() as f64 * EXAMPLE_LINE;
    let mut bands = [0.0; crate::sound::BANDS];
    for (i, band) in bands.iter_mut().enumerate() {
        let x = i as f64;
        *band = (0.55 + 0.3 * (t * 2.3 + x * 0.45).sin() * (t * 0.7 + x * 0.21).cos() - x * 0.012).clamp(0.05, 1.0) as f32;
    }
    let sound = crate::sound::Heard { level: 0.7, bands, beat: (0.5 + 0.5 * (t * 4.0).sin()).max(0.0) as f32 };
    Played { lines: example_lines(), position: t.rem_euclid(length), length, title: "晨光序曲".into(), artist: "Glance".into(), cover: example_cover(), sound }.moment(now)
}

/// A track playing as it is told, not as a player tells it: its lines, how far in it is and how long (seconds), what
/// it is, its cover and what is heard (the settings' example, the studio's song).
pub struct Played {
    pub lines: Arc<Vec<Line>>,
    pub position: f64,
    pub length: f64,
    pub title: String,
    pub artist: String,
    pub cover: Arc<Cover>,
    pub sound: crate::sound::Heard,
}

impl Played {
    /// As it is drawn at `now`: the line sung at its position, come when that line began.
    fn moment(self, now: Instant) -> Moment {
        let sung = crate::lyrics::sung_at(&self.lines, self.position);
        let since = sung.map(|index| now.checked_sub(Duration::from_secs_f64((self.position - self.lines[index].at).max(0.0))).unwrap_or(now));
        Moment {
            lines: Some(self.lines),
            now: sung,
            since,
            position: self.position,
            length: Some((self.length, false)),
            dragged_to: None,
            title: self.title,
            artist: self.artist,
            said: None,
            playing: true,
            cover: Some(self.cover),
            no_cover: false,
            sound: self.sound,
        }
    }
}

/// Whether any music is followed now (else the settings show the example: see `Music::paint_example`).
pub fn heard() -> bool {
    crate::playing::music().is_some_and(|music| !music.title.is_empty())
}

/// The line at `index` of what plays now played from its start (pressed where the wheel turned to it).
pub fn play_line(index: u16) {
    let Some(Sung::Lines { lines, .. }) = crate::playing::music().and_then(|now| now.sung) else { return };
    if let Some(line) = lines.get(index as usize) {
        crate::playing::act_on(crate::playing::Follow::Music, crate::playing::Act::Seek(line.at));
    }
}

impl Music {
    /// The wheel turned by `delta` (a notch, 120) over it: the lines turned by as many, from the one sung now (or the
    /// one turned to), down toward the end for a turn toward the user.
    pub fn wheel(&mut self, delta: i16, on_sound: bool) {
        // Over the sound's key or its slider (`on_sound`, as its host finds it where the wheel turned): the sound
        // turned, two in a hundred a notch (as Windows' own does); a part of a notch kept toward the next (a touchpad
        // turns it by little ones).
        if on_sound {
            let steps = crate::playing::wheel_steps(&mut self.sound_wheeled, delta, 2);
            if let (Some((level, _)), true) = (crate::playing::sound(), steps != 0) {
                let to = (i32::from(level) + steps).clamp(0, 100) as u8;
                crate::playing::set_sound(Some(to), Some(false));
            }
            return;
        }
        self.wheeled -= delta as f32;
        let lines = (self.wheeled / NOTCH).trunc();
        if lines == 0.0 {
            return;
        }
        self.wheeled -= lines * NOTCH;
        self.turned += lines as i32;
        self.turned_at = Instant::now();
    }

    /// Its own ground alone, `size` large (its shadow, the cover blurred under a wash of its colour), under a widget
    /// merged into it: what each part shows drawn over it (see `Ground::Host`).
    pub fn paint_ground(&mut self, frame: &Frame, size: (f32, f32)) {
        let heard = crate::playing::music();
        let now = Instant::now();
        if let Some(cover) = heard.as_ref().filter(|heard| !heard.title.is_empty()).and_then(crate::cover::of) {
            self.tint = toward(self.tint, readable(cover.base, 0.72), 0.2);
            self.accent = toward(self.accent, readable(cover.accent, 0.62), 0.2);
            self.backdrop(frame, &cover, now);
        } else if heard.as_ref().filter(|heard| !heard.title.is_empty()).is_some_and(crate::cover::has_none) {
            self.tint = toward(self.tint, TINT, 0.2);
            self.accent = toward(self.accent, ACCENT, 0.2);
            self.no_backdrop(now);
        }
        let shown = self.fading.as_ref().map_or(1.0, |(_, since)| (now.saturating_duration_since(*since).as_secs_f32() / 1.2).min(1.0));
        let before = self.fading.as_ref().map(|(before, _)| before.clone());
        let backdrop = Backdrop { size, radius: RADIUS, backdrop: self.backdrop.as_ref().map(|(_, bitmap)| bitmap.clone()), before, shown, accent: self.accent, tint: self.tint, own: true };
        let _ = backdrop.paint(frame, |_| {});
    }

    /// Back to the line sung now (a line turned to was pressed).
    pub fn back_to_now(&mut self) {
        (self.turned, self.wheeled) = (0, 0.0);
    }

    /// What it drew with, let go of (its window hidden).
    pub fn release(&mut self) {
        self.ground.release();
        self.backdrop = None;
        self.fading = None;
    }

    /// Draws it in `size` (DIPs, from the frame's origin), showing `shows`, at `scale` physical pixels a DIP, the
    /// pointer at `pointer` (DIPs, from its corner) while it is over it.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(&mut self, frame: &Frame, size: (f32, f32), shows: &Shows, ground: Ground, scale: f32, pointer: Option<(f32, f32)>, now: Instant) -> Painted {
        self.paint_moment(frame, size, shows, ground, scale, pointer, now, moment())
    }

    /// As `paint`, the example the settings show while no music plays, `t` seconds into it (see `EXAMPLE`).
    #[allow(clippy::too_many_arguments)]
    pub fn paint_example(&mut self, frame: &Frame, size: (f32, f32), shows: &Shows, ground: Ground, scale: f32, pointer: Option<(f32, f32)>, now: Instant, t: f64) -> Painted {
        self.paint_moment(frame, size, shows, ground, scale, pointer, now, example(t, now))
    }

    /// As `paint`, a track playing as `played` tells it (see `Played`).
    #[cfg(feature = "studio")]
    #[allow(clippy::too_many_arguments)]
    pub fn paint_played(&mut self, frame: &Frame, size: (f32, f32), shows: &Shows, ground: Ground, scale: f32, pointer: Option<(f32, f32)>, now: Instant, played: Played) -> Painted {
        self.paint_moment(frame, size, shows, ground, scale, pointer, now, played.moment(now))
    }

    /// As `paint`, what plays as `moment` says.
    #[allow(clippy::too_many_arguments)]
    fn paint_moment(&mut self, frame: &Frame, size: (f32, f32), shows: &Shows, ground: Ground, scale: f32, pointer: Option<(f32, f32)>, now: Instant, moment: Moment) -> Painted {
        // A player less tall than the least it is (sized toward the strip, not yet nearer it): that least, smaller as
        // a whole (see `player_zoom`).
        let zoom = zoom_at(Layout::for_room(size, shows, self.layout), size, shows);
        if zoom >= 1.0 {
            return self.paint_laid(frame, size, shows, ground, scale, pointer, now, moment, 1.0);
        }
        let mut painted = Painted { hits: Vec::new(), busy: false };
        frame.turned(0.0, 0.0, 0.0, (zoom, zoom), |frame| {
            let _moving = frame.moving_while(true);
            let pointer = pointer.map(|(x, y)| (x / zoom, y / zoom));
            painted = self.paint_laid(frame, (size.0 / zoom, size.1 / zoom), shows, ground, scale * zoom, pointer, now, moment, zoom);
        });
        for (x, y, w, h, _) in &mut painted.hits {
            (*x, *y, *w, *h) = (*x * zoom, *y * zoom, *w * zoom, *h * zoom);
        }
        painted
    }

    /// As `paint_moment`, in `size` as it is laid out there, drawn `zoom` as large (see `zoom_at`).
    #[allow(clippy::too_many_arguments)]
    fn paint_laid(&mut self, frame: &Frame, size: (f32, f32), shows: &Shows, ground: Ground, scale: f32, pointer: Option<(f32, f32)>, now: Instant, mut moment: Moment, zoom: f32) -> Painted {
        let step = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        self.step = step;
        self.easing = false;
        // The colours, toward the cover's (a little each frame); with none, toward its own.
        let ease = (step * 3.0).min(1.0);
        if let Some(cover) = &moment.cover {
            self.tint = toward(self.tint, readable(cover.base, 0.72), ease);
            self.accent = toward(self.accent, readable(cover.accent, 0.62), ease);
        } else if moment.no_cover {
            self.tint = toward(self.tint, TINT, ease);
            self.accent = toward(self.accent, ACCENT, ease);
        }
        // Turned to a line a while ago and left there: back to the one sung now.
        if self.turned != 0 && now.saturating_duration_since(self.turned_at) >= TURNED_HOLD {
            self.back_to_now();
        }
        // Other lyrics (another track): from the line sung now, not turned (turned for the lyrics before, they were
        // first shown turned as far, and sprang back).
        let key = moment.lines.as_ref().map(|lines| Arc::as_ptr(lines) as usize);
        let other_lyrics = key != self.lyrics;
        if other_lyrics {
            self.back_to_now();
        }
        // Turned no further than the lines go.
        if let (Some(lines), true) = (&moment.lines, self.turned != 0) {
            // (From the line sung now, or from before the first where none is: to the first line at least, the last at
            // most; with none sung yet, the last was out of reach.)
            let at = moment.now.map_or(-1, |now| now as i32);
            self.turned = self.turned.clamp(-at.max(0), lines.len() as i32 - 1 - at);
        }
        // The rail, sprung toward the line aimed at (as Apple Music's lyrics move: quick, then settling).
        // (The line turned to, as every part that shows it finds it: see `turned_to`.)
        let count = moment.lines.as_ref().map_or(0, |lines| lines.len());
        let target = turned_to(moment.now, self.turned, count).map_or(moment.now.map_or(-1.0, |now| now as f32), |index| index as f32);
        if other_lyrics {
            self.lyrics = key;
            self.rail = target;
            self.rail_speed = 0.0;
        }
        let (stiffness, damping, mass) = (142.0, 28.0, 0.82);
        let mut left = step;
        while left > 0.0 {
            let dt = left.min(1.0 / 240.0);
            let force = -stiffness * (self.rail - target) - damping * self.rail_speed;
            self.rail_speed += force / mass * dt;
            self.rail += self.rail_speed * dt;
            left -= dt;
        }
        let settling = (self.rail - target).abs() > 0.002 || self.rail_speed.abs() > 0.002;
        // Near enough and slow enough, it is there (a spring only nears its rest, by less and less, never reaching it).
        if !settling {
            self.rail = target;
            self.rail_speed = 0.0;
        }
        // The controls, in toward the pointer over it and out after it (kept showing, always in).
        let controls_to = if pointer.is_some() || shows.controls { 1.0 } else { 0.0 };
        // (A frame long after the last, as the first after a rest is, moves them no further than a short one would:
        // they come and go smoothly, not half way at once.)
        let fade_step = step.min(1.0 / 30.0);
        self.controls_in = if controls_to > self.controls_in { (self.controls_in + fade_step / CONTROLS_FADE).min(controls_to) } else { (self.controls_in - fade_step / CONTROLS_FADE).max(controls_to) };
        let coming = self.controls_in != controls_to;
        // The spectrum, falling where the sound has fallen (drawn from what it shows, not what is heard).
        for (shown, heard) in self.bands.iter_mut().zip(moment.sound.bands) {
            *shown = if heard >= *shown { heard } else { (*shown - step * SPECTRUM_FALL).max(heard) };
        }
        moment.sound.bands = self.bands;
        let falling = !moment.playing && self.bands.iter().any(|&band| band > 0.0);
        if moment.playing {
            self.turned_by = (self.turned_by + step * 360.0 / RECORD_TURN) % 360.0;
        }
        let arm_to = if moment.playing { 1.0 } else { 0.0 };
        // (Each way no further than where it is going: on the record, it rests there.)
        self.arm = if arm_to > self.arm { (self.arm + step / ARM_SWING).min(arm_to) } else { (self.arm - step / ARM_SWING).max(arm_to) };
        let swinging = self.arm != arm_to;
        let layout = Layout::for_room(size, shows, self.layout);
        if self.layout != Some(layout) {
            crate::journal::note(format!("music widget: {}", layout.key()));
            self.layout = Some(layout);
        }
        // Laid out anew (the player too, from its line to its lines, or back): its parts move from where they were,
        // from where they had got to if they were moving still. (Where they are as drawn, each layout as large as it is
        // drawn: see `zoom_at`; kept so, and in this one's terms as it is drawn.)
        let lines = layout == Layout::Player && player_shows_lines(size, shows);
        let drawn = (size.0 * zoom, size.1 * zoom);
        if let Some((was, showed, had_lines)) = self.shown.filter(|shown| *shown != (layout, *shows, lines)) {
            let was_zoom = zoom_at(was, drawn, &showed);
            let was_at = placed(was, (drawn.0 / was_zoom, drawn.1 / was_zoom), &showed).scaled(was_zoom);
            let from = match self.turning.and_then(|(since, from, _)| turned(now, since).map(|t| (from, t))) {
                Some((from, t)) => tween(from, was_at, t),
                None => was_at,
            };
            self.turning = Some((now, from, (was, had_lines)));
        }
        self.shown = Some((layout, *shows, lines));
        let turn = self.turning.and_then(|(since, from, _)| turned(now, since).map(|t| (from.scaled(1.0 / zoom), t)));
        let was = self.turning.map(|(.., was)| was);
        if turn.is_none() {
            self.turning = None;
        }
        // On a light ground, its colours deepened to be read on it, and its words dark.
        let light = matches!(ground, Ground::Host { dark: false, .. });
        let family = match ground {
            Ground::Host { family, .. } => Some(family),
            _ => None,
        };
        let shows = &Shows { inset: match ground {
            Ground::Host { inset, .. } => Some(inset.round() as u32),
            _ => shows.inset,
        }, ..*shows };
        let (tint, accent) = if light { (deep(self.tint), deep(self.accent)) } else { (self.tint, self.accent) };
        let ink = if light { 0x1A1A1A } else { 0xFFFFFF };
        let mut drawing = Drawing { music: self, frame, now, moment: &moment, shows, ground, ink, edged: ground == Ground::Clear, light, tint, accent, fonts: Fonts::in_family(family), scale, pointer, size, turn, hits: Vec::new(), clips: Vec::new() };
        drawing.ground(layout);
        drawing.disc(layout);
        // Turning from another layout: what that one showed off its ground (its lines, its spectrum) fades out as this
        // one's comes in, never a frame with neither. It is drawn as large as this one, its parts where it puts them
        // there; what it would change of what is kept from frame to frame, and where it could be pressed, stay this
        // one's.
        if let (Some((_, t)), Some((was, had_lines))) = (turn, was.filter(|was| *was != (layout, lines))) {
            let kept = drawing.music.kept();
            let (hits, turning) = (drawing.hits.len(), drawing.turn.take());
            let everything = windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F { left: -f32::MAX, top: -f32::MAX, right: f32::MAX, bottom: f32::MAX };
            let layer = D2D1_LAYER_PARAMETERS1 { contentBounds: everything, opacity: 1.0 - t, ..Default::default() };
            unsafe { frame.dc.PushLayer(&layer, None) };
            drawing.content(was, had_lines);
            unsafe { frame.dc.PopLayer() };
            drawing.turn = turning;
            drawing.hits.truncate(hits);
            drawing.music.restore(kept);
        }
        // Turning into this layout: what is not on its ground comes in as its parts move.
        if let Some((_, t)) = turn {
            let everything = windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F { left: -f32::MAX, top: -f32::MAX, right: f32::MAX, bottom: f32::MAX };
            let layer = D2D1_LAYER_PARAMETERS1 { contentBounds: everything, opacity: t, ..Default::default() };
            unsafe { frame.dc.PushLayer(&layer, None) };
        }
        drawing.content(layout, lines);
        if turn.is_some() {
            unsafe { frame.dc.PopLayer() };
        }
        let hits = drawing.hits;
        Painted { hits, busy: self.easing || self.anchoring || moment.playing || settling || coming || falling || swinging || self.fading.is_some() || self.turned != 0 || moment.dragged_to.is_some() || turn.is_some() }
    }
}

/// What drawing a layout's content changes of what is kept from one frame to the next (see `Drawing::content`).
struct Kept {
    anchor: Option<Anchor>,
    anchoring: bool,
    easing: bool,
    sound: (bool, f32, f32, Option<Instant>, Option<(u8, bool)>),
    track_lit: f32,
    #[cfg(test)]
    drawn_lines: Vec<(usize, f32)>,
}

impl Music {
    fn kept(&self) -> Kept {
        Kept {
            anchor: self.anchor,
            anchoring: self.anchoring,
            easing: self.easing,
            sound: (self.sound_open, self.sound_in, self.sound_lit, self.sound_left, self.sound_seen),
            track_lit: self.track_lit,
            #[cfg(test)]
            drawn_lines: self.drawn_lines.clone(),
        }
    }

    fn restore(&mut self, kept: Kept) {
        (self.anchor, self.anchoring, self.easing, self.track_lit) = (kept.anchor, kept.anchoring, kept.easing, kept.track_lit);
        (self.sound_open, self.sound_in, self.sound_lit, self.sound_left, self.sound_seen) = kept.sound;
        #[cfg(test)]
        {
            self.drawn_lines = kept.drawn_lines;
        }
    }
}

/// One frame being drawn.
struct Drawing<'a> {
    music: &'a mut Music,
    frame: &'a Frame<'a>,
    /// The moment it is drawn for (every motion in it timed from this, not from the clock as it draws).
    now: Instant,
    moment: &'a Moment,
    shows: &'a Shows,
    ground: Ground,
    /// Its words' colour (white, or dark on a light ground), whether they are edged (no ground), whether the ground
    /// is light, and the cover's colours as they are drawn on it.
    ink: u32,
    edged: bool,
    light: bool,
    tint: [f32; 3],
    accent: [f32; 3],
    /// Its type: its own, or the host's.
    fonts: Fonts,
    scale: f32,
    pointer: Option<(f32, f32)>,
    size: (f32, f32),
    /// Turning from another layout: where its parts were, and how far they have come (0 to 1, eased).
    turn: Option<(Placed, f32)>,
    hits: Vec<HitBox>,
    /// What is drawn within (see `clip`): each, and how many hits there were as it began.
    clips: Vec<((f32, f32, f32, f32), usize)>,
}

/// How far a turn begun at `since` has come by `now` (0 to 1, eased); none once it is over.
fn turned(now: Instant, since: Instant) -> Option<f32> {
    let t = now.saturating_duration_since(since).as_secs_f32() / TURN.as_secs_f32();
    (t < 1.0).then(|| super::morph::ease(t))
}

/// Its parts `t` of the way from where they were (`from`) to where they go (`to`): each there is in both moved, the
/// others where they go.
/// The size of the stage's line sung now, `h` tall: larger as it is taller.
fn stage_line(h: f32) -> f32 {
    (h / 16.0).clamp(18.0, NOW_FONT.size)
}

/// How far across the record its label is, of its own (the cover on it, in the stage).
const RECORD_LABEL: f32 = 0.62;

/// The corners of a cover `side` across, on a card or beside what plays.
fn cover_corner(side: f32) -> f32 {
    if side < 50.0 { 6.0 } else { 9.0 }
}

impl Placed {
    /// Its parts as they are drawn `k` as large.
    fn scaled(self, k: f32) -> Placed {
        Placed {
            cover: self.cover.map(|(x, y, side)| (x * k, y * k, side * k)),
            title: self.title.map(|(x, y, w, small)| (x * k, y * k, w * k, small)),
            title_zoom: self.title_zoom * k,
            below: self.below * k,
            left: self.left * k,
            width: self.width * k,
            ..self
        }
    }
}

fn tween(from: Placed, to: Placed, t: f32) -> Placed {
    let at = |a: f32, b: f32| a + (b - a) * t;
    Placed {
        cover: match (from.cover, to.cover) {
            (Some(a), Some(b)) => Some((at(a.0, b.0), at(a.1, b.1), at(a.2, b.2))),
            (_, b) => b,
        },
        title: match (from.title, to.title) {
            (Some(a), Some(b)) => Some((at(a.0, b.0), at(a.1, b.1), at(a.2, b.2), b.3)),
            (_, b) => b,
        },
        round: at(from.round, to.round),
        record: at(from.record, to.record),
        title_zoom: at(from.title_zoom, to.title_zoom),
        below: at(from.below, to.below),
        left: at(from.left, to.left),
        width: at(from.width, to.width),
    }
}

/// The stage's record: its middle and how wide it is, in a room `w` × `h` showing `shows` (as tall as the room above
/// the rows along the bottom as they are, no wider than two fifths of it).
fn stage_record((w, h): (f32, f32), shows: &Shows) -> ((f32, f32), f32) {
    let pad = pad(Layout::Stage);
    // The rows along the bottom: kept showing, the controls with the spectrum over them; else the spectrum alone (the
    // controls come over it), or the controls' row where there is no spectrum.
    let rows = match (shows.controls, shows.spectrum) {
        (true, true) => CONTROL + ROW_GAP + STAGE_SPECTRUM,
        (false, true) => STAGE_SPECTRUM,
        _ => CONTROL,
    };
    let above = h - pad - STAGE_FOOT - rows - 14.0;
    let side = above.min(w * 0.4).max(0.0);
    // (In the middle of the room above the rows along the bottom, where it is narrower than that is tall.)
    ((pad + side / 2.0, pad + above.max(0.0) / 2.0), side)
}

/// How tall the rows along the bottom are (DIPs): the controls' (or the spectrum's, the controls coming over it), and
/// with the controls kept, the spectrum's over them.
fn rows_tall(shows: &Shows) -> f32 {
    CONTROL + if shows.controls && shows.spectrum { ROW_GAP + SPECTRUM_ROW } else { 0.0 }
}

/// Whether the player shows the lines round the one sung now, `size` large, showing `shows` (else the line sung now
/// and what is under it).
fn player_shows_lines(size: (f32, f32), shows: &Shows) -> bool {
    let at = placed(Layout::Player, size, shows);
    let bottom = size.1 - pad(Layout::Player) - rows_tall(shows) - 8.0;
    // Room for a row of the line sung now and a row of each line next to it, smaller (as the rail draws them), and
    // the room between them.
    let line = PLAYER_LINE.size * 1.35;
    bottom - at.below >= line * (1.0 + 2.0 * 0.68) + 2.0 * 10.0
}

/// Each layout's room inside its edge as `shows` says: its own, or the host's.
fn pad_of(layout: Layout, shows: &Shows) -> f32 {
    shows.inset.map_or(pad(layout), |inset| inset as f32)
}

/// What a strip `w` wide (DIPs, `pad` inside its edges, its words from `start`: past its cover, or, `centred`, the
/// words in its middle with no cover) ends its row with, showing `shows`: where its keys go and where its spectrum
/// does (from, how wide), each only where it fits in its row after where its words begin (a strip narrower than they
/// need shows its cover and its words alone: they reached out of its left edge, over its cover); where its words end;
/// and, centred, how far in from either side they keep (clear of the keys, kept showing or come with the pointer).
/// (The keys' plates inside its padding, not their glyphs alone: as low as it is, a plate under the pointer reached
/// its edge, and the keys looked pushed to its end.)
fn strip_ends(w: f32, start: f32, pad: f32, shows: &Shows, centred: bool) -> StripEnds {
    let keys_wide = 3.0 * CONTROL + 2.0 * KEY_GAP;
    let mut end = w - pad;
    let mut ends = StripEnds { keys: None, spectrum: None, end, side: 0.0 };
    if centred {
        // The words clear of the keys at either side, in the middle: as wide on each side as the keys are.
        let side = keys_wide + 14.0;
        if w - 2.0 * (pad + side) >= 0.0 {
            ends.keys = Some(if shows.controls { end - keys_wide } else { pad });
            ends.side = side;
        }
        return ends;
    }
    // (With the room kept between it and the words.)
    let fits = |end: f32, taken: f32| end - taken >= start;
    if shows.controls {
        if fits(end, keys_wide + 10.0) {
            ends.keys = Some(end - keys_wide);
            end -= keys_wide + 10.0;
            if shows.spectrum && fits(end, 80.0) {
                ends.spectrum = Some((end - 70.0, 70.0));
                end -= 80.0;
            }
        }
    } else {
        // The spectrum where the keys come with the pointer (as wide as they are, or a little wider).
        let wide = keys_wide.max(84.0);
        if fits(end, wide + 12.0) {
            ends.spectrum = shows.spectrum.then_some((end - wide, wide));
            ends.keys = Some(end - keys_wide);
            end -= wide + 12.0;
        }
    }
    ends.end = end;
    ends
}

/// What a strip's row ends with (see `strip_ends`).
#[derive(Debug, PartialEq)]
struct StripEnds {
    keys: Option<f32>,
    spectrum: Option<(f32, f32)>,
    end: f32,
    side: f32,
}

/// Each layout's room inside its edge, and the corners of its ground (DIPs).
fn pad(layout: Layout) -> f32 {
    match layout {
        Layout::Strip | Layout::Column => 10.0,
        Layout::Player => 18.0,
        Layout::Card | Layout::Tall => 22.0,
        Layout::Stage => 30.0,
    }
}
/// How much larger than its longer side the cover blurred behind it is laid, and how darkened.
const BACKDROP_GROWN: f32 = 1.35;
const BACKDROP_DARKENED: f32 = 0.42;
/// Its corners' radius, with its own ground.
pub const RADIUS: f32 = 14.0;
/// The strip's cover inside another widget, at most (DIPs).
const HOSTED_COVER: f32 = 44.0;
/// How tall a line's translation under it is in the lines round the one sung now (DIPs, at the line sung now's size).
const TRANSLATION_ROW: f32 = 20.0;
/// The cover in the card's top row, beside what plays (DIPs).
const CARD_COVER: f32 = 64.0;

/// Where its parts go, as `layout` lays them out in `size` showing `shows`: the cover (its corner and side), what
/// plays (its corner, how wide, centred or not), and where the rest begins below them.
#[derive(Clone, Copy)]
struct Placed {
    cover: Option<(f32, f32, f32)>,
    /// How round the cover is (0: a card's corners, 1: round, the record's label), and how much of the record round
    /// it shows (0 to 1): a layout turning into the stage or out of it, the cover moves into the record's middle or
    /// out of it, rounding as it goes, and the record grows in round it or goes.
    round: f32,
    record: f32,
    title: Option<(f32, f32, f32, bool)>,
    /// How large what plays is drawn, of its size (the stage's, larger as it is taller, as its lines are).
    title_zoom: f32,
    below: f32,
    /// Where the words beside the cover begin (the player and the strip), and how wide they are.
    left: f32,
    width: f32,
}

/// The player's cover, `w` × `h` large showing `shows`: as tall as the room above the rows along the bottom, no wider
/// than a third.
fn player_side((w, h): (f32, f32), shows: &Shows) -> f32 {
    let pad = pad_of(Layout::Player, shows);
    (h - 2.0 * pad - rows_tall(shows) - 10.0).max(0.0).min(w * 0.36).max(0.0)
}

fn placed(layout: Layout, (w, h): (f32, f32), shows: &Shows) -> Placed {
    let pad = pad_of(layout, shows);
    match layout {
        Layout::Strip => {
            // (Inside another widget, no taller than a lane's line of words and what is under it, in its middle.) No
            // larger than the player's as it begins, as drawn there (the least player, smaller as a whole: see
            // `player_zoom`): made smaller from the player into the strip, the cover grows no larger on the way (as
            // tall as the strip, it grew as the strip came, and then shrank).
            let more = if shows.controls && shows.spectrum { SPECTRUM_ROW + ROW_GAP } else { 0.0 };
            let zoom = player_zoom(strip_under(more), more);
            let player = player_side((w / zoom, PLAYER_FROM + more), shows) * zoom;
            let side = (h - 2.0 * pad).min(if shows.inset.is_some() { HOSTED_COVER } else { f32::MAX }).min(player).max(0.0);
            let cover = shows.cover.then_some((pad, (h - side) / 2.0, side));
            let left = if shows.cover { pad + side + 12.0 } else { pad + 4.0 };
            Placed { cover, round: 0.0, record: 0.0, title: None, title_zoom: 1.0, below: 0.0, left, width: (w - pad - left).max(0.0) }
        }
        // The cover as tall as the room above the rows along the bottom (which go all across), no wider than a third.
        // (Narrower than the room above the rows is tall, the cover and the column beside it, level with its top,
        // stand together in the middle of that room.)
        Layout::Player => {
            let room = (h - 2.0 * pad - rows_tall(shows) - 10.0).max(0.0);
            let side = player_side((w, h), shows);
            let top = if shows.cover { pad + (room - side) / 2.0 } else { pad };
            let cover = shows.cover.then_some((pad, top, side));
            let left = if shows.cover { pad + side + 16.0 } else { pad };
            let width = (w - left - pad).max(0.0);
            let title = shows.title.then_some((left, top - 2.0, width, false));
            Placed { cover, round: 0.0, record: 0.0, title, title_zoom: 1.0, below: if shows.title { top + 46.0 } else { top + 4.0 }, left, width }
        }
        Layout::Card => {
            let cover = shows.cover.then_some((pad, pad, CARD_COVER));
            let tx = if shows.cover { pad + CARD_COVER + 14.0 } else { pad };
            let title = shows.title.then_some((tx, pad + if shows.cover { 12.0 } else { 0.0 }, (w - tx - pad).max(0.0), false));
            let below = pad + if shows.cover { CARD_COVER + 12.0 } else if shows.title { 44.0 } else { 0.0 };
            Placed { cover, round: 0.0, record: 0.0, title, title_zoom: 1.0, below, left: pad, width: (w - 2.0 * pad).max(0.0) }
        }
        // The record drawn as it turns (not on the ground); what plays at the top of the column beside it.
        // (Level with the record's top: the record and the column beside it together in the middle.)
        Layout::Stage => {
            let ((cx, cy), side) = stage_record((w, h), shows);
            let top = cy - side / 2.0;
            // The cover the record's label (see `RECORD_LABEL`), round, the record round it.
            let label = side / 2.0 * RECORD_LABEL;
            let cover = Some((cx - label, cy - label, 2.0 * label));
            let left = pad + side + side * 0.28 + 12.0;
            let width = (w - left - pad).max(0.0);
            let title = shows.title.then_some((left, top + 6.0, width, false));
            // (As large as its lines are, of theirs at the least: see `stage`.)
            let title_zoom = stage_line(h) / 18.0;
            Placed { cover, round: 1.0, record: 1.0, title, title_zoom, below: top + if shows.title { 58.0 * title_zoom } else { 0.0 }, left, width }
        }
        // The cover at the top, as wide as it is, no taller than two fifths of it.
        Layout::Column => {
            let wide = (w - 2.0 * pad).max(0.0);
            let side = wide.min(h * 0.4).max(0.0);
            let cover = shows.cover.then_some(((w - side) / 2.0, pad, side));
            let below = pad + if shows.cover { side + 10.0 } else { 0.0 };
            Placed { cover, round: 0.0, record: 0.0, title: None, title_zoom: 1.0, below, left: pad, width: wide }
        }
        Layout::Tall => {
            let wide = (w - 2.0 * pad).max(0.0);
            let side = wide.min(h * 0.3);
            let cover = shows.cover.then_some(((w - side) / 2.0, pad, side));
            let mut y = pad + if shows.cover { side + 12.0 } else { 0.0 };
            let title = shows.title.then_some((pad, y, wide, true));
            if shows.title {
                y += 44.0;
            }
            Placed { cover, round: 0.0, record: 0.0, title, title_zoom: 1.0, below: y, left: pad, width: wide }
        }
    }
}

impl Drawing<'_> {
    /// What `layout` shows off its ground: its lines (a player's, the lines round the one sung now as `lines` says),
    /// the spectrum, the controls.
    fn content(&mut self, layout: Layout, lines: bool) {
        match layout {
            Layout::Strip => self.strip(),
            Layout::Player => self.player(lines),
            Layout::Card | Layout::Tall => self.card(layout),
            Layout::Stage => self.stage(),
            Layout::Column => self.column(),
        }
    }

    /// Where its parts are, `layout` laying them out: on their way there while it turns.
    fn placed(&self, layout: Layout) -> Placed {
        let to = placed(layout, self.size, self.shows);
        match self.turn {
            Some((from, t)) => tween(from, to, t),
            None => to,
        }
    }

    /// Its ground (the cover blurred under a wash of its colour, inside its rounded corners, with a shadow; none,
    /// clear), the cover and what plays: kept, drawn again only as one of them changes.
    fn ground(&mut self, layout: Layout) {
        let (w, h) = self.size;
        let frame = self.frame;
        if let Some(cover) = &self.moment.cover {
            self.music.backdrop(frame, cover, self.now);
        } else if self.moment.no_cover {
            self.music.no_backdrop(self.now);
        }
        let shown = self.music.fading.as_ref().map_or(1.0, |(_, since)| (self.now.saturating_duration_since(*since).as_secs_f32() / 1.2).min(1.0));
        let before = self.music.fading.as_ref().map(|(before, _)| before.clone());
        if shown >= 1.0 {
            self.music.fading = None;
        }
        let backdrop = self.music.backdrop.as_ref().map(|(key, bitmap)| (*key, bitmap.clone()));
        let (accent, tint) = (self.music.accent, self.music.tint);
        let (shows, ink, edged, own) = (*self.shows, self.ink, self.edged, self.ground == Ground::Own);
        let at = self.placed(layout);
        let cover = self.moment.cover.clone();
        let (title, artist) = (self.moment.title.clone(), self.moment.artist.clone());
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            (w.to_bits(), h.to_bits(), self.scale.to_bits(), layout, shows, ink, own, self.turn.map(|(_, t)| t.to_bits()), backdrop.as_ref().map(|(key, _)| *key), (shown * 64.0).round() as u32).hash(&mut hasher);
            accent.iter().chain(&tint).map(|v| (v * 255.0).round() as u8).collect::<Vec<_>>().hash(&mut hasher);
            (&title, &artist, cover.as_ref().map(|cover| cover.key)).hash(&mut hasher);
            hasher.finish()
        };
        let radius = RADIUS.min(h / 2.0);
        let ground = Backdrop { size: (w, h), radius, backdrop: backdrop.map(|(_, bitmap)| bitmap), before, shown, accent, tint, own };
        // (Round it, room for its shadow.)
        let room = 24.0;
        let _ = self.music.ground.draw(frame, key, windows_numerics::Matrix3x2::identity(), (-room, -room, w + 2.0 * room, h + 2.0 * room), self.scale, None, |frame| {
            ground.paint(frame, |frame| {
                // (With the record round it, drawn over the record: see `disc`.)
                if let Some((x, y, side)) = at.cover.filter(|_| at.record <= 0.0) {
                    let corner = cover_corner(side);
                    // (Inside another widget, flat as what it shows is.)
                    if shows.inset.is_none() {
                        frame.shadow(Color::hex(0x000000, 0.35), x, y, side, side, corner, 10.0, 3.0);
                    }
                    match &cover {
                        Some(cover) => {
                            let picture = super::canvas::Picture { key: cover.key, side: crate::cover::SIDE, pixels: cover.pixels.clone() };
                            frame.picture(&picture, x, y, side, side, corner, 1.0);
                        }
                        // None (its player gives none, or it is being read): a plate with a note on it.
                        None => {
                            frame.fill_rounded(Color::hex(ink, if edged { 0.18 } else { 0.08 }), x, y, side, side, corner);
                            frame.icon(Icon::Music, Point { x: x + side / 2.0, y: y + side / 2.0 }, side * 0.42, Color::hex(ink, 0.5));
                        }
                    }
                }
                if let Some((x, y, wide, centred)) = at.title {
                    let zoom = at.title_zoom;
                    let (title_font, about) = (Font { size: self.fonts.title.size * zoom, ..self.fonts.title }, Font { size: self.fonts.about.size * zoom, ..self.fonts.about });
                    for (text, font, alpha, down) in [(&title, title_font, 0.95, 0.0), (&artist, about, 0.62, 21.0 * zoom)] {
                        let text = fitted(frame, text, font, wide);
                        let from = if centred { x + (wide - frame.measure(&text, font)) / 2.0 } else { x };
                        words(frame, &text, font, Color::hex(ink, alpha), from, y + down, wide + 2.0, Align::Start, edged);
                    }
                }
            })
        });
    }

    /// The rows along the bottom, from `x`, `wide`, up from `edge`: the controls kept showing in a row of their own
    /// with the spectrum's row above them; else the spectrum, the controls coming over it as the pointer does. Where
    /// they leave room down to.
    fn rows(&mut self, x: f32, edge: f32, wide: f32, beside: Option<(f32, f32)>) -> f32 {
        let row = edge - CONTROL;
        let (accent, tint) = (self.accent, self.tint);
        if self.shows.controls {
            self.bottom_row(x, x + wide, row, 1.0, beside);
            if !self.shows.spectrum {
                return row;
            }
            let top = row - ROW_GAP - SPECTRUM_ROW;
            spectrum(self.frame, self.moment, (accent, tint), (x, row - ROW_GAP, wide, SPECTRUM_ROW), 0.85);
            return top;
        }
        let shown_in = self.music.controls_shown();
        if shown_in < 1.0 && self.shows.spectrum {
            spectrum(self.frame, self.moment, (accent, tint), (x, edge - 2.0, wide, CONTROL - 4.0), 1.0 - shown_in);
        }
        if shown_in > 0.0 {
            self.bottom_row(x, x + wide, row, shown_in, beside);
        }
        row
    }

    /// The row along the bottom, from `x` to `right` (where the drawings at its ends begin and end) on `row`, `fade`
    /// of it shown: the keys first, spread across under the cover where the cover stands over them (`beside`: the
    /// cover's right edge, and where the words beside it begin); the track from where those words begin (else after
    /// the keys), the time left at its end; the sound's key last, its drawing ending at `right`.
    fn bottom_row(&mut self, x: f32, right: f32, row: f32, fade: f32, beside: Option<(f32, f32)>) {
        let gap = beside.map_or(KEY_GAP, |(cover_right, _)| ((cover_right - x + 2.0 * KEY_LEAD - 3.0 * CONTROL) / 2.0).max(KEY_GAP));
        let end = self.keys(x - KEY_LEAD, gap, row, fade);
        let from = beside.map_or(end + 10.0, |(_, words)| words.max(end + 6.0));
        self.sound_key(from, right + SOUND_TRAIL - CONTROL, row, fade);
    }

    /// The sound's key, its box at `home` on `row` (at the row's end), the track before it from `from`: its icon
    /// (pressed, the sound muted or not); with the pointer on it (or on what opens), the key moving left and a slider of
    /// how loud the sound is shown to its right as it goes, the track giving way (nothing laid over what else it
    /// shows). The slider lies where it will lie, uncovered as the key moves off it (its knob never slides); no level
    /// said (pressed, dragged or turned by the wheel, it is heard).
    fn sound_key(&mut self, from: f32, home: f32, row: f32, fade: f32) {
        let frame = self.frame;
        // (On whole pixels, as the keys are: a drawing between them is spread over two, and blurred.)
        let scale = self.scale;
        let whole = move |v: f32| (v * scale).round() / scale;
        let home = whole(home);
        if let Some(read) = crate::playing::sound() {
            self.music.sound_seen = Some(read);
        }
        let sound = self.music.sound_seen;
        let (level, muted) = sound.unwrap_or((0, false));
        let inside = |(px, py): (f32, f32), x: f32, y: f32, w: f32, h: f32| px >= x && px < x + w && py >= y && py < y + h;
        let room = SOUND_WIDE + 8.0;
        let x = home - room * self.music.sound_in;
        let over = self.pointer.is_some_and(|p| inside(p, x, row - 6.0, home + CONTROL - x, CONTROL + 12.0));
        let on_home = self.pointer.is_some_and(|p| inside(p, home, row, CONTROL, CONTROL));
        let wants = on_home || crate::playing::volume_held() || (self.music.sound_open && over);
        self.music.sound_left = if wants { None } else { self.music.sound_left.or(Some(self.now)) };
        let lingering = self.music.sound_open && self.music.sound_left.is_some_and(|left| self.now.saturating_duration_since(left) < SOUND_LINGER);
        self.music.sound_open = sound.is_some() && fade > 0.0 && (wants || lingering);
        let to = if self.music.sound_open { 1.0 } else { 0.0 };
        let fade_step = self.music.step.min(1.0 / 30.0) / SOUND_OPEN;
        // (Each way no further than where it is going: once open, it stays open, not a step short of it every other
        // frame, the key and the track's end shaking by a pixel.)
        self.music.sound_in = if to > self.music.sound_in { (self.music.sound_in + fade_step).min(to) } else { (self.music.sound_in - fade_step).max(to) };
        let opened = self.music.sound_in * self.music.sound_in * (3.0 - 2.0 * self.music.sound_in);
        let x = whole(home - room * opened);
        // The track, ending before the key wherever the key is.
        self.progress(from, x - 4.0, row, fade, 1.0);
        let on = self.pointer.is_some_and(|p| inside(p, x, row, CONTROL, CONTROL));
        if on {
            frame.fill_rounded(Color::hex(self.ink, 0.14 * fade), x, row, CONTROL, CONTROL, CONTROL / 2.0);
        }
        let ink = Color::hex(self.ink, (if on || self.music.sound_open { 1.0 } else { 0.85 }) * fade);
        let centre = Point { x: x + CONTROL / 2.0, y: row + CONTROL / 2.0 };
        let icon = if muted || level == 0 { Icon::Muted } else { Icon::Volume };
        if self.edged {
            frame.icon(icon, Point { x: centre.x + 0.6, y: centre.y + 1.0 }, KEY_ICON, Color::hex(0x000000, 0.45 * ink.a));
        }
        frame.icon(icon, centre, KEY_ICON, ink);
        if self.music.controls_in >= 0.5 {
            self.hits.push((x, row, CONTROL, CONTROL, Hit::Media(Key::Mute, crate::playing::Follow::Any)));
        }
        // The slider where it lies once open (from past the key's open place to the row's end), shown right of the
        // key as it goes.
        let slider = whole(home - room + CONTROL + 4.0);
        let over_slider = self.pointer.is_some_and(|p| inside(p, slider - 4.0, row, SOUND_WIDE + 8.0, CONTROL));
        let lit_to = if opened >= 1.0 && (over_slider || crate::playing::volume_held()) { 1.0 } else { 0.0 };
        self.music.sound_lit = eased_toward(self.music.sound_lit, lit_to, self.music.step);
        // (Lingering, it closes once the time is up: a frame wanted then.)
        self.music.easing |= lingering || self.music.sound_in != to || self.music.sound_lit != lit_to;
        if opened <= 0.0 {
            return;
        }
        let fraction = if muted { 0.0 } else { level as f32 / 100.0 };
        let lit = self.music.sound_lit;
        let thick = 3.0 + 2.0 * lit;
        let alpha = fade * opened;
        frame.clip(x + CONTROL, row, home + CONTROL - (x + CONTROL), CONTROL);
        frame.fill_rounded(Color::hex(self.ink, 0.25 * alpha), slider, centre.y - thick / 2.0, SOUND_WIDE, thick, thick / 2.0);
        frame.fill_rounded(colour(self.accent, 0.95 * alpha), slider, centre.y - thick / 2.0, SOUND_WIDE * fraction, thick, thick / 2.0);
        frame.fill_circle(Color::hex(self.ink, alpha), Point { x: slider + SOUND_WIDE * fraction, y: centre.y }, 4.5 + 1.5 * lit);
        frame.unclip();
        if self.music.controls_in >= 0.5 && opened >= 1.0 {
            // (Its box as wide as its track, as every slider's: where it is pressed along the box is where along the
            // track it is set to, and a wider box set it off by as much.)
            self.hits.push((slider, row, SOUND_WIDE, CONTROL, Hit::Volume));
        }
    }

    /// What goes back, pauses or plays, and skips, their boxes in a row from `x`, `gap` apart, at `row`, `fade` of
    /// them shown (each lit under the pointer, pressed only once they are half come); where the row ends.
    fn keys(&mut self, mut x: f32, gap: f32, row: f32, fade: f32) -> f32 {
        for key in [Key::Previous, Key::Toggle, Key::Next] {
            self.key(key, x, row, fade);
            x += CONTROL + gap;
        }
        x - gap
    }

    /// The key `key`, its box at (`x`, `row`), `fade` of it shown (lit under the pointer, pressed only once it is half
    /// come).
    fn key(&mut self, key: Key, x: f32, row: f32, fade: f32) {
        let frame = self.frame;
        let icon = match key {
            Key::Previous => Icon::SkipBack,
            Key::Next => Icon::SkipForward,
            _ => if self.moment.playing { Icon::Pause } else { Icon::Play },
        };
        let on = self.pointer.is_some_and(|(px, py)| px >= x && px < x + CONTROL && py >= row && py < row + CONTROL);
        if on {
            frame.fill_rounded(Color::hex(self.ink, 0.14 * fade), x, row, CONTROL, CONTROL, CONTROL / 2.0);
        }
        let side = if key == Key::Toggle { TOGGLE_ICON } else { KEY_ICON };
        let colour = Color::hex(self.ink, (if on { 1.0 } else { 0.85 }) * fade);
        let centre = Point { x: x + CONTROL / 2.0, y: row + CONTROL / 2.0 };
        // (With no ground, a dark edge under it, as its words have.)
        if self.edged {
            frame.icon(icon, Point { x: centre.x + 0.6, y: centre.y + 1.0 }, side, Color::hex(0x000000, 0.45 * colour.a));
        }
        frame.icon(icon, centre, side, colour);
        if self.music.controls_in >= 0.5 {
            self.hits.push((x, row, CONTROL, CONTROL, Hit::Media(key, crate::playing::Follow::Music)));
        }
    }

    /// What plays's track, from `from` to `to` on `row`, with how long is left at its end, `fade` of it shown (thicker
    /// under the pointer, or dragged, its knob on it; no times said over it: dragged, the time left at its end is that
    /// from where it is dragged to). Pressed and dragged along it (see `Hit::Seek`), what plays is shown that far in,
    /// and played from there once let go. (Only the full screen keeps its track thin and bare, and says the time at
    /// the pointer.)
    fn progress(&mut self, from: f32, to: f32, row: f32, fade: f32, time_shown: f32) {
        let Some((length, seekable)) = self.moment.length.filter(|(length, _)| *length > 0.0) else { return };
        let frame = self.frame;
        let position = self.moment.dragged_to.unwrap_or(self.moment.position);
        let left_over = format!("-{}", super::text::clock((length - position).max(0.0).round() as u32));
        let time_wide = frame.measure(&left_over, self.fonts.time) + 2.0;
        // (The time left going, the track takes its room.)
        let track = (to - (time_wide + 10.0) * time_shown - from).max(0.0);
        if track < 24.0 {
            return;
        }
        let lit = self.pointer.is_some_and(|(px, py)| px >= from && px < from + track && py >= row && py < row + CONTROL);
        let dragged = self.moment.dragged_to.is_some();
        let middle = row + CONTROL / 2.0;
        let fraction = (position / length).clamp(0.0, 1.0) as f32;
        // (Eased, not at once: the track moving under a pointer that stands still, as the sound's slider opens,
        // made it jump between thin and thick.)
        let lit_to = if seekable && (lit || dragged) { 1.0 } else { 0.0 };
        self.music.track_lit = eased_toward(self.music.track_lit, lit_to, self.music.step);
        self.music.easing |= self.music.track_lit != lit_to;
        let shine = self.music.track_lit;
        let thick = 3.0 + 2.0 * shine;
        frame.fill_rounded(Color::hex(self.ink, 0.2 * fade), from, middle - thick / 2.0, track, thick, thick / 2.0);
        frame.fill_rounded(colour(self.accent, 0.95 * fade), from, middle - thick / 2.0, track * fraction, thick, thick / 2.0);
        if shine > 0.0 {
            frame.fill_circle(Color::hex(self.ink, fade * shine), Point { x: from + track * fraction, y: middle }, 4.0 + 2.0 * shine);
        }
        if time_shown > 0.0 {
            let at = ((to - time_wide) * self.scale).round() / self.scale;
            words(frame, &left_over, self.fonts.time, Color::hex(self.ink, 0.6 * fade * time_shown), at, middle - self.fonts.time.size * 0.7, time_wide, Align::End, self.edged);
        }
        if seekable && self.music.controls_in >= 0.5 {
            self.hits.push((from, row, track, CONTROL, Hit::Seek(crate::playing::Follow::Music)));
        }
    }

    /// Whether its player lets what plays be moved along (see `Timeline::seekable`).
    /// Draws within `x, y, w, h` from now on (till `unclip`): what is pressed there too (a line partly out of it is
    /// pressed only where it shows: pressed where it was cut off, it played from a line not seen).
    fn clip(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.frame.clip(x, y, w, h);
        self.clips.push(((x, y, w, h), self.hits.len()));
    }

    /// Draws wherever again (see `clip`): what was pressed within it, cut to it.
    fn unclip(&mut self) {
        self.frame.unclip();
        let ((cx, cy, cw, ch), from) = self.clips.pop().expect("clipped");
        let within: Vec<HitBox> = self.hits.drain(from..).filter_map(|(x, y, w, h, hit)| {
            let (left, top) = (x.max(cx), y.max(cy));
            let (right, bottom) = ((x + w).min(cx + cw), (y + h).min(cy + ch));
            (right > left && bottom > top).then_some((left, top, right - left, bottom - top, hit))
        }).collect();
        self.hits.extend(within);
    }

    fn seekable(&self) -> bool {
        self.moment.length.is_some_and(|(_, seekable)| seekable)
    }

    /// The line at `index`, turned to by the wheel: whole, its time beside it to be played from (pressed there, or
    /// on it), in one row from `(x, y)` at most `width` wide, in `font`.
    fn turned_to(&mut self, lines: &[Line], index: usize, x: f32, y: f32, width: f32, font: Font) {
        let frame = self.frame;
        let line = &lines[index];
        // (A player that lets no one move it along, QQ Music: the line alone, nothing that offers to play from it.)
        if !self.seekable() {
            let text = if line.text.trim().is_empty() { "…".to_string() } else { line.text.clone() };
            words(frame, &fitted(frame, &text, font, width), font, Color::hex(self.ink, 0.92), x, y, width + 2.0, Align::Start, self.edged);
            return;
        }
        let chip = super::text::clock(line.at.max(0.0) as u32);
        let chip_wide = frame.measure(&chip, self.fonts.time) + 26.0;
        let text = if line.text.trim().is_empty() { "…".to_string() } else { line.text.clone() };
        let room = (width - chip_wide - 10.0).max(0.0);
        let shown = fitted(frame, &text, font, room);
        let wide = frame.measure(&shown, font);
        words(frame, &shown, font, Color::hex(self.ink, 0.92), x, y, room + 2.0, Align::Start, self.edged);
        let (cx, cy) = (x + wide + 10.0, y + font.size * 0.66 - 9.0);
        let on = self.pointer.is_some_and(|(px, py)| px >= cx && px < cx + chip_wide && py >= cy && py < cy + 18.0);
        frame.fill_rounded(Color::hex(self.ink, if on { 0.24 } else { 0.1 }), cx, cy, chip_wide, 18.0, 9.0);
        frame.stroke_rounded(Fill::Solid(Color::hex(self.ink, 0.5)), cx, cy, chip_wide, 18.0, 9.0, 1.0);
        frame.icon(Icon::Play, Point { x: cx + 10.0, y: cy + 9.0 }, 10.0, Color::hex(self.ink, 0.95));
        frame.text(&chip, self.fonts.time, Color::hex(self.ink, 0.95), cx + 17.0, cy + 2.0, chip_wide, Align::Start);
        self.hits.push((x, y, wide, font.size * 1.35, Hit::Line(index as u16)));
        self.hits.push((cx, cy, chip_wide, 18.0, Hit::Line(index as u16)));
    }

    /// As `turned_to`, on the rail: the line in rows as it is when sung (`full`, in rows `width / zoom` wide, drawn
    /// `zoom` times as large, as the rail draws its lines; `settled` as it comes to rest), not cut short (cut short, played from, it came whole and
    /// moved); what plays it from there after its last row, or under it where that row has no room (see `chip_at`).
    #[allow(clippy::too_many_arguments)]
    fn turned_wrapped(&mut self, lines: &[Line], index: usize, x: f32, y: f32, width: f32, full: Font, zoom: f32, settled: f32) {
        let frame = self.frame;
        let line = &lines[index];
        let text = if line.text.trim().is_empty() { "…".to_string() } else { line.text.clone() };
        let room = width / settled;
        let colour = Color::hex(self.ink, 0.92);
        let edged = self.edged;
        frame.scaled(x, y, zoom, |frame| {
            if edged {
                edge(frame, colour.a, |frame| frame.text_wrapped(&text, full, Color::hex(0xFFFFFF, 1.0), x, y, room));
            }
            frame.text_wrapped(&text, full, colour, x, y, room);
        });
        let tall = frame.gfx.wrapped_height(&text, full, room) * zoom;
        self.hits.push((x, y, width, tall, Hit::Line(index as u16)));
        if !self.seekable() {
            return;
        }
        let chip = super::text::clock(line.at.max(0.0) as u32);
        let chip_wide = frame.measure(&chip, self.fonts.time) + 26.0;
        let (dx, dy, _) = chip_at(frame, &text, full, width, zoom, settled, chip_wide);
        let (cx, cy) = (x + dx, y + dy);
        let on = self.pointer.is_some_and(|(px, py)| px >= cx && px < cx + chip_wide && py >= cy && py < cy + 18.0);
        frame.fill_rounded(Color::hex(self.ink, if on { 0.24 } else { 0.1 }), cx, cy, chip_wide, 18.0, 9.0);
        frame.stroke_rounded(Fill::Solid(Color::hex(self.ink, 0.5)), cx, cy, chip_wide, 18.0, 9.0, 1.0);
        frame.icon(Icon::Play, Point { x: cx + 10.0, y: cy + 9.0 }, 10.0, Color::hex(self.ink, 0.95));
        frame.text(&chip, self.fonts.time, Color::hex(self.ink, 0.95), cx + 17.0, cy + 2.0, chip_wide, Align::Start);
        self.hits.push((cx, cy, chip_wide, 18.0, Hit::Line(index as u16)));
    }

    /// The strip: the cover small at its start, the line sung now, and at its end the spectrum or the controls (kept
    /// showing: the controls at the end, the spectrum before them). With no ground and no cover, the line large in its
    /// middle and what is under it below, the controls at its start as the pointer comes.
    fn strip(&mut self) {
        let (w, h) = self.size;
        let pad = pad_of(Layout::Strip, self.shows);
        let at = self.placed(Layout::Strip);
        let (accent, tint) = (self.accent, self.tint);
        let row = (h - CONTROL) / 2.0;
        let centred = self.edged && !self.shows.cover;
        let shown_in = self.music.controls_shown();
        let ends = strip_ends(w, at.left, pad, self.shows, centred);
        if let Some((x, wide)) = ends.spectrum {
            let alpha = if self.shows.controls { 0.85 } else { 1.0 - shown_in };
            if alpha > 0.0 {
                spectrum(self.frame, self.moment, (accent, tint), (x, h / 2.0 + 10.0, wide, 20.0), alpha);
            }
        }
        if let Some(x) = ends.keys {
            let fade = if self.shows.controls { 1.0 } else { shown_in };
            if fade > 0.0 {
                self.keys(x, KEY_GAP, row, fade);
            }
        }
        if centred {
            // Clear of the controls at either end, the words kept in the middle.
            let side = if self.shows.controls { ends.side } else { ends.side * shown_in };
            self.lines_centred(pad + side, w - pad - side, h);
            return;
        }
        let left = at.left;
        let width = (ends.end - left).max(0.0);
        let frame = self.frame;
        self.clip(left - 4.0, 0.0, width + 8.0, h);
        let (lines, now) = (self.moment.lines.clone(), self.moment.now);
        match (&lines, now) {
            (Some(lines), _) if self.music.turned != 0 => {
                let index = turned_to(now, self.music.turned, lines.len()).unwrap_or(0);
                self.turned_to(lines, index, left, h / 2.0 - self.fonts.strip.size * 0.7, width, self.fonts.strip);
            }
            (Some(lines), Some(now)) => {
                let line = &lines[now];
                let next = lines.get(now + 1).map(|line| line.at);
                let (text, parts) = parts(line, next);
                let eased = self.came();
                // (Rising in, drawn where it falls: see `Frame::set_moving`.)
                let _moving = frame.moving_while(eased < 1.0);
                let (dim, bright) = self.line_colours(eased);
                if text.trim().is_empty() {
                    let y = h / 2.0 - self.fonts.strip.size * 0.7 + (1.0 - eased) * 8.0;
                    interlude(frame, left, y, self.fonts.strip.size, between(line, next, self.moment.position), dim, bright);
                } else {
                    // In one row, as large as it fits (down to the least); too long for that, in two rows where the strip
                    // is tall enough, as large as they fit; else one row cut short with an ellipsis. Never cut through.
                    let room = h - 2.0 * pad_of(Layout::Strip, self.shows);
                    // The size at which it fills its row (its size goes with the strip's width): whether it is shown in
                    // one row is told by that size, and that row is the strip's size's laid out and drawn smaller, so the
                    // sizes' rounding never decides it (measured again at the size, the line went now into one row,
                    // now into two, as the strip was sized).
                    let fills = self.fonts.strip.size * width / frame.measure(&text, self.fonts.strip).max(1.0);
                    let font = Font { size: fills.clamp(STRIP_LEAST, self.fonts.strip.size), ..self.fonts.strip };
                    let one_row = fills >= STRIP_LEAST;
                    let two_rows = (!one_row)
                        .then(|| (0..).map(|step| self.fonts.strip.size - step as f32).take_while(|size| *size >= STRIP_LEAST).map(|size| Font { size, ..self.fonts.strip }).find(|font| {
                            let tall = frame.gfx.wrapped_height(&text, *font, width);
                            tall <= room && tall <= 2.0 * frame.gfx.wrapped_height("M", *font, width) + 0.5
                        }))
                        .flatten();
                    // (Laid out at `font`, drawn `zoom` times as large.)
                    let (font, text, wrap, zoom) = match two_rows {
                        Some(font) => (font, text.clone(), width, 1.0),
                        None if one_row => (self.fonts.strip, text.clone(), 100_000.0, font.size / self.fonts.strip.size),
                        None => (font, fitted(frame, &text, font, width), 100_000.0, 1.0),
                    };
                    let tall = frame.gfx.wrapped_height(&text, font, wrap) * zoom;
                    let y = (h - tall) / 2.0 + (1.0 - eased) * 8.0;
                    let layout = frame.gfx.wrapped(&text, font, wrap);
                    let sung = sung_to(&parts, self.moment.position);
                    frame.scaled(left, y, zoom, |_| self.filled(&layout, &text, font, left, y, wrap, sung, dim, bright, 12.0 / zoom));
                }
            }
            (Some(lines), None) => {
                let first = lines.first().map_or(0.0, |line| line.at);
                let done = if first > 0.0 { (self.moment.position / first).clamp(0.0, 1.0) as f32 } else { 0.0 };
                let (dim, bright) = self.line_colours(1.0);
                interlude(frame, left, h / 2.0 - self.fonts.strip.size * 0.7, self.fonts.strip.size, done, dim, bright);
            }
            (None, _) => {
                let said = [self.moment.title.as_str(), self.moment.artist.as_str()].into_iter().filter(|one| !one.is_empty()).collect::<Vec<_>>().join(" · ");
                let said = if said.is_empty() { self.moment.said.clone().unwrap_or_default() } else { said };
                words(frame, &fitted(frame, &said, self.fonts.about, width), self.fonts.about, Color::hex(self.ink, 0.8), left, h / 2.0 - self.fonts.about.size * 0.7, width + 2.0, Align::Start, self.edged);
            }
        }
        self.unclip();
    }

    /// The strip down: the cover at its top (see `placed`), the line sung now under it written down (and what is under
    /// it beside it, where two go across), the key to play or pause at its foot where there is room (kept showing, or
    /// as the pointer comes).
    fn column(&mut self) {
        let (w, h) = self.size;
        let pad = pad_of(Layout::Column, self.shows);
        let at = self.placed(Layout::Column);
        let fade = if self.shows.controls { 1.0 } else { self.music.controls_shown() };
        // (The key under the words, where they keep a little room; else none.)
        let keyed = h - pad - at.below >= CONTROL + 8.0 + 2.0 * STRIP_LEAST;
        let foot = if keyed { h - pad - CONTROL } else { h - pad };
        if keyed && fade > 0.0 {
            self.key(Key::Toggle, (w - CONTROL) / 2.0, foot, fade);
        }
        let room = (foot - 8.0 - at.below).max(0.0);
        if room < STRIP_LEAST || at.width < STRIP_LEAST {
            return;
        }
        let frame = self.frame;
        let (left, top, width) = (at.left, at.below, at.width);
        // Written down where the room for the words is taller than wide; else across, what is under it below.
        let down = room > width * 1.2;
        let lines = self.moment.lines.clone();
        match (&lines, self.moment.now) {
            (Some(lines), Some(now)) => {
                let line = &lines[now];
                let next = lines.get(now + 1).map(|line| line.at);
                let (text, parts) = parts(line, next);
                let eased = self.came();
                let _moving = frame.moving_while(eased < 1.0);
                let (dim, bright) = self.line_colours(eased);
                let under = self.under(lines, now);
                if text.trim().is_empty() {
                    let size = self.fonts.strip.size.min(width / INTERLUDE_WIDE);
                    interlude(frame, left + (width - size * INTERLUDE_WIDE) / 2.0, top + (1.0 - eased) * 8.0, size, between(line, next, self.moment.position), dim, bright);
                    return;
                }
                // Down, what is under it beside it where both go in half the width unbroken; else the line alone.
                let gap = 6.0;
                let half = (width - gap) / 2.0;
                let least = Font { size: STRIP_LEAST, ..self.fonts.strip };
                let two = down && !under.is_empty() && unbroken(frame, &text, least, half, down) && unbroken(frame, &under, least, half, down);
                let column = if two { half } else { width };
                let font = sized_down(frame, &text, self.fonts.strip, column, room, down);
                let wrap = down_wrap(&text, font, column, down);
                // (Right to left, as words written down are read: the line sung now first.)
                let x = if two { left + width - column + (column - wrap) / 2.0 } else { left + (width - wrap) / 2.0 };
                let y = top + (1.0 - eased) * 8.0;
                let row = row_of(frame, &text, font, wrap);
                let tall = frame.gfx.wrapped_height(&text, font, wrap);
                // (Cut where a row ends, as many whole rows as there is room for.)
                self.clip(left - 2.0, top, width + 4.0, tall.min((room / row).floor() * row) + 2.0);
                let layout = frame.gfx.wrapped(&text, font, wrap);
                let sung = sung_to(&parts, self.moment.position);
                self.filled(&layout, &text, font, x, y, wrap, sung, dim, bright, 6.0);
                self.unclip();
                let small = Font { size: (font.size * 0.82).max(STRIP_LEAST.min(font.size)), weight: 500.0, ..font };
                let colour = Color::hex(self.ink, 0.6 * eased);
                if two {
                    let wrap = down_wrap(&under, small, column, down);
                    let row = row_of(frame, &under, small, wrap);
                    self.clip(left - 2.0, top, column + 4.0, (room / row).floor() * row + 2.0);
                    words_down(frame, &under, small, colour, left + (column - wrap) / 2.0, y, wrap, self.edged);
                    self.unclip();
                } else if !down && !under.is_empty() {
                    // Across: under it, where a whole row of it has room.
                    let below = y + tall + 4.0;
                    let row = row_of(frame, &under, small, width);
                    let rows = ((top + room - below) / row).floor();
                    if rows >= 1.0 {
                        self.clip(left - 2.0, below, width + 4.0, rows * row + 2.0);
                        words_down(frame, &under, small, colour, left, below, width, self.edged);
                        self.unclip();
                    }
                }
            }
            (Some(lines), None) => {
                let first = lines.first().map_or(0.0, |line| line.at);
                let done = if first > 0.0 { (self.moment.position / first).clamp(0.0, 1.0) as f32 } else { 0.0 };
                let (dim, bright) = self.line_colours(1.0);
                let size = self.fonts.strip.size.min(width / INTERLUDE_WIDE);
                interlude(frame, left + (width - size * INTERLUDE_WIDE) / 2.0, top, size, done, dim, bright);
            }
            (None, _) => {
                // What plays, written as the lines are.
                let said = if self.moment.title.is_empty() { self.moment.said.clone().unwrap_or_default() } else { self.moment.title.clone() };
                let font = sized_down(frame, &said, self.fonts.about, width, room, down);
                let wrap = down_wrap(&said, font, width, down);
                let row = row_of(frame, &said, font, wrap);
                self.clip(left - 2.0, top, width + 4.0, (room / row).floor() * row + 2.0);
                words_down(frame, &said, font, Color::hex(self.ink, 0.8), left + (width - wrap) / 2.0, top, wrap, self.edged);
                self.unclip();
            }
        }
    }

    /// The line sung now large in the middle between `from` and `to` (floating lyrics), what is under it below where
    /// there is room: glowing in the cover's colour, a dark haze round it to be read on anything.
    fn lines_centred(&mut self, from: f32, to: f32, h: f32) {
        let frame = self.frame;
        let room = (to - from).max(0.0);
        let under_room = |size: f32| h >= size * 1.4 + size * FLOAT_UNDER * 1.4 + 16.0;
        let size = (h * 0.34).clamp(STRIP_LEAST, self.fonts.float.size);
        let font = Font { size, ..self.fonts.float };
        let under_font = Font { size: (size * FLOAT_UNDER).max(11.0), weight: 600.0, ..self.fonts.float };
        let with_under = under_room(size);
        let top = if with_under { (h - size * 1.4 - under_font.size * 1.4) / 2.0 } else { (h - size * 1.4) / 2.0 };
        let Some(lines) = self.moment.lines.clone() else {
            if let Some(said) = &self.moment.said {
                let wide = frame.measure(said, under_font).min(room);
                outlined(frame, said, under_font, Color::hex(0xFFFFFF, 0.85), from + (room - wide) / 2.0, (h - under_font.size * 1.4) / 2.0, wide + 2.0);
            }
            return;
        };
        if self.music.turned != 0 {
            let index = turned_to(self.moment.now, self.music.turned, lines.len()).unwrap_or(0);
            let text = lines[index].text.clone();
            let wide = (frame.measure(&text, font) + frame.measure("0:00", self.fonts.time) + 36.0).min(room);
            self.turned_to(&lines, index, from + (room - wide) / 2.0, top, wide, font);
            return;
        }
        let Some(now) = self.moment.now else { return };
        let line = &lines[now];
        let next = lines.get(now + 1).map(|line| line.at);
        let (text, parts) = parts(line, next);
        let eased = self.came();
        let _moving = self.frame.moving_while(eased < 1.0);
        let y = top + (1.0 - eased) * 14.0;
        let (accent, tint) = (self.accent, self.tint);
        let (dim, bright) = (Color::hex(0xFFFFFF, 0.6 * eased), colour(tint, eased));
        if text.trim().is_empty() {
            let x = from + (room - font.size * INTERLUDE_WIDE) / 2.0;
            let done = between(line, next, self.moment.position);
            frame.glow(Color::hex(0x000000, 0.55 * eased), 3.0, |frame| interlude(frame, x, y + 1.0, font.size, 0.0, Color::hex(0xFFFFFF, 1.0), Color::hex(0xFFFFFF, 1.0)));
            frame.glow(colour(accent, 0.75 * eased), 10.0, |frame| interlude(frame, x, y, font.size, done, Color::hex(0, 0.0), Color::hex(0xFFFFFF, 1.0)));
            interlude(frame, x, y, font.size, done, dim, bright);
        } else {
            let wide = frame.measure(&text, font).min(room);
            let x = from + (room - wide) / 2.0;
            let layout = frame.gfx.wrapped(&text, font, wide + 2.0);
            let sung = sung_to(&parts, self.moment.position);
            let width = wide + 2.0;
            // A dark haze for it to be read on anything; a glow in the cover's colour round what is sung.
            frame.glow(Color::hex(0x000000, 0.55 * eased), 3.0, |frame| frame.text_wrapped(&text, font, Color::hex(0xFFFFFF, 1.0), x, y + 1.0, width));
            frame.glow(colour(accent, 0.75 * eased), 10.0, |frame| sung_part(frame, &layout, &text, font, x, y, width, sung, Color::hex(0xFFFFFF, 1.0), 14.0));
            filled(frame, &layout, &text, font, x, y, width, sung, dim, bright, 14.0);
        }
        if with_under {
            let under = self.under(&lines, now);
            if !under.is_empty() {
                let wide = frame.measure(&under, under_font).min(room);
                outlined(frame, &fitted(frame, &under, under_font, room), under_font, Color::hex(0xFFFFFF, 0.78 * eased), from + (room - wide) / 2.0, y + size * 1.45, wide + 2.0);
            }
        }
    }

    /// The player: the cover at the start, what plays beside it, the line sung now (rising in as it comes, filled as
    /// it is sung, wrapped where it is too long) and what is under it, the rows along the bottom.
    fn player(&mut self, lines: bool) {
        let (_, h) = self.size;
        let pad = pad_of(Layout::Player, self.shows);
        let at = self.placed(Layout::Player);
        let (left, width) = (at.left, at.width);
        // (The keys spread across under the cover, the track from where the words beside it begin.)
        let free = self.rows(pad, h - pad, self.size.0 - 2.0 * pad, at.cover.map(|(x, _, side)| (x + side, at.left)));
        let line_top = at.below;
        let bottom = free - 8.0;
        let under_tall = self.fonts.under.size * 1.35;
        if bottom <= line_top {
            return;
        }
        // Room for more than the line sung now and the one under it: the lines round it, as the card's.
        if let (Some(lines), true) = (self.moment.lines.clone(), lines) {
            // (The line sung now level with the cover's middle, where there is a cover, as far as there is room for a
            // line above it and below it: the lines all the way from under the title to the rows along the bottom.)
            let middle = at.cover.map_or((line_top + bottom) / 2.0, |(_, y, side)| y + side / 2.0);
            let row = self.fonts.line.size * 1.6;
            let anchor = middle.clamp(line_top + row * 1.2, (bottom - row * 1.2).max(line_top + row * 1.2));
            // (At its line's size, as when it shows the line sung now alone: made taller, it shows more lines, not
            // smaller ones, though its cover grows and the column beside it narrows.)
            self.rail_of_lines(&lines, left, line_top, bottom, width, Some(anchor), self.fonts.line.size);
            return;
        }
        let frame = self.frame;
        self.clip(left - 4.0, line_top - 4.0, width + 8.0, bottom - line_top + 8.0);
        let (lines, now) = (self.moment.lines.clone(), self.moment.now);
        match (&lines, now) {
            (Some(lines), _) if self.music.turned != 0 => {
                let index = turned_to(now, self.music.turned, lines.len()).unwrap_or(0);
                let under = lines[index].translation.clone().filter(|_| self.shows.translation).unwrap_or_default();
                // As the line sung now is sized (below): in rows, at the largest size at which they fit with what plays
                // it from (after its last row, or under it) and its translation, else without that; cut short only
                // where none does. (Cut short as it was, played from, it came whole, and moved.)
                let text = if lines[index].text.trim().is_empty() { "…".to_string() } else { lines[index].text.clone() };
                let chip = if self.seekable() { Some(frame.measure(&super::text::clock(lines[index].at.max(0.0) as u32), self.fonts.time) + 26.0) } else { None };
                let tall = |font: Font| {
                    let rows = frame.gfx.wrapped_height(&text, font, width);
                    rows + match chip {
                        Some(chip) if chip_at(frame, &text, font, width, 1.0, 1.0, chip).2 => CHIP_ROW,
                        _ => 0.0,
                    }
                };
                let sizes = || (0..).map(|step| self.fonts.line.size - step as f32).take_while(|size| *size >= self.fonts.under.size).map(|size| Font { size, ..self.fonts.line });
                let room = bottom - line_top;
                let wants_under = if under.is_empty() { 0.0 } else { 2.0 + under_tall };
                let (line_tall, under_font) = match sizes().find(|font| tall(*font) + wants_under <= room).or_else(|| sizes().find(|font| tall(*font) <= room)) {
                    Some(font) => {
                        self.turned_wrapped(lines, index, left, line_top, width, font, 1.0, 1.0);
                        (tall(font), self.fonts.under)
                    }
                    None => {
                        self.turned_to(lines, index, left, line_top, width, self.fonts.line);
                        (self.fonts.line.size * 1.4, self.fonts.under)
                    }
                };
                let under_top = line_top + line_tall + 2.0;
                if !under.is_empty() && under_top + under_tall <= bottom {
                    words(frame, &fitted(frame, &under, under_font, width), under_font, Color::hex(self.ink, 0.55), left, under_top, width + 2.0, Align::Start, self.edged);
                }
            }
            (Some(lines), Some(now)) => {
                let line = &lines[now];
                let next = lines.get(now + 1).map(|line| line.at);
                let (text, parts) = parts(line, next);
                let eased = self.came();
                let _moving = frame.moving_while(eased < 1.0);
                let under = self.under(lines, now);
                // A line's size: too long for a row, wrapped, at the largest size (down to that of what is under it) at
                // which all its rows fit with what is under it; else all its rows alone; else one row at the least, cut
                // short with an ellipsis (never a row shown cut through). How tall it is, so, and whether it is one row
                // cut short.
                let room = bottom - line_top;
                let fit = |text: &str, with_under: bool| -> (Font, f32, bool) {
                    let sizes = || (0..).map(|step| self.fonts.line.size - step as f32).take_while(|size| *size >= self.fonts.under.size).map(|size| Font { size, ..self.fonts.line });
                    let height = |font: Font| frame.gfx.wrapped_height(text, font, width);
                    let wants_under = if with_under { 2.0 + under_tall } else { 0.0 };
                    match sizes().find(|font| height(*font) + wants_under <= room).or_else(|| sizes().find(|font| height(*font) <= room)) {
                        Some(font) => (font, height(font), false),
                        None => {
                            let font = Font { size: self.fonts.under.size, ..self.fonts.line };
                            (font, frame.gfx.wrapped_height("M", font, 100_000.0), true)
                        }
                    }
                };
                // As it is drawn: whole, or one row cut short.
                let shown = |text: &str, font: Font, cut: bool| if cut { fitted(frame, text, font, width) } else { text.to_string() };
                // Come from under the line before (the next line, shown under it): it grows into its place as that
                // goes up and out, and the line after it comes in under it.
                let before = now.checked_sub(1).filter(|&before| eased < 1.0 && !text.trim().is_empty() && !lines[before].text.trim().is_empty() && self.under(lines, before) == line.text);
                // How far down what is under it goes: under the line as it is now, wherever that is.
                let (line_y, tall) = match before {
                    Some(before) => {
                        let (old_font, old_tall, old_cut) = fit(&lines[before].text, true);
                        let (_, sung_colour) = self.line_colours(1.0);
                        frame.text_wrapped(&shown(&lines[before].text, old_font, old_cut), old_font, sung_colour.alpha(1.0 - eased), left, line_top - eased * old_tall * 0.7, width);
                        let (font, tall, cut) = fit(&text, !under.is_empty());
                        let text = shown(&text, font, cut);
                        let from = line_top + old_tall + 2.0;
                        let y = from + (line_top - from) * eased;
                        let size = self.fonts.under.size + (font.size - self.fonts.under.size) * eased;
                        let grown = Font { size, ..font };
                        let layout = frame.gfx.wrapped(&text, grown, width);
                        let sung = sung_to(&parts, self.moment.position);
                        let (dim, bright) = self.line_colours(eased);
                        self.filled(&layout, &text, grown, left, y, width, sung, dim, bright, 12.0);
                        (y, frame.gfx.wrapped_height(&text, grown, width).max(tall * eased))
                    }
                    // Else it rises in as it comes, what is under it with it, as the one before (with what was under it)
                    // goes up and out.
                    None => {
                        // (The two move on together as the lines of a page do, the one before gone as it nears the
                        // edge, never shown cut by it.)
                        let mut step = 18.0;
                        if let Some(before) = now.checked_sub(1).filter(|_| eased < 1.0) {
                            let was = &lines[before];
                            if !was.text.trim().is_empty() {
                                let (old_font, old_tall, old_cut) = fit(&was.text, true);
                                let was_under = self.under(lines, before);
                                let with_under = !was_under.is_empty() && line_top + old_tall + 2.0 + under_tall <= bottom;
                                step = old_tall + 4.0 + if with_under { under_tall } else { 0.0 };
                                let up = line_top - eased * step;
                                let going = (1.0 - eased * 1.8).max(0.0);
                                let (_, sung_colour) = self.line_colours(1.0);
                                frame.text_wrapped(&shown(&was.text, old_font, old_cut), old_font, sung_colour.alpha(going), left, up, width);
                                if with_under {
                                    words(frame, &fitted(frame, &was_under, self.fonts.under, width), self.fonts.under, Color::hex(self.ink, 0.5 * going), left, up + old_tall + 2.0, width + 2.0, Align::Start, self.edged);
                                }
                            }
                        }
                        let y = line_top + (1.0 - eased) * step;
                        let (dim, bright) = self.line_colours(eased);
                        if text.trim().is_empty() {
                            interlude(frame, left, y, self.fonts.line.size, between(line, next, self.moment.position), dim, bright);
                            (y, self.fonts.line.size * 1.4)
                        } else {
                            let (font, tall, cut) = fit(&text, !under.is_empty());
                            // (Not a row of it, where not one fits.)
                            if tall <= room {
                                let text = shown(&text, font, cut);
                                let layout = frame.gfx.wrapped(&text, font, width);
                                let sung = sung_to(&parts, self.moment.position);
                                self.filled(&layout, &text, font, left, y, width, sung, dim, bright, 12.0);
                            }
                            (y, tall)
                        }
                    }
                };
                // Under it, where there is room, its translation or the next line (coming in with it).
                let under_top = line_y + tall + 2.0;
                if !under.is_empty() && line_top + tall + 2.0 + under_tall <= bottom {
                    words(frame, &fitted(frame, &under, self.fonts.under, width), self.fonts.under, Color::hex(self.ink, 0.5 * eased), left, under_top, width + 2.0, Align::Start, self.edged);
                }
            }
            // None sung yet: the dots, lit as the music before the first line goes on.
            (Some(lines), None) => {
                let first = lines.first().map_or(0.0, |line| line.at);
                let done = if first > 0.0 { (self.moment.position / first).clamp(0.0, 1.0) as f32 } else { 0.0 };
                let (dim, bright) = self.line_colours(1.0);
                interlude(frame, left, line_top, self.fonts.line.size, done, dim, bright);
            }
            // None to be had: why, in their place.
            (None, _) => {
                let said = self.moment.said.clone().unwrap_or_default();
                if self.edged {
                    outlined(frame, &fitted(frame, &said, self.fonts.under, width), self.fonts.under, Color::hex(self.ink, 0.8), left, line_top + 2.0, width + 2.0);
                } else {
                    frame.text_wrapped(&said, self.fonts.under, Color::hex(self.ink, 0.62), left, line_top + 2.0, width);
                }
            }
        }
        self.unclip();
    }

    /// The stage: the record at the start (the cover in its middle, turning as it plays), what plays and the lines
    /// round the one sung now beside it, and along the bottom the spectrum over the controls and the track.
    fn stage(&mut self) {
        let (w, h) = self.size;
        let pad = pad_of(Layout::Stage, self.shows);
        let at = self.placed(Layout::Stage);
        // (The record: see `disc`.)
        let ((_, cy), _) = stage_record(self.size, self.shows);
        // Along the bottom, all across: kept showing, the controls and the track, and the spectrum over them; else the
        // spectrum, the controls coming over it as the pointer does (as in every layout).
        // (Nearer its foot than its sides: as near as a card's rows are to its own.)
        let foot = h - STAGE_FOOT;
        let row = foot - CONTROL;
        let floor = if self.shows.controls {
            self.bottom_row(pad, w - pad, row, 1.0, None);
            if self.shows.spectrum {
                spectrum(self.frame, self.moment, (self.accent, self.tint), (pad, row - ROW_GAP, w - 2.0 * pad, STAGE_SPECTRUM), 0.85);
            }
            row - ROW_GAP - if self.shows.spectrum { STAGE_SPECTRUM + 10.0 } else { 0.0 }
        } else {
            let shown_in = self.music.controls_shown();
            if shown_in < 1.0 && self.shows.spectrum {
                spectrum(self.frame, self.moment, (self.accent, self.tint), (pad, foot - 2.0, w - 2.0 * pad, STAGE_SPECTRUM), 0.85 * (1.0 - shown_in));
            }
            if shown_in > 0.0 {
                self.bottom_row(pad, w - pad, row, shown_in, None);
            }
            foot - if self.shows.spectrum { STAGE_SPECTRUM } else { CONTROL } - 10.0
        };
        // (Down to the rows along the bottom; the line sung now level with the record's middle, as far as the lines
        // next to it stay whole.)
        let (top, bottom) = (at.below, floor);
        if bottom <= top + 20.0 {
            return;
        }
        match self.moment.lines.clone() {
            // (As large as the stage is tall: made taller, its record grows and the column beside it narrows, and the
            // lines grow with it, not smaller.)
            Some(lines) => self.rail_of_lines(&lines, at.left, top, bottom, at.width, Some(cy), stage_line(h)),
            None => {
                let said = self.moment.said.clone().unwrap_or_default();
                let y = (top + bottom) / 2.0 - self.fonts.under_font.size;
                words(self.frame, &fitted(self.frame, &said, self.fonts.under_font, at.width), self.fonts.under_font, Color::hex(self.ink, 0.75), at.left, y, at.width + 2.0, Align::Start, self.edged);
            }
        }
    }

    /// The record round the cover, as much of it as the layout shows (see `Placed::record`): the stage's, or the
    /// record growing in round the cover or going as a layout turns into the stage or out of it, the cover moving
    /// into its middle or out of it, rounding as it goes, and turned as the record has turned, as far as it is round
    /// (so as it comes to rest in the record's middle it is turned as the record is, and nothing jumps).
    fn disc(&mut self, layout: Layout) {
        let at = self.placed(layout);
        let Some((x, y, side)) = at.cover.filter(|_| at.record > 0.0) else { return };
        let frame = self.frame;
        let (cx, cy, label) = (x + side / 2.0, y + side / 2.0, side / 2.0);
        let r = label / RECORD_LABEL;
        let faded = |paint: &dyn Fn()| {
            if at.record < 1.0 {
                let everything = windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F { left: -f32::MAX, top: -f32::MAX, right: f32::MAX, bottom: f32::MAX };
                let layer = D2D1_LAYER_PARAMETERS1 { contentBounds: everything, opacity: at.record, ..Default::default() };
                unsafe { frame.dc.PushLayer(&layer, None) };
                paint();
                unsafe { frame.dc.PopLayer() };
            } else {
                paint();
            }
        };
        faded(&|| self.record_under(cx, cy, r));
        // The cover over it: its corners from a card's to round, turned as the record is as far as it is round.
        let corner = cover_corner(side) + (label - cover_corner(side)) * at.round;
        // (`turned_by` in degrees, as the record's turn is kept; turned in radians.)
        let angle = (self.music.turned_by * at.round).to_radians();
        let (ink, edged) = (self.ink, self.edged);
        frame.turned(cx, cy, angle, (1.0, 1.0), |frame| match &self.moment.cover {
            Some(cover) => {
                let picture = super::canvas::Picture { key: cover.key, side: crate::cover::SIDE, pixels: cover.pixels.clone() };
                frame.picture(&picture, x, y, side, side, corner, 1.0);
            }
            None => {
                frame.fill_rounded(Color::hex(0x2A2A30, 1.0), x, y, side, side, corner);
                frame.icon(Icon::Music, Point { x: cx, y: cy }, side * 0.42, Color::hex(ink, if edged { 0.6 } else { 0.5 }));
            }
        });
        faded(&|| self.record_over(cx, cy, r));
    }

    /// The record `r` in radius round `(cx, cy)`, under its label: black, its grooves catching the light.
    fn record_under(&self, cx: f32, cy: f32, r: f32) {
        let frame = self.frame;
        let side = 2.0 * r;
        frame.shadow(Color::hex(0x000000, 0.45), cx - r, cy - r, side, side, r, 14.0, 5.0);
        frame.fill_circle(Color::hex(0x0E0E10, 1.0), Point { x: cx, y: cy }, r);
        // Its grooves: fine rings, lighter toward its edge.
        let label = r * RECORD_LABEL;
        let mut ring = label + 3.0;
        while ring < r - 3.0 {
            let shade = 0.035 + 0.05 * (ring - label) / (r - label);
            frame.stroke_rounded(Fill::Solid(Color::hex(0xFFFFFF, shade)), cx - ring - 0.4, cy - ring - 0.4, 2.0 * ring + 0.8, 2.0 * ring + 0.8, ring + 0.4, 0.8);
            ring += 4.0;
        }
        frame.stroke_rounded(Fill::Solid(Color::hex(0xFFFFFF, 0.12)), cx - r, cy - r, side, side, r, 1.0);
    }

    /// Over the record's label: a ring of the cover's colour round it, a sheen of it catching the light over the
    /// record, its hole, and its arm.
    fn record_over(&self, cx: f32, cy: f32, r: f32) {
        let frame = self.frame;
        let label = r * RECORD_LABEL;
        frame.stroke_rounded(Fill::Solid(colour(self.accent, 0.9)), cx - label - 3.25, cy - label - 3.25, 2.0 * label + 6.5, 2.0 * label + 6.5, label + 3.25, 2.5);
        let mut sheen = r - 6.0;
        while sheen > label + 8.0 {
            frame.stroke_rounded(Fill::Solid(colour(self.accent, 0.05 * (sheen - label) / (r - label))), cx - sheen - 0.75, cy - sheen - 0.75, 2.0 * sheen + 1.5, 2.0 * sheen + 1.5, sheen + 0.75, 1.5);
            sheen -= 11.0;
        }
        // Its hole.
        frame.fill_circle(Color::hex(0x0E0E10, 1.0), Point { x: cx, y: cy }, (r * 0.04).max(3.0));
        self.arm_of(cx, cy, r);
    }

    /// The record's arm: its pivot past the record's top right; the arm reaching from it, bent a little as a tonearm
    /// is, to its headshell, on the record's grooves while something plays and swung off it (to its right) while
    /// nothing does, swinging between as it starts and stops.
    fn arm_of(&self, cx: f32, cy: f32, r: f32) {
        let frame = self.frame;
        let pivot = Point { x: cx + r * 0.98, y: cy - r * 0.98 };
        let reach = r * 1.35;
        let swung = self.music.arm * self.music.arm * (3.0 - 2.0 * self.music.arm);
        let angle = (ARM_OFF + (ARM_ON - ARM_OFF) * swung).to_radians();
        let head = Point { x: pivot.x + reach * angle.cos(), y: pivot.y + reach * angle.sin() };
        // Bent outward (away from the record) at its middle: a quadratic curve through a point off the straight line.
        let (dx, dy) = (head.x - pivot.x, head.y - pivot.y);
        let bend = Point { x: (pivot.x + head.x) / 2.0 - dy * 0.16, y: (pivot.y + head.y) / 2.0 + dx * 0.16 };
        let curve: Vec<Point> = (0..=24)
            .map(|step| {
                let t = step as f32 / 24.0;
                let (a, b, c) = ((1.0 - t) * (1.0 - t), 2.0 * (1.0 - t) * t, t * t);
                Point { x: a * pivot.x + b * bend.x + c * head.x, y: a * pivot.y + b * bend.y + c * head.y }
            })
            .collect();
        let thick = (r * 0.028).max(2.5);
        let shade: Vec<Point> = curve.iter().map(|p| Point { x: p.x + 1.5, y: p.y + 2.5 }).collect();
        frame.stroke(&shade, Color::hex(0x000000, 0.28), thick + 1.0);
        frame.stroke(&curve, Color::hex(0xF4F4F6, 1.0), thick);
        // Its headshell, along the way the arm ends: white, a slot across it, a line of the cover's colour.
        let along = (head.y - bend.y).atan2(head.x - bend.x).to_degrees();
        let (long, wide) = ((r * 0.2).max(16.0), (r * 0.075).max(7.0));
        let mut parent = windows_numerics::Matrix3x2::default();
        unsafe { frame.dc.GetTransform(&mut parent) };
        let turned = windows_numerics::Matrix3x2::rotation_around(along, windows_numerics::Vector2 { X: head.x, Y: head.y });
        unsafe { frame.dc.SetTransform(&(turned * parent)) };
        frame.shadow(Color::hex(0x000000, 0.35), head.x - long * 0.3, head.y - wide / 2.0, long, wide, wide * 0.3, 4.0, 2.0);
        frame.fill_rounded(Color::hex(0xF4F4F6, 1.0), head.x - long * 0.3, head.y - wide / 2.0, long, wide, wide * 0.3);
        frame.fill_rounded(Color::hex(0x2A2A2E, 0.85), head.x + long * 0.12, head.y - wide * 0.32, long * 0.08, wide * 0.64, 1.0);
        frame.fill_rounded(colour(self.accent, 1.0), head.x + long * 0.32, head.y - wide * 0.18, long * 0.34, wide * 0.36, wide * 0.18);
        unsafe { frame.dc.SetTransform(&parent) };
        // Its pivot: a ring of white round a dark hub.
        let ring = (r * 0.075).max(7.0);
        frame.shadow(Color::hex(0x000000, 0.4), pivot.x - ring, pivot.y - ring, 2.0 * ring, 2.0 * ring, ring, 5.0, 2.0);
        frame.fill_circle(Color::hex(0xF4F4F6, 1.0), pivot, ring);
        frame.fill_circle(Color::hex(0x2A2A2E, 1.0), pivot, ring * 0.48);
        frame.fill_circle(Color::hex(0xFFFFFF, 0.5), Point { x: pivot.x - ring * 0.15, y: pivot.y - ring * 0.15 }, ring * 0.14);
    }

    /// The card (and the card stood on end): what plays at the top, the lines round the one sung now gliding past
    /// it, the rows along the bottom.
    fn card(&mut self, layout: Layout) {
        let (_, h) = self.size;
        let pad = pad_of(layout, self.shows);
        let at = self.placed(layout);
        let (x, width) = (at.left, at.width);
        let free = self.rows(x, h - pad, width, None);
        let (top, bottom) = (at.below, free - 10.0);
        if bottom <= top + 20.0 {
            return;
        }
        match self.moment.lines.clone() {
            Some(lines) => self.rail_of_lines(&lines, x, top, bottom, width, None, (width / 16.0).clamp(18.0, self.fonts.now.size)),
            None => {
                let said = self.moment.said.clone().unwrap_or_default();
                let height = self.frame.gfx.wrapped_height(&said, self.fonts.under_font, width);
                let y = (top + bottom - height) / 2.0;
                if self.edged {
                    outlined(self.frame, &fitted(self.frame, &said, self.fonts.under_font, width), self.fonts.under_font, Color::hex(self.ink, 0.8), x, y, width + 2.0);
                } else {
                    self.frame.text_wrapped(&said, self.fonts.under_font, Color::hex(self.ink, 0.7), x, y, width);
                }
            }
        }
    }

    /// The lines before and after the one aimed at (sung now, or turned to by the wheel), smaller and fainter the
    /// farther they are from it, the rail gliding; the one sung now filled as it is sung, its translation under it;
    /// the one turned to, its time beside it. `size`: the line aimed at's (the others' smaller).
    fn rail_of_lines(&mut self, lines: &[Line], x: f32, top: f32, bottom: f32, width: f32, anchor: Option<f32>, size: f32) {
        let frame = self.frame;
        // (Short, the line aimed at in its middle, as much shown above as below it; tall, a little above the middle,
        // more of what is to come shown.)
        let anchor = anchor.unwrap_or(top + (bottom - top) * if bottom - top < 220.0 { 0.5 } else { 0.38 });
        let (fade_top, fade_bottom) = (((bottom - top) * 0.12).min(28.0), ((bottom - top) * 0.15).min(34.0));
        let current = self.moment.now;
        let turned = turned_to(current, self.music.turned, lines.len());
        let translated = self.shows.translation;
        // Each line's size and how it shows, by how far it is from the rail.
        let scale_at = |d: f32| {
            let d = d.abs();
            if d <= 1.0 { 1.0 - 0.32 * d } else { (0.68 * 0.9f32.powf(d - 1.0)).clamp(0.5, 0.68) }
        };
        let opacity_at = |d: f32| {
            if d >= 0.0 {
                if d <= 1.0 { 1.0 - 0.28 * d } else { (0.72 - 0.18 * (d - 1.0)).max(0.3) }
            } else {
                let d = -d;
                if d <= 1.0 { 1.0 - 0.48 * d } else { (0.52 - 0.12 * (d - 1.0)).max(0.22) }
            }
        };
        let gap = 10.0;
        // Each line is laid out in the room it has at its largest (see `at_rest` below), and drawn at its size as a
        // whole. All laid out at one size (the line's full size at its largest), in the room they have at that size, and
        // drawn at theirs: where the size goes with the room (a card made larger), each line's room at that size is
        // the same whatever its size, and so are its rows (laid out at each size, the rounding of the sizes moved
        // where a line just filling its rows broke).
        let full = self.fonts.now;
        let zoom = |scale: f32| scale * size / full.size;
        // The line turned to, and whether what plays it from goes under it (a row more: see `chip_at`).
        let seekable = self.seekable();
        let chip_under = |index: usize, scale: f32| {
            let line = &lines[index];
            let text = if line.text.trim().is_empty() { "…" } else { &line.text };
            let chip = frame.measure(&super::text::clock(line.at.max(0.0) as u32), self.fonts.time) + 26.0;
            seekable && turned == Some(index) && chip_at(frame, text, full, width, zoom(scale), zoom(scale_at(0.0)), chip).2
        };
        // Each line in rows as it is at its largest, the size it comes to rest at as the one aimed at, and drawn smaller
        // as a whole (laid out again at each size it was drawn at, it broke into other rows as it grew, and the
        // lyrics moved as they glided).
        let at_rest = width / zoom(scale_at(0.0));
        let height_of = |index: usize, scale: f32| {
            let line = &lines[index];
            frame.gfx.wrapped_height(if line.text.trim().is_empty() { "M" } else { &line.text }, full, at_rest) * zoom(scale)
                + if translated && line.translation.is_some() { TRANSLATION_ROW * scale } else { 0.0 }
                + if chip_under(index, scale) { CHIP_ROW } else { 0.0 }
        };
        let rail = self.music.rail;
        let base = rail.floor().max(-1.0);
        // The lines from a few before the rail to a few after, each placed from the one the rail is nearest.
        let first = (base as isize - 4).max(0) as usize;
        let last = ((base as isize + 7).max(0) as usize).min(lines.len());
        if first >= last {
            return;
        }
        // Their places (the middle of each), laid one after another at their sizes, then shifted so the rail's
        // place is at the anchor.
        let mut middles = Vec::with_capacity(last - first);
        let mut y = 0.0;
        for index in first..last {
            let scale = scale_at(index as f32 - rail);
            let tall = height_of(index, scale);
            middles.push((index, y + tall / 2.0, scale, tall));
            y += tall + gap;
        }
        let at_rail = {
            let below = middles.iter().find(|m| m.0 as f32 >= rail).or(middles.last()).map_or(0.0, |m| m.1);
            let above = middles.iter().rev().find(|m| (m.0 as f32) <= rail).map_or(below, |m| m.1);
            let t = rail - rail.floor();
            if rail < first as f32 { below } else { above + (below - above) * t }
        };
        // Where it is asked for, as far as the lines next to the one aimed at stay within it: one before, one after.
        let anchor = {
            // (The line it glides to: not the one it is nearest on its way, which would change half way there.)
            let aimed = turned.or(current).map_or(rail.round() as isize, |index| index as isize);
            // (Clear of the fades at its edges: shown whole.)
            // (As they will be once glided there, at their sizes then, the one aimed at at the anchor: not as they are on
            // the way, which would move it as they glide.)
            let tall_at = |index: isize, d: f32| (index >= 0 && (index as usize) < lines.len()).then(|| height_of(index as usize, scale_at(d)));
            let aimed_half = tall_at(aimed, 0.0).unwrap_or(0.0) / 2.0;
            let least = tall_at(aimed - 1, -1.0).map_or(f32::MIN, |tall| top + fade_top + aimed_half + gap + tall);
            let most = tall_at(aimed + 1, 1.0).map_or(f32::MAX, |tall| bottom + 12.0 - fade_bottom - aimed_half - gap - tall);
            if least <= most { anchor.clamp(least, most) } else { (least + most) / 2.0 }
        };
        // Moving there as the rail glides (from where it was; a rail come anew, there at once).
        let to = anchor - top;
        let aimed = turned.or(current).map_or(-1, |index| index as isize);
        let mut state = match self.music.anchor.filter(|was| (was.shown - to).abs() < bottom - top) {
            Some(was) if was.aimed == aimed && (was.to - to).abs() <= 0.5 => was,
            Some(was) => Anchor { from: was.shown, from_rail: rail, aimed, to, shown: was.shown },
            None => Anchor { from: to, from_rail: rail, aimed, to, shown: to },
        };
        let span = aimed as f32 - state.from_rail;
        let along = if span.abs() > 1e-3 { ((rail - state.from_rail) / span).clamp(0.0, 1.0) } else { 1.0 };
        state.shown = state.from + (state.to - state.from) * along;
        self.music.anchoring = along < 1.0 && (state.shown - to).abs() > 0.3;
        self.music.anchor = Some(state);
        let anchor = top + state.shown;
        let (tint, clear) = (self.tint, self.edged);
        // While the rail glides, its lines drawn where they fall (see `Frame::set_moving`): put on whole pixels, a line
        // easing the last of the way stood still, then stepped a pixel.
        let _moving = frame.moving_while(rail != rail.round() || self.music.anchoring);
        self.clip(x - 8.0, top, width + 16.0, bottom - top + 12.0);
        #[cfg(test)]
        self.music.drawn_lines.clear();
        for (index, middle, scale, tall) in middles {
            let d = index as f32 - rail;
            let y = anchor + middle - at_rail - tall / 2.0;
            #[cfg(test)]
            self.music.drawn_lines.push((index, y));
            if y + tall < top - 4.0 || y > bottom + 12.0 {
                continue;
            }
            // Faded toward the rail's edges.
            let edge = ((y - top) / fade_top).clamp(0.0, 1.0).min(((bottom + 12.0 - (y + tall)) / fade_bottom).clamp(0.0, 1.0));
            let alpha = opacity_at(d) * edge;
            let line = &lines[index];
            let font = Font { size: size * scale, ..self.fonts.now };
            if turned == Some(index) {
                self.turned_wrapped(lines, index, x, y, width, full, zoom(scale), zoom(scale_at(0.0)));
            } else if Some(index) == current {
                let next = lines.get(index + 1).map(|line| line.at);
                let (text, parts) = parts(line, next);
                let bright = colour(if self.light { toward([0.0; 3], tint, 0.75) } else { toward([1.0; 3], tint, 0.35) }, alpha);
                let dim = Color::hex(self.ink, if clear { 0.55 } else { 0.34 } * alpha);
                if text.trim().is_empty() {
                    let done = between(line, next, self.moment.position);
                    // What has gone glows in the cover's colour.
                    frame.glow(colour(tint, 0.5 * alpha), 6.0, |frame| interlude(frame, x, y, font.size, done, Color::hex(0, 0.0), bright));
                    interlude(frame, x, y, font.size, done, dim, bright);
                } else {
                    // (At its full size, drawn smaller as a whole: see `full`.)
                    let room = at_rest;
                    let layout = frame.gfx.wrapped(&text, full, room);
                    let sung = sung_to(&parts, self.moment.position);
                    let soft = (font.size * 0.45).clamp(6.0, 16.0) / zoom(scale);
                    frame.scaled(x, y, zoom(scale), |frame| {
                        // What is sung glows in the cover's colour.
                        frame.glow(colour(tint, 0.5 * alpha), 6.0, |frame| sung_part(frame, &layout, &text, full, x, y, room, sung, bright, soft));
                        self.filled(&layout, &text, full, x, y, room, sung, dim, bright, soft);
                    });
                }
                if let Some(translation) = line.translation.as_ref().filter(|_| translated) {
                    let under = if text.trim().is_empty() { font.size * 1.4 } else { frame.gfx.wrapped_height(&text, full, at_rest) * zoom(scale) };
                    let small = Font { size: self.fonts.under_font.size * scale, ..self.fonts.under_font };
                    words(frame, &fitted(frame, translation, small, width), small, Color::hex(self.ink, 0.62 * alpha), x, y + under + 2.0 * scale, width, Align::Start, clear);
                }
            } else {
                let shown = Color::hex(self.ink, if clear { 0.7 } else { 0.5 } * alpha);
                if line.text.trim().is_empty() {
                    interlude(frame, x, y, font.size, 0.0, shown, shown);
                } else {
                    let room = at_rest;
                    frame.scaled(x, y, zoom(scale), |frame| if clear { outlined(frame, &line.text, full, shown, x, y, room) } else { frame.text_wrapped(&line.text, full, shown, x, y, room) });
                }
                // Its translation under it, as the line sung now's is (smaller with it).
                if let Some(translation) = line.translation.as_ref().filter(|_| translated && !line.text.trim().is_empty()) {
                    let under = frame.gfx.wrapped_height(&line.text, full, at_rest) * zoom(scale);
                    let small = Font { size: self.fonts.under_font.size * scale, ..self.fonts.under_font };
                    words(frame, &fitted(frame, translation, small, width), small, Color::hex(self.ink, 0.42 * alpha), x, y + under + 2.0 * scale, width, Align::Start, clear);
                }
                // Pressed, played from (where the wheel has turned the lines: as the line turned to is).
                if turned.is_some() && self.seekable() {
                    self.hits.push((x, y, width, tall, Hit::Line(index as u16)));
                }
            }
        }
        self.unclip();
    }

    /// How far the line sung now has risen in as it came (0 to 1, eased).
    fn came(&self) -> f32 {
        let came = self.moment.since.map_or(1.0, |since| (self.now.saturating_duration_since(since).as_secs_f32() / 0.28).min(1.0));
        1.0 - (1.0 - came).powi(3)
    }

    /// The line sung now's colours: what is not sung yet, and what is (near white, the cover's colour in it).
    fn line_colours(&self, eased: f32) -> (Color, Color) {
        let dim = if self.edged { 0.6 } else { 0.36 };
        let toward_ink = if self.light { [0.0; 3] } else { [1.0; 3] };
        (Color::hex(self.ink, dim * eased), colour(toward(self.tint, toward_ink, if self.light { 0.25 } else { 0.65 }), eased))
    }

    /// What is under the line sung now: its translation (where there is one and they are on), else the next line.
    fn under(&self, lines: &[Line], now: usize) -> String {
        let translation = lines[now].translation.clone().filter(|_| self.shows.translation);
        let next = lines.get(now + 1).map(|line| line.text.clone());
        translation.or(next).unwrap_or_default()
    }

    /// A line drawn in `dim`, and over it in `bright` as far as it is sung; edged, with no ground.
    #[allow(clippy::too_many_arguments)]
    fn filled(&self, layout: &IDWriteTextLayout, text: &str, font: Font, x: f32, y: f32, width: f32, sung: Filling, dim: Color, bright: Color, soft: f32) {
        if self.edged {
            edge(self.frame, bright.a, |frame| frame.text_wrapped(text, font, Color::hex(0xFFFFFF, 1.0), x, y, width));
        }
        filled(self.frame, layout, text, font, x, y, width, sung, dim, bright, soft);
    }
}

impl Music {
    /// How it is laid out at `size`, as `paint` lays it out (see `Layout::for_room`).
    pub fn layout_at(&self, size: (f32, f32), shows: &Shows) -> Layout {
        Layout::for_room(size, shows, self.layout)
    }

    /// Where buttons set in its top right corner go, `size` large: near the corner, not as far in as what it shows (they
    /// are the window's, not what it shows).
    pub fn top_right(&self, size: (f32, f32)) -> (f32, f32) {
        (CORNER, size.0 - CORNER)
    }

    /// The colour its own ground shows most along its edges, `size` large (for what stands in for it, as where it
    /// would stick): the mean of the blurred cover where it shows round the edge (it is laid centred, as large as the
    /// longer side and a third more), darkened and washed as the ground is. With no cover, the ground's own dark.
    pub fn ground_colour(&self, (w, h): (f32, f32)) -> [f32; 3] {
        let Some(pixels) = &self.ground_pixels else { return [0.064, 0.064, 0.075] };
        let side = crate::cover::SIDE as usize;
        let laid = w.max(h).max(1.0) * BACKDROP_GROWN;
        // What of it shows (as far each way from its middle, of its side), and how far in from the edge counts.
        let (half_x, half_y) = (w / 2.0 / laid, h / 2.0 / laid);
        let band = w.min(h) * 0.15 / laid;
        let (mut sum, mut count) = ([0.0f32; 3], 0.0f32);
        for y in 0..side {
            let v = (y as f32 + 0.5) / side as f32 - 0.5;
            for x in 0..side {
                let u = (x as f32 + 0.5) / side as f32 - 0.5;
                let shows = u.abs() <= half_x && v.abs() <= half_y;
                if shows && (u.abs() >= half_x - band || v.abs() >= half_y - band) {
                    let p = &pixels[(y * side + x) * 4..][..3];
                    sum = [sum[0] + p[2] as f32, sum[1] + p[1] as f32, sum[2] + p[0] as f32];
                    count += 1.0;
                }
            }
        }
        let mean = sum.map(|c| c / count.max(1.0) / 255.0);
        std::array::from_fn(|i| mean[i] * (1.0 - BACKDROP_DARKENED) + self.accent[i] * 0.06)
    }

    /// How far the controls have come in over the spectrum (0 to 1), eased.
    fn controls_shown(&self) -> f32 {
        self.controls_in * self.controls_in * (3.0 - 2.0 * self.controls_in)
    }

    /// The cover behind it, blurred: made once for each cover (the one before fading out over a moment).
    /// No cover under it (the track has none): the last one's fades.
    fn no_backdrop(&mut self, now: Instant) {
        if let Some((_, before)) = self.backdrop.take() {
            self.fading = Some((before, now));
            self.ground_pixels = None;
        }
    }

    fn backdrop(&mut self, frame: &Frame, cover: &Arc<Cover>, now: Instant) {
        let key = cover.key;
        if self.backdrop.as_ref().is_some_and(|(of, _)| *of == key) {
            return;
        }
        let side = crate::cover::SIDE as usize;
        let pixels = blurred(&cover.pixels, side, 9);
        let properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
            ..Default::default()
        };
        if let Ok(bitmap) = unsafe { frame.dc.CreateBitmap(D2D_SIZE_U { width: side as u32, height: side as u32 }, Some(pixels.as_ptr().cast()), side as u32 * 4, &properties) } {
            if let Some((_, before)) = self.backdrop.take() {
                self.fading = Some((before, now));
            }
            self.backdrop = Some((key, bitmap));
            self.ground_pixels = Some(Arc::new(pixels));
        }
    }
}

/// How a line is sung, part by part: each part's place in the text drawn (UTF-16, from and to) and its time (from
/// and to, seconds). Word by word where that is known; else each character of Chinese or Japanese, each word of
/// other writing (weighed by its length), sharing nine tenths of the line's time.
/// The line the wheel has turned the lyrics to (`turned` lines on from the one sung now, `now`, or from before the first
/// where none is, as the rail goes: see `Music::paint_moment`), of `count`; none where they are not turned. (Turned
/// from the first line where none was sung, the line shown largest, which the rail went to, was the one before it.)
fn turned_to(now: Option<usize>, turned: i32, count: usize) -> Option<usize> {
    (turned != 0 && count > 0).then(|| (now.map_or(-1, |now| now as i32) + turned).clamp(0, count as i32 - 1) as usize)
}

/// Where what plays a turned line from goes (see `Drawing::turned_wrapped`), the line `text` laid out in rows of
/// `width / zoom` at `full`, drawn `zoom` times as large, what plays it `chip` wide: after its last row where that row
/// has room for it at `settled` (the zoom the line comes to rest at, the largest it is drawn at: on the rail it grows
/// to it as the rail glides to it, and judged at each zoom on the way, what plays it went under it only as it came
/// there), else under the line. From the line's corner; and whether under it (a row more for the line).
fn chip_at(frame: &Frame, text: &str, full: Font, width: f32, zoom: f32, settled: f32, chip: f32) -> (f32, f32, bool) {
    // (In rows as at rest, at every size: see `rail_of_lines`.)
    let room = width / settled;
    let (end, _) = frame.gfx.wrapped_end(text, full, room);
    if end * settled + 10.0 + chip <= width {
        let (end, row_top) = frame.gfx.wrapped_end(text, full, room);
        // Its middle level with the middle of the row's words as the eye takes it: between the baseline and a top
        // half way between a capital's and a small letter's (most of a row's letters are small; level with the
        // capitals' middle, it stood above the words, and reckoned from the font's size, more so).
        let (cap_top, _) = frame.gfx.ink("H", full);
        let (small_top, _) = frame.gfx.ink("x", full);
        let middle = ((cap_top + small_top) / 2.0 + frame.gfx.baseline(full).0) / 2.0;
        (end * zoom + 10.0, (row_top + middle) * zoom - 9.0, false)
    } else {
        (0.0, frame.gfx.wrapped_height(text, full, room) * zoom + 4.0, true)
    }
}

/// The room a turned line's chip takes under it, where its last row has none for it (see `chip_at`).
const CHIP_ROW: f32 = 22.0;

fn parts(line: &Line, next: Option<f64>) -> (String, Vec<(usize, usize, f64, f64)>) {
    if !line.words.is_empty() {
        let mut text = String::new();
        let mut parts = Vec::new();
        for word in &line.words {
            let from = text.encode_utf16().count();
            text.push_str(&word.text);
            parts.push((from, text.encode_utf16().count(), word.at, word.end.max(word.at + 0.01)));
        }
        return (text, parts);
    }
    let text = line.text.clone();
    let end = line.end.or(next).unwrap_or(line.at + 4.0).max(line.at + 0.3);
    let span = (end - line.at) * 0.9;
    // The pieces: a character of the scripts written without spaces, a run of anything else up to a space.
    let mut pieces: Vec<(usize, usize, f64)> = Vec::new();
    let mut at = 0;
    let mut run: Option<usize> = None;
    let close = |pieces: &mut Vec<(usize, usize, f64)>, run: &mut Option<usize>, to: usize| {
        if let Some(from) = run.take() {
            pieces.push((from, to, 1.0 + 0.15 * (to - from) as f64));
        }
    };
    for c in text.chars() {
        let units = c.len_utf16();
        let wide = matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF);
        if wide {
            close(&mut pieces, &mut run, at);
            pieces.push((at, at + units, 1.0));
        } else if c.is_whitespace() {
            close(&mut pieces, &mut run, at);
        } else if run.is_none() {
            run = Some(at);
        }
        at += units;
    }
    close(&mut pieces, &mut run, at);
    // Each piece on to where the next begins (the spaces between them filled as the piece before them is sung, not
    // jumped over as the next one begins).
    let ends: Vec<usize> = pieces.iter().skip(1).map(|piece| piece.0).chain([at]).collect();
    for (piece, end) in pieces.iter_mut().zip(ends) {
        piece.1 = end;
    }
    let weight: f64 = pieces.iter().map(|p| p.2).sum::<f64>().max(1e-6);
    let mut t = line.at;
    let parts = pieces
        .into_iter()
        .map(|(from, to, w)| {
            let took = span * w / weight;
            let part = (from, to, t, t + took);
            t += took;
            part
        })
        .collect();
    (text, parts)
}

/// How far a line is sung: all of it before `from` (UTF-16), and `part` (0 to 1) of the way across what runs from
/// `from` to `to`, the part being sung, by how wide it is drawn: filled at an even pace across it, however wide each
/// of its characters (a character and the space after it share a part's time: by units, the fill crossed the one fast
/// and crawled over the other).
#[derive(Clone, Copy, PartialEq, Debug)]
struct Filling {
    from: usize,
    to: usize,
    part: f32,
}

/// How far the text drawn is sung at `t`.
fn sung_to(parts: &[(usize, usize, f64, f64)], t: f64) -> Filling {
    let mut done = Filling { from: 0, to: 0, part: 0.0 };
    for (from, to, at, end) in parts {
        if t < *at {
            break;
        }
        done = if t >= *end { Filling { from: *to, to: *to, part: 0.0 } } else { Filling { from: *from, to: *to, part: ((t - at) / (end - at)) as f32 } };
    }
    done
}

/// How far through it is (0 to 1) at `t`: a line without words (the music between), from its start to its end or
/// the next line's.
fn between(line: &Line, next: Option<f64>, t: f64) -> f32 {
    let end = line.end.or(next).unwrap_or(line.at + 4.0).max(line.at + 0.3);
    ((t - line.at) / (end - line.at)).clamp(0.0, 1.0) as f32
}

/// The music between lines, where a line of words `size` DIPs tall would be (its box's corner at `(x, y)`): three
/// dots in `dim`, each coming to `bright` (and a little larger) as its third of it goes (`done`, 0 to 1): how many
/// are lit, how much of it has gone.
fn interlude(frame: &Frame, x: f32, y: f32, size: f32, done: f32, dim: Color, bright: Color) {
    let radius = size * INTERLUDE_DOT;
    let centre_y = y + size * 0.68;
    for index in 0..3 {
        let lit = (done * 3.0 - index as f32).clamp(0.0, 1.0);
        let lit = lit * lit * (3.0 - 2.0 * lit);
        let color = Color { r: dim.r + (bright.r - dim.r) * lit, g: dim.g + (bright.g - dim.g) * lit, b: dim.b + (bright.b - dim.b) * lit, a: dim.a + (bright.a - dim.a) * lit };
        if color.a > 0.0 {
            frame.fill_circle(color, Point { x: x + radius * 1.25 + index as f32 * size * INTERLUDE_STEP, y: centre_y }, radius * (1.0 + 0.25 * lit));
        }
    }
}

/// How tall a row of words written down is, of their size.
const DOWN_ROW: f32 = 1.2;

/// Whether `text` is in a script written down a character under another (Chinese, Japanese, Korean): others are set
/// in rows across, as narrow as the strip, as they are never written down.
pub(crate) fn written_down(text: &str) -> bool {
    text.chars().any(|c| matches!(c as u32, 0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF))
}

/// How wide `text` in `font` is set in a column `column` wide: `down`, a character to a row where its script is
/// written so (see `written_down`); else rows as wide as the column.
fn down_wrap(text: &str, font: Font, column: f32, down: bool) -> f32 {
    if down && written_down(text) { (font.size * DOWN_ROW).min(column) } else { column }
}

/// How tall a row of `text` in `font` is, set `wrap` wide: its rows' height shared among them (a script's own, the
/// rows of Chinese taller than of Latin letters).
fn row_of(frame: &Frame, text: &str, font: Font, wrap: f32) -> f32 {
    let mut metrics = windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_METRICS::default();
    let _ = unsafe { frame.gfx.wrapped(text, font, wrap).GetMetrics(&mut metrics) };
    metrics.height / metrics.lineCount.max(1) as f32
}

/// Whether `text` in `font` goes in a column `column` wide (`down` as in `down_wrap`) with no word broken: written a
/// character to a row, always; else its widest word no wider than the column.
fn unbroken(frame: &Frame, text: &str, font: Font, column: f32, down: bool) -> bool {
    (down && written_down(text)) || text.split_whitespace().all(|word| frame.measure(word, font) <= column)
}

/// The size `text` is set at in a column `column` wide and `room` tall (`down` as in `down_wrap`): `font`'s, no wider
/// than the column, smaller till it all fits with no word broken, down to the least (`STRIP_LEAST`; longer still,
/// what is past the room is not shown).
fn sized_down(frame: &Frame, text: &str, font: Font, column: f32, room: f32, down: bool) -> Font {
    let widest = Font { size: if down && written_down(text) { font.size.min(column / DOWN_ROW) } else { font.size }.max(1.0), ..font };
    (0..)
        .map(|step| widest.size - step as f32)
        .take_while(|size| *size >= STRIP_LEAST.min(widest.size))
        .map(|size| Font { size, ..font })
        .find(|font| unbroken(frame, text, *font, column, down) && frame.gfx.wrapped_height(text, *font, down_wrap(text, *font, column, down)) <= room)
        .unwrap_or(Font { size: STRIP_LEAST.min(widest.size), ..font })
}

/// `text` written down (see `down_wrap`) from (`x`, `y`), `wrap` wide, in `colour`; edged, with no ground.
#[allow(clippy::too_many_arguments)]
fn words_down(frame: &Frame, text: &str, font: Font, colour: Color, x: f32, y: f32, wrap: f32, clear: bool) {
    if clear {
        edge(frame, colour.a, |frame| frame.text_wrapped(text, font, Color::hex(0xFFFFFF, 1.0), x, y, wrap));
    }
    frame.text_wrapped(text, font, colour, x, y, wrap);
}

/// A line drawn in `dim`, and over it in `bright` as far as it is sung (see `sung_part`).
#[allow(clippy::too_many_arguments)]
fn filled(frame: &Frame, layout: &IDWriteTextLayout, text: &str, font: Font, x: f32, y: f32, width: f32, sung: Filling, dim: Color, bright: Color, soft: f32) {
    frame.text_wrapped(text, font, dim, x, y, width);
    sung_part(frame, layout, text, font, x, y, width, sung, bright, soft);
}

/// As much of a line as is sung (`sung`), in `bright`, its edge soft: row by row as `layout` breaks
/// it, the row being sung up to the place sung, the rows before it whole.
#[allow(clippy::too_many_arguments)]
fn sung_part(frame: &Frame, layout: &IDWriteTextLayout, text: &str, font: Font, x: f32, y: f32, width: f32, sung: Filling, bright: Color, soft: f32) {
    if sung.to == 0 {
        return;
    }
    let mut count = 0;
    let _ = unsafe { layout.GetLineMetrics(None, &mut count) };
    let mut rows = vec![DWRITE_LINE_METRICS::default(); count as usize];
    let _ = unsafe { layout.GetLineMetrics(Some(&mut rows), &mut count) };
    // Where the place sung is: its row's top and the x it is at (between the characters about it).
    let place = |unit: usize| {
        let (mut px, mut py, mut metrics) = (0.0, 0.0, DWRITE_HIT_TEST_METRICS::default());
        let _ = unsafe { layout.HitTestTextPosition(unit as u32, false, &mut px, &mut py, &mut metrics) };
        (px, metrics.top)
    };
    // The part sung as it is laid out, a piece on each row it runs over: as far along all of them as it is sung (on
    // the second row as soon as the first is passed; taken as on its first alone, it stood still there while it
    // was sung, then the rows after it came all at once).
    let mut count = 0;
    let length = (sung.to - sung.from) as u32;
    let _ = unsafe { layout.HitTestTextRange(sung.from as u32, length, 0.0, 0.0, None, &mut count) };
    let mut pieces = vec![DWRITE_HIT_TEST_METRICS::default(); count as usize];
    let laid = unsafe { layout.HitTestTextRange(sung.from as u32, length, 0.0, 0.0, Some(&mut pieces), &mut count) }.is_ok();
    pieces.truncate(if laid { count as usize } else { 0 });
    let (top, at) = match pieces.first() {
        None => {
            let (x0, top) = place(sung.from);
            (top, x0)
        }
        Some(first) => {
            let mut left = pieces.iter().map(|piece| piece.width).sum::<f32>() * sung.part;
            let mut on = (first.top, first.left);
            for piece in &pieces {
                on = (piece.top, piece.left + left.min(piece.width));
                if left <= piece.width {
                    break;
                }
                left -= piece.width;
            }
            on
        }
    };
    let mut row_top = 0.0;
    for row in &rows {
        let height = row.height;
        if row_top + height <= top + 0.5 {
            frame.clip(x - 4.0, y + row_top, width + 8.0, height);
            frame.text_wrapped(text, font, bright, x, y, width);
            frame.unclip();
        } else if row_top <= top + 0.5 {
            // The row being sung: whole to the place, then fading over the soft edge, in steps.
            frame.clip(x - 4.0, y + row_top, at + 4.0, height);
            frame.text_wrapped(text, font, bright, x, y, width);
            frame.unclip();
            for step in 0..4 {
                let from = at + soft * step as f32 / 4.0;
                frame.clip(x + from, y + row_top, soft / 4.0 + 0.5, height);
                frame.text_wrapped(text, font, bright.alpha(1.0 - (step as f32 + 0.5) / 4.0), x, y, width);
                frame.unclip();
            }
            break;
        }
        row_top += height;
    }
}

/// How far apart the spectrum's bars are (DIPs): dense, each thin.
const SPECTRUM_PITCH: f32 = 7.0;

/// The spectrum moving with the music: as many bars as fit `SPECTRUM_PITCH` apart across `width` from `x`, standing on `floor`, `tall` at most,
/// shaded from the accent to the tint, `alpha` of it shown.
fn spectrum(frame: &Frame, moment: &Moment, (accent, tint): ([f32; 3], [f32; 3]), (x, floor, width, tall): (f32, f32, f32, f32), alpha: f32) {
    let bars = ((width / SPECTRUM_PITCH).round() as usize).max(8);
    let bar = width / bars as f32;
    for index in 0..bars {
        let at = index as f32 / (bars - 1) as f32 * (crate::sound::BANDS - 1) as f32;
        let (low, frac) = (at.floor() as usize, at.fract());
        let value = moment.sound.bands[low] * (1.0 - frac) + moment.sound.bands[(low + 1).min(crate::sound::BANDS - 1)] * frac;
        let height = 2.0 + value.powf(1.6) * (tall - 2.0);
        let shade = toward(accent, tint, index as f32 / bars as f32);
        // Standing on the floor: its column put on whole pixels once, the bar as tall as it is from the column's
        // bottom (put on pixels alone, a bar's bottom moved by a pixel as its height did).
        let column = frame.on_pixels(x + index as f32 * bar + bar * 0.2, floor - tall, bar * 0.6, tall);
        let (bx, by, bw, bh) = super::view::filled_part(frame, column, height / tall, true);
        frame.fill_rounded(colour(shade, (0.3 + 0.45 * value) * alpha), bx, by, bw, bh, bar * 0.3);
    }
}

/// Its own ground: its shadow, the cover blurred behind (the one before fading out) and darkened, the cover's colour
/// washed from a corner; all inside its rounded corners, with what `on` draws. With none of its own, only what `on`
/// draws.
struct Backdrop {
    size: (f32, f32),
    radius: f32,
    backdrop: Option<ID2D1Bitmap1>,
    before: Option<ID2D1Bitmap1>,
    shown: f32,
    accent: [f32; 3],
    tint: [f32; 3],
    /// Its own (else none: no ground, or another widget's).
    own: bool,
}

impl Backdrop {
    fn paint(&self, frame: &Frame, on: impl FnOnce(&Frame)) -> windows::core::Result<()> {
        if !self.own {
            on(frame);
            return Ok(());
        }
        let ((w, h), radius) = (self.size, self.radius);
        frame.shadow(Color::hex(0x000000, 0.4), 0.0, 0.0, w, h, radius, 8.0, 4.0);
        let mask = unsafe { frame.gfx.factory.CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT { rect: rect(0.0, 0.0, w, h), radiusX: radius, radiusY: radius }) }?;
        let layer = D2D1_LAYER_PARAMETERS1 {
            contentBounds: rect(0.0, 0.0, w, h),
            geometricMask: std::mem::ManuallyDrop::new(mask.cast().ok()),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: windows_numerics::Matrix3x2::identity(),
            opacity: 1.0,
            ..Default::default()
        };
        unsafe { frame.dc.PushLayer(&layer, None) };
        frame.fill(Color::hex(0x101014, 1.0), 0.0, 0.0, w, h);
        let (bw, bh) = (w.max(h) * BACKDROP_GROWN, w.max(h) * BACKDROP_GROWN);
        let place = rect((w - bw) / 2.0, (h - bh) / 2.0, bw, bh);
        if let Some(bitmap) = &self.backdrop {
            if let Some(before) = &self.before {
                unsafe { frame.dc.DrawBitmap(before, Some(&place), 1.0 - self.shown, D2D1_INTERPOLATION_MODE_LINEAR, None, None) };
            }
            unsafe { frame.dc.DrawBitmap(bitmap, Some(&place), self.shown, D2D1_INTERPOLATION_MODE_LINEAR, None, None) };
        }
        // Darkened for the words to be read on, the cover's colour washed from a corner.
        frame.fill(Color::hex(0x000000, BACKDROP_DARKENED), 0.0, 0.0, w, h);
        frame.fill_shape(&[Point { x: 0.0, y: 0.0 }, Point { x: w, y: 0.0 }, Point { x: w, y: h }, Point { x: 0.0, y: h }], Fill::Down { top: 0.0, from: colour(self.accent, 0.12), bottom: h, to: colour(self.tint, 0.0) });
        on(frame);
        unsafe { frame.dc.PopLayer() };
        drop(std::mem::ManuallyDrop::into_inner(layer.geometricMask));
        Ok(())
    }
}

/// `colour` as a canvas colour, as opaque as `alpha`.
fn colour([r, g, b]: [f32; 3], alpha: f32) -> Color {
    Color { r, g, b, a: alpha }
}

/// `a` eased toward `b` by `t`.
fn toward(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// A colour deep enough to read on the light (its hue kept; its lightness lowered to at most 0.4).
fn deep(rgb: [f32; 3]) -> [f32; 3] {
    let (h, s, l) = crate::cover::hsl(rgb);
    crate::cover::rgb(h, s.min(0.85), l.min(0.4))
}

/// A colour light enough to read on the dark (its hue kept; its lightness raised to at least `least`).
fn readable(rgb: [f32; 3], least: f32) -> [f32; 3] {
    let (h, s, l) = crate::cover::hsl(rgb);
    crate::cover::rgb(h, s.min(0.85), l.max(least))
}

/// A cover's pixels blurred (three box blurs, of `radius` pixels, across then down), for behind it.
fn blurred(pixels: &[u8], side: usize, radius: usize) -> Vec<u8> {
    let mut image: Vec<f32> = pixels.iter().map(|&b| b as f32).collect();
    let mut other = image.clone();
    for _ in 0..3 {
        box_pass(&image, &mut other, side, radius, true);
        box_pass(&other, &mut image, side, radius, false);
    }
    image.into_iter().map(|v| v.round().clamp(0.0, 255.0) as u8).collect()
}

/// One box blur of a square BGRA image, across its rows or down its columns (the edges held).
fn box_pass(from: &[f32], to: &mut [f32], side: usize, radius: usize, across: bool) {
    let at = |line: usize, i: usize| if across { (line * side + i) * 4 } else { (i * side + line) * 4 };
    let span = (2 * radius + 1) as f32;
    for line in 0..side {
        for channel in 0..4 {
            let mut sum: f32 = (0..=2 * radius).map(|k| from[at(line, (k as isize - radius as isize).clamp(0, side as isize - 1) as usize) + channel]).sum();
            for i in 0..side {
                to[at(line, i) + channel] = sum / span;
                let out = (i as isize - radius as isize).clamp(0, side as isize - 1) as usize;
                let into = (i + radius + 1).min(side - 1);
                sum += from[at(line, into) + channel] - from[at(line, out) + channel];
            }
        }
    }
}

/// `text`, or as much of its start as is no wider than `room` with an ellipsis after it.
fn fitted(frame: &Frame, text: &str, font: Font, room: f32) -> String {
    if frame.measure(text, font) <= room {
        return text.to_string();
    }
    let mut kept: Vec<char> = text.chars().collect();
    loop {
        kept.pop();
        let cut = format!("{}…", kept.iter().collect::<String>().trim_end());
        if kept.is_empty() || frame.measure(&cut, font) <= room {
            return cut;
        }
    }
}

/// Words, edged where there is no ground (`clear`) to be read on whatever is behind.
#[allow(clippy::too_many_arguments)]
/// `now` moved toward `to` (0 or 1) by as much of the way as `LIGHT_UP` takes in a frame `step` long (no more than a
/// thirtieth of a second's worth: smooth after a rest too).
fn eased_toward(now: f32, to: f32, step: f32) -> f32 {
    let by = step.min(1.0 / 30.0) / LIGHT_UP;
    if to > now { (now + by).min(to) } else { (now - by).max(to) }
}

fn words(frame: &Frame, text: &str, font: Font, colour: Color, x: f32, y: f32, width: f32, align: Align, clear: bool) {
    if clear {
        edge(frame, colour.a, |frame| frame.text(text, font, Color::hex(0xFFFFFF, 1.0), x, y, width, align));
    }
    frame.text(text, font, colour, x, y, width, align);
}

/// A dark edge all round what `paint` draws (as words are drawn with no ground), `alpha` as strong: its shape
/// blurred a little, even on every side wherever it falls on the pixels. (Drawn again a little aside each way, the
/// copies fell now on one pixel, now on the next, as what was drawn moved, and the edge came and went.)
fn edge(frame: &Frame, alpha: f32, paint: impl FnOnce(&Frame)) {
    frame.glow(Color::hex(0x000000, 0.7 * alpha), 1.2, paint);
}

/// Words with a dark edge under them, to be read on whatever is behind.
fn outlined(frame: &Frame, text: &str, font: Font, colour: Color, x: f32, y: f32, width: f32) {
    words(frame, text, font, colour, x, y, width, Align::Start, true);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::Word;

    /// A player heard but not telling Windows what it plays is said to be so, by its name; with nothing heard,
    /// nothing plays.
    #[test]
    fn a_player_that_tells_nothing_is_said_to() {
        use crate::ui::text::Lang;
        let heard = crate::playing::Now { app: "QQ音乐".into(), playing: true, ..Default::default() };
        assert_eq!(unsaid(Some(&heard), Lang::Zh), "QQ音乐未向系统提供歌曲信息");
        assert_eq!(unsaid(Some(&crate::playing::Now { app: String::new(), ..heard.clone() }), Lang::En), "This player does not tell Windows what it plays");
        assert_eq!(unsaid(None, Lang::Zh), "当前没有正在播放的歌曲");
        // (One that says what it plays, with no lyrics shown, is not said to tell nothing.)
        assert_eq!(unsaid(Some(&crate::playing::Now { title: "渡尘寰".into(), ..heard }), Lang::Zh), "当前没有正在播放的歌曲");
    }

    fn line(text: &str, words: Vec<Word>) -> Line {
        Line { at: 10.0, end: None, text: text.into(), words, backing: None, translation: None }
    }

    /// How far it is sung, in UTF-16 units (fractional across the part being sung).
    fn units(sung: Filling) -> f32 {
        sung.from as f32 + (sung.to - sung.from) as f32 * sung.part
    }

    /// Where what plays a turned line goes, after its last row or under it, is judged at the size the line comes to
    /// rest at, the same at every size it grows through on the way (judged at each, it went under the line only as the
    /// line came there).
    #[test]
    #[cfg(feature = "studio")]
    fn a_turned_lines_chip_keeps_its_place_as_the_line_grows() {
        let _device = crate::ui::gfx::held().unwrap();
        let gfx = crate::ui::gfx::Gfx::new().expect("graphics");
        let full = Font::new(crate::ui::canvas::Family::Segoe, 26.0, 700.0);
        gfx.draw_offscreen((8, 8), 1.0, |frame| {
            for text in ["这是我的温柔", "Every heartbeat sounds like thunder", "短"] {
                for width in [180.0f32, 220.0, 260.0, 400.0] {
                    let settled = chip_at(frame, text, full, width, 1.0, 1.0, 64.0).2;
                    for zoom in [0.55f32, 0.68, 0.8, 0.9, 1.0] {
                        assert_eq!(chip_at(frame, text, full, width, zoom, 1.0, 64.0).2, settled, "{text} in {width} at {zoom}");
                    }
                }
            }
        })
        .expect("drawn");
    }

    /// Made taller from the strip into the player, at any width, the cover never grows smaller on the way (as tall as
    /// the strip, it was larger near the player than the player's as it began), as drawn.
    #[test]
    fn the_cover_grows_from_the_strip_into_the_player() {
        let shows = Shows::from(&serde_json::Value::Null);
        for w in [240.0f32, 300.0, 420.0, 600.0] {
            let mut was = 0.0f32;
            let mut h = LEAST.1;
            while h < CARD_FROM {
                let layout = Layout::for_room((w, h), &shows, None);
                // (As drawn: a player less tall than the least it is, that, smaller as a whole: see `zoom_at`.)
                let zoom = zoom_at(layout, (w, h), &shows);
                let side = placed(layout, (w / zoom, h / zoom), &shows).cover.map_or(0.0, |(_, _, side)| side * zoom);
                assert!(side + 1e-3 >= was, "at {w} x {h} ({}): {side}, before {was}", layout.key());
                was = side;
                h += 2.0;
            }
        }
    }

    /// Turned by the wheel with no line sung yet, the first notch is the first line, the line the rail goes to and the
    /// one shown largest alike; the last line is in reach; turned back, none.
    #[test]
    fn the_line_turned_to_with_none_sung() {
        assert_eq!(turned_to(None, 1, 5), Some(0));
        assert_eq!(turned_to(None, 5, 5), Some(4));
        assert_eq!(turned_to(None, 0, 5), None);
        assert_eq!(turned_to(Some(2), -1, 5), Some(1));
        assert_eq!(turned_to(Some(2), 9, 5), Some(4));
        assert_eq!(turned_to(None, 1, 0), None);
    }

    #[test]
    fn a_line_sung_part_by_part() {
        // Word by word: as the words say.
        let worded = line("ab cd", vec![Word { at: 10.0, end: 11.0, text: "ab ".into() }, Word { at: 11.0, end: 12.0, text: "cd".into() }]);
        let (text, worded) = parts(&worded, None);
        assert_eq!(text, "ab cd");
        assert_eq!(units(sung_to(&worded, 10.5)), 1.5);
        assert_eq!(units(sung_to(&worded, 11.5)), 4.0);
        assert_eq!(units(sung_to(&worded, 20.0)), 5.0);
        assert_eq!(units(sung_to(&worded, 9.0)), 0.0);
        // By line: each Chinese character a part, the time shared.
        let (_, shared) = parts(&line("你好 hi", Vec::new()), Some(14.0));
        assert_eq!(shared.iter().map(|p| (p.0, p.1)).collect::<Vec<_>>(), [(0, 1), (1, 3), (3, 5)]);
        // The spaces between characters filled with the one before them: no jump over them.
        let (_, spaced) = parts(&line("微 微 微 微", Vec::new()), Some(14.0));
        assert_eq!(spaced.iter().map(|p| (p.0, p.1)).collect::<Vec<_>>(), [(0, 2), (2, 4), (4, 6), (6, 7)]);
        let between = |t: f64| units(sung_to(&spaced, t));
        assert!((between(10.0 + 0.9 * 0.75) - 1.5).abs() < 1e-6, "three quarters through the first part, the fill is half across its space");
        assert!((shared.last().unwrap().3 - (10.0 + 4.0 * 0.9)).abs() < 1e-9);
    }

    /// Whatever a strip's size, what ends its row (its keys, its spectrum) lies in it after where its words begin, never
    /// over its cover nor out of its edge (a strip as small as a widget is let be showed its spectrum out past its left
    /// edge, over its cover); its words never end before they begin.
    #[test]
    fn a_strip_ends_inside_itself() {
        let keys_wide = 3.0 * CONTROL + 2.0 * KEY_GAP;
        let all = Shows { cover: true, title: true, translation: true, spectrum: true, controls: false, clear: false, inset: None };
        for shows in [all, Shows { controls: true, ..all }, Shows { cover: false, ..all }, Shows { cover: false, controls: true, ..all }] {
            for centred in [false, true] {
                let mut w = LEAST.0;
                while w < 480.0 {
                    for h in [LEAST.1, 72.0, STRIP_MOST] {
                        let pad = pad_of(Layout::Strip, &shows);
                        let start = if centred { pad } else { placed(Layout::Strip, (w, h), &shows).left };
                        let ends = strip_ends(w, start, pad, &shows, centred);
                        if let Some(x) = ends.keys {
                            assert!(x >= start - 1e-3 && x + keys_wide <= w - pad + 1e-3, "keys at {x} in {w} x {h} from {start}: {ends:?}");
                        }
                        if let Some((x, wide)) = ends.spectrum {
                            assert!(x >= start - 1e-3 && x + wide <= w - pad + 1e-3, "spectrum at {x} in {w} x {h} from {start}: {ends:?}");
                        }
                        assert!(centred || ends.end >= start - 1e-3 || ends.keys.is_none(), "{w} x {h}: {ends:?}");
                        if centred {
                            assert!(pad + ends.side <= w - pad - ends.side + 1e-3, "{w} x {h}: {ends:?}");
                        }
                    }
                    w += 2.0;
                }
            }
        }
    }

    #[test]
    fn the_room_chooses_the_layout() {
        let all = Shows { cover: true, title: true, translation: true, spectrum: true, controls: false, clear: false, inset: None };
        assert_eq!(Layout::for_room((420.0, 76.0), &all, None), Layout::Strip);
        assert_eq!(Layout::for_room((470.0, 178.0), &all, None), Layout::Player);
        assert_eq!(Layout::for_room((470.0, 400.0), &all, None), Layout::Card);
        assert_eq!(Layout::for_room((290.0, 470.0), &all, None), Layout::Tall);
        assert_eq!(Layout::for_room((900.0, 470.0), &all, None), Layout::Stage);
        assert_eq!(Layout::for_room((900.0, 470.0), &Shows { cover: false, ..all }, None), Layout::Card);
        // Narrow: a strip down, as tall as it is wide or taller; short, a strip across.
        assert_eq!(Layout::for_room((80.0, 300.0), &all, None), Layout::Column);
        assert_eq!(Layout::for_room((150.0, 150.0), &all, None), Layout::Column);
        assert_eq!(Layout::for_room((56.0, 56.0), &all, None), Layout::Column);
        assert_eq!(Layout::for_room((150.0, 60.0), &all, None), Layout::Strip);
        // Without the lines, a player however tall.
                // Controls kept over the spectrum: a row more before the player.
        assert_eq!(Layout::for_room((470.0, 130.0), &Shows { controls: true, ..all }, None), Layout::Strip);
        // About a threshold, the layout it has is kept.
        assert_eq!(Layout::for_room((470.0, 245.0), &all, Some(Layout::Card)), Layout::Card);
        assert_eq!(Layout::for_room((470.0, 255.0), &all, Some(Layout::Player)), Layout::Player);
        assert_eq!(Layout::for_room((470.0, 270.0), &all, Some(Layout::Player)), Layout::Card);
    }

    #[test]
    fn a_size_comes_to_rest_where_its_layout_is_right() {
        let all = Shows { cover: true, title: true, translation: true, spectrum: true, controls: false, clear: false, inset: None };
        // A strip pulled taller, short of a player: back to the strip's tallest; a player short of a card: its.
        assert_eq!(rests_at((420.0, 110.0), &all), (420.0, STRIP_MOST));
        // Sized between the strip and the player, it is the one it is nearer, and comes to it.
        let more = 0.0;
        let half = strip_under(more);
        assert_eq!(Layout::for_room((420.0, half - 1.0), &all, None), Layout::Strip);
        assert_eq!(Layout::for_room((420.0, half + 1.0), &all, None), Layout::Player);
        assert_eq!(rests_at((420.0, half + 1.0), &all), (420.0, PLAYER_FROM + more));
        assert!(player_zoom(half + 1.0, more) < 1.0 && player_zoom(PLAYER_FROM + more, more) == 1.0);
        assert_eq!(rests_at((420.0, 245.0), &all), (420.0, PLAYER_MOST));
        // Within the range: where it was let go; the card, however tall; never under the least.
        assert_eq!(rests_at((420.0, 180.0), &all), (420.0, 180.0));
        assert_eq!(rests_at((470.0, 520.0), &all), (470.0, 520.0));
        assert_eq!(rests_at((20.0, 10.0), &all), LEAST);
        assert_eq!(rests_at((100.0, 20.0), &all), (100.0, LEAST.1));
        // A strip down, where it was let go.
        assert_eq!(rests_at((80.0, 300.0), &all), (80.0, 300.0));
    }
}

#[cfg(test)]
#[cfg(feature = "studio")]
mod pictures {
    use super::*;
    use crate::lyrics::Word;

    /// A track playing, its lines sung word by word (the second sung now, its third word being sung; or, `between`,
    /// the music between two lines), its cover a warm sky over a dark sea.
    fn playing(between: bool) -> Moment {
        let worded = |at: f64, text: &str| {
            let pieces: Vec<&str> = text.split_inclusive(' ').collect();
            let words = pieces.iter().enumerate().map(|(i, piece)| Word { at: at + i as f64 * 0.4, end: at + (i + 1) as f64 * 0.4, text: piece.to_string() }).collect();
            Line { at, end: Some(at + pieces.len() as f64 * 0.4), text: text.into(), words, backing: None, translation: None }
        };
        let mut lines = vec![
            worded(10.0, "Somewhere far beyond the city lights "),
            worded(14.0, "我们 在 黑夜里 奔跑 "),
            Line { at: 16.0, end: Some(20.0), text: String::new(), words: Vec::new(), backing: None, translation: None },
            worded(20.0, "Every heartbeat sounds like thunder "),
            worded(24.0, "直到 晨光 照亮 归途 "),
            worded(28.0, "And the morning finds us singing "),
        ];
        lines[1].translation = Some("We run through the night".into());
        lines[1].words.push(Word { at: 15.6, end: 15.9, text: "在 黎明 到来 之前".into() });
        lines[1].text.push_str("在 黎明 到来 之前");
        lines[0].translation = Some("远在城市灯火之外".into());
        lines[3].translation = Some("每一次心跳都像雷声".into());
        lines[4].translation = Some("Till the dawn lights the way home".into());
        let now = if between { 2 } else { 1 };
        let position = if between { 17.6 } else { lines[1].words[2].at + 0.2 };
        let mut bands = [0.0; crate::sound::BANDS];
        for (i, band) in bands.iter_mut().enumerate() {
            *band = (0.85 - i as f32 * 0.025 + 0.15 * (i as f32 * 1.7).sin()).clamp(0.0, 1.0);
        }
        let side = crate::cover::SIDE as usize;
        let mut pixels = vec![0u8; side * side * 4];
        for y in 0..side {
            for x in 0..side {
                let (fx, fy) = (x as f32 / side as f32, y as f32 / side as f32);
                let sun = (((fx - 0.62).powi(2) + (fy - 0.4).powi(2)).sqrt() < 0.12) as u8 as f32;
                let [r, g, b] = if fy < 0.6 { [0.95 - 0.3 * fy, 0.45 + 0.2 * fy, 0.35 + 0.1 * fx] } else { [0.08, 0.18 + 0.1 * fx, 0.32] };
                let at = (y * side + x) * 4;
                pixels[at] = ((b * (1.0 - sun) + 0.85 * sun) * 255.0) as u8;
                pixels[at + 1] = ((g * (1.0 - sun) + 0.95 * sun) * 255.0) as u8;
                pixels[at + 2] = ((r * (1.0 - sun) + 1.0 * sun) * 255.0) as u8;
                pixels[at + 3] = 255;
            }
        }
        Moment {
            lines: Some(Arc::new(lines)),
            now: Some(now),
            since: Some(Instant::now().checked_sub(Duration::from_secs(2)).unwrap_or_else(Instant::now)),
            position,
            length: Some((214.0, true)),
            dragged_to: None,
            title: "Night Runners".into(),
            artist: "Some Artist".into(),
            said: None,
            playing: true,
            cover: Some(Arc::new(Cover::new(pixels, [0.85, 0.5, 0.4], [0.95, 0.6, 0.35]))),
            no_cover: false,
            sound: crate::sound::Heard { level: 0.8, bands, beat: 0.3 },
        }
    }

    /// What plays drawn as a widget draws it (in a layer of its own: see `widget::MusicShown`) at a window's corner as
    /// the compositor's atlas might put it (1, 451, 902 and 1350 pixels down, a third of a DIP apart each at 1.5): drawn the
    /// same, pixel for pixel, at each
    /// (drawn straight onto the surface, its text came out a little differently in some of them, and shook).
    #[test]
    #[ignore = "draws on this PC's graphics card"]
    fn what_plays_is_drawn_the_same_wherever_in_the_atlas() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let all = Shows { cover: true, title: true, translation: true, spectrum: true, controls: false, clear: false, inset: None };
        let gfx = crate::ui::gfx::held().unwrap();
        let places = [1u32, 451, 902, 1350];
        let (w, h) = (470.0, 178.0);
        let margin = 34.0;
        let (width, tall) = (((w + 2.0 * margin) * 1.5) as u32, ((h + 2.0 * margin) * 1.5) as u32);
        // (One moment for all, nothing easing from one to the next.)
        let now = Instant::now();
        let pixels = gfx
            .draw_offscreen((width, 1350 + tall + 2), 1.5, |frame| {
                for at in places {
                    let mut music = Music { last_frame: now, ..Music::default() };
                    let moment = playing(false);
                    music.bands = moment.sound.bands;
                    music.rail = 1.0;
                    music.lyrics = moment.lines.as_ref().map(|lines| Arc::as_ptr(lines) as usize);
                    let mut layer = Layer::default();
                    frame.origin(0.0, at as f32 / 1.5);
                    let local = windows_numerics::Matrix3x2::translation(margin, margin);
                    let area = (-24.0, -24.0, w + 48.0, h + 48.0);
                    layer
                        .draw(frame, 1, local, area, 1.5, None, |frame| {
                            music.paint_moment(frame, (w, h), &all, Ground::Own, 1.5, None, now, moment);
                            Ok(())
                        })
                        .unwrap();
                }
            })
            .unwrap();
        let region = |at: u32| -> Vec<u8> { (at..at + tall).flat_map(|y| pixels[(y * width * 4) as usize..((y + 1) * width * 4) as usize].to_vec()).collect() };
        let first = region(places[0]);
        for at in places {
            let differing = first.chunks(4).zip(region(at).chunks(4)).filter(|(a, b)| a != b).count();
            println!("at {at}: {differing} pixels differ from at {}", places[0]);
            assert_eq!(differing, 0, "drawn at {at}, it differs from drawn at {}", places[0]);
        }
    }

    /// A player sized into a card, drawn as it turns (at its start, a third, two thirds and the end of the turn), to
    /// PNGs in the target folder: `cargo test --features studio -- --ignored draws_a_turn --nocapture`.
    #[test]
    #[ignore = "draws on this PC's graphics card, to pictures to be looked at"]
    fn draws_a_turn() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let all = Shows { cover: true, title: true, translation: true, spectrum: true, controls: false, clear: false, inset: None };
        let out = format!("{}/target/music", env!("CARGO_MANIFEST_DIR"));
        std::fs::create_dir_all(&out).unwrap();
        let gfx = crate::ui::gfx::held().unwrap();
        let start = Instant::now();
        let mut music = Music { last_frame: start, ..Music::default() };
        let (w, h) = (470.0, 400.0);
        let margin = 30.0;
        let size = (((w + 2.0 * margin) * 1.5) as u32, ((h + 2.0 * margin) * 1.5) as u32);
        // (Drawn once as a player, then as a card at each step of the turn.)
        for (name, room, at) in [("turn-before", (470.0, 178.0), 0u64), ("turn-0", (w, h), 1), ("turn-1", (w, h), 87), ("turn-2", (w, h), 174), ("turn-3", (w, h), 300)] {
            let moment = playing(false);
            music.bands = moment.sound.bands;
            music.rail = 1.0;
            music.lyrics = moment.lines.as_ref().map(|lines| Arc::as_ptr(lines) as usize);
            let now = start + Duration::from_millis(at);
            let pixels = gfx
                .draw_offscreen(size, 1.5, |frame| {
                    frame.fill(Color::hex(0x5A6470, 1.0), 0.0, 0.0, w + 2.0 * margin, h + 2.0 * margin);
                    frame.origin(margin, margin);
                    music.paint_moment(frame, room, &all, Ground::Own, 1.5, None, now, moment);
                })
                .unwrap();
            crate::studio::save_png(&std::path::PathBuf::from(format!("{out}/{name}.png")), size.0, size.1, &pixels).unwrap();
        }
    }

    /// A player sized down into a strip (each height a row, at its own size, 1.5 pixels a DIP), stacked into one PNG
    /// in the target folder: `cargo test --features studio -- --ignored draws_sized_into_a_strip`.
    #[test]
    #[ignore = "draws on this PC's graphics card, to a picture to be looked at"]
    fn draws_sized_into_a_strip() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let all = Shows { cover: true, title: true, translation: true, spectrum: true, controls: false, clear: false, inset: None };
        let out = format!("{}/target/music", env!("CARGO_MANIFEST_DIR"));
        std::fs::create_dir_all(&out).unwrap();
        let gfx = crate::ui::gfx::held().unwrap();
        let heights = [170.0f32, 150.0, 140.0, 130.0, 122.0, 120.0, 110.0, 96.0];
        let (w, margin) = (420.0f32, 12.0f32);
        let tall: f32 = heights.iter().map(|h| h + margin).sum::<f32>() + margin;
        let size = (((w + 2.0 * margin) * 1.5) as u32, (tall * 1.5) as u32);
        let now = Instant::now();
        let pixels = gfx
            .draw_offscreen(size, 1.5, |frame| {
                frame.fill(Color::hex(0x5A6470, 1.0), 0.0, 0.0, w + 2.0 * margin, tall);
                let mut y = margin;
                for h in heights {
                    let mut music = Music { last_frame: now, ..Music::default() };
                    let moment = playing(false);
                    music.bands = moment.sound.bands;
                    music.rail = 1.0;
                    music.lyrics = moment.lines.as_ref().map(|lines| Arc::as_ptr(lines) as usize);
                    frame.origin(margin, y);
                    music.paint_moment(frame, (w, h), &all, Ground::Own, 1.5, None, now, moment);
                    y += h + margin;
                }
            })
            .unwrap();
        crate::studio::save_png(&std::path::PathBuf::from(format!("{out}/sized-into-a-strip.png")), size.0, size.1, &pixels).unwrap();
    }

    /// The strip down at a few sizes, on its own ground and on none, a line sung in Chinese and one in English
    /// (`cargo test --features studio --lib -- --ignored draws_a_column` writes `target/music/column.png`): the line
    /// sung now written down, what is under it beside it where two go across, the key at the foot where it has room.
    #[test]
    #[ignore = "draws on this PC's graphics card"]
    fn draws_a_column() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let all = Shows { cover: true, title: true, translation: true, spectrum: true, controls: true, clear: false, inset: None };
        let out = format!("{}/target/music", env!("CARGO_MANIFEST_DIR"));
        std::fs::create_dir_all(&out).unwrap();
        let gfx = crate::ui::gfx::held().unwrap();
        let sizes = [(56.0f32, 56.0f32), (64.0, 320.0), (96.0, 380.0), (130.0, 420.0), (170.0, 200.0)];
        let margin = 14.0f32;
        let wide: f32 = sizes.iter().map(|(w, _)| w + margin).sum::<f32>() + margin;
        let row = 420.0 + margin;
        let tall = 4.0 * row + margin;
        let size = ((wide * 1.5) as u32, (tall * 1.5) as u32);
        let now = Instant::now();
        let pixels = gfx
            .draw_offscreen(size, 1.5, |frame| {
                frame.fill(Color::hex(0x5A6470, 1.0), 0.0, 0.0, wide, tall);
                for (at, (ground, english)) in [(Ground::Own, false), (Ground::Own, true), (Ground::Clear, false), (Ground::Own, false)].into_iter().enumerate() {
                    let mut x = margin;
                    for (w, h) in sizes {
                        let mut music = Music { last_frame: now, ..Music::default() };
                        let mut moment = playing(at == 3);
                        if english {
                            moment.now = Some(0);
                            moment.position = moment.lines.as_ref().unwrap()[0].words[2].at + 0.2;
                        }
                        music.bands = moment.sound.bands;
                        music.lyrics = moment.lines.as_ref().map(|lines| Arc::as_ptr(lines) as usize);
                        frame.origin(x, margin + at as f32 * row);
                        let shows = Shows { clear: ground == Ground::Clear, ..all };
                        music.paint_moment(frame, (w, h), &shows, ground, 1.5, None, now, moment);
                        x += w + margin;
                    }
                }
            })
            .unwrap();
        crate::studio::save_png(&std::path::PathBuf::from(format!("{out}/column.png")), size.0, size.1, &pixels).unwrap();
    }

    /// A player drawn smaller (sized toward the strip) turning into the strip begins from where its cover is drawn at
    /// that size, not from where it would be at its own (it shrank at once, then grew back as the turn began).
    #[test]
    #[ignore = "draws on this PC's graphics card"]
    fn a_smaller_player_turns_into_the_strip_from_where_it_was_drawn() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let all = Shows { cover: true, title: true, translation: true, spectrum: true, controls: false, clear: false, inset: None };
        let gfx = crate::ui::gfx::held().unwrap();
        let now = Instant::now();
        let mut music = Music { last_frame: now, ..Music::default() };
        let half = strip_under(0.0);
        let (w, before, after) = (470.0, half + 1.0, half - HOLD - 1.0);
        gfx.draw_offscreen((800, 300), 1.5, |frame| {
            music.paint_moment(frame, (w, before), &all, Ground::Own, 1.5, None, now, playing(false));
            music.paint_moment(frame, (w, after), &all, Ground::Own, 1.5, None, now, playing(false));
        })
        .unwrap();
        // (Where the player is drawn as large as it now is: its layout's parts begin a turn where they are at the size
        // it turns at, as the hand sizes it on from where they were.)
        let zoom = zoom_at(Layout::Player, (w, after), &all);
        let drawn = placed(Layout::Player, (w / zoom, after / zoom), &all).cover.unwrap().2 * zoom;
        let from = music.turning.expect("turning into the strip").1.cover.unwrap().2;
        assert!((from - drawn).abs() < 2.0, "the turn begins at {from}, the cover was drawn {drawn}");
    }

    /// Sized from one layout into another (a card into the stage, a player from its line to the lines round it), what
    /// it showed still shows as the turn begins (fading out as the new one's comes in), never a frame with no lines.
    #[test]
    #[ignore = "draws on this PC's graphics card"]
    fn a_turn_into_another_layout_never_shows_no_lines() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let all = Shows { cover: true, title: true, translation: true, spectrum: true, controls: false, clear: false, inset: None };
        let gfx = crate::ui::gfx::held().unwrap();
        let now = Instant::now();
        // (The least height at which a player this wide shows the lines round the one sung now.)
        let wide = 520.0;
        let lines_from = (PLAYER_FROM as u32..PLAYER_MOST as u32).map(|h| h as f32).find(|h| player_shows_lines((wide, *h), &all)).unwrap();
        let turns = [((440.0, 330.0), (600.0, 330.0)), ((wide, lines_from - 1.0), (wide, lines_from))];
        assert_eq!(Layout::for_room(turns[0].0, &all, None), Layout::Card);
        assert_eq!(Layout::for_room(turns[0].1, &all, Some(Layout::Card)), Layout::Stage);
        assert_eq!((Layout::for_room(turns[1].0, &all, None), Layout::for_room(turns[1].1, &all, None)), (Layout::Player, Layout::Player));
        let mut seen = Vec::new();
        for (from, to) in turns {
            let mut music = Music { last_frame: now, ..Music::default() };
            let mut draw = |size: (f32, f32)| gfx.draw_offscreen((900, 495), 1.5, |frame| {
                music.paint_moment(frame, size, &all, Ground::Own, 1.5, None, now, playing(false));
            });
            seen.push((draw(from).unwrap(), draw(to).unwrap()));
        }
        crate::ui::gfx::let_go();
        // (The words are near white, as nothing else it draws is: as many of them as before, near enough, what plays
        // at the top among them.)
        let words = |pixels: &[u8]| pixels.chunks(4).filter(|p| p[0] > 200 && p[1] > 200 && p[2] > 200).count();
        for ((settled, turning), (from, to)) in seen.iter().zip(turns) {
            let (before, as_it_turns) = (words(settled), words(turning));
            assert!(as_it_turns * 5 >= before * 4, "{from:?} to {to:?}: {as_it_turns} pixels of words as the turn begins, {before} before it");
        }
    }

    /// Where the fill's edge is (DIPs from the line's start) as a line of characters with spaces between them is sung,
    /// shared out evenly (the line 微 微 微 微 of 微微, from 43.06 s, the next at 48.09 s): printed every 50 ms.
    #[test]
    #[ignore = "prints for a person to read"]
    fn the_fill_goes_on_evenly() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let gfx = crate::ui::gfx::held().unwrap();
        let line = Line { at: 43.06, end: None, text: "微 微 微 微".into(), words: Vec::new(), backing: None, translation: None };
        let (text, parts) = parts(&line, Some(48.09));
        let layout = gfx.wrapped(&text, PLAYER_LINE, 1000.0);
        let place = |unit: usize| {
            let (mut px, mut py, mut metrics) = (0.0, 0.0, DWRITE_HIT_TEST_METRICS::default());
            let _ = unsafe { layout.HitTestTextPosition(unit as u32, false, &mut px, &mut py, &mut metrics) };
            px
        };
        println!("units: {:?}", (0..=text.encode_utf16().count()).map(place).collect::<Vec<_>>());
        let mut t = 43.0;
        while t < 47.5 {
            let sung = sung_to(&parts, t);
            let at = place(sung.from) + (place(sung.to) - place(sung.from)) * sung.part;
            println!("{t:.2} s  {sung:?}  edge {at:.1}");
            t += 0.05;
        }
    }

    /// The player as a line comes (from under the line before, where it was shown as the next): at its start, and 70,
    /// 140 and 210 ms in, stacked into one PNG in the target folder: `cargo test --features studio -- --ignored
    /// draws_a_line_coming --nocapture`.
    #[test]
    #[ignore = "draws on this PC's graphics card, to a picture to be looked at"]
    fn draws_a_line_coming() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let all = Shows { cover: true, title: true, translation: true, spectrum: true, controls: false, clear: false, inset: None };
        let out = format!("{}/target/music", env!("CARGO_MANIFEST_DIR"));
        std::fs::create_dir_all(&out).unwrap();
        let gfx = crate::ui::gfx::held().unwrap();
        let (w, h) = (470.0, 178.0);
        let margin = 20.0;
        let one = (((w + 2.0 * margin) * 1.5) as u32, ((h + 2.0 * margin) * 1.5) as u32);
        let mut sheet = Vec::new();
        for ms in [0u64, 70, 140, 210] {
            let mut moment = playing(false);
            let now = Instant::now();
            moment.since = Some(now - Duration::from_millis(ms));
            moment.position = 14.05;
            let mut music = Music { last_frame: now, ..Music::default() };
            music.bands = moment.sound.bands;
            let pixels = gfx
                .draw_offscreen(one, 1.5, |frame| {
                    frame.fill(Color::hex(0x5A6470, 1.0), 0.0, 0.0, w + 2.0 * margin, h + 2.0 * margin);
                    frame.origin(margin, margin);
                    music.paint_moment(frame, (w, h), &all, Ground::Own, 1.5, None, now, moment);
                })
                .unwrap();
            sheet.extend(pixels);
        }
        crate::studio::save_png(&std::path::PathBuf::from(format!("{out}/coming.png")), one.0, one.1 * 4, &sheet).unwrap();
    }

    /// What plays as a row under readings on a light skin (inside another widget, its margins the skin's), to a PNG.
    #[test]
    #[ignore = "draws on this PC's graphics card, to a picture to be looked at"]
    fn draws_what_plays_hosted() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let all = Shows { cover: true, title: true, translation: true, spectrum: true, controls: false, clear: false, inset: None };
        let gfx = crate::ui::gfx::held().unwrap();
        let (w, h) = (780.0, 64.0);
        let size = ((w * 1.5) as u32, (h * 1.5) as u32);
        let mut music = Music::default();
        let moment = playing(false);
        music.bands = moment.sound.bands;
        let pixels = gfx
            .draw_offscreen(size, 1.5, |frame| {
                frame.fill(Color::hex(0xF4F4F6, 1.0), 0.0, 0.0, w, h);
                music.paint_moment(frame, (w, h), &all, Ground::Host { dark: false, family: Family::Inter, inset: 18.0 }, 1.5, None, Instant::now(), moment);
            })
            .unwrap();
        crate::studio::save_png(&std::path::PathBuf::from(format!("{}/target/music/hosted.png", env!("CARGO_MANIFEST_DIR"))), size.0, size.1, &pixels).unwrap();
    }

    /// The lines glide on from one line sung to the next without a jump: frame by frame (16 ms apart) after the line
    /// sung changes, each line's place moves on by no more than it did the frame before by much (no step in its
    /// speed), in the card and in a player short enough that its rail is bounded by the lines next to the one sung.
    #[test]
    #[ignore = "draws on this PC's graphics card"]
    fn the_lines_glide_without_a_jump() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let all = Shows { cover: true, title: true, translation: true, spectrum: true, controls: false, clear: false, inset: None };
        let gfx = crate::ui::gfx::held().unwrap();
        for size in [(470.0, 400.0), (520.0, 236.0)] {
            let start = Instant::now();
            let mut music = Music { last_frame: start, ..Music::default() };
            let mut tracks: Vec<Vec<f32>> = Vec::new();
            // (The same lyrics throughout: other lyrics are another track's, and the rail starts afresh on them.)
            let base = playing(false);
            for frame_index in 0..40u64 {
                let now = start + Duration::from_millis(16 * frame_index);
                // The line before sung for the first frames, then the next.
                let mut moment = base.clone();
                if frame_index < 4 {
                    moment.now = Some(0);
                    moment.position = 11.0;
                }
                moment.since = Some(now - Duration::from_secs(2));
                gfx.draw_offscreen(((size.0 * 1.5) as u32, (size.1 * 1.5) as u32), 1.5, |frame| {
                    music.paint_moment(frame, size, &all, Ground::Own, 1.5, None, now, moment);
                })
                .unwrap();
                let at = |index: usize| music.drawn_lines.iter().find(|(i, _)| *i == index).map(|(_, y)| *y);
                tracks.push((0..4).map(|index| at(index).unwrap_or(f32::NAN)).collect());
            }
            println!("{size:?}:");
            for (frame_index, ys) in tracks.iter().enumerate() {
                println!("  {frame_index:2}  {}", ys.iter().map(|y| format!("{y:7.1}")).collect::<Vec<_>>().join(" "));
            }
            // From the change on: each line's move a frame no more than 3 DIPs off its move the frame before.
            for line in 0..4 {
                let ys: Vec<f32> = tracks[4..].iter().map(|ys| ys[line]).filter(|y| y.is_finite()).collect();
                for window in ys.windows(3) {
                    let (a, b) = (window[1] - window[0], window[2] - window[1]);
                    assert!((b - a).abs() <= 3.0, "{size:?}: line {line} jumped: moved {a:.1}, then {b:.1}");
                    // One way all along: never back the way it came.
                    assert!(!(a.abs() > 0.3 && b.abs() > 0.3 && a.signum() != b.signum()), "{size:?}: line {line} turned back: moved {a:.1}, then {b:.1}");
                }
            }
        }
    }

    /// What plays in each layout, at each size, with its switches as each case says, drawn to PNGs in the target
    /// folder: `cargo test --features studio -- --ignored draws_what_plays --nocapture`.
    #[test]
    #[ignore = "draws on this PC's graphics card, to pictures to be looked at"]
    fn draws_what_plays() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let all = Shows { cover: true, title: true, translation: true, spectrum: true, controls: false, clear: false, inset: None };
        let floating = Shows { cover: false, title: false, spectrum: false, clear: true, ..all };
        // Name, size, switches, the pointer over it (the controls come), lines turned by the wheel, the track dragged,
        // the music between lines.
        let cases: [(&str, (f32, f32), Shows, bool, i32, bool, bool); 24] = [
            ("strip", (420.0, 76.0), all, false, 0, false, false),
            ("strip-over", (420.0, 76.0), all, true, 0, false, false),
            ("strip-narrow-tall", (360.0, 96.0), all, false, 0, false, false),
            ("strip-narrow", (250.0, 56.0), all, false, 0, false, false),
            ("strip-kept", (480.0, 76.0), Shows { controls: true, ..all }, false, 0, false, false),
            ("player", (470.0, 178.0), all, false, 0, false, false),
            ("player-over", (470.0, 178.0), all, true, 0, false, false),
            ("player-dragged", (470.0, 178.0), all, true, 0, true, false),
            ("player-kept", (470.0, 210.0), Shows { controls: true, ..all }, false, 0, false, false),
            ("player-lines", (520.0, 236.0), all, false, 0, false, false),
            ("player-lines-short", (440.0, 190.0), all, false, 0, false, false),
            ("player-lines-tall", (700.0, 320.0), all, false, 0, false, false),
            ("player-nocover", (470.0, 178.0), Shows { cover: false, ..all }, false, 0, false, false),
            ("player-turned", (470.0, 178.0), all, false, 2, false, false),
            ("player-between", (470.0, 178.0), all, false, 0, false, true),
            ("card", (470.0, 400.0), all, false, 0, false, false),
            ("card-turned", (470.0, 400.0), all, false, 2, false, false),
            ("tall", (290.0, 480.0), all, false, 0, false, false),
            ("stage", (900.0, 470.0), all, false, 0, false, false),
            ("stage-narrow", (530.0, 330.0), all, false, 0, false, false),
            ("stage-off", (900.0, 470.0), all, false, 0, false, false),
            ("clear-player", (470.0, 178.0), Shows { clear: true, ..all }, false, 0, false, false),
            ("floating", (760.0, 110.0), floating, false, 0, false, false),
            ("floating-over", (760.0, 110.0), floating, true, 0, false, false),
        ];
        let out = format!("{}/target/music", env!("CARGO_MANIFEST_DIR"));
        std::fs::create_dir_all(&out).unwrap();
        let gfx = crate::ui::gfx::held().unwrap();
        for (name, (w, h), shows, over, turned, dragged, between) in cases {
            let mut music = Music { controls_in: if over || shows.controls { 1.0 } else { 0.0 }, turned, turned_at: Instant::now(), arm: if name.ends_with("off") { 0.0 } else { 1.0 }, turned_by: 30.0, ..Music::default() };
            let mut moment = playing(between);
            moment.dragged_to = dragged.then_some(150.0);
            music.bands = moment.sound.bands;
            // (The rail at the line aimed at, as it would be once glided there.)
            music.rail = moment.now.map_or(-1.0, |now| now as f32) + turned as f32;
            music.lyrics = moment.lines.as_ref().map(|lines| Arc::as_ptr(lines) as usize);
            let margin = 30.0;
            let size = (((w + 2.0 * margin) * 1.5) as u32, ((h + 2.0 * margin) * 1.5) as u32);
            let pointer = over.then_some((w * 0.5, h * 0.5));
            let pixels = gfx
                .draw_offscreen(size, 1.5, |frame| {
                    // (On a desktop of a mid grey.)
                    frame.fill(Color::hex(0x5A6470, 1.0), 0.0, 0.0, w + 2.0 * margin, h + 2.0 * margin);
                    frame.origin(margin, margin);
                    music.paint_moment(frame, (w, h), &shows, Ground::of(&shows), 1.5, pointer, Instant::now(), moment);
                })
                .unwrap();
            let path = std::path::PathBuf::from(format!("{out}/{name}.png"));
            crate::studio::save_png(&path, size.0, size.1, &pixels).unwrap();
            println!("{}", path.display());
        }
        // How long a frame takes as it is sized a little each frame (as a hand drags it), and at one size.
        {
            let shows = all;
            let mut music = Music { turned_by: 30.0, arm: 1.0, ..Music::default() };
            let start = Instant::now();
            for (label, sizing) in [("one size", false), ("sized each frame", true)] {
                let mut times = Vec::new();
                for i in 0..60 {
                    let (w, h) = if sizing { (400.0 + i as f32 * 3.0, 300.0 + i as f32 * 2.0) } else { (500.0, 360.0) };
                    let at = start + std::time::Duration::from_millis(16 * i as u64 + if sizing { 10_000 } else { 0 });
                    let began = Instant::now();
                    gfx.draw_offscreen(((w * 1.5) as u32 + 90, (h * 1.5) as u32 + 90), 1.5, |frame| {
                        frame.origin(30.0, 30.0);
                        music.paint_moment(frame, (w, h), &shows, Ground::of(&shows), 1.5, None, at, playing(false));
                    })
                    .unwrap();
                    times.push(began.elapsed().as_secs_f64() * 1000.0);
                }
                times.sort_by(f64::total_cmp);
                println!("music, {label}: median {:.1} ms, slowest {:.1} ms", times[30], times[59]);
            }
        }
        // Turning from the card into the stage and back, at moments on the way: the cover moving into the record's
        // middle, rounding, the record growing in round it (`turn-…-<ms>.png`).
        let shows = all;
        for (name, from, to) in [("card-stage", (480.0, 420.0), (900.0, 470.0)), ("stage-card", (900.0, 470.0), (480.0, 420.0))] {
            let mut music = Music { controls_in: 0.0, turned_by: 30.0, arm: 1.0, ..Music::default() };
            let moment = playing(false);
            music.bands = moment.sound.bands;
            music.rail = moment.now.map_or(-1.0, |now| now as f32);
            music.lyrics = moment.lines.as_ref().map(|lines| Arc::as_ptr(lines) as usize);
            let margin = 30.0;
            let (w, h) = (900.0f32, 470.0f32);
            let size = (((w + 2.0 * margin) * 1.5) as u32, ((h + 2.0 * margin) * 1.5) as u32);
            let start = Instant::now();
            let paint = |music: &mut Music, room: (f32, f32), at: Instant| {
                gfx.draw_offscreen(size, 1.5, |frame| {
                    frame.fill(Color::hex(0x5A6470, 1.0), 0.0, 0.0, w + 2.0 * margin, h + 2.0 * margin);
                    frame.origin(margin, margin);
                    music.paint_moment(frame, room, &shows, Ground::of(&shows), 1.5, None, at, playing(false));
                })
                .unwrap()
            };
            paint(&mut music, from, start);
            for ms in [0u64, 60, 130, 200, 300] {
                let pixels = paint(&mut music, to, start + std::time::Duration::from_millis(ms + 1));
                let path = std::path::PathBuf::from(format!("{out}/turn-{name}-{ms:03}.png"));
                crate::studio::save_png(&path, size.0, size.1, &pixels).unwrap();
                println!("{}", path.display());
            }
        }
    }
}
