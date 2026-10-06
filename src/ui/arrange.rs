//! How the panel is laid out for a screen, on any platform: how many
//! columns, at what zoom, and the shape it keeps while it is up.

use std::collections::HashMap;

use super::theme::Theme;
use super::view::{Layout, COLUMN_WIDTH};
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
///
/// `columns`, when given, is kept instead (a panel already on screen keeps
/// its shape), as far as the screen allows it.
pub fn arrange(theme: &Theme, edge: Edge, heights: Vec<f32>, work: (f32, f32), columns: Option<usize>) -> (Layout, f32) {
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
    let layout = match columns {
        Some(columns) => Layout::with_columns(heights, columns.min(max_columns), theme),
        None => Layout::new(heights, room, max_columns, theme),
    };
    // Zoomed out only if even the most columns are too tall, or one is too wide.
    let zoom = (room / layout.height()).min(room_width / layout.width()).min(1.0);
    (layout, zoom)
}

/// A panel's shape while it is on screen: the columns it opened with, and
/// each lane, by its module, as tall as it has been since. Readings come and
/// go (a fan that stops, a sensor read now and then, a battery lane missed
/// once), and a panel laid out afresh with them would change its columns,
/// or its zoom, under the pointer. Made anew for each opening, and when the
/// settings change.
#[derive(Default)]
pub struct Shape {
    columns: Option<usize>,
    heights: HashMap<String, f32>,
    /// The smallest zoom so far: a panel that came to fit by zooming out
    /// does not grow again while up (wider than the desktop taken behind it).
    zoom: Option<f32>,
}

impl Shape {
    /// `arrange` for the lanes there are now (each by its module, with its
    /// height), keeping the columns, and no lane shorter than it has been.
    pub fn arrange(&mut self, theme: &Theme, edge: Edge, lanes: &[(&str, f32)], work: (f32, f32)) -> (Layout, f32) {
        let heights: Vec<f32> = lanes
            .iter()
            .map(|(id, height)| {
                let held = self.heights.entry(id.to_string()).or_insert(*height);
                *held = held.max(*height);
                *held
            })
            .collect();
        let (layout, zoom) = arrange(theme, edge, heights, work, self.columns);
        // The columns chosen first stand: fewer lanes for a moment (one
        // missing) lay out in fewer, and do not lower them for later.
        self.columns.get_or_insert(layout.columns);
        let zoom = self.zoom.map_or(zoom, |held| held.min(zoom));
        self.zoom = Some(zoom);
        (layout, zoom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::Skin;

    #[test]
    fn fits_narrow_and_short_screens() {
        let theme = Theme::new(Skin::Paper, false);
        // A portrait screen 600 DIPs wide holds one column, not three.
        let (layout, zoom) = arrange(&theme, Edge::Right, vec![400.0; 6], (600.0, 1000.0), None);
        assert_eq!(layout.columns, 1);
        assert!(zoom < 1.0 && layout.height() * zoom <= 1000.0 - 2.0 * GAP + 0.01);
        // Narrower than a column: the column is zoomed to fit across.
        let (layout, zoom) = arrange(&theme, Edge::Top, vec![100.0], (300.0, 1000.0), None);
        assert!(layout.width() * zoom <= 300.0 - 2.0 * GAP + 0.01);
        assert!(zoom.is_finite() && zoom > 0.0);
    }

    #[test]
    fn keeps_its_shape_while_readings_come_and_go() {
        let theme = Theme::new(Skin::Glass, false);
        let work = (1400.0, 900.0);
        // Four lanes as tall as two columns can hold: the next row tips the
        // layout into a third.
        let columns = |height: f32| arrange(&theme, Edge::Right, vec![height; 4], work, None).0.columns;
        let most = (100..900).map(|h| h as f32).take_while(|h| columns(*h) <= 2).last().unwrap();
        let lanes = [("cpu", most), ("gpu:0", most), ("memory", most), ("battery", most)];
        let mut shape = Shape::default();
        let (opened, _) = shape.arrange(&theme, Edge::Right, &lanes, work);
        assert_eq!(opened.columns, 2);
        // A row more in one lane would take another column if laid out
        // afresh; held, the panel keeps its columns.
        let taller = [("cpu", most), ("gpu:0", most), ("memory", most + 30.0), ("battery", most)];
        assert_ne!(arrange(&theme, Edge::Right, taller.iter().map(|lane| lane.1).collect(), work, None).0.columns, opened.columns);
        let (held, _) = shape.arrange(&theme, Edge::Right, &taller, work);
        assert_eq!(held.columns, opened.columns);
        // The row gone again: the lane keeps its height.
        let (after, _) = shape.arrange(&theme, Edge::Right, &lanes, work);
        assert_eq!(after.height(), held.height());
        // A lane missing for a moment (the battery not read once), then back:
        // the columns hold throughout, and the lanes keep their heights.
        let (missing, _) = shape.arrange(&theme, Edge::Right, &lanes[..3], work);
        assert_eq!(missing.columns, opened.columns);
        let (back, _) = shape.arrange(&theme, Edge::Right, &lanes, work);
        assert_eq!((back.columns, back.height()), (held.columns, held.height()));
        // Each lane's height is its own module's, wherever the lane stands.
        let reordered = [("memory", most), ("cpu", most), ("gpu:0", most), ("battery", most)];
        assert_eq!(shape.arrange(&theme, Edge::Right, &reordered, work).0.heights[0], most + 30.0);
        // Two lanes too tall to share a column, one of them missed for a
        // moment: back, they stand in their two columns again.
        let pair = [("cpu", 2.0 * most), ("battery", 2.0 * most)];
        let mut shape = Shape::default();
        assert_eq!(shape.arrange(&theme, Edge::Right, &pair, work).0.columns, 2);
        assert_eq!(shape.arrange(&theme, Edge::Right, &pair[..1], work).0.columns, 1);
        assert_eq!(shape.arrange(&theme, Edge::Right, &pair, work).0.columns, 2);
        // Zoomed out to fit, it stays so when a lane goes for a moment.
        let mut shape = Shape::default();
        let (_, fitted) = shape.arrange(&theme, Edge::Right, &[("cpu", 1500.0), ("memory", 1500.0)], work);
        assert!(fitted < 1.0);
        assert_eq!(shape.arrange(&theme, Edge::Right, &[("cpu", 1500.0)], work).1, fitted);
    }
}
