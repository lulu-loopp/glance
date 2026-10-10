//! The studio: Glance's panel filmed off screen, frame by frame, from a
//! script of shots, for the promotional video and the README's pictures.
//! It draws with the panel's own code, over a chosen picture, with readings
//! made up to a script (a load that rises and falls) on this machine's own
//! hardware names. Built only with the `studio` feature:
//!
//! ```text
//! cargo run --release --features studio -- --studio studio\shots.json target\studio
//! ```
//!
//! Each shot becomes a folder of numbered PNG frames.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use windows::core::HSTRING;
use windows::Win32::Foundation::{GENERIC_WRITE, RECT};
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::{D2D1_INTERPOLATION_MODE_LINEAR, D2D1_LAYER_PARAMETERS1};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA, IWICImagingFactory,
    WICBitmapEncoderNoCache,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows_numerics::{Matrix3x2, Vector2};

use crate::panel::{CLOSE, OPEN, OPEN_FADE, PEN_LAG_MS};
use crate::reading::{
    BatterySample, BoardSensors, CpuSensors, DriveTemperature, GameSample, GpuLimit, GpuSample, MemorySample, NetworkInfo, ProcessSample,
    Sample, StaticInfo, SystemSample, VolumeSample,
};
use crate::settings::Edge;
use crate::ui::gfx::Gfx;
use crate::ui::prefs::Prefs;
use crate::ui::arrange::{self, GAP};
use crate::ui::render::{self, PanelLayers};
use crate::ui::seen::Seen;
use crate::ui::skins;
use crate::ui::text::Lang;
use crate::ui::theme::{self, Entrance, Skin, Theme};
use crate::ui::view::{self, Scene};
use crate::ui::wallpaper;
use crate::ui::arrange::{Choice, Form, Want};
use crate::ui::backdrop::Capture;
use crate::ui::forms::{self, Small};
use crate::ui::gfx::{Frame, Layer};
use crate::ui::morph::{self, Part};
use crate::widget::{GIVE, MORPH, SETTLE};

#[derive(Deserialize)]
struct Script {
    /// The frames' size in pixels, and how many pixels a panel DIP takes.
    width: u32,
    height: u32,
    scale: f32,
    fps: f32,
    /// The picture the panel is filmed over, as the desktop.
    wallpaper: String,
    /// How tall the taskbar along the bottom of the picture is (DIPs): the
    /// screen's work area ends above it.
    #[serde(default)]
    taskbar: f32,
    shots: Vec<Shot>,
}

#[derive(Deserialize)]
struct Shot {
    name: String,
    /// "paper", "glass" or "fluent".
    skin: String,
    /// "light", "dark", or "backdrop" (as the picture behind decides).
    #[serde(default = "backdrop")]
    theme: String,
    /// "zh" or "en".
    #[serde(default = "zh")]
    lang: String,
    seconds: f32,
    /// When the panel slides in; shown from the start if absent.
    slide_in: Option<f32>,
    /// When it slides away again, if it does.
    slide_out: Option<f32>,
    /// How many times slower than life the sliding is filmed (1 if absent).
    #[serde(default)]
    slow: f32,
    /// The load, 0 (idle) to 1 (flat out), from the start to the end.
    #[serde(default)]
    load: (f32, f32),
    /// The load at given moments instead, (seconds, load) in order, eased
    /// between them and held past the ends.
    #[serde(default)]
    loads: Vec<(f32, f32)>,
    /// The lanes shown, in order; all of this machine's but "system" if absent.
    modules: Option<Vec<String>>,
    /// A game being played, by its name, if one is.
    game: Option<String>,
    /// When the game starts (from the start, if absent).
    #[serde(default)]
    game_from: f32,
    /// A widget's menu, over the first widget of the shot.
    menu: Option<MenuShot>,
    /// The overlay, in the screen's top left corner, laid out so (a strip
    /// in one row).
    overlay: Option<crate::ui::overlay::Layout>,
    /// Whether the panel is in the shot (it is unless said).
    #[serde(default = "yes")]
    panel: bool,
    /// A widget torn off the panel, and where it is taken after.
    torn: Option<Torn>,
    /// Widgets on the desktop from the start.
    #[serde(default)]
    widgets: Vec<Placed>,
}

fn yes() -> bool {
    true
}

/// A widget's right-click menu shown from `from` to `to` seconds with its
/// corner `at` (DIPs from the widget's corner), the choice under the
/// pointer from each moment on, (seconds, choice) in order.
#[derive(Deserialize)]
struct MenuShot {
    from: f32,
    to: f32,
    at: (f32, f32),
    #[serde(default)]
    lit: Vec<(f32, usize)>,
}

/// Where a widget would stick, as it is shown: its box (DIPs on the
/// screen), whether firm, how strong.
type Shows = (f32, f32, f32, f32, bool, f32);

/// A widget's track, while it shows, and the room it was first sized to
/// (a widget placed from the start).
type Following<'a> = (Option<&'a Track>, Option<(f32, f32)>);

/// A widget torn off the panel: when (the panel goes as it is), and where
/// it is taken and how large it is made, key by key, eased between them
/// (its corner from where the panel's corner rests).
#[derive(Deserialize)]
struct Torn {
    at: f32,
    #[serde(flatten)]
    track: Track,
}

/// A widget on the desktop from the start: its corner (DIPs on the screen),
/// the room it was sized to, and where it is taken after (from its corner).
#[derive(Deserialize)]
struct Placed {
    at: (f32, f32),
    room: (f32, f32),
    #[serde(flatten)]
    track: Track,
}

/// Where a widget is taken, key by key; and where it shows it would stick
/// as it nears an edge.
#[derive(Deserialize)]
struct Track {
    keys: Vec<Key>,
    /// Whether its keys place its bottom edge (else its top): sized, it
    /// grows and shrinks from the bottom up, as one standing on the
    /// taskbar would.
    #[serde(default)]
    bottom: bool,
    #[serde(default)]
    hint: Option<Hint>,
}

/// Where a widget is at `t` seconds: its corner, DIPs from its track's
/// origin; while it is sized by its corner, the room it is dragged to (DIPs
/// on the screen), between two keys that both have one; and from a key
/// that says so on, stuck to that edge of the screen ("left", "right",
/// "top", "bottom").
#[derive(Deserialize, Clone)]
struct Key {
    t: f32,
    x: f32,
    y: f32,
    #[serde(default)]
    room: Option<(f32, f32)>,
    #[serde(default)]
    stick: Option<String>,
}

/// Where a widget would stick, shown from `from` seconds as it nears the
/// `side`, firm from `firm` (letting go would stick it), till it sticks.
#[derive(Deserialize)]
struct Hint {
    side: String,
    from: f32,
    firm: f32,
}

/// Where the widget is at `t` (its corner from its origin) and the room it
/// is sized to, if it is: eased from key to key, held past the last.
fn keyed(keys: &[Key], t: f32) -> ((f32, f32), Option<(f32, f32)>) {
    let Some(next) = keys.iter().position(|key| key.t > t) else {
        let last = keys.last().expect("a widget's track has keys");
        return ((last.x, last.y), None);
    };
    if next == 0 {
        return ((keys[0].x, keys[0].y), None);
    }
    let (a, b) = (&keys[next - 1], &keys[next]);
    let s = ((t - a.t) / (b.t - a.t)).clamp(0.0, 1.0);
    let s = s * s * (3.0 - 2.0 * s);
    let mix = |p: f32, q: f32| p + (q - p) * s;
    let room = a.room.zip(b.room).map(|(p, q)| (mix(p.0, q.0), mix(p.1, q.1)));
    ((mix(a.x, b.x), mix(a.y, b.y)), room)
}

/// The edge the widget is stuck to at `t`, if a key up to then says so.
fn stuck(keys: &[Key], t: f32) -> Option<&str> {
    keys.iter().take_while(|key| key.t <= t).filter_map(|key| key.stick.as_deref()).last()
}

/// The small layout a widget stuck to `side` shows, as the widget's own.
fn stuck_form(side: &str) -> Form {
    match side {
        "left" | "right" => Form::Small(Small::Rail),
        _ => Form::Small(Small::Strip { rich: false }),
    }
}

/// Where a widget `size` DIPs large stuck to `side` of `work` has its
/// corner, its middle `along` the edge where it was (as `Widget::stuck_at`).
fn stuck_at(side: &str, along: f32, size: (f32, f32), work: (f32, f32)) -> (f32, f32) {
    let (x, y) = match side {
        "left" => (0.0, along - size.1 / 2.0),
        "right" => (work.0 - size.0, along - size.1 / 2.0),
        "top" => (along - size.0 / 2.0, 0.0),
        _ => (along - size.0 / 2.0, work.1 - size.1),
    };
    (x.clamp(0.0, (work.0 - size.0).max(0.0)), y.clamp(0.0, (work.1 - size.1).max(0.0)))
}

/// What a filmed widget showed: its form at its zoom, its size (its own
/// DIPs), its lanes laid out (or the small layout it showed) and how much
/// of each; as the widget's own (see `widget.rs`).
#[derive(Clone)]
struct Shown {
    form: Form,
    zoom: f32,
    size: (f32, f32),
    layout: view::Layout,
    small: Option<Small>,
    detail: view::Detail,
}

/// What a layout turns from: what was shown, or what was drawn as it was
/// still turning.
enum Before {
    Shown(Shown),
    Drawn(Vec<Part>),
}

/// A torn widget as it is filmed: laid out, turned from layout to layout
/// and settled as the widget on the desktop is, on the shot's clock.
struct Filming {
    /// Where its track's keys are from (DIPs on the screen).
    origin: (f32, f32),
    /// Stuck to an edge: which, and where its middle is along it.
    stuck: Option<(String, f32)>,
    /// Where it shows on the screen (DIPs), as last laid out.
    at: (f32, f32),
    choice: Choice,
    taken: Option<arrange::Shown>,
    last: Option<Shown>,
    morph: Option<(f32, Before)>,
    drawn: Vec<Part>,
    /// Let go after it was sized: since when, from what surface.
    settle: Option<(f32, (f32, f32))>,
    sized: Option<(f32, f32)>,
    surface: (f32, f32),
    layers: PanelLayers,
    morph_layer: Layer,
}

impl Filming {
    fn new(choice: Choice, origin: (f32, f32)) -> Self {
        Filming {
            origin,
            stuck: None,
            at: origin,
            choice,
            taken: None,
            last: None,
            morph: None,
            drawn: Vec::new(),
            settle: None,
            sized: None,
            surface: (0.0, 0.0),
            layers: PanelLayers::default(),
            morph_layer: Layer::default(),
        }
    }

    /// Laid out at `t` as its `track` has it: moved, sized, or stuck to an
    /// edge of `work` (DIPs); where it shows is left in `at`.
    fn follow(&mut self, t: f32, track: &Track, scene: &Scene, theme: &Theme, seen: &Seen, work: (f32, f32)) {
        let (offset, room) = keyed(&track.keys, t);
        if let Some(side) = stuck(&track.keys, t).filter(|_| self.stuck.is_none()) {
            // Stuck: the small layout of its edge, settling from what it
            // showed, its middle where it was along the edge.
            let zoom = self.last.as_ref().map_or(1.0, |shown| shown.zoom);
            let middle = match side {
                "left" | "right" => self.at.1 + self.surface.1 * zoom / 2.0,
                _ => self.at.0 + self.surface.0 * zoom / 2.0,
            };
            self.stuck = Some((side.to_string(), middle));
            self.choice = Choice { form: stuck_form(side), scale: 1.0, extra: (0.0, 0.0) };
            self.settle = Some((t, self.surface));
        }
        let room = room.filter(|_| self.stuck.is_none());
        let shown = self.lay_out(t, scene, theme, seen, work, room);
        self.at = match &self.stuck {
            Some((side, along)) => stuck_at(side, *along, (self.surface.0 * shown.zoom, self.surface.1 * shown.zoom), work),
            None if track.bottom => (self.origin.0 + offset.0, self.origin.1 + offset.1 - self.surface.1 * shown.zoom),
            None => (self.origin.0 + offset.0, self.origin.1 + offset.1),
        };
    }

    /// Where it would stick to `side` of `work` (DIPs on the screen): as
    /// large as it would be there, its middle where its middle is now.
    fn would_stick(&self, side: &str, scene: &Scene, theme: &Theme, seen: &Seen, work: (f32, f32)) -> (f32, f32, f32, f32) {
        let heights = view::heights(scene);
        let lanes: Vec<(&str, arrange::Heights)> = heights.iter().map(|(id, h)| (id.as_str(), *h)).collect();
        let kept = Want::Kept(Choice { form: stuck_form(side), scale: 1.0, extra: (0.0, 0.0) });
        let there = arrange::Opening::new(theme, Edge::Right, &lanes, forms::count(scene), work, None, 1.0, kept, seen.clone());
        let size = (there.size.0 * there.zoom, there.size.1 * there.zoom);
        let zoom = self.last.as_ref().map_or(1.0, |shown| shown.zoom);
        let middle = match side {
            "left" | "right" => self.at.1 + self.surface.1 * zoom / 2.0,
            _ => self.at.0 + self.surface.0 * zoom / 2.0,
        };
        let (x, y) = stuck_at(side, middle, size, work);
        (x, y, size.0, size.1)
    }

    /// Laid out at `t` for `room` if it is being sized, else as it was
    /// kept: what it shows, and its surface (its own DIPs).
    fn lay_out(&mut self, t: f32, scene: &Scene, theme: &Theme, seen: &Seen, work: (f32, f32), room: Option<(f32, f32)>) -> Shown {
        let heights = view::heights(scene);
        let lanes: Vec<(&str, arrange::Heights)> = heights.iter().map(|(id, h)| (id.as_str(), *h)).collect();
        let readings = forms::count(scene);
        let want = match room {
            Some(room) => Want::Room(room, Some(self.taken.filter(|taken| taken.form == self.choice.form).unwrap_or(arrange::Shown { form: self.choice.form, taken: room }))),
            None => Want::Kept(self.choice),
        };
        let opening = arrange::Opening::new(theme, Edge::Right, &lanes, readings, work, None, 1.0, want, seen.clone());
        let choice = opening.choice.unwrap_or(self.choice);
        if let Some(room) = room {
            self.taken = Some(arrange::Shown::after(self.taken, choice.form, room));
        }
        self.choice = choice;
        let shown = Shown { form: choice.form, zoom: opening.zoom, size: opening.size, layout: opening.layout.clone(), small: opening.small(), detail: opening.detail() };
        if let Some(last) = self.last.take().filter(|last| last.form != shown.form) {
            let before = if self.morph.is_some() { Before::Drawn(std::mem::take(&mut self.drawn)) } else { Before::Shown(last) };
            self.morph = Some((t, before));
        }
        self.last = Some(shown.clone());
        if self.morph.as_ref().is_some_and(|(since, _)| t - since >= MORPH.as_secs_f32()) {
            self.morph = None;
            self.drawn.clear();
        }
        // Its surface: the room the hand gives it, past what it shows only
        // part of the way; settling from that once let go.
        if self.sized.is_some() && room.is_none() {
            self.settle = Some((t, self.surface));
        }
        self.sized = room;
        let give = |have: f32, fill: f32| if have <= fill { fill } else { fill + (have - fill) * GIVE };
        self.surface = match (room, self.settle) {
            (Some(room), _) => (give(room.0 / shown.zoom, shown.size.0), give(room.1 / shown.zoom, shown.size.1)),
            (None, Some((since, from))) => {
                let k = ((t - since) / SETTLE.as_secs_f32()).min(1.0);
                if k >= 1.0 {
                    self.settle = None;
                }
                let eased = morph::ease(k);
                (from.0 + (shown.size.0 - from.0) * eased, from.1 + (shown.size.1 - from.1) * eased)
            }
            _ => shown.size,
        };
        shown
    }

    /// Draws it at `at` on the frame (DIPs on the screen), at `t`.
    #[allow(clippy::too_many_arguments)]
    fn draw(&mut self, frame: &Frame, scene: &Scene, theme: &Theme, desktop: &Capture, frost: f32, px: f32, at: (f32, f32), t: f32, rough: bool) {
        let Some(shown) = self.last.clone() else { return };
        let zoom = shown.zoom;
        let size = self.surface;
        let lanes = view::lanes_at(scene, |_| shown.detail);
        let behind = desktop.bitmap(&frame.dc, px * zoom).ok();
        let backdrop = behind.as_ref().map(|bitmap| (bitmap, Vector2 { X: at.0 / zoom, Y: at.1 / zoom }, desktop.digest, px * zoom));
        let small = shown.small.map(|small| (small, shown.size));
        let picture = render::Picture { scene, lanes: &lanes, layout: &shown.layout, small, size, at: (0.0, 0.0), edge: None, backdrop, frost, rough };
        let local = Matrix3x2::scale(zoom, zoom) * Matrix3x2::translation(at.0, at.1);
        let turning = self.morph.as_ref().map(|(since, _)| morph::ease((t - since) / MORPH.as_secs_f32()));
        match (turning, self.morph.as_mut().map(|(_, before)| before)) {
            (Some(k), Some(before)) => {
                let _ = self.layers.draw_ground(frame, &picture, local, px * zoom);
                if let Before::Shown(old) = before {
                    let old_lanes = view::lanes_at(scene, |_| old.detail);
                    let scale = old.zoom / zoom;
                    *before = Before::Drawn(render::record(frame, scene, &old_lanes, &old.layout, old.small.map(|small| (small, old.size)), scale, (0.0, 0.0)));
                }
                let now_drawn = render::record(frame, scene, &lanes, &shown.layout, small, 1.0, (0.0, 0.0));
                let Before::Drawn(from) = before else { return };
                let parts = morph::between(from, &now_drawn, k);
                let halo = (theme.skin == Skin::Glass).then_some(theme.legibility);
                let _ = self.morph_layer.draw(frame, k.to_bits() as u64 + 1, local, (0.0, 0.0, size.0, size.1), px * zoom, halo, |frame| {
                    morph::draw(frame, &parts);
                    Ok(())
                });
                self.drawn = parts;
            }
            _ => {
                let _ = self.layers.draw(frame, &picture, local, px * zoom);
            }
        }
    }
}

fn backdrop() -> String {
    "backdrop".into()
}

fn zh() -> String {
    "zh".into()
}

/// Films every shot in `script` into a folder of its own under `out`.
pub fn run(script: &Path, out: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(script).map_err(|e| format!("{}: {e}", script.display()))?;
    let script: Script = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", script.display()))?;
    // This machine's own names, read as Glance reads them.
    let info = crate::metrics::Sampler::new().info;
    let gfx = Gfx::new().map_err(|e| e.to_string())?;
    let desktop = wallpaper::picture(&script.wallpaper, script.width, script.height).map_err(|e| format!("{}: {e}", script.wallpaper))?;
    for shot in &script.shots {
        let folder = out.join(&shot.name);
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        film(&gfx, &script, shot, &info, &desktop, &folder)?;
        println!("{}: {} frames", shot.name, (shot.seconds * script.fps).round());
    }
    Ok(())
}

fn film(gfx: &Gfx, script: &Script, shot: &Shot, info: &StaticInfo, desktop: &crate::ui::backdrop::Capture, folder: &Path) -> Result<(), String> {
    let skin = Skin::named(&shot.skin);
    let lang = if shot.lang == "en" { Lang::En } else { Lang::Zh };
    let px = script.scale;
    let (sw, sh) = (script.width as f32 / px, script.height as f32 / px);
    let mut known = vec!["game".to_string(), "cpu".to_string()];
    known.extend(info.gpu_modules());
    known.extend(["memory", "network", "disk", "processes", "storage", "board", "battery", "system", "wsl", "docker"].map(String::from));
    let mut prefs = Prefs::resolve(&serde_json::Value::Null, &known);
    if let Some(shown) = &shot.modules {
        prefs.modules.sort_by_key(|entry| shown.iter().position(|id| *id == entry.id).unwrap_or(usize::MAX));
        prefs.modules.iter_mut().for_each(|entry| entry.on = shown.contains(&entry.id));
    }

    let frames = (shot.seconds * script.fps).round() as usize;
    let interval = 1000.0;
    let start_ms = 1_760_000_000_000.0;
    let backlog = prefs.chart_seconds + 5.0;
    let load = |t: f32| match shot.loads.iter().position(|(at, _)| *at > t) {
        _ if shot.loads.is_empty() => shot.load.0 + (shot.load.1 - shot.load.0) * (t / shot.seconds).clamp(0.0, 1.0),
        Some(0) => shot.loads[0].1,
        None => shot.loads[shot.loads.len() - 1].1,
        Some(next) => {
            let ((a, p), (b, q)) = (shot.loads[next - 1], shot.loads[next]);
            let k = ((t - a) / (b - a)).clamp(0.0, 1.0);
            p + (q - p) * k * k * (3.0 - 2.0 * k)
        }
    };
    // A sample a second, from well before the shot to its end.
    let history: Vec<Sample> = (0..=(backlog + shot.seconds as f64) as usize)
        .map(|i| {
            let t = (i as f64 - backlog) as f32;
            made_up(info, start_ms + i as f64 * interval - backlog * 1000.0, i as u64, load(t.max(0.0)), shot.game.as_deref().filter(|_| t >= shot.game_from))
        })
        .collect();

    // Laid out and placed as the panel would be on a screen this size, for
    // the readings of the moment, and held through the shot as the panel
    // holds its outline while it is up (see `arrange::Opening`).
    let edge = Edge::Right;
    let opening: std::cell::RefCell<Option<arrange::Opening>> = std::cell::RefCell::new(None);
    let place = |scene: &Scene, theme: &Theme| {
        let mut held = opening.borrow_mut();
        let opening = match held.as_mut() {
            Some(opening) => {
                opening.grow(scene.seen, info, |id, seen, detail| view::lane_height(scene, id, seen, detail));
                opening
            }
            None => {
                let heights = view::heights(scene);
                let lanes: Vec<(&str, arrange::Heights)> = heights.iter().map(|(id, heights)| (id.as_str(), *heights)).collect();
                held.insert(arrange::Opening::new(theme, edge, &lanes, 0, (sw, sh), None, 1.0, arrange::Want::Itself, scene.seen.clone()))
            }
        };
        let (layout, zoom) = (opening.layout.clone(), opening.zoom);
        let (pw, ph) = (layout.width() * zoom, layout.height() * zoom);
        let rest = (sw - theme.inset * zoom - pw, ((sh - ph) / 2.0).max(GAP));
        (layout, zoom, rest, opening.seen.clone())
    };
    // Toned by the desktop where it first rests.
    let measure = Theme::new(skin, false);
    let first = &history[..=backlog as usize];
    let first_seen = Seen::of(first);
    let probe = Scene { info, prefs: &prefs, theme: &measure, lang, history: first, seen: &first_seen, pen_ms: 0.0, process_scroll: 0.0, hover: None, pinned: false, overlay: false, buttons: true };
    let (layout, zoom, rest, _) = place(&probe, &measure);
    let (pw, ph) = (layout.width() * zoom, layout.height() * zoom);
    let behind = RECT {
        left: (rest.0 * px) as i32,
        top: (rest.1 * px) as i32,
        right: ((rest.0 + pw) * px) as i32,
        bottom: ((rest.1 + ph) * px) as i32,
    };
    let tone = desktop.luminance(behind, px);
    let dark = match shot.theme.as_str() {
        "light" => false,
        "dark" => true,
        _ => theme::is_dark(crate::ui::prefs::ThemePref::Backdrop, Some(tone.0).filter(|_| skin.sees_backdrop())),
    };
    let theme = Theme::new(skin, dark);
    let frost = if skin == Skin::Glass { skins::frost(tone.0, tone.1, dark) } else { 0.0 };
    let ids: Vec<&str> = prefs.modules.iter().filter(|entry| entry.on).map(|entry| entry.id.as_str()).collect();
    let mut placed = serde_json::Value::Null;

    let mut layers = PanelLayers::default();
    // The widgets: the one torn off (once it is), then those placed.
    let mut torn: Option<Filming> = None;
    let mut placed_widgets: Vec<Filming> = shot.widgets.iter().map(|w| Filming::new(Choice { form: Form::Lanes { detail: view::Detail::Full, columns: 1 }, scale: 1.0, extra: (0.0, 0.0) }, w.at)).collect();
    let work = (sw, sh - script.taskbar);
    let mut placements = Vec::new();
    for index in 0..frames {
        let t = index as f32 / script.fps;
        let now_ms = start_ms + t as f64 * 1000.0;
        let read = history.iter().take_while(|sample| sample.t as f64 <= now_ms).count().max(1);
        let seen = Seen::of(&history[..read]);
        let scene = Scene {
            info,
            prefs: &prefs,
            theme: &theme,
            lang,
            history: &history[..read],
            seen: &seen,
            pen_ms: now_ms - interval - PEN_LAG_MS,
            process_scroll: 0.0,
            hover: None,
            pinned: false,
            overlay: false,
            buttons: true,
        };
        let (layout, zoom, rest, held) = place(&scene, &theme);
        // What the panel holds as it has been up: drawn in the boxes it opened with.
        let scene = Scene { seen: &held, ..scene };
        let lanes = view::lanes_at(&scene, |_| view::Detail::Full);
        let travel = match theme.entrance {
            Entrance::Beyond(extra) => layout.width() + extra,
            Entrance::Slide(distance) => distance,
        };
        // Where the panel, the pieces it is drawn as (each lane and the bar
        // on glass, else one slab) and each lane are in the frame, in
        // pixels, for the compositor: where the shot leaves them.
        let rect = |r: &view::Rect| {
            serde_json::json!({ "x": (rest.0 + r.x * zoom) * px, "y": (rest.1 + r.y * zoom) * px, "w": r.w * zoom * px, "h": r.h * zoom * px })
        };
        let panel = view::Rect { x: 0.0, y: 0.0, w: layout.width(), h: layout.height() };
        let boxes = layout.lanes();
        let pieces: Vec<_> = if skin == Skin::Glass { boxes.iter().chain([&layout.bar()]).map(rect).collect() } else { vec![rect(&panel)] };
        let named: serde_json::Map<String, serde_json::Value> = ids.iter().zip(&boxes).map(|(id, r)| (id.to_string(), rect(r))).collect();
        placed = serde_json::json!({ "panel": rect(&panel), "radius": theme.radius * zoom * px, "pieces": pieces, "lanes": named });
        // The widgets: the one torn off, from the panel as it showed; those
        // placed, laid out first for the room they were sized to. Each laid
        // out for where it is taken, how large, and the edge it sticks to;
        // and where each would stick, as it nears an edge.
        // As a widget's are (see `panel::scene_for`): its bar without the panel's buttons.
        let fresh = Scene { seen: &seen, buttons: false, ..scene };
        let mut showing: Vec<(usize, bool, Option<Shows>)> = Vec::new();
        let tracks: Vec<Following> = std::iter::once((shot.torn.as_ref().filter(|torn| t >= torn.at).map(|torn| &torn.track), None))
            .chain(shot.widgets.iter().map(|w| (Some(&w.track), Some(w.room))))
            .collect();
        if shot.torn.as_ref().is_some_and(|torn| t >= torn.at) && torn.is_none() {
            torn = Some(Filming::new(Choice { form: Form::Lanes { detail: view::Detail::Full, columns: layout.columns }, scale: zoom, extra: (0.0, 0.0) }, rest));
        }
        for (index, (track, first_room)) in tracks.iter().enumerate() {
            let Some(track) = track else { continue };
            let filming = if index == 0 { torn.as_mut().unwrap() } else { &mut placed_widgets[index - 1] };
            if filming.last.is_none() {
                if let Some(room) = first_room {
                    filming.lay_out(t, &fresh, &theme, &seen, work, Some(*room));
                    filming.sized = None;
                }
            }
            filming.follow(t, track, &fresh, &theme, &seen, work);
            let still = keyed(&track.keys, t).0 == keyed(&track.keys, t + 1.0 / script.fps).0;
            let hint = track.hint.as_ref().filter(|hint| t >= hint.from && filming.stuck.is_none()).map(|hint| {
                let (x, y, w, h) = filming.would_stick(&hint.side, &fresh, &theme, &seen, work);
                let firm = t >= hint.firm;
                // Nearer and firmer, as the widget's own (see `Widget::frame`).
                let strength = if firm { 1.0 } else { 0.25 + 0.5 * ((t - hint.from) / (hint.firm - hint.from)).clamp(0.0, 1.0) };
                (x, y, w, h, firm, strength)
            });
            showing.push((index, still, hint));
        }
        // Where the panel is at `at` seconds: off by `shift` of its travel,
        // at `opacity`.
        let moved = |at: f32| motion(shot, at);
        let mut draw = |shift: f32, opacity: f32, layers: &mut PanelLayers| {
            gfx.draw_offscreen((script.width, script.height), px, |frame| {
                let bitmap = desktop.bitmap(&frame.dc, px).ok();
                if let Some(bitmap) = &bitmap {
                    unsafe { frame.dc.DrawBitmap(bitmap, None, 1.0, D2D1_INTERPOLATION_MODE_LINEAR, None, None) };
                }
                // The panel while it shows; then the widget torn off it.
                (|| {
                    if opacity <= 0.0 {
                        return;
                    }
                let everything = D2D_RECT_F { left: -f32::MAX, top: -f32::MAX, right: f32::MAX, bottom: f32::MAX };
                let layer = D2D1_LAYER_PARAMETERS1 { contentBounds: everything, opacity, ..Default::default() };
                unsafe { frame.dc.PushLayer(&layer, None) };
                // The desktop as the panel's own DIPs measure it, zoomed with
                // the panel, as the panel takes it (its pixels per DIP
                // include the zoom).
                let behind = desktop.bitmap(&frame.dc, px * zoom).ok();
                let backdrop = behind.as_ref().map(|bitmap| (bitmap, Vector2 { X: rest.0 / zoom, Y: rest.1 / zoom }, desktop.digest, px * zoom));
                let size = (layout.width(), layout.height());
                let picture = render::Picture { scene: &scene, lanes: &lanes, layout: &layout, small: None, size, at: (0.0, 0.0), edge: Some(edge), backdrop, frost, rough: false };
                let local = Matrix3x2::scale(zoom, zoom) * Matrix3x2::translation(rest.0 + shift * travel * zoom, rest.1);
                let _ = layers.draw(frame, &picture, local, px * zoom);
                if let (Some(layout), Some(sample)) = (shot.overlay, scene.history.last()) {
                    use crate::ui::overlay::{self, Metrics, INSET, MARGIN};
                    let items = crate::settings::OverlaySettings::default().chosen();
                    let playing = sample.game.is_some();
                    let readings = overlay::readings(sample, sample.game.as_ref(), playing, &items, lang);
                    let frames = overlay::frames(scene.history.iter(), playing);
                    let width = |text: &str, font| frame.gfx.measure(text, font);
                    let line = |font| frame.gfx.baseline(font);
                    let metrics = Metrics { width: &width, line: &line };
                    let shape = overlay::Shape { layout, rows: 1, width: sw - 2.0 * INSET };
                    let (w, h) = overlay::size(&readings, shape, px, &metrics);
                    let radius = overlay::radius(&readings, shape, px, &metrics);
                    // Tinted for the desktop around it, over the desktop behind it.
                    let whole = |dips: f32| (dips * px).round() as i32;
                    let hole = RECT { left: whole(INSET), top: whole(INSET), right: whole(INSET + w), bottom: whole(INSET + h) };
                    let ring = RECT { left: 0, top: 0, right: whole(INSET + w + MARGIN + 16.0), bottom: whole(INSET + h + MARGIN + 16.0) };
                    let glass = overlay::glass(&mut desktop.luminances(ring, hole, 1), None);
                    let at = Matrix3x2::translation(INSET, INSET);
                    if let Ok(bitmap) = desktop.bitmap(&frame.dc, px) {
                        frame.frosted(&bitmap, Matrix3x2::identity(), (0.0, 0.0, w, h, radius), at);
                    }
                    frame.place(at);
                    frame.crisp_text();
                    overlay::shadow(frame, &readings, shape, glass, false, px);
                    overlay::paint(frame, &readings, &frames, shape, glass, overlay::Hand::Free, px);
                    frame.origin(0.0, 0.0);
                }
                unsafe { frame.dc.PopLayer() };
                })();
                for (index, still, hint) in &showing {
                    let filming = if *index == 0 { torn.as_mut().unwrap() } else { &mut placed_widgets[index - 1] };
                    let at = filming.at;
                    filming.draw(frame, &fresh, &theme, desktop, frost, px, at, t, !still);
                    // Where it would stick, over it (as the widget's own
                    // window for it is).
                    if let Some((x, y, w, h, firm, strength)) = hint {
                        frame.place(Matrix3x2::translation(*x, *y));
                        crate::widget::paint_hint(frame, &theme, (*w, *h), *firm, *strength);
                        frame.origin(0.0, 0.0);
                    }
                }
                // The first widget's menu, as it draws it: light or dark as
                // the theme is.
                if let (Some(menu), Some((index, ..))) = (shot.menu.as_ref().filter(|menu| (menu.from..menu.to).contains(&t)), showing.first()) {
                    let filming = if *index == 0 { torn.as_ref().unwrap() } else { &placed_widgets[index - 1] };
                    let items = crate::widget::menu_items(lang, false, false, true);
                    let sheet = crate::ui::menu::Sheet::new(gfx, &items);
                    let lit = menu.lit.iter().take_while(|(at, _)| *at <= t).last().map(|(_, choice)| *choice);
                    frame.place(Matrix3x2::translation(filming.at.0 + menu.at.0, filming.at.1 + menu.at.1));
                    sheet.paint(frame, &items, theme.dark, lit);
                    frame.origin(0.0, 0.0);
                }
            })
            .map_err(|e| e.to_string())
        };
        // While the panel moves, the frame is the mean of several moments
        // across half the frame's time: a camera's motion blur.
        let pixels = if moved(t) == moved(t + 1.0 / script.fps) {
            let (shift, opacity) = moved(t);
            draw(shift, opacity, &mut layers)?
        } else {
            const MOMENTS: usize = 8;
            let mut sum = vec![0u32; (script.width * script.height * 4) as usize];
            for k in 0..MOMENTS {
                let (shift, opacity) = moved(t + k as f32 / MOMENTS as f32 * 0.5 / script.fps);
                for (total, value) in sum.iter_mut().zip(draw(shift, opacity, &mut layers)?) {
                    *total += value as u32;
                }
            }
            sum.into_iter().map(|total| ((total + MOMENTS as u32 / 2) / MOMENTS as u32) as u8).collect()
        };
        let (shift, _) = moved(t);
        let panel_at = view::Rect { x: shift * travel, y: 0.0, w: layout.width(), h: layout.height() };
        let where_is = |filming: &Filming| {
            let zoom = filming.last.as_ref().map_or(1.0, |shown| shown.zoom);
            serde_json::json!({
                "x": filming.at.0 * px, "y": filming.at.1 * px, "w": filming.surface.0 * zoom * px, "h": filming.surface.1 * zoom * px,
                "form": filming.choice.form.key(),
            })
        };
        let widget = showing.iter().find(|(index, ..)| *index == 0).and(torn.as_ref()).map(where_is);
        let others: Vec<_> = showing.iter().filter(|(index, ..)| *index > 0).map(|(index, ..)| where_is(&placed_widgets[index - 1])).collect();
        let hints: Vec<_> = showing.iter().filter_map(|(_, _, hint)| hint.map(|(x, y, w, h, firm, _)| serde_json::json!({ "x": x * px, "y": y * px, "w": w * px, "h": h * px, "firm": firm }))).collect();
        placements.push(serde_json::json!({ "t": t, "panel": rect(&panel_at), "widget": widget, "widgets": others, "hints": hints }));
        gfx.sweep();
        save_png(&folder.join(format!("{:05}.png", index + 1)), script.width, script.height, &pixels).map_err(|e| e.to_string())?;
    }
    std::fs::write(folder.join("frames.json"), serde_json::to_string(&placements).unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(folder.join("lanes.json"), serde_json::to_string_pretty(&placed).unwrap()).map_err(|e| e.to_string())
}

/// Where a shot's panel is at `t` seconds: how far out of its travel
/// (1 out, 0 in place) and how opaque, sliding in and away as the panel
/// does, with the same curves and fades, slowed `slow` times if asked.
fn motion(shot: &Shot, t: f32) -> (f32, f32) {
    if !shot.panel {
        return (1.0, 0.0);
    }
    let slow = shot.slow.max(1.0);
    let (mut shift, mut opacity) = match shot.slide_in {
        Some(at) if t < at => (1.0, 0.0),
        Some(at) => {
            let into = (t - at) / slow;
            (1.0 - OPEN.1.at((into / OPEN.0.as_secs_f32()).min(1.0)), (into / OPEN_FADE.as_secs_f32()).min(1.0))
        }
        None => (0.0, 1.0),
    };
    if let Some(at) = shot.slide_out.or(shot.torn.as_ref().map(|torn| torn.at)).filter(|at| t >= *at) {
        let out = ((t - at) / slow / CLOSE.0.as_secs_f32()).min(1.0);
        shift = shift.max(CLOSE.1.at(out));
        opacity = opacity.min(1.0 - out);
    }
    (shift, opacity)
}

/// Readings for a machine with `info`'s hardware, at `load` (0 idle, 1 flat
/// out), playing `game` if one is named, varying from second to second as
/// real ones do: the same for the same `seed`, so that a shot films the
/// same every time.
fn made_up(info: &StaticInfo, t_ms: f64, seed: u64, load: f32, game: Option<&str>) -> Sample {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
    let mut noise = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 40) as f32 / (1u64 << 24) as f32
    };
    let wave = |period: f32, phase: f32| ((seed as f32 / period + phase) * std::f32::consts::TAU).sin() * 0.5 + 0.5;
    let busy = load.clamp(0.0, 1.0);
    let cpu = (4.0 + 62.0 * busy + 8.0 * wave(9.0, 0.1) * busy + 3.0 * noise()).clamp(0.0, 100.0);
    let threads = (0..info.threads)
        .map(|i| Some((cpu * (0.4 + 1.2 * ((i as f32 * 0.61 + seed as f32 * 0.07).sin() * 0.5 + 0.5)) + 4.0 * noise()).clamp(0.0, 100.0)))
        .collect();
    let gb = 1u64 << 30;
    let gpus = info
        .gpus
        .iter()
        .enumerate()
        .map(|(i, gpu)| {
            // The first card does the work; the others rest.
            let g = if i == 0 { busy } else { 0.02 };
            let usage = (2.0 + 92.0 * g + 4.0 * wave(5.0, 0.3) * g + 2.0 * noise()).clamp(0.0, 100.0);
            GpuSample {
                usage: Some(usage),
                engines: Some(vec![("3D".into(), usage), ("Copy".into(), 3.0 * g), ("VideoDecode".into(), 0.0)]),
                mem_used: Some(((0.12 + 0.55 * g) * gpu.mem_total as f32) as u64),
                shared_used: Some((0.2 * gb as f32) as u64),
                temp: Some(36.0 + 34.0 * g + 1.5 * noise()),
                clock_mhz: Some(if g > 0.05 { 2400.0 + 300.0 * g + 30.0 * noise() } else { 210.0 + 40.0 * noise() }),
                // A card whose fans turn even at rest, so that its lane
                // keeps its rows through a shot.
                fan_rpm: Some((780.0 + 1100.0 * g) as u32),
                power: (i == 0).then_some(18.0 + 255.0 * g + 12.0 * wave(4.0, 0.7) * g + 2.0 * noise()),
            }
        })
        .collect();
    let used = ((0.22 + 0.25 * busy + 0.02 * noise()) * info.mem_total as f32) as u64;
    let package = 38.0 + 36.0 * busy + 2.0 * wave(7.0, 0.2) + noise();
    let names = ["blender", "chrome", "code", "obs64", "glance", "explorer", "dwm", "svchost"];
    let processes = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let share = 1.0 / (i as f32 + 1.5);
            ProcessSample {
                name: name.to_string(),
                cpu: cpu * share * (0.8 + 0.4 * noise()),
                mem: ((0.5 + 3.0 * share) * gb as f32) as u64,
                io: (2.0e6 * share * (0.5 + noise())) as f64,
                gpu: Some(if i == 0 { 60.0 * busy } else { 2.0 * share }),
            }
        })
        .collect();
    // Traffic comes in bursts: now and then several times the usual.
    let mut burst = |base: f64| {
        let (level, spike) = (noise() as f64, noise());
        base * (0.3 + 1.4 * level) * if spike > 0.8 { 6.0 } else { 1.0 }
    };
    let (net_down, net_up) = (burst(180_000.0 + 2_500_000.0 * busy as f64), burst(40_000.0));
    let (disk_read, disk_write) = (burst(1_200_000.0 + 40_000_000.0 * busy as f64), burst(400_000.0));
    let (ghz, cpu_power) = (4.3 + 1.0 * busy + 0.1 * noise(), 22.0 + 150.0 * busy + 6.0 * noise());
    // Held near a 165 Hz screen's rate, with a stutter now and then.
    let game = game.map(|name| {
        let stutter = noise() > 0.9;
        GameSample {
            name: name.to_string(),
            program: name.to_string(),
            pid: 1,
            fps: 138.0 + 14.0 * wave(11.0, 0.4) + 4.0 * noise() - if stutter { 9.0 } else { 0.0 },
            low: Some(96.0 + 6.0 * noise()),
            longest_ms: if stutter { 18.0 + 14.0 * noise() } else { 7.6 + 1.6 * noise() },
            refresh_hz: Some(165),
            screen: None,
            gpu_index: Some(0),
            cpu: Some(14.0 + 6.0 * noise()),
            gpu: Some(93.0 + 5.0 * noise()),
            mem: Some((6.2 * gb as f32) as u64),
            vram: Some((7.9 * gb as f32) as u64),
            gpu_limit: Some(GpuLimit::Power),
            playing_s: 47 * 60 + seed,
            by_hand: false,
        }
    });
    Sample {
        t: t_ms as u64,
        cpu: Some(cpu),
        threads,
        ghz: Some(ghz),
        kinds_ghz: Vec::new(),
        threads_ghz: Vec::new(),
        memory: MemorySample { used, committed: used + 6 * gb, commit_limit: info.mem_total + 8 * gb, cached: 12 * gb },
        gpus,
        net_down: Some(net_down),
        net_up: Some(net_up),
        net_total_down: 38 * gb + seed * 900_000,
        net_total_up: 4 * gb + seed * 120_000,
        network: Some(NetworkInfo {
            name: "WLAN".into(),
            model: info.network_adapter.clone().unwrap_or_default(),
            ipv4: Some("192.168.1.23".into()),
            link_bps: 2_401_000_000,
        }),
        disk_read: Some(disk_read),
        disk_write: Some(disk_write),
        disk_active: Some(2.0 + 30.0 * busy),
        volumes: vec![
            VolumeSample { name: "C:".into(), used: 205 * gb, total: 600 * gb },
            VolumeSample { name: "D:".into(), used: 268 * gb, total: 1262 * gb },
        ],
        processes,
        system: SystemSample { uptime_s: 3 * 3600 + 25 * 60 + seed, processes: 312, threads: 4810, handles: 168_000 },
        battery: None::<BatterySample>,
        cpu_sensors: Some(CpuSensors { temp: Some(package), ccds: vec![(0, package + 2.0), (1, package - 14.0)], power: Some(cpu_power) }),
        board: Some(BoardSensors {
            temps: vec![
                ("system".into(), 33.0 + 3.0 * busy),
                ("chipset".into(), 45.0 + 4.0 * busy),
                ("cpu_socket".into(), 52.0 + 18.0 * busy),
                ("pcie_x16".into(), 36.0 + 8.0 * busy),
                ("vrm".into(), 40.0 + 14.0 * busy),
                ("vsoc".into(), 39.0 + 6.0 * busy),
            ],
            fans: vec![("cpu_fan".into(), 950.0 + 900.0 * busy), ("system_fan_2".into(), 880.0 + 500.0 * busy)],
        }),
        drive_temps: info.drives.first().map(|name| DriveTemperature { id: 0, name: name.clone(), celsius: 34.0 + 9.0 * busy }).into_iter().collect(),
        dimm_temps: vec![35.0 + 7.0 * busy, 34.0 + 7.0 * busy],
        mic_muted: Some(false),
        game,
        // A machine working: WSL with a distribution up, and a few
        // containers in it.
        wsl: Some(crate::reading::WslSample::Running {
            distros: Some(vec!["Ubuntu-24.04".into()]),
            cpu: Some(4.0 + 18.0 * busy),
            used: Some(6 * gb + (busy * 2.0 * gb as f32) as u64),
            total: Some(31 * gb),
            gpu: Some(12.0 * busy),
        }),
        docker: Some(crate::reading::DockerSample::Running {
            context: "desktop-linux".into(),
            containers: [("postgres", 3.1, 640u64, 2048u64, 120e3, 2.4e6), ("web", 1.4, 256, 512, 860e3, 0.0), ("redis", 0.3, 48, 0, 15e3, 4e3)]
                .into_iter()
                .map(|(name, cpu, mem, limit, net, io)| crate::reading::ContainerSample {
                    name: name.into(),
                    status: "Up 3 hours".into(),
                    cpu: Some(cpu * (0.6 + busy)),
                    mem: Some(mem << 20),
                    limit: (limit > 0).then_some(limit << 20),
                    net: Some(net * (0.5 + busy) as f64),
                    io: Some(io * busy as f64),
                })
                .collect(),
        }),
    }
}

/// Writes premultiplied BGRA `pixels` (opaque: drawn over the picture) as a PNG.
fn save_png(path: &PathBuf, width: u32, height: u32, pixels: &[u8]) -> windows::core::Result<()> {
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = factory.CreateStream()?;
        stream.InitializeFromFilename(&HSTRING::from(path.as_os_str()), GENERIC_WRITE.0)?;
        let encoder = factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        let mut options = None;
        encoder.CreateNewFrame(&mut frame, &mut options)?;
        let frame = frame.unwrap();
        frame.Initialize(options.as_ref())?;
        frame.SetSize(width, height)?;
        let mut format = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut format)?;
        frame.WritePixels(height, width * 4, pixels)?;
        frame.Commit()?;
        encoder.Commit()?;
    }
    Ok(())
}
