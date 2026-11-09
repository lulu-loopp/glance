//! What Glance reads from a machine, as the rest of it sees the readings:
//! the hardware that does not change while Glance runs, and each sample.
//! The same on every platform; how each platform fills them in is its own.

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct StaticInfo {
    pub cpu_name: String,
    /// The memory modules, as "2 × 32 GB DDR5-6000", when the firmware says.
    pub memory_modules: Option<String>,
    /// What the memory runs at (MT/s), and what it is rated for where more.
    pub memory_speed: Option<(u16, Option<u16>)>,
    /// The model of each physical drive.
    pub drives: Vec<String>,
    /// Each of `drives`' number among the physical drives.
    pub drive_ids: Vec<u32>,
    /// The model of the adapter internet traffic leaves by, when the app started.
    pub network_adapter: Option<String>,
    /// The motherboard's model, as its firmware names it.
    pub board: String,
    pub threads: usize,
    pub mem_total: u64,
    pub gpus: Vec<GpuInfo>,
    /// What of the hardware Glance found to read, and how, a line each:
    /// for the diagnostics the settings copy.
    pub found: Vec<String>,
}

impl StaticInfo {
    /// The modules of the GPUs, in order ("gpu:0", …).
    pub fn gpu_modules(&self) -> impl Iterator<Item = String> + '_ {
        self.gpus.iter().map(|gpu| format!("gpu:{}", gpu.slot))
    }

    /// Which of `gpus` (and of each sample's) module `id` shows.
    pub fn gpu_of(&self, id: &str) -> Option<usize> {
        let slot: usize = id.strip_prefix("gpu:")?.parse().ok()?;
        self.gpus.iter().position(|gpu| gpu.slot == slot)
    }

    /// This, with its drives as `listed` lists them (and all else as it is: its GPUs, as they were numbered).
    pub fn with_drives_of(&self, listed: &StaticInfo) -> StaticInfo {
        StaticInfo { drives: listed.drives.clone(), drive_ids: listed.drive_ids.clone(), ..self.clone() }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct GpuInfo {
    /// Its number in the module that shows it ("gpu:N"). Numbers stay with
    /// the GPUs they were given to (see `settings::slot_gpus`), so that each
    /// GPU's lane keeps the settings made for it.
    pub slot: usize,
    /// What it is, as no other GPU here is: its maker's and model's numbers,
    /// and where it sits on the PCI bus.
    pub identity: String,
    pub name: String,
    pub mem_total: u64,
    pub shared_total: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Sample {
    /// Milliseconds since the Unix epoch.
    pub t: u64,
    /// The whole processor's use (%); `None` (as for each reading that may
    /// be) where it was not read this time.
    pub cpu: Option<f32>,
    /// Each logical processor's use (%), in order; `None` where it was not
    /// read this time.
    pub threads: Vec<Option<f32>>,
    /// The cores' clock, where the system says it.
    pub ghz: Option<f32>,
    /// On a processor with cores of more than one kind (big and little),
    /// each kind's clock, its cores' average (GHz), the most capable kind
    /// first; empty on one whose cores are all alike.
    pub kinds_ghz: Vec<Option<f32>>,
    /// Each logical processor's clock (GHz), in the order of `threads`.
    pub threads_ghz: Vec<Option<f32>>,
    pub memory: MemorySample,
    pub gpus: Vec<GpuSample>,
    pub net_down: Option<f64>,
    pub net_up: Option<f64>,
    /// Bytes moved since the interfaces came up.
    pub net_total_down: u64,
    pub net_total_up: u64,
    pub network: Option<NetworkInfo>,
    /// The network's latency (ms), while it is measured, as last answered (see `latency`); none where no echo came
    /// back in time.
    #[serde(default)]
    pub ping_ms: Option<f32>,
    pub disk_read: Option<f64>,
    pub disk_write: Option<f64>,
    /// Percent of the time the disks were busy.
    pub disk_active: Option<f32>,
    pub volumes: Vec<VolumeSample>,
    /// The busiest programs by CPU, and by memory.
    /// The busiest programs by each measure, together (see `ProcessTable`).
    pub processes: Vec<ProcessSample>,
    pub system: SystemSample,
    pub battery: Option<BatterySample>,
    /// The CPU's own temperatures and power, read through the driver.
    pub cpu_sensors: Option<CpuSensors>,
    /// The motherboard's temperatures and fans, read through the driver.
    pub board: Option<BoardSensors>,
    /// Drives that report their temperature to Windows directly.
    pub drive_temps: Vec<DriveTemperature>,
    /// Each memory module's temperature, in slot order, read through the driver.
    pub dimm_temps: Vec<f32>,
    /// Whether the default microphone is muted; none without one.
    pub mic_muted: Option<bool>,
    /// The game presenting frames, the one on the pointer's screen first;
    /// none while no game presents frames.
    pub game: Option<GameSample>,
    /// WSL 2, while its module is on (none: not watched).
    pub wsl: Option<WslSample>,
    /// Docker, while its module is on and once its engine was first asked.
    pub docker: Option<DockerSample>,
}

/// Docker's engine, as the CLI's current context reaches it.
#[derive(Clone, Serialize, Deserialize)]
pub enum DockerSample {
    /// Not answering (Docker not running).
    Stopped { context: String },
    /// At an address this does not reach (ssh, TLS).
    Unsupported { context: String, host: String },
    Running { context: String, containers: Vec<ContainerSample> },
}

/// A container running, and what it uses: its share of the processors
/// (percent of all of them), memory in use and its limit (bytes), and the
/// network and disk traffic it makes (bytes a second, both ways together);
/// rates none the first time it is seen.
#[derive(Clone, Serialize, Deserialize)]
pub struct ContainerSample {
    pub name: String,
    pub status: String,
    pub cpu: Option<f32>,
    pub mem: Option<u64>,
    pub limit: Option<u64>,
    pub net: Option<f64>,
    pub io: Option<f64>,
}

/// WSL 2 as Windows sees it.
#[derive(Clone, Serialize, Deserialize)]
pub enum WslSample {
    /// No distribution registered.
    Missing,
    /// Its virtual machine not running.
    Stopped,
    Running {
        /// The distributions running, by name; none until first listed.
        distros: Option<Vec<String>>,
        /// How busy its processors are, all of them on average (percent).
        cpu: Option<f32>,
        /// What it takes of the machine's memory, and what it may (bytes).
        used: Option<u64>,
        total: Option<u64>,
        /// How much of the GPU its programs use (CUDA, DirectML, its
        /// desktop's windows), as Windows counts it for the machine's
        /// worker process (percent).
        gpu: Option<f32>,
    },
}

/// A game and its frames.
#[derive(Clone, Serialize, Deserialize)]
pub struct GameSample {
    /// What it calls itself (its window's title), or its program's name.
    pub name: String,
    /// Its program's name.
    pub program: String,
    /// Its process: which game this is, its name read or not.
    pub pid: u32,
    /// Its frames a second; none where frames cannot be counted (Glance left without the right to trace them).
    pub fps: Option<f32>,
    /// The 1% low, as frames a second; none until enough frames have come.
    pub low: Option<f32>,
    /// The longest frame of the last second, in milliseconds; none where `fps` is.
    pub longest_ms: Option<f32>,
    /// The refresh rate of its screen.
    pub refresh_hz: Option<u32>,
    /// Its screen, in physical pixels: left, top, right, bottom.
    pub screen: Option<[i32; 4]>,
    /// The GPU it uses most, as `StaticInfo::gpus` and `Sample::gpus` list it.
    pub gpu_index: Option<usize>,
    /// Its share of every processor, in percent, at the last look at the
    /// process list.
    pub cpu: Option<f32>,
    /// Its use of the GPU it uses most, in percent.
    pub gpu: Option<f32>,
    /// Its private memory, in bytes.
    pub mem: Option<u64>,
    /// The video memory it holds on its cards, in bytes.
    pub vram: Option<u64>,
    /// What holds back the clock of the GPU it uses most, where its driver
    /// says (NVIDIA's).
    pub gpu_limit: Option<GpuLimit>,
    /// How long it has been played (seconds): since it first presented
    /// frames, through short times away from it.
    pub playing_s: u64,
    /// Taken for a game by hand (the window in front, not covering its
    /// screen; see `presents::GameMode`): for the widget that asked,
    /// not the game overlay.
    pub by_hand: bool,
}

/// What holds a GPU's clock below what it could run at.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum GpuLimit {
    /// Nothing: it runs as fast as it is asked to.
    Free,
    /// Its power limit.
    Power,
    /// Its temperature.
    Thermal,
    /// The board slowing it, for its power supply or its heat, without
    /// saying which.
    Hardware,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MemorySample {
    pub used: u64,
    pub committed: u64,
    pub commit_limit: u64,
    pub cached: u64,
}

/// (Its default: nothing read of it.)
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct GpuSample {
    /// Percent, as busy as its busiest engine; `None` (as the memory's)
    /// where the counters could not be read this time.
    pub usage: Option<f32>,
    /// Busiest engine of each kind (3D, Copy, VideoDecode, …); a kind not
    /// listed has nothing running. `None` where the engines were not read.
    pub engines: Option<Vec<(String, f32)>>,
    pub mem_used: Option<u64>,
    pub shared_used: Option<u64>,
    pub temp: Option<f32>,
    /// How warm the warmest point of its chip is (°C), where its driver
    /// says that apart from its temperature.
    pub hot_spot: Option<f32>,
    pub clock_mhz: Option<f32>,
    pub fan_rpm: Option<u32>,
    /// Watts, as a discrete card's driver reports them.
    pub power: Option<f32>,
    /// Whether it sleeps (powered down, as a laptop's discrete GPU is while nothing draws on it): its driver is not
    /// asked anything then, and it has no readings but its use and its memory, as Windows counts them.
    #[serde(default)]
    pub asleep: bool,
}

/// One program: every process sharing an executable name, added together.
#[derive(Clone, Serialize, Deserialize)]
pub struct ProcessSample {
    pub name: String,
    /// Percent of the whole machine.
    pub cpu: f32,
    pub mem: u64,
    /// Bytes read and written per second, to disk and to the network alike
    /// (Windows counts a process's I/O without telling them apart).
    pub io: f64,
    /// Percent of its busiest GPU engine, as Task Manager shows it; `None`
    /// where the GPUs' counters could not be read (or the system offers
    /// none per process).
    pub gpu: Option<f32>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct VolumeSample {
    pub name: String,
    pub used: u64,
    pub total: u64,
}

/// The interface the default route goes through.
#[derive(Clone, Serialize, Deserialize)]
pub struct NetworkInfo {
    /// The connection's name ("WLAN", "Ethernet"), and the adapter's model.
    pub name: String,
    pub model: String,
    pub ipv4: Option<String>,
    pub link_bps: u64,
    /// The wireless network it is on, by its name; none on a wire, or
    /// where Windows does not say (it asks for its location service to be
    /// on before it tells a program which network it is on).
    pub wifi: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SystemSample {
    pub uptime_s: u64,
    pub processes: u32,
    pub threads: u32,
    pub handles: u32,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BatterySample {
    pub percent: u8,
    /// Taking charge now (not merely plugged in: a full battery on mains power takes none).
    pub charging: bool,
    /// On mains power.
    #[serde(default)]
    pub plugged: bool,
    pub seconds_left: Option<u32>,
    /// Watts going in (charging, above 0) or out (below 0): on battery
    /// power, what the whole machine draws.
    pub watts: Option<f32>,
    /// What it holds when full, as a share of what it was made to hold
    /// (percent).
    pub health: Option<f32>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CpuSensors {
    pub temp: Option<f32>,
    /// Each chiplet's temperature, by its number from 0, on CPUs that have
    /// several; one not read this time is left out.
    pub ccds: Vec<(usize, f32)>,
    pub power: Option<f32>,
    /// What the graphics built into the CPU draw, of `power`, where the CPU counts it apart (Intel's PP1 plane):
    /// shown as theirs (see `metrics::Sampler::from_driver`), not with the CPU.
    #[serde(default)]
    pub graphics_power: Option<f32>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BoardSensors {
    /// Temperatures by what they measure: a name from the board's layout
    /// (see `layout`), the chip's name for the input, or its number.
    pub temps: Vec<(String, f32)>,
    /// Fan headers that report a speed, named the same way, in RPM.
    pub fans: Vec<(String, f32)>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DriveTemperature {
    /// Which physical drive it is: drives of one model have one name.
    pub id: u32,
    pub name: String,
    pub celsius: f32,
}

#[cfg(test)]
mod tests {
    use super::{GpuInfo, StaticInfo};

    #[test]
    fn finds_gpus_by_the_number_they_were_given() {
        let gpu = |slot| GpuInfo { slot, identity: String::new(), name: format!("GPU {slot}"), mem_total: 0, shared_total: 0 };
        // The second adapter (a virtual display's) left out: the third keeps "gpu:2".
        let info = StaticInfo {
            cpu_name: String::new(),
            memory_modules: None,
            memory_speed: None,
            drives: Vec::new(),
            drive_ids: Vec::new(),
            network_adapter: None,
            board: String::new(),
            threads: 1,
            mem_total: 0,
            gpus: vec![gpu(0), gpu(2)],
            found: Vec::new(),
        };
        assert_eq!(info.gpu_modules().collect::<Vec<_>>(), ["gpu:0", "gpu:2"]);
        assert_eq!(info.gpu_of("gpu:2"), Some(1));
        assert_eq!(info.gpu_of("gpu:1"), None);
        assert_eq!(info.gpu_of("cpu"), None);
    }

    /// What Glance started with, from a sampler of the meanwhile (no drive listed), takes its drives from what the
    /// readings come from once it has looked at them, and keeps all else.
    #[test]
    fn takes_the_drives_listed_later() {
        let gpu = GpuInfo { slot: 1, identity: "10DE".into(), name: "GPU".into(), mem_total: 0, shared_total: 0 };
        let first = StaticInfo {
            cpu_name: "CPU".into(),
            memory_modules: None,
            memory_speed: None,
            drives: Vec::new(),
            drive_ids: Vec::new(),
            network_adapter: None,
            board: String::new(),
            threads: 8,
            mem_total: 16,
            gpus: vec![gpu],
            found: Vec::new(),
        };
        let listed = StaticInfo { drives: vec!["NVMe".into(), "USB".into()], drive_ids: vec![0, 2], cpu_name: "other".into(), gpus: Vec::new(), ..first.clone() };
        let now = first.with_drives_of(&listed);
        assert_eq!((now.drives, now.drive_ids), (vec!["NVMe".to_string(), "USB".to_string()], vec![0, 2]));
        assert_eq!((now.cpu_name.as_str(), now.gpus.len(), now.gpus[0].slot), ("CPU", 1, 1));
    }
}
