//! What the panel shows and how: the settings window stores these as the
//! settings' `view`, in the shape its page uses.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Prefs {
    pub modules: Vec<ModuleEntry>,
    /// The switches each module's items had before they were the modules'
    /// own (up to 0.1.7): read once, to carry the choices over, and not
    /// written again.
    #[serde(skip_serializing)]
    cpu: Option<CpuPrefs>,
    #[serde(skip_serializing)]
    gpu: Option<GpuPrefs>,
    #[serde(skip_serializing)]
    memory: Option<MemoryPrefs>,
    #[serde(skip_serializing)]
    disk: Option<DiskPrefs>,
    pub network: NetworkPrefs,
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

#[derive(Clone, Deserialize, Serialize)]
pub struct ModuleEntry {
    pub id: String,
    pub on: bool,
    /// Each of its items switched from what it is by default, by name (see
    /// `items`). Kept while the module is off, and in force again with it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub items: BTreeMap<String, bool>,
}

/// One thing a module's lane can show, which its own switch turns on and
/// off: its name in the settings file, and whether it is on by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item {
    pub name: &'static str,
    pub on: bool,
}

const fn item(name: &'static str, on: bool) -> Item {
    Item { name, on }
}

// The items of each kind of module, in the order the settings list them.
const CPU: [Item; 6] = [item("chart", true), item("temp", true), item("clock", true), item("power", true), item("ccds", true), item("threads", true)];
const GPU: [Item; 8] = [
    item("chart", true),
    item("temp", true),
    item("vram", true),
    item("clock", true),
    item("power", true),
    item("fan", true),
    item("shared", true),
    item("engines", false),
];
const MEMORY: [Item; 4] = [item("chart", true), item("dimms", true), item("committed", false), item("cached", false)];
const NETWORK: [Item; 5] = [item("chart", true), item("adapter", false), item("address", false), item("link", false), item("totals", false)];
const DISK: [Item; 3] = [item("chart", true), item("drives", true), item("active", false)];
const BOARD: [Item; 2] = [item("temps", true), item("fans", true)];
const BATTERY: [Item; 1] = [item("chart", true)];
const SYSTEM: [Item; 4] = [item("uptime", true), item("processes", true), item("threads", true), item("handles", true)];
const GAME: [Item; 9] = [
    item("chart", true),
    item("frametimes", true),
    item("low", true),
    item("longest", true),
    item("usage", true),
    item("memory", true),
    item("limit", true),
    item("time", true),
    item("mic", true),
];

/// The items of module `module` ("cpu", "gpu:1", …), in the order the
/// settings list them.
pub fn items(module: &str) -> &'static [Item] {
    match module.split(':').next().unwrap_or(module) {
        "cpu" => &CPU,
        "gpu" => &GPU,
        "memory" => &MEMORY,
        "network" => &NETWORK,
        "disk" => &DISK,
        "board" => &BOARD,
        "battery" => &BATTERY,
        "system" => &SYSTEM,
        "game" => &GAME,
        _ => &[],
    }
}

// The switches before 0.1.8, as they were stored.
#[derive(Clone, Deserialize)]
#[serde(default)]
struct CpuPrefs {
    threads: bool,
    clock: bool,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
struct GpuPrefs {
    memory: bool,
    sensors: bool,
    engines: bool,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
struct MemoryPrefs {
    details: bool,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
struct DiskPrefs {
    active: bool,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct NetworkPrefs {
    pub bits: bool,
    /// Before 0.1.8: the adapter, address, link and totals together.
    #[serde(skip_serializing)]
    details: Option<bool>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ProcessPrefs {
    pub count: usize,
    pub sort: ProcessSort,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcessSort {
    #[default]
    Cpu,
    Memory,
    Io,
    Gpu,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
    Backdrop,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
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
            cpu: None,
            gpu: None,
            memory: None,
            disk: None,
            network: NetworkPrefs::default(),
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

/// `entries` matched to this machine's modules (`known`, in default order):
/// modules it no longer has are dropped, new ones put where the default
/// order puts them (before the first module listed after them there), on
/// as `on` says.
fn fit(entries: &mut Vec<ModuleEntry>, known: &[String], on: impl Fn(&str) -> bool) {
    entries.retain(|entry| known.contains(&entry.id));
    for (index, id) in known.iter().enumerate() {
        if !entries.iter().any(|entry| &entry.id == id) {
            let at = known[index + 1..].iter().find_map(|next| entries.iter().position(|entry| &entry.id == next)).unwrap_or(entries.len());
            entries.insert(at, ModuleEntry { id: id.clone(), on: on(id), items: BTreeMap::new() });
        }
    }
}

impl Prefs {
    /// The stored preferences, completed with defaults, with the module list
    /// matched to this machine's modules (`known`, in default order):
    /// modules it no longer has are dropped, new ones appended.
    pub fn resolve(stored: &serde_json::Value, known: &[String]) -> Self {
        let mut prefs: Prefs = serde_json::from_value(stored.clone()).unwrap_or_default();
        // A hand-edited file may hold numbers the settings never offer.
        prefs.chart_seconds = prefs.chart_seconds.clamp(10.0, 300.0);
        prefs.hot_load = prefs.hot_load.clamp(1.0, 100.0);
        prefs.hot_temp = prefs.hot_temp.clamp(1.0, 150.0);
        prefs.processes.count = prefs.processes.count.clamp(1, 30);
        fit(&mut prefs.modules, known, |id| !DEFAULT_OFF.contains(&id));
        prefs.carry_over();
        prefs
    }

    /// Whether module `id` shows its item `name` (whether or not the module
    /// itself is on).
    pub fn shows(&self, id: &str, name: &str) -> bool {
        let chosen = self.modules.iter().find(|entry| entry.id == id).and_then(|entry| entry.items.get(name).copied());
        chosen.unwrap_or_else(|| items(id).iter().find(|item| item.name == name).is_some_and(|item| item.on))
    }

    /// Turns module `id`'s item `name` on or off.
    pub fn set_item(&mut self, id: &str, name: &str, on: bool) {
        let default = items(id).iter().find(|item| item.name == name).is_some_and(|item| item.on);
        if let Some(entry) = self.modules.iter_mut().find(|entry| entry.id == id) {
            // Only what differs from the default is kept.
            if on == default {
                entry.items.remove(name);
            } else {
                entry.items.insert(name.to_string(), on);
            }
        }
    }

    /// The switches stored before 0.1.8 made the modules' items: the
    /// choices they held, for every module of their kind.
    fn carry_over(&mut self) {
        let (cpu, gpu, memory, disk, network) = (self.cpu.take(), self.gpu.take(), self.memory.take(), self.disk.take(), self.network.details.take());
        let mut carried: Vec<(&str, &str, bool)> = Vec::new();
        if let Some(cpu) = cpu {
            carried.extend([("cpu", "threads", cpu.threads), ("cpu", "clock", cpu.clock)]);
        }
        if let Some(gpu) = gpu {
            carried.push(("gpu", "vram", gpu.memory));
            carried.extend(["temp", "clock", "power", "fan", "shared"].map(|name| ("gpu", name, gpu.sensors)));
            carried.push(("gpu", "engines", gpu.engines));
        }
        if let Some(memory) = memory {
            carried.extend([("memory", "committed", memory.details), ("memory", "cached", memory.details)]);
        }
        if let Some(disk) = disk {
            carried.push(("disk", "active", disk.active));
        }
        if let Some(details) = network {
            carried.extend(["adapter", "address", "link", "totals"].map(|name| ("network", name, details)));
        }
        let ids: Vec<String> = self.modules.iter().map(|entry| entry.id.clone()).collect();
        for (kind, name, on) in carried {
            for id in ids.iter().filter(|id| id.split(':').next() == Some(kind)) {
                self.set_item(id, name, on);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> Vec<String> {
        ["cpu", "gpu:0", "gpu:1", "memory", "network", "disk", "processes"].map(String::from).to_vec()
    }

    #[test]
    fn puts_a_new_module_where_the_default_order_has_it() {
        // Settings from before the game module, the user's own order.
        let stored = serde_json::json!({"modules": [{"id": "memory", "on": true}, {"id": "cpu", "on": true}, {"id": "disk", "on": false}]});
        let known: Vec<String> = ["game", "cpu", "memory", "disk", "storage"].map(String::from).to_vec();
        let prefs = Prefs::resolve(&stored, &known);
        let order: Vec<&str> = prefs.modules.iter().map(|entry| entry.id.as_str()).collect();
        // The game before the CPU, which follows it by default; the user's
        // order kept; storage, last by default, after the rest.
        assert_eq!(order, ["memory", "game", "cpu", "disk", "storage"]);
        assert!(!prefs.modules[3].on);
    }

    #[test]
    fn items_default_and_switch() {
        let mut prefs = Prefs::resolve(&serde_json::Value::Null, &known());
        assert!(prefs.shows("cpu", "clock"));
        assert!(!prefs.shows("gpu:1", "engines"));
        // Each GPU its own.
        prefs.set_item("gpu:0", "power", false);
        assert!(!prefs.shows("gpu:0", "power") && prefs.shows("gpu:1", "power"));
        // Back to the default: nothing kept.
        prefs.set_item("gpu:0", "power", true);
        assert!(prefs.modules.iter().all(|entry| entry.items.is_empty()));
    }

    #[test]
    fn carries_the_old_switches_over() {
        let stored = serde_json::json!({
            "modules": [{ "id": "cpu", "on": true }, { "id": "gpu:0", "on": true }, { "id": "network", "on": false }],
            "cpu": { "threads": false, "clock": true },
            "gpu": { "memory": true, "sensors": false, "engines": true },
            "network": { "bits": true, "details": true },
        });
        let prefs = Prefs::resolve(&stored, &known());
        assert!(!prefs.shows("cpu", "threads") && prefs.shows("cpu", "clock"));
        assert!(!prefs.shows("gpu:0", "temp") && !prefs.shows("gpu:0", "fan") && prefs.shows("gpu:0", "engines"));
        // A GPU not in the old list takes the old switches too.
        assert!(!prefs.shows("gpu:1", "power"));
        assert!(prefs.network.bits && prefs.shows("network", "address"));
        // Saved, the old fields are gone and the choices stay.
        let saved = serde_json::to_value(&prefs).unwrap();
        assert!(saved.get("cpu").is_none() && saved["network"].get("details").is_none());
        let again = Prefs::resolve(&saved, &known());
        assert!(!again.shows("cpu", "threads") && again.shows("network", "address") && again.shows("gpu:0", "engines"));
    }
}
