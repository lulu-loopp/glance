//! macOS readings: the kernel's counters through sysctl, Mach and libproc,
//! the drives' through IOKit. Rates and percentages come from the change
//! between two samples.

use std::collections::HashMap;
use std::ffi::{c_void, CStr, CString};
use std::mem::{size_of, zeroed};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use objc2_core_foundation::{CFBoolean, CFString, CFType};
use objc2_io_kit::{IOPSCopyPowerSourcesInfo, IOPSCopyPowerSourcesList, IOPSGetPowerSourceDescription};

use super::hid::Sensors;
use super::iokit;
use super::network::{display_name, Network};
use super::ioreport::Report;
use super::smc::Smc;
use crate::reading::{BatterySample, BoardSensors, CpuSensors, DriveTemperature, GpuInfo, GpuSample, MemorySample, NetworkInfo, ProcessSample, Sample, StaticInfo, SystemSample, VolumeSample};

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
    /// The System Management Controller, for the fans.
    smc: Option<Smc>,
    /// IOReport, for power and clocks.
    report: Option<Report>,
    /// The system's network configuration, for the interface in use.
    network: Option<Network>,
}

impl Sampler {
    pub fn new() -> Self {
        let mut timebase = mach2::mach_time::mach_timebase_info { numer: 1, denom: 1 };
        unsafe { mach2::mach_time::mach_timebase_info(&mut timebase) };
        let threads = sysctl_number("hw.logicalcpu").unwrap_or(1) as usize;
        let network = Network::open();
        let primary = primary_interface(network.as_ref(), &interfaces());
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
            smc: Smc::open(),
            report: Report::open(),
            network,
        };
        sampler.info.found = vec![
            format!(
                "Temperature sensors: {}",
                sampler.sensors.as_ref().map_or_else(|| "not found".to_string(), |sensors| format!("{} (HID)", sensors.read().len()))
            ),
            format!("Fans: {}", sampler.smc.as_ref().map_or_else(|| "SMC not opened".to_string(), |smc| format!("{} (SMC)", smc.fans().len()))),
            format!("Power and clocks: {}", if sampler.report.is_some() { "IOReport" } else { "not available" }),
        ];
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

        let interfaces = interfaces();
        let net = hardware_bytes(&interfaces);
        let rate = |now: u64, was: u64| now.saturating_sub(was) as f64 / seconds;
        let (net_down, net_up) = (rate(net.0, self.previous.net.0), rate(net.1, self.previous.net.1));
        let disk = drive_bytes();
        let (disk_read, disk_write) = (rate(disk.0, self.previous.disk.0), rate(disk.1, self.previous.disk.1));

        let (processes, counted, times) = self.processes(seconds);
        let temperatures = self.sensors.as_ref().map_or_else(Vec::new, Sensors::read);
        let report = self.report.as_mut().map(|report| report.read(seconds)).unwrap_or_default();
        let chip = hottest(&temperatures, "PMU tdie");
        let mut gpus: Vec<GpuSample> = graphics().into_iter().map(|(_, reading)| reading).collect();
        // Apple silicon has one GPU: IOReport's power and clock are its.
        if let Some(gpu) = gpus.first_mut() {
            gpu.power = report.gpu_power;
            gpu.clock_mhz = report.gpu_mhz;
        }

        let sample = Sample {
            t: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64),
            cpu,
            threads,
            ghz: report.cpu_mhz.map(|mhz| mhz / 1000.0),
            memory: memory(self.info.mem_total),
            gpus,
            net_down,
            net_up,
            net_total_down: net.0,
            net_total_up: net.1,
            network: primary_interface(self.network.as_ref(), &interfaces),
            disk_read,
            disk_write,
            disk_active: 0.0,
            volumes: volumes(),
            processes,
            system: SystemSample { uptime_s: uptime(), processes: counted, threads: 0, handles: 0 },
            battery: battery(),
            cpu_sensors: (chip.is_some() || report.cpu_power.is_some()).then(|| CpuSensors { temp: chip, ccds: Vec::new(), power: report.cpu_power }),
            // A Mac's fans, where it has any.
            board: self.smc.as_ref().map(Smc::fans).filter(|fans| !fans.is_empty()).map(|fans| BoardSensors { temps: Vec::new(), fans }),
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
        // Others' processes (root's, the system's users') are not a user's to
        // read; /bin/ps, the system's own and privileged, tells their CPU
        // time, memory and name (not what they read and write).
        let mut others = None;
        for pid in &pids {
            let (cpu_ns, moved, memory, name) = match rusage(*pid) {
                Some(usage) => (
                    (usage.ri_user_time + usage.ri_system_time) * self.timebase.0 / self.timebase.1,
                    usage.ri_diskio_bytesread + usage.ri_diskio_byteswritten,
                    usage.ri_phys_footprint,
                    process_name(*pid),
                ),
                None => match others.get_or_insert_with(listed).get(pid) {
                    Some(other) => (other.cpu_ns, 0, other.resident, Some(process_name(*pid).unwrap_or_else(|| other.name.clone()))),
                    None => continue,
                },
            };
            let Some(name) = name else { continue };
            times.insert(*pid, (cpu_ns, moved));
            let (cpu, io) = match self.previous.processes.get(pid) {
                Some((was_ns, was_moved)) => (
                    cpu_ns.saturating_sub(*was_ns) as f64 / 1e9 / seconds / cores * 100.0,
                    moved.saturating_sub(*was_moved) as f64 / seconds,
                ),
                None => (0.0, 0.0),
            };
            let program = programs.entry(name.clone()).or_insert(ProcessSample { name, cpu: 0.0, mem: 0, io: 0.0, gpu: 0.0 });
            program.cpu += cpu as f32;
            program.mem += memory;
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

/// Each interface's bytes in and out and its link speed (bit/s), from the
/// 64-bit counters, by BSD name.
fn interfaces() -> HashMap<String, (u64, u64, u64)> {
    let mut found = HashMap::new();
    let mut mib = [libc::CTL_NET, libc::PF_ROUTE, 0, 0, libc::NET_RT_IFLIST2, 0];
    let mut size = 0usize;
    unsafe {
        if libc::sysctl(mib.as_mut_ptr(), mib.len() as u32, std::ptr::null_mut(), &mut size, std::ptr::null_mut(), 0) != 0 {
            return found;
        }
        let mut buffer = vec![0u8; size];
        if libc::sysctl(mib.as_mut_ptr(), mib.len() as u32, buffer.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0) != 0 {
            return found;
        }
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
                if !libc::if_indextoname(message.ifm_index as u32, name.as_mut_ptr()).is_null() {
                    let data = message.ifm_data;
                    let name = CStr::from_ptr(name.as_ptr()).to_string_lossy().into_owned();
                    found.insert(name, (data.ifi_ibytes, data.ifi_obytes, data.ifi_baudrate));
                }
            }
            offset += length;
        }
    }
    found
}

/// Bytes in and out over the interfaces that are hardware (Ethernet and
/// Wi-Fi, en*).
fn hardware_bytes(interfaces: &HashMap<String, (u64, u64, u64)>) -> (u64, u64) {
    interfaces.iter().filter(|(name, _)| name.starts_with("en")).fold((0, 0), |(down, up), (_, (i, o, _))| (down + i, up + o))
}

/// The interface the default route uses now, named as System Settings
/// names it, with its IPv4 address and link speed.
fn primary_interface(network: Option<&Network>, interfaces: &HashMap<String, (u64, u64, u64)>) -> Option<NetworkInfo> {
    let bsd = network?.primary()?;
    let shown = display_name(&bsd).unwrap_or_else(|| bsd.clone());
    Some(NetworkInfo { name: shown.clone(), model: shown, ipv4: ipv4(&bsd), link_bps: interfaces.get(&bsd).map_or(0, |(_, _, speed)| *speed) })
}

/// An interface's IPv4 address.
fn ipv4(bsd: &str) -> Option<String> {
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return None;
    }
    let mut found = None;
    let mut entry = list;
    while !entry.is_null() {
        let item = unsafe { &*entry };
        entry = item.ifa_next;
        let name = unsafe { CStr::from_ptr(item.ifa_name) }.to_string_lossy();
        if name != bsd || item.ifa_addr.is_null() || unsafe { (*item.ifa_addr).sa_family } as i32 != libc::AF_INET {
            continue;
        }
        let address = unsafe { &*(item.ifa_addr as *const libc::sockaddr_in) };
        found = Some(std::net::Ipv4Addr::from(u32::from_be(address.sin_addr.s_addr)).to_string());
        break;
    }
    unsafe { libc::freeifaddrs(list) };
    found
}

/// The battery, where there is one: its charge, whether it is charging,
/// and, on battery power, how long it has left.
fn battery() -> Option<BatterySample> {
    let blob = IOPSCopyPowerSourcesInfo()?;
    let sources = unsafe { IOPSCopyPowerSourcesList(Some(&blob)) }?;
    (0..sources.count()).find_map(|i| {
        let source = unsafe { sources.value_at_index(i) } as *const CFType;
        let description = unsafe { IOPSGetPowerSourceDescription(Some(&blob), source.as_ref()) }?;
        let text = |key| iokit::get(&description, key)?.downcast::<CFString>().ok().map(|value| value.to_string());
        if text("Type").as_deref() != Some("InternalBattery") {
            return None;
        }
        let flag = |key| iokit::get(&description, key).and_then(|value| value.downcast::<CFBoolean>().ok()).is_some_and(|value| value.value());
        let (current, most) = (iokit::number(&description, "Current Capacity")?, iokit::number(&description, "Max Capacity")?.max(1));
        let on_battery = text("Power Source State").as_deref() == Some("Battery Power");
        // Minutes, or -1 while the system is still working it out.
        let left = iokit::number(&description, "Time to Empty").filter(|minutes| on_battery && *minutes > 0);
        Some(BatterySample {
            percent: (current * 100 / most).clamp(0, 100) as u8,
            charging: flag("Is Charging"),
            seconds_left: left.map(|minutes| minutes as u32 * 60),
        })
    })
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

/// A process as /bin/ps tells it.
struct Listed {
    cpu_ns: u64,
    /// Bytes.
    resident: u64,
    name: String,
}

/// Every process /bin/ps sees, by its id.
fn listed() -> HashMap<i32, Listed> {
    let Ok(output) = std::process::Command::new("/bin/ps").args(["-axo", "pid=,time=,rss=,comm="]).output() else { return HashMap::new() };
    String::from_utf8_lossy(&output.stdout).lines().filter_map(listing).collect()
}

/// One line of ps: id, CPU time, resident KB, and the command's path, which
/// may hold spaces, last.
fn listing(line: &str) -> Option<(i32, Listed)> {
    let mut rest = line.trim_start();
    let mut field = || {
        let (value, after) = rest.split_once(char::is_whitespace)?;
        rest = after.trim_start();
        Some(value)
    };
    let pid = field()?.parse().ok()?;
    let cpu_ns = cpu_time(field()?)?;
    let resident = field()?.parse::<u64>().ok()? * 1024;
    let name = rest.rsplit('/').next().filter(|name| !name.is_empty())?.to_string();
    Some((pid, Listed { cpu_ns, resident, name }))
}

/// ps's CPU time, "[[days-]hours:]minutes:seconds.hundredths", in ns.
fn cpu_time(text: &str) -> Option<u64> {
    let (days, rest) = match text.split_once('-') {
        Some((days, rest)) => (days.parse::<f64>().ok()?, rest),
        None => (0.0, text),
    };
    let seconds = rest.split(':').try_fold(0.0, |total, part| part.parse::<f64>().ok().map(|value| total * 60.0 + value))?;
    Some(((days * 86_400.0 + seconds) * 1e9) as u64)
}

fn rusage(pid: i32) -> Option<libc::rusage_info_v2> {
    let mut usage: libc::rusage_info_v2 = unsafe { zeroed() };
    let read = unsafe { libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V2, (&mut usage as *mut libc::rusage_info_v2).cast()) };
    (read == 0).then_some(usage)
}

fn process_name(pid: i32) -> Option<String> {
    let mut buffer = [0u8; 256];
    let length = unsafe { libc::proc_name(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    (length > 0).then(|| String::from_utf8_lossy(&buffer[..length as usize]).into_owned())
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

#[cfg(test)]
mod tests {
    use super::{cpu_time, listing};

    #[test]
    fn reads_ps_times() {
        assert_eq!(cpu_time("0:07.81"), Some(7_810_000_000));
        assert_eq!(cpu_time("510:03.03"), Some(30_603_030_000_000));
        assert_eq!(cpu_time("1:02:03.50"), Some(3_723_500_000_000));
        assert_eq!(cpu_time("2-00:00:01.00"), Some(172_801_000_000_000));
        assert_eq!(cpu_time("n/a"), None);
    }

    #[test]
    fn reads_ps_lines() {
        let (pid, process) = listing("  412  12:03.50  20480 /System/Library/Private Frameworks/X.framework/Support/Some Daemon").unwrap();
        assert_eq!((pid, process.cpu_ns, process.resident, process.name.as_str()), (412, 723_500_000_000, 20_971_520, "Some Daemon"));
        assert!(listing("  9 0:00.01 16").is_none());
    }
}
