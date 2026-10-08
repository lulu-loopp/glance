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
            "vrm" => ("VRM 供电", "VRM"),
            "vsoc" => ("SoC 供电", "SoC VRM"),
            "cpu_fan" => ("CPU 风扇", "CPU fan"),
            "gpu_fan" => ("显卡风扇", "GPU fan"),
            "mid_fan" => ("中间风扇", "Middle fan"),
            "system_fan_1" => ("系统风扇 1", "System fan 1"),
            "system_fan_2" => ("系统风扇 2", "System fan 2"),
            "system_fan_3" => ("系统风扇 3", "System fan 3"),
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

pub fn usage(used: u64, total: u64) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_readings() {
        assert_eq!(rate(1536.0, false), "1.5 KB/s");
        assert_eq!(rate(999.0, false), "999 B/s");
        assert_eq!(rate(125_000.0, true), "1.0 Mbps");
        assert_eq!(percent(4.26), "4.3%");
        assert_eq!(percent(42.6), "43%");
        assert_eq!(usage(30 * 1024 * 1024 * 1024, 64 * 1024 * 1024 * 1024), "30.0 / 64.0 GB");
        assert_eq!(link_speed(2_402_000_000), "2.4 Gbps");
        assert_eq!(Lang::Zh.duration(3 * 3600 + 5 * 60), "3 小时 5 分");
    }
}
