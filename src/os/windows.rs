//! What Glance asks of Windows for its look.

use windows::core::{w, BOOL};
use windows::UI::ViewManagement::{UIColorType, UISettings};
use windows::Win32::Globalization::GetUserDefaultUILanguage;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

/// An active desktop display, identified by Windows' display device name.
#[derive(Clone, Debug, PartialEq)]
pub struct Display {
    pub id: String,
    pub bounds: RECT,
    pub primary: bool,
}

pub fn displays() -> Vec<Display> {
    unsafe extern "system" fn collect(
        handle: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let displays = unsafe { &mut *(data.0 as *mut Vec<Display>) };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if unsafe { GetMonitorInfoW(handle, &mut info as *mut _ as *mut MONITORINFO) }.as_bool() {
            let end = info
                .szDevice
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(info.szDevice.len());
            displays.push(Display {
                id: String::from_utf16_lossy(&info.szDevice[..end]),
                bounds: info.monitorInfo.rcMonitor,
                primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
            });
        }
        BOOL(1)
    }
    let mut displays: Vec<Display> = Vec::new();
    let _ = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut displays as *mut _ as isize),
        )
    };
    displays.sort_by(|a, b| a.id.cmp(&b.id));
    displays
}

/// A disconnected selection falls back to the primary display, without
/// forgetting the selection so it can be used when reconnected.
pub fn selected_display<'a>(displays: &'a [Display], id: &str) -> Option<&'a Display> {
    displays
        .iter()
        .find(|display| display.id == id)
        .or_else(|| displays.iter().find(|display| display.primary))
        .or_else(|| displays.first())
}

/// Whether the user's interface language is Chinese, in any of its variants
/// (by its primary language id).
pub fn speaks_chinese() -> bool {
    (unsafe { GetUserDefaultUILanguage() } & 0x3FF) == 0x04
}

/// Whether the system's apps are dark.
pub fn apps_dark() -> bool {
    let mut light = 1u32;
    let mut size = 4u32;
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut light as *mut _ as *mut _),
            Some(&mut size),
        )
    };
    read.is_ok() && light == 0
}

/// The user's accent colour as 0xRRGGBB, in the shade the system pairs with
/// a dark or a light theme; `None` where the system will not say.
pub fn accent(dark: bool) -> Option<u32> {
    let shade = if dark { UIColorType::AccentLight2 } else { UIColorType::AccentDark1 };
    let color = UISettings::new().and_then(|settings| settings.GetColorValue(shade)).ok()?;
    Some(((color.R as u32) << 16) | ((color.G as u32) << 8) | color.B as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disconnected_display_returns_when_reconnected() {
        let primary = Display {
            id: "primary".into(),
            bounds: RECT::default(),
            primary: true,
        };
        let external = Display {
            id: "external".into(),
            bounds: RECT::default(),
            primary: false,
        };
        let connected = [external.clone(), primary.clone()];
        assert_eq!(selected_display(&connected, "external"), Some(&external));
        assert_eq!(
            selected_display(std::slice::from_ref(&primary), "external"),
            Some(&primary)
        );
        assert_eq!(selected_display(&connected, "external"), Some(&external));
        assert_eq!(selected_display(&[], "external"), None);
    }
}
