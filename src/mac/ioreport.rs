//! Power and clocks on Apple silicon, from IOReport (private API, declared
//! here, which needs no rights to call): the energy the CPU and the GPU used
//! since the last look, and how long each core and the GPU spent at each of
//! their performance states, priced by the clocks the power manager lists.

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_core_foundation::{CFArray, CFDictionary, CFMutableDictionary, CFRetained, CFString, CFType};

use super::iokit;

#[link(name = "IOReport")]
extern "C" {
    fn IOReportCopyChannelsInGroup(group: &CFString, subgroup: Option<&CFString>, a: u64, b: u64, c: u64) -> *mut CFMutableDictionary;
    fn IOReportMergeChannels(into: &CFMutableDictionary, from: &CFMutableDictionary, unused: *const c_void);
    fn IOReportCreateSubscription(
        unused: *const c_void,
        channels: &CFMutableDictionary,
        subscribed: *mut *mut CFMutableDictionary,
        id: u64,
        unused2: *const c_void,
    ) -> *mut CFType;
    fn IOReportCreateSamples(subscription: &CFType, channels: &CFMutableDictionary, unused: *const c_void) -> *mut CFDictionary;
    fn IOReportCreateSamplesDelta(previous: &CFDictionary, current: &CFDictionary, unused: *const c_void) -> *mut CFDictionary;
    fn IOReportChannelGetGroup(channel: *const c_void) -> *const CFString;
    fn IOReportChannelGetChannelName(channel: *const c_void) -> *const CFString;
    fn IOReportChannelGetUnitLabel(channel: *const c_void) -> *const CFString;
    fn IOReportSimpleGetIntegerValue(channel: *const c_void, unused: i32) -> i64;
    fn IOReportStateGetCount(channel: *const c_void) -> i32;
    fn IOReportStateGetNameForIndex(channel: *const c_void, index: i32) -> *const CFString;
    fn IOReportStateGetResidency(channel: *const c_void, index: i32) -> i64;
}

/// What a look at IOReport finds, over the time since the last one.
#[derive(Default)]
pub struct Reading {
    /// Watts.
    pub cpu_power: Option<f32>,
    pub gpu_power: Option<f32>,
    /// The cores' clock, and the GPU's, where they were working (MHz).
    pub cpu_mhz: Option<f32>,
    pub gpu_mhz: Option<f32>,
}

pub struct Report {
    subscription: CFRetained<CFType>,
    channels: CFRetained<CFMutableDictionary>,
    previous: CFRetained<CFDictionary>,
    /// The clocks of each performance state, in MHz, lowest first: the
    /// efficiency cores', the performance cores', the GPU's.
    efficiency: Vec<f32>,
    performance: Vec<f32>,
    graphics: Vec<f32>,
}

/// A string IOReport hands back without a reference of its own.
fn text(string: *const CFString) -> String {
    if string.is_null() { String::new() } else { unsafe { &*string }.to_string() }
}

impl Report {
    pub fn open() -> Option<Self> {
        let energy = CFString::from_str("Energy Model");
        let channels = NonNull::new(unsafe { IOReportCopyChannelsInGroup(&energy, None, 0, 0, 0) })?;
        let channels = unsafe { CFRetained::from_raw(channels) };
        for (group, subgroup) in [("CPU Stats", "CPU Core Performance States"), ("GPU Stats", "GPU Performance States")] {
            let (group, subgroup) = (CFString::from_str(group), CFString::from_str(subgroup));
            if let Some(more) = NonNull::new(unsafe { IOReportCopyChannelsInGroup(&group, Some(&subgroup), 0, 0, 0) }) {
                let more = unsafe { CFRetained::from_raw(more) };
                unsafe { IOReportMergeChannels(&channels, &more, std::ptr::null()) };
            }
        }
        let mut subscribed: *mut CFMutableDictionary = std::ptr::null_mut();
        let subscription = NonNull::new(unsafe { IOReportCreateSubscription(std::ptr::null(), &channels, &mut subscribed, 0, std::ptr::null()) })?;
        let subscription = unsafe { CFRetained::from_raw(subscription) };
        let subscribed = unsafe { CFRetained::from_raw(NonNull::new(subscribed)?) };
        let previous = NonNull::new(unsafe { IOReportCreateSamples(&subscription, &subscribed, std::ptr::null()) })?;
        let previous = unsafe { CFRetained::from_raw(previous) };
        let pmgr = iokit::services("AppleARMIODevice").into_iter().find(|device| device.name() == "pmgr");
        let table = |key: &str| pmgr.as_ref().and_then(|pmgr| pmgr.data(key)).map(|bytes| clocks(&bytes)).unwrap_or_default();
        Some(Report {
            subscription,
            channels: subscribed,
            previous,
            efficiency: table("voltage-states1-sram"),
            performance: table("voltage-states5-sram"),
            graphics: table("voltage-states9-sram"),
        })
    }

    /// What was used, and at what clocks, since the last look `seconds` ago.
    pub fn read(&mut self, seconds: f64) -> Reading {
        let Some(current) = NonNull::new(unsafe { IOReportCreateSamples(&self.subscription, &self.channels, std::ptr::null()) }) else {
            return Reading::default();
        };
        let current = unsafe { CFRetained::from_raw(current) };
        let delta = NonNull::new(unsafe { IOReportCreateSamplesDelta(&self.previous, &current, std::ptr::null()) });
        self.previous = current;
        let Some(delta) = delta else { return Reading::default() };
        let delta = unsafe { CFRetained::from_raw(delta) };
        let Some(channels) = iokit::get(&delta, "IOReportChannels").and_then(|channels| channels.downcast::<CFArray>().ok()) else {
            return Reading::default();
        };
        let mut reading = Reading::default();
        // Clocks weighed by the time spent at them: (MHz × time, time).
        let (mut cpu, mut gpu) = ((0.0f64, 0.0f64), (0.0f64, 0.0f64));
        for i in 0..channels.count() {
            let channel = unsafe { channels.value_at_index(i) };
            let group = text(unsafe { IOReportChannelGetGroup(channel) });
            let name = text(unsafe { IOReportChannelGetChannelName(channel) });
            match group.as_str() {
                "Energy Model" if name == "CPU Energy" || name == "GPU Energy" => {
                    let joules = energy(channel);
                    let watts = Some((joules / seconds) as f32);
                    if name == "CPU Energy" { reading.cpu_power = watts } else { reading.gpu_power = watts }
                }
                "CPU Stats" if name.starts_with("ECPU") => weigh(channel, &self.efficiency, &mut cpu),
                "CPU Stats" if name.starts_with("PCPU") => weigh(channel, &self.performance, &mut cpu),
                "GPU Stats" if name.starts_with("GPUPH") => weigh(channel, &self.graphics, &mut gpu),
                _ => {}
            }
        }
        reading.cpu_mhz = (cpu.1 > 0.0).then(|| (cpu.0 / cpu.1) as f32);
        reading.gpu_mhz = (gpu.1 > 0.0).then(|| (gpu.0 / gpu.1) as f32);
        reading
    }
}

/// A channel's energy, in joules, by the unit it gives.
fn energy(channel: *const c_void) -> f64 {
    let value = unsafe { IOReportSimpleGetIntegerValue(channel, 0) } as f64;
    let scale = match text(unsafe { IOReportChannelGetUnitLabel(channel) }).as_str() {
        "mJ" => 1e-3,
        "uJ" | "µJ" => 1e-6,
        "nJ" => 1e-9,
        _ => 1.0,
    };
    value * scale
}

/// Adds a channel's time at each working state, priced by `clocks`, to
/// `total`. States that are not working (idle, off, down) count for
/// nothing. The power manager lists a clock for every state, or for every
/// state but the first idle one: the list lines up from its end.
fn weigh(channel: *const c_void, clocks: &[f32], total: &mut (f64, f64)) {
    let states = unsafe { IOReportStateGetCount(channel) }.max(0) as usize;
    if clocks.is_empty() || clocks.len() > states {
        return;
    }
    let offset = states - clocks.len();
    for state in offset..states {
        let name = text(unsafe { IOReportStateGetNameForIndex(channel, state as i32) });
        if matches!(name.as_str(), "IDLE" | "OFF" | "DOWN") {
            continue;
        }
        let time = unsafe { IOReportStateGetResidency(channel, state as i32) }.max(0) as f64;
        let clock = clocks[state - offset] as f64;
        if clock > 0.0 {
            total.0 += clock * time;
            total.1 += time;
        }
    }
}

/// A power manager's table of clocks, as MHz: pairs of (clock, voltage) as
/// little-endian 32-bit numbers. The clocks are in Hz on some chips and
/// tables, in kHz on others, with nothing to say which: as no core of Apple
/// silicon tops out below 100 MHz nor above 100 GHz, a table whose fastest
/// clock exceeds 10^8 is in Hz, any other in kHz.
fn clocks(bytes: &[u8]) -> Vec<f32> {
    let raw: Vec<u32> = bytes.chunks_exact(8).map(|pair| u32::from_le_bytes([pair[0], pair[1], pair[2], pair[3]])).collect();
    let hz = raw.iter().copied().max().unwrap_or(0) > 100_000_000;
    raw.into_iter().map(|clock| if hz { clock as f32 / 1e6 } else { clock as f32 / 1e3 }).collect()
}

#[cfg(test)]
mod tests {
    use super::clocks;

    #[test]
    fn reads_clock_tables_in_either_unit() {
        // M4's efficiency cores, in kHz: 900 MHz first.
        let khz = [0xa0, 0xbb, 0x0d, 0x00, 0x16, 0x03, 0x00, 0x00, 0xe0, 0x20, 0x2c, 0x00, 0xed, 0x03, 0x00, 0x00];
        assert_eq!(clocks(&khz), vec![900.0, 2892.0]);
        // M4's GPU, in Hz, with its off state first.
        let hz = [0x00, 0x00, 0x00, 0x00, 0x0c, 0x03, 0x00, 0x00, 0x80, 0x78, 0x25, 0x14, 0x0c, 0x03, 0x00, 0x00];
        assert_eq!(clocks(&hz), vec![0.0, 338.0]);
    }
}
