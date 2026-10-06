//! How the panel is laid out for a screen, on any platform: how many
//! columns, at what zoom, and the shape it keeps while it is up.

use super::seen::Seen;
use super::theme::Theme;
use super::view::{Layout, COLUMN_WIDTH};
use crate::reading::StaticInfo;
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

/// A panel while it is up: laid out once, as it opened, and held. Its
/// size, its columns, its zoom and each lane's box stay as they were; a
/// reading the machine shows for the first time joins its lane at once
/// only where the lane's box has room for it, and anything that would need
/// more waits for the next opening. A reading gone stays in its place,
/// unread. (Settings changed while it is up lay it out afresh, holding
/// what it held: see `Seen::join`.)
pub struct Opening {
    pub layout: Layout,
    pub zoom: f32,
    /// The lanes it opened with, by module.
    lanes: Vec<String>,
    /// What its lanes hold.
    pub seen: Seen,
}

impl Opening {
    /// Lays out `lanes` (each by module, with its height when holding what
    /// `seen` holds) for a work area `work` DIPs large.
    pub fn new(theme: &Theme, edge: Edge, lanes: &[(&str, f32)], work: (f32, f32), seen: Seen) -> Self {
        let (layout, zoom) = arrange(theme, edge, lanes.iter().map(|(_, height)| *height).collect(), work, None);
        Opening { layout, zoom, lanes: lanes.iter().map(|(id, _)| id.to_string()).collect(), seen }
    }

    /// Takes in what the machine has shown since (`now`), lane by lane,
    /// where it fits the lane's box: `height` is a lane's height holding a
    /// given set of readings.
    pub fn grow(&mut self, now: &Seen, info: &StaticInfo, height: impl Fn(&str, &Seen) -> Option<f32>) {
        self.seen.rename(now);
        let boxes = self.layout.lanes();
        for (id, area) in self.lanes.iter().zip(&boxes) {
            // A reading at a time: each that fits is taken, whether or not
            // the others do.
            for item in self.seen.news(now, id, info) {
                let grown = self.seen.with(&item, now);
                if height(id, &grown).is_some_and(|height| height <= area.h) {
                    self.seen = grown;
                }
            }
        }
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
    fn holds_its_outline_while_up() {
        use crate::reading::GpuInfo;
        let theme = Theme::new(Skin::Paper, false);
        let info = StaticInfo {
            cpu_name: String::new(),
            memory_modules: None,
            drives: Vec::new(),
            network_adapter: None,
            board: String::new(),
            threads: 1,
            mem_total: 1,
            gpus: vec![GpuInfo { slot: 0, name: String::new(), mem_total: 0, shared_total: 0 }],
            found: Vec::new(),
        };
        // The CPU's lane a column of its own; the GPU's and the memory's
        // share the other, and the room it has below them.
        let mut opened = Seen::default();
        opened.gpus.push(crate::ui::seen::GpuSeen { present: true, ..Default::default() });
        let lanes = [("cpu", 700.0), ("gpu:0", 100.0), ("memory", 100.0)];
        let mut opening = Opening::new(&theme, Edge::Right, &lanes, (1400.0, 900.0), opened.clone());
        assert_eq!(opening.layout.columns, 2);
        let (layout, zoom) = (opening.layout.height(), opening.zoom);
        // The GPU's clock and power read for the first time together: the
        // clock's row fits its box, both rows would not. The clock is taken,
        // the power waits; the outline never moves.
        let mut now = opened.clone();
        now.gpus[0].clock = true;
        now.gpus[0].power = true;
        let gpu = |seen: &Seen| 100.0 + if seen.gpus[0].clock { 19.0 } else { 0.0 } + if seen.gpus[0].power { 5000.0 } else { 0.0 };
        opening.grow(&now, &info, |id, seen| Some(if id == "gpu:0" { gpu(seen) } else { lanes.iter().find(|lane| lane.0 == id).unwrap().1 }));
        assert!(opening.seen.gpus[0].clock);
        assert!(!opening.seen.gpus[0].power);
        assert_eq!((opening.layout.height(), opening.zoom), (layout, zoom));
    }
}
