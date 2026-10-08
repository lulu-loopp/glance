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
const PAD: (f32, f32) = (10.0, 7.0);
const LINE: f32 = 19.0;
/// Between a reading's name and its value.
const LABEL_GAP: f32 = 10.0;
const RADIUS: f32 = 6.0;
const LABEL: Font = Font::new(Family::Segoe, 12.0, 600.0);
const VALUE: Font = Font::new(Family::Segoe, 13.0, 600.0);
const PLATE: u32 = 0x101214;
/// The plate while it is dragged.
const DRAGGED: Color = Color::hex(0x1D4F91, 0.9);
const NAME: Color = Color::hex(0xFFFFFF, 0.62);
const FIGURE: Color = Color::hex(0xFFFFFF, 1.0);
/// The frame rate's name, set off from the rest.
const FRAMES: Color = Color::hex(0x8FE3A4, 1.0);
/// A reading to heed (a muted microphone).
const HOT: Color = Color::hex(0xFF7B6B, 1.0);

/// One line of the overlay: a name, its value, and whether the value is to
/// be heeded.
pub type Line = (String, String, bool);

/// What the overlay shows of sample `s`, line by line: the `items` chosen
/// that are read. A game's readings are `game`'s, if there is one.
pub fn lines(s: &Sample, game: Option<&GameSample>, items: &[String], lang: Lang) -> Vec<Line> {
    let on = |name: &str| items.iter().any(|item| item == name);
    let joined = |parts: Vec<Option<String>>| parts.into_iter().flatten().collect::<Vec<_>>().join(" · ");
    let celsius = |t: f32| format!("{t:.0} °C");
    let watts = |w: f32| format!("{w:.0} W");
    // The GPU the game uses, or the busiest.
    let gpu = game
        .and_then(|game| game.gpu_index)
        .and_then(|i| s.gpus.get(i))
        .or_else(|| s.gpus.iter().max_by(|a, b| a.usage.unwrap_or(0.0).total_cmp(&b.usage.unwrap_or(0.0))));
    let cpu_sensors = s.cpu_sensors.as_ref();
    let mut lines = Vec::new();
    if let Some(game) = game {
        let fps = on("fps").then(|| format!("{:.0}", game.fps));
        let low = on("low").then(|| game.low.map(|low| format!("1% {low:.0}"))).flatten();
        lines.push(("FPS".to_string(), joined(vec![fps, low])));
        if on("frametime") {
            lines.push((lang.pick("帧时间", "Frame").to_string(), format!("{:.1} ms", game.longest_ms)));
        }
    }
    lines.push((
        "CPU".into(),
        joined(vec![
            on("cpu").then(|| s.cpu.map(text::percent)).flatten(),
            on("cpu_temp").then(|| cpu_sensors.and_then(|c| c.temp).map(celsius)).flatten(),
            on("cpu_power").then(|| cpu_sensors.and_then(|c| c.power).map(watts)).flatten(),
        ]),
    ));
    lines.push((
        "GPU".into(),
        joined(vec![
            on("gpu").then(|| gpu.and_then(|g| g.usage).map(text::percent)).flatten(),
            on("gpu_temp").then(|| gpu.and_then(|g| g.temp).map(celsius)).flatten(),
            on("gpu_power").then(|| gpu.and_then(|g| g.power).map(watts)).flatten(),
        ]),
    ));
    if on("memory") {
        lines.push((lang.pick("内存", "RAM").into(), text::size(s.memory.used)));
    }
    if on("vram") {
        lines.push((lang.pick("显存", "VRAM").into(), gpu.and_then(|g| g.mem_used).map(text::size).unwrap_or_default()));
    }
    if on("network") {
        let rate = |rate: Option<f64>| rate.map(|rate| text::rate(rate, false));
        let value = joined(vec![rate(s.net_down).map(|r| format!("↓ {r}")), rate(s.net_up).map(|r| format!("↑ {r}"))]);
        lines.push((lang.pick("网速", "Net").into(), value));
    }
    let mut lines: Vec<Line> = lines.into_iter().map(|(name, value)| (name, value, false)).collect();
    if let Some(muted) = s.mic_muted.filter(|_| on("mic")) {
        let value = if muted { lang.pick("已静音", "Muted") } else { lang.pick("开启", "On") };
        lines.push((lang.pick("麦克风", "Mic").into(), value.into(), muted));
    }
    // A line with nothing read on it is left out.
    lines.into_iter().filter(|(_, value, _)| !value.is_empty()).collect()
}

/// What the overlay would show of sample `s` over a game: its game's
/// readings, or with none running, an example game's, for the settings'
/// preview.
pub fn preview(s: &Sample, items: &[String], lang: Lang) -> Vec<Line> {
    let example = GameSample {
        name: String::new(),
        program: String::new(),
        is_game: true,
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
    lines(s, Some(s.game.as_ref().unwrap_or(&example)), items, lang)
}

/// How large `lines` are drawn, plate and all (DIPs), with `measure` giving
/// a text's width in a font.
pub fn size(lines: &[Line], measure: impl Fn(&str, Font) -> f32) -> (f32, f32) {
    let label = lines.iter().map(|(name, ..)| measure(name, LABEL)).fold(0.0, f32::max);
    let value = lines.iter().map(|(_, value, _)| measure(value, VALUE)).fold(0.0, f32::max);
    ((2.0 * PAD.0 + label + LABEL_GAP + value).ceil(), (2.0 * PAD.1 + lines.len() as f32 * LINE).ceil())
}

/// Draws `lines` on their plate, `opacity` opaque (0–1), from the canvas's
/// corner; `dragged`, on the plate that says it is being moved.
pub fn paint(frame: &dyn Canvas, lines: &[Line], opacity: f32, dragged: bool) {
    let (width, height) = size(lines, |text, font| frame.measure(text, font));
    let plate = if dragged { DRAGGED } else { Color::hex(PLATE, opacity.clamp(0.0, 1.0)) };
    frame.fill_rounded(plate, 0.0, 0.0, width, height, RADIUS);
    let label = lines.iter().map(|(name, ..)| frame.measure(name, LABEL)).fold(0.0, f32::max);
    // The two faces' baselines level.
    let (ascent_label, _) = frame.baseline(LABEL);
    let (ascent_value, descent_value) = frame.baseline(VALUE);
    for (i, (name, value, hot)) in lines.iter().enumerate() {
        // The value's line box centred in its line, as tall as the face
        // makes it (more than its size).
        let y = PAD.1 + i as f32 * LINE + (LINE - ascent_value - descent_value) / 2.0;
        let color = if name == "FPS" { FRAMES } else { NAME };
        frame.text(name, LABEL, color, PAD.0, y + ascent_value - ascent_label, label, Align::Start);
        frame.text(value, VALUE, if *hot { HOT } else { FIGURE }, PAD.0 + label + LABEL_GAP, y, width, Align::Start);
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
                is_game: true,
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
        let plain = |lines: Vec<Line>| lines.into_iter().map(|(name, value, _)| (name, value)).collect::<Vec<_>>();
        assert_eq!(
            plain(lines(&s, s.game.as_ref(), &chosen, Lang::Zh)),
            [("FPS".into(), "144".into()), ("帧时间".into(), "9.3 ms".into()), ("CPU".into(), text::percent(40.0)), ("内存".into(), "8.0 GB".to_string())]
        );
        // Without a game, only the machine's.
        let s = sample(false);
        assert_eq!(plain(lines(&s, None, &items(&["fps", "cpu"]), Lang::En)), [("CPU".into(), text::percent(40.0))]);
        assert!(lines(&s, None, &items(&["fps"]), Lang::En).is_empty());
        // A muted microphone, to be heeded.
        assert_eq!(lines(&s, None, &items(&["mic"]), Lang::En), [("Mic".into(), "Muted".into(), true)]);
    }
}
