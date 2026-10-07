//! The frames programs present, from Event Tracing for Windows: DXGI's
//! Present_Start (Direct3D 10, 11 and 12) and Direct3D 9's. Nothing is
//! asked of the programs themselves: no handle to a game's process, nothing
//! injected, so no anti-cheat has anything to see. A real-time session needs
//! administrator rights, which Glance has; without them there are no frames.

use std::collections::HashMap;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use windows::core::{GUID, PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_SUCCESS, HWND, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplaySettingsW, GetMonitorInfoW, MonitorFromWindow, DEVMODEW, ENUM_CURRENT_SETTINGS, MONITORINFO, MONITORINFOEXW, MONITOR_DEFAULTTONULL};
use windows::Win32::System::Diagnostics::Etw::{
    CloseTrace, ControlTraceW, EnableTraceEx2, OpenTraceW, ProcessTrace, StartTraceW, CONTROLTRACE_HANDLE, EVENT_CONTROL_CODE_ENABLE_PROVIDER, EVENT_RECORD,
    EVENT_TRACE_CONTROL_STOP, EVENT_TRACE_LOGFILEW, EVENT_TRACE_PROPERTIES, EVENT_TRACE_REAL_TIME_MODE, PROCESS_TRACE_MODE_EVENT_RECORD,
    PROCESS_TRACE_MODE_REAL_TIME, TRACE_LEVEL_INFORMATION, WNODE_FLAG_TRACED_GUID,
};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId, IsZoomed};

use crate::frames::{FrameStats, Presents};

const DXGI: GUID = GUID::from_u128(0xCA11C036_0102_4A2D_A6AD_F03CFED5D3C9);
const D3D9: GUID = GUID::from_u128(0x783ACA0A_790E_4D7F_8451_AA850511C6B9);
/// Present_Start, in each provider.
const DXGI_PRESENT: u16 = 42;
const D3D9_PRESENT: u16 = 1;

/// The session's name: one per Glance (a debug build has its own).
#[cfg(not(debug_assertions))]
const SESSION: &str = "Glance.Frames";
#[cfg(debug_assertions)]
const SESSION: &str = "Glance.Frames.Debug";

/// Each process's presents, by process id, in QPC ticks since boot.
static PRESENTS: Mutex<Option<HashMap<u32, Presents>>> = Mutex::new(None);
/// QPC ticks a second.
static FREQUENCY: Mutex<i64> = Mutex::new(0);

fn ticks(qpc: i64) -> Duration {
    let frequency = *FREQUENCY.lock().unwrap();
    Duration::from_nanos((qpc as i128 * 1_000_000_000 / frequency.max(1) as i128) as u64)
}

unsafe extern "system" fn on_event(record: *mut EVENT_RECORD) {
    let header = unsafe { &(*record).EventHeader };
    let present = match header.ProviderId {
        DXGI => header.EventDescriptor.Id == DXGI_PRESENT,
        D3D9 => header.EventDescriptor.Id == D3D9_PRESENT,
        _ => false,
    };
    if present {
        let at = ticks(header.TimeStamp);
        if let Some(presents) = PRESENTS.lock().unwrap().as_mut() {
            presents.entry(header.ProcessId).or_default().push(at);
        }
    }
}

/// The session's properties, its name after them.
struct Properties(Vec<u8>);

impl Properties {
    fn new(name: &[u16]) -> Self {
        let size = size_of::<EVENT_TRACE_PROPERTIES>() + name.len() * 2;
        let mut buffer = vec![0u8; size];
        let properties = buffer.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES;
        unsafe {
            (*properties).Wnode.BufferSize = size as u32;
            (*properties).Wnode.Flags = WNODE_FLAG_TRACED_GUID;
            // Timestamps from the performance counter.
            (*properties).Wnode.ClientContext = 1;
            (*properties).LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
            (*properties).LoggerNameOffset = size_of::<EVENT_TRACE_PROPERTIES>() as u32;
        }
        Properties(buffer)
    }

    fn get(&mut self) -> *mut EVENT_TRACE_PROPERTIES {
        self.0.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES
    }
}

fn name() -> Vec<u16> {
    SESSION.encode_utf16().chain([0]).collect()
}

/// Starts listening, on a thread of its own, for as long as Glance runs
/// (`stop` ends it). Without administrator rights it does nothing, and no
/// program has frames.
pub fn start() {
    let mut frequency = 0i64;
    if unsafe { QueryPerformanceFrequency(&mut frequency) }.is_err() {
        return;
    }
    *FREQUENCY.lock().unwrap() = frequency;
    let name = name();
    let mut handle = CONTROLTRACE_HANDLE::default();
    let mut status = unsafe { StartTraceW(&mut handle, PCWSTR(name.as_ptr()), Properties::new(&name).get()) };
    // Left over from a Glance that ended without stopping it: taken over.
    if status == ERROR_ALREADY_EXISTS {
        stop();
        status = unsafe { StartTraceW(&mut handle, PCWSTR(name.as_ptr()), Properties::new(&name).get()) };
    }
    if status != ERROR_SUCCESS {
        crate::journal::note(format!("frames: no trace session ({})", status.0));
        return;
    }
    for provider in [DXGI, D3D9] {
        let _ = unsafe { EnableTraceEx2(handle, &provider, EVENT_CONTROL_CODE_ENABLE_PROVIDER.0, TRACE_LEVEL_INFORMATION as u8, u64::MAX, 0, 0, None) };
    }
    *PRESENTS.lock().unwrap() = Some(HashMap::new());
    thread::spawn(move || {
        let mut logfile = EVENT_TRACE_LOGFILEW { LoggerName: PWSTR(name.as_ptr() as *mut u16), ..Default::default() };
        logfile.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
        logfile.Anonymous2.EventRecordCallback = Some(on_event);
        unsafe {
            let trace = OpenTraceW(&mut logfile);
            // Returns once the session is stopped.
            let _ = ProcessTrace(&[trace], None, None);
            let _ = CloseTrace(trace);
        }
    });
}

/// Ends the session: it would outlive Glance otherwise.
pub fn stop() {
    let name = name();
    let _ = unsafe { ControlTraceW(CONTROLTRACE_HANDLE::default(), PCWSTR(name.as_ptr()), Properties::new(&name).get(), EVENT_TRACE_CONTROL_STOP) };
}

/// The program in front, if it is presenting frames: its id, its frames,
/// whether its window covers its whole screen without being maximized (a
/// game, borderless or in exclusive fullscreen: a maximized window on a
/// screen with no taskbar covers it too, and is no game), and that
/// screen's refresh rate.
pub struct Front {
    pub pid: u32,
    pub stats: FrameStats,
    pub fills_screen: bool,
    pub refresh_hz: Option<u32>,
}

/// What the program in front presents now; programs that have stopped
/// presenting are forgotten.
pub fn front() -> Option<Front> {
    let mut now = 0i64;
    unsafe { QueryPerformanceCounter(&mut now) }.ok()?;
    let now = ticks(now);
    let window = unsafe { GetForegroundWindow() };
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    let stats = {
        let mut presents = PRESENTS.lock().unwrap();
        let presents = presents.as_mut()?;
        presents.retain(|_, frames| frames.stats(now).is_some());
        presents.get(&pid)?.stats(now)?
    };
    let (fills_screen, refresh_hz) = screen_of(window);
    Some(Front { pid, stats, fills_screen, refresh_hz })
}

/// Whether `window` covers its monitor, not maximized, and the monitor's
/// refresh rate.
fn screen_of(window: HWND) -> (bool, Option<u32>) {
    unsafe {
        let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONULL);
        let mut info = MONITORINFOEXW { monitorInfo: MONITORINFO { cbSize: size_of::<MONITORINFOEXW>() as u32, ..Default::default() }, ..Default::default() };
        if monitor.is_invalid() || !GetMonitorInfoW(monitor, &mut info.monitorInfo).as_bool() {
            return (false, None);
        }
        let mut rect = RECT::default();
        let fills = !IsZoomed(window).as_bool() && GetWindowRect(window, &mut rect).is_ok() && {
            let screen = info.monitorInfo.rcMonitor;
            rect.left <= screen.left && rect.top <= screen.top && rect.right >= screen.right && rect.bottom >= screen.bottom
        };
        let mut mode = DEVMODEW { dmSize: size_of::<DEVMODEW>() as u16, ..Default::default() };
        let refresh = EnumDisplaySettingsW(PCWSTR(info.szDevice.as_ptr()), ENUM_CURRENT_SETTINGS, &mut mode)
            .as_bool()
            .then_some(mode.dmDisplayFrequency)
            // 0 and 1 stand for the hardware's default.
            .filter(|&hz| hz > 1);
        (fills, refresh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// For a look at a game in front: run elevated, with the game in front
    /// (`PRESENTS_OUT` names the file the readings go to).
    #[test]
    #[ignore = "needs administrator rights and a game in front"]
    fn reads_the_program_in_front() {
        use std::io::Write;
        let mut out = std::fs::File::create(std::env::var("PRESENTS_OUT").unwrap()).unwrap();
        start();
        for second in 0..60 {
            thread::sleep(Duration::from_secs(1));
            let mut now = 0i64;
            unsafe { QueryPerformanceCounter(&mut now) }.unwrap();
            let now = ticks(now);
            let mut rates: Vec<(u32, f32)> = PRESENTS
                .lock()
                .unwrap()
                .as_ref()
                .map(|presents| presents.iter().filter_map(|(pid, frames)| Some((*pid, frames.stats(now)?.fps))).collect())
                .unwrap_or_default();
            rates.sort_by(|a, b| b.1.total_cmp(&a.1));
            let front = front().map(|front| format!("pid {} fps {:.1} low {:?} longest {:.1} ms fills {} refresh {:?}", front.pid, front.stats.fps, front.stats.low, front.stats.longest_ms, front.fills_screen, front.refresh_hz));
            writeln!(out, "{second:>2}s presenting {rates:?}\n    front: {front:?}").unwrap();
        }
        stop();
    }
}
