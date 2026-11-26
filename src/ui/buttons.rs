//! The buttons a widget and the overlay show while the pointer is over
//! them: the same ones, in the same order, each saying what it does to a
//! pointer that stays on it.

use std::time::Duration;

use super::canvas::{Canvas, Color, Fill, Point};
use super::icons::Icon;
use super::text::Lang;

/// What one does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Button {
    /// The window in front taken for a game, or no longer.
    Game,
    Settings,
    /// Kept where it is.
    Pin,
    /// Clicks pass through it to what is behind.
    Through,
    Close,
    /// The music shown over the whole screen.
    Full,
}

/// All of them, as they stand in their row.
pub const ALL: [Button; 5] = [Button::Game, Button::Settings, Button::Pin, Button::Through, Button::Close];

/// A button's width, and the room between two (DIPs).
pub const SIZE: f32 = 22.0;
pub const GAP: f32 = 4.0;

/// How long the pointer stays over what lets clicks through before the one
/// button that ends that shows: a pointer crossing it does not bring it.
pub const LINGER: Duration = Duration::from_millis(500);

impl Button {
    fn icon(self) -> Icon {
        match self {
            Button::Game => Icon::Game,
            Button::Settings => Icon::Settings,
            Button::Pin => Icon::Pin,
            Button::Through => Icon::Pointer,
            Button::Close => Icon::Close,
            Button::Full => Icon::Maximize,
        }
    }

    /// What it says of itself, `lit` or not, on a widget or on the overlay.
    pub fn says(self, lit: bool, lang: Lang, overlay: bool) -> &'static str {
        let p = |zh, en| lang.pick(zh, en);
        match (self, lit) {
            (Button::Game, false) => p("显示当前窗口的帧率", "Show the active window's frame rate"),
            (Button::Game, true) => p("停止显示帧率", "Stop showing the frame rate"),
            (Button::Settings, _) => p("设置", "Settings"),
            (Button::Pin, false) if overlay => p("锁定位置", "Lock position"),
            (Button::Pin, true) if overlay => p("解除锁定", "Unlock position"),
            (Button::Pin, false) => p("固定位置和大小", "Pin position and size"),
            (Button::Pin, true) => p("取消固定", "Unpin"),
            (Button::Through, false) => p("鼠标穿透", "Click-through"),
            (Button::Through, true) => p("关闭鼠标穿透", "Turn off click-through"),
            (Button::Close, _) if overlay => p("关闭悬浮窗", "Close overlay"),
            (Button::Close, _) => p("收起", "Hide"),
            (Button::Full, _) => p("全屏", "Full screen"),
        }
    }
}

/// A button's `plate`, `hovered` (the pointer over it), a little toward its `mark`, else as it is: told from it as it
/// rests, and far from it lit (inverted), as the bar's buttons keep the two apart (see `view::panel_button`).
fn toward(plate: Color, mark: Color, hovered: bool) -> Color {
    let by = if hovered { 0.14 } else { 0.0 };
    Color { r: plate.r + (mark.r - plate.r) * by, g: plate.g + (mark.g - plate.g) * by, b: plate.b + (mark.b - plate.b) * by, a: plate.a }
}

/// Draws `button` round `centre` beside a widget whose ground is `ground` (straight RGB): a plate of that colour,
/// a little lighter, the mark white; lit (on), inverted; `hovered` and not lit, tinted (see `toward`).
#[allow(clippy::too_many_arguments)]
pub fn paint_on(frame: &dyn Canvas, centre: Point, button: Button, lit: bool, hovered: bool, [r, g, b]: [f32; 3], grown: f32) {
    let lighter = |c: f32| c + (1.0 - c) * 0.12;
    let plate = Color { r: lighter(r), g: lighter(g), b: lighter(b), a: 0.94 };
    let mark = Color::hex(0xFFFFFF, 1.0);
    let (ground, ink) = if lit { (mark, plate.alpha(1.0)) } else { (toward(plate, mark, hovered), mark) };
    let size = SIZE * grown;
    frame.fill_circle(ground, centre, size / 2.0);
    frame.stroke_rounded(Fill::Solid(Color::hex(0xFFFFFF, 0.22)), centre.x - size / 2.0, centre.y - size / 2.0, size, size, size / 2.0, 1.0);
    frame.icon(button.icon(), centre, 13.0 * grown, ink);
}

/// Draws `button` round `centre`: a plate of its own over whatever is under
/// it, light on a light ground and dark on a dark one; lit (on), inverted; `hovered` and not lit, tinted (see
/// `toward`).
#[allow(clippy::too_many_arguments)]
pub fn paint(frame: &dyn Canvas, centre: Point, button: Button, lit: bool, hovered: bool, dark: bool, rim: Color, grown: f32) {
    let (light, deep) = (Color::hex(0xFFFFFF, 0.94), Color::hex(0x1E1E1E, 0.94));
    let (plate, mark) = if dark { (deep, Color::hex(0xFFFFFF, 1.0)) } else { (light, Color::hex(0x1A1A1A, 1.0)) };
    let (ground, ink) = if lit { (mark, plate) } else { (toward(plate, mark, hovered), mark) };
    let size = SIZE * grown;
    frame.fill_circle(ground, centre, size / 2.0);
    frame.stroke_rounded(Fill::Solid(rim), centre.x - size / 2.0, centre.y - size / 2.0, size, size, size / 2.0, 1.0);
    frame.icon(button.icon(), centre, 13.0 * grown, ink);
}
