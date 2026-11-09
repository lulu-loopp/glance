//! The overlay's window: never taking the focus. Unlocked, it is dragged
//! where it is to stay; locked, it stays. Either way a right click on it
//! offers the lock, turning it off and the settings (so it takes the clicks that
//! land on it: a game holding the pointer is not one of them), and the
//! pointer over it brings a row of buttons for the same, as a widget's are.
//! Letting clicks through, it takes none: the pointer staying over it a
//! while brings the one button that ends that. Drawn by the panel's thread
//! at every sample.
//!
//! What shows (its glass and its shadow) is drawn in a window of its own,
//! its stage, larger than the glass by as much as it can grow, that every
//! click passes through; what the pointer does to it, a window as large as
//! the glass that shows nothing takes. Sized, the glass grows and shrinks
//! inside its stage, which stays as it is: a window sized as it was drawn
//! again was cut to its new size a moment before the new drawing showed
//! (the system sizes the window at once, and shows what is drawn on it at
//! its next commit), and the glass's end was cut off as it shrank. Moved,
//! the stage moves with it, what is on it as it was.

use std::rc::Rc;
use std::time::Instant;

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, KillTimer, SetTimer, IDC_ARROW};

use std::collections::VecDeque;

use crate::reading::Sample;
use crate::settings::OverlaySettings;
use crate::ui::backdrop::Capture;
use crate::ui::buttons::{self, Button};
use crate::ui::canvas::{Color, Point};
use crate::ui::gfx::{self, Gfx, Surface};
use crate::ui::glass::Frosted;
use crate::ui::life::{Lives, Showing};
use crate::ui::overlay::{self, Glass, Hand, Layout, Metrics, Shape, Tone, INSET, MARGIN};
use crate::ui::text::Lang;
use crate::ui::window::Window;


/// What its right-click menu offers.
pub enum Choice {
    Lock(bool),
    Through(bool),
    Close,
    Settings,
}

impl Lives for Overlay {
    fn device(&self) -> &Rc<Gfx> {
        &self.gfx
    }

    fn showing(&mut self) -> &mut Option<Showing> {
        &mut self.showing
    }

    /// What its stage's and its buttons' surfaces hold.
    fn let_go_of_shown(&mut self) {
        self.surface.release();
        if let Some((gfx, surface)) = &mut self.bar_surface {
            surface.release(gfx);
        }
    }

    /// Its buttons' surface.
    fn let_go_of_kept(&mut self) {
        self.bar_surface = None;
    }
}

pub struct Overlay {
    gfx: Rc<Gfx>,
    /// Counted as shown on its device while it is (see `life`).
    showing: Option<Showing>,
    /// Where it was last placed, with its shadow (physical px).
    placed: std::cell::Cell<RECT>,
    /// The shape its window takes the pointer in, as last given it (see `shape`): its width, height and corners'
    /// radius (physical px).
    shaped: (i32, i32, i32),
    /// What takes the pointer: as large as its glass, showing nothing.
    pub window: Window,
    /// What shows (see the top): its stage, where it is (physical px), and what is drawn on it, its glass and its
    /// shadow.
    stage: Window,
    stage_at: RECT,
    surface: Frosted,
    shown: bool,
    locked: bool,
    /// How clear its glass is (see `OverlaySettings::clear`), and whether
    /// clicks pass through it.
    clear: f32,
    through: bool,
    /// Its buttons, in a window of their own beside it: those that show
    /// now (none: the window is away), and its content.
    bar: Window,
    bar_surface: Option<(Rc<Gfx>, Surface)>,
    bar_shows: Vec<Button>,
    /// Its settings' button drawn lit (the settings asked from it open).
    bar_settings: bool,
    /// Since when the pointer has been over it or its buttons.
    over: Option<Instant>,
    /// Whether a game shows (its button lit), as the last sample had it:
    /// (the samples are held as it is drawn: not asked for again there).
    game: bool,
    /// Where its buttons' window was last put (none: away).
    bar_at: Option<RECT>,
    /// The button the pointer is on, said under it.
    tip: Option<Button>,
    /// How clear its glass was just made: said on it until it has been
    /// left so for a moment (see `SAID_MS`).
    said: Option<f32>,
    lang: Lang,
    /// Where its glass is on screen (physical pixels).
    rect: RECT,
    /// The screen it is on.
    screen: RECT,
    /// Its screen's scale, and the size it is drawn at (1 as designed).
    dpi: f32,
    size: f32,
    /// What it shows, the frame rate's recent course, and how: drawn again
    /// as it is dragged.
    drawn: (Vec<overlay::Reading>, Vec<Option<f32>>, Shape),
    /// How its glass is tinted, as the screen around it last asked.
    glass: Option<Glass>,
    /// The tint its last look at the screen needed, and when it looked (see `look`).
    needed: Option<f32>,
    looked: Instant,
    /// Being dragged by the pointer: to move it, or by its edges, to size it.
    grab: Option<Grab>,
    /// The screen of the game played, kept while it presents no frames
    /// for a moment.
    game_screen: Option<[i32; 4]>,
    /// The panel's window while the panel is up: kept above the overlay.
    beneath: Option<HWND>,
    /// Where its settings' button was as the settings were asked from it, while they are open (see `settings_here`).
    settings_from: Option<RECT>,
    /// A menu's window while one is up: put back above the overlay each time the overlay is placed (see `under_menu`).
    menu: Option<HWND>,
}

/// A drag under way: where the pointer, the overlay and its size were as it
/// began, and which edges it holds (none: the whole overlay, to move it).
#[derive(Clone, Copy)]
struct Grab {
    from: POINT,
    /// The glass's place on screen as the drag began.
    glass: RECT,
    size: f32,
    edges: Edges,
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Edges {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}

impl Edges {
    fn any(self) -> bool {
        self.left || self.right || self.top || self.bottom
    }
}

/// How near its edge the pointer sizes it rather than moves it (DIPs).
const EDGE: f32 = 8.0;

/// How far around the overlay's shadow the screen is looked at, for how to
/// tint the glass (DIPs): what is behind it, blurred, is much like what is
/// around it.
const AROUND: f32 = 16.0;
/// How often it looks at what is behind it, the readings or not (see `watch`).
const LOOK_EVERY: std::time::Duration = std::time::Duration::from_millis(250);

/// While it is dragged, the pointer is followed this often (ms) by a timer
/// of this id: never in front, the overlay is told of the pointer only
/// while it is over it, and a quick drag leaves it.
pub const FOLLOW_TIMER: usize = 1;
const FOLLOW_MS: u32 = 8;

/// Where it is moved to, and how large it is, as a drag left it.
pub struct Placed {
    /// See `OverlaySettings::at`.
    pub at: (f32, f32),
    /// A point on the screen it is over.
    pub point: (i32, i32),
    pub size: f32,
}

/// A monitor's whole area, and its scale.
fn monitor_info(monitor: HMONITOR) -> (RECT, f32) {
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = unsafe { GetMonitorInfoW(monitor, &mut info) };
    let (mut dpi, mut unused) = (96u32, 96u32);
    let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut unused) };
    (info.rcMonitor, dpi as f32 / 96.0)
}

/// The room glass `size` large has on `screen`, inside the inset: its
/// left, top, and how far it can move across and down.
fn room(screen: RECT, size: (i32, i32), scale: f32) -> (i32, i32, i32, i32) {
    let inset = (INSET * scale).round() as i32;
    let across = (screen.right - screen.left - 2 * inset - size.0).max(0);
    let down = (screen.bottom - screen.top - 2 * inset - size.1).max(0);
    (screen.left + inset, screen.top + inset, across, down)
}


/// The timer that ends the overlay's saying how clear its glass was made,
/// and how long after it was last changed (as the panel's size is said
/// only while it is being set).
pub const SAID_TIMER: usize = 7;
const SAID_MS: u32 = 600;
/// How near a corner the pointer sizes it by both edges there (DIPs): no
/// nearer than half its shorter side lets.
const CORNER: f32 = 16.0;

/// The room round the buttons in their window, and between it and the glass (DIPs).
const BAR_PAD: f32 = 3.0;
const BAR_GAP: f32 = 3.0;

fn cursor() -> Option<POINT> {
    let mut cursor = POINT::default();
    unsafe { GetCursorPos(&mut cursor) }.ok().map(|_| cursor)
}

impl Overlay {
    pub fn new() -> windows::core::Result<Self> {
        // (Drawn first on the main screen's device: it takes up its game's screen's as it draws, see `draw`.)
        let gfx = gfx::for_main_screen()?;
        let window = Window::see_through()?;
        let mut stage = match Window::see_through() {
            Ok(stage) => stage,
            Err(error) => {
                window.destroy();
                return Err(error);
            }
        };
        stage.set_click_through(true);
        let bar = match Window::new() {
            Ok(bar) => bar,
            Err(error) => {
                window.destroy();
                stage.destroy();
                return Err(error);
            }
        };
        let surface = match Frosted::new(&gfx, stage.hwnd, (crate::ui::glass::BLUR, crate::ui::glass::SATURATION)) {
            Ok(surface) => surface,
            Err(error) => {
                // Tried again at the next sample, with windows of its own.
                window.destroy();
                stage.destroy();
                bar.destroy();
                return Err(error);
            }
        };
        Ok(Overlay {
            gfx,
            window,
            stage,
            stage_at: RECT::default(),
            surface,
            showing: None,
            shown: false,
            placed: std::cell::Cell::new(RECT::default()),
            settings_from: None,
            shaped: (0, 0, 0),
            locked: false,
            clear: 0.0,
            through: false,
            bar,
            bar_surface: None,
            bar_shows: Vec::new(),
            bar_settings: false,
            over: None,
            game: false,
            bar_at: None,
            tip: None,
            said: None,
            lang: Lang::En,
            rect: RECT::default(),
            screen: RECT::default(),
            dpi: 1.0,
            size: 1.0,
            drawn: (Vec::new(), Vec::new(), Shape { layout: Layout::Card, rows: 1, width: f32::INFINITY }),
            glass: None,
            needed: None,
            looked: Instant::now(),
            grab: None,
            game_screen: None,
            beneath: None,
            menu: None,
        })
    }

    /// Shows the readings of the latest sample in `history` as `settings`
    /// ask, while `wanted`: while a game is played (`playing`), with its
    /// frames, on its screen; otherwise on the screen it was put on. Hidden
    /// while not wanted, and while there is nothing to show. Above all but
    /// `beneath` (the open panel's window).
    pub fn show(&mut self, history: &VecDeque<Sample>, settings: &OverlaySettings, lang: Lang, playing: bool, wanted: bool, beneath: Option<HWND>) {
        self.locked = settings.locked;
        let clear = settings.clear.clamp(0.0, 1.0);
        if self.shown && clear != self.clear {
            self.said = Some(clear);
            unsafe { SetTimer(Some(self.window.hwnd), SAID_TIMER, SAID_MS, None) };
        }
        self.clear = clear;
        self.through = settings.click_through;
        self.window.set_click_through(self.through);
        self.game = crate::ui::view::game_shown(history.back().map(std::slice::from_ref).unwrap_or_default());
        self.beneath = beneath;
        self.lang = lang;
        let sample = history.back();
        // The game played: one found, or one taken for a game by hand.
        let game = sample.and_then(|s| s.game.as_ref()).filter(|game| playing || game.by_hand);
        // (One taken by hand does not bring the overlay up: shown where it is up, its frames are.)
        let framed = playing || game.is_some();
        let readings = sample.filter(|_| wanted).map(|s| overlay::readings(s, game, framed, &settings.chosen(), lang)).unwrap_or_default();
        if readings.is_empty() {
            return self.hide();
        }
        let frames = overlay::frames(history.iter(), framed);
        // Held as the pointer has it while it is dragged.
        if self.grab.is_some() {
            self.drawn = (readings, frames, self.drawn.2);
            let (width, height) = self.dims();
            let r = self.rect;
            self.rect = RECT { left: r.left, top: r.top, right: r.left + width, bottom: r.top + height };
            return self.draw();
        }
        // On the screen it was put on; never put anywhere, over a game, the
        // game's screen.
        if let Some(screen) = game.and_then(|game| game.screen) {
            self.game_screen = Some(screen);
        }
        if !playing {
            self.game_screen = None;
        }
        let game_point = self.game_screen.map(|[left, top, right, bottom]| ((left + right) / 2, (top + bottom) / 2));
        let point = settings.screen.or(game_point);
        let monitor = match point {
            Some((x, y)) => unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) },
            None => unsafe { MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY) },
        };
        (self.screen, self.dpi) = monitor_info(monitor);
        self.size = settings.size.clamp(crate::panel::SIZES.0, crate::panel::SIZES.1);
        // As wide as its screen at most, inside the inset (DIPs at its size).
        let widest = (self.screen.right - self.screen.left) as f32 / self.scale() - 2.0 * INSET / self.size;
        self.drawn = (readings, frames, settings.shape(widest));
        let size = self.dims();
        let (left, top, across, down) = room(self.screen, size, self.dpi);
        let x = left + (settings.at.0.clamp(0.0, 1.0) * across as f32).round() as i32;
        let y = top + (settings.at.1.clamp(0.0, 1.0) * down as f32).round() as i32;
        self.rect = RECT { left: x, top: y, right: x + size.0, bottom: y + size.1 };
        self.look();
        self.draw();
    }

    /// The scale it is drawn at: its screen's, by its size.
    fn scale(&self) -> f32 {
        self.dpi * self.size
    }

    /// Its shadow's margin (physical pixels).
    fn margin(&self) -> i32 {
        (MARGIN * self.scale()).round() as i32
    }

    /// Where its shadow's window is: around the glass by the margin.
    fn shadow_rect(&self) -> RECT {
        let (m, r) = (self.margin(), self.rect);
        RECT { left: r.left - m, top: r.top - m, right: r.right + m, bottom: r.bottom + m }
    }

    /// The glass's width and height on screen, at its size (physical
    /// pixels), and its corners' radius.
    fn measures(&self) -> (i32, i32, f32) {
        let scale = self.scale();
        let (readings, _, shape) = &self.drawn;
        let width = |text: &str, font| self.gfx.measure(text, font);
        let line = |font| self.gfx.baseline(font);
        let metrics = Metrics { width: &width, line: &line };
        // Whole pixels already: rounded, not lifted a pixel by a float's error.
        let (width, height) = overlay::size(readings, *shape, scale, &metrics);
        ((width * scale).round() as i32, (height * scale).round() as i32, overlay::radius(readings, *shape, scale, &metrics) * scale)
    }

    fn dims(&self) -> (i32, i32) {
        let (width, height, _) = self.measures();
        (width, height)
    }

    /// Where its stage is to be, for the glass where it is (see the top): its shadow's room round the glass, and room
    /// for the glass to grow to its largest from whichever corner it is sized by, on its screen.
    fn stage_for(&self) -> RECT {
        let scale = self.dpi * crate::panel::SIZES.1;
        let (readings, _, shape) = &self.drawn;
        let width = |text: &str, font| self.gfx.measure(text, font);
        let line = |font| self.gfx.baseline(font);
        let metrics = Metrics { width: &width, line: &line };
        let (most_w, most_h) = overlay::size(readings, *shape, scale, &metrics);
        let (most_w, most_h) = ((most_w * scale).ceil() as i32, (most_h * scale).ceil() as i32);
        let (r, m, s) = (self.rect, self.margin(), self.screen);
        let (grow_x, grow_y) = ((most_w - (r.right - r.left)).max(0), (most_h - (r.bottom - r.top)).max(0));
        // (No further than its screen goes, but always round its shadow.)
        RECT {
            left: (r.left - m - grow_x).max(s.left).min(r.left - m),
            top: (r.top - m - grow_y).max(s.top).min(r.top - m),
            right: (r.right + m + grow_x).min(s.right).max(r.right + m),
            bottom: (r.bottom + m + grow_y).min(s.bottom).max(r.bottom + m),
        }
    }

    /// Its stage, where it is, kept while the glass and its shadow are inside it (see the top); else placed anew.
    fn staged(&mut self) {
        let (r, s) = (self.shadow_rect(), self.stage_at);
        if !(s.left <= r.left && s.top <= r.top && r.right <= s.right && r.bottom <= s.bottom) {
            self.stage_at = self.stage_for();
        }
    }

    /// Looks at the screen around it, and tints the glass for it (see
    /// `overlay::glass`); unseen (the lock screen has the display, or none
    /// of its screen is around it), for anything.
    fn look(&mut self) {
        let around = (AROUND * self.dpi).round() as i32;
        let (r, s) = (self.shadow_rect(), self.screen);
        // On its screen only: past the screen's edge is nothing.
        let area = RECT { left: (r.left - around).max(s.left), top: (r.top - around).max(s.top), right: (r.right + around).min(s.right), bottom: (r.bottom + around).min(s.bottom) };
        let seen = Capture::take(area);
        let mut behind = seen.map(|capture| capture.luminances(area, r, self.dpi.round().max(1.0) as i32)).unwrap_or_default();
        let (glass, needed) = overlay::glass(&mut behind, self.glass.zip(self.needed));
        (self.glass, self.needed, self.looked) = (Some(glass), Some(needed), Instant::now());
    }

    /// Places its windows: the one that takes the pointer where the glass is, above all but the open panel; its stage
    /// just beneath it.
    fn place(&self) {
        self.window.place_under(self.rect, self.beneath);
        self.menu_above();
        self.stage.place_under(self.stage_at, Some(self.window.hwnd));
        // Where it was and where it is: a widget that took the desktop with
        // it there takes it again.
        let now = self.shadow_rect();
        let was = self.placed.replace(now);
        if was != now {
            crate::widget::own_window_changed(was);
            crate::widget::own_window_changed(now);
        }
    }

    /// Its window shaped as its glass is, `width` × `height` with corners `radius` round (physical px): the pointer
    /// is taken on the glass alone (past its rounded corners, a press went to it, not to what was under them). One not
    /// given it is asked for again as it is next drawn (see `gfx::give_region`).
    fn shape(&mut self, width: i32, height: i32, radius: f32) {
        let shaped = (width, height, radius.round() as i32);
        if self.shaped == shaped {
            return;
        }
        // (A round rectangle's region leaves its right and bottom edges out: one more each way.)
        let region = unsafe { windows::Win32::Graphics::Gdi::CreateRoundRectRgn(0, 0, width + 1, height + 1, 2 * shaped.2, 2 * shaped.2) };
        if gfx::give_region(self.window.hwnd, region, true) {
            self.shaped = shaped;
        }
    }

    /// Draws what it shows where it is, set off in colour while it is dragged.
    fn draw(&mut self) {
        // This thread's device for the screen its glass is on: a new one after the last was lost, or on another
        // adapter's screen (see `gfx::on_screen`).
        let Ok(gfx) = gfx::for_area(self.rect) else { return };
        // (What it drew with on another device goes with it: see `life`.)
        if !Rc::ptr_eq(&gfx, &self.gfx) {
            self.gone();
            self.gfx = gfx;
        }
        self.shown();
        self.staged();
        // Placed above all again each time (a game may have taken the top),
        // but for the open panel.
        self.place();
        let scale = self.scale();
        let glass = Glass { presence: 1.0 - self.clear, ..self.glass.unwrap_or_else(|| overlay::glass(&mut [], None).0) };
        let (width, height, radius) = self.measures();
        self.shape(width, height, radius);
        let (readings, frames, shape) = (&self.drawn.0, &self.drawn.1, self.drawn.2);
        let dragged = self.grab.is_some();
        let hand = if dragged { Hand::Dragged } else { Hand::Free };
        // Its glass is its tint alone: what is behind it is not blurred (the system's compositor would then draw
        // the game behind into it every frame, some frames in a hundred of the game's).
        self.surface.set_frost(0.0);
        let said = self.said.map(|clear| (if self.lang == Lang::Zh { "透明度" } else { "Clarity" }, format!("{:.0}", clear * 100.0)));
        // The glass where it is on its stage (physical px, and DIPs).
        let stage = self.stage_at;
        let (at_x, at_y) = (self.rect.left - stage.left, self.rect.top - stage.top);
        let size = ((stage.right - stage.left) as u32, (stage.bottom - stage.top) as u32);
        let pane = (at_x as f32, at_y as f32, width as f32, height as f32, radius);
        let painted = self.surface.draw(&self.gfx, size, scale, pane, |frame| {
            frame.origin(at_x as f32 / scale, at_y as f32 / scale);
            frame.crisp_text();
            overlay::shadow(frame, readings, shape, glass, dragged, scale);
            overlay::paint(frame, readings, frames, shape, glass, hand, scale);
            if let Some((name, figure)) = &said {
                overlay::say(frame, (name, figure, "%"), (width as f32 / scale, height as f32 / scale), Glass { presence: 1.0, ..glass });
            }
        });
        if let Err(error) = &painted {
            // Drawn again at the next sample (the device gone, on a new one).
            gfx::failed("overlay", &self.gfx, Some(error));
            return;
        }
        self.gfx.sweep();
        if !self.shown {
            self.window.show();
            self.stage.show();
            self.place();
            self.shown = true;
        }
        // Its buttons, lit as it is now; and what one of them says, over it still.
        self.draw_bar();
        crate::ui::tip::above();
    }

    /// Where each of its buttons' middles is on the screen, whether it
    /// shows or not (physical pixels): a row ending at the glass's right
    /// edge, above it, or under it where its screen has no room above.
    fn bar_middles(&self) -> [POINT; 5] {
        let px = |dips: f32| (dips * self.dpi).round() as i32;
        let (tall, r) = (px(buttons::SIZE + 2.0 * BAR_PAD), self.rect);
        let top = if r.top - px(BAR_GAP) - tall < self.screen.top { r.bottom + px(BAR_GAP) } else { r.top - px(BAR_GAP) - tall };
        let count = buttons::ALL.len();
        std::array::from_fn(|place| POINT { x: r.right - px(BAR_PAD + buttons::SIZE / 2.0 + (count - 1 - place) as f32 * (buttons::SIZE + buttons::GAP)), y: top + tall / 2 })
    }

    /// The buttons' window: round those that show.
    fn bar_rect(&self) -> RECT {
        let half = ((buttons::SIZE / 2.0 + BAR_PAD) * self.dpi).round() as i32;
        let middles = self.bar_middles();
        let shown = |button: &Button| middles[buttons::ALL.iter().position(|one| one == button).unwrap()];
        let (first, last) = (self.bar_shows.first().map_or(middles[0], shown), self.bar_shows.last().map_or(middles[4], shown));
        RECT { left: first.x - half, top: first.y - half, right: last.x + half, bottom: last.y + half }
    }

    /// The button of those showing that a point of the screen is on.
    fn bar_button_at(&self, at: POINT) -> Option<Button> {
        let reach = (buttons::SIZE / 2.0 + 2.0) * self.dpi;
        let middles = self.bar_middles();
        self.bar_shows.iter().copied().find(|button| {
            let middle = middles[buttons::ALL.iter().position(|one| one == button).unwrap()];
            ((middle.x - at.x) as f32).hypot((middle.y - at.y) as f32) <= reach
        })
    }

    /// Whether `hwnd` is its buttons' window.
    pub fn owns_bar(&self, hwnd: HWND) -> bool {
        self.bar.hwnd == hwnd
    }

    /// Draws the buttons that show, in their window where it belongs; none, puts it away.
    fn draw_bar(&mut self) {
        if self.bar_shows.is_empty() || !self.shown {
            if let Some((gfx, surface)) = &mut self.bar_surface {
                surface.release(gfx);
            }
            self.bar.hide();
            // (A widget that took the desktop with them there takes it again.)
            if let Some(was) = self.bar_at.take() {
                crate::widget::own_window_changed(was);
            }
            return;
        }
        let rect = self.bar_rect();
        let surface = match self.bar_surface.take() {
            Some((made_on, surface)) if Rc::ptr_eq(&made_on, &self.gfx) => Ok(surface),
            _ => Surface::new(&self.gfx, self.bar.hwnd),
        };
        let Ok(mut surface) = surface else { return };
        let dark = self.glass.is_none_or(|glass| glass.tone == Tone::Dark);
        let rim = if dark { Color::hex(0xFFFFFF, 0.3) } else { Color::hex(0x000000, 0.3) };
        let (shows, locked, through, game, settings) = (&self.bar_shows, self.locked, self.through, self.game, self.bar_settings);
        let drawn = surface.draw(&self.gfx, ((rect.right - rect.left) as u32, (rect.bottom - rect.top) as u32), self.dpi, |frame| {
            for (place, button) in shows.iter().enumerate() {
                let lit = (*button == Button::Pin && locked) || (*button == Button::Through && through) || (*button == Button::Game && game) || (*button == Button::Settings && settings);
                let centre = Point { x: BAR_PAD + buttons::SIZE / 2.0 + place as f32 * (buttons::SIZE + buttons::GAP), y: BAR_PAD + buttons::SIZE / 2.0 };
                buttons::paint(frame, centre, *button, lit, dark, rim, 1.0);
            }
        });
        self.bar_surface = Some((self.gfx.clone(), surface));
        if let Err(error) = &drawn {
            gfx::failed("overlay", &self.gfx, Some(error));
            return;
        }
        self.bar.place_under(rect, self.beneath);
        self.menu_above();
        self.bar.show();
        crate::ui::tip::above();
        if self.bar_at.replace(rect) != Some(rect) {
            crate::widget::own_window_changed(rect);
        }
    }

    /// Looks for the pointer (letting clicks through, it hears nothing of
    /// it): over it or its buttons, they show; while it lets clicks
    /// through, only the one that ends that, and once the pointer has
    /// stayed a while. Called as often as the pointer is looked at.
    pub fn watch(&mut self) {
        let now = Instant::now();
        // What is behind looked at often, between the readings: tinted again soon after it changes (a game come up),
        // drawn again only where the tint changed.
        if self.shown && self.grab.is_none() && now.duration_since(self.looked) >= LOOK_EVERY {
            let was = self.glass;
            self.look();
            if self.glass != was {
                self.draw();
            }
        }
        // (The glass, the whole row and what lies between them.)
        let reach = {
            let half = ((buttons::SIZE / 2.0 + BAR_PAD) * self.dpi).round() as i32;
            let (middles, r) = (self.bar_middles(), self.rect);
            RECT { left: r.left.min(middles[0].x - half), top: r.top.min(middles[0].y - half), right: r.right, bottom: r.bottom.max(middles[0].y + half) }
        };
        let inside = self.shown && self.grab.is_none() && cursor().is_some_and(|c| (reach.left..reach.right).contains(&c.x) && (reach.top..reach.bottom).contains(&c.y));
        self.over = if inside { self.over.or(Some(now)) } else { None };
        // The settings asked from its button, open: told where it is now (the overlay moved meanwhile), so they pour
        // back into it there, and its buttons shown, it lit. Closed, or asked from elsewhere: no longer its.
        let gear = self.button_place(Button::Settings);
        let here = self.settings_here();
        if here && self.settings_from != Some(gear) {
            if let Some(from) = self.settings_from {
                crate::ui::settings_window::asked_from_moved(from, Some(gear));
            }
            self.settings_from = Some(gear);
        } else if !here {
            self.settings_from = None;
        }
        let shows = match (self.over, self.through) {
            (None, _) if here => buttons::ALL.to_vec(),
            (None, _) => Vec::new(),
            (Some(_), false) => buttons::ALL.to_vec(),
            (Some(since), true) if now.duration_since(since) >= buttons::LINGER => vec![Button::Through],
            (Some(_), true) => Vec::new(),
        };
        if shows != self.bar_shows || here != self.bar_settings {
            self.bar_shows = shows;
            self.bar_settings = here;
            self.draw_bar();
        }
        if !inside && self.tip.take().is_some() {
            crate::ui::tip::hide();
        }
    }

    /// The pointer moved over its buttons: what the one under it does, said under it.
    pub fn bar_hover(&mut self) {
        let Some(at) = cursor() else { return };
        let over = self.bar_button_at(at);
        self.bar.point(if over.is_some() { windows::Win32::UI::WindowsAndMessaging::IDC_HAND } else { IDC_ARROW });
        if over == self.tip {
            return;
        }
        self.tip = over;
        match over {
            Some(button) => {
                let lit = match button {
                    Button::Pin => self.locked,
                    Button::Through => self.through,
                    Button::Game => self.game,
                    _ => false,
                };
                // (Under the button itself: not wherever the pointer came onto it.)
                let half = (buttons::SIZE / 2.0 * self.dpi).round() as i32;
                let middle = self.bar_middles()[buttons::ALL.iter().position(|one| *one == button).unwrap()];
                let about = RECT { left: middle.x - half, top: middle.y - half, right: middle.x + half, bottom: middle.y + half };
                crate::ui::tip::show(self.window.hwnd, button.says(lit, self.lang, true), about, crate::os::apps_dark());
            }
            None => crate::ui::tip::hide(),
        }
    }

    /// Where its button `button` is on the screen (physical px).
    fn button_place(&self, button: Button) -> RECT {
        let middle = self.bar_middles()[buttons::ALL.iter().position(|one| *one == button).unwrap()];
        let half = (buttons::SIZE / 2.0 * self.dpi).round() as i32;
        RECT { left: middle.x - half, top: middle.y - half, right: middle.x + half, bottom: middle.y + half }
    }

    /// Whether the settings, open, were asked from its settings' button.
    fn settings_here(&self) -> bool {
        let from = crate::ui::settings_window::asked_from();
        from.is_some() && from == self.settings_from
    }

    /// One of its buttons pressed: what is asked for, where it is another's to do.
    pub fn bar_press(&mut self) -> Option<(Choice, RECT)> {
        let button = self.bar_button_at(cursor()?)?;
        // (Where the button is on the screen.)
        let place = self.button_place(button);
        if button == Button::Settings {
            self.settings_from = Some(place);
        }
        if self.tip.take().is_some() {
            crate::ui::tip::hide();
        }
        // (Its buttons come again as the pointer stays: at once, or, letting clicks through, after a while.)
        self.over = None;
        match button {
            Button::Game => {
                // Showing a game (found or asked for): off; else on.
                let shown = self.game;
                crate::source::set_game_mode(if shown { crate::presents::GameMode::Off } else { crate::presents::GameMode::On });
                None
            }
            Button::Settings => Some(Choice::Settings),
            Button::Pin => Some(Choice::Lock(!self.locked)),
            Button::Through => Some(Choice::Through(!self.through)),
            Button::Close => Some(Choice::Close),
            // (Not among its buttons: the music's.)
            Button::Full => None,
        }
        .map(|choice| (choice, place))
    }

    /// The edges of it the pointer at `at` is on, if it is near them.
    fn edges_at(&self, at: POINT) -> Edges {
        let near = (EDGE * self.dpi).round() as i32;
        let r = self.rect;
        let corner = ((CORNER * self.dpi).round() as i32).min((r.right - r.left).min(r.bottom - r.top) / 2).max(near);
        // How far in from each edge; near one, or in a corner, near both
        // of the corner's.
        let (left, right, top, bottom) = (at.x - r.left, r.right - 1 - at.x, at.y - r.top, r.bottom - 1 - at.y);
        let on = |own: i32, a: i32, b: i32| own < near || (own < corner && (a < corner || b < corner));
        Edges { left: on(left, top, bottom), right: on(right, top, bottom), top: on(top, left, right), bottom: on(bottom, left, right) }
    }

    /// The pointer's shape over edges `edges` (none: the whole overlay).
    fn point_at(&self, edges: Edges) {
        self.window.point(crate::panel::sizing(edges.left, edges.right, edges.top, edges.bottom));
    }

    /// The left button went down on it: unless it is locked, it is moved, or
    /// if the pointer is on its edges, sized, until it is let go.
    pub fn press(&mut self) {
        let Some(from) = cursor().filter(|_| !self.locked) else { return };
        let edges = self.edges_at(from);
        self.grab = Some(Grab { from, glass: self.rect, size: self.size, edges });
        // (Its buttons are away while it is in hand.)
        self.watch();
        unsafe {
            SetCapture(self.window.hwnd);
            SetTimer(Some(self.window.hwnd), FOLLOW_TIMER, FOLLOW_MS, None);
        }
        self.draw();
    }

    /// Whether it is being dragged: till it is told the button went up.
    pub fn held(&self) -> bool {
        self.grab.is_some()
    }

    /// The pointer moved over it, or while it is dragged: it moves, or grows
    /// and shrinks as a whole from the corner opposite the edges held (its
    /// shape is its readings').
    pub fn moved(&mut self) {
        self.watch();
        let Some(at) = cursor() else { return };
        let Some(grab) = self.grab else {
            if self.locked {
                self.window.point(IDC_ARROW);
            } else {
                self.point_at(self.edges_at(at));
            }
            return;
        };
        self.point_at(grab.edges);
        let (dx, dy) = (at.x - grab.from.x, at.y - grab.from.y);
        let g = grab.glass;
        if !grab.edges.any() {
            let rect = RECT { left: g.left + dx, top: g.top + dy, right: g.right + dx, bottom: g.bottom + dy };
            // Followed at every tick: moved only when the pointer has; its stage with it, what is on it as it was.
            if rect != self.rect {
                let (by_x, by_y) = (rect.left - self.rect.left, rect.top - self.rect.top);
                let s = self.stage_at;
                self.stage_at = RECT { left: s.left + by_x, top: s.top + by_y, right: s.right + by_x, bottom: s.bottom + by_y };
                self.rect = rect;
                self.place();
            }
            return;
        }
        let (width, height) = ((g.right - g.left) as f32, (g.bottom - g.top) as f32);
        let e = grab.edges;
        let across = (e.left || e.right).then(|| (width + if e.right { dx } else { -dx } as f32) / width);
        let down = (e.top || e.bottom).then(|| (height + if e.bottom { dy } else { -dy } as f32) / height);
        let ratio = match (across, down) {
            (Some(a), Some(d)) => a.max(d),
            (Some(a), None) => a,
            (None, Some(d)) => d,
            (None, None) => 1.0,
        };
        let size = (grab.size * ratio).clamp(crate::panel::SIZES.0, crate::panel::SIZES.1);
        if size == self.size {
            return;
        }
        self.size = size;
        let (width, height) = self.dims();
        let x = if e.left { g.right - width } else { g.left };
        let y = if e.top { g.bottom - height } else { g.top };
        self.rect = RECT { left: x, top: y, right: x + width, bottom: y + height };
        self.draw();
    }

    /// Let go: where it now is and how large, as kept; none unless it was
    /// being dragged.
    pub fn release(&mut self) -> Option<Placed> {
        self.grab.take()?;
        unsafe {
            let _ = KillTimer(Some(self.window.hwnd), FOLLOW_TIMER);
            let _ = ReleaseCapture();
        }
        let glass = self.rect;
        let centre = POINT { x: (glass.left + glass.right) / 2, y: (glass.top + glass.bottom) / 2 };
        (self.screen, self.dpi) = monitor_info(unsafe { MonitorFromPoint(centre, MONITOR_DEFAULTTONEAREST) });
        // Tinted for where it was put.
        self.look();
        self.draw();
        let (left, top, across, down) = room(self.screen, (glass.right - glass.left, glass.bottom - glass.top), self.dpi);
        let share = |at: i32, room: i32| if room > 0 { (at as f32 / room as f32).clamp(0.0, 1.0) } else { 0.0 };
        Some(Placed { at: (share(glass.left - left, across), share(glass.top - top, down)), point: (centre.x, centre.y), size: self.size })
    }

    /// Its glass left as clear as it was made for a moment: no longer said.
    pub fn hush(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.window.hwnd), SAID_TIMER);
        }
        if self.said.take().is_some() && self.shown {
            self.draw();
        }
    }

    /// Its menu, and what was chosen in it (the focus put back where it
    /// was by the menu itself).
    /// Kept under `menu`'s window while it is up (none: no menu is): placed above all as it is drawn (at each sample,
    /// as its glass clears, as its bar shows), it covered the menu, and a click on it went to the menu underneath.
    pub fn under_menu(&mut self, menu: Option<HWND>) {
        self.menu = menu;
        self.menu_above();
    }

    fn menu_above(&self) {
        use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE};
        if let Some(menu) = self.menu {
            unsafe {
                let _ = SetWindowPos(menu, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
    }

    /// Its menu, up (see `chose`).
    pub fn menu(&self, over_the_panel: bool) -> Option<crate::ui::menu::Menu> {
        let zh = self.lang == Lang::Zh;
        use crate::ui::icons::Icon;
        use crate::ui::menu::{Item, Menu};
        let items = vec![
            Item { label: if zh { "锁定位置" } else { "Lock position" }, mark: Some(crate::ui::menu::Mark::Icon(Icon::Pin)), checked: self.locked, rule_before: false },
            Item { label: if zh { "鼠标穿透" } else { "Let clicks through" }, mark: Some(crate::ui::menu::Mark::Icon(Icon::Pointer)), checked: self.through, rule_before: false },
            Item { label: if zh { "关闭悬浮窗" } else { "Close the overlay" }, mark: Some(crate::ui::menu::Mark::Icon(Icon::Close)), checked: false, rule_before: false },
            Item { label: if zh { "设置…" } else { "Settings…" }, mark: Some(crate::ui::menu::Mark::Icon(Icon::Settings)), checked: false, rule_before: true },
        ];
        Menu::open(items, crate::ui::menu::Look::system(), over_the_panel)
    }

    /// The choice at `row` of its menu made (see `menu`).
    pub fn chose(&self, row: usize) -> Choice {
        match row {
            0 => Choice::Lock(!self.locked),
            1 => Choice::Through(!self.through),
            2 => Choice::Close,
            _ => Choice::Settings,
        }
    }

    /// Whether it is hidden on a device no screen would be drawn for any longer (see `gfx::needed`: a laptop's discrete
    /// card, its screen unplugged as no game was played): let go of then, all it draws with going with it, and made
    /// again as it is next wanted (as it is first), so that it keeps no card awake meanwhile.
    pub fn hidden_on_a_card_not_needed(&self) -> bool {
        !self.shown && !gfx::needed(&self.gfx)
    }

    fn hide(&mut self) {
        if self.grab.take().is_some() {
            unsafe {
                let _ = ReleaseCapture();
            }
        }
        if !self.shown {
            return;
        }
        self.shown = false;
        self.window.hide();
        self.stage.hide();
        self.bar_shows.clear();
        self.over = None;
        self.draw_bar();
        if self.tip.take().is_some() {
            crate::ui::tip::hide();
        }
        crate::widget::own_window_changed(self.shadow_rect());
        self.placed.set(RECT::default());
        // (What it draws with given back while it is away: see `life`.)
        self.hidden();
    }
}
