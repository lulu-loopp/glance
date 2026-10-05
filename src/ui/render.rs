//! The panel as a picture: laid out for a screen, and drawn wherever and at
//! whatever size it is wanted, on screen or in the settings' preview.

use std::hash::{DefaultHasher, Hash, Hasher};

use windows::core::Result;
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap1;
use windows_numerics::{Matrix3x2, Vector2};

use super::gfx::{Frame, Layer};
use super::skins;
use super::theme::{Skin, Theme};
use super::view::{self, HitBox, Lane, Layout, Pass, Scene, COLUMN_WIDTH};
use crate::settings::Edge;

/// Closest the panel gets to the ends of the work area (DIPs).
pub const GAP: f32 = 12.0;
/// Columns a panel along a side may spread over before it zooms out instead.
const MAX_COLUMNS: usize = 3;
/// Share of the work area's height a panel opened from the top may take.
const TOP_SHARE: f32 = 0.6;

/// Lays the lanes out for a work area `work` DIPs large, and the zoom that
/// fits the panel into it: as many columns as the lanes need to show at full
/// size, up to what the screen allows, and zoomed out only if even that is
/// too little.
pub fn arrange(theme: &Theme, edge: Edge, heights: Vec<f32>, work: (f32, f32)) -> (Layout, f32) {
    // The room the panel has: across from its edge it keeps a gap from the
    // far side; along its edge, from both ends.
    let (room_width, room) = match edge {
        Edge::Left | Edge::Right => (work.0 - GAP - theme.inset, work.1 - 2.0 * GAP),
        // Along the top the panel grows sideways instead, and no lower than a
        // share of the screen.
        Edge::Top => (work.0 - 2.0 * GAP, work.1 * TOP_SHARE - GAP - theme.inset),
    };
    // As many columns as fit across, and along a side no more than a few.
    let fit = (((room_width + theme.column_gap) / (COLUMN_WIDTH + theme.column_gap)) as usize).max(1);
    let max_columns = if edge == Edge::Top { fit } else { fit.min(MAX_COLUMNS) };
    let layout = Layout::new(heights, room, max_columns, theme);
    // Zoomed out only if even the most columns are too tall, or one is too wide.
    let zoom = (room / layout.height()).min(room_width / layout.width()).min(1.0);
    (layout, zoom)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::Skin;

    #[test]
    fn fits_narrow_and_short_screens() {
        let theme = Theme::new(Skin::Paper, false);
        // A portrait screen 600 DIPs wide holds one column, not three.
        let (layout, zoom) = arrange(&theme, Edge::Right, vec![400.0; 6], (600.0, 1000.0));
        assert_eq!(layout.columns, 1);
        assert!(zoom < 1.0 && layout.height() * zoom <= 1000.0 - 2.0 * GAP + 0.01);
        // Narrower than a column: the column is zoomed to fit across.
        let (layout, zoom) = arrange(&theme, Edge::Top, vec![100.0], (300.0, 1000.0));
        assert!(layout.width() * zoom <= 300.0 - 2.0 * GAP + 0.01);
        assert!(zoom.is_finite() && zoom > 0.0);
    }
}

/// What one drawing of the panel shows.
pub struct Picture<'a> {
    pub scene: &'a Scene<'a>,
    pub lanes: &'a [Lane],
    pub layout: &'a Layout,
    pub edge: Edge,
    /// The desktop behind, with the panel's corner at the given point in it,
    /// and what tells one desktop from another.
    pub backdrop: Option<(&'a ID2D1Bitmap1, Vector2, u64)>,
    pub frost: f32,
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
    /// Draws the panel with its corner placed by `local` within the frame's
    /// current transform, at `scale` physical pixels per panel DIP.
    pub fn draw(&mut self, frame: &Frame, picture: &Picture, local: Matrix3x2, scale: f32) -> Result<Drawn> {
        let (scene, layout) = (picture.scene, picture.layout);
        let theme = scene.theme;
        let boxes = layout.lanes();
        let bar = layout.bar();
        let (width, height) = (layout.width(), layout.height());

        // What each layer shows: when none of it changed, the one drawn for
        // an earlier frame serves again.
        let mut key = DefaultHasher::new();
        (theme.skin, theme.dark, picture.edge, scale.to_bits(), picture.frost.to_bits()).hash(&mut key);
        boxes.iter().chain([&bar]).for_each(|r| [r.x, r.y, r.w, r.h].map(f32::to_bits).hash(&mut key));
        picture.backdrop.map(|(_, at, digest)| (digest, at.X.to_bits(), at.Y.to_bits())).hash(&mut key);
        let ground_key = key.finish();
        (picture.lanes, &layout.cuts, layout.columns, scene.lang, scene.process_scroll.to_bits(), scene.hover).hash(&mut key);
        let content_key = key.finish();

        let mut grounded = false;
        let margin = theme.margin;
        let below = skins::Ground {
            theme,
            edge: picture.edge,
            size: (width, height),
            lanes: &boxes,
            bar,
            backdrop: picture.backdrop.map(|(bitmap, at, _)| (bitmap, at)),
            frost: picture.frost,
        };
        self.ground.draw(frame, ground_key, local, (-margin, -margin, width + 2.0 * margin, height + 2.0 * margin), scale, None, |frame| {
            skins::draw(frame, &below).expect("panel surface");
            grounded = true;
        })?;
        let mut hits = None;
        let halo = (theme.skin == Skin::Glass).then_some(theme.legibility);
        self.content.draw(frame, content_key, local, (0.0, 0.0, width, height), scale, halo, |frame| {
            hits = Some(view::paint(frame, scene, picture.lanes, layout, Pass::Content));
        })?;
        // The charts, drawn afresh every frame.
        let mut parent = Matrix3x2::default();
        unsafe {
            frame.dc.GetTransform(&mut parent);
            frame.dc.SetTransform(&(local * parent));
        }
        view::paint(frame, scene, picture.lanes, layout, Pass::Plots);
        unsafe { frame.dc.SetTransform(&parent) };
        Ok(Drawn { hits, grounded })
    }

    pub fn release(&mut self) {
        self.ground.release();
        self.content.release();
    }
}
