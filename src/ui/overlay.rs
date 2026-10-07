//! The overlay over a game: a few readings in a corner of its screen, on a
//! dark plate that reads over any picture. What it shows and how it is
//! drawn; its window is the panel thread's (see `crate::overlay`).

use super::canvas::{Align, Canvas, Color, Family, Font};
use super::text::{self, Lang};
use crate::reading::Sample;
use crate::settings::Detail;

/// Room around the readings, and the height of a line of them (DIPs).
const PAD: (f32, f32) = (10.0, 7.0);
const LINE: f32 = 19.0;
/// Between a reading's name and its value.
const LABEL_GAP: f32 = 10.0;
const RADIUS: f32 = 6.0;
const LABEL: Font = Font::new(Family::Segoe, 12.0, 600.0);
const VALUE: Font = Font::new(Family::Segoe, 13.0, 600.0);
const PLATE: Color = Color::hex(0x101214, 0.62);
const NAME: Color = Color::hex(0xFFFFFF, 0.62);
const FIGURE: Color = Color::hex(0xFFFFFF, 1.0);
/// The frame rate's name, set off from the rest.
const FRAMES: Color = Color::hex(0x8FE3A4, 1.0);

/// What the overlay shows of sample `s`, line by line (a name and its
/// value), at `detail`; nothing without a game.
pub fn lines(s: &Sample, detail: Detail, lang: Lang) -> Vec<(String, String)> {
    let Some(game) = s.game.as_ref() else { return Vec::new() };
    let joined = |parts: Vec<Option<String>>| parts.into_iter().flatten().collect::<Vec<_>>().join(" · ");
    let celsius = |t: f32| format!("{t:.0} °C");
    let watts = |w: f32| format!("{w:.0} W");
    // The GPU the game uses, or the busiest.
    let gpu = game.gpu_index.and_then(|i| s.gpus.get(i)).or_else(|| s.gpus.iter().max_by(|a, b| a.usage.unwrap_or(0.0).total_cmp(&b.usage.unwrap_or(0.0))));
    let fps = format!("{:.0}", game.fps);
    let cpu_temp = s.cpu_sensors.as_ref().and_then(|c| c.temp);
    let cpu_power = s.cpu_sensors.as_ref().and_then(|c| c.power);
    let low = game.low.map(|low| format!("1% {low:.0}"));
    let lines: Vec<(String, String)> = match detail {
        Detail::Simple => vec![("FPS".into(), fps)],
        Detail::Standard => vec![
            ("FPS".into(), joined(vec![Some(fps), low])),
            ("CPU".into(), joined(vec![s.cpu.map(text::percent), cpu_temp.map(celsius)])),
            ("GPU".into(), joined(vec![gpu.and_then(|g| g.usage).map(text::percent), gpu.and_then(|g| g.temp).map(celsius)])),
        ],
        Detail::Detailed => vec![
            ("FPS".into(), joined(vec![Some(fps), low])),
            (lang.pick("帧时间", "Frame").into(), format!("{:.1} ms", game.longest_ms)),
            ("CPU".into(), joined(vec![s.cpu.map(text::percent), cpu_temp.map(celsius), cpu_power.map(watts)])),
            (
                "GPU".into(),
                joined(vec![gpu.and_then(|g| g.usage).map(text::percent), gpu.and_then(|g| g.temp).map(celsius), gpu.and_then(|g| g.power).map(watts)]),
            ),
            (lang.pick("内存", "RAM").into(), text::size(s.memory.used)),
            (lang.pick("显存", "VRAM").into(), gpu.and_then(|g| g.mem_used).map(text::size).unwrap_or_default()),
        ],
    };
    // A line with nothing read on it is left out.
    lines.into_iter().filter(|(_, value)| !value.is_empty()).collect()
}

/// How large `lines` are drawn, plate and all (DIPs), with `measure` giving
/// a text's width in a font.
pub fn size(lines: &[(String, String)], measure: impl Fn(&str, Font) -> f32) -> (f32, f32) {
    let label = lines.iter().map(|(name, _)| measure(name, LABEL)).fold(0.0, f32::max);
    let value = lines.iter().map(|(_, value)| measure(value, VALUE)).fold(0.0, f32::max);
    ((2.0 * PAD.0 + label + LABEL_GAP + value).ceil(), (2.0 * PAD.1 + lines.len() as f32 * LINE).ceil())
}

/// Draws `lines` on their plate, from the canvas's corner.
pub fn paint(frame: &dyn Canvas, lines: &[(String, String)]) {
    let (width, height) = size(lines, |text, font| frame.measure(text, font));
    frame.fill_rounded(PLATE, 0.0, 0.0, width, height, RADIUS);
    let label = lines.iter().map(|(name, _)| frame.measure(name, LABEL)).fold(0.0, f32::max);
    for (i, (name, value)) in lines.iter().enumerate() {
        let y = PAD.1 + i as f32 * LINE;
        // The two faces' baselines level.
        let (ascent_label, _) = frame.baseline(LABEL);
        let (ascent_value, _) = frame.baseline(VALUE);
        let color = if i == 0 { FRAMES } else { NAME };
        frame.text(name, LABEL, color, PAD.0, y + (LINE - VALUE.size) / 2.0 + ascent_value - ascent_label, label, Align::Start);
        frame.text(value, VALUE, FIGURE, PAD.0 + label + LABEL_GAP, y + (LINE - VALUE.size) / 2.0, width, Align::Start);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reading::{GameSample, MemorySample, SystemSample};

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
            game: game.then(|| GameSample {
                name: "Game".into(),
                program: "game".into(),
                is_game: true,
                marked: false,
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

    #[test]
    fn shows_what_its_detail_asks_and_is_there() {
        assert!(lines(&sample(false), Detail::Standard, Lang::En).is_empty());
        let s = sample(true);
        assert_eq!(lines(&s, Detail::Simple, Lang::En), [("FPS".to_string(), "144".to_string())]);
        // Unread readings left out, not shown as gaps; no GPU, no line.
        let standard = lines(&s, Detail::Standard, Lang::En);
        assert_eq!(standard[1], ("CPU".into(), text::percent(40.0)));
        assert_eq!(standard.len(), 2);
        let detailed = lines(&s, Detail::Detailed, Lang::Zh);
        assert_eq!(detailed[1], ("帧时间".into(), "9.3 ms".into()));
        // No GPU: no GPU line, no video memory.
        assert_eq!(detailed.len(), 4);
    }
}
