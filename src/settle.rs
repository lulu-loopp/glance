//! When Glance's heavy work may begin (search's first listing of every drive, the recent files' journals read back),
//! and how its threads do it: not while Windows is still starting (a laptop froze at every sign-in, its disk and
//! processors taken by Windows and every program it starts, and by Glance listing every drive), and never in the way
//! of what the user does.
//!
//! The machine is settled as Windows itself tells a start to be over: its boot performance diagnostics
//! (Microsoft-Windows-Diagnostics-Performance, event 100) end the time after the desktop shows (post-boot) once the
//! processors and the disks are 80 per cent idle for 10 seconds. Asked once a second here, each second counted: the
//! processors' time (every group's) and the disks' (`\PhysicalDisk(_Total)\% Idle Time`), the processors' alone where
//! the disks' counter cannot be read. Glance started later (after an update, by hand) finds it so within seconds, and
//! on a machine kept busy (a game) the work waits till it is not, or till the user asks for what it makes (`want`):
//! search called up, the recent files shown.

use std::sync::{Condvar, Mutex, Once};
use std::time::Duration;

/// How idle the processors and the disks are to be (their busy share at most), and for how many seconds running.
const BUSY_MOST: f64 = 0.2;
const QUIET_SECONDS: u32 = 10;

/// The heavy work, each kind begun on its own.
#[derive(Clone, Copy)]
pub enum Work {
    /// Search's drives listed.
    Search,
    /// The recent files' journals read.
    Recent,
}

/// Whether the machine has settled, and whether the user asked for what each kind of work makes.
struct Gate {
    settled: bool,
    wanted: [bool; 2],
}

static GATE: (Mutex<Gate>, Condvar) = (Mutex::new(Gate { settled: false, wanted: [false; 2] }), Condvar::new());

impl Gate {
    fn open(&self, work: Work) -> bool {
        self.settled || self.wanted[work as usize]
    }
}

/// Whether `work` may begin now: the machine settled, or the user asked for what it makes.
pub fn ready(work: Work) -> bool {
    watch();
    GATE.0.lock().unwrap().open(work)
}

/// Waits till `work` may begin (see `ready`).
pub fn wait(work: Work) {
    watch();
    drop(GATE.1.wait_while(GATE.0.lock().unwrap(), |gate| !gate.open(work)).unwrap());
}

/// The user asked for what `work` makes: it begins now, settled or not.
pub fn want(work: Work) {
    GATE.0.lock().unwrap().wanted[work as usize] = true;
    GATE.1.notify_all();
}

/// The machine looked at once a second, on a thread of its own, till it has settled (begun once).
fn watch() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new().name("glance: settling".into()).spawn(|| {
            let mut quiet = Quiet::default();
            let mut sampled = Sampled::new();
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let (processors, disks) = sampled.busy();
                if quiet.saw(processors, disks) {
                    break;
                }
            }
            GATE.0.lock().unwrap().settled = true;
            GATE.1.notify_all();
        });
    });
}

/// How many seconds running the machine was idle enough.
#[derive(Default)]
struct Quiet {
    seconds: u32,
}

impl Quiet {
    /// A second whose busy shares were `processors` and `disks` (none: not told) seen: whether it has settled. A
    /// second not told of the processors is not counted quiet.
    fn saw(&mut self, processors: Option<f64>, disks: Option<f64>) -> bool {
        let quiet = processors.is_some_and(|busy| busy <= BUSY_MOST) && disks.is_none_or(|busy| busy <= BUSY_MOST);
        self.seconds = if quiet { self.seconds + 1 } else { 0 };
        self.seconds >= QUIET_SECONDS
    }
}

/// The processors' and the disks' busy shares since they were last asked.
struct Sampled {
    processors: Option<(u64, u64)>,
    query: windows::Win32::System::Performance::PDH_HQUERY,
    disks: Option<windows::Win32::System::Performance::PDH_HCOUNTER>,
}

impl Sampled {
    fn new() -> Sampled {
        use windows::Win32::System::Performance::{PdhAddEnglishCounterW, PdhCollectQueryData, PdhOpenQueryW, PDH_HCOUNTER, PDH_HQUERY};
        let mut query = PDH_HQUERY::default();
        let opened = unsafe { PdhOpenQueryW(windows::core::PCWSTR::null(), 0, &mut query) } == 0;
        let disks = opened
            .then(|| {
                let mut counter = PDH_HCOUNTER::default();
                (unsafe { PdhAddEnglishCounterW(query, windows::core::w!(r"\PhysicalDisk(_Total)\% Idle Time"), 0, &mut counter) } == 0).then_some(counter)
            })
            .flatten();
        // (A rate's first collection is what the next is told against.)
        if disks.is_some() {
            unsafe { PdhCollectQueryData(query) };
        }
        Sampled { processors: crate::metrics::system_times(), query, disks }
    }

    fn busy(&mut self) -> (Option<f64>, Option<f64>) {
        let now = crate::metrics::system_times();
        let processors = match (self.processors, now) {
            (Some((idle_was, busy_was)), Some((idle, busy))) => {
                let (idle, busy) = (idle.saturating_sub(idle_was), busy.saturating_sub(busy_was));
                (idle + busy > 0).then(|| busy as f64 / (idle + busy) as f64)
            }
            _ => None,
        };
        self.processors = now;
        let disks = self.disks.and_then(|counter| {
            unsafe { windows::Win32::System::Performance::PdhCollectQueryData(self.query) };
            crate::metrics::read_scalar(counter, windows::Win32::System::Performance::PDH_FMT_DOUBLE).map(|idle| 1.0 - (idle / 100.0).clamp(0.0, 1.0))
        });
        (processors, disks)
    }
}

impl Drop for Sampled {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::System::Performance::PdhCloseQuery(self.query);
        }
    }
}

/// This thread's work put in the background: its processor time, its disk reads and writes, and the memory it touches
/// all below what the user's programs ask (`THREAD_MODE_BACKGROUND_BEGIN`). Where Windows refuses it (for a reason
/// other than its being so already), that is noted, and its processor time at least is put lowest.
pub fn background(what: &str) {
    use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN, THREAD_PRIORITY_LOWEST};
    /// ERROR_THREAD_MODE_ALREADY_BACKGROUND.
    const ALREADY: u32 = 400;
    let Err(error) = (unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) }) else { return };
    if error.code() == windows::core::HRESULT::from_win32(ALREADY) {
        return;
    }
    let lowest = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST) };
    crate::journal::note(format!("{what}: not put in the background ({error}), its processor time {}", if lowest.is_ok() { "put lowest" } else { "as it was" }));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Settled as Windows tells a start over: ten seconds running with the processors and the disks 80 per cent idle.
    /// A busy second (of either) begins the count again, one not told of the processors is not quiet, and the disks not
    /// told of, the processors' alone count.
    #[test]
    fn the_machine_settles_after_ten_quiet_seconds_running() {
        let mut quiet = Quiet::default();
        for _ in 0..9 {
            assert!(!quiet.saw(Some(0.05), Some(0.1)));
        }
        assert!(!quiet.saw(Some(0.05), Some(0.6)), "the disks busy");
        for _ in 0..9 {
            assert!(!quiet.saw(Some(0.1), Some(0.1)));
        }
        assert!(!quiet.saw(Some(0.5), Some(0.0)), "the processors busy");
        assert!(!quiet.saw(None, Some(0.0)), "the processors not told of");
        for _ in 0..9 {
            assert!(!quiet.saw(Some(0.2), None));
        }
        assert!(quiet.saw(Some(0.2), None));
    }

    /// Put in the background, a thread is so (asked again, it stays so): its processor time below the lowest it is put
    /// at where Windows refuses it.
    #[test]
    fn a_thread_is_put_in_the_background() {
        std::thread::spawn(|| {
            background("test");
            background("test");
            use windows::Win32::System::Threading::{GetCurrentThread, GetThreadPriority, THREAD_PRIORITY_LOWEST};
            assert!(unsafe { GetThreadPriority(GetCurrentThread()) } < THREAD_PRIORITY_LOWEST.0);
        })
        .join()
        .unwrap();
    }
}
