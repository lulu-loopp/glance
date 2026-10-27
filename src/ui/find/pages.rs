//! The pages of Windows' Settings, and the system's own tools (Device Manager, the environment variables), found by
//! search as programs are: each by its name as Windows 11 shows it, in Chinese and in English, where it is in Settings,
//! the other words it is looked for by, and what opens it.

use crate::ui::text::Lang;

/// A page, or a tool.
pub struct Page {
    pub zh: &'static str,
    pub en: &'static str,
    /// Where it is in Settings (none for a tool).
    pub at_zh: &'static str,
    pub at_en: &'static str,
    /// Other words it is looked for by, either language's.
    pub also: &'static [&'static str],
    /// What opens it: a Settings page's address (`ms-settings:…`), or a program and what it is given.
    pub opens: Opens,
}

pub enum Opens {
    Settings(&'static str),
    Tool(&'static str, &'static str),
}

impl Page {
    pub fn name(&self, lang: Lang) -> &'static str {
        lang.pick(self.zh, self.en)
    }

    pub fn at(&self, lang: Lang) -> &'static str {
        lang.pick(self.at_zh, self.at_en)
    }

    /// Whether it is a tool of the system's (not a page of Settings).
    pub fn tool(&self) -> bool {
        matches!(self.opens, Opens::Tool(..))
    }
}

const fn page(zh: &'static str, en: &'static str, at_zh: &'static str, at_en: &'static str, also: &'static [&'static str], uri: &'static str) -> Page {
    Page { zh, en, at_zh, at_en, also, opens: Opens::Settings(uri) }
}

const fn tool(zh: &'static str, en: &'static str, also: &'static [&'static str], program: &'static str, given: &'static str) -> Page {
    Page { zh, en, at_zh: "系统工具", at_en: "System tool", also, opens: Opens::Tool(program, given) }
}

pub const PAGES: &[Page] = &[
    page("显示", "Display", "系统", "System", &["亮度", "分辨率", "缩放", "多显示器", "brightness", "resolution", "scale", "monitor"], "ms-settings:display"),
    page("夜间模式", "Night light", "系统 > 显示", "System > Display", &["护眼", "暖色", "blue light"], "ms-settings:nightlight"),
    page("HDR", "HDR", "系统 > 显示", "System > Display", &[], "ms-settings:display-hdr"),
    page("显示卡", "Graphics", "系统 > 显示", "System > Display", &["图形", "独显", "gpu", "graphics preference"], "ms-settings:display-advancedgraphics"),
    page("声音", "Sound", "系统", "System", &["音量", "扬声器", "volume", "speaker"], "ms-settings:sound"),
    page("所有声音设备", "All sound devices", "系统 > 声音", "System > Sound", &["耳机", "麦克风", "headphones", "microphone"], "ms-settings:sound-devices"),
    page("音量合成器", "Volume mixer", "系统 > 声音", "System > Sound", &["应用音量", "app volume"], "ms-settings:apps-volume"),
    page("通知", "Notifications", "系统", "System", &["勿扰", "do not disturb"], "ms-settings:notifications"),
    page("专注", "Focus", "系统", "System", &["专注助手", "focus assist"], "ms-settings:quiethours"),
    page("电源", "Power", "系统", "System", &["睡眠", "休眠", "屏幕关闭", "电源模式", "sleep", "power mode"], "ms-settings:powersleep"),
    page("电池", "Battery", "系统 > 电源", "System > Power", &["节电模式", "battery saver"], "ms-settings:batterysaver"),
    page("存储", "Storage", "系统", "System", &["磁盘空间", "存储感知", "清理", "disk space", "storage sense", "cleanup"], "ms-settings:storagesense"),
    page("附近共享", "Nearby sharing", "系统", "System", &["nearby share"], "ms-settings:crossdevice"),
    page("多任务处理", "Multitasking", "系统", "System", &["贴靠", "窗口", "虚拟桌面", "snap", "virtual desktops"], "ms-settings:multitasking"),
    page("激活", "Activation", "系统", "System", &["产品密钥", "product key"], "ms-settings:activation"),
    page("疑难解答", "Troubleshoot", "系统", "System", &["故障排除"], "ms-settings:troubleshoot"),
    page("恢复", "Recovery", "系统", "System", &["重置此电脑", "高级启动", "reset this pc", "advanced startup"], "ms-settings:recovery"),
    page("剪贴板", "Clipboard", "系统", "System", &["剪贴板历史", "clipboard history"], "ms-settings:clipboard"),
    page("远程桌面", "Remote Desktop", "系统", "System", &["rdp"], "ms-settings:remotedesktop"),
    page("系统信息", "About", "系统", "System", &["关于", "设备名称", "重命名这台电脑", "device name", "rename this pc", "specifications"], "ms-settings:about"),
    page("开发者选项", "For developers", "系统", "System", &["开发人员模式", "developer mode"], "ms-settings:developers"),
    page("蓝牙和其他设备", "Bluetooth & devices", "蓝牙和其他设备", "Bluetooth & devices", &["蓝牙", "bluetooth"], "ms-settings:bluetooth"),
    page("打印机和扫描仪", "Printers & scanners", "蓝牙和其他设备", "Bluetooth & devices", &["打印机", "扫描仪", "printer", "scanner"], "ms-settings:printers"),
    page("鼠标", "Mouse", "蓝牙和其他设备", "Bluetooth & devices", &["指针速度", "滚轮", "pointer speed", "scroll"], "ms-settings:mousetouchpad"),
    page("触摸板", "Touchpad", "蓝牙和其他设备", "Bluetooth & devices", &["手势", "gestures"], "ms-settings:devices-touchpad"),
    page("笔和 Windows Ink", "Pen & Windows Ink", "蓝牙和其他设备", "Bluetooth & devices", &["手写笔", "pen"], "ms-settings:pen"),
    page("自动播放", "AutoPlay", "蓝牙和其他设备", "Bluetooth & devices", &[], "ms-settings:autoplay"),
    page("USB", "USB", "蓝牙和其他设备", "Bluetooth & devices", &[], "ms-settings:usb"),
    page("摄像头", "Cameras", "蓝牙和其他设备", "Bluetooth & devices", &["相机", "webcam"], "ms-settings:camera"),
    page("网络和 Internet", "Network & internet", "网络和 Internet", "Network & internet", &["网络", "联网", "network"], "ms-settings:network-status"),
    page("WLAN", "Wi-Fi", "网络和 Internet", "Network & internet", &["无线", "wifi", "wlan", "wireless"], "ms-settings:network-wifi"),
    page("以太网", "Ethernet", "网络和 Internet", "Network & internet", &["有线", "网线", "wired"], "ms-settings:network-ethernet"),
    page("VPN", "VPN", "网络和 Internet", "Network & internet", &[], "ms-settings:network-vpn"),
    page("移动热点", "Mobile hotspot", "网络和 Internet", "Network & internet", &["热点", "hotspot"], "ms-settings:network-mobilehotspot"),
    page("飞行模式", "Airplane mode", "网络和 Internet", "Network & internet", &[], "ms-settings:network-airplanemode"),
    page("代理", "Proxy", "网络和 Internet", "Network & internet", &["proxy"], "ms-settings:network-proxy"),
    page("高级网络设置", "Advanced network settings", "网络和 Internet", "Network & internet", &["网络适配器", "network adapter"], "ms-settings:network-advancedsettings"),
    page("背景", "Background", "个性化", "Personalization", &["壁纸", "桌面背景", "wallpaper"], "ms-settings:personalization-background"),
    page("颜色", "Colors", "个性化", "Personalization", &["深色模式", "浅色模式", "主题色", "dark mode", "light mode", "accent color"], "ms-settings:colors"),
    page("主题", "Themes", "个性化", "Personalization", &["桌面图标", "desktop icons"], "ms-settings:themes"),
    page("锁屏界面", "Lock screen", "个性化", "Personalization", &["锁屏"], "ms-settings:lockscreen"),
    page("开始", "Start", "个性化", "Personalization", &["开始菜单", "start menu"], "ms-settings:personalization-start"),
    page("任务栏", "Taskbar", "个性化", "Personalization", &["托盘", "系统托盘", "tray"], "ms-settings:taskbar"),
    page("字体", "Fonts", "个性化", "Personalization", &["font"], "ms-settings:fonts"),
    page("已安装的应用", "Installed apps", "应用", "Apps", &["卸载", "程序", "uninstall", "programs"], "ms-settings:appsfeatures"),
    page("默认应用", "Default apps", "应用", "Apps", &["打开方式", "默认浏览器", "open with", "default browser"], "ms-settings:defaultapps"),
    page("可选功能", "Optional features", "应用", "Apps", &[], "ms-settings:optionalfeatures"),
    page("启动", "Startup", "应用", "Apps", &["开机启动", "自启动", "startup apps"], "ms-settings:startupapps"),
    page("你的信息", "Your info", "账户", "Accounts", &["账户", "头像", "account"], "ms-settings:yourinfo"),
    page("电子邮件和账户", "Email & accounts", "账户", "Accounts", &["邮箱", "email"], "ms-settings:emailandaccounts"),
    page("登录选项", "Sign-in options", "账户", "Accounts", &["密码", "pin", "指纹", "面部", "password", "fingerprint", "windows hello"], "ms-settings:signinoptions"),
    page("Windows 备份", "Windows backup", "账户", "Accounts", &["备份", "backup"], "ms-settings:backup"),
    page("日期和时间", "Date & time", "时间和语言", "Time & language", &["时区", "时间", "time zone", "clock"], "ms-settings:dateandtime"),
    page("语言和区域", "Language & region", "时间和语言", "Time & language", &["语言", "区域", "language", "region"], "ms-settings:regionlanguage"),
    page("输入", "Typing", "时间和语言", "Time & language", &["输入法", "键盘", "触摸键盘", "ime", "keyboard"], "ms-settings:typing"),
    page("语音", "Speech", "时间和语言", "Time & language", &[], "ms-settings:speech"),
    page("游戏模式", "Game Mode", "游戏", "Gaming", &["游戏"], "ms-settings:gaming-gamemode"),
    page("Game Bar", "Game Bar", "游戏", "Gaming", &["游戏栏", "xbox"], "ms-settings:gaming-gamebar"),
    page("屏幕截图", "Captures", "游戏", "Gaming", &["录屏", "游戏录制", "recording"], "ms-settings:gaming-gamedvr"),
    page("文本大小", "Text size", "辅助功能", "Accessibility", &["字体大小", "字号", "font size"], "ms-settings:easeofaccess-display"),
    page("视觉效果", "Visual effects", "辅助功能", "Accessibility", &["透明效果", "动画效果", "transparency", "animation"], "ms-settings:easeofaccess-visualeffects"),
    page("鼠标指针和触控", "Mouse pointer and touch", "辅助功能", "Accessibility", &["指针大小", "鼠标指针", "pointer size", "cursor"], "ms-settings:easeofaccess-mousepointer"),
    page("放大镜", "Magnifier", "辅助功能", "Accessibility", &[], "ms-settings:easeofaccess-magnifier"),
    page("颜色滤镜", "Color filters", "辅助功能", "Accessibility", &["色盲"], "ms-settings:easeofaccess-colorfilter"),
    page("讲述人", "Narrator", "辅助功能", "Accessibility", &["读屏"], "ms-settings:easeofaccess-narrator"),
    page("键盘", "Keyboard", "辅助功能", "Accessibility", &["粘滞键", "屏幕键盘", "sticky keys", "on-screen keyboard"], "ms-settings:easeofaccess-keyboard"),
    page("Windows 安全中心", "Windows Security", "隐私和安全性", "Privacy & security", &["杀毒", "防火墙", "病毒和威胁防护", "defender", "antivirus", "firewall"], "ms-settings:windowsdefender"),
    page("位置", "Location", "隐私和安全性", "Privacy & security", &["定位"], "ms-settings:privacy-location"),
    page("摄像头权限", "Camera permissions", "隐私和安全性", "Privacy & security", &["摄像头", "camera"], "ms-settings:privacy-webcam"),
    page("麦克风权限", "Microphone permissions", "隐私和安全性", "Privacy & security", &["麦克风", "microphone"], "ms-settings:privacy-microphone"),
    page("Windows 更新", "Windows Update", "Windows 更新", "Windows Update", &["更新", "升级", "update"], "ms-settings:windowsupdate"),
    page("更新历史记录", "Update history", "Windows 更新", "Windows Update", &[], "ms-settings:windowsupdate-history"),
    page("高级选项", "Advanced options", "Windows 更新", "Windows Update", &["暂停更新", "pause updates"], "ms-settings:windowsupdate-options"),
    page("Windows 预览体验计划", "Windows Insider Program", "Windows 更新", "Windows Update", &["insider", "预览版"], "ms-settings:windowsinsider"),
    tool("设备管理器", "Device Manager", &["驱动", "driver", "devmgmt"], "devmgmt.msc", ""),
    tool("磁盘管理", "Disk Management", &["分区", "partition", "diskmgmt"], "diskmgmt.msc", ""),
    tool("控制面板", "Control Panel", &["control"], "control.exe", ""),
    tool("编辑系统环境变量", "Edit environment variables", &["环境变量", "path", "environment variables"], "rundll32.exe", "sysdm.cpl,EditEnvironmentVariables"),
    tool("服务", "Services", &["services"], "services.msc", ""),
    tool("事件查看器", "Event Viewer", &["日志", "eventvwr", "logs"], "eventvwr.msc", ""),
    tool("网络连接", "Network Connections", &["适配器", "ncpa", "adapter"], "ncpa.cpl", ""),
    tool("程序和功能", "Programs and Features", &["卸载程序", "appwiz", "uninstall a program"], "appwiz.cpl", ""),
    tool("系统属性", "System Properties", &["高级系统设置", "虚拟内存", "advanced system settings", "virtual memory"], "sysdm.cpl", ""),
    tool("任务计划程序", "Task Scheduler", &["计划任务", "taskschd"], "taskschd.msc", ""),
    tool("资源监视器", "Resource Monitor", &["resmon"], "resmon.exe", ""),
    tool("注册表编辑器", "Registry Editor", &["注册表", "regedit", "registry"], "regedit.exe", ""),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every page is named in both languages, opens something, and is told apart from the others by its name.
    #[test]
    fn every_page_is_named_and_opens() {
        let mut names = std::collections::HashSet::new();
        for page in PAGES {
            assert!(!page.zh.is_empty() && !page.en.is_empty());
            assert!(names.insert(page.zh), "{} twice", page.zh);
            match page.opens {
                Opens::Settings(uri) => assert!(uri.starts_with("ms-settings:") && !page.at_zh.is_empty()),
                Opens::Tool(program, _) => assert!(!program.is_empty()),
            }
        }
    }
}
