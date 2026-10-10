//! How the panel is laid out for a screen, on any platform: how many
//! columns, at what zoom, and the shape it keeps while it is up.

use super::seen::Seen;
use super::theme::Theme;
use super::forms::Small;
use super::view::{Detail, Layout, COLUMN_WIDTH};
use crate::reading::StaticInfo;
use crate::settings::Edge;

/// Closest the panel gets to the ends of the work area (DIPs).
pub const GAP: f32 = 12.0;
/// Columns a panel along a side may spread over before it zooms out instead.
const MAX_COLUMNS: usize = 3;
/// Share of the work area's height a panel opened from the top may take.
const TOP_SHARE: f32 = 0.6;

/// Lays the lanes out for a work area `work` DIPs large, and the zoom that
/// fits the panel into it at the size chosen (`size`, 1 as designed): as
/// many columns as the lanes need to show at that size, up to what the
/// screen allows, and zoomed out from it only if even that is too little.
///
/// The size scales the panel as it shows at 1: one zoomed out to fit the
/// screen at 1 is that much smaller again at less (laid out for more room,
/// it would otherwise be zoomed back to fit the screen, and look the same).
///
/// `columns`, when given (chosen in the settings), is kept instead, as far
/// as the screen's width allows it: chosen, it may be more than a panel
/// along a side takes by itself.
pub fn arrange(theme: &Theme, edge: Edge, heights: Vec<f32>, work: (f32, f32), columns: Option<usize>, size: f32) -> (Layout, f32) {
    let (layout, zoom) = fitted(theme, edge, heights.clone(), work, columns, size);
    let (_, at_one) = fitted(theme, edge, heights, work, columns, 1.0);
    (layout, zoom.min(size * at_one))
}

/// The panel laid out at `size` and the zoom that fits it on the screen:
/// laid out for the screen as the panel's own DIPs at that size measure
/// it, the gaps from the screen's ends staying as they are on the screen.
fn fitted(theme: &Theme, edge: Edge, heights: Vec<f32>, work: (f32, f32), columns: Option<usize>, size: f32) -> (Layout, f32) {
    let (layout, zoom) = arrange_designed(theme, edge, heights, (work.0 / size, work.1 / size), columns, GAP / size);
    (layout, zoom * size)
}

/// The room a panel has on a work area `work` large, `gap` from its ends:
/// across from its edge it keeps a gap from the far side; along its edge,
/// from both ends. Its width and its height.
/// The largest size, up to `most`, at which the panel still grows: past it
/// the screen holds no more of it (see `arrange`). In hundredths, as sizes
/// are chosen.
pub fn largest(theme: &Theme, edge: Edge, heights: &[f32], work: (f32, f32), columns: Option<usize>, most: f32) -> f32 {
    let (_, at_one) = fitted(theme, edge, heights.to_vec(), work, columns, 1.0);
    let grows = |size: f32| size * at_one <= fitted(theme, edge, heights.to_vec(), work, columns, size).1 + 1e-4;
    let mut size = (most * 100.0).round() as i32;
    while size > 100 && !grows(size as f32 / 100.0) {
        size -= 1;
    }
    size as f32 / 100.0
}

fn room(theme: &Theme, edge: Edge, work: (f32, f32), gap: f32) -> (f32, f32) {
    match edge {
        Edge::Left | Edge::Right => (work.0 - gap - theme.inset, work.1 - 2.0 * gap),
        // Along the top the panel grows sideways instead, and no lower than a
        // share of the screen.
        Edge::Top => (work.0 - 2.0 * gap, work.1 * TOP_SHARE - gap - theme.inset),
    }
}

fn arrange_designed(theme: &Theme, edge: Edge, heights: Vec<f32>, work: (f32, f32), columns: Option<usize>, gap: f32) -> (Layout, f32) {
    let (room_width, room) = room(theme, edge, work, gap);
    // As many columns as fit across, and along a side no more than a few.
    let fit = (((room_width + theme.column_gap) / (COLUMN_WIDTH + theme.column_gap)) as usize).max(1);
    let max_columns = if edge == Edge::Top { fit } else { fit.min(MAX_COLUMNS) };
    let layout = match columns {
        Some(columns) => Layout::with_columns(heights, columns.min(fit), theme),
        None => Layout::new(heights, room, max_columns, theme),
    };
    // Zoomed out only if even the most columns are too tall, or one is too wide.
    let zoom = (room / layout.height()).min(room_width / layout.width()).min(1.0);
    (layout, zoom)
}

/// A lane's height with all of it, and compact.
pub type Heights = [f32; 2];

fn at(heights: &Heights, detail: Detail) -> f32 {
    match detail {
        Detail::Full => heights[0],
        Detail::Compact => heights[1],
    }
}

/// What the whole panel is laid out as: its lanes, all of each or compact,
/// in so many columns; or one of the small layouts.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Form {
    Lanes { detail: Detail, columns: usize },
    Small(Small),
}

impl Form {
    /// How much it shows, 0 the most: forms of one level show the same,
    /// laid out differently (in columns, tiles across or down, a strip or
    /// a rail).
    pub fn level(self) -> u8 {
        match self {
            Form::Lanes { detail: Detail::Full, .. } => 0,
            Form::Lanes { detail: Detail::Compact, .. } => 1,
            Form::Small(Small::Tiles { .. }) => 2,
            Form::Small(Small::Corner | Small::Tall) => 3,
            Form::Small(Small::Strip { rich: true } | Small::Rail) => 4,
            Form::Small(Small::Strip { rich: false }) => 5,
            Form::Small(Small::Micro) => 6,
        }
    }

    /// The name it is kept under in the settings.
    pub fn key(self) -> String {
        match self {
            Form::Lanes { detail: Detail::Full, columns } => format!("full-{columns}"),
            Form::Lanes { detail: Detail::Compact, columns } => format!("compact-{columns}"),
            Form::Small(small) => small.key(),
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        let lanes = |detail, columns: &str| Some(Form::Lanes { detail, columns: columns.parse().ok().filter(|&c| c > 0)? });
        if let Some(columns) = key.strip_prefix("full-") {
            return lanes(Detail::Full, columns);
        }
        if let Some(columns) = key.strip_prefix("compact-") {
            return lanes(Detail::Compact, columns);
        }
        Small::from_key(key).map(Form::Small)
    }
}

/// A form as the panel shows it: at a scale (1 as designed), and so many
/// DIPs wider and taller than designed, its parts spread over the room.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Choice {
    pub form: Form,
    pub scale: f32,
    pub extra: (f32, f32),
}

impl Choice {
    /// As the settings keep it.
    pub fn kept(self) -> crate::settings::PanelLayout {
        crate::settings::PanelLayout { form: self.form.key(), scale: self.scale, extra: self.extra }
    }

    /// From the settings; none where its form is not one there is.
    pub fn from_kept(kept: &crate::settings::PanelLayout) -> Option<Self> {
        Some(Choice { form: Form::from_key(&kept.form)?, scale: kept.scale, extra: kept.extra })
    }
}

/// The most a form is scaled up: a larger panel shows more, not larger type.
const MOST_SCALE: f32 = 1.3;
/// The least the lanes are scaled down to before fewer of their parts show:
/// a panel made a little smaller is the same panel, smaller.
const LANES_FLOOR: f32 = 0.8;
/// Within its level, a form shown keeps being shown unless another covers
/// this much more of the room: no flicker between two layouts where they
/// cover about as much.
const STICK: f32 = 1.05;
/// A level showing more than the one shown comes back only once the room is
/// this much wider or taller than when that one was taken: a size on the
/// line between two levels keeps whichever it came with.
const UP: f32 = 1.08;

/// A form the panel offers, its size as designed, how much wider and
/// taller it may be drawn, and the least it may be scaled to.
#[derive(Clone, Copy, Debug)]
struct Rung {
    form: Form,
    size: (f32, f32),
    stretch: (f32, f32),
    floor: f32,
}

/// Every form the panel offers for its lanes and readings, the most shown
/// first: all of each lane in three columns down to one, compact in three
/// down to one, then the small layouts.
pub struct Ladder {
    rungs: Vec<Rung>,
}

impl Ladder {
    pub fn new(theme: &Theme, lanes: &[Heights], readings: usize) -> Self {
        let mut rungs = Vec::new();
        for detail in [Detail::Full, Detail::Compact] {
            for columns in (1..=MAX_COLUMNS.min(lanes.len())).rev() {
                let layout = Layout::with_columns(lanes.iter().map(|heights| at(heights, detail)).collect(), columns, theme);
                rungs.push(Rung {
                    form: Form::Lanes { detail, columns },
                    size: (layout.width(), layout.height()),
                    stretch: (120.0 * columns as f32, 60.0),
                    floor: LANES_FLOOR,
                });
            }
        }
        for small in Small::ladder(readings) {
            rungs.push(Rung { form: Form::Small(small), size: small.size_within(readings, theme.radius), stretch: small.stretch(), floor: small.floor() });
        }
        Ladder { rungs }
    }

    fn rung(&self, form: Form) -> Option<&Rung> {
        self.rungs.iter().find(|rung| rung.form == form)
    }

    /// What shows in `room` (DIPs): as much as fits. Of the forms that fit
    /// in it at their least scale or larger, those of the level showing the
    /// most (see `Form::level`); of these, the one covering the most of the
    /// room, scaled up as far as it fits (to `MOST_SCALE`) and stretched into
    /// what is left, though `shown` (the form shown, as it is sized) stays
    /// unless another covers clearly more (see `STICK`). A level showing
    /// more than `shown`'s counts only once the room is `UP` times as wide
    /// or tall as the room `shown` was taken in. Where none fits, the
    /// smallest at its least.
    ///
    /// The less room, the less shows: as the room shrinks (or grows) both
    /// ways, the level only goes down (or up), never back.
    pub fn choose(&self, room: (f32, f32), shown: Option<Shown>) -> Choice {
        let area = (room.0 * room.1).max(1.0);
        let grown = shown.is_none_or(|shown| room.0 >= shown.taken.0 * UP || room.1 >= shown.taken.1 * UP);
        let fitting: Vec<(Choice, f32)> = self
            .rungs
            .iter()
            .filter_map(|rung| {
                let fit = (room.0 / rung.size.0).min(room.1 / rung.size.1);
                let richer = shown.is_some_and(|shown| rung.form.level() < shown.form.level());
                (fit >= rung.floor && (grown || !richer)).then(|| {
                    let choice = fill(rung, room, fit.min(MOST_SCALE));
                    let (w, h) = self.drawn(choice);
                    (choice, w * h / area)
                })
            })
            .collect();
        let Some(level) = fitting.iter().map(|(choice, _)| choice.form.level()).min() else {
            let least = self.rungs.last().unwrap();
            return Choice { form: least.form, scale: least.floor, extra: (0.0, 0.0) };
        };
        let mine = fitting.iter().filter(|(choice, _)| choice.form.level() == level);
        let best = mine.clone().fold(None::<&(Choice, f32)>, |best, this| match best {
            Some(best) if best.1 >= this.1 => Some(best),
            _ => Some(this),
        });
        let (best, covered) = *best.unwrap();
        match mine.clone().find(|(choice, _)| shown.is_some_and(|shown| shown.form == choice.form)) {
            Some((kept, kept_covered)) if covered < kept_covered * STICK => *kept,
            _ => best,
        }
    }

    /// `choice` as this ladder has it, its stretch held to its form's. A
    /// form no longer offered (fewer lanes, other readings) gives way to its
    /// nearest: the lanes in fewer columns, tiles as many across; else the
    /// first form there is.
    pub fn keep(&self, choice: Choice) -> Choice {
        let offered = |form: &Form| self.rung(*form).is_some();
        let form = match choice.form {
            form if offered(&form) => Some(form),
            Form::Lanes { detail, columns } => (1..columns).rev().map(|columns| Form::Lanes { detail, columns }).find(offered),
            Form::Small(Small::Tiles { across, .. }) => self.rungs.iter().map(|rung| rung.form).find(|form| matches!(form, Form::Small(Small::Tiles { across: a, .. }) if *a == across)),
            Form::Small(_) => None,
        }
        .unwrap_or_else(|| self.rungs[0].form);
        let rung = self.rung(form).unwrap();
        Choice { form, scale: choice.scale.clamp(rung.floor, MOST_SCALE), extra: (choice.extra.0.clamp(0.0, rung.stretch.0), choice.extra.1.clamp(0.0, rung.stretch.1)) }
    }

    /// How large `choice` is drawn (the panel's own DIPs at the size chosen).
    pub fn drawn(&self, choice: Choice) -> (f32, f32) {
        let size = self.rung(choice.form).map_or((0.0, 0.0), |rung| rung.size);
        ((size.0 + choice.extra.0) * choice.scale, (size.1 + choice.extra.1) * choice.scale)
    }

    /// The form's size as designed, stretched by `choice`'s extra.
    pub fn stretched(&self, choice: Choice) -> (f32, f32) {
        let size = self.rung(choice.form).map_or((0.0, 0.0), |rung| rung.size);
        (size.0 + choice.extra.0, size.1 + choice.extra.1)
    }
}

/// `rung` at `scale`, stretched as far as it may into `room`.
fn fill(rung: &Rung, room: (f32, f32), scale: f32) -> Choice {
    let extra = (
        (room.0 / scale - rung.size.0).clamp(0.0, rung.stretch.0),
        (room.1 / scale - rung.size.1).clamp(0.0, rung.stretch.1),
    );
    Choice { form: rung.form, scale, extra }
}

/// The form shown and the room (DIPs) it was taken in.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Shown {
    pub form: Form,
    pub taken: (f32, f32),
}

impl Shown {
    /// What shows once `form` is chosen in `room`, after `before`.
    pub fn after(before: Option<Shown>, form: Form, room: (f32, f32)) -> Shown {
        match before {
            Some(before) if before.form == form => before,
            _ => Shown { form, taken: room },
        }
    }
}

/// What a panel is laid out as.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Want {
    /// By itself: all of each lane, as many columns as the screen needs.
    Itself,
    /// As it was kept (see `Choice`).
    Kept(Choice),
    /// What shows in a room this large (DIPs on the screen), as it is sized
    /// by its edges, and what it showed (see `Ladder::choose`).
    Room((f32, f32), Option<Shown>),
}

/// A panel while it is up: laid out once, as it opened, and held. Its
/// size, its columns, its zoom and each lane's box stay as they were; a
/// reading the machine shows for the first time joins its lane at once
/// only where the lane's box has room for it, and anything that would need
/// more waits for the next opening. A reading gone stays in its place,
/// unread. (Settings changed while it is up lay it out afresh, holding
/// what it held: see `Seen::join`.)
pub struct Opening {
    /// Its lanes laid out (for a small layout, all of each in one column,
    /// unshown).
    pub layout: Layout,
    pub zoom: f32,
    /// What it shows: none, laid out by itself (all of each lane, as many
    /// columns as the screen needs; see `arrange`).
    pub choice: Option<Choice>,
    /// Its size, its own DIPs (as drawn at `zoom`).
    pub size: (f32, f32),
    /// The lanes it opened with, by module.
    lanes: Vec<String>,
    /// What its lanes hold.
    pub seen: Seen,
}

impl Opening {
    /// Lays out `lanes` (each by module, with its heights when holding what
    /// `seen` holds) and the small layouts' `readings` for a work area
    /// `work` DIPs large, at the size chosen, as `want` says: by itself,
    /// in `columns` if chosen (see `arrange`); else as far as the screen
    /// holds what is wanted, and otherwise what best fills the screen.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        theme: &Theme,
        edge: Edge,
        lanes: &[(&str, Heights)],
        readings: usize,
        work: (f32, f32),
        columns: Option<usize>,
        size: f32,
        want: Want,
        seen: Seen,
    ) -> Self {
        let heights: Vec<Heights> = lanes.iter().map(|(_, heights)| *heights).collect();
        let ids = lanes.iter().map(|(id, _)| id.to_string()).collect();
        let ladder = Ladder::new(theme, &heights, readings);
        let kept = match want {
            Want::Itself => {
                let (layout, zoom) = arrange(theme, edge, heights.iter().map(|heights| heights[0]).collect(), work, columns, size);
                return Opening { size: (layout.width(), layout.height()), layout, zoom, choice: None, lanes: ids, seen };
            }
            Want::Kept(choice) => ladder.keep(choice),
            Want::Room((width, height), shown) => ladder.choose((width / size, height / size), shown.map(|shown| Shown { taken: (shown.taken.0 / size, shown.taken.1 / size), ..shown })),
        };
        let (room_width, room_height) = room(theme, edge, (work.0 / size, work.1 / size), GAP / size);
        let drawn = ladder.drawn(kept);
        let choice = if drawn.0 <= room_width + 0.5 && drawn.1 <= room_height + 0.5 { kept } else { ladder.choose((room_width, room_height), Some(Shown { form: kept.form, taken: (room_width, room_height) })) };
        let stretched = ladder.stretched(choice);
        // At its scale, unless the screen cannot hold even the least of it.
        let zoom = size * choice.scale * (room_width / (stretched.0 * choice.scale)).min(room_height / (stretched.1 * choice.scale)).min(1.0);
        let layout = match choice.form {
            Form::Lanes { detail, columns } => {
                let designed = Layout::with_columns(heights.iter().map(|heights| at(heights, detail)).collect(), columns, theme);
                designed.stretched(choice.extra)
            }
            Form::Small(_) => Layout::with_columns(heights.iter().map(|heights| heights[0]).collect(), 1, theme),
        };
        Opening { layout, zoom, choice: Some(choice), size: stretched, lanes: ids, seen }
    }

    /// How much of each lane it shows.
    pub fn detail(&self) -> Detail {
        match self.choice.map(|choice| choice.form) {
            Some(Form::Lanes { detail, .. }) => detail,
            _ => Detail::Full,
        }
    }

    /// The small layout it shows, if it shows one.
    pub fn small(&self) -> Option<Small> {
        match self.choice?.form {
            Form::Small(small) => Some(small),
            Form::Lanes { .. } => None,
        }
    }

    /// Takes in what the machine has shown since (`now`), lane by lane,
    /// where it fits the lane's box: `height` is a lane's height at a
    /// detail holding a given set of readings.
    pub fn grow(&mut self, now: &Seen, info: &StaticInfo, height: impl Fn(&str, &Seen, Detail) -> Option<f32>) {
        self.seen.rename(now);
        if self.small().is_some() {
            return;
        }
        let detail = self.detail();
        let boxes = self.layout.lanes();
        for (id, area) in self.lanes.iter().zip(&boxes) {
            // A reading at a time: each that fits is taken, whether or not
            // the others do.
            for item in self.seen.news(now, id, info) {
                let grown = self.seen.with(&item, now);
                if height(id, &grown, detail).is_some_and(|height| height <= area.h) {
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
        let (layout, zoom) = arrange(&theme, Edge::Right, vec![400.0; 6], (600.0, 1000.0), None, 1.0);
        assert_eq!(layout.columns, 1);
        assert!(zoom < 1.0 && layout.height() * zoom <= 1000.0 - 2.0 * GAP + 0.01);
        // Narrower than a column: the column is zoomed to fit across.
        let (layout, zoom) = arrange(&theme, Edge::Top, vec![100.0], (300.0, 1000.0), None, 1.0);
        assert!(layout.width() * zoom <= 300.0 - 2.0 * GAP + 0.01);
        assert!(zoom.is_finite() && zoom > 0.0);
    }

    #[test]
    fn keeps_the_columns_chosen_as_far_as_the_screen_is_wide() {
        let theme = Theme::new(Skin::Paper, false);
        // Lanes that fit one column on a tall screen: four chosen, four taken,
        // more than a side takes by itself.
        let (layout, _) = arrange(&theme, Edge::Right, vec![100.0; 6], (3000.0, 2000.0), Some(4), 1.0);
        assert_eq!(layout.columns, 4);
        // On a screen that holds two across, two.
        let two = 2.0 * COLUMN_WIDTH + theme.column_gap + GAP + theme.inset + 1.0;
        let (layout, _) = arrange(&theme, Edge::Right, vec![100.0; 6], (two, 2000.0), Some(4), 1.0);
        assert_eq!(layout.columns, 2);
        // One chosen and too tall for the screen: one, zoomed out.
        let (layout, zoom) = arrange(&theme, Edge::Right, vec![400.0; 6], (3000.0, 1000.0), Some(1), 1.0);
        assert_eq!(layout.columns, 1);
        assert!(zoom < 1.0);
    }

    #[test]
    fn grows_to_the_size_chosen_as_far_as_the_screen_allows() {
        let theme = Theme::new(Skin::Paper, false);
        // Half again as large, with room for it.
        let (layout, zoom) = arrange(&theme, Edge::Right, vec![300.0], (3000.0, 2000.0), None, 1.5);
        assert_eq!((layout.columns, zoom), (1, 1.5));
        // Twice as large on a screen with room for less: as large as fits.
        let (layout, zoom) = arrange(&theme, Edge::Right, vec![600.0], (400.0, 1000.0), Some(1), 2.0);
        assert!(zoom < 2.0 && layout.height() * zoom <= 1000.0 - 2.0 * GAP + 0.01);
    }

    #[test]
    fn a_panel_filling_the_screen_shrinks_with_its_size() {
        let theme = Theme::new(Skin::Paper, false);
        // One column too tall for the screen: zoomed to fit at 1, and three
        // quarters of that at three quarters, not zoomed back to fit.
        let lanes = || vec![400.0; 6];
        let (_, one) = arrange(&theme, Edge::Right, lanes(), (3000.0, 1000.0), Some(1), 1.0);
        let (_, less) = arrange(&theme, Edge::Right, lanes(), (3000.0, 1000.0), Some(1), 0.75);
        assert!(one < 1.0);
        assert!((less - 0.75 * one).abs() < 1e-4);
    }

    /// Six lanes 300 DIPs tall with all of each, 150 compact.
    fn lanes() -> Vec<Heights> {
        vec![[300.0, 150.0]; 6]
    }

    #[test]
    fn a_large_room_shows_all_of_each_lane_and_a_small_one_rings() {
        let theme = Theme::new(Skin::Paper, false);
        let ladder = Ladder::new(&theme, &lanes(), 6);
        let three = Ladder::new(&theme, &lanes(), 6).drawn(Choice { form: Form::Lanes { detail: Detail::Full, columns: 3 }, scale: 1.0, extra: (0.0, 0.0) });
        assert_eq!(ladder.choose(three, None).form, Form::Lanes { detail: Detail::Full, columns: 3 });
        assert_eq!(ladder.choose((130.0, 40.0), None).form, Form::Small(Small::Micro));
        assert_eq!(ladder.choose((90.0, 320.0), None).form, Form::Small(Small::Rail));
        assert!(matches!(ladder.choose((700.0, 36.0), None).form, Form::Small(Small::Strip { .. })));
    }

    #[test]
    fn between_its_forms_a_panel_grows_with_its_room() {
        let theme = Theme::new(Skin::Paper, false);
        let ladder = Ladder::new(&theme, &lanes(), 6);
        // Within one form, a little more room is a little larger, stretched
        // and scaled: never a jump.
        let a = ladder.choose((480.0, 230.0), None);
        let b = ladder.choose((490.0, 234.0), None);
        assert_eq!(a.form, b.form);
        let (wa, ha) = ladder.drawn(a);
        let (wb, hb) = ladder.drawn(b);
        assert!(wb >= wa && hb >= ha && wb - wa <= 10.5 && hb - ha <= 4.5, "{:?} {:?}", (wa, ha), (wb, hb));
        // Never larger than its room, nor scaled past the most.
        for room in [(300.0, 200.0), (1200.0, 900.0), (2000.0, 1400.0), (100.0, 40.0)] {
            let choice = ladder.choose(room, None);
            let (w, h) = ladder.drawn(choice);
            assert!(choice.scale <= MOST_SCALE + 1e-4);
            assert!(w <= room.0 + 0.5 && h <= room.1 + 0.5 || matches!(choice.form, Form::Small(Small::Micro)), "{room:?} {choice:?}");
        }
    }

    #[test]
    fn a_level_left_comes_back_only_with_room_to_spare() {
        let theme = Theme::new(Skin::Paper, false);
        let lanes = [[196.0, 89.0], [177.0, 113.0], [134.0, 89.0], [111.0, 85.0], [85.0, 85.0], [159.0, 115.0], [89.0, 89.0]];
        let ladder = Ladder::new(&theme, &lanes, 6);
        // Narrowed a pixel at a time until the level drops, then widened:
        // a few pixels back keep the level it dropped to, `UP` times the
        // width it dropped at brings the other back.
        let mut drops = 0;
        for height in [600.0, 440.0, 320.0, 200.0] {
            let mut shown = Shown::after(None, ladder.choose((1400.0, height), None).form, (1400.0, height));
            let mut width = 1400.0;
            while width > 200.0 {
                let room = (width - 1.0, height);
                let next = Shown::after(Some(shown), ladder.choose(room, Some(shown)).form, room);
                if next.form.level() > shown.form.level() {
                    drops += 1;
                    for back in [width, width + 4.0, width * UP - 2.0] {
                        assert_eq!(ladder.choose((back, height), Some(next)).form.level(), next.form.level(), "{height}: {back} from {next:?}");
                    }
                    let room = (width * UP + 1.0, height);
                    assert!(ladder.choose(room, Some(next)).form.level() <= shown.form.level(), "{height}: {room:?} from {next:?}");
                }
                shown = next;
                width -= 1.0;
            }
        }
        assert!(drops >= 8, "{drops}");
    }

    #[test]
    fn a_narrow_tall_room_gets_the_corner_on_end() {
        let theme = Theme::new(Skin::Paper, false);
        let lanes = [[196.0, 89.0], [177.0, 113.0], [134.0, 89.0], [111.0, 85.0], [85.0, 85.0], [159.0, 115.0], [89.0, 89.0]];
        let ladder = Ladder::new(&theme, &lanes, 6);
        assert_eq!(ladder.choose((260.0, 420.0), None).form, Form::Small(Small::Tall));
        assert_eq!(ladder.choose((320.0, 190.0), None).form, Form::Small(Small::Corner));
    }

    #[test]
    fn the_less_room_the_less_shows_never_back() {
        let theme = Theme::new(Skin::Paper, false);
        let lanes = [[196.0, 89.0], [177.0, 113.0], [134.0, 89.0], [111.0, 85.0], [85.0, 85.0], [159.0, 115.0], [89.0, 89.0]];
        let ladder = Ladder::new(&theme, &lanes, 6);
        // Narrower and narrower, then shorter and shorter, each from the
        // form shown: the level only goes down, and passes the tiles on the
        // way where they fit at all.
        let sweep = |rooms: Vec<(f32, f32)>| {
            let mut shown: Option<Shown> = None;
            let mut levels = Vec::new();
            for room in rooms {
                let choice = ladder.choose(room, shown);
                shown = Some(Shown::after(shown, choice.form, room));
                levels.push(choice.form.level());
            }
            levels
        };
        for height in [600.0, 440.0, 320.0, 200.0] {
            let levels = sweep((0..680).map(|i| (1400.0 - 2.0 * i as f32, height)).collect());
            assert!(levels.windows(2).all(|pair| pair[1] >= pair[0]), "{height}: {levels:?}");
        }
        for width in [900.0, 500.0, 300.0] {
            let levels = sweep((0..490).map(|i| (width, 1000.0 - 2.0 * i as f32)).collect());
            assert!(levels.windows(2).all(|pair| pair[1] >= pair[0]), "{width}: {levels:?}");
        }
        let levels = sweep((0..680).map(|i| (1400.0 - 2.0 * i as f32, 440.0)).collect());
        assert!(levels.contains(&2), "{levels:?}");
    }

    #[test]
    fn a_kept_form_is_held_to_the_screen() {
        let theme = Theme::new(Skin::Paper, false);
        let lanes = [("cpu", [300.0, 150.0]), ("gpu:0", [300.0, 150.0]), ("memory", [300.0, 150.0])];
        let three = Choice { form: Form::Lanes { detail: Detail::Full, columns: 3 }, scale: 1.3, extra: (0.0, 0.0) };
        // Room for it: as kept.
        let opening = Opening::new(&theme, Edge::Right, &lanes, 6, (3000.0, 2000.0), None, 1.0, Want::Kept(three), Seen::default());
        assert_eq!(opening.choice, Some(three));
        // A screen too small for it: what best fills the screen.
        let opening = Opening::new(&theme, Edge::Right, &lanes, 6, (800.0, 600.0), None, 1.0, Want::Kept(three), Seen::default());
        assert_ne!(opening.choice.unwrap().form, three.form);
        assert!(opening.size.0 * opening.zoom <= 800.0 && opening.size.1 * opening.zoom <= 600.0);
    }

    #[test]
    fn keeps_each_form_under_a_name() {
        for form in [Form::Lanes { detail: Detail::Full, columns: 2 }, Form::Lanes { detail: Detail::Compact, columns: 1 }, Form::Small(Small::Rail)] {
            assert_eq!(Form::from_key(&form.key()), Some(form));
        }
        assert_eq!(Form::from_key("full-0"), None);
    }

    #[test]
    fn holds_its_outline_while_up() {
        use crate::reading::GpuInfo;
        let theme = Theme::new(Skin::Paper, false);
        let info = StaticInfo {
            cpu_name: String::new(),
            memory_modules: None,
            memory_speed: None,
            drives: Vec::new(),
            drive_ids: Vec::new(),
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
        let lanes = [("cpu", [700.0; 2]), ("gpu:0", [100.0; 2]), ("memory", [100.0; 2])];
        let mut opening = Opening::new(&theme, Edge::Right, &lanes, 3, (1400.0, 900.0), None, 1.0, Want::Itself, opened.clone());
        assert_eq!(opening.layout.columns, 2);
        let (layout, zoom) = (opening.layout.height(), opening.zoom);
        // The GPU's clock and power read for the first time together: the
        // clock's row fits its box, both rows would not. The clock is taken,
        // the power waits; the outline never moves.
        let mut now = opened.clone();
        now.gpus[0].clock = true;
        now.gpus[0].power = true;
        let gpu = |seen: &Seen| 100.0 + if seen.gpus[0].clock { 19.0 } else { 0.0 } + if seen.gpus[0].power { 5000.0 } else { 0.0 };
        opening.grow(&now, &info, |id, seen, _| Some(if id == "gpu:0" { gpu(seen) } else { lanes.iter().find(|lane| lane.0 == id).unwrap().1[0] }));
        assert!(opening.seen.gpus[0].clock);
        assert!(!opening.seen.gpus[0].power);
        assert_eq!((opening.layout.height(), opening.zoom), (layout, zoom));
    }
}
