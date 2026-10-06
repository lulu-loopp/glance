#[cfg(windows)]
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
#[cfg(windows)]
use windows::Win32::System::Com::CoTaskMemFree;
#[cfg(windows)]
use windows::Win32::UI::Shell::{FOLDERID_RoamingAppData, SHGetKnownFolderPath, KF_FLAG_DEFAULT};

#[cfg(windows)]
const FILE: &str = "settings.json";
/// Larger than this, a settings file is not one Glance wrote.
#[cfg(windows)]
const MOST: u64 = 1 << 20;

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Left,
    #[default]
    Right,
    Top,
}

/// Where along the edge the panel opens.
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Anchor {
    /// Centred on the pointer, as far as the screen allows.
    #[default]
    Pointer,
    Center,
}

/// A key combination: modifier keys held, and one other key, by its
/// Windows virtual-key code.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Shortcut {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub key: u16,
}

impl Default for Shortcut {
    /// Ctrl+Alt+G.
    fn default() -> Self {
        Shortcut { ctrl: true, alt: true, shift: false, win: false, key: u16::from(b'G') }
    }
}

impl Shortcut {
    /// Whether it can be a shortcut: a key that is not itself a modifier,
    /// with Ctrl, Alt or Win held (Shift alone would take a key from typing).
    pub fn usable(&self) -> bool {
        // Shift, Ctrl, Alt (either side or neither) and the Windows keys.
        const MODIFIERS: [u16; 9] = [0x10, 0x11, 0x12, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5];
        let key = self.key != 0 && !MODIFIERS.contains(&self.key) && self.key != 0x5B && self.key != 0x5C;
        key && (self.ctrl || self.alt || self.win)
    }
}

/// What opens the panel over a game in exclusive fullscreen.
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OverFullscreen {
    /// Nothing: the game keeps the screen.
    Never,
    /// The shortcut, which is asked for; a push into the edge may be an
    /// accident mid-game.
    #[default]
    Shortcut,
    /// The shortcut and a push into the edge.
    Both,
}

/// How hard the pointer has to push into the edge.
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sensitivity {
    Light,
    #[default]
    Medium,
    Firm,
}

impl Sensitivity {
    /// Raw mouse counts of outward travel against the edge.
    pub fn pressure(self) -> i32 {
        match self {
            Sensitivity::Light => 50,
            Sensitivity::Medium => 120,
            Sensitivity::Firm => 260,
        }
    }
}

/// Everything the backend acts on, plus the page's own preferences, which it
/// stores without looking inside.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub edge: Edge,
    pub skin: String,
    pub anchor: Anchor,
    /// How many columns the panel's lanes are dealt into; none chosen, as
    /// few as fit the screen's height.
    pub columns: Option<usize>,
    pub sensitivity: Sensitivity,
    pub close_delay_ms: u64,
    pub interval_ms: u64,
    /// Refresh the desktop behind the glass while the panel is open. The
    /// panel then hides itself from every screen capture, screenshots too.
    pub live_backdrop: bool,
    /// Ask GitHub once a day whether a newer Glance is out.
    pub check_updates: bool,
    /// The shortcut (Ctrl+Alt+G unless another is chosen) opens and closes
    /// the panel.
    pub hotkey: bool,
    pub shortcut: Shortcut,
    /// What opens the panel over a game holding the screen in exclusive
    /// fullscreen, which the panel showing sends to the background.
    pub over_fullscreen: OverFullscreen,
    /// Tell from the tray when the CPU or a graphics card stays at or above
    /// the temperature alert.
    pub heat_alert: bool,
    pub view: serde_json::Value,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            edge: Edge::default(),
            skin: "paper".into(),
            anchor: Anchor::default(),
            columns: None,
            sensitivity: Sensitivity::default(),
            close_delay_ms: 200,
            interval_ms: 1000,
            live_backdrop: false,
            check_updates: true,
            hotkey: true,
            shortcut: Shortcut::default(),
            over_fullscreen: OverFullscreen::default(),
            heat_alert: false,
            view: serde_json::Value::Null,
        }
    }
}

impl Settings {
    pub fn close_delay(&self) -> Duration {
        Duration::from_millis(self.close_delay_ms)
    }

    pub fn interval(&self) -> Duration {
        Duration::from_millis(self.interval_ms)
    }
}

/// Where Glance keeps its settings: the user's roaming application data,
/// under Glance's identifier (a debug build's, under its own, so it never
/// rewrites an installed Glance's).
#[cfg(windows)]
pub fn config_dir() -> PathBuf {
    let roaming = unsafe { SHGetKnownFolderPath(&FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, None) }.expect("application data folder");
    let path = PathBuf::from(unsafe { roaming.to_string() }.expect("folder path"));
    unsafe { CoTaskMemFree(Some(roaming.0 as *const _)) };
    path.join(if cfg!(debug_assertions) { "dev.weiyi.glance.debug" } else { "dev.weiyi.glance" })
}

#[cfg(windows)]
impl Settings {
    /// A missing or hand-edited file that no longer parses means defaults;
    /// numbers outside what the settings offer are brought within it.
    /// Read and written so that Glance, running elevated, never follows a
    /// link someone put in the user's folder (see `elevation::read_in_place`).
    pub fn load(dir: &Path) -> Self {
        let mut settings: Settings =
            crate::elevation::read_in_place(dir, FILE, MOST).and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();
        settings.interval_ms = settings.interval_ms.clamp(250, 10_000);
        settings.close_delay_ms = settings.close_delay_ms.min(10_000);
        if !settings.shortcut.usable() {
            settings.shortcut = Shortcut::default();
        }
        settings
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        crate::elevation::ensure_folder(dir)?;
        crate::elevation::write_in_place(dir, FILE, serde_json::to_string_pretty(self).unwrap().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_only_shortcuts_that_leave_typing_alone() {
        let key = |ctrl, alt, shift, win, key| Shortcut { ctrl, alt, shift, win, key }.usable();
        assert!(Shortcut::default().usable());
        assert!(key(true, false, true, false, u16::from(b'K')));
        assert!(key(false, false, false, true, 0x70));
        // A key alone, or with Shift alone, is typing.
        assert!(!key(false, false, false, false, u16::from(b'K')));
        assert!(!key(false, false, true, false, u16::from(b'K')));
        // Modifiers alone are no shortcut.
        assert!(!key(true, true, false, false, 0x12));
        assert!(!key(true, false, false, false, 0x5B));
    }

    #[test]
    fn settings_from_before_take_the_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"edge":"right","hotkey":true}"#).unwrap();
        assert_eq!(settings.shortcut, Shortcut::default());
        assert!(settings.over_fullscreen == OverFullscreen::Shortcut);
        assert_eq!(settings.columns, None);
    }
}
