//! The overlay: a few readings floating over the screen on a pane of frosted
//! glass, as a card (the frame rate large, the rest in tiles) or a strip.
//! What it shows and how it is drawn; its window is the panel thread's (see
//! `crate::overlay`), its frosting the window's (the screen behind, blurred).
//! The glass is tinted dark or light, as little as keeps every word of it
//! legible over what is behind (see `glass`).

use serde::{Deserialize, Serialize};

use super::canvas::{Align, Canvas, Color, Family, Fill, Font, Point};
use super::prefs::ModuleEntry;
use super::text::{self, Lang};
use crate::reading::{GameSample, Sample};

/// What the overlay can show, group by group (a group is one reading, a
/// tile on the card), in the order it shows them unless they are moved: a
/// group's id, and its items, each with whether it is shown by default. The
/// first item chosen of a group is its main figure; the rest follow it.
pub const GROUPS: [(&str, &[(&str, bool)]); 7] = [
    ("frames", &[("fps", true), ("low", true), ("frametime", false)]),
    ("cpu", &[("cpu", true), ("cpu_temp", true), ("cpu_power", false)]),
    ("gpu", &[("gpu", true), ("gpu_temp", true), ("gpu_power", false)]),
    ("memory", &[("memory", true)]),
    ("vram", &[("vram", true)]),
    ("network", &[("down", true), ("up", true)]),
    ("mic", &[("mic", true)]),
];

/// The groups shown at first.
const GROUPS_ON: [&str; 3] = ["frames", "cpu", "gpu"];

/// A group's items.
fn group_items(id: &str) -> &'static [(&'static str, bool)] {
    GROUPS.iter().find(|(group, _)| *group == id).map_or(&[], |(_, items)| items)
}

/// The groups as they are at first: in order, the first few on.
pub fn default_groups() -> Vec<ModuleEntry> {
    GROUPS.iter().map(|(id, _)| ModuleEntry { id: id.to_string(), on: GROUPS_ON.contains(id), items: Default::default() }).collect()
}

/// Every group once, in the order `groups` has them, those it does not
/// (new ones) after them, off; none it names that the overlay no longer has.
pub fn complete(groups: &mut Vec<ModuleEntry>) {
    groups.retain(|entry| GROUPS.iter().any(|(id, _)| *id == entry.id));
    let mut seen = std::collections::HashSet::new();
    groups.retain(|entry| seen.insert(entry.id.clone()));
    for (id, _) in GROUPS {
        if !groups.iter().any(|entry| entry.id == id) {
            groups.push(ModuleEntry { id: id.to_string(), on: false, items: Default::default() });
        }
    }
}

/// The groups that the items chosen before there were groups (up to 0.2.1,
/// a list of names; "network" both directions) make: a group on if any of
/// its items was chosen, each item as it was.
pub fn carried_over(items: &[String]) -> Vec<ModuleEntry> {
    let chosen = |name: &str| items.iter().any(|item| item == name || (item == "network" && (name == "down" || name == "up")));
    let mut groups = default_groups();
    for (id, names) in GROUPS {
        let on = names.iter().any(|(name, _)| chosen(name));
        if let Some(entry) = groups.iter_mut().find(|entry| entry.id == id) {
            entry.on = on;
        }
        if on {
            for (name, _) in names {
                set_item(&mut groups, id, name, chosen(name));
            }
        }
    }
    groups
}

/// Whether group `id` shows item `name` (while it is on): as chosen, or as
/// it is by default.
pub fn shows(groups: &[ModuleEntry], id: &str, name: &str) -> bool {
    let chosen = groups.iter().find(|entry| entry.id == id).and_then(|entry| entry.items.get(name).copied());
    chosen.unwrap_or_else(|| group_items(id).iter().any(|(item, on)| *item == name && *on))
}

/// Group `id` to show item `name`, or not; only what differs from the
/// default is kept.
pub fn set_item(groups: &mut [ModuleEntry], id: &str, name: &str, on: bool) {
    let default = group_items(id).iter().any(|(item, shown)| *item == name && *shown);
    if let Some(entry) = groups.iter_mut().find(|entry| entry.id == id) {
        if on == default {
            entry.items.remove(name);
        } else {
            entry.items.insert(name.to_string(), on);
        }
    }
}

/// The items `groups` show, group by group in their order: those chosen of
/// each group that is on.
pub fn chosen(groups: &[ModuleEntry]) -> Vec<&'static str> {
    groups
        .iter()
        .filter(|entry| entry.on)
        .flat_map(|entry| group_items(&entry.id).iter().map(|(name, _)| *name).filter(|name| shows(groups, &entry.id, name)))
        .collect()
}

/// The group item `name` is of.
fn group_of(name: &str) -> Option<&'static str> {
    GROUPS.iter().find(|(_, items)| items.iter().any(|(item, _)| *item == name)).map(|(id, _)| *id)
}

/// How far the glass keeps from its screen's edges (DIPs).
pub const INSET: f32 = 16.0;

/// The room around the glass its shadow falls in (DIPs).
pub const MARGIN: f32 = 12.0;

/// How the readings are laid out on the glass.
#[derive(Clone, Copy, Default, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    /// A card: the frame rate large, with its recent course beside it, and
    /// the rest in tiles two by two.
    #[default]
    Card,
    /// One row, for the top or bottom of the screen.
    Strip,
}

/// How the readings are laid out: as a card, or a strip in so many rows
/// (more where they would be wider than `width`, DIPs: their screen).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Shape {
    pub layout: Layout,
    pub rows: usize,
    pub width: f32,
}

/// The rows a strip can have.
pub const ROWS: [usize; 3] = [1, 2, 3];

/// The glass's tint: dark, for light words, or light, for dark ones.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tone {
    Dark,
    Light,
}

/// How the glass is tinted: in which tone, and how opaque the tint is (the
/// rest is the blurred screen behind).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Glass {
    pub tone: Tone,
    pub opacity: f32,
}

/// The colours of a tone.
struct Palette {
    /// The tint.
    tint: u32,
    /// Figures, and the words that name them.
    ink: Color,
    second: Color,
    /// A reading to heed (a muted microphone).
    hot: Color,
    /// A tile on the glass.
    tile: Color,
    /// The glass's rim: bright where light catches it at the top, fainter
    /// down its sides.
    rim: (Color, Color),
    /// Its shadows: a wide soft one and a tight one under its edge (colour,
    /// blur, drop).
    shadows: [(Color, f32, f32); 2],
    /// What each kind of reading is marked with.
    marks: [Color; 6],
}

const DARK: Palette = Palette {
    tint: 0x1E2229,
    ink: Color::hex(0xFFFFFF, 1.0),
    second: Color::hex(0xFFFFFF, 0.66),
    hot: Color::hex(0xFFB0A6, 1.0),
    tile: Color::hex(0xFFFFFF, 0.07),
    rim: (Color::hex(0xFFFFFF, 0.30), Color::hex(0xFFFFFF, 0.10)),
    shadows: [(Color::hex(0x000000, 0.30), 5.0, 3.0), (Color::hex(0x000000, 0.22), 1.0, 1.0)],
    marks: [
        Color::hex(0x8BE08F, 1.0),
        Color::hex(0x6FB6FF, 1.0),
        Color::hex(0xFFB35C, 1.0),
        Color::hex(0xC99BFF, 1.0),
        Color::hex(0x6FE0D2, 1.0),
        Color::hex(0xC8CDD5, 1.0),
    ],
};

const LIGHT: Palette = Palette {
    tint: 0xF4F6F9,
    ink: Color::hex(0x10141A, 1.0),
    second: Color::hex(0x10141A, 0.66),
    hot: Color::hex(0xB3261E, 1.0),
    tile: Color::hex(0x10141A, 0.05),
    rim: (Color::hex(0xFFFFFF, 0.90), Color::hex(0xFFFFFF, 0.40)),
    shadows: [(Color::hex(0x000000, 0.18), 5.0, 3.0), (Color::hex(0x000000, 0.12), 1.0, 1.0)],
    marks: [
        Color::hex(0x1F9A3A, 1.0),
        Color::hex(0x1668D6, 1.0),
        Color::hex(0xD36A00, 1.0),
        Color::hex(0x8240D8, 1.0),
        Color::hex(0x0F8577, 1.0),
        Color::hex(0x5A6270, 1.0),
    ],
};

impl Tone {
    fn palette(self) -> &'static Palette {
        match self {
            Tone::Dark => &DARK,
            Tone::Light => &LIGHT,
        }
    }
}

/// The glass while it is dragged: deep blue, whatever is behind.
const DRAGGED: Glass = Glass { tone: Tone::Dark, opacity: 0.9 };
const DRAGGED_TINT: u32 = 0x1D4F91;

/// The contrast every word on the glass keeps over what is behind (WCAG's
/// for ordinary text).
const LEGIBLE: f32 = 4.5;
/// The least tint the glass has, legible or not: enough to read as glass.
const LEAST: f32 = 0.3;
/// How much less tint the other tone has to need to be taken: the glass
/// does not flip between tones over a picture between the two.
const SWITCH: f32 = 0.1;
/// How much the tint may clear from one look at the screen to the next (it
/// darkens at once, for the words' sake).
const CLEARING: f32 = 0.1;
/// The share of what is behind taken as its brightest and its darkest: a
/// few stray pixels do not decide the tint.
const EXTREME: f32 = 0.1;

// The kinds of reading, by the mark each has.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Frames,
    Cpu,
    Gpu,
    Memory,
    Network,
    Mic,
}

impl Kind {
    fn mark(self, palette: &Palette) -> Color {
        palette.marks[self as usize]
    }
}

/// How a part of a reading can be written: as a whole (where it follows the
/// main figure), and as a figure and its unit (where it is the main one),
/// each in the widest forms it takes, so that the glass is as wide as those
/// and holds still as values change. The system font's digits are all as
/// wide: the widest form is the most digits a unit is shown with.
struct Form {
    whole: &'static [&'static str],
    split: &'static [(&'static str, &'static str)],
}

const PERCENT: Form = Form { whole: &["100%"], split: &[("100", "%")] };
const CELSIUS: Form = Form { whole: &["100 °C"], split: &[("100", "°C")] };
const WATTS: Form = Form { whole: &["999 W"], split: &[("999", "W")] };
const FRAMES_A_SECOND: Form = Form { whole: &["999"], split: &[("999", "")] };
const LOW: Form = Form { whole: &["1% 999"], split: &[("1% 999", "")] };
const FRAME_TIME: Form = Form { whole: &["999.9 ms"], split: &[("999.9", "ms")] };
const SIZE: Form = Form { whole: &["999.9 GB", "999 MB"], split: &[("999.9", "GB"), ("999", "MB")] };
const RATE: Form = Form {
    whole: &["999 B/s", "99.9 KB/s", "999 KB/s", "99.9 MB/s", "999 MB/s", "99.9 GB/s"],
    split: &[("999", "B/s"), ("99.9", "KB/s"), ("999", "KB/s"), ("99.9", "MB/s"), ("999", "MB/s"), ("99.9", "GB/s")],
};
const MIC_ZH: Form = Form { whole: &["已静音", "开启"], split: &[("已静音", ""), ("开启", "")] };
const MIC_EN: Form = Form { whole: &["Muted", "On"], split: &[("Muted", ""), ("On", "")] };
/// Between the parts that follow the main figure.
const SEPARATOR: &str = " · ";

/// A part of a reading as it reads now: whole, and as figure and unit.
struct Part {
    whole: String,
    figure: String,
    unit: String,
    form: &'static Form,
}

/// A part read as `whole`, its unit what follows its last figure.
fn part(whole: String, form: &'static Form) -> Part {
    let figures = whole.trim_end_matches(|c: char| !(c.is_ascii_digit() || c == '—')).len();
    let (figure, unit) = whole.split_at(figures);
    let (figure, unit) = if form.split.iter().all(|(_, unit)| unit.is_empty()) { (whole.as_str(), "") } else { (figure.trim_end(), unit.trim_start()) };
    Part { figure: figure.to_string(), unit: unit.to_string(), whole: whole.clone(), form }
}

/// One reading of the overlay: what it is, its name, its main figure and
/// unit, the rest of it after them, and whether it is to be heeded.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    kind: Kind,
    pub name: String,
    pub figure: String,
    pub unit: String,
    pub rest: String,
    pub hot: bool,
    /// The main figure is a word ("On"), not figures: set in the words' face.
    word: bool,
    /// The widest forms of the main figure and unit, and of each part of the rest.
    widest: (&'static [(&'static str, &'static str)], Vec<&'static [&'static str]>),
}

/// A reading of the parts that are read, the first the main one; none if
/// none is.
fn reading(kind: Kind, name: &str, parts: Vec<Option<Part>>) -> Option<Reading> {
    let mut parts = parts.into_iter().flatten();
    let first = parts.next()?;
    let rest: Vec<Part> = parts.collect();
    Some(Reading {
        kind,
        name: name.to_string(),
        figure: first.figure,
        unit: first.unit,
        rest: rest.iter().map(|part| part.whole.as_str()).collect::<Vec<_>>().join(SEPARATOR),
        hot: false,
        word: false,
        widest: (first.form.split, rest.iter().map(|part| part.form.whole).collect()),
    })
}

/// What the overlay shows of sample `s`, reading by reading: the `items`
/// chosen that are read, a group's together, the groups in the order the
/// items have them. A game's readings are `game`'s while one is `playing`,
/// "—" otherwise or while it presents no frames for a moment (it lost the
/// front to a menu, it loads), so that they hold their place.
pub fn readings(s: &Sample, game: Option<&GameSample>, playing: bool, items: &[&str], lang: Lang) -> Vec<Reading> {
    let on = |name: &str| items.contains(&name);
    let celsius = |t: f32| format!("{t:.0} °C");
    let watts = |w: f32| format!("{w:.0} W");
    // The GPU the game uses, or the busiest.
    let gpu = game
        .and_then(|game| game.gpu_index)
        .and_then(|i| s.gpus.get(i))
        .or_else(|| s.gpus.iter().max_by(|a, b| a.usage.unwrap_or(0.0).total_cmp(&b.usage.unwrap_or(0.0))));
    let cpu_sensors = s.cpu_sensors.as_ref();
    let rate = |rate: Option<f64>| rate.map(|rate| text::rate(rate, false));
    let chosen = |name: &str, value: Option<String>, form: &'static Form| on(name).then_some(value).flatten().map(|value| part(value, form));
    let game = game.filter(|_| playing);
    let group = |id: &str| -> Vec<Option<Reading>> {
        match id {
            "frames" => vec![reading(
                Kind::Frames,
                lang.pick("帧率", "FPS"),
                vec![
                    chosen("fps", Some(game.map_or("—".into(), |game| format!("{:.0}", game.fps))), &FRAMES_A_SECOND),
                    // No 1% low yet is no part, as for any reading not yet read.
                    chosen("low", game.map_or(Some("1% —".into()), |game| game.low.map(|low| format!("1% {low:.0}"))), &LOW),
                    chosen("frametime", Some(game.map_or("— ms".into(), |game| format!("{:.1} ms", game.longest_ms))), &FRAME_TIME),
                ],
            )],
            "cpu" => vec![reading(
                Kind::Cpu,
                "CPU",
                vec![
                    chosen("cpu", s.cpu.map(text::percent), &PERCENT),
                    chosen("cpu_temp", cpu_sensors.and_then(|c| c.temp).map(celsius), &CELSIUS),
                    chosen("cpu_power", cpu_sensors.and_then(|c| c.power).map(watts), &WATTS),
                ],
            )],
            "gpu" => vec![reading(
                Kind::Gpu,
                "GPU",
                vec![
                    chosen("gpu", gpu.and_then(|g| g.usage).map(text::percent), &PERCENT),
                    chosen("gpu_temp", gpu.and_then(|g| g.temp).map(celsius), &CELSIUS),
                    chosen("gpu_power", gpu.and_then(|g| g.power).map(watts), &WATTS),
                ],
            )],
            "memory" => vec![reading(Kind::Memory, lang.pick("内存", "RAM"), vec![chosen("memory", Some(text::size(s.memory.used)), &SIZE)])],
            "vram" => vec![reading(Kind::Memory, lang.pick("显存", "VRAM"), vec![chosen("vram", gpu.and_then(|g| g.mem_used).map(text::size), &SIZE)])],
            // Each way a reading of its own: neither is the other's detail.
            "network" => vec![
                reading(Kind::Network, lang.pick("下载", "Down"), vec![chosen("down", rate(s.net_down), &RATE)]),
                reading(Kind::Network, lang.pick("上传", "Up"), vec![chosen("up", rate(s.net_up), &RATE)]),
            ],
            _ => vec![s.mic_muted.filter(|_| on("mic")).and_then(|muted| {
                let value = if muted { lang.pick("已静音", "Muted") } else { lang.pick("开启", "On") };
                let form = if lang == Lang::Zh { &MIC_ZH } else { &MIC_EN };
                reading(Kind::Mic, lang.pick("麦克风", "Mic"), vec![Some(part(value.to_string(), form))]).map(|r| Reading { hot: muted, word: true, ..r })
            })],
        }
    };
    let mut groups: Vec<&str> = Vec::new();
    for id in items.iter().filter_map(|item| group_of(item)) {
        if !groups.contains(&id) {
            groups.push(id);
        }
    }
    groups.into_iter().flat_map(group).flatten().collect()
}

/// The frame rates of the game played, from `history` (oldest first), for
/// the card's chart: as many of the latest samples as it holds, `None` where
/// no game presented frames.
pub fn frames<'a>(history: impl DoubleEndedIterator<Item = &'a Sample>, playing: bool) -> Vec<Option<f32>> {
    if !playing {
        return Vec::new();
    }
    let mut frames: Vec<Option<f32>> = history.rev().take(CHART).map(|s| s.game.as_ref().filter(|game| !game.by_hand).map(|game| game.fps)).collect();
    frames.reverse();
    frames
}

/// What the overlay would show of sample `s` over a game: its game's
/// readings, or with none running, an example game's, for the settings'
/// preview.
pub fn preview(s: &Sample, items: &[&str], lang: Lang) -> Vec<Reading> {
    let example = GameSample {
        name: String::new(),
        program: String::new(),
        pid: 0,
        fps: 144.0,
        low: Some(118.0),
        longest_ms: 8.4,
        refresh_hz: Some(144),
        screen: None,
        gpu_index: None,
        cpu: None,
        gpu: None,
        mem: None,
        vram: None,
        gpu_limit: None,
        playing_s: 0,
        by_hand: false,
    };
    readings(s, Some(s.game.as_ref().unwrap_or(&example)), true, items, lang)
}

/// An example of the frame rate's course, for the preview.
pub fn preview_frames() -> Vec<Option<f32>> {
    (0..CHART).map(|i| Some(144.0 - 6.0 * ((i as f32) * 0.7).sin().abs() - if i % 17 == 9 { 20.0 } else { 0.0 })).collect()
}

/// How many samples of the frame rate the card charts.
const CHART: usize = 60;

// The card's and the strip's measures (DIPs).
const PAD: f32 = 14.0;
const GAP: f32 = 8.0;
const CARD_RADIUS: f32 = 18.0;
const TILE_PAD: (f32, f32) = (11.0, 8.0);
const TILE_RADIUS: f32 = 10.0;
/// The least a tile is wide, and the frame rate's chart.
const TILE_LEAST: f32 = 84.0;
const CHART_LEAST: f32 = 72.0;
const STRIP_PAD: (f32, f32) = (16.0, 10.0);
/// Between a strip's rows, and its corners when it has more than one.
const STRIP_LEADING: f32 = 6.0;
const STRIP_RADIUS: f32 = 14.0;
/// Between a strip's readings, either side of the rule between them.
const STRIP_GAP: f32 = 12.0;
/// A mark's diameter and the room after it.
const MARK: f32 = 6.0;
const MARK_GAP: f32 = 5.0;
/// Between a figure and its unit, and the figure and the rest.
const UNIT_GAP: f32 = 3.0;
const REST_GAP: f32 = 7.0;
/// Between the lines of a tile (name, figure, rest).
const LEADING: f32 = 2.0;

const NAME: Font = Font::new(Family::Segoe, 12.0, 500.0);
const UNIT: Font = Font::new(Family::Segoe, 12.0, 500.0);
const REST: Font = Font::new(Family::Segoe, 12.0, 400.0);
const HERO: Font = Font::new(Family::SegoeDisplay, 40.0, 600.0);
const FIGURE: Font = Font::new(Family::SegoeDisplay, 22.0, 600.0);
const STRIP_FIGURE: Font = Font::new(Family::SegoeDisplay, 18.0, 600.0);
/// A word in place of figures (a Chinese face has no semibold; its bold is
/// the figures' weight).
const HERO_WORD: Font = Font::new(Family::Segoe, 28.0, 700.0);
const WORD: Font = Font::new(Family::Segoe, 17.0, 700.0);
const STRIP_WORD: Font = Font::new(Family::Segoe, 15.0, 700.0);

/// What the overlay measures text with: widths, and a face's line box
/// (ascent and descent).
pub struct Metrics<'a> {
    pub width: &'a dyn Fn(&str, Font) -> f32,
    pub line: &'a dyn Fn(Font) -> (f32, f32),
}

/// The colour a piece of text takes.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Ink {
    Main,
    Second,
    Hot,
}

/// What the glass carries, placed (DIPs from the glass's corner).
enum Piece {
    Tile { x: f32, y: f32, width: f32, height: f32 },
    Mark { kind: Kind, x: f32, y: f32 },
    /// Text by its baseline.
    Text { text: String, font: Font, ink: Ink, x: f32, baseline: f32 },
    Rule { x: f32, y: f32, height: f32 },
    Chart { x: f32, y: f32, width: f32, height: f32 },
}

/// The glass's size and corners, and what is on it.
struct Arranged {
    size: (f32, f32),
    radius: f32,
    pieces: Vec<Piece>,
}

/// The fonts a reading is set in at a size: main figure, unit, rest.
fn faces(reading: &Reading, figure: Font, word: Font) -> (Font, Font, Font) {
    (if reading.word { word } else { figure }, UNIT, REST)
}

/// How wide a reading's main figure and unit can be, and its rest.
fn widest(reading: &Reading, figure: Font, m: &Metrics) -> (f32, f32) {
    let (figure, unit, rest) = faces(reading, figure, figure);
    let main = |f: &str, u: &str| (m.width)(f, figure) + if u.is_empty() { 0.0 } else { UNIT_GAP + (m.width)(u, unit) };
    let now = main(&reading.figure, &reading.unit);
    let main = reading.widest.0.iter().map(|(f, u)| main(f, u)).fold(now, f32::max);
    let parts: f32 = reading.widest.1.iter().map(|forms| forms.iter().map(|form| (m.width)(form, rest)).fold(0.0, f32::max)).sum();
    let rest = (parts + reading.widest.1.len().saturating_sub(1) as f32 * (m.width)(SEPARATOR, rest)).max((m.width)(&reading.rest, rest));
    (main, rest)
}

/// Lays `readings` out as `shape` has them, at `px` physical pixels a DIP:
/// every line on a whole pixel.
fn arrange(readings: &[Reading], shape: Shape, m: &Metrics, px: f32) -> Arranged {
    let Shape { layout, rows, width } = shape;
    let snap = |v: f32| (v * px).round() / px;
    let up = |v: f32| (v * px).ceil() / px;
    let height = |font: Font| {
        let (ascent, descent) = (m.line)(font);
        (ascent, ascent + descent)
    };
    match layout {
        Layout::Card => {
            let mut pieces = Vec::new();
            let (hero, tiles): (Vec<&Reading>, Vec<&Reading>) = readings.iter().partition(|r| r.kind == Kind::Frames);
            let hero = hero.first().copied();
            // The tiles' width: the widest of their contents.
            let name_width = |r: &Reading| MARK + MARK_GAP + (m.width)(&r.name, NAME);
            let tile_inner = tiles
                .iter()
                .map(|r| {
                    let (main, rest) = widest(r, if r.word { WORD } else { FIGURE }, m);
                    name_width(r).max(main).max(rest)
                })
                .fold(0.0, f32::max);
            let tile_width = up((tile_inner + 2.0 * TILE_PAD.0).max(TILE_LEAST));
            let columns = tiles.len().min(2);
            let hero_inner = hero.map(|r| {
                let (main, rest) = widest(r, if r.word { HERO_WORD } else { HERO }, m);
                up(name_width(r).max(main).max(rest))
            });
            let inner = (columns as f32 * tile_width + columns.saturating_sub(1) as f32 * GAP).max(hero_inner.map_or(0.0, |w| w + GAP + CHART_LEAST));
            let tile_width = if columns == 2 { snap((inner - GAP) / 2.0) } else { inner };
            let (name_ascent, name_height) = height(NAME);
            let (rest_ascent, rest_height) = height(REST);
            let mut y = PAD;
            if let Some(r) = hero {
                let figure = if r.word { HERO_WORD } else { HERO };
                let (figure_ascent, figure_height) = height(figure);
                let name_base = snap(y + name_ascent);
                pieces.push(Piece::Mark { kind: r.kind, x: PAD, y: name_base - name_ascent / 2.0 - 1.0 });
                pieces.push(Piece::Text { text: r.name.clone(), font: NAME, ink: Ink::Second, x: PAD + MARK + MARK_GAP, baseline: name_base });
                let figure_base = snap(y + name_height + LEADING + figure_ascent);
                let figure_width = (m.width)(&r.figure, figure);
                pieces.push(Piece::Text { text: r.figure.clone(), font: figure, ink: if r.hot { Ink::Hot } else { Ink::Main }, x: PAD, baseline: figure_base });
                if !r.unit.is_empty() {
                    pieces.push(Piece::Text { text: r.unit.clone(), font: UNIT, ink: Ink::Second, x: snap(PAD + figure_width + UNIT_GAP), baseline: figure_base });
                }
                let mut bottom = y + name_height + LEADING + figure_height;
                if !r.rest.is_empty() {
                    let rest_base = snap(bottom + LEADING + rest_ascent);
                    pieces.push(Piece::Text { text: r.rest.clone(), font: REST, ink: Ink::Second, x: PAD, baseline: rest_base });
                    bottom += LEADING + rest_height;
                }
                let chart_x = PAD + hero_inner.unwrap() + GAP;
                pieces.push(Piece::Chart { x: chart_x, y: snap(y + name_height / 2.0), width: PAD + inner - chart_x, height: snap(bottom - y - name_height / 2.0) });
                y = up(bottom);
                if !tiles.is_empty() {
                    y += GAP + 4.0;
                }
            }
            if !tiles.is_empty() {
                let (figure_ascent, figure_height) = height(FIGURE);
                let rests = tiles.iter().any(|r| !r.rest.is_empty());
                let tile_height = up(2.0 * TILE_PAD.1 + name_height + LEADING + figure_height + if rests { LEADING + rest_height } else { 0.0 });
                for (i, r) in tiles.iter().enumerate() {
                    let (row, column) = (i / 2, i % 2);
                    // An odd last tile spans the row.
                    let alone = columns == 2 && i == tiles.len() - 1 && column == 0;
                    let x = PAD + column as f32 * (tile_width + GAP);
                    let top = y + row as f32 * (tile_height + GAP);
                    let width = if alone { inner } else { tile_width };
                    pieces.push(Piece::Tile { x, y: top, width, height: tile_height });
                    let (cx, cy) = (x + TILE_PAD.0, top + TILE_PAD.1);
                    let name_base = snap(cy + name_ascent);
                    pieces.push(Piece::Mark { kind: r.kind, x: cx, y: name_base - name_ascent / 2.0 - 1.0 });
                    pieces.push(Piece::Text { text: r.name.clone(), font: NAME, ink: Ink::Second, x: cx + MARK + MARK_GAP, baseline: name_base });
                    // A word sits on the figures' baseline.
                    let figure = if r.word { WORD } else { FIGURE };
                    let figure_base = snap(cy + name_height + LEADING + figure_ascent);
                    pieces.push(Piece::Text { text: r.figure.clone(), font: figure, ink: if r.hot { Ink::Hot } else { Ink::Main }, x: cx, baseline: figure_base });
                    if !r.unit.is_empty() {
                        let after = (m.width)(&r.figure, figure);
                        pieces.push(Piece::Text { text: r.unit.clone(), font: UNIT, ink: Ink::Second, x: snap(cx + after + UNIT_GAP), baseline: figure_base });
                    }
                    if !r.rest.is_empty() {
                        let rest_base = snap(cy + name_height + LEADING + figure_height + LEADING + rest_ascent);
                        pieces.push(Piece::Text { text: r.rest.clone(), font: REST, ink: Ink::Second, x: cx, baseline: rest_base });
                    }
                }
                let rows = tiles.len().div_ceil(2);
                y += rows as f32 * tile_height + (rows - 1) as f32 * GAP;
            }
            Arranged { size: (up(2.0 * PAD + inner), up(y + PAD)), radius: CARD_RADIUS, pieces }
        }
        // In as many rows as asked, or more where so many do not fit.
        Layout::Strip => (rows.max(1)..=readings.len().max(1))
            .map(|rows| strip(readings, rows, m, px))
            .find(|strip| strip.size.0 <= width)
            .unwrap_or_else(|| strip(readings, readings.len().max(1), m, px)),
    }
}

/// Lays `readings` out as a strip in `rows` rows (fewer if there are fewer
/// readings), filled in turn, each as long as it needs (the last may be
/// shorter); a column's readings' parts aligned.
fn strip(readings: &[Reading], rows: usize, m: &Metrics, px: f32) -> Arranged {
    let snap = |v: f32| (v * px).round() / px;
    let up = |v: f32| (v * px).ceil() / px;
    let height = |font: Font| {
        let (ascent, descent) = (m.line)(font);
        (ascent, ascent + descent)
    };
    let rows = rows.clamp(1, readings.len().max(1));
    let across = readings.len().div_ceil(rows).max(1);
    let rows = readings.len().div_ceil(across).max(1);
    let (figure_ascent, figure_height) = height(STRIP_FIGURE);
    let (name_ascent, _) = height(NAME);
    let pitch = up(figure_height + STRIP_LEADING);
    let glass_height = up(2.0 * STRIP_PAD.1 + figure_height + (rows - 1) as f32 * pitch);
    // Each column's widest name, figure and rest.
    let mut columns = vec![(0.0f32, 0.0f32, 0.0f32); across];
    for (i, r) in readings.iter().enumerate() {
        let (main, rest) = widest(r, if r.word { STRIP_WORD } else { STRIP_FIGURE }, m);
        let column = &mut columns[i % across];
        *column = (column.0.max((m.width)(&r.name, NAME)), column.1.max(main), column.2.max(if r.rest.is_empty() { 0.0 } else { rest }));
    }
    let mut pieces = Vec::new();
    let mut x = STRIP_PAD.0;
    for (c, &(name, main, rest)) in columns.iter().enumerate() {
        if c > 0 {
            x += STRIP_GAP;
            let inset = up(STRIP_PAD.1 + figure_height * 0.2);
            pieces.push(Piece::Rule { x: snap(x), y: inset, height: glass_height - 2.0 * inset });
            x += STRIP_GAP;
        }
        for (row, r) in readings.iter().skip(c).step_by(across).enumerate() {
            let baseline = snap(STRIP_PAD.1 + row as f32 * pitch + figure_ascent);
            pieces.push(Piece::Mark { kind: r.kind, x, y: baseline - name_ascent / 2.0 - 1.0 });
            pieces.push(Piece::Text { text: r.name.clone(), font: NAME, ink: Ink::Second, x: snap(x + MARK + MARK_GAP), baseline });
            let at = x + MARK + MARK_GAP + name + REST_GAP;
            let figure = if r.word { STRIP_WORD } else { STRIP_FIGURE };
            pieces.push(Piece::Text { text: r.figure.clone(), font: figure, ink: if r.hot { Ink::Hot } else { Ink::Main }, x: snap(at), baseline });
            if !r.unit.is_empty() {
                let after = (m.width)(&r.figure, figure);
                pieces.push(Piece::Text { text: r.unit.clone(), font: UNIT, ink: Ink::Second, x: snap(at + after + UNIT_GAP), baseline });
            }
            if !r.rest.is_empty() {
                pieces.push(Piece::Text { text: r.rest.clone(), font: REST, ink: Ink::Second, x: snap(at + main + REST_GAP), baseline });
            }
        }
        x += MARK + MARK_GAP + name + REST_GAP + main + if rest > 0.0 { REST_GAP + rest } else { 0.0 };
    }
    // One row is a pill; more, a card's corners.
    let radius = if rows == 1 { glass_height / 2.0 } else { STRIP_RADIUS };
    Arranged { size: (up(x + STRIP_PAD.0), glass_height), radius, pieces }
}

/// How large the glass is for `readings` laid out as `shape` has them
/// (DIPs), at `px` physical pixels a DIP: as wide as their values can be,
/// not as they are now. The overlay's window is `MARGIN` larger every way.
pub fn size(readings: &[Reading], shape: Shape, px: f32, m: &Metrics) -> (f32, f32) {
    arrange(readings, shape, m, px).size
}

/// The glass's corner radius (DIPs).
pub fn radius(readings: &[Reading], shape: Shape, px: f32, m: &Metrics) -> f32 {
    arrange(readings, shape, m, px).radius
}

/// Draws the shadow the glass for `readings` casts, the glass's corner at
/// the canvas's: outside the glass only, `MARGIN` around it at most.
pub fn shadow(frame: &dyn Canvas, readings: &[Reading], shape: Shape, glass: Glass, dragged: bool, px: f32) {
    let m = Metrics { width: &|text, font| frame.measure(text, font), line: &|font| frame.baseline(font) };
    let arranged = arrange(readings, shape, &m, px);
    let palette = if dragged { DRAGGED.tone.palette() } else { glass.tone.palette() };
    let (width, height) = arranged.size;
    for (color, blur, drop) in palette.shadows {
        frame.shadow(color, 0.0, 0.0, width, height, arranged.radius, blur, drop);
    }
}

/// How the overlay is in hand: not; taken (letting clicks through, it
/// takes them now); dragged.
#[derive(Clone, Copy, PartialEq)]
pub enum Hand {
    Free,
    Taken,
    Dragged,
}

/// Draws the glass for `readings` from the canvas's corner: its tint (over
/// the screen behind, blurred, which the window draws), rim, and the
/// readings on it, at `px` physical pixels a DIP; dragged, in the colour
/// that says it is being moved; taken, outlined (as a widget says it). Its
/// shadow is drawn apart (see `shadow`).
pub fn paint(frame: &dyn Canvas, readings: &[Reading], frames: &[Option<f32>], shape: Shape, glass: Glass, hand: Hand, px: f32) {
    let (dragged, taken) = (hand == Hand::Dragged, hand == Hand::Taken);
    let m = Metrics { width: &|text, font| frame.measure(text, font), line: &|font| frame.baseline(font) };
    let arranged = arrange(readings, shape, &m, px);
    let (glass, tint) = if dragged { (DRAGGED, DRAGGED_TINT) } else { (glass, glass.tone.palette().tint) };
    let palette = glass.tone.palette();
    let (width, height) = arranged.size;
    let (x0, y0, radius) = (0.0, 0.0, arranged.radius);
    frame.fill_rounded(Color::hex(tint, glass.opacity), x0, y0, width, height, radius);
    if taken {
        frame.stroke_rounded(Fill::Solid(palette.second), x0 + 0.75, y0 + 0.75, width - 1.5, height - 1.5, radius - 0.75, 1.5);
    }
    let ink = |ink: Ink| match ink {
        Ink::Main => palette.ink,
        Ink::Second => palette.second,
        Ink::Hot => palette.hot,
    };
    for piece in &arranged.pieces {
        match *piece {
            Piece::Tile { x, y, width, height } => frame.fill_rounded(palette.tile, x0 + x, y0 + y, width, height, TILE_RADIUS),
            Piece::Mark { kind, x, y } => frame.fill_circle(kind.mark(palette), Point { x: x0 + x + MARK / 2.0, y: y0 + y }, MARK / 2.0),
            Piece::Text { ref text, font, ink: color, x, baseline } => {
                let (ascent, _) = frame.baseline(font);
                frame.text(text, font, ink(color), x0 + x, y0 + baseline - ascent, frame.measure(text, font) + 1.0, Align::Start);
            }
            Piece::Rule { x, y, height } => frame.fill(palette.rim.1, x0 + x, y0 + y, 1.0 / px, height),
            Piece::Chart { x, y, width, height } => chart(frame, frames, Kind::Frames.mark(palette), x0 + x, y0 + y, width, height, px),
        }
    }
    // The rim: one pixel inside the glass's edge, lit from above.
    let line = 1.0 / px;
    let rim = Fill::Down { top: y0, from: palette.rim.0, bottom: y0 + height, to: palette.rim.1 };
    frame.stroke_rounded(rim, x0 + line / 2.0, y0 + line / 2.0, width - line, height - line, radius - line / 2.0, line);
}

/// The frame rate's recent course, in `color`, filling the box at (`x`,
/// `y`): its range from a little under its least to its most, the latest on
/// the right; gaps where no frames came.
#[allow(clippy::too_many_arguments)]
fn chart(frame: &dyn Canvas, frames: &[Option<f32>], color: Color, x: f32, y: f32, width: f32, height: f32, px: f32) {
    let read: Vec<f32> = frames.iter().flatten().copied().collect();
    if read.len() < 2 {
        return;
    }
    let most = read.iter().copied().fold(f32::MIN, f32::max);
    let least = read.iter().copied().fold(f32::MAX, f32::min);
    // At least a tenth of the most as range: a steady rate draws near flat.
    let span = (most - least).max(most * 0.1).max(1.0);
    let low = most - span * 1.1;
    let step = width / (CHART - 1) as f32;
    let start = CHART.saturating_sub(frames.len());
    let line = 1.5;
    let at = |i: usize, fps: f32| Point { x: x + (start + i) as f32 * step, y: y + line + (1.0 - (fps - low) / (most - low)) * (height - 2.0 * line) };
    // Each unbroken run of samples, drawn alone.
    let mut run: Vec<Point> = Vec::new();
    let flush = |run: &mut Vec<Point>| {
        if run.len() >= 2 {
            let mut area = run.clone();
            area.push(Point { x: run.last().unwrap().x, y: y + height });
            area.push(Point { x: run[0].x, y: y + height });
            frame.fill_shape(&area, Fill::Down { top: y, from: color.alpha(0.28), bottom: y + height, to: color.alpha(0.0) });
            frame.stroke(run, color, line);
        }
        run.clear();
    };
    for (i, fps) in frames.iter().enumerate() {
        match fps {
            Some(fps) => run.push(at(i, *fps)),
            None => flush(&mut run),
        }
    }
    let last = run.last().copied();
    flush(&mut run);
    if let Some(point) = last {
        frame.fill_circle(color, point, (2.5 * px).round() / px);
    }
}

/// Relative luminance (0 black, 1 white) of an sRGB colour channel set,
/// as WCAG measures it.
pub fn luminance(r: f32, g: f32, b: f32) -> f32 {
    let linear = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// The sRGB grey of relative luminance `l`.
fn grey(l: f32) -> f32 {
    if l <= 0.0031308 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    }
}

/// The contrast of text in `color` on `tone`'s glass tinted to `opacity`,
/// on a tile or not, over a grey of luminance `behind`. Blended as the
/// desktop blends them, in sRGB values; compared as WCAG compares.
fn contrast(color: Color, tone: Tone, opacity: f32, tile: bool, behind: f32) -> f32 {
    let palette = tone.palette();
    let tint = Color::hex(palette.tint, 1.0);
    let over = |top: f32, alpha: f32, under: f32| alpha * top + (1.0 - alpha) * under;
    let back = grey(behind);
    let mut ground = (over(tint.r, opacity, back), over(tint.g, opacity, back), over(tint.b, opacity, back));
    if tile {
        let t = palette.tile;
        ground = (over(t.r, t.a, ground.0), over(t.g, t.a, ground.1), over(t.b, t.a, ground.2));
    }
    let text = (over(color.r, color.a, ground.0), over(color.g, color.a, ground.1), over(color.b, color.a, ground.2));
    let (a, b) = (luminance(ground.0, ground.1, ground.2), luminance(text.0, text.1, text.2));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// The least tint `tone`'s glass needs for every colour of words on it to
/// keep WCAG's contrast over a grey of luminance `behind`; wholly opaque if
/// even that is not enough.
fn least_tint(tone: Tone, behind: f32) -> f32 {
    let palette = tone.palette();
    (0..=100)
        .map(|i| i as f32 / 100.0)
        .find(|&opacity| {
            [palette.ink, palette.second, palette.hot].iter().all(|&color| [false, true].iter().all(|&tile| contrast(color, tone, opacity, tile, behind) >= LEGIBLE))
        })
        .unwrap_or(1.0)
        .max(LEAST)
}

/// How the glass is tinted over a screen of `behind`'s luminances (0–1,
/// any order; none: not seen, taken as anything from black to white),
/// following `before`, how it was: in the tone that needs less tint to keep
/// its words legible over the brightest of what is behind (dark glass) or
/// the darkest (light glass), switching only for a clear gain; as little
/// tint as that, clearing no faster than `CLEARING` a look.
pub fn glass(behind: &mut [f32], before: Option<Glass>) -> Glass {
    let (darkest, brightest) = if behind.is_empty() {
        (0.0, 1.0)
    } else {
        behind.sort_by(f32::total_cmp);
        let at = |share: f32| behind[((behind.len() - 1) as f32 * share).round() as usize];
        (at(EXTREME), at(1.0 - EXTREME))
    };
    let dark = least_tint(Tone::Dark, brightest);
    let light = least_tint(Tone::Light, darkest);
    let tone = match before.map(|glass| glass.tone) {
        Some(Tone::Dark) if light + SWITCH >= dark => Tone::Dark,
        Some(Tone::Light) if dark + SWITCH >= light => Tone::Light,
        _ if dark <= light => Tone::Dark,
        _ => Tone::Light,
    };
    let needed = if tone == Tone::Dark { dark } else { light };
    let opacity = match before {
        Some(before) if before.tone == tone => needed.max(before.opacity - CLEARING),
        _ => needed,
    };
    Glass { tone, opacity }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reading::{MemorySample, SystemSample};

    #[test]
    fn as_little_tint_as_keeps_the_words_legible() {
        for tone in [Tone::Dark, Tone::Light] {
            for behind in [0.0, 0.05, 0.2, 0.5, 1.0] {
                let opacity = least_tint(tone, behind);
                let palette = tone.palette();
                let holds = |opacity: f32| {
                    [palette.ink, palette.second, palette.hot].iter().all(|&c| [false, true].iter().all(|&tile| contrast(c, tone, opacity, tile, behind) >= LEGIBLE))
                };
                // Every colour holds its contrast at that tint (or the tint
                // is wholly opaque); a step clearer, one does not, unless
                // the least glass is already more.
                assert!(holds(opacity) || opacity == 1.0, "{tone:?} {behind} {opacity}");
                assert!(opacity == LEAST || !holds(opacity - 0.01), "{tone:?} {behind} {opacity}");
            }
        }
        // Dark glass over a dark game is clear; light glass over a white page.
        assert_eq!(least_tint(Tone::Dark, 0.02), LEAST);
        assert_eq!(least_tint(Tone::Light, 1.0), LEAST);
    }

    #[test]
    fn the_tone_follows_what_is_behind_without_flipping() {
        let dark_scene = || vec![0.02; 100];
        let white_page = || vec![1.0; 100];
        let first = glass(&mut dark_scene(), None);
        assert_eq!(first.tone, Tone::Dark);
        let page = glass(&mut white_page(), Some(first));
        assert_eq!(page.tone, Tone::Light);
        // A few bright pixels do not decide it.
        let mut specks = dark_scene();
        specks[..5].fill(1.0);
        assert_eq!(glass(&mut specks, Some(first)), first);
        // Clearing slowly, darkening at once.
        let dense = Glass { tone: Tone::Dark, opacity: 0.9 };
        assert!((glass(&mut dark_scene(), Some(dense)).opacity - (0.9 - CLEARING)).abs() < 1e-6);
        let thin = Glass { tone: Tone::Dark, opacity: LEAST };
        assert_eq!(glass(&mut vec![0.2; 100], Some(thin)).opacity, least_tint(Tone::Dark, 0.2));
        // Unseen, the tint is enough for anything.
        let unseen = glass(&mut [], None);
        assert!(unseen.opacity >= least_tint(unseen.tone, if unseen.tone == Tone::Dark { 1.0 } else { 0.0 }));
    }

    fn sample(game: bool) -> Sample {
        Sample {
            t: 0,
            cpu: Some(40.0),
            threads: Vec::new(),
            ghz: None,
            kinds_ghz: Vec::new(),
            threads_ghz: Vec::new(),
            memory: MemorySample { used: 8 << 30, committed: 0, commit_limit: 0, cached: 0 },
            gpus: Vec::new(),
            net_down: None,
            net_up: None,
            net_total_down: 0,
            net_total_up: 0,
            network: None,
            disk_read: None,
            disk_write: None,
            disk_active: None,
            volumes: Vec::new(),
            processes: Vec::new(),
            system: SystemSample { uptime_s: 0, processes: 0, threads: 0, handles: 0 },
            battery: None,
            cpu_sensors: None,
            board: None,
            drive_temps: Vec::new(),
            dimm_temps: Vec::new(),
            mic_muted: Some(true),
            game: game.then(|| GameSample {
                name: "Game".into(),
                program: "game".into(),
                pid: 1,
                fps: 143.6,
                low: None,
                longest_ms: 9.26,
                refresh_hz: Some(144),
                screen: None,
                gpu_index: None,
                cpu: None,
                gpu: None,
                mem: None,
                vram: None,
                gpu_limit: None,
                playing_s: 0,
                by_hand: false,
            }),
            wsl: None,
            docker: None,
        }
    }

    fn items<'a>(names: &[&'a str]) -> Vec<&'a str> {
        names.to_vec()
    }

    fn plain(readings: Vec<Reading>) -> Vec<(String, String, String, String)> {
        readings.into_iter().map(|r| (r.name, r.figure, r.unit, r.rest)).collect()
    }

    #[test]
    fn shows_the_items_chosen_that_are_read() {
        let chosen = items(&["fps", "low", "frametime", "cpu", "cpu_temp", "gpu", "memory"]);
        let s = sample(true);
        let s4 = |a: &str, b: &str, c: &str, d: &str| (a.to_string(), b.to_string(), c.to_string(), d.to_string());
        // Unread readings left out, not shown as gaps: no 1% low yet, no
        // CPU heat, no GPU.
        assert_eq!(
            plain(readings(&s, s.game.as_ref(), true, &chosen, Lang::Zh)),
            [s4("帧率", "144", "", "9.3 ms"), s4("CPU", "40", "%", ""), s4("内存", "8.0", "GB", "")]
        );
        // Without a game, the frame rate is a dash, in its place.
        let s = sample(false);
        assert_eq!(
            plain(readings(&s, None, false, &items(&["fps", "low", "cpu"]), Lang::En)),
            [s4("FPS", "—", "", "1% —"), s4("CPU", "40", "%", "")]
        );
        // A muted microphone, to be heeded.
        let mic = readings(&s, None, false, &items(&["mic"]), Lang::En);
        assert_eq!((mic[0].figure.as_str(), mic[0].hot), ("Muted", true));
        // Network: each way a reading of its own.
        let mut s = sample(false);
        s.net_down = Some(2048.0);
        s.net_up = Some(10.0);
        let net = readings(&s, None, false, &items(&["down", "up"]), Lang::En);
        assert_eq!(plain(net), [s4("Down", "2.0", "KB/s", ""), s4("Up", "10", "B/s", "")]);
        // In the order of the items' groups.
        let moved = readings(&s, None, false, &items(&["memory", "cpu"]), Lang::En);
        assert_eq!(moved.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["RAM", "CPU"]);
    }

    #[test]
    fn the_glass_holds_still_as_values_change() {
        // A character a unit wide; lines 10 units tall.
        let width = |text: &str, _: Font| text.chars().count() as f32;
        let line = |_: Font| (8.0, 2.0);
        let m = Metrics { width: &width, line: &line };
        let mut s = sample(false);
        let few = readings(&s, None, false, &items(&["cpu", "memory"]), Lang::En);
        s.cpu = Some(100.0);
        s.memory.used = 999 << 30;
        let many = readings(&s, None, false, &items(&["cpu", "memory"]), Lang::En);
        let card = Shape { layout: Layout::Card, rows: 1, width: f32::INFINITY };
        let strip = |rows| Shape { layout: Layout::Strip, rows, width: f32::INFINITY };
        for shape in [card, strip(1), strip(2)] {
            assert_eq!(size(&few, shape, 1.0, &m), size(&many, shape, 1.0, &m));
            // Whole pixels at 150%.
            let (w, h) = size(&few, shape, 1.5, &m);
            assert_eq!(((w * 1.5).fract(), (h * 1.5).fract()), (0.0, 0.0));
        }
        // The strip is one line; the card stacks.
        assert!(size(&few, strip(1), 1.0, &m).1 < size(&few, card, 1.0, &m).1);
        // In two rows, a strip is narrower and taller; never more rows than
        // readings.
        let (one, two) = (size(&few, strip(1), 1.0, &m), size(&few, strip(2), 1.0, &m));
        assert!(two.0 < one.0 && two.1 > one.1);
        assert_eq!(size(&few, strip(3), 1.0, &m), two);
        // Too wide for its screen in one row, in two.
        assert_eq!(size(&few, Shape { width: one.0 - 1.0, ..strip(1) }, 1.0, &m), two);
    }

    #[test]
    fn groups_show_their_items_in_their_order() {
        let mut groups = default_groups();
        assert_eq!(chosen(&groups), ["fps", "low", "cpu", "cpu_temp", "gpu", "gpu_temp"]);
        // Moved, switched, and an item chosen.
        groups.swap(0, 1);
        groups[2].on = false;
        set_item(&mut groups, "cpu", "cpu_power", true);
        assert_eq!(chosen(&groups), ["cpu", "cpu_temp", "cpu_power", "fps", "low"]);
        // Only what differs from the default is kept.
        set_item(&mut groups, "cpu", "cpu_power", false);
        assert!(groups[0].items.is_empty());
        // The choices of 0.2.1 and before carried over.
        let old = ["fps", "cpu", "network", "mic"].map(String::from);
        assert_eq!(chosen(&carried_over(&old)), ["fps", "cpu", "down", "up", "mic"]);
        // Every group once, known ones only.
        let mut odd = vec![ModuleEntry { id: "mic".into(), on: true, items: Default::default() }, ModuleEntry { id: "gone".into(), on: true, items: Default::default() }];
        complete(&mut odd);
        assert_eq!(odd.len(), GROUPS.len());
        assert_eq!((odd[0].id.as_str(), odd[0].on, odd[1].on), ("mic", true, false));
    }

    #[test]
    fn charts_the_latest_frame_rates() {
        let mut history: Vec<Sample> = (0..70).map(|_| sample(true)).collect();
        history[69].game = None;
        let frames = frames(history.iter(), true);
        assert_eq!(frames.len(), CHART);
        assert_eq!((frames[0], frames[CHART - 1]), (Some(143.6), None));
        assert!(super::frames(history.iter(), false).is_empty());
    }
}
