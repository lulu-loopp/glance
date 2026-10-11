#[cfg(windows)]
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
#[cfg(windows)]
use windows::Win32::System::Com::CoTaskMemFree;
#[cfg(windows)]
use windows::Win32::UI::Shell::{FOLDERID_RoamingAppData, SHGetKnownFolderPath, KF_FLAG_DEFAULT};

#[cfg(windows)]
const FILE: &str = "settings.json";

/// Whether Glance has saved settings in `dir` before.
#[cfg(windows)]
pub fn saved(dir: &Path) -> bool {
    dir.join(FILE).is_file()
}
/// Larger than this, a settings file is not one Glance wrote.
#[cfg(windows)]
const MOST: u64 = 1 << 20;

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

/// The edges of one screen that open the panel: the screen by the name
/// that stays its own (see `screens`); those that open it where they are
/// open, to a pointer pushed into them; and those that open it where
/// another screen lies against them (a seam), to a pointer resting there.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ScreenEdges {
    pub screen: String,
    pub edges: Vec<Edge>,
    #[serde(default)]
    pub seams: Vec<Edge>,
}

/// Which of a screen's edges open the panel.
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
    /// right, bottom); none, when none of its edges opens the panel.
    pub fn nearest(self, monitor: (i32, i32, i32, i32), point: (i32, i32)) -> Option<Edge> {
        let (left, top, right, _) = monitor;
        self.each().min_by_key(|edge| match edge {
            Edge::Left => point.0 - left,
            Edge::Top => point.1 - top,
            Edge::Right => right - point.0,
        })
    }
}

/// How a screen's edges open the panel: where they are open, and where
/// another screen lies against them.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Opens {
    pub edges: Lit,
    pub seams: Lit,
}

impl Opens {
    /// The edges that open the panel one way or the other.
    pub fn any(self) -> Lit {
        Lit { left: self.edges.left || self.seams.left, top: self.edges.top || self.seams.top, right: self.edges.right || self.seams.right }
    }
}

/// How the edges of a screen the settings do not name open the panel (one
/// not seen before, and all of them up to 0.2.6): from one `edge`, where
/// the pointer stops at it if `pushed`, and where another screen lies
/// against it if `seam`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rest {
    pub edge: Edge,
    pub pushed: bool,
    pub seam: bool,
}

/// How the edges of the screen named `screen` open the panel, as `kept`
/// has it; as `rest` has it, for a screen it does not name.
pub fn lit(kept: &[ScreenEdges], rest: Rest, screen: &str) -> Opens {
    let one = |on: bool| if on { Lit::of(&[rest.edge]) } else { Lit::default() };
    match kept.iter().find(|entry| entry.screen == screen) {
        Some(entry) => Opens { edges: Lit::of(&entry.edges), seams: Lit::of(&entry.seams) },
        None => Opens { edges: one(rest.pushed), seams: one(rest.seam) },
    }
}

/// Whether `edge` of any screen opens the panel where another screen lies
/// against it, as `kept` and `rest` have it.
pub fn seam_anywhere(kept: &[ScreenEdges], rest: Rest, edge: Edge) -> bool {
    (rest.seam && rest.edge == edge) || kept.iter().any(|entry| entry.seams.contains(&edge))
}

/// Turns `edge` of the screen named `screen` on, or off, in `kept`: where
/// it is open, or (`on_seam`) where another screen lies against it.
pub fn flip_edge(kept: &mut Vec<ScreenEdges>, rest: Rest, screen: &str, edge: Edge, on_seam: bool) {
    let now = lit(kept, rest, screen);
    let flipped = |lit: Lit, flip: bool| EDGES.into_iter().filter(|e| lit.has(*e) != (flip && *e == edge)).collect();
    let (edges, seams) = (flipped(now.edges, !on_seam), flipped(now.seams, on_seam));
    match kept.iter_mut().find(|entry| entry.screen == screen) {
        Some(entry) => (entry.edges, entry.seams) = (edges, seams),
        None => kept.push(ScreenEdges { screen: screen.to_string(), edges, seams }),
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
}

/// A desktop widget as it is kept: its surface's corner on the screen
/// (physical px), what it shows, whether it is pinned (not moved or sized),
/// lets clicks through and shows only while a game runs, the side of its
/// screen it is stuck to, if it is, and what it showed before it was.
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
    #[serde(default)]
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
    /// How clear its glass is: 0, all there (what is behind blurred and
    /// tinted); 1, gone (the words alone over what is behind).
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

impl OverlaySettings {
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

/// Everything the backend acts on, plus the page's own preferences, which it
/// stores without looking inside.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The edge of a screen that opens the panel, for every screen
    /// `screens` does not name (see `Rest`).
    pub edge: Edge,
    /// Whether that edge opens the panel to a pointer pushed into it.
    pub pushed: bool,
    /// The screens whose edges were chosen, each with those that open the
    /// panel.
    pub screens: Vec<ScreenEdges>,
    pub skin: String,
    pub anchor: Anchor,
    /// How many columns the panel's lanes are dealt into; none chosen, as
    /// few as fit the screen's height.
    pub columns: Option<usize>,
    /// How large the panel is drawn, 1 as designed (0.75 to 2).
    pub panel_size: f32,
    pub sensitivity: Sensitivity,
    /// Whether that edge opens the panel where another screen lies against
    /// it too (a seam), to a pointer resting there: for every screen
    /// `screens` does not name, as `edge` is.
    pub seam: bool,
    pub close_delay_ms: u64,
    pub interval_ms: u64,
    /// Refresh the desktop behind the glass while the panel is open. The
    /// panel then hides itself from every screen capture, screenshots too.
    pub live_backdrop: bool,
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
    /// The version that last ran: a newer one starting says it was updated.
    pub last_version: Option<String>,
    pub overlay: OverlaySettings,
    /// See `PanelAt`; none while the panel opens from the edge.
    pub panel_at: Option<PanelAt>,
    /// Pinned open at its edge, along it where this point is (physical px;
    /// its screen's): open so again as Glance starts.
    #[serde(default)]
    pub panel_pinned: Option<(i32, i32)>,
    /// The desktop widgets torn off the panel.
    pub widgets: Vec<WidgetAt>,
    pub view: serde_json::Value,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            edge: Edge::default(),
            pushed: true,
            screens: Vec::new(),
            skin: "paper".into(),
            anchor: Anchor::default(),
            columns: None,
            panel_size: 1.0,
            sensitivity: Sensitivity::default(),
            seam: false,
            close_delay_ms: 200,
            interval_ms: 1000,
            live_backdrop: false,
            check_updates: true,
            shortcut: Some(Shortcut::default()),
            hotkey: None,
            over_fullscreen: OverFullscreen::default(),
            heat_alert: false,
            last_version: None,
            overlay: OverlaySettings::default(),
            panel_at: None,
            panel_pinned: None,
            widgets: Vec::new(),
            view: serde_json::Value::Null,
        }
    }
}

impl Settings {
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
    let roaming = unsafe { SHGetKnownFolderPath(&FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, None) }.expect("application data folder");
    let path = PathBuf::from(unsafe { roaming.to_string() }.expect("folder path"));
    unsafe { CoTaskMemFree(Some(roaming.0 as *const _)) };
    path.join(if cfg!(debug_assertions) { "dev.weiyi.glance.debug" } else { "dev.weiyi.glance" })
}

#[cfg(windows)]
impl Settings {
    /// A missing or hand-edited file that no longer parses means defaults;
    /// numbers outside what the settings offer are brought within it.
    /// Read and written so that Glance, running elevated, never follows a
    /// link someone put in the user's folder (see `elevation::read_in_place`).
    pub fn load(dir: &Path) -> Self {
        let mut settings: Settings =
            crate::elevation::read_in_place(dir, FILE, MOST).and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();
        // A number too large to be one (it reads as infinite, and is written
        // as none at all): not a file Glance wrote.
        if serde_json::to_value(&settings).and_then(serde_json::from_value::<Settings>).is_err() {
            settings = Settings::default();
        }
        settings.interval_ms = settings.interval_ms.clamp(250, 10_000);
        settings.close_delay_ms = settings.close_delay_ms.min(10_000);
        settings.overlay.clear = settings.overlay.clear.clamp(0.0, 1.0);
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
        if self.hotkey.take() == Some(false) {
            self.shortcut = None;
        }
        if self.shortcut.is_some_and(|shortcut| !shortcut.usable()) {
            self.shortcut = Some(Shortcut::default());
        }
    }

    /// How the edges of a screen `screens` does not name open the panel.
    pub fn rest(&self) -> Rest {
        Rest { edge: self.edge, pushed: self.pushed, seam: self.seam }
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        crate::elevation::ensure_folder(dir)?;
        crate::elevation::write_in_place(dir, FILE, serde_json::to_string_pretty(self).unwrap().as_bytes())
    }
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
    serde_json::from_value(value).expect("settings merged from settings")
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_screen_opens_the_panel_from_the_edges_chosen_for_it() {
        // None chosen for it: the one edge every screen had, pushed into
        // and where another screen lies against it as that was asked for.
        let mut kept = Vec::new();
        let rest = Rest { edge: Edge::Left, pushed: true, seam: false };
        let only = |edges: &[Edge]| Opens { edges: Lit::of(edges), seams: Lit::default() };
        assert_eq!(lit(&kept, rest, "a"), only(&[Edge::Left]));
        assert_eq!(lit(&kept, Rest { seam: true, ..rest }, "a"), Opens { edges: Lit::of(&[Edge::Left]), seams: Lit::of(&[Edge::Left]) });
        assert_eq!(lit(&kept, Rest { pushed: false, seam: true, ..rest }, "a"), Opens { edges: Lit::default(), seams: Lit::of(&[Edge::Left]) });
        // One more turned on, the first turned off, the last too: none.
        flip_edge(&mut kept, rest, "a", Edge::Top, false);
        assert_eq!(lit(&kept, rest, "a"), only(&[Edge::Left, Edge::Top]));
        flip_edge(&mut kept, rest, "a", Edge::Left, false);
        flip_edge(&mut kept, rest, "a", Edge::Top, false);
        assert_eq!(lit(&kept, rest, "a"), Opens::default());
        assert_eq!(kept.len(), 1);
        // Where another screen lies against an edge is turned on apart.
        flip_edge(&mut kept, rest, "a", Edge::Right, true);
        assert_eq!(lit(&kept, rest, "a"), Opens { edges: Lit::default(), seams: Lit::of(&[Edge::Right]) });
        assert_eq!(lit(&kept, rest, "a").any(), Lit::of(&[Edge::Right]));
        // Another screen is as it was.
        assert_eq!(lit(&kept, rest, "b"), only(&[Edge::Left]));
        // A screen named before seams were chosen apart has none.
        let before: Vec<ScreenEdges> = serde_json::from_str(r#"[{"screen":"c","edges":["top"]}]"#).unwrap();
        assert_eq!(lit(&before, Rest { seam: true, ..rest }, "c"), only(&[Edge::Top]));
        // Opened by hand: from the nearest of them.
        let both = Lit::of(&[Edge::Left, Edge::Right]);
        assert_eq!(both.nearest((0, 0, 1000, 500), (700, 20)), Some(Edge::Right));
        assert_eq!(both.nearest((0, 0, 1000, 500), (300, 20)), Some(Edge::Left));
        assert_eq!(Lit::default().nearest((0, 0, 1000, 500), (300, 20)), None);
        // Pushing turned off before: no edge opens the panel so, where
        // screens meet as it was; how hard to push, as it is at first.
        let mut settings: Settings = serde_json::from_str(r#"{"edge":"top","sensitivity":"off","seam":true}"#).unwrap();
        settings.carry_over();
        assert_eq!(lit(&settings.screens, settings.rest(), "a"), Opens { edges: Lit::default(), seams: Lit::of(&[Edge::Top]) });
        assert!(settings.sensitivity == Sensitivity::default());
        assert!(Settings::default().pushed);
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
