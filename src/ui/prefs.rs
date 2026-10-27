//! What the panel shows and how: the settings window stores these as the
//! settings' `view`, in the shape its page uses.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Prefs {
    /// The readings' modules, in their order, each on or off with its items.
    pub modules: Vec<ModuleEntry>,
    /// The workbench's, as the readings' are (the time and what plays in it are its own, apart from the readings').
    pub workbench: Vec<ModuleEntry>,
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
    pub music: MusicPrefs,
    #[serde(rename = "chartSeconds")]
    pub chart_seconds: f64,
    #[serde(rename = "hotTemp")]
    pub hot_temp: f32,
    pub theme: ThemePref,
    pub language: LanguagePref,
}

/// The load (percent) past which a reading is marked as high: its figure in the signal colour, its chart's line drawn.
pub const HOT_LOAD: f32 = 85.0;

#[derive(Clone, PartialEq, Debug, Hash, Deserialize, Serialize)]
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
/// (The CPU's fans: of the board's, as its group, "fans:cpu_fan"; see `CPU_FAN`.)
const CPU: [Item; 7] = [item("chart", true), item("temp", true), item("clock", true), item("power", true), item("fans", true), item("ccds", true), item("threads", true)];
/// The fan the CPU's lane shows by default, of the board's (as the board's chip and a laptop's firmware name the
/// CPU's).
pub const CPU_FAN: &str = "cpu_fan";
const GPU: [Item; 9] = [
    item("chart", true),
    item("temp", true),
    item("hotspot", true),
    item("vram", true),
    item("clock", true),
    item("power", true),
    item("fan", true),
    item("shared", true),
    item("engines", false),
];
const MEMORY: [Item; 5] = [item("chart", true), item("speed", true), item("dimms", true), item("committed", false), item("cached", false)];
/// (Its latency off till asked for: it sends echoes out, see `latency`.)
const NETWORK: [Item; 7] = [item("chart", true), item("adapter", false), item("wifi", false), item("address", false), item("link", false), item("totals", false), item("latency", false)];
const DISK: [Item; 3] = [item("chart", true), item("drives", true), item("active", false)];
const BOARD: [Item; 2] = [item("temps", true), item("fans", true)];
/// Each drive the machine has, by its letter ("volumes:C:"), as the group.
const STORAGE: [Item; 1] = [item("volumes", true)];
const BATTERY: [Item; 3] = [item("chart", true), item("power", true), item("health", true)];
const WSL: [Item; 4] = [item("chart", true), item("memory", true), item("gpu", true), item("distros", true)];
const TIME: [Item; 3] = [item("sound", true), item("media", true), item("lyrics", false)];
/// What plays, as the music widgets show it (see `music::Shows`): the translation under the line sung now, else the
/// next line; the cover, what plays, the lines round the one sung now, the spectrum, the controls kept showing. (What
/// they are drawn on is a choice of its own: see `MusicGround`.)
const LYRICS: [Item; 5] = [item("translation", true), item("cover", true), item("title", true), item("spectrum", true), item("controls", false)];
/// The station: what was copied lately under what is on it.
const STATION: [Item; 0] = [];
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

/// Whether the lyrics are offered in the settings (their module, as Music,
/// and the line the time module shows). A player that says no position
/// (NetEase's own, without its plugin) has the rest of Music, and is told why
/// it has no lyrics.
pub const LYRICS_OFFERED: bool = true;

/// Whether module `module` is offered in the settings (see `LYRICS_OFFERED`).
pub fn offered(module: &str) -> bool {
    LYRICS_OFFERED || module != "lyrics"
}

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
        "storage" => &STORAGE,
        "battery" => &BATTERY,
        "system" => &SYSTEM,
        "time" => &TIME,
        "station" => &STATION,
        "lyrics" => &LYRICS,
        "wsl" => &WSL,
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
    /// Where its latency is measured to (see `latency`); none: as the language has it (see `ping_to`).
    pub ping: Option<PingTarget>,
    /// Before 0.1.8: the adapter, address, link and totals together.
    #[serde(skip_serializing)]
    details: Option<bool>,
}

/// Where the network's latency is measured to: the router the way out goes through, or a public resolver (Alibaba's,
/// Tencent's, Cloudflare's).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PingTarget {
    Gateway,
    Alibaba,
    Tencent,
    Cloudflare,
}

impl PingTarget {
    /// As the settings list them.
    pub const ALL: [PingTarget; 4] = [PingTarget::Gateway, PingTarget::Alibaba, PingTarget::Tencent, PingTarget::Cloudflare];
}

impl NetworkPrefs {
    /// Where its latency is measured to: as chosen, else a resolver near those who read `lang` (Alibaba's for Chinese,
    /// Cloudflare's otherwise).
    pub fn ping_to(&self, lang: crate::ui::text::Lang) -> PingTarget {
        self.ping.unwrap_or(if lang == crate::ui::text::Lang::Zh { PingTarget::Alibaba } else { PingTarget::Cloudflare })
    }
}

/// What the music widgets, and the panel showing what plays, are drawn on: the cover blurred (their own look), the
/// skin's ground (as every other widget is), or none (their words edged, on whatever is behind).
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MusicGround {
    #[default]
    Cover,
    Skin,
    Clear,
}

impl MusicGround {
    /// As the settings list them.
    pub const ALL: [MusicGround; 3] = [MusicGround::Cover, MusicGround::Skin, MusicGround::Clear];
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct MusicPrefs {
    pub ground: MusicGround,
}

/// What the music widgets are drawn on, as the stored settings say: as chosen; never chosen, none where the lyrics'
/// switch for no ground (up to 0.4.2) was on, else the cover.
pub fn music_ground(stored: &serde_json::Value) -> MusicGround {
    match serde_json::from_value(stored["music"]["ground"].clone()) {
        Ok(ground) => ground,
        Err(_) if item_on(stored, "lyrics", "clear") => MusicGround::Clear,
        Err(_) => MusicGround::Cover,
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ProcessPrefs {
    pub count: usize,
    pub sort: ProcessSort,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug, Deserialize, Serialize)]
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
            workbench: Vec::new(),
            cpu: None,
            gpu: None,
            memory: None,
            disk: None,
            network: NetworkPrefs::default(),
            processes: ProcessPrefs::default(),
            music: MusicPrefs::default(),
            chart_seconds: 60.0,
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

/// The workbench's modules, in default order (each on until chosen otherwise).
pub const WORKBENCH: [&str; 5] = ["search", "time", "station", "notes", "apps"];

/// Whether module `id` among `entries` shows its item `name` (whether or not the module itself is on).
pub fn shows_in(entries: &[ModuleEntry], id: &str, name: &str) -> bool {
    let chosen = entries.iter().find(|entry| entry.id == id).and_then(|entry| entry.items.get(name).copied());
    chosen.unwrap_or_else(|| shows_by_default(entries, id, name))
}

/// Whether module `id` among `entries` shows item `name` unless it is switched: as the module's items are by default;
/// one of a group the machine names ("temps:CPU", a board's sensor), as the group is; of the board's fans, the CPU's
/// shows its own (see `CPU_FAN`).
fn shows_by_default(entries: &[ModuleEntry], id: &str, name: &str) -> bool {
    match name.split_once(':') {
        Some(("fans", fan)) if id == "cpu" => fan == CPU_FAN && shows_in(entries, id, "fans"),
        Some((group, _)) => shows_in(entries, id, group),
        None => items(id).iter().find(|item| item.name == name).is_some_and(|item| item.on),
    }
}

/// Turns module `id`'s item `name` among `entries` on or off.
pub fn set_item_in(entries: &mut [ModuleEntry], id: &str, name: &str, on: bool) {
    let default = shows_by_default(entries, id, name);
    if let Some(entry) = entries.iter_mut().find(|entry| entry.id == id) {
        // Only what differs from the default is kept.
        if on == default {
            entry.items.remove(name);
        } else {
            entry.items.insert(name.to_string(), on);
        }
    }
}

/// Modules off until chosen.
const DEFAULT_OFF: [&str; 5] = ["system", "time", "lyrics", "wsl", "docker"];

/// Whether module `id` is on in the stored preferences `stored` (as kept,
/// before they are resolved: off unless listed on).
pub fn module_on(stored: &serde_json::Value, id: &str) -> bool {
    module_on_in(stored, "modules", id)
}

/// As `module_on`, in the stored list `list` ("modules", the readings'; "workbench", the workbench's, each of its
/// modules on unless listed off).
/// `prefs` as they are kept, over the preferences `held` they were resolved from: the modules of what is not on this
/// machine now (a GPU taken out, an external one unplugged) kept as they were, each after the one it followed. (Left
/// out, as `resolve` leaves them out of what is shown, a change of any preference lost their switches and their
/// place, and they came back as new.) What earlier versions kept, carried over into `prefs`, is not written again.
pub fn kept(prefs: &Prefs, held: &serde_json::Value) -> serde_json::Value {
    let mut kept = serde_json::to_value(prefs).unwrap();
    for list in ["modules", "workbench"] {
        let (Some(held), Some(now)) = (held[list].as_array(), kept[list].as_array_mut()) else { continue };
        let id = |entry: &serde_json::Value| entry["id"].as_str().map(str::to_string);
        // Where the next one away goes: after the one held before it, as it is now.
        let mut at = 0;
        for entry in held {
            let Some(this) = id(entry) else { continue };
            match now.iter().position(|shown| id(shown).as_deref() == Some(this.as_str())) {
                Some(shown) => at = shown + 1,
                None => {
                    now.insert(at, entry.clone());
                    at += 1;
                }
            }
        }
    }
    kept
}

pub fn module_on_in(stored: &serde_json::Value, list: &str, id: &str) -> bool {
    let entry = stored[list].as_array().and_then(|modules| modules.iter().find(|entry| entry["id"] == id));
    match entry {
        Some(entry) => entry["on"] == true,
        None => list == "workbench" && WORKBENCH.contains(&id),
    }
}

/// Whether module `id` shows item `name` in the stored preferences `stored`: as switched there, else as it is
/// by default.
pub fn item_on(stored: &serde_json::Value, id: &str, name: &str) -> bool {
    item_on_in(stored, "modules", id, name)
}

/// As `item_on`, in the stored list `list` (see `module_on_in`).
pub fn item_on_in(stored: &serde_json::Value, list: &str, id: &str, name: &str) -> bool {
    let switched = stored[list].as_array().and_then(|modules| modules.iter().find(|entry| entry["id"] == id)).and_then(|entry| entry["items"][name].as_bool());
    switched.unwrap_or_else(|| items(id).iter().find(|item| item.name == name).is_some_and(|item| item.on))
}

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
    /// What of it a small layout draws from (which readings, their items,
    /// where they turn hot), hashed into `key`.
    pub fn hash_drawn(&self, key: &mut impl std::hash::Hasher) {
        use std::hash::Hash;
        self.modules.hash(key);
        (self.hot_temp.to_bits(), self.chart_seconds.to_bits()).hash(key);
    }

    /// The stored preferences, completed with defaults, with the module list
    /// matched to this machine's modules (`known`, in default order):
    /// modules it no longer has are dropped, new ones appended.
    pub fn resolve(stored: &serde_json::Value, known: &[String]) -> Self {
        let mut prefs: Prefs = serde_json::from_value(stored.clone()).unwrap_or_default();
        // A hand-edited file may hold numbers the settings never offer.
        prefs.chart_seconds = prefs.chart_seconds.clamp(10.0, 300.0);
        prefs.hot_temp = prefs.hot_temp.clamp(1.0, 150.0);
        prefs.processes.count = prefs.processes.count.clamp(1, 30);
        prefs.music.ground = music_ground(stored);
        fit(&mut prefs.modules, known, |id| !DEFAULT_OFF.contains(&id));
        fit(&mut prefs.workbench, &WORKBENCH.map(String::from), |_| true);
        prefs.carry_over();
        prefs
    }

    /// As a panel showing `part` draws its lanes: the readings' modules (what plays not among them: it is a
    /// component of its own), or the workbench's.
    pub fn holding(mut self, part: crate::settings::Part) -> Prefs {
        use crate::settings::Part;
        match part {
            Part::Workbench => self.modules = std::mem::take(&mut self.workbench),
            Part::Monitor | Part::Music => self.modules.retain(|module| module.id != "lyrics"),
        }
        self
    }

    /// Whether module `id` shows its item `name` (whether or not the module
    /// itself is on).
    pub fn shows(&self, id: &str, name: &str) -> bool {
        shows_in(&self.modules, id, name)
    }

    /// Turns module `id`'s item `name` on or off.
    pub fn set_item(&mut self, id: &str, name: &str, on: bool) {
        set_item_in(&mut self.modules, id, name, on);
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
    /// The latency is measured, unless chosen otherwise, to a resolver near those who read the language shown, and
    /// what is chosen is kept as it is named.
    #[test]
    fn the_latency_is_measured_to_where_it_was_chosen() {
        use super::{NetworkPrefs, PingTarget};
        use crate::ui::text::Lang;
        let unchosen = NetworkPrefs::default();
        assert_eq!((unchosen.ping_to(Lang::Zh), unchosen.ping_to(Lang::En)), (PingTarget::Alibaba, PingTarget::Cloudflare));
        let chosen = NetworkPrefs { ping: Some(PingTarget::Gateway), ..Default::default() };
        assert_eq!(chosen.ping_to(Lang::Zh), PingTarget::Gateway);
        let kept = serde_json::to_value(&chosen).unwrap();
        assert_eq!(kept["ping"], "gateway");
        assert_eq!(serde_json::from_value::<NetworkPrefs>(kept).unwrap().ping, Some(PingTarget::Gateway));
    }

    /// A module of what is not here now is kept as it was, where it was, through a change of another preference.
    #[test]
    fn a_gpu_away_keeps_its_module() {
        let known: Vec<String> = ["cpu", "gpu:0", "memory"].map(String::from).to_vec();
        let held = serde_json::json!({ "modules": [
            { "id": "cpu", "on": true },
            { "id": "gpu:1", "on": false, "items": { "temp": false } },
            { "id": "gpu:0", "on": true },
            { "id": "memory", "on": true },
        ] });
        let mut prefs = super::Prefs::resolve(&held, &known);
        assert_eq!(prefs.modules.iter().map(|entry| entry.id.as_str()).collect::<Vec<_>>(), ["cpu", "gpu:0", "memory"]);
        prefs.processes.sort = super::ProcessSort::Memory;
        let kept = super::kept(&prefs, &held);
        let modules = kept["modules"].as_array().unwrap();
        assert_eq!(modules.iter().map(|entry| entry["id"].as_str().unwrap()).collect::<Vec<_>>(), ["cpu", "gpu:1", "gpu:0", "memory"]);
        assert_eq!(modules[1], held["modules"][1]);
        // Back: as it was.
        let known: Vec<String> = ["cpu", "gpu:0", "gpu:1", "memory"].map(String::from).to_vec();
        let back = super::Prefs::resolve(&kept, &known);
        assert!(!back.modules.iter().find(|entry| entry.id == "gpu:1").unwrap().on);
    }

    #[test]
    fn a_panel_shows_what_its_edge_holds() {
        use crate::settings::Part::{Monitor, Workbench};
        let known: Vec<String> = ["cpu", "memory", "lyrics", "time"].map(String::from).to_vec();
        let stored = serde_json::json!({ "modules": [{ "id": "cpu", "on": true }, { "id": "lyrics", "on": true }, { "id": "time", "on": false }] });
        let mut prefs = super::Prefs::resolve(&stored, &known);
        let on = |prefs: &super::Prefs| prefs.modules.iter().filter(|module| module.on).map(|module| module.id.clone()).collect::<Vec<_>>();
        // The readings without what plays; the workbench its search, time, station, notes and quick launch, on, on their
        // own.
        assert_eq!(on(&prefs.clone().holding(Monitor)), ["cpu", "memory"]);
        assert_eq!(on(&prefs.clone().holding(Workbench)), ["search", "time", "station", "notes", "apps"]);
        // Its items its own: the workbench's time switched, the readings' as it was.
        super::set_item_in(&mut prefs.workbench, "time", "lyrics", true);
        assert!(prefs.clone().holding(Workbench).shows("time", "lyrics"));
        assert!(!prefs.shows("time", "lyrics"));
    }

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

    /// What the music widgets are drawn on: the cover, unless chosen otherwise; none, where the lyrics' switch for no
    /// ground was on before it was a choice of its own; and the choice kept as it is saved.
    #[test]
    fn the_music_ground_carries_the_clear_switch_over() {
        use super::{music_ground, MusicGround};
        assert_eq!(music_ground(&serde_json::json!({})), MusicGround::Cover);
        let clear = serde_json::json!({ "modules": [{ "id": "lyrics", "on": true, "items": { "clear": true } }] });
        assert_eq!(music_ground(&clear), MusicGround::Clear);
        let prefs = Prefs::resolve(&clear, &known());
        assert_eq!(prefs.music.ground, MusicGround::Clear);
        // Chosen since, the choice stands, the old switch still in the file or not.
        let mut chosen = serde_json::to_value(&prefs).unwrap();
        chosen["music"]["ground"] = "skin".into();
        assert_eq!(music_ground(&chosen), MusicGround::Skin);
        assert_eq!(Prefs::resolve(&chosen, &known()).music.ground, MusicGround::Skin);
        chosen["music"]["ground"] = "cover".into();
        assert_eq!(Prefs::resolve(&chosen, &known()).music.ground, MusicGround::Cover);
    }
}
