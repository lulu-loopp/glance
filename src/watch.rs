//! What Glance tells from the tray between openings: the readings in brief,
//! as the icon's tooltip, and, when asked, a warning that the CPU or a
//! graphics card has stayed hot.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::metrics::{Sample, StaticInfo};
use crate::ui::prefs::Prefs;
use crate::ui::text::Lang;

/// How long a part has to stay at or above the temperature alert before
/// Glance says so, and how far it has to cool before it may say so again.
const HOT_FOR: Duration = Duration::from_secs(30);
const COOL_BY: f32 = 5.0;

/// Follows the readings, one sample at a time, on the sampling thread.
#[derive(Default)]
pub struct Watch {
    /// Each part's heat, by its place in `temperatures`: the CPU and each
    /// graphics card count their own stretches.
    heat: HashMap<usize, Heat>,
    /// The alert's temperature the counts were kept against.
    limit: f32,
    tip: String,
}

impl Watch {
    pub fn sample(&mut self, sample: &Sample, now: Instant) {
        let app = crate::app();
        let (alert, view) = {
            let settings = app.settings.lock().unwrap();
            (settings.heat_alert, settings.view.clone())
        };
        let prefs = Prefs::resolve(&view, &app.controller.known_modules());
        let lang = Lang::resolve(prefs.language);
        let tip = summary(sample, &app.info, lang);
        if tip != self.tip {
            crate::tray::set_tip(&tip);
            self.tip = tip;
        }
        // Off, nothing is counted: turned on, a stretch starts afresh; so
        // too when the alert's temperature changes.
        let limit = prefs.hot_temp;
        if !alert || limit != self.limit {
            self.heat.clear();
            self.limit = limit;
        }
        if !alert {
            return;
        }
        for (i, (part, temp)) in temperatures(sample, &app.info).into_iter().enumerate() {
            let tell = self.heat.entry(i).or_default().step(temp, limit, now);
            let Some(temp) = temp.filter(|_| tell) else { continue };
            let (title, text) = match lang {
                Lang::Zh => ("Glance：温度过高".to_string(), format!("{part} 已持续 30 秒在 {temp:.0} °C，高于警示值 {limit:.0} °C。")),
                Lang::En => ("Glance: running hot".to_string(), format!("{part} has stayed at {temp:.0} °C for 30 seconds, above the {limit:.0} °C alert.")),
            };
            crate::tray::notify(&title, &text);
        }
    }
}

/// Whether a part has stayed hot: told once it has been at or above the
/// limit for `HOT_FOR` without a break, and again only after it has cooled
/// `COOL_BY` below the limit and stayed hot that long once more.
#[derive(Default)]
struct Heat {
    since: Option<Instant>,
    told: bool,
}

impl Heat {
    /// Takes the hottest reading now (`None` when none can be read) and says
    /// whether to tell the user.
    fn step(&mut self, hottest: Option<f32>, limit: f32, now: Instant) -> bool {
        match hottest {
            Some(temp) if temp >= limit => {
                let since = *self.since.get_or_insert(now);
                if !self.told && now.duration_since(since) >= HOT_FOR {
                    self.told = true;
                    return true;
                }
            }
            Some(temp) if temp < limit - COOL_BY => *self = Heat::default(),
            // Just under the limit, or unreadable: the stretch is broken.
            _ => self.since = None,
        }
        false
    }
}

/// The CPU's temperature and each graphics card's, by name (`None` where
/// it cannot be read just now).
fn temperatures(sample: &Sample, info: &StaticInfo) -> Vec<(String, Option<f32>)> {
    let cpu = ("CPU".to_string(), sample.cpu_sensors.as_ref().and_then(|sensors| sensors.temp));
    let gpus = sample.gpus.iter().zip(&info.gpus).map(|(reading, gpu)| (gpu.name.clone(), reading.temp));
    std::iter::once(cpu).chain(gpus).collect()
}

/// The tooltip: Glance's name, then the CPU, each graphics card and the
/// memory, a line each.
fn summary(sample: &Sample, info: &StaticInfo, lang: Lang) -> String {
    let mut rows = vec![("CPU".to_string(), sample.cpu, sample.cpu_sensors.as_ref().and_then(|sensors| sensors.temp))];
    let several = sample.gpus.len() > 1;
    for (i, gpu) in sample.gpus.iter().enumerate() {
        let name = if several { format!("GPU {}", i + 1) } else { "GPU".to_string() };
        rows.push((name, gpu.usage, gpu.temp));
    }
    let memory = sample.memory.used as f32 / info.mem_total.max(1) as f32 * 100.0;
    rows.push((lang.pick("内存", "Memory").to_string(), memory, None));
    // The name, then the use and the temperature, each lined up at its end.
    let cells: Vec<Vec<String>> = rows
        .into_iter()
        .map(|(label, usage, temp)| {
            let mut cells = vec![label, format!("{usage:.0}%")];
            cells.extend(temp.map(|temp| format!("· {temp:.0} °C")));
            cells
        })
        .collect();
    // The tooltip holds 127 characters, "Glance" and its line break among them.
    std::iter::once("Glance".to_string()).chain(crate::tray::columns(&cells, 127 - 7)).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_of_heat_once_per_hot_stretch() {
        let start = Instant::now();
        let at = |s: u64| start + Duration::from_secs(s);
        let mut heat = Heat::default();
        // Hot, but not yet for long enough.
        assert!(!heat.step(Some(90.0), 85.0, at(0)));
        assert!(!heat.step(Some(91.0), 85.0, at(29)));
        // Thirty seconds: told, once.
        assert!(heat.step(Some(90.0), 85.0, at(30)));
        assert!(!heat.step(Some(92.0), 85.0, at(60)));
        // Just under the limit is not cool enough to be told again.
        assert!(!heat.step(Some(83.0), 85.0, at(61)));
        assert!(!heat.step(Some(90.0), 85.0, at(62)));
        assert!(!heat.step(Some(90.0), 85.0, at(100)));
        // Cooled well below, then hot for thirty seconds again: told again.
        assert!(!heat.step(Some(70.0), 85.0, at(101)));
        assert!(!heat.step(Some(90.0), 85.0, at(102)));
        assert!(heat.step(Some(90.0), 85.0, at(132)));
    }

    #[test]
    fn a_break_restarts_the_count() {
        let start = Instant::now();
        let at = |s: u64| start + Duration::from_secs(s);
        let mut heat = Heat::default();
        assert!(!heat.step(Some(90.0), 85.0, at(0)));
        assert!(!heat.step(Some(84.0), 85.0, at(20)));
        assert!(!heat.step(Some(90.0), 85.0, at(21)));
        assert!(!heat.step(Some(90.0), 85.0, at(50)));
        assert!(heat.step(Some(90.0), 85.0, at(51)));
        // An unreadable sensor breaks the stretch too.
        let mut heat = Heat::default();
        assert!(!heat.step(Some(90.0), 85.0, at(0)));
        assert!(!heat.step(None, 85.0, at(15)));
        assert!(!heat.step(Some(90.0), 85.0, at(31)));
    }
}
