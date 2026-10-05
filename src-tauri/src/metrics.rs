//! System metrics: one PDH query for every rate counter, plus the direct APIs
//! PDH has no counter for (memory, NIC octets, GPU sensors, process list).

use std::collections::HashMap;
use std::ffi::c_void;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use windows::core::{w, PCWSTR};
use windows::Wdk::Graphics::Direct3D::{
    D3DKMTOpenAdapterFromLuid, D3DKMTQueryAdapterInfo, D3DKMT_ADAPTER_PERFDATA,
    D3DKMT_NODE_PERFDATA, D3DKMT_OPENADAPTERFROMLUID, D3DKMT_QUERYADAPTERINFO,
    KMTQAITYPE_ADAPTERPERFDATA, KMTQAITYPE_NODEPERFDATA,
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

#[derive(Clone, Serialize)]
pub struct StaticInfo {
    pub cpu_name: String,
    pub threads: usize,
    pub mem_total: u64,
    pub gpus: Vec<GpuInfo>,
}

#[derive(Clone, Serialize)]
pub struct GpuInfo {
    pub name: String,
    pub mem_total: u64,
    pub shared_total: u64,
}

#[derive(Clone, Serialize)]
pub struct Sample {
    /// Milliseconds since the Unix epoch.
    pub t: u64,
    pub cpu: f32,
    pub threads: Vec<f32>,
    pub ghz: f32,
    pub memory: MemorySample,
    pub gpus: Vec<GpuSample>,
    pub net_down: f64,
    pub net_up: f64,
    /// Bytes moved since the interfaces came up.
    pub net_total_down: u64,
    pub net_total_up: u64,
    pub network: Option<NetworkInfo>,
    pub disk_read: f64,
    pub disk_write: f64,
    /// Percent of the time the disks were busy.
    pub disk_active: f32,
    pub volumes: Vec<VolumeSample>,
    /// The busiest programs by CPU, and by memory.
    pub by_cpu: Vec<ProcessSample>,
    pub by_memory: Vec<ProcessSample>,
    pub system: SystemSample,
    pub battery: Option<BatterySample>,
}

#[derive(Clone, Serialize)]
pub struct MemorySample {
    pub used: u64,
    pub committed: u64,
    pub commit_limit: u64,
    pub cached: u64,
}

#[derive(Clone, Serialize)]
pub struct GpuSample {
    pub usage: f32,
    /// Busiest engine of each kind (3D, Copy, VideoDecode, …).
    pub engines: Vec<(String, f32)>,
    pub mem_used: u64,
    pub shared_used: u64,
    pub temp: Option<f32>,
    pub clock_mhz: Option<f32>,
    pub fan_rpm: Option<u32>,
}

/// One program: every process sharing an executable name, added together.
#[derive(Clone, Serialize)]
pub struct ProcessSample {
    pub name: String,
    /// Percent of the whole machine.
    pub cpu: f32,
    pub mem: u64,
}

#[derive(Clone, Serialize)]
pub struct VolumeSample {
    pub name: String,
    pub used: u64,
    pub total: u64,
}

/// The interface the default route goes through.
#[derive(Clone, Serialize)]
pub struct NetworkInfo {
    pub name: String,
    pub ipv4: Option<String>,
    pub link_bps: u64,
}

#[derive(Clone, Serialize)]
pub struct SystemSample {
    pub uptime_s: u64,
    pub processes: u32,
    pub threads: u32,
    pub handles: u32,
}

#[derive(Clone, Serialize)]
pub struct BatterySample {
    pub percent: u8,
    pub charging: bool,
    pub seconds_left: Option<u32>,
}

impl Sample {
    /// What decides how many rows the panel has, and so how tall it is.
    pub fn layout_rows(&self) -> Vec<String> {
        let mut rows: Vec<String> = self.volumes.iter().map(|volume| volume.name.clone()).collect();
        rows.push(format!("{} {}", self.by_cpu.len(), self.by_memory.len()));
        rows.push(format!("battery {}", self.battery.is_some()));
        rows.push(format!("network {}", self.network.as_ref().is_some_and(|n| n.ipv4.is_some())));
        for gpu in &self.gpus {
            rows.extend(gpu.engines.iter().map(|(kind, _)| kind.clone()));
            rows.push(format!("{:?} {:?} {:?}", gpu.temp.is_some(), gpu.clock_mhz.is_some(), gpu.fan_rpm.is_some()));
        }
        rows
    }
}

/// PDH_FMT_DOUBLE with PDH_FMT_NOCAP100, which lets a percentage counter
/// report above 100 (turbo frequencies do).
const PDH_FMT_DOUBLE_NOCAP100: PDH_FMT = PDH_FMT(PDH_FMT_DOUBLE.0 | 0x8000);

/// How many programs each ranking lists.
const TOP_PROCESSES: usize = 8;
const DRIVE_FIXED: u32 = 3;

struct Adapter {
    luid: (u32, i32),
    kmt_handle: u32,
}

/// What is read less often than every sample (see `Sampler::sample`).
#[derive(Default)]
struct Slow {
    by_cpu: Vec<ProcessSample>,
    by_memory: Vec<ProcessSample>,
    network: Option<NetworkInfo>,
}

pub struct Sampler {
    query: PDH_HQUERY,
    cpu_time: PDH_HCOUNTER,
    cpu_performance: PDH_HCOUNTER,
    gpu_engine: PDH_HCOUNTER,
    gpu_dedicated: PDH_HCOUNTER,
    gpu_shared: PDH_HCOUNTER,
    disk_read: PDH_HCOUNTER,
    disk_write: PDH_HCOUNTER,
    disk_idle: PDH_HCOUNTER,
    processes: ProcessTable,
    slow: Slow,
    base_mhz: f64,
    adapters: Vec<Adapter>,
    net_prev: (u64, u64, Instant),
    buf: Vec<u64>,
    pub info: StaticInfo,
}

// PDH handles are plain pointers owned by this struct; it lives on one thread at a time.
unsafe impl Send for Sampler {}

impl Sampler {
    pub fn new() -> Self {
        let mut query = PDH_HQUERY::default();
        pdh(unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) });
        let add = |path: PCWSTR| {
            let mut counter = PDH_HCOUNTER::default();
            pdh(unsafe { PdhAddEnglishCounterW(query, path, 0, &mut counter) });
            counter
        };
        let cpu_time = add(w!(r"\Processor Information(*)\% Processor Time"));
        let cpu_performance = add(w!(r"\Processor Information(_Total)\% Processor Performance"));
        let gpu_engine = add(w!(r"\GPU Engine(*)\Utilization Percentage"));
        let gpu_dedicated = add(w!(r"\GPU Adapter Memory(*)\Dedicated Usage"));
        let gpu_shared = add(w!(r"\GPU Adapter Memory(*)\Shared Usage"));
        let disk_read = add(w!(r"\PhysicalDisk(_Total)\Disk Read Bytes/sec"));
        let disk_write = add(w!(r"\PhysicalDisk(_Total)\Disk Write Bytes/sec"));
        let disk_idle = add(w!(r"\PhysicalDisk(_Total)\% Idle Time"));
        // Rate counters need a first collection to diff against.
        unsafe { PdhCollectQueryData(query) };
        let mut processes = ProcessTable::default();
        processes.sample(1);

        let (adapters, gpus) = enumerate_gpus();
        let (down, up) = net_octets();
        let cpu_key = w!(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0");
        let info = StaticInfo {
            cpu_name: reg_string(cpu_key, w!("ProcessorNameString")),
            threads: std::thread::available_parallelism().map_or(1, |n| n.get()),
            mem_total: performance_info().PhysicalTotal as u64 * performance_info().PageSize as u64,
            gpus,
        };
        Sampler {
            query,
            cpu_time,
            cpu_performance,
            gpu_engine,
            gpu_dedicated,
            gpu_shared,
            disk_read,
            disk_write,
            disk_idle,
            processes,
            slow: Slow::default(),
            base_mhz: reg_dword(cpu_key, w!("~MHz")) as f64,
            adapters,
            net_prev: (down, up, Instant::now()),
            buf: Vec::new(),
            info,
        }
    }

    /// Takes one sample. Reading the process list and the network adapters is
    /// the expensive part and nothing charts it, so the caller says when it is
    /// worth refreshing. Returns `None` when PDH has no valid data for this
    /// interval (it reports that for a tick now and then, e.g. after resume).
    pub fn sample(&mut self, refresh_slow: bool) -> Option<Sample> {
        if unsafe { PdhCollectQueryData(self.query) } != ERROR_SUCCESS.0 {
            return None;
        }

        let mut cpu = None;
        let mut threads: Vec<((u32, u32), f32)> = Vec::new();
        for (name, value) in read_array(self.cpu_time, &mut self.buf)? {
            if name == "_Total" {
                cpu = Some(value as f32);
            } else if let Some((group, index)) = name.split_once(',') {
                if let (Ok(group), Ok(index)) = (group.parse(), index.parse()) {
                    threads.push(((group, index), value as f32));
                }
            }
        }
        threads.sort_by_key(|(key, _)| *key);
        let performance = read_scalar(self.cpu_performance, PDH_FMT_DOUBLE_NOCAP100)?;

        let gpus = self.sample_gpus()?;

        if refresh_slow {
            (self.slow.by_cpu, self.slow.by_memory) = self.processes.sample(self.info.threads);
            self.slow.network = default_interface();
        }

        let (down, up) = net_octets();
        let now = Instant::now();
        let (prev_down, prev_up, prev_at) = self.net_prev;
        let dt = now.duration_since(prev_at).as_secs_f64();
        self.net_prev = (down, up, now);

        let perf = performance_info();
        let page = perf.PageSize as u64;
        Some(Sample {
            t: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
            cpu: cpu?,
            threads: threads.into_iter().map(|(_, value)| value).collect(),
            ghz: (self.base_mhz * performance / 100_000.0) as f32,
            memory: MemorySample {
                used: (perf.PhysicalTotal - perf.PhysicalAvailable) as u64 * page,
                committed: perf.CommitTotal as u64 * page,
                commit_limit: perf.CommitLimit as u64 * page,
                cached: perf.SystemCache as u64 * page,
            },
            gpus,
            net_down: down.saturating_sub(prev_down) as f64 / dt,
            net_up: up.saturating_sub(prev_up) as f64 / dt,
            net_total_down: down,
            net_total_up: up,
            network: self.slow.network.clone(),
            disk_read: read_scalar(self.disk_read, PDH_FMT_DOUBLE)?,
            disk_write: read_scalar(self.disk_write, PDH_FMT_DOUBLE)?,
            disk_active: (100.0 - read_scalar(self.disk_idle, PDH_FMT_DOUBLE)?).clamp(0.0, 100.0) as f32,
            volumes: volumes(),
            by_cpu: self.slow.by_cpu.clone(),
            by_memory: self.slow.by_memory.clone(),
            system: SystemSample {
                uptime_s: unsafe { GetTickCount64() } / 1000,
                processes: perf.ProcessCount,
                threads: perf.ThreadCount,
                handles: perf.HandleCount,
            },
            battery: battery(),
        })
    }

    fn sample_gpus(&mut self) -> Option<Vec<GpuSample>> {
        // An engine's utilisation is the sum over the processes using it; a
        // kind of engine is as busy as its busiest engine; an adapter is as
        // busy as its busiest kind.
        let mut engines: HashMap<((u32, i32), u32), (String, f64)> = HashMap::new();
        for (name, value) in read_array(self.gpu_engine, &mut self.buf)? {
            if let (Some(luid), Some(engine), Some(kind)) = (parse_luid(&name), parse_engine(&name), parse_kind(&name)) {
                engines.entry((luid, engine)).or_insert_with(|| (kind, 0.0)).1 += value;
            }
        }
        let mut memory = |counter| -> Option<HashMap<(u32, i32), u64>> {
            let mut per_adapter = HashMap::new();
            for (name, value) in read_array(counter, &mut self.buf)? {
                if let Some(luid) = parse_luid(&name) {
                    *per_adapter.entry(luid).or_default() += value as u64;
                }
            }
            Some(per_adapter)
        };
        let dedicated = memory(self.gpu_dedicated)?;
        let shared = memory(self.gpu_shared)?;

        Some(
            self.adapters
                .iter()
                .map(|adapter| {
                    let mut kinds: Vec<(String, f32)> = Vec::new();
                    for ((luid, _), (kind, usage)) in &engines {
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
                    let perf = adapter_perf(adapter.kmt_handle);
                    GpuSample {
                        usage: kinds.iter().map(|(_, usage)| *usage).fold(0.0, f32::max),
                        engines: kinds,
                        mem_used: dedicated.get(&adapter.luid).copied().unwrap_or(0),
                        shared_used: shared.get(&adapter.luid).copied().unwrap_or(0),
                        temp: perf.and_then(|p| (p.Temperature != 0).then(|| p.Temperature as f32 / 10.0)),
                        fan_rpm: perf.and_then(|p| (p.FanRPM != 0).then_some(p.FanRPM)),
                        clock_mhz: graphics_clock(adapter.kmt_handle),
                    }
                })
                .collect(),
        )
    }
}

fn pdh(status: u32) {
    assert_eq!(status, ERROR_SUCCESS.0, "PDH call failed: {status:#x}");
}

fn valid(status: u32) -> bool {
    status == PDH_CSTATUS_VALID_DATA || status == PDH_CSTATUS_NEW_DATA
}

fn read_scalar(counter: PDH_HCOUNTER, format: PDH_FMT) -> Option<f64> {
    let mut value = PDH_FMT_COUNTERVALUE::default();
    let status = unsafe { PdhGetFormattedCounterValue(counter, format, None, &mut value) };
    (status == ERROR_SUCCESS.0 && valid(value.CStatus)).then(|| unsafe { value.Anonymous.doubleValue })
}

/// Reads every instance of a wildcard counter as (instance name, value).
fn read_array(counter: PDH_HCOUNTER, buf: &mut Vec<u64>) -> Option<Vec<(String, f64)>> {
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
                .filter(|item| valid(item.FmtValue.CStatus))
                .map(|item| unsafe {
                    (item.szName.to_string().unwrap_or_default(), item.FmtValue.Anonymous.doubleValue)
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
}

const SYSTEM_PROCESS_INFORMATION: u32 = 5;
const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC0000004u32 as i32;
const IDLE_PROCESS_ID: usize = 0;

#[link(name = "ntdll", kind = "raw-dylib")]
extern "system" {
    fn NtQuerySystemInformation(class: u32, info: *mut c_void, length: u32, returned: *mut u32) -> i32;
}

/// CPU time of every process at the previous sample, to diff against.
#[derive(Default)]
struct ProcessTable {
    /// Keyed by (process id, creation time): ids are reused.
    cpu_time: HashMap<(usize, i64), i64>,
    at: Option<Instant>,
    buf: Vec<u64>,
}

impl ProcessTable {
    /// The busiest programs since the previous call, by CPU and by memory.
    /// Processes sharing an executable name are added together.
    fn sample(&mut self, processors: usize) -> (Vec<ProcessSample>, Vec<ProcessSample>) {
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
        let mut cpu_time = HashMap::with_capacity(self.cpu_time.len());
        let mut programs: HashMap<String, (i64, u64)> = HashMap::new();
        let mut offset = 0usize;
        loop {
            let record = unsafe { &*((self.buf.as_ptr() as *const u8).add(offset) as *const ProcessRecord) };
            if record.process_id != IDLE_PROCESS_ID {
                let key = (record.process_id, record.create_time);
                let total = record.user_time + record.kernel_time;
                // A process that was not there before has spent all its time since.
                let spent = total - self.cpu_time.get(&key).copied().unwrap_or(0);
                cpu_time.insert(key, total);
                let name = unsafe { record.image_name.Buffer.to_string() }.unwrap_or_default();
                let program = programs.entry(name.trim_end_matches(".exe").to_string()).or_default();
                program.0 += spent;
                program.1 += record.working_set_private as u64;
            }
            if record.next_entry_offset == 0 {
                break;
            }
            offset += record.next_entry_offset as usize;
        }
        self.cpu_time = cpu_time;
        self.at = Some(now);

        // Without a previous sample there is no interval to take a share of.
        let Some(interval) = interval else { return (Vec::new(), Vec::new()) };
        let all: Vec<ProcessSample> = programs
            .into_iter()
            .map(|(name, (spent, mem))| ProcessSample {
                name,
                cpu: (spent as f64 / (interval * processors as f64) * 100.0) as f32,
                mem,
            })
            .collect();
        let mut by_cpu = all.clone();
        by_cpu.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.mem.cmp(&a.mem)));
        by_cpu.truncate(TOP_PROCESSES);
        let mut by_memory = all;
        by_memory.sort_by(|a, b| b.mem.cmp(&a.mem));
        by_memory.truncate(TOP_PROCESSES);
        (by_cpu, by_memory)
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

fn enumerate_gpus() -> (Vec<Adapter>, Vec<GpuInfo>) {
    let mut adapters = Vec::new();
    let mut infos = Vec::new();
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.expect("DXGI factory");
    let mut index = 0;
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(index) } {
        index += 1;
        let desc = unsafe { adapter.GetDesc1() }.expect("adapter description");
        if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }
        let mut open = D3DKMT_OPENADAPTERFROMLUID { AdapterLuid: desc.AdapterLuid, hAdapter: 0 };
        unsafe { D3DKMTOpenAdapterFromLuid(&mut open) }.ok().expect("open adapter");
        let LUID { LowPart, HighPart } = desc.AdapterLuid;
        adapters.push(Adapter { luid: (LowPart, HighPart), kmt_handle: open.hAdapter });
        let name_len = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
        infos.push(GpuInfo {
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

/// Clock of the adapter's first engine (the graphics engine), in MHz.
fn graphics_clock(kmt_handle: u32) -> Option<f32> {
    let mut perf = D3DKMT_NODE_PERFDATA::default();
    let mut query = D3DKMT_QUERYADAPTERINFO {
        hAdapter: kmt_handle,
        Type: KMTQAITYPE_NODEPERFDATA,
        pPrivateDriverData: &mut perf as *mut _ as *mut _,
        PrivateDriverDataSize: size_of::<D3DKMT_NODE_PERFDATA>() as u32,
    };
    let answered = unsafe { D3DKMTQueryAdapterInfo(&mut query) }.is_ok();
    (answered && perf.Frequency != 0).then(|| (perf.Frequency as f64 / 1e6) as f32)
}

/// Total octets (received, sent) across hardware network interfaces. Virtual
/// and filter interfaces carry the same traffic again and are left out.
fn net_octets() -> (u64, u64) {
    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    unsafe { GetIfTable2(&mut table) }.ok().expect("GetIfTable2");
    let rows = unsafe { std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize) };
    const HARDWARE_INTERFACE: u8 = 1;
    let totals = rows
        .iter()
        .filter(|row| row.InterfaceAndOperStatusFlags._bitfield & HARDWARE_INTERFACE != 0)
        .fold((0, 0), |(down, up), row| (down + row.InOctets, up + row.OutOctets));
    unsafe { FreeMibTable(table as *const _) };
    totals
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

/// `None` on machines without a battery.
fn battery() -> Option<BatterySample> {
    let mut status = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut status) }.ok()?;
    const NO_BATTERY: u8 = 128;
    const UNKNOWN: u8 = 255;
    if status.BatteryFlag & NO_BATTERY != 0 || status.BatteryLifePercent == UNKNOWN {
        return None;
    }
    Some(BatterySample {
        percent: status.BatteryLifePercent,
        charging: status.ACLineStatus == 1,
        seconds_left: (status.BatteryLifeTime != u32::MAX).then_some(status.BatteryLifeTime),
    })
}

fn reg_dword(key: PCWSTR, value: PCWSTR) -> u32 {
    let mut data = 0u32;
    let mut size = 4u32;
    unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, key, value, RRF_RT_REG_DWORD, None, Some(&mut data as *mut _ as *mut _), Some(&mut size))
    }
    .ok()
    .expect("registry read");
    data
}

fn reg_string(key: PCWSTR, value: PCWSTR) -> String {
    let mut data = [0u16; 256];
    let mut size = size_of_val(&data) as u32;
    unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, key, value, RRF_RT_REG_SZ, None, Some(data.as_mut_ptr() as *mut _), Some(&mut size))
    }
    .ok()
    .expect("registry read");
    let len = data.iter().position(|&c| c == 0).unwrap_or(data.len());
    String::from_utf16_lossy(&data[..len]).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn samples_this_machine() {
        let mut sampler = Sampler::new();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let sample = sampler.sample(true).expect("a valid sample after one interval");
        println!("{}", serde_json::to_string_pretty(&sampler.info).unwrap());
        println!("{}", serde_json::to_string_pretty(&sample).unwrap());
        assert_eq!(sample.threads.len(), sampler.info.threads);
        assert!((0.0..=100.0).contains(&sample.cpu));
        assert!(sample.memory.used > 0 && sample.memory.used < sampler.info.mem_total);
        assert!(sample.memory.committed <= sample.memory.commit_limit);
        assert_eq!(sample.gpus.len(), sampler.info.gpus.len());
        assert_eq!(sample.by_cpu.len(), TOP_PROCESSES);
        assert!(sample.by_memory.windows(2).all(|pair| pair[0].mem >= pair[1].mem));
        assert!(sample.volumes.iter().any(|volume| volume.name == "C:"));
        assert!(sample.system.uptime_s > 0 && sample.system.processes > 0);
    }
}
