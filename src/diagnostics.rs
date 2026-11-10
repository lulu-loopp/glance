//! What the settings copy for a problem report: this Glance, the system,
//! the hardware Glance found to read and how, and which readings it is
//! getting. Nothing that names the user: no paths, no network addresses.

use windows::core::w;
use windows::Win32::Foundation::HWND;

use crate::metrics::reg_string;
use crate::reading::{Sample, StaticInfo};

/// The report, in English (it is for an issue on GitHub).
pub fn report() -> String {
    let app = crate::app();
    let info = &app.info();
    let windows_key = w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
    let bios = w!(r"HARDWARE\DESCRIPTION\System\BIOS");
    let yes = |b: bool| if b { "yes" } else { "no" };
    let mut lines = vec![
        format!("Glance {}", env!("CARGO_PKG_VERSION")),
        windows(windows_key),
        format!(
            "Sensors read: {}",
            if crate::source::linked() {
                "in their own process, elevated"
            } else if crate::elevation::is_elevated() {
                "by Glance itself, elevated"
            } else {
                "by Glance itself, without administrator rights"
            }
        ),
        format!("Installed in a protected folder: {}", yes(crate::elevation::may_start_unasked())),
        format!("PawnIO driver: {}", crate::elevation::pawnio_version().unwrap_or_else(|| "not installed".into())),
        format!("CPU: {} ({} threads)", info.cpu_name, info.threads),
        format!("Motherboard: {} {}", reg_string(bios, w!("BaseBoardManufacturer")), info.board),
        // (A laptop's model is here: its maker gives its board a name shared by many of its models.)
        format!(
            "Machine: {} {} (family {}, version {})",
            reg_string(bios, w!("SystemManufacturer")),
            reg_string(bios, w!("SystemProductName")),
            reg_string(bios, w!("SystemFamily")),
            reg_string(bios, w!("SystemVersion"))
        ),
        format!("Memory: {}", info.memory_modules.clone().unwrap_or_else(|| "unknown".into())),
    ];
    lines.extend(info.gpus.iter().map(|gpu| format!("GPU: {}", gpu.name)));
    lines.push(format!("Drawn on: {} ({} devices held)", crate::ui::gfx::drawn_on().unwrap_or_else(|| "nothing yet".into()), crate::ui::gfx::alive()));
    // (Each screen's name, which what is laid out on it is kept under: two screens are never to share one.)
    lines.extend(crate::screens::describe());
    lines.extend(crate::source::found().unwrap_or_else(|| info.found.clone()));
    lines.extend(crate::source::sensor_lines());
    lines.extend(crate::drives::describe());
    // (Enough to see what led up to a fault, well within what a report may hold.)
    let log = crate::journal::recent(80);
    lines.push(format!("Recent log ({} lines):", log.len()));
    lines.extend(log.into_iter().map(|line| format!("  {line}")));
    // What the phone remote finds, where it is there: the lights, the monitors, what else it sets.
    if let Some(remote) = crate::phone::report() {
        lines.push("Phone remote:".into());
        lines.extend(remote.lines().map(|line| format!("  {line}")));
    }
    lines.extend(presenting());
    lines.push("Latest readings:".into());
    match app.controller.history.read().unwrap().back() {
        Some(sample) => lines.extend(readings(sample, info).into_iter().map(|line| format!("  {line}"))),
        None => lines.push("  none yet".into()),
    }
    lines.join("\n")
}

/// Sends `text` (the report, as the user was shown it, their own `words` in it) to Glance's site (see
/// `update::own_site`), with a secret made for it alone, which the report is followed by (see `reports`): the short
/// name it was given there, to quote when asking about it, and the report remembered to be followed (with small copies
/// of its pictures, on this thread: see `reports::remember`). None if it did not get there.
pub fn send(text: &str, words: &str, pictures: &[Vec<u8>]) -> Option<String> {
    let secret = crate::reports::secret()?;
    let encoded: Vec<String> = pictures.iter().map(|picture| base64(picture)).collect();
    let sent = serde_json::json!({ "version": env!("CARGO_PKG_VERSION"), "text": text, "pictures": encoded, "secret": secret }).to_string();
    let answer: serde_json::Value = serde_json::from_slice(&crate::update::post(&format!("{}/api/report", crate::update::own_site()), sent.as_bytes())?).ok()?;
    let id = answer.get("id")?.as_str()?.to_string();
    crate::reports::remember(&id, secret, words, pictures);
    Some(id)
}

/// `bytes` in base64 (RFC 4648), as JSON carries a picture.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const LETTERS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for three in bytes.chunks(3) {
        let bits = three.iter().enumerate().fold(0u32, |bits, (at, &byte)| bits | (byte as u32) << (16 - 8 * at));
        for at in 0..4 {
            text.push(if at <= three.len() { LETTERS[(bits >> (18 - 6 * at) & 63) as usize] as char } else { '=' });
        }
    }
    text
}

/// What the sensors have to say for a report, beyond what they found as they started: what a later look for the
/// board's chip found, how its reads went, and which programs that share its bus are running. Said by whichever
/// process reads the sensors.
pub fn sensor_lines() -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(later) = crate::superio::found_later() {
        lines.push(format!("Motherboard chip, looked for again: {later}"));
    }
    let (reads, missed, relocked) = crate::superio::reads();
    if reads > 0 {
        lines.push(format!("Motherboard chip reads: {reads}, the bus another program's all the while at {missed}, its monitor found locked again by another program and unlocked {relocked} times"));
    }
    // Other programs that read the board or light it, running now: they share the bus Glance reads the board by.
    let sharing = sharing_the_bus();
    if !sharing.is_empty() {
        lines.push(format!("Running that share the bus: {}", sharing.join(", ")));
    }
    lines
}

/// Programs known to read a board's chip or light it (and so to take the bus it is read by), by the names their
/// processes run under: those running now. Only the list of processes is read; none is opened.
fn sharing_the_bus() -> Vec<String> {
    const KNOWN: &[&str] = &[
        "armourycrate", "armourycrateservice", "lightingservice", "asuscertservice", "aisuite3", "asus_framework", "hwinfo64", "hwinfo32", "aida64", "librehardwaremonitor",
        "openhardwaremonitor", "fancontrol", "signalrgb", "icue", "msi.centralserver", "msi_center", "mysticlight", "rgbfusion", "gcc", "openrgb", "hwmonitor", "speedfan", "argus monitor",
        "nzxt cam", "corsair.service", "thermaltake", "aura", "auraservice",
    ];
    let mut found: Vec<String> = Vec::new();
    for (_, name) in processes() {
        let bare = name.to_lowercase().trim_end_matches(".exe").to_string();
        if KNOWN.contains(&bare.as_str()) && !found.contains(&name) {
            found.push(name);
        }
    }
    found
}

/// The processes running, each by its id and the name it runs under, from the list of processes: none is opened.
fn processes() -> Vec<(u32, String)> {
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    let mut found = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return found };
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
        while more {
            let length = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            found.push((entry.th32ProcessID, String::from_utf16_lossy(&entry.szExeFile[..length])));
            more = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = windows::Win32::Foundation::CloseHandle(snapshot);
    }
    found
}

/// The programs presenting frames now, as a game is looked for among them (see `presents::presenting`): each by the
/// name its process runs under (not its window's title, which may say what the user is doing), its frames a second,
/// and whether its window covers its screen as a game's does and is in front. Then the game taken, as the latest
/// readings have it. What tells why a game was taken, or not.
fn presenting() -> Vec<String> {
    let running = processes();
    let name = |pid: u32| running.iter().find(|(id, _)| *id == pid).map_or_else(|| format!("process {pid}"), |(_, name)| name.clone());
    let mut lines = vec![match crate::presents::counting() {
        true => "Programs presenting frames:".to_string(),
        false => "Programs presenting frames: not counted".to_string(),
    }];
    let yes = |b: bool| if b { "yes" } else { "no" };
    for program in crate::presents::presenting() {
        lines.push(format!(
            "  {}: {:.0} frames a second, covers its screen: {}, in front: {}",
            name(program.pid),
            program.stats.fps,
            yes(program.fills_screen),
            yes(program.in_front)
        ));
    }
    let game = crate::app().controller.history.read().unwrap().back().and_then(|sample| sample.game.clone());
    lines.push(match game {
        Some(game) => format!("  game taken: {} ({}), {}", game.program, if game.by_hand { "by hand" } else { "found" }, game.fps.map_or("frames not counted".into(), |fps| format!("{fps:.0} frames a second"))),
        None => "  game taken: none".into(),
    });
    lines.push(format!("  game mode: {:?}", crate::presents::game_mode()));
    lines
}

/// Windows' name, release and build. Windows 11 keeps "Windows 10" in its
/// product name; its builds, from 22000, tell it apart.
fn windows(key: windows::core::PCWSTR) -> String {
    let build = reg_string(key, w!("CurrentBuild"));
    let mut name = reg_string(key, w!("ProductName"));
    if build.parse::<u32>().is_ok_and(|build| build >= 22000) {
        name = name.replace("Windows 10", "Windows 11");
    }
    format!("Windows: {name} {} (build {build}.{})", reg_string(key, w!("DisplayVersion")), crate::metrics::reg_dword(key, w!("UBR")))
}

/// Which readings arrive, and what they are now.
fn readings(sample: &Sample, info: &StaticInfo) -> Vec<String> {
    let value = |reading: Option<f32>, unit: &str| reading.map_or("not read".to_string(), |v| format!("{v:.1} {unit}"));
    let cpu = sample.cpu_sensors.clone().unwrap_or_default();
    let mut lines = vec![
        // (Its time busy, as Glance shows it, and its utility, as Task Manager's Performance page does: the second is
        // the first weighed by the clock against the base, and apart by much more than that, Windows' account of the
        // time is short.)
        format!("CPU use: {} of the time busy, utility {}, clock {}", value(sample.cpu, "%"), value(sample.cpu_utility, "%"), value(sample.ghz, "GHz")),
        format!("CPU temperature: {}", value(cpu.temp, "°C")),
        format!("CPU package power: {}", value(cpu.power, "W")),
        format!("CPU chiplet temperatures: {}", cpu.ccds.len()),
    ];
    for (reading, gpu) in sample.gpus.iter().zip(&info.gpus) {
        if reading.asleep {
            lines.push(format!("{}: asleep (powered down), its driver not asked", gpu.name));
            continue;
        }
        lines.push(format!(
            "{}: temperature {}, hot spot {}, clock {}, fan {}, power {}",
            gpu.name,
            value(reading.temp, "°C"),
            value(reading.hot_spot, "°C"),
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
    let yes = |b: bool| if b { "yes" } else { "no" };
    lines.push(match &sample.battery {
        Some(battery) => format!(
            "Battery: {}%, plugged in {}, charging {}, power {}",
            battery.percent,
            yes(battery.plugged),
            yes(battery.charging),
            battery.watts.map_or("not said".to_string(), |watts| format!("{watts:.1} W"))
        ),
        None => "Battery: none".to_string(),
    });
    lines
}

/// Puts `text` on the clipboard, as the window `owner` holds it.
pub fn copy(owner: HWND, text: &str) -> bool {
    crate::clipboard::put(owner, &[(crate::clipboard::TEXT, crate::clipboard::utf16_z(text))])
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_is_as_the_standard_has_it() {
        for (bytes, text) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(super::base64(bytes.as_bytes()), text);
        }
    }

    #[test]
    #[ignore = "sends a report to Glance's site"]
    fn sends_a_report() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let picture = std::mem::take(&mut crate::ui::picture::from_file(concat!(env!("CARGO_MANIFEST_DIR"), "/icons/128x128@2x.png"), (84, 52)).expect("a picture to send").jpeg);
        let id = super::send("Glance (a test of sending from this program; nothing is wrong)", "", &[picture]);
        println!("sent as {id:?}");
        assert!(id.is_some_and(|id| id.len() == 6));
    }
}
