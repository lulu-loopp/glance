//! The panel's words, in Chinese and English, and how readings are written.


use super::prefs::LanguagePref;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    Zh,
    En,
}

impl Lang {
    pub fn resolve(pref: LanguagePref) -> Self {
        match pref {
            LanguagePref::Zh => Lang::Zh,
            LanguagePref::En => Lang::En,
            LanguagePref::System => {
                if crate::os::speaks_chinese() {
                    Lang::Zh
                } else {
                    Lang::En
                }
            }
        }
    }

    pub fn pick<'a>(self, zh: &'a str, en: &'a str) -> &'a str {
        if self == Lang::Zh { zh } else { en }
    }

    /// The name of a key the backend uses for a board input or GPU engine.
    pub fn name(self, key: &str) -> String {
        let (zh, en) = match key {
            "system" => ("系统", "System"),
            "cpu" => ("CPU", "CPU"),
            "chipset" => ("芯片组", "Chipset"),
            "cpu_socket" => ("CPU", "CPU"),
            "pcie_x16" => ("PCIe x16 插槽", "PCIe x16 slot"),
            "pcie_x1" => ("PCIe x1 插槽", "PCIe x1 slot"),
            "m2_1" => ("M.2 插槽 1", "M.2 slot 1"),
            "vrm" => ("VRM 供电", "VRM"),
            "vsoc" => ("SoC 供电", "SoC VRM"),
            "aux" => ("辅助传感器", "Auxiliary sensor"),
            "cpu_fan" => ("CPU 风扇", "CPU fan"),
            "gpu_fan" => ("显卡风扇", "GPU fan"),
            "mid_fan" => ("中间风扇", "Middle fan"),
            "pch_fan" => ("芯片组风扇", "Chipset fan"),
            "system_fan_1" => ("系统风扇 1", "System fan 1"),
            "system_fan_2" => ("系统风扇 2", "System fan 2"),
            "system_fan_3" => ("系统风扇 3", "System fan 3"),
            "system_fan_4" => ("系统风扇 4", "System fan 4"),
            "system_fan_5" => ("系统风扇 5", "System fan 5"),
            "system_fan_6" => ("系统风扇 6", "System fan 6"),
            "pump_fan" => ("水泵", "Pump"),
            "system_fan_4_pump" => ("系统风扇 4 / 水泵", "System fan 4 / pump"),
            "cpu_opt_fan" => ("CPU 辅助风扇", "CPU optional fan"),
            "3D" => ("3D", "3D"),
            "Copy" => ("复制", "Copy"),
            "VideoDecode" => ("视频解码", "Video decode"),
            "VideoEncode" => ("视频编码", "Video encode"),
            "VideoCodec" => ("视频编解码", "Video codec"),
            "Compute" => ("计算", "Compute"),
            other => return other.to_string(),
        };
        self.pick(zh, en).to_string()
    }

    pub fn duration(self, seconds: u64) -> String {
        let (days, hours, minutes) = (seconds / 86400, seconds % 86400 / 3600, seconds % 3600 / 60);
        match self {
            Lang::Zh if days > 0 => format!("{days} 天 {hours} 小时"),
            Lang::Zh if hours > 0 => format!("{hours} 小时 {minutes} 分"),
            Lang::Zh => format!("{minutes} 分钟"),
            Lang::En if days > 0 => format!("{days} d {hours} h"),
            Lang::En if hours > 0 => format!("{hours} h {minutes} min"),
            Lang::En => format!("{minutes} min"),
        }
    }
}

/// A time into a track as a player shows it: minutes and seconds ("3:07"), with the hours before them past an
/// hour ("1:02:07").
pub fn clock(seconds: u32) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// Three significant figures at most, in the largest unit that keeps the value above one.
fn scaled(mut value: f64, base: f64, units: &[&str]) -> String {
    let mut unit = 0;
    while value >= 1000.0 && unit < units.len() - 1 {
        value /= base;
        unit += 1;
    }
    let digits = if unit == 0 || value >= 100.0 { 0 } else { 1 };
    format!("{value:.digits$} {}", units[unit])
}

pub fn rate(bytes_per_second: f64, bits: bool) -> String {
    if bits {
        scaled(bytes_per_second * 8.0, 1000.0, &["bps", "Kbps", "Mbps", "Gbps"])
    } else {
        scaled(bytes_per_second, 1024.0, &["B/s", "KB/s", "MB/s", "GB/s"])
    }
}

pub fn bytes(value: f64) -> String {
    scaled(value, 1024.0, &["B", "KB", "MB", "GB", "TB"])
}

pub fn size(value: u64) -> String {
    let value = value as f64;
    if value >= GIB { format!("{:.1} GB", value / GIB) } else { format!("{:.0} MB", value / 1024.0 / 1024.0) }
}

/// How much of `total` is used, both in the whole's unit: MB for a whole under a GB (an integrated card's own
/// memory, which tenths of a GB told as 0.0 / 0.1), else GB.
pub fn usage(used: u64, total: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    if (total as f64) < GIB {
        return format!("{:.0} / {:.0} MB", used as f64 / MIB, total as f64 / MIB);
    }
    let decimals = if total as f64 >= 100.0 * GIB { 0 } else { 1 };
    format!("{:.decimals$} / {:.decimals$} GB", used as f64 / GIB, total as f64 / GIB)
}

pub fn percent(value: f32) -> String {
    if value < 10.0 { format!("{value:.1}%") } else { format!("{value:.0}%") }
}

pub fn link_speed(bits_per_second: u64) -> String {
    let bps = bits_per_second as f64;
    if bps >= 1e9 {
        let g = bps / 1e9;
        if g.fract() == 0.0 { format!("{g:.0} Gbps") } else { format!("{g:.1} Gbps") }
    } else {
        format!("{:.0} Mbps", bps / 1e6)
    }
}

/// A moment where this PC is: the hour and minute of its day, the month
/// and the day of it, and the day of the week (0, Sunday).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Local {
    pub hour: u32,
    pub minute: u32,
    pub month: u32,
    pub day: u32,
    pub weekday: u32,
}

/// Whether the system writes the time with a twelve-hour clock, as its user has it (Windows: the short time's format;
/// macOS: the locale's own hours, as its user has set them).
#[cfg(windows)]
pub fn twelve_hours() -> bool {
    use windows::Win32::Globalization::{GetLocaleInfoEx, LOCALE_SSHORTTIME};
    let mut format = [0u16; 80];
    let written = unsafe { GetLocaleInfoEx(windows::core::PCWSTR::null(), LOCALE_SSHORTTIME, Some(&mut format)) };
    // (Its length with the null that ends it: none written, nothing read.)
    written > 1 && hours_of_twelve(&String::from_utf16_lossy(&format[..written as usize - 1]))
}

#[cfg(target_os = "macos")]
pub fn twelve_hours() -> bool {
    use objc2_core_foundation::{CFDateFormatter, CFLocale, CFString};
    let locale = CFLocale::current();
    // ("j": the hours as the locale writes them.)
    let format = unsafe { CFDateFormatter::new_date_format_from_template(None, Some(&CFString::from_str("j")), 0, locale.as_deref()) };
    format.is_some_and(|format| hours_of_twelve(&format.to_string()))
}

/// Whether the time's format `format` writes the hours of a twelve-hour clock ("h", "hh", or "K"), not of a
/// twenty-four-hour one ("H", "k"): what is quoted in it (as 'h') is words, not a field.
fn hours_of_twelve(format: &str) -> bool {
    let mut quoted = false;
    for c in format.chars() {
        match c {
            '\'' => quoted = !quoted,
            'h' | 'K' if !quoted => return true,
            'H' | 'k' if !quoted => return false,
            _ => {}
        }
    }
    false
}

impl Local {
    /// The moment `t` (milliseconds since the Unix epoch).
    #[cfg(windows)]
    pub fn at(t: u64) -> Local {
        use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
        use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
        // (A file time counts hundreds of nanoseconds from 1601.)
        let ticks = (t + 11_644_473_600_000) * 10_000;
        let file = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
        let (mut utc, mut local) = (SYSTEMTIME::default(), SYSTEMTIME::default());
        unsafe {
            let _ = FileTimeToSystemTime(&file, &mut utc);
            let _ = SystemTimeToTzSpecificLocalTime(None, &utc, &mut local);
        }
        Local { hour: local.wHour as u32, minute: local.wMinute as u32, month: local.wMonth as u32, day: local.wDay as u32, weekday: local.wDayOfWeek as u32 }
    }

    #[cfg(target_os = "macos")]
    pub fn at(t: u64) -> Local {
        let seconds = (t / 1000) as libc::time_t;
        let mut local: libc::tm = unsafe { std::mem::zeroed() };
        unsafe { libc::localtime_r(&seconds, &mut local) };
        Local { hour: local.tm_hour as u32, minute: local.tm_min as u32, month: local.tm_mon as u32 + 1, day: local.tm_mday as u32, weekday: local.tm_wday as u32 }
    }

    /// Hours and minutes: of a 24-hour clock ("15:05"), or of one of
    /// `twelve`, with which half of the day after them ("3:05", "PM").
    pub fn time(self, twelve: bool, lang: Lang) -> (String, &'static str) {
        if !twelve {
            return (format!("{:02}:{:02}", self.hour, self.minute), "");
        }
        let half = match (self.hour < 12, lang) {
            (true, Lang::Zh) => "上午",
            (false, Lang::Zh) => "下午",
            (true, Lang::En) => "AM",
            (false, Lang::En) => "PM",
        };
        (format!("{}:{:02}", (self.hour + 11) % 12 + 1, self.minute), half)
    }

    /// The day: "10月11日 周日", "Sun, Oct 11".
    pub fn date(self, lang: Lang) -> String {
        let (weekday, month) = (self.weekday as usize % 7, (self.month as usize).clamp(1, 12) - 1);
        match lang {
            Lang::Zh => format!("{}月{}日 周{}", self.month, self.day, ["日", "一", "二", "三", "四", "五", "六"][weekday]),
            Lang::En => format!(
                "{}, {} {}",
                ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][weekday],
                ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][month],
                self.day
            ),
        }
    }
}

/// How long ago `then` was at `now` (ms since 1970), as a list of things lately come says it: just now; minutes ago,
/// within the hour; the time of day, today; yesterday; else the day.
#[cfg(any(windows, target_os = "macos"))]
pub fn since(then: u64, now: u64, lang: Lang) -> String {
    const MINUTE: u64 = 60_000;
    const DAY: u64 = 24 * 60 * MINUTE;
    let ago = now.saturating_sub(then);
    if ago < MINUTE {
        return lang.pick("刚刚", "Just now").into();
    }
    if ago < 60 * MINUTE {
        let minutes = ago / MINUTE;
        return match lang {
            Lang::Zh => format!("{minutes} 分钟前"),
            Lang::En => format!("{minutes} min ago"),
        };
    }
    let (at, today, yesterday) = (Local::at(then), Local::at(now), Local::at(now.saturating_sub(DAY)));
    let same = |a: Local, b: Local| (a.month, a.day) == (b.month, b.day);
    if same(at, today) && ago < DAY {
        at.time(false, lang).0
    } else if same(at, yesterday) && ago < 2 * DAY {
        lang.pick("昨天", "Yesterday").into()
    } else {
        let date = at.date(lang);
        // (The day without its weekday: "10月16日", "Oct 16".)
        match lang {
            Lang::Zh => date.split(' ').next().unwrap_or_default().to_string(),
            Lang::En => date.split(", ").nth(1).unwrap_or_default().to_string(),
        }
    }
}

/// Which day `then` was at `now` (ms since 1970), as a list of things lately come groups them: today, yesterday, or
/// earlier.
#[cfg(any(windows, target_os = "macos"))]
pub fn day_of(then: u64, now: u64, lang: Lang) -> &'static str {
    const DAY: u64 = 24 * 3_600_000;
    let (at, today, yesterday) = (Local::at(then), Local::at(now), Local::at(now.saturating_sub(DAY)));
    let same = |a: Local, b: Local| (a.month, a.day) == (b.month, b.day);
    if same(at, today) && now.saturating_sub(then) < DAY {
        lang.pick("今天", "Today")
    } else if same(at, yesterday) && now.saturating_sub(then) < 2 * DAY {
        lang.pick("昨天", "Yesterday")
    } else {
        lang.pick("更早", "Earlier")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hours of a time's format: a twelve-hour clock's or a twenty-four-hour one's, as Windows' and the
    /// locales' formats write them, quoted words not taken for them.
    #[test]
    fn a_format_says_its_clock() {
        for twelve in ["h:mm tt", "hh:mm", "tt h:mm", "h 'h' mm", "K:mm a", "ah:mm"] {
            assert!(hours_of_twelve(twelve), "{twelve}");
        }
        for twenty_four in ["H:mm", "HH:mm", "'h' H:mm", "k:mm", "HH 'h' mm", ""] {
            assert!(!hours_of_twelve(twenty_four), "{twenty_four}");
        }
    }

    #[test]
    fn writes_readings() {
        assert_eq!(rate(1536.0, false), "1.5 KB/s");
        assert_eq!(rate(999.0, false), "999 B/s");
        assert_eq!(rate(125_000.0, true), "1.0 Mbps");
        assert_eq!(percent(4.26), "4.3%");
        assert_eq!(percent(42.6), "43%");
        assert_eq!(usage(30 * 1024 * 1024 * 1024, 64 * 1024 * 1024 * 1024), "30.0 / 64.0 GB");
        // A whole under a GB (an integrated card's own memory) in MB, as no tenths of a GB tell it.
        assert_eq!(usage(40 * 1024 * 1024, 128 * 1024 * 1024), "40 / 128 MB");
        assert_eq!(usage(0, 512 * 1024 * 1024), "0 / 512 MB");
        assert_eq!(usage(1024 * 1024 * 1024, 1024 * 1024 * 1024), "1.0 / 1.0 GB");
        assert_eq!(link_speed(2_402_000_000), "2.4 Gbps");
        assert_eq!(Lang::Zh.duration(3 * 3600 + 5 * 60), "3 小时 5 分");
        let noon = Local { hour: 9, minute: 5, month: 10, day: 11, weekday: 0 };
        assert_eq!((noon.time(false, Lang::Zh).0.as_str(), noon.date(Lang::Zh).as_str(), noon.date(Lang::En).as_str()), ("09:05", "10月11日 周日", "Sun, Oct 11"));
        assert_eq!(noon.time(true, Lang::En), ("9:05".to_string(), "AM"));
        assert_eq!(Local { hour: 0, ..noon }.time(true, Lang::Zh), ("12:05".to_string(), "上午"));
        assert_eq!(Local { hour: 15, ..noon }.time(true, Lang::En), ("3:05".to_string(), "PM"));
    }

    #[test]
    fn says_how_long_ago() {
        let now = 1_760_800_000_000;
        assert_eq!(since(now - 30_000, now, Lang::Zh), "刚刚");
        assert_eq!(since(now - 5 * 60_000, now, Lang::Zh), "5 分钟前");
        assert_eq!(since(now - 59 * 60_000, now, Lang::En), "59 min ago");
        // A week ago: its day, as the date says it without its weekday.
        let week = now - 7 * 24 * 3_600_000;
        assert_eq!(since(week, now, Lang::Zh), format!("{}月{}日", Local::at(week).month, Local::at(week).day));
    }
}
