//! The skins' colours, faces and measures.

use windows::core::w;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

use super::gfx::{Color, Family, Font};
use super::prefs::ThemePref;

pub struct Theme {
    pub surface: Color,
    pub text: Color,
    pub text2: Color,
    pub text3: Color,
    pub rule: Color,
    pub trace: Color,
    pub trace2: Color,
    pub wash: Color,
    pub track: Color,
    pub signal: Color,
    pub shadow: Color,
    pub body: Font,
    pub small: Font,
    pub title: Font,
    pub figure: Font,
    pub unit: Font,
    /// Narrower figures in tables and rate readouts.
    pub value: Font,
    /// Corner radius of the panel on its free side (DIPs).
    pub radius: f32,
}

/// 记录纸: chart paper and ink. The panel is one ink until something runs
/// hot, and only then does the signal colour appear.
pub fn paper(dark: bool) -> Theme {
    let archivo = |size, weight| Font::new(Family::Archivo, size, weight);
    let base = Theme {
        surface: Color::hex(0xEEF2EC, 1.0),
        text: Color::hex(0x16221E, 1.0),
        text2: Color::hex(0x55645C, 1.0),
        text3: Color::hex(0x8A9990, 1.0),
        rule: Color::hex(0xD3DBD2, 1.0),
        trace: Color::hex(0x16221E, 1.0),
        trace2: Color::hex(0x8A9990, 1.0),
        wash: Color::hex(0x16221E, 0.07),
        track: Color::hex(0x16221E, 0.07),
        signal: Color::hex(0xD23F1C, 1.0),
        shadow: Color::hex(0x0A1410, 0.35),
        body: archivo(13.0, 500.0),
        small: archivo(12.0, 500.0),
        title: archivo(13.0, 650.0),
        figure: archivo(42.0, 620.0).width(66.0),
        unit: archivo(15.0, 500.0).width(80.0),
        value: archivo(13.0, 500.0).width(85.0),
        radius: 16.0,
    };
    if !dark {
        return base;
    }
    Theme {
        surface: Color::hex(0x14231F, 1.0),
        text: Color::hex(0xE7EEE7, 1.0),
        text2: Color::hex(0xA1B0A8, 1.0),
        text3: Color::hex(0x6B7B73, 1.0),
        rule: Color::hex(0x263732, 1.0),
        trace: Color::hex(0xE7EEE7, 1.0),
        trace2: Color::hex(0x6B7B73, 1.0),
        wash: Color::hex(0xE7EEE7, 0.08),
        track: Color::hex(0xE7EEE7, 0.08),
        signal: Color::hex(0xFF7B4A, 1.0),
        shadow: Color::hex(0x000000, 0.6),
        ..base
    }
}

/// Whether the panel is dark: as chosen, or as the system's apps are.
pub fn is_dark(pref: ThemePref) -> bool {
    match pref {
        ThemePref::Light => false,
        ThemePref::Dark => true,
        ThemePref::System | ThemePref::Backdrop => {
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
    }
}
