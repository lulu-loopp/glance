//! The readings over time, as an agent asks for them: by series, each named as `get_history` names it (see `Series`).
//! Glance holds its last minutes of samples for the panel's charts (see `panel::History`). While the interface is on it
//! also keeps a day of minutes: each series' average over each minute (see `keep`), for trends longer than those.

use crate::reading::Sample;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// A series of readings, by what it is of.
#[derive(Clone, Debug, PartialEq)]
pub enum Series {
    CpuPercent,
    CpuCelsius,
    CpuWatts,
    CpuGhz,
    /// A GPU's, by its index among the GPUs (as `get_hardware` lists them).
    Gpu(usize, GpuSeries),
    MemoryUsed,
    MemoryCommitted,
    NetworkDown,
    NetworkUp,
    Ping,
    DiskRead,
    DiskWrite,
    DiskActive,
    /// A physical drive's, by its number.
    Drive(u32, DriveSeries),
    BatteryPercent,
    BatteryWatts,
    GameFps,
    /// A motherboard temperature, or a fan, by its name.
    Board(String),
    Fan(String),
    /// A program's, by its executable's name (case folded).
    Program(String, ProgramSeries),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuSeries {
    Percent,
    Celsius,
    Watts,
    MemoryUsed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveSeries {
    Read,
    Write,
    Celsius,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgramSeries {
    Cpu,
    Memory,
}

impl Series {
    /// The series named `name` (see `get_history`): none where no series is named so.
    pub fn parse(name: &str) -> Option<Series> {
        let fixed = match name {
            "cpu.percent" => Some(Series::CpuPercent),
            "cpu.celsius" => Some(Series::CpuCelsius),
            "cpu.watts" => Some(Series::CpuWatts),
            "cpu.ghz" => Some(Series::CpuGhz),
            "memory.used_bytes" => Some(Series::MemoryUsed),
            "memory.committed_bytes" => Some(Series::MemoryCommitted),
            "network.down_bytes_per_s" => Some(Series::NetworkDown),
            "network.up_bytes_per_s" => Some(Series::NetworkUp),
            "network.ping_ms" => Some(Series::Ping),
            "disk.read_bytes_per_s" => Some(Series::DiskRead),
            "disk.write_bytes_per_s" => Some(Series::DiskWrite),
            "disk.active_percent" => Some(Series::DiskActive),
            "battery.percent" => Some(Series::BatteryPercent),
            "battery.watts" => Some(Series::BatteryWatts),
            "game.fps" => Some(Series::GameFps),
            _ => None,
        };
        if fixed.is_some() {
            return fixed;
        }
        let numbered = |prefix: &str| -> Option<(u32, &str)> {
            let rest = name.strip_prefix(prefix)?;
            let (number, field) = rest.split_once('.')?;
            (!number.is_empty() && number.len() <= 4 && number.bytes().all(|byte| byte.is_ascii_digit())).then(|| (number.parse().ok(), field)).and_then(|(number, field)| Some((number?, field)))
        };
        if let Some((index, field)) = numbered("gpu") {
            let field = match field {
                "percent" => GpuSeries::Percent,
                "celsius" => GpuSeries::Celsius,
                "watts" => GpuSeries::Watts,
                "memory_used_bytes" => GpuSeries::MemoryUsed,
                _ => return None,
            };
            return Some(Series::Gpu(index as usize, field));
        }
        if let Some((id, field)) = numbered("drive") {
            let field = match field {
                "read_bytes_per_s" => DriveSeries::Read,
                "write_bytes_per_s" => DriveSeries::Write,
                "celsius" => DriveSeries::Celsius,
                _ => return None,
            };
            return Some(Series::Drive(id, field));
        }
        if let Some(sensor) = name.strip_prefix("board.").and_then(|rest| rest.strip_suffix(".celsius")).filter(|sensor| !sensor.is_empty()) {
            return Some(Series::Board(sensor.to_string()));
        }
        if let Some(fan) = name.strip_prefix("fan.").and_then(|rest| rest.strip_suffix(".rpm")).filter(|fan| !fan.is_empty()) {
            return Some(Series::Fan(fan.to_string()));
        }
        let rest = name.strip_prefix("program.")?;
        let (program, field) = if let Some(program) = rest.strip_suffix(".cpu_percent") {
            (program, ProgramSeries::Cpu)
        } else {
            (rest.strip_suffix(".memory_bytes")?, ProgramSeries::Memory)
        };
        (!program.is_empty()).then(|| Series::Program(program.to_lowercase(), field))
    }

    /// Its name (as `parse` reads it).
    pub fn name(&self) -> String {
        match self {
            Series::CpuPercent => "cpu.percent".into(),
            Series::CpuCelsius => "cpu.celsius".into(),
            Series::CpuWatts => "cpu.watts".into(),
            Series::CpuGhz => "cpu.ghz".into(),
            Series::Gpu(index, field) => format!(
                "gpu{index}.{}",
                match field {
                    GpuSeries::Percent => "percent",
                    GpuSeries::Celsius => "celsius",
                    GpuSeries::Watts => "watts",
                    GpuSeries::MemoryUsed => "memory_used_bytes",
                }
            ),
            Series::MemoryUsed => "memory.used_bytes".into(),
            Series::MemoryCommitted => "memory.committed_bytes".into(),
            Series::NetworkDown => "network.down_bytes_per_s".into(),
            Series::NetworkUp => "network.up_bytes_per_s".into(),
            Series::Ping => "network.ping_ms".into(),
            Series::DiskRead => "disk.read_bytes_per_s".into(),
            Series::DiskWrite => "disk.write_bytes_per_s".into(),
            Series::DiskActive => "disk.active_percent".into(),
            Series::Drive(id, field) => format!(
                "drive{id}.{}",
                match field {
                    DriveSeries::Read => "read_bytes_per_s",
                    DriveSeries::Write => "write_bytes_per_s",
                    DriveSeries::Celsius => "celsius",
                }
            ),
            Series::BatteryPercent => "battery.percent".into(),
            Series::BatteryWatts => "battery.watts".into(),
            Series::GameFps => "game.fps".into(),
            Series::Board(sensor) => format!("board.{sensor}.celsius"),
            Series::Fan(fan) => format!("fan.{fan}.rpm"),
            Series::Program(program, ProgramSeries::Cpu) => format!("program.{program}.cpu_percent"),
            Series::Program(program, ProgramSeries::Memory) => format!("program.{program}.memory_bytes"),
        }
    }

    /// Its value in `sample`: none where it was not read.
    pub fn of(&self, sample: &Sample) -> Option<f64> {
        let f = |value: Option<f32>| value.map(f64::from);
        match self {
            Series::CpuPercent => f(sample.cpu),
            Series::CpuCelsius => f(sample.cpu_sensors.as_ref()?.temp),
            Series::CpuWatts => f(sample.cpu_sensors.as_ref()?.power),
            Series::CpuGhz => f(sample.ghz),
            Series::Gpu(index, field) => {
                let gpu = sample.gpus.get(*index)?;
                match field {
                    GpuSeries::Percent => f(gpu.usage),
                    GpuSeries::Celsius => f(gpu.temp),
                    GpuSeries::Watts => f(gpu.power),
                    GpuSeries::MemoryUsed => gpu.mem_used.map(|bytes| bytes as f64),
                }
            }
            Series::MemoryUsed => Some(sample.memory.used as f64),
            Series::MemoryCommitted => Some(sample.memory.committed as f64),
            Series::NetworkDown => sample.net_down,
            Series::NetworkUp => sample.net_up,
            Series::Ping => f(sample.ping_ms),
            Series::DiskRead => sample.disk_read,
            Series::DiskWrite => sample.disk_write,
            Series::DiskActive => f(sample.disk_active),
            Series::Drive(id, DriveSeries::Read) => sample.drive_rates.iter().find(|rate| rate.id == *id)?.read,
            Series::Drive(id, DriveSeries::Write) => sample.drive_rates.iter().find(|rate| rate.id == *id)?.write,
            Series::Drive(id, DriveSeries::Celsius) => f(Some(sample.drive_temps.iter().find(|drive| drive.id == *id)?.celsius)),
            Series::BatteryPercent => Some(f64::from(sample.battery.as_ref()?.percent)),
            Series::BatteryWatts => f(sample.battery.as_ref()?.watts),
            Series::GameFps => f(sample.game.as_ref()?.fps),
            Series::Board(sensor) => f(Some(sample.board.as_ref()?.temps.iter().find(|(name, _)| name == sensor)?.1)),
            Series::Fan(fan) => f(Some(sample.board.as_ref()?.fans.iter().find(|(name, _)| name == fan)?.1)),
            Series::Program(program, field) => {
                let found = sample.processes.iter().find(|found| found.name.eq_ignore_ascii_case(program))?;
                Some(match field {
                    ProgramSeries::Cpu => f64::from(found.cpu),
                    ProgramSeries::Memory => found.mem as f64,
                })
            }
        }
    }

    /// Every series `sample` has a value of, and that value.
    pub fn each_in(sample: &Sample, mut each: impl FnMut(Series, f64)) {
        let mut fixed = vec![
            Series::CpuPercent,
            Series::CpuCelsius,
            Series::CpuWatts,
            Series::CpuGhz,
            Series::MemoryUsed,
            Series::MemoryCommitted,
            Series::NetworkDown,
            Series::NetworkUp,
            Series::Ping,
            Series::DiskRead,
            Series::DiskWrite,
            Series::DiskActive,
            Series::BatteryPercent,
            Series::BatteryWatts,
            Series::GameFps,
        ];
        for index in 0..sample.gpus.len() {
            fixed.extend([GpuSeries::Percent, GpuSeries::Celsius, GpuSeries::Watts, GpuSeries::MemoryUsed].map(|field| Series::Gpu(index, field)));
        }
        let mut drives: Vec<u32> = sample.drive_rates.iter().map(|rate| rate.id).chain(sample.drive_temps.iter().map(|drive| drive.id)).collect();
        drives.sort_unstable();
        drives.dedup();
        for id in drives {
            fixed.extend([DriveSeries::Read, DriveSeries::Write, DriveSeries::Celsius].map(|field| Series::Drive(id, field)));
        }
        if let Some(board) = &sample.board {
            fixed.extend(board.temps.iter().map(|(name, _)| Series::Board(name.clone())));
            fixed.extend(board.fans.iter().map(|(name, _)| Series::Fan(name.clone())));
        }
        for program in &sample.processes {
            let name = program.name.to_lowercase();
            fixed.push(Series::Program(name.clone(), ProgramSeries::Cpu));
            fixed.push(Series::Program(name, ProgramSeries::Memory));
        }
        for series in fixed {
            if let Some(value) = series.of(sample) {
                each(series, value);
            }
        }
    }
}

/// A minute kept: when it began (ms since 1970), how many samples came in it, and each series' average over those
/// that had it (by the series' number in `Minutes::names`).
#[derive(Clone, Debug, PartialEq)]
pub struct Minute {
    pub start: u64,
    pub samples: u32,
    /// Each series' average over the minute, by its number, and how many of the minute's samples had it.
    values: Vec<(u32, f32, u16)>,
}

/// A day of minutes, and the one under way.
/// A minute under way: when it began, its samples, and each series' (by its number) sum and count.
type Open = (u64, u32, HashMap<u32, (f64, u32)>);

#[derive(Default)]
pub struct Minutes {
    kept: VecDeque<Minute>,
    /// The minute under way: when it began, its samples, and each series' sum and count.
    open: Option<Open>,
    names: Vec<Box<str>>,
    numbers: HashMap<Box<str>, u32>,
}

/// How long a minute is, and how many are kept.
pub const MINUTE_MS: u64 = 60_000;
const KEPT_MS: u64 = 24 * 60 * MINUTE_MS;

impl Minutes {
    /// `sample` counted into its minute: the minute before kept once one after it begins, those over a day before
    /// the newest let go of.
    pub fn add(&mut self, sample: &Sample) {
        let start = sample.t - sample.t % MINUTE_MS;
        // (A clock set back: what was kept after it is no longer before it.)
        if self.open.as_ref().is_some_and(|(open, ..)| start < *open) || self.kept.back().is_some_and(|last| start <= last.start) {
            self.kept.retain(|minute| minute.start < start);
            if self.open.as_ref().is_some_and(|(open, ..)| start < *open) {
                self.open = None;
            }
        }
        if self.open.as_ref().is_some_and(|(open, ..)| *open != start) {
            self.close();
        }
        let mut values = Vec::new();
        Series::each_in(sample, |series, value| values.push((series.name(), value)));
        let (_, samples, sums) = self.open.get_or_insert_with(|| (start, 0, HashMap::new()));
        *samples += 1;
        for (name, value) in values {
            let number = match self.numbers.get(name.as_str()) {
                Some(&number) => number,
                None => {
                    let number = self.names.len() as u32;
                    self.names.push(name.clone().into());
                    self.numbers.insert(name.into(), number);
                    number
                }
            };
            let (sum, count) = sums.entry(number).or_insert((0.0, 0));
            *sum += value;
            *count += 1;
        }
        while self.kept.front().is_some_and(|first| first.start + KEPT_MS <= start) {
            self.kept.pop_front();
        }
    }

    /// The minute under way kept as it is.
    fn close(&mut self) {
        if let Some((start, samples, sums)) = self.open.take() {
            let mut values: Vec<(u32, f32, u16)> = sums.into_iter().map(|(number, (sum, count))| (number, (sum / f64::from(count)) as f32, u16::try_from(count).unwrap_or(u16::MAX))).collect();
            values.sort_unstable_by_key(|&(number, ..)| number);
            self.kept.push_back(Minute { start, samples, values });
        }
    }

    /// The minutes kept whole that began at `from` or after and ended by `until` (ms since 1970), oldest first.
    pub fn between(&self, from: u64, until: u64) -> Vec<Minute> {
        self.kept.iter().filter(|minute| minute.start >= from && minute.start + MINUTE_MS <= until).cloned().collect()
    }

    /// `series`' average over `minute`, and how many of its samples had it: none where none did.
    pub fn value(&self, minute: &Minute, series: &Series) -> Option<(f64, u32)> {
        let number = *self.numbers.get(series.name().as_str())?;
        let at = minute.values.binary_search_by_key(&number, |&(number, ..)| number).ok()?;
        let (_, average, count) = minute.values[at];
        Some((f64::from(average), u32::from(count)))
    }

    /// How many minutes are kept, and when the first began.
    pub fn span(&self) -> (usize, Option<u64>) {
        (self.kept.len(), self.kept.front().map(|minute| minute.start))
    }
}

/// The day of minutes, while the interface is on.
static MINUTES: Mutex<Option<Minutes>> = Mutex::new(None);

/// Whether a day of minutes is kept, from now: kept from nothing as it is turned on, let go of as it is turned off.
pub fn keep(on: bool) {
    let mut minutes = MINUTES.lock().unwrap();
    match (on, minutes.is_some()) {
        (true, false) => *minutes = Some(Minutes::default()),
        (false, true) => *minutes = None,
        _ => {}
    }
}

/// `sample`, just read, counted into its minute, where minutes are kept.
pub fn heard(sample: &Sample) {
    if let Some(minutes) = MINUTES.lock().unwrap().as_mut() {
        minutes.add(sample);
    }
}

/// What `read` makes of the minutes kept, where they are.
pub fn with_minutes<R>(read: impl FnOnce(Option<&Minutes>) -> R) -> R {
    read(MINUTES.lock().unwrap().as_ref())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Every series is named as it is read, and only names of series are read.
    #[test]
    fn series_are_read_by_their_names() {
        for name in [
            "cpu.percent",
            "gpu0.percent",
            "gpu12.memory_used_bytes",
            "drive3.celsius",
            "board.CPU Socket.celsius",
            "fan.CPU_FAN.rpm",
            "program.rustc.exe.cpu_percent",
            "program.code.exe.memory_bytes",
        ] {
            assert_eq!(Series::parse(name).map(|series| series.name()), Some(name.to_lowercase().replace("board.cpu socket", "board.CPU Socket").replace("fan.cpu_fan", "fan.CPU_FAN")), "{name}");
        }
        assert_eq!(Series::parse("program.RustC.EXE.cpu_percent"), Some(Series::Program("rustc.exe".into(), ProgramSeries::Cpu)));
        for wrong in ["cpu", "cpu.percnt", "gpu.percent", "gpux.percent", "gpu0.fps", "drive-1.celsius", "board..celsius", "program..cpu_percent", "program.x.exe"] {
            assert_eq!(Series::parse(wrong), None, "{wrong}");
        }
    }

    pub(crate) fn sample(t: u64, cpu: Option<f32>) -> Sample {
        serde_json::from_value(serde_json::json!({
            "t": t, "cpu": cpu, "threads": [], "ghz": null, "kinds_ghz": [], "threads_ghz": [],
            "memory": { "used": 100, "committed": 200, "commit_limit": 300, "cached": 50 },
            "gpus": [], "net_down": null, "net_up": null, "net_total_down": 0, "net_total_up": 0, "network": null,
            "disk_read": null, "disk_write": null, "disk_active": null, "volumes": [],
            "processes": [{ "name": "rustc.exe", "cpu": 50.0, "mem": 1000, "io": 0.0, "gpu": null }],
            "system": { "uptime_s": 1, "processes": 1, "threads": 1, "handles": 1 },
            "battery": null, "cpu_sensors": null, "board": null, "drive_temps": [], "dimm_temps": [], "mic_muted": null,
            "game": null, "wsl": null, "docker": null
        }))
        .unwrap()
    }

    /// Each minute keeps each series' average over the samples that had it, once the next minute begins; a series
    /// none of a minute's samples had has none. A day of minutes is kept, no more.
    #[test]
    fn minutes_keep_each_series_average() {
        let mut minutes = Minutes::default();
        let base = 1_000 * MINUTE_MS;
        minutes.add(&sample(base, Some(10.0)));
        minutes.add(&sample(base + 1_000, None));
        minutes.add(&sample(base + 2_000, Some(30.0)));
        assert!(minutes.between(0, u64::MAX).is_empty(), "the minute under way is not kept yet");
        minutes.add(&sample(base + MINUTE_MS, None));
        let kept = minutes.between(0, u64::MAX);
        assert_eq!(kept.len(), 1);
        assert_eq!((kept[0].start, kept[0].samples), (base, 3));
        assert_eq!(minutes.value(&kept[0], &Series::CpuPercent), Some((20.0, 2)), "of the two samples that had it");
        assert_eq!(minutes.value(&kept[0], &Series::MemoryUsed), Some((100.0, 3)));
        assert_eq!(minutes.value(&kept[0], &Series::Program("rustc.exe".into(), ProgramSeries::Cpu)), Some((50.0, 3)));
        assert_eq!(minutes.value(&kept[0], &Series::GameFps), None);
        // A day on: the first minute let go of.
        minutes.add(&sample(base + 2 * MINUTE_MS, None));
        minutes.add(&sample(base + KEPT_MS + MINUTE_MS, None));
        assert_eq!(minutes.between(0, u64::MAX).iter().map(|minute| minute.start).collect::<Vec<_>>(), [base + 2 * MINUTE_MS]);
        // The clock set back: what was after it let go of.
        minutes.add(&sample(base + MINUTE_MS, Some(5.0)));
        assert!(minutes.between(0, u64::MAX).is_empty());
    }
}
