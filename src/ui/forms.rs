//! The panel's small layouts, for when it is sized smaller than its lanes
//! can be: each reading a ring round its icon (how full, for what has a
//! share), with its figure; laid out as tiles, a corner, a strip, a rail
//! along the screen's edge, or rings alone.

use super::canvas::{Align, Canvas, Color, Fill, Font, Point};
use super::icons::Icon;
use super::morph::Recorder;
use super::text;
use super::view::Scene;
use crate::reading::Sample;

/// A small layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Small {
    /// A tile each, `across` in a row.
    Tiles { across: usize, down: usize },
    /// The CPU's last minutes as a chart, the chips' rings and the network.
    Corner,
    /// The corner stood on end: the chart over the chips two to a row,
    /// larger, and a row for each pair of rates.
    Tall,
    /// One line; `rich`, with a chart, the chips' temperatures and the
    /// busiest process.
    Strip { rich: bool },
    /// One column, for the edge of the screen.
    Rail,
    /// The chips' rings and nothing else.
    Micro,
}

/// A tile's size, and the room round the tiles (DIPs).
const TILE: (f32, f32) = (156.0, 112.0);
const TILES_PAD: f32 = 12.0;
/// A strip's line, and one reading's room in it with and without what the
/// rich strip adds beside it.
const STRIP: f32 = 44.0;
/// A tall corner's row of two chips, and a pair of rates' row (DIPs).
const TALL_CHIPS: f32 = 40.0;
const TALL_PAIR: f32 = 34.0;
/// A rail's width, and a reading's height in it.
const RAIL: f32 = 96.0;
const RAIL_ROW: f32 = 36.0;
/// A rail at least this wide sets what goes with a figure (a chip's
/// temperature, a pair's second rate) beside it rather than under it.
const RAIL_WIDE: f32 = 150.0;

impl Small {
    /// Its size as designed, for `count` readings (DIPs).
    pub fn size(self, count: usize) -> (f32, f32) {
        let count = count.max(1) as f32;
        match self {
            Small::Tiles { across, down } => (across as f32 * TILE.0 + 2.0 * TILES_PAD, down as f32 * TILE.1 + 2.0 * TILES_PAD),
            Small::Corner => (300.0, 176.0),
            Small::Tall => (240.0, 28.0 + 110.0 + 8.0 + 2.0 * TALL_CHIPS + 2.0 * TALL_PAIR),
            Small::Strip { rich: true } => (24.0 + 84.0 + count * 96.0 + 90.0, STRIP),
            Small::Strip { rich: false } => (24.0 + count * 70.0, STRIP),
            Small::Rail => (RAIL, 120.0 + count * RAIL_ROW),
            Small::Micro => (16.0 + 34.0 * count.min(3.0), 38.0),
        }
    }

    /// Its size as drawn for a skin whose corners are `radius` round: as
    /// designed, grown by how much further in from its edges it is laid
    /// (see `inside`), so that its content has the room it was designed with.
    pub fn size_within(self, count: usize, radius: f32) -> (f32, f32) {
        let (w, h) = self.size(count);
        let Some((pad, taller)) = self.pad() else { return (w, h) };
        let grown = inside(radius, pad) - pad;
        (w + 2.0 * grown, if taller { h + grown } else { h })
    }

    /// The least room round what it lays out, where it draws a chart by
    /// a rounded corner (which it keeps clear of), and whether that chart
    /// is at its top (a strip's is at its start, its height as designed).
    fn pad(self) -> Option<(f32, bool)> {
        match self {
            Small::Corner | Small::Tall => Some((14.0, true)),
            Small::Rail => Some((10.0, true)),
            Small::Strip { .. } => Some((12.0, false)),
            _ => None,
        }
    }

    /// How much wider and taller than designed it may be drawn, its parts
    /// spread over the room (DIPs).
    pub fn stretch(self) -> (f32, f32) {
        match self {
            Small::Tiles { across, down } => (36.0 * across as f32, 24.0 * down as f32),
            Small::Corner => (200.0, 170.0),
            Small::Tall => (120.0, 220.0),
            Small::Strip { rich: true } => (260.0, 10.0),
            Small::Strip { rich: false } => (120.0, 10.0),
            Small::Rail => (96.0, 160.0),
            Small::Micro => (30.0, 6.0),
        }
    }

    /// The least it may be scaled to before a smaller layout takes over.
    pub fn floor(self) -> f32 {
        match self {
            Small::Tiles { .. } => 0.9,
            Small::Corner | Small::Tall => 0.85,
            Small::Strip { .. } | Small::Rail => 0.76,
            Small::Micro => 0.9,
        }
    }

    /// The name it is kept under in the settings.
    pub fn key(self) -> String {
        match self {
            Small::Tiles { across, down } => format!("tiles-{across}x{down}"),
            Small::Corner => "corner".into(),
            Small::Tall => "corner-tall".into(),
            Small::Strip { rich: true } => "strip".into(),
            Small::Strip { rich: false } => "strip-short".into(),
            Small::Rail => "rail".into(),
            Small::Micro => "micro".into(),
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "corner" => Small::Corner,
            "corner-tall" => Small::Tall,
            "strip" => Small::Strip { rich: true },
            "strip-short" => Small::Strip { rich: false },
            "rail" => Small::Rail,
            "micro" => Small::Micro,
            _ => {
                let (across, down) = key.strip_prefix("tiles-")?.split_once('x')?;
                Small::Tiles { across: across.parse().ok()?, down: down.parse().ok()? }
            }
        })
    }

    /// The small layouts there are for `count` readings, the most shown
    /// first.
    pub fn ladder(count: usize) -> Vec<Small> {
        let mut ladder = Vec::new();
        for across in [3, 2] {
            if count > 0 {
                ladder.push(Small::Tiles { across, down: count.div_ceil(across) });
            }
        }
        ladder.extend([Small::Corner, Small::Tall, Small::Strip { rich: true }, Small::Strip { rich: false }, Small::Rail, Small::Micro]);
        ladder
    }
}

/// What a reading is of.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Game,
    Cpu,
    Gpu(usize),
    Memory,
    /// Down and up.
    Network,
    /// Read and written.
    Disk,
}

/// One of a pair of rates: its mark ("↓", "读"), and its figure and unit
/// written out ("8.2", "MB/s") and short ("8.2", "M").
struct Rate {
    mark: String,
    full: (String, String),
    short: (String, String),
}

/// A reading as the small layouts show it.
struct Reading {
    kind: Kind,
    /// Its module's lane, what names its parts (see `morph`).
    id: String,
    /// Its module, for its ink.
    module: &'static str,
    icon: Icon,
    label: String,
    /// How full its ring is (0–1); none where it has no share of anything.
    share: Option<f32>,
    figure: String,
    unit: String,
    /// A second figure beside it (a chip's temperature).
    beside: String,
    /// A detail for a tile's corner.
    detail: String,
    /// Its load runs hot (its figure, ring and chart go red); its
    /// temperature does (the temperature beside it, or in the corner, does),
    /// as the panel's lanes tell them apart.
    hot: bool,
    warm: bool,
    /// The load past which its chart's line is red, as the lanes' are.
    hot_at: Option<f64>,
    /// A pair of rates instead of one figure: down and up, read and written.
    pair: Option<[Rate; 2]>,
}

impl Reading {
    /// Its value in a sample, as its chart draws it; `second`, a pair's
    /// second rate.
    fn value(&self, s: &Sample, scene: &Scene, second: bool) -> Option<f64> {
        match (self.kind, second) {
            (Kind::Game, _) => s.game.as_ref().map(|g| g.fps as f64),
            (Kind::Cpu, _) => s.cpu.map(f64::from),
            (Kind::Gpu(index), _) => s.gpus.get(index)?.usage.map(f64::from),
            (Kind::Memory, _) => Some(s.memory.used as f64 / scene.info.mem_total.max(1) as f64 * 100.0),
            (Kind::Network, false) => s.net_down,
            (Kind::Network, true) => s.net_up,
            (Kind::Disk, false) => s.disk_read,
            (Kind::Disk, true) => s.disk_write,
        }
    }

    /// The most its chart reaches to: a share's whole, else what it reached
    /// (both of a pair's rates).
    fn most(&self, scene: &Scene) -> f64 {
        let reached = |floor: f64| {
            let both = [false, true].into_iter().take(if self.pair.is_some() { 2 } else { 1 });
            both.flat_map(|second| scene.history.iter().filter_map(move |s| self.value(s, scene, second))).fold(floor, f64::max)
        };
        match self.kind {
            Kind::Cpu | Kind::Gpu(_) | Kind::Memory => 100.0,
            // A frame rate against the screen's, or the most it reached.
            Kind::Game => reached(scene.history.last().and_then(|s| s.game.as_ref()?.refresh_hz).map_or(0.0, f64::from).max(30.0)),
            _ => reached(10.0 * 1024.0),
        }
    }
}

/// A rate as a figure and its unit; `short`, one letter of a unit ("8.2M").
fn rate(value: Option<f64>, bits: bool, short: bool) -> (String, String) {
    let Some(value) = value else { return ("—".into(), String::new()) };
    let written = text::rate(value, bits);
    let (figure, unit) = written.split_once(' ').unwrap_or((&written, ""));
    let unit = if short { unit.chars().next().filter(|c| c.is_ascii_uppercase() && unit.len() > 3).map(String::from).unwrap_or_default() } else { unit.to_string() };
    (figure.to_string(), unit)
}

/// The readings of the modules that are on, in a fixed order: a game's
/// frame rate while one runs (with its module on), the CPU, the first
/// graphics card, memory, the network both ways, the disk both ways.
fn readings(scene: &Scene) -> Vec<Reading> {
    let (s, prefs, lang) = (scene.history.last().unwrap(), scene.prefs, scene.lang);
    let on = |module: &str| prefs.modules.iter().any(|entry| entry.on && entry.id == module);
    let celsius = |value: Option<f32>| value.map(|t| format!("{t:.0}°")).unwrap_or_default();
    let percent = |value: Option<f32>| value.map_or("—".into(), |v| format!("{v:.0}"));
    let mut readings = Vec::new();
    // A game's frame rate; asked for by hand (the window in front taken
    // for a game) with none presenting yet, its place kept, waiting.
    let game = s.game.as_ref().filter(|_| on("game") && !super::view::game_hidden());
    if game.is_some() || (on("game") && super::view::front_is_game()) {
        // Its ring: how many of the frames its screen shows it draws (or of
        // the most it drew, if more).
        let most = scene.history.iter().filter_map(|s| s.game.as_ref().map(|g| g.fps)).fold(game.and_then(|g| g.refresh_hz).map_or(0.0, |hz| hz as f32), f32::max);
        // Its 1% low, as the game's own lane shows it when switched on.
        let low = game.and_then(|g| g.low).filter(|_| prefs.shows("game", "low")).map(|low| format!("1% low {low:.0}"));
        readings.push(Reading {
            kind: Kind::Game,
            id: "game".into(),
            module: "game",
            icon: Icon::Game,
            label: lang.pick("帧率", "Frame rate").into(),
            share: Some(game.map_or(0.0, |g| if most > 0.0 { g.fps / most } else { 0.0 })),
            figure: game.map_or("—".into(), |game| format!("{:.0}", game.fps)),
            unit: "FPS".into(),
            beside: low.clone().unwrap_or_default(),
            detail: match game {
                Some(_) => low.unwrap_or_default(),
                None => super::view::waiting(lang),
            },
            hot: false,
            warm: false,
            hot_at: None,
            pair: None,
        });
    }
    if on("cpu") {
        let temp = s.cpu_sensors.as_ref().and_then(|c| c.temp).filter(|_| scene.seen.cpu_temp && prefs.shows("cpu", "temp"));
        readings.push(Reading {
            kind: Kind::Cpu,
            id: "cpu".into(),
            module: "cpu",
            icon: Icon::Cpu,
            label: "CPU".into(),
            share: s.cpu.map(|v| v / 100.0),
            figure: percent(s.cpu),
            unit: "%".into(),
            beside: celsius(temp),
            detail: temp.map(|t| format!("{t:.0} °C")).unwrap_or_default(),
            hot: s.cpu.is_some_and(|v| v > prefs.hot_load),
            warm: temp.is_some_and(|t| t > prefs.hot_temp),
            hot_at: Some(prefs.hot_load as f64),
            pair: None,
        });
    }
    // The first card the machine has shown, if its lane is on.
    let first_card = scene.seen.gpus.iter().position(|seen| seen.present).map(|index| (index, format!("gpu:{}", scene.info.gpus.get(index).map_or(0, |g| g.slot))));
    if let Some((index, id)) = first_card.filter(|(_, id)| on(id)) {
        let reading = s.gpus.get(index);
        let usage = reading.and_then(|g| g.usage);
        let temp = reading.and_then(|g| g.temp).filter(|_| prefs.shows(&id, "temp"));
        readings.push(Reading {
            kind: Kind::Gpu(index),
            id,
            module: "gpu",
            icon: Icon::Gpu,
            label: "GPU".into(),
            share: usage.map(|v| v / 100.0),
            figure: percent(usage),
            unit: "%".into(),
            beside: celsius(temp),
            detail: temp.map(|t| format!("{t:.0} °C")).unwrap_or_default(),
            hot: usage.is_some_and(|v| v > prefs.hot_load),
            warm: temp.is_some_and(|t| t > prefs.hot_temp),
            hot_at: Some(prefs.hot_load as f64),
            pair: None,
        });
    }
    if on("memory") {
        let share = s.memory.used as f32 / scene.info.mem_total.max(1) as f32;
        readings.push(Reading {
            kind: Kind::Memory,
            id: "memory".into(),
            module: "memory",
            icon: Icon::Memory,
            label: lang.pick("内存", "Memory").into(),
            share: Some(share),
            figure: format!("{:.0}", share * 100.0),
            unit: "%".into(),
            beside: String::new(),
            detail: text::size(s.memory.used),
            hot: share * 100.0 > prefs.hot_load,
            warm: false,
            hot_at: Some(prefs.hot_load as f64),
            pair: None,
        });
    }
    // Two rates as one reading, each marked.
    let pair = |kind, module: &'static str, icon, label: &str, marks: [&str; 2], values: [Option<f64>; 2], bits: bool| Reading {
        kind,
        id: module.into(),
        module,
        icon,
        label: label.into(),
        share: None,
        figure: String::new(),
        unit: String::new(),
        beside: String::new(),
        detail: String::new(),
        hot: false,
        warm: false,
        hot_at: None,
        pair: Some([0, 1].map(|i| Rate { mark: marks[i].into(), full: rate(values[i], bits, false), short: rate(values[i], bits, true) })),
    };
    if on("network") {
        readings.push(pair(Kind::Network, "network", Icon::Network, lang.pick("网络", "Network"), ["↓", "↑"], [s.net_down, s.net_up], prefs.network.bits));
    }
    if on("disk") {
        let marks = if lang == super::text::Lang::Zh { ["读", "写"] } else { ["R", "W"] };
        readings.push(pair(Kind::Disk, "disk", Icon::Disk, lang.pick("磁盘", "Disk"), marks, [s.disk_read, s.disk_write], false));
    }
    readings
}

/// How many readings the small layouts have for this scene.
pub fn count(scene: &Scene) -> usize {
    readings(scene).len()
}

/// Draws `small` filling `size` (DIPs), its corner at the frame's origin.
pub fn paint(frame: &dyn Canvas, scene: &Scene, small: Small, size: (f32, f32)) {
    let readings = readings(scene);
    let painter = Painter { frame, scene, readings: &readings };
    match small {
        Small::Tiles { across, down } => painter.tiles(across, down, size),
        Small::Corner => painter.corner(size),
        Small::Tall => painter.tall(size),
        Small::Strip { rich } => painter.strip(rich, size),
        Small::Rail => painter.rail(size),
        Small::Micro => painter.micro(size),
    }
    frame.key("");
}

/// What goes on a chart: a reading, and (dashed) its pair's second rate or
/// another reading.
#[derive(Clone, Copy)]
enum Second<'a> {
    None,
    Pair,
    Other(&'a Reading),
}

struct Painter<'a> {
    frame: &'a dyn Canvas,
    scene: &'a Scene<'a>,
    readings: &'a [Reading],
}

impl Painter<'_> {
    fn theme(&self) -> &super::theme::Theme {
        self.scene.theme
    }

    /// The same painter drawing on `frame`.
    fn on<'b>(&'b self, frame: &'b dyn Canvas) -> Painter<'b> {
        Painter { frame, scene: self.scene, readings: self.readings }
    }

    /// How wide `draw` draws, drawing nothing.
    fn width_of(&self, draw: impl FnOnce(&Painter) -> f32) -> f32 {
        let nothing = Recorder::new(self.frame, 1.0, (0.0, 0.0));
        draw(&self.on(&nothing))
    }

    fn name(&self, reading: &Reading, part: &str) {
        self.frame.key(&format!("{}.{part}", reading.id));
    }

    /// What a temperature is written in: red as it runs hot.
    fn warmth(&self, reading: &Reading, otherwise: Color) -> Color {
        if reading.warm { self.theme().signal } else { otherwise }
    }

    fn ink(&self, reading: &Reading) -> Color {
        if reading.hot { self.theme().signal } else { self.theme().ink(reading.module).trace }
    }

    /// A font of the panel's own face; `figure`, of its figures'.
    fn font(&self, size: f32, weight: f32) -> Font {
        Font { size, weight, ..self.theme().body }
    }

    fn figure_font(&self, size: f32, weight: f32) -> Font {
        Font { size, weight, ..self.theme().figure }
    }

    /// The top of `font`'s line box that puts the ink of a figure in it
    /// centred on `middle`: the line every text on a row stands on.
    fn baseline_at(&self, font: Font, middle: f32) -> f32 {
        let (top, bottom) = self.frame.ink("8", font);
        let (ascent, _) = self.frame.baseline(font);
        middle - (top + bottom) / 2.0 + ascent
    }

    /// `text` standing on `baseline`; how wide it is.
    fn on_line(&self, text: &str, font: Font, color: Color, x: f32, baseline: f32) -> f32 {
        let (ascent, _) = self.frame.baseline(font);
        let width = self.frame.measure(text, font);
        self.frame.text(text, font, color, x, baseline - ascent, width + 2.0, Align::Start);
        width
    }

    /// `text` with its ink centred on `middle`, from `x` (or ending there,
    /// `End`); how wide it is.
    fn level(&self, text: &str, font: Font, color: Color, x: f32, middle: f32, align: Align) -> f32 {
        let (top, bottom) = self.frame.ink(text, font);
        let width = self.frame.measure(text, font);
        let left = if align == Align::End { x - width } else { x };
        self.frame.text(text, font, color, left, middle - (top + bottom) / 2.0, width + 2.0, Align::Start);
        width
    }

    /// The reading's ring round its icon, `radius` round `centre`.
    fn ring(&self, reading: &Reading, centre: Point, radius: f32) {
        self.ring_alone(reading, centre, radius);
        self.frame.icon(reading.icon, centre, radius * 1.3, self.ink(reading));
    }

    /// The reading's ring (or, without a share, a disc), nothing in it.
    fn ring_alone(&self, reading: &Reading, centre: Point, radius: f32) {
        let (theme, ink) = (self.theme(), self.ink(reading));
        self.name(reading, "icon");
        match reading.share {
            Some(share) => {
                let width = (radius * 0.16).max(1.5);
                self.frame.arc(centre, radius, 0.0, std::f32::consts::TAU, theme.track, width);
                self.frame.arc(centre, radius, 0.0, std::f32::consts::TAU * share.clamp(0.0, 1.0), ink, width);
            }
            None => self.frame.fill_circle(theme.track, centre, radius * 1.1),
        }
    }

    /// The reading's figure and its unit on `baseline`, the unit `unit`
    /// large: a sign close, a word a space apart. How wide they are.
    #[allow(clippy::too_many_arguments)]
    fn figure(&self, reading: &Reading, x: f32, baseline: f32, font: Font, unit: Font, unit_color: Color) -> f32 {
        let theme = self.theme();
        self.name(reading, "value");
        let width = self.on_line(&reading.figure, font, if reading.hot { theme.signal } else { theme.text }, x, baseline);
        if reading.unit.is_empty() {
            return width;
        }
        let gap = if reading.unit.starts_with(|c: char| c.is_alphabetic()) { font.size * 0.25 } else { 1.0 };
        self.name(reading, "unit");
        width + gap + self.on_line(&reading.unit, unit, unit_color, x + width + gap, baseline)
    }

    /// One of a pair's rates on `baseline`: its mark, centred on the
    /// figure's ink, its figure and unit. How wide it is.
    #[allow(clippy::too_many_arguments)]
    fn rate(&self, reading: &Reading, second: bool, x: f32, baseline: f32, font: Font, unit: Font, short: bool) -> f32 {
        let theme = self.theme();
        let Some(pair) = &reading.pair else { return 0.0 };
        let rate = &pair[second as usize];
        let which = if second { "b" } else { "a" };
        let (figure, unit_text) = if short { &rate.short } else { &rate.full };
        let (ascent, _) = self.frame.baseline(font);
        let (top, bottom) = self.frame.ink(figure, font);
        let middle = baseline - ascent + (top + bottom) / 2.0;
        self.name(reading, &format!("{which}.mark"));
        let mut width = self.level(&rate.mark, Font { size: font.size * 0.86, weight: 500.0, ..self.font(0.0, 0.0) }, theme.text3, x, middle, Align::Start) + 3.0;
        self.name(reading, &format!("{which}.value"));
        width += self.on_line(figure, font, if second { theme.text2 } else { theme.text }, x + width, baseline);
        if !unit_text.is_empty() {
            self.name(reading, &format!("{which}.unit"));
            width += 1.0 + self.on_line(unit_text, unit, theme.text2, x + width + 1.0, baseline);
        }
        width
    }

    /// A chip on a row: the reading's ring, then its figure and, with
    /// `beside`, its temperature; a pair's two rates short. How wide.
    fn chip(&self, reading: &Reading, x: f32, middle: f32, size: f32, beside: bool, radius: f32) -> f32 {
        let theme = self.theme();
        self.ring(reading, Point { x: x + radius, y: middle }, radius);
        let start = x + 2.0 * radius + 6.0;
        let font = self.font(size, 600.0);
        let baseline = self.baseline_at(font, middle);
        if reading.pair.is_some() {
            let first = self.rate(reading, false, start, baseline, font, self.font(size * 0.78, 500.0), true);
            let second = self.rate(reading, true, start + first + 6.0, baseline, font, self.font(size * 0.78, 500.0), true);
            return start + first + 6.0 + second - x;
        }
        let mut end = start + self.figure(reading, start, baseline, font, font, if reading.hot { theme.signal } else { theme.text });
        if beside && !reading.beside.is_empty() {
            self.name(reading, "beside");
            end += 5.0 + self.on_line(&reading.beside, self.font(size, 500.0), self.warmth(reading, theme.text3), end + 5.0, baseline);
        }
        end - x
    }

    /// `readings` as chips in a row from `left` to `right`: the first at
    /// the one, the last ending at the other, the space between shared.
    #[allow(clippy::too_many_arguments)]
    /// What goes beside a figure goes only where the row holds it.
    fn row(&self, readings: &[&Reading], left: f32, right: f32, middle: f32, size: f32, beside: bool, radius: f32) {
        let widths_with = |beside: bool| -> Vec<f32> { readings.iter().map(|r| self.width_of(|p| p.chip(r, 0.0, middle, size, beside, radius))).collect() };
        let mut widths = widths_with(beside);
        let gaps = readings.len().saturating_sub(1) as f32 * 8.0;
        let beside = beside && widths.iter().sum::<f32>() + gaps <= right - left;
        if !beside {
            widths = widths_with(false);
        }
        let gap = if readings.len() > 1 { (right - left - widths.iter().sum::<f32>()) / (readings.len() - 1) as f32 } else { 0.0 };
        let mut x = left;
        for (reading, width) in readings.iter().zip(widths) {
            self.chip(reading, x, middle, size, beside, radius);
            x += width + gap;
        }
    }

    /// The last of a reading's samples as a line across the box, the newest
    /// at its right edge, as many as it has room for at about three DIPs a
    /// sample; filled below for `fill`; red above the load it runs hot at.
    #[allow(clippy::too_many_arguments)]
    fn line(&self, reading: &Reading, second: bool, most: f64, (x, y, w, h): (f32, f32, f32, f32), color: Color, width: f32, dashed: bool, fill: Option<Color>) {
        let history = self.scene.history;
        let room = ((w / 3.0) as usize).max(20);
        let samples = &history[history.len().saturating_sub(room)..];
        let step = w / samples.len().saturating_sub(1).max(1) as f32;
        let points: Vec<Point> = samples
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let value = reading.value(s, self.scene, second)?;
                Some(Point { x: x + i as f32 * step, y: y + h - h * (value / most).clamp(0.0, 1.0) as f32 })
            })
            .collect();
        if points.len() < 2 {
            return;
        }
        if let Some(fill) = fill {
            let mut area = points.clone();
            area.push(Point { x: points.last().unwrap().x, y: y + h });
            area.push(Point { x: points[0].x, y: y + h });
            self.frame.fill_shape(&area, Fill::Solid(fill));
        }
        let stroke = |color| {
            if dashed {
                self.frame.stroke_dashed(&points, color, width);
            } else {
                self.frame.stroke(&points, color, width);
            }
        };
        stroke(color);
        if let Some(hot) = reading.hot_at.filter(|hot| !second && *hot < most) {
            let line = y + h - h * (hot / most) as f32;
            self.frame.clip(x, y - 2.0, w, line - y + 2.0);
            stroke(self.theme().signal);
            self.frame.unclip();
        }
    }

    /// `reading`'s chart in the box, ruled, filled below its line, and a
    /// second line dashed over it; where the skin gives every reading the
    /// same ink, a key to which line is which in its corner, for `legend`.
    fn chart(&self, reading: &Reading, second: Second, (x, y, w, h): (f32, f32, f32, f32), legend: bool) {
        let theme = self.theme();
        if w < 2.0 || h < 2.0 {
            return;
        }
        self.name(reading, "chart");
        for i in 0..=4 {
            self.frame.fill(theme.rule, (x + w * i as f32 / 4.0).round(), y, 1.0, h);
        }
        let ink = theme.ink(reading.module);
        let first = ink.trace;
        let most = reading.most(self.scene);
        let alike = |other: &Reading| theme.same_ink(reading.module, other.module);
        let other = match second {
            // Another reading's line, on its own scale: in its own ink, or
            // grey where it would share this one's.
            Second::Other(other) => {
                let color = if alike(other) { theme.text3 } else { theme.ink(other.module).trace.alpha(0.9) };
                Some((other, false, color, other.most(self.scene)))
            }
            Second::Pair => Some((reading, true, ink.trace2, most)),
            Second::None => None,
        };
        self.line(reading, false, most, (x, y, w, h), first, 1.5, false, Some(ink.wash));
        if let Some((other, pair, color, other_most)) = other {
            self.line(other, pair, other_most, (x, y, w, h), color, 1.2, true, None);
            // Which is which, where the two would not tell apart by colour.
            if legend && (alike(other) || other.kind == Kind::Game || reading.kind == Kind::Game) && h >= 40.0 {
                self.legend(x + 4.0, y + 4.0, w - 8.0, [(&reading.label, first, false), (&other.label, color, true)]);
            }
        }
    }

    /// Which line is which, from (`x`, `y`): a piece of each line, then its
    /// name, on a plate of their own; none where `room` is too narrow.
    fn legend(&self, x: f32, y: f32, room: f32, lines: [(&str, Color, bool); 2]) {
        let theme = self.theme();
        let font = self.font(10.5, 500.0);
        let (pad, sample, gap, apart) = (5.0, 12.0, 4.0, 9.0);
        let widths = lines.map(|(name, ..)| self.frame.measure(name, font));
        let width = widths.iter().sum::<f32>() + 2.0 * (sample + gap) + apart + 2.0 * pad;
        if width > room {
            return;
        }
        let height = font.size + 2.0 * pad - 2.0;
        let plate = if theme.dark { Color::hex(0x000000, 0.35) } else { Color::hex(0xFFFFFF, 0.72) };
        self.frame.fill_rounded(plate, x, y, width, height, height / 2.0);
        let middle = y + height / 2.0;
        let mut at = x + pad;
        for ((name, color, dashed), width) in lines.into_iter().zip(widths) {
            let piece = [Point { x: at, y: middle }, Point { x: at + sample, y: middle }];
            if dashed {
                self.frame.stroke_dashed(&piece, color, 1.2);
            } else {
                self.frame.stroke(&piece, color, 1.5);
            }
            at += sample + gap;
            self.level(name, font, theme.text2, at, middle, Align::Start);
            at += width + apart;
        }
    }

    fn inside(&self, pad: f32) -> f32 {
        inside(self.theme().radius, pad)
    }

    fn first(&self, kind: fn(&Kind) -> bool) -> Option<&Reading> {
        self.readings.iter().find(|r| kind(&r.kind))
    }

    /// The CPU's chart with the card's over it, keyed for `legend`; while a
    /// game is shown, its frame rate's with the card's (or the first
    /// reading's).
    fn history(&self, area: (f32, f32, f32, f32), with_card: bool, legend: bool) {
        let Some(main) = self.first(|k| *k == Kind::Game).or(self.first(|k| *k == Kind::Cpu)).or(self.readings.first()) else { return };
        let card = self.first(|k| matches!(k, Kind::Gpu(_))).filter(|_| with_card && matches!(main.kind, Kind::Cpu | Kind::Game));
        self.chart(main, card.map_or(Second::None, Second::Other), area, legend);
    }

    fn tiles(&self, across: usize, down: usize, (w, h): (f32, f32)) {
        let theme = self.theme();
        let (tw, th) = ((w - 2.0 * TILES_PAD) / across as f32, (h - 2.0 * TILES_PAD) / down as f32);
        for (i, reading) in self.readings.iter().take(across * down).enumerate() {
            let (x, y) = (TILES_PAD + (i % across) as f32 * tw, TILES_PAD + (i / across) as f32 * th);
            self.frame.key("");
            if i % across > 0 {
                self.frame.fill(theme.rule, x, y + 8.0, 1.0, th - 16.0);
            }
            if i >= across {
                self.frame.fill(theme.rule, x + 8.0, y, tw - 16.0, 1.0);
            }
            self.ring(reading, Point { x: x + 22.0, y: y + 22.0 }, 11.0);
            self.name(reading, "label");
            self.level(&reading.label, theme.small, theme.text2, x + 40.0, y + 22.0, Align::Start);
            if !reading.detail.is_empty() {
                self.name(reading, "detail");
                self.level(&reading.detail, theme.small, self.warmth(reading, theme.text3), x + tw - 12.0, y + 22.0, Align::End);
            }
            // The figures, then the chart under them in what height is left:
            // a pair, a line each; else the figure large, its unit after it.
            let below = match reading.pair {
                Some(_) => {
                    let (font, unit) = (self.figure_font(17.0, 620.0), self.font(11.0, 500.0));
                    self.rate(reading, false, x + 12.0, y + 58.0, font, unit, false);
                    self.rate(reading, true, x + 12.0, y + 78.0, font, unit, false);
                    y + 86.0
                }
                None => {
                    self.figure(reading, x + 12.0, y + 70.0, self.figure_font(30.0, theme.figure.weight), self.font(12.0, 500.0), theme.text2);
                    y + 78.0
                }
            };
            let second = if reading.pair.is_some() { Second::Pair } else { Second::None };
            self.chart(reading, second, (x + 12.0, below, tw - 24.0, (y + th - 10.0 - below).max(14.0)), false);
        }
    }

    fn corner(&self, (w, h): (f32, f32)) {
        let (pad, base) = (self.inside(14.0), 14.0);
        self.history((pad, pad, w - 2.0 * pad, h - pad - base - 70.0), true, true);
        let chips: Vec<&Reading> = self.readings.iter().filter(|r| r.pair.is_none()).take(3).collect();
        self.row(&chips, pad, w - pad, h - base - 46.0, 14.0, true, 10.0);
        let pairs: Vec<&Reading> = self.readings.iter().filter(|r| r.pair.is_some()).collect();
        self.row(&pairs, pad, w - pad, h - base - 12.0, 13.0, false, 10.0);
    }

    fn tall(&self, (w, h): (f32, f32)) {
        let (pad, base) = (self.inside(14.0), 14.0);
        let chips: Vec<&Reading> = self.readings.iter().filter(|r| r.pair.is_none()).take(4).collect();
        let pairs: Vec<&Reading> = self.readings.iter().filter(|r| r.pair.is_some()).collect();
        let rows = chips.len().div_ceil(2) as f32;
        let below = rows * TALL_CHIPS + pairs.len() as f32 * TALL_PAIR;
        self.history((pad, pad, w - 2.0 * pad, h - pad - base - 8.0 - below), true, true);
        // The chips two to a row, each in its half; then each pair edge to
        // edge, as the corner's rows are.
        let top = h - base - below;
        let half = (w - 2.0 * pad) / 2.0;
        for (i, reading) in chips.iter().enumerate() {
            let middle = top + (i / 2) as f32 * TALL_CHIPS + TALL_CHIPS / 2.0;
            // What goes beside its figure only where the half holds it.
            let beside = self.width_of(|p| p.chip(reading, 0.0, middle, 16.0, true, 11.0)) <= half - 8.0;
            self.chip(reading, pad + (i % 2) as f32 * half, middle, 16.0, beside, 11.0);
        }
        for (i, reading) in pairs.iter().enumerate() {
            let middle = top + rows * TALL_CHIPS + i as f32 * TALL_PAIR + TALL_PAIR / 2.0;
            self.row(&[reading], pad, w - pad, middle, 15.0, false, 11.0);
        }
    }

    fn strip(&self, rich: bool, (w, h): (f32, f32)) {
        let theme = self.theme();
        let middle = h / 2.0;
        let shown: Vec<&Reading> = self.readings.iter().filter(|r| r.kind != Kind::Disk).collect();
        // Each part's width, then the room left spread between them.
        let chart = rich && !self.readings.is_empty();
        let mut widths: Vec<f32> = Vec::new();
        if chart {
            widths.push(84.0);
        }
        widths.extend(shown.iter().map(|r| self.width_of(|p| p.chip(r, 0.0, middle, 14.0, rich, 8.5))));
        let busiest = rich.then(|| busiest(self.scene)).flatten();
        let busiest_font = theme.body;
        // Its ends clear of its rounded corners.
        let pad = self.inside(12.0);
        // The busiest process as it is written, or in what room is left
        // with the parts before it at their closest (its name cut short).
        if let Some((name, share)) = &busiest {
            let whole = self.frame.measure(name, busiest_font) + 8.0 + self.frame.measure(share, busiest_font);
            let room = w - 2.0 * pad - widths.iter().sum::<f32>() - 6.0 * widths.len() as f32;
            widths.push(whole.min(room.max(0.0)));
        }
        let gap = ((w - 2.0 * pad - widths.iter().sum::<f32>()) / (widths.len().max(2) - 1) as f32).max(6.0);
        let mut x = pad;
        let mut widths = widths.into_iter();
        if chart {
            let width = widths.next().unwrap();
            self.history((x, 9.0, width, h - 18.0), false, false);
            x += width + gap;
        }
        for reading in shown {
            let width = widths.next().unwrap();
            self.chip(reading, x, middle, 14.0, rich, 8.5);
            x += width + gap;
        }
        if let (Some((name, share)), Some(width)) = (busiest, widths.next()) {
            // Its share whole, after its name in what room is left.
            self.frame.key("busiest");
            let share_width = self.frame.measure(&share, busiest_font);
            let (top, bottom) = self.frame.ink(&name, busiest_font);
            let room = (width - share_width - 8.0).max(0.0);
            self.frame.text(&name, busiest_font, theme.text2, x, middle - (top + bottom) / 2.0, room, Align::Start);
            let named = self.frame.measure(&name, busiest_font).min(room);
            self.level(&share, busiest_font, theme.text2, x + named + 8.0, middle, Align::Start);
        }
    }

    fn rail(&self, (w, h): (f32, f32)) {
        let theme = self.theme();
        let (pad, base) = (self.inside(10.0), 10.0);
        let rows = self.readings.len() as f32;
        let chart = (h - pad - base - rows * RAIL_ROW - 8.0).max(36.0);
        self.history((pad, pad, w - 2.0 * pad, chart), true, true);
        let gap = ((h - pad - base - chart - 8.0 - rows * RAIL_ROW) / rows.max(1.0)).max(0.0);
        // Wide enough, what goes with a figure beside it; else under it.
        let wide = w >= RAIL_WIDE;
        let (font, small) = (self.font(13.0, 600.0), self.font(11.0, 500.0));
        for (i, reading) in self.readings.iter().enumerate() {
            let top = pad + chart + 8.0 + i as f32 * (RAIL_ROW + gap) + gap / 2.0;
            let middle = top + 11.0;
            self.ring(reading, Point { x: pad + 9.0, y: middle }, 8.5);
            let x = pad + 24.0;
            let (baseline, under) = (self.baseline_at(font, middle), self.baseline_at(small, middle + 15.0));
            if reading.pair.is_some() {
                let first = self.rate(reading, false, x, baseline, font, self.font(10.0, 500.0), true);
                if wide {
                    self.rate(reading, true, x + first + 6.0, baseline, font, self.font(10.0, 500.0), true);
                } else {
                    self.rate(reading, true, x, under, small, self.font(9.0, 500.0), true);
                }
                continue;
            }
            let width = self.figure(reading, x, baseline, font, font, if reading.hot { theme.signal } else { theme.text });
            if !reading.beside.is_empty() {
                self.name(reading, "beside");
                // Beside it where it fits the rail, else under it.
                let fits = x + width + 5.0 + self.frame.measure(&reading.beside, self.font(13.0, 500.0)) <= w - pad;
                if wide && fits {
                    self.on_line(&reading.beside, self.font(13.0, 500.0), self.warmth(reading, theme.text3), x + width + 5.0, baseline);
                } else if x + self.frame.measure(&reading.beside, small) <= w - pad {
                    self.on_line(&reading.beside, small, self.warmth(reading, theme.text3), x, under);
                }
            }
        }
    }

    fn micro(&self, (w, h): (f32, f32)) {
        let chips: Vec<&Reading> = self.readings.iter().filter(|r| r.share.is_some()).take(3).collect();
        let cell = (w - 16.0) / chips.len().max(1) as f32;
        for (i, reading) in chips.iter().enumerate() {
            let (centre, radius) = (Point { x: 8.0 + cell * (i as f32 + 0.5), y: h / 2.0 }, (h / 2.0 - 5.0).min(13.0));
            if reading.kind != Kind::Game {
                self.ring(reading, centre, radius);
                continue;
            }
            // A frame rate is its figure, in its ring.
            self.ring_alone(reading, centre, radius);
            let font = self.font(radius * 0.78, 650.0);
            let width = self.frame.measure(&reading.figure, font);
            self.name(reading, "value");
            self.level(&reading.figure, font, self.ink(reading), centre.x - width / 2.0, centre.y, Align::Start);
        }
    }
}

/// How far in from its edges a layout is laid, `pad` at least: clear of
/// corners `radius` round, the rounder the further in, so that a chart at
/// its top keeps off them and what is under it lines up with it.
fn inside(radius: f32, pad: f32) -> f32 {
    (radius * 0.75).max(pad)
}

/// The process using the most CPU and its share, if the process list is on.
fn busiest(scene: &Scene) -> Option<(String, String)> {
    if !scene.prefs.modules.iter().any(|entry| entry.on && entry.id == "processes") {
        return None;
    }
    let s = scene.history.last()?;
    let p = s.processes.iter().max_by(|a, b| a.cpu.total_cmp(&b.cpu))?;
    Some((p.name.clone(), text::percent(p.cpu)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_each_layout_under_a_name() {
        for small in Small::ladder(6).into_iter().chain(Small::ladder(2)) {
            assert_eq!(Small::from_key(&small.key()), Some(small));
        }
        assert_eq!(Small::from_key("tiles-x"), None);
    }

    #[test]
    fn tiles_take_as_many_rows_as_the_readings_need() {
        assert_eq!(Small::ladder(6)[..2], [Small::Tiles { across: 3, down: 2 }, Small::Tiles { across: 2, down: 3 }]);
        assert_eq!(Small::ladder(4)[..2], [Small::Tiles { across: 3, down: 2 }, Small::Tiles { across: 2, down: 2 }]);
        assert!(!Small::ladder(0).iter().any(|s| matches!(s, Small::Tiles { .. })));
    }

    #[test]
    fn writes_rates_short() {
        assert_eq!(rate(Some(8.6e6), false, true), ("8.2".into(), "M".into()));
        assert_eq!(rate(Some(190_000.0), false, true), ("186".into(), "K".into()));
        assert_eq!(rate(Some(500.0), false, true), ("500".into(), String::new()));
        assert_eq!(rate(Some(8.6e6), false, false), ("8.2".into(), "MB/s".into()));
        assert_eq!(rate(None, false, true).0, "—");
    }
}
