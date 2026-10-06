//! The skins: each one's colours, faces and measures.

use super::canvas::{Color, Family, Font};
use super::prefs::ThemePref;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Skin {
    /// 记录纸: one slab of chart paper flush with the screen edge.
    Paper,
    /// Windows 11: one acrylic sheet, as the Start menu and Quick Settings.
    Fluent,
    /// 磨砂玻璃: separate pieces of clear glass bending the desktop at their rims.
    Glass,
}

impl Skin {
    /// The skin a setting names; an unknown name is chart paper.
    pub fn named(name: &str) -> Self {
        match name {
            "fluent" => Skin::Fluent,
            "glass" => Skin::Glass,
            _ => Skin::Paper,
        }
    }

    /// Whether the skin draws the desktop behind the panel.
    pub fn sees_backdrop(self) -> bool {
        self != Skin::Paper
    }
}

/// A box shadow as CSS writes one: offset down, blur radius, spread, colour.
#[derive(Clone, Copy)]
pub struct Shadow {
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
}

/// How far the panel travels as it comes in.
#[derive(Clone, Copy)]
pub enum Entrance {
    /// All the way in from past the edge: its own depth and this much more.
    Beyond(f32),
    /// A short slide under the fade.
    Slide(f32),
}

/// What each lane's charts and meters are drawn in.
#[derive(Clone, Copy)]
pub struct Ink {
    pub trace: Color,
    pub trace2: Color,
    /// The fill under a chart's main trace, at its top and at the bottom.
    pub wash: Color,
    pub wash_end: Color,
}

pub struct Theme {
    pub skin: Skin,
    pub dark: bool,
    pub text: Color,
    pub text2: Color,
    pub text3: Color,
    pub rule: Color,
    pub track: Color,
    pub signal: Color,
    /// Behind a button under the pointer.
    pub hover: Color,
    /// Ink for each lane, by its module.
    inks: Vec<(&'static str, Ink)>,
    ink: Ink,
    pub body: Font,
    pub small: Font,
    pub title: Font,
    pub figure: Font,
    /// The figure's line height, as a share of its size.
    pub figure_line: f32,
    pub unit: Font,
    /// Narrower figures in tables and rate readouts.
    pub value: Font,

    /// A lane's padding: sides, top, bottom.
    pub pad_x: f32,
    pub pad_top: f32,
    pub pad_bottom: f32,
    /// Around the lanes as a whole: top, sides, bottom.
    pub outer: (f32, f32, f32),
    pub lane_gap: f32,
    pub column_gap: f32,
    /// Between the lanes and the bar.
    pub bar_gap: f32,
    pub bar_height: f32,
    /// The bar's padding: left, right.
    pub bar_pad: (f32, f32),
    /// Lanes ruled off below, and columns beside each other.
    pub ruled: bool,
    pub meter_height: f32,
    pub meter_radius: f32,
    pub thread_radius: f32,
    pub control_radius: f32,

    /// The panel's corner radius (paper, Windows 11) or each piece's (glass).
    pub radius: f32,
    pub shadows: Vec<Shadow>,
    /// Room the shadow needs around the panel.
    pub margin: f32,
    /// Gap between the panel and the screen edge.
    pub inset: f32,
    pub entrance: Entrance,

    /// Chart paper's sheet.
    pub paper: Color,
    /// Windows 11's sheet: the luminosity layer that evens out the desktop's
    /// brightness, the tint over it, its stroke and the footer strip.
    pub luminosity: Color,
    pub tint: Color,
    pub stroke: Color,
    pub footer: Color,
    /// Glass: its tint at no frost, and how much more at full frost, and the
    /// faint halo that keeps text legible.
    pub glass: Color,
    pub glass_clear: f32,
    pub glass_frosted: f32,
    pub legibility: Color,
}

impl Theme {
    pub fn new(skin: Skin, dark: bool) -> Self {
        match skin {
            Skin::Paper => paper(dark),
            Skin::Fluent => fluent(dark),
            Skin::Glass => glass(dark),
        }
    }

    /// The ink of the lane for module `id`.
    pub fn ink(&self, id: &str) -> Ink {
        let kind = id.split(':').next().unwrap();
        self.inks.iter().find(|(module, _)| *module == kind).map_or(self.ink, |(_, ink)| *ink)
    }
}

/// One colour at several strengths, as CSS's `color-mix(in srgb, c p%, transparent)`.
fn ink(trace: Color, second: f32, wash: f32, wash_end: f32) -> Ink {
    Ink { trace, trace2: trace.alpha(second), wash: trace.alpha(wash), wash_end: trace.alpha(wash_end) }
}

fn rgba(rgb: u32, a: f32) -> Color {
    Color::hex(rgb, a)
}

/// 记录纸: chart paper and ink. The panel is one ink until something runs
/// hot, and only then does the signal colour appear.
fn paper(dark: bool) -> Theme {
    let archivo = |size, weight| Font::new(Family::Archivo, size, weight);
    let (sheet, text, text2, text3, rule, wash, signal, shadow) = if dark {
        (0x14231F, 0xE7EEE7, 0xA1B0A8, 0x6B7B73, 0x263732, 0.08, 0xFF7B4A, (0x000000, 0.6, 0.3))
    } else {
        (0xEEF2EC, 0x16221E, 0x55645C, 0x8A9990, 0xD3DBD2, 0.07, 0xD23F1C, (0x0A1410, 0.35, 0.12))
    };
    let ink = Ink { trace: rgba(text, 1.0), trace2: rgba(text3, 1.0), wash: rgba(text, wash), wash_end: rgba(text, wash) };
    Theme {
        skin: Skin::Paper,
        dark,
        text: rgba(text, 1.0),
        text2: rgba(text2, 1.0),
        text3: rgba(text3, 1.0),
        rule: rgba(rule, 1.0),
        track: rgba(text, wash),
        signal: rgba(signal, 1.0),
        hover: rgba(text, if dark { 0.07 } else { 0.06 }),
        inks: Vec::new(),
        ink,
        body: archivo(13.0, 500.0),
        small: archivo(12.0, 500.0),
        title: archivo(13.0, 650.0),
        figure: archivo(42.0, 620.0).width(66.0).tracking(-0.01),
        figure_line: 0.84,
        unit: archivo(15.0, 500.0).width(80.0),
        value: archivo(13.0, 500.0).width(85.0),
        pad_x: 18.0,
        pad_top: 12.0,
        pad_bottom: 12.0,
        outer: (0.0, 0.0, 0.0),
        lane_gap: 0.0,
        column_gap: 0.0,
        bar_gap: 0.0,
        bar_height: 44.0,
        bar_pad: (18.0, 10.0),
        ruled: true,
        meter_height: 3.0,
        meter_radius: 0.0,
        thread_radius: 0.0,
        control_radius: 6.0,
        radius: 16.0,
        shadows: vec![
            Shadow { y: 18.0, blur: 40.0, spread: -12.0, color: rgba(shadow.0, shadow.1) },
            Shadow { y: 2.0, blur: 6.0, spread: 0.0, color: rgba(shadow.0, shadow.2) },
        ],
        margin: 40.0,
        inset: 0.0,
        entrance: Entrance::Beyond(40.0),
        paper: rgba(sheet, 1.0),
        luminosity: Color::CLEAR,
        tint: Color::CLEAR,
        stroke: rgba(rule, 1.0),
        footer: Color::CLEAR,
        glass: Color::CLEAR,
        glass_clear: 0.0,
        glass_frosted: 0.0,
        legibility: Color::CLEAR,
    }
}

/// The user's accent colour, in the shade the system pairs with the theme.
pub fn accent(dark: bool) -> Color {
    // Where the system will not say, Windows' default blue in that shade.
    Color::hex(crate::os::accent(dark).unwrap_or(if dark { 0x99EBFF } else { 0x005FB8 }), 1.0)
}

/// Windows 11, as the Start menu and Quick Settings draw it: one acrylic
/// sheet with an 8 px radius, a hairline stroke and a soft shadow; content
/// grouped by headings and space rather than boxes; a slightly darker footer;
/// the user's accent colour for anything that is data.
fn fluent(dark: bool) -> Theme {
    let segoe = |size, weight| Font::new(Family::Segoe, size, weight);
    let ink = ink(accent(dark), 0.5, 0.22, 0.0);
    let (on, a) = if dark { (0xFFFFFF, [1.0, 0.76, 0.5, 0.06, 0.14]) } else { (0x000000, [0.9, 0.6, 0.44, 0.06, 0.1]) };
    Theme {
        skin: Skin::Fluent,
        dark,
        text: rgba(on, a[0]),
        text2: rgba(on, a[1]),
        text3: rgba(on, a[2]),
        rule: rgba(on, a[3]),
        track: rgba(on, a[4]),
        signal: if dark { rgba(0xFF99A4, 1.0) } else { rgba(0xC42B1C, 1.0) },
        hover: if dark { rgba(0xFFFFFF, 0.06) } else { rgba(0x000000, 0.04) },
        inks: Vec::new(),
        ink,
        body: segoe(13.0, 500.0),
        small: segoe(12.0, 500.0),
        title: segoe(14.0, 600.0),
        figure: Font::new(Family::SegoeDisplay, 34.0, 600.0).tracking(-0.02),
        figure_line: 0.86,
        unit: segoe(15.0, 500.0),
        value: segoe(13.0, 500.0),
        pad_x: 16.0,
        pad_top: 10.0,
        pad_bottom: 12.0,
        outer: (10.0, 6.0, 6.0),
        lane_gap: 0.0,
        column_gap: 0.0,
        bar_gap: 0.0,
        bar_height: 52.0,
        bar_pad: (22.0, 14.0),
        ruled: false,
        meter_height: 4.0,
        meter_radius: 2.0,
        thread_radius: 1.0,
        control_radius: 4.0,
        radius: 8.0,
        shadows: if dark {
            vec![
                Shadow { y: 8.0, blur: 32.0, spread: 0.0, color: rgba(0, 0.4) },
                Shadow { y: 2.0, blur: 8.0, spread: 0.0, color: rgba(0, 0.25) },
            ]
        } else {
            vec![
                Shadow { y: 8.0, blur: 32.0, spread: 0.0, color: rgba(0, 0.18) },
                Shadow { y: 2.0, blur: 8.0, spread: 0.0, color: rgba(0, 0.08) },
            ]
        },
        margin: 32.0,
        inset: 12.0,
        entrance: Entrance::Slide(40.0),
        paper: Color::CLEAR,
        luminosity: if dark { rgba(0x2C2C2C, 0.96) } else { rgba(0xFCFCFC, 0.85) },
        tint: if dark { rgba(0x2C2C2C, 0.15) } else { rgba(0xFCFCFC, 0.0) },
        stroke: if dark { rgba(0xFFFFFF, 0.09) } else { rgba(0x000000, 0.1) },
        footer: rgba(0x000000, if dark { 0.18 } else { 0.035 }),
        glass: Color::CLEAR,
        glass_clear: 0.0,
        glass_frosted: 0.0,
        legibility: Color::CLEAR,
    }
}

/// 磨砂玻璃: separate pieces of clear glass floating over the desktop, each
/// bending what is behind it at its rim and catching light along its edge.
/// The theme sets the glass's colour; the busier or less fitting the
/// backdrop, the more it frosts, so text stays legible over anything.
fn glass(dark: bool) -> Theme {
    let inter = |size, weight| Font::new(Family::Inter, size, weight);
    let [blue, green, orange, purple, teal] = if dark {
        [0x3D9BFF, 0x30D158, 0xFF9F0A, 0xC77DFF, 0x5AC8F5]
    } else {
        [0x0A7AFF, 0x24A148, 0xE8890C, 0xA64FD6, 0x1491B8]
    };
    let lane = |rgb| ink(rgba(rgb, 1.0), 0.55, 0.34, 0.0);
    let (on, a) = if dark { (0xFFFFFF, [0.95, 0.68, 0.46, 0.1, 0.16]) } else { (0x000000, [0.86, 0.58, 0.42, 0.08, 0.1]) };
    Theme {
        skin: Skin::Glass,
        dark,
        text: rgba(on, a[0]),
        text2: rgba(on, a[1]),
        text3: rgba(on, a[2]),
        rule: rgba(on, a[3]),
        track: rgba(on, a[4]),
        signal: if dark { rgba(0xFF6961, 1.0) } else { rgba(0xFF3B30, 1.0) },
        hover: if dark { rgba(0xFFFFFF, 0.1) } else { rgba(0xFFFFFF, 0.35) },
        inks: vec![
            ("gpu", lane(green)),
            ("memory", lane(orange)),
            ("network", lane(purple)),
            ("disk", lane(teal)),
            ("storage", lane(teal)),
        ],
        ink: lane(blue),
        body: inter(13.0, 500.0),
        small: inter(12.0, 500.0),
        title: inter(13.0, 650.0),
        figure: inter(40.0, 600.0).tracking(-0.035),
        figure_line: 0.84,
        unit: inter(15.0, 500.0),
        value: inter(13.0, 500.0),
        pad_x: 18.0,
        pad_top: 14.0,
        pad_bottom: 14.0,
        outer: (0.0, 0.0, 0.0),
        lane_gap: 8.0,
        column_gap: 8.0,
        bar_gap: 8.0,
        bar_height: 48.0,
        bar_pad: (20.0, 8.0),
        ruled: false,
        meter_height: 6.0,
        meter_radius: 3.0,
        thread_radius: 2.0,
        control_radius: 16.0,
        radius: 24.0,
        shadows: if dark {
            vec![
                Shadow { y: 10.0, blur: 30.0, spread: 0.0, color: rgba(0, 0.3) },
                Shadow { y: 1.0, blur: 3.0, spread: 0.0, color: rgba(0, 0.2) },
            ]
        } else {
            vec![
                Shadow { y: 10.0, blur: 30.0, spread: 0.0, color: rgba(0, 0.12) },
                Shadow { y: 1.0, blur: 3.0, spread: 0.0, color: rgba(0, 0.08) },
            ]
        },
        margin: 24.0,
        inset: 10.0,
        entrance: Entrance::Slide(36.0),
        paper: Color::CLEAR,
        luminosity: Color::CLEAR,
        tint: Color::CLEAR,
        stroke: Color::CLEAR,
        footer: Color::CLEAR,
        glass: if dark { rgba(0x14141A, 1.0) } else { rgba(0xFFFFFF, 1.0) },
        glass_clear: if dark { 0.16 } else { 0.08 },
        glass_frosted: if dark { 0.68 } else { 0.78 },
        legibility: if dark { rgba(0x000000, 0.5) } else { rgba(0xFFFFFF, 0.6) },
    }
}

/// Whether the panel is dark: as chosen, as the system's apps are, or, to
/// follow the backdrop, as the desktop behind it is (`backdrop`, its mean
/// luminance 0–1, when the skin sees it).
pub fn is_dark(pref: ThemePref, backdrop: Option<f32>) -> bool {
    match pref {
        ThemePref::Light => false,
        ThemePref::Dark => true,
        ThemePref::Backdrop if backdrop.is_some() => backdrop.unwrap() <= 0.5,
        ThemePref::System | ThemePref::Backdrop => crate::os::apps_dark(),
    }
}
