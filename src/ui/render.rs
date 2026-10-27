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

/// The panel's cached layers: what the skin puts behind the readings, in
/// the parts it has (see `skins::Part`: only the desktop it shows is drawn
/// again as it moves over the desktop), and the readings but for the
/// charts, which alone move between samples.
#[derive(Default)]
pub struct PanelLayers {
    ground: GroundLayers,
    content: Layer,
}

/// What the skin puts behind something, in the parts it has (see `skins::Part`), each drawn again only as what it
/// shows changes: under the panel's readings, or under what plays drawn on the skin (one piece, as large as all of it).
#[derive(Default)]
pub struct GroundLayers([Layer; 3]);

impl GroundLayers {
    /// Draws `below` with its corner placed by `local`, at `scale` physical pixels a DIP (`rough`, moving: the desktop
    /// at half of that), `digest` telling the desktop behind from another; whether any of it was drawn afresh.
    fn draw(&mut self, frame: &Frame, below: &skins::Ground, digest: Option<u64>, local: Matrix3x2, scale: f32, rough: bool) -> Result<bool> {
        let theme = below.theme;
        let (width, height) = below.size;
        // What every part of it shows: its shape, as the theme has it.
        let mut shape = DefaultHasher::new();
        (theme.skin, theme.dark, below.edge, width.to_bits(), height.to_bits()).hash(&mut shape);
        below.lanes.iter().chain([&below.bar]).for_each(|r| [r.x, r.y, r.w, r.h].map(f32::to_bits).hash(&mut shape));
        let mut grounded = false;
        let margin = theme.margin;
        for (layer, part) in self.0.iter_mut().zip(skins::parts(theme.skin)) {
            // When none of what a part shows changed, the one drawn for
            // an earlier frame serves again: the desktop, and how frosted it
            // is, for the part that shows it; how tinted, for what is over
            // it; under it, nothing more.
            let mut key = shape.clone();
            let scale = match part {
                skins::Part::Desktop if rough => scale / 2.0,
                _ => scale,
            };
            (part, scale.to_bits()).hash(&mut key);
            match part {
                skins::Part::Under => {}
                skins::Part::Desktop => (below.frost.to_bits(), below.backdrop.zip(digest).map(|((_, at, covers), digest)| (digest, at.X.to_bits(), at.Y.to_bits(), covers.to_bits()))).hash(&mut key),
                skins::Part::Over => below.frost.to_bits().hash(&mut key),
            }
            layer.draw(frame, key.finish(), local, (-margin, -margin, width + 2.0 * margin, height + 2.0 * margin), scale, None, |frame| {
                grounded = true;
                skins::draw(frame, below, *part)
            })?;
        }
        Ok(grounded)
    }

    /// Draws what the skin puts behind something drawn whole on it (what plays, as the skin draws it): one piece, as
    /// large as `size`, away from any edge; the desktop behind as `Picture::backdrop` gives it.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_whole(&mut self, frame: &Frame, theme: &super::theme::Theme, size: (f32, f32), backdrop: Option<(&ID2D1Bitmap1, Vector2, u64, f32)>, frost: f32, rough: bool, local: Matrix3x2, scale: f32) -> Result<bool> {
        let whole = [Rect { x: 0.0, y: 0.0, w: size.0, h: size.1 }];
        let below = skins::Ground {
            theme,
            edge: None,
            size,
            lanes: &whole,
            bar: Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
            backdrop: backdrop.map(|(bitmap, at, _, covers)| (bitmap, at, covers)),
            frost,
        };
        self.draw(frame, &below, backdrop.map(|(_, _, digest, _)| digest), local, scale, rough)
    }

    pub fn release(&mut self) {
        self.0.iter_mut().for_each(Layer::release);
    }
}

impl PanelLayers {
    /// Draws what the skin puts behind the panel's readings (see `draw`);
    /// whether any of it was drawn afresh.
    fn ground(&mut self, frame: &Frame, picture: &Picture, local: Matrix3x2, scale: f32, others: &[Rect]) -> Result<bool> {
        let theme = picture.scene.theme;
        let (width, height) = picture.size;
        let (ax, ay) = picture.at;
        let shifted = |r: Rect| Rect { x: r.x + ax, y: r.y + ay, ..r };
        // A small layout is one piece; lanes, a piece each and the bar.
        let (boxes, bar) = match picture.small {
            Some(_) => (vec![Rect { x: 0.0, y: 0.0, w: width, h: height }], Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }),
            None => (picture.layout.lanes().into_iter().map(shifted).collect::<Vec<_>>(), shifted(picture.layout.bar())),
        };
        // (And the parts of what is merged with it, grounded as its lanes are.)
        let boxes: Vec<Rect> = boxes.into_iter().chain(others.iter().copied()).collect();
        let below = skins::Ground {
            theme,
            edge: picture.edge,
            size: (width, height),
            lanes: &boxes,
            bar,
            backdrop: picture.backdrop.map(|(bitmap, at, _, covers)| (bitmap, at, covers)),
            frost: picture.frost,
        };
        self.ground.draw(frame, &below, picture.backdrop.map(|(_, _, digest, _)| digest), local, scale, picture.rough)
    }

    /// Draws only what the skin puts behind the readings, for readings
    /// drawn otherwise (as one layout turns into another, see `morph`).
    pub fn draw_ground(&mut self, frame: &Frame, picture: &Picture, local: Matrix3x2, scale: f32) -> Result<bool> {
        self.ground(frame, picture, local, scale, &[])
    }

    /// Draws the panel with its corner placed by `local` within the frame's
    /// current transform, at `scale` physical pixels per panel DIP.
    pub fn draw(&mut self, frame: &Frame, picture: &Picture, local: Matrix3x2, scale: f32) -> Result<Drawn> {
        self.draw_on(frame, picture, local, scale, Some(&[]))
    }

    /// As `draw`, its skin's ground under its readings and under `others` too (the parts of what is merged with it,
    /// from its corner); or (`None`) on no ground of its own (merged into what has its own).
    pub fn draw_on(&mut self, frame: &Frame, picture: &Picture, local: Matrix3x2, scale: f32, others: Option<&[Rect]>) -> Result<Drawn> {
        let (scene, layout) = (picture.scene, picture.layout);
        let theme = scene.theme;
        let (ax, ay) = picture.at;
        // What each layer shows: when none of it changed, the one drawn for
        // an earlier frame serves again.
        let grounded = match others {
            Some(others) => self.ground(frame, picture, local, scale, others)?,
            None => false,
        };
        // The readings' own: not the desktop behind them, which moves under
        // a widget moved over it while the readings stay as they were.
        let mut key = DefaultHasher::new();
        (theme.skin, theme.dark, scale.to_bits()).hash(&mut key);
        layout.lanes().iter().chain([&layout.bar()]).for_each(|r| [r.x, r.y, r.w, r.h].map(f32::to_bits).hash(&mut key));
        (picture.lanes, &layout.cuts, layout.columns, scene.lang, scene.scrolls.bits(), scene.hover, scene.pinned).hash(&mut key);
        // (The sliders' knobs as they ease: kept as they were, the knob grew only as a sample came, once a second.)
        scene.thumbs.hash(&mut key);
        scene.overlay.hash(&mut key);
        // Drawn afresh as a sample comes (a small layout's charts are part
        // of it, and the bar's uptime is), and as what is chosen to show
        // changes.
        (scene.history.len(), scene.history.last().map(|sample| sample.t)).hash(&mut key);
        scene.prefs.hash_drawn(&mut key);
        // (What plays and how loud change between samples: as they are pressed.)
        crate::playing::stamp().hash(&mut key);
        // (A thing on a station's lane pressed, carried off it, springing back as it is put back.)
        view::tiles_drawn().hash(&mut key);
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

/// What shows a station's lane, as a thing is carried off it (see `carry`): what it drew meanwhile, kept, and the thing.
#[derive(Default)]
pub struct Away(Option<(view::Hit, Vec<Part>)>);

impl Away {
    /// What it draws now recorded by `record` (as it turns, see `morph`): kept while a thing is carried off the lane
    /// in it; that thing put down and gone from it (taken off the station, as its settings may say), what it drew
    /// while it was carried, for it to turn from into what it draws now (the room left closing, the rest moving up).
    pub fn follow(&mut self, record: impl Fn() -> Vec<Part>) -> Option<Vec<Part>> {
        let drawn = |parts: &[Part], hit: view::Hit| {
            let key = format!("tile.{hit:?}");
            parts.iter().any(|part| part.key.as_deref() == Some(key.as_str()))
        };
        match view::away_tile() {
            Some(hit) => {
                let parts = record();
                self.0 = drawn(&parts, hit).then_some((hit, parts));
                None
            }
            None => {
                let (hit, parts) = self.0.take()?;
                (!drawn(&record(), hit)).then_some(parts)
            }
        }
    }
}

/// What a thing on a station's lane (pressed as `hit`) draws, at rest (not lit, nor pressed: as it is lifted off its
/// room, see `carry`), recorded as `record` does; none where it is not drawn.
#[allow(clippy::too_many_arguments)]
pub fn record_tile(frame: &dyn Canvas, scene: &Scene, lanes: &[Lane], layout: &Layout, small: Option<(Small, (f32, f32))>, scale: f32, at: (f32, f32), hit: view::Hit) -> Vec<Part> {
    let at_rest = Scene { hover: None, hint: None, thumbs: &view::RESTING, ..*scene };
    let key = format!("tile.{hit:?}");
    record(frame, &at_rest, lanes, layout, small, scale, at).into_iter().filter(|part| part.key.as_deref() == Some(key.as_str())).collect()
}

#[cfg(test)]
mod tests {
    use super::super::morph::{Op, Part};
    use super::super::view::{self, Hit};
    use super::Away;

    /// What shows a station's lane, a thing carried off it: put back, nothing turns; taken off the station, what was
    /// drawn as it was carried is turned from, once; none of it drawn there, nothing kept.
    #[test]
    fn what_shows_a_thing_carried_off_turns_as_it_is_taken_off() {
        let _holding = view::HOLDING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let part = |key: &str, x: f32| Part { key: Some(key.into()), ops: vec![Op::Fill { color: super::super::canvas::Color::hex(0, 1.0), x, y: 0.0, w: 10.0, h: 10.0 }] };
        let (one, two) = (Hit::Shelf(0x5EED_0011), Hit::Shelf(0x5EED_0012));
        let both = || vec![part(&format!("tile.{one:?}"), 0.0), part(&format!("tile.{two:?}"), 20.0)];
        let (mut away, mut elsewhere) = (Away::default(), Away::default());
        view::carry_tile(Some(one));
        assert_eq!(away.follow(both), None);
        assert_eq!(elsewhere.follow(|| vec![part("cpu", 0.0)]), None);
        view::carry_tile(None);
        assert_eq!(away.follow(both), None);
        assert_eq!(elsewhere.follow(|| vec![part("cpu", 0.0)]), None);
        view::carry_tile(Some(one));
        assert_eq!(away.follow(both), None);
        view::carry_tile(None);
        let after = || vec![part(&format!("tile.{two:?}"), 0.0)];
        assert_eq!(away.follow(after), Some(both()));
        assert_eq!(away.follow(after), None);
    }
}
