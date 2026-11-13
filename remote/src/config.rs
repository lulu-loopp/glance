//! What the helper keeps: the port it listens on, the key shared with the
//! paired phone, and how the lights are when they are on. In the user's own
//! folder; the helper runs as the user.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Night mode as it is to be ended: the power plan or mode to go back to, and whether night muted the sound (it was
/// not muted already), to be unmuted again.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Night {
    pub power: Option<crate::power::Before>,
    pub muted: bool,
}

/// The port the helper listens on unless another is kept.
const PORT: u16 = 47631;
/// How many LEDs a header for addressable strips is taken to have while
/// OpenRGB has none for it: enough for a few fans in a chain.
const STRIP_LEDS: u32 = 120;

/// What the lights do while they are on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    /// One colour, steady.
    #[default]
    Steady,
    /// That colour, swelling and fading.
    Breathing,
    /// Every colour in turn (the colour chosen is not used).
    Rainbow,
    /// A colour for how warm the PC is, from blue through to red (the
    /// colour chosen, while Glance is not there to say how warm).
    Heat,
}

/// How the screens are put out (see `display`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Screens {
    /// Windows stops the signal to every monitor.
    #[default]
    Signal,
    /// The signal is kept: each monitor is turned off by itself, or covered in black.
    Kept,
}

/// How the lights are when they are on: their colour (RRGGBB), what they
/// do, and how bright they are (percent of the colour's own brightness).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Light {
    pub color: String,
    pub effect: Effect,
    pub brightness: u8,
}

impl Default for Light {
    fn default() -> Self {
        Light { color: "FFFFFF".into(), effect: Effect::Steady, brightness: 100 }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub port: u16,
    /// Phones control this PC (the phone remote is on). Off, this helper runs for nearby transfer alone: what phones
    /// ask is refused, but for what nearby transfer has of them (see `nearby` in main) and their unpairing.
    pub remote: bool,
    /// Nearby transfer: whether it is on, what this PC is called, the devices whose files are taken without asking.
    pub nearby: crate::nearby::Kept,
    /// This PC, as the phone names it.
    pub name: String,
    /// The one key every phone shared, before each had its own: read once,
    /// to go on as a phone of its own, and not written again.
    #[serde(skip_serializing)]
    key: String,
    /// The phones paired with, and the pairings offered and not yet taken.
    pub phones: Vec<Phone>,
    /// Where OpenRGB is, if it is not where its installer puts it.
    pub openrgb: Option<PathBuf>,
    pub strip_leds: u32,
    pub light: Light,
    /// How bright the light under a laptop's keys is while the lights are
    /// on (see `keys`), once it has been chosen from a phone: none, and
    /// that light is left as it is, on or off.
    pub keys: Option<u8>,
    /// The board's and the card's lights are left alone (the user turned
    /// their control off), whatever OpenRGB there is on this PC.
    pub lights_left_alone: bool,
    /// The OpenRGB this helper started, by its program's number: its own
    /// to stop and start afresh (one the user started is not).
    pub openrgb_started: Option<u32>,
    /// OpenRGB is run by Glance's sensors, with an administrator's rights (what the memory's lights need): this
    /// helper waits for that one and starts none of its own.
    pub lights_raised: bool,
    /// Whether the lights were last put on or out by this helper: what it
    /// says of them while no OpenRGB runs to be asked (as the PC has started
    /// and nothing has asked for it yet).
    pub lit: Option<bool>,
    /// The key this PC's own Glance asks by (its settings show the phone's
    /// page, to be used there as on a phone): taken from this PC alone.
    pub local: Option<String>,
    /// The colours the lights are chosen from (RRGGBB), in their order, as
    /// a phone or this PC's settings last kept them: the same for all of
    /// them. None kept yet: each shows the ones it starts with, and a phone
    /// that kept its own before this did sends them.
    pub colours: Option<Vec<String>>,
    /// How the screens are put out, and whether that was chosen: till it
    /// is, as this PC is best served (see `way`).
    pub screens: Screens,
    pub screens_chosen: bool,
    /// The monitors this helper is keeping dark with their signal kept,
    /// each as it is to be put back: one that ends before it has put them
    /// back does so when it next starts.
    pub darkened: Vec<crate::display::Darkened>,
    /// Night, while this helper has it on, as it is to be ended: one that ends before it has ended it has it ended
    /// when the next starts.
    pub night: Option<Night>,
    /// A key that puts night on and off and does nothing else (hex, 32 bytes), where one is kept: for a phone's
    /// shortcut, which cannot sign what it asks (see `night_key` in main). Made on this PC by hand, never offered.
    pub night_key: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            port: PORT,
            remote: true,
            nearby: crate::nearby::Kept::default(),
            name: std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".into()),
            key: String::new(),
            phones: Vec::new(),
            openrgb: None,
            strip_leds: STRIP_LEDS,
            light: Light::default(),
            keys: None,
            lights_left_alone: false,
            openrgb_started: None,
            lights_raised: false,
            lit: None,
            local: None,
            colours: None,
            screens: Screens::default(),
            screens_chosen: false,
            darkened: Vec::new(),
            night: None,
            night_key: None,
        }
    }
}

impl Config {
    /// The way the screens are put out: as chosen; not chosen, with their
    /// signal kept on a PC that stands by with its screens (there, Windows
    /// putting them out puts the PC to standby: its downloads stop, and it
    /// asks to be signed in to as they come on), else as kept.
    pub fn way(&self) -> Screens {
        if !self.screens_chosen && crate::actions::stands_by_with_its_screens() {
            Screens::Kept
        } else {
            self.screens
        }
    }
}

/// The folder what the helper keeps is in (its configuration, its log, its certificate).
#[cfg(not(test))]
pub fn folder() -> PathBuf {
    let roaming = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default();
    roaming.join("dev.weiyi.glance.remote")
}

/// In tests, a folder of the test run's own: they never read nor write the user's (on 2026-11-12 the nearby tests'
/// lines were found in the user's log).
#[cfg(test)]
pub fn folder() -> PathBuf {
    std::env::temp_dir().join(format!("glance-remote-tests-{}", std::process::id()))
}

fn path() -> PathBuf {
    folder().join("config.json")
}

/// How long a pairing offered may wait to be taken (seconds).
const OFFERED_FOR: u64 = 600;

/// A phone paired with, by the key that is its alone.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Phone {
    /// What it is told from the others by, to whoever lists them: the name
    /// of its key (see `sign::key_id`), which says nothing of the key.
    pub id: String,
    /// The key shared with it (hex, 32 bytes).
    pub key: String,
    /// What it calls itself.
    pub name: String,
    /// The number it gave itself, once it has used the key: the key is
    /// that phone's from then on (a second phone that scanned the same
    /// code is told to ask for its own).
    pub device: Option<String>,
    /// Offered and not yet taken: when the offer ends (seconds since 1970).
    pub offered_till: Option<u64>,
}

/// Held while the configuration is read, changed and kept again: by one
/// of this program at a time, whichever of them (the one listening, and
/// those Glance asks things of) and whichever of its threads.
struct Alone(windows::Win32::Foundation::HANDLE);

impl Alone {
    fn wait() -> std::io::Result<Alone> {
        use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject, INFINITE};
        unsafe {
            let held = CreateMutexW(None, false, windows::core::w!("Local\\dev.weiyi.glance.remote.config")).map_err(std::io::Error::other)?;
            // (Left held by one that ended before letting go, it is this one's all the same.)
            WaitForSingleObject(held, INFINITE);
            Ok(Alone(held))
        }
    }
}

impl Drop for Alone {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::System::Threading::ReleaseMutex(self.0);
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

impl Config {
    /// The configuration as it is kept: a new one where none is kept yet;
    /// not one at all where what is kept cannot be read (it is not to be
    /// written over as if there had been nothing).
    fn read() -> std::io::Result<Config> {
        let mut config: Config = match std::fs::read_to_string(path()) {
            Ok(text) => serde_json::from_str(&text).map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(error) => return Err(error),
        };
        if crate::sign::key(&config.key).is_some() && !config.phones.iter().any(|phone| phone.key == config.key) {
            config.phones.push(Phone { key: std::mem::take(&mut config.key), ..Phone::default() });
        }
        let now = crate::sign::now();
        // The pairings offered and never taken are gone from it.
        config.phones.retain(|phone| phone.offered_till.is_none_or(|till| till > now));
        config.phones.retain_mut(|phone| match crate::sign::key(&phone.key) {
            Some(key) => {
                phone.id = crate::sign::key_id(&key);
                true
            }
            None => false,
        });
        Ok(config)
    }

    /// The configuration kept (or a new one), with the one key of before
    /// as a phone of its own.
    pub fn load() -> Config {
        Config::read().unwrap_or_default()
    }

    /// Whether what is kept can be read (or nothing is kept yet).
    pub fn readable() -> bool {
        Config::read().is_ok()
    }

    /// Changes the configuration as it is kept now (another of this
    /// program may have changed it since it was read) and keeps it: one
    /// change at a time, each whole.
    pub fn change(with: impl FnOnce(&mut Config)) -> std::io::Result<Config> {
        let _alone = Alone::wait()?;
        let mut config = Config::read()?;
        with(&mut config);
        config.save()?;
        Ok(config)
    }

    /// Offers a pairing: a key of its own for the phone that takes it,
    /// which is returned.
    pub fn offer() -> std::io::Result<String> {
        let mut key = [0u8; 32];
        getrandom::getrandom(&mut key).map_err(|error| std::io::Error::other(error.to_string()))?;
        let key = crate::sign::hex(&key);
        Config::change(|config| config.phones.push(Phone { key: key.clone(), offered_till: Some(crate::sign::now() + OFFERED_FOR), ..Phone::default() }))?;
        Ok(key)
    }

    /// The key this PC itself asks by: made the first time it is wanted.
    pub fn local() -> std::io::Result<String> {
        if let Some(key) = Config::read()?.local {
            return Ok(key);
        }
        let mut key = [0u8; 32];
        getrandom::getrandom(&mut key).map_err(|error| std::io::Error::other(error.to_string()))?;
        let key = crate::sign::hex(&key);
        Ok(Config::change(|config| {
            config.local.get_or_insert(key);
        })?
        .local
        .unwrap_or_default())
    }

    /// When the configuration was last kept.
    pub fn kept_at() -> Option<std::time::SystemTime> {
        std::fs::metadata(path()).and_then(|file| file.modified()).ok()
    }

    /// Kept whole or not at all: written beside, then put in its place.
    fn save(&self) -> std::io::Result<()> {
        let path = path();
        std::fs::create_dir_all(path.parent().unwrap())?;
        let beside = path.with_extension("json.new");
        std::fs::write(&beside, serde_json::to_string_pretty(self).unwrap())?;
        std::fs::rename(beside, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Night is kept as it is to be ended (a power mode's GUID whole), and a configuration kept before night was
    /// kept reads as having none.
    #[test]
    fn night_is_kept_as_it_is_to_be_ended() {
        let mode = windows::core::GUID::from_u128(0x961cc777_2547_4f9d_8174_7d86181b8a7a);
        let kept = Config { night: Some(Night { power: Some(crate::power::Before::Mode(mode.to_u128())), muted: true }), ..Config::default() };
        let read: Config = serde_json::from_str(&serde_json::to_string_pretty(&kept).unwrap()).unwrap();
        assert_eq!(read.night, kept.night);
        assert!(matches!(read.night, Some(Night { power: Some(crate::power::Before::Mode(number)), .. }) if windows::core::GUID::from_u128(number) == mode));
        let before: Config = serde_json::from_str(r#"{"port":47631}"#).unwrap();
        assert_eq!(before.night, None);
    }
}
