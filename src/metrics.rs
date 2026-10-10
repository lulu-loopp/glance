//! System metrics: one PDH query for every rate counter, plus the direct APIs
//! PDH has no counter for (memory, NIC octets, GPU sensors, process list).

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};


use crate::dimm::Dimms;
use crate::gpu_power::{self, GpuPower};
use crate::reading::{BatterySample, GameSample, GpuInfo, GpuSample, MemorySample, NetworkInfo, ProcessSample, Sample, StaticInfo, SystemSample, VolumeSample};
use crate::sensors::CpuReader;
use crate::mic::Microphone;
use crate::superio::SuperIo;
use windows::core::{w, PCWSTR};
use windows::Wdk::Graphics::Direct3D::{
    D3DKMTCloseAdapter, D3DKMTOpenAdapterFromLuid, D3DKMTQueryAdapterInfo, D3DKMT_ADAPTER_PERFDATA, D3DKMT_CLOSEADAPTER,
    D3DKMT_ADAPTERADDRESS, D3DKMT_ADAPTERTYPE, D3DKMT_NODE_PERFDATA, D3DKMT_OPENADAPTERFROMLUID, D3DKMT_QUERYADAPTERINFO,
    KMTQAITYPE_ADAPTERADDRESS, KMTQAITYPE_ADAPTERPERFDATA, KMTQAITYPE_ADAPTERTYPE, KMTQAITYPE_NODEPERFDATA,
};
use windows::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_SUCCESS, LUID, UNICODE_STRING};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};
use windows::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetAdaptersAddresses, GetBestInterface, GetIfTable2, GAA_FLAG_SKIP_ANYCAST,
    GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH, MIB_IF_TABLE2,
};
use windows::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives};
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhGetFormattedCounterValue, PdhOpenQueryW, PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA,
    PDH_FMT, PDH_FMT_COUNTERVALUE, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER,
    PDH_HQUERY, PDH_MORE_DATA,
};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ};
use windows::Win32::System::SystemInformation::GetTickCount64;

/// PDH_FMT_DOUBLE with PDH_FMT_NOCAP100, which lets a percentage counter
/// report above 100 (turbo frequencies do).
const PDH_FMT_DOUBLE_NOCAP100: PDH_FMT = PDH_FMT(PDH_FMT_DOUBLE.0 | 0x8000);

/// How many programs each ranking contributes; the page shows the top of
/// whichever ranking is chosen, and scrolls through the rest.
const TOP_PROCESSES: usize = 40;
const DRIVE_FIXED: u32 = 3;
/// How long a game can be away (another window in front, a screen that
/// draws nothing) and still be the same time playing it.
const PLAY_BREAK: Duration = Duration::from_secs(5 * 60);

struct Adapter {
    luid: (u32, i32),
    kmt_handle: u32,
    /// How its maker's driver gives its power, if it does.
    power: Option<gpu_power::Reader>,
}

/// What is read less often than every sample (see `Sampler::sample`).
#[derive(Default)]
struct Slow {
    processes: Vec<ProcessSample>,
    network: Option<NetworkInfo>,
}

/// Performance counters a machine may lack (disabled, or a display driver
/// too old for the GPU ones) are absent here; what they measure then goes
/// unreported, rather than everything else with it.
pub struct Sampler {
    /// When the system last started (see `Started`).
    started: Started,
    /// WSL, while its module is on.
    wsl: Option<crate::wsl::Wsl>,
    /// Docker, while its module is on.
    docker: Option<crate::docker::Docker>,
    query: PDH_HQUERY,
    cpu_time: Option<PDH_HCOUNTER>,
    cpu_performance: Option<PDH_HCOUNTER>,
    /// Each logical processor's performance and nominal clock, and, on a
    /// processor with cores of more than one kind, its kind (see `Cores`).
    cores: Cores,
    gpu_engine: Option<PDH_HCOUNTER>,
    gpu_dedicated: Option<PDH_HCOUNTER>,
    gpu_shared: Option<PDH_HCOUNTER>,
    disk_read: Option<PDH_HCOUNTER>,
    disk_write: Option<PDH_HCOUNTER>,
    disk_idle: Option<PDH_HCOUNTER>,
    /// The GPUs' last readings, kept through a momentary counter failure.
    /// The processors' idle and busy time at the last sample (100 ns units).
    system_times: Option<(u64, u64)>,
    processes: ProcessTable,
    slow: Slow,
    base_mhz: f64,
    adapters: Vec<Adapter>,
    gpu_power: GpuPower,
    /// The totals at the last successful read of the interface table.
    net_prev: Option<(HashMap<u64, (u64, u64)>, Instant)>,
    /// Readers that need the driver; absent without it or without rights.
    cpu_sensors: Option<CpuReader>,
    super_io: Option<SuperIo>,
    /// An ASUS laptop's fans, as its firmware reports them.
    asus_fans: Option<crate::asus::AsusFans>,
    dimms: Option<Dimms>,
    mic: Option<Microphone>,
    /// Each process's GPU use at the last sample, by process id.
    /// Each process's use of the GPUs (unread where an instance of its
    /// was), from the last reading of the engine counters; `None` while
    /// they cannot be read.
    gpu_by_pid: Option<HashMap<usize, Option<f32>>>,
    /// The adapter each process uses most, from the same reading.
    adapter_by_pid: HashMap<usize, (u32, i32)>,
    /// Each process's video memory, its dedicated part.
    gpu_process_memory: Option<PDH_HCOUNTER>,
    /// The game being played: its program, when it was first seen, and when
    /// last.
    playing: Option<(String, Instant, Instant)>,
    buf: Vec<u64>,
    pub info: StaticInfo,
}

// PDH handles are plain pointers owned by this struct; it lives on one thread at a time.
unsafe impl Send for Sampler {}

impl Sampler {
    pub fn new() -> Self {
        let mut query = PDH_HQUERY::default();
        // Without a query no counter can be added, and every sample fails.
        let _ = unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) };
        let add = |path: PCWSTR| {
            let mut counter = PDH_HCOUNTER::default();
            (unsafe { PdhAddEnglishCounterW(query, path, 0, &mut counter) } == ERROR_SUCCESS.0).then_some(counter)
        };
        let cpu_time = add(w!(r"\Processor Information(*)\% Processor Time"));
        let cpu_performance = add(w!(r"\Processor Information(_Total)\% Processor Performance"));
        let cores = Cores {
            classes: Cores::kinds_of_this_machine(),
            performance: add(w!(r"\Processor Information(*)\% Processor Performance")),
            nominal: add(w!(r"\Processor Information(*)\Processor Frequency")),
        };
        let gpu_engine = add(w!(r"\GPU Engine(*)\Utilization Percentage"));
        let gpu_dedicated = add(w!(r"\GPU Adapter Memory(*)\Dedicated Usage"));
        let gpu_shared = add(w!(r"\GPU Adapter Memory(*)\Shared Usage"));
        let gpu_process_memory = add(w!(r"\GPU Process Memory(*)\Dedicated Usage"));
        let disk_read = add(w!(r"\PhysicalDisk(_Total)\Disk Read Bytes/sec"));
        let disk_write = add(w!(r"\PhysicalDisk(_Total)\Disk Write Bytes/sec"));
        let disk_idle = add(w!(r"\PhysicalDisk(_Total)\% Idle Time"));
        // Rate counters need a first collection to diff against.
        unsafe { PdhCollectQueryData(query) };
        let mut processes = ProcessTable::default();
        processes.sample(1, None);

        let gpu_power = GpuPower::open();
        let (adapters, gpus) = enumerate_gpus(&gpu_power);
        let cpu_key = w!(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0");
        let cpu_sensors = CpuReader::open();
        let super_io = SuperIo::open(&reg_string(w!(r"HARDWARE\DESCRIPTION\System\BIOS"), w!("BaseBoardManufacturer")));
        let dimms = Dimms::open();
        let asus_fans = crate::asus::AsusFans::open();
        let missing = || "not found".to_string();
        let mut found = vec![
            format!("CPU sensors: {}", cpu_sensors.as_ref().map_or_else(missing, |cpu| cpu.describe().to_string())),
            format!("Motherboard chip: {}", super_io.as_ref().map_or_else(|why| format!("not found ({why})"), SuperIo::describe)),
            format!("Memory sensors: {}", dimms.as_ref().map_or_else(missing, Dimms::describe)),
            format!("Laptop fans: {}", asus_fans.as_ref().map_or_else(missing, crate::asus::AsusFans::describe)),
        ];
        found.extend(adapters.iter().zip(&gpus).map(|(adapter, gpu)| {
            format!("GPU power, {}: {}", gpu.name, adapter.power.map_or("not available", gpu_power::Reader::describe))
        }));
        let modules = crate::smbios::memory_modules();
        let drives = crate::drives::models();
        let info = StaticInfo {
            cpu_name: reg_string(cpu_key, w!("ProcessorNameString")),
            memory_modules: crate::smbios::describe(&modules),
            memory_speed: crate::smbios::speed(&modules),
            drives: drives.iter().map(|(_, name)| name.clone()).collect(),
            drive_ids: drives.iter().map(|(id, _)| *id).collect(),
            network_adapter: default_interface().map(|adapter| adapter.model),
            board: reg_string(w!(r"HARDWARE\DESCRIPTION\System\BIOS"), w!("BaseBoardProduct")),
            threads: std::thread::available_parallelism().map_or(1, |n| n.get()),
            mem_total: performance_info().PhysicalTotal as u64 * performance_info().PageSize as u64,
            gpus,
            found,
        };
        Sampler {
            query,
            cpu_time,
            cpu_performance,
            cores,
            gpu_engine,
            gpu_dedicated,
            gpu_shared,
            disk_read,
            disk_write,
            disk_idle,
            system_times: system_times(),
            processes,
            slow: Slow::default(),
            base_mhz: reg_dword(cpu_key, w!("~MHz")) as f64,
            adapters,
            gpu_power,
            net_prev: net_octets().map(|adapters| (adapters, Instant::now())),
            cpu_sensors,
            super_io: super_io.ok(),
            asus_fans,
            dimms,
            mic: Microphone::open(),
            gpu_by_pid: None,
            adapter_by_pid: HashMap::new(),
            gpu_process_memory,
            playing: None,
            buf: Vec::new(),
            info,
            started: Started::read(),
            wsl: None,
            docker: None,
        }
    }

    /// The game to show: of the programs presenting frames, a game on the
    /// pointer's screen, else a game on another.
    /// A game is one whose window covers its screen, or (asked for by hand,
    /// see `presents::GameMode`) the one in front.
    /// Named by its window's title, its program from the process list, and
    /// its use of the machine from the counters (never by opening its
    /// process: anti-cheat watches for that). `collected` says whether the
    /// counters were read this time.
    fn game(&mut self, collected: bool) -> Option<GameSample> {
        // A game: a program presenting frames whose window covers its screen.
        let front = crate::presents::game_mode() == crate::presents::GameMode::On;
        let found: Vec<crate::presents::Presenting> = crate::presents::presenting().into_iter().filter(|p| p.fills_screen || (front && p.in_front)).collect();
        let now = Instant::now();
        let chosen = found
            .iter()
            .find(|p| front && p.in_front)
            .or_else(|| found.iter().find(|p| p.under_pointer))
            .or_else(|| found.first());
        let Some(presenting) = chosen else {
            // The game turned off by hand has gone: the next is shown as found.
            if crate::presents::game_mode() == crate::presents::GameMode::Off {
                crate::presents::set_game_mode(crate::presents::GameMode::Auto);
            }
            self.forget_playing(now);
            return None;
        };
        let pid = presenting.pid as usize;
        let program = self.processes.name_of(pid).unwrap_or_default();
        // The same game again soon after it was last seen is the same time
        // playing it (a look at another window, a loading screen that drew
        // nothing).
        let start = match &self.playing {
            Some((was, start, last)) if *was == program && now.duration_since(*last) < PLAY_BREAK => *start,
            _ => now,
        };
        self.playing = Some((program.clone(), start, now));
        let playing_s = now.duration_since(start).as_secs();
        let usage = self.processes.usage_of(pid);
        let adapter = self.adapter_by_pid.get(&pid).and_then(|luid| self.adapters.iter().find(|a| a.luid == *luid));
        let gpu_limit = adapter.and_then(|a| a.power).and_then(|reader| self.gpu_power.limit(reader));
        // Its memory on the card it uses most (or on every card, while it
        // uses none); unread where an instance of it is.
        let card = self.adapter_by_pid.get(&pid).copied();
        let vram = self.gpu_process_memory.filter(|_| collected).and_then(|counter| {
            let mut total = Some(0u64);
            for (name, value) in read_array(counter, &mut self.buf)? {
                if parse_pid(&name) == Some(pid) && card.is_none_or(|card| parse_luid(&name) == Some(card)) {
                    total = total.zip(value).map(|(total, value)| total + value as u64);
                }
            }
            total
        });
        Some(GameSample {
            name: if presenting.title.is_empty() { program.clone() } else { presenting.title.clone() },
            program: program.clone(),
            pid: presenting.pid,
            fps: presenting.stats.fps,
            low: presenting.stats.low,
            longest_ms: presenting.stats.longest_ms,
            refresh_hz: presenting.refresh_hz,
            screen: presenting.screen,
            gpu_index: self.adapter_by_pid.get(&pid).and_then(|luid| self.adapters.iter().position(|a| a.luid == *luid)),
            cpu: usage.map(|(cpu, _)| cpu),
            // A process the counters list nothing for uses no GPU.
            gpu: self.gpu_by_pid.as_ref().and_then(|by_pid| by_pid.get(&pid).copied().unwrap_or(Some(0.0))),
            mem: usage.map(|(_, mem)| mem),
            vram,
            gpu_limit,
            playing_s,
            by_hand: !presenting.fills_screen,
        })
    }

    /// Forgets the game played once it has been gone a while.
    fn forget_playing(&mut self, now: Instant) {
        if self.playing.as_ref().is_some_and(|(_, _, last)| now.duration_since(*last) >= PLAY_BREAK) {
            self.playing = None;
        }
    }

    /// Takes one sample. Reading the process list and the network adapters is
    /// the expensive part and nothing charts it, so the caller says when it is
    /// worth refreshing. Returns `None` when PDH has no valid data for this
    /// interval (it reports that for a tick now and then, e.g. after resume).
    /// A sample now; `refresh_slow`, with what is read less often read
    /// again; `wsl` and `docker`, with those watched (their modules on).
    pub fn sample(&mut self, refresh_slow: bool, wsl: bool, docker: bool) -> Option<Sample> {
        // Watched only while asked for: nothing is read for them otherwise.
        if wsl != self.wsl.is_some() {
            self.wsl = wsl.then(crate::wsl::Wsl::new);
        }
        if docker != self.docker.is_some() {
            self.docker = docker.then(crate::docker::Docker::new);
        }
        // Counters not collected this time read as absent, not as their
        // last values; what does not depend on them is sampled regardless.
        let collected = unsafe { PdhCollectQueryData(self.query) } == ERROR_SUCCESS.0;
        let counter = |counter: Option<PDH_HCOUNTER>| counter.filter(|_| collected);

        let mut cpu = None;
        // Each thread in its processor's place (its group's first, and its
        // number in the group); one not read this time, or not listed,
        // stays unread there.
        let per_thread = counter(self.cpu_time).and_then(|counter| read_array(counter, &mut self.buf));
        // Looked up each time: processors can be added while Glance runs.
        let (group_starts, processors) = processor_groups();
        let mut threads: Vec<Option<f32>> = if per_thread.is_some() { vec![None; processors] } else { Vec::new() };
        for (name, value) in per_thread.unwrap_or_default() {
            if name == "_Total" {
                cpu = value.map(|value| value as f32);
            } else if let Some((group, index)) = name.split_once(',') {
                if let (Ok(group), Ok(index)) = (group.parse::<usize>(), index.parse::<usize>()) {
                    let place = group_starts.get(group).map(|start| start + index);
                    if let Some(cell) = place.and_then(|place| threads.get_mut(place)) {
                        *cell = value.map(|value| value as f32);
                    }
                }
            }
        }
        // Each thread's clock, and each kind of core's.
        let (kinds_ghz, threads_ghz) = if collected { self.cores.read(&group_starts, processors, &mut self.buf) } else { (Vec::new(), Vec::new()) };
        // Without the counters, the whole processor's use from the kernel's
        // own account of its time (and no per-thread grid).
        let times = system_times();
        let cpu = cpu.or_else(|| {
            let ((idle, busy), (idle_before, busy_before)) = (times?, self.system_times?);
            let total = (idle - idle_before) + (busy - busy_before);
            (total > 0).then(|| ((busy - busy_before) as f64 / total as f64 * 100.0) as f32)
        });
        self.system_times = times;
        // Without the performance counter, the clock reads as its base.
        let performance = counter(self.cpu_performance).and_then(|c| read_scalar(c, PDH_FMT_DOUBLE_NOCAP100)).unwrap_or(100.0);

        let gpus = self.sample_gpus(collected);

        if refresh_slow {
            // A process's share of every processor there is now.
            self.slow.processes = self.processes.sample(processors, self.gpu_by_pid.as_ref());
            self.slow.network = default_interface();
        }

        // Traffic since the last successful read, over the time since then:
        // a read that fails counts nothing, and the next spans both. Each
        // adapter against its own last count: one that has just appeared
        // counts from its next read, one whose counters restarted counts
        // from zero.
        let now = Instant::now();
        let octets = net_octets();
        // No rate without two reads to take it between.
        let (net_down, net_up) = match (&octets, &self.net_prev) {
            (Some(adapters), Some((before, at))) => {
                let dt = now.duration_since(*at).as_secs_f64();
                let moved = |now: u64, then: u64| if now >= then { now - then } else { now };
                let (down, up) = adapters.iter().fold((0u64, 0u64), |(down, up), (luid, (rx, tx))| match before.get(luid) {
                    Some((rx_then, tx_then)) => (down + moved(*rx, *rx_then), up + moved(*tx, *tx_then)),
                    None => (down, up),
                });
                (Some(down as f64 / dt), Some(up as f64 / dt))
            }
            _ => (None, None),
        };
        if let Some(adapters) = octets {
            self.net_prev = Some((adapters, now));
        }
        let (down, up) = self
            .net_prev
            .as_ref()
            .map_or((0, 0), |(adapters, _)| adapters.values().fold((0, 0), |(down, up), (rx, tx)| (down + rx, up + tx)));

        let perf = performance_info();
        let page = perf.PageSize as u64;
        Some(Sample {
            t: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
            cpu,
            threads,
            ghz: Some((self.base_mhz * performance / 100_000.0) as f32),
            kinds_ghz,
            threads_ghz,
            memory: MemorySample {
                used: (perf.PhysicalTotal - perf.PhysicalAvailable) as u64 * page,
                committed: perf.CommitTotal as u64 * page,
                commit_limit: perf.CommitLimit as u64 * page,
                cached: perf.SystemCache as u64 * page,
            },
            gpus,
            net_down,
            net_up,
            net_total_down: down,
            net_total_up: up,
            network: self.slow.network.clone(),
            disk_read: counter(self.disk_read).and_then(|c| read_scalar(c, PDH_FMT_DOUBLE)),
            disk_write: counter(self.disk_write).and_then(|c| read_scalar(c, PDH_FMT_DOUBLE)),
            disk_active: counter(self.disk_idle).and_then(|c| read_scalar(c, PDH_FMT_DOUBLE)).map(|idle| (100.0 - idle).clamp(0.0, 100.0) as f32),
            volumes: volumes(),
            processes: self.slow.processes.clone(),
            system: SystemSample {
                uptime_s: self.started.uptime_s(),
                processes: perf.ProcessCount,
                threads: perf.ThreadCount,
                handles: perf.HandleCount,
            },
            battery: battery(),
            cpu_sensors: self.cpu_sensors.as_mut().map(CpuReader::read),
            board: self.board(),
            drive_temps: crate::drives::temperatures(),
            dimm_temps: self.dimms.as_mut().map(Dimms::read).unwrap_or_default(),
            mic_muted: self.mic.as_ref().and_then(Microphone::muted),
            wsl: self.wsl.as_mut().map(|wsl| wsl.read(self.gpu_by_pid.as_ref(), &self.processes.names)),
            docker: self.docker.as_ref().and_then(crate::docker::Docker::read),
            game: self.game(collected),
        })
    }

    /// Each adapter's readings. Each counter is read on its own: one that
    /// cannot be read leaves its readings unread ("—"), and the others and
    /// what each driver gives (temperature, fan, clock, power) stand.
    /// Uncollected (`collected` false), the counters would only repeat their
    /// last values, and all of theirs are unread.
    fn sample_gpus(&mut self, collected: bool) -> Vec<GpuSample> {
        let engines = if collected { self.engine_use() } else { None };
        if engines.is_none() {
            // The processes kept from an earlier look no longer know theirs.
            self.gpu_by_pid = None;
            self.slow.processes.iter_mut().for_each(|process| process.gpu = None);
        }
        // Each adapter's memory in use, unread where any of its instances is.
        let mut memory = |counter: Option<PDH_HCOUNTER>| -> Option<HashMap<(u32, i32), Option<u64>>> {
            let mut per_adapter: HashMap<(u32, i32), Option<u64>> = HashMap::new();
            for (name, value) in read_array(counter?, &mut self.buf)? {
                if let Some(luid) = parse_luid(&name) {
                    let total = per_adapter.entry(luid).or_insert(Some(0));
                    *total = total.zip(value).map(|(total, value)| total + value as u64);
                }
            }
            Some(per_adapter)
        };
        let (dedicated, shared) = if collected { (memory(self.gpu_dedicated), memory(self.gpu_shared)) } else { (None, None) };
        self.adapters
            .iter()
            .map(|adapter| {
                // An adapter the counters list nothing for has nothing
                // running; one with an instance not read has no use known.
                let kinds: Option<Vec<(String, f32)>> = engines.as_ref().filter(|(_, unread)| !unread.contains(&adapter.luid)).map(|(engines, _)| {
                    let mut kinds: Vec<(String, f32)> = Vec::new();
                    for ((luid, _), (kind, usage)) in engines {
                        if *luid != adapter.luid {
                            continue;
                        }
                        let usage = usage.min(100.0) as f32;
                        match kinds.iter_mut().find(|(known, _)| known == kind) {
                            Some((_, busiest)) => *busiest = busiest.max(usage),
                            None => kinds.push((kind.clone(), usage)),
                        }
                    }
                    kinds.sort_by(|a, b| a.0.cmp(&b.0));
                    kinds
                });
                let counted = GpuSample {
                    usage: kinds.as_ref().map(|kinds| kinds.iter().map(|(_, usage)| *usage).fold(0.0, f32::max)),
                    engines: kinds,
                    mem_used: dedicated.as_ref().and_then(|used| used.get(&adapter.luid).copied().unwrap_or(Some(0))),
                    shared_used: shared.as_ref().and_then(|used| used.get(&adapter.luid).copied().unwrap_or(Some(0))),
                    temp: None,
                    fan_rpm: None,
                    clock_mhz: None,
                    power: None,
                };
                self.from_driver(adapter, counted)
            })
            .collect()
    }

    /// Each engine's use (its kind, and percent), by adapter and engine,
    /// and the adapters with an instance not read this time; each process's
    /// use of the GPUs is kept besides. `None` when the counters cannot be
    /// read.
    fn engine_use(&mut self) -> Option<Engines> {
        // An engine's utilisation is the sum over the processes using it; a
        // kind of engine is as busy as its busiest engine; an adapter is as
        // busy as its busiest kind.
        let mut engines: HashMap<((u32, i32), u32), (String, f64)> = HashMap::new();
        // A process's use is that of the engine it uses most.
        // Each process's use of each engine; unread where an instance of it is.
        let mut by_process: HashMap<(usize, (u32, i32), u32), Option<f64>> = HashMap::new();
        let mut unread = HashSet::new();
        for (name, value) in read_array(self.gpu_engine?, &mut self.buf)? {
            if let (Some(luid), Some(engine), Some(kind)) = (parse_luid(&name), parse_engine(&name), parse_kind(&name)) {
                if let Some(pid) = parse_pid(&name) {
                    let total = by_process.entry((pid, luid, engine)).or_insert(Some(0.0));
                    *total = total.zip(value).map(|(total, value)| total + value);
                }
                let Some(value) = value else {
                    unread.insert(luid);
                    continue;
                };
                engines.entry((luid, engine)).or_insert_with(|| (kind, 0.0)).1 += value;
            }
        }
        let mut by_pid: HashMap<usize, Option<f32>> = HashMap::new();
        // The adapter of each process's busiest engine, and that engine's use.
        let mut adapters: HashMap<usize, ((u32, i32), f64)> = HashMap::new();
        for ((pid, luid, _), value) in by_process {
            let use_ = by_pid.entry(pid).or_insert(Some(0.0));
            *use_ = use_.zip(value).map(|(most, value)| most.max(value.min(100.0) as f32));
            // A process using no engine now uses no card.
            let value = value.unwrap_or(0.0);
            if value > 0.0 && adapters.get(&pid).is_none_or(|(_, most)| value > *most) {
                adapters.insert(pid, (luid, value));
            }
        }
        self.gpu_by_pid = Some(by_pid);
        self.adapter_by_pid = adapters.into_iter().map(|(pid, (luid, _))| (pid, luid)).collect();
        Some((engines, unread))
    }

    /// `gpu` with what the adapter's driver gives now: its temperature,
    /// fan, clock and power.
    fn from_driver(&self, adapter: &Adapter, gpu: GpuSample) -> GpuSample {
        let perf = adapter_perf(adapter.kmt_handle);
        GpuSample {
            temp: perf.and_then(|p| (p.Temperature != 0).then(|| p.Temperature as f32 / 10.0)),
            fan_rpm: perf.and_then(|p| (p.FanRPM != 0).then_some(p.FanRPM)),
            clock_mhz: graphics_clock(adapter.kmt_handle),
            power: adapter.power.and_then(|reader| self.gpu_power.read(reader)),
            ..gpu
        }
    }
}

/// One processor's times since boot, as the kernel keeps them (100 ns units;
/// kernel time includes idle time).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ProcessorTimes {
    idle: i64,
    kernel: i64,
    user: i64,
    dpc: i64,
    interrupt: i64,
    interrupt_count: u32,
}

/// The query class that answers with each processor's times, for one group.
const PROCESSOR_TIMES: u32 = 8;

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQuerySystemInformationEx(class: u32, input: *const core::ffi::c_void, input_length: u32, output: *mut core::ffi::c_void, output_length: u32, returned: *mut u32) -> i32;
}

/// Each active processor group's first processor, counted across the
/// groups, and how many active processors there are in all.
/// What reads each logical processor's clock, and the kinds of cores a
/// processor has (big and little: Windows' efficiency classes, the most
/// capable highest), where it has more than one.
struct Cores {
    /// Each logical processor's class, by its counters' instance name
    /// ("group,number"); none where they are all alike.
    classes: Option<HashMap<String, u8>>,
    performance: Option<PDH_HCOUNTER>,
    nominal: Option<PDH_HCOUNTER>,
}

impl Cores {
    /// Each logical processor's class, if they are not all alike.
    fn kinds_of_this_machine() -> Option<HashMap<String, u8>> {
        use windows::Win32::System::SystemInformation::{GetSystemCpuSetInformation, SYSTEM_CPU_SET_INFORMATION};
        let mut needed = 0u32;
        let _ = unsafe { GetSystemCpuSetInformation(None, 0, &mut needed, None, None) };
        let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
        if !unsafe { GetSystemCpuSetInformation(Some(buf.as_mut_ptr().cast()), needed, &mut needed, None, None) }.as_bool() {
            return None;
        }
        // Records one after another, each as long as it says.
        let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), needed as usize) };
        let mut classes = HashMap::new();
        let mut at = 0;
        while at + size_of::<SYSTEM_CPU_SET_INFORMATION>() <= bytes.len() {
            let record = unsafe { &*(bytes.as_ptr().add(at) as *const SYSTEM_CPU_SET_INFORMATION) };
            if record.Size == 0 {
                break;
            }
            let set = unsafe { record.Anonymous.CpuSet };
            classes.insert(format!("{},{}", set.Group, set.LogicalProcessorIndex), set.EfficiencyClass);
            at += record.Size as usize;
        }
        let mut kinds: Vec<u8> = classes.values().copied().collect();
        kinds.sort_unstable();
        kinds.dedup();
        (kinds.len() > 1).then_some(classes)
    }

    /// Each kind's clock now (GHz), the most capable first (none for a
    /// processor whose cores are alike), and each of the `processors`
    /// logical processors' (placed as `group_starts` say, as the threads'
    /// loads are).
    fn read(&self, group_starts: &[usize], processors: usize, buf: &mut Vec<u64>) -> (Vec<Option<f32>>, Vec<Option<f32>>) {
        let performance = self.performance.and_then(|c| read_array(c, buf)).unwrap_or_default();
        let nominal: HashMap<String, f64> = self.nominal.and_then(|c| read_array(c, buf)).unwrap_or_default().into_iter().filter_map(|(name, mhz)| Some((name, mhz?))).collect();
        let clocks: Vec<(String, f64)> = performance.into_iter().filter_map(|(name, share)| Some((name.clone(), share? / 100.0 * nominal.get(&name)? / 1000.0))).collect();
        let mut threads = vec![None; processors];
        for (name, ghz) in &clocks {
            let Some((group, index)) = name.split_once(',') else { continue };
            let (Ok(group), Ok(index)) = (group.parse::<usize>(), index.parse::<usize>()) else { continue };
            if let Some(cell) = group_starts.get(group).map(|start| start + index).and_then(|place| threads.get_mut(place)) {
                *cell = Some(*ghz as f32);
            }
        }
        let kinds = match &self.classes {
            Some(classes) => average_by_kind(classes, clocks.iter().filter_map(|(name, ghz)| Some((*classes.get(name)?, *ghz)))),
            None => Vec::new(),
        };
        (kinds, threads)
    }
}

/// Each class's average of `clocks` (class, GHz), the highest class first;
/// a class `classes` has that has no clock read, none.
fn average_by_kind(classes: &HashMap<String, u8>, clocks: impl Iterator<Item = (u8, f64)>) -> Vec<Option<f32>> {
    let mut kinds: Vec<u8> = classes.values().copied().collect();
    kinds.sort_unstable_by(|a, b| b.cmp(a));
    kinds.dedup();
    let mut sums: HashMap<u8, (f64, u32)> = HashMap::new();
    for (class, ghz) in clocks {
        let sum = sums.entry(class).or_default();
        *sum = (sum.0 + ghz, sum.1 + 1);
    }
    kinds.iter().map(|class| sums.get(class).map(|(total, count)| (total / *count as f64) as f32)).collect()
}

/// When the system last started: from off, a restart, or with Fast Startup
/// on (where "Shut down" hibernates the system and the next start resumes
/// it, the tick count running on) or out of hibernation. Waking from sleep
/// is not starting it.
struct Started {
    /// The moment, as milliseconds of the tick count (which runs on through
    /// sleep and hibernation).
    at_tick: u64,
    /// How long the system had been asleep or hibernating, all told, when
    /// it was read (milliseconds): grown since, it may have started again.
    suspended: u64,
}

impl Started {
    fn read() -> Self {
        let (tick, suspended) = ticks();
        // The system's own record of the last start, else the tick count's
        // (which knows only a start from off).
        let at_tick = last_boot().map_or(0, |ago| tick.saturating_sub(ago));
        Started { at_tick, suspended }
    }

    fn uptime_s(&mut self) -> u64 {
        let (tick, suspended) = ticks();
        // Suspended since it was read: out of hibernation, a start of its
        // own (out of sleep, the record is as it was).
        if suspended > self.suspended + 1000 {
            *self = Started::read();
        }
        tick.saturating_sub(self.at_tick) / 1000
    }
}

/// The tick count (milliseconds since the system last started from off,
/// sleep and hibernation counted), and how much of it the system spent
/// asleep or hibernating.
fn ticks() -> (u64, u64) {
    let tick = unsafe { GetTickCount64() };
    let mut unbiased = 0u64;
    let _ = unsafe { windows::Win32::System::WindowsProgramming::QueryUnbiasedInterruptTime(&mut unbiased) };
    (tick, tick.saturating_sub(unbiased / 10_000))
}

/// How long ago the system last started (milliseconds), as its event log
/// records it: Kernel-Boot's event 27, written at each start from off, by
/// Fast Startup and out of hibernation, not out of sleep.
fn last_boot() -> Option<u64> {
    use windows::Win32::System::EventLog::{
        EvtClose, EvtCreateRenderContext, EvtNext, EvtQuery, EvtQueryChannelPath, EvtQueryReverseDirection, EvtRender, EvtRenderContextValues, EvtRenderEventValues,
        EvtVarTypeFileTime, EVT_HANDLE, EVT_VARIANT,
    };
    struct Handle(EVT_HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            let _ = unsafe { EvtClose(self.0) };
        }
    }
    let query = w!("*[System[Provider[@Name='Microsoft-Windows-Kernel-Boot'] and EventID=27]]");
    let results = Handle(unsafe { EvtQuery(None, w!("System"), query, EvtQueryChannelPath.0 | EvtQueryReverseDirection.0) }.ok()?);
    let mut events = [0isize; 1];
    let mut returned = 0u32;
    unsafe { EvtNext(results.0, &mut events, 0, 0, &mut returned) }.ok()?;
    if returned == 0 {
        return None;
    }
    let event = Handle(EVT_HANDLE(events[0]));
    let paths = [w!("Event/System/TimeCreated/@SystemTime")];
    let context = Handle(unsafe { EvtCreateRenderContext(Some(&paths), EvtRenderContextValues.0) }.ok()?);
    let mut value = EVT_VARIANT::default();
    let (mut used, mut count) = (0u32, 0u32);
    unsafe { EvtRender(Some(context.0), event.0, EvtRenderEventValues.0, size_of::<EVT_VARIANT>() as u32, Some(&mut value as *mut _ as *mut _), &mut used, &mut count) }.ok()?;
    if value.Type != EvtVarTypeFileTime.0 as u32 {
        return None;
    }
    // FILETIME (100 ns since 1601) against now.
    let then = unsafe { value.Anonymous.FileTimeVal };
    let now = unsafe { windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime() };
    let now = (now.dwHighDateTime as u64) << 32 | now.dwLowDateTime as u64;
    Some(now.saturating_sub(then) / 10_000)
}

fn processor_groups() -> (Vec<usize>, usize) {
    use windows::Win32::System::Threading::{GetActiveProcessorCount, GetActiveProcessorGroupCount};
    let mut starts = Vec::new();
    let mut total = 0;
    for group in 0..unsafe { GetActiveProcessorGroupCount() } {
        starts.push(total);
        total += unsafe { GetActiveProcessorCount(group) } as usize;
    }
    (starts, total)
}

/// All processors' idle time, and their busy (kernel and user, less idle)
/// time, since boot, in 100 ns units: every processor group's, where
/// GetSystemTimes would count only one group's beyond 64 processors.
fn system_times() -> Option<(u64, u64)> {
    use windows::Win32::System::Threading::{GetActiveProcessorCount, GetActiveProcessorGroupCount};
    let (mut idle, mut busy) = (0u64, 0u64);
    for group in 0..unsafe { GetActiveProcessorGroupCount() } {
        let mut times = vec![ProcessorTimes::default(); unsafe { GetActiveProcessorCount(group) } as usize];
        let mut returned = 0u32;
        let status = unsafe {
            NtQuerySystemInformationEx(
                PROCESSOR_TIMES,
                (&group as *const u16).cast(),
                size_of::<u16>() as u32,
                times.as_mut_ptr().cast(),
                size_of_val(times.as_slice()) as u32,
                &mut returned,
            )
        };
        if status < 0 {
            return None;
        }
        for time in &times[..returned as usize / size_of::<ProcessorTimes>()] {
            idle += time.idle as u64;
            busy += (time.kernel + time.user - time.idle) as u64;
        }
    }
    Some((idle, busy))
}

fn valid(status: u32) -> bool {
    status == PDH_CSTATUS_VALID_DATA || status == PDH_CSTATUS_NEW_DATA
}

pub(crate) fn read_scalar(counter: PDH_HCOUNTER, format: PDH_FMT) -> Option<f64> {
    let mut value = PDH_FMT_COUNTERVALUE::default();
    let status = unsafe { PdhGetFormattedCounterValue(counter, format, None, &mut value) };
    (status == ERROR_SUCCESS.0 && valid(value.CStatus)).then(|| unsafe { value.Anonymous.doubleValue })
}

/// Reads every instance of a wildcard counter as (instance name, value); an
/// instance whose data is not valid this time has no value.
pub(crate) fn read_array(counter: PDH_HCOUNTER, buf: &mut Vec<u64>) -> Option<Vec<(String, Option<f64>)>> {
    loop {
        let mut bytes = (buf.len() * 8) as u32;
        let mut count = 0u32;
        let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
        let status = unsafe {
            PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut bytes, &mut count, Some(items))
        };
        if status == PDH_MORE_DATA {
            // The instance set can grow between the sizing call and the read.
            buf.resize((bytes as usize).div_ceil(8), 0);
            continue;
        }
        if status != ERROR_SUCCESS.0 {
            return None;
        }
        let items = unsafe { std::slice::from_raw_parts(items, count as usize) };
        return Some(
            items
                .iter()
                .map(|item| unsafe {
                    let value = valid(item.FmtValue.CStatus).then(|| item.FmtValue.Anonymous.doubleValue);
                    (item.szName.to_string().unwrap_or_default(), value)
                })
                .collect(),
        );
    }
}

/// The head of each record `NtQuerySystemInformation` returns for the
/// process list. The performance counters for processes carry the same
/// numbers but cost an order of magnitude more to collect.
#[repr(C)]
struct ProcessRecord {
    next_entry_offset: u32,
    number_of_threads: u32,
    working_set_private: i64,
    hard_fault_count: u32,
    threads_high_watermark: u32,
    cycle_time: u64,
    create_time: i64,
    user_time: i64,
    kernel_time: i64,
    image_name: UNICODE_STRING,
    base_priority: i32,
    process_id: usize,
    inherited_from_process_id: usize,
    handle_count: u32,
    session_id: u32,
    process_key: usize,
    peak_virtual_size: usize,
    virtual_size: usize,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
    private_page_count: usize,
    read_operation_count: i64,
    write_operation_count: i64,
    other_operation_count: i64,
    read_transfer_count: i64,
    write_transfer_count: i64,
    other_transfer_count: i64,
}

const SYSTEM_PROCESS_INFORMATION: u32 = 5;
const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC0000004u32 as i32;
const IDLE_PROCESS_ID: usize = 0;

#[link(name = "ntdll", kind = "raw-dylib")]
extern "system" {
    fn NtQuerySystemInformation(class: u32, info: *mut c_void, length: u32, returned: *mut u32) -> i32;
}

/// CPU time and bytes moved of every process at the previous sample, to
/// diff against.
#[derive(Default)]
struct ProcessTable {
    /// Keyed by (process id, creation time): ids are reused.
    totals: HashMap<(usize, i64), (i64, i64)>,
    /// Each process's name, by id, as of the last look.
    names: HashMap<usize, String>,
    /// Each process's share of every processor (percent) and private
    /// memory, by id, as of the last look.
    usage: HashMap<usize, (f32, u64)>,
    at: Option<Instant>,
    buf: Vec<u64>,
}

impl ProcessTable {
    /// The name of process `pid` at the last look, if it was there.
    fn name_of(&self, pid: usize) -> Option<String> {
        self.names.get(&pid).cloned()
    }

    /// The share of every processor (percent) and the private memory of
    /// process `pid` at the last look, if it was there and there was a look
    /// before it to take the share since.
    fn usage_of(&self, pid: usize) -> Option<(f32, u64)> {
        self.usage.get(&pid).copied()
    }

    /// The busiest programs since the previous call: the top of the ranking
    /// by CPU, by memory, by I/O and by GPU, together. Processes sharing an
    /// executable name are added together.
    fn sample(&mut self, processors: usize, gpu_by_pid: Option<&HashMap<usize, Option<f32>>>) -> Vec<ProcessSample> {
        let mut returned = 0u32;
        loop {
            let status = unsafe {
                NtQuerySystemInformation(
                    SYSTEM_PROCESS_INFORMATION,
                    self.buf.as_mut_ptr() as *mut c_void,
                    (self.buf.len() * 8) as u32,
                    &mut returned,
                )
            };
            if status != STATUS_INFO_LENGTH_MISMATCH {
                assert!(status >= 0, "process list query failed: {status:#x}");
                break;
            }
            // Processes can start between the sizing call and the read.
            self.buf.resize((returned as usize).div_ceil(8) + 1024, 0);
        }

        let now = Instant::now();
        // CPU time is in 100 ns units.
        let interval = self.at.map(|at| now.duration_since(at).as_secs_f64() * 1e7);
        let mut totals = HashMap::with_capacity(self.totals.len());
        let mut names = HashMap::with_capacity(self.names.len());
        let mut usage = HashMap::with_capacity(self.usage.len());
        // CPU time, as a share of every processor over the interval.
        let share = |time: f64| interval.map(|interval| (time / (interval * processors as f64) * 100.0) as f32);
        let mut programs: HashMap<String, ProcessSample> = HashMap::new();
        let mut moved: HashMap<String, i64> = HashMap::new();
        let mut offset = 0usize;
        loop {
            let record = unsafe { &*((self.buf.as_ptr() as *const u8).add(offset) as *const ProcessRecord) };
            if record.process_id != IDLE_PROCESS_ID {
                let key = (record.process_id, record.create_time);
                let time = record.user_time + record.kernel_time;
                let bytes = record.read_transfer_count + record.write_transfer_count + record.other_transfer_count;
                // A process that was not there before has spent all its time,
                // and moved all its bytes, since.
                let (time_before, bytes_before) = self.totals.get(&key).copied().unwrap_or((0, 0));
                totals.insert(key, (time, bytes));
                // A counted string, not necessarily terminated.
                let image = &record.image_name;
                let name = if image.Buffer.is_null() {
                    String::new()
                } else {
                    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(image.Buffer.0, image.Length as usize / 2) })
                };
                let name = name.trim_end_matches(".exe").to_string();
                names.insert(record.process_id, name.clone());
                if let Some(cpu) = share((time - time_before) as f64) {
                    usage.insert(record.process_id, (cpu, record.working_set_private as u64));
                }
                *moved.entry(name.clone()).or_default() += bytes - bytes_before;
                let program = programs.entry(name.clone()).or_insert_with(|| ProcessSample {
                    name,
                    cpu: 0.0,
                    mem: 0,
                    io: 0.0,
                    gpu: gpu_by_pid.map(|_| 0.0),
                });
                program.cpu += (time - time_before) as f32;
                program.mem += record.working_set_private as u64;
                // A process the counters list nothing for uses no GPU.
                let used = gpu_by_pid.map(|by_pid| by_pid.get(&record.process_id).copied().unwrap_or(Some(0.0)));
                program.gpu = program.gpu.zip(used.flatten()).map(|(total, used)| total + used);
            }
            if record.next_entry_offset == 0 {
                break;
            }
            offset += record.next_entry_offset as usize;
        }
        self.totals = totals;
        self.names = names;
        self.usage = usage;
        self.at = Some(now);

        // Without a previous sample there is no interval to take a share of.
        let Some(interval) = interval else { return Vec::new() };
        let mut all: Vec<ProcessSample> = programs
            .into_values()
            .map(|mut program| {
                program.cpu = share(program.cpu as f64).unwrap_or_default();
                program.io = moved[&program.name] as f64 / (interval / 1e7);
                program.gpu = program.gpu.map(|gpu| gpu.min(100.0));
                program
            })
            .collect();
        // The top of each ranking; a program high in several appears once.
        let mut keep = std::collections::HashSet::new();
        let rankings: [fn(&ProcessSample, &ProcessSample) -> std::cmp::Ordering; 4] = [
            |a, b| b.cpu.total_cmp(&a.cpu),
            |a, b| b.mem.cmp(&a.mem),
            |a, b| b.io.total_cmp(&a.io),
            |a, b| b.gpu.unwrap_or(-1.0).total_cmp(&a.gpu.unwrap_or(-1.0)),
        ];
        for ranking in rankings {
            all.sort_by(ranking);
            keep.extend(all.iter().take(TOP_PROCESSES).map(|program| program.name.clone()));
        }
        all.retain(|program| keep.contains(&program.name));
        all
    }
}

/// Space on every fixed drive that is mounted with a letter.
fn volumes() -> Vec<VolumeSample> {
    let mounted = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|letter| mounted & (1 << letter) != 0)
        .filter_map(|letter| {
            let name = format!("{}:", (b'A' + letter) as char);
            let root: Vec<u16> = format!("{name}\\").encode_utf16().chain([0]).collect();
            let root = PCWSTR(root.as_ptr());
            if unsafe { GetDriveTypeW(root) } != DRIVE_FIXED {
                return None;
            }
            let (mut total, mut free) = (0u64, 0u64);
            // A drive can be present but unreadable (locked BitLocker volume).
            unsafe { GetDiskFreeSpaceExW(root, None, Some(&mut total), Some(&mut free)) }.ok()?;
            Some(VolumeSample { name, used: total - free, total })
        })
        .collect()
}

/// `..._luid_0x00000000_0x00011D2B_...` → (low, high)
fn parse_luid(instance: &str) -> Option<(u32, i32)> {
    let rest = &instance[instance.find("luid_0x")? + 7..];
    let high = u32::from_str_radix(rest.get(..8)?, 16).ok()? as i32;
    let low = u32::from_str_radix(rest.get(11..19)?, 16).ok()?;
    Some((low, high))
}

/// `pid_12016_luid_...` → 12016
fn parse_pid(instance: &str) -> Option<usize> {
    let rest = instance.strip_prefix("pid_")?;
    rest[..rest.find('_')?].parse().ok()
}

/// `..._eng_3_engtype_VideoDecode` → 3
fn parse_engine(instance: &str) -> Option<u32> {
    let rest = &instance[instance.find("_eng_")? + 5..];
    rest[..rest.find('_')?].parse().ok()
}

/// `..._engtype_VideoDecode` → VideoDecode. Drivers spell kinds with spaces
/// and number repeated engines (`Compute_1`, `Video Codec 0`); numbered
/// engines are one kind, and spaces are dropped.
fn parse_kind(instance: &str) -> Option<String> {
    let kind = &instance[instance.find("_engtype_")? + 9..];
    let kind = match kind.rsplit_once(['_', ' ']) {
        Some((base, number)) if number.parse::<u32>().is_ok() => base,
        _ => kind,
    };
    Some(kind.replace(' ', ""))
}

fn enumerate_gpus(power: &GpuPower) -> (Vec<Adapter>, Vec<GpuInfo>) {
    let mut adapters = Vec::new();
    let mut infos = Vec::new();
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else { return (adapters, infos) };
    let mut index = 0;
    // Each adapter's number, as Glance has counted them since its first
    // version: those the kernel opens, whether or not they render.
    let mut slots = 0;
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(index) } {
        index += 1;
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else { continue };
        if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }
        // An adapter the kernel will not open for us (a remote session's,
        // say) is left out.
        let mut open = D3DKMT_OPENADAPTERFROMLUID { AdapterLuid: desc.AdapterLuid, hAdapter: 0 };
        if unsafe { D3DKMTOpenAdapterFromLuid(&mut open) }.is_err() {
            continue;
        }
        // A display-only adapter (a virtual display's, as Parsec's or a
        // streaming tool's) goes by the name of the GPU that draws for it,
        // and has no work of its own to show: that GPU is listed already.
        let slot = slots;
        slots += 1;
        if !renders(open.hAdapter) {
            let _ = unsafe { D3DKMTCloseAdapter(&D3DKMT_CLOSEADAPTER { hAdapter: open.hAdapter }) };
            continue;
        }
        let LUID { LowPart, HighPart } = desc.AdapterLuid;
        let power = adapter_address(open.hAdapter).and_then(|address| power.reader(address));
        adapters.push(Adapter { luid: (LowPart, HighPart), kmt_handle: open.hAdapter, power });
        let name_len = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
        infos.push(GpuInfo {
            slot,
            name: String::from_utf16_lossy(&desc.Description[..name_len]),
            mem_total: desc.DedicatedVideoMemory as u64,
            shared_total: desc.SharedSystemMemory as u64,
        });
    }
    (adapters, infos)
}

/// What the driver reports to the kernel about the adapter (the same source
/// Task Manager uses). `None` when the driver does not answer.
fn adapter_perf(kmt_handle: u32) -> Option<D3DKMT_ADAPTER_PERFDATA> {
    let mut perf = D3DKMT_ADAPTER_PERFDATA::default();
    let mut query = D3DKMT_QUERYADAPTERINFO {
        hAdapter: kmt_handle,
        Type: KMTQAITYPE_ADAPTERPERFDATA,
        pPrivateDriverData: &mut perf as *mut _ as *mut _,
        PrivateDriverDataSize: size_of::<D3DKMT_ADAPTER_PERFDATA>() as u32,
    };
    unsafe { D3DKMTQueryAdapterInfo(&mut query) }.is_ok().then_some(perf)
}

/// Whether the adapter renders, as the kernel classes it; one that does not
/// say is taken to.
fn renders(kmt_handle: u32) -> bool {
    const RENDER_SUPPORTED: u32 = 1;
    let mut kind = D3DKMT_ADAPTERTYPE::default();
    let mut query = D3DKMT_QUERYADAPTERINFO {
        hAdapter: kmt_handle,
        Type: KMTQAITYPE_ADAPTERTYPE,
        pPrivateDriverData: &mut kind as *mut _ as *mut _,
        PrivateDriverDataSize: size_of::<D3DKMT_ADAPTERTYPE>() as u32,
    };
    unsafe { D3DKMTQueryAdapterInfo(&mut query) }.is_err() || unsafe { kind.Anonymous.Value } & RENDER_SUPPORTED != 0
}

/// Where the adapter sits on the PCI bus.
fn adapter_address(kmt_handle: u32) -> Option<gpu_power::PciAddress> {
    let mut address = D3DKMT_ADAPTERADDRESS::default();
    let mut query = D3DKMT_QUERYADAPTERINFO {
        hAdapter: kmt_handle,
        Type: KMTQAITYPE_ADAPTERADDRESS,
        pPrivateDriverData: &mut address as *mut _ as *mut _,
        PrivateDriverDataSize: size_of::<D3DKMT_ADAPTERADDRESS>() as u32,
    };
    unsafe { D3DKMTQueryAdapterInfo(&mut query) }.is_ok().then_some((address.BusNumber, address.DeviceNumber, address.FunctionNumber))
}

/// Each engine's use (kind, and percent) by adapter and engine, and the
/// adapters with an instance not read.
type Engines = (HashMap<((u32, i32), u32), (String, f64)>, HashSet<(u32, i32)>);

/// Clock of the adapter's first engine (the graphics engine), in MHz. A
/// driver that gives the engine's top clock gives its clock: 0 is a GPU at
/// rest, its clock stopped, not one without a clock to read (a GPU powered
/// off does not answer at all).
fn graphics_clock(kmt_handle: u32) -> Option<f32> {
    let mut perf = D3DKMT_NODE_PERFDATA::default();
    let mut query = D3DKMT_QUERYADAPTERINFO {
        hAdapter: kmt_handle,
        Type: KMTQAITYPE_NODEPERFDATA,
        pPrivateDriverData: &mut perf as *mut _ as *mut _,
        PrivateDriverDataSize: size_of::<D3DKMT_NODE_PERFDATA>() as u32,
    };
    let answered = unsafe { D3DKMTQueryAdapterInfo(&mut query) }.is_ok();
    (answered && perf.MaxFrequency != 0).then(|| (perf.Frequency as f64 / 1e6) as f32)
}

/// Octets received and sent by each hardware network interface, by its LUID. Virtual
/// and filter interfaces carry the same traffic again and are left out.
fn net_octets() -> Option<HashMap<u64, (u64, u64)>> {
    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    unsafe { GetIfTable2(&mut table) }.ok().ok()?;
    let rows = unsafe { std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize) };
    const HARDWARE_INTERFACE: u8 = 1;
    let adapters = rows
        .iter()
        .filter(|row| row.InterfaceAndOperStatusFlags._bitfield & HARDWARE_INTERFACE != 0)
        .map(|row| (unsafe { row.InterfaceLuid.Value }, (row.InOctets, row.OutOctets)))
        .collect();
    unsafe { FreeMibTable(table as *const _) };
    Some(adapters)
}

/// The interface that traffic to the internet would leave by. Asking for the
/// route sends nothing.
fn default_interface() -> Option<NetworkInfo> {
    let mut index = 0u32;
    // 1.1.1.1, in network byte order as the API wants it.
    if unsafe { GetBestInterface(u32::from_ne_bytes([1, 1, 1, 1]), &mut index) } != ERROR_SUCCESS.0 {
        return None;
    }
    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    let mut buf: Vec<u64> = Vec::new();
    let mut size = 0u32;
    loop {
        let list = buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;
        let status = unsafe { GetAdaptersAddresses(AF_INET.0 as u32, flags, None, Some(list), &mut size) };
        if status == ERROR_BUFFER_OVERFLOW.0 {
            buf.resize((size as usize).div_ceil(8), 0);
            continue;
        }
        if status != ERROR_SUCCESS.0 {
            return None;
        }
        let mut adapter = list as *const IP_ADAPTER_ADDRESSES_LH;
        while !adapter.is_null() {
            let a = unsafe { &*adapter };
            if unsafe { a.Anonymous1.Anonymous.IfIndex } == index {
                let ipv4 = (!a.FirstUnicastAddress.is_null()).then(|| {
                    let socket = unsafe { (*a.FirstUnicastAddress).Address.lpSockaddr as *const SOCKADDR_IN };
                    let octets = unsafe { (*socket).sin_addr.S_un.S_addr }.to_ne_bytes();
                    format!("{}.{}.{}.{}", octets[0], octets[1], octets[2], octets[3])
                });
                return Some(NetworkInfo {
                    name: unsafe { a.FriendlyName.to_string() }.unwrap_or_default(),
                    model: unsafe { a.Description.to_string() }.unwrap_or_default(),
                    ipv4,
                    link_bps: a.ReceiveLinkSpeed,
                });
            }
            adapter = a.Next;
        }
        return None;
    }
}

fn performance_info() -> PERFORMANCE_INFORMATION {
    let mut info = PERFORMANCE_INFORMATION::default();
    unsafe { GetPerformanceInfo(&mut info, size_of::<PERFORMANCE_INFORMATION>() as u32) }
        .expect("GetPerformanceInfo");
    info
}

impl Sampler {
    /// The motherboard's sensors, and a laptop's fans its firmware reports.
    fn board(&mut self) -> Option<crate::reading::BoardSensors> {
        let mut board = self.super_io.as_mut().map(SuperIo::read);
        if let Some(fans) = &self.asus_fans {
            board.get_or_insert_with(Default::default).fans.extend(fans.read());
        }
        board
    }
}

/// `None` on machines without a battery.
fn battery() -> Option<BatterySample> {
    let mut status = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut status) }.ok()?;
    const NO_BATTERY: u8 = 128;
    const UNKNOWN: u8 = 255;
    if status.BatteryFlag & NO_BATTERY != 0 || status.BatteryLifePercent == UNKNOWN {
        return None;
    }
    let figures = crate::battery::read();
    Some(BatterySample {
        percent: status.BatteryLifePercent,
        charging: status.ACLineStatus == 1,
        seconds_left: (status.BatteryLifeTime != u32::MAX).then_some(status.BatteryLifeTime),
        watts: figures.watts,
        health: figures.health,
    })
}

/// A number from the machine's registry; 0 where the value is missing.
pub fn reg_dword(key: PCWSTR, value: PCWSTR) -> u32 {
    let mut data = 0u32;
    let mut size = 4u32;
    let read = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, key, value, RRF_RT_REG_DWORD, None, Some(&mut data as *mut _ as *mut _), Some(&mut size))
    };
    if read.is_ok() { data } else { 0 }
}

/// Text from the machine's registry; empty where the value is missing.
pub fn reg_string(key: PCWSTR, value: PCWSTR) -> String {
    let mut data = [0u16; 256];
    let mut size = size_of_val(&data) as u32;
    let read = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, key, value, RRF_RT_REG_SZ, None, Some(data.as_mut_ptr() as *mut _), Some(&mut size))
    };
    if read.is_err() {
        return String::new();
    }
    let len = data.iter().position(|&c| c == 0).unwrap_or(data.len());
    String::from_utf16_lossy(&data[..len]).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_every_processors_time() {
        use windows::Win32::Foundation::FILETIME;
        use windows::Win32::System::Threading::GetSystemTimes;
        let (idle, busy) = system_times().expect("processor times");
        let (mut i, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
        unsafe { GetSystemTimes(Some(&mut i), Some(&mut k), Some(&mut u)) }.unwrap();
        let ticks = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
        // On a machine of one processor group the two accounts agree, but
        // for the moment between the two calls (a second at most). Beyond
        // one group, GetSystemTimes sees only its own, and there is nothing
        // to compare with.
        if unsafe { windows::Win32::System::Threading::GetActiveProcessorGroupCount() } != 1 {
            return;
        }
        let total = idle + busy;
        let system = ticks(k) + ticks(u);
        let cpus = std::thread::available_parallelism().unwrap().get() as u64;
        assert!(total.abs_diff(system) < cpus * 10_000_000, "{total} vs {system}");
    }

    #[test]
    fn averages_each_kind_of_core() {
        // Two big cores (class 1), two little (class 0).
        let classes: HashMap<String, u8> = [("0,0", 1), ("0,1", 1), ("0,2", 0), ("0,3", 0)].map(|(n, c)| (n.to_string(), c)).into_iter().collect();
        let kinds = average_by_kind(&classes, [(1, 5.0), (1, 4.6), (0, 3.6), (0, 3.2)].into_iter());
        assert_eq!(kinds.len(), 2);
        assert!((kinds[0].unwrap() - 4.8).abs() < 1e-5 && (kinds[1].unwrap() - 3.4).abs() < 1e-5);
        // A kind none of whose clocks was read: none, in its place.
        assert_eq!(average_by_kind(&classes, [(1, 5.0)].into_iter())[1], None);
    }

    #[test]
    #[ignore = "reads this machine's processors, its first half taken for one kind of core and the rest another; run with --ignored"]
    fn reads_kinds_of_cores() {
        let threads = processor_groups().1;
        let classes: HashMap<String, u8> = (0..threads).map(|i| (format!("0,{i}"), u8::from(i < threads / 2))).collect();
        let mut query = PDH_HQUERY::default();
        let _ = unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) };
        let add = |path| {
            let mut counter = PDH_HCOUNTER::default();
            (unsafe { PdhAddEnglishCounterW(query, path, 0, &mut counter) } == ERROR_SUCCESS.0).then_some(counter)
        };
        let kinds = Cores { classes: Some(classes), performance: add(w!(r"\Processor Information(*)\% Processor Performance")), nominal: add(w!(r"\Processor Information(*)\Processor Frequency")) };
        unsafe { PdhCollectQueryData(query) };
        std::thread::sleep(std::time::Duration::from_millis(1000));
        unsafe { PdhCollectQueryData(query) };
        let (starts, processors) = processor_groups();
        let (read, threads) = kinds.read(&starts, processors, &mut Vec::new());
        println!("{read:?} {threads:?}");
        assert!(read.len() == 2 && read.iter().all(|ghz| ghz.is_some_and(|ghz| (0.4..8.0).contains(&ghz))));
        assert!(threads.len() == processors && threads.iter().all(Option::is_some));
        assert_eq!(Cores::kinds_of_this_machine().map(|classes| classes.len()), None, "this machine's cores are all of one kind");
    }

    #[test]
    fn parses_gpu_instance_names() {
        let engine = "pid_12016_luid_0x00000000_0x00011D2B_phys_0_eng_3_engtype_VideoDecode";
        assert_eq!(parse_luid(engine), Some((0x11D2B, 0)));
        assert_eq!(parse_engine(engine), Some(3));
        assert_eq!(parse_kind(engine).as_deref(), Some("VideoDecode"));
        assert_eq!(parse_kind("pid_4_luid_0x0_0x1_phys_0_eng_9_engtype_Compute_1").as_deref(), Some("Compute"));
        assert_eq!(parse_kind("luid_0x0_0x1_phys_0_eng_2_engtype_Video Codec 0").as_deref(), Some("VideoCodec"));
        assert_eq!(parse_luid("luid_0x00000001_0x0000C6B4_phys_0"), Some((0xC6B4, 1)));
        assert_eq!(parse_engine("luid_0x00000001_0x0000C6B4_phys_0"), None);
    }

    #[test]
    #[ignore = "opens the PawnIO driver's readers when elevated; run with --ignored"]
    fn samples_this_machine() {
        let mut sampler = Sampler::new();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let sample = sampler.sample(true, true, false).expect("a valid sample after one interval");
        println!("{}", serde_json::to_string_pretty(&sampler.info).unwrap());
        println!("{}", serde_json::to_string_pretty(&sample).unwrap());
        assert_eq!(sample.threads.len(), sampler.info.threads);
        // Every processor's thread read, each in its place.
        assert!(sample.threads.iter().all(Option::is_some));
        assert!(sample.cpu.is_some_and(|cpu| (0.0..=100.0).contains(&cpu)));
        assert!(sample.memory.used > 0 && sample.memory.used < sampler.info.mem_total);
        assert!(sample.memory.committed <= sample.memory.commit_limit);
        assert_eq!(sample.gpus.len(), sampler.info.gpus.len());
        assert!(sample.processes.len() >= TOP_PROCESSES);
        assert!(sample.processes.iter().any(|p| p.io > 0.0));
        assert!(sample.volumes.iter().any(|volume| volume.name == "C:"));
        assert!(sample.system.uptime_s > 0 && sample.system.processes > 0);
        // The last start the system recorded, no earlier than the tick
        // count's start from off.
        let since = last_boot().expect("the last start in the event log");
        assert!(since <= unsafe { GetTickCount64() } + 1000, "{since} ms ago, the tick count {}", unsafe { GetTickCount64() });
        assert!(sample.system.uptime_s.abs_diff(since / 1000) <= 2);
    }
}
