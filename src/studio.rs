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
    /// When the shots begin (ms since 1970: what their clocks show), if not the studio's own moment.
    #[serde(default)]
    start_ms: Option<f64>,
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
    /// The edge the panel opens at: "left" or "right" (if absent).
    #[serde(default)]
    edge: Option<String>,
    /// The workbench (its station, notes and quick launch holding made-up things, see `pretend_workbench`) rather
    /// than the readings: its lanes are `modules`' then.
    #[serde(default)]
    workbench: bool,
    /// The station's page shown: "shelf" (if absent), "copies" (Recently copied) or "recent" (Recent files).
    #[serde(default)]
    tab: Option<String>,
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
    /// The pointer on the row of the drive of this letter (see `view::Hit::Drive`).
    #[serde(default)]
    hover_drive: Option<char>,
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
    /// How clear the overlay's glass is (see `OverlaySettings::clear`).
    #[serde(default)]
    overlay_clear: f32,
    /// Whether the panel is in the shot (it is unless said).
    #[serde(default = "yes")]
    panel: bool,
    /// A widget torn off the panel, and where it is taken after.
    torn: Option<Torn>,
    /// Widgets on the desktop from the start.
    #[serde(default)]
    widgets: Vec<Placed>,
    /// A song playing, filmed as a music widget on the desktop or as the music full screen, instead of the panel.
    #[serde(default)]
    music: Option<MusicShot>,
}

/// A song as the music widget or the full screen shows it: its lines (as Glance's lyrics hold them, in a JSON file),
/// what is heard of it, its cover (a picture), what it is called and by whom, how long it is,
/// and how far into it the shot begins (seconds).
#[derive(Deserialize)]
struct MusicShot {
    lyrics: String,
    heard: String,
    cover: String,
    title: String,
    artist: String,
    length: f64,
    #[serde(default)]
    from: f64,
    /// The widget: where it is and how large (DIPs on the screen) and how opaque, key by key, eased between them;
    /// sliding in and away as the shot's `slide_in` and `slide_out` say, out of its `edge`, as a panel does.
    #[serde(default)]
    keys: Vec<MusicKey>,
    /// The full screen instead, over the whole frame, its lines in the serif face in the file named (else in YaHei,
    /// as till the face is had).
    #[serde(default)]
    full: bool,
    #[serde(default)]
    serif: Option<String>,
    /// What the widget shows, as the music's switches say (as they are at first if absent).
    #[serde(default)]
    shows: MusicSwitches,
}

/// The music's switches (see `ui::music::Shows`), each as it is at first unless said.
#[derive(Deserialize)]
#[serde(default)]
struct MusicSwitches {
    cover: bool,
    title: bool,
    translation: bool,
    spectrum: bool,
    controls: bool,
    clear: bool,
}

impl Default for MusicSwitches {
    fn default() -> Self {
        MusicSwitches { cover: true, title: true, translation: true, spectrum: true, controls: false, clear: false }
    }
}

#[derive(Deserialize, Clone, Copy)]
struct MusicKey {
    t: f32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    #[serde(default = "opaque")]
    opacity: f32,
}

fn opaque() -> f32 {
    1.0
}

/// What is heard of a song every `step` seconds, as `sound::Heard` says it.
#[derive(Deserialize)]
struct HeardSong {
    step: f64,
    level: Vec<f32>,
    bands: Vec<[f32; crate::sound::BANDS]>,
    beat: Vec<f32>,
}

impl HeardSong {
    /// What is heard at `at` seconds into the song (the last look before it: nothing before the first, the last after
    /// the end).
    fn at(&self, at: f64) -> crate::sound::Heard {
        if at < self.step {
            return crate::sound::Heard::default();
        }
        let index = ((at / self.step) as usize - 1).min(self.level.len() - 1);
        crate::sound::Heard { level: self.level[index], bands: self.bands[index], beat: self.beat[index] }
    }
}

/// The widget at `t` seconds, its keys eased between (held past the ends).
fn music_key(keys: &[MusicKey], t: f32) -> MusicKey {
    match keys.iter().position(|key| key.t > t) {
        Some(0) => keys[0],
        None => keys[keys.len() - 1],
        Some(next) => {
            let (a, b) = (keys[next - 1], keys[next]);
            let k = ((t - a.t) / (b.t - a.t)).clamp(0.0, 1.0);
            let k = k * k * (3.0 - 2.0 * k);
            let mix = |p: f32, q: f32| p + (q - p) * k;
            MusicKey { t, x: mix(a.x, b.x), y: mix(a.y, b.y), w: mix(a.w, b.w), h: mix(a.h, b.h), opacity: mix(a.opacity, b.opacity) }
        }
    }
}

/// How long the music is drawn before a shot's first frame is kept (seconds): its colours come to the cover's, its
/// spectrum and the full screen's smoke to the song, as they would have while it played.
const MUSIC_BEFORE: f32 = 2.0;

/// A song playing (see `MusicShot`), filmed at the script's moments: each frame drawn for its own moment, every motion
/// in it timed from that moment, not from the clock as it draws.
fn film_music(gfx: &Gfx, script: &Script, shot: &Shot, music: &MusicShot, desktop: &crate::ui::backdrop::Capture, folder: &Path) -> Result<(), String> {
    use crate::ui::music::{Ground, Music, Played, Shows as MusicShows};
    let read = |path: &str| std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"));
    let lines: std::sync::Arc<Vec<crate::lyrics::Line>> = std::sync::Arc::new(serde_json::from_str(&read(&music.lyrics)?).map_err(|e| format!("{}: {e}", music.lyrics))?);
    let heard: HeardSong = serde_json::from_str(&read(&music.heard)?).map_err(|e| format!("{}: {e}", music.heard))?;
    let cover = std::sync::Arc::new(crate::cover::of_picture(Path::new(&music.cover)).ok_or_else(|| format!("{}: not a picture", music.cover))?);
    let lang = if shot.lang == "en" { Lang::En } else { Lang::Zh };
    if let Some(serif) = &music.serif {
        crate::serif::have(std::fs::read(serif).map_err(|e| format!("{serif}: {e}"))?);
    }
    // (Its volume, as the sound's key shows it.)
    crate::playing::pretend(crate::playing::Now { timeline: None, players: 1, title: music.title.clone(), artist: music.artist.clone(), album: String::new(), app: lang.pick("音乐", "Music").into(), playing: true, volume: Some(60), muted: false, sung: None });
    let px = script.scale;
    let (sw, sh) = (script.width as f32 / px, script.height as f32 / px);
    let on = &music.shows;
    let shows = MusicShows { cover: on.cover, title: on.title, translation: on.translation, spectrum: on.spectrum, controls: on.controls, ground: if on.clear { crate::ui::prefs::MusicGround::Clear } else { crate::ui::prefs::MusicGround::Cover }, inset: None };
    // Sliding out of its edge as a panel does, where the shot says when (its look's entrance, out of the edge it is at).
    let look = Theme::new(Skin::named(&shot.skin), true);
    let (entrance, family) = (look.entrance, look.body.family);
    let outward = if shot.edge.as_deref() == Some("left") { -1.0 } else { 1.0 };
    let mut widget = Music::default();
    let mut full = crate::fullscreen::Shown::new(std::time::Instant::now());
    let mut smoke = if music.full { Some(crate::ui::smoke::Smoke::new(gfx).map_err(|e| e.to_string())?) } else { None };
    let began = std::time::Instant::now() + std::time::Duration::from_secs_f32(MUSIC_BEFORE);
    let frames = (shot.seconds * script.fps).round() as i64;
    let before = (MUSIC_BEFORE * script.fps).round() as i64;
    for index in -before..frames {
        let t = index as f32 / script.fps;
        let now = if t >= 0.0 { began + std::time::Duration::from_secs_f32(t) } else { began - std::time::Duration::from_secs_f32(-t) };
        let position = (music.from + t as f64).max(0.0);
        let sound = heard.at(music.from + t as f64);
        let played = || Played { lines: lines.clone(), position, length: music.length, title: music.title.clone(), artist: music.artist.clone(), cover: cover.clone(), sound };
        let pixels = if let Some(smoke) = smoke.as_mut() {
            full.follow(now, sound, Some(Some(&cover)), Some(lines.clone()), crate::lyrics::sung_at(&lines, position, crate::lyrics::EARLY));
            full.render_smoke(smoke, (script.width, script.height)).map_err(|e| e.to_string())?;
            let smoke = &*smoke;
            gfx.draw_offscreen((script.width, script.height), px, |frame| full.paint(frame, (sw, sh), Some(smoke), position, now))
        } else {
            let mut key = music_key(&music.keys, t);
            if shot.slide_in.is_some() {
                let (shift, opacity) = motion(shot, t);
                let travel = match entrance {
                    Entrance::Beyond(extra) => key.w + extra,
                    Entrance::Slide(distance) => distance,
                };
                key.x += outward * shift * travel;
                key.opacity *= opacity;
            }
            gfx.draw_offscreen((script.width, script.height), px, |frame| {
                if let Ok(bitmap) = desktop.bitmap(&frame.dc, px) {
                    unsafe { frame.dc.DrawBitmap(&bitmap, None, 1.0, D2D1_INTERPOLATION_MODE_LINEAR, None, None) };
                }
                if key.opacity <= 0.0 {
                    return;
                }
                let everything = D2D_RECT_F { left: -f32::MAX, top: -f32::MAX, right: f32::MAX, bottom: f32::MAX };
                let layer = D2D1_LAYER_PARAMETERS1 { contentBounds: everything, opacity: key.opacity, ..Default::default() };
                unsafe { frame.dc.PushLayer(&layer, None) };
                frame.place(Matrix3x2::translation(key.x, key.y));
                widget.paint_played(frame, (key.w, key.h), &shows, Ground::of(&shows, true), family, px, None, now, played());
                frame.origin(0.0, 0.0);
                unsafe { frame.dc.PopLayer() };
            })
        }
        .map_err(|e| e.to_string())?;
        gfx.sweep();
        if index >= 0 {
            save_png(&folder.join(format!("{:05}.png", index + 1)), script.width, script.height, &pixels).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
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
/// (its corner from where the panel's corner rests); or the lane `only`
/// torn off alone, as a pull on a workbench's lane tears it (its corner
/// from where that lane is).
#[derive(Deserialize)]
struct Torn {
    at: f32,
    #[serde(default)]
    only: Option<String>,
    #[serde(flatten)]
    track: Track,
}

/// A widget on the desktop from the start: its corner (DIPs on the screen),
/// the room it was sized to, and where it is taken after (from its corner).
#[derive(Deserialize)]
struct Placed {
    at: (f32, f32),
    room: (f32, f32),
    /// The lanes it shows, in order (the shot's if absent).
    #[serde(default)]
    modules: Option<Vec<String>>,
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
            // Stuck: as it is, its middle where it was along the edge.
            let zoom = self.last.as_ref().map_or(1.0, |shown| shown.zoom);
            let middle = match side {
                "left" | "right" => self.at.1 + self.surface.1 * zoom / 2.0,
                _ => self.at.0 + self.surface.0 * zoom / 2.0,
            };
            self.stuck = Some((side.to_string(), middle));
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
    /// large as it is, its middle where its middle is now.
    fn would_stick(&self, side: &str, work: (f32, f32)) -> (f32, f32, f32, f32) {
        let zoom = self.last.as_ref().map_or(1.0, |shown| shown.zoom);
        let size = (self.surface.0 * zoom, self.surface.1 * zoom);
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
/// How many of the files' pictures (see `station::thumbs`) have been made.
static PICTURES_MADE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The workbench as it is filmed: its station holding a picture, a few documents, a folder, words and a link (made in
/// `props`, named in `lang`); a few programs kept at hand (Windows' own); three notes.
pub fn pretend_workbench(lang: Lang, props: &Path) -> Result<(), String> {
    use crate::notes::{Colour, Note};
    use crate::station::shelf::{Item, What};
    std::fs::create_dir_all(props).map_err(|e| e.to_string())?;
    // (Whole paths, as the station keeps them: one that is not is taken for a Store app's name.)
    let props = &std::path::absolute(props).map_err(|e| e.to_string())?;
    let zh = lang == Lang::Zh;
    let made = |name: &str, from: Option<&str>| -> Result<PathBuf, String> {
        let path = props.join(name);
        match from {
            Some(from) => std::fs::copy(from, &path).map(|_| ()),
            None if name.contains('.') => std::fs::write(&path, b"").map(|_| ()),
            None => std::fs::create_dir_all(&path),
        }
        .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(path)
    };
    let photo = made(if zh { "桌面壁纸.jpg" } else { "Wallpaper.jpg" }, Some(r"C:\Windows\Web\Screen\img100.jpg"))?;
    let sheet = made(if zh { "第三季度报表.xlsx" } else { "Q3 report.xlsx" }, None)?;
    let doc = made(if zh { "会议纪要.docx" } else { "Meeting notes.docx" }, None)?;
    let folder = made(if zh { "项目资料" } else { "Project files" }, None)?;
    let now = crate::station::now_ms();
    let item = |id: u64, what: What, ago_min: u64| Item { id, what, added_ms: now - ago_min * 60_000, pinned: id == 2, copied: false, stays_here: false, own_bytes: 0 };
    // What was copied lately, the oldest first as the station keeps it.
    let copied = |id: u64, what: What, ago_min: u64| Item { copied: true, pinned: false, ..item(id, what, ago_min) };
    let text = |text: &str| What::Text { text: text.into() };
    crate::station::run::pretend(vec![
        item(1, What::Files { paths: vec![photo.clone()], kept: Vec::new() }, 300),
        item(2, What::Text { text: if zh { "会议改到周四下午三点\n地点：二楼小会议室" } else { "Meeting moved to Thursday, 3 pm\nRoom 2B, second floor" }.into() }, 180),
        copied(6, text(if zh { "收货地址：上海市徐汇区漕溪北路 88 号 1203 室" } else { "Ship to: 88 Elm Street, Apt 1203, Springfield" }), 160),
        copied(7, What::Link { url: "https://github.com/lulu-loopp/glance".into() }, 95),
        item(3, What::Files { paths: vec![sheet.clone(), doc.clone()], kept: Vec::new() }, 45),
        copied(8, text(if zh { "验证码 482916，5 分钟内有效" } else { "Your code is 482916, valid for 5 minutes" }), 40),
        item(4, What::Link { url: "https://glancepc.com".into() }, 30),
        copied(9, What::Files { paths: vec![sheet.clone()], kept: Vec::new() }, 20),
        item(5, What::Files { paths: vec![folder], kept: Vec::new() }, 12),
        copied(10, text(if zh { "SF1428 6093 5521" } else { "1Z 999 AA1 0123 4567 84" }), 6),
        copied(11, text(if zh { "周四 15:00 二楼小会议室，记得带投影转接头" } else { "Thursday 3 pm, room 2B: bring the HDMI adapter" }), 2),
    ], crate::settings::ShelfView::Cards);
    // (What was copied, words mostly, read best in a list.)
    crate::station::run::pretend_pages(crate::settings::ShelfView::List, crate::settings::ShelfView::Cards);
    // The files come lately, newest first: downloads, screenshots, photos come together, documents saved.
    let own = |path: &[&str], from: Option<&str>| -> Result<PathBuf, String> {
        let file = path.iter().fold(props.join(if zh { "此电脑" } else { "This PC" }), |path, name| path.join(name));
        std::fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
        match from {
            Some(from) => std::fs::copy(from, &file).map(|_| ()),
            None => std::fs::write(&file, b""),
        }
        .map_err(|e| format!("{}: {e}", file.display()))?;
        Ok(file)
    };
    use crate::station::recent::{Came, Key, Kind};
    let came = |file: u128, path: PathBuf, ago_min: u64, bytes: u64, kind: Kind, site: Option<&str>| Came {
        key: Key { drive: 'D', file },
        path,
        came_ms: now - ago_min * 60_000,
        bytes,
        kind,
        downloaded: site.is_some(),
        site: site.map(String::from),
    };
    let (downloads, pictures, shots, documents) = if zh { ("下载", "图片", "屏幕截图", "文档") } else { ("Downloads", "Pictures", "Screenshots", "Documents") };
    let screen = r"C:\Windows\Web\Screen";
    crate::station::recent::pretend(vec![
        came(1, own(&[downloads, if zh { "十月发票.pdf" } else { "Invoice October 2026.pdf" }], None)?, 4, 186_000, Kind::Document, Some("mail.qq.com")),
        came(2, own(&[pictures, shots, if zh { "屏幕截图 2026-10-22 143015.png" } else { "Screenshot 2026-10-22 143015.png" }], Some(&format!(r"{screen}\img104.jpg")))?, 9, 1_420_000, Kind::Screenshot, None),
        came(3, own(&[documents, if zh { "工作" } else { "Work" }, &sheet.file_name().unwrap().to_string_lossy()], None)?, 26, 48_000, Kind::Document, None),
        came(4, own(&[pictures, if zh { "手机" } else { "Phone" }, "IMG_2041.jpg"], Some(&format!(r"{screen}\img101.jpg")))?, 70, 3_900_000, Kind::Picture, None),
        came(5, own(&[pictures, if zh { "手机" } else { "Phone" }, "IMG_2042.jpg"], Some(&format!(r"{screen}\img102.jpg")))?, 70, 4_100_000, Kind::Picture, None),
        came(6, own(&[pictures, if zh { "手机" } else { "Phone" }, "IMG_2043.jpg"], Some(&format!(r"{screen}\img103.jpg")))?, 70, 3_700_000, Kind::Picture, None),
        came(7, own(&[downloads, "GlanceSetup-0.4.2.exe"], None)?, 150, 9_800_000, Kind::Program, Some("glancepc.com")),
        came(8, own(&[downloads, if zh { "合同扫描件.zip" } else { "Contract scans.zip" }], None)?, 60 * 26, 12_400_000, Kind::Archive, Some("pan.baidu.com")),
    ]);
    let app = |target: &str, name: &str| crate::station::apps::App { target: target.into(), name: name.into() };
    crate::station::apps::pretend(vec![
        app(r"C:\Windows\explorer.exe", if zh { "文件资源管理器" } else { "File Explorer" }),
        app(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe", "Microsoft Edge"),
        app(r"shell:AppsFolder\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App", if zh { "计算器" } else { "Calculator" }),
        app(r"shell:AppsFolder\Microsoft.WindowsNotepad_8wekyb3d8bbwe!App", if zh { "记事本" } else { "Notepad" }),
        app(r"C:\Windows\System32\cmd.exe", if zh { "命令提示符" } else { "Command Prompt" }),
    ]);
    let note = |name: &str, text: &str, colour: Colour, ago_min: u64| Note { name: name.into(), text: text.into(), colour, stamp: crate::notes::store::Stamp { bytes: 0, written_ms: now - ago_min * 60_000, digest: 0 } };
    crate::notes::pretend(if zh {
        vec![
            note("a.md", "# 本周要做的事\n- [x] 交季度报表\n- [ ] 订周六的餐厅\n- [ ] 给妈妈打电话", Colour::Blue, 3),
            note("b.md", "显卡价格记录\n5070 Ti 京东 6299", Colour::Yellow, 60 * 20),
            note("c.md", "周末开车听的歌\n- 晴天\n- 后来", Colour::Green, 60 * 50),
        ]
    } else {
        vec![
            note("a.md", "# This week\n- [x] Send the Q3 report\n- [ ] Book Saturday's table\n- [ ] Call Mum", Colour::Blue, 3),
            note("b.md", "GPU prices\n5070 Ti 549 USD", Colour::Yellow, 60 * 20),
            note("c.md", "Songs for the drive\n- Here Comes the Sun", Colour::Green, 60 * 50),
        ]
    });
    Ok(())
}

pub fn run(script: &Path, out: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(script).map_err(|e| format!("{}: {e}", script.display()))?;
    let script: Script = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", script.display()))?;
    // This machine's own names, read as Glance reads them.
    let info = crate::metrics::Sampler::new().info;
    let gfx = Gfx::new().map_err(|e| e.to_string())?;
    let desktop = wallpaper::picture(&script.wallpaper, script.width, script.height).map_err(|e| format!("{}: {e}", script.wallpaper))?;
    // The files' pictures, made on a thread of their own (for the workbench), counted as they come.
    if script.shots.iter().any(|shot| shot.workbench) {
        crate::station::thumbs::start(|| {
            PICTURES_MADE.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        });
    }
    for shot in &script.shots {
        let folder = out.join(&shot.name);
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        film(&gfx, &script, shot, &info, &desktop, &folder)?;
        println!("{}: {} frames", shot.name, (shot.seconds * script.fps).round());
    }
    Ok(())
}

fn film(gfx: &Gfx, script: &Script, shot: &Shot, info: &StaticInfo, desktop: &crate::ui::backdrop::Capture, folder: &Path) -> Result<(), String> {
    if let Some(music) = &shot.music {
        return film_music(gfx, script, shot, music, desktop, folder);
    }
    let skin = Skin::named(&shot.skin);
    let lang = if shot.lang == "en" { Lang::En } else { Lang::Zh };
    let px = script.scale;
    let (sw, sh) = (script.width as f32 / px, script.height as f32 / px);
    let mut known = vec!["game".to_string(), "cpu".to_string()];
    known.extend(info.gpu_modules());
    known.extend(["memory", "network", "disk", "processes", "storage", "board", "battery", "system", "time", "wsl", "docker"].map(String::from));
    if shot.workbench {
        known.extend(["station", "notes", "apps"].map(String::from));
        pretend_workbench(lang, &folder.join("props"))?;
    }
    let tab = match shot.tab.as_deref() {
        Some("copies") => crate::ui::view::Tab::Copies(Default::default()),
        Some("recent") => crate::ui::view::Tab::Recent(Default::default()),
        _ => crate::ui::view::Tab::Shelf,
    };
    let mut prefs = Prefs::resolve(&serde_json::Value::Null, &known);
    if let Some(shown) = &shot.modules {
        prefs.modules.sort_by_key(|entry| shown.iter().position(|id| *id == entry.id).unwrap_or(usize::MAX));
        prefs.modules.iter_mut().for_each(|entry| entry.on = shown.contains(&entry.id));
    }

    // (The media module, where a shot shows it: a track made up.)
    crate::playing::pretend(crate::playing::Now { timeline: None, players: 1, title: "Clair de lune".into(), artist: "Claude Debussy".into(), album: "Suite bergamasque".into(), app: lang.pick("音乐", "Music").into(), playing: true, volume: Some(40), muted: false, sung: None });
    let frames = (shot.seconds * script.fps).round() as usize;
    let interval = 1000.0;
    let start_ms = script.start_ms.unwrap_or(1_760_000_000_000.0);
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
    let edge = if shot.edge.as_deref() == Some("left") { Edge::Left } else { Edge::Right };
    // (Out of the panel's edge, away from it, as it slides in.)
    let outward = if edge == Edge::Left { -1.0 } else { 1.0 };
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
        let x = if edge == Edge::Left { theme.inset * zoom } else { sw - theme.inset * zoom - pw };
        let rest = (x, ((sh - ph) / 2.0).max(GAP));
        (layout, zoom, rest, opening.seen.clone())
    };
    // Toned by the desktop where it first rests.
    let measure = Theme::new(skin, false);
    let first = &history[..=backlog as usize];
    let first_seen = Seen::of(first);
    let probe = Scene { music_apart: false, thumbs: &crate::ui::view::RESTING, info, prefs: &prefs, theme: &measure, lang, history: first, seen: &first_seen, pen_ms: 0.0, scrolls: crate::ui::view::Scrolls::default(), tab, hover: None, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
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
    // A widget torn off the workbench is drawn as the widget's own are (see `widget::workbench_dressed`).
    let widget_theme = if shot.workbench { crate::widget::workbench_dressed(Theme::new(skin, dark)) } else { Theme::new(skin, dark) };
    let frost = if skin == Skin::Glass { skins::frost(tone.0, tone.1, dark) } else { 0.0 };
    let ids: Vec<&str> = prefs.modules.iter().filter(|entry| entry.on).map(|entry| entry.id.as_str()).collect();
    let mut placed = serde_json::Value::Null;

    let mut layers = PanelLayers::default();
    // Each widget's choices: the shot's, or its own lanes alone (the one torn off, then those placed).
    let only = |shown: &[String]| {
        let mut own = prefs.clone();
        own.modules.sort_by_key(|entry| shown.iter().position(|id| *id == entry.id).unwrap_or(usize::MAX));
        own.modules.iter_mut().for_each(|entry| entry.on = shown.contains(&entry.id));
        own
    };
    let widget_prefs: Vec<Option<Prefs>> = std::iter::once(shot.torn.as_ref().and_then(|torn| torn.only.as_ref()).map(|id| only(std::slice::from_ref(id))))
        .chain(shot.widgets.iter().map(|w| w.modules.as_deref().map(only)))
        .collect();
    // The widgets: the one torn off (once it is), then those placed.
    let mut torn: Option<Filming> = None;
    let mut placed_widgets: Vec<Filming> = shot.widgets.iter().map(|w| Filming::new(Choice { form: Form::Lanes { detail: view::Detail::Full, columns: 1 }, scale: 1.0, extra: (0.0, 0.0) }, w.at)).collect();
    let work = (sw, sh - script.taskbar);
    let mut placements = Vec::new();
    // The files' pictures (a station's, quick launch's) come as they are made, on their own thread: the first frame
    // drawn again till none has come for a while (some are slow to make), before any is kept.
    let mut settling = shot.workbench;
    let mut still = 0;
    let mut made = PICTURES_MADE.load(std::sync::atomic::Ordering::Acquire);
    let mut index = 0;
    while index < frames {
        let t = index as f32 / script.fps;
        let now_ms = start_ms + t as f64 * 1000.0;
        let read = history.iter().take_while(|sample| sample.t as f64 <= now_ms).count().max(1);
        let seen = Seen::of(&history[..read]);
        let scene = Scene { music_apart: false, thumbs: &crate::ui::view::RESTING,
            info,
            prefs: &prefs,
            theme: &theme,
            lang,
            history: &history[..read],
            seen: &seen,
            pen_ms: now_ms - interval - PEN_LAG_MS,
            scrolls: crate::ui::view::Scrolls::default(),
            tab,
            hover: shot.hover_drive.map(view::Hit::Drive),
            pinned: false,
            overlay: false,
            buttons: true, newer: false, settings_lit: None,
        };
        let (layout, zoom, rest, held) = place(&scene, &theme);
        // What the panel holds as it has been up: drawn in the boxes it opened with.
        let scene = Scene { music_apart: false, thumbs: &crate::ui::view::RESTING, seen: &held, ..scene };
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
        let fresh = Scene { music_apart: false, thumbs: &crate::ui::view::RESTING, seen: &seen, buttons: false, theme: &widget_theme, ..scene };
        let mut showing: Vec<(usize, bool, Option<Shows>)> = Vec::new();
        let tracks: Vec<Following> = std::iter::once((shot.torn.as_ref().filter(|torn| t >= torn.at).map(|torn| &torn.track), None))
            .chain(shot.widgets.iter().map(|w| (Some(&w.track), Some(w.room))))
            .collect();
        if let Some(torn_off) = shot.torn.as_ref().filter(|torn_off| t >= torn_off.at && torn.is_none()) {
            // Whole, from where the panel rests; a lane alone, from where it is on it, in one column.
            let lane = torn_off.only.as_ref().map(|id| boxes[ids.iter().position(|shown| shown == id).expect("the lane torn off is shown")]);
            let (columns, from) = match lane {
                Some(lane) => (1, (rest.0 + lane.x * zoom, rest.1 + lane.y * zoom)),
                None => (layout.columns, rest),
            };
            torn = Some(Filming::new(Choice { form: Form::Lanes { detail: view::Detail::Full, columns }, scale: zoom, extra: (0.0, 0.0) }, from));
        }
        for (index, (track, first_room)) in tracks.iter().enumerate() {
            let Some(track) = track else { continue };
            let fresh = Scene { prefs: widget_prefs[index].as_ref().unwrap_or(&prefs), ..fresh };
            let filming = if index == 0 { torn.as_mut().unwrap() } else { &mut placed_widgets[index - 1] };
            if filming.last.is_none() {
                if let Some(room) = first_room {
                    filming.lay_out(t, &fresh, &widget_theme, &seen, work, Some(*room));
                    filming.sized = None;
                }
            }
            filming.follow(t, track, &fresh, &widget_theme, &seen, work);
            let still = keyed(&track.keys, t).0 == keyed(&track.keys, t + 1.0 / script.fps).0;
            let hint = track.hint.as_ref().filter(|hint| t >= hint.from && filming.stuck.is_none()).map(|hint| {
                let (x, y, w, h) = filming.would_stick(&hint.side, work);
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
                let local = Matrix3x2::scale(zoom, zoom) * Matrix3x2::translation(rest.0 + outward * shift * travel * zoom, rest.1);
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
                    // Tinted for the desktop around it, over the desktop behind it.
                    let whole = |dips: f32| (dips * px).round() as i32;
                    let hole = RECT { left: whole(INSET), top: whole(INSET), right: whole(INSET + w), bottom: whole(INSET + h) };
                    let ring = RECT { left: 0, top: 0, right: whole(INSET + w + MARGIN + 16.0), bottom: whole(INSET + h + MARGIN + 16.0) };
                    let presence = 1.0 - shot.overlay_clear.clamp(0.0, 1.0);
                    let glass = overlay::Glass { presence, ..overlay::glass(&mut desktop.luminances(ring, hole, 1), None).0 };
                    let at = Matrix3x2::translation(INSET, INSET);
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
                    let fresh = Scene { prefs: widget_prefs[*index].as_ref().unwrap_or(&prefs), ..fresh };
                    filming.draw(frame, &fresh, &widget_theme, desktop, frost, px, at, t, !still);
                    // Where it would stick, over it (as the widget's own
                    // window for it is).
                    if let Some((x, y, w, h, firm, strength)) = hint {
                        frame.place(Matrix3x2::translation(*x, *y));
                        crate::widget::paint_hint(frame, &widget_theme, (*w, *h), *firm, *strength, None);
                        frame.origin(0.0, 0.0);
                    }
                }
                // The first widget's menu, as it draws it: light or dark as
                // the theme is.
                if let (Some(menu), Some((index, ..))) = (shot.menu.as_ref().filter(|menu| (menu.from..menu.to).contains(&t)), showing.first()) {
                    let filming = if *index == 0 { torn.as_ref().unwrap() } else { &placed_widgets[index - 1] };
                    let items = crate::widget::menu_items(lang, false, false, true, false);
                    let sheet = crate::ui::menu::Sheet::new(gfx, &items, crate::ui::menu::Look::of(theme.skin, theme.dark));
                    let lit = menu.lit.iter().take_while(|(at, _)| *at <= t).last().map(|(_, choice)| *choice);
                    frame.place(Matrix3x2::translation(filming.at.0 + menu.at.0, filming.at.1 + menu.at.1));
                    sheet.paint(frame, &items, crate::ui::menu::Look::of(theme.skin, theme.dark), lit, false);
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
        if settling {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let now = PICTURES_MADE.load(std::sync::atomic::Ordering::Acquire);
            still = if now == made { still + 1 } else { 0 };
            made = now;
            settling = still < 3;
            if settling {
                continue;
            }
        }
        let panel_at = view::Rect { x: outward * shift * travel, y: 0.0, w: layout.width(), h: layout.height() };
        let where_is = |filming: &Filming| {
            let zoom = filming.last.as_ref().map_or(1.0, |shown| shown.zoom);
            // (Its bar too, where it has one: what it shows above it is cut out by it.)
            let bar = filming.last.as_ref().map(|shown| shown.layout.bar()).filter(|bar| bar.h > 0.0).map(|bar| {
                serde_json::json!({ "x": (filming.at.0 + bar.x * zoom) * px, "y": (filming.at.1 + bar.y * zoom) * px, "w": bar.w * zoom * px, "h": bar.h * zoom * px })
            });
            serde_json::json!({
                "x": filming.at.0 * px, "y": filming.at.1 * px, "w": filming.surface.0 * zoom * px, "h": filming.surface.1 * zoom * px,
                "form": filming.choice.form.key(), "bar": bar,
            })
        };
        let widget = showing.iter().find(|(index, ..)| *index == 0).and(torn.as_ref()).map(where_is);
        let others: Vec<_> = showing.iter().filter(|(index, ..)| *index > 0).map(|(index, ..)| where_is(&placed_widgets[index - 1])).collect();
        let hints: Vec<_> = showing.iter().filter_map(|(_, _, hint)| hint.map(|(x, y, w, h, firm, _)| serde_json::json!({ "x": x * px, "y": y * px, "w": w * px, "h": h * px, "firm": firm }))).collect();
        placements.push(serde_json::json!({ "t": t, "panel": rect(&panel_at), "widget": widget, "widgets": others, "hints": hints }));
        gfx.sweep();
        save_png(&folder.join(format!("{:05}.png", index + 1)), script.width, script.height, &pixels).map_err(|e| e.to_string())?;
        index += 1;
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
pub(crate) fn made_up(info: &StaticInfo, t_ms: f64, seed: u64, load: f32, game: Option<&str>) -> Sample {
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
                hot_spot: None,
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
            fps: Some(138.0 + 14.0 * wave(11.0, 0.4) + 4.0 * noise() - if stutter { 9.0 } else { 0.0 }),
            low: Some(96.0 + 6.0 * noise()),
            longest_ms: Some(if stutter { 18.0 + 14.0 * noise() } else { 7.6 + 1.6 * noise() }),
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
        ping_ms: Some(12.0 + (seed % 7) as f32),
        network: Some(NetworkInfo {
            name: "WLAN".into(),
            model: info.network_adapter.clone().unwrap_or_default(),
            ipv4: Some("192.168.1.23".into()),
            link_bps: 2_401_000_000,
            wifi: None,
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
        cpu_sensors: Some(CpuSensors { temp: Some(package), ccds: vec![(0, package + 2.0), (1, package - 14.0)], power: Some(cpu_power), graphics_power: None }),
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
pub(crate) fn save_png(path: &PathBuf, width: u32, height: u32, pixels: &[u8]) -> windows::core::Result<()> {
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

/// Times a widget's frames on a window of its own, over the picture at
/// `wallpaper` as the desktop: each form on glass, left alone, with a new
/// sample every frame, and moved; each frame's time part by part (the
/// surface given, the ground, the readings, the surface handed back), the
/// middle one of them and the slowest twentieth's. Once as the widget
/// draws, and once with what was drawn sent to the graphics card after each
/// part, so that each part's time is its own.
pub fn frames(wallpaper: &str) -> Result<(), String> {
    use std::time::{Duration, Instant};
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_POPUP, WS_VISIBLE};
    let (width, height, px) = (2560u32, 1440u32, 1.5f32);
    let info = crate::metrics::Sampler::new().info;
    let gfx = Gfx::new().map_err(|e| e.to_string())?;
    let desktop = wallpaper::picture(wallpaper, width, height).map_err(|e| format!("{wallpaper}: {e}"))?;
    let mut known = vec!["game".to_string(), "cpu".to_string()];
    known.extend(info.gpu_modules());
    known.extend(["memory", "network", "disk", "processes", "storage", "board", "battery", "system", "time", "wsl", "docker"].map(String::from));
    let prefs = Prefs::resolve(&serde_json::Value::Null, &known);
    let history: Vec<Sample> = (0..400).map(|i| made_up(&info, 1_760_000_000_000.0 + i as f64 * 1000.0, i as u64, 0.4, None)).collect();
    let theme = Theme::new(Skin::Glass, true);
    let work = (width as f32 / px, height as f32 / px);
    let window = unsafe { CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, w!("STATIC"), None, WS_POPUP | WS_VISIBLE, 200, 200, 1400, 1200, None, None, None, None) }.map_err(|e| e.to_string())?;
    // Cloaked: timed on whoever's desktop it runs on, it shows nothing there (what is drawn is drawn all the same).
    unsafe {
        let cloak: windows::core::BOOL = true.into();
        let _ = windows::Win32::Graphics::Dwm::DwmSetWindowAttribute(window, windows::Win32::Graphics::Dwm::DWMWA_CLOAK, &cloak as *const _ as *const _, 4);
    }
    let mut surface = crate::ui::gfx::Surface::new(&gfx, window).map_err(|e| e.to_string())?;
    let forms = [
        ("strip", Form::Small(Small::Strip { rich: true })),
        ("corner", Form::Small(Small::Corner)),
        ("tiles", Form::Small(Small::Tiles { across: 3, down: 2 })),
        ("compact-2", Form::Lanes { detail: view::Detail::Compact, columns: 2 }),
    ];
    println!("{:<10} {:<7} {:<6} {:>7} {:>7} {:>7} {:>7} {:>7}   (ms: middle / slowest twentieth)", "form", "frames", "sent", "given", "ground", "reads", "handed", "all");
    for (name, form) in forms {
        for sent in [false, true] {
            for case in ["still", "sample", "moved", "rough", "second"] {
                let mut filming = Filming::new(Choice { form, scale: 0.79, extra: (0.0, 0.0) }, (40.0, 40.0));
                let mut bitmap = None;
                let mut times: Vec<[Duration; 5]> = Vec::new();
                // (A sample a second, as a widget on the desktop has them: the frames a second apart.)
                let paced = matches!(case, "second" | "cleared" | "trimmed");
                let rounds = if paced { 14 } else { 150 };
                for round in 0..rounds {
                    if paced {
                        std::thread::sleep(Duration::from_secs(1));
                    }
                    // (As the panel does once its ground is drawn, and as it does shut.)
                    match case {
                        "cleared" => gfx.clear_caches(),
                        "trimmed" => gfx.trim(),
                        _ => {}
                    }
                    let read = if case == "sample" || paced { 200 + round } else { 200 };
                    let seen = Seen::of(&history[..read]);
                    let scene = Scene { music_apart: false, thumbs: &crate::ui::view::RESTING, info: &info, prefs: &prefs, theme: &theme, lang: Lang::Zh, history: &history[..read], seen: &seen, pen_ms: history[read - 1].t as f64 - 1000.0 - PEN_LAG_MS, scrolls: crate::ui::view::Scrolls::default(), tab: crate::ui::view::Tab::Shelf, hover: None, pinned: false, overlay: false, buttons: false, newer: false, settings_lit: None };
                    let shown = filming.lay_out(0.0, &scene, &theme, &seen, work, None);
                    let zoom = shown.zoom;
                    let at = if case == "moved" || case == "rough" { (40.0 + round as f32 * 2.0, 40.0 + round as f32) } else { (40.0, 40.0) };
                    let lanes = view::lanes_at(&scene, |_| shown.detail);
                    let (mut ground, mut reads) = (Duration::ZERO, Duration::ZERO);
                    let began = Instant::now();
                    let layers = &mut filming.layers;
                    let size = filming.surface;
                    let drawn = surface.draw(&gfx, (1400, 1200), px, |frame| {
                        if bitmap.is_none() {
                            bitmap = desktop.bitmap(&frame.dc, px * zoom).ok();
                        }
                        let backdrop = bitmap.as_ref().map(|bitmap| (bitmap, Vector2 { X: at.0 / zoom, Y: at.1 / zoom }, desktop.digest, px * zoom));
                        let picture = render::Picture { scene: &scene, lanes: &lanes, layout: &shown.layout, small: shown.small.map(|small| (small, shown.size)), size, at: (0.0, 0.0), edge: None, backdrop, frost: 0.5, rough: case == "rough" };
                        let local = Matrix3x2::scale(zoom, zoom) * Matrix3x2::translation(at.0, at.1);
                        let from = Instant::now();
                        let _ = layers.draw_ground(frame, &picture, local, px * zoom);
                        if sent {
                            let _ = unsafe { frame.dc.Flush(None, None) };
                        }
                        ground = from.elapsed();
                        let from = Instant::now();
                        let _ = layers.draw(frame, &picture, local, px * zoom);
                        if sent {
                            let _ = unsafe { frame.dc.Flush(None, None) };
                        }
                        reads = from.elapsed();
                    });
                    drawn.map_err(|e| e.to_string())?;
                    gfx.sweep();
                    // (The first frames make what the later ones keep.)
                    if round >= 10 || (paced && round >= 2) {
                        times.push([surface.took.given, ground, reads, surface.took.handed, began.elapsed()]);
                    }
                }
                let of = |part: usize| {
                    let mut all: Vec<f64> = times.iter().map(|time| time[part].as_secs_f64() * 1000.0).collect();
                    all.sort_by(f64::total_cmp);
                    format!("{:.1}/{:.1}", all[all.len() / 2], all[all.len() * 19 / 20])
                };
                println!("{:<10} {:<7} {:<6} {:>7} {:>7} {:>7} {:>7} {:>7}", name, case, if sent { "each" } else { "-" }, of(0), of(1), of(2), of(3), of(4));
            }
        }
    }
    unsafe {
        let _ = DestroyWindow(window);
    }
    Ok(())
}

/// The phone's page as the settings draw it, for a PC whose lights are on,
/// with the ring a colour is made on open: one picture of each stretch of
/// the page, in `out`.
pub fn phone_page(out: &Path) -> Result<(), String> {
    use crate::ui::phone_art::{self, Making, SCREEN};
    let info = crate::metrics::Sampler::new().info;
    let gfx = Gfx::new().map_err(|e| e.to_string())?;
    let sample = made_up(&info, 1_760_000_000_000.0, 7, 0.4, None);
    let reads = crate::remote::figures(&sample, &info);
    let state = serde_json::json!({
        "night": false, "screen": true, "lights": true, "lights_there": true, "colours_there": true,
        "light": { "color": "3CE0A0", "effect": "steady", "brightness": 80 },
        "power": [{ "id": "efficiency", "name": "", "on": false }, { "id": "balanced", "name": "", "on": true }, { "id": "performance", "name": "", "on": false }],
        "colours": ["FF7B4A", "FFFFFF", "70B3FF", "59C98A", "C68CFF", "FF5E7E", "112233", "AA22CC", "FFE14A"], "monitors": [], "volume": 40, "muted": false, "media": null, "keys": null,
    });
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let px = 2.0;
    let (w, h) = ((SCREEN.0 * px) as u32, (SCREEN.1 * px) as u32);
    let making = Making { was: "3CE0A0".into(), hue: 210.0, depth: 0.6 };
    // (And as a laptop is shown: its picture, its lights the ones under its keys.)
    let mut laptop = state.clone();
    laptop["laptop"] = true.into();
    laptop["colours_there"] = false.into();
    laptop["keys"] = serde_json::json!({ "level": 2, "most": 3 });
    // (In each language: the site's phone page shows the one it is in.)
    for (lang, suffix) in [(Lang::Zh, ""), (Lang::En, "-en")] {
        for (name, making, scroll) in [("shut", None, 0.0), ("ring", Some(&making), 330.0), ("ring-top", None, 330.0), ("laptop", None, 0.0)] {
            let state = if name == "laptop" { &laptop } else { &state };
            let pixels = gfx
                .draw_offscreen((w, h), px, |frame| {
                    phone_art::paint(frame, view::Rect { x: 0.0, y: 0.0, w: SCREEN.0, h: SCREEN.1 }, lang, "DESKTOP", &reads, Some(state), false, 3.0, scroll, None, making, name == "ring-top");
                })
                .map_err(|e| e.to_string())?;
            save_png(&out.join(format!("{name}{suffix}.png")), w, h, &pixels).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// The graphics card's memory this process uses now (its local segment, MB), as Task Manager's "dedicated GPU
/// memory" shows it.
fn card_memory() -> u64 {
    use windows::core::Interface;
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter3, IDXGIFactory1, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO};
    let adapter: Option<IDXGIAdapter3> = unsafe { CreateDXGIFactory1::<IDXGIFactory1>().ok().and_then(|f| f.EnumAdapters1(0).ok()).and_then(|a| a.cast().ok()) };
    let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
    adapter.and_then(|a| unsafe { a.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut info) }.ok()).map_or(0, |_| info.CurrentUsage >> 20)
}

/// A window (cloaked: nothing shows) sized frame by frame for six seconds as a hand sizes one, drawn `way`: as Glance
/// draws (`now`), on a surface made once at its largest (`fixed`), or as it is drawn while sized by hand
/// (`sizing`); its window not sized, only what is drawn (`unmoved`). The graphics card's memory, before, at most,
/// and after.
pub fn surface_memory(way: &str) -> Result<(), String> {
    use std::time::{Duration, Instant};
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, SetWindowPos, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_POPUP, WS_VISIBLE};
    let gfx = Gfx::new().map_err(|e| e.to_string())?;
    let window = unsafe { CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, w!("STATIC"), None, WS_POPUP | WS_VISIBLE, 100, 100, 800, 600, None, None, None, None) }.map_err(|e| e.to_string())?;
    unsafe {
        let cloak: windows::core::BOOL = true.into();
        let _ = windows::Win32::Graphics::Dwm::DwmSetWindowAttribute(window, windows::Win32::Graphics::Dwm::DWMWA_CLOAK, &cloak as *const _ as *const _, 4);
    }
    let mut surface = crate::ui::gfx::Surface::new(&gfx, window).map_err(|e| e.to_string())?;
    let px = 1.5;
    let largest = (2600u32, 1500u32);
    let sizing = crate::ui::gfx::sizing(way == "sizing");
    let draw = |surface: &mut crate::ui::gfx::Surface, size: (u32, u32)| {
        let drawn = if way == "fixed" { largest } else { size };
        surface.draw(&gfx, drawn, px, |frame| {
            let (w, h) = (size.0 as f32 / px, size.1 as f32 / px);
            let canvas: &dyn crate::ui::canvas::Canvas = frame;
            canvas.fill_rounded(crate::ui::canvas::Color::hex(0x2040A0, 0.9), 10.0, 10.0, w - 20.0, h - 20.0, 16.0);
            canvas.fill(crate::ui::canvas::Color::hex(0xFFFFFF, 0.5), 40.0, 40.0, w / 2.0, 30.0);
        })
    };
    if let Some(skin) = way.strip_prefix("panel-") {
        return panel_memory(&gfx, &mut surface, window, skin);
    }
    // PROBE (memory): a scene arranged anew at another size again and again (its frame pool made anew each time), a
    // frame of each read; then let go of. The process stays a while for its memory to be read from outside.
    if way == "scene" {
        println!("before: card MB {}", card_memory());
        std::thread::sleep(Duration::from_secs(4));
        // Made and let go of again and again at one size, as a panel's is as it opens and goes: what stays.
        if let Some(cycles) = std::env::var("GLANCE_CYCLES").ok().and_then(|s| s.parse::<usize>().ok()) {
            // Kept, resting between (as a panel's is while it is hidden), where GLANCE_REST is set.
            let rests = std::env::var_os("GLANCE_REST").is_some();
            let mut kept: Option<crate::ui::scene::Scene> = None;
            // At another size each time, where GLANCE_GROW is set (a pool made anew at each).
            let grows = std::env::var_os("GLANCE_GROW").is_some();
            for cycle in 0..cycles {
                let grow = if grows { (cycle % 10) as i32 * 37 } else { 0 };
                let area = RECT { left: 200, top: 200, right: 1000 + grow, bottom: 700 + grow };
                let wake: std::sync::Arc<dyn Fn() + Send + Sync> = std::sync::Arc::new(|| {});
                let mut scene = match kept.take() {
                    Some(scene) => scene,
                    None => crate::ui::scene::Scene::new(area, wake, &gfx).ok_or("no scene")?,
                };
                let arranging = Instant::now();
                scene.arrange(area, window);
                let arranged = arranging.elapsed();
                let began = Instant::now();
                scene.want();
                while began.elapsed() < Duration::from_millis(600) && scene.ready().is_none() {
                    std::thread::sleep(Duration::from_millis(10));
                }
                println!("  arranged in {:.1} ms, a frame read {:.1} ms after", arranged.as_secs_f64() * 1000.0, began.elapsed().as_secs_f64() * 1000.0);
                if rests {
                    scene.rest();
                    kept = Some(scene);
                } else {
                    drop(scene);
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            gfx.trim();
            println!("{cycles} scenes made and let go: card MB {}", card_memory());
            std::thread::sleep(Duration::from_secs(10));
            return Ok(());
        }
        let steps: usize = std::env::var("GLANCE_STEPS").ok().and_then(|s| s.parse().ok()).unwrap_or(40);
        let wake: std::sync::Arc<dyn Fn() + Send + Sync> = std::sync::Arc::new(|| {});
        let mut scene = crate::ui::scene::Scene::new(RECT { left: 200, top: 200, right: 1000, bottom: 700 }, wake, &gfx).ok_or("no scene")?;
        let mut taken = 0;
        for step in 0..steps {
            let grow = (step % 10) as i32 * 37;
            let area = RECT { left: 200, top: 200, right: 1000 + grow, bottom: 700 + grow };
            scene.arrange(area, window);
            let began = Instant::now();
            if std::env::var_os("GLANCE_NO_READ").is_some() {
                std::thread::sleep(Duration::from_millis(150));
                continue;
            }
            scene.want();
            while began.elapsed() < Duration::from_millis(600) {
                if let Some(capture) = scene.ready() {
                    taken += 1;
                    drop(capture);
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        println!("scene: {taken} of {steps} frames taken; card MB (as Glance's device counts it) {}", card_memory());
        drop(scene);
        std::thread::sleep(Duration::from_secs(2));
        println!("scene let go: card MB {}", card_memory());
        gfx.trim();
        std::thread::sleep(Duration::from_secs(2));
        println!("then trimmed: card MB {}", card_memory());
        std::thread::sleep(Duration::from_secs(15));
        return Ok(());
    }
    draw(&mut surface, (800, 600)).map_err(|e| e.to_string())?;
    std::thread::sleep(Duration::from_secs(2));
    let before = card_memory();
    let (began, mut most, mut frames) = (Instant::now(), before, 0);
    while began.elapsed() < Duration::from_secs(6) {
        let t = began.elapsed().as_secs_f32();
        let size = ((1400.0 + 1100.0 * (t * 1.3).sin()) as u32, (800.0 + 600.0 * (t * 0.9).cos()) as u32);
        if way != "unmoved" {
            unsafe {
                let _ = SetWindowPos(window, None, 0, 0, size.0 as i32, size.1 as i32, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
        draw(&mut surface, size).map_err(|e| e.to_string())?;
        frames += 1;
        most = most.max(card_memory());
        std::thread::sleep(Duration::from_millis(16));
    }
    drop(sizing);
    let mut after = Vec::new();
    for _ in 0..5 {
        std::thread::sleep(Duration::from_secs(1));
        after.push(card_memory());
    }
    println!("{way:<8} frames {frames:>4}  card MB: before {before}, at most {most}, then {after:?}");
    Ok(())
}

/// As `surface_memory`, with a panel of two lanes drawn on it as Glance draws one (its ground, its readings, over a
/// desktop) in `skin` ("glass", "fluent", "paper"; "-still" after it: not sized), sized by its zoom frame by frame.
fn panel_memory(gfx: &Gfx, surface: &mut crate::ui::gfx::Surface, window: windows::Win32::Foundation::HWND, skin: &str) -> Result<(), String> {
    use std::time::{Duration, Instant};
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER};
    let (still, skin_name) = match skin.strip_suffix("-still") {
        Some(name) => (true, name),
        None => (false, skin),
    };
    let skin = match skin_name {
        "glass" => Skin::Glass,
        "fluent" => Skin::Fluent,
        _ => Skin::Paper,
    };
    let px = 1.5f32;
    let (width, height) = (2560u32, 1440u32);
    let info = crate::metrics::Sampler::new().info;
    let mut known = vec!["game".to_string(), "cpu".to_string()];
    known.extend(info.gpu_modules());
    known.extend(["memory", "network", "disk", "processes", "storage", "board", "battery", "system", "time", "wsl", "docker"].map(String::from));
    let prefs = Prefs::resolve(&serde_json::Value::Null, &known);
    let history: Vec<Sample> = (0..400).map(|i| made_up(&info, 1_760_000_000_000.0 + i as f64 * 1000.0, i as u64, 0.4, None)).collect();
    let theme = Theme::new(skin, true);
    let work = (width as f32 / px, height as f32 / px);
    let desktop = {
        let pixels: Vec<u8> = (0..width * height).flat_map(|i| { let v = ((i % width) * 255 / width) as u8; [v, 128u8, 255 - v, 255] }).collect();
        crate::ui::backdrop::Capture::from_pixels(RECT { left: 0, top: 0, right: width as i32, bottom: height as i32 }, pixels)
    };
    let form = Form::Lanes { detail: view::Detail::Compact, columns: 2 };
    let mut filming = Filming::new(Choice { form, scale: 1.0, extra: (0.0, 0.0) }, (40.0, 40.0));
    let seen = Seen::of(&history);
    let scene = Scene { music_apart: false, thumbs: &crate::ui::view::RESTING, info: &info, prefs: &prefs, theme: &theme, lang: Lang::Zh, history: &history, seen: &seen, pen_ms: history[399].t as f64 - 1000.0 - PEN_LAG_MS, scrolls: crate::ui::view::Scrolls::default(), tab: crate::ui::view::Tab::Shelf, hover: None, pinned: false, overlay: false, buttons: false, newer: false, settings_lit: None };
    let mut bitmap = None;
    let mut frame_at = |zoom: f32, surface: &mut crate::ui::gfx::Surface, filming: &mut Filming| -> Result<(), String> {
        filming.choice.scale = zoom;
        let shown = filming.lay_out(0.0, &scene, &theme, &seen, work, None);
        let lanes = view::lanes_at(&scene, |_| shown.detail);
        let margin = theme.margin;
        let size = ((((shown.size.0 + 2.0 * margin) * px * shown.zoom).ceil()) as u32, (((shown.size.1 + 2.0 * margin) * px * shown.zoom).ceil()) as u32);
        unsafe {
            let _ = SetWindowPos(window, None, 0, 0, size.0 as i32, size.1 as i32, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
        let layers = &mut filming.layers;
        let drawn = surface.draw(gfx, size, px, |frame| {
            let scale = px * shown.zoom;
            // (As the panel does: the same pixels at each zoom.)
            bitmap = match bitmap.take() {
                Some((made, _)) => desktop.at_scale(&frame.dc, &made, scale).ok(),
                None => desktop.bitmap(&frame.dc, scale).ok(),
            }
            .map(|b| (b, scale));
            let backdrop = bitmap.as_ref().map(|(bitmap, scale)| (bitmap, Vector2 { X: 200.0, Y: 100.0 }, desktop.digest, *scale));
            let picture = render::Picture { scene: &scene, lanes: &lanes, layout: &shown.layout, small: None, size: shown.size, at: (0.0, 0.0), edge: None, backdrop, frost: 0.5, rough: false };
            let local = Matrix3x2::scale(shown.zoom, shown.zoom) * Matrix3x2::translation(margin * shown.zoom, margin * shown.zoom);
            let part = std::env::var("GLANCE_PART").unwrap_or_default();
            if part == "ground" {
                let _ = layers.draw_ground(frame, &picture, local, scale);
            } else {
                let _ = layers.draw(frame, &picture, local, scale);
            }
        });
        gfx.sweep();
        if std::env::var("GLANCE_PART").as_deref() == Ok("cleared") {
            gfx.clear_caches();
        }
        drawn.map_err(|e| e.to_string())
    };
    frame_at(1.0, surface, &mut filming)?;
    std::thread::sleep(Duration::from_secs(2));
    // (As a window sized by hand is drawn.)
    let sizing = crate::ui::gfx::sizing(!still && std::env::var_os("GLANCE_AT_REST").is_none());
    let before = card_memory();
    let (began, mut most, mut frames) = (Instant::now(), before, 0);
    let mut took = Vec::new();
    while began.elapsed() < Duration::from_secs(6) {
        let t = began.elapsed().as_secs_f32();
        let zoom = if still { 1.0 } else { 1.0 + 0.45 * (t * 1.3).sin() };
        let frame_began = Instant::now();
        frame_at(zoom, surface, &mut filming)?;
        took.push(frame_began.elapsed().as_secs_f64() * 1000.0);
        frames += 1;
        most = most.max(card_memory());
        std::thread::sleep(Duration::from_millis(16));
    }
    // Let go: drawn once more at rest, as it then is, and what was made as it was sized let go (as Glance does).
    drop(sizing);
    frame_at(1.0, surface, &mut filming)?;
    gfx.settle();
    let mut after = Vec::new();
    for _ in 0..5 {
        std::thread::sleep(Duration::from_secs(1));
        after.push(card_memory());
    }
    took.sort_by(f64::total_cmp);
    let at = |share: f64| took[((took.len() - 1) as f64 * share) as usize];
    println!("panel {skin_name:<6} still {still:<5} frames {frames:>4}  ms a frame: median {:.2}, p95 {:.2}, most {:.2}  card MB: before {before}, at most {most}, then {after:?}", at(0.5), at(0.95), at(1.0));
    Ok(())
}
