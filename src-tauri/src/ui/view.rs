//! The panel's content: each module as a lane of blocks, laid out in
//! columns, and drawn for a sample and the history before it.

use std::hash::{Hash, Hasher};

use windows::Win32::Graphics::Direct2D::Common::{D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_OPEN};
use windows::Win32::Graphics::Direct2D::{D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_ELLIPSE};
use windows_numerics::Vector2;

use super::gfx::{rect, Align, Color, Family, Font, Frame};
use super::prefs::{Prefs, ProcessSort};
use super::text::{self, Lang};
use super::theme::Theme;
use crate::metrics::{ProcessSample, Sample, StaticInfo};

/// A column's width, and the room around a lane's content (DIPs).
pub const COLUMN_WIDTH: f32 = 356.0;
const PAD_X: f32 = 18.0;
const PAD_Y: f32 = 12.0;
/// The label column of a lane, and the gap after it.
const LABEL: f32 = 96.0;
const LABEL_GAP: f32 = 12.0;
const HEAD: f32 = 17.0;
const HEAD_GAP: f32 = 8.0;
const PLOT: f32 = 40.0;
const RATE_PLOT: f32 = 36.0;
const LINE: f32 = 16.0;
const FACT_GAP: f32 = 3.0;
pub const TABLE_ROW: f32 = 22.0;
pub const BAR: f32 = 44.0;
/// How wide a percentage chart is; the charts of rates are a little narrower.
pub const PLOT_WIDTH: f32 = COLUMN_WIDTH - 2.0 * PAD_X - LABEL - LABEL_GAP;
/// Rates below this full scale are drawn against it, so idle chatter stays low.
const MIN_RATE_SCALE: f64 = 10.0 * 1024.0;

/// What a click or a wheel turn on the panel lands on.
#[derive(Clone, Copy, PartialEq)]
pub enum Hit {
    Settings,
    Sort(ProcessSort),
    /// The process list, and how far it scrolls.
    Processes(f32),
}

/// What the readings are drawn with: the moment, the data, and the choices.
pub struct Scene<'a> {
    pub info: &'a StaticInfo,
    pub prefs: &'a Prefs,
    pub theme: &'a Theme,
    pub lang: Lang,
    pub history: &'a [Sample],
    /// The pen's time: the chart runs a little behind the newest sample, so
    /// the next one has always arrived by the time it is drawn to.
    pub pen_ms: f64,
    pub process_scroll: f32,
}

impl Scene<'_> {
    fn latest(&self) -> &Sample {
        self.history.last().unwrap()
    }
}

/// A chart: series read from samples, drawn against a full scale.
struct Plot {
    series: Vec<Box<dyn Fn(&Sample) -> f64>>,
    max: f64,
    hot: Option<f64>,
}

impl Plot {
    /// A chart against `max`, or with `None`, against the busiest moment on screen.
    fn new(scene: &Scene, series: Vec<Box<dyn Fn(&Sample) -> f64>>, max: Option<f64>, hot: Option<f64>) -> Self {
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
    Readout { figure: String, unit: &'static str, hot: bool, plot: Plot },
    Rates { rows: Vec<(String, String)>, plot: Plot },
    Threads(Vec<(f32, bool)>),
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
            Block::Rates { rows, plot } => (rows, plot.max.to_bits()).hash(state),
            Block::Threads(cells) => cells.iter().for_each(|(load, hot)| (load.to_bits(), hot).hash(state)),
            Block::Meter { label, fraction, value, hot, gap } => (label, fraction.to_bits(), value, hot, gap.to_bits()).hash(state),
            Block::Facts { rows, gap } => (rows, gap.to_bits()).hash(state),
            Block::Table { headings, sort, rows, visible } => {
                headings.iter().for_each(|(label, key)| (label, *key as u8).hash(state));
                (*sort as u8, rows, visible).hash(state);
            }
        }
    }
}

#[derive(Hash)]
pub struct Lane {
    blocks: Vec<Block>,
}

impl Lane {
    pub fn height(&self) -> f32 {
        2.0 * PAD_Y + self.blocks.iter().map(Block::height).sum::<f32>()
    }
}

/// The lanes of the modules that are on and have something to show.
pub fn lanes(scene: &Scene) -> Vec<Lane> {
    scene
        .prefs
        .modules
        .iter()
        .filter(|entry| entry.on)
        .filter_map(|entry| lane(scene, &entry.id))
        .map(|blocks| Lane { blocks })
        .collect()
}

fn head(title: impl Into<String>, device: impl Into<String>, aside: impl Into<String>, aside_hot: bool) -> Block {
    Block::Head { title: title.into(), device: device.into(), aside: aside.into(), aside_hot }
}

fn celsius(value: f32) -> String {
    format!("{value:.0} °C")
}

fn lane(scene: &Scene, id: &str) -> Option<Vec<Block>> {
    let (s, prefs, lang, info) = (scene.latest(), scene.prefs, scene.lang, scene.info);
    let hot_load = prefs.hot_load;
    let hot_temp = prefs.hot_temp;
    let readout = |value: f32, read: Box<dyn Fn(&Sample) -> f64>| Block::Readout {
        figure: format!("{value:.0}"),
        unit: "%",
        hot: value > hot_load,
        plot: Plot::new(scene, vec![read], Some(100.0), Some(hot_load as f64)),
    };
    Some(match id {
        "cpu" => {
            let sensors = s.cpu_sensors.as_ref();
            let temp = sensors.and_then(|c| c.temp);
            let clock = prefs.cpu.clock.then(|| format!("{:.2} GHz", s.ghz));
            // The temperature takes the corner, as on the GPU lanes; the
            // clock, the power and each chiplet's temperature go below.
            let aside = temp.map(celsius).or(clock.clone()).unwrap_or_default();
            let mut facts = Vec::new();
            if let (Some(_), Some(clock)) = (temp, clock) {
                facts.push((lang.pick("频率", "Clock").into(), clock, false));
            }
            if let Some(power) = sensors.and_then(|c| c.power) {
                facts.push((lang.pick("功耗", "Power").into(), format!("{power:.1} W"), false));
            }
            if let Some(ccds) = sensors.map(|c| &c.ccds).filter(|ccds| ccds.len() > 1) {
                for (i, value) in ccds.iter().enumerate() {
                    facts.push((format!("CCD {}", i + 1), celsius(*value), *value > hot_temp));
                }
            }
            let mut blocks = vec![
                head("CPU", &info.cpu_name, aside, temp.is_some_and(|t| t > hot_temp)),
                readout(s.cpu, Box::new(|s| s.cpu as f64)),
                Block::Facts { rows: facts, gap: 10.0 },
            ];
            if prefs.cpu.threads {
                blocks.push(Block::Threads(s.threads.iter().map(|&load| (load / 100.0, load > hot_load)).collect()));
            }
            blocks
        }
        _ if id.starts_with("gpu:") => {
            let index: usize = id[4..].parse().ok()?;
            let (gpu, reading) = (info.gpus.get(index)?, s.gpus.get(index)?);
            let temp = reading.temp.filter(|_| prefs.gpu.sensors);
            let mut blocks = vec![
                head("GPU", &gpu.name, temp.map(celsius).unwrap_or_default(), temp.is_some_and(|t| t > hot_temp)),
                readout(reading.usage, Box::new(move |s| s.gpus.get(index).map_or(0.0, |g| g.usage as f64))),
            ];
            if prefs.gpu.memory {
                blocks.push(Block::Meter {
                    label: lang.pick("显存", "VRAM").into(),
                    fraction: reading.mem_used as f32 / gpu.mem_total.max(1) as f32,
                    value: text::usage(reading.mem_used, gpu.mem_total),
                    hot: false,
                    gap: 8.0,
                });
            }
            if prefs.gpu.engines {
                const SHOWN: [&str; 6] = ["3D", "Copy", "VideoDecode", "VideoEncode", "VideoCodec", "Compute"];
                for (kind, load) in reading.engines.iter().filter(|(kind, _)| SHOWN.contains(&kind.as_str())) {
                    blocks.push(Block::Meter {
                        label: lang.name(kind),
                        fraction: load / 100.0,
                        value: text::percent(*load),
                        hot: *load > hot_load,
                        gap: 6.0,
                    });
                }
            }
            let mut facts = Vec::new();
            if prefs.gpu.sensors {
                if let Some(clock) = reading.clock_mhz {
                    facts.push((lang.pick("频率", "Clock").into(), format!("{clock:.0} MHz"), false));
                }
                if let Some(rpm) = reading.fan_rpm {
                    facts.push((lang.pick("风扇", "Fan").into(), format!("{rpm} RPM"), false));
                }
                facts.push((lang.pick("共享显存", "Shared").into(), text::usage(reading.shared_used, gpu.shared_total), false));
            }
            blocks.push(Block::Facts { rows: facts, gap: 10.0 });
            blocks
        }
        "memory" => {
            let total = info.mem_total.max(1) as f64;
            let percent = (s.memory.used as f64 / total * 100.0) as f32;
            let mut facts = Vec::new();
            if prefs.memory.details {
                facts.push((lang.pick("已提交", "Committed").into(), text::usage(s.memory.committed, s.memory.commit_limit), false));
                facts.push((lang.pick("缓存", "Cached").into(), text::size(s.memory.cached), false));
            }
            vec![
                head(lang.pick("内存", "Memory"), "", text::usage(s.memory.used, info.mem_total), false),
                Block::Readout {
                    figure: format!("{percent:.0}"),
                    unit: "%",
                    hot: percent > hot_load,
                    plot: Plot::new(scene, vec![Box::new(move |s| s.memory.used as f64 / total * 100.0)], Some(100.0), Some(hot_load as f64)),
                },
                Block::Facts { rows: facts, gap: 10.0 },
            ]
        }
        "network" => {
            let bits = prefs.network.bits;
            let plot = Plot::new(scene, vec![Box::new(|s| s.net_down), Box::new(|s| s.net_up)], None, None);
            let scale = plot.max;
            let mut facts = Vec::new();
            if prefs.network.details {
                let adapter = s.network.as_ref();
                facts.push((lang.pick("网卡", "Adapter").into(), adapter.map_or(lang.pick("未连接", "Not connected").into(), |a| a.name.clone()), false));
                if let Some(ip) = adapter.and_then(|a| a.ipv4.clone()) {
                    facts.push((lang.pick("地址", "Address").into(), ip, false));
                }
                if let Some(link) = adapter.filter(|a| a.link_bps > 0) {
                    facts.push((lang.pick("链路", "Link").into(), text::link_speed(link.link_bps), false));
                }
                let (down, up) = (text::bytes(s.net_total_down as f64), text::bytes(s.net_total_up as f64));
                let total = if lang == Lang::Zh { format!("下载 {down}，上传 {up}") } else { format!("{down} down, {up} up") };
                facts.push((lang.pick("开机以来", "Since boot").into(), total, false));
            }
            vec![
                head(lang.pick("网络", "Network"), "", full_scale_text(lang, scale, bits), false),
                Block::Rates {
                    rows: vec![
                        (lang.pick("下载", "Down").into(), text::rate(s.net_down, bits)),
                        (lang.pick("上传", "Up").into(), text::rate(s.net_up, bits)),
                    ],
                    plot,
                },
                Block::Facts { rows: facts, gap: 10.0 },
            ]
        }
        "disk" => {
            let plot = Plot::new(scene, vec![Box::new(|s| s.disk_read), Box::new(|s| s.disk_write)], None, None);
            let scale = plot.max;
            let hottest = s.drive_temps.iter().map(|d| d.celsius).fold(None, |max: Option<f32>, t| Some(max.map_or(t, |m| m.max(t))));
            // A drive's temperature takes the corner, as a GPU's does, and the
            // chart's scale moves below.
            let mut facts: Vec<(String, String, bool)> = Vec::new();
            if s.drive_temps.len() > 1 {
                for drive in &s.drive_temps {
                    facts.push((drive.name.clone(), celsius(drive.celsius), drive.celsius > hot_temp));
                }
            }
            if prefs.disk.active {
                facts.push((lang.pick("活动时间", "Active time").into(), text::percent(s.disk_active), false));
            }
            let aside = match hottest {
                Some(t) => {
                    facts.push((lang.pick("满刻度", "Scale").into(), text::rate(scale, false), false));
                    celsius(t)
                }
                None => full_scale_text(lang, scale, false),
            };
            vec![
                head(lang.pick("磁盘", "Disk"), info.drives.join(", "), aside, hottest.is_some_and(|t| t > hot_temp)),
                Block::Rates {
                    rows: vec![
                        (lang.pick("读取", "Read").into(), text::rate(s.disk_read, false)),
                        (lang.pick("写入", "Write").into(), text::rate(s.disk_write, false)),
                    ],
                    plot,
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
                    .map(|p| [p.name.clone(), text::percent(p.cpu), text::size(p.mem), text::rate(p.io, false), text::percent(p.gpu)])
                    .collect(),
                visible: prefs.processes.count,
            }]
        }
        "storage" => {
            let mut blocks = vec![head(lang.pick("存储", "Storage"), "", "", false)];
            for (i, volume) in s.volumes.iter().enumerate() {
                let fraction = volume.used as f32 / volume.total.max(1) as f32;
                blocks.push(Block::Meter {
                    label: volume.name.clone(),
                    fraction,
                    value: text::usage(volume.used, volume.total),
                    hot: fraction * 100.0 > hot_load,
                    gap: if i == 0 { 0.0 } else { 8.0 },
                });
            }
            blocks
        }
        "board" => {
            let board = s.board.as_ref()?;
            let named = |name: &str, numbered: (&str, &str)| {
                if name.parse::<u32>().is_ok() { format!("{} {name}", lang.pick(numbered.0, numbered.1)) } else { lang.name(name) }
            };
            vec![
                head(lang.pick("主板", "Motherboard"), &info.board, "", false),
                Block::Facts {
                    rows: board.temps.iter().map(|(n, t)| (named(n, ("传感器", "Sensor")), celsius(*t), *t > hot_temp)).collect(),
                    gap: 0.0,
                },
                Block::Facts {
                    rows: board.fans.iter().map(|(n, rpm)| (named(n, ("风扇", "Fan")), format!("{rpm:.0} RPM"), false)).collect(),
                    gap: 10.0,
                },
            ]
        }
        "battery" => {
            let battery = s.battery.as_ref()?;
            let state = if battery.charging {
                lang.pick("正在充电", "Charging").to_string()
            } else {
                match battery.seconds_left {
                    Some(left) => {
                        let left = lang.duration(left as u64);
                        if lang == Lang::Zh { format!("剩余 {left}") } else { format!("{left} left") }
                    }
                    None => lang.pick("使用电池", "On battery").into(),
                }
            };
            vec![
                head(lang.pick("电池", "Battery"), "", state, false),
                Block::Readout {
                    figure: battery.percent.to_string(),
                    unit: "%",
                    hot: !battery.charging && battery.percent <= 20,
                    plot: Plot::new(scene, vec![Box::new(|s| s.battery.as_ref().map_or(0.0, |b| b.percent as f64))], Some(100.0), None),
                },
            ]
        }
        "system" => vec![
            head(lang.pick("系统", "System"), "", "", false),
            Block::Facts {
                rows: vec![
                    (lang.pick("开机时长", "Uptime").into(), lang.duration(s.system.uptime_s), false),
                    (lang.pick("进程", "Processes").into(), s.system.processes.to_string(), false),
                    (lang.pick("线程", "Threads").into(), s.system.threads.to_string(), false),
                    (lang.pick("句柄", "Handles").into(), s.system.handles.to_string(), false),
                ],
                gap: 0.0,
            },
        ],
        _ => return None,
    })
}

fn sort_value(p: &ProcessSample, sort: ProcessSort) -> f64 {
    match sort {
        ProcessSort::Cpu => p.cpu as f64,
        ProcessSort::Memory => p.mem as f64,
        ProcessSort::Io => p.io,
        ProcessSort::Gpu => p.gpu as f64,
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

fn full_scale(scene: &Scene, series: &[Box<dyn Fn(&Sample) -> f64>]) -> f64 {
    let oldest = scene.pen_ms - scene.prefs.chart_seconds * 1000.0;
    let peak = scene
        .history
        .iter()
        .filter(|s| s.t as f64 >= oldest)
        .flat_map(|s| series.iter().map(move |read| read(s)))
        .fold(MIN_RATE_SCALE, f64::max);
    round_up_rate(peak)
}

/// Where each column starts, and the tallest column's height, when the
/// lanes, in order, are split into `columns` so the tallest is as short as
/// it can be.
pub fn split(heights: &[f32], columns: usize) -> (Vec<usize>, f32) {
    let n = heights.len();
    let span = |from: usize, to: usize| heights[from..to].iter().sum::<f32>();
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

/// The panel laid out: lanes dealt into columns.
pub struct Layout {
    pub columns: usize,
    pub cuts: Vec<usize>,
    pub heights: Vec<f32>,
    /// The tallest column; every column is drawn this tall.
    pub lanes_height: f32,
}

impl Layout {
    /// The fewest columns, up to `max_columns`, whose tallest fits in `room`
    /// DIPs (bar included); failing that, the most.
    pub fn new(heights: Vec<f32>, room: f32, max_columns: usize) -> Self {
        let most = max_columns.min(heights.len()).max(1);
        let mut chosen = (1, split(&heights, 1));
        for columns in 1..=most {
            let result = split(&heights, columns);
            chosen = (columns, result);
            if chosen.1 .1 + BAR <= room {
                break;
            }
        }
        let (columns, (cuts, tallest)) = chosen;
        Layout { columns, cuts, heights, lanes_height: tallest }
    }

    pub fn width(&self) -> f32 {
        self.columns as f32 * COLUMN_WIDTH
    }

    pub fn height(&self) -> f32 {
        self.lanes_height + BAR
    }
}

/// Draws one pass over the panel with its top-left corner at the origin,
/// and returns where clicks and wheel turns land.
pub fn paint(frame: &Frame, scene: &Scene, lanes: &[Lane], layout: &Layout, pass: Pass) -> Vec<(f32, f32, f32, f32, Hit)> {
    let theme = scene.theme;
    let mut hits = Vec::new();
    for column in 0..layout.columns {
        let start = layout.cuts[column];
        let end = layout.cuts.get(column + 1).copied().unwrap_or(lanes.len());
        let x = column as f32 * COLUMN_WIDTH;
        // A shorter column's lanes share what is left, so columns end level.
        let natural: f32 = layout.heights[start..end].iter().sum();
        let extra = if end > start { (layout.lanes_height - natural) / (end - start) as f32 } else { 0.0 };
        let mut y = 0.0;
        for index in start..end {
            let height = layout.heights[index] + extra;
            paint_lane(frame, scene, &lanes[index], x, y, pass, &mut hits);
            if pass == Pass::Content {
                fill(frame, theme.rule, x, y + height - 1.0, COLUMN_WIDTH, 1.0);
            }
            y += height;
        }
        if column > 0 && pass == Pass::Content {
            fill(frame, theme.rule, x, 0.0, 1.0, layout.lanes_height);
        }
    }
    if pass == Pass::Content {
        paint_bar(frame, scene, layout, &mut hits);
    }
    hits
}

fn fill(frame: &Frame, color: Color, x: f32, y: f32, w: f32, h: f32) {
    unsafe { frame.dc.FillRectangle(&rect(x, y, w, h), frame.brush(color)) };
}

fn paint_lane(frame: &Frame, scene: &Scene, lane: &Lane, x: f32, top: f32, pass: Pass, hits: &mut Vec<(f32, f32, f32, f32, Hit)>) {
    let theme = scene.theme;
    let left = x + PAD_X;
    let width = COLUMN_WIDTH - 2.0 * PAD_X;
    let plot_left = left + LABEL + LABEL_GAP;
    let plot_width = PLOT_WIDTH;
    let mut y = top + PAD_Y;
    for block in &lane.blocks {
        let height = block.height();
        match (block, pass) {
            (Block::Readout { plot, .. }, Pass::Plots) => paint_plot(frame, scene, plot, plot_left, y, plot_width, height),
            (Block::Rates { plot, .. }, Pass::Plots) => paint_plot(frame, scene, plot, plot_left + 26.0, y, plot_width - 26.0, height),
            (_, Pass::Plots) => {}
            (Block::Head { title, device, aside, aside_hot }, Pass::Content) => {
                let title_width = frame.gfx.measure(title, theme.title);
                frame.text(title, theme.title, theme.text, left, y, width, Align::Start);
                let aside_width = if aside.is_empty() { 0.0 } else { frame.gfx.measure(aside, theme.body) };
                let device_left = left + title_width + 8.0;
                let device_width = width - title_width - 8.0 - if aside_width > 0.0 { aside_width + 8.0 } else { 0.0 };
                frame.text(device, theme.small, theme.text3, device_left, y + 1.0, device_width, Align::Start);
                let color = if *aside_hot { theme.signal } else { theme.text2 };
                frame.text(aside, theme.body, color, left, y, width, Align::End);
            }
            (Block::Readout { figure, unit, hot, .. }, Pass::Content) => {
                let color = if *hot { theme.signal } else { theme.text };
                // Figures sit on the plot's baseline.
                let figure_width = frame.gfx.measure(figure, theme.figure);
                let baseline = y + height;
                frame.text(figure, theme.figure, color, left, baseline - theme.figure.size * 1.08, LABEL, Align::Start);
                frame.text(unit, theme.unit, theme.text2, left + figure_width + 3.0, baseline - theme.unit.size * 1.2, 30.0, Align::Start);
            }
            (Block::Rates { rows, .. }, Pass::Content) => {
                for (i, (label, value)) in rows.iter().enumerate() {
                    let row_y = y + height - (rows.len() - i) as f32 * (LINE + 2.0) + 1.0;
                    let stroke = if i == 0 { (theme.trace, 2.0) } else { (theme.trace2, 1.0) };
                    fill(frame, stroke.0, left, row_y + LINE / 2.0, 10.0, stroke.1);
                    frame.text(label, theme.small, theme.text2, left + 15.0, row_y + 1.0, 40.0, Align::Start);
                    frame.text(value, theme.value, theme.text, left, row_y, LABEL + 26.0, Align::End);
                }
            }
            (Block::Threads(cells), Pass::Content) => {
                let gap = 2.0;
                let count = cells.len().max(1) as f32;
                let cell = (width - gap * (count - 1.0)) / count;
                for (i, (load, hot)) in cells.iter().enumerate() {
                    let cx = left + i as f32 * (cell + gap);
                    let cy = y + 10.0;
                    fill(frame, theme.track, cx, cy, cell, 14.0);
                    let filled = 14.0 * load.clamp(0.0, 1.0);
                    fill(frame, if *hot { theme.signal } else { theme.trace }, cx, cy + 14.0 - filled, cell, filled);
                }
            }
            (Block::Meter { label, fraction, value, hot, gap }, Pass::Content) => {
                let row_y = y + gap;
                frame.text(label, theme.small, theme.text2, left, row_y, LABEL, Align::Start);
                let value_width = frame.gfx.measure(value, theme.small);
                frame.text(value, theme.small, theme.text2, left, row_y, width, Align::End);
                let bar_left = plot_left;
                let bar_width = left + width - value_width - LABEL_GAP - bar_left;
                let bar_y = row_y + LINE / 2.0 - 1.5;
                fill(frame, theme.track, bar_left, bar_y, bar_width, 3.0);
                fill(frame, if *hot { theme.signal } else { theme.trace }, bar_left, bar_y, bar_width * fraction.clamp(0.0, 1.0), 3.0);
            }
            (Block::Facts { rows, gap }, Pass::Content) => {
                for (i, (label, value, hot)) in rows.iter().enumerate() {
                    let row_y = y + gap + i as f32 * (LINE + FACT_GAP);
                    frame.text(label, theme.small, theme.text2, left, row_y, LABEL, Align::Start);
                    let color = if *hot { theme.signal } else { theme.text };
                    frame.text(value, theme.small, color, plot_left, row_y, left + width - plot_left, Align::Start);
                }
            }
            (Block::Table { headings, sort, rows, visible }, Pass::Content) => {
                // The headings share the lane's head row, after the title.
                let head_y = y - HEAD - HEAD_GAP;
                let columns = table_columns(left, width);
                for ((label, key), (cx, cw)) in headings.iter().zip(&columns[1..]) {
                    let chosen = key == sort;
                    let font = if chosen { Font { weight: 650.0, ..theme.small } } else { theme.small };
                    frame.text(label, font, if chosen { theme.text } else { theme.text2 }, *cx, head_y + 1.0, *cw, Align::End);
                    hits.push((*cx, head_y, *cw, HEAD, Hit::Sort(*key)));
                }
                let list_height = *visible as f32 * TABLE_ROW;
                let max_scroll = (rows.len().saturating_sub(*visible)) as f32 * TABLE_ROW;
                hits.push((left, y, width, list_height, Hit::Processes(max_scroll)));
                let scroll = scene.process_scroll.clamp(0.0, max_scroll);
                unsafe {
                    frame.dc.PushAxisAlignedClip(&rect(left, y, width, list_height), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                }
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
                unsafe { frame.dc.PopAxisAlignedClip() };
                // A thin thumb shows where in the list the view is.
                if max_scroll > 0.0 {
                    let track = list_height;
                    let thumb = (track * *visible as f32 / rows.len() as f32).max(12.0);
                    let at = y + (track - thumb) * scroll / max_scroll;
                    fill(frame, theme.text3.alpha(0.6), left + width + 6.0, at, 2.0, thumb);
                }
            }
        }
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

fn paint_plot(frame: &Frame, scene: &Scene, plot: &Plot, left: f32, top: f32, width: f32, height: f32) {
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
        fill(frame, theme.rule, x(t).round(), top, 1.0, height);
        t -= rule_ms;
    }
    fill(frame, theme.rule, left, bottom, width, 1.0);

    // One sample beyond each end, so the trace runs off both sides.
    let history = scene.history;
    let Some(first) = history.iter().position(|s| s.t as f64 >= pen - span) else { return };
    let visible = &history[first.saturating_sub(1)..];
    let last = visible.last().unwrap();
    unsafe { frame.dc.PushAxisAlignedClip(&rect(left, top - 2.0, width, height + 2.0), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
    for (order, read) in plot.series.iter().enumerate() {
        let mut points: Vec<Vector2> = visible.iter().map(|s| Vector2 { X: x(s.t as f64), Y: y(read(s)) }).collect();
        // The pen holds its position if the next sample is late.
        if (last.t as f64) < pen {
            points.push(Vector2 { X: right, Y: y(read(last)) });
        }
        if order == 0 {
            let mut area = points.clone();
            area.push(Vector2 { X: points.last().unwrap().X.max(right), Y: bottom });
            area.push(Vector2 { X: points[0].X, Y: bottom });
            if let Ok(path) = polyline(frame, &area, true) {
                unsafe { frame.dc.FillGeometry(&path, frame.brush(theme.wash), None) };
            }
        }
        let Ok(path) = polyline(frame, &points, false) else { continue };
        let (color, stroke) = if order == 0 { (theme.trace, 1.5) } else { (theme.trace2, 1.0) };
        unsafe { frame.dc.DrawGeometry(&path, frame.brush(color), stroke, None) };
        if let (0, Some(hot)) = (order, plot.hot) {
            unsafe {
                frame.dc.PushAxisAlignedClip(&rect(left, top - 2.0, width, y(hot) - top + 2.0), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                frame.dc.DrawGeometry(&path, frame.brush(theme.signal), stroke, None);
                frame.dc.PopAxisAlignedClip();
            }
        }
    }
    unsafe { frame.dc.PopAxisAlignedClip() };

    // The pen tip sits at the moment being drawn.
    let read = &plot.series[0];
    let next = visible.iter().position(|s| s.t as f64 >= pen);
    let (before, after) = match next {
        Some(0) | None => (last, last),
        Some(i) => (&visible[i - 1], &visible[i]),
    };
    let progress = if after.t == before.t { 0.0 } else { (pen - before.t as f64) / (after.t - before.t) as f64 };
    let value = read(before) + (read(after) - read(before)) * progress;
    let color = if plot.hot.is_some_and(|hot| value > hot) { theme.signal } else { theme.trace };
    let dot = D2D1_ELLIPSE { point: Vector2 { X: right, Y: y(value) }, radiusX: 2.5, radiusY: 2.5 };
    unsafe { frame.dc.FillEllipse(&dot, frame.brush(color)) };
}

fn polyline(frame: &Frame, points: &[Vector2], closed: bool) -> windows::core::Result<windows::Win32::Graphics::Direct2D::ID2D1PathGeometry1> {
    let path = unsafe { frame.gfx.factory.CreatePathGeometry()? };
    let sink = unsafe { path.Open()? };
    unsafe {
        sink.BeginFigure(points[0], if closed { D2D1_FIGURE_BEGIN_FILLED } else { D2D1_FIGURE_BEGIN_HOLLOW });
        sink.AddLines(&points[1..]);
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
        sink.Close()?;
    }
    Ok(path)
}

fn paint_bar(frame: &Frame, scene: &Scene, layout: &Layout, hits: &mut Vec<(f32, f32, f32, f32, Hit)>) {
    let theme = scene.theme;
    let top = layout.lanes_height;
    let width = layout.width();
    let uptime = scene.lang.duration(scene.latest().system.uptime_s);
    let label = if scene.lang == Lang::Zh { format!("已开机 {uptime}") } else { format!("Up {uptime}") };
    frame.text(&label, theme.small, theme.text2, PAD_X, top + (BAR - LINE) / 2.0, width / 2.0, Align::Start);
    // The Settings glyph of the system's icon font, centred in its button.
    let button = (width - 10.0 - 32.0, top + (BAR - 32.0) / 2.0);
    let glyph = "\u{E713}";
    let icon = Font::new(Family::Icons, 16.0, 400.0);
    let glyph_left = button.0 + (32.0 - frame.gfx.measure(glyph, icon)) / 2.0;
    frame.text(glyph, icon, theme.text2, glyph_left, button.1 + 8.0, 32.0, Align::Start);
    hits.push((button.0, button.1, 32.0, 32.0, Hit::Settings));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_lanes_into_the_shortest_columns() {
        let heights = [100.0, 200.0, 50.0, 150.0];
        assert_eq!(split(&heights, 1), (vec![0], 500.0));
        // [100, 200] | [50, 150] beats [100] | [200, 50, 150].
        assert_eq!(split(&heights, 2), (vec![0, 2], 300.0));
        assert_eq!(split(&heights, 3).1, 200.0);
    }

    #[test]
    fn takes_the_fewest_columns_that_fit() {
        let heights = vec![300.0, 300.0, 300.0];
        assert_eq!(Layout::new(heights.clone(), 1000.0 + BAR, 3).columns, 1);
        assert_eq!(Layout::new(heights.clone(), 700.0, 3).columns, 2);
        // Too short for even the most columns: the most, to be zoomed out.
        assert_eq!(Layout::new(heights, 200.0, 2).columns, 2);
    }

    #[test]
    fn rounds_rates_up_to_a_clean_scale() {
        assert_eq!(round_up_rate(1500.0 * 1024.0), 2.0 * 1024.0 * 1024.0);
        assert_eq!(round_up_rate(30.0 * 1024.0), 50.0 * 1024.0);
        assert_eq!(round_up_rate(10.0 * 1024.0), 10.0 * 1024.0);
    }
}
