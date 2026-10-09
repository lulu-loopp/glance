//! The panel as a picture: laid out for a screen, and drawn wherever and at
//! whatever size it is wanted, on screen or in the settings' preview.

use std::hash::{DefaultHasher, Hash, Hasher};

use windows::core::Result;
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap1;
use windows_numerics::{Matrix3x2, Vector2};

use super::canvas::Canvas;
use super::forms::{self, Small};
use super::morph::{Part, Recorder};
use super::gfx::{Frame, Layer};
use super::skins;
use super::theme::Skin;
use super::view::{self, HitBox, Lane, Layout, Pass, Rect, Scene};
use crate::settings::Edge;

/// What one drawing of the panel shows.
pub struct Picture<'a> {
    pub scene: &'a Scene<'a>,
    pub lanes: &'a [Lane],
    pub layout: &'a Layout,
    /// The small layout it shows instead of its lanes, if it does, and how
    /// large that is drawn (DIPs).
    pub small: Option<(Small, (f32, f32))>,
    /// The size of its surface (DIPs): as large as what it shows, or (while
    /// it is sized, and settling after) the room it is given, what it shows
    /// at `at` in it.
    pub size: (f32, f32),
    pub at: (f32, f32),
    /// The screen edge it is against; none for a panel moved away from it.
    pub edge: Option<Edge>,
    /// The desktop behind, with the panel's corner at the given point in it,
    /// what tells one desktop from another, and how many of the desktop's
    /// pixels a panel DIP covers (whatever the scale it is drawn at).
    pub backdrop: Option<(&'a ID2D1Bitmap1, Vector2, u64, f32)>,
    pub frost: f32,
    /// Moving (in hand, flung): what the skin puts behind it drawn at half
    /// the resolution, a quarter of the work; blurred, it looks the same.
    pub rough: bool,
}

/// What a drawing of the panel did: where clicks land, if the readings were
/// drawn again, and whether the skin's surface was.
pub struct Drawn {
    pub hits: Option<Vec<HitBox>>,
    pub grounded: bool,
}

/// The panel's two cached layers: what the skin puts behind the readings,
/// and the readings but for the charts, which alone move between samples.
#[derive(Default)]
pub struct PanelLayers {
    ground: Layer,
    content: Layer,
}

impl PanelLayers {
    /// Draws what the skin puts behind the panel's readings (see `draw`);
    /// whether it was drawn afresh. What hashes it is left in `key`.
    fn ground(&mut self, frame: &Frame, picture: &Picture, local: Matrix3x2, scale: f32, key: &mut DefaultHasher) -> Result<bool> {
        let theme = picture.scene.theme;
        let (width, height) = picture.size;
        let scale = if picture.rough { scale / 2.0 } else { scale };
        let (ax, ay) = picture.at;
        let shifted = |r: Rect| Rect { x: r.x + ax, y: r.y + ay, ..r };
        // A small layout is one piece; lanes, a piece each and the bar.
        let (boxes, bar) = match picture.small {
            Some(_) => (vec![Rect { x: 0.0, y: 0.0, w: width, h: height }], Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }),
            None => (picture.layout.lanes().into_iter().map(shifted).collect(), shifted(picture.layout.bar())),
        };
        // When none of it changed, the one drawn for an earlier frame serves
        // again.
        (theme.skin, theme.dark, picture.edge, scale.to_bits(), picture.frost.to_bits(), width.to_bits(), height.to_bits()).hash(key);
        boxes.iter().chain([&bar]).for_each(|r| [r.x, r.y, r.w, r.h].map(f32::to_bits).hash(key));
        picture.backdrop.map(|(_, at, digest, covers)| (digest, at.X.to_bits(), at.Y.to_bits(), covers.to_bits())).hash(key);
        let mut grounded = false;
        let margin = theme.margin;
        let below = skins::Ground {
            theme,
            edge: picture.edge,
            size: (width, height),
            lanes: &boxes,
            bar,
            backdrop: picture.backdrop.map(|(bitmap, at, _, covers)| (bitmap, at, covers)),
            frost: picture.frost,
        };
        self.ground.draw(frame, key.finish(), local, (-margin, -margin, width + 2.0 * margin, height + 2.0 * margin), scale, None, |frame| {
            grounded = true;
            skins::draw(frame, &below)
        })?;
        Ok(grounded)
    }

    /// Draws only what the skin puts behind the readings, for readings
    /// drawn otherwise (as one layout turns into another, see `morph`).
    pub fn draw_ground(&mut self, frame: &Frame, picture: &Picture, local: Matrix3x2, scale: f32) -> Result<bool> {
        self.ground(frame, picture, local, scale, &mut DefaultHasher::new())
    }

    /// Draws the panel with its corner placed by `local` within the frame's
    /// current transform, at `scale` physical pixels per panel DIP.
    pub fn draw(&mut self, frame: &Frame, picture: &Picture, local: Matrix3x2, scale: f32) -> Result<Drawn> {
        let (scene, layout) = (picture.scene, picture.layout);
        let theme = scene.theme;
        let (ax, ay) = picture.at;
        // What each layer shows: when none of it changed, the one drawn for
        // an earlier frame serves again.
        let grounded = self.ground(frame, picture, local, scale, &mut DefaultHasher::new())?;
        // The readings' own: not the desktop behind them, which moves under
        // a widget moved over it while the readings stay as they were.
        let mut key = DefaultHasher::new();
        (theme.skin, theme.dark, scale.to_bits()).hash(&mut key);
        layout.lanes().iter().chain([&layout.bar()]).for_each(|r| [r.x, r.y, r.w, r.h].map(f32::to_bits).hash(&mut key));
        (picture.lanes, &layout.cuts, layout.columns, scene.lang, scene.process_scroll.to_bits(), scene.hover, scene.pinned).hash(&mut key);
        scene.overlay.hash(&mut key);
        // Drawn afresh as a sample comes (a small layout's charts are part
        // of it, and the bar's uptime is), and as what is chosen to show
        // changes.
        (scene.history.len(), scene.history.last().map(|sample| sample.t)).hash(&mut key);
        scene.prefs.hash_drawn(&mut key);
        if let Some((small, drawn)) = picture.small {
            (small, drawn.0.to_bits(), drawn.1.to_bits()).hash(&mut key);
        }
        let content_key = key.finish();
        let mut hits = None;
        let halo = (theme.skin == Skin::Glass).then_some(theme.legibility);
        let content = Matrix3x2::translation(ax, ay) * local;
        let drawn = picture.small.map_or((layout.width(), layout.height()), |(_, drawn)| drawn);
        self.content.draw(frame, content_key, content, (0.0, 0.0, drawn.0, drawn.1), scale, halo, |frame| {
            match picture.small {
                Some((small, drawn)) => {
                    forms::paint(frame, scene, small, drawn);
                    hits = Some(Vec::new());
                }
                None => hits = Some(view::paint(frame, scene, picture.lanes, layout, Pass::Content)),
            }
            Ok(())
        })?;
        // The lanes' charts, drawn afresh every frame.
        if picture.small.is_none() {
            let mut parent = Matrix3x2::default();
            unsafe {
                frame.dc.GetTransform(&mut parent);
                frame.dc.SetTransform(&(content * parent));
            }
            view::paint(frame, scene, picture.lanes, layout, Pass::Plots);
            unsafe { frame.dc.SetTransform(&parent) };
        }
        Ok(Drawn { hits, grounded })
    }

    pub fn release(&mut self) {
        self.ground.release();
        self.content.release();
    }
}

/// What a picture's readings draw, recorded as named parts (see `morph`):
/// its lanes laid out as `layout`, or `small` at its size; scaled by
/// `scale` and moved by `at`, measured on `frame`.
pub fn record(frame: &dyn Canvas, scene: &Scene, lanes: &[Lane], layout: &Layout, small: Option<(Small, (f32, f32))>, scale: f32, at: (f32, f32)) -> Vec<Part> {
    let recorder = Recorder::new(frame, scale, at);
    match small {
        Some((small, size)) => forms::paint(&recorder, scene, small, size),
        None => {
            view::paint(&recorder, scene, lanes, layout, Pass::Content);
            view::paint(&recorder, scene, lanes, layout, Pass::Plots);
        }
    }
    recorder.parts()
}
