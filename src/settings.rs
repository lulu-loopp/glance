#[cfg(windows)]
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
#[cfg(windows)]
#[cfg(not(test))]
use windows::Win32::System::Com::CoTaskMemFree;
#[cfg(windows)]
use windows::Win32::UI::Shell::{FOLDERID_LocalAppData, FOLDERID_RoamingAppData};
#[cfg(all(windows, not(test)))]
use windows::Win32::UI::Shell::{SHGetKnownFolderPath, KF_FLAG_DEFAULT};

#[cfg(windows)]
const FILE: &str = "settings.json";

/// Whether Glance has saved settings in `dir` before.
#[cfg(windows)]
pub fn saved(dir: &Path) -> bool {
    dir.join(FILE).is_file()
}
/// Larger than this, a settings file is not one Glance wrote (it writes none larger: see `Settings::save`).
#[cfg(windows)]
const MOST: u64 = 16 << 20;
/// The settings file as it was, where it did not read whole (see `Settings::load`).
#[cfg(windows)]
pub const UNREAD: &str = "settings.unread.json";

/// The settings `text` holds, and the parts of it left out, by where they are (`widgets[1].layout`): none where it
/// reads whole. Where it does not, every part that does is kept (see `salvaged`).
fn read(text: &str) -> (Settings, Vec<String>) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else { return (Settings::default(), vec!["all of it (not JSON)".to_string()]) };
    read_value(value, &Settings::default())
}

/// The settings `value` holds, and the parts of it left out (see `salvaged`), each as `from` has it.
fn read_value(value: serde_json::Value, from: &Settings) -> (Settings, Vec<String>) {
    if usable(&value) {
        if let Ok(settings) = serde_json::from_value(value.clone()) {
            return (settings, Vec::new());
        }
    }
    salvaged(value, from)
}

/// Whether `value` is settings as Glance writes them: read, written again and read again alike (a number too large
/// to be one reads as infinite, and is written as none at all).
fn usable(value: &serde_json::Value) -> bool {
    serde_json::from_value::<Settings>(value.clone())
        .ok()
        .and_then(|settings| serde_json::to_value(&settings).ok())
        .is_some_and(|again| serde_json::from_value::<Settings>(again).is_ok())
}

/// Settings from `kept` that does not read whole: from `from` (the defaults, read from a file), each part of it put
/// in its place where the whole still reads; one that does not, an object or a list, its own parts one by one (a widget that does not read
/// leaves the others); a part that reads in no way, left as the defaults have it. Returns them, and the parts left
/// out, by where they are.
fn salvaged(kept: serde_json::Value, from: &Settings) -> (Settings, Vec<String>) {
    let mut whole = serde_json::to_value(from).unwrap_or(serde_json::Value::Null);
    let mut left_out = Vec::new();
    let serde_json::Value::Object(fields) = kept else { return (from.clone(), vec!["all of it (not an object)".to_string()]) };
    for (name, part) in fields {
        graft(&mut whole, &[Step::Key(name.clone())], &name, part, &mut left_out);
    }
    (serde_json::from_value(whole).unwrap_or_else(|_| from.clone()), left_out)
}

/// A step into a JSON value: a field of an object, or a place in a list.
#[derive(Clone)]
enum Step {
    Key(String),
    Place(usize),
}

/// `part` put at `path` in `whole` where `whole` still reads as settings (see `usable`); where it does not, an
/// object or a list, put there empty first and filled part by part (each that fits kept, the others left out and
/// named in `left_out` as the file has them: `shown`); else left out whole, `whole` as it was.
fn graft(whole: &mut serde_json::Value, path: &[Step], shown: &str, part: serde_json::Value, left_out: &mut Vec<String>) {
    let before = at(whole, path).cloned();
    *slot(whole, path) = part.clone();
    if usable(whole) {
        return;
    }
    let empty = match &part {
        serde_json::Value::Object(_) => Some(serde_json::Value::Object(serde_json::Map::new())),
        serde_json::Value::Array(_) => Some(serde_json::Value::Array(Vec::new())),
        _ => None,
    };
    // (An object starts from what the defaults have there, where they have one: its fields that do not read keep
    // theirs.)
    let start = match (&part, &before) {
        (serde_json::Value::Object(_), Some(serde_json::Value::Object(defaults))) => Some(serde_json::Value::Object(defaults.clone())),
        _ => empty,
    };
    if let Some(start) = start {
        *slot(whole, path) = start;
        if usable(whole) {
            match part {
                serde_json::Value::Object(fields) => {
                    for (name, inner) in fields {
                        let mut deeper = path.to_vec();
                        deeper.push(Step::Key(name.clone()));
                        graft(whole, &deeper, &format!("{shown}.{name}"), inner, left_out);
                    }
                }
                serde_json::Value::Array(items) => {
                    for (index, item) in items.into_iter().enumerate() {
                        let place = at(whole, path).and_then(serde_json::Value::as_array).map_or(0, Vec::len);
                        if let Some(serde_json::Value::Array(list)) = at_mut(whole, path) {
                            list.push(serde_json::Value::Null);
                        }
                        let mut deeper = path.to_vec();
                        deeper.push(Step::Place(place));
                        graft(whole, &deeper, &format!("{shown}[{index}]"), item, left_out);
                        // (One that does not fit is taken out again, not left as a hole.)
                        if at(whole, &deeper).is_some_and(serde_json::Value::is_null) {
                            if let Some(serde_json::Value::Array(list)) = at_mut(whole, path) {
                                list.pop();
                            }
                        }
                    }
                }
                _ => {}
            }
            return;
        }
    }
    left_out.push(shown.to_string());
    match before {
        Some(before) => *slot(whole, path) = before,
        None => remove(whole, path),
    }
}

/// The value at `path` in `whole`, if there is one.
fn at<'a>(whole: &'a serde_json::Value, path: &[Step]) -> Option<&'a serde_json::Value> {
    path.iter().try_fold(whole, |value, step| match step {
        Step::Key(name) => value.get(name),
        Step::Place(place) => value.get(*place),
    })
}

fn at_mut<'a>(whole: &'a mut serde_json::Value, path: &[Step]) -> Option<&'a mut serde_json::Value> {
    path.iter().try_fold(whole, |value, step| match step {
        Step::Key(name) => value.get_mut(name),
        Step::Place(place) => value.get_mut(*place),
    })
}

/// The place at `path` in `whole`, made where it is not there yet (its parent is: `graft` goes down from it).
fn slot<'a>(whole: &'a mut serde_json::Value, path: &[Step]) -> &'a mut serde_json::Value {
    let (last, parent) = path.split_last().expect("a path has a step");
    let parent = at_mut(whole, parent).expect("the parent is there");
    match last {
        Step::Key(name) => {
            if !parent.is_object() {
                *parent = serde_json::Value::Object(serde_json::Map::new());
            }
            parent.as_object_mut().unwrap().entry(name.clone()).or_insert(serde_json::Value::Null)
        }
        Step::Place(place) => &mut parent.as_array_mut().expect("a list")[*place],
    }
}

fn remove(whole: &mut serde_json::Value, path: &[Step]) {
    let Some((Step::Key(name), parent)) = path.split_last() else { return };
    if let Some(serde_json::Value::Object(fields)) = at_mut(whole, parent) {
        fields.remove(name);
    }
}


#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Left,
    #[default]
    Right,
    Top,
}

/// The edges a panel can open from, in the order they are gone through.
pub const EDGES: [Edge; 3] = [Edge::Left, Edge::Top, Edge::Right];

/// What an edge opens: a component, the panel it slides out.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Part {
    /// The PC's readings.
    Monitor,
    /// What the user has at hand: the time and what plays (the station, notes and quick launch to come).
    Workbench,
    /// What plays, its lyrics and its controls.
    Music,
}

/// Every component there is, in the order the settings offer them.
pub const PARTS: [Part; 3] = [Part::Monitor, Part::Workbench, Part::Music];

/// One edge of a screen and the component it opens.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Hold {
    pub edge: Edge,
    pub part: Part,
}

/// What the edges of one screen open: the screen by the name that stays its own (see `screens`).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ScreenHolds {
    pub screen: String,
    pub holds: Vec<Hold>,
}

/// What each screen edge opens: a screen's own where it was set apart, else as every screen not set apart does
/// (every screen alike, while `same`). An edge holding a component opens it where the pointer is pushed into it, and
/// where another screen lies against it (a seam), where the pointer rests there.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Places {
    pub same: bool,
    pub every: Vec<Hold>,
    pub screens: Vec<ScreenHolds>,
}

impl Default for Places {
    /// The readings at the right edge of every screen.
    fn default() -> Self {
        Places { same: true, every: vec![Hold { edge: Edge::Right, part: Part::Monitor }], screens: Vec::new() }
    }
}

impl Places {
    /// What the edges of the screen named `screen` open.
    pub fn holds(&self, screen: &str) -> &[Hold] {
        match self.screens.iter().find(|entry| !self.same && entry.screen == screen) {
            Some(entry) => &entry.holds,
            None => &self.every,
        }
    }

    /// What `edge` of the screen named `screen` opens (none: nothing).
    pub fn part(&self, screen: &str, edge: Edge) -> Option<Part> {
        self.holds(screen).iter().find(|hold| hold.edge == edge).map(|hold| hold.part)
    }

    /// The edges of the screen named `screen` that open a panel.
    pub fn lit(&self, screen: &str) -> Lit {
        Lit::of(&EDGES.into_iter().filter(|edge| self.part(screen, *edge).is_some()).collect::<Vec<_>>())
    }

    /// Every list of edges kept: every screen's, and while they are set apart, each one's.
    fn all(&self) -> impl Iterator<Item = &Hold> {
        let apart = if self.same { &[][..] } else { &self.screens[..] };
        self.every.iter().chain(apart.iter().flat_map(|entry| &entry.holds))
    }

    /// Whether `edge` of any screen opens a panel.
    pub fn anywhere(&self, edge: Edge) -> bool {
        self.all().any(|hold| hold.edge == edge)
    }

    /// Whether any edge of any screen opens `part`.
    pub fn shows(&self, part: Part) -> bool {
        self.all().any(|hold| hold.part == part)
    }

    /// What the edges of the screen named `screen` open, to be changed: that screen's own (set apart from the rest as
    /// it was, if it was not), or every screen's while they are alike.
    fn holds_mut(&mut self, screen: &str) -> &mut Vec<Hold> {
        if self.same {
            return &mut self.every;
        }
        let at = match self.screens.iter().position(|entry| entry.screen == screen) {
            Some(at) => at,
            None => {
                self.screens.push(ScreenHolds { screen: screen.to_string(), holds: self.every.clone() });
                self.screens.len() - 1
            }
        };
        &mut self.screens[at].holds
    }

    /// `part` put on `edge` of the screen named `screen`, in place of what it held. Moved there from another edge
    /// (`from`, its screen and edge), that one takes what this held, if it held anything: the two change places.
    pub fn put(&mut self, screen: &str, edge: Edge, part: Part, from: Option<(&str, Edge)>) {
        let was = self.part(screen, edge);
        if let Some((from_screen, from_edge)) = from {
            self.take(from_screen, from_edge);
            if let Some(was) = was {
                self.holds_mut(from_screen).push(Hold { edge: from_edge, part: was });
            }
        }
        let holds = self.holds_mut(screen);
        holds.retain(|hold| hold.edge != edge);
        holds.push(Hold { edge, part });
    }

    /// What `edge` of the screen named `screen` held, taken off it: it opens nothing.
    pub fn take(&mut self, screen: &str, edge: Edge) {
        self.holds_mut(screen).retain(|hold| hold.edge != edge);
    }

    /// Every screen alike (as the screen named `screen` is now), or each set apart as it is now.
    pub fn set_same(&mut self, same: bool, screen: &str) {
        if same && !self.same {
            self.every = self.holds(screen).to_vec();
            self.screens.clear();
        }
        self.same = same;
    }

    /// Where a panel opened by hand (the shortcut, the tray) on a screen none of whose edges opens one opens, and
    /// what it shows: as the first edge kept; the readings at the right edge, where no edge opens anything anywhere.
    pub fn by_hand(&self) -> (Edge, Part) {
        self.all().next().map_or((Edge::Right, Part::Monitor), |hold| (hold.edge, hold.part))
    }
}

/// The edges of one screen that opened the panel, up to 0.3.11 (see `Places`): the screen by the name that stays
/// its own; those that opened it where they are open, to a pointer pushed into them; and those that opened it where
/// another screen lies against them (a seam), to a pointer resting there.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ScreenEdges {
    pub screen: String,
    pub edges: Vec<Edge>,
    #[serde(default)]
    pub seams: Vec<Edge>,
}

/// Which of a screen's edges open a panel.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Lit {
    pub left: bool,
    pub top: bool,
    pub right: bool,
}

impl Lit {
    pub fn of(edges: &[Edge]) -> Lit {
        Lit { left: edges.contains(&Edge::Left), top: edges.contains(&Edge::Top), right: edges.contains(&Edge::Right) }
    }

    pub fn has(self, edge: Edge) -> bool {
        match edge {
            Edge::Left => self.left,
            Edge::Top => self.top,
            Edge::Right => self.right,
        }
    }

    pub fn each(self) -> impl Iterator<Item = Edge> {
        EDGES.into_iter().filter(move |edge| self.has(*edge))
    }

    /// The one of them nearest `point` on a screen at `monitor` (left, top,
    /// right, bottom); none, when none of its edges opens a panel.
    pub fn nearest(self, monitor: (i32, i32, i32, i32), point: (i32, i32)) -> Option<Edge> {
        let (left, top, right, _) = monitor;
        self.each().min_by_key(|edge| match edge {
            Edge::Left => point.0 - left,
            Edge::Top => point.1 - top,
            Edge::Right => right - point.0,
        })
    }
}

/// What a desktop widget was sized to show (see `arrange::Choice`): its
/// form by name ("full-2", "compact-1", "tiles-3x2", "rail", …), its
/// scale, and how much wider and taller than designed it is drawn (DIPs).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelLayout {
    pub form: String,
    pub scale: f32,
    pub extra: (f32, f32),
    /// Its lanes the workbench's, not the readings'.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub workbench: bool,
}

/// A player music is followed from: as Windows identifies it, and by the name it is shown by (said in the settings
/// while it is not running).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct MusicSource {
    pub id: String,
    pub name: String,
}

/// The form a widget showing what plays is kept as, in place of the readings' (see `PanelLayout`): an older Glance,
/// which has no such form, leaves it out rather than show something else.
pub const MUSIC_FORM: &str = "music";

impl WidgetAt {
    /// Whether it shows what plays (merged into it or not).
    pub fn shows_music(&self) -> bool {
        self.layout.form == MUSIC_FORM || self.parts.iter().flatten().any(|part| part.form == MUSIC_FORM)
    }
}

/// A desktop widget as it is kept: its surface's corner on the screen
/// (physical px), what it shows, whether it is pinned (not moved or sized),
/// lets clicks through and shows only while a game runs, and the side of its
/// screen it is stuck to, if it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WidgetAt {
    pub at: (i32, i32),
    pub layout: PanelLayout,
    #[serde(default)]
    pub pinned: bool,
    /// Clicks pass through it (Ctrl held, it takes them).
    #[serde(default)]
    pub click_through: bool,
    #[serde(default)]
    pub game_only: bool,
    /// Kept on the desktop, under every other window, rather than above
    /// them all.
    #[serde(default)]
    pub on_desktop: bool,
    #[serde(default)]
    pub stuck: Option<WidgetSide>,
    /// The side across `stuck` it is stuck to as well: it is stuck in their corner.
    #[serde(default)]
    pub corner: Option<WidgetSide>,
    /// How large it is (DIPs), where its size is its own (what plays) rather than its form's.
    #[serde(default)]
    pub size: Option<(f32, f32)>,
    /// Widgets merged into it: each part's form (its own first), and whether they are beside it rather than below.
    #[serde(default)]
    pub parts: Option<Vec<PanelLayout>>,
    #[serde(default)]
    pub beside: bool,
    /// What it showed before it was stuck to an edge and made a rail or a strip there (as widgets were up to 0.3.13:
    /// stuck now, one shows what it showed): read, to show that again (see `Settings::carry_over`), never written.
    #[serde(default, skip_serializing)]
    pub before: Option<PanelLayout>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WidgetSide {
    Left,
    Right,
    Top,
    Bottom,
}

/// Where along the edge the panel opens.
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Anchor {
    /// Centred on the pointer, as far as the screen allows.
    #[default]
    Pointer,
    Center,
}

/// A key combination: modifier keys held, and one other key, by its
/// Windows virtual-key code.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Shortcut {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub key: u16,
}

impl Default for Shortcut {
    /// Ctrl+Alt+G.
    fn default() -> Self {
        Shortcut { ctrl: true, alt: true, shift: false, win: false, key: u16::from(b'G') }
    }
}

impl Shortcut {
    /// Whether it can be a shortcut: a key that is not itself a modifier,
    /// with Ctrl, Alt or Win held (Shift alone would take a key from typing).
    pub fn usable(&self) -> bool {
        // Shift, Ctrl, Alt (either side or neither) and the Windows keys.
        const MODIFIERS: [u16; 9] = [0x10, 0x11, 0x12, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5];
        let key = self.key != 0 && !MODIFIERS.contains(&self.key) && self.key != 0x5B && self.key != 0x5C;
        key && (self.ctrl || self.alt || self.win)
    }
}

/// Where the panel was moved to, away from the screen's edge: it stays
/// there, Glance restarting too, until it is moved back to the edge.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct PanelAt {
    /// How far across and down the room there is for it on its screen's
    /// work area (0 at the left or top, 1 at the right or bottom).
    pub at: (f32, f32),
    /// A point on its screen (physical pixels).
    pub screen: (i32, i32),
    /// Pinned there: not moved or sized by the pointer.
    #[serde(default)]
    pub locked: bool,
}

/// A few readings floating over the screen: all the time, or while a game
/// is played (over the game) only.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlaySettings {
    /// Shown all the time.
    pub on: bool,
    /// Shown while a game is played, off or not otherwise.
    pub in_game: bool,
    /// What it shows, group by group, in order (see `ui::overlay::GROUPS`).
    pub groups: Vec<crate::ui::prefs::ModuleEntry>,
    /// What it showed up to 0.2.1, by name: read once, to carry the choices
    /// over, and not written again.
    #[serde(skip_serializing)]
    items: Option<Vec<String>>,
    /// Where it is on its screen: how far across and down the room there is
    /// for it (0 at the left or top, 1 at the right or bottom).
    pub at: (f32, f32),
    /// A point on the screen it was put on (physical pixels), where it
    /// shows, a game played or not; never put anywhere, it shows over the
    /// game's screen.
    pub screen: Option<(i32, i32)>,
    /// Locked where it is: not dragged.
    pub locked: bool,
    /// How clear its glass is: 0, all there (what is behind tinted); 1,
    /// gone (the words alone over what is behind).
    pub clear: f32,
    /// Clicks pass through it (Ctrl held, it takes them).
    pub click_through: bool,
    /// A card or a strip.
    pub layout: crate::ui::overlay::Layout,
    /// A strip's rows (see `ui::overlay::ROWS`).
    pub rows: usize,
    /// How large it is drawn, 1 as designed (0.75 to 2).
    pub size: f32,
    /// Offered once already: the first game told of it.
    pub offered: bool,
}

/// When the overlay shows, as its settings offer it: never, while a game is played, or all the time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OverlayShown {
    Off,
    InGame,
    Always,
}

impl OverlaySettings {
    /// When it shows: all the time over while a game is played (which it is then too).
    pub fn shown(&self) -> OverlayShown {
        match (self.on, self.in_game) {
            (true, _) => OverlayShown::Always,
            (false, true) => OverlayShown::InGame,
            (false, false) => OverlayShown::Off,
        }
    }

    /// Shows it `when`: all the time leaves while a game is played as it was (the overlay shows then either way, and
    /// turned back from all the time it is what it was).
    pub fn show(&mut self, when: OverlayShown) {
        self.on = when == OverlayShown::Always;
        if when != OverlayShown::Always {
            self.in_game = when == OverlayShown::InGame;
        }
        self.offered = true;
    }

    /// What it shows, by item, group by group.
    pub fn chosen(&self) -> Vec<&'static str> {
        crate::ui::overlay::chosen(&self.groups)
    }

    /// How its readings are laid out, on a screen `width` DIPs wide (at
    /// its size).
    pub fn shape(&self, width: f32) -> crate::ui::overlay::Shape {
        crate::ui::overlay::Shape { layout: self.layout, rows: self.rows, width }
    }
}

impl Default for OverlaySettings {
    fn default() -> Self {
        OverlaySettings {
            on: false,
            in_game: false,
            groups: crate::ui::overlay::default_groups(),
            items: None,
            at: (0.0, 0.0),
            screen: None,
            locked: false,
            clear: 0.0,
            click_through: false,
            layout: Default::default(),
            rows: 1,
            size: 1.0,
            offered: false,
        }
    }
}

/// What opens the panel over a fullscreen game, borderless or exclusive.
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OverFullscreen {
    /// Nothing: the game keeps the screen.
    Never,
    /// The shortcut, which is asked for; a push into the edge may be an
    /// accident mid-game.
    #[default]
    Shortcut,
    /// The shortcut and a push into the edge.
    Both,
}

/// How hard the pointer has to push into the edge.
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sensitivity {
    /// Up to 0.2.6, pushing into the edge opened nothing; read as no edge
    /// opening the panel so (see `Settings::pushed`), and not written again.
    Off,
    Light,
    #[default]
    Medium,
    Firm,
}

impl Sensitivity {
    /// Raw mouse counts of outward travel against the edge; none when the
    /// edge opens nothing.
    pub fn pressure(self) -> Option<i32> {
        match self {
            Sensitivity::Off => None,
            Sensitivity::Light => Some(50),
            Sensitivity::Medium => Some(120),
            Sensitivity::Firm => Some(260),
        }
    }
}

/// How large what plays is as the panel slides it out (DIPs, at the panel's size 1), from a side edge and from the
/// top, where the hand has sized it there by its edges; none: as it was designed (see `music::PANEL_SIDE`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MusicRooms {
    pub side: Option<(f32, f32)>,
    pub top: Option<(f32, f32)>,
}

impl MusicRooms {
    /// The room from the top (`top`) or from a side.
    pub fn at(&self, top: bool) -> (f32, f32) {
        match top {
            true => self.top.unwrap_or(crate::ui::music::PANEL_TOP),
            false => self.side.unwrap_or(crate::ui::music::PANEL_SIDE),
        }
    }

    /// The room from the top (`top`) or from a side made `room`.
    pub fn set(&mut self, top: bool, room: (f32, f32)) {
        *if top { &mut self.top } else { &mut self.side } = Some(room);
    }
}

/// Everything the backend acts on, plus the page's own preferences, which it
/// stores without looking inside.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// What each screen edge opens; none, in settings from before (see `carry_over`).
    #[serde(default)]
    pub places: Option<Places>,
    /// Up to 0.3.11, the edge every screen `screens` did not name opened the panel from; whether a pointer pushed
    /// into it opened it, and whether one resting where another screen lies against it did; and the screens whose
    /// edges were chosen, each with those that opened it. Read once (see `carry_over`), not written again.
    #[serde(skip_serializing)]
    edge: Edge,
    #[serde(skip_serializing)]
    pushed: bool,
    #[serde(skip_serializing)]
    seam: bool,
    #[serde(skip_serializing)]
    screens: Vec<ScreenEdges>,
    pub skin: String,
    pub anchor: Anchor,
    /// How many columns the panel's lanes are dealt into; none chosen, as
    /// few as fit the screen's height.
    pub columns: Option<usize>,
    /// How large the panel is drawn, 1 as designed (0.75 to 2).
    pub panel_size: f32,
    pub sensitivity: Sensitivity,
    pub close_delay_ms: u64,
    pub interval_ms: u64,
    /// Keep what shows behind the glass up with what changes there (see
    /// `ui::scene`).
    pub live_backdrop: bool,
    /// The panel, pulled away from its edge, is torn off as a widget; off,
    /// pulling it does nothing.
    pub tear_off: bool,
    /// Ask GitHub once a day whether a newer Glance is out.
    pub check_updates: bool,
    /// The shortcut that opens and closes the panel (Ctrl+Alt+G unless
    /// another is chosen); none, when it is cleared.
    pub shortcut: Option<Shortcut>,
    /// Up to 0.1.8, the shortcut's own switch: off, it reads as cleared.
    /// Read once, not written again.
    #[serde(skip_serializing)]
    hotkey: Option<bool>,
    /// What opens the panel over a game holding the screen in exclusive
    /// fullscreen, which the panel showing sends to the background.
    pub over_fullscreen: OverFullscreen,
    /// Tell from the tray when the CPU or a graphics card stays at or above
    /// the temperature alert.
    pub heat_alert: bool,
    /// Whether the plugin loader in NetEase Cloud Music's folder is the one Glance put there (installing InfLink-rs
    /// for it): only then is it taken back with the plugin.
    pub netease_loader: bool,
    /// The version that last ran: a newer one starting says it was updated.
    pub last_version: Option<String>,
    /// The day Glance first ran here (days since 1970), where it has been
    /// known since: asking its site for a newer version, Glance says how
    /// many weeks ago that was, and no more of when.
    pub began: Option<u64>,
    /// The version that last asked Glance's site for a newer one and was
    /// answered: the next to ask says which it came from, where another.
    pub asked_as: Option<String>,
    pub overlay: OverlaySettings,
    /// See `PanelAt`; none while the panel opens from the edge.
    pub panel_at: Option<PanelAt>,
    /// Pinned open at its edge, along it where this point is (physical px;
    /// its screen's): open so again as Glance starts.
    #[serde(default)]
    pub panel_pinned: Option<(i32, i32)>,
    /// The edge it was pinned at (none, pinned before this was kept: the edge nearest that point).
    #[serde(default)]
    pub panel_pinned_edge: Option<Edge>,
    /// The panels pinned open at their edges, each along its edge where its point is (physical px; its screen's),
    /// and the edge (none, pinned before this was kept: the edge nearest that point): open so again as Glance starts.
    /// (Each edge has a panel of its own: those of several may be pinned at once. Settings that kept one alone, in
    /// `panel_pinned`, are carried over into it.)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub panels_pinned: Vec<((i32, i32), Option<Edge>)>,
    /// Each GPU's number in the module that shows it ("gpu:N"), by what it is (see `GpuInfo::identity`), as it was
    /// first given (see `slot_gpus`).
    #[serde(default)]
    pub gpu_slots: Vec<(String, usize)>,
    /// How large what plays is as the panel slides it out, from a side edge and from the top, as the hand made it.
    pub music_rooms: MusicRooms,
    /// The desktop widgets torn off the panel.
    pub widgets: Vec<WidgetAt>,
    /// The player music is followed from, while it plays (see `media::choose_music`); none: as Glance picks.
    pub music_source: Option<MusicSource>,
    /// The region of Apple Music's catalog searched for tracks and their metadata (none: as picked from the track's script).
    #[serde(default)]
    pub apple_storefront: Option<String>,
    /// The station's (see `station`).
    #[serde(default)]
    pub station: StationSettings,
    /// Folders whose files Recent files does not list (see `station::recent`).
    #[serde(default)]
    pub recent_hidden: Vec<PathBuf>,
    /// Drives whose files Recent files does not list (see `HiddenDrive`).
    #[serde(default)]
    pub recent_hidden_drives: Vec<HiddenDrive>,
    /// Whether Recent files lists only the files in `recent_folders` (and in the folders in them), the folders kept
    /// while it does not.
    #[serde(default)]
    pub recent_only: bool,
    #[serde(default)]
    pub recent_folders: Vec<PathBuf>,
    /// What is kept at hand on the workbench, in its order (see `station::apps`).
    #[serde(default)]
    pub apps: Vec<crate::station::apps::App>,
    /// The notes' folder, where another than `Documents\Glance 便签` was chosen (see `notes`).
    #[serde(default)]
    pub notes_folder: Option<PathBuf>,
    /// How the notes show (see `NotesView`).
    #[serde(default)]
    pub notes_view: NotesView,
    /// Whether notes show their Markdown as written, marks and all (see `notes::markdown::Shown`), not as it reads.
    #[serde(default)]
    pub notes_source: bool,
    pub view: serde_json::Value,
}

/// How a page of the station shows what is on it: as cards (a picture, its name and what it is under it), three
/// across, or larger, two across; or a list (a line each). (Larger cards were pictures alone, by that name.)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShelfView {
    #[default]
    Cards,
    #[serde(alias = "pictures")]
    Large,
    List,
}

/// The panel at `edge` of the screen `point` is on pinned there along it at `point`, or (not `pinned`) no longer, in
/// `pinned_panels` (see `Settings::panels_pinned`): that edge's of that screen alone (`same_screen` tells two points
/// on one), the others' as they are (one kept with no edge, from before, is that screen's).
pub fn pin(pinned_panels: &mut Vec<((i32, i32), Option<Edge>)>, point: (i32, i32), edge: Edge, pinned: bool, same_screen: impl Fn((i32, i32), (i32, i32)) -> bool) {
    pinned_panels.retain(|(at, at_edge)| !(at_edge.is_none_or(|at_edge| at_edge == edge) && same_screen(*at, point)));
    if pinned {
        pinned_panels.push((point, Some(edge)));
    }
}

/// A drive whose files Recent files does not list: its volume, as Windows names it whatever letter it is given
/// (`\\?\Volume{…}\`: an external disk plugged in under another letter is still left out, and a disk formatted
/// anew keeps it, where its serial number changes), and its label and letter as last seen, to name it while it is away.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct HiddenDrive {
    pub volume: String,
    pub label: String,
    pub letter: char,
}

/// The station's pages, each shown its own way (see `ShelfView`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum StationPage {
    Shelf,
    Copies,
    Recent,
}

/// How the notes show: as a deck (the last written whole, the others' titles over it), as small cards, or a list.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotesView {
    #[default]
    Deck,
    Cards,
    List,
}

/// How the station keeps what is put on it, and what is copied.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct StationSettings {
    /// What is copied kept in Recently copied.
    pub history: bool,
    /// The files come lately listed (see `station::recent`).
    pub recent: bool,
    /// How what is on it is shown, what was copied, and the files come lately (see `ShelfView`).
    pub view: ShelfView,
    pub copies_view: ShelfView,
    pub recent_view: ShelfView,
    /// How many days what is not pinned stays.
    pub keep_days: u32,
    /// Files put on it kept as a copy too (else only where they are).
    pub always_copy: bool,
    /// What is dragged off it taken off it.
    pub remove_after_drag: bool,
}

impl Default for StationSettings {
    fn default() -> Self {
        StationSettings { history: true, recent: true, view: ShelfView::Cards, copies_view: ShelfView::List, recent_view: ShelfView::List, keep_days: 7, always_copy: false, remove_after_drag: false }
    }
}

/// The most the station keeps of its own (GB): past it, what is oldest and not pinned goes first.
const STATION_MOST_GB: u64 = 2;

impl StationSettings {
    /// What the shelf keeps to (see `station::shelf::Rules`).
    pub fn rules(&self) -> crate::station::shelf::Rules {
        crate::station::shelf::Rules { keep_days: self.keep_days, most_bytes: STATION_MOST_GB << 30 }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            places: Some(Places::default()),
            edge: Edge::default(),
            pushed: true,
            seam: false,
            screens: Vec::new(),
            skin: "paper".into(),
            anchor: Anchor::default(),
            columns: None,
            panel_size: 1.0,
            sensitivity: Sensitivity::default(),
            close_delay_ms: 200,
            interval_ms: 1000,
            live_backdrop: false,
            tear_off: true,
            check_updates: true,
            shortcut: Some(Shortcut::default()),
            hotkey: None,
            over_fullscreen: OverFullscreen::default(),
            heat_alert: false,
            netease_loader: false,
            last_version: None,
            began: None,
            asked_as: None,
            overlay: OverlaySettings::default(),
            panel_at: None,
            panel_pinned: None,
            panel_pinned_edge: None,
            panels_pinned: Vec::new(),
            gpu_slots: Vec::new(),
            music_rooms: MusicRooms::default(),
            widgets: Vec::new(),
            music_source: None,
            apple_storefront: None,
            station: StationSettings::default(),
            recent_hidden: Vec::new(),
            recent_hidden_drives: Vec::new(),
            recent_only: false,
            recent_folders: Vec::new(),
            apps: Vec::new(),
            notes_folder: None,
            notes_view: NotesView::Deck,
            notes_source: false,
            view: serde_json::Value::Null,
        }
    }
}

impl Settings {
    /// What each screen edge opens.
    pub fn places(&self) -> &Places {
        self.places.as_ref().expect("places carried over as the settings were read")
    }

    /// What each screen edge opens, to be changed.
    pub fn places_mut(&mut self) -> &mut Places {
        self.places.as_mut().expect("places carried over as the settings were read")
    }

    pub fn close_delay(&self) -> Duration {
        Duration::from_millis(self.close_delay_ms)
    }

    pub fn interval(&self) -> Duration {
        Duration::from_millis(self.interval_ms)
    }
}

/// Where Glance keeps its settings: the user's roaming application data,
/// under Glance's identifier (a debug build's, under its own, so it never
/// rewrites an installed Glance's).
#[cfg(windows)]
pub fn config_dir() -> PathBuf {
    own_folder_in(&FOLDERID_RoamingAppData)
}

/// Where Glance keeps what it can fetch again (what it has fetched, so as not to ask again): the user's local
/// application data, which goes with no roaming profile, under Glance's identifier as `config_dir`.
#[cfg(windows)]
pub fn local_dir() -> PathBuf {
    own_folder_in(&FOLDERID_LocalAppData)
}

/// In a test, a folder of this test process's own, where both are (a test never reads or writes the user's: run as
/// a release, as tests are, they are the installed Glance's).
#[cfg(all(windows, test))]
fn own_folder_in(known: &windows::core::GUID) -> PathBuf {
    let which = if *known == FOLDERID_RoamingAppData { "roaming" } else { "local" };
    std::env::temp_dir().join(format!("glance-test-{}", std::process::id())).join(which)
}

#[cfg(all(windows, not(test)))]
fn own_folder_in(known: &windows::core::GUID) -> PathBuf {
    let folder = unsafe { SHGetKnownFolderPath(known, KF_FLAG_DEFAULT, None) }.expect("application data folder");
    let path = PathBuf::from(unsafe { folder.to_string() }.expect("folder path"));
    unsafe { CoTaskMemFree(Some(folder.0 as *const _)) };
    path.join(if cfg!(debug_assertions) { "dev.weiyi.glance.debug" } else { "dev.weiyi.glance" })
}

#[cfg(windows)]
impl Settings {
    /// A missing file means defaults. A file that does not read whole (written by a later version, or edited by
    /// hand) keeps every part of it that does read, the rest as it would be without it (see `salvaged`); what was
    /// left out is noted, and the file as it was kept beside it (`UNREAD`), as the next save writes over it. Numbers
    /// outside what the settings offer are brought within it. Read and written so that Glance, running elevated,
    /// never follows a link someone put in the user's folder (see `elevation::read_in_place`).
    pub fn load(dir: &Path) -> Self {
        let kept = crate::elevation::read_in_place(dir, FILE, MOST);
        // (One there not read at all, as not one Glance wrote (too large, not text, a link): put aside as it is, as the
        // next save writes over it.)
        if kept.is_none() && saved(dir) {
            let aside = crate::elevation::put_aside_in_place(dir, FILE, UNREAD);
            crate::journal::note(match aside {
                Ok(()) => format!("settings: the file not read (not one Glance writes); put aside as {UNREAD}"),
                Err(error) => format!("settings: the file not read (not one Glance writes), nor put aside ({error})"),
            });
        }
        // With none kept, this is where Glance begins here.
        let begins = kept.is_none().then(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs() / 86400));
        let mut settings = match kept {
            None => Settings::default(),
            Some(text) => {
                let (settings, left_out) = read(&text);
                if !left_out.is_empty() {
                    crate::journal::note(format!("settings: read without {} (the file as it was kept as {UNREAD})", left_out.join(", ")));
                    if let Err(error) = crate::elevation::write_in_place(dir, UNREAD, text.as_bytes()) {
                        crate::journal::note(format!("settings: the file as it was not kept ({error})"));
                    }
                }
                settings
            }
        };
        if begins.is_some() {
            settings.began = begins;
        }
        settings.interval_ms = settings.interval_ms.clamp(250, 10_000);
        settings.close_delay_ms = settings.close_delay_ms.min(10_000);
        settings.overlay.clear = settings.overlay.clear.clamp(0.0, 1.0);
        settings.station.keep_days = settings.station.keep_days.clamp(1, 365);
        settings.carry_over();
        settings
    }

    /// Settings from before, in today's terms; a shortcut that cannot be one
    /// (a file not written by Glance) is the default.
    fn carry_over(&mut self) {
        if let Some(items) = self.overlay.items.take() {
            self.overlay.groups = crate::ui::overlay::carried_over(&items);
        }
        crate::ui::overlay::complete(&mut self.overlay.groups);
        if self.sensitivity == Sensitivity::Off {
            self.pushed = false;
            self.sensitivity = Sensitivity::default();
        }
        if self.places.is_none() {
            self.places = Some(self.places_before());
        }
        if let Some(point) = self.panel_pinned.take() {
            self.panels_pinned.push((point, self.panel_pinned_edge.take()));
        }
        if self.hotkey.take() == Some(false) {
            self.shortcut = None;
        }
        if self.shortcut.is_some_and(|shortcut| !shortcut.usable()) {
            self.shortcut = Some(Shortcut::default());
        }
        for widget in &mut self.widgets {
            if let Some(before) = widget.before.take() {
                widget.layout = before;
            }
        }
    }

    /// What each screen edge opens, as settings up to 0.3.13 had the panel open: the readings, at the edges that
    /// opened it (pushed into, or rested on where screens meet).
    fn places_before(&self) -> Places {
        let holds = |edges: &[Edge]| edges.iter().map(|edge| Hold { edge: *edge, part: Part::Monitor }).collect::<Vec<_>>();
        let opened = |screen: &ScreenEdges| {
            let mut edges: Vec<Edge> = screen.edges.clone();
            edges.extend(screen.seams.iter().filter(|edge| !screen.edges.contains(edge)));
            edges
        };
        Places {
            same: self.screens.is_empty(),
            every: if self.pushed || self.seam { holds(&[self.edge]) } else { Vec::new() },
            screens: self.screens.iter().map(|screen| ScreenHolds { screen: screen.screen.clone(), holds: holds(&opened(screen)) }).collect(),
        }
    }

    /// Saved in `dir`, whole or not at all; never larger than Glance reads (see `load`): such settings are not saved.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).unwrap();
        if text.len() as u64 > MOST {
            return Err(std::io::Error::new(std::io::ErrorKind::FileTooLarge, format!("{} bytes, more than Glance reads", text.len())));
        }
        crate::elevation::ensure_folder(dir)?;
        crate::elevation::write_in_place(dir, FILE, text.as_bytes())
    }
}

/// Gives each GPU of `gpus` its number in the module that shows it: the one it was given before, by what it is
/// (`kept`); a GPU not seen before, the lowest no other was given, and is kept with it. (Numbered as Windows listed
/// them, a GPU took another's number, and its settings, as a screen's cable moved from one to the other or an
/// external one came: they were listed in another order.) With none kept yet, each keeps the number counted as before
/// (the order Windows lists them in), that the settings made for it stay its own. Whether any was new.
pub fn slot_gpus(gpus: &mut [crate::reading::GpuInfo], kept: &mut Vec<(String, usize)>) -> bool {
    let first = kept.is_empty();
    let mut new = false;
    for gpu in gpus.iter_mut() {
        if let Some((_, slot)) = kept.iter().find(|(identity, _)| *identity == gpu.identity) {
            gpu.slot = *slot;
            continue;
        }
        if !first {
            gpu.slot = (0..).find(|slot| !kept.iter().any(|(_, taken)| taken == slot)).unwrap();
        }
        kept.push((gpu.identity.clone(), gpu.slot));
        new = true;
    }
    new
}

/// `held` (the settings as they are now) with what `mine` changed from
/// `base` (as they were when `mine` was taken from them): a change made
/// elsewhere meanwhile stands unless `mine` changed the same setting.
/// Compared setting by setting, down to each preference in `view`.
pub fn merged(held: &Settings, base: &Settings, mine: &Settings) -> Settings {
    fn merge(held: &mut serde_json::Value, base: &serde_json::Value, mine: &serde_json::Value) {
        use serde_json::Value;
        match (held, base, mine) {
            (Value::Object(held), Value::Object(base), Value::Object(mine)) => {
                for (key, value) in mine {
                    match (held.get_mut(key), base.get(key)) {
                        (Some(held), Some(base)) => merge(held, base, value),
                        (_, Some(base)) if base == value => {}
                        _ => {
                            held.insert(key.clone(), value.clone());
                        }
                    }
                }
                // Taken out by `mine`.
                for key in base.keys().filter(|key| !mine.contains_key(*key)) {
                    held.remove(key);
                }
            }
            (held, base, mine) => {
                if base != mine {
                    *held = mine.clone();
                }
            }
        }
    }
    let mut value = serde_json::to_value(held).unwrap();
    merge(&mut value, &serde_json::to_value(base).unwrap(), &serde_json::to_value(mine).unwrap());
    // (A number that is none, as one infinite or not a number is written, reads not at all: left as it is held,
    // the rest taken. Read whole, Glance stopped.)
    let (settings, left_out) = read_value(value, held);
    if !left_out.is_empty() {
        #[cfg(windows)]
        crate::journal::note(format!("settings: changed without {}, which could not be kept", left_out.join(", ")));
    }
    settings
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The overlay shows when it is asked to, each way read back as it was set, and offered once it was.
    #[test]
    fn the_overlay_shows_when_asked() {
        for (on, in_game) in [(false, false), (false, true), (true, false), (true, true)] {
            for when in [OverlayShown::Off, OverlayShown::InGame, OverlayShown::Always] {
                let mut overlay = OverlaySettings { on, in_game, ..OverlaySettings::default() };
                overlay.show(when);
                assert_eq!(overlay.shown(), when, "from {on} {in_game}");
                assert!(overlay.offered);
                assert_eq!(overlay.on, when == OverlayShown::Always);
                if when != OverlayShown::Always {
                    assert_eq!(overlay.in_game, when == OverlayShown::InGame);
                }
            }
        }
    }

    /// Each edge's panel is pinned or not on its own: pinned at the left and the top at once, the left unpinned, the
    /// top stays; pinned again along its edge elsewhere, the one before is replaced; another screen's left is its own.
    /// One kept from before with no edge is its screen's and goes as any of that screen's is pinned anew.
    #[test]
    fn pins_each_edges_panel_on_its_own() {
        // (Two screens: x below 1000, and from it.)
        let same = |a: (i32, i32), b: (i32, i32)| (a.0 < 1000) == (b.0 < 1000);
        let mut pinned = Vec::new();
        pin(&mut pinned, (0, 300), Edge::Left, true, same);
        pin(&mut pinned, (500, 0), Edge::Top, true, same);
        assert_eq!(pinned, vec![((0, 300), Some(Edge::Left)), ((500, 0), Some(Edge::Top))]);
        pin(&mut pinned, (1000, 300), Edge::Left, true, same);
        assert_eq!(pinned.len(), 3);
        pin(&mut pinned, (0, 300), Edge::Left, false, same);
        assert_eq!(pinned, vec![((500, 0), Some(Edge::Top)), ((1000, 300), Some(Edge::Left))]);
        pin(&mut pinned, (700, 0), Edge::Top, true, same);
        assert_eq!(pinned, vec![((1000, 300), Some(Edge::Left)), ((700, 0), Some(Edge::Top))]);
        let mut from_before = vec![((0, 300), None)];
        pin(&mut from_before, (0, 500), Edge::Right, true, same);
        assert_eq!(from_before, vec![((0, 500), Some(Edge::Right))]);
    }

    /// A file that does not read whole keeps every part that does: a field of the wrong kind and a widget that does
    /// not read leave the others, and only they are named. (Before, every setting went back to its default, and the
    /// next save wrote over the file.)
    #[test]
    fn a_file_that_does_not_read_whole_keeps_what_does() {
        let widget = |x: i32| serde_json::json!({ "at": [x, 0], "layout": { "form": "music", "scale": 1.0, "extra": [0.0, 0.0] } });
        let mut broken = widget(30);
        broken["layout"]["scale"] = serde_json::json!("large");
        let text = serde_json::json!({
            "skin": "glass",
            "interval_ms": "fast",
            "panel_size": 1.5,
            "widgets": [widget(10), broken, widget(20)],
        })
        .to_string();
        // (Read whole, as it was before, it reads not at all: every setting went back to its default.)
        assert!(serde_json::from_str::<Settings>(&text).is_err());
        let (settings, left_out) = read(&text);
        assert_eq!((settings.skin.as_str(), settings.panel_size), ("glass", 1.5));
        assert_eq!(settings.interval_ms, Settings::default().interval_ms);
        assert_eq!(settings.widgets.iter().map(|widget| widget.at.0).collect::<Vec<_>>(), vec![10, 20]);
        assert_eq!(left_out, vec!["interval_ms".to_string(), "widgets[1]".to_string()]);
        // A file that reads whole names nothing.
        let (_, none) = read(&serde_json::to_string(&settings).unwrap());
        assert!(none.is_empty(), "{none:?}");
    }

    /// Each GPU keeps its number by what it is, listed in whatever order; the first time, each keeps the number
    /// counted from Windows' order; one new takes the lowest free.
    #[test]
    fn a_gpu_keeps_its_number_whatever_order_it_comes_in() {
        let gpu = |identity: &str, slot| crate::reading::GpuInfo { slot, identity: identity.into(), name: String::new(), mem_total: 0, shared_total: 0 };
        let slots = |gpus: &[crate::reading::GpuInfo]| gpus.iter().map(|gpu| (gpu.identity.clone(), gpu.slot)).collect::<Vec<_>>();
        let mut kept = Vec::new();
        let mut gpus = [gpu("discrete", 0), gpu("integrated", 2)];
        assert!(slot_gpus(&mut gpus, &mut kept));
        assert_eq!(slots(&gpus), [("discrete".into(), 0), ("integrated".into(), 2)]);
        // Listed the other way round, with an external one: each its own, the new one the lowest free.
        let mut gpus = [gpu("integrated", 0), gpu("external", 1), gpu("discrete", 2)];
        assert!(slot_gpus(&mut gpus, &mut kept));
        assert_eq!(slots(&gpus), [("integrated".into(), 2), ("external".into(), 1), ("discrete".into(), 0)]);
        let mut gpus = [gpu("discrete", 0)];
        assert!(!slot_gpus(&mut gpus, &mut kept));
        assert_eq!(gpus[0].slot, 0);
    }

    /// Read from its file as Glance reads it: numbers outside what the settings offer brought within it; a file that
    /// does not read whole read in part, the file as it was kept beside it.
    #[test]
    #[cfg(windows)]
    fn loads_from_its_file_as_glance_reads_it() {
        let dir = std::env::temp_dir().join(format!("glance-settings-load-{}", std::process::id()));
        crate::elevation::ensure_folder(&dir).unwrap();
        crate::elevation::write_in_place(&dir, FILE, br#"{ "interval_ms": 1, "close_delay_ms": 99999, "skin": "glass" }"#).unwrap();
        let settings = Settings::load(&dir);
        assert_eq!((settings.interval_ms, settings.close_delay_ms, settings.skin.as_str()), (250, 10_000, "glass"));
        assert!(!dir.join(UNREAD).exists());
        let broken = br#"{ "skin": "glass", "interval_ms": "fast" }"#;
        crate::elevation::write_in_place(&dir, FILE, broken).unwrap();
        let settings = Settings::load(&dir);
        assert_eq!(settings.skin, "glass");
        assert_eq!(std::fs::read(dir.join(UNREAD)).unwrap(), broken);
        // One not read at all (too large to be one Glance wrote): put aside, not written over by the next save.
        let large = format!(r#"{{ "skin": "glass", "x": "{}" }}"#, "x".repeat(MOST as usize));
        crate::elevation::write_in_place(&dir, FILE, large.as_bytes()).unwrap();
        let settings = Settings::load(&dir);
        assert_eq!(settings.skin, Settings::default().skin);
        assert!(std::fs::read(dir.join(UNREAD)).unwrap() == large.as_bytes(), "put aside as it was");
        assert!(!saved(&dir));
        // And Glance writes none such.
        let wide = Settings { skin: "x".repeat(MOST as usize), ..Settings::default() };
        assert_eq!(wide.save(&dir).unwrap_err().kind(), std::io::ErrorKind::FileTooLarge);
        assert!(!saved(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// What Recent files lists, from settings kept before it could be told (0.4's: folders not listed alone) and as
    /// it is kept now: every drive and every folder listed but those not listed. The drives left out, and the folders
    /// listed only, read back as they were written (the folders kept while the switch is off).
    #[test]
    fn reads_what_recent_files_lists() {
        let (older, _) = read_value(serde_json::json!({ "recent_hidden": [r"D:\Private"], "station": { "recent": true } }), &Settings::default());
        assert_eq!((older.recent_hidden.clone(), older.recent_hidden_drives.clone(), older.recent_only, older.recent_folders.clone()), (vec![PathBuf::from(r"D:\Private")], Vec::new(), false, Vec::new()));
        let mut now = older;
        now.recent_hidden_drives.push(HiddenDrive { volume: r"\\?\Volume{1b2c}\".into(), label: "Phone backup".into(), letter: 'E' });
        now.recent_folders.push(PathBuf::from(r"D:\Photos"));
        let (again, left_out) = read_value(serde_json::to_value(&now).unwrap(), &Settings::default());
        assert!(left_out.is_empty());
        assert_eq!((again.recent_hidden_drives, again.recent_only, again.recent_folders), (now.recent_hidden_drives, false, vec![PathBuf::from(r"D:\Photos")]));
    }

    /// A change that is not a number (not kept: written as none at all) is left as it is held, the others taken
    /// (Glance stopped).
    #[test]
    fn a_change_not_a_number_is_left_as_held() {
        let held = Settings { panel_size: 1.25, ..Settings::default() };
        let base = held.clone();
        let mine = Settings { panel_size: f32::NAN, skin: "glass".into(), ..base.clone() };
        let settings = merged(&held, &base, &mine);
        assert_eq!((settings.panel_size, settings.skin.as_str()), (1.25, "glass"));
    }

    /// A number too large to be one (infinite as an f32, written as none at all) is left out alone, not the whole file.
    #[test]
    fn a_number_too_large_is_left_out_alone() {
        let (settings, left_out) = read(r#"{ "skin": "glass", "panel_size": 1e39 }"#);
        assert_eq!(settings.skin, "glass");
        assert_eq!(settings.panel_size, Settings::default().panel_size);
        assert_eq!(left_out, vec!["panel_size".to_string()]);
    }

    #[test]
    fn music_rooms_kept_per_edge() {
        let mut settings: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.music_rooms.at(true), crate::ui::music::PANEL_TOP);
        settings.music_rooms.set(false, (300.0, 120.0));
        let read: Settings = serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!((read.music_rooms.at(false), read.music_rooms.at(true)), ((300.0, 120.0), crate::ui::music::PANEL_TOP));
    }

    #[test]
    fn a_change_made_elsewhere_stands() {
        let base = Settings::default();
        // The overlay moved from the overlay, a preference changed from the
        // panel, while the settings window, from `base`, changed the size.
        let mut held = base.clone();
        held.overlay.at = (0.25, 0.75);
        held.view = serde_json::json!({ "language": "zh" });
        let mut mine = base.clone();
        mine.panel_size = 1.5;
        let merged = merged(&held, &base, &mine);
        assert_eq!((merged.overlay.at, merged.panel_size), ((0.25, 0.75), 1.5));
        assert_eq!(merged.view["language"], "zh");
        // The same setting changed in both: the window's, made last, stands.
        mine.overlay.at = (1.0, 0.0);
        assert_eq!(super::merged(&held, &base, &mine).overlay.at, (1.0, 0.0));
    }

    #[test]
    fn takes_only_shortcuts_that_leave_typing_alone() {
        let key = |ctrl, alt, shift, win, key| Shortcut { ctrl, alt, shift, win, key }.usable();
        assert!(Shortcut::default().usable());
        assert!(key(true, false, true, false, u16::from(b'K')));
        assert!(key(false, false, false, true, 0x70));
        // A key alone, or with Shift alone, is typing.
        assert!(!key(false, false, false, false, u16::from(b'K')));
        assert!(!key(false, false, true, false, u16::from(b'K')));
        // Modifiers alone are no shortcut.
        assert!(!key(true, true, false, false, 0x12));
        assert!(!key(true, false, false, false, 0x5B));
    }

    #[test]
    fn each_edge_opens_what_it_holds() {
        use Part::{Monitor, Music, Workbench};
        // At first, the readings at the right edge of every screen.
        let mut places = Places::default();
        assert_eq!(places.part("a", Edge::Right), Some(Monitor));
        assert_eq!(places.lit("b"), Lit::of(&[Edge::Right]));
        // Every screen alike: what one is given, all are.
        places.put("a", Edge::Left, Music, None);
        assert_eq!(places.part("b", Edge::Left), Some(Music));
        // Put on an edge that held something: in its place.
        places.put("a", Edge::Right, Workbench, None);
        assert_eq!(places.part("a", Edge::Right), Some(Workbench));
        assert_eq!(places.holds("a").len(), 2);
        // Moved from another edge onto one that held something: the two change places; onto an empty one, it leaves.
        places.put("a", Edge::Right, Music, Some(("a", Edge::Left)));
        assert_eq!((places.part("a", Edge::Right), places.part("a", Edge::Left)), (Some(Music), Some(Workbench)));
        places.put("a", Edge::Top, Workbench, Some(("a", Edge::Left)));
        assert_eq!((places.part("a", Edge::Top), places.part("a", Edge::Left)), (Some(Workbench), None));
        // Each set apart: one screen changed, the others as they were (one not named, as every screen is).
        places.set_same(false, "a");
        places.take("a", Edge::Top);
        assert_eq!(places.part("a", Edge::Top), None);
        assert_eq!(places.part("b", Edge::Top), Some(Workbench));
        places.put("b", Edge::Left, Monitor, Some(("a", Edge::Right)));
        assert_eq!((places.part("b", Edge::Left), places.part("a", Edge::Right)), (Some(Monitor), None));
        assert!(places.anywhere(Edge::Left));
        assert!(places.shows(Monitor) && places.shows(Music));
        // Alike again, as the screen it was made so from.
        places.set_same(true, "b");
        assert_eq!(places.lit("a"), Lit::of(&[Edge::Left, Edge::Top, Edge::Right]));
        // Opened by hand where no edge opens anything: as the first edge that does.
        assert_eq!(Places { same: true, every: Vec::new(), screens: Vec::new() }.by_hand(), (Edge::Right, Monitor));
        // Opened by hand: from the nearest edge.
        let both = Lit::of(&[Edge::Left, Edge::Right]);
        assert_eq!(both.nearest((0, 0, 1000, 500), (700, 20)), Some(Edge::Right));
        assert_eq!(both.nearest((0, 0, 1000, 500), (300, 20)), Some(Edge::Left));
        assert_eq!(Lit::default().nearest((0, 0, 1000, 500), (300, 20)), None);
    }

    /// A widget made a rail as it was stuck to an edge (up to 0.3.13) shows again what it showed before (stuck now, a
    /// widget keeps what it shows: kept a rail, pulled away it stayed one, and what it showed was lost).
    #[test]
    fn a_widget_made_a_rail_shows_what_it_did() {
        let json = r#"{"widgets":[{"at":[10,20],"layout":{"form":"rail","scale":1.0,"extra":[0.0,0.0]},"stuck":"right","before":{"form":"full-2","scale":1.2,"extra":[4.0,0.0]}}]}"#;
        let mut settings: Settings = serde_json::from_str(json).unwrap();
        settings.carry_over();
        let widget = &settings.widgets[0];
        assert_eq!((widget.layout.form.as_str(), widget.layout.scale, widget.stuck), ("full-2", 1.2, Some(WidgetSide::Right)));
        assert!(!serde_json::to_string(&settings).unwrap().contains("\"before\""));
    }

    #[test]
    fn edges_chosen_before_open_the_readings() {
        let read = |json: &str| {
            let mut settings: Settings = serde_json::from_str(json).unwrap();
            settings.carry_over();
            settings
        };
        // The one edge every screen had.
        let settings = read(r#"{"edge":"left"}"#);
        assert_eq!(settings.places(), &Places { same: true, every: vec![Hold { edge: Edge::Left, part: Part::Monitor }], screens: Vec::new() });
        // Pushing turned off, and screens' seams not: nothing.
        let settings = read(r#"{"edge":"top","sensitivity":"off"}"#);
        assert!(settings.places().every.is_empty());
        assert!(settings.sensitivity == Sensitivity::default());
        // Where screens meet, alone: it opens there.
        let settings = read(r#"{"edge":"top","sensitivity":"off","seam":true}"#);
        assert_eq!(settings.places().part("a", Edge::Top), Some(Part::Monitor));
        // Screens chosen apart, their edges and seams together.
        let settings = read(r#"{"screens":[{"screen":"a","edges":["left"],"seams":["right"]}]}"#);
        assert!(!settings.places().same);
        assert_eq!(settings.places().lit("a"), Lit::of(&[Edge::Left, Edge::Right]));
        assert_eq!(settings.places().lit("b"), Lit::of(&[Edge::Right]));
        // Written as places, the old fields not again; read back as written.
        let written = serde_json::to_string(&settings).unwrap();
        assert!(written.contains("\"places\"") && !written.contains("\"pushed\""));
        assert_eq!(read(&written).places(), settings.places());
        assert!(Settings::default().places().shows(Part::Monitor));
    }

    #[test]
    fn a_number_too_large_is_none_glance_wrote() {
        // Read as infinite, it would be written as nothing, and what was
        // written not read again (see `merged`).
        let settings: Settings = serde_json::from_str(r#"{"overlay":{"clear":1e39}}"#).unwrap();
        assert!(serde_json::to_value(&settings).and_then(serde_json::from_value::<Settings>).is_err());
        let sound = Settings::default();
        assert!(serde_json::to_value(&sound).and_then(serde_json::from_value::<Settings>).is_ok());
    }

    #[test]
    fn settings_from_before_take_the_defaults() {
        let mut settings: Settings = serde_json::from_str(r#"{"edge":"right","hotkey":true}"#).unwrap();
        settings.carry_over();
        assert_eq!(settings.shortcut, Some(Shortcut::default()));
        assert!(settings.over_fullscreen == OverFullscreen::Shortcut);
        assert_eq!(settings.columns, None);
        // The shortcut switched off before is cleared; one cleared stays so,
        // and its switch is not written again.
        let mut settings: Settings = serde_json::from_str(r#"{"hotkey":false}"#).unwrap();
        settings.carry_over();
        assert_eq!(settings.shortcut, None);
        let mut settings: Settings = serde_json::from_str(r#"{"shortcut":null}"#).unwrap();
        settings.carry_over();
        assert_eq!(settings.shortcut, None);
        assert!(!serde_json::to_string(&settings).unwrap().contains("hotkey"));
        // The overlay's readings chosen by name, in groups now; the names
        // not written again.
        let mut settings: Settings = serde_json::from_str(r#"{"overlay":{"items":["gpu","network"],"style":"plate"}}"#).unwrap();
        settings.carry_over();
        assert_eq!(settings.overlay.chosen(), ["gpu", "down", "up"]);
        assert!(!serde_json::to_string(&settings).unwrap().contains("\"items\":[\""));
    }
}
