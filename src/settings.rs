use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_RoamingAppData, SHGetKnownFolderPath, KF_FLAG_DEFAULT};

const FILE: &str = "settings.json";

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
    pub sensitivity: Sensitivity,
    pub close_delay_ms: u64,
    pub interval_ms: u64,
    /// Refresh the desktop behind the glass while the panel is open. The
    /// panel then hides itself from every screen capture, screenshots too.
    pub live_backdrop: bool,
    /// Ask GitHub once a day whether a newer Glance is out.
    pub check_updates: bool,
    /// Ctrl+Alt+G opens and closes the panel.
    pub hotkey: bool,
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
            sensitivity: Sensitivity::default(),
            close_delay_ms: 200,
            interval_ms: 1000,
            live_backdrop: false,
            check_updates: true,
            hotkey: true,
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
/// under Glance's identifier.
pub fn config_dir() -> PathBuf {
    let roaming = unsafe { SHGetKnownFolderPath(&FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, None) }.expect("application data folder");
    let path = PathBuf::from(unsafe { roaming.to_string() }.expect("folder path"));
    unsafe { CoTaskMemFree(Some(roaming.0 as *const _)) };
    path.join("dev.weiyi.glance")
}

impl Settings {
    /// A missing or hand-edited file that no longer parses means defaults;
    /// numbers outside what the settings offer are brought within it.
    /// Read and written so that Glance, running elevated, never follows a
    /// link someone put in the user's folder (see `elevation::read_in_place`).
    pub fn load(dir: &Path) -> Self {
        let mut settings: Settings =
            crate::elevation::read_in_place(dir, FILE).and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();
        settings.interval_ms = settings.interval_ms.clamp(250, 10_000);
        settings.close_delay_ms = settings.close_delay_ms.min(10_000);
        settings
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        crate::elevation::ensure_folder(dir)?;
        crate::elevation::write_in_place(dir, FILE, serde_json::to_string_pretty(self).unwrap().as_bytes())
    }
}
