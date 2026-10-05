//! What the panel shows and how: the settings window stores these as the
//! settings' `view`, in the shape its page uses.

use serde::Deserialize;

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub modules: Vec<ModuleEntry>,
    pub cpu: CpuPrefs,
    pub gpu: GpuPrefs,
    pub memory: MemoryPrefs,
    pub network: NetworkPrefs,
    pub disk: DiskPrefs,
    pub processes: ProcessPrefs,
    #[serde(rename = "chartSeconds")]
    pub chart_seconds: f64,
    #[serde(rename = "hotLoad")]
    pub hot_load: f32,
    #[serde(rename = "hotTemp")]
    pub hot_temp: f32,
    pub theme: ThemePref,
    pub language: LanguagePref,
}

#[derive(Clone, Deserialize)]
pub struct ModuleEntry {
    pub id: String,
    pub on: bool,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct CpuPrefs {
    pub threads: bool,
    pub clock: bool,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct GpuPrefs {
    pub memory: bool,
    pub sensors: bool,
    pub engines: bool,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct MemoryPrefs {
    pub details: bool,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct NetworkPrefs {
    pub bits: bool,
    pub details: bool,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct DiskPrefs {
    pub active: bool,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct ProcessPrefs {
    pub count: usize,
    pub sort: ProcessSort,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcessSort {
    #[default]
    Cpu,
    Memory,
    Io,
    Gpu,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
    Backdrop,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LanguagePref {
    #[default]
    System,
    Zh,
    En,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            modules: Vec::new(),
            cpu: CpuPrefs::default(),
            gpu: GpuPrefs::default(),
            memory: MemoryPrefs::default(),
            network: NetworkPrefs::default(),
            disk: DiskPrefs::default(),
            processes: ProcessPrefs::default(),
            chart_seconds: 60.0,
            hot_load: 85.0,
            hot_temp: 85.0,
            theme: ThemePref::System,
            language: LanguagePref::System,
        }
    }
}

impl Default for CpuPrefs {
    fn default() -> Self {
        CpuPrefs { threads: true, clock: true }
    }
}

impl Default for GpuPrefs {
    fn default() -> Self {
        GpuPrefs { memory: true, sensors: true, engines: false }
    }
}

impl Default for ProcessPrefs {
    fn default() -> Self {
        ProcessPrefs { count: 5, sort: ProcessSort::Cpu }
    }
}

/// Modules off until chosen.
const DEFAULT_OFF: [&str; 1] = ["system"];

impl Prefs {
    /// The stored preferences, completed with defaults, with the module list
    /// matched to this machine's modules (`known`, in default order):
    /// modules it no longer has are dropped, new ones appended.
    pub fn resolve(stored: &serde_json::Value, known: &[String]) -> Self {
        let mut prefs: Prefs = serde_json::from_value(stored.clone()).unwrap_or_default();
        prefs.modules.retain(|entry| known.contains(&entry.id));
        for id in known {
            if !prefs.modules.iter().any(|entry| &entry.id == id) {
                prefs.modules.push(ModuleEntry { id: id.clone(), on: !DEFAULT_OFF.contains(&id.as_str()) });
            }
        }
        prefs
    }
}
