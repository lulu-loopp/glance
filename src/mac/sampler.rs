//! macOS readings: the kernel's counters through sysctl, Mach and libproc,
//! the drives' through IOKit. Rates and percentages come from the change
//! between two samples.

use std::collections::HashMap;
use std::ffi::{c_void, CStr, CString};
use std::mem::{size_of, zeroed};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::hid::Sensors;
use super::iokit;
use crate::reading::{CpuSensors, DriveTemperature, GpuInfo, GpuSample, MemorySample, NetworkInfo, ProcessSample, Sample, StaticInfo, SystemSample, VolumeSample};

// The Mach calls libc no longer recommends and mach2 does not have.
extern "C" {
    fn host_processor_info(
        host: mach2::mach_types::host_t,
        flavor: i32,
        count: *mut u32,
        info: *mut *mut i32,
        info_count: *mut u32,
    ) -> mach2::kern_return::kern_return_t;
    fn host_statistics64(host: mach2::mach_types::host_t, flavor: i32, info: *mut i32, count: *mut u32) -> mach2::kern_return::kern_return_t;
}

/// processor_info's PROCESSOR_CPU_LOAD_INFO, and host_statistics64's
/// HOST_VM_INFO64.
const PROCESSOR_CPU_LOAD_INFO: i32 = 2;
const HOST_VM_INFO64: i32 = 4;

/// A core's ticks in each state: user, system, idle, nice.
#[repr(C)]
struct CpuLoad {
    ticks: [u32; 4],
}

/// The busiest programs kept by each measure (CPU, memory, I/O).
const TOP_PROCESSES: usize = 40;

/// What the next sample is measured against.
struct Previous {
    at: Instant,
    /// Each core's busy and total ticks.
    cores: Vec<(u64, u64)>,
    /// Bytes in and out over the physical interfaces.
    net: (u64, u64),
    /// Bytes read and written by the drives.
    disk: (u64, u64),
    /// Each process's CPU time (ns) and bytes moved, by its id.
    processes: HashMap<i32, (u64, u64)>,
}

pub struct Sampler {
    pub info: StaticInfo,
    previous: Previous,
    /// Mach absolute time units to nanoseconds: numerator, denominator.
    timebase: (u64, u64),
    /// The temperature sensors, where the system offers them.
    sensors: Option<Sensors>,
}

impl Sampler {
    pub fn new() -> Self {
        let mut timebase = mach2::mach_time::mach_timebase_info { numer: 1, denom: 1 };
        unsafe { mach2::mach_time::mach_timebase_info(&mut timebase) };
        let threads = sysctl_number("hw.logicalcpu").unwrap_or(1) as usize;
        let primary = primary_interface();
        let info = StaticInfo {
            cpu_name: sysctl_string("machdep.cpu.brand_string").unwrap_or_default(),
            memory_modules: None,
            drives: drive_models(),
            network_adapter: primary.as_ref().map(|adapter| adapter.model.clone()),
            board: sysctl_string("hw.model").unwrap_or_default(),
            threads,
            mem_total: sysctl_number("hw.memsize").unwrap_or(0),
            gpus: graphics().iter().map(|(info, _)| info.clone()).collect(),
            found: Vec::new(),
        };
        let mut sampler = Sampler {
            info,
            previous: Previous { at: Instant::now(), cores: Vec::new(), net: (0, 0), disk: (0, 0), processes: HashMap::new() },
            timebase: (timebase.numer as u64, timebase.denom.max(1) as u64),
            sensors: Sensors::open(),
        };
        sampler.info.found = vec![format!(
            "Temperature sensors: {}",
            sampler.sensors.as_ref().map_or_else(|| "not found".to_string(), |sensors| format!("{} (HID)", sensors.read().len()))
        )];
        // A first look, for the first sample's rates to be measured against.
        sampler.sample();
        sampler
    }

    /// One sample, its rates since the last.
    pub fn sample(&mut self) -> Sample {
        let now = Instant::now();
        let seconds = now.duration_since(self.previous.at).as_secs_f64().max(1e-3);

        let cores = core_ticks();
        let threads: Vec<f32> = cores
            .iter()
            .enumerate()
            .map(|(i, (busy, total))| {
                let (was_busy, was_total) = self.previous.cores.get(i).copied().unwrap_or((0, 0));
                let span = total.saturating_sub(was_total);
                if span == 0 { 0.0 } else { (busy.saturating_sub(was_busy) as f64 / span as f64 * 100.0) as f32 }
            })
            .collect();
        let cpu = if threads.is_empty() { 0.0 } else { threads.iter().sum::<f32>() / threads.len() as f32 };

        let net = interface_bytes();
        let rate = |now: u64, was: u64| now.saturating_sub(was) as f64 / seconds;
        let (net_down, net_up) = (rate(net.0, self.previous.net.0), rate(net.1, self.previous.net.1));
        let disk = drive_bytes();
        let (disk_read, disk_write) = (rate(disk.0, self.previous.disk.0), rate(disk.1, self.previous.disk.1));

        let (processes, counted, times) = self.processes(seconds);
        let temperatures = self.sensors.as_ref().map_or_else(Vec::new, Sensors::read);

        let sample = Sample {
            t: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64),
            cpu,
            threads,
            ghz: None,
            memory: memory(self.info.mem_total),
            gpus: graphics().into_iter().map(|(_, reading)| reading).collect(),
            net_down,
            net_up,
            net_total_down: net.0,
            net_total_up: net.1,
            network: primary_interface(),
            disk_read,
            disk_write,
            disk_active: 0.0,
            volumes: volumes(),
            processes,
            system: SystemSample { uptime_s: uptime(), processes: counted, threads: 0, handles: 0 },
            battery: None,
            cpu_sensors: hottest(&temperatures, "PMU tdie").map(|temp| CpuSensors { temp: Some(temp), ccds: Vec::new(), power: None }),
            board: None,
            // The internal SSD's, by its NAND channels' sensors.
            drive_temps: hottest(&temperatures, "NAND")
                .map(|celsius| DriveTemperature { name: self.info.drives.first().cloned().unwrap_or_default(), celsius })
                .into_iter()
                .collect(),
            dimm_temps: Vec::new(),
        };
        self.previous = Previous { at: now, cores, net, disk, processes: times };
        sample
    }

    /// The busiest programs (each a name's processes together), how many
    /// processes there are, and each one's CPU time and bytes moved so far.
    fn processes(&self, seconds: f64) -> (Vec<ProcessSample>, u32, HashMap<i32, (u64, u64)>) {
        let pids = all_pids();
        let cores = self.info.threads.max(1) as f64;
        let mut times = HashMap::with_capacity(pids.len());
        let mut programs: HashMap<String, ProcessSample> = HashMap::new();
        for pid in &pids {
            // Others' processes (root's, other users') are not ours to read.
            let Some(usage) = rusage(*pid) else { continue };
            let cpu_ns = (usage.ri_user_time + usage.ri_system_time) * self.timebase.0 / self.timebase.1;
            let moved = usage.ri_diskio_bytesread + usage.ri_diskio_byteswritten;
            times.insert(*pid, (cpu_ns, moved));
            let (cpu, io) = match self.previous.processes.get(pid) {
                Some((was_ns, was_moved)) => (
                    cpu_ns.saturating_sub(*was_ns) as f64 / 1e9 / seconds / cores * 100.0,
                    moved.saturating_sub(*was_moved) as f64 / seconds,
                ),
                None => (0.0, 0.0),
            };
            let name = process_name(*pid);
            let program = programs.entry(name.clone()).or_insert(ProcessSample { name, cpu: 0.0, mem: 0, io: 0.0, gpu: 0.0 });
            program.cpu += cpu as f32;
            program.mem += usage.ri_phys_footprint;
            program.io += io;
        }
        // The busiest by each measure, together.
        let all: Vec<ProcessSample> = programs.into_values().collect();
        let mut kept: Vec<usize> = Vec::new();
        let mut by = |key: &dyn Fn(&ProcessSample) -> f64| {
            let mut order: Vec<usize> = (0..all.len()).collect();
            order.sort_by(|a, b| key(&all[*b]).total_cmp(&key(&all[*a])));
            kept.extend(order.into_iter().take(TOP_PROCESSES));
        };
        by(&|p| p.cpu as f64);
        by(&|p| p.mem as f64);
        by(&|p| p.io);
        kept.sort_unstable();
        kept.dedup();
        let busiest = kept.into_iter().map(|i| all[i].clone()).collect();
        (busiest, pids.len() as u32, times)
    }
}

/// Prints the static information, then `count` samples a second apart, as
/// JSON.
pub fn print(count: usize) {
    let mut sampler = Sampler::new();
    println!("{}", serde_json::to_string_pretty(&sampler.info).unwrap());
    for _ in 0..count {
        std::thread::sleep(std::time::Duration::from_secs(1));
        println!("{}", serde_json::to_string_pretty(&sampler.sample()).unwrap());
    }
}

/// The hottest of the sensors whose names start with `prefix`. The chip's
/// is the hottest of its die's ("PMU tdie"), as a desktop CPU reports its
/// hottest point.
fn hottest(temperatures: &[(&str, f32)], prefix: &str) -> Option<f32> {
    temperatures.iter().filter(|(name, _)| name.starts_with(prefix)).map(|(_, temp)| *temp).reduce(f32::max)
}

fn sysctl_string(name: &str) -> Option<String> {
    let name = CString::new(name).ok()?;
    let mut size = 0usize;
    unsafe {
        if libc::sysctlbyname(name.as_ptr(), std::ptr::null_mut(), &mut size, std::ptr::null_mut(), 0) != 0 {
            return None;
        }
        let mut buffer = vec![0u8; size];
        if libc::sysctlbyname(name.as_ptr(), buffer.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0) != 0 {
            return None;
        }
        Some(CStr::from_bytes_until_nul(&buffer).ok()?.to_string_lossy().into_owned())
    }
}

fn sysctl_number(name: &str) -> Option<u64> {
    let name = CString::new(name).ok()?;
    let mut value = 0u64;
    let mut size = size_of::<u64>();
    let read = unsafe { libc::sysctlbyname(name.as_ptr(), (&mut value as *mut u64).cast(), &mut size, std::ptr::null_mut(), 0) };
    // Some are 32-bit: the low bytes hold them, the rest stay zero.
    (read == 0).then_some(value)
}

/// Each core's busy (user, system and nice) and total ticks.
fn core_ticks() -> Vec<(u64, u64)> {
    let mut count = 0u32;
    let mut info: *mut i32 = std::ptr::null_mut();
    let mut info_count = 0u32;
    let host = unsafe { mach2::mach_init::mach_host_self() };
    let status = unsafe { host_processor_info(host, PROCESSOR_CPU_LOAD_INFO, &mut count, &mut info, &mut info_count) };
    if status != 0 || info.is_null() {
        return Vec::new();
    }
    let loads = unsafe { std::slice::from_raw_parts(info as *const CpuLoad, count as usize) };
    let ticks = loads
        .iter()
        .map(|load| {
            let [user, system, idle, nice] = load.ticks.map(u64::from);
            let busy = user + system + nice;
            (busy, busy + idle)
        })
        .collect();
    unsafe {
        mach2::vm::mach_vm_deallocate(mach2::traps::mach_task_self(), info as u64, info_count as u64 * size_of::<i32>() as u64);
    }
    ticks
}

/// Memory as Activity Monitor counts it: used is the apps' memory, the
/// wired and the compressed; cached, the files kept in memory. Committed is
/// what is used with what is swapped out, against memory and swap together.
fn memory(total: u64) -> MemorySample {
    let mut stats: mach2::vm_statistics::vm_statistics64 = unsafe { zeroed() };
    let mut count = mach2::host_info::HOST_VM_INFO64_COUNT;
    let read = unsafe { host_statistics64(mach2::mach_init::mach_host_self(), HOST_VM_INFO64, (&mut stats as *mut mach2::vm_statistics::vm_statistics64).cast(), &mut count) };
    let page = sysctl_number("hw.pagesize").unwrap_or(16384);
    let mut swap: libc::xsw_usage = unsafe { zeroed() };
    let mut size = size_of::<libc::xsw_usage>();
    let name = CString::new("vm.swapusage").unwrap();
    unsafe { libc::sysctlbyname(name.as_ptr(), (&mut swap as *mut libc::xsw_usage).cast(), &mut size, std::ptr::null_mut(), 0) };
    if read != 0 {
        return MemorySample { used: 0, committed: 0, commit_limit: total, cached: 0 };
    }
    let apps = (stats.internal_page_count as u64).saturating_sub(stats.purgeable_count as u64);
    let used = (apps + stats.wire_count as u64 + stats.compressor_page_count as u64) * page;
    MemorySample {
        used,
        committed: used + swap.xsu_used,
        commit_limit: total + swap.xsu_total,
        cached: stats.external_page_count as u64 * page,
    }
}

/// Bytes in and out over the interfaces that are hardware (Ethernet and
/// Wi-Fi, en*), from their 64-bit counters.
fn interface_bytes() -> (u64, u64) {
    let mut mib = [libc::CTL_NET, libc::PF_ROUTE, 0, 0, libc::NET_RT_IFLIST2, 0];
    let mut size = 0usize;
    unsafe {
        if libc::sysctl(mib.as_mut_ptr(), mib.len() as u32, std::ptr::null_mut(), &mut size, std::ptr::null_mut(), 0) != 0 {
            return (0, 0);
        }
        let mut buffer = vec![0u8; size];
        if libc::sysctl(mib.as_mut_ptr(), mib.len() as u32, buffer.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0) != 0 {
            return (0, 0);
        }
        let (mut down, mut up) = (0u64, 0u64);
        let mut offset = 0usize;
        while offset + size_of::<libc::if_msghdr>() <= size {
            let header = buffer.as_ptr().add(offset) as *const libc::if_msghdr;
            let length = (*header).ifm_msglen as usize;
            if length == 0 {
                break;
            }
            if (*header).ifm_type as i32 == libc::RTM_IFINFO2 {
                let message = std::ptr::read_unaligned(buffer.as_ptr().add(offset) as *const libc::if_msghdr2);
                let mut name = [0 as libc::c_char; libc::IF_NAMESIZE];
                if !libc::if_indextoname(message.ifm_index as u32, name.as_mut_ptr()).is_null()
                    && CStr::from_ptr(name.as_ptr()).to_bytes().starts_with(b"en")
                {
                    let data = message.ifm_data;
                    down += data.ifi_ibytes;
                    up += data.ifi_obytes;
                }
            }
            offset += length;
        }
        (down, up)
    }
}

/// The hardware interface (en*) that is up and has an IPv4 address: the one
/// traffic leaves by.
fn primary_interface() -> Option<NetworkInfo> {
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return None;
    }
    let mut found = None;
    let mut entry = list;
    while !entry.is_null() {
        let item = unsafe { &*entry };
        entry = item.ifa_next;
        let name = unsafe { CStr::from_ptr(item.ifa_name) }.to_string_lossy().into_owned();
        let up = item.ifa_flags & (libc::IFF_UP | libc::IFF_RUNNING) as u32 == (libc::IFF_UP | libc::IFF_RUNNING) as u32;
        if !name.starts_with("en") || !up || item.ifa_addr.is_null() || unsafe { (*item.ifa_addr).sa_family } as i32 != libc::AF_INET {
            continue;
        }
        let address = unsafe { &*(item.ifa_addr as *const libc::sockaddr_in) };
        let ip = std::net::Ipv4Addr::from(u32::from_be(address.sin_addr.s_addr));
        found = Some(NetworkInfo { name: name.clone(), model: name, ipv4: Some(ip.to_string()), link_bps: 0 });
        break;
    }
    unsafe { libc::freeifaddrs(list) };
    found
}

/// Each graphics processor and its readings now, from its driver's
/// statistics. On Apple silicon its memory is the system's: what it has in
/// use, of all there is, with no separate pool to borrow from.
fn graphics() -> Vec<(GpuInfo, GpuSample)> {
    let memory = sysctl_number("hw.memsize").unwrap_or(0);
    iokit::services("IOAccelerator")
        .iter()
        .filter_map(|accelerator| {
            let model = accelerator.string("model")?;
            let statistics = accelerator.dictionary("PerformanceStatistics")?;
            let percent = |key| iokit::number(&statistics, key).unwrap_or(0).clamp(0, 100) as f32;
            let info = GpuInfo { name: format!("{model} GPU"), mem_total: memory, shared_total: 0 };
            let reading = GpuSample {
                usage: percent("Device Utilization %"),
                engines: vec![("3D".into(), percent("Renderer Utilization %"))],
                mem_used: iokit::number(&statistics, "In use system memory").unwrap_or(0).max(0) as u64,
                shared_used: 0,
                temp: None,
                clock_mhz: None,
                fan_rpm: None,
                power: None,
            };
            Some((info, reading))
        })
        .collect()
}

/// Bytes read and written by every drive, from their drivers' statistics.
fn drive_bytes() -> (u64, u64) {
    iokit::services("IOBlockStorageDriver").iter().fold((0, 0), |(read, written), driver| {
        let Some(statistics) = driver.dictionary("Statistics") else { return (read, written) };
        let value = |key| iokit::number(&statistics, key).unwrap_or(0).max(0) as u64;
        (read + value("Bytes (Read)"), written + value("Bytes (Write)"))
    })
}

/// The model of each drive.
fn drive_models() -> Vec<String> {
    iokit::services("IOBlockStorageDevice")
        .iter()
        .filter_map(|device| device.dictionary("Device Characteristics"))
        .filter_map(|characteristics| iokit::get(&characteristics, "Product Name"))
        .filter_map(|name| name.downcast::<objc2_core_foundation::CFString>().ok().map(|name| name.to_string().trim().to_string()))
        .filter(|name| !name.is_empty())
        .collect()
}

/// The volumes a user sees: the startup disk and those mounted under
/// /Volumes, each with the space its container has used and holds.
fn volumes() -> Vec<VolumeSample> {
    let mut mounts: *mut libc::statfs = std::ptr::null_mut();
    let count = unsafe { libc::getmntinfo(&mut mounts, libc::MNT_NOWAIT) };
    if count <= 0 {
        return Vec::new();
    }
    let mounts = unsafe { std::slice::from_raw_parts(mounts, count as usize) };
    mounts
        .iter()
        .filter_map(|mount| {
            let path = unsafe { CStr::from_ptr(mount.f_mntonname.as_ptr()) }.to_string_lossy().into_owned();
            let shown = path == "/" || (path.starts_with("/Volumes/") && mount.f_flags & libc::MNT_LOCAL as u32 != 0);
            if !shown || mount.f_blocks == 0 {
                return None;
            }
            let block = mount.f_bsize as u64;
            let total = mount.f_blocks * block;
            // What the container has left, as Finder counts it.
            let used = total.saturating_sub(mount.f_bavail * block);
            Some(VolumeSample { name: volume_name(&path), used, total })
        })
        .collect()
}

/// A volume's name, as Finder shows it.
fn volume_name(path: &str) -> String {
    #[repr(C, packed(4))]
    struct Reply {
        length: u32,
        name: libc::attrreference_t,
        text: [u8; 256],
    }
    let mut list: libc::attrlist = unsafe { zeroed() };
    list.bitmapcount = libc::ATTR_BIT_MAP_COUNT;
    list.volattr = libc::ATTR_VOL_INFO | libc::ATTR_VOL_NAME;
    let mut reply: Reply = unsafe { zeroed() };
    let c_path = CString::new(path).unwrap();
    let read = unsafe { libc::getattrlist(c_path.as_ptr(), (&mut list as *mut libc::attrlist).cast(), (&mut reply as *mut Reply).cast(), size_of::<Reply>(), 0) };
    if read != 0 {
        return path.rsplit('/').find(|part| !part.is_empty()).unwrap_or(path).to_string();
    }
    let name = reply.name;
    let start = size_of::<u32>() + name.attr_dataoffset as usize;
    let bytes = unsafe { std::slice::from_raw_parts((&reply as *const Reply).cast::<u8>().add(start), name.attr_length as usize) };
    CStr::from_bytes_until_nul(bytes).map_or_else(|_| path.to_string(), |name| name.to_string_lossy().into_owned())
}

fn all_pids() -> Vec<i32> {
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Vec::new();
    }
    // Room for some started since.
    let mut pids = vec![0i32; count as usize + 64];
    let got = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast::<c_void>(), (pids.len() * size_of::<i32>()) as i32) };
    pids.truncate(got.max(0) as usize);
    pids.retain(|pid| *pid > 0);
    pids
}

fn rusage(pid: i32) -> Option<libc::rusage_info_v2> {
    let mut usage: libc::rusage_info_v2 = unsafe { zeroed() };
    let read = unsafe { libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V2, (&mut usage as *mut libc::rusage_info_v2).cast()) };
    (read == 0).then_some(usage)
}

fn process_name(pid: i32) -> String {
    let mut buffer = [0u8; 256];
    let length = unsafe { libc::proc_name(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    if length <= 0 {
        return format!("{pid}");
    }
    String::from_utf8_lossy(&buffer[..length as usize]).into_owned()
}

/// Seconds since the system started.
fn uptime() -> u64 {
    let mut boot: libc::timeval = unsafe { zeroed() };
    let mut size = size_of::<libc::timeval>();
    let mut mib = [libc::CTL_KERN, libc::KERN_BOOTTIME];
    let read = unsafe { libc::sysctl(mib.as_mut_ptr(), 2, (&mut boot as *mut libc::timeval).cast(), &mut size, std::ptr::null_mut(), 0) };
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if read == 0 { now.saturating_sub(boot.tv_sec as u64) } else { 0 }
}
