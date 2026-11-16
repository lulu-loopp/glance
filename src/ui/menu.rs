//! A menu as Windows 11 draws its own (File Explorer's), or in the skin of
//! what it is opened on (see `Look`): a rounded sheet with a soft shadow, a
//! row for each choice with its icon, a light highlight under the pointer, a
//! tick for a choice that is on, thin rules between groups. Shown at the pointer, it is up until a choice is made or
//! it is left (a click elsewhere, Esc, Alt, another window to the front),
//! as a system menu is. A choice may open choices of its own beside it (a chevron after it), shown while it is lit; a
//! choice may say more under it, and may be shown dimmed, not to be chosen (what is under it saying why).

use std::borrow::Cow;
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{SetCapture, VIRTUAL_KEY, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RETURN, VK_RIGHT, VK_UP};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, IsIconic, PostMessageW, SetForegroundWindow, MSG, SC_RESTORE, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP,
    WM_SYSCOMMAND, WM_SYSKEYDOWN,
};

use super::canvas::{Align, Canvas, Color, Family, Fill, Font, Point};
use super::gfx::{self, Surface};
use super::icons::Icon;
use super::window::{Window, CAPTURE_LOST};

/// How a menu looks: in the skin of what it is opened on (`skin`: the panel's, a widget's, the page beside), or as
/// Windows' own (none: the tray's, the overlay's, which are Windows' things); light or `dark`. On glass, as frosted as
/// the glass it is opened on is drawn (`frost`, 0–1), so that it is a piece of that glass: as clear over a calm
/// desktop, as tinted, its colours as lifted. As large as what it is opened on is drawn (`zoom`: the panel or the
/// widget made larger or smaller on its screen, 1 as its screen's scale has it).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Look {
    pub skin: Option<super::theme::Skin>,
    pub dark: bool,
    pub frost: f32,
    pub zoom: f32,
}

impl Look {
    /// As Windows' own menus, light or dark as the apps are.
    pub fn system() -> Look {
        Look { skin: None, dark: crate::os::apps_dark(), frost: 1.0, zoom: 1.0 }
    }

    /// In `skin`, light or `dark`, as frosted as glass is at its most (glass with no picture of what is behind it is
    /// drawn so: see `skins::glass_tint`).
    pub fn of(skin: super::theme::Skin, dark: bool) -> Look {
        Look { skin: Some(skin), dark, frost: 1.0, zoom: 1.0 }
    }

    /// The same, opened on glass drawn `frost` frosted: `behind`, with a picture of what is behind it to frost (without
    /// one, glass is drawn as at its most frosted).
    pub fn frosted(self, frost: f32, behind: bool) -> Look {
        Look { frost: if behind { frost.clamp(0.0, 1.0) } else { 1.0 }, ..self }
    }

    /// The same, opened on what is drawn `zoom` times as large as its screen's scale has it.
    pub fn zoomed(self, zoom: f32) -> Look {
        Look { zoom, ..self }
    }

    /// Physical px to a DIP of it on a screen of `scale`: as many as on what it is opened on.
    pub fn px(self, scale: f32) -> f32 {
        scale * self.zoom
    }
}

/// What a menu's window shows: what is drawn, or (glass's, Windows 11's) that over a pane of the screen behind it,
/// frosted by the compositor as the screen changes (see `glass::Frosted`), as the panel's lanes are frosted over the
/// desktop. Whatever else is shown as a menu is (the picker of a to-do's time) is shown on one too.
pub enum Drawn {
    Plain(Surface),
    Frosted(super::glass::Frosted),
}

impl Drawn {
    /// What the window `hwnd` shows, a menu in `look`: glass's over the screen frosted as the glass it is opened on is,
    /// blurred and its colour lifted as its lanes are, Windows 11's as its acrylic is (each where the system frosts what
    /// is behind a window), the rest drawn alone.
    pub fn new(gfx: &Rc<gfx::Gfx>, hwnd: HWND, look: Look) -> windows::core::Result<Drawn> {
        if let Some((frosting, frost)) = frosting(look) {
            if let Ok(mut frosted) = super::glass::Frosted::new(gfx, hwnd, frosting) {
                frosted.set_frost(frost);
                return Ok(Drawn::Frosted(frosted));
            }
        }
        Surface::new(gfx, hwnd).map(Drawn::Plain)
    }

    /// The room round a sheet on it for its shadows (DIPs): glass's as far as its lanes' reach.
    pub fn room(&self, look: Look) -> f32 {
        match (self, look.skin) {
            (Drawn::Frosted(_), Some(super::theme::Skin::Glass)) => super::theme::room(&super::theme::Theme::new(super::theme::Skin::Glass, look.dark).shadows).max(SHADOW),
            _ => SHADOW,
        }
    }

    /// Draws on it, `size` (physical px) at `px` to a DIP, a menu's sheet `width` × `height` in `look`, `room` in from
    /// its corner: what is `on` it under what is written on it (a choice lit, rules), then its `words` (what is written,
    /// and drawn as words are), each from the sheet's corner and told whether it is frosted. Glass's is a piece of the
    /// glass it is opened on, drawn as its lanes are (their shadows, the light inside their edge and along it), its
    /// words kept in `layer` while `key` is what they show and ringed by the glass's faint halo, read over it as
    /// clearly as the lanes' are.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, gfx: &Rc<gfx::Gfx>, size: (u32, u32), px: f32, look: Look, room: f32, (width, height): (f32, f32), layer: &mut gfx::Layer, key: u64, on: impl FnOnce(&gfx::Frame, bool), words: impl FnOnce(&gfx::Frame, bool)) -> windows::core::Result<()> {
        match self {
            Drawn::Plain(surface) => surface.draw(gfx, size, px, |frame| {
                frame.origin(room, room);
                let _ = sheet(frame, look, (0.0, 0.0, width, height), false);
                on(frame, false);
                words(frame, false);
            }),
            Drawn::Frosted(frosted) => {
                let radius = sheet_radius(look, true, height);
                let pane = (room * px, room * px, width * px, height * px, radius * px);
                let halo = (look.skin == Some(super::theme::Skin::Glass)).then(|| super::theme::Theme::new(super::theme::Skin::Glass, look.dark).legibility);
                frosted.draw(gfx, size, px, pane, |frame| {
                    frame.origin(room, room);
                    if let Err(error) = sheet(frame, look, (0.0, 0.0, width, height), true) {
                        gfx::failed("menu", gfx, Some(&error));
                    }
                    on(frame, true);
                    let laid = layer.draw(frame, key, windows_numerics::Matrix3x2::identity(), (0.0, 0.0, width, height), px, halo, |frame| {
                        words(frame, true);
                        Ok(())
                    });
                    if let Err(error) = laid {
                        gfx::failed("menu", gfx, Some(&error));
                    }
                })
            }
        }
    }
}

/// How what is drawn in `look` over the screen is frosted, where the system frosts what is behind a window: glass's
/// blurred and its colour lifted as its lanes are, as frosted as the glass it is opened on is drawn; Windows 11's as
/// its acrylic is; the rest not (chart paper's sheets are whole).
pub fn frosting(look: Look) -> Option<((f32, f32), f32)> {
    use super::skins::{ACRYLIC_BLUR, ACRYLIC_SATURATION, FROST_BLUR, GLASS_SATURATION};
    match look.skin {
        Some(super::theme::Skin::Glass) => Some(((FROST_BLUR, GLASS_SATURATION), look.frost)),
        Some(super::theme::Skin::Fluent) => Some(((ACRYLIC_BLUR, ACRYLIC_SATURATION), 1.0)),
        _ => None,
    }
}

/// How round a sheet `height` tall in `look` is (DIPs), over the screen `frosted` or not.
pub fn sheet_radius(look: Look, frosted: bool, height: f32) -> f32 {
    Colours::new(look, frosted).radius.min(height / 2.0)
}

/// A sheet (`x`, `y`, `width`, `height`) in `look`, as a menu's is, from where the frame is placed: over the screen
/// `frosted` by the compositor (see `Drawn`), glass's a piece of the glass it is opened on (its shadows, its tint as
/// frosted as that glass is, the light inside its edge and along it), Windows 11's a little clear over its acrylic;
/// else whole over what it covers. What is drawn on it is to be drawn as the plate it gives says.
pub fn sheet(frame: &gfx::Frame, look: Look, (x, y, width, height): (f32, f32, f32, f32), frosted: bool) -> windows::core::Result<Plate> {
    let colours = Colours::new(look, frosted);
    let radius = colours.radius.min(height / 2.0);
    match (frosted, look.skin) {
        (true, Some(super::theme::Skin::Glass)) => {
            let theme = super::theme::Theme::new(super::theme::Skin::Glass, look.dark);
            let mut placed = windows_numerics::Matrix3x2::default();
            unsafe { frame.dc.GetTransform(&mut placed) };
            let drawn = super::skins::glass_sheet(frame, &theme, super::view::Rect { x, y, w: width, h: height }, radius, look.frost);
            unsafe { frame.dc.SetTransform(&placed) };
            drawn?;
        }
        _ => paint_plate(frame, &colours, (x, y, width, height), radius),
    }
    Ok(Plate { icon: colours.icon, highlight: colours.highlight, radius })
}

/// A sheet's plate in `colours`, `radius` round: its shadow, its ground, its edge, and (glass's) the light along its top,
/// as its lanes are lit.
fn paint_plate(frame: &dyn Canvas, colours: &Colours, (x, y, width, height): (f32, f32, f32, f32), radius: f32) {
    frame.shadow(colours.shadow, x, y, width, height, radius, SHADOW_BLUR, SHADOW_DROP);
    frame.fill_rounded(colours.sheet, x, y, width, height, radius);
    frame.stroke_rounded(Fill::Solid(colours.stroke), x, y, width, height, radius, 1.0);
    if let Some(rim) = colours.rim {
        frame.stroke_rounded(Fill::Down { top: y, from: rim, bottom: y + height, to: rim.alpha(rim.a * 0.3) }, x + 1.0, y + 1.0, width - 2.0, height - 2.0, radius - 1.0, 1.0);
    }
}

/// What stands before a choice: an icon, or a colour, as a dot of it (a note's colours: an icon said nothing of which).
#[derive(Clone, Copy)]
pub enum Mark {
    Icon(Icon),
    Colour(Color),
}

/// How large a colour's dot is across (DIPs): as a note's dot by its name, a little smaller than an icon.
const DOT: f32 = 12.0;

/// One choice: what it says (and, smaller, under it), what stands before it, whether it is on (ticked), whether a rule
/// sets it off from the choices before it, whether it is shown dimmed (not to be chosen), and the choices it opens
/// beside it (none: it is chosen itself).
#[derive(Clone)]
pub struct Item<'a> {
    pub label: Cow<'a, str>,
    pub detail: Option<Cow<'a, str>>,
    pub mark: Option<Mark>,
    pub checked: bool,
    pub rule_before: bool,
    pub dimmed: bool,
    pub more: Vec<Item<'a>>,
}

impl<'a> Item<'a> {
    /// A choice saying `label`, `mark` before it, ticked where `checked`, a rule before it where `rule_before`.
    pub fn new(label: impl Into<Cow<'a, str>>, mark: Option<Mark>, checked: bool, rule_before: bool) -> Item<'a> {
        Item { label: label.into(), detail: None, mark, checked, rule_before, dimmed: false, more: Vec::new() }
    }

    /// How tall its row is: taller where it says more under it.
    fn tall(&self) -> f32 {
        if self.detail.is_some() {
            ROW_TWO
        } else {
            ROW
        }
    }

    /// Whether it can be chosen, or lit: not dimmed.
    fn open(&self) -> bool {
        !self.dimmed
    }
}

/// Its measures (DIPs), as Windows 11's menus have them; a row saying more under its words, and the size of those.
const ROW: f32 = 32.0;
const ROW_TWO: f32 = 46.0;
const DETAIL: f32 = 12.0;
/// How far a menu of choices opened beside one comes over the menu it is opened from (DIPs).
const SUB_OVER: f32 = 2.0;
const RULE: f32 = 9.0;
const INSET: f32 = 4.0;
const RADIUS: f32 = 8.0;
const ICON: f32 = 16.0;
const TEXT_LEFT: f32 = 44.0;
const MIN_WIDTH: f32 = 200.0;
/// The shadow under the sheet: how blurred (the Gaussian's deviation) and how far down it falls.
const SHADOW_BLUR: f32 = 12.0;
const SHADOW_DROP: f32 = 6.0;
/// Room round the sheet for its shadow: as far as it reaches (twice its blur's deviation, past how far down it falls,
/// as `theme::room` has it), so that a window it is drawn in never cuts it off.
pub const SHADOW: f32 = 2.0 * SHADOW_BLUR + SHADOW_DROP;

/// Counts menus shown and gone: a capture of the desktop taken while one
/// was up may hold it (see `Panel::collect_behind`).
pub static SHOWN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
const FONT: Font = Font::new(Family::Segoe, 14.0, 400.0);

impl Look {
    /// What its choices are written in: Windows' own face (Windows 11's look, the system's), or the skin's (glass's
    /// Inter, chart paper's Archivo), its size.
    pub(super) fn font(self) -> Font {
        match self.skin {
            Some(skin @ (super::theme::Skin::Glass | super::theme::Skin::Paper)) => Font { size: FONT.size, weight: FONT.weight, ..super::theme::Theme::new(skin, self.dark).body },
            _ => FONT,
        }
    }

    /// What is said under a choice is written in: its face, smaller.
    fn detail_font(self) -> Font {
        Font { size: DETAIL, ..self.font() }
    }
}

/// What a menu made of a message for its window.
pub enum Handled {
    /// None of its own: for the window, as any other message.
    Not,
    /// Taken, the menu still up.
    Taken,
    /// Done with: the index of the choice made, if one was (none, left).
    Done(Option<usize>),
}

/// A menu up: shown at the pointer, or where it is asked, till a choice is made in it or it is left. Run by the loop
/// of the thread that showed it, as any other window of it is (its messages handed to `handle`, and `left` asked
/// after them): what else that thread shows goes on meanwhile. (Run by a loop of its own, it held back every other
/// window of the thread, the panel's live readings, the widgets' and the music full screen, till it was gone.)
pub struct Menu {
    gfx: Rc<gfx::Gfx>,
    window: Window,
    surface: Drawn,
    items: Vec<Item<'static>>,
    sheet: Sheet,
    look: Look,
    scale: f32,
    /// The room round its sheet for its shadows (DIPs).
    room: f32,
    /// Its window's size (physical px) and where it is.
    size: (i32, i32),
    rect: RECT,
    lit: Option<usize>,
    /// What was in front before it, given the front back as it goes.
    before: HWND,
    /// Opened with the panel up (see `over_the_panel`).
    with_the_panel: bool,
    /// Its choices as drawn on glass, ringed by the glass's halo as the lanes' words are (see `draw`).
    choices: gfx::Layer,
    /// The choices one of its own opens, up beside it (which, and that menu), and whether the keys go to them (moved
    /// into by the right arrow, or the pointer).
    sub: Option<(usize, Box<Menu>)>,
    in_sub: bool,
    /// Where the pointer was last told to be (in its window's px): Windows tells it again where it has not moved, as
    /// a window under it changes, which is no move of its (the keys' choice is kept).
    pointer: Option<(i32, i32)>,
    /// Shown till it goes (see `life`): counted before its surface is made, so that it is let go of after it.
    _showing: super::life::Showing,
}

impl Menu {
    /// Shows `items` at the pointer, light or `dark`.
    pub fn open(items: Vec<Item<'static>>, look: Look, with_the_panel: bool) -> Option<Menu> {
        let mut cursor = POINT::default();
        unsafe { GetCursorPos(&mut cursor) }.ok()?;
        Menu::open_at(items, look, cursor, false, with_the_panel)
    }

    /// Shows `items` with a corner at `at` (physical px), its bottom there for `above` (as a menu from the taskbar
    /// opens), light or `dark`. None where it could not be shown, or was not let to the front.
    pub fn open_at(items: Vec<Item<'static>>, look: Look, at: POINT, above: bool, with_the_panel: bool) -> Option<Menu> {
        // At the pointer, turned back where it would leave its screen.
        let mut menu = Menu::made(items, look, at, Window::activating(), with_the_panel, |(w, h), margin, _, work| {
            let mut left = at.x - margin;
            if left + w - margin > work.right {
                left = at.x - w + margin;
            }
            let mut top = at.y - margin;
            if above || top + h - margin > work.bottom {
                top = at.y - h + margin;
            }
            (left, top)
        })?;
        // In front, with the keyboard and the mouse, as a system menu is: its keys come to it, not to the program
        // behind; a click anywhere comes to it too; and it is left as anything else comes to the front.
        menu.before = unsafe { GetForegroundWindow() };
        // Not let to the front (Windows keeps it for the program in use): it could neither hear its keys nor know it
        // was left, so it goes.
        if !unsafe { SetForegroundWindow(menu.window.hwnd) }.as_bool() {
            crate::journal::note("menu: not let to the front; not shown");
            menu.close();
            return None;
        }
        unsafe { SetCapture(menu.window.hwnd) };
        Some(menu)
    }

    /// A menu of `items` in `look`, its window (`window`) made, drawn and put where `place` says from its size and the
    /// room round its sheet for its shadow (physical px), the scale it is drawn at and the work area of the screen
    /// `at` is on (its top left corner, kept on that screen), and shown, not yet in front. None where it could not be.
    fn made(items: Vec<Item<'static>>, look: Look, at: POINT, window: windows::core::Result<Window>, with_the_panel: bool, place: impl FnOnce((i32, i32), i32, f32, RECT) -> (i32, i32)) -> Option<Menu> {
        let contact = crate::mouse::monitor_at(at)?;
        // (On the device for its screen: see `gfx::on_screen`.)
        let gfx = gfx::for_area(contact.monitor).ok()?;
        let showing = super::life::Showing::of(&gfx);
        let window = window.ok()?;
        let surface = match Drawn::new(&gfx, window.hwnd, look) {
            Ok(surface) => surface,
            Err(error) => {
                window.destroy();
                gfx::failed("menu", &gfx, Some(&error));
                return None;
            }
        };
        let scale = look.px(contact.scale);
        let sheet = Sheet::new(&gfx, &items, look);
        let room = surface.room(look);
        let (w, h) = (((sheet.width + 2.0 * room) * scale) as i32, ((sheet.height + 2.0 * room) * scale) as i32);
        let margin = (room * scale) as i32;
        let work = contact.work;
        let (left, top) = place((w, h), margin, scale, work);
        let left = left.clamp(work.left - margin, (work.right - w + margin).max(work.left - margin));
        let top = top.clamp(work.top - margin, (work.bottom - h + margin).max(work.top - margin));
        let rect = RECT { left, top, right: left + w, bottom: top + h };
        let mut menu = Menu { gfx, window, surface, items, sheet, look, scale, room, size: (w, h), rect, lit: None, before: HWND::default(), with_the_panel, choices: gfx::Layer::default(), sub: None, in_sub: false, pointer: None, _showing: showing };
        if !menu.draw() {
            menu.window.destroy();
            return None;
        }
        menu.window.place(rect);
        menu.window.show();
        // Up, and gone: a widget that took the desktop with it there takes it again.
        crate::widget::own_window_changed(rect);
        SHOWN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Some(menu)
    }

    /// Whether `hwnd` is its window (or that of the choices up beside it).
    pub fn owns(&self, hwnd: HWND) -> bool {
        self.window.hwnd == hwnd || self.sub.as_ref().is_some_and(|(_, sub)| sub.owns(hwnd))
    }

    /// How many choices are its own (those opened beside them not counted).
    pub fn rows(&self) -> usize {
        self.items.len()
    }

    /// The choices its choice `row` opens now `more` (as what they list changed while it is up): drawn so beside it at
    /// once, where they are up, the same one lit (as far as there is one).
    pub fn set_more(&mut self, row: usize, more: Vec<Item<'static>>) {
        self.items[row].more = more;
        if self.sub.as_ref().is_none_or(|(at, _)| *at != row) {
            return;
        }
        let (in_sub, lit) = (self.in_sub, self.sub.as_ref().and_then(|(_, sub)| sub.lit));
        if let Some((_, sub)) = self.sub.take() {
            sub.close();
        }
        self.follow_sub(Some(row));
        if in_sub && self.sub.is_some() {
            self.in_sub = true;
            self.sub_light(|sub| lit.filter(|at| *at < sub.items.len()));
        }
    }

    /// What a choice made in the choices that the choice `row` opens is told as, the `at`th of them: after all the
    /// menu's own, the first choice `row` opens right after those of the choices before it that open some.
    pub fn sub_choice(items: &[Item], row: usize, at: usize) -> usize {
        items.len() + items[..row].iter().map(|item| item.more.len()).sum::<usize>() + at
    }

    /// Takes a message for its window, if it is one of its own. (The mouse is its window's while it is up, the
    /// choices beside it too: where the pointer is, is told it as its window has it.)
    pub fn handle(&mut self, msg: &MSG) -> Handled {
        let (x, y) = ((msg.lParam.0 & 0xFFFF) as i16 as i32, ((msg.lParam.0 >> 16) & 0xFFFF) as i16 as i32);
        // Where the pointer is in the choices up beside it, where it is on their sheet.
        let in_sub = self.sub.as_ref().map(|(_, sub)| (x + self.rect.left - sub.rect.left, y + self.rect.top - sub.rect.top)).filter(|(sx, sy)| self.sub.as_ref().is_some_and(|(_, sub)| sub.inside(*sx, *sy)));
        match msg.message {
            WM_MOUSEMOVE if self.pointer == Some((x, y)) => Handled::Taken,
            WM_MOUSEMOVE => {
                self.pointer = Some((x, y));
                match in_sub {
                    Some((sx, sy)) => {
                        self.in_sub = true;
                        self.sub_light(|sub| sub.row_at(sx, sy))
                    }
                    None => {
                        let row = self.row_at(x, y);
                        // (Back on its own choices: the keys there, and nothing lit beside it.)
                        if row.is_some() {
                            if std::mem::take(&mut self.in_sub) {
                                self.sub_light(|_| None);
                            }
                            self.follow_sub(row);
                        }
                        if let Some((_, sub)) = self.sub.as_mut() {
                            if sub.lit.is_some() && row.is_none() {
                                return Handled::Taken;
                            }
                        }
                        self.light(row)
                    }
                }
            }
            // A press outside leaves it; inside, the choice is made as the button comes up.
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN if !self.inside(x, y) && in_sub.is_none() => Handled::Done(None),
            WM_LBUTTONUP | WM_RBUTTONUP if in_sub.is_some() => {
                let (sx, sy) = in_sub.unwrap_or_default();
                let Some((row, sub)) = self.sub.as_ref() else { return Handled::Taken };
                match sub.row_at(sx, sy).filter(|at| sub.items[*at].open()) {
                    Some(at) => Handled::Done(Some(Menu::sub_choice(&self.items, *row, at))),
                    None => Handled::Taken,
                }
            }
            WM_LBUTTONUP | WM_RBUTTONUP if self.inside(x, y) => match self.row_at(x, y) {
                Some(row) if self.items[row].open() && self.items[row].more.is_empty() => Handled::Done(Some(row)),
                _ => Handled::Taken,
            },
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONUP => Handled::Taken,
            WM_KEYDOWN => self.key(VIRTUAL_KEY(msg.wParam.0 as u16)),
            // Alt, as with a system menu, or the mouse taken from it.
            WM_SYSKEYDOWN | CAPTURE_LOST => Handled::Done(None),
            _ => Handled::Not,
        }
    }

    /// A key down: the arrows go up and down among the choices that can be chosen (in those beside it, where the keys
    /// are there), the right arrow and Enter into the choices a choice opens, the left arrow and Esc out of them again
    /// (Esc leaves it, where they are not up); Enter makes the choice lit.
    fn key(&mut self, key: VIRTUAL_KEY) -> Handled {
        if self.in_sub {
            return match key {
                VK_ESCAPE | VK_LEFT => {
                    self.in_sub = false;
                    if let Some((_, sub)) = self.sub.as_mut() {
                        sub.light(None);
                    }
                    Handled::Taken
                }
                VK_UP | VK_DOWN => self.sub_light(|sub| sub.step(sub.lit, key == VK_UP)),
                VK_RETURN => match self.sub.as_ref().and_then(|(row, sub)| Some((*row, sub.lit?))) {
                    Some((row, at)) => Handled::Done(Some(Menu::sub_choice(&self.items, row, at))),
                    None => Handled::Taken,
                },
                _ => Handled::Taken,
            };
        }
        match key {
            VK_ESCAPE => Handled::Done(None),
            VK_UP | VK_DOWN => {
                let row = self.step(self.lit, key == VK_UP);
                self.follow_sub(row);
                self.light(row)
            }
            VK_RIGHT | VK_RETURN if self.lit.is_some_and(|row| !self.items[row].more.is_empty()) => {
                self.follow_sub(self.lit);
                if self.sub.is_none() {
                    return Handled::Taken;
                }
                self.in_sub = true;
                self.sub_light(|sub| sub.step(None, false))
            }
            VK_RETURN if self.lit.is_some() => Handled::Done(self.lit),
            _ => Handled::Taken,
        }
    }

    /// The choice after `from` (before it, `up`) that can be chosen, round from the other end (none lit: the first, or
    /// the last); none where none can be.
    fn step(&self, from: Option<usize>, up: bool) -> Option<usize> {
        let n = self.items.len();
        (1..=n)
            .map(|k| match (from, up) {
                (None, true) => n - k,
                (None, false) => k - 1,
                (Some(i), true) => (i + n * 2 - k) % n,
                (Some(i), false) => (i + k) % n,
            })
            .find(|row| self.items[*row].open())
    }

    /// The choices `row` opens up beside it (those up of another closed first), where it opens some; none where it
    /// opens none.
    fn follow_sub(&mut self, row: Option<usize>) {
        if self.sub.as_ref().map(|(at, _)| *at) == row.filter(|row| !self.items[*row].more.is_empty()) {
            return;
        }
        if let Some((_, sub)) = self.sub.take() {
            sub.close();
        }
        self.in_sub = false;
        let Some(row) = row.filter(|row| !self.items[*row].more.is_empty() && self.items[*row].open()) else { return };
        let Some(beside) = self.row_on_screen(row) else { return };
        let (inset, over) = ((INSET * self.scale).round() as i32, (SUB_OVER * self.scale).round() as i32);
        let sub = Menu::made(self.items[row].more.clone(), self.look, POINT { x: beside.right, y: beside.top }, Window::new(), self.with_the_panel, |(w, _), margin, _, work| {
            // Beside the menu's sheet, its first choice level with the one it is opened from: on its right, or its
            // left where the right would leave the screen.
            let right = beside.right - over - margin;
            let left = if right + w - margin > work.right { beside.left + over + margin - w } else { right };
            (left, beside.top - inset - margin)
        });
        if let Some(sub) = sub {
            self.sub = Some((row, Box::new(sub)));
        }
    }

    /// The choice that `which` says, in the choices up beside it, lit (the keys there): those closed where they could
    /// not be drawn so.
    fn sub_light(&mut self, which: impl FnOnce(&Menu) -> Option<usize>) -> Handled {
        let Some((_, sub)) = self.sub.as_mut() else { return Handled::Taken };
        let row = which(sub);
        if let Handled::Done(_) = sub.light(row) {
            if let Some((_, sub)) = self.sub.take() {
                sub.close();
            }
            self.in_sub = false;
        }
        Handled::Taken
    }

    /// Whether it was opened with the panel up: one that was not is left as the panel comes (by its shortcut, an
    /// edge), which would cover it and take what it was opened for.
    pub fn over_the_panel(&self) -> bool {
        self.with_the_panel
    }

    /// Its window.
    pub fn hwnd(&self) -> HWND {
        self.window.hwnd
    }

    /// Whether it has been left by something else coming to the front.
    pub fn left(&self) -> bool {
        (unsafe { GetForegroundWindow() }) != self.window.hwnd
    }

    /// Gone: the front given back where it was, unless something else has taken it.
    pub fn close(mut self) {
        if let Some((_, sub)) = self.sub.take() {
            sub.close();
        }
        let held = !self.left();
        super::window::let_go_of_capture(self.window.hwnd);
        self.window.destroy();
        crate::widget::own_window_changed(self.rect);
        SHOWN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // (A game that went down, a fullscreen one stepping out as the menu came, is restored as a click on its
        // taskbar button would.)
        if held && !self.before.is_invalid() {
            unsafe {
                let _ = SetForegroundWindow(self.before);
                if IsIconic(self.before).as_bool() {
                    let _ = PostMessageW(Some(self.before), WM_SYSCOMMAND, WPARAM(SC_RESTORE as usize), LPARAM(0));
                }
            }
        }
    }

    /// The choice `row` lit (left, where it could not be drawn so). One dimmed is not lit.
    fn light(&mut self, row: Option<usize>) -> Handled {
        let row = row.filter(|row| self.items[*row].open());
        if row != self.lit {
            self.lit = row;
            if !self.draw() {
                return Handled::Done(None);
            }
        }
        Handled::Taken
    }

    fn draw(&mut self) -> bool {
        let (sheet, items, look, lit) = (&self.sheet, &self.items, self.look, self.lit);
        let size = (self.size.0 as u32, self.size.1 as u32);
        // (Its choices' words are as they were whichever is lit.)
        let drawn = self.surface.draw(&self.gfx, size, self.scale, look, self.room, (sheet.width, sheet.height), &mut self.choices, 0, |frame, frosted| sheet.paint_rows(frame, items, look, lit, frosted), |frame, frosted| sheet.paint_choices(frame, items, look, frosted));
        if let Err(error) = &drawn {
            gfx::failed("menu", &self.gfx, Some(error));
        }
        drawn.is_ok()
    }

    /// Which choice a point in the window (physical px) is on.
    fn row_at(&self, x: i32, y: i32) -> Option<usize> {
        let (x, y) = (x as f32 / self.scale - self.room, y as f32 / self.scale - self.room);
        if x < 0.0 || x > self.sheet.width {
            return None;
        }
        self.sheet.tops.iter().zip(&self.items).position(|(top, item)| y >= *top && y < top + item.tall())
    }

    /// Where the choice `row` is on the screen (physical px).
    pub fn row_on_screen(&self, row: usize) -> Option<RECT> {
        let top = *self.sheet.tops.get(row)?;
        let tall = self.items.get(row)?.tall();
        let px = |dips: f32| (dips * self.scale).round() as i32;
        Some(RECT { left: self.rect.left + px(self.room), top: self.rect.top + px(self.room + top), right: self.rect.left + px(self.room + self.sheet.width), bottom: self.rect.top + px(self.room + top + tall) })
    }

    fn inside(&self, x: i32, y: i32) -> bool {
        let (x, y) = (x as f32 / self.scale - self.room, y as f32 / self.scale - self.room);
        x >= 0.0 && x <= self.sheet.width && y >= 0.0 && y <= self.sheet.height
    }
}

/// A menu's sheet laid out for its items: its size (DIPs) and where each
/// row starts.
pub struct Sheet {
    pub width: f32,
    pub height: f32,
    tops: Vec<f32>,
}

impl Sheet {
    pub fn new(gfx: &gfx::Gfx, items: &[Item], look: Look) -> Self {
        // As wide as its longest choice (or what is said under one).
        let detail = look.detail_font();
        let widest = items.iter().map(|item| gfx.measure(&item.label, look.font()).max(item.detail.as_deref().map_or(0.0, |words| gfx.measure(words, detail)))).fold(0.0, f32::max);
        let width = (TEXT_LEFT + widest + 48.0).max(MIN_WIDTH);
        let tops: Vec<f32> = items
            .iter()
            .scan(INSET, |y, item| {
                *y += if item.rule_before { RULE } else { 0.0 };
                let top = *y;
                *y += item.tall();
                Some(top)
            })
            .collect();
        let height = tops.last().zip(items.last()).map_or(INSET, |(top, item)| top + item.tall()) + INSET;
        Sheet { width, height, tops }
    }

    /// Draws it, its corner at the frame's origin (its shadow beyond),
    /// as `look` has it, the choice `lit` highlighted; over the screen behind
    /// it, `frosted` (see `Drawn`). (The studio's, and the tests': a menu's window draws it through `Drawn::draw`.)
    #[cfg(feature = "studio")]
    pub fn paint(&self, frame: &gfx::Frame, items: &[Item], look: Look, lit: Option<usize>, frosted: bool) {
        self.paint_sheet(frame, items, look, lit, frosted);
        self.paint_choices(frame, items, look, frosted);
    }

    /// Its sheet: the plate, the rules between groups and the choice `lit`'s highlight.
    #[cfg(feature = "studio")]
    fn paint_sheet(&self, frame: &gfx::Frame, items: &[Item], look: Look, lit: Option<usize>, frosted: bool) {
        let (width, height) = (self.width, self.height);
        let colours = Colours::new(look, frosted);
        paint_plate(frame, &colours, (0.0, 0.0, width, height), colours.radius);
        self.paint_rows(frame, items, look, lit, frosted);
    }

    /// On its sheet: the rules between groups and the choice `lit`'s highlight.
    fn paint_rows(&self, frame: &gfx::Frame, items: &[Item], look: Look, lit: Option<usize>, frosted: bool) {
        let width = self.width;
        let colours = Colours::new(look, frosted);
        let highlight_radius = colours.radius - INSET;
        for (i, (item, top)) in items.iter().zip(&self.tops).enumerate() {
            if item.rule_before {
                frame.fill(colours.rule, 0.0, top - RULE / 2.0 - 0.5, width, 1.0);
            }
            if lit == Some(i) {
                frame.fill_rounded(colours.highlight, INSET, *top, width - 2.0 * INSET, item.tall(), highlight_radius);
            }
        }
    }

    /// What is on its sheet: each choice's mark, words and tick.
    fn paint_choices(&self, frame: &gfx::Frame, items: &[Item], look: Look, frosted: bool) {
        let width = self.width;
        let colours = Colours::new(look, frosted);
        for (item, top) in items.iter().zip(&self.tops) {
            let middle = top + item.tall() / 2.0;
            // (Dimmed: shown, not to be chosen.)
            let seen = |color: Color| if item.dimmed { color.alpha(0.45) } else { color };
            match item.mark {
                Some(Mark::Icon(icon)) => frame.icon(icon, Point { x: INSET + 16.0, y: middle }, ICON, seen(colours.icon)),
                Some(Mark::Colour(colour)) => frame.fill_circle(seen(colour), Point { x: INSET + 16.0, y: middle }, DOT / 2.0),
                None => {}
            }
            // (Its words above the middle where more is said under them.)
            let words = if item.detail.is_some() { middle - 8.0 } else { middle };
            let (ink_top, ink_bottom) = frame.ink(&item.label, look.font());
            frame.text(&item.label, look.font(), seen(colours.text), TEXT_LEFT, words - (ink_top + ink_bottom) / 2.0, width - TEXT_LEFT - 36.0, Align::Start);
            if let Some(detail) = item.detail.as_deref() {
                let font = look.detail_font();
                let (ink_top, ink_bottom) = frame.ink(detail, font);
                frame.text(detail, font, seen(colours.icon), TEXT_LEFT, middle + 9.0 - (ink_top + ink_bottom) / 2.0, width - TEXT_LEFT - 36.0, Align::Start);
            }
            if item.checked {
                frame.icon(Icon::Check, Point { x: width - INSET - 18.0, y: middle }, ICON, seen(colours.text));
            }
            if !item.more.is_empty() {
                frame.icon(Icon::ChevronRight, Point { x: width - INSET - 18.0, y: middle }, ICON, seen(colours.icon));
            }
        }
    }
}

/// What a menu is drawn in: Windows 11's (light or dark), or its skin's, as the panel's lanes are (glass's a near opaque
/// pane of its glass, lit along its top, round as its lanes; chart paper's its paper ruled in ink, square-ish); its
/// corners' radius (its rows' highlight concentric within them).
/// What a plate drawn as a menu's sheet is (see `plate`): the ink of what is on it, of a choice lit on it, and how round
/// its corners are.
pub struct Plate {
    pub icon: Color,
    pub highlight: Color,
    pub radius: f32,
}

/// A plate `width` × `height` from (`x`, `y`) drawn as a menu's sheet in `look` (its shadow, its edge), for a few choices
/// shown as one (a note's format bar): what is drawn on it is to be drawn so.
pub fn plate(frame: &dyn super::canvas::Canvas, look: Look, rect: (f32, f32, f32, f32)) -> Plate {
    plate_seen(frame, look, rect, 1.0)
}

/// The same over what it covers (its sheet whole: nothing under it shows through), `seen` of it shown (0 none, 1 all)
/// as it comes and goes; what is drawn on it to be shown as much.
pub fn plate_seen(frame: &dyn super::canvas::Canvas, look: Look, (x, y, width, height): (f32, f32, f32, f32), seen: f32) -> Plate {
    let colours = Colours::new(look, false);
    let radius = colours.radius.min(height / 2.0);
    let shown = |color: Color| color.alpha(seen);
    let faded = Colours { sheet: Color { a: seen, ..colours.sheet }, stroke: shown(colours.stroke), rim: colours.rim.map(shown), shadow: shown(colours.shadow), ..colours };
    paint_plate(frame, &faded, (x, y, width, height), radius);
    Plate { icon: colours.icon, highlight: colours.highlight, radius }
}

pub(super) struct Colours {
    pub(super) sheet: Color,
    pub(super) stroke: Color,
    pub(super) rim: Option<Color>,
    pub(super) shadow: Color,
    pub(super) text: Color,
    pub(super) icon: Color,
    pub(super) highlight: Color,
    pub(super) rule: Color,
    pub(super) radius: f32,
}

impl Colours {
    pub(super) fn new(look: Look, frosted: bool) -> Self {
        use super::theme::Skin;
        let dark = look.dark;
        let theme = look.skin.map(|skin| super::theme::Theme::new(skin, dark));
        match (look.skin, theme) {
            (Some(Skin::Glass), Some(theme)) => Colours {
                // (Over the screen frosted, tinted as the lanes of the glass it is opened on are, as frosted as they
                // are; else near opaque, what is behind it sharp.)
                sheet: match (dark, frosted) {
                    (_, true) => super::skins::glass_tint(&theme, look.frost, true),
                    (true, false) => Color::hex(0x2A2D35, 0.94),
                    (false, false) => Color::hex(0xF7F8FA, 0.94),
                },
                stroke: if dark { Color::hex(0x000000, 0.35) } else { Color::hex(0x000000, 0.08) },
                rim: Some(Color::hex(0xFFFFFF, if dark { 0.16 } else { 0.9 })),
                shadow: Color::hex(0x000000, if dark { 0.4 } else { 0.16 }),
                text: theme.text,
                icon: theme.text2,
                highlight: if dark { Color::hex(0xFFFFFF, 0.08) } else { Color::hex(0x000000, 0.05) },
                rule: theme.rule,
                radius: 14.0,
            },
            (Some(Skin::Paper), Some(theme)) => Colours {
                sheet: Color { a: 1.0, ..theme.paper },
                stroke: theme.stroke,
                rim: None,
                shadow: Color::hex(0x000000, if dark { 0.3 } else { 0.1 }),
                text: theme.text,
                icon: theme.text2,
                highlight: theme.hover,
                rule: theme.rule,
                radius: 8.0,
            },
            _ if dark => Colours {
                sheet: Color::hex(0x2C2C2C, if frosted { 0.78 } else { 0.98 }),
                stroke: Color::hex(0x000000, 0.2),
                rim: None,
                shadow: Color::hex(0x000000, 0.35),
                text: Color::hex(0xFFFFFF, 1.0),
                icon: Color::hex(0xFFFFFF, 0.86),
                highlight: Color::hex(0xFFFFFF, 0.06),
                rule: Color::hex(0xFFFFFF, 0.08),
                radius: RADIUS,
            },
            _ => Colours {
                sheet: Color::hex(0xF9F9F9, if frosted { 0.78 } else { 0.98 }),
                stroke: Color::hex(0x000000, 0.08),
                rim: None,
                shadow: Color::hex(0x000000, 0.16),
                text: Color::hex(0x000000, 0.9),
                icon: Color::hex(0x000000, 0.72),
                highlight: Color::hex(0x000000, 0.04),
                rule: Color::hex(0x000000, 0.08),
                radius: RADIUS,
            },
        }
    }
}

#[cfg(test)]
mod sized {
    use super::*;

    /// A menu opened on a panel or a widget made smaller on its screen is as small as it: its words as large as theirs.
    #[test]
    fn a_menu_is_as_large_as_what_it_is_opened_on() {
        use super::super::theme::Skin;
        let look = Look::of(Skin::Glass, true).frosted(0.4, true).zoomed(0.8);
        assert_eq!(look.px(1.5), 1.5 * 0.8);
        assert_eq!(Look::of(Skin::Glass, true).px(1.5), 1.5);
        assert_eq!(Look::system().px(1.25), 1.25);
    }
}

#[cfg(all(test, feature = "studio"))]
mod tests {
    use super::*;

    /// A choice made among those one opens is told after all the menu's own, after those of the choices before it that
    /// open some; a row saying more under its words is taller.
    #[test]
    fn choices_beside_one_are_told_after_the_menu_s_own() {
        let mut first = Item::new("甲", None, false, false);
        first.more = vec![Item::new("一", None, false, false), Item::new("二", None, false, false)];
        let mut third = Item::new("丙", None, false, false);
        third.more = vec![Item::new("三", None, false, false)];
        let items = vec![first, Item::new("乙", None, false, false), third];
        assert_eq!(Menu::sub_choice(&items, 0, 1), 4);
        assert_eq!(Menu::sub_choice(&items, 2, 0), 5);
        let mut said = Item::new("丁", None, false, false);
        assert_eq!(said.tall(), ROW);
        said.detail = Some("已配对的手机".into());
        assert_eq!(said.tall(), ROW_TWO);
    }

    /// A choice marked by a colour (a note's) shows a dot of that colour where an icon stands, round, in every look.
    #[test]
    fn a_colour_s_choice_shows_a_dot_of_it() {
        let _device = crate::ui::gfx::held().unwrap();
        use super::super::theme::Skin;
        let gfx = gfx::Gfx::new().expect("graphics");
        let pink = Color::hex(0xEE556D, 1.0);
        let items = vec![Item::new("粉色", Some(Mark::Colour(pink)), false, false)];
        for look in [Look { skin: None, dark: false, frost: 1.0, zoom: 1.0 }, Look::of(Skin::Glass, true), Look::of(Skin::Paper, false)] {
            let sheet = Sheet::new(&gfx, &items, look);
            let (w, h) = (sheet.width.ceil() as u32, sheet.height.ceil() as u32);
            let pixels = gfx.draw_offscreen((w, h), 1.0, |frame| sheet.paint(frame, &items, look, None, false)).expect("drawn");
            let at = |x: f32, y: f32| {
                let i = ((y as u32 * w + x as u32) * 4) as usize;
                (pixels[i + 2], pixels[i + 1], pixels[i])
            };
            let middle = sheet.tops[0] + ROW / 2.0;
            // Its middle the colour; a corner of the icon's square (outside the round dot) not.
            assert_eq!(at(INSET + 16.0, middle), (0xEE, 0x55, 0x6D), "{look:?}");
            assert_ne!(at(INSET + 16.0 - DOT / 2.0 + 0.5, middle - DOT / 2.0 + 0.5), (0xEE, 0x55, 0x6D), "{look:?}: not round");
        }
    }

    /// A menu opened on glass is tinted as that glass's lanes are, as frosted as they are drawn; on glass with nothing
    /// behind it to frost (drawn as at its most frosted), as at its most frosted.
    #[test]
    fn a_menu_on_glass_is_tinted_as_the_glass_it_is_opened_on() {
        use super::super::theme::{Skin, Theme};
        for dark in [false, true] {
            let theme = Theme::new(Skin::Glass, dark);
            for frost in [0.0, 0.35, 1.0] {
                let look = Look::of(Skin::Glass, dark).frosted(frost, true);
                assert_eq!(Colours::new(look, true).sheet, super::super::skins::glass_tint(&theme, frost, true), "dark {dark}, frost {frost}");
            }
            assert_eq!(Look::of(Skin::Glass, dark).frosted(0.2, false).frost, 1.0);
        }
    }

    /// A menu in each look, light and dark, a row lit, and under them glass's as it is opened on glass (a piece of it,
    /// over what the compositor frosts behind it, drawn here over a plain ground) (`cargo test --features studio --lib
    /// -- --ignored menu_looks` writes `target/looks/menus.png`).
    #[test]
    #[ignore = "writes a picture to look at"]
    fn menu_looks() {
        let _device = crate::ui::gfx::held().unwrap();
        use super::super::theme::Skin;
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let gfx = gfx::Gfx::new().expect("graphics");
        let mut send = Item::new("发送到", Some(Mark::Icon(Icon::Send)), false, false);
        send.more = vec![Item::new("Galaxy Z Fold6", Some(Mark::Icon(Icon::Smartphone)), false, false)];
        let mut away = Item::new("书房电脑", Some(Mark::Icon(Icon::Monitor)), false, false);
        (away.detail, away.dimmed) = (Some("未开启接收".into()), true);
        let mut paired = Item::new("Galaxy Z Fold6", Some(Mark::Icon(Icon::Smartphone)), false, false);
        paired.detail = Some("已配对的手机".into());
        let items = vec![
            Item::new("小卡片", Some(Mark::Icon(Icon::LayoutGrid)), false, false),
            Item::new("大卡片", Some(Mark::Icon(Icon::Image)), false, false),
            Item::new("列表", Some(Mark::Icon(Icon::List)), true, false),
            send,
            paired,
            away,
            Item::new("清空中转站", None, false, true),
        ];
        let looks = [Look { skin: None, dark: false, frost: 1.0, zoom: 1.0 }, Look::of(Skin::Glass, false), Look::of(Skin::Paper, false), Look { skin: None, dark: true, frost: 1.0, zoom: 1.0 }, Look::of(Skin::Glass, true), Look::of(Skin::Paper, true)];
        let sheets: Vec<Sheet> = looks.iter().map(|look| Sheet::new(&gfx, &items, *look)).collect();
        let widest = sheets.iter().map(|sheet| sheet.width).fold(0.0, f32::max);
        let room = super::super::theme::room(&super::super::theme::Theme::new(Skin::Glass, false).shadows).max(SHADOW);
        let (cell_w, cell_h, px) = (widest + 2.0 * room, sheets[0].height + 2.0 * room, 2.0f32);
        let (w, h) = ((3.0 * cell_w * px) as u32, (3.0 * cell_h * px) as u32);
        let pixels = gfx.draw_offscreen((w, h), px, |frame| {
            frame.fill(Color::hex(0x8899AA, 1.0), 0.0, 0.0, 3.0 * cell_w, cell_h);
            frame.fill(Color::hex(0x223344, 1.0), 0.0, cell_h, 3.0 * cell_w, cell_h);
            frame.fill(Color::hex(0x8899AA, 1.0), 0.0, 2.0 * cell_h, 1.5 * cell_w, cell_h);
            frame.fill(Color::hex(0x223344, 1.0), 1.5 * cell_w, 2.0 * cell_h, 1.5 * cell_w, cell_h);
            for (at, (look, sheet)) in looks.iter().zip(&sheets).enumerate() {
                frame.origin((at % 3) as f32 * cell_w + room, (at / 3) as f32 * cell_h + room);
                sheet.paint(frame, &items, *look, Some(1), false);
            }
            for (at, dark) in [false, true].into_iter().enumerate() {
                let look = Look::of(Skin::Glass, dark);
                let sheet = Sheet::new(&gfx, &items, look);
                let (x, y) = (at as f32 * 1.5 * cell_w + room, 2.0 * cell_h + room);
                frame.origin(x, y);
                let piece = super::super::view::Rect { x: 0.0, y: 0.0, w: sheet.width, h: sheet.height };
                super::super::skins::glass_sheet(frame, &super::super::theme::Theme::new(Skin::Glass, dark), piece, Colours::new(look, true).radius, look.frost).unwrap();
                frame.origin(x, y);
                sheet.paint_rows(frame, &items, look, Some(1), true);
            }
        }).expect("drawn");
        let out = std::path::PathBuf::from(format!("{}/target/looks", env!("CARGO_MANIFEST_DIR")));
        std::fs::create_dir_all(&out).unwrap();
        crate::studio::save_png(&out.join("menus.png"), w, h, &pixels).unwrap();
    }
}
