//! The calls on what Glance reads: how it stands, the hardware, the latest sample, the readings over time, the busiest
//! programs, and what AI tools spent.

use super::history::{Minute, Series, MINUTE_MS};
use super::tools::Args;
use super::Failure;
use crate::reading::{Sample, StaticInfo};
use serde_json::{json, Map, Value};

/// The parts of a sample `get_readings` may be asked for.
pub const PARTS: [&str; 13] = ["cpu", "gpu", "memory", "network", "disk", "volumes", "sensors", "battery", "game", "devices", "wsl", "docker", "system"];

/// A reading of single precision as a number, to the thousandth (its last places are noise).
fn num(value: f32) -> Value {
    if !value.is_finite() {
        return Value::Null;
    }
    json!((f64::from(value) * 1000.0).round() / 1000.0)
}

fn maybe(value: Option<f32>) -> Value {
    value.map_or(Value::Null, num)
}

/// A rate, or a count, in whole units.
fn whole(value: Option<f64>) -> Value {
    value.filter(|value| value.is_finite()).map_or(Value::Null, |value| json!(value.round() as i64))
}

pub fn status() -> Result<Value, Failure> {
    let app = crate::app();
    let interval = app.settings.lock().unwrap().interval_ms;
    let samples = app.controller.history.samples();
    let (minutes, oldest_minute) = super::history::with_minutes(|minutes| minutes.map_or((0, None), |minutes| minutes.span()));
    let standing = crate::search::standing();
    let (complete, _) = crate::search::listed();
    let readings = if crate::source::linked() {
        "sensors"
    } else if crate::elevation::is_elevated() {
        "direct"
    } else {
        "own"
    };
    Ok(json!({
        "glance": super::VERSION,
        "interface": super::INTERFACE,
        "interval_ms": interval,
        "readings": readings,
        "elevated": crate::elevation::is_elevated(),
        "history": {
            "samples": samples.len(),
            "oldest_ms": samples.first().map(|sample| sample.t),
            "newest_ms": samples.last().map(|sample| sample.t),
            "minutes": minutes,
            "oldest_minute_ms": oldest_minute,
        },
        "search": {
            "started": !standing.is_empty() || crate::search::drives_found(),
            "complete": complete,
            "drives": standing.iter().map(|drive| json!({ "root": drive.root, "ready": drive.ready, "current": drive.current, "names": drive.names })).collect::<Vec<_>>(),
        },
    }))
}

pub fn hardware() -> Result<Value, Failure> {
    Ok(hardware_of(&crate::app().controller.info()))
}

fn hardware_of(info: &StaticInfo) -> Value {
    json!({
        "cpu": info.cpu_name,
        "threads": info.threads,
        "board": info.board,
        "memory": {
            "total_bytes": info.mem_total,
            "modules": info.memory_modules,
            "speed_mts": info.memory_speed.map(|(speed, _)| speed),
            "rated_mts": info.memory_speed.and_then(|(_, rated)| rated),
        },
        "gpus": info.gpus.iter().enumerate().map(|(index, gpu)| json!({ "index": index, "name": gpu.name, "memory_bytes": gpu.mem_total, "shared_bytes": gpu.shared_total })).collect::<Vec<_>>(),
        "drives": info.drive_ids.iter().zip(&info.drives).map(|(id, model)| json!({ "id": id, "model": model })).collect::<Vec<_>>(),
        "network_adapter": info.network_adapter,
    })
}

/// The latest sample, or why there is none.
fn latest() -> Result<std::sync::Arc<Sample>, Failure> {
    crate::app().controller.history.latest().ok_or_else(|| Failure::new("no_readings_yet", "Glance has not read anything yet. Try again in a few seconds."))
}

pub fn readings(args: &Args) -> Result<Value, Failure> {
    let parts = args.strings("parts")?;
    let sample = latest()?;
    Ok(readings_of(&sample, &crate::app().controller.info(), parts.as_deref()))
}

/// `sample`'s `parts` (none: all) as `get_readings` tells them, its GPUs named as `info` names them.
fn readings_of(sample: &Sample, info: &StaticInfo, parts: Option<&[String]>) -> Value {
    let asked = |part: &str| parts.is_none_or(|parts| parts.iter().any(|asked| asked == part));
    let mut told = Map::new();
    told.insert("t_ms".into(), json!(sample.t));
    if asked("cpu") {
        let sensors = sample.cpu_sensors.as_ref();
        told.insert(
            "cpu".into(),
            json!({
                "percent": maybe(sample.cpu),
                "utility_percent": maybe(sample.cpu_utility),
                "ghz": maybe(sample.ghz),
                "kinds_ghz": sample.kinds_ghz.iter().map(|ghz| maybe(*ghz)).collect::<Vec<_>>(),
                "threads_percent": sample.threads.iter().map(|percent| maybe(*percent)).collect::<Vec<_>>(),
                "celsius": maybe(sensors.and_then(|sensors| sensors.temp)),
                "ccds_celsius": sensors.map(|sensors| sensors.ccds.iter().map(|(ccd, celsius)| json!([ccd, num(*celsius)])).collect::<Vec<_>>()).unwrap_or_default(),
                "watts": maybe(sensors.and_then(|sensors| sensors.power)),
                "graphics_watts": maybe(sensors.and_then(|sensors| sensors.graphics_power)),
            }),
        );
    }
    if asked("gpu") {
        let gpus: Vec<Value> = sample
            .gpus
            .iter()
            .enumerate()
            .map(|(index, gpu)| {
                json!({
                    "index": index,
                    "name": info.gpus.get(index).map(|known| known.name.as_str()),
                    "percent": maybe(gpu.usage),
                    "engines": gpu.engines.as_ref().map(|engines| engines.iter().map(|(engine, percent)| json!([engine, num(*percent)])).collect::<Vec<_>>()),
                    "memory_used_bytes": gpu.mem_used,
                    "shared_used_bytes": gpu.shared_used,
                    "celsius": maybe(gpu.temp),
                    "hot_spot_celsius": maybe(gpu.hot_spot),
                    "mhz": maybe(gpu.clock_mhz),
                    "fan_rpm": gpu.fan_rpm,
                    "watts": maybe(gpu.power),
                    "asleep": gpu.asleep,
                })
            })
            .collect();
        told.insert("gpu".into(), json!(gpus));
    }
    if asked("memory") {
        told.insert(
            "memory".into(),
            json!({
                "used_bytes": sample.memory.used,
                "total_bytes": info.mem_total,
                "committed_bytes": sample.memory.committed,
                "commit_limit_bytes": sample.memory.commit_limit,
                "cached_bytes": sample.memory.cached,
                "dimm_celsius": sample.dimm_temps.iter().map(|celsius| num(*celsius)).collect::<Vec<_>>(),
            }),
        );
    }
    if asked("network") {
        told.insert(
            "network".into(),
            json!({
                "down_bytes_per_s": whole(sample.net_down),
                "up_bytes_per_s": whole(sample.net_up),
                "total_down_bytes": sample.net_total_down,
                "total_up_bytes": sample.net_total_up,
                "adapter": sample.network.as_ref().map(|network| json!({
                    "name": network.name,
                    "model": network.model,
                    "link_bps": network.link_bps,
                    "ipv4": network.ipv4,
                    "ipv6": network.ipv6,
                    "gateway": network.gateway,
                    "wifi": network.wifi,
                })),
                "ping_ms": maybe(sample.ping_ms),
                "ping_loss_percent": maybe(sample.ping_loss.map(|loss| loss * 100.0)),
            }),
        );
    }
    if asked("disk") {
        let mut ids: Vec<u32> = sample.drive_rates.iter().map(|rate| rate.id).chain(sample.drive_temps.iter().map(|drive| drive.id)).collect();
        ids.sort_unstable();
        ids.dedup();
        let drives: Vec<Value> = ids
            .into_iter()
            .map(|id| {
                let rate = sample.drive_rates.iter().find(|rate| rate.id == id);
                json!({
                    "id": id,
                    "read_bytes_per_s": whole(rate.and_then(|rate| rate.read)),
                    "write_bytes_per_s": whole(rate.and_then(|rate| rate.write)),
                    "celsius": maybe(sample.drive_temps.iter().find(|drive| drive.id == id).map(|drive| drive.celsius)),
                })
            })
            .collect();
        told.insert(
            "disk".into(),
            json!({ "read_bytes_per_s": whole(sample.disk_read), "write_bytes_per_s": whole(sample.disk_write), "active_percent": maybe(sample.disk_active), "drives": drives }),
        );
    }
    if asked("volumes") {
        told.insert("volumes".into(), json!(sample.volumes.iter().map(|volume| json!({ "name": volume.name, "used_bytes": volume.used, "total_bytes": volume.total })).collect::<Vec<_>>()));
    }
    if asked("sensors") {
        if let Some(board) = &sample.board {
            told.insert(
                "sensors".into(),
                json!({
                    "board_celsius": board.temps.iter().map(|(name, celsius)| json!([name, num(*celsius)])).collect::<Vec<_>>(),
                    "fans_rpm": board.fans.iter().map(|(name, rpm)| json!([name, num(*rpm)])).collect::<Vec<_>>(),
                }),
            );
        }
    }
    if asked("battery") {
        if let Some(battery) = &sample.battery {
            told.insert(
                "battery".into(),
                json!({ "percent": battery.percent, "charging": battery.charging, "plugged": battery.plugged, "seconds_left": battery.seconds_left, "watts": maybe(battery.watts), "health_percent": maybe(battery.health) }),
            );
        }
    }
    if asked("game") {
        // (Its window's title is not told: it may name a document, or a chat.)
        told.insert(
            "game".into(),
            sample.game.as_ref().map_or(Value::Null, |game| {
                json!({
                    "program": game.program,
                    "fps": maybe(game.fps),
                    "low_1_percent_fps": maybe(game.low),
                    "longest_frame_ms": maybe(game.longest_ms),
                    "refresh_hz": game.refresh_hz,
                    "gpu_index": game.gpu_index,
                    "cpu_percent": maybe(game.cpu),
                    "gpu_percent": maybe(game.gpu),
                    "memory_bytes": game.mem,
                    "vram_bytes": game.vram,
                    "gpu_limit": game.gpu_limit.map(|limit| match limit {
                        crate::reading::GpuLimit::Free => "free",
                        crate::reading::GpuLimit::Power => "power",
                        crate::reading::GpuLimit::Thermal => "thermal",
                        crate::reading::GpuLimit::Hardware => "hardware",
                    }),
                    "playing_s": game.playing_s,
                })
            }),
        );
    }
    if asked("devices") {
        told.insert(
            "devices".into(),
            json!(sample
                .devices
                .iter()
                .map(|device| json!({
                    "name": device.called(false),
                    "kind": format!("{:?}", device.kind).to_lowercase(),
                    "via": format!("{:?}", device.via).to_lowercase(),
                    "percent": device.level,
                    "charging": device.charging,
                }))
                .collect::<Vec<_>>()),
        );
    }
    if asked("wsl") {
        if let Some(wsl) = &sample.wsl {
            told.insert(
                "wsl".into(),
                match wsl {
                    crate::reading::WslSample::Missing => json!({ "state": "missing" }),
                    crate::reading::WslSample::Stopped => json!({ "state": "stopped" }),
                    crate::reading::WslSample::Running { distros, cpu, used, total, gpu } => {
                        json!({ "state": "running", "distros": distros, "cpu_percent": maybe(*cpu), "used_bytes": used, "total_bytes": total, "gpu_percent": maybe(*gpu) })
                    }
                },
            );
        }
    }
    if asked("docker") {
        if let Some(docker) = &sample.docker {
            told.insert(
                "docker".into(),
                match docker {
                    crate::reading::DockerSample::Stopped { context } => json!({ "state": "stopped", "context": context }),
                    crate::reading::DockerSample::Unsupported { context, .. } => json!({ "state": "unsupported", "context": context }),
                    crate::reading::DockerSample::Running { context, containers } => json!({
                        "state": "running",
                        "context": context,
                        "containers": containers.iter().map(|container| json!({
                            "name": container.name,
                            "status": container.status,
                            "cpu_percent": maybe(container.cpu),
                            "memory_bytes": container.mem,
                            "limit_bytes": container.limit,
                            "net_bytes_per_s": whole(container.net),
                            "io_bytes_per_s": whole(container.io),
                        })).collect::<Vec<_>>(),
                    }),
                },
            );
        }
    }
    if asked("system") {
        told.insert(
            "system".into(),
            json!({ "uptime_s": sample.system.uptime_s, "processes": sample.system.processes, "threads": sample.system.threads, "handles": sample.system.handles }),
        );
    }
    Value::Object(told)
}

pub fn processes(args: &Args) -> Result<Value, Failure> {
    let by = args.choice("by")?.to_string();
    let limit = args.int("limit")?.unwrap_or(10) as usize;
    let name = args.string("name")?.map(str::to_lowercase);
    let sample = latest()?;
    Ok(processes_of(&sample, &by, limit, name.as_deref()))
}

/// The `limit` busiest programs of `sample` by `by`, those whose names hold `name` (folded) alone where it is given.
fn processes_of(sample: &Sample, by: &str, limit: usize, name: Option<&str>) -> Value {
    let measure = |program: &crate::reading::ProcessSample| -> Option<f64> {
        match by {
            "memory" => Some(program.mem as f64),
            "io" => Some(program.io),
            "gpu" => program.gpu.map(f64::from),
            _ => Some(f64::from(program.cpu)),
        }
    };
    let mut programs: Vec<&crate::reading::ProcessSample> = sample.processes.iter().filter(|program| name.is_none_or(|name| program.name.to_lowercase().contains(name))).collect();
    // (The busiest first, one not measured last, the same by name.)
    programs.sort_by(|a, b| measure(b).unwrap_or(f64::NEG_INFINITY).total_cmp(&measure(a).unwrap_or(f64::NEG_INFINITY)).then_with(|| a.name.cmp(&b.name)));
    programs.truncate(limit);
    json!({
        "t_ms": sample.t,
        "by": by,
        "sampled": "the top 40 programs by each measure",
        "processes": programs.iter().map(|program| json!({
            "name": program.name,
            "cpu_percent": num(program.cpu),
            "memory_bytes": program.mem,
            "io_bytes_per_s": whole(Some(program.io)),
            "gpu_percent": maybe(program.gpu),
        })).collect::<Vec<_>>(),
    })
}

pub fn history(args: &Args) -> Result<Value, Failure> {
    let names = args.strings("series")?.unwrap_or_default();
    let mut series = Vec::new();
    for name in &names {
        series.push(Series::parse(name).ok_or_else(|| Failure::invalid(format!("{name} is not a series. See the call's description for the series there are.")))?);
    }
    let seconds = args.int("seconds")?.unwrap_or(60) as u64;
    let interval = crate::app().settings.lock().unwrap().interval_ms;
    let step = args.int("step_ms")?.map(|step| step as u64);
    if let Some(step) = step.filter(|&step| step < interval) {
        return Err(Failure::invalid(format!("step_ms ({step}) must not be less than the interval Glance reads at ({interval} ms).")));
    }
    let samples = crate::app().controller.history.samples();
    Ok(super::history::with_minutes(|minutes| history_of(&samples, minutes, &series, seconds, step, interval)))
}

/// A point of the readings over time: when, how many samples it stands for (a minute's many), and each series' value.
struct Point {
    t: u64,
    weight: u32,
    values: Vec<Option<f64>>,
}

/// `series` over the last `seconds` before the newest of `samples` (every one Glance holds) and of `minutes` (the day
/// of them kept, where it is), averaged into buckets `step` wide where asked, read every `interval` ms.
fn history_of(samples: &[std::sync::Arc<Sample>], minutes: Option<&super::history::Minutes>, series: &[Series], seconds: u64, step: Option<u64>, interval: u64) -> Value {
    let first_sample = samples.first().map(|sample| sample.t);
    let newest = samples.last().map(|sample| sample.t).or_else(|| minutes.and_then(|minutes| minutes.between(0, u64::MAX).last().map(|minute| minute.start + MINUTE_MS)));
    let Some(newest) = newest else {
        return json!({ "interval_ms": interval, "covered": { "from_ms": null, "to_ms": null, "samples": 0, "gaps": [] }, "minutes_until_ms": null, "t_ms": [], "series": series.iter().map(|series| (series.name(), json!([]))).collect::<Map<_, _>>(), "unknown": series.iter().map(Series::name).collect::<Vec<_>>() });
    };
    let from = newest.saturating_sub(seconds * 1000);
    // Minutes wholly before the samples held, then the samples.
    let kept: Vec<Minute> = minutes.map(|minutes| minutes.between(from, first_sample.unwrap_or(u64::MAX))).unwrap_or_default();
    let minutes_until = kept.last().map(|minute| minute.start + MINUTE_MS);
    let mut points: Vec<(Point, u64)> = kept
        .iter()
        .map(|minute| (Point { t: minute.start, weight: minute.samples, values: series.iter().map(|series| minutes.and_then(|minutes| minutes.value(minute, series))).collect() }, MINUTE_MS))
        .collect();
    points.extend(samples.iter().filter(|sample| sample.t >= from).map(|sample| (Point { t: sample.t, weight: 1, values: series.iter().map(|series| series.of(sample)).collect() }, interval)));
    // Spans with nothing read in them: where a point is missing, the next coming nearer where the one after it was
    // due than where its own was (more than one and a half times as far as points come there: a point a little late,
    // as samples come, is not one missing).
    let mut gaps = Vec::new();
    for pair in points.windows(2) {
        let (before, after) = (&pair[0], &pair[1]);
        let spacing = before.1.max(after.1);
        // (A minute's point stands at its start: the minute reaches on to its end.)
        let end = before.0.t + if before.1 == MINUTE_MS { MINUTE_MS } else { 0 };
        if after.0.t > end + spacing + spacing / 2 {
            gaps.push(json!({ "from_ms": end, "to_ms": after.0.t }));
        }
    }
    let counted: u64 = points.iter().map(|(point, _)| u64::from(point.weight)).sum();
    let covered_from = points.first().map(|(point, _)| point.t);
    let points: Vec<Point> = match step {
        None => points.into_iter().map(|(point, _)| point).collect(),
        Some(step) => {
            // (Each when it begins, how many points are in it, and each series' weighted sum and weight.)
            type Bucket = (u64, u32, Vec<(f64, f64)>);
            let mut buckets: Vec<Bucket> = Vec::new();
            for (point, _) in points {
                let start = from + (point.t - from) / step * step;
                if buckets.last().is_none_or(|(at, ..)| *at != start) {
                    buckets.push((start, 0, vec![(0.0, 0.0); series.len()]));
                }
                let (_, weight, sums) = buckets.last_mut().unwrap();
                *weight += point.weight;
                for (sum, value) in sums.iter_mut().zip(&point.values) {
                    if let Some(value) = value {
                        sum.0 += value * f64::from(point.weight);
                        sum.1 += f64::from(point.weight);
                    }
                }
            }
            buckets.into_iter().map(|(t, weight, sums)| Point { t, weight, values: sums.into_iter().map(|(sum, count)| (count > 0.0).then(|| sum / count)).collect() }).collect()
        }
    };
    let value = |value: Option<f64>| value.map_or(Value::Null, |value| json!((value * 1000.0).round() / 1000.0));
    let mut told = Map::new();
    let mut unknown = Vec::new();
    for (at, series) in series.iter().enumerate() {
        let values: Vec<Value> = points.iter().map(|point| value(point.values[at])).collect();
        if values.iter().all(Value::is_null) {
            unknown.push(series.name());
        }
        told.insert(series.name(), json!(values));
    }
    json!({
        "interval_ms": interval,
        "covered": { "from_ms": covered_from, "to_ms": newest, "samples": counted, "gaps": gaps },
        "minutes_until_ms": minutes_until,
        "t_ms": points.iter().map(|point| point.t).collect::<Vec<_>>(),
        "series": told,
        "unknown": unknown,
    })
}

pub fn ai_usage(args: &Args, call: &super::server::Call) -> Result<Value, Failure> {
    let days = args.int("days")?.unwrap_or(7) as usize;
    // Kept current a while from now, as if it were shown: read whole as it is first wanted, which may take a while.
    let first = !super::wants_ai();
    super::AI_ASKED.store(super::now_ms(), std::sync::atomic::Ordering::Relaxed);
    if first {
        crate::ai::look_now();
    }
    let usage = loop {
        if let Some(usage) = crate::ai::usage() {
            break usage;
        }
        call.going()?;
        if call.left() < std::time::Duration::from_secs(1) {
            return Err(Failure::new("not_ready", "Glance is reading what AI tools spent on this PC. Try again in a minute."));
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    };
    Ok(usage_of(&usage, days))
}

/// What a tool is called in what is told.
fn tool_name(tool: crate::ai::Tool) -> String {
    format!("{tool:?}").to_lowercase()
}

fn usage_of(usage: &crate::ai::Usage, days: usize) -> Value {
    let days = days.min(7);
    // (Each day by its date on this PC's clock, today last: told from its noon, which no change of the clock moves
    // into another day.)
    let dates: Vec<String> = (0..days as u64)
        .rev()
        .map(|back| {
            let day = crate::ui::text::Local::at(usage.today + 12 * 3_600_000 - back * 86_400_000);
            format!("{:04}-{:02}-{:02}", day.year, day.month, day.day)
        })
        .collect();
    let tokens = |tokens: &crate::ai::Tokens| json!({ "input": tokens.input, "output": tokens.output, "cache_read": tokens.cache_read, "cache_write": tokens.cache_write, "reasoning": tokens.reasoning, "total": tokens.total() });
    json!({
        "now_ms": usage.now,
        "today_from_ms": usage.today,
        "today": usage.spent_today.iter().map(|(tool, model, spent)| json!({ "tool": tool_name(*tool), "model": model, "tokens": tokens(spent) })).collect::<Vec<_>>(),
        "dates": dates,
        "days": usage.days.iter().map(|(tool, each)| json!({ "tool": tool_name(*tool), "tokens": each[each.len() - days..].to_vec() })).collect::<Vec<_>>(),
        "limits": usage.limits.iter().map(|limit| json!({
            "tool": tool_name(limit.tool),
            "id": limit.id,
            "name": limit.name,
            "plan": limit.plan,
            "windows": limit.windows.iter().map(|window| json!({ "minutes": window.minutes, "used_percent": num(window.used), "resets_ms": window.resets })).collect::<Vec<_>>(),
            "balance": limit.balance.as_ref().map(|balance| json!({ "currency": balance.currency, "amount": balance.amount, "spent_today": balance.today })),
            "told_ms": limit.told,
        })).collect::<Vec<_>>(),
        "failed": usage.failed.iter().map(|(tool, failed)| json!({ "tool": tool_name(*tool), "why": match failed {
            crate::ai::Failed::Key => "key_refused",
            crate::ai::Failed::Limited => "asked_too_often",
            crate::ai::Failed::Unavailable => "not_reached",
        } })).collect::<Vec<_>>(),
        "unkeyed": usage.unkeyed.iter().map(|tool| tool_name(*tool)).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::history::tests::sample;
    use std::sync::Arc;

    /// The readings over time: samples within the seconds asked, the series named, null where a sample had none, one
    /// with none at all among the unknown, and a span with nothing read in it told as a gap.
    #[test]
    fn readings_over_time_are_the_seconds_asked() {
        let samples: Vec<Arc<Sample>> = [(100_000, Some(10.0)), (101_000, None), (102_000, Some(30.0)), (106_000, Some(40.0))].into_iter().map(|(t, cpu)| Arc::new(sample(t, cpu))).collect();
        let series = [Series::CpuPercent, Series::GameFps];
        let told = history_of(&samples, None, &series, 5, None, 1000);
        assert_eq!(told["t_ms"], json!([101_000, 102_000, 106_000]));
        assert_eq!(told["series"]["cpu.percent"], json!([null, 30.0, 40.0]));
        assert_eq!(told["unknown"], json!(["game.fps"]));
        assert_eq!(told["covered"]["gaps"], json!([{ "from_ms": 102_000, "to_ms": 106_000 }]));
        // Averaged into buckets of two seconds, from the first second asked.
        let told = history_of(&samples, None, &series, 6, Some(2000), 1000);
        assert_eq!(told["t_ms"], json!([100_000, 102_000, 106_000]));
        assert_eq!(told["series"]["cpu.percent"], json!([10.0, 30.0, 40.0]));
    }

    /// What AI tools spent on the last days is told with each day's date on this PC's clock, today last, as many as asked.
    #[test]
    fn days_spent_are_told_with_their_dates() {
        let today = {
            // (This PC's midnight of a day, as Glance's AI reading has it.)
            let noon = crate::ui::text::Local::at(1_795_780_800_000);
            1_795_780_800_000 - u64::from(noon.hour) * 3_600_000 - u64::from(noon.minute) * 60_000
        };
        let usage = crate::ai::Usage { now: today + 3_600_000, today, spent_today: Vec::new(), days: vec![(crate::ai::Tool::Claude, [1, 2, 3, 4, 5, 6, 7])], quarters: Vec::new(), sessions: Vec::new(), limits: Vec::new(), failed: Vec::new(), unkeyed: Vec::new() };
        let told = usage_of(&usage, 3);
        let at = |ms: u64| {
            let day = crate::ui::text::Local::at(ms);
            format!("{:04}-{:02}-{:02}", day.year, day.month, day.day)
        };
        assert_eq!(told["dates"], json!([at(today - 2 * 86_400_000 + 3_600_000 * 12), at(today - 86_400_000 + 3_600_000 * 12), at(today + 1)]));
        assert_eq!(told["days"], json!([{ "tool": "claude", "tokens": [5, 6, 7] }]));
    }

    /// Samples a few milliseconds late, as they come, leave no gap between them: one missing does.
    #[test]
    fn samples_come_late_without_a_gap() {
        let at = |times: &[u64]| -> Vec<Arc<Sample>> { times.iter().map(|&t| Arc::new(sample(t, Some(1.0)))).collect() };
        let told = history_of(&at(&[100_000, 101_003, 102_010, 102_998, 104_004]), None, &[Series::CpuPercent], 10, None, 1000);
        assert_eq!(told["covered"]["gaps"], json!([]), "late, not missing");
        let told = history_of(&at(&[100_000, 101_003, 103_020, 104_010]), None, &[Series::CpuPercent], 10, None, 1000);
        assert_eq!(told["covered"]["gaps"], json!([{ "from_ms": 101_003, "to_ms": 103_020 }]), "one missing");
    }

    /// Older than the samples held, the minutes kept come first, each its series' average, said up to when they reach.
    #[test]
    fn older_readings_are_minutes() {
        let mut minutes = crate::agent::history::Minutes::default();
        for second in 0..120 {
            minutes.add(&sample(600_000 + second * 1000, Some(if second < 60 { 20.0 } else { 40.0 })));
        }
        let samples: Vec<Arc<Sample>> = (0..5).map(|second| Arc::new(sample(720_000 + second * 1000, Some(50.0)))).collect();
        minutes.add(&samples[0]);
        let told = history_of(&samples, Some(&minutes), &[Series::CpuPercent], 3600, None, 1000);
        assert_eq!(told["t_ms"], json!([600_000, 660_000, 720_000, 721_000, 722_000, 723_000, 724_000]));
        assert_eq!(told["series"]["cpu.percent"], json!([20.0, 40.0, 50.0, 50.0, 50.0, 50.0, 50.0]));
        assert_eq!(told["minutes_until_ms"], json!(720_000));
        assert_eq!(told["covered"]["samples"], json!(125));
        assert_eq!(told["covered"]["gaps"], json!([]));
    }

    /// The busiest programs by the measure asked, one not measured by it last, those whose names hold what was asked.
    #[test]
    fn programs_are_ranked_by_the_measure_asked() {
        let mut sample = sample(1, None);
        sample.processes = serde_json::from_value(json!([
            { "name": "a.exe", "cpu": 5.0, "mem": 300, "io": 1.0, "gpu": null },
            { "name": "B.exe", "cpu": 50.0, "mem": 100, "io": 3.0, "gpu": 10.0 },
            { "name": "c.exe", "cpu": 20.0, "mem": 200, "io": 2.0, "gpu": 20.0 }
        ]))
        .unwrap();
        let names = |told: Value| told["processes"].as_array().unwrap().iter().map(|program| program["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(names(processes_of(&sample, "cpu", 10, None)), ["B.exe", "c.exe", "a.exe"]);
        assert_eq!(names(processes_of(&sample, "memory", 2, None)), ["a.exe", "c.exe"]);
        assert_eq!(names(processes_of(&sample, "gpu", 10, None)), ["c.exe", "B.exe", "a.exe"]);
        assert_eq!(names(processes_of(&sample, "cpu", 10, Some("b.e"))), ["B.exe"]);
    }

    /// Only the parts asked are told, a part not read on this PC not at all, and a game's window title never.
    #[test]
    fn only_the_parts_asked_are_told() {
        let info: StaticInfo = serde_json::from_value(json!({ "cpu_name": "CPU", "memory_modules": null, "memory_speed": null, "drives": [], "drive_ids": [], "network_adapter": null, "board": "", "threads": 8, "mem_total": 1000, "gpus": [], "found": [] })).unwrap();
        let sample = sample(5, Some(12.3456));
        let told = readings_of(&sample, &info, Some(&["cpu".to_string(), "battery".to_string()]));
        assert_eq!(told.as_object().unwrap().keys().collect::<Vec<_>>(), ["cpu", "t_ms"]);
        assert_eq!(told["cpu"]["percent"], json!(12.346));
        let all = readings_of(&sample, &info, None);
        assert!(all.get("battery").is_none() && all.get("sensors").is_none());
        assert_eq!(all["memory"]["total_bytes"], json!(1000));
    }
}
