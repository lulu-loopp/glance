//! What Glance asks of Windows for its look.

use windows::core::w;
use windows::UI::ViewManagement::{UIColorType, UISettings};
use windows::Win32::Globalization::GetUserDefaultUILanguage;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

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
