//! What the settings copy for a problem report: this Glance, the system,
//! the hardware Glance found to read and how, and which readings it is
//! getting. Nothing that names the user: no paths, no network addresses.

use windows::core::w;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;

use crate::metrics::{reg_string, Sample, StaticInfo};

/// The report, in English (it is for an issue on GitHub).
pub fn report() -> String {
    let app = crate::app();
    let info = &app.info;
    let windows_key = w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
    let bios = w!(r"HARDWARE\DESCRIPTION\System\BIOS");
    let yes = |b: bool| if b { "yes" } else { "no" };
    let mut lines = vec![
        format!("Glance {}", env!("CARGO_PKG_VERSION")),
        format!(
            "Windows: {} {} (build {}.{})",
            reg_string(windows_key, w!("ProductName")),
            reg_string(windows_key, w!("DisplayVersion")),
            reg_string(windows_key, w!("CurrentBuild")),
            crate::metrics::reg_dword(windows_key, w!("UBR")),
        ),
        format!("Running elevated: {}", yes(crate::elevation::is_elevated())),
        format!("Installed in a protected folder: {}", yes(crate::elevation::may_start_unasked())),
        format!("PawnIO driver: {}", crate::elevation::pawnio_version().unwrap_or_else(|| "not installed".into())),
        format!("CPU: {} ({} threads)", info.cpu_name, info.threads),
        format!("Motherboard: {} {}", reg_string(bios, w!("BaseBoardManufacturer")), info.board),
        format!("Memory: {}", info.memory_modules.clone().unwrap_or_else(|| "unknown".into())),
    ];
    lines.extend(info.gpus.iter().map(|gpu| format!("GPU: {}", gpu.name)));
    lines.extend(info.found.iter().cloned());
    lines.push("Latest readings:".into());
    match app.controller.history.lock().unwrap().back() {
        Some(sample) => lines.extend(readings(sample, info).into_iter().map(|line| format!("  {line}"))),
        None => lines.push("  none yet".into()),
    }
    lines.join("\n")
}

/// Which readings arrive, and what they are now.
fn readings(sample: &Sample, info: &StaticInfo) -> Vec<String> {
    let value = |reading: Option<f32>, unit: &str| reading.map_or("not read".to_string(), |v| format!("{v:.1} {unit}"));
    let cpu = sample.cpu_sensors.clone().unwrap_or_default();
    let mut lines = vec![
        format!("CPU temperature: {}", value(cpu.temp, "°C")),
        format!("CPU package power: {}", value(cpu.power, "W")),
        format!("CPU chiplet temperatures: {}", cpu.ccds.len()),
    ];
    for (reading, gpu) in sample.gpus.iter().zip(&info.gpus) {
        lines.push(format!(
            "{}: temperature {}, clock {}, fan {}, power {}",
            gpu.name,
            value(reading.temp, "°C"),
            value(reading.clock_mhz, "MHz"),
            reading.fan_rpm.map_or("not read".into(), |rpm| format!("{rpm} RPM")),
            value(reading.power, "W"),
        ));
    }
    let board = sample.board.clone().unwrap_or_default();
    lines.push(format!("Motherboard: {} temperatures, {} fans", board.temps.len(), board.fans.len()));
    lines.extend(board.temps.iter().map(|(name, temp)| format!("  {name}: {temp:.1} °C")));
    lines.extend(board.fans.iter().map(|(name, rpm)| format!("  {name}: {rpm:.0} RPM")));
    lines.push(format!("Memory module temperatures: {}", sample.dimm_temps.len()));
    lines.push(format!("Drive temperatures: {}", sample.drive_temps.len()));
    lines.push(format!("Battery: {}", if sample.battery.is_some() { "present" } else { "none" }));
    lines
}

/// Puts `text` on the clipboard, as the window `owner` holds it.
pub fn copy(owner: HWND, text: &str) -> bool {
    let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
    let bytes = units.len() * 2;
    unsafe {
        if OpenClipboard(Some(owner)).is_err() {
            return false;
        }
        let placed = (|| {
            EmptyClipboard().ok()?;
            let memory = GlobalAlloc(GMEM_MOVEABLE, bytes).ok()?;
            let target = GlobalLock(memory) as *mut u16;
            if target.is_null() {
                let _ = GlobalFree(Some(memory));
                return None;
            }
            std::ptr::copy_nonoverlapping(units.as_ptr(), target, units.len());
            let _ = GlobalUnlock(memory);
            // The clipboard owns the memory once it takes it.
            if SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(memory.0))).is_err() {
                let _ = GlobalFree(Some(HGLOBAL(memory.0)));
                return None;
            }
            Some(())
        })();
        let _ = CloseClipboard();
        placed.is_some()
    }
}
