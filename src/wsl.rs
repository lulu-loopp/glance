//! WSL 2, read from Windows alone: never from inside a distribution, which
//! would keep it running (reading its files once a second does) or start
//! one that has stopped.
//!
//! Its virtual machine is a Hyper-V partition the Host Compute Service says
//! WSL owns; how busy its processors are is the hypervisor's own count of
//! the time each spends running the guest, and what it takes of the
//! machine's memory is the working set of the process that holds it
//! (`vmmemWSL`), as Task Manager shows it; the GPU its programs use is
//! what Windows counts for the process that runs the machine (`vmwp`, given
//! the machine's id when it is started).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::{w, PWSTR};
use windows::Win32::Foundation::{ERROR_SUCCESS, HLOCAL, LocalFree};
use windows::Win32::System::HostComputeSystem::{HcsCloseOperation, HcsCreateOperation, HcsEnumerateComputeSystems, HcsWaitForOperationResult};
use windows::Win32::System::Performance::{PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhOpenQueryW, PDH_HCOUNTER, PDH_HQUERY};
use windows::Win32::System::Registry::{RegCloseKey, RegOpenKeyExW, RegQueryInfoKeyW, HKEY, HKEY_CURRENT_USER, KEY_READ};

use crate::reading::WslSample;

/// How often the distributions running are asked for, while the machine is,
/// and how often WSL's machine is looked for again among partitions that
/// did not show it.
const LISTED: Duration = Duration::from_secs(5);
/// How long `wsl.exe` may take to list them before it is stopped.
const LISTING: Duration = Duration::from_secs(10);
/// The size of a page the hypervisor counts.
const PAGE: u64 = 4096;

pub struct Wsl {
    query: PDH_HQUERY,
    /// Each virtual processor's time running the guest, every partition's.
    processors: Option<PDH_HCOUNTER>,
    /// The memory each partition has, in pages: for WSL's, the memory its
    /// machine was made with (`.wslconfig`'s `memory`, or its default),
    /// held whole from the start, however little of it is in use.
    pages: Option<PDH_HCOUNTER>,
    /// What the machine holding WSL's memory has of the machine's.
    held: Option<PDH_HCOUNTER>,
    /// The partitions there were when WSL's was last looked for, and WSL's
    /// among them (its id, in capitals, as the counters name it).
    partitions: Vec<String>,
    partition: Option<String>,
    /// When WSL's machine was last looked for.
    looked: Option<Instant>,
    /// The distributions running, as last listed, and when that was asked.
    distros: Arc<Mutex<Option<Vec<String>>>>,
    listed: Option<Instant>,
    /// A listing under way (one at a time), and WSL no longer watched.
    listing: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    /// The process running the machine whose id is the first, once found.
    worker: Option<(String, usize)>,
    buf: Vec<u64>,
}

impl Wsl {
    pub fn new() -> Self {
        let mut query = PDH_HQUERY::default();
        let _ = unsafe { PdhOpenQueryW(windows::core::PCWSTR::null(), 0, &mut query) };
        let add = |path| {
            let mut counter = PDH_HCOUNTER::default();
            (unsafe { PdhAddEnglishCounterW(query, path, 0, &mut counter) } == ERROR_SUCCESS.0).then_some(counter)
        };
        let wsl = Wsl {
            processors: add(w!("\\Hyper-V Hypervisor Virtual Processor(*)\\% Guest Run Time")),
            pages: add(w!("\\Hyper-V VM Vid Partition(*)\\Physical Pages Allocated")),
            held: add(w!("\\Process(vmmemWSL)\\Working Set - Private")),
            query,
            partitions: Vec::new(),
            partition: None,
            looked: None,
            distros: Arc::new(Mutex::new(None)),
            listed: None,
            listing: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
            buf: Vec::new(),
        };
        // A first reading, for the rates the next one gives.
        unsafe { PdhCollectQueryData(query) };
        wsl
    }

    /// What WSL is doing now; `gpu_by_pid`, each process's use of the GPU,
    /// and `names`, each process's name, as the sampler last read them.
    pub fn read(&mut self, gpu_by_pid: Option<&HashMap<usize, Option<f32>>>, names: &HashMap<usize, String>) -> WslSample {
        if !installed() {
            return WslSample::Missing;
        }
        let collected = unsafe { PdhCollectQueryData(self.query) } == ERROR_SUCCESS.0;
        let pages = self.pages.filter(|_| collected).and_then(|c| crate::metrics::read_array(c, &mut self.buf)).unwrap_or_default();
        // The partitions changed (a machine started or stopped): which is
        // WSL's asked again.
        let mut partitions: Vec<String> = pages.iter().map(|(name, _)| name.to_uppercase()).filter(|name| name != "_TOTAL").collect();
        partitions.sort();
        // Looked for as the partitions change, and again now and then
        // while there are some and it was not found among them (the
        // service may not have said yet, or not answered).
        let missing = self.partition.is_none() && !partitions.is_empty() && self.looked.is_none_or(|at| at.elapsed() >= LISTED);
        if partitions != self.partitions || missing {
            self.partition = wsl_partition().filter(|id| partitions.contains(id));
            self.partitions = partitions;
            self.looked = Some(Instant::now());
        }
        let Some(id) = self.partition.clone() else {
            self.listed = None;
            *self.distros.lock().unwrap() = None;
            return WslSample::Stopped;
        };
        // The share of its processors' time spent running it: each of
        // them, on average.
        let prefix = format!("{id}:");
        let cpu = self.processors.filter(|_| collected).and_then(|c| crate::metrics::read_array(c, &mut self.buf)).and_then(|all| {
            let ours: Vec<f64> = all.iter().filter(|(name, _)| name.to_uppercase().starts_with(&prefix)).filter_map(|(_, value)| *value).collect();
            (!ours.is_empty()).then(|| (ours.iter().sum::<f64>() / ours.len() as f64).clamp(0.0, 100.0) as f32)
        });
        let total = pages.iter().find(|(name, _)| name.to_uppercase() == id).and_then(|(_, value)| *value).map(|pages| pages as u64 * PAGE);
        let used = self.held.filter(|_| collected).and_then(|c| crate::metrics::read_scalar(c, windows::Win32::System::Performance::PDH_FMT_DOUBLE)).map(|bytes| bytes as u64);
        // The distributions running, asked for now and then (asking starts
        // none, and keeps none running), on a thread of its own.
        if self.listed.is_none_or(|at| at.elapsed() >= LISTED) && !self.listing.swap(true, Ordering::Relaxed) {
            self.listed = Some(Instant::now());
            let (distros, listing, stop) = (self.distros.clone(), self.listing.clone(), self.stop.clone());
            std::thread::spawn(move || {
                let running = running_distros();
                if let Some(running) = running.filter(|_| !stop.load(Ordering::Relaxed)) {
                    *distros.lock().unwrap() = Some(running);
                }
                listing.store(false, Ordering::Relaxed);
            });
        }
        let distros = self.distros.lock().unwrap().clone();
        // The process running this machine: found among those called vmwp
        // by its id, once (a new machine has a new one).
        if self.worker.as_ref().is_none_or(|(of, _)| *of != id) {
            self.worker = names
                .iter()
                .filter(|(_, name)| name.eq_ignore_ascii_case("vmwp"))
                .find(|(pid, _)| command_line(**pid as u32).is_some_and(|line| line.to_uppercase().contains(&id)))
                .map(|(pid, _)| (id.clone(), *pid));
        }
        // One the counters list nothing for uses none.
        let gpu = self.worker.as_ref().zip(gpu_by_pid).and_then(|((_, pid), by_pid)| by_pid.get(pid).copied().unwrap_or(Some(0.0)));
        WslSample::Running { distros, cpu, used, total, gpu }
    }
}

impl Drop for Wsl {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        unsafe { PdhCloseQuery(self.query) };
    }
}

/// The command line process `pid` was started with.
fn command_line(pid: u32) -> Option<String> {
    use windows::Wdk::System::Threading::{NtQueryInformationProcess, ProcessCommandLineInformation};
    use windows::Win32::Foundation::{CloseHandle, UNICODE_STRING};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = vec![0u64; 512];
    let mut length = 0u32;
    let status = unsafe { NtQueryInformationProcess(process, ProcessCommandLineInformation, buf.as_mut_ptr().cast(), (buf.len() * 8) as u32, &mut length) };
    let _ = unsafe { CloseHandle(process) };
    if status.is_err() {
        return None;
    }
    // A UNICODE_STRING, its characters after it in the same buffer.
    let text = unsafe { &*(buf.as_ptr() as *const UNICODE_STRING) };
    let chars = unsafe { std::slice::from_raw_parts(text.Buffer.0, text.Length as usize / 2) };
    Some(String::from_utf16_lossy(chars))
}

/// Whether WSL has a distribution registered for this user.
fn installed() -> bool {
    let mut key = HKEY::default();
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, w!("Software\\Microsoft\\Windows\\CurrentVersion\\Lxss"), None, KEY_READ, &mut key) } != ERROR_SUCCESS {
        return false;
    }
    let mut subkeys = 0u32;
    let read = unsafe { RegQueryInfoKeyW(key, None, None, None, Some(&mut subkeys), None, None, None, None, None, None, None) };
    let _ = unsafe { RegCloseKey(key) };
    read == ERROR_SUCCESS && subkeys > 0
}

/// The id of WSL's virtual machine, if it is running, as the Host Compute
/// Service lists it (in capitals).
fn wsl_partition() -> Option<String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct System {
        id: String,
        #[serde(default)]
        owner: String,
    }
    let operation = unsafe { HcsCreateOperation(None, None) };
    if operation.is_invalid() {
        return None;
    }
    let mut document = PWSTR::null();
    let listed = unsafe { HcsEnumerateComputeSystems(windows::core::PCWSTR::null(), operation).and_then(|_| HcsWaitForOperationResult(operation, 5000, Some(&mut document))) };
    unsafe { HcsCloseOperation(operation) };
    let text = (!document.is_null()).then(|| unsafe { document.to_string() }.unwrap_or_default());
    if !document.is_null() {
        unsafe { LocalFree(Some(HLOCAL(document.0.cast()))) };
    }
    listed.ok()?;
    let systems: Vec<System> = serde_json::from_str(&text?).ok()?;
    systems.into_iter().find(|system| system.owner == "WSL").map(|system| system.id.to_uppercase())
}

/// The distributions running, by name, as `wsl.exe` lists them; none if it
/// did not within `LISTING` (it is stopped then).
fn running_distros() -> Option<Vec<String>> {
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut child = std::process::Command::new("wsl.exe")
        .args(["--list", "--running", "--quiet"])
        .env("WSL_UTF8", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .ok()?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < LISTING => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    // (A few names: they fit in the pipe while it runs.)
    let mut bytes = Vec::new();
    child.stdout.take()?.read_to_end(&mut bytes).ok()?;
    let output = std::process::Output { status, stdout: bytes, stderr: Vec::new() };
    // Running none, it says so and exits with an error.
    let bytes = output.stdout;
    // UTF-8 as asked for; one that predates asking writes UTF-16.
    let text = if bytes.len() >= 2 && bytes[1] == 0 {
        String::from_utf16_lossy(&bytes.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect::<Vec<_>>())
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    if !output.status.success() {
        return Some(Vec::new());
    }
    Some(text.lines().map(|line| line.trim_matches(|c: char| c.is_whitespace() || c == '\0').to_string()).filter(|line| !line.is_empty()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "reads this machine's WSL, as administrator, with a distribution running; run with --ignored"]
    fn reads_wsl() {
        let mut wsl = Wsl::new();
        for _ in 0..4 {
            std::thread::sleep(Duration::from_millis(1000));
            println!("{}", serde_json::to_string(&wsl.read(None, &HashMap::new())).unwrap());
        }
        assert!(matches!(wsl.read(None, &HashMap::new()), WslSample::Running { distros: Some(_), cpu: Some(_), used: Some(_), total: Some(_), .. }));
    }
}
