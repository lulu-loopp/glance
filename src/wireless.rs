//! The wireless devices connected now and the charge each has left (stand-in till the readers land).

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub key: String,
    pub name: String,
    pub kind: Kind,
    pub via: Via,
    pub level: u8,
    pub charging: bool,
    pub container: Option<u128>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Headphones,
    Earbuds,
    Headset,
    Speaker,
    Keyboard,
    Mouse,
    Gamepad,
    Phone,
    Watch,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    Bluetooth,
    Logitech,
    Xbox,
}

pub fn start() {}

pub fn now() -> Vec<Device> {
    Vec::new()
}
