use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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

fn path(app: &AppHandle) -> PathBuf {
    app.path().app_config_dir().expect("config directory").join("settings.json")
}

impl Settings {
    /// A missing or hand-edited file that no longer parses means defaults.
    pub fn load(app: &AppHandle) -> Self {
        fs::read_to_string(path(app))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, app: &AppHandle) -> std::io::Result<()> {
        let path = path(app);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, serde_json::to_string_pretty(self).unwrap())
    }
}
