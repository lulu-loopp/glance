//! The wireless devices as a sample carries them (from `wireless`, read in Glance's own process), and the notification
//! of one running low, where the devices' module asks for it.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::reading::{DeviceKind, DeviceSample, DeviceVia, LOW_CHARGE};
use crate::ui::text::Lang;
use crate::wireless::{self, Kind, Via};

/// The devices connected now with their charge, as a sample carries them, while `on` (their module on: else none, and
/// none read): the reader started as they are first asked for, in the order they are shown (by what they are, then by
/// name). Kept as the latest too, for what else shows them (see `latest`).
pub fn read(on: bool) -> Vec<DeviceSample> {
    let devices = if on {
        wireless::start();
        let mut devices: Vec<DeviceSample> = wireless::now().into_iter().map(of).collect();
        devices.sort_by(|a, b| (a.kind as u8, &a.name, &a.key).cmp(&(b.kind as u8, &b.name, &b.key)));
        devices
    } else {
        Vec::new()
    };
    *LATEST.lock().unwrap() = devices.clone();
    devices
}

static LATEST: Mutex<Vec<DeviceSample>> = Mutex::new(Vec::new());

/// The devices as last read (see `read`): what the sound's outputs are matched to (see `ui::outputs`).
pub fn latest() -> Vec<DeviceSample> {
    LATEST.lock().unwrap().clone()
}

/// Of `devices`, the one whose container is `container` (an output's: see `audio::Output::container`).
pub fn in_container<'d>(devices: &'d [DeviceSample], container: Option<&str>) -> Option<&'d DeviceSample> {
    let container = container?;
    devices.iter().find(|device| device.container.as_deref() == Some(container))
}

fn of(device: wireless::Device) -> DeviceSample {
    DeviceSample {
        key: device.key,
        name: device.name,
        kind: match device.kind {
            Kind::Headphones => DeviceKind::Headphones,
            Kind::Earbuds => DeviceKind::Earbuds,
            Kind::Headset => DeviceKind::Headset,
            Kind::Speaker => DeviceKind::Speaker,
            Kind::Keyboard => DeviceKind::Keyboard,
            Kind::Mouse => DeviceKind::Mouse,
            Kind::Gamepad => DeviceKind::Gamepad,
            Kind::Phone => DeviceKind::Phone,
            Kind::Watch => DeviceKind::Watch,
            Kind::Other => DeviceKind::Other,
        },
        via: match device.via {
            Via::Bluetooth => DeviceVia::Bluetooth,
            Via::Logitech => DeviceVia::Logitech,
            Via::Xbox => DeviceVia::Xbox,
            Via::Receiver => DeviceVia::Receiver,
            Via::Cable => DeviceVia::Cable,
        },
        level: device.level.min(100),
        charging: device.charging,
        container: device.container.map(|id| format!("{id:032x}")),
    }
}

/// The charges (percent) a device running down is told of at, the first first: once as it comes to each.
const TOLD_AT: [u8; 2] = [LOW_CHARGE, 10];

/// How far above the first a device's charge comes back (charged a little, or read a point higher) before it is told
/// of again as it runs down: a reading that wavers about it is told of once.
const TOLD_AGAIN_ABOVE: u8 = 5;

/// What was told of each device running low, by its key: the lowest charge it was told at.
#[derive(Default)]
pub struct Notices {
    told: HashMap<String, u8>,
}

impl Notices {
    /// Takes in the devices of a sample: those come down to a charge they are told at, told of, where `on` (the
    /// switch of the devices' module) asks for it.
    pub fn look(&mut self, devices: &[DeviceSample], on: bool, lang: Lang) {
        for device in devices {
            let told = self.told.get(&device.key).copied();
            let (now, tell) = step(told, device.level, device.charging);
            match now {
                Some(at) => self.told.insert(device.key.clone(), at),
                None => self.told.remove(&device.key),
            };
            if tell && on {
                notify(device, lang);
            }
        }
    }
}

/// From what was told of a device (`told`: the lowest charge it was told at), at charge `level`, charging or not: what
/// is told of it now, and whether it is told of again.
fn step(told: Option<u8>, level: u8, charging: bool) -> (Option<u8>, bool) {
    if charging || level > LOW_CHARGE + TOLD_AGAIN_ABOVE {
        return (None, false);
    }
    match TOLD_AT.iter().rev().find(|at| level <= **at) {
        Some(&at) if told.is_none_or(|told| at < told) => (Some(at), true),
        _ => (told, false),
    }
}

/// The notification of `device` running low (one for each device, the later taking its place).
fn notify(device: &DeviceSample, lang: Lang) {
    use crate::toast::escaped;
    let (title, line) = match lang {
        Lang::Zh => (format!("{} 电量低", device.called(true)), format!("剩余 {}%，请及时充电。", device.level)),
        _ => (format!("{} is low on charge", device.called(false)), format!("{}% left. Charge it soon.", device.level)),
    };
    let xml = format!(r#"<toast launch="devices:"><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual></toast>"#, escaped(&title), escaped(&line));
    if let Err(error) = crate::toast::show(&xml, &device.key, "devices", false) {
        crate::journal::note(format!("devices: a device running low not told of ({error})"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A device running down from full is told of at 20 % and again at 10 %, each once, its reading wavering about
    /// them or not. Charged, it is told of again the next time it runs down.
    #[test]
    fn a_device_is_told_of_at_a_fifth_and_a_tenth_each_once() {
        let run = |levels: &[(u8, bool)]| {
            let mut told = None;
            levels
                .iter()
                .filter_map(|&(level, charging)| {
                    let (now, tell) = step(told, level, charging);
                    told = now;
                    tell.then_some(level)
                })
                .collect::<Vec<_>>()
        };
        let down: Vec<(u8, bool)> = (0..=100).rev().map(|level| (level, false)).collect();
        assert_eq!(run(&down), [20, 10]);
        assert_eq!(run(&[(21, false), (20, false), (21, false), (20, false), (19, false), (11, false), (10, false), (11, false), (9, false)]), [20, 10]);
        // Charged up past the fifth and run down again: told again. Charging: never told.
        assert_eq!(run(&[(20, false), (30, true), (60, false), (20, false)]), [20, 20]);
        assert_eq!(run(&[(15, true), (8, true)]), Vec::<u8>::new());
        // Come to be read already low: told of at once, at the lower charge it is under.
        assert_eq!(run(&[(8, false), (6, false)]), [8]);
    }
}
