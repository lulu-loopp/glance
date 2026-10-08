//! The overlay: a few readings floating over the screen, on a dark plate
//! (as opaque as chosen) that reads over any picture, whatever the panel's
//! look. What it shows and how it is drawn; its window is the panel
//! thread's (see `crate::overlay`).

use super::canvas::{Align, Canvas, Color, Family, Font};
use super::text::{self, Lang};
use crate::reading::{GameSample, Sample};

/// What the overlay can show, in the order it shows them: name, and what
/// the settings call it in Chinese and English. Readings of one thing go
/// on one line (the CPU's use, heat and power).
pub const ITEMS: [(&str, &str, &str); 13] = [
    ("fps", "帧率", "FPS"),
    ("low", "1% low", "1% low"),
    ("frametime", "帧时间", "Frame time"),
    ("cpu", "CPU 占用", "CPU use"),
    ("cpu_temp", "CPU 温度", "CPU temp"),
    ("cpu_power", "CPU 功耗", "CPU power"),
    ("gpu", "GPU 占用", "GPU use"),
    ("gpu_temp", "GPU 温度", "GPU temp"),
    ("gpu_power", "GPU 功耗", "GPU power"),
    ("memory", "内存", "Memory"),
    ("vram", "显存", "VRAM"),
    ("network", "网速", "Network"),
    ("mic", "麦克风", "Microphone"),
];

/// How far the plate keeps from its screen's edges (DIPs).
pub const INSET: f32 = 16.0;

/// Room around the readings, and the height of a line of them (DIPs).
const PAD: (f32, f32) = (12.0, 8.0);
const LINE: f32 = 19.0;
/// Between a reading's name and its value.
const LABEL_GAP: f32 = 10.0;
const RADIUS: f32 = 8.0;
/// Names and values alike in size (Chinese shows a size apart at once),
/// told apart by colour and weight.
const LABEL: Font = Font::new(Family::Segoe, 13.0, 500.0);
const VALUE: Font = Font::new(Family::Segoe, 13.0, 600.0);
/// A cool, soft grey: lighter than black, so the plate reads as glass over
/// a game rather than a hole in it.
const PLATE: u32 = 0x1E2229;
/// The plate while it is dragged.
const DRAGGED: Color = Color::hex(0x1D4F91, 0.9);
const NAME: Color = Color::hex(0xFFFFFF, 0.74);
const FIGURE: Color = Color::hex(0xFFFFFF, 1.0);
/// The frame rate's name, set off from the rest.
const FRAMES: Color = Color::hex(0x8FE3A4, 1.0);
/// Readings are outlined on a plate less opaque than this, in this.
const OUTLINED_BELOW: f32 = 0.35;
const OUTLINE: Color = Color::hex(0x000000, 0.85);
/// A reading to heed (a muted microphone).
const HOT: Color = Color::hex(0xFF7B6B, 1.0);

/// One line of the overlay: a name, its value, whether the value is to be
/// heeded, and for each of the value's parts, the widest it can be written
/// (the plate is as wide as those, so that it holds still as values change).
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub name: String,
    pub value: String,
    pub hot: bool,
    widest: Vec<&'static [&'static str]>,
}

// The widest each reading can be written: the system font's digits are all
// as wide, so it is the most digits each unit is shown with.
const PERCENT: &[&str] = &["100%"];
const CELSIUS: &[&str] = &["100 °C"];
const WATTS: &[&str] = &["999 W"];
const FRAMES_A_SECOND: &[&str] = &["999"];
const LOW: &[&str] = &["1% 999"];
const FRAME_TIME: &[&str] = &["999.9 ms"];
const SIZE: &[&str] = &["999.9 GB", "999 MB"];
const DOWN: &[&str] = &["↓ 999 B/s", "↓ 99.9 KB/s", "↓ 999 KB/s", "↓ 99.9 MB/s", "↓ 999 MB/s", "↓ 99.9 GB/s"];
const UP: &[&str] = &["↑ 999 B/s", "↑ 99.9 KB/s", "↑ 999 KB/s", "↑ 99.9 MB/s", "↑ 999 MB/s", "↑ 99.9 GB/s"];
const MIC_ZH: &[&str] = &["已静音", "开启"];
const MIC_EN: &[&str] = &["Muted", "On"];
/// Between the parts of a value.
const SEPARATOR: &str = " · ";

/// A line of the parts that are read (each with the widest it can be); none
/// if none is.
fn line(name: &str, parts: Vec<(Option<String>, &'static [&'static str])>, hot: bool) -> Option<Line> {
    let read: Vec<(String, &'static [&'static str])> = parts.into_iter().filter_map(|(value, widest)| Some((value?, widest))).collect();
    (!read.is_empty()).then(|| Line {
        name: name.to_string(),
        value: read.iter().map(|(value, _)| value.as_str()).collect::<Vec<_>>().join(SEPARATOR),
        hot,
        widest: read.iter().map(|(_, widest)| *widest).collect(),
    })
}

/// What the overlay shows of sample `s`, line by line: the `items` chosen
/// that are read. A game's readings are `game`'s while one is `playing`;
/// "—" while it presents no frames for a moment (it lost the front to a
/// menu, it loads), so that its lines hold their place.
pub fn lines(s: &Sample, game: Option<&GameSample>, playing: bool, items: &[String], lang: Lang) -> Vec<Line> {
    let on = |name: &str| items.iter().any(|item| item == name);
    let celsius = |t: f32| format!("{t:.0} °C");
    let watts = |w: f32| format!("{w:.0} W");
    // The GPU the game uses, or the busiest.
    let gpu = game
        .and_then(|game| game.gpu_index)
        .and_then(|i| s.gpus.get(i))
        .or_else(|| s.gpus.iter().max_by(|a, b| a.usage.unwrap_or(0.0).total_cmp(&b.usage.unwrap_or(0.0))));
    let cpu_sensors = s.cpu_sensors.as_ref();
    let rate = |rate: Option<f64>| rate.map(|rate| text::rate(rate, false));
    let mut lines = Vec::new();
    if playing {
        let unread = || "—".to_string();
        let fps = game.map_or_else(unread, |game| format!("{:.0}", game.fps));
        // No 1% low yet is no line part, as for any reading not yet read.
        let low = match game {
            Some(game) => game.low.map(|low| format!("1% {low:.0}")),
            None => Some(unread()),
        };
        let frame = game.map_or_else(unread, |game| format!("{:.1} ms", game.longest_ms));
        lines.push(line("FPS", vec![(on("fps").then_some(fps), FRAMES_A_SECOND), (low.filter(|_| on("low")), LOW)], false));
        lines.push(line(lang.pick("帧时间", "Frame"), vec![(on("frametime").then_some(frame), FRAME_TIME)], false));
    }
    lines.push(line(
        "CPU",
        vec![
            (on("cpu").then(|| s.cpu.map(text::percent)).flatten(), PERCENT),
            (on("cpu_temp").then(|| cpu_sensors.and_then(|c| c.temp).map(celsius)).flatten(), CELSIUS),
            (on("cpu_power").then(|| cpu_sensors.and_then(|c| c.power).map(watts)).flatten(), WATTS),
        ],
        false,
    ));
    lines.push(line(
        "GPU",
        vec![
            (on("gpu").then(|| gpu.and_then(|g| g.usage).map(text::percent)).flatten(), PERCENT),
            (on("gpu_temp").then(|| gpu.and_then(|g| g.temp).map(celsius)).flatten(), CELSIUS),
            (on("gpu_power").then(|| gpu.and_then(|g| g.power).map(watts)).flatten(), WATTS),
        ],
        false,
    ));
    lines.push(line(lang.pick("内存", "RAM"), vec![(on("memory").then(|| text::size(s.memory.used)), SIZE)], false));
    lines.push(line(lang.pick("显存", "VRAM"), vec![(on("vram").then(|| gpu.and_then(|g| g.mem_used).map(text::size)).flatten(), SIZE)], false));
    lines.push(line(
        lang.pick("网速", "Net"),
        vec![
            (on("network").then(|| rate(s.net_down).map(|r| format!("↓ {r}"))).flatten(), DOWN),
            (on("network").then(|| rate(s.net_up).map(|r| format!("↑ {r}"))).flatten(), UP),
        ],
        false,
    ));
    if let Some(muted) = s.mic_muted.filter(|_| on("mic")) {
        let value = if muted { lang.pick("已静音", "Muted") } else { lang.pick("开启", "On") };
        let widest = if lang == Lang::Zh { MIC_ZH } else { MIC_EN };
        lines.push(line(lang.pick("麦克风", "Mic"), vec![(Some(value.to_string()), widest)], muted));
    }
    lines.into_iter().flatten().collect()
}

/// What the overlay would show of sample `s` over a game: its game's
/// readings, or with none running, an example game's, for the settings'
/// preview.
pub fn preview(s: &Sample, items: &[String], lang: Lang) -> Vec<Line> {
    let example = GameSample {
        name: String::new(),
        program: String::new(),
        fps: 144.0,
        low: Some(118.0),
        longest_ms: 8.4,
        fills_screen: true,
        refresh_hz: Some(144),
        screen: None,
        gpu_index: None,
        cpu: None,
        gpu: None,
        mem: None,
        vram: None,
        gpu_limit: None,
        playing_s: None,
    };
    lines(s, Some(s.game.as_ref().unwrap_or(&example)), true, items, lang)
}

/// How large `lines` are drawn, plate and all (DIPs), with `measure` giving
/// a text's width in a font: as wide as their values can be, not as they
/// are now.
pub fn size(lines: &[Line], measure: impl Fn(&str, Font) -> f32) -> (f32, f32) {
    let label = lines.iter().map(|line| measure(&line.name, LABEL)).fold(0.0, f32::max);
    let widest = |line: &Line| {
        let parts: f32 = line.widest.iter().map(|forms| forms.iter().map(|form| measure(form, VALUE)).fold(0.0, f32::max)).sum();
        parts + line.widest.len().saturating_sub(1) as f32 * measure(SEPARATOR, VALUE)
    };
    let value = lines.iter().map(|line| widest(line).max(measure(&line.value, VALUE))).fold(0.0, f32::max);
    ((2.0 * PAD.0 + label + LABEL_GAP + value).ceil(), (2.0 * PAD.1 + lines.len() as f32 * LINE).ceil())
}

/// Draws `lines` on their plate, `opacity` opaque (0–1), from the canvas's
/// corner, at `px` physical pixels a DIP (each line put on a whole pixel,
/// not smeared across two); `dragged`, on the plate that says it is being
/// moved.
pub fn paint(frame: &dyn Canvas, lines: &[Line], opacity: f32, dragged: bool, px: f32) {
    let snap = |at: f32| (at * px).round() / px;
    let (width, height) = size(lines, |text, font| frame.measure(text, font));
    let plate = if dragged { DRAGGED } else { Color::hex(PLATE, opacity.clamp(0.0, 1.0)) };
    frame.fill_rounded(plate, 0.0, 0.0, width, height, RADIUS);
    let label = lines.iter().map(|line| frame.measure(&line.name, LABEL)).fold(0.0, f32::max);
    // The two faces' baselines level.
    let (ascent_label, _) = frame.baseline(LABEL);
    let (ascent_value, descent_value) = frame.baseline(VALUE);
    // On a plate too clear to set them off, the readings take an outline to
    // read over any picture: a steady one, a physical pixel wide. A faint
    // one, coming and going with what is behind, would only look unsteady.
    let outline = (!dragged && opacity < OUTLINED_BELOW).then_some(OUTLINE);
    let pixel = 1.0 / px;
    let outlined = |text: &str, font: Font, color: Color, x: f32, y: f32, room: f32| {
        if let Some(outline) = outline {
            for (dx, dy) in [(-pixel, 0.0), (pixel, 0.0), (0.0, -pixel), (0.0, pixel)] {
                frame.text(text, font, outline, x + dx, y + dy, room, Align::Start);
            }
        }
        frame.text(text, font, color, x, y, room, Align::Start);
    };
    for (i, Line { name, value, hot, .. }) in lines.iter().enumerate() {
        // The value's line box centred in its line, as tall as the face
        // makes it (more than its size).
        let y = PAD.1 + i as f32 * LINE + (LINE - ascent_value - descent_value) / 2.0;
        // Each face's baseline on a whole pixel.
        let (label_top, value_top) = (snap(y + ascent_value) - ascent_label, snap(y + ascent_value) - ascent_value);
        let color = if name == "FPS" { FRAMES } else { NAME };
        outlined(name, LABEL, color, snap(PAD.0), label_top, label);
        outlined(value, VALUE, if *hot { HOT } else { FIGURE }, snap(PAD.0 + label + LABEL_GAP), value_top, width);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reading::{MemorySample, SystemSample};

    fn sample(game: bool) -> Sample {
        Sample {
            t: 0,
            cpu: Some(40.0),
            threads: Vec::new(),
            ghz: None,
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
                        fps: 143.6,
                low: None,
                longest_ms: 9.26,
                fills_screen: true,
                refresh_hz: Some(144),
                screen: None,
                gpu_index: None,
                cpu: None,
                gpu: None,
                mem: None,
                vram: None,
                gpu_limit: None,
                playing_s: None,
            }),
        }
    }

    fn items(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn shows_the_items_chosen_that_are_read() {
        let chosen = items(&["fps", "low", "frametime", "cpu", "gpu", "memory"]);
        let s = sample(true);
        // Unread readings left out, not shown as gaps: no 1% low yet, no GPU.
        let plain = |lines: Vec<Line>| lines.into_iter().map(|line| (line.name, line.value)).collect::<Vec<(String, String)>>();
        assert_eq!(
            plain(lines(&s, s.game.as_ref(), true, &chosen, Lang::Zh)),
            [("FPS".into(), "144".into()), ("帧时间".into(), "9.3 ms".into()), ("CPU".into(), text::percent(40.0)), ("内存".into(), "8.0 GB".to_string())]
        );
        // Without a game, only the machine's.
        let s = sample(false);
        assert_eq!(plain(lines(&s, None, false, &items(&["fps", "cpu"]), Lang::En)), [("CPU".into(), text::percent(40.0))]);
        assert!(lines(&s, None, false, &items(&["fps"]), Lang::En).is_empty());
        // A game played that presents no frame for a moment keeps its line.
        assert_eq!(plain(lines(&s, None, true, &items(&["fps"]), Lang::En)), [("FPS".into(), "—".into())]);
        // A muted microphone, to be heeded.
        let mic = lines(&s, None, false, &items(&["mic"]), Lang::En);
        assert_eq!((mic[0].value.as_str(), mic[0].hot), ("Muted", true));
        // As wide as its widest value, whatever it reads now: a character
        // a unit wide here.
        let measure = |text: &str, _| text.chars().count() as f32;
        let now = size(&lines(&s, None, false, &items(&["cpu"]), Lang::En), measure);
        assert_eq!(now.0, (2.0 * PAD.0 + 3.0 + LABEL_GAP + 4.0).ceil());
    }
}
