//! The panel's content: each module as a lane of blocks, laid out in
//! columns, and drawn for a sample and the history before it.

use std::hash::{Hash, Hasher};

use super::canvas::{Align, Canvas, Color, Family, Fill, Font, Point};
use super::prefs::{Prefs, ProcessSort};
use super::seen::Seen;
use super::text::{self, Lang};
use super::theme::{Ink, Skin, Theme};
use crate::reading::{GpuLimit, ProcessSample, Sample, StaticInfo};

/// A column's width (DIPs), whatever the skin.
pub const COLUMN_WIDTH: f32 = 356.0;
/// The label column of a lane, and the gap after it.
const LABEL: f32 = 96.0;
const LABEL_GAP: f32 = 12.0;
const HEAD: f32 = 17.0;
const HEAD_GAP: f32 = 8.0;
const PLOT: f32 = 40.0;
const RATE_PLOT: f32 = 36.0;
/// Rate charts start this much further right, past the rates beside them.
const RATE_INDENT: f32 = 26.0;
const LINE: f32 = 16.0;
const FACT_GAP: f32 = 3.0;
pub const TABLE_ROW: f32 = 22.0;
/// The most processes a lane past `Detail::Full` lists.
const COMPACT_PROCESSES: usize = 3;
/// The settings button in the bar.
const BUTTON: f32 = 32.0;
/// Between two of the bar's buttons, so that two lit side by side stay two.
const BUTTON_GAP: f32 = 4.0;
/// Rates below this full scale are drawn against it, so idle chatter stays low.
const MIN_RATE_SCALE: f64 = 10.0 * 1024.0;
/// The frame time chart's least full scale, in milliseconds.
const MIN_FRAME_SCALE: f64 = 10.0;

/// What a click or a wheel turn on the panel lands on.
#[derive(Clone, Copy, PartialEq, Hash)]
pub enum Hit {
    Settings,
    Pin,
    Sort(ProcessSort),
    /// The process list, and how far it scrolls.
    Processes(u32),
    /// The overlay's switch.
    Overlay,
}

impl Hit {
    /// How far the process list scrolls, for a hit on it.
    pub fn max_scroll(self) -> Option<f32> {
        match self {
            Hit::Processes(bits) => Some(f32::from_bits(bits)),
            _ => None,
        }
    }
}

/// A region of the panel and what lands on it, in DIPs from its corner.
pub type HitBox = (f32, f32, f32, f32, Hit);

/// What the readings are drawn with: the moment, the data, and the choices.
pub struct Scene<'a> {
    pub info: &'a StaticInfo,
    pub prefs: &'a Prefs,
    pub theme: &'a Theme,
    pub lang: Lang,
    pub history: &'a [Sample],
    /// What the machine has shown it can read: the rows and lanes there are.
    pub seen: &'a Seen,
    /// The pen's time: the chart runs a little behind the newest sample, so
    /// the next one has always arrived by the time it is drawn to.
    pub pen_ms: f64,
    pub process_scroll: f32,
    /// What the pointer is over.
    pub hover: Option<Hit>,
    /// The panel is pinned open.
    pub pinned: bool,
    /// The overlay is on.
    pub overlay: bool,
    /// The bar has its buttons: the panel's, not a widget's (a widget has
    /// its own pin and menu, and its game button: a game asked for or
    /// turned off by hand shows in widgets, not the panel).
    pub buttons: bool,
}

impl Scene<'_> {
    fn latest(&self) -> &Sample {
        self.history.last().unwrap()
    }
}

/// One line of a chart: its reading in a sample, if the sample has it.
type Series = Box<dyn Fn(&Sample) -> Option<f64>>;

/// A chart: series read from samples, drawn against a full scale.
struct Plot {
    series: Vec<Series>,
    max: f64,
    hot: Option<f64>,
}

impl Plot {
    /// A chart against `max`, or with `None`, against the busiest moment on screen.
    fn new(scene: &Scene, series: Vec<Series>, max: Option<f64>, hot: Option<f64>) -> Self {
        let max = max.unwrap_or_else(|| full_scale(scene, &series));
        Plot { series, max, hot }
    }
}

/// What a pass over the panel draws: everything that holds still between
/// samples, or the charts, which move with every frame.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pass {
    Content,
    Plots,
}

enum Block {
    Head { title: String, device: String, aside: String, aside_hot: bool },
    /// A figure, and its chart unless the module's chart is switched off.
    Readout { figure: String, unit: &'static str, hot: bool, plot: Option<Plot> },
    Rates { rows: Vec<(String, String)>, plot: Option<Plot> },
    /// A second chart below a readout's, aligned with it, `gap` below it:
    /// what it charts named over its value.
    Trace { label: String, value: String, hot: bool, plot: Plot, gap: f32 },
    /// Each thread's load (0–1), unread where `None`, and whether it is hot.
    Threads(Vec<(Option<f32>, bool)>),
    Meter { label: String, fraction: f32, value: String, hot: bool, gap: f32 },
    Facts { rows: Vec<(String, String, bool)>, gap: f32 },
    Table { headings: Vec<(String, ProcessSort)>, sort: ProcessSort, rows: Vec<[String; 5]>, visible: usize },
}

impl Block {
    fn height(&self) -> f32 {
        match self {
            Block::Head { .. } => HEAD + HEAD_GAP,
            Block::Readout { .. } => PLOT,
            Block::Rates { .. } => RATE_PLOT,
            Block::Trace { gap, .. } => gap + RATE_PLOT,
            Block::Threads(_) => 10.0 + 14.0,
            Block::Meter { gap, .. } => gap + LINE,
            Block::Facts { rows, gap } if !rows.is_empty() => gap + rows.len() as f32 * LINE + (rows.len() - 1) as f32 * FACT_GAP,
            Block::Facts { .. } => 0.0,
            Block::Table { visible, .. } => *visible as f32 * TABLE_ROW,
        }
    }
}

impl Hash for Block {
    /// Everything the content pass draws from: two blocks that hash alike
    /// draw alike.
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Block::Head { title, device, aside, aside_hot } => (title, device, aside, aside_hot).hash(state),
            Block::Readout { figure, unit, hot, .. } => (figure, unit, hot).hash(state),
            Block::Rates { rows, plot } => (rows, plot.as_ref().map(|plot| plot.max.to_bits())).hash(state),
            Block::Trace { label, value, hot, plot, gap } => (label, value, hot, plot.max.to_bits(), gap.to_bits()).hash(state),
            Block::Threads(cells) => cells.iter().for_each(|(load, hot)| (load.map(f32::to_bits), hot).hash(state)),
            Block::Meter { label, fraction, value, hot, gap } => (label, fraction.to_bits(), value, hot, gap.to_bits()).hash(state),
            Block::Facts { rows, gap } => (rows, gap.to_bits()).hash(state),
            Block::Table { headings, sort, rows, visible } => {
                headings.iter().for_each(|(label, key)| (label, *key as u8).hash(state));
                (*sort as u8, rows, visible).hash(state);
            }
        }
    }
}

/// How much of its module a lane shows (see `arrange::Form`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Detail {
    /// All of it.
    #[default]
    Full,
    /// Its figure and chart, and what belongs with them (a card's video
    /// memory, its volumes, a few processes); none of the rows below.
    Compact,
}

pub struct Lane {
    /// Its module's (what names its parts, see `morph`).
    id: String,
    blocks: Vec<Block>,
    ink: Ink,
}

impl Hash for Lane {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.blocks.hash(state);
    }
}

/// Whether the window in front is taken for a game, asked for by hand
/// The game in `s`: in the panel (`panel`), only one found, not one taken
/// for a game by hand (that is the widgets' own).
fn found_game(s: &Sample, panel: bool) -> Option<&crate::reading::GameSample> {
    s.game.as_ref().filter(|game| !(panel && game.by_hand))
}

/// The game in `s`, if it is the process `pid`: a chart of a game's frames
/// is of that game's alone, not of another's before it.
fn same_game(s: &Sample, panel: bool, pid: Option<u32>) -> Option<&crate::reading::GameSample> {
    found_game(s, panel).filter(|game| Some(game.pid) == pid)
}

/// (see `presents::GameMode`).
pub fn front_is_game() -> bool {
    #[cfg(windows)]
    return crate::presents::game_mode() == crate::presents::GameMode::On;
    #[cfg(not(windows))]
    false
}

/// Whether games are turned off by hand: none shows, found or not.
pub fn game_hidden() -> bool {
    #[cfg(windows)]
    return crate::presents::game_mode() == crate::presents::GameMode::Off;
    #[cfg(not(windows))]
    false
}

/// What a game asked for by hand says before its first frames.
pub fn waiting(lang: Lang) -> String {
    if front_is_game() { lang.pick("等待画面", "No frames yet").into() } else { String::new() }
}

/// The lanes of the modules that are on and have something to show, all
/// of each.
pub fn lanes(scene: &Scene) -> Vec<Lane> {
    lanes_at(scene, |_| Detail::Full)
}

impl Lane {
    pub fn height(&self, theme: &Theme) -> f32 {
        theme.pad_top + theme.pad_bottom + self.blocks.iter().map(Block::height).sum::<f32>()
    }
}

/// The lanes of the modules that are on and have something to show, each
/// showing as much as `detail` says for it.
pub fn lanes_at(scene: &Scene, detail: impl Fn(&str) -> Detail) -> Vec<Lane> {
    scene
        .prefs
        .modules
        .iter()
        .filter(|entry| entry.on)
        .filter_map(|entry| Some(Lane { id: entry.id.clone(), blocks: lane(scene, &entry.id, detail(&entry.id))?, ink: scene.theme.ink(&entry.id) }))
        .collect()
}

fn head(title: impl Into<String>, device: impl Into<String>, aside: impl Into<String>, aside_hot: bool) -> Block {
    Block::Head { title: title.into(), device: device.into(), aside: aside.into(), aside_hot }
}

fn celsius(value: f32) -> String {
    format!("{value:.0} °C")
}

/// What a reading shows while it is not there to read.
const UNREAD: &str = "—";

/// A reading as a fact's value: as `show` writes it, or "—".
fn shown<T>(value: Option<T>, show: impl Fn(T) -> String) -> String {
    value.map_or_else(|| UNREAD.to_string(), show)
}

/// Each lane's module and its height at every detail (see
/// `arrange::Heights`), for the modules that are on and have something to
/// show.
pub fn heights(scene: &Scene) -> Vec<(String, [f32; 2])> {
    let height = |id: &str, detail: Detail| lane(scene, id, detail).map(|blocks| scene.theme.pad_top + scene.theme.pad_bottom + blocks.iter().map(Block::height).sum::<f32>());
    scene
        .prefs
        .modules
        .iter()
        .filter(|entry| entry.on)
        .filter_map(|entry| Some((entry.id.clone(), [height(&entry.id, Detail::Full)?, height(&entry.id, Detail::Compact)?])))
        .collect()
}

/// The height of the lane of module `id` showing `detail`, holding what
/// `seen` holds; `None` where there is no such lane.
pub fn lane_height(scene: &Scene, id: &str, seen: &Seen, detail: Detail) -> Option<f32> {
    let scene = Scene { seen, ..*scene };
    let blocks = lane(&scene, id, detail)?;
    Some(scene.theme.pad_top + scene.theme.pad_bottom + blocks.iter().map(Block::height).sum::<f32>())
}

/// What a board's sensor `name` of `group` ("temps" or "fans") is called:
/// by what it is wired to, where the board is known; else by its number.
pub fn board_sensor(name: &str, group: &str, lang: Lang) -> String {
    if name.parse::<u32>().is_ok() {
        let numbered = if group == "fans" { ("风扇", "Fan") } else { ("传感器", "Sensor") };
        format!("{} {name}", lang.pick(numbered.0, numbered.1))
    } else {
        lang.name(name)
    }
}

/// The blocks of module `id`'s lane: all of it, or (`Compact`) none of the
/// rows below its figure and chart.
fn lane(scene: &Scene, id: &str, detail: Detail) -> Option<Vec<Block>> {
    let (s, prefs, lang, info) = (scene.latest(), scene.prefs, scene.lang, scene.info);
    let hot_load = prefs.hot_load;
    let hot_temp = prefs.hot_temp;
    let full = detail == Detail::Full;
    // Whether the module's item is switched on.
    let on = |item: &str| prefs.shows(id, item);
    let chart = on("chart");
    let readout = |value: Option<f32>, read: Series| Block::Readout {
        figure: shown(value, |value| format!("{value:.0}")),
        unit: "%",
        hot: value.is_some_and(|value| value > hot_load),
        plot: chart.then(|| Plot::new(scene, vec![read], Some(100.0), Some(hot_load as f64))),
    };
    Some(match id {
        "cpu" => {
            let sensors = s.cpu_sensors.as_ref();
            let temp = sensors.and_then(|c| c.temp);
            let had_temp = on("temp") && scene.seen.cpu_temp;
            let clock = s.ghz.map(|ghz| format!("{ghz:.2} GHz"));
            let had_clock = on("clock") && scene.seen.cpu_clock;
            // The temperature takes the corner, as on the GPU lanes; the
            // clock, the power and each chiplet's temperature go below.
            let aside = if had_temp {
                shown(temp, celsius)
            } else if had_clock {
                clock.clone().unwrap_or_else(|| UNREAD.into())
            } else {
                String::new()
            };
            let mut facts = Vec::new();
            if had_temp && had_clock {
                facts.push((lang.pick("频率", "Clock").into(), clock.unwrap_or_else(|| UNREAD.into()), false));
            }
            if on("power") && scene.seen.cpu_power {
                facts.push((lang.pick("功耗", "Power").into(), shown(sensors.and_then(|c| c.power), |power| format!("{power:.1} W")), false));
            }
            let chiplets = &scene.seen.ccds;
            if on("ccds") && chiplets.len() > 1 {
                for &ccd in chiplets {
                    let value = sensors.and_then(|c| c.ccds.iter().find(|(read, _)| *read == ccd)).map(|(_, value)| *value);
                    facts.push((format!("CCD {}", ccd + 1), shown(value, celsius), value.is_some_and(|v| v > hot_temp)));
                }
            }
            if !full {
                facts.clear();
            }
            let mut blocks = vec![
                head("CPU", &info.cpu_name, aside, had_temp && temp.is_some_and(|t| t > hot_temp)),
                readout(s.cpu, Box::new(|s| s.cpu.map(f64::from))),
                Block::Facts { rows: facts, gap: 10.0 },
            ];
            let threads = scene.seen.threads;
            if full && on("threads") && threads > 0 {
                let load = |i: usize| s.threads.get(i).copied().flatten();
                blocks.push(Block::Threads((0..threads).map(|i| (load(i).map(|load| load / 100.0), load(i).is_some_and(|load| load > hot_load))).collect()));
            }
            blocks
        }
        _ if id.starts_with("gpu:") => {
            let index = info.gpu_of(id)?;
            let gpu = &info.gpus[index];
            let had = scene.seen.gpus.get(index).filter(|seen| seen.present)?;
            let reading = s.gpus.get(index);
            let shows_temp = on("temp") && had.temp;
            let temp = reading.and_then(|g| g.temp).filter(|_| shows_temp);
            let aside = if shows_temp { shown(temp, celsius) } else { String::new() };
            let usage = reading.and_then(|g| g.usage);
            let mut blocks = vec![
                head("GPU", &gpu.name, aside, temp.is_some_and(|t| t > hot_temp)),
                readout(usage, Box::new(move |s| s.gpus.get(index)?.usage.map(f64::from))),
            ];
            if on("vram") {
                let used = reading.and_then(|g| g.mem_used);
                blocks.push(Block::Meter {
                    label: lang.pick("显存", "VRAM").into(),
                    fraction: used.map_or(0.0, |used| used as f32 / gpu.mem_total.max(1) as f32),
                    value: shown(used, |used| text::usage(used, gpu.mem_total)),
                    hot: false,
                    gap: 8.0,
                });
            }
            if full && on("engines") {
                // An engine the counters no longer list has nothing running
                // on it; with the engines unread, none is known.
                const SHOWN: [&str; 6] = ["3D", "Copy", "VideoDecode", "VideoEncode", "VideoCodec", "Compute"];
                for kind in had.engines.iter().map(String::as_str).filter(|kind| SHOWN.contains(kind)) {
                    let load = reading.and_then(|g| g.engines.as_ref()).map(|engines| engines.iter().find(|(known, _)| known == kind).map_or(0.0, |(_, load)| *load));
                    blocks.push(Block::Meter {
                        label: lang.name(kind),
                        fraction: load.unwrap_or(0.0) / 100.0,
                        value: shown(load, text::percent),
                        hot: load.is_some_and(|load| load > hot_load),
                        gap: 6.0,
                    });
                }
            }
            let mut facts = Vec::new();
            if on("clock") && had.clock {
                facts.push((lang.pick("频率", "Clock").into(), shown(reading.and_then(|g| g.clock_mhz), |clock| format!("{clock:.0} MHz")), false));
            }
            if on("power") && had.power {
                facts.push((lang.pick("功耗", "Power").into(), shown(reading.and_then(|g| g.power), |power| format!("{power:.1} W")), false));
            }
            if on("fan") && had.fan {
                facts.push((lang.pick("风扇", "Fan").into(), shown(reading.and_then(|g| g.fan_rpm), |rpm| format!("{rpm} RPM")), false));
            }
            // Memory the card borrows from the system's, where it has its own
            // besides (not where all of it is the system's, as on a Mac).
            if on("shared") && gpu.shared_total > 0 {
                facts.push((lang.pick("共享显存", "Shared").into(), shown(reading.and_then(|g| g.shared_used), |used| text::usage(used, gpu.shared_total)), false));
            }
            if full {
                blocks.push(Block::Facts { rows: facts, gap: 10.0 });
            }
            blocks
        }
        "memory" => {
            let total = info.mem_total.max(1) as f64;
            let percent = (s.memory.used as f64 / total * 100.0) as f32;
            let mut facts = Vec::new();
            // Each module's own temperature, as each chiplet's on the CPU lane.
            for i in 0..if on("dimms") { scene.seen.dimms } else { 0 } {
                let name = if lang == Lang::Zh { format!("内存条 {}", i + 1) } else { format!("Module {}", i + 1) };
                let value = s.dimm_temps.get(i).copied();
                facts.push((name, shown(value, celsius), value.is_some_and(|v| v > hot_temp)));
            }
            if on("committed") {
                facts.push((lang.pick("已提交", "Committed").into(), text::usage(s.memory.committed, s.memory.commit_limit), false));
            }
            if on("cached") {
                facts.push((lang.pick("缓存", "Cached").into(), text::size(s.memory.cached), false));
            }
            if !full {
                facts.clear();
            }
            vec![
                head(lang.pick("内存", "Memory"), info.memory_modules.clone().unwrap_or_default(), text::usage(s.memory.used, info.mem_total), false),
                Block::Readout {
                    figure: format!("{percent:.0}"),
                    unit: "%",
                    hot: percent > hot_load,
                    plot: chart.then(|| Plot::new(scene, vec![Box::new(move |s| Some(s.memory.used as f64 / total * 100.0))], Some(100.0), Some(hot_load as f64))),
                },
                Block::Facts { rows: facts, gap: 10.0 },
            ]
        }
        "network" => {
            let bits = prefs.network.bits;
            let plot = Plot::new(scene, vec![Box::new(|s| s.net_down), Box::new(|s| s.net_up)], None, None);
            let scale = plot.max;
            let mut facts = Vec::new();
            let adapter = s.network.as_ref();
            if on("adapter") {
                facts.push((lang.pick("网卡", "Adapter").into(), adapter.map_or(lang.pick("未连接", "Not connected").into(), |a| a.name.clone()), false));
            }
            if on("address") && scene.seen.address {
                facts.push((lang.pick("地址", "Address").into(), shown(adapter.and_then(|a| a.ipv4.clone()), |ip| ip), false));
            }
            let link = |s: &Sample| s.network.as_ref().map(|a| a.link_bps).filter(|bps| *bps > 0);
            if on("link") && scene.seen.link {
                facts.push((lang.pick("链路", "Link").into(), shown(link(s), text::link_speed), false));
            }
            if on("totals") {
                let (down, up) = (text::bytes(s.net_total_down as f64), text::bytes(s.net_total_up as f64));
                let total = if lang == Lang::Zh { format!("下载 {down}，上传 {up}") } else { format!("{down} down, {up} up") };
                facts.push((lang.pick("开机以来", "Since boot").into(), total, false));
            }
            if !full {
                facts.clear();
            }
            vec![
                // The adapter traffic leaves by now, which may have changed.
                // The chart's scale goes with the chart.
                head(lang.pick("网络", "Network"), s.network.as_ref().map(|a| a.model.clone()).unwrap_or_default(), if chart { full_scale_text(lang, scale, bits) } else { String::new() }, false),
                Block::Rates {
                    rows: vec![
                        (lang.pick("下载", "Down").into(), shown(s.net_down, |rate| text::rate(rate, bits))),
                        (lang.pick("上传", "Up").into(), shown(s.net_up, |rate| text::rate(rate, bits))),
                    ],
                    plot: chart.then_some(plot),
                },
                Block::Facts { rows: facts, gap: 10.0 },
            ]
        }
        "disk" => {
            let plot = Plot::new(scene, vec![Box::new(|s| s.disk_read), Box::new(|s| s.disk_write)], None, None);
            let scale = plot.max;
            let drives: &[(u32, String)] = if on("drives") { &scene.seen.drives } else { &[] };
            let temp = |id: u32| s.drive_temps.iter().find(|d| d.id == id).map(|d| d.celsius);
            let hottest = drives.iter().filter_map(|(id, _)| temp(*id)).fold(None, |max: Option<f32>, t| Some(max.map_or(t, |m| m.max(t))));
            // A drive's temperature takes the corner, as a GPU's does, and the
            // chart's scale moves below.
            let mut facts: Vec<(String, String, bool)> = Vec::new();
            if drives.len() > 1 {
                for (id, name) in drives {
                    let value = temp(*id);
                    facts.push((name.clone(), shown(value, celsius), value.is_some_and(|t| t > hot_temp)));
                }
            }
            if on("active") {
                facts.push((lang.pick("活动时间", "Active time").into(), shown(s.disk_active, text::percent), false));
            }
            // The chart's scale goes with the chart.
            let aside = if drives.is_empty() {
                if chart { full_scale_text(lang, scale, false) } else { String::new() }
            } else {
                if chart {
                    facts.push((lang.pick("满刻度", "Scale").into(), text::rate(scale, false), false));
                }
                shown(hottest, celsius)
            };
            if !full {
                facts.clear();
            }
            vec![
                head(lang.pick("磁盘", "Disk"), info.drives.join(", "), aside, hottest.is_some_and(|t| t > hot_temp)),
                Block::Rates {
                    rows: vec![
                        (lang.pick("读取", "Read").into(), shown(s.disk_read, |rate| text::rate(rate, false))),
                        (lang.pick("写入", "Write").into(), shown(s.disk_write, |rate| text::rate(rate, false))),
                    ],
                    plot: chart.then_some(plot),
                },
                Block::Facts { rows: facts, gap: 10.0 },
            ]
        }
        "processes" => {
            let sort = prefs.processes.sort;
            let mut ranked: Vec<&ProcessSample> = s.processes.iter().collect();
            ranked.sort_by(|a, b| sort_value(b, sort).total_cmp(&sort_value(a, sort)));
            vec![head(lang.pick("进程", "Processes"), "", "", false), Block::Table {
                headings: vec![
                    ("CPU".into(), ProcessSort::Cpu),
                    (lang.pick("内存", "Memory").into(), ProcessSort::Memory),
                    (lang.pick("读写", "I/O").into(), ProcessSort::Io),
                    ("GPU".into(), ProcessSort::Gpu),
                ],
                sort,
                rows: ranked
                    .iter()
                    .map(|p| [p.name.clone(), text::percent(p.cpu), text::size(p.mem), text::rate(p.io, false), shown(p.gpu, text::percent)])
                    .collect(),
                visible: if full { prefs.processes.count } else { prefs.processes.count.min(COMPACT_PROCESSES) },
            }]
        }
        "storage" => {
            let mut blocks = vec![head(lang.pick("存储", "Storage"), "", "", false)];
            // The drives switched on (see `Prefs::shows`); none, no lane.
            let drives: Vec<&String> = scene.seen.volumes.iter().filter(|name| on(&format!("volumes:{name}"))).collect();
            if drives.is_empty() {
                return None;
            }
            for (i, name) in drives.into_iter().enumerate() {
                let volume = s.volumes.iter().find(|v| v.name == *name);
                let fraction = volume.map_or(0.0, |v| v.used as f32 / v.total.max(1) as f32);
                blocks.push(Block::Meter {
                    label: name.clone(),
                    fraction,
                    value: shown(volume, |v| text::usage(v.used, v.total)),
                    hot: fraction * 100.0 > hot_load,
                    gap: if i == 0 { 0.0 } else { 8.0 },
                });
            }
            blocks
        }
        "board" => {
            let had = scene.seen.board.as_ref()?;
            let board = s.board.as_ref();
            let named = |name: &str, group: &str| board_sensor(name, group, lang);
            // Each sensor shown as it is switched (see `Prefs::shows`).
            let temps: Vec<&String> = had.temps.iter().filter(|n| on(&format!("temps:{n}"))).collect();
            let fans: Vec<&String> = had.fans.iter().filter(|n| on(&format!("fans:{n}"))).collect();
            let value = |list: Option<&Vec<(String, f32)>>, name: &str| list.and_then(|list| list.iter().find(|(n, _)| n == name)).map(|(_, v)| *v);
            vec![
                head(lang.pick("主板", "Motherboard"), &info.board, "", false),
                Block::Facts {
                    rows: temps
                        .iter()
                        .map(|n| {
                            let t = value(board.map(|b| &b.temps), n);
                            (named(n, "temps"), shown(t, celsius), t.is_some_and(|t| t > hot_temp))
                        })
                        .collect(),
                    gap: 0.0,
                },
                Block::Facts {
                    rows: fans
                        .iter()
                        .filter(|_| full)
                        .map(|n| (named(n, "fans"), shown(value(board.map(|b| &b.fans), n), |rpm| format!("{rpm:.0} RPM")), false))
                        .collect(),
                    gap: 10.0,
                },
            ]
        }
        "game" => {
            // A game played, or asked for by hand (waiting for its frames).
            let by_hand = !scene.buttons;
            if !(scene.seen.game || by_hand && front_is_game()) || (by_hand && game_hidden()) {
                return None;
            }
            // In the panel, only a game found (one taken for a game by hand is
            // the widgets' own).
            let panel = scene.buttons;
            let game = found_game(s, panel);
            let pid = game.map(|g| g.pid);
            // Its screen's refresh rate in the corner: the most frames it can show.
            let refresh = game.and_then(|g| g.refresh_hz);
            // A stutter: a frame taking as long as three at the screen's rate.
            let stutter = refresh.map(|hz| 3000.0 / hz as f64);
            let longest = game.map(|g| g.longest_ms);
            let frametimes = on("frametimes");
            let mut facts = Vec::new();
            if on("low") {
                facts.push(("1% low".into(), shown(game.and_then(|g| g.low), |low| format!("{low:.0} FPS")), false));
            }
            // Beside its chart, with that on.
            if on("longest") && !frametimes {
                let hot = longest.zip(stutter).is_some_and(|(ms, stutter)| ms as f64 > stutter);
                facts.push((lang.pick("最长一帧", "Longest frame").into(), shown(longest, |ms| format!("{ms:.1} ms")), hot));
            }
            if on("usage") {
                let cpu = shown(game.and_then(|g| g.cpu), text::percent);
                let gpu = shown(game.and_then(|g| g.gpu), text::percent);
                facts.push((lang.pick("占用", "Use").into(), format!("CPU {cpu} · GPU {gpu}"), false));
            }
            if on("memory") {
                let memory = shown(game.and_then(|g| g.mem), text::size);
                let vram = shown(game.and_then(|g| g.vram), text::size);
                let value = if lang == Lang::Zh { format!("{memory} · 显存 {vram}") } else { format!("{memory} · VRAM {vram}") };
                facts.push((lang.pick("内存", "Memory").into(), value, false));
            }
            if on("limit") && scene.seen.game_limit {
                let limit = game.and_then(|g| g.gpu_limit);
                let value = shown(limit, |limit| {
                    match limit {
                        GpuLimit::Free => lang.pick("未受限", "Not limited"),
                        GpuLimit::Power => lang.pick("功耗墙", "Power limit"),
                        GpuLimit::Thermal => lang.pick("温度墙", "Thermal limit"),
                        GpuLimit::Hardware => lang.pick("硬件降频", "Slowed by board"),
                    }
                    .into()
                });
                facts.push((lang.pick("显卡", "GPU").into(), value, limit.is_some_and(|limit| matches!(limit, GpuLimit::Thermal | GpuLimit::Hardware))));
            }
            if on("time") {
                facts.push((lang.pick("已玩", "Played").into(), shown(game.map(|g| g.playing_s), |s| lang.duration(s)), false));
            }
            if on("mic") && scene.seen.mic {
                let muted = s.mic_muted;
                let value = shown(muted, |muted| if muted { lang.pick("已静音", "Muted") } else { lang.pick("开启", "On") }.into());
                facts.push((lang.pick("麦克风", "Microphone").into(), value, muted == Some(true)));
            }
            // Full scale: the screen's rate, or the most frames drawn, if more
            // (a game not held to the screen's rate draws frames it never shows).
            let most = scene.history.iter().filter_map(|s| same_game(s, panel, pid).map(|g| g.fps as f64)).fold(0.0, f64::max);
            let scale = refresh.map(f64::from).map(|hz| hz.max(most));
            let mut blocks = vec![
                head(lang.pick("游戏", "Game"), game.map_or_else(|| if by_hand { waiting(lang) } else { String::new() }, |g| g.name.clone()), shown(refresh, |hz| lang.pick(&format!("屏幕 {hz} Hz"), &format!("Screen {hz} Hz")).to_string()), false),
                Block::Readout {
                    figure: shown(game.map(|g| g.fps), |fps| format!("{fps:.0}")),
                    unit: "FPS",
                    hot: false,
                    plot: chart.then(|| Plot::new(scene, vec![Box::new(move |s| same_game(s, panel, pid).map(|g| g.fps as f64))], scale, None)),
                },
            ];
            if !full {
                facts.clear();
            }
            if full && frametimes {
                // Full scale: two frames at the screen's rate, or the longest
                // frame on screen, if longer, rounded up.
                let series: Series = Box::new(move |s| same_game(s, panel, pid).map(|g| g.longest_ms as f64));
                let floor = stutter.map_or(MIN_FRAME_SCALE, |stutter| (stutter * 2.0 / 3.0).max(MIN_FRAME_SCALE));
                let peak = visible_peak(scene, &series).max(floor);
                blocks.push(Block::Trace {
                    label: lang.pick("帧时间", "Frame time").into(),
                    value: shown(longest, |ms| format!("{ms:.1} ms")),
                    hot: longest.zip(stutter).is_some_and(|(ms, stutter)| ms as f64 > stutter),
                    plot: Plot { series: vec![series], max: round_up(peak), hot: stutter },
                    gap: 10.0,
                });
            }
            blocks.push(Block::Facts { rows: facts, gap: 10.0 });
            blocks
        }
        "battery" => {
            if !scene.seen.battery {
                return None;
            }
            let battery = s.battery.as_ref();
            let state = match battery {
                Some(battery) if battery.charging => lang.pick("正在充电", "Charging").to_string(),
                Some(battery) => match battery.seconds_left {
                    Some(left) => {
                        let left = lang.duration(left as u64);
                        if lang == Lang::Zh { format!("剩余 {left}") } else { format!("{left} left") }
                    }
                    None => lang.pick("使用电池", "On battery").into(),
                },
                None => UNREAD.into(),
            };
            // In, charging; out, what the whole machine draws.
            let power = |watts: f32| match watts {
                w if w > 0.05 => format!("{} {w:.1} W", lang.pick("充电", "In")),
                w if w < -0.05 => format!("{} {:.1} W", lang.pick("耗电", "Out"), -w),
                _ => "0 W".to_string(),
            };
            let mut facts = Vec::new();
            if scene.seen.battery_power && on("power") {
                facts.push((lang.pick("功率", "Power").to_string(), shown(battery.and_then(|b| b.watts), power), false));
            }
            if scene.seen.battery_health && on("health") {
                facts.push((lang.pick("健康度", "Health").to_string(), shown(battery.and_then(|b| b.health), |h| format!("{h:.0}%")), false));
            }
            if !full {
                facts.clear();
            }
            vec![
                head(lang.pick("电池", "Battery"), "", state, false),
                Block::Readout {
                    figure: shown(battery, |b| b.percent.to_string()),
                    unit: "%",
                    hot: battery.is_some_and(|b| !b.charging && b.percent <= 20),
                    plot: chart.then(|| Plot::new(scene, vec![Box::new(|s| s.battery.as_ref().map(|b| b.percent as f64))], Some(100.0), None)),
                },
                Block::Facts { rows: facts, gap: 8.0 },
            ]
        }
        "wsl" => {
            use crate::reading::WslSample;
            // What it is doing at the top; a machine stopped or not there
            // keeps its rows, unread, so the lane does not change size.
            let (state, distros, cpu, used, total) = match &s.wsl {
                Some(WslSample::Missing) => (lang.pick("未安装", "Not installed"), None, None, None, None),
                Some(WslSample::Stopped) => (lang.pick("已停止", "Stopped"), None, None, None, None),
                Some(WslSample::Running { distros, cpu, used, total }) => (lang.pick("运行中", "Running"), distros.as_ref(), *cpu, *used, *total),
                None => (UNREAD, None, None, None, None),
            };
            let names = match distros {
                Some(names) if names.is_empty() => lang.pick("没有发行版在运行", "No distribution running").to_string(),
                Some(names) => names.join(", "),
                None => String::new(),
            };
            let mut blocks = vec![
                head("WSL", if on("distros") { names } else { String::new() }, state, false),
                readout(cpu, Box::new(|s| match &s.wsl {
                    Some(crate::reading::WslSample::Running { cpu, .. }) => cpu.map(f64::from),
                    _ => None,
                })),
            ];
            if on("memory") {
                let both = used.zip(total);
                blocks.push(Block::Meter {
                    label: lang.pick("内存", "Memory").into(),
                    fraction: both.map_or(0.0, |(used, total)| used as f32 / total.max(1) as f32),
                    value: shown(both, |(used, total)| text::usage(used, total)),
                    hot: false,
                    gap: 8.0,
                });
            }
            blocks
        }
        "system" => {
            let rows = [
                ("uptime", lang.pick("开机时长", "Uptime"), lang.duration(s.system.uptime_s)),
                ("processes", lang.pick("进程", "Processes"), s.system.processes.to_string()),
                ("threads", lang.pick("线程", "Threads"), s.system.threads.to_string()),
                ("handles", lang.pick("句柄", "Handles"), s.system.handles.to_string()),
            ];
            vec![
                head(lang.pick("系统", "System"), "", "", false),
                // Past `Full`, the first of them alone.
                Block::Facts {
                    rows: rows
                        .into_iter()
                        .filter(|(item, ..)| on(item))
                        .map(|(_, label, value)| (label.into(), value, false))
                        .take(if full { usize::MAX } else { 1 })
                        .collect(),
                    gap: 0.0,
                },
            ]
        }
        _ => return None,
    })
}

fn sort_value(p: &ProcessSample, sort: ProcessSort) -> f64 {
    match sort {
        ProcessSort::Cpu => p.cpu as f64,
        ProcessSort::Memory => p.mem as f64,
        ProcessSort::Io => p.io,
        // Not read sorts last.
        ProcessSort::Gpu => p.gpu.map_or(-1.0, f64::from),
    }
}

fn full_scale_text(lang: Lang, scale: f64, bits: bool) -> String {
    let value = text::rate(scale, bits);
    if lang == Lang::Zh { format!("满刻度 {value}") } else { format!("Scale {value}") }
}

/// Rounds a byte rate up to 1, 2 or 5 times a power of ten of its unit.
fn round_up_rate(value: f64) -> f64 {
    let unit = 1024f64.powf((value.ln() / 1024f64.ln()).floor());
    let magnitude = 10f64.powf((value / unit).log10().floor());
    let leading = value / unit / magnitude;
    let step = if leading <= 1.0 { 1.0 } else if leading <= 2.0 { 2.0 } else if leading <= 5.0 { 5.0 } else { 10.0 };
    step * magnitude * unit
}

/// The most `series` reads in the samples on screen, or 0.
fn visible_peak(scene: &Scene, series: &Series) -> f64 {
    let oldest = scene.pen_ms - scene.prefs.chart_seconds * 1000.0;
    scene.history.iter().filter(|s| s.t as f64 >= oldest).filter_map(series).fold(0.0, f64::max)
}

/// Rounds `value` (above 0) up to 1, 2 or 5 times a power of ten.
fn round_up(value: f64) -> f64 {
    let magnitude = 10f64.powf(value.log10().floor());
    let leading = value / magnitude;
    let step = if leading <= 1.0 { 1.0 } else if leading <= 2.0 { 2.0 } else if leading <= 5.0 { 5.0 } else { 10.0 };
    step * magnitude
}

fn full_scale(scene: &Scene, series: &[Series]) -> f64 {
    let oldest = scene.pen_ms - scene.prefs.chart_seconds * 1000.0;
    let peak = scene
        .history
        .iter()
        .filter(|s| s.t as f64 >= oldest)
        .flat_map(|s| series.iter().filter_map(move |read| read(s)))
        .fold(MIN_RATE_SCALE, f64::max);
    round_up_rate(peak)
}

/// Where each column starts, and the tallest column's height, when the
/// lanes, in order and `gap` apart, are split into `columns` so the tallest
/// is as short as it can be.
pub fn split(heights: &[f32], columns: usize, gap: f32) -> (Vec<usize>, f32) {
    let n = heights.len();
    // With every module switched off, there is only the bar.
    if n == 0 {
        return (vec![0; columns], 0.0);
    }
    let span = |from: usize, to: usize| heights[from..to].iter().sum::<f32>() + (to - from).saturating_sub(1) as f32 * gap;
    let mut best = vec![vec![f32::INFINITY; n + 1]; columns + 1];
    let mut from = vec![vec![0usize; n + 1]; columns + 1];
    best[0][0] = 0.0;
    for k in 1..=columns {
        for i in k..=n {
            for j in (k - 1)..i {
                let candidate = best[k - 1][j].max(span(j, i));
                if candidate < best[k][i] {
                    best[k][i] = candidate;
                    from[k][i] = j;
                }
            }
        }
    }
    let mut cuts = vec![0; columns];
    let mut i = n;
    for k in (1..=columns).rev() {
        cuts[k - 1] = from[k][i];
        i = from[k][i];
    }
    (cuts, best[columns][n])
}

/// A rectangle in DIPs: left, top, width, height.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// The panel laid out: lanes dealt into columns, and the bar below them.
#[derive(Clone)]
pub struct Layout {
    pub columns: usize,
    pub cuts: Vec<usize>,
    pub heights: Vec<f32>,
    /// The tallest column; every column is drawn this tall.
    pub lanes_height: f32,
    outer: (f32, f32, f32),
    lane_gap: f32,
    column_gap: f32,
    bar_gap: f32,
    bar_height: f32,
    /// Each column's width: as designed, or wider (see `stretched`).
    column_width: f32,
    /// How much taller than its lanes need each column was made, for their
    /// charts to grow into (see `stretched`).
    grow: f32,
}

impl Layout {
    /// The fewest columns, up to `max_columns`, whose tallest fits in `room`
    /// DIPs (bar included); failing that, the most.
    pub fn new(heights: Vec<f32>, room: f32, max_columns: usize, theme: &Theme) -> Self {
        let most = max_columns.min(heights.len()).max(1);
        let chrome = theme.outer.0 + theme.outer.2 + theme.bar_gap + theme.bar_height;
        let mut chosen = (1, split(&heights, 1, theme.lane_gap));
        for columns in 1..=most {
            chosen = (columns, split(&heights, columns, theme.lane_gap));
            if chosen.1 .1 + chrome <= room {
                break;
            }
        }
        let (columns, (cuts, tallest)) = chosen;
        Layout::built(heights, columns, cuts, tallest, theme)
    }

    /// The lanes in exactly `columns` columns (at least one, and no more
    /// than there are lanes), however tall that makes them.
    pub fn with_columns(heights: Vec<f32>, columns: usize, theme: &Theme) -> Self {
        let columns = columns.min(heights.len()).max(1);
        let (cuts, tallest) = split(&heights, columns, theme.lane_gap);
        Layout::built(heights, columns, cuts, tallest, theme)
    }

    fn built(heights: Vec<f32>, columns: usize, cuts: Vec<usize>, tallest: f32, theme: &Theme) -> Self {
        Layout {
            columns,
            cuts,
            heights,
            lanes_height: tallest,
            outer: theme.outer,
            lane_gap: theme.lane_gap,
            column_gap: theme.column_gap,
            bar_gap: theme.bar_gap,
            bar_height: theme.bar_height,
            column_width: COLUMN_WIDTH,
            grow: 0.0,
        }
    }

    /// The same, `by` DIPs wider and taller: the width shared out among
    /// the columns, the height given to every column, for its charts (see
    /// `grows`).
    pub fn stretched(mut self, by: (f32, f32)) -> Self {
        self.column_width += by.0.max(0.0) / self.columns as f32;
        self.lanes_height += by.1.max(0.0);
        self.grow = by.1.max(0.0);
        self
    }

    /// How much each lane's charts may grow, in order of the lanes: its
    /// column's share of what the layout was stretched by.
    pub fn grows(&self) -> Vec<f32> {
        (0..self.columns)
            .flat_map(|column| {
                let start = self.cuts[column];
                let end = self.cuts.get(column + 1).copied().unwrap_or(self.heights.len());
                std::iter::repeat_n(self.grow / (end - start).max(1) as f32, end - start)
            })
            .collect()
    }

    pub fn width(&self) -> f32 {
        self.columns as f32 * self.column_width + (self.columns - 1) as f32 * self.column_gap
    }

    pub fn height(&self) -> f32 {
        self.outer.0 + self.lanes_height + self.outer.2 + self.bar_gap + self.bar_height
    }

    /// Each lane's box, in order. A shorter column's lanes share what is
    /// left over, so every column ends at the same line.
    pub fn lanes(&self) -> Vec<Rect> {
        let (top, side, _) = self.outer;
        let column_width = (self.width() - 2.0 * side - (self.columns - 1) as f32 * self.column_gap) / self.columns as f32;
        let mut boxes = Vec::with_capacity(self.heights.len());
        for column in 0..self.columns {
            let start = self.cuts[column];
            let end = self.cuts.get(column + 1).copied().unwrap_or(self.heights.len());
            let count = (end - start) as f32;
            let natural = self.heights[start..end].iter().sum::<f32>() + (count - 1.0).max(0.0) * self.lane_gap;
            let extra = if end > start { (self.lanes_height - natural) / count } else { 0.0 };
            let x = side + column as f32 * (column_width + self.column_gap);
            let mut y = top;
            for height in &self.heights[start..end] {
                boxes.push(Rect { x, y, w: column_width, h: height + extra });
                y += height + extra + self.lane_gap;
            }
        }
        boxes
    }

    pub fn bar(&self) -> Rect {
        let y = self.outer.0 + self.lanes_height + self.outer.2 + self.bar_gap;
        Rect { x: 0.0, y, w: self.width(), h: self.bar_height }
    }

    /// How wide a percentage chart is (rate charts are a little narrower).
    pub fn plot_width(&self, theme: &Theme) -> f32 {
        self.lanes().first().map_or(COLUMN_WIDTH, |lane| lane.w) - 2.0 * theme.pad_x - LABEL - LABEL_GAP
    }
}

/// Draws one pass over the panel with its top-left corner at the origin,
/// and returns where clicks and wheel turns land.
pub fn paint(frame: &dyn Canvas, scene: &Scene, lanes: &[Lane], layout: &Layout, pass: Pass) -> Vec<HitBox> {
    let theme = scene.theme;
    let mut hits = Vec::new();
    let boxes = layout.lanes();
    for ((lane, area), grow) in lanes.iter().zip(&boxes).zip(layout.grows()) {
        paint_lane(frame, scene, lane, *area, grow, pass, &mut hits);
        // Chart paper rules each lane off below; the last rule in a column
        // also sets off the bar.
        if pass == Pass::Content && theme.ruled {
            frame.fill(theme.rule, area.x, area.y + area.h - 1.0, area.w, 1.0);
        }
    }
    if pass == Pass::Content {
        if theme.ruled {
            for column in 1..layout.columns {
                let x = boxes[layout.cuts[column]].x;
                frame.fill(theme.rule, x, 0.0, 1.0, layout.lanes_height);
            }
        }
        paint_bar(frame, scene, layout, &mut hits);
    }
    hits
}

/// Draws `text` so that its baseline sits where a CSS line box `line` DIPs
/// tall, aligned to `bottom`, would put it.
#[allow(clippy::too_many_arguments)]
fn text_on_line(frame: &dyn Canvas, text: &str, font: Font, color: Color, x: f32, bottom: f32, line: f32, width: f32) {
    let (ascent, descent) = frame.baseline(font);
    // The glyphs' ascent and descent centred in the line box.
    let baseline = bottom - line / 2.0 + (ascent - descent) / 2.0;
    frame.text(text, font, color, x, baseline - ascent, width, Align::Start);
}

/// Draws `lane` in `area`, its charts `grow` DIPs taller between them (each
/// at most twice as tall as designed).
fn paint_lane(frame: &dyn Canvas, scene: &Scene, lane: &Lane, area: Rect, grow: f32, pass: Pass, hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    let ink = lane.ink;
    let left = area.x + theme.pad_x;
    let width = area.w - 2.0 * theme.pad_x;
    let plot_left = left + LABEL + LABEL_GAP;
    let plot_width = left + width - plot_left;
    let mut y = area.y + theme.pad_top;
    let charts = lane.blocks.iter().filter(|block| matches!(block, Block::Readout { plot: Some(_), .. } | Block::Rates { plot: Some(_), .. })).count();
    let taller = if charts > 0 { (grow / charts as f32).min(RATE_PLOT) } else { 0.0 };
    // Each part named by its lane and what it is, the small layouts naming
    // theirs alike (see `morph`): the CPU's figure is "cpu.value" in both.
    let id = &lane.id;
    let name = |part: &str| frame.key(&format!("{id}.{part}"));
    let mut kinds: Vec<&str> = Vec::new();
    for block in &lane.blocks {
        let kind = match block {
            Block::Head { .. } => "head",
            Block::Readout { .. } => "readout",
            Block::Rates { .. } => "rates",
            Block::Trace { .. } => "trace",
            Block::Threads(_) => "threads",
            Block::Meter { .. } => "meter",
            Block::Facts { .. } => "facts",
            Block::Table { .. } => "table",
        };
        let nth = kinds.iter().filter(|k| **k == kind).count();
        kinds.push(kind);
        let part = |what: &str| name(&format!("{kind}{nth}.{what}"));
        let charted = matches!(block, Block::Readout { plot: Some(_), .. } | Block::Rates { plot: Some(_), .. });
        let height = block.height() + if charted { taller } else { 0.0 };
        match (block, pass) {
            (Block::Readout { plot: Some(plot), .. }, Pass::Plots) => {
                name("chart");
                paint_plot(frame, scene, ink, plot, plot_left, y, plot_width, height);
            }
            (Block::Rates { plot: Some(plot), .. }, Pass::Plots) => {
                name("chart");
                paint_plot(frame, scene, ink, plot, plot_left + RATE_INDENT, y, plot_width - RATE_INDENT, height);
            }
            (Block::Trace { plot, gap, .. }, Pass::Plots) => {
                part("chart");
                paint_plot(frame, scene, ink, plot, plot_left, y + gap, plot_width, height - gap);
            }
            (_, Pass::Plots) => {}
            (Block::Head { title, device, aside, aside_hot }, Pass::Content) => {
                let title_width = frame.measure(title, theme.title);
                let row = y + (HEAD - theme.title.size) / 2.0 - 1.0;
                name("label");
                frame.text(title, theme.title, theme.text, left, row, width, Align::Start);
                let aside_width = if aside.is_empty() { 0.0 } else { frame.measure(aside, theme.body) };
                let device_left = left + title_width + 8.0;
                let device_width = width - title_width - 8.0 - if aside_width > 0.0 { aside_width + 8.0 } else { 0.0 };
                name("device");
                frame.text(device, theme.small, theme.text3, device_left, row + 1.0, device_width, Align::Start);
                let color = if *aside_hot { theme.signal } else { theme.text2 };
                name("detail");
                frame.text(aside, theme.body, color, left, row, width, Align::End);
            }
            (Block::Readout { figure, unit, hot, .. }, Pass::Content) => {
                let color = if *hot { theme.signal } else { theme.text };
                // The figure and its unit stand on the chart's baseline.
                let figure_width = frame.measure(figure, theme.figure);
                let bottom = y + height;
                name("value");
                text_on_line(frame, figure, theme.figure, color, left, bottom, theme.figure.size * theme.figure_line, LABEL);
                name("unit");
                text_on_line(frame, unit, theme.unit, theme.text2, left + figure_width + 3.0, bottom, theme.unit.size, 30.0);
            }
            (Block::Rates { rows, .. }, Pass::Content) => {
                for (i, (label, value)) in rows.iter().enumerate() {
                    let row_y = y + height - (rows.len() - i) as f32 * (LINE + 2.0) + 1.0;
                    // A stroke sample in front of each label says which trace it names.
                    let stroke = if i == 0 { (ink.trace, 2.0) } else { (ink.trace2, 1.0) };
                    let which = if i == 0 { "a" } else { "b" };
                    name(&format!("{which}.mark"));
                    frame.fill(stroke.0, left, row_y + LINE / 2.0, 10.0, stroke.1);
                    frame.text(label, theme.small, theme.text2, left + 15.0, row_y + 1.0, 40.0, Align::Start);
                    name(&format!("{which}.value"));
                    frame.text(value, theme.value, theme.text, left, row_y, LABEL + RATE_INDENT, Align::End);
                }
            }
            (Block::Trace { label, value, hot, .. }, Pass::Content) => {
                // The value on the chart's baseline, its name above it.
                let bottom = y + height;
                part("text");
                frame.text(label, theme.small, theme.text2, left, bottom - 2.0 * LINE - 2.0, LABEL, Align::Start);
                frame.text(value, theme.value, if *hot { theme.signal } else { theme.text }, left, bottom - LINE, LABEL, Align::Start);
            }
            (Block::Threads(cells), Pass::Content) => {
                part("cells");
                let gap = 2.0;
                let count = cells.len().max(1) as f32;
                let cell = (width - gap * (count - 1.0)) / count;
                let radius = theme.thread_radius;
                for (i, (load, hot)) in cells.iter().enumerate() {
                    // A thread not read leaves its cell empty.
                    let Some(load) = load else { continue };
                    let cx = left + i as f32 * (cell + gap);
                    let cy = y + 10.0;
                    frame.fill_rounded(theme.track, cx, cy, cell, 14.0, radius);
                    let filled = 14.0 * load.clamp(0.0, 1.0);
                    if filled > 0.0 {
                        // The load fills from the bottom, inside the cell's rounding.
                        frame.clip(cx, cy + 14.0 - filled, cell, filled);
                        frame.fill_rounded(if *hot { theme.signal } else { ink.trace }, cx, cy, cell, 14.0, radius);
                        frame.unclip();
                    }
                }
            }
            (Block::Meter { label, fraction, value, hot, gap }, Pass::Content) => {
                let row_y = y + gap;
                part("meter");
                frame.text(label, theme.small, theme.text2, left, row_y, LABEL, Align::Start);
                let value_width = frame.measure(value, theme.small);
                frame.text(value, theme.small, theme.text2, left, row_y, width, Align::End);
                let bar_width = left + width - value_width - LABEL_GAP - plot_left;
                let bar_y = row_y + (LINE - theme.meter_height) / 2.0;
                let (height, radius) = (theme.meter_height, theme.meter_radius);
                frame.fill_rounded(theme.track, plot_left, bar_y, bar_width, height, radius);
                let filled = bar_width * fraction.clamp(0.0, 1.0);
                if filled > 0.0 {
                    frame.clip(plot_left, bar_y, filled, height);
                    frame.fill_rounded(if *hot { theme.signal } else { ink.trace }, plot_left, bar_y, bar_width, height, radius);
                    frame.unclip();
                }
            }
            (Block::Facts { rows, gap }, Pass::Content) => {
                for (i, (label, value, hot)) in rows.iter().enumerate() {
                    let row_y = y + gap + i as f32 * (LINE + FACT_GAP);
                    part(&i.to_string());
                    frame.text(label, theme.small, theme.text2, left, row_y, LABEL, Align::Start);
                    let color = if *hot { theme.signal } else { theme.text };
                    frame.text(value, theme.small, color, plot_left, row_y, left + width - plot_left, Align::Start);
                }
            }
            (Block::Table { headings, sort, rows, visible }, Pass::Content) => {
                part("rows");
                // The headings share the lane's head row, after the title.
                let head_y = y - HEAD - HEAD_GAP;
                let columns = table_columns(left, width);
                // Every heading's ink centred on the title's: Chinese, from
                // another face than the Latin, and a heavier weight each sit
                // differently in their line boxes.
                let title_top = head_y + (HEAD - theme.title.size) / 2.0 - 1.0;
                let middle = |text: &str, font: Font| {
                    let (top, bottom) = frame.ink(text, font);
                    (top + bottom) / 2.0
                };
                let centre = title_top + middle(scene.lang.pick("进程", "Processes"), theme.title);
                for ((label, key), (cx, cw)) in headings.iter().zip(&columns[1..]) {
                    let chosen = key == sort;
                    let font = if chosen { Font { weight: 650.0, ..theme.small } } else { theme.small };
                    let lit = chosen || scene.hover == Some(Hit::Sort(*key));
                    frame.text(label, font, if lit { theme.text } else { theme.text2 }, *cx, centre - middle(label, font), *cw, Align::End);
                    hits.push((*cx, head_y, *cw, HEAD, Hit::Sort(*key)));
                }
                let list_height = *visible as f32 * TABLE_ROW;
                let max_scroll = (rows.len().saturating_sub(*visible)) as f32 * TABLE_ROW;
                hits.push((left, y, width, list_height, Hit::Processes(max_scroll.to_bits())));
                let scroll = scene.process_scroll.clamp(0.0, max_scroll);
                frame.clip(left, y, width, list_height);
                for (i, row) in rows.iter().enumerate() {
                    let row_y = y + i as f32 * TABLE_ROW - scroll;
                    if row_y + TABLE_ROW < y || row_y > y + list_height {
                        continue;
                    }
                    let text_y = row_y + (TABLE_ROW - LINE) / 2.0;
                    for (j, (cx, cw)) in columns.iter().enumerate() {
                        let (font, color, align) = if j == 0 { (theme.body, theme.text, Align::Start) } else { (theme.small, theme.text2, Align::End) };
                        frame.text(&row[j], font, color, *cx, text_y, *cw, align);
                    }
                }
                frame.unclip();
                // A thin thumb shows where in the list the view is.
                if max_scroll > 0.0 {
                    let thumb = (list_height * *visible as f32 / rows.len() as f32).max(12.0);
                    let at = y + (list_height - thumb) * scroll / max_scroll;
                    frame.fill_rounded(theme.text3.alpha(0.6), left + width + 6.0, at, 2.0, thumb, 1.0);
                }
            }
        }
        frame.key("");
        y += height;
    }
}

/// x and width of the name and the four measures of the process table.
fn table_columns(left: f32, width: f32) -> [(f32, f32); 5] {
    let widths = [40.0, 52.0, 62.0, 36.0];
    let gap = 6.0;
    let measures: f32 = widths.iter().sum::<f32>() + gap * widths.len() as f32;
    let mut columns = [(left, width - measures); 5];
    let mut x = left + width - measures + gap;
    for (i, w) in widths.iter().enumerate() {
        columns[i + 1] = (x, *w);
        x += w + gap;
    }
    columns
}

#[allow(clippy::too_many_arguments)]
fn paint_plot(frame: &dyn Canvas, scene: &Scene, ink: Ink, plot: &Plot, left: f32, top: f32, width: f32, height: f32) {
    let theme = scene.theme;
    let span = scene.prefs.chart_seconds * 1000.0;
    let pen = scene.pen_ms;
    let right = left + width;
    let bottom = top + height;
    let max = plot.max;
    let x = |t: f64| right - ((pen - t) / span) as f32 * width;
    let y = |value: f64| bottom - ((value / max).clamp(0.0, 1.0) as f32) * (height - 2.0);

    // Time rules travel with the paper.
    let rule_ms = span / 6.0;
    let mut t = (pen / rule_ms).floor() * rule_ms;
    while x(t) > left {
        frame.fill(theme.rule, x(t).round(), top, 1.0, height);
        t -= rule_ms;
    }
    frame.fill(theme.rule, left, bottom, width, 1.0);

    // One sample beyond each end, so the trace runs off both sides.
    let history = scene.history;
    let Some(first) = history.iter().position(|s| s.t as f64 >= pen - span) else { return };
    let visible = &history[first.saturating_sub(1)..];
    let last = visible.last().unwrap();
    frame.clip(left, top - 2.0, width, height + 2.0);
    for (order, read) in plot.series.iter().enumerate() {
        let mut points: Vec<Option<Point>> = visible.iter().map(|s| read(s).map(|value| Point { x: x(s.t as f64), y: y(value) })).collect();
        // The pen holds its position if the next sample is late.
        if (last.t as f64) < pen {
            points.push(read(last).map(|value| Point { x: right, y: y(value) }));
        }
        // A sample without the reading breaks the line: each stretch of
        // readings is drawn on its own, the last one to the pen.
        let stretches: Vec<&[Option<Point>]> = points.split(Option::is_none).collect();
        let last_stretch = stretches.len() - 1;
        for (i, stretch) in stretches.into_iter().enumerate().filter(|(_, stretch)| !stretch.is_empty()) {
            let stretch: Vec<Point> = stretch.iter().flatten().copied().collect();
            let to_pen = i == last_stretch;
            if order == 0 {
                let mut area = stretch.clone();
                let end = stretch.last().unwrap().x;
                area.push(Point { x: if to_pen { end.max(right) } else { end }, y: bottom });
                area.push(Point { x: stretch[0].x, y: bottom });
                // The wash under the trace, fading where the skin's fades.
                let wash = if ink.wash == ink.wash_end { Fill::Solid(ink.wash) } else { Fill::Down { top, from: ink.wash, bottom, to: ink.wash_end } };
                frame.fill_shape(&area, wash);
            }
            let (color, stroke) = if order == 0 { (ink.trace, 1.5) } else { (ink.trace2, 1.0) };
            frame.stroke(&stretch, color, stroke);
            if let (0, Some(hot)) = (order, plot.hot) {
                frame.clip(left, top - 2.0, width, y(hot) - top + 2.0);
                frame.stroke(&stretch, theme.signal, stroke);
                frame.unclip();
            }
        }
    }
    frame.unclip();

    // The pen tip sits at the moment being drawn.
    let read = &plot.series[0];
    let next = visible.iter().position(|s| s.t as f64 >= pen);
    let (before, after) = match next {
        Some(0) | None => (last, last),
        Some(i) => (&visible[i - 1], &visible[i]),
    };
    let progress = if after.t == before.t { 0.0 } else { (pen - before.t as f64) / (after.t - before.t) as f64 };
    // No tip in a gap: the line is not there to have one.
    let (Some(from), Some(to)) = (read(before), read(after)) else { return };
    let value = from + (to - from) * progress;
    let color = if plot.hot.is_some_and(|hot| value > hot) { theme.signal } else { ink.trace };
    frame.fill_circle(color, Point { x: right, y: y(value) }, 2.5);
}

fn paint_bar(frame: &dyn Canvas, scene: &Scene, layout: &Layout, hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    let bar = layout.bar();
    frame.key("bar");
    // Windows 11 sets the bar off as a slightly darker footer strip.
    if theme.skin == Skin::Fluent {
        frame.fill(theme.footer, bar.x, bar.y, bar.w, bar.h);
        frame.fill(theme.rule, bar.x, bar.y, bar.w, 1.0);
    }
    // Settings at the end, and the pin before it: glyphs of the system's
    // icon font, each centred in its button. A pinned pin stays lit.
    let top = bar.y + (bar.h - BUTTON) / 2.0;
    let settings = bar.x + bar.w - theme.bar_pad.1 - BUTTON;
    let pin = settings - BUTTON_GAP - BUTTON;
    let overlay = pin - BUTTON_GAP - BUTTON;
    let uptime = scene.lang.duration(scene.latest().system.uptime_s);
    if !scene.buttons {
        let label = if scene.lang == Lang::Zh { format!("已开机 {uptime}") } else { format!("Up {uptime}") };
        let start = bar.x + theme.bar_pad.0;
        frame.text(&label, theme.small, theme.text2, start, bar.y + (bar.h - LINE) / 2.0, bar.w - 2.0 * start, Align::Start);
        frame.key("");
        return;
    }
    let label = if scene.lang == Lang::Zh { format!("已开机 {uptime}") } else { format!("Up {uptime}") };
    let start = bar.x + theme.bar_pad.0;
    frame.text(&label, theme.small, theme.text2, start, bar.y + (bar.h - LINE) / 2.0, overlay - 8.0 - start, Align::Start);
    let pin_glyph = if scene.pinned { "\u{E840}" } else { "\u{E718}" };
    // The overlay's: a window with a trace in it, lit while it is on.
    let buttons = [(overlay, "\u{E9D9}", Hit::Overlay, scene.overlay), (pin, pin_glyph, Hit::Pin, scene.pinned), (settings, "\u{E713}", Hit::Settings, false)];
    for (left, glyph, hit, lit) in buttons {
        let hovered = scene.hover == Some(hit);
        if hovered || lit {
            frame.fill_rounded(theme.hover, left, top, BUTTON, BUTTON, theme.control_radius.min(BUTTON / 2.0));
        }
        let icon = Font::new(Family::Icons, 16.0, 400.0);
        let glyph_left = left + (BUTTON - frame.measure(glyph, icon)) / 2.0;
        frame.text(glyph, icon, if hovered || lit { theme.text } else { theme.text2 }, glyph_left, top + 8.0, BUTTON, Align::Start);
        hits.push((left, top, BUTTON, BUTTON, hit));
    }
    frame.key("");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_lanes_into_the_shortest_columns() {
        let heights = [100.0, 200.0, 50.0, 150.0];
        assert_eq!(split(&heights, 1, 0.0), (vec![0], 500.0));
        // [100, 200] | [50, 150] beats [100] | [200, 50, 150].
        assert_eq!(split(&heights, 2, 0.0), (vec![0, 2], 300.0));
        assert_eq!(split(&heights, 3, 0.0).1, 200.0);
        // Gaps count between the lanes of a column, not after the last.
        assert_eq!(split(&heights, 2, 8.0), (vec![0, 2], 308.0));
        assert_eq!(split(&[], 1, 8.0), (vec![0], 0.0));
    }

    #[test]
    fn lays_out_a_panel_with_no_lanes() {
        let theme = Theme::new(Skin::Paper, false);
        let layout = Layout::new(Vec::new(), 1000.0, 3, &theme);
        assert_eq!(layout.columns, 1);
        assert!(layout.lanes().is_empty());
        assert_eq!(layout.height(), theme.bar_height);
    }

    #[test]
    fn takes_the_fewest_columns_that_fit() {
        let theme = Theme::new(Skin::Paper, false);
        let bar = theme.bar_height;
        let heights = vec![300.0, 300.0, 300.0];
        assert_eq!(Layout::new(heights.clone(), 900.0 + bar, 3, &theme).columns, 1);
        assert_eq!(Layout::new(heights.clone(), 700.0, 3, &theme).columns, 2);
        // Too short for even the most columns: the most, to be zoomed out.
        assert_eq!(Layout::new(heights, 200.0, 2, &theme).columns, 2);
    }

    #[test]
    fn deals_lanes_into_level_columns() {
        let theme = Theme::new(Skin::Glass, false);
        let layout = Layout::new(vec![100.0, 200.0, 50.0, 150.0], 400.0, 2, &theme);
        let boxes = layout.lanes();
        assert_eq!(layout.columns, 2);
        assert_eq!(boxes[2].x, COLUMN_WIDTH + theme.column_gap);
        // Both columns end at the same line.
        let end = |r: &Rect| r.y + r.h;
        assert_eq!(end(&boxes[1]), end(&boxes[3]));
        assert_eq!(layout.width(), 2.0 * COLUMN_WIDTH + theme.column_gap);
    }

    fn sample(clock: Option<f32>, battery: Option<u8>, fan: Option<f32>) -> Sample {
        use crate::reading::*;
        Sample {
            t: 0,
            cpu: Some(10.0),
            threads: vec![Some(10.0); 4],
            ghz: Some(3.0),
            memory: MemorySample { used: 1 << 30, committed: 1 << 30, commit_limit: 1 << 31, cached: 0 },
            gpus: vec![GpuSample { usage: Some(5.0), engines: Some(Vec::new()), mem_used: Some(0), shared_used: Some(0), temp: Some(50.0), clock_mhz: clock, fan_rpm: None, power: None }],
            net_down: Some(0.0),
            net_up: Some(0.0),
            net_total_down: 0,
            net_total_up: 0,
            network: None,
            disk_read: Some(0.0),
            disk_write: Some(0.0),
            disk_active: Some(0.0),
            volumes: Vec::new(),
            processes: Vec::new(),
            system: SystemSample { uptime_s: 60, processes: 1, threads: 1, handles: 1 },
            battery: battery.map(|percent| BatterySample { percent, charging: false, seconds_left: None, watts: None, health: None }),
            cpu_sensors: None,
            board: fan.map(|rpm| BoardSensors { temps: Vec::new(), fans: vec![("1".into(), rpm)] }),
            drive_temps: Vec::new(),
            dimm_temps: Vec::new(),
            mic_muted: None,
            game: None,
            wsl: None,
        }
    }

    fn info() -> StaticInfo {
        use crate::reading::GpuInfo;
        StaticInfo {
            cpu_name: "CPU".into(),
            memory_modules: None,
            drives: Vec::new(),
            network_adapter: None,
            board: "Board".into(),
            threads: 4,
            mem_total: 1 << 32,
            gpus: vec![GpuInfo { slot: 0, name: "GPU".into(), mem_total: 1 << 30, shared_total: 0 }],
            found: Vec::new(),
        }
    }

    /// The facts of a lane: label and value of each row.
    fn facts(blocks: &[Block]) -> Vec<(String, String)> {
        blocks
            .iter()
            .filter_map(|block| match block {
                Block::Facts { rows, .. } => Some(rows.iter().map(|(label, value, _)| (label.clone(), value.clone()))),
                _ => None,
            })
            .flatten()
            .collect()
    }

    #[test]
    fn keeps_a_reading_in_place_while_it_is_missing() {
        let (info, prefs, theme) = (info(), Prefs::default(), Theme::new(Skin::Paper, false));
        let scene = |history: &'static [Sample]| Scene { info: &info, prefs: &prefs, theme: &theme, lang: Lang::En, history, seen: Box::leak(Box::new(Seen::of(history))), pen_ms: 0.0, process_scroll: 0.0, hover: None, pinned: false, overlay: false, buttons: true };
        let leak = |history: Vec<Sample>| -> &'static [Sample] { Box::leak(history.into_boxed_slice()) };
        // The GPU's clock, the battery and a fan read a moment ago, and not now.
        let history = leak(vec![sample(Some(1350.0), Some(80), Some(900.0)), sample(None, None, None)]);
        let gpu = lane(&scene(history), "gpu:0", Detail::Full).unwrap();
        assert!(facts(&gpu).contains(&("Clock".into(), UNREAD.into())));
        let battery = lane(&scene(history), "battery", Detail::Full).expect("the battery lane stays");
        assert!(matches!(&battery[1], Block::Readout { figure, .. } if figure == UNREAD));
        let board = lane(&scene(history), "board", Detail::Full).expect("the board lane stays");
        assert_eq!(facts(&board), vec![("Fan 1".to_string(), UNREAD.to_string())]);
        // Read again: shown.
        let history = leak(vec![sample(None, None, None), sample(Some(1350.0), Some(80), Some(900.0))]);
        assert!(facts(&lane(&scene(history), "gpu:0", Detail::Full).unwrap()).contains(&("Clock".into(), "1350 MHz".into())));
        // Never read in the span: no row, no lane.
        let history = leak(vec![sample(None, None, None), sample(None, None, None)]);
        assert!(!facts(&lane(&scene(history), "gpu:0", Detail::Full).unwrap()).iter().any(|(label, _)| label == "Clock"));
        assert!(lane(&scene(history), "battery", Detail::Full).is_none());
        assert!(lane(&scene(history), "board", Detail::Full).is_none());
    }

    #[test]
    fn tells_readings_apart_by_what_they_are_of() {
        use crate::reading::{CpuSensors, DriveTemperature};
        let (info, prefs, theme) = (info(), Prefs::default(), Theme::new(Skin::Paper, false));
        let scene = |history: &'static [Sample]| Scene { info: &info, prefs: &prefs, theme: &theme, lang: Lang::En, history, seen: Box::leak(Box::new(Seen::of(history))), pen_ms: 0.0, process_scroll: 0.0, hover: None, pinned: false, overlay: false, buttons: true };
        let leak = |history: Vec<Sample>| -> &'static [Sample] { Box::leak(history.into_boxed_slice()) };
        let with = |ccds: Vec<(usize, f32)>, drives: Vec<(u32, f32)>| {
            let mut s = sample(None, None, None);
            s.cpu_sensors = Some(CpuSensors { temp: Some(50.0), ccds, power: None });
            s.drive_temps = drives.into_iter().map(|(id, celsius)| DriveTemperature { id, name: "SSD".into(), celsius }).collect();
            s
        };
        // Two drives of one model: a row each.
        let history = leak(vec![with(Vec::new(), vec![(0, 35.0), (1, 95.0)])]);
        let disk = facts(&lane(&scene(history), "disk", Detail::Full).unwrap());
        assert_eq!(&disk[..2], [("SSD".to_string(), "35 °C".to_string()), ("SSD".to_string(), "95 °C".to_string())]);
        // The first chiplet missed this time: the second's reading stays its own.
        let history = leak(vec![with(vec![(0, 40.0), (1, 95.0)], Vec::new()), with(vec![(1, 95.0)], Vec::new())]);
        let cpu = facts(&lane(&scene(history), "cpu", Detail::Full).unwrap());
        assert!(cpu.contains(&("CCD 1".into(), UNREAD.into())));
        assert!(cpu.contains(&("CCD 2".into(), "95 °C".into())));
    }

    #[test]
    fn keeps_the_order_of_what_it_has_seen() {
        let fans = |names: &[&str]| {
            let mut s = sample(None, None, None);
            s.board = Some(crate::reading::BoardSensors { temps: Vec::new(), fans: names.iter().map(|n| (n.to_string(), 900.0)).collect() });
            s
        };
        let order = |history: Vec<Sample>| Seen::of(&history).board.unwrap().fans;
        // Fan 2 alone at first, then both: 1 goes before 2, as read.
        assert_eq!(order(vec![fans(&["2"]), fans(&["1", "2"])]), ["1", "2"]);
        // One missing later keeps its place, beside what came before it.
        assert_eq!(order(vec![fans(&["1", "2", "3"]), fans(&["1", "3"])]), ["1", "2", "3"]);
        assert_eq!(order(vec![fans(&["1", "2", "3"]), fans(&["2", "3"])]), ["1", "2", "3"]);
    }

    #[test]
    fn rounds_rates_up_to_a_clean_scale() {
        assert_eq!(round_up_rate(1500.0 * 1024.0), 2.0 * 1024.0 * 1024.0);
        assert_eq!(round_up_rate(30.0 * 1024.0), 50.0 * 1024.0);
        assert_eq!(round_up_rate(10.0 * 1024.0), 10.0 * 1024.0);
    }
}
