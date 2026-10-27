//! The panel's content: each module as a lane of blocks, laid out in
//! columns, and drawn for a sample and the history before it.

use std::hash::{Hash, Hasher};

use super::canvas::{Align, Canvas, Color, Fill, Font, Point};
use super::icons::Icon;
use super::prefs::{Prefs, ProcessSort};
use super::seen::Seen;
use super::text::{self, Lang};
use super::theme::{Ink, Skin, Theme};
use crate::settings::ShelfView;
use crate::reading::{GpuLimit, ProcessSample, Sample, StaticInfo};

/// A column's width (DIPs), whatever the skin.
pub const COLUMN_WIDTH: f32 = 356.0;
/// The label column of a lane, and the gap after it.
const LABEL: f32 = 96.0;
const LABEL_GAP: f32 = 12.0;
const HEAD: f32 = 17.0;
const HEAD_GAP: f32 = 8.0;
/// How far below its top a lane's head reaches (its title row, and the room under it), past its padding: what a lane is
/// taken by, to be torn off alone.
pub const HEAD_BAND: f32 = HEAD + HEAD_GAP;
const PLOT: f32 = 40.0;
const RATE_PLOT: f32 = 36.0;
/// Rate charts start this much further right, past the rates beside them.
const RATE_INDENT: f32 = 26.0;
const LINE: f32 = 16.0;
/// The sound and what plays: the room over each, and how tall what is pressed there is.
const MEDIA_GAP: f32 = 8.0;
const MEDIA_ROW: f32 = 24.0;

/// How far the lyrics have glid to the line sung now (0 just come to it, 1 there), and what they glide from: the line
/// sung before.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Glide {
    pub done: f32,
    pub was: String,
}

impl Hash for Glide {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.done.to_bits(), &self.was).hash(state);
    }
}

thread_local! {
    /// Whether a line sung was drawn on this thread still gliding into its place (see `gliding_drawn`).
    static GLIDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The notes stuck on the desktop written in where they are now, by their names (see `written_in_place`).
    static IN_PLACE: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The note `name` stuck on the desktop written in where it is (`written`), or no longer: while it is, its card is
/// drawn by what writes it (see `ui::note::Writing::paint`), not here, its words where they are written.
pub fn written_in_place(name: &str, written: bool) {
    IN_PLACE.with_borrow_mut(|names| {
        names.retain(|kept| kept != name);
        if written {
            names.push(name.to_string());
        }
    });
}

/// Whether what was drawn on this thread since this was last asked held a line sung still gliding into its place
/// (to be drawn again at the next frame): asked before a window draws (it forgets), and after. Only a window that
/// shows the line is drawn as often as it glides.
pub fn gliding_drawn() -> bool {
    GLIDING.with(|gliding| gliding.replace(false))
}

/// The glide to the line sung now, as far as it has gone (eased: quick, then settling).
fn glide_of(heard: Option<&crate::playing::Now>) -> Glide {
    use crate::playing::Sung;
    let Some(Sung::Lines { lines, now: Some(now), since }) = heard.and_then(|heard| heard.sung.as_ref()) else {
        return Glide { done: 1.0, ..Default::default() };
    };
    let t = (since.elapsed().as_secs_f32() / crate::playing::GLIDE.as_secs_f32()).min(1.0);
    let text = |at: Option<usize>| at.and_then(|at| lines.get(at)).map_or(String::new(), |line| line.text.clone());
    Glide { done: 1.0 - (1.0 - t).powi(3), was: text(now.checked_sub(1)) }
}

/// What is said of what plays (`heard`): its title (else its player's name, else that nothing plays), and under it
/// whether it plays, by whom and in which player.
/// The music, while what plays is it (a video playing meanwhile is not).
fn playing_music() -> Option<crate::playing::Now> {
    crate::playing::music().filter(|music| crate::playing::now().is_some_and(|now| now.title == music.title && now.app == music.app))
}

/// What of what plays the time's lane would show now: the sound's level, what plays, its track, its line sung (see
/// `Seen::note_playing`).
pub fn playing_rows() -> (bool, bool, bool, bool) {
    let Some(heard) = crate::playing::now() else { return (false, false, false, false) };
    let music = playing_music();
    (heard.volume.is_some(), true, heard.timeline.is_some(), music.is_some() && sung_now(music.as_ref(), Lang::En, false).is_some())
}

fn said_of(heard: &crate::playing::Now, lang: Lang) -> (String, String) {
    let nothing = heard.title.is_empty() && heard.app.is_empty();
    let title = match (heard.title.is_empty(), nothing) {
        (_, true) => lang.pick("没有正在播放的内容", "Nothing is playing").to_string(),
        (true, false) => heard.app.clone(),
        (false, _) => heard.title.clone(),
    };
    let state = if heard.playing { lang.pick("正在播放", "Playing") } else { lang.pick("已暂停", "Paused") };
    let about = if nothing { String::new() } else { [state, heard.artist.as_str(), if heard.title.is_empty() { "" } else { heard.app.as_str() }].into_iter().filter(|one| !one.is_empty()).collect::<Vec<_>>().join(" · ") };
    (title, about)
}

/// What the lyrics show for what plays now (`heard`): the line sung now and under it its translation (with
/// `translated`) or the next; else why there are none, and whether that is it (`said`); nothing where none were found.
fn sung_now(heard: Option<&crate::playing::Now>, lang: Lang, translated: bool) -> Option<(String, String, bool)> {
    use crate::playing::Sung;
    let said = |line: &str, under: &str| Some((line.to_string(), under.to_string(), true));
    let Some(sung) = heard.and_then(|heard| heard.sung.as_ref()) else {
        return said(lang.pick("没有正在播放的歌曲", "Nothing is playing"), "");
    };
    match sung {
        Sung::Untimed { netease: true } => said(lang.pick("网易云音乐没有提供播放进度", "NetEase Cloud Music does not say how far in it is"), lang.pick("可在设置的“音乐”页安装 InfLink-rs 插件以显示歌词", "Install the InfLink-rs plugin on the Music page of Settings to show lyrics")),
        Sung::Untimed { netease: false } => said(lang.pick("这个播放器没有提供播放进度", "This player does not say how far in it is"), lang.pick("无法跟着播放显示歌词", "so no line can be shown as it is sung")),
        Sung::Asking => said(lang.pick("正在查找歌词…", "Finding the lyrics…"), ""),
        Sung::Unknown => None,
        Sung::Instrumental => said(lang.pick("纯音乐", "Instrumental"), ""),
        Sung::Lines { lines, now, .. } => {
            let line = now.map_or(String::new(), |now| sung_line(&lines[now].text));
            let translation = now.and_then(|now| lines[now].translation.clone()).filter(|_| translated);
            let next = lines.get(now.map_or(0, |now| now + 1)).map(|line| line.text.clone()).unwrap_or_default();
            Some((line, translation.unwrap_or(next), false))
        }
    }
}

/// A line as it is shown: one with nothing sung (the music between lines), three dots.
fn sung_line(text: &str) -> String {
    if text.is_empty() { "· · ·".to_string() } else { text.to_string() }
}

const FACT_GAP: f32 = 3.0;
pub const TABLE_ROW: f32 = 22.0;
/// The most processes a lane past `Detail::Full` lists.
const COMPACT_PROCESSES: usize = 3;
/// The settings button in the bar.
const BUTTON: f32 = 32.0;
/// Between two of the bar's buttons, so that two lit side by side stay two.
const BUTTON_GAP: f32 = 4.0;
/// Rates below this full scale are drawn against it, so idle chatter stays low.
const MIN_RATE_SCALE: f64 = 10.0 * 1024.0;
/// The frame time chart's least full scale, in milliseconds.
const MIN_FRAME_SCALE: f64 = 10.0;

/// The name `label` of a row that opens something (drawn at `left`, `top`),
/// the pointer on it, marked as its skin marks what opens: chart paper
/// underlines it in ink, as a link on paper is; Windows 11 sets it in the
/// accent (by the caller), as its links are; glass, already lit along its
/// rims, leaves it bright and no more. And after it, a folder: what a press opens. The folder is
/// centred on the name's ink, not its line (a name of capitals and a colon
/// has no descender, and sits high in its line).
fn opens_marked(frame: &dyn Canvas, theme: &Theme, label: &str, left: f32, top: f32) {
    let font = theme.small;
    let (ink_top, ink_bottom) = frame.ink(label, font);
    let wide = frame.measure(label, font);
    match theme.skin {
        Skin::Paper => frame.fill(theme.text, left, top + ink_bottom + 1.5, wide, 1.0),
        Skin::Glass | Skin::Fluent => {}
    }
    let middle = top + (ink_top + ink_bottom) / 2.0;
    let ink = if theme.skin == Skin::Fluent { super::theme::accent(theme.dark) } else { theme.text2 };
    frame.icon(Icon::Folder, Point { x: left + wide + 5.0 + 6.0, y: middle }, 12.0, ink);
}

/// What a click or a wheel turn on the panel lands on.
#[derive(Clone, Copy, PartialEq, Hash, Debug)]
pub enum Hit {
    Settings,
    Pin,
    /// The music shown over the whole of its screen.
    Full,
    Sort(ProcessSort),
    /// A list scrolled through (the processes', the station's), and how far it scrolls (DIPs, as `f32::to_bits`).
    List(List, u32),
    /// A list turned through by pages (see `PAGE`), as far as it turns; and the way to its next page.
    Pages(List, u32),
    NextPage(List, u32),
    /// The overlay's switch.
    Overlay,
    /// The game button: the window in front taken for a game, or games turned off (see `presents::GameMode`).
    Game,
    /// A thread's cell in the CPU's grid: its clock and load shown while the
    /// pointer is on it.
    Thread(u16),
    /// What is followed (see `playing::Follow`) paused, played or skipped, or the sound muted; and the sound's
    /// track, pressed where along it the sound is to be.
    Media(crate::playing::Key, crate::playing::Follow),
    Volume,
    /// The sound's track standing up (over its key, in what plays): pressed and dragged up and down it.
    VolumeUp,
    /// The track of what is followed, pressed (and dragged along) where in it it is to play from.
    Seek(crate::playing::Follow),
    /// A drive's row in the storage lane: the drive opened, by its letter (not its place among the drives seen: a
    /// drive put in or taken out between the drawing and the press would move another into it).
    Drive(char),
    /// A line of the lyrics turned to by the wheel (by its place among them): played from its start.
    Line(u16),
    /// A thing on the station, by its number (see `station::shelf::Item`); something copied lately, by its.
    Shelf(u64),
    Copied(u64),
    /// A file come lately, by its number on its drive, or files come together, as the page's filter shows them (see
    /// `station::recent::Entry`).
    Recent(crate::station::recent::Entry),
    /// What is kept at hand, by its place (see `station::apps`); the place to add more; where something dragged may
    /// be dropped to be kept at hand.
    App(u64),
    /// While one kept at hand is held and dragged: the place among them the pointer is at (where it would go; these
    /// stand still as the others move to make room, so that what the pointer is over is not moved from under it).
    AppSlot(u16),
    AddApp,
    AppsArea,
    /// A note, by its place among them (see `notes::shown`): opened beside; a new note; the menu of how they show.
    Note(u64),
    /// A note's words where they are written in where they show (a note stuck on the desktop's, by its number: see
    /// `widget`): pressed, written in there.
    Words(u64),
    NewNote,
    /// The notes shown so (one of their views, chosen in their head).
    NotesShow(crate::settings::NotesView),
    /// One of the station's pages chosen (see `Tab`; Recent files', with which of them it shows); its menu (how what is
    /// on the page shows, and the rest).
    Tab(Tab),
    /// The station's pages as one, where their names do not fit side by side: its menu of them.
    StationPages,
    StationView,
    /// Where a station is: what is dragged there may be dropped on it (see `station::run::shows_at`).
    Station,
    /// The thing whose box was put just before it, its name shown shortened there (see `named`).
    Shortened,
}

impl Hit {
    /// Whether a press on it does something of its own (and so moves nothing): the process list and the threads'
    /// cells only scroll and show, under the wheel and the pointer.
    pub fn takes_press(self) -> bool {
        !matches!(self, Hit::List(..) | Hit::Thread(_) | Hit::Station | Hit::AppsArea | Hit::Shortened | Hit::AppSlot(_))
    }

    /// The track it is, where it is one dragged along.
    pub fn slider(self) -> Option<crate::playing::Slider> {
        match self {
            Hit::Volume | Hit::VolumeUp => Some(crate::playing::Slider::Volume),
            Hit::Seek(follow) => Some(crate::playing::Slider::Seek(follow)),
            _ => None,
        }
    }

    /// Whether it eases as the pointer comes over it and leaves (see `Scene::thumbs`): a slider's thumb, a note's strip
    /// in the deck (lifted from it).
    pub fn eases(self) -> bool {
        self.slider().is_some() || matches!(self, Hit::Note(_))
    }

    /// Where what is dropped on it goes, where something dragged may be dropped on it.
    pub fn lands(self) -> Option<crate::station::run::Lands> {
        match self {
            Hit::Station => Some(crate::station::run::Lands::Station),
            Hit::AppsArea => Some(crate::station::run::Lands::Apps),
            _ => None,
        }
    }

    /// Whether a press on it may drag what it is off (a thing on the station, copied lately, a file come lately: see
    /// `station::drag_off`).
    pub fn drags(self) -> bool {
        matches!(self, Hit::Shelf(_) | Hit::Copied(_) | Hit::Recent(_))
    }

    /// Whether a press on it opens a menu (the station's head's buttons: see `station::head_menu`).
    pub fn opens_menu(self) -> bool {
        matches!(self, Hit::StationView | Hit::StationPages)
    }

    /// Whether it is the sound's: its key, or its slider (the wheel over it turns the sound).
    pub fn of_sound(self) -> bool {
        matches!(self, Hit::Volume | Hit::VolumeUp | Hit::Media(crate::playing::Key::Mute, _))
    }

    /// Whether it is a track standing up (dragged along from its bottom).
    pub fn upright(self) -> bool {
        self == Hit::VolumeUp
    }

    /// The list it is, and how far it scrolls, for a hit on one.
    pub fn list(self) -> Option<(List, f32)> {
        match self {
            Hit::List(list, bits) => Some((list, f32::from_bits(bits))),
            _ => None,
        }
    }

    /// The list turned through by pages it is, and how far it turns, for a hit on one.
    pub fn pages(self) -> Option<(List, f32)> {
        match self {
            Hit::Pages(list, bits) | Hit::NextPage(list, bits) => Some((list, f32::from_bits(bits))),
            _ => None,
        }
    }
}

/// A list a lane is scrolled through by the wheel.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum List {
    Processes,
    /// What is on the station, what was copied lately, the files come lately, quick launch's rail, and the notes.
    Shelf,
    Copies,
    Recent,
    Apps,
    Notes,
    /// A note's words on its card stuck on the desktop, while it is not written in (written in, the box it is written
    /// in scrolls them).
    Words,
}

/// How many lists there are.
const LISTS: usize = 7;

/// Which of its pages the station's lane shows: what is on it, what was copied lately, or the files come lately (each
/// of the last two, those its filter takes).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Tab {
    #[default]
    Shelf,
    Copies(crate::station::shelf::Filter),
    Recent(crate::station::recent::Filter),
}

impl Tab {
    /// Which page it is (whatever its filter shows of it).
    pub fn page(self) -> Tab {
        match self {
            Tab::Copies(_) => Tab::Copies(Default::default()),
            Tab::Recent(_) => Tab::Recent(Default::default()),
            page => page,
        }
    }

    /// The page chosen again, with the filter it has while it is the one shown (`now`).
    pub fn again(self, now: Tab) -> Tab {
        if self.page() == now.page() { now } else { self.page() }
    }
}

/// How far each list is scrolled (DIPs), and how far it is going (each frame a little nearer: see `ease`).
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Scrolls {
    at: [f32; LISTS],
    to: [f32; LISTS],
    /// Where each list turned by pages is turned to, before it comes to a page (see `turn_pages`).
    turning: [f32; LISTS],
    /// What each shows (see `lists_show`), as it was scrolled for it.
    shows: [u8; LISTS],
}

impl Scrolls {
    /// How far `list` is scrolled.
    pub fn at(&self, list: List) -> f32 {
        self.at[list as usize]
    }

    /// `list` to be scrolled `by` more (DIPs), within `most`.
    pub fn turn(&mut self, list: List, by: f32, most: f32) {
        let to = &mut self.to[list as usize];
        *to = (*to + by).clamp(0.0, most);
    }

    /// `list`, turned through by pages `page` long, to be turned `by` more (DIPs), within `most`: to the page nearest
    /// where that comes to (half a page turned by a touchpad goes on to the next; turned less, it goes back).
    pub fn turn_pages(&mut self, list: List, by: f32, page: f32, most: f32) {
        let (to, turning) = (&mut self.to[list as usize], &mut self.turning[list as usize]);
        if (*turning - *to).abs() > page / 2.0 {
            *turning = *to;
        }
        *turning = (*turning + by).clamp(0.0, most);
        *to = ((*turning / page).round() * page).clamp(0.0, most);
    }

    /// How far `list` is going (see `at`: how far it is now, on its way there).
    pub fn to(&self, list: List) -> f32 {
        self.to[list as usize]
    }

    /// `list`, turned through by pages, turned to `at` (DIPs, within `most`): sliding there.
    pub fn to_page(&mut self, list: List, at: f32, most: f32) {
        let at = at.clamp(0.0, most);
        (self.to[list as usize], self.turning[list as usize]) = (at, at);
    }

    /// `list` at `at` (DIPs) at once: where what scrolled it last left it (a note's words, where the box they were
    /// written in left them).
    pub fn set(&mut self, list: List, at: f32) {
        let (at, list) = (at.max(0.0), list as usize);
        (self.at[list], self.to[list], self.turning[list]) = (at, at, at);
    }

    /// `list`, turned through by pages, held at `at` (DIPs, within `most`) by a hand swiping it: there at once.
    pub fn hold_pages(&mut self, list: List, at: f32, most: f32) {
        let at = at.clamp(0.0, most);
        let list = list as usize;
        (self.at[list], self.to[list], self.turning[list]) = (at, at, at);
    }

    /// `list`, turned through by pages `page` long, let go by a hand swiping it: on to the page nearest where it is,
    /// or flicked (`lean`: forward above 0, back below), the next page that way.
    pub fn let_go_pages(&mut self, list: List, lean: f32, page: f32, most: f32) {
        let pages = self.at[list as usize] / page;
        let to = if lean > 0.0 {
            pages.floor() + 1.0
        } else if lean < 0.0 {
            pages.ceil() - 1.0
        } else {
            pages.round()
        };
        let to = (to * page).clamp(0.0, most);
        (self.to[list as usize], self.turning[list as usize]) = (to, to);
    }

    /// Each list `dt` seconds nearer where it is going (`ease`: the seconds it takes to come most of the way);
    /// whether one is still on its way.
    pub fn ease(&mut self, dt: f32, ease: f32) -> bool {
        let mut moving = false;
        for (at, to) in self.at.iter_mut().zip(self.to) {
            *at += (to - *at) * (1.0 - std::f32::consts::E.powf(-dt / ease));
            moving |= (to - *at).abs() > 0.05;
        }
        moving
    }

    /// Each list back at its top.
    pub fn reset(&mut self) {
        *self = Scrolls::default();
    }

    /// What each list shows now (see `lists_show`): one showing its things another way, or other things, back at its
    /// top (scrolled as far as it was for what it showed before, it would open part way down).
    pub fn follow(&mut self, shows: [u8; LISTS]) {
        for list in 0..shows.len() {
            if self.shows[list] != shows[list] {
                (self.at[list], self.to[list], self.turning[list]) = (0.0, 0.0, 0.0);
            }
        }
        self.shows = shows;
    }

    /// As each is scrolled, for what is drawn to tell one drawing from another.
    pub fn bits(&self) -> [u32; LISTS] {
        self.at.map(f32::to_bits)
    }
}

/// What each list shows, as far as one is scrolled for it alone (see `Scrolls::follow`): the station's things in their
/// view, what was copied lately and the files come lately under the filters chosen on `tab`, the notes in their view.
pub fn lists_show(tab: Tab) -> [u8; LISTS] {
    let mut shows = [0; LISTS];
    shows[List::Shelf as usize] = crate::station::run::shown().view as u8;
    shows[List::Copies as usize] = if let Tab::Copies(filter) = tab { filter as u8 + 1 } else { 0 };
    shows[List::Recent as usize] = if let Tab::Recent(filter) = tab { filter as u8 + 1 } else { 0 };
    shows[List::Notes as usize] = crate::notes::view() as u8;
    shows
}

/// Which note the notes turned through by pages show (see `keep_note`): its name and its place as last drawn, and the
/// note open beside them as last seen.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct NoteShown {
    kept: Option<(String, usize)>,
    writing: Option<String>,
}

/// The notes turned through by pages (`notes`, in their order: a workbench lane in a shape) kept on the note they show
/// as their order changes (a note written in comes first): the same note shown, where it is now, at once. A note
/// opened beside them (`writing`, by its name: a new one, once it is first kept) turned to, as it is opened.
pub fn keep_note(scrolls: &mut Scrolls, shown: &mut NoteShown, notes: &[crate::notes::Note], writing: Option<&str>) {
    let place = |name: &str| notes.iter().position(|note| note.name == name);
    let most = notes.len().saturating_sub(1) as f32 * PAGE;
    if shown.writing.as_deref() != writing {
        shown.writing = writing.map(str::to_string);
        if let Some(at) = writing.and_then(place) {
            scrolls.to_page(List::Notes, at as f32 * PAGE, most);
        }
    } else if let Some((name, at)) = &shown.kept {
        // (Its place holding another: the order changed, not the page turned, which is to another place.)
        if notes.get(*at).is_none_or(|note| note.name != *name) {
            if let Some(now) = place(name) {
                scrolls.hold_pages(List::Notes, now as f32 * PAGE, most);
            }
        }
    }
    let at = ((scrolls.to(List::Notes) / PAGE).round().max(0.0) as usize).min(notes.len().saturating_sub(1));
    shown.kept = notes.get(at).map(|note| (note.name.clone(), at));
}

/// Sliders' knobs all at rest (for what keeps none of its own).
pub static RESTING: super::thumb::Dots<Hit> = super::thumb::Dots::new();

/// A region of the panel and what lands on it, in DIPs from its corner. A press lands on the first region it is in, as
/// they were drawn: what is on a list is pushed before the list (and the room round it) it is on.
pub type HitBox = (f32, f32, f32, f32, Hit);

/// What the readings are drawn with: the moment, the data, and the choices.
pub struct Scene<'a> {
    pub info: &'a StaticInfo,
    pub prefs: &'a Prefs,
    pub theme: &'a Theme,
    pub lang: Lang,
    pub history: &'a [Sample],
    /// What the machine has shown it can read: the rows and lanes there are.
    pub seen: &'a Seen,
    /// The pen's time: the chart runs a little behind the newest sample, so
    /// the next one has always arrived by the time it is drawn to.
    pub pen_ms: f64,
    /// How far each list is scrolled (see `Scrolls`).
    pub scrolls: Scrolls,
    /// The station's page shown (see `Tab`).
    pub tab: Tab,
    /// What the pointer is over.
    pub hover: Option<Hit>,
    /// Its sliders' knobs' dots, as they ease (see `thumb::Dots`).
    pub thumbs: &'a super::thumb::Dots<Hit>,
    /// The panel is pinned open.
    pub pinned: bool,
    /// The overlay is on.
    pub overlay: bool,
    /// The bar has its buttons: the panel's, not a widget's (a widget has
    /// its own, round it).
    pub buttons: bool,
    /// A newer Glance is out: a dot on the settings button, for whoever
    /// did not see it said.
    pub newer: bool,
    /// The settings asked from its settings' button are open, or have just poured back into it: the button lit, as
    /// large as this (1 as it is, more while it bounces as they land: see `settings_window::bounce`).
    pub settings_lit: Option<f32>,
    /// What plays is drawn apart (a music widget merged into these readings): the music lane is its head, and room
    /// left under it for that (see `music_room`).
    pub music_apart: bool,
}

impl Scene<'_> {
    fn latest(&self) -> &Sample {
        self.history.last().unwrap()
    }
}

/// One line of a chart: its reading in a sample, if the sample has it.
type Series = Box<dyn Fn(&Sample) -> Option<f64>>;

/// A chart: series read from samples, drawn against a full scale.
struct Plot {
    series: Vec<Series>,
    max: f64,
    hot: Option<f64>,
}

impl Plot {
    /// A chart against `max`, or with `None`, against the busiest moment on screen.
    fn new(scene: &Scene, series: Vec<Series>, max: Option<f64>, hot: Option<f64>) -> Self {
        let max = max.unwrap_or_else(|| full_scale(scene, &series));
        Plot { series, max, hot }
    }
}

/// What a pass over the panel draws: everything that holds still between
/// samples, or the charts, which move with every frame.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pass {
    Content,
    Plots,
}

enum Block {
    Head { title: String, device: String, aside: String, aside_hot: bool },
    /// A figure, and its chart unless the module's chart is switched off.
    Readout { figure: String, unit: &'static str, hot: bool, plot: Option<Plot> },
    /// Two rates by name, and their chart unless the module's chart is switched off, with what its top stands for.
    Rates { rows: Vec<(String, String)>, plot: Option<Plot>, scale: String },
    /// A second chart below a readout's, aligned with it, `gap` below it:
    /// what it charts named over its value.
    Trace { label: String, value: String, hot: bool, plot: Plot, gap: f32 },
    /// Each thread's load (0–1), unread where `None`, whether it is hot,
    /// and what the tag over it says while the pointer is on it.
    Threads(Vec<(Option<f32>, bool, String)>),
    /// A labelled bar; `opens`: a click on it opens that drive (its place among those seen: see `Hit::Drive`).
    Meter { label: String, fraction: f32, value: String, hot: bool, gap: f32, opens: Option<char> },
    Facts { rows: Vec<(String, String, bool)>, gap: f32 },
    /// How loud the sound is, on a track it is dragged along, and what
    /// mutes it.
    Sound { level: u8, muted: bool },
    /// What plays (and, under it, by whom and where), with what pauses,
    /// plays and skips it.
    /// `players`: how many tell Windows what they play (the line under the title switches between them).
    Playing { title: String, about: String, playing: bool, players: u8 },
    /// How far into its track what plays is and how long the track is (whole seconds), on a track it is dragged
    /// along where its player lets it be (`seekable`).
    Progress { at: u32, length: u32, seekable: bool },
    /// The line sung now, and under it its translation (or the next line); `said`, where they say why there are
    /// none instead.
    Lyric { line: String, under: String, said: bool, glide: Glide },
    /// A list in five columns: headings that sort it (the processes'), or
    /// not; scrolled through past `visible` rows, or not.
    Table { headings: Vec<(String, Option<ProcessSort>)>, sort: Option<ProcessSort>, rows: Vec<[String; 5]>, visible: usize, scrolls: bool, widths: [f32; 4] },
    /// Room this tall left empty, for what is drawn there apart (see `Scene::music_apart`).
    Room(f32),
    /// The station's head: its pages as tabs (`tab` the one shown; how many things are on it; whether what was copied
    /// lately is a page of it, and the files come lately), and its buttons (how what is on it shows, while that is
    /// shown; the rest).
    Tabs { tab: Tab, count: usize, copies: bool, recent: bool, view: ShelfView },
    /// What is on the station, the newest first, as `shows` lays it out in room `tall` high, scrolled through past
    /// it; `said`, what it says instead of them (nothing on it), or over them (no room left); `over`, something
    /// dragged over it, to be dropped.
    Shelf { tiles: Vec<Tile>, shows: Shows, tall: f32, said: Option<String>, over: bool },
    /// What was copied lately, the newest first, of those `filter` takes (its choices over them), as `view` shows
    /// them, in room `tall` high, scrolled through past it.
    Copies { rows: Vec<Tile>, filter: crate::station::shelf::Filter, view: ShelfView, tall: f32 },
    /// The notes, the last written first, as `view` shows them, in room `tall` high (scrolled through past it); in a
    /// small lane, the last written alone.
    Notes { notes: Vec<NoteCard>, view: crate::settings::NotesView, tall: f32 },
    /// A note stuck on the desktop: its page as it is written in, all its lane is (see `stuck_note_lane`).
    StuckNote(StuckNote),
    /// What is kept at hand (quick launch's), each by its name and what opens it, in `rows` rows (named, as a whole
    /// lane shows them; else one row of their pictures alone), and the place to add more.
    /// (`over`: something dragged over it from elsewhere, to be kept at hand as it is let go.)
    Apps { apps: Vec<(String, String)>, rows: usize, named: bool, over: bool },
    /// The files come lately that `filter` takes, the newest first (those come together as one: see
    /// `station::recent::listed`), as `view` shows them (a list under the day each came), in room `tall` high under the
    /// filter's choices, scrolled through past it, and `nowhere`: where none can come (see `station::recent::nowhere`).
    Recent { rows: Vec<Lately>, filter: crate::station::recent::Filter, view: ShelfView, tall: f32, nowhere: bool },
    /// A workbench lane in a shape of its own: what it holds, drawn in all the room it is given, in the shape that room
    /// is (see `bench_shape`).
    Bench(Bench),
}

/// What a workbench lane holds, drawn in a shape (see `Block::Bench`).
#[derive(Clone, PartialEq, Hash, Debug)]
pub enum Bench {
    /// The station's page shown: its things (each with what a press on it finds), how many, which page.
    Station { tiles: Vec<(Tile, Hit)>, tab: Tab },
    /// The notes, the last written first.
    Notes(Vec<NoteCard>),
    /// What is kept at hand.
    Apps(Vec<(String, String)>),
    /// The time, and the day.
    Time { figure: String, unit: String, date: String },
}

/// The most of a note's lines under its title a workbench lane in a shape shows (as many as its room takes).
const BENCH_LINES: usize = 12;

/// How tall a workbench lane in a shape is laid out (it is drawn in all the room it is given).
const BENCH: f32 = 36.0;

/// The most a workbench lane in a shape is padded by (DIPs): small, it is mostly its things.
const BENCH_PAD: f32 = 8.0;

/// The least room a workbench lane in a shape leaves round what it shows (DIPs; see `paint_bench`).
const BENCH_BREATH: f32 = 12.0;

/// How a workbench lane in a shape is padded (across, over, under): as `theme`'s lanes, at most `BENCH_PAD`.
fn bench_pads(theme: &Theme) -> (f32, f32, f32) {
    (theme.pad_x.min(BENCH_PAD), theme.pad_top.min(BENCH_PAD), theme.pad_bottom.min(BENCH_PAD))
}

/// A note as its card shows it: its title, its first lines, how long ago it was written, its colour, and whether it is
/// held from the panel.
#[derive(Clone, Debug, PartialEq, Hash)]
pub struct NoteCard {
    /// Its number, as a press on it names it (see `notes::number_of`).
    number: u64,
    title: String,
    lines: Vec<String>,
    when: String,
    colour: crate::notes::Colour,
    held: Held,
}

/// A note's card as it is held from the panel: pressed there (pulled off as the pointer goes far enough), drawn pressed
/// in a little; or carried away, the room it left outlined till it is put down elsewhere or comes back to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Held {
    #[default]
    Not,
    Pressed,
    Away,
}

/// The note pressed on the panel to be pulled off it, by its number (see `press_note`).
static PRESSED_NOTE: std::sync::Mutex<Option<u64>> = std::sync::Mutex::new(None);
/// The notes carried off the panel now, by their numbers (see `carry_notes`).
static AWAY_NOTES: std::sync::Mutex<Vec<u64>> = std::sync::Mutex::new(Vec::new());

/// The note pressed on the panel to be pulled off it (its number), or none; whether that changed.
pub fn press_note(number: Option<u64>) -> bool {
    std::mem::replace(&mut *PRESSED_NOTE.lock().unwrap(), number) != number
}

/// The notes carried off the panel now (their numbers: in hand, or going back to it); whether they changed.
pub fn carry_notes(numbers: Vec<u64>) -> bool {
    let mut away = AWAY_NOTES.lock().unwrap();
    let changed = *away != numbers;
    *away = numbers;
    changed
}

/// How the note numbered `number` is held from the panel.
fn held(number: u64) -> Held {
    if AWAY_NOTES.lock().unwrap().contains(&number) {
        Held::Away
    } else if *PRESSED_NOTE.lock().unwrap() == Some(number) {
        Held::Pressed
    } else {
        Held::Not
    }
}

/// What is drawn of a note's card held from the panel in the box `room`, its corners rounded by `radius`: carried away,
/// the room it left, outlined (none: nothing of the card drawn there); pressed, the box drawn in by a little round its
/// middle; else the box.
fn held_room(frame: &dyn Canvas, theme: &Theme, held: Held, (x, y, w, h): (f32, f32, f32, f32), radius: f32) -> Option<(f32, f32, f32, f32)> {
    match held {
        Held::Not => Some((x, y, w, h)),
        Held::Pressed => {
            let (dx, dy) = (w * (1.0 - PRESSED_IN) / 2.0, h * (1.0 - PRESSED_IN) / 2.0);
            Some((x + dx, y + dy, w - 2.0 * dx, h - 2.0 * dy))
        }
        Held::Away => {
            frame.fill_rounded(theme.text.alpha(0.03), x, y, w, h, radius);
            frame.stroke_rounded_dashed(theme.text.alpha(0.22), x, y, w, h, radius, 1.5);
            None
        }
    }
}

/// How large a note's card pressed to be pulled off the panel is drawn, of its box.
const PRESSED_IN: f32 = 0.97;

/// A thing on a station's lane (a thing on it, copied lately, a file come lately) as it is held to be dragged off it:
/// pressed; carried off it by Glance (see `carry`), its room outlined; put back in it, springing back to its size
/// from a little smaller (since when). One at a time, as the pointer holds one.
static PRESSED_TILE: std::sync::Mutex<Option<Hit>> = std::sync::Mutex::new(None);
static AWAY_TILE: std::sync::Mutex<Option<Hit>> = std::sync::Mutex::new(None);
static LANDED_TILE: std::sync::Mutex<Option<(Hit, std::time::Instant)>> = std::sync::Mutex::new(None);

/// How a thing put back in its room springs back to its size (stiffness, damping: see `Spring`; a little past it as
/// it comes, about a third of a second), from how large, and how long it is drawn on for.
const LANDING: (f32, f32) = (400.0, 16.0);
const LANDED_FROM: f32 = 0.94;
const LANDED_FOR: std::time::Duration = std::time::Duration::from_millis(600);

/// How many times how a thing on a station's lane is held has changed: what shows one is drawn again as it does.
static TILE_CHANGES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many times how a thing on a station's lane is held has changed (see `TILE_CHANGES`).
pub fn tile_changes() -> u64 {
    TILE_CHANGES.load(std::sync::atomic::Ordering::Relaxed)
}

fn tile_changed(changed: bool) -> bool {
    if changed {
        TILE_CHANGES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    changed
}

/// The thing on a station's lane pressed to be dragged off it, or none; whether that changed.
pub fn press_tile(hit: Option<Hit>) -> bool {
    tile_changed(std::mem::replace(&mut *PRESSED_TILE.lock().unwrap(), hit) != hit)
}

/// The thing carried off a station's lane, or none (put back as it is said so: see `land_tile`); whether that changed.
pub fn carry_tile(hit: Option<Hit>) -> bool {
    tile_changed(std::mem::replace(&mut *AWAY_TILE.lock().unwrap(), hit) != hit)
}

/// The thing carried off a station's lane put back in its room at `at`: no longer carried, springing back to its size.
pub fn land_tile(hit: Hit, at: std::time::Instant) {
    carry_tile(None);
    *LANDED_TILE.lock().unwrap() = Some((hit, at));
    tile_changed(true);
}

/// Whether a thing put back on a station's lane is springing back to its size still (what shows it is drawn on).
pub fn tile_landing() -> bool {
    LANDED_TILE.lock().unwrap().is_some_and(|(_, at)| at.elapsed() < LANDED_FOR)
}

/// How the things on a station's lane are held as they are drawn now: what was drawn of them serves again while it is
/// the same (see `render::PanelLayers::draw_on`), drawn anew as one is held otherwise and as one springs back.
pub fn tiles_drawn() -> (u64, Option<std::time::Duration>) {
    let landed = *LANDED_TILE.lock().unwrap();
    (tile_changes(), landed.map(|(_, at)| at.elapsed()).filter(|since| *since < LANDED_FOR))
}

/// Held by a test that holds a thing on a station's lane: one at a time, as the pointer holds one.
#[cfg(test)]
pub static HOLDING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The thing carried off a station's lane, if one is.
pub fn away_tile() -> Option<Hit> {
    *AWAY_TILE.lock().unwrap()
}

/// How the thing on a station's lane pressed as `hit` is held to be dragged off it.
fn tile_held(hit: Hit) -> Held {
    if *AWAY_TILE.lock().unwrap() == Some(hit) {
        Held::Away
    } else if *PRESSED_TILE.lock().unwrap() == Some(hit) {
        Held::Pressed
    } else {
        Held::Not
    }
}

/// The box a thing on a station's lane (pressed as `hit`) draws its picture in, of `face`: pressed, drawn in a little;
/// put back lately, as large as it has sprung back to by now, about its middle.
fn tile_face(hit: Hit, face: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    let (x, y, w, h) = face;
    let k = match (tile_held(hit), *LANDED_TILE.lock().unwrap()) {
        (Held::Pressed, _) => PRESSED_IN,
        (_, Some((landed, at))) if landed == hit && at.elapsed() < LANDED_FOR => {
            let mut spring = crate::ui::motion::Spring::at(LANDED_FROM, LANDING.0, LANDING.1);
            spring.target = 1.0;
            spring.step(at.elapsed().as_secs_f32());
            spring.value
        }
        _ => 1.0,
    };
    let (dx, dy) = (w * (1.0 - k) / 2.0, h * (1.0 - k) / 2.0);
    (x + dx, y + dy, w - 2.0 * dx, h - 2.0 * dy)
}

/// A note stuck on the desktop as its page shows it (see `ui::note::paint_stuck`): its number (as a press on it names
/// it), its words, its colour, how long ago it was written, its file's name, and whether it is written in where it is
/// (see `written_in_place`).
#[derive(Clone, Debug, PartialEq, Hash)]
pub struct StuckNote {
    number: u64,
    text: String,
    colour: crate::notes::Colour,
    when: String,
    name: String,
    written: bool,
}

/// A file come lately as its row shows it, or files come together (see `station::recent::listed`): what a press on it
/// finds, the day it came (as the rows are grouped), its name, where it came from and how large it is, how long ago,
/// its picture's file, and how many more files there are with it.
#[derive(Clone, Debug, PartialEq, Hash)]
pub struct Lately {
    entry: crate::station::recent::Entry,
    day: &'static str,
    label: String,
    about: String,
    when: String,
    path: std::path::PathBuf,
    more: usize,
}

/// The folder `path` is in, by its last two names ("Documents\Work").
fn folder_named(path: &std::path::Path) -> String {
    let folders: Vec<String> = path.parent().into_iter().flat_map(|parent| parent.components()).filter_map(|part| match part {
        std::path::Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
        _ => None,
    }).collect();
    folders[folders.len().saturating_sub(2)..].join("\\")
}

impl Lately {
    fn of(came: &crate::station::recent::Came, lang: Lang, now: u64) -> Lately {
        let label = came.path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        // The folder it is in, and where it came from, where it says, and in which folder it is unless it is still in
        // Downloads.
        let folder = folder_named(&came.path);
        let from = match &came.site {
            Some(site) if came.downloaded => format!("{} {site}", lang.pick("来自", "from")),
            Some(site) => format!("{folder} · {} {site}", lang.pick("来自", "from")),
            None => folder,
        };
        Lately { entry: crate::station::recent::Entry::File(came.key), day: text::day_of(came.came_ms, now, lang), label, about: format!("{from} · {}", text::bytes(came.bytes as f64)), when: text::since(came.came_ms, now, lang), path: came.path.clone(), more: 0 }
    }

    /// The files come together `files` (the newest first, all in one folder), as `filter` shows them: how many, and
    /// in which folder (named, and where it is, with how large they are together), when the newest came, and its
    /// picture.
    fn together(files: &[&crate::station::recent::Came], filter: crate::station::recent::Filter, lang: Lang, now: u64) -> Lately {
        let newest = files[0];
        let name = newest.path.parent().and_then(|folder| folder.file_name()).map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let label = format!("{} {} · {name}", files.len(), lang.pick("个文件", "files"));
        let bytes: u64 = files.iter().map(|came| came.bytes).sum();
        Lately {
            entry: crate::station::recent::Entry::Burst(filter, newest.key),
            day: text::day_of(newest.came_ms, now, lang),
            label,
            about: format!("{} · {}", folder_named(&newest.path), text::bytes(bytes as f64)),
            when: text::since(newest.came_ms, now, lang),
            path: newest.path.clone(),
            more: files.len() - 1,
        }
    }

    /// As a card shows it: its picture, its name, and when it came under it.
    fn tile(&self) -> Tile {
        Tile { id: 0, label: self.label.clone(), about: self.when.clone(), face: Face::Picture { path: self.path.clone(), more: self.more }, pinned: false }
    }

    /// The page's rows of `files` (newest first) that `filter` takes (see `station::recent::listed`).
    fn rows(files: &[crate::station::recent::Came], filter: crate::station::recent::Filter, lang: Lang, now: u64) -> Vec<Lately> {
        crate::station::recent::listed(files, filter).iter().map(|row| match row.as_slice() {
            [came] => Lately::of(came, lang, now),
            together => Lately::together(together, filter, lang, now),
        }).collect()
    }
}

/// `name` (the whole of it) shown as `shown` for what `hit` finds: where it was shortened and `hit`'s box was just put
/// in `hits` (it shows), a mark after it that its name is shortened there (see `named`).
fn name_shown(hits: &mut Vec<HitBox>, hit: Hit, name: &str, shown: &str) {
    if shown == name {
        return;
    }
    let Some(&(x, y, w, h, last)) = hits.last() else { return };
    if last != hit {
        return;
    }
    hits.push((x, y, w, h, Hit::Shortened));
}

/// The whole name of what `hit` finds, where `hits` (as last drawn) show it shortened: said under it as the pointer
/// comes over it. (Its name as it is now, in `lang`: of the thing on the station, come lately or kept at hand that it
/// finds. None is kept of the names shown.)
pub fn named(hits: &[HitBox], hit: Hit, lang: Lang) -> Option<String> {
    let shortened = hits.windows(2).any(|pair| pair[0].4 == hit && pair[1].4 == Hit::Shortened && (pair[0].0, pair[0].1, pair[0].2, pair[0].3) == (pair[1].0, pair[1].1, pair[1].2, pair[1].3));
    if !shortened {
        return None;
    }
    match hit {
        Hit::Shelf(id) | Hit::Copied(id) => crate::station::run::shown().items.iter().find(|item| item.id == id).map(|item| Tile::of(item, &crate::station::folder::Folder::own(), lang, 0).label),
        Hit::Recent(crate::station::recent::Entry::File(key)) => crate::station::recent::shown().iter().find(|came| came.key == key).map(|came| Lately::of(came, lang, 0).label),
        Hit::Recent(entry @ crate::station::recent::Entry::Burst(filter, _)) => Lately::rows(&crate::station::recent::shown(), filter, lang, 0).into_iter().find(|row| row.entry == entry).map(|row| row.label),
        Hit::App(number) => crate::station::apps::numbered(number).map(|app| app.name),
        _ => None,
    }
}

/// How the station's things are laid out: as `ShelfView` says, or (a small lane) in one row of pictures, named.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Shows {
    View(ShelfView),
    Row,
}

/// A thing on the station as its tile shows it: its number, its name, what it is and when it came (said under it),
/// its face, and whether it is pinned.
#[derive(Clone, Debug, PartialEq, Hash)]
pub struct Tile {
    id: u64,
    label: String,
    about: String,
    face: Face,
    pinned: bool,
}

/// What a tile shows: a file's picture as Explorer shows it (with how many more files there are with it), the
/// first lines of text, or a web address's mark.
#[derive(Clone, Debug, PartialEq, Hash)]
enum Face {
    Picture { path: std::path::PathBuf, more: usize },
    Text(String),
    Link,
}

impl Tile {
    /// `item` as its tile shows it (its own files in `folder`), `now` (ms since 1970).
    fn of(item: &crate::station::shelf::Item, folder: &crate::station::folder::Folder, lang: Lang, now: u64) -> Tile {
        use crate::station::shelf::What;
        let (label, kind, face) = match &item.what {
            What::Files { paths, kept } => {
                // (A file gone from where it was, by its copy where one is kept. Whether it is gone as last looked at,
                // on a thread of its own: a drive slow to answer, one on the network gone, never holds the drawing.)
                let gone = |path: &std::path::Path| crate::station::thumbs::looked(path) == Some(crate::station::thumbs::Looked::Gone);
                let first = match kept.first().filter(|name| !name.is_empty() && paths.first().is_some_and(|path| gone(path))) {
                    Some(name) => folder.path(name),
                    None => paths.first().cloned().unwrap_or_default(),
                };
                let name = paths.first().and_then(|path| path.file_name()).map_or_else(String::new, |name| name.to_string_lossy().into_owned());
                // (All gone, with no copy kept of any: said so, as nothing of them can be given again.)
                let lost = !paths.is_empty() && paths.iter().enumerate().all(|(at, path)| gone(path) && kept.get(at).is_none_or(|name| name.is_empty()));
                let kind = match (paths.len(), lost) {
                    (1, false) => lang.pick("文件", "File").to_string(),
                    (1, true) => lang.pick("文件已不存在", "File no longer exists").to_string(),
                    (n, false) => format!("{n} {}", lang.pick("个文件", "files")),
                    (n, true) => format!("{n} {}", lang.pick("个文件已不存在", "files no longer exist")),
                };
                (name, kind, Face::Picture { path: first, more: paths.len().saturating_sub(1) })
            }
            What::Text { text } => (
                text.split_whitespace().collect::<Vec<_>>().join(" "),
                format!("{} {}", text.chars().filter(|c| !c.is_whitespace()).count(), lang.pick("字", "characters")),
                Face::Text(text.lines().filter(|line| !line.trim().is_empty()).take(FACE_LINES).collect::<Vec<_>>().join("\n")),
            ),
            What::Link { url } => (url.split("://").nth(1).unwrap_or(url).trim_end_matches('/').to_string(), lang.pick("链接", "Link").into(), Face::Link),
            What::Picture { file } => (lang.pick("图片", "Picture").into(), lang.pick("图片", "Picture").into(), Face::Picture { path: folder.path(file), more: 0 }),
            What::Saved { file, name, words } => (name.clone(), if *words { lang.pick("文字", "Text") } else { lang.pick("文件", "File") }.into(), Face::Picture { path: folder.path(file), more: 0 }),
        };
        let about = format!("{kind} · {}", text::since(item.added_ms, now, lang));
        Tile { id: item.id, label, about, face, pinned: item.pinned }
    }
}

/// How many of a text's lines its tile keeps to show (more than the largest face holds rows, that one left off is
/// known of by the ellipsis on the last).
const FACE_LINES: usize = 16;

/// What a note stuck on the desktop is shown as: a lane of this name and its file's (see `stuck_note_lane`).
pub const STUCK_NOTE_LANE: &str = "note:";

/// How tall the room under a whole notes lane's head is, and a small lane's.
const NOTES_TALL: f32 = 250.0;
const NOTE_SMALL: f32 = 64.0;
/// A note's title strip in the deck, a row of the list, a small card.
const NOTE_STRIP: f32 = 30.0;
const NOTE_ROW: f32 = 40.0;
const NOTE_CARD: f32 = 104.0;
/// The gap between the cards, two across.
const NOTE_CARDS_GAP: f32 = 8.0;
/// How far a note's strip in the deck is lifted under the pointer (DIPs).
const NOTE_LIFT: f32 = 3.0;
/// How many of a note's lines after its title its card in the notes' lane is given.
const NOTE_CARD_LINES: usize = 6;

/// The notes' lane: whole (`full`), its name and how many there are, the menu of how they show and a new note, over
/// them as they show; else the last written alone.
fn notes_lane(full: bool, lang: Lang) -> Vec<Block> {
    let now = crate::station::now_ms();
    let notes: Vec<NoteCard> = crate::notes::shown().iter().map(|note| note_card(note, lang, now, NOTE_CARD_LINES)).collect();
    // (The number alone, as the station's count is.)
    let count = match notes.len() {
        0 => String::new(),
        n => n.to_string(),
    };
    let view = crate::notes::view();
    if full {
        vec![head(lang.pick("便签", "Notes"), count, "", false), Block::Notes { notes, view, tall: NOTES_TALL }]
    } else {
        vec![head(lang.pick("便签", "Notes"), count, "", false), Block::Notes { notes: notes.into_iter().take(1).collect(), view: crate::settings::NotesView::Deck, tall: NOTE_SMALL }]
    }
}

/// `note` as its card shows it, with up to `lines` of its lines after its title.
fn note_card(note: &crate::notes::Note, lang: Lang, now: u64, lines: usize) -> NoteCard {
    let read = crate::notes::markdown::plain_lines(&note.text);
    let title = read.first().map_or(lang.pick("无标题", "Untitled"), String::as_str);
    NoteCard {
        number: crate::notes::number_of(&note.name),
        // (A conflict's copy said to be one: its words, and so its title, are those of the note it was kept apart from.)
        title: format!(
            "{}{}",
            title,
            if crate::notes::is_apart(&note.name) { lang.pick("（冲突副本）", " (conflicted copy)") } else { "" }
        ),
        lines: read.iter().skip(1).take(lines).cloned().collect(),
        when: text::since(note.stamp.written_ms, now, lang),
        colour: note.colour,
        held: held(crate::notes::number_of(&note.name)),
    }
}

/// The note numbered `number` as its card on the panel shows it (in its lane's cards: as many of its lines), in the box
/// (`x`, `y`, `w`, `h`), held by nothing: as it is carried off the panel (see `widget`). Nothing where it is gone.
pub(crate) fn paint_carried_card(frame: &dyn Canvas, scene: &Scene, number: u64, room: (f32, f32, f32, f32)) {
    let shown = crate::notes::shown();
    let Some(note) = shown.iter().find(|note| crate::notes::number_of(&note.name) == number) else { return };
    let card = NoteCard { held: Held::Not, ..note_card(note, scene.lang, crate::station::now_ms(), NOTE_CARD_LINES) };
    paint_note_card(frame, scene, &card, room, false);
}

/// A note stuck on the desktop (`name`, its file's): its page as it is written in (see `ui::note::paint_stuck`), so that
/// written in where it is, it looks as it did. None where the note is gone.
fn stuck_note_lane(name: &str, lang: Lang) -> Option<Vec<Block>> {
    let shown = crate::notes::shown();
    let note = shown.iter().find(|note| note.name == name)?;
    Some(vec![stuck_note(note, lang, crate::station::now_ms())])
}

/// The note `note` stuck on the desktop, as it is drawn (see `stuck_note_lane`) at `now`.
fn stuck_note(note: &crate::notes::Note, lang: Lang, now: u64) -> Block {
    let written = IN_PLACE.with_borrow(|names| names.contains(&note.name));
    Block::StuckNote(StuckNote { number: crate::notes::number_of(&note.name), text: note.text.clone(), colour: note.colour, when: text::since(note.stamp.written_ms, now, lang), name: note.name.clone(), written })
}

/// How large a note stuck on the desktop is made where it is not stuck from its page (DIPs): its page's width beside
/// the panel, room to write in under its head and over its foot.
pub const STUCK_NOTE: (f32, f32) = (360.0, 320.0);

/// The least a note stuck on the desktop is made (DIPs): its head, a few lines, its foot.
pub const STUCK_NOTE_LEAST: (f32, f32) = (220.0, 180.0);

/// A note's colour's mark: its dot (chart paper's, a list's), its band (Windows 11's), its page's.
pub(crate) fn note_colour(colour: crate::notes::Colour) -> Color {
    use crate::notes::Colour;
    match colour {
        Colour::Blue => Color::hex(0x5B8DEF, 1.0),
        Colour::Yellow => Color::hex(0xE8B64A, 1.0),
        Colour::Green => Color::hex(0x4FBF82, 1.0),
        Colour::Pink => Color::hex(0xEF7AA5, 1.0),
        Colour::Purple => Color::hex(0x9B7BF0, 1.0),
        Colour::Grey => Color::hex(0x8E949C, 1.0),
    }
}

/// A note's colour as a pane of coloured glass (glass's cards: the swatches of alexko0421/glass-card, MIT): its tint,
/// laid over what it frosts `GLASS_TINT` strong, and the ink written on it in the light (a deep shade of the tint).
fn note_glass(colour: crate::notes::Colour) -> (Color, Color) {
    use crate::notes::Colour;
    let (tint, ink) = match colour {
        Colour::Blue => (0x3769F5, 0x18428E),
        Colour::Yellow => (0xF2F766, 0x5A5E12),
        Colour::Green => (0x68BD81, 0x1F5E33),
        Colour::Pink => (0xEE556D, 0x881D2D),
        Colour::Purple => (0x9B38EC, 0x4A1478),
        Colour::Grey => (0x3A3F4A, 0x15181E),
    };
    (Color::hex(tint, 1.0), Color::hex(ink, 1.0))
}

/// How strong a glass note's tint is over what it frosts. (In the dark as in the light, over a pane of light of its own:
/// see `note_light`; over the dark sheet alone, its colours went deep and muddy.)
fn glass_tint(_dark: bool) -> f32 {
    0.58
}

/// What a glass note's tint lies over in the dark, that it have the colours it has in the light: a pane of white as
/// much as the light sheet shows under it.
fn note_light(dark: bool) -> Option<Color> {
    dark.then(|| Color::hex(0xFFFFFF, 0.55))
}

/// How blurred what lies under a glass note shows through it (DIPs).
const NOTE_FROST: f32 = 7.0;

/// What is written on a note's card in `theme`: its title's, its words', and the faint (when it was written). On glass
/// the tint's ink (white in the dark); else the skin's own.
pub(crate) fn note_inks(theme: &Theme, colour: crate::notes::Colour) -> (Color, Color, Color) {
    match theme.skin {
        Skin::Glass => {
            let ink = note_glass(colour).1;
            (ink, ink.alpha(0.84), ink.alpha(0.7))
        }
        _ => (theme.text, theme.text2, theme.text3),
    }
}

/// Windows 11's note card's ground: its sheet, so much of the note's colour in it.
fn fluent_note_ground(theme: &Theme, colour: crate::notes::Colour) -> Color {
    let mark = note_colour(colour);
    let sheet = if theme.dark { Color::hex(0x282A2E, 1.0) } else { Color::hex(0xFAFBFC, 1.0) };
    let k = 0.16;
    Color { r: sheet.r + (mark.r - sheet.r) * k, g: sheet.g + (mark.g - sheet.g) * k, b: sheet.b + (mark.b - sheet.b) * k, a: 0.94 }
}

/// What lies under a pane of glass, drawn (see `Canvas::frosted`).
type Under<'a> = &'a dyn Fn(&dyn Canvas);

/// A note's card at (`x`, `y`), `w` × `h`, its corners rounded by `radius`, as its skin draws it, `lit` under the
/// pointer:
/// - glass a pane of the note's coloured glass, frosting what `below` draws (the cards it lies over: none, where it
///   lies over none), a soft white rim round it, its shadow under it; `plain` (drawn as what another frosts) its tint
///   alone, as a pane under another shows through it;
/// - Windows 11 its sheet a little of its colour, ruled round, a band of the colour along its top;
/// - chart paper what lies on its sheet (the station's things' plates), ruled round; its colour a dot by its title
///   (see `note_dot`).
///
/// Its shadow on the sheet, its ground of its colour (see `note_card_fill`), its rim (see `note_card_rim`). `plain`
/// (under another in a deck), on glass its colour alone.
#[allow(clippy::too_many_arguments)]
fn note_card_ground(frame: &dyn Canvas, theme: &Theme, colour: crate::notes::Colour, (x, y, w, h): (f32, f32, f32, f32), radius: f32, lit: bool, plain: bool, below: Option<Under>) {
    let glass = theme.skin == Skin::Glass;
    if !plain {
        let shade = if theme.dark { 2.5 } else { 1.0 };
        match theme.skin {
            Skin::Glass => {
                frame.shadow(Color::hex(0, (if lit { 0.06 } else { 0.035 }) * shade), x, y, w, h, radius, 12.0, 8.0);
                frame.shadow(Color::hex(0, 0.045 * shade), x, y, w, h, radius, 3.0, 2.0);
            }
            // (A soft shadow under it, as Windows 11's cards on a sheet have, and as it needs stuck on the desktop.)
            Skin::Fluent => frame.shadow(Color::hex(0, if theme.dark { 0.3 } else { 0.08 }), x, y, w, h, radius, 4.0, 2.0),
            Skin::Paper => {}
        }
    }
    note_card_fill(frame, theme, colour, (x, y, w, h), radius, below.filter(|_| !plain));
    if !(plain && glass) {
        note_card_rim(frame, theme, colour, (x, y, w, h), radius, lit);
    }
}

/// A note's card's ground: glass's its coloured glass (the desktop frosted under it by `below`), Windows 11's its
/// sheet a little of the colour, the band of it along its top; chart paper's its paper, opaque as an index card is.
fn note_card_fill(frame: &dyn Canvas, theme: &Theme, colour: crate::notes::Colour, (x, y, w, h): (f32, f32, f32, f32), radius: f32, below: Option<Under>) {
    match theme.skin {
        Skin::Glass => {
            if let Some(below) = below {
                frame.frosted(x, y, w, h, radius, NOTE_FROST, below);
            }
            if let Some(light) = note_light(theme.dark) {
                frame.fill_rounded(light, x, y, w, h, radius);
            }
            frame.fill_rounded(note_glass(colour).0.alpha(glass_tint(theme.dark)), x, y, w, h, radius);
        }
        Skin::Fluent => {
            frame.fill_rounded(fluent_note_ground(theme, colour), x, y, w, h, radius);
            frame.clip(x, y, w, 4.0);
            frame.fill_rounded(note_colour(colour), x, y, w, h.min(2.0 * radius + 4.0), radius);
            frame.unclip();
        }
        Skin::Paper => frame.fill_rounded(over(theme.paper, theme.track), x, y, w, h, radius),
    }
}

/// The gap between a lane's pages side by side as they are turned, and between them and its edge (see `page_place`).
const PAGE_GAP: f32 = 10.0;

/// Where a lane's page is drawn, `off` pages from where the lane is turned to (the lane `wide` across): how large (of
/// the lane), how far along it its left edge is, and how much of it shows. Each page whole within the lane all the
/// while: the one turned from going smaller to one side as the one turned to comes up from nothing at the other,
/// at that edge; half way, the two side by side, as large as two go in it with the gap between them and
/// round them; then the one turned from going to nothing by its edge as the one turned to comes to the middle, whole.
/// (Slid in from past the lane's edge, a page was cut off there, or faded to nothing before it was in.)
fn page_place(wide: f32, off: f32) -> (f32, f32, f32) {
    let (far, side) = (off.abs().min(1.0), off.signum());
    // Its left and right edges half way: its half of the lane, the gap round it.
    let half = ((wide - 3.0 * PAGE_GAP) / 2.0).max(0.0);
    let (left, right) = if side > 0.0 { (wide - PAGE_GAP - half, wide - PAGE_GAP) } else { (PAGE_GAP, PAGE_GAP + half) };
    let (left, right, shows) = if far <= 0.5 {
        // From the whole lane to its half.
        let t = far / 0.5;
        (left * t, wide + (right - wide) * t, 1.0)
    } else {
        // From its half to nothing at the lane's edge (the gap between it and the other page opening as they go).
        let t = (far - 0.5) / 0.5;
        if side > 0.0 { (left + (wide - left) * t, right + (wide - right) * t, 1.0 - t) } else { (left * (1.0 - t), right * (1.0 - t), 1.0 - t) }
    };
    ((right - left) / wide, left, shows)
}

/// A note's card's rim: glass's soft white, brighter under the pointer; Windows 11's a rule, the note's colour under
/// the pointer; chart paper's ruled round in the sheet's ink, as chart paper's rules are (and lit all over).
fn note_card_rim(frame: &dyn Canvas, theme: &Theme, colour: crate::notes::Colour, (x, y, w, h): (f32, f32, f32, f32), radius: f32, lit: bool) {
    match theme.skin {
        Skin::Glass => {
            let rim = match (theme.dark, lit) {
                (false, false) => 0.65,
                (false, true) => 0.9,
                (true, false) => 0.22,
                (true, true) => 0.45,
            };
            frame.stroke_rounded(Fill::Solid(Color::hex(0xFFFFFF, rim)), x, y, w, h, radius, 1.5);
        }
        Skin::Fluent => frame.stroke_rounded(Fill::Solid(if lit { note_colour(colour).alpha(0.8) } else { theme.rule }), x, y, w, h, radius, if lit { 1.5 } else { 1.0 }),
        Skin::Paper => {
            frame.stroke_rounded(Fill::Solid(theme.stroke), x, y, w, h, radius, 1.0);
            if lit {
                frame.fill_rounded(theme.hover, x, y, w, h, radius);
            }
        }
    }
}

/// A note's page (`width` × `height`, on the skin's sheet) of its colour as its card on the panel is: glass's a pane of
/// its coloured glass (the desktop frosted under it by the sheet), its soft white rim round it; Windows 11's its sheet
/// a little of the colour, ruled round; chart paper's its paper.
pub(crate) fn note_page(frame: &dyn Canvas, theme: &Theme, colour: crate::notes::Colour, (x, y, width, height): (f32, f32, f32, f32)) {
    match theme.skin {
        Skin::Glass => {
            // (Lighter than its card: over a page's breadth the card's own reads as a sheet of paint, the desktop frosted
            // under it no longer seen through; at half of it, it read as another colour.)
            // (No rim of its own: the sheet's is round it, the two a thick frame.)
            // (In the dark, less of the pane of light under it than a card has: its words, the skin's light ones, read
            // on it; with more, a light colour's page, yellow's, came to a middle grey no words read well on.)
            if let Some(light) = note_light(theme.dark) {
                frame.fill_rounded(light.alpha(0.2), x, y, width, height, theme.radius);
            }
            frame.fill_rounded(note_glass(colour).0.alpha(glass_tint(theme.dark) * 0.45), x, y, width, height, theme.radius);
        }
        // Windows 11's: its acrylic through it, a little of the note's colour over it, the band of it along its top
        // its card has (near opaque, the acrylic was lost and the card's band with it).
        Skin::Fluent => {
            frame.fill_rounded(note_colour(colour).alpha(if theme.dark { 0.1 } else { 0.08 }), x, y, width, height, theme.radius);
            frame.clip(x, y, width, 4.0);
            frame.fill_rounded(note_colour(colour), x, y, width, 2.0 * theme.radius + 4.0, theme.radius);
            frame.unclip();
        }
        Skin::Paper => {}
    }
}

/// `top` laid over `under` (an opaque colour): the opaque colour they make.
pub(crate) fn over(under: Color, top: Color) -> Color {
    let mix = |a: f32, b: f32| a + (b - a) * top.a;
    Color { r: mix(under.r, top.r), g: mix(under.g, top.g), b: mix(under.b, top.b), a: 1.0 }
}

/// A note's colour as a dot, its middle at (`x` + its radius, `middle`); the room it takes before its title.
fn note_dot(frame: &dyn Canvas, colour: crate::notes::Colour, x: f32, middle: f32) -> f32 {
    frame.fill_circle(note_colour(colour), Point { x: x + 3.5, y: middle }, 3.5);
    15.0
}

/// How round a note's card's corners are in `theme`: glass's round (its panes'), Windows 11's a little, chart paper's
/// a little more (the owner's: its square ones too stark).
pub(crate) fn note_radius(theme: &Theme, small: bool) -> f32 {
    match theme.skin {
        // (Concentric with the lane's corners near them, as the lane's padding leaves them, a little rounder for the
        // deck's large card; see `plate_round`.)
        Skin::Glass => if small { 10.0 } else { 12.0 },
        Skin::Fluent => 6.0,
        Skin::Paper => 6.0,
    }
}

/// The notes (see `Block::Notes`) from (`left`, `top`), `width` across, in room `tall` high, as `view` lays them out:
/// pressed, each is opened beside.
#[allow(clippy::too_many_arguments)]
fn paint_notes(frame: &dyn Canvas, scene: &Scene, (left, top, width): (f32, f32, f32), notes: &[NoteCard], view: crate::settings::NotesView, tall: f32, hits: &mut Vec<HitBox>) {
    use crate::settings::NotesView;
    let theme = scene.theme;
    let lang = scene.lang;
    if notes.is_empty() {
        let middle = top + tall / 2.0;
        frame.icon(Icon::File, Point { x: left + width / 2.0, y: middle - 12.0 }, 20.0, theme.text3);
        centred(frame, lang.pick("还没有便签，点加号新建一条", "No notes yet: the plus makes one"), theme.small, theme.text2, left, middle + 4.0, width);
        return;
    }
    let lit = |at: usize| notes.get(at).is_some_and(|note| scene.hover == Some(Hit::Note(note.number)));
    let bold = |font: Font, weight: f32| Font { weight, ..font };
    // (Chart paper's colour a dot by the title; glass's card is of it, Windows 11's banded in it.)
    let dotted = theme.skin == Skin::Paper;
    let dot = |frame: &dyn Canvas, note: &NoteCard, x: f32, y: f32| if dotted { note_dot(frame, note.colour, x, y) } else { 0.0 };
    // (Under Windows 11's band, what is on its card a little lower.)
    let banded = if theme.skin == Skin::Fluent { 2.0 } else { 0.0 };
    let inset = 14.0;
    // (Glass's cards' shadows reach past them: room for them either side, and under.)
    let reach = if theme.skin == Skin::Glass { 12.0 } else { 4.0 };
    frame.clip(left - reach, top - 2.0, width + 2.0 * reach, tall + reach);
    match view {
        NotesView::Deck => {
            // The others' titles as the tops of cards stacked over the last written, each lying over the one before
            // it (a strip of each shows; on glass, the one under showing through it), as many as leave it a card's
            // room, the rest scrolled to; the last written whole at the foot.
            let others = notes.len() - 1;
            let room = (others as f32 * NOTE_STRIP).min((tall - NOTE_CARD).max(0.0));
            let (most, scroll) = notes_scroll(scene, others as f32 * NOTE_STRIP, room);
            let radius = note_radius(theme, false);
            // (As tall as reaches under the next: its foot hidden there, as a card's in a stack.)
            let foot = radius + 6.0;
            // (Under the pointer, a strip lifted a little from the deck, easing up and back: as far up as its thumb's ease
            // has come, of the way.)
            let lift = |at: usize| NOTE_LIFT * ((scene.thumbs.of(Hit::Note(notes[at].number)) - 1.0) / (super::thumb::OVER - 1.0)).clamp(0.0, 1.0);
            let strip_y = |at: usize| top + (at - 1) as f32 * NOTE_STRIP - scroll - lift(at);
            // (The strips cut off where they scroll out at the top only: their feet, and their shadows, reach on under
            // the last written, which shows the last of them through it on glass.)
            let (side, under_room) = (12.0, room + foot + 12.0);
            let shows = |at: usize| strip_y(at) + NOTE_STRIP >= top && strip_y(at) <= top + room;
            // On glass, each the narrower the farther back it lies (by so much a strip it is above the last written,
            // so as it scrolls too), as a stack drawn away: its sides inside the corners of the one over it. Of the
            // same width, a card under showed sharp in the corners of the one over it and blurred under it, broken
            // off at its edge.
            // (Chart paper's index cards so too, a little more: of one width and ruled alike, they read as rows.)
            let recede = match theme.skin {
                Skin::Glass => radius * 2.0 / 3.0,
                Skin::Paper => 8.0,
                Skin::Fluent => 0.0,
            };
            let across = |at: usize| {
                let back = recede * ((top + room - strip_y(at)) / NOTE_STRIP).max(0.0);
                (left + back, width - 2.0 * back)
            };
            let strip = |frame: &dyn Canvas, at: usize, plain: bool| {
                let note = &notes[at];
                let y = strip_y(at);
                // What it lies over: the strips before it, as they show.
                let under = |frame: &dyn Canvas| {
                    frame.clip(left - side, top - 2.0, width + 2.0 * side, under_room);
                    for below in (1..at).filter(|below| shows(*below) && notes[*below].held != Held::Away) {
                        let (x, w) = across(below);
                        note_card_ground(frame, theme, notes[below].colour, (x, strip_y(below), w, NOTE_STRIP + foot), radius, false, true, None);
                    }
                    frame.unclip();
                };
                let (x, w) = across(at);
                if note.held == Held::Away {
                    if !plain {
                        held_room(frame, theme, Held::Away, (x, y, w, NOTE_STRIP), radius);
                    }
                    return;
                }
                note_card_ground(frame, theme, note.colour, (x, y, w, NOTE_STRIP + foot), radius, !plain && lit(at), plain, (at > 1).then_some(&under as Under));
                let (title_ink, _, faint) = note_inks(theme, note.colour);
                let line = y + (NOTE_STRIP - LINE) / 2.0 + banded;
                let when_w = frame.measure(&note.when, theme.small);
                let lead = dot(frame, note, x + inset, line + LINE / 2.0);
                let room_w = w - inset - lead - when_w - 24.0;
                frame.text(&fitted(frame, &note.title, bold(theme.body, 600.0), room_w), bold(theme.body, 600.0), title_ink, x + inset + lead, line, room_w, Align::Start);
                frame.text(&note.when, theme.small, faint, x, line + 1.0, w - 14.0, Align::End);
            };
            frame.clip(left - side, top - 2.0, width + 2.0 * side, under_room);
            for at in (1..notes.len()).filter(|at| shows(*at)) {
                // On glass, up to the pane lying on it: under that it is seen only through it, frosted, not sharp. (The
                // others' cards hide what they lie on, and in their corners it shows, as a stack's does.)
                let covered = if at + 1 < notes.len() { strip_y(at + 1) } else { top + room };
                if theme.skin == Skin::Glass {
                    frame.clip(left - side, top - 2.0, width + 2.0 * side, covered - (top - 2.0));
                }
                // (Each named, that it moves from where it was as the notes are laid out anew: see `morph`.)
                frame.key(&format!("note.{}", notes[at].number));
                strip(frame, at, false);
                frame.key("");
                if theme.skin == Skin::Glass {
                    frame.unclip();
                }
                // (Where it lies at rest: lifted, it does not move from under the pointer.)
                let (x, w) = across(at);
                hits.extend(shown_in((x, strip_y(at) + lift(at), w, NOTE_STRIP, Hit::Note(notes[at].number)), (left - 4.0, top, width + 8.0, room)));
            }
            frame.unclip();
            // (After its strips: a press finds a strip before the list they are on.)
            if most > 0.0 {
                hits.push((left, top, width, room, Hit::List(List::Notes, most.to_bits())));
            }
            // The last written over the strips' feet: on glass, the last of them showing through it.
            let under = |frame: &dyn Canvas| {
                frame.clip(left - side, top - 2.0, width + 2.0 * side, under_room);
                for at in (1..notes.len()).filter(|at| shows(*at)) {
                    strip(frame, at, true);
                }
                frame.unclip();
            };
            let front_top = top + room;
            frame.key(&format!("note.{}", notes[0].number));
            paint_front_note(frame, scene, &notes[0], (left, front_top, width, tall - room), lit(0), (others > 0).then_some(&under as Under), hits);
            frame.key("");
        }
        NotesView::Cards => {
            let across = 2;
            let gap = NOTE_CARDS_GAP;
            let card_w = (width - gap) / across as f32;
            let whole = notes.len().div_ceil(across) as f32 * (NOTE_CARD + gap) - gap;
            let (most, scroll) = notes_scroll(scene, whole, tall);
            for (at, note) in notes.iter().enumerate() {
                let (x, y) = (left + (at % across) as f32 * (card_w + gap), top + (at / across) as f32 * (NOTE_CARD + gap) - scroll);
                if y + NOTE_CARD < top || y > top + tall {
                    continue;
                }
                frame.key(&format!("note.{}", note.number));
                paint_note_card(frame, scene, note, (x, y, card_w, NOTE_CARD), lit(at));
                frame.key("");
                hits.extend(shown_in((x, y, card_w, NOTE_CARD, Hit::Note(note.number)), (left - 4.0, top, width + 8.0, tall)));
            }
            if most > 0.0 {
                hits.push((left, top, width, tall, Hit::List(List::Notes, most.to_bits())));
            }
        }
        NotesView::List => {
            // A row each: its colour (Windows 11's a band of it, the others' a dot), its title over its first line,
            // when it was written (no rule between them: as the station's rows are not).
            let (most, scroll) = notes_scroll(scene, notes.len() as f32 * NOTE_ROW, tall);
            for (at, note) in notes.iter().enumerate() {
                let y = top + at as f32 * NOTE_ROW - scroll;
                if y + NOTE_ROW < top || y > top + tall {
                    continue;
                }
                frame.key(&format!("note.{}", note.number));
                paint_note_row(frame, scene, note, (left, y, width, NOTE_ROW), lit(at));
                frame.key("");
                hits.extend(shown_in((left, y, width, NOTE_ROW, Hit::Note(note.number)), (left - 4.0, top, width + 8.0, tall)));
            }
            if most > 0.0 {
                hits.push((left, top, width, tall, Hit::List(List::Notes, most.to_bits())));
            }
        }
    }
    frame.unclip();
}

/// A note's card as the cards show it, in the box (`x`, `y`, `w`, `h`), `lit` under the pointer: its colour's ground,
/// its title (after its colour's dot, on chart paper), as many of its lines as fit over when it was written, at its
/// foot.
fn paint_note_card(frame: &dyn Canvas, scene: &Scene, note: &NoteCard, room: (f32, f32, f32, f32), lit: bool) {
    let theme = scene.theme;
    let Some((x, y, w, h)) = held_room(frame, theme, note.held, room, note_radius(theme, true)) else { return };
    note_card_ground(frame, theme, note.colour, (x, y, w, h), note_radius(theme, true), lit, false, None);
    let (title_ink, words, faint) = note_inks(theme, note.colour);
    let inner = if theme.skin == Skin::Glass { 13.0 } else { 11.0 };
    // (Under Windows 11's band, a little lower.)
    let head = y + 10.0 + if theme.skin == Skin::Fluent { 2.0 } else { 0.0 };
    let lead = if theme.skin == Skin::Paper { note_dot(frame, note.colour, x + inner, head + LINE / 2.0) } else { 0.0 };
    let title = Font { weight: 700.0, ..theme.body };
    let room = w - inner - lead - 10.0;
    frame.text(&fitted(frame, &note.title, title, room), title, title_ink, x + inner + lead, head, room, Align::Start);
    let when_y = y + h - LINE - 8.0;
    for (at, line) in note.lines.iter().enumerate() {
        let line_y = head + LINE + 6.0 + at as f32 * (LINE + 1.0);
        if line_y + LINE > when_y - 2.0 {
            break;
        }
        frame.text(&fitted(frame, line, theme.small, w - inner - 10.0), theme.small, words, x + inner, line_y, w - inner - 10.0, Align::Start);
    }
    frame.text(&note.when, theme.small, faint, x + inner, when_y, w - inner - 10.0, Align::Start);
}

/// A note's row as the list shows it, in the box (`left`, `y`, `width`, `h`), `lit` under the pointer: its colour
/// (Windows 11's a band of it, the others' a dot), its title over its first line, when it was written at its end.
fn paint_note_row(frame: &dyn Canvas, scene: &Scene, note: &NoteCard, room: (f32, f32, f32, f32), lit: bool) {
    let theme = scene.theme;
    let Some((left, y, width, h)) = held_room(frame, theme, note.held, room, plate_round(theme, GLASS_ROW_ROUND - 4.0, 4.0)) else { return };
    if lit {
        frame.fill_rounded(theme.hover, left - 4.0, y + 1.0, width + 8.0, h - 2.0, plate_round(theme, GLASS_ROW_ROUND - 4.0, 4.0));
    }
    // (Its title over its first line; with none, in the row's middle, as when it was written is.)
    let title_y = if note.lines.is_empty() { y + (NOTE_ROW - LINE) / 2.0 } else { y + 4.0 };
    if theme.skin == Skin::Fluent {
        frame.fill_rounded(note_colour(note.colour), left, y + 7.0, 4.0, h - 14.0, 2.0);
    } else {
        note_dot(frame, note.colour, left, title_y + LINE / 2.0);
    }
    let when_w = frame.measure(&note.when, theme.small);
    let x = left + if theme.skin == Skin::Fluent { 14.0 } else { 15.0 };
    let title = Font { weight: 600.0, ..theme.body };
    frame.text(&fitted(frame, &note.title, title, width - 24.0 - when_w), title, theme.text, x, title_y, width - 24.0 - when_w, Align::Start);
    // (Short of when it was written, as the title is: under it, it ran on under the time.)
    for (at, line) in note.lines.iter().enumerate() {
        let line_y = y + 4.0 + LINE + at as f32 * (LINE + 1.0);
        if line_y + LINE > y + h - 4.0 {
            break;
        }
        frame.text(&fitted(frame, line, theme.small, width - 24.0 - when_w), theme.small, theme.text3, x, line_y, width - 24.0 - when_w, Align::Start);
    }
    frame.text(&note.when, theme.small, theme.text3, left, y + (NOTE_ROW - LINE) / 2.0, width, Align::End);
}

/// The deck's last written note, whole, in the box (`x`, `y`, `w`, `h`), `lit` under the pointer, over what `below`
/// draws (see `note_card_ground`): when it was written, its title, as many of its lines as fit.
#[allow(clippy::too_many_arguments)]
fn paint_front_note(frame: &dyn Canvas, scene: &Scene, note: &NoteCard, room: (f32, f32, f32, f32), lit: bool, below: Option<Under>, hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    let Some((left, y, width, card)) = held_room(frame, theme, note.held, room, note_radius(theme, false)) else { return };
    note_card_ground(frame, theme, note.colour, (left, y, width, card), note_radius(theme, false), lit, false, below);
    let (title_ink, words, faint) = note_inks(theme, note.colour);
    let x = left + if theme.skin == Skin::Glass { 16.0 } else { 14.0 };
    let head = y + if theme.skin == Skin::Glass { 14.0 } else { 12.0 } + if theme.skin == Skin::Fluent { 2.0 } else { 0.0 };
    // (Before when it was written: chart paper's its colour's dot, Windows 11's a note's mark; glass's nothing, the
    // card of its colour.)
    let lead = match theme.skin {
        Skin::Paper => note_dot(frame, note.colour, x, head + LINE / 2.0),
        Skin::Fluent => {
            frame.icon(Icon::File, Point { x: x + 6.0, y: head + LINE / 2.0 }, 12.0, theme.text3);
            18.0
        }
        Skin::Glass => 0.0,
    };
    let room = width - (x - left) - 14.0;
    frame.text(&note.when, theme.small, if theme.skin == Skin::Fluent { theme.text2 } else { faint }, x + lead, head + 1.0, room - lead, Align::Start);
    let title = Font { size: theme.title.size + 1.0, weight: 700.0, ..theme.title };
    frame.text(&fitted(frame, &note.title, title, room), title, title_ink, x, head + LINE + 6.0, room, Align::Start);
    for (at, line) in note.lines.iter().enumerate() {
        let line_y = head + LINE + 6.0 + title.size * 1.4 + 4.0 + at as f32 * (LINE + 4.0);
        if line_y + LINE > y + card - 8.0 {
            break;
        }
        frame.text(&fitted(frame, line, theme.body, room), theme.body, words, x, line_y, room, Align::Start);
    }
    hits.push((left, y, width, card, Hit::Note(note.number)));
}

/// How far the notes scroll at most, their whole `whole` tall in room `tall`, and how far they are scrolled now.
fn notes_scroll(scene: &Scene, whole: f32, tall: f32) -> (f32, f32) {
    let most = (whole - tall).max(0.0);
    (most, scene.scrolls.at(List::Notes).clamp(0.0, most))
}

/// What is kept at hand: its cell (a picture and its name under it) as a whole lane lays it out, the picture's side,
/// and the gap between rows; a rail's cell, and its picture's side.
const APP_CELL_W: f32 = 64.0;
const APP_CELL_H: f32 = 62.0;
const APP_SIDE: f32 = 36.0;
const APP_GAP: f32 = 6.0;
const RAIL_CELL: f32 = 46.0;
const RAIL_SIDE: f32 = 32.0;
/// The most rows of what is kept at hand a whole lane shows.
const APP_ROWS: usize = 3;

/// Quick launch's lane: whole (`full`), what is kept at hand under its name, named, in as many rows as they take (up
/// to `APP_ROWS`, the place to add more after them); else their pictures in one row.
fn apps_lane(full: bool, theme: &Theme, lang: Lang) -> Vec<Block> {
    let apps: Vec<(String, String)> = crate::station::apps::shown().iter().map(|app| (app.name.clone(), app.target.clone())).collect();
    let rows = if full { apps_rows(apps.len(), theme) } else { 1 };
    vec![head(lang.pick("快捷 App", "Quick launch"), "", "", false), Block::Apps { apps, rows, named: full, over: crate::station::run::shown().apps_over }]
}

/// The rows a whole quick launch lane takes for `count` things (and the place to add more) across a column's room in
/// `theme`'s skin, up to `APP_ROWS` (more scrolled to).
fn apps_rows(count: usize, theme: &Theme) -> usize {
    let across = (((COLUMN_WIDTH - 2.0 * theme.pad_x) / APP_CELL_W) as usize).max(1);
    (count + 1).div_ceil(across).clamp(1, APP_ROWS)
}

/// The picture of what is kept at hand (`target`, what opens it) in the square `side` wide at (`x`, `y`): as Explorer
/// shows it; a web address, by its site's first letter.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_app(frame: &dyn Canvas, theme: &Theme, name: &str, target: &str, x: f32, y: f32, side: f32) {
    let web = target.starts_with("https://") || target.starts_with("http://");
    let path = std::path::Path::new(target);
    // A file or folder on the disk: a folder, or a file not a program, by its mark on a plate (as the station draws
    // them); a program by its picture. One no longer there, faint.
    let on_disk = !web && !target.starts_with("shell:");
    let found = on_disk.then(|| crate::station::thumbs::looked(path)).flatten();
    let gone = found == Some(crate::station::thumbs::Looked::Gone);
    // (One gone, or not yet looked at: a folder where it has no extension.)
    let folder = match found {
        Some(crate::station::thumbs::Looked::Folder) => true,
        Some(crate::station::thumbs::Looked::File(_)) => false,
        _ => path.extension().is_none(),
    };
    if on_disk && (folder || !crate::station::apps::pictured(path)) {
        let centre = Point { x: x + side / 2.0, y: y + side / 2.0 };
        match crate::station::thumbs::icon(path, APP_PIXELS).filter(|_| !gone) {
            Some(icon) => frame.picture(&icon, x, y, side, side, 0.0, 1.0),
            None => paint_file_mark(frame, theme, folder, path, centre, side * 0.8, side >= 26.0, gone),
        }
        return;
    }
    // (A program by its icon alone: its thumbnail is, for some, its icon on a white square.)
    match crate::station::thumbs::icon(path, APP_PIXELS).filter(|_| !web) {
        Some(picture) => frame.picture(&picture, x, y, side, side, side / 5.0, if gone { 0.4 } else { 1.0 }),
        None => {
            frame.fill_rounded(theme.track, x, y, side, side, side / 5.0);
            let letter: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
            let font = Font { size: side * 0.45, weight: 650.0, ..theme.body };
            let ink = if gone { theme.text2.alpha(theme.text2.a * 0.45) } else { theme.text2 };
            centred(frame, &letter, font, ink, x, y + (side - font.size * 1.3) / 2.0, side);
        }
    }
}

/// The place to add to what is kept at hand: a plus in a dashed round, `side` wide at (`x`, `y`), lit under the pointer.
/// (`over`: something dragged over what is kept at hand: the place it goes to, in the accent, an arrow down in it.)
fn paint_adder(frame: &dyn Canvas, scene: &Scene, x: f32, y: f32, side: f32, over: bool, hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    let lit = scene.hover == Some(Hit::AddApp);
    let centre = Point { x: x + side / 2.0, y: y + side / 2.0 };
    let accent = if theme.skin == Skin::Paper { theme.text } else { super::theme::accent(theme.dark) };
    let edge = if over { accent } else if lit { theme.text2 } else { theme.text3 };
    // Its ring dashed, its corners rounded as its skin rounds a small square (glass's the most).
    let (ring, round) = (side * 0.92, match theme.skin {
        Skin::Glass => side * 0.26,
        Skin::Fluent => 4.0,
        Skin::Paper => 6.0,
    });
    let (rx, ry) = (centre.x - ring / 2.0, centre.y - ring / 2.0);
    if over {
        frame.fill_rounded(accent.alpha(0.12), rx, ry, ring, ring, round);
    }
    frame.stroke_dashed(&rounded_outline(rx, ry, ring, ring, round), edge, if over { 1.6 } else { 1.2 });
    match over {
        true => frame.icon(Icon::ArrowDown, centre, side * 0.45, accent),
        false => frame.icon(Icon::Add, centre, side * 0.45, if lit { theme.text } else { theme.text2 }),
    }
    hits.push((x, y, side, side, Hit::AddApp));
}

/// The outline of the box `w` × `h` at (`x`, `y`), its corners rounded by `radius`, as points round it (closed), for a
/// dashed line to be drawn along.
pub(crate) fn rounded_outline(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Vec<Point> {
    let radius = radius.min(w / 2.0).min(h / 2.0);
    let corners = [(x + w - radius, y + radius, -0.25), (x + w - radius, y + h - radius, 0.0), (x + radius, y + h - radius, 0.25), (x + radius, y + radius, 0.5)];
    let mut points = Vec::new();
    for (cx, cy, from) in corners {
        for step in 0..=6 {
            let angle = (from + step as f32 / 6.0 * 0.25) * std::f32::consts::TAU;
            points.push(Point { x: cx + radius * angle.cos(), y: cy + radius * angle.sin() });
        }
    }
    points.push(points[0]);
    points
}

/// What is kept at hand as a lane lays it out (see `Block::Apps`) from (`left`, `top`), `width` across, in its `rows`:
/// named cells spread over the width, scrolled through where they take more rows; or one row of pictures (as many as
/// fit); the place to add more after them. What is dragged there may be dropped to be kept at hand.
#[allow(clippy::too_many_arguments)]
fn paint_apps(frame: &dyn Canvas, scene: &Scene, (left, top, width): (f32, f32, f32), apps: &[(String, String)], rows: usize, named: bool, over: bool, hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    let (cell_w, cell_h) = if named { (APP_CELL_W, APP_CELL_H) } else { (APP_SIDE + APP_GAP, APP_SIDE) };
    let tall = rows as f32 * cell_h + rows.saturating_sub(1) as f32 * APP_GAP;
    let mut placed = Vec::new();
    let across = ((width / cell_w) as usize).max(1);
    let gap = if named && across > 1 { (width - across as f32 * cell_w) / (across - 1) as f32 } else { 0.0 };
    let count = if named { apps.len() + 1 } else { (apps.len() + 1).min(across) };
    let whole = count.div_ceil(across) as f32 * (cell_h + APP_GAP) - APP_GAP;
    let most = (whole - tall).max(0.0);
    let scroll = scene.scrolls.at(List::Apps).clamp(0.0, most);
    let view = (left - 4.0, top - 2.0, width + 8.0, tall + 4.0);
    frame.clip(view.0, view.1, view.2, view.3);
    let cell = |at: usize| (left + (at % across) as f32 * (cell_w + gap), top + (at / across) as f32 * (cell_h + APP_GAP) - scroll);
    // (The one held: its picture's middle at the pointer, its cell round it.)
    let picture_in = if named { (cell_w - APP_SIDE) / 2.0 } else { 0.0 };
    let held_cell = |(px, py): (f32, f32)| (px - APP_SIDE / 2.0 - picture_in, py - APP_SIDE / 2.0);
    for at in 0..count {
        // Each in its place, or on its way to it; the one held drawn after all, where the pointer has it.
        let spot = match apps.get(at) {
            Some((_, target)) => app_spot(scene, target, at, &cell, &held_cell),
            None => Some(cell(at)),
        };
        let Some((x, y)) = spot else { continue };
        if y + cell_h < top || y > top + tall {
            continue;
        }
        let px = if named { x + (cell_w - APP_SIDE) / 2.0 } else { x };
        let Some((name, target)) = apps.get(at) else {
            paint_adder(frame, scene, px, y, APP_SIDE, over, &mut placed);
            break;
        };
        let hit = Hit::App(crate::station::apps::number_of(target));
        if scene.hover == Some(hit) {
            frame.fill_rounded(theme.hover, x - 2.0, y - 2.0, (if named { cell_w } else { APP_SIDE }) + 4.0, cell_h + 4.0, plate_round(theme, GLASS_ROW_ROUND, 2.0));
        }
        frame.key(&format!("app.{target}"));
        paint_app(frame, theme, name, target, px, y, APP_SIDE);
        if named {
            centred(frame, &fitted(frame, name, theme.small, cell_w), theme.small, theme.text2, x, y + APP_SIDE + 4.0, cell_w);
        }
        placed.push((x, y, if named { cell_w } else { APP_SIDE }, cell_h, hit));
    }
    frame.key("");
    if let Some((held, point)) = scene.thumbs.app_held() {
        placed.retain(|hit| !matches!(hit.4, Hit::App(_)));
        if let Some((name, target)) = apps.iter().find(|(_, target)| target == held) {
            let (x, y) = held_cell(point);
            paint_held_app(frame, scene, name, target, x + picture_in, y, APP_SIDE);
        }
    }
    frame.unclip();
    // (Its things first, as much of them as shows: a press finds a thing before the list, and the room round it.)
    hits.extend(placed.into_iter().filter_map(|hit| shown_in(hit, view)));
    // The places one dragged among them may go to, standing still (see `Hit::AppSlot`), after all else.
    let places: Vec<HitBox> = (0..apps.len()).map(|at| {
        let (x, y) = cell(at);
        (x, y, if named { cell_w } else { APP_SIDE }, cell_h, Hit::AppSlot(at as u16))
    }).collect();
    if most > 0.0 {
        hits.push((left, top, width, tall, Hit::List(List::Apps, most.to_bits())));
    }
    hits.push((left - 4.0, top - SHELF_GAP - HEAD - HEAD_GAP, width + 8.0, tall + SHELF_GAP + HEAD + HEAD_GAP + 4.0, Hit::AppsArea));
    hits.extend(places.into_iter().filter_map(|hit| shown_in(hit, view)));
    scroll_thumb(frame, theme, left + width + 6.0, top, tall, whole, scroll, most);
}

/// A thing kept at hand held and dragged, its picture `side` square at (`x`, `y`): lifted off the others, a little
/// larger, on its shadow.
fn paint_held_app(frame: &dyn Canvas, scene: &Scene, name: &str, target: &str, x: f32, y: f32, side: f32) {
    let grown = side * 1.12;
    let (x, y) = (x - (grown - side) / 2.0, y - (grown - side) / 2.0);
    frame.shadow(Color::hex(0, if scene.theme.dark { 0.45 } else { 0.22 }), x, y, grown, grown, grown * 0.22, 8.0, 4.0);
    paint_app(frame, scene.theme, name, target, x, y, grown);
}

/// Where the thing kept at hand `target` (at `at` among them) is drawn, its cell's corner (see `thumb::Dots::apps`):
/// in its place (`cell` of a place), between two places as it goes from one to the other, or on its way from where
/// it was let go (its cell there as it was drawn held: `held_cell` of the pointer) to its place; none while it is held.
fn app_spot(scene: &Scene, target: &str, at: usize, cell: &dyn Fn(usize) -> (f32, f32), held_cell: &dyn Fn((f32, f32)) -> (f32, f32)) -> Option<(f32, f32)> {
    use super::thumb::AppPlace;
    let lerp = |a: (f32, f32), b: (f32, f32), t: f32| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    Some(match scene.thumbs.app_place(target, at)? {
        AppPlace::Slot(slot) => {
            let low = slot.floor().max(0.0);
            lerp(cell(low as usize), cell(slot.ceil().max(0.0) as usize), slot - low)
        }
        AppPlace::From { x, y, to, done } => lerp(held_cell((x, y)), cell(to), done),
    })
}

/// Quick launch's lane as a rail along the panel's edge (`area`, at `side`): what is kept at hand, a picture each,
/// down it (across it, under the columns), the place to add more after them, and at its foot the panel's buttons
/// as its bar has them (see `panel_button`): the pin, the settings.
fn paint_rail(frame: &dyn Canvas, scene: &Scene, lane: &Lane, area: Rect, side: RailSide, hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    let (apps, over): (&[(String, String)], bool) = lane.blocks.iter().find_map(|block| if let Block::Apps { apps, over, .. } = block { Some((apps.as_slice(), *over)) } else { None }).unwrap_or((&[], false));
    let across = side == RailSide::Bottom;
    let foot = RAIL_FOOT;
    // (Along the rail: down it, or across it under the columns.)
    let (start, length) = if across { (area.x + 6.0, area.w - 12.0 - foot) } else { (area.y + 8.0, area.h - 16.0 - foot) };
    let cell = |at: usize| -> (f32, f32) {
        let along = start + at as f32 * RAIL_CELL;
        if across { (along, area.y + (area.h - RAIL_CELL) / 2.0) } else { (area.x + (area.w - RAIL_CELL) / 2.0, along) }
    };
    let lit_on = |hit: Hit| scene.hover == Some(hit);
    // Its things, and the place to add more after them, scrolled along by the wheel where they are more than fit.
    let whole = (apps.len() + 1) as f32 * RAIL_CELL;
    let most = (whole - length).max(0.0);
    let scroll = scene.scrolls.at(List::Apps).clamp(0.0, most);
    let (view_x, view_y, view_w, view_h) = if across { (start, area.y, length, area.h) } else { (area.x, start, area.w, length) };
    frame.clip(view_x, view_y, view_w, view_h);
    let mut placed = Vec::new();
    // (Scrolled along: where each place shows.)
    let shown_cell = |at: usize| {
        let (x, y) = cell(at);
        if across { (x - scroll, y) } else { (x, y - scroll) }
    };
    let inset = (RAIL_CELL - RAIL_SIDE) / 2.0;
    // (The one held: along the rail as the pointer has it, in its middle across it.)
    let held_cell = |(px, py): (f32, f32)| if across { (px - RAIL_CELL / 2.0, area.y + (area.h - RAIL_CELL) / 2.0) } else { (area.x + (area.w - RAIL_CELL) / 2.0, py - RAIL_CELL / 2.0) };
    for at in 0..=apps.len() {
        // Each in its place, or on its way to it; the one held drawn after all, where the pointer has it.
        let spot = match apps.get(at) {
            Some((_, target)) => app_spot(scene, target, at, &shown_cell, &held_cell),
            None => Some(shown_cell(at)),
        };
        let Some((x, y)) = spot else { continue };
        if if across { x + RAIL_CELL < view_x || x > view_x + view_w } else { y + RAIL_CELL < view_y || y > view_y + view_h } {
            continue;
        }
        let Some((name, target)) = apps.get(at) else {
            paint_adder(frame, scene, x + inset, y + inset, RAIL_SIDE, over, &mut placed);
            continue;
        };
        let hit = Hit::App(crate::station::apps::number_of(target));
        if lit_on(hit) {
            app_plate(frame, theme, (x + 2.0, y + 2.0, RAIL_CELL - 4.0), (RAIL_CELL - 4.0 - RAIL_SIDE) / 2.0);
        }
        paint_app(frame, theme, name, target, x + inset, y + inset, RAIL_SIDE);
        placed.push((x, y, RAIL_CELL, RAIL_CELL, hit));
    }
    // The one held, lifted where the pointer has it along the rail; the places it may go to, standing still, all of
    // the rail between them (before the first, past the last).
    if scene.thumbs.app_held().is_some() {
        placed.retain(|hit| !matches!(hit.4, Hit::App(_)));
    }
    if let Some((held, point)) = scene.thumbs.app_held() {
        if let Some((name, target)) = apps.iter().find(|(_, target)| target == held) {
            let (x, y) = held_cell(point);
            paint_held_app(frame, scene, name, target, x + inset, y + inset, RAIL_SIDE);
        }
    }
    frame.unclip();
    // The places one dragged among them may go to, standing still (see `Hit::AppSlot`), all of the rail between them
    // (before the first, past the last, the "+" over it): after all else.
    let places: Vec<HitBox> = (0..apps.len()).map(|at| {
        let (x, y) = shown_cell(at);
        let (lead, tail) = (if at == 0 { 1000.0 } else { 0.0 }, if at + 1 == apps.len() { 1000.0 } else { 0.0 });
        if across { (x - lead, area.y, RAIL_CELL + lead + tail, area.h, Hit::AppSlot(at as u16)) } else { (area.x, y - lead, area.w, RAIL_CELL + lead + tail, Hit::AppSlot(at as u16)) }
    }).collect();
    // (Only what shows of them is pressed.)
    hits.extend(placed.into_iter().filter_map(|hit| shown_in(hit, (view_x, view_y, view_w, view_h))));
    if most > 0.0 {
        hits.push((view_x, view_y, view_w, view_h, Hit::List(List::Apps, most.to_bits())));
    }
    // Where something dragged may be dropped to be kept at hand: all of the rail (after its things).
    hits.push((area.x, area.y, area.w, area.h, Hit::AppsArea));
    hits.extend(places.into_iter().filter_map(|hit| shown_in(hit, (view_x, view_y, view_w, view_h))));
    // The foot: the pin (lit while pinned), the settings, as the bar has them and as far apart.
    for (at, hit) in [Hit::Pin, Hit::Settings].into_iter().enumerate() {
        let along = at as f32 * (BUTTON + BUTTON_GAP);
        let (x, y) = if across { (area.x + area.w - 6.0 - (foot - RAIL_FOOT_GAP) + along, area.y + (area.h - BUTTON) / 2.0) } else { (area.x + (area.w - BUTTON) / 2.0, area.y + area.h - 8.0 - (foot - RAIL_FOOT_GAP) + along) };
        // (Before the rail's own: a press on the foot is the button's.)
        hits.insert(0, panel_button(frame, scene, hit, x, y));
    }
    // Chart paper sets the rail off from the columns by a rule.
    if theme.ruled {
        match side {
            RailSide::Start => frame.fill(theme.rule, area.x + area.w, area.y, 1.0, area.h),
            RailSide::End => frame.fill(theme.rule, area.x - 1.0, area.y, 1.0, area.h),
            RailSide::Bottom => frame.fill(theme.rule, area.x, area.y - 1.0, area.w, 1.0),
        }
    }
}

/// The station's tiles as one row (a small lane): each one's room across, its picture's side, its room down (the
/// picture and its name under it); the gap between tiles (and cards: see `cards_across`).
const TILE_W: f32 = 72.0;
const TILE_PICTURE: f32 = 56.0;
const TILE_H: f32 = TILE_PICTURE + 4.0 + 16.0;
const TILE_GAP: f32 = 8.0;
/// The room between the station's head and what is under it.
const SHELF_GAP: f32 = 6.0;
/// How tall the room under a whole station lane's head is, whichever page it shows.
const STATION_TALL: f32 = 284.0;
/// A row of the station's list, and of what was copied lately; the side of the picture at its start.
const LIST_ROW: f32 = 40.0;
const LIST_PICTURE: f32 = 30.0;
/// How many large cards go across (see `ShelfView::Large`), their plate as tall as so much of their width at most (two
/// rows of them whole in the room they are shown in: see `large_plate`).
const LARGE_ACROSS: usize = 2;
const LARGE_PLATE: f32 = 0.75;

/// How tall a large card's plate is, `w` wide, shown in room `tall` high: as tall as `LARGE_PLATE` of its width, no
/// taller than lets two rows of them, their words under them, show whole (the second's names were cut off under the
/// room's foot, the files on no plate of their own seeming to float).
fn large_plate(w: f32, tall: f32) -> f32 {
    (w * LARGE_PLATE).min((tall - CARDS_LEAD - TILE_GAP) / 2.0 - CARD_WORDS).max(w * 0.4)
}

/// How far down their room cards lie (see `paint_tiles`).
const CARDS_LEAD: f32 = 4.0;
/// A tab's height, and the room round its name.
const TAB_H: f32 = 24.0;
const TAB_PAD: f32 = 8.0;
/// How many physical px the side a picture is asked for at is rounded up to a whole number of: a face sized a little
/// (the panel's width dragged) asks for no new one at each pixel.
const PICTURE_STEP: u32 = 32;
/// The side a thing kept at hand's icon is asked for (physical px): the largest every program has (Windows' extra
/// large icon). Asked larger, a program with only small icons is given one small in the middle of a white box.
pub(crate) const APP_PIXELS: u32 = 48;

/// A card's plate as tall as so much of its width, and the room under it for its name and what it is.
const PLATE: f32 = 0.74;
const CARD_WORDS: f32 = 5.0 + 2.0 * LINE;

/// The side a picture drawn `side` DIPs square on `frame` is asked for at (physical px): as many as it is drawn on,
/// rounded up to `PICTURE_STEP`.
pub(super) fn picture_pixels(frame: &dyn Canvas, side: f32) -> u32 {
    ((side * frame.pixels_per_dip()).ceil().max(1.0) as u32).div_ceil(PICTURE_STEP) * PICTURE_STEP
}

/// How many cards go across `width`: three in a column (as many as fit in a wider one), each at least a tile wide.
fn cards_across(width: f32) -> usize {
    (((width + TILE_GAP) / (TILE_W + 24.0 + TILE_GAP)) as usize).clamp(1, 4)
}

/// How many tiles go across `width`.
fn tiles_across(width: f32) -> usize {
    (((width + TILE_GAP) / (TILE_W + TILE_GAP)) as usize).max(1)
}

/// The station's lane: whole (`full`), its head's pages (what is on it, and with Recently copied on, what was copied
/// lately, and with Recent files on, the files come lately) over the page `tab` (as far as there is one); else what is on it in a row
/// under its name.
fn station_lane(full: bool, tab: Tab, lang: Lang) -> Vec<Block> {
    let shown = crate::station::run::shown();
    let folder = crate::station::folder::Folder::own();
    let now = crate::station::now_ms();
    let tiles: Vec<Tile> = shown.items.iter().rev().filter(|item| !item.copied).map(|item| Tile::of(item, &folder, lang, now)).collect();
    use crate::station::keep::Refused;
    let said = if let Some(refused) = shown.refused {
        Some(match refused {
            Refused::Full => lang.pick("空间已满：请取消一些固定，或在设置中调大上限", "No room left: unpin some, or allow more in the settings"),
            Refused::Unwritten => lang.pick("未能保存副本：请检查磁盘空间，并确认文件未被其他程序占用", "A copy could not be kept: check the disk's free space, and that no other program holds the file"),
        }.to_string())
    } else if tiles.is_empty() {
        Some(lang.pick("把文件、图片或文字拖到这里", "Drag files, pictures or text here").to_string())
    } else {
        None
    };
    if !full {
        let count = if tiles.is_empty() { String::new() } else { tiles.len().to_string() };
        return vec![head(lang.pick("中转站", "Station"), count, "", false), Block::Shelf { tiles, shows: Shows::Row, tall: TILE_H, said, over: shown.over }];
    }
    let copies = shown.history;
    let tab = if !copies && matches!(tab, Tab::Copies(_)) { Tab::Shelf } else { tab };
    let recent = crate::station::recent::on();
    let tab = if !recent && matches!(tab, Tab::Recent(_)) { Tab::Shelf } else { tab };
    let view = match tab {
        Tab::Shelf => shown.view,
        Tab::Copies(_) => shown.copies_view,
        Tab::Recent(_) => shown.recent_view,
    };
    let mut blocks = vec![Block::Tabs { tab, count: tiles.len(), copies, recent, view }];
    blocks.push(match tab {
        Tab::Shelf => Block::Shelf { tiles, shows: Shows::View(view), tall: STATION_TALL, said, over: shown.over },
        Tab::Copies(filter) => Block::Copies { rows: copied_lately(&shown.items, filter).map(|item| Tile::of(item, &folder, lang, now)).collect(), filter, view, tall: STATION_TALL },
        Tab::Recent(filter) => Block::Recent { rows: Lately::rows(&crate::station::recent::shown(), filter, lang, now), filter, view, tall: STATION_TALL, nowhere: crate::station::recent::nowhere() },
    });
    blocks
}

/// What was copied lately of `items` (the station's), the newest first, of those `filter` takes.
fn copied_lately(items: &[crate::station::shelf::Item], filter: crate::station::shelf::Filter) -> impl Iterator<Item = &crate::station::shelf::Item> {
    items.iter().rev().filter(move |item| item.copied && filter.takes(&item.what))
}

/// The least a list at a lane's foot is shown in (a couple of rows; the rest scrolled to), as its lane is made
/// shorter than it is laid out (a workbench widget sized down: see `Layout::fitted`).
const LIST_LEAST: f32 = 96.0;

/// The least a workbench lane is drawn across (DIPs, its padding in): its things laid out again for the width it has
/// down to this (the station's pages, where their names do not fit side by side, one menu: see `paint_tabs`).
pub const LANE_LEAST: f32 = 220.0;

impl Block {
    /// How tall a list at a lane's foot is laid out (what of it may give way, down to `LIST_LEAST`); none for others.
    fn list_tall(&self) -> Option<f32> {
        match self {
            Block::Shelf { tall, .. } | Block::Copies { tall, .. } | Block::Recent { tall, .. } | Block::Notes { tall, .. } => Some(*tall),
            _ => None,
        }
    }

    fn height(&self) -> f32 {
        match self {
            Block::Head { .. } => HEAD + HEAD_GAP,
            Block::Readout { .. } => PLOT,
            Block::Rates { .. } => RATE_PLOT,
            Block::Trace { gap, .. } => gap + RATE_PLOT,
            Block::Threads(_) => 10.0 + 14.0,
            Block::Meter { gap, .. } => gap + LINE,
            Block::Facts { rows, gap } if !rows.is_empty() => gap + rows.len() as f32 * LINE + (rows.len() - 1) as f32 * FACT_GAP,
            Block::Facts { .. } => 0.0,
            Block::Sound { .. } => MEDIA_GAP + MEDIA_ROW,
            Block::Playing { .. } => MEDIA_GAP + 2.0 * LINE,
            Block::Progress { .. } => MEDIA_ROW,
            Block::Lyric { .. } => MEDIA_GAP + 2.0 * LINE,
            Block::Table { visible, .. } => *visible as f32 * TABLE_ROW,
            Block::Room(tall) => *tall,
            Block::Tabs { .. } => HEAD + HEAD_GAP,
            Block::StuckNote(_) => STUCK_NOTE.1,
            Block::Bench(_) => BENCH,
            Block::Shelf { tall, .. } | Block::Copies { tall, .. } | Block::Recent { tall, .. } | Block::Notes { tall, .. } => SHELF_GAP + tall,
            Block::Apps { rows, named, .. } => SHELF_GAP + *rows as f32 * if *named { APP_CELL_H } else { APP_SIDE } + rows.saturating_sub(1) as f32 * APP_GAP,
        }
    }
}

impl Hash for Block {
    /// Everything the content pass draws from: two blocks that hash alike
    /// draw alike.
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Block::Head { title, device, aside, aside_hot } => (title, device, aside, aside_hot).hash(state),
            Block::Room(tall) => tall.to_bits().hash(state),
            Block::Readout { figure, unit, hot, .. } => (figure, unit, hot).hash(state),
            Block::Rates { rows, plot, scale } => (rows, plot.as_ref().map(|plot| plot.max.to_bits()), scale).hash(state),
            Block::Trace { label, value, hot, plot, gap } => (label, value, hot, plot.max.to_bits(), gap.to_bits()).hash(state),
            Block::Threads(cells) => cells.iter().for_each(|(load, hot, tag)| (load.map(f32::to_bits), hot, tag).hash(state)),
            Block::Meter { label, fraction, value, hot, gap, opens } => (label, fraction.to_bits(), value, hot, gap.to_bits(), opens).hash(state),
            Block::Facts { rows, gap } => (rows, gap.to_bits()).hash(state),
            Block::Sound { level, muted } => (level, muted).hash(state),
            Block::Playing { title, about, playing, players } => (title, about, playing, players).hash(state),
            Block::Progress { at, length, seekable } => (at, length, seekable).hash(state),
            Block::Lyric { line, under, said, glide } => (line, under, said, glide).hash(state),
            Block::Table { headings, sort, rows, visible, scrolls, widths } => {
                headings.iter().for_each(|(label, key)| (label, key.map(|key| key as u8)).hash(state));
                (sort.map(|sort| sort as u8), rows, visible, scrolls, widths.map(f32::to_bits)).hash(state);
            }
            Block::Tabs { tab, count, copies, recent, view } => (tab, count, copies, recent, view).hash(state),
            Block::Bench(what) => what.hash(state),
            Block::Notes { notes, view, tall } => (notes, view, tall.to_bits()).hash(state),
            Block::StuckNote(note) => note.hash(state),
            Block::Apps { apps, rows, named, over } => {
                (apps, rows, named, over).hash(state);
                crate::station::thumbs::changes().hash(state);
            }
            Block::Recent { rows, filter, view, tall, nowhere } => {
                (rows, filter, view, tall.to_bits(), nowhere).hash(state);
                crate::station::thumbs::changes().hash(state);
            }
            Block::Shelf { tiles, shows, tall, said, over } => {
                (tiles, shows, tall.to_bits(), said, over).hash(state);
                crate::station::thumbs::changes().hash(state);
            }
            Block::Copies { rows, filter, view, tall } => {
                (rows, filter, view, tall.to_bits()).hash(state);
                crate::station::thumbs::changes().hash(state);
            }
        }
    }
}

/// How much of its module a lane shows (see `arrange::Form`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Detail {
    /// All of it.
    #[default]
    Full,
    /// Its figure and chart, and what belongs with them (a card's video
    /// memory, its volumes, a few processes); none of the rows below.
    Compact,
    /// A workbench lane in a widget too small for it all: its things in a shape of their own (see `Shape`).
    Shape(Shape),
}

/// What a workbench lane is drawn as in a widget too small for all of it: a strip across (a row of its things), a
/// strip down (a column of them), or the least (one, and how many there are).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Shape {
    Across,
    Down,
    Least,
}

impl Shape {
    /// How a widget of `lanes` workbench lanes in shape `self` sets them (how many columns): across, side by side,
    /// down, one over another, the least, as near a square as they go.
    pub fn columns(self, lanes: usize) -> usize {
        match self {
            Shape::Across => lanes.max(1),
            Shape::Down => 1,
            Shape::Least => ((lanes as f32).sqrt().ceil() as usize).max(1),
        }
    }

    /// Its name, as a form is saved by (see `arrange::Form::key`).
    pub fn key(self) -> &'static str {
        match self {
            Shape::Across => "across",
            Shape::Down => "down",
            Shape::Least => "least",
        }
    }

    /// The shape a workbench lane takes in a widget `size` (DIPs) too small for all of it: a strip down where it is
    /// clearly taller than wide, across where clearly wider than tall, else the least.
    pub fn of(size: (f32, f32)) -> Shape {
        let (w, h) = size;
        if h >= w * 1.3 && h >= 90.0 {
            Shape::Down
        } else if w >= h * 1.6 && w >= 120.0 {
            Shape::Across
        } else {
            Shape::Least
        }
    }
}

pub struct Lane {
    /// Its module's (what names its parts, see `morph`).
    id: String,
    blocks: Vec<Block>,
    ink: Ink,
}

impl Lane {
    /// Its module's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Whether it draws a chart, which creeps along as time goes.
    fn charts(&self) -> bool {
        self.blocks.iter().any(|block| matches!(block, Block::Readout { plot: Some(_), .. } | Block::Rates { plot: Some(_), .. } | Block::Trace { .. }))
    }
}

impl Hash for Lane {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.blocks.hash(state);
    }
}

/// The game in `s`: one found, or the window in front taken for one by hand.
fn found_game(s: &Sample) -> Option<&crate::reading::GameSample> {
    s.game.as_ref()
}

/// The game in `s`, if it is the process `pid`: a chart of a game's frames
/// is of that game's alone, not of another's before it.
fn same_game(s: &Sample, pid: Option<u32>) -> Option<&crate::reading::GameSample> {
    found_game(s).filter(|game| Some(game.pid) == pid)
}

/// (see `presents::GameMode`).
pub fn front_is_game() -> bool {
    #[cfg(windows)]
    return crate::presents::game_mode() == crate::presents::GameMode::On;
    #[cfg(not(windows))]
    false
}

/// Whether a game shows (the game button lit): one found and not turned off, or one asked for by hand.
pub fn game_shown(history: &[Sample]) -> bool {
    if front_is_game() {
        true
    } else if game_hidden() {
        false
    } else {
        history.last().is_some_and(|s| s.game.is_some())
    }
}

/// Whether games are turned off by hand: none shows, found or not.
pub fn game_hidden() -> bool {
    #[cfg(windows)]
    return crate::presents::game_mode() == crate::presents::GameMode::Off;
    #[cfg(not(windows))]
    false
}

/// Whether a module's lane shows: switched on; or the game's, asked for by hand (the game button pressed: the
/// window in front taken for a game) though it is switched off.
pub fn shows_module(entry: &super::prefs::ModuleEntry) -> bool {
    entry.on || (entry.id == "game" && front_is_game())
}

/// What a game asked for by hand says before its first frames.
pub fn waiting(lang: Lang) -> String {
    if front_is_game() { lang.pick("等待画面", "No frames yet").into() } else { String::new() }
}

/// The lanes of the modules that are on and have something to show, all
/// of each.
pub fn lanes(scene: &Scene) -> Vec<Lane> {
    lanes_at(scene, |_| Detail::Full)
}

impl Lane {
    pub fn height(&self, theme: &Theme) -> f32 {
        theme.pad_top + theme.pad_bottom + self.blocks.iter().map(Block::height).sum::<f32>()
    }
}

/// The lanes of the modules that are on and have something to show, each
/// showing as much as `detail` says for it.
pub fn lanes_at(scene: &Scene, detail: impl Fn(&str) -> Detail) -> Vec<Lane> {
    scene
        .prefs
        .modules
        .iter()
        .filter(|entry| shows_module(entry))
        .filter_map(|entry| Some(Lane { id: entry.id.clone(), blocks: lane(scene, &entry.id, detail(&entry.id))?, ink: scene.theme.ink(&entry.id) }))
        .collect()
}

fn head(title: impl Into<String>, device: impl Into<String>, aside: impl Into<String>, aside_hot: bool) -> Block {
    Block::Head { title: title.into(), device: device.into(), aside: aside.into(), aside_hot }
}

fn celsius(value: f32) -> String {
    format!("{value:.0} °C")
}

/// What a reading shows while it is not there to read.
const UNREAD: &str = "—";

/// A reading as a fact's value: as `show` writes it, or "—".
fn shown<T>(value: Option<T>, show: impl Fn(T) -> String) -> String {
    value.map_or_else(|| UNREAD.to_string(), show)
}

/// Each lane's module and its height at every detail (see
/// `arrange::Heights`), for the modules that are on and have something to
/// show.
pub fn heights(scene: &Scene) -> Vec<(String, [f32; 2])> {
    let height = |id: &str, detail: Detail| lane(scene, id, detail).map(|blocks| scene.theme.pad_top + scene.theme.pad_bottom + blocks.iter().map(Block::height).sum::<f32>());
    scene
        .prefs
        .modules
        .iter()
        .filter(|entry| shows_module(entry))
        .filter_map(|entry| Some((entry.id.clone(), [height(&entry.id, Detail::Full)?, height(&entry.id, Detail::Compact)?])))
        .collect()
}

/// How much shorter than designed each lane of `heights` may be drawn (its list at its foot given way to
/// `LIST_LEAST`; none for a lane with no list at its foot), in the same order (see `Layout::giving`).
pub fn gives(scene: &Scene) -> Vec<f32> {
    scene
        .prefs
        .modules
        .iter()
        .filter(|entry| shows_module(entry))
        .filter_map(|entry| {
            let blocks = lane(scene, &entry.id, Detail::Full)?;
            Some(blocks.last().and_then(Block::list_tall).map_or(0.0, |tall| (tall - LIST_LEAST).max(0.0)))
        })
        .collect()
}

/// What each of a column's lanes is drawn taller (or, less than 0, shorter) by, the column made `change` taller than
/// its lanes need, each lane able to be drawn shorter by its `gives`: the lanes with a list at their foot (some give)
/// take it, made shorter as evenly as each can give; the others are drawn as designed (their readings and controls
/// fill them as they are: drawn shorter, they were drawn over the lane under). Only with none to take it, or made
/// shorter than they all give (smaller than its least: a widget's lanes take shapes then, see `widget`), is it shared
/// by all.
fn shares(gives: &[f32], change: f32) -> Vec<f32> {
    let count = gives.len();
    let mut takers: Vec<usize> = (0..count).filter(|&at| gives[at] > 0.0).collect();
    let mut shares = vec![0.0; count];
    if takers.is_empty() {
        return vec![change / count.max(1) as f32; count];
    }
    if change >= 0.0 {
        for &at in &takers {
            shares[at] = change / takers.len() as f32;
        }
        return shares;
    }
    // Shorter: the least giving give all they can first, the rest an even share of what is left.
    takers.sort_by(|a, b| gives[*a].total_cmp(&gives[*b]));
    let mut left = -change;
    for (done, &at) in takers.iter().enumerate() {
        let taken = (left / (takers.len() - done) as f32).min(gives[at]);
        shares[at] = -taken;
        left -= taken;
    }
    if left > 0.0 {
        for share in &mut shares {
            *share -= left / count as f32;
        }
    }
    shares
}

/// The height of the lane of module `id` showing `detail`, holding what
/// `seen` holds; `None` where there is no such lane.
pub fn lane_height(scene: &Scene, id: &str, seen: &Seen, detail: Detail) -> Option<f32> {
    let scene = Scene { seen, ..*scene };
    let blocks = lane(&scene, id, detail)?;
    Some(scene.theme.pad_top + scene.theme.pad_bottom + blocks.iter().map(Block::height).sum::<f32>())
}

/// What a board's sensor `name` of `group` ("temps" or "fans") is called:
/// by what it is wired to, where the board is known; else by its number.
pub fn board_sensor(name: &str, group: &str, lang: Lang) -> String {
    if name.parse::<u32>().is_ok() {
        let numbered = if group == "fans" { ("风扇", "Fan") } else { ("传感器", "Sensor") };
        format!("{} {name}", lang.pick(numbered.0, numbered.1))
    } else {
        lang.name(name)
    }
}

/// The blocks of module `id`'s lane: all of it, or (`Compact`) none of the
/// rows below its figure and chart.
fn lane(scene: &Scene, id: &str, detail: Detail) -> Option<Vec<Block>> {
    let (s, prefs, lang, info) = (scene.latest(), scene.prefs, scene.lang, scene.info);
    let hot_load = super::prefs::HOT_LOAD;
    let hot_temp = prefs.hot_temp;
    let full = detail == Detail::Full;
    // Whether the module's item is switched on.
    let on = |item: &str| prefs.shows(id, item);
    let chart = on("chart");
    let readout = |value: Option<f32>, read: Series| Block::Readout {
        figure: shown(value, |value| format!("{value:.0}")),
        unit: "%",
        hot: value.is_some_and(|value| value > hot_load),
        plot: chart.then(|| Plot::new(scene, vec![read], Some(100.0), Some(hot_load as f64))),
    };
    // A workbench lane in a shape of its own (see `Shape`).
    // (A note stuck on the desktop alone is its page whatever its size: it has none.)
    if let (Detail::Shape(_), Some(what)) = (detail, bench(scene, id)) {
        return Some(vec![Block::Bench(what)]);
    }
    Some(match id {
        "cpu" => {
            let sensors = s.cpu_sensors.as_ref();
            let temp = sensors.and_then(|c| c.temp);
            let had_temp = on("temp") && scene.seen.cpu_temp;
            let clock = s.ghz.map(|ghz| format!("{ghz:.2} GHz"));
            let had_clock = on("clock") && scene.seen.cpu_clock;
            // The temperature takes the corner, as on the GPU lanes; the
            // clock, the power and each chiplet's temperature go below.
            let aside = if had_temp {
                shown(temp, celsius)
            } else if had_clock {
                clock.clone().unwrap_or_else(|| UNREAD.into())
            } else {
                String::new()
            };
            let mut facts = Vec::new();
            // Cores of more than one kind: each kind's clock, the big first.
            let kinds = &s.kinds_ghz;
            if on("clock") && kinds.len() > 1 {
                let names: &[(&str, &str)] = if kinds.len() == 2 { &[("大核频率", "P-core clock"), ("小核频率", "E-core clock")] } else { &[("大核频率", "P-core clock"), ("小核频率", "E-core clock"), ("低功耗核频率", "LP E-core clock")] };
                for (ghz, (zh, en)) in kinds.iter().zip(names) {
                    facts.push((lang.pick(zh, en).into(), shown(*ghz, |ghz| format!("{ghz:.2} GHz")), false));
                }
            } else if had_temp && had_clock {
                facts.push((lang.pick("频率", "Clock").into(), clock.unwrap_or_else(|| UNREAD.into()), false));
            }
            if on("power") && scene.seen.cpu_power {
                facts.push((lang.pick("功耗", "Power").into(), shown(sensors.and_then(|c| c.power), |power| format!("{power:.1} W")), false));
            }
            // Its fans, of the board's seen turning (a header with none on it reads 0), as switched: its own by default
            // (see `prefs::CPU_FAN`). One alone, as "Fan"; more, each by its name.
            if let Some(board) = scene.seen.board.as_ref().filter(|_| on("fans")) {
                let fans: Vec<&String> = board.fans.iter().filter(|n| board.turning.contains(n) && on(&format!("fans:{n}"))).collect();
                for n in &fans {
                    let rpm = s.board.as_ref().and_then(|b| b.fans.iter().find(|(read, _)| read == *n)).map(|(_, rpm)| *rpm);
                    let label = if fans.len() == 1 { lang.pick("风扇", "Fan").into() } else { board_sensor(n, "fans", lang) };
                    facts.push((label, shown(rpm, |rpm| format!("{rpm:.0} RPM")), false));
                }
            }
            let chiplets = &scene.seen.ccds;
            if on("ccds") && chiplets.len() > 1 {
                for &ccd in chiplets {
                    let value = sensors.and_then(|c| c.ccds.iter().find(|(read, _)| *read == ccd)).map(|(_, value)| *value);
                    facts.push((format!("CCD {}", ccd + 1), shown(value, celsius), value.is_some_and(|v| v > hot_temp)));
                }
            }
            if !full {
                facts.clear();
            }
            let mut blocks = vec![
                head("CPU", &info.cpu_name, aside, had_temp && temp.is_some_and(|t| t > hot_temp)),
                readout(s.cpu, Box::new(|s| s.cpu.map(f64::from))),
                Block::Facts { rows: facts, gap: 10.0 },
            ];
            let threads = scene.seen.threads;
            if full && on("threads") && threads > 0 {
                let load = |i: usize| s.threads.get(i).copied().flatten();
                // Each one's tag: which thread, its clock and its load.
                let tag = |i: usize| {
                    let ghz = s.threads_ghz.get(i).copied().flatten();
                    let thread = if lang == Lang::Zh { format!("线程 {}", i + 1) } else { format!("Thread {}", i + 1) };
                    match ghz {
                        Some(ghz) => format!("{thread} · {ghz:.2} GHz · {}", shown(load(i), text::percent)),
                        None => format!("{thread} · {}", shown(load(i), text::percent)),
                    }
                };
                blocks.push(Block::Threads((0..threads).map(|i| (load(i).map(|load| load / 100.0), load(i).is_some_and(|load| load > hot_load), tag(i))).collect()));
            }
            blocks
        }
        _ if id.starts_with("gpu:") => {
            let index = info.gpu_of(id)?;
            let gpu = &info.gpus[index];
            let had = scene.seen.gpus.get(index).filter(|seen| seen.present)?;
            let reading = s.gpus.get(index);
            let shows_temp = on("temp") && had.temp;
            let temp = reading.and_then(|g| g.temp).filter(|_| shows_temp);
            let aside = if shows_temp { shown(temp, celsius) } else { String::new() };
            let usage = reading.and_then(|g| g.usage);
            let mut blocks = vec![
                head("GPU", &gpu.name, aside, temp.is_some_and(|t| t > hot_temp)),
                readout(usage, Box::new(move |s| s.gpus.get(index)?.usage.map(f64::from))),
            ];
            if on("vram") {
                let used = reading.and_then(|g| g.mem_used);
                blocks.push(Block::Meter {
                    label: lang.pick("显存", "VRAM").into(),
                    fraction: used.map_or(0.0, |used| used as f32 / gpu.mem_total.max(1) as f32),
                    value: shown(used, |used| text::usage(used, gpu.mem_total)),
                    hot: false,
                    gap: 8.0,
                    opens: None,
                });
            }
            if full && on("engines") {
                // An engine the counters no longer list has nothing running
                // on it; with the engines unread, none is known.
                const SHOWN: [&str; 6] = ["3D", "Copy", "VideoDecode", "VideoEncode", "VideoCodec", "Compute"];
                for kind in had.engines.iter().map(String::as_str).filter(|kind| SHOWN.contains(kind)) {
                    let load = reading.and_then(|g| g.engines.as_ref()).map(|engines| engines.iter().find(|(known, _)| known == kind).map_or(0.0, |(_, load)| *load));
                    blocks.push(Block::Meter {
                        label: lang.name(kind),
                        fraction: load.unwrap_or(0.0) / 100.0,
                        value: shown(load, text::percent),
                        hot: load.is_some_and(|load| load > hot_load),
                        gap: 6.0,
                        opens: None,
                    });
                }
            }
            let mut facts = Vec::new();
            // (The warmest point of the chip, on the cards that say it apart: red as the temperature is.)
            if on("hotspot") && had.hot_spot {
                let hot_spot = reading.and_then(|g| g.hot_spot);
                facts.push((lang.pick("热点温度", "Hot spot").into(), shown(hot_spot, celsius), hot_spot.is_some_and(|t| t > scene.prefs.hot_temp)));
            }
            if on("clock") && had.clock {
                facts.push((lang.pick("频率", "Clock").into(), shown(reading.and_then(|g| g.clock_mhz), |clock| format!("{clock:.0} MHz")), false));
            }
            if on("power") && had.power {
                facts.push((lang.pick("功耗", "Power").into(), shown(reading.and_then(|g| g.power), |power| format!("{power:.1} W")), false));
            }
            if on("fan") && had.fan {
                facts.push((lang.pick("风扇", "Fan").into(), shown(reading.and_then(|g| g.fan_rpm), |rpm| format!("{rpm} RPM")), false));
            }
            // Memory the card borrows from the system's, where it has its own
            // besides (not where all of it is the system's, as on a Mac).
            if on("shared") && gpu.shared_total > 0 {
                facts.push((lang.pick("共享显存", "Shared").into(), shown(reading.and_then(|g| g.shared_used), |used| text::usage(used, gpu.shared_total)), false));
            }
            if full {
                blocks.push(Block::Facts { rows: facts, gap: 10.0 });
            }
            blocks
        }
        "memory" => {
            let total = info.mem_total.max(1) as f64;
            let percent = (s.memory.used as f64 / total * 100.0) as f32;
            let mut facts = Vec::new();
            // What it runs at, and what it is rated for where that is more.
            if let Some((speed, rated)) = info.memory_speed.filter(|_| on("speed")) {
                let value = match rated {
                    Some(rated) if lang == Lang::Zh => format!("{speed} MT/s（额定 {rated}）"),
                    Some(rated) => format!("{speed} MT/s (rated {rated})"),
                    None => format!("{speed} MT/s"),
                };
                facts.push((lang.pick("频率", "Speed").into(), value, false));
            }
            // Each module's own temperature, as each chiplet's on the CPU lane.
            for i in 0..if on("dimms") { scene.seen.dimms } else { 0 } {
                let name = if lang == Lang::Zh { format!("内存条 {}", i + 1) } else { format!("Module {}", i + 1) };
                let value = s.dimm_temps.get(i).copied();
                facts.push((name, shown(value, celsius), value.is_some_and(|v| v > hot_temp)));
            }
            if on("committed") {
                facts.push((lang.pick("已提交", "Committed").into(), text::usage(s.memory.committed, s.memory.commit_limit), false));
            }
            if on("cached") {
                facts.push((lang.pick("缓存", "Cached").into(), text::size(s.memory.cached), false));
            }
            if !full {
                facts.clear();
            }
            vec![
                head(lang.pick("内存", "Memory"), info.memory_modules.clone().unwrap_or_default(), text::usage(s.memory.used, info.mem_total), false),
                Block::Readout {
                    figure: format!("{percent:.0}"),
                    unit: "%",
                    hot: percent > hot_load,
                    plot: chart.then(|| Plot::new(scene, vec![Box::new(move |s| Some(s.memory.used as f64 / total * 100.0))], Some(100.0), Some(hot_load as f64))),
                },
                Block::Facts { rows: facts, gap: 10.0 },
            ]
        }
        "network" => {
            let bits = prefs.network.bits;
            let plot = Plot::new(scene, vec![Box::new(|s| s.net_down), Box::new(|s| s.net_up)], None, None);
            let scale = plot.max;
            let mut facts = Vec::new();
            let adapter = s.network.as_ref();
            if on("adapter") {
                facts.push((lang.pick("网卡", "Adapter").into(), adapter.map_or(lang.pick("未连接", "Not connected").into(), |a| a.model.clone()), false));
            }
            if on("wifi") && scene.seen.wifi {
                facts.push(("Wi-Fi".into(), shown(adapter.and_then(|a| a.wifi.clone()), |name| name), false));
            }
            if on("address") && scene.seen.address {
                facts.push((lang.pick("地址", "Address").into(), shown(adapter.and_then(|a| a.ipv4.clone()), |ip| ip), false));
            }
            let link = |s: &Sample| s.network.as_ref().map(|a| a.link_bps).filter(|bps| *bps > 0);
            if on("link") && scene.seen.link {
                facts.push((lang.pick("链路", "Link").into(), shown(link(s), text::link_speed), false));
            }
            if on("latency") {
                facts.push((lang.pick("延迟", "Latency").into(), shown(s.ping_ms, |ms| format!("{ms:.0} ms")), false));
            }
            if on("totals") {
                let (down, up) = (text::bytes(s.net_total_down as f64), text::bytes(s.net_total_up as f64));
                let total = if lang == Lang::Zh { format!("下载 {down}，上传 {up}") } else { format!("{down} down, {up} up") };
                facts.push((lang.pick("开机以来", "Since boot").into(), total, false));
            }
            if !full {
                facts.clear();
            }
            vec![
                // The network it is on (the wireless network's name, else the connection's), by the adapter traffic
                // leaves by now, which may have changed.
                head(lang.pick("网络", "Network"), adapter.map_or(lang.pick("未连接", "Not connected").into(), |a| a.wifi.clone().unwrap_or_else(|| a.name.clone())), String::new(), false),
                Block::Rates {
                    rows: vec![
                        (lang.pick("下载", "Down").into(), shown(s.net_down, |rate| text::rate(rate, bits))),
                        (lang.pick("上传", "Up").into(), shown(s.net_up, |rate| text::rate(rate, bits))),
                    ],
                    plot: chart.then_some(plot),
                    scale: if chart { text::rate(scale, bits) } else { String::new() },
                },
                Block::Facts { rows: facts, gap: 10.0 },
            ]
        }
        "disk" => {
            let plot = Plot::new(scene, vec![Box::new(|s| s.disk_read), Box::new(|s| s.disk_write)], None, None);
            let scale = plot.max;
            let drives: &[(u32, String)] = if on("drives") { &scene.seen.drives } else { &[] };
            let temp = |id: u32| s.drive_temps.iter().find(|d| d.id == id).map(|d| d.celsius);
            let hottest = drives.iter().filter_map(|(id, _)| temp(*id)).fold(None, |max: Option<f32>, t| Some(max.map_or(t, |m| m.max(t))));
            // A drive's temperature takes the corner, as a GPU's does.
            let mut facts: Vec<(String, String, bool)> = Vec::new();
            // More than one drive: each on a line of its own, by name, with
            // its temperature (unread, for one that gives none).
            // (Those the machine started with, and any that has given a
            // temperature since: one plugged in later.)
            let mut all: Vec<(u32, &String)> = info.drive_ids.iter().copied().zip(&info.drives).collect();
            all.extend(drives.iter().filter(|(id, _)| !info.drive_ids.contains(id)).map(|(id, name)| (*id, name)));
            let every = if on("drives") { all.len() } else { 0 };
            if every > 1 {
                for (id, name) in &all {
                    let value = temp(*id);
                    facts.push(((*name).clone(), shown(value, celsius), value.is_some_and(|t| t > hot_temp)));
                }
            }
            if on("active") {
                facts.push((lang.pick("活动时间", "Active time").into(), shown(s.disk_active, text::percent), false));
            }
            // (Its drives' hottest at the corner, where they are shown.)
            let aside = if drives.is_empty() { String::new() } else { shown(hottest, celsius) };
            if !full {
                facts.clear();
            }
            vec![
                // Named below, where there are several.
                head(lang.pick("磁盘", "Disk"), if every > 1 { if lang == Lang::Zh { format!("{every} 块硬盘") } else { format!("{every} drives") } } else { info.drives.join(", ") }, aside, hottest.is_some_and(|t| t > hot_temp)),
                Block::Rates {
                    rows: vec![
                        (lang.pick("读取", "Read").into(), shown(s.disk_read, |rate| text::rate(rate, false))),
                        (lang.pick("写入", "Write").into(), shown(s.disk_write, |rate| text::rate(rate, false))),
                    ],
                    plot: chart.then_some(plot),
                    scale: if chart { text::rate(scale, false) } else { String::new() },
                },
                Block::Facts { rows: facts, gap: 10.0 },
            ]
        }
        "processes" => {
            let sort = prefs.processes.sort;
            let mut ranked: Vec<&ProcessSample> = s.processes.iter().collect();
            ranked.sort_by(|a, b| sort_value(b, sort).total_cmp(&sort_value(a, sort)));
            vec![head(lang.pick("进程", "Processes"), "", "", false), Block::Table {
                headings: vec![
                    ("CPU".into(), Some(ProcessSort::Cpu)),
                    (lang.pick("内存", "Memory").into(), Some(ProcessSort::Memory)),
                    (lang.pick("读写", "I/O").into(), Some(ProcessSort::Io)),
                    ("GPU".into(), Some(ProcessSort::Gpu)),
                ],
                sort: Some(sort),
                scrolls: true,
                widths: PROCESS_COLUMNS,
                rows: ranked
                    .iter()
                    .map(|p| [p.name.clone(), text::percent(p.cpu), text::size(p.mem), text::rate(p.io, false), shown(p.gpu, text::percent)])
                    .collect(),
                visible: if full { prefs.processes.count } else { prefs.processes.count.min(COMPACT_PROCESSES) },
            }]
        }
        "storage" => {
            let mut blocks = vec![head(lang.pick("存储", "Storage"), "", "", false)];
            // The drives switched on (see `Prefs::shows`); none, no lane.
            let drives: Vec<&String> = scene.seen.volumes.iter().filter(|name| on(&format!("volumes:{name}"))).collect();
            if drives.is_empty() {
                return None;
            }
            for (i, name) in drives.into_iter().enumerate() {
                let volume = s.volumes.iter().find(|v| v.name == *name);
                let fraction = volume.map_or(0.0, |v| v.used as f32 / v.total.max(1) as f32);
                blocks.push(Block::Meter {
                    label: name.clone(),
                    fraction,
                    value: shown(volume, |v| text::usage(v.used, v.total)),
                    hot: fraction * 100.0 > hot_load,
                    gap: if i == 0 { 0.0 } else { 8.0 },
                    opens: scene.seen.volumes.iter().any(|seen| seen == name).then(|| name.chars().next()).flatten(),
                });
            }
            blocks
        }
        "board" => {
            let had = scene.seen.board.as_ref()?;
            let board = s.board.as_ref();
            let named = |name: &str, group: &str| board_sensor(name, group, lang);
            // Each sensor shown as it is switched (see `Prefs::shows`).
            let temps: Vec<&String> = had.temps.iter().filter(|n| on(&format!("temps:{n}"))).collect();
            let fans: Vec<&String> = had.fans.iter().filter(|n| on(&format!("fans:{n}"))).collect();
            let value = |list: Option<&Vec<(String, f32)>>, name: &str| list.and_then(|list| list.iter().find(|(n, _)| n == name)).map(|(_, v)| *v);
            vec![
                head(lang.pick("主板", "Motherboard"), &info.board, "", false),
                Block::Facts {
                    rows: temps
                        .iter()
                        .map(|n| {
                            let t = value(board.map(|b| &b.temps), n);
                            (named(n, "temps"), shown(t, celsius), t.is_some_and(|t| t > hot_temp))
                        })
                        .collect(),
                    gap: 0.0,
                },
                Block::Facts {
                    rows: fans
                        .iter()
                        .filter(|_| full)
                        .map(|n| (named(n, "fans"), shown(value(board.map(|b| &b.fans), n), |rpm| format!("{rpm:.0} RPM")), false))
                        .collect(),
                    gap: 10.0,
                },
            ]
        }
        "game" => {
            // A game played, or asked for by hand (waiting for its frames).
            // (In the panel as in the widgets: the game button is the same everywhere.)
            let by_hand = front_is_game();
            if !(scene.seen.game || by_hand) || game_hidden() {
                return None;
            }
            // In the panel, only a game found (one taken for a game by hand is
            // the widgets' own).
            let game = found_game(s);
            let pid = game.map(|g| g.pid);
            // Its screen's refresh rate in the corner: the most frames it can show.
            let refresh = game.and_then(|g| g.refresh_hz);
            // A stutter: a frame taking as long as three at the screen's rate.
            let stutter = refresh.map(|hz| 3000.0 / hz as f64);
            let longest = game.and_then(|g| g.longest_ms);
            let frametimes = on("frametimes");
            let mut facts = Vec::new();
            if on("low") {
                facts.push(("1% low".into(), shown(game.and_then(|g| g.low), |low| format!("{low:.0} FPS")), false));
            }
            // Beside its chart, with that on.
            if on("longest") && !frametimes {
                let hot = longest.zip(stutter).is_some_and(|(ms, stutter)| ms as f64 > stutter);
                facts.push((lang.pick("最长一帧", "Longest frame").into(), shown(longest, |ms| format!("{ms:.1} ms")), hot));
            }
            if on("usage") {
                let cpu = shown(game.and_then(|g| g.cpu), text::percent);
                let gpu = shown(game.and_then(|g| g.gpu), text::percent);
                facts.push((lang.pick("占用", "Use").into(), format!("CPU {cpu} · GPU {gpu}"), false));
            }
            if on("memory") {
                let memory = shown(game.and_then(|g| g.mem), text::size);
                let vram = shown(game.and_then(|g| g.vram), text::size);
                let value = if lang == Lang::Zh { format!("{memory} · 显存 {vram}") } else { format!("{memory} · VRAM {vram}") };
                facts.push((lang.pick("内存", "Memory").into(), value, false));
            }
            if on("limit") && scene.seen.game_limit {
                let limit = game.and_then(|g| g.gpu_limit);
                let value = shown(limit, |limit| {
                    match limit {
                        GpuLimit::Free => lang.pick("未受限", "Not limited"),
                        GpuLimit::Power => lang.pick("功耗墙", "Power limit"),
                        GpuLimit::Thermal => lang.pick("温度墙", "Thermal limit"),
                        GpuLimit::Hardware => lang.pick("硬件降频", "Slowed by board"),
                    }
                    .into()
                });
                facts.push((lang.pick("显卡", "GPU").into(), value, limit.is_some_and(|limit| matches!(limit, GpuLimit::Thermal | GpuLimit::Hardware))));
            }
            if on("time") {
                facts.push((lang.pick("已玩", "Played").into(), shown(game.map(|g| g.playing_s), |s| lang.duration(s)), false));
            }
            if on("mic") && scene.seen.mic {
                let muted = s.mic_muted;
                let value = shown(muted, |muted| if muted { lang.pick("已静音", "Muted") } else { lang.pick("开启", "On") }.into());
                facts.push((lang.pick("麦克风", "Microphone").into(), value, muted == Some(true)));
            }
            // Full scale: the screen's rate, or the most frames drawn, if more
            // (a game not held to the screen's rate draws frames it never shows).
            let most = scene.history.iter().filter_map(|s| same_game(s, pid).and_then(|g| g.fps).map(f64::from)).fold(0.0, f64::max);
            let scale = refresh.map(f64::from).map(|hz| hz.max(most));
            let mut blocks = vec![
                head(lang.pick("游戏", "Game"), game.map_or_else(|| if by_hand { waiting(lang) } else { String::new() }, |g| g.name.clone()), shown(refresh, |hz| lang.pick(&format!("屏幕 {hz} Hz"), &format!("Screen {hz} Hz")).to_string()), false),
                Block::Readout {
                    figure: shown(game.and_then(|g| g.fps), |fps| format!("{fps:.0}")),
                    unit: "FPS",
                    hot: false,
                    plot: chart.then(|| Plot::new(scene, vec![Box::new(move |s| same_game(s, pid).and_then(|g| g.fps).map(f64::from))], scale, None)),
                },
            ];
            if !full {
                facts.clear();
            }
            if full && frametimes {
                // Full scale: two frames at the screen's rate, or the longest
                // frame on screen, if longer, rounded up.
                let series: Series = Box::new(move |s| same_game(s, pid).and_then(|g| g.longest_ms).map(f64::from));
                let floor = stutter.map_or(MIN_FRAME_SCALE, |stutter| (stutter * 2.0 / 3.0).max(MIN_FRAME_SCALE));
                let peak = visible_peak(scene, &series).max(floor);
                blocks.push(Block::Trace {
                    label: lang.pick("帧时间", "Frame time").into(),
                    value: shown(longest, |ms| format!("{ms:.1} ms")),
                    hot: longest.zip(stutter).is_some_and(|(ms, stutter)| ms as f64 > stutter),
                    plot: Plot { series: vec![series], max: round_up(peak), hot: stutter },
                    gap: 10.0,
                });
            }
            blocks.push(Block::Facts { rows: facts, gap: 10.0 });
            blocks
        }
        "battery" => {
            if !scene.seen.battery {
                return None;
            }
            let battery = s.battery.as_ref();
            let state = match battery {
                Some(battery) if battery.charging => lang.pick("正在充电", "Charging").to_string(),
                Some(battery) if battery.plugged => lang.pick("已接通电源", "Plugged in").to_string(),
                Some(battery) => match battery.seconds_left {
                    Some(left) => {
                        let left = lang.duration(left as u64);
                        if lang == Lang::Zh { format!("剩余 {left}") } else { format!("{left} left") }
                    }
                    None => lang.pick("使用电池", "On battery").into(),
                },
                None => UNREAD.into(),
            };
            // In, charging; out, what the whole machine draws.
            let power = |watts: f32| match watts {
                w if w > 0.05 => format!("{} {w:.1} W", lang.pick("充电", "In")),
                w if w < -0.05 => format!("{} {:.1} W", lang.pick("耗电", "Out"), -w),
                _ => "0 W".to_string(),
            };
            let mut facts = Vec::new();
            if scene.seen.battery_power && on("power") {
                facts.push((lang.pick("功率", "Power").to_string(), shown(battery.and_then(|b| b.watts), power), false));
            }
            if scene.seen.battery_health && on("health") {
                facts.push((lang.pick("健康度", "Health").to_string(), shown(battery.and_then(|b| b.health), |h| format!("{h:.0}%")), false));
            }
            if !full {
                facts.clear();
            }
            vec![
                head(lang.pick("电池", "Battery"), "", state, false),
                Block::Readout {
                    figure: shown(battery, |b| b.percent.to_string()),
                    unit: "%",
                    hot: battery.is_some_and(|b| !b.plugged && b.percent <= 20),
                    plot: chart.then(|| Plot::new(scene, vec![Box::new(|s| s.battery.as_ref().map(|b| b.percent as f64))], Some(100.0), None)),
                },
                Block::Facts { rows: facts, gap: 8.0 },
            ]
        }
        "docker" => {
            use crate::reading::DockerSample;
            // Only while its engine runs (see `Seen::docker`).
            if !scene.seen.docker {
                return None;
            }
            let (context, state, mut containers) = match &s.docker {
                Some(DockerSample::Stopped { context }) => (context.clone(), lang.pick("未运行", "Not running").to_string(), Vec::new()),
                Some(DockerSample::Unsupported { context, .. }) => (context.clone(), lang.pick("不支持这种连接", "Connection not supported").to_string(), Vec::new()),
                Some(DockerSample::Running { context, containers }) => {
                    let count = containers.len();
                    (context.clone(), if lang == Lang::Zh { format!("{count} 个容器") } else if count == 1 { "1 container".into() } else { format!("{count} containers") }, containers.iter().collect())
                }
                None => (String::new(), UNREAD.to_string(), Vec::new()),
            };
            containers.sort_by(|a, b| b.cpu.unwrap_or(0.0).total_cmp(&a.cpu.unwrap_or(0.0)));
            vec![
                // The head row holds the table's headings: the engine and
                // what it is doing go under the list.
                head("Docker", "", "", false),
                Block::Table {
                    headings: vec![
                        ("CPU".into(), None),
                        (lang.pick("内存", "Memory").into(), None),
                        (lang.pick("网络", "Network").into(), None),
                        (lang.pick("读写", "I/O").into(), None),
                    ],
                    sort: None,
                    rows: containers
                        .iter()
                        .map(|c| [c.name.clone(), shown(c.cpu, text::percent), shown(c.mem, text::size), shown(c.net, |rate| text::rate(rate, false)), shown(c.io, |rate| text::rate(rate, false))])
                        .collect(),
                    // A row for each of the most there have been at once (an
                    // open panel is laid out for those), as many as the
                    // processes' list shows at most.
                    visible: scene.seen.containers.min(if full { prefs.processes.count } else { prefs.processes.count.min(COMPACT_PROCESSES) }),
                    scrolls: false,
                    widths: CONTAINER_COLUMNS,
                },
                Block::Facts { rows: vec![(if context.is_empty() { "Docker".into() } else { context }, state, false)], gap: 6.0 },
            ]
        }
        "wsl" => {
            use crate::reading::WslSample;
            // Only while its machine runs (see `Seen::wsl`); stopped while the panel is up, it keeps its rows,
            // unread, so the lane does not change size.
            if !scene.seen.wsl {
                return None;
            }
            let (state, distros, cpu, used, total, gpu) = match &s.wsl {
                Some(WslSample::Missing) => (lang.pick("未安装", "Not installed"), None, None, None, None, None),
                Some(WslSample::Stopped) => (lang.pick("已停止", "Stopped"), None, None, None, None, None),
                Some(WslSample::Running { distros, cpu, used, total, gpu }) => (lang.pick("运行中", "Running"), distros.as_ref(), *cpu, *used, *total, *gpu),
                None => (UNREAD, None, None, None, None, None),
            };
            let names = match distros {
                Some(names) if names.is_empty() => lang.pick("没有发行版在运行", "No distribution running").to_string(),
                Some(names) => names.join(", "),
                None => String::new(),
            };
            // Its processors in the figure (named, as the lane is not theirs
            // alone), its memory and its GPU charted under them.
            let mut figure = readout(cpu, Box::new(|s| match &s.wsl {
                Some(crate::reading::WslSample::Running { cpu, .. }) => cpu.map(f64::from),
                _ => None,
            }));
            if let Block::Readout { unit, .. } = &mut figure {
                *unit = "% CPU";
            }
            let mut blocks = vec![head("WSL", if on("distros") { names } else { String::new() }, state, false), figure];
            if on("memory") {
                let both = used.zip(total);
                let value = shown(both, |(used, total)| text::usage(used, total));
                let series: Series = Box::new(|s| match &s.wsl {
                    Some(crate::reading::WslSample::Running { used: Some(used), total: Some(total), .. }) => Some(*used as f64 / (*total).max(1) as f64 * 100.0),
                    _ => None,
                });
                if chart {
                    blocks.push(Block::Trace { label: lang.pick("内存", "Memory").into(), value, hot: false, plot: Plot::new(scene, vec![series], Some(100.0), None), gap: 10.0 });
                } else {
                    blocks.push(Block::Meter {
                        label: lang.pick("内存", "Memory").into(),
                        fraction: both.map_or(0.0, |(used, total)| used as f32 / total.max(1) as f32),
                        value,
                        hot: false,
                        gap: 8.0,
                        opens: None,
                    });
                }
            }
            // The GPU as the memory: charted with the usage chart on.
            if on("gpu") {
                let value = shown(gpu, text::percent);
                let hot = gpu.is_some_and(|gpu| gpu > hot_load);
                if chart {
                    let series: Series = Box::new(|s| match &s.wsl {
                        Some(crate::reading::WslSample::Running { gpu, .. }) => gpu.map(f64::from),
                        _ => None,
                    });
                    blocks.push(Block::Trace { label: "GPU".into(), value, hot, plot: Plot::new(scene, vec![series], Some(100.0), Some(hot_load as f64)), gap: 10.0 });
                } else {
                    blocks.push(Block::Meter { label: "GPU".into(), fraction: gpu.unwrap_or(0.0) / 100.0, value, hot, gap: 8.0, opens: None });
                }
            }
            blocks
        }
        // The time of day, and the day under its name, as the sample has them.
        // With it, what else is the PC's own to set at a glance: how loud the sound is, and what plays (as last
        // looked at: see `playing`).
        "time" => {
            let now = text::Local::at(s.t);
            let (figure, unit) = now.time(text::twelve_hours(), lang);
            let mut blocks = vec![head(lang.pick("时间", "Time"), now.date(lang), "", false), Block::Readout { figure, unit, hot: false, plot: None }];
            // Each of what plays as the panel was laid out for it (see `Seen::note_playing`): not there yet, nothing
            // (drawn as it came, it was drawn over the lane under, the panel laid out before it was known); there and
            // gone a while, its room kept, as a reading's is.
            let seen = scene.seen;
            let heard = crate::playing::now();
            if on("sound") && seen.sound {
                blocks.push(match heard.as_ref().and_then(|heard| heard.volume.map(|level| (level, heard.muted))) {
                    Some((level, muted)) => Block::Sound { level, muted },
                    None => Block::Room(Block::Sound { level: 0, muted: false }.height()),
                });
            }
            if on("media") && seen.media {
                let heard = heard.clone().unwrap_or_default();
                let (title, about) = said_of(&heard, lang);
                blocks.push(Block::Playing { title, about, playing: heard.playing, players: heard.players });
                if seen.timeline {
                    blocks.push(match heard.timeline {
                        Some(timeline) => {
                            let whole = |seconds: f64| seconds.max(0.0).round() as u32;
                            Block::Progress { at: whole(timeline.at).min(whole(timeline.length)), length: whole(timeline.length), seekable: timeline.seekable }
                        }
                        None => Block::Room(Block::Progress { at: 0, length: 0, seekable: false }.height()),
                    });
                }
            }
            // The music's line, while what plays is it (a video playing meanwhile has none); none where none were found.
            if on("lyrics") && seen.lyric {
                let heard = playing_music();
                blocks.push(match sung_now(heard.as_ref(), lang, prefs.shows("lyrics", "translation")).filter(|_| heard.is_some()) {
                    Some((line, under, said)) => Block::Lyric { line, under, said, glide: glide_of(heard.as_ref()) },
                    None => Block::Room(Block::Lyric { line: String::new(), under: String::new(), said: false, glide: Glide::default() }.height()),
                });
            }
            blocks
        }
        "station" => station_lane(full, scene.tab, lang),
        "apps" => apps_lane(full, scene.theme, lang),
        "notes" => notes_lane(full, lang),
        id if id.starts_with(STUCK_NOTE_LANE) => return stuck_note_lane(&id[STUCK_NOTE_LANE.len()..], lang),
        // What plays, in a widget merged with readings: drawn apart (see `Scene::music_apart`), its lane its head and
        // room under it (a player's, full; a strip's, compact).
        "lyrics" if scene.music_apart => {
            let heard = crate::playing::music();
            vec![
                head(lang.pick("音乐", "Music"), heard.as_ref().map_or(String::new(), |heard| heard.app.clone()), "", false),
                Block::Room(if full { MUSIC_ROOM.0 } else { MUSIC_ROOM.1 }),
            ]
        }
        "system" => {
            let rows = [
                ("uptime", lang.pick("开机时长", "Uptime"), lang.duration(s.system.uptime_s)),
                ("processes", lang.pick("进程", "Processes"), s.system.processes.to_string()),
                ("threads", lang.pick("线程", "Threads"), s.system.threads.to_string()),
                ("handles", lang.pick("句柄", "Handles"), s.system.handles.to_string()),
            ];
            vec![
                head(lang.pick("系统", "System"), "", "", false),
                // Past `Full`, the first of them alone.
                Block::Facts {
                    rows: rows
                        .into_iter()
                        .filter(|(item, ..)| on(item))
                        .map(|(_, label, value)| (label.into(), value, false))
                        .take(if full { usize::MAX } else { 1 })
                        .collect(),
                    gap: 0.0,
                },
            ]
        }
        _ => return None,
    })
}

fn sort_value(p: &ProcessSample, sort: ProcessSort) -> f64 {
    match sort {
        ProcessSort::Cpu => p.cpu as f64,
        ProcessSort::Memory => p.mem as f64,
        ProcessSort::Io => p.io,
        // Not read sorts last.
        ProcessSort::Gpu => p.gpu.map_or(-1.0, f64::from),
    }
}

/// Rounds a byte rate up to 1, 2 or 5 times a power of ten of its unit.
fn round_up_rate(value: f64) -> f64 {
    let unit = 1024f64.powf((value.ln() / 1024f64.ln()).floor());
    let magnitude = 10f64.powf((value / unit).log10().floor());
    let leading = value / unit / magnitude;
    let step = if leading <= 1.0 { 1.0 } else if leading <= 2.0 { 2.0 } else if leading <= 5.0 { 5.0 } else { 10.0 };
    step * magnitude * unit
}

/// The most `series` reads in the samples on screen, or 0.
fn visible_peak(scene: &Scene, series: &Series) -> f64 {
    let oldest = scene.pen_ms - scene.prefs.chart_seconds * 1000.0;
    scene.history.iter().filter(|s| s.t as f64 >= oldest).filter_map(series).fold(0.0, f64::max)
}

/// Rounds `value` (above 0) up to 1, 2 or 5 times a power of ten.
fn round_up(value: f64) -> f64 {
    let magnitude = 10f64.powf(value.log10().floor());
    let leading = value / magnitude;
    let step = if leading <= 1.0 { 1.0 } else if leading <= 2.0 { 2.0 } else if leading <= 5.0 { 5.0 } else { 10.0 };
    step * magnitude
}

fn full_scale(scene: &Scene, series: &[Series]) -> f64 {
    let oldest = scene.pen_ms - scene.prefs.chart_seconds * 1000.0;
    let peak = scene
        .history
        .iter()
        .filter(|s| s.t as f64 >= oldest)
        .flat_map(|s| series.iter().filter_map(move |read| read(s)))
        .fold(MIN_RATE_SCALE, f64::max);
    round_up_rate(peak)
}

/// Where each column starts, and the tallest column's height, when the
/// lanes, in order and `gap` apart, are split into `columns` so the tallest
/// is as short as it can be.
pub fn split(heights: &[f32], columns: usize, gap: f32) -> (Vec<usize>, f32) {
    let n = heights.len();
    // With every module switched off, there is only the bar.
    if n == 0 {
        return (vec![0; columns], 0.0);
    }
    let span = |from: usize, to: usize| heights[from..to].iter().sum::<f32>() + (to - from).saturating_sub(1) as f32 * gap;
    let mut best = vec![vec![f32::INFINITY; n + 1]; columns + 1];
    let mut from = vec![vec![0usize; n + 1]; columns + 1];
    best[0][0] = 0.0;
    for k in 1..=columns {
        for i in k..=n {
            for j in (k - 1)..i {
                let candidate = best[k - 1][j].max(span(j, i));
                if candidate < best[k][i] {
                    best[k][i] = candidate;
                    from[k][i] = j;
                }
            }
        }
    }
    let mut cuts = vec![0; columns];
    let mut i = n;
    for k in (1..=columns).rev() {
        cuts[k - 1] = from[k][i];
        i = from[k][i];
    }
    (cuts, best[columns][n])
}

/// A rectangle in DIPs: left, top, width, height.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// The panel laid out: lanes dealt into columns, and the bar below them.
/// Where a panel's rail goes (see `Layout::with_rail`): along its start (the screen's left edge), its end (the right),
/// or under its columns (the top edge's panel).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RailSide {
    Start,
    End,
    Bottom,
}

/// A rail's width beside the columns, and its height under them.
pub const RAIL_W: f32 = 60.0;
pub const RAIL_ROW: f32 = 56.0;
/// The least a rail down a side is tall: its foot's buttons, a thing over them, and the room round them (see
/// `paint_rail`).
pub(crate) const RAIL_LEAST: f32 = RAIL_FOOT + RAIL_CELL + 16.0;

/// How long a rail down a side is to show all of `things` and the place to add more over its foot, with the room round
/// them (see `paint_rail`).
pub(crate) fn rail_length(things: usize) -> f32 {
    RAIL_FOOT + 16.0 + (things + 1) as f32 * RAIL_CELL
}
/// The room a rail's foot takes along it: the room before its two buttons (see `paint_rail`), and them.
const RAIL_FOOT: f32 = RAIL_FOOT_GAP + 2.0 * BUTTON + BUTTON_GAP;
const RAIL_FOOT_GAP: f32 = 8.0;

#[derive(Clone)]
pub struct Layout {
    pub columns: usize,
    pub cuts: Vec<usize>,
    pub heights: Vec<f32>,
    /// The tallest column; every column is drawn this tall.
    pub lanes_height: f32,
    /// How much shorter than in `heights` each lane may be drawn (see `giving`); none, each lane takes an even share of
    /// what its column is made taller or shorter by.
    gives: Vec<f32>,
    outer: (f32, f32, f32),
    lane_gap: f32,
    column_gap: f32,
    bar_gap: f32,
    bar_height: f32,
    /// Each column's width: as designed, or wider (see `stretched`).
    column_width: f32,
    /// How much taller than its lanes need each column was made, for their
    /// charts to grow into (see `stretched`).
    grow: f32,
    /// The lane drawn as a rail instead of in a column (its place among the lanes; `heights` leaves it out), and
    /// where; the panel's bar's buttons are at the rail's foot then, and the bar goes.
    rail: Option<(usize, RailSide)>,
}

impl Layout {
    /// The fewest columns, up to `max_columns`, whose tallest fits in `room`
    /// DIPs (bar included); failing that, the most.
    pub fn new(heights: Vec<f32>, room: f32, max_columns: usize, theme: &Theme) -> Self {
        let most = max_columns.min(heights.len()).max(1);
        let chrome = theme.outer.0 + theme.outer.2 + theme.bar_gap + theme.bar_height;
        let mut chosen = (1, split(&heights, 1, theme.lane_gap));
        for columns in 1..=most {
            chosen = (columns, split(&heights, columns, theme.lane_gap));
            if chosen.1 .1 + chrome <= room {
                break;
            }
        }
        let (columns, (cuts, tallest)) = chosen;
        Layout::built(heights, columns, cuts, tallest, theme)
    }

    /// The lanes in exactly `columns` columns (at least one, and no more
    /// than there are lanes), however tall that makes them.
    pub fn with_columns(heights: Vec<f32>, columns: usize, theme: &Theme) -> Self {
        let columns = columns.min(heights.len()).max(1);
        let (cuts, tallest) = split(&heights, columns, theme.lane_gap);
        Layout::built(heights, columns, cuts, tallest, theme)
    }

    fn built(heights: Vec<f32>, columns: usize, cuts: Vec<usize>, tallest: f32, theme: &Theme) -> Self {
        Layout {
            columns,
            cuts,
            heights,
            lanes_height: tallest,
            gives: Vec::new(),
            outer: theme.outer,
            lane_gap: theme.lane_gap,
            column_gap: theme.column_gap,
            bar_gap: theme.bar_gap,
            bar_height: theme.bar_height,
            column_width: COLUMN_WIDTH,
            grow: 0.0,
            rail: None,
        }
    }

    /// The same with each lane able to be drawn shorter by `gives` (in the order of the lanes, see `gives`): made larger
    /// or smaller than its lanes need (see `fitted`), the lanes with a list at their foot take it (see `shares`).
    pub fn giving(mut self, gives: Vec<f32>) -> Self {
        self.gives = gives;
        self
    }

    /// The least it may be made as tall as, its lanes each as short as it gives (see `giving`), its columns as they are.
    pub fn least_height(&self) -> f32 {
        let least = (0..self.columns)
            .map(|column| {
                let start = self.cuts[column];
                let end = self.cuts.get(column + 1).copied().unwrap_or(self.heights.len());
                (start..end).map(|at| self.heights[at] - self.gives.get(at).copied().unwrap_or(0.0)).sum::<f32>() + (end - start).saturating_sub(1) as f32 * self.lane_gap
            })
            .fold(0.0, f32::max);
        self.height() - self.lanes_height + least
    }

    /// The same with the lane at `at` among the lanes (its height left out of the columns') drawn as a rail at `side`,
    /// down a side `length` long where the columns are shorter.
    pub fn with_rail(mut self, at: usize, side: RailSide, length: f32) -> Self {
        self.rail = Some((at, side));
        self.bar_gap = 0.0;
        self.bar_height = 0.0;
        // (A rail down a side as tall as the columns, and no shorter than it is long nor than its foot and a thing
        // over it: with few lanes, or none, beside it, the columns are drawn as tall.)
        if side != RailSide::Bottom {
            self.lanes_height = self.lanes_height.max(length).max(RAIL_LEAST);
        }
        self
    }

    /// The rail's lane and where it goes, if it has one.
    pub fn rail(&self) -> Option<(usize, RailSide)> {
        self.rail
    }

    /// The room the rail takes beside the columns.
    fn rail_across(&self) -> f32 {
        match self.rail {
            Some((_, RailSide::Start | RailSide::End)) => RAIL_W + self.column_gap,
            _ => 0.0,
        }
    }

    /// The room the rail takes under the columns.
    fn rail_down(&self) -> f32 {
        match self.rail {
            Some((_, RailSide::Bottom)) => RAIL_ROW + self.lane_gap,
            _ => 0.0,
        }
    }

    /// The same, `by` DIPs wider and taller: the width shared out among
    /// the columns, the height given to every column, for its charts (see
    /// `grows`).
    pub fn stretched(mut self, by: (f32, f32)) -> Self {
        self.column_width += by.0.max(0.0) / self.columns as f32;
        self.lanes_height += by.1.max(0.0);
        self.grow = by.1.max(0.0);
        self
    }

    /// The same, as large as `size` (DIPs) whatever it is laid out as: its columns as wide as that leaves them, and as
    /// tall (shorter, its lanes' lists given way: see `paint_lane`). A workbench widget's, laid out again for its size,
    /// not scaled to it.
    pub fn fitted(mut self, size: (f32, f32)) -> Self {
        let chrome = self.outer.0 + self.outer.2 + self.rail_down() + self.bar_gap + self.bar_height;
        self.column_width = (size.0 - self.rail_across() - (self.columns - 1) as f32 * self.column_gap) / self.columns as f32;
        self.lanes_height = size.1 - chrome;
        self.grow = 0.0;
        self
    }

    /// The shape each workbench lane in a shape is drawn in (see `paint_bench`), as the room it is given is; in order of
    /// the lanes.
    pub fn shapes(&self, theme: &Theme) -> Vec<Shape> {
        let (pad_x, pad_top, pad_bottom) = bench_pads(theme);
        let mut lanes = self.lanes();
        if let Some((at, _)) = self.rail {
            lanes.remove(at.min(lanes.len() - 1));
        }
        lanes.iter().map(|lane| Shape::of((lane.w - 2.0 * pad_x, lane.h - pad_top - pad_bottom))).collect()
    }

    /// How what it shows flows in its columns: how many of the station's cards, and of quick launch's things, go
    /// across one (as it is laid out again for another width, these change by steps: see `widget`'s turns).
    pub fn flow(&self, theme: &Theme) -> (usize, usize) {
        let inner = self.column_width - 2.0 * theme.pad_x;
        (cards_across(inner), ((inner / APP_CELL_W) as usize).max(1))
    }

    /// How much each lane's charts may grow, in order of the lanes: its
    /// column's share of what the layout was stretched by.
    pub fn grows(&self) -> Vec<f32> {
        let mut grows: Vec<f32> = (0..self.columns)
            .flat_map(|column| {
                let start = self.cuts[column];
                let end = self.cuts.get(column + 1).copied().unwrap_or(self.heights.len());
                std::iter::repeat_n(self.grow / (end - start).max(1) as f32, end - start)
            })
            .collect();
        if let Some((at, _)) = self.rail {
            grows.insert(at.min(grows.len()), 0.0);
        }
        grows
    }

    pub fn width(&self) -> f32 {
        self.columns as f32 * self.column_width + (self.columns - 1) as f32 * self.column_gap + self.rail_across()
    }

    pub fn height(&self) -> f32 {
        self.outer.0 + self.lanes_height + self.rail_down() + self.outer.2 + self.bar_gap + self.bar_height
    }

    /// Where a column starts across.
    fn column_x(&self, column: usize) -> f32 {
        let (_, side, _) = self.outer;
        let column_width = (self.width() - self.rail_across() - 2.0 * side - (self.columns - 1) as f32 * self.column_gap) / self.columns as f32;
        let start = if matches!(self.rail, Some((_, RailSide::Start))) { side + self.rail_across() } else { side };
        start + column as f32 * (column_width + self.column_gap)
    }

    /// Each lane's box, in order. A shorter column's lanes share what is
    /// left over, so every column ends at the same line.
    /// (The rail's lane, where there is one, is its rail's box, in its place among them.)
    pub fn lanes(&self) -> Vec<Rect> {
        let (top, side, _) = self.outer;
        let column_width = (self.width() - self.rail_across() - 2.0 * side - (self.columns - 1) as f32 * self.column_gap) / self.columns as f32;
        let mut boxes = Vec::with_capacity(self.heights.len() + 1);
        for column in 0..self.columns {
            let start = self.cuts[column];
            let end = self.cuts.get(column + 1).copied().unwrap_or(self.heights.len());
            let count = (end - start) as f32;
            let natural = self.heights[start..end].iter().sum::<f32>() + (count - 1.0).max(0.0) * self.lane_gap;
            let gives: Vec<f32> = (start..end).map(|at| self.gives.get(at).copied().unwrap_or(0.0)).collect();
            let x = self.column_x(column);
            let mut y = top;
            for (height, extra) in self.heights[start..end].iter().zip(shares(&gives, self.lanes_height - natural)) {
                boxes.push(Rect { x, y, w: column_width, h: height + extra });
                y += height + extra + self.lane_gap;
            }
        }
        if let Some((at, rail)) = self.rail {
            let width = self.width();
            let rail = match rail {
                RailSide::Start => Rect { x: side, y: top, w: RAIL_W, h: self.lanes_height },
                RailSide::End => Rect { x: width - side - RAIL_W, y: top, w: RAIL_W, h: self.lanes_height },
                RailSide::Bottom => Rect { x: side, y: top + self.lanes_height + self.lane_gap, w: width - 2.0 * side, h: RAIL_ROW },
            };
            boxes.insert(at.min(boxes.len()), rail);
        }
        boxes
    }

    pub fn bar(&self) -> Rect {
        let y = self.outer.0 + self.lanes_height + self.rail_down() + self.outer.2 + self.bar_gap;
        Rect { x: 0.0, y, w: self.width(), h: self.bar_height }
    }

    /// How wide a percentage chart is (rate charts are a little narrower), in the first of `lanes` (laid out here)
    /// that draws one: none where none does (a workbench, or quick launch alone on its rail).
    pub fn plot_width(&self, theme: &Theme, lanes: &[Lane]) -> Option<f32> {
        lanes.iter().zip(self.lanes()).find(|(lane, _)| lane.charts()).map(|(_, area)| area.w - 2.0 * theme.pad_x - LABEL - LABEL_GAP)
    }
}

/// Draws one pass over the panel with its top-left corner at the origin,
/// and returns where clicks and wheel turns land.
pub fn paint(frame: &dyn Canvas, scene: &Scene, lanes: &[Lane], layout: &Layout, pass: Pass) -> Vec<HitBox> {
    let theme = scene.theme;
    let mut hits = Vec::new();
    let boxes = layout.lanes();
    let rail = layout.rail();
    for (at, ((lane, area), grow)) in lanes.iter().zip(&boxes).zip(layout.grows()).enumerate() {
        // The rail's lane (quick launch's) along the edge, its own way.
        if let Some((_, side)) = rail.filter(|(rail, _)| *rail == at) {
            if pass == Pass::Content {
                paint_rail(frame, scene, lane, *area, side, &mut hits);
            }
            continue;
        }
        paint_lane(frame, scene, lane, *area, grow, pass, &mut hits);
        // Chart paper rules each lane off below; the last rule in a column
        // also sets off the bar.
        if pass == Pass::Content && theme.ruled {
            frame.fill(theme.rule, area.x, area.y + area.h - 1.0, area.w, 1.0);
        }
    }
    if pass == Pass::Content {
        if theme.ruled {
            for column in 1..layout.columns {
                let x = layout.column_x(column);
                frame.fill(theme.rule, x, 0.0, 1.0, layout.lanes_height);
            }
        }
        // (None where it has no room: a widget of the workbench's lanes.)
        if rail.is_none() && layout.bar().h > 0.0 {
            paint_bar(frame, scene, layout, &mut hits);
        }
    }
    hits
}

/// Draws `text` so that its baseline sits where a CSS line box `line` DIPs
/// tall, aligned to `bottom`, would put it.
#[allow(clippy::too_many_arguments)]
fn text_on_line(frame: &dyn Canvas, text: &str, font: Font, color: Color, x: f32, bottom: f32, line: f32, width: f32) {
    let (ascent, descent) = frame.baseline(font);
    // The glyphs' ascent and descent centred in the line box.
    let baseline = bottom - line / 2.0 + (ascent - descent) / 2.0;
    frame.text(text, font, color, x, baseline - ascent, width, Align::Start);
}

/// How tall the room left in the music lane for what plays drawn apart is (DIPs): in full, and compact.
const MUSIC_ROOM: (f32, f32) = (176.0, 64.0);

/// Where what plays is drawn apart in the readings `layout` lays out, their lanes `ids` in its order (see
/// `Scene::music_apart`): the music lane's room under its head, as wide as what the lanes show; none without one.
pub fn music_room(layout: &Layout, ids: &[String], theme: &Theme) -> Option<Rect> {
    let index = ids.iter().position(|id| id == "lyrics")?;
    let lane = layout.lanes().get(index).copied()?;
    let top = lane.y + theme.pad_top + HEAD + HEAD_GAP;
    Some(Rect { x: lane.x + theme.pad_x, y: top, w: lane.w - 2.0 * theme.pad_x, h: (lane.y + lane.h - theme.pad_bottom - top).max(0.0) })
}

/// Draws `lane` in `area`, its charts `grow` DIPs taller between them (each
/// at most twice as tall as designed).
fn paint_lane(frame: &dyn Canvas, scene: &Scene, lane: &Lane, area: Rect, grow: f32, pass: Pass, hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    let ink = lane.ink;
    // (A workbench lane in a shape, padded less: see `BENCH_PAD`.)
    let (pad_x, pad_top, pad_bottom) = if matches!(lane.blocks.as_slice(), [Block::Bench(_)]) { bench_pads(theme) } else { (theme.pad_x, theme.pad_top, theme.pad_bottom) };
    let left = area.x + pad_x;
    let width = area.w - 2.0 * pad_x;
    let plot_left = left + LABEL + LABEL_GAP;
    let plot_width = left + width - plot_left;
    let mut y = area.y + pad_top;
    let charts = lane.blocks.iter().filter(|block| matches!(block, Block::Readout { plot: Some(_), .. } | Block::Rates { plot: Some(_), .. })).count();
    let taller = if charts > 0 { (grow / charts as f32).min(RATE_PLOT) } else { 0.0 };
    // The room the lane is given past what its blocks take (a column's lanes share what is left over): a list at its
    // foot takes it, its things down to the lane's foot, not stopping part way.
    let natural = pad_top + pad_bottom + lane.blocks.iter().map(Block::height).sum::<f32>() + taller * charts as f32;
    // (Given less, as a workbench widget is sized down, it gives way down to `LIST_LEAST`.)
    let spare = match lane.blocks.last().and_then(Block::list_tall) {
        Some(tall) => (area.h - natural).max(LIST_LEAST.min(tall) - tall),
        None => 0.0,
    };
    // Each part named by its lane and what it is, the small layouts naming
    // theirs alike (see `morph`): the CPU's figure is "cpu.value" in both.
    let id = &lane.id;
    let name = |part: &str| frame.key(&format!("{id}.{part}"));
    let mut kinds: Vec<&str> = Vec::new();
    for block in &lane.blocks {
        let kind = match block {
            Block::Head { .. } => "head",
            Block::Room(_) => "room",
            Block::Readout { .. } => "readout",
            Block::Rates { .. } => "rates",
            Block::Trace { .. } => "trace",
            Block::Threads(_) => "threads",
            Block::Meter { .. } => "meter",
            Block::Facts { .. } => "facts",
            Block::Sound { .. } => "sound",
            Block::Playing { .. } => "playing",
            Block::Progress { .. } => "progress",
            Block::Lyric { .. } => "lyric",
            Block::Table { .. } => "table",
            Block::Tabs { .. } => "tabs",
            Block::Shelf { .. } => "shelf",
            Block::Copies { .. } => "copies",
            Block::Recent { .. } => "recent",
            Block::Apps { .. } => "apps",
            Block::Notes { .. } => "notes",
            Block::StuckNote(_) => "stuck",
            Block::Bench(_) => "bench",
        };
        let nth = kinds.iter().filter(|k| **k == kind).count();
        kinds.push(kind);
        let part = |what: &str| name(&format!("{kind}{nth}.{what}"));
        let charted = matches!(block, Block::Readout { plot: Some(_), .. } | Block::Rates { plot: Some(_), .. });
        let height = block.height() + if charted { taller } else { 0.0 };
        match (block, pass) {
            (Block::Room(_), _) => {}
            (Block::Readout { plot: Some(plot), .. }, Pass::Plots) => {
                name("chart");
                paint_plot(frame, scene, ink, plot, plot_left, y, plot_width, height);
            }
            (Block::Rates { plot: Some(plot), .. }, Pass::Plots) => {
                name("chart");
                paint_plot(frame, scene, ink, plot, plot_left + RATE_INDENT, y, plot_width - RATE_INDENT, height);
            }
            (Block::Trace { plot, gap, .. }, Pass::Plots) => {
                part("chart");
                paint_plot(frame, scene, ink, plot, plot_left, y + gap, plot_width, height - gap);
            }
            (_, Pass::Plots) => {}
            (Block::Head { title, device, .. }, Pass::Content) if lane.id == "notes" => {
                let title_width = frame.measure(title, theme.title);
                let row = y + (HEAD - theme.title.size) / 2.0 - 1.0;
                frame.text(title, theme.title, theme.text, left, row, width, Align::Start);
                frame.text(device, theme.small, theme.text3, left + title_width + 8.0, row + 1.0, width, Align::Start);
                let button = y + (HEAD - MEDIA_ROW) / 2.0;
                pressed(frame, scene, hits, Hit::NewNote, Icon::Add, left + width - MEDIA_ROW, button, false);
                // Its views side by side, the one shown marked as the skin marks the station's chosen tab: glass's on a
                // pill of its glass (see `glass_choice`), Windows 11's a bar of its accent under it, chart paper's
                // underlined in ink.
                let shown = crate::notes::view();
                let views = [(crate::settings::NotesView::Deck, Icon::Layers), (crate::settings::NotesView::Cards, Icon::LayoutGrid), (crate::settings::NotesView::List, Icon::List)];
                let cell = MEDIA_ROW + 2.0;
                let wide = views.len() as f32 * cell + 4.0;
                let x0 = left + width - MEDIA_ROW - 8.0 - wide;
                let round = theme.control_radius.min(MEDIA_ROW / 2.0 + 2.0);
                {
                    let boxes: Vec<(f32, f32)> = (0..views.len()).map(|at| (x0 + 2.0 + at as f32 * cell, cell)).collect();
                    let at = |view: crate::settings::NotesView| views.iter().position(|(one, _)| *one == view);
                    let slid = slid_at(scene.thumbs.slid(CHOICE_NOTES, Hit::NotesShow(shown)), |from| match from {
                        Hit::NotesShow(from) => at(from),
                        _ => None,
                    });
                    if theme.skin == Skin::Glass {
                        let hovered = views.iter().position(|(view, _)| scene.hover == Some(Hit::NotesShow(*view)));
                        glass_choice(frame, theme, (button, MEDIA_ROW), &boxes, at(shown).unwrap_or(0), slid, hovered);
                    } else {
                        choice_mark(frame, theme, &boxes, at(shown).unwrap_or(0), slid, button + MEDIA_ROW + 1.0, 14.0, 4.0);
                    }
                }
                for (at, (view, icon)) in views.into_iter().enumerate() {
                    let x = x0 + 2.0 + at as f32 * cell;
                    let hit = Hit::NotesShow(view);
                    let chosen = view == shown;
                    match theme.skin {
                        Skin::Glass => {}
                        _ if chosen => {}
                        _ if scene.hover == Some(hit) => frame.fill_rounded(theme.hover, x, button, cell, MEDIA_ROW, if theme.skin == Skin::Glass { MEDIA_ROW / 2.0 } else { round }),
                        _ => {}
                    }
                    let ink = if chosen { theme.text } else { theme.text3 };
                    frame.icon(icon, Point { x: x + cell / 2.0, y: button + MEDIA_ROW / 2.0 }, 16.0, ink);
                    hits.push((x, button, cell, MEDIA_ROW, hit));
                }
            }
            (Block::Head { title, device, aside, aside_hot }, Pass::Content) => {
                let title_width = frame.measure(title, theme.title);
                let row = y + (HEAD - theme.title.size) / 2.0 - 1.0;
                name("label");
                frame.text(title, theme.title, theme.text, left, row, width, Align::Start);
                let aside_width = if aside.is_empty() { 0.0 } else { frame.measure(aside, theme.body) };
                let device_left = left + title_width + 8.0;
                let device_width = width - title_width - 8.0 - if aside_width > 0.0 { aside_width + 8.0 } else { 0.0 };
                name("device");
                frame.text(device, theme.small, theme.text3, device_left, row + 1.0, device_width, Align::Start);
                let color = if *aside_hot { theme.signal } else { theme.text2 };
                name("detail");
                frame.text(aside, theme.body, color, left, row, width, Align::End);
            }
            (Block::Readout { figure, unit, hot, plot }, Pass::Content) => {
                let color = if *hot { theme.signal } else { theme.text };
                // The figure and its unit stand on the chart's baseline: in
                // the room before the chart, or, with none, the lane's.
                let room = if plot.is_some() { LABEL } else { width };
                let figure_width = frame.measure(figure, theme.figure);
                let bottom = y + height;
                name("value");
                text_on_line(frame, figure, theme.figure, color, left, bottom, theme.figure.size * theme.figure_line, room);
                name("unit");
                text_on_line(frame, unit, theme.unit, theme.text2, left + figure_width + 3.0, bottom, theme.unit.size, (room - figure_width - 3.0).max(30.0));
            }
            (Block::Rates { rows, scale, .. }, Pass::Content) => {
                // What the chart's top stands for, small in its corner.
                if !scale.is_empty() {
                    name("scale");
                    frame.text(scale, theme.small, theme.text3, plot_left + RATE_INDENT, y, plot_width - RATE_INDENT, Align::End);
                }
                for (i, (label, value)) in rows.iter().enumerate() {
                    let row_y = y + height - (rows.len() - i) as f32 * (LINE + 2.0) + 1.0;
                    // A stroke sample in front of each label says which trace it names.
                    let stroke = if i == 0 { (ink.trace, 2.0) } else { (ink.trace2, 1.0) };
                    let which = if i == 0 { "a" } else { "b" };
                    name(&format!("{which}.mark"));
                    frame.fill(stroke.0, left, row_y + LINE / 2.0, 10.0, stroke.1);
                    frame.text(label, theme.small, theme.text2, left + 15.0, row_y + 1.0, 40.0, Align::Start);
                    name(&format!("{which}.value"));
                    frame.text(value, theme.value, theme.text, left, row_y, LABEL + RATE_INDENT, Align::End);
                }
            }
            (Block::Trace { label, value, hot, .. }, Pass::Content) => {
                // The value on the chart's baseline, its name above it.
                let bottom = y + height;
                part("text");
                frame.text(label, theme.small, theme.text2, left, bottom - 2.0 * LINE - 2.0, LABEL, Align::Start);
                frame.text(value, theme.value, if *hot { theme.signal } else { theme.text }, left, bottom - LINE, LABEL, Align::Start);
            }
            (Block::Threads(cells), Pass::Content) => {
                part("cells");
                let gap = 2.0;
                let count = cells.len().max(1) as f32;
                let cell = (width - gap * (count - 1.0)) / count;
                let radius = theme.thread_radius;
                for (i, (load, hot, _)) in cells.iter().enumerate() {
                    // A thread not read leaves its cell empty.
                    let Some(load) = load else { continue };
                    let cx = left + i as f32 * (cell + gap);
                    let cy = y + 10.0;
                    // Its clock and load, in the CPU's facts, while the pointer is on it.
                    hits.push((cx - gap / 2.0, y, cell + gap, 24.0, Hit::Thread(i as u16)));
                    let track = if scene.hover == Some(Hit::Thread(i as u16)) { theme.text3 } else { theme.track };
                    let whole = frame.on_pixels(cx, cy, cell, 14.0);
                    let (bx, by, bw, bh) = whole;
                    frame.fill_rounded(track, bx, by, bw, bh, radius);
                    // The load fills from the bottom, inside the cell's rounding, to the cell's own bottom.
                    if filled_part(frame, whole, *load, true).3 > 0.0 {
                        frame.clip_part(cx, cy, cell, 14.0, *load, true);
                        frame.fill_rounded(if *hot { theme.signal } else { ink.trace }, bx, by, bw, bh, radius);
                        frame.unclip();
                    }
                }
                // The cell pointed at: its tag over it, in the ink's colour
                // and against it, inside the lane.
                if let Some(Hit::Thread(i)) = scene.hover {
                    if let Some((_, hot, tag)) = cells.get(i as usize) {
                        let width_of = frame.measure(tag, theme.small) + 16.0;
                        let centre = left + i as f32 * (cell + gap) + cell / 2.0;
                        let tag_left = (centre - width_of / 2.0).clamp(left, (left + width - width_of).max(left));
                        let (tag_height, tag_top) = (LINE + 6.0, y + 10.0 - LINE - 10.0);
                        // A hot thread's tag in the signal colour, as its cell is.
                        let (ground, ink_on) = match (hot, theme.dark) {
                            (true, _) => (theme.signal, Color::hex(0xFFFFFF, 1.0)),
                            (false, true) => (theme.text, Color::hex(0x000000, 1.0)),
                            (false, false) => (theme.text, Color::hex(0xFFFFFF, 1.0)),
                        };
                        name("tag");
                        frame.fill_rounded(ground, tag_left, tag_top, width_of, tag_height, theme.control_radius.min(tag_height / 2.0));
                        frame.text(tag, theme.small, ink_on, tag_left + 8.0, tag_top + 3.0, width_of - 16.0, Align::Start);
                    }
                }
            }
            (Block::Sound { level, muted }, Pass::Content) => {
                // As a meter is laid out: its name, its track (dragged along, a knob where it is), its figure;
                // and after that what mutes it.
                let (row, middle) = (y + MEDIA_GAP, y + MEDIA_GAP + MEDIA_ROW / 2.0);
                part("label");
                frame.text(scene.lang.pick("音量", "Volume"), theme.small, theme.text2, left, middle - LINE / 2.0, LABEL, Align::Start);
                let said = format!("{level}%");
                let said_width = frame.measure("100%", theme.small);
                let mute = left + width - MEDIA_ROW;
                let track = (mute - 6.0 - said_width - LABEL_GAP - plot_left).max(0.0);
                part("track");
                slider(frame, theme, if *muted { theme.text3 } else { ink.trace }, plot_left, middle, track, *level as f32 / 100.0, Some(scene.thumbs.of(Hit::Volume)));
                hits.push((plot_left, row, track, MEDIA_ROW, Hit::Volume));
                part("level");
                frame.text(&said, theme.small, if *muted { theme.text3 } else { theme.text2 }, mute - 6.0 - said_width, middle - LINE / 2.0, said_width, Align::End);
                let hit = Hit::Media(crate::playing::Key::Mute, crate::playing::Follow::Any);
                pressed(frame, scene, hits, hit, if *muted { Icon::Muted } else { Icon::Volume }, mute, row, *muted);
            }
            (Block::Progress { at, length, seekable }, Pass::Content) => {
                // How far in it is where a meter's name is; the track, with a knob where it is dragged along; how
                // long it is at the end.
                let middle = y + MEDIA_ROW / 2.0;
                part("at");
                frame.text(&text::clock(*at), theme.small, theme.text2, left, middle - LINE / 2.0, LABEL, Align::Start);
                let long = text::clock(*length);
                let long_width = frame.measure(&long, theme.small);
                let track = (left + width - long_width - LABEL_GAP - plot_left).max(0.0);
                part("track");
                let fraction = if *length > 0 { *at as f32 / *length as f32 } else { 0.0 };
                slider(frame, theme, ink.trace, plot_left, middle, track, fraction, seekable.then(|| scene.thumbs.of(Hit::Seek(crate::playing::Follow::Any))));
                if *seekable {
                    hits.push((plot_left, y, track, MEDIA_ROW, Hit::Seek(crate::playing::Follow::Any)));
                }
                part("length");
                frame.text(&long, theme.small, theme.text3, left + width - long_width, middle - LINE / 2.0, long_width, Align::End);
            }
            (Block::Playing { title, about, playing, players }, Pass::Content) => {
                use crate::playing::Key;
                // What plays and what is said of it, in the room before the three that are pressed.
                let top = y + MEDIA_GAP;
                let pressed_from = left + width - 3.0 * MEDIA_ROW - 2.0 * 2.0;
                part("title");
                frame.text(title, theme.body, theme.text, left, top, pressed_from - 8.0 - left, Align::Start);
                part("about");
                // Where several players tell what they play: the line is pressed to follow the next (lit under the
                // pointer; what a press does said under it).
                let switches = *players > 1;
                let switch = Hit::Media(crate::playing::Key::Player, crate::playing::Follow::Any);
                let lit = switches && scene.hover == Some(switch);
                frame.text(about, theme.small, if lit { theme.text } else { theme.text3 }, left, top + LINE, pressed_from - 8.0 - left, Align::Start);
                if switches {
                    hits.push((left, top + LINE, pressed_from - 8.0 - left, LINE, switch));
                }
                let row = top + (2.0 * LINE - MEDIA_ROW) / 2.0;
                for (place, (key, icon)) in [(Key::Previous, Icon::SkipBack), (Key::Toggle, if *playing { Icon::Pause } else { Icon::Play }), (Key::Next, Icon::SkipForward)].into_iter().enumerate() {
                    pressed(frame, scene, hits, Hit::Media(key, crate::playing::Follow::Any), icon, pressed_from + place as f32 * (MEDIA_ROW + 2.0), row, false);
                }
            }
            (Block::Lyric { line, under, said, glide }, Pass::Content) => {
                // The line sung now rises into its place as the one before rises away, each fading as it goes.
                let top = y + MEDIA_GAP;
                let rise = (1.0 - glide.done) * LINE;
                if glide.done < 1.0 {
                    GLIDING.with(|gliding| gliding.set(true));
                }
                frame.clip(left - 2.0, top, width + 4.0, 2.0 * LINE);
                if glide.done < 1.0 && !glide.was.is_empty() {
                    part("was");
                    frame.text(&glide.was, theme.body, theme.text.alpha(1.0 - glide.done), left, top + rise - LINE, width, Align::Start);
                }
                part("line");
                let fade = if glide.was.is_empty() { 1.0 } else { glide.done };
                frame.text(line, if *said { theme.small } else { theme.body }, (if *said { theme.text2 } else { theme.text }).alpha(fade), left, top + rise, width, Align::Start);
                part("under");
                frame.text(under, theme.small, theme.text3.alpha(fade), left, top + LINE + rise, width, Align::Start);
                frame.unclip();
            }
            (Block::Meter { label, fraction, value, hot, gap, opens }, Pass::Content) => {
                let row_y = y + gap;
                part("meter");
                if let Some(drive) = opens {
                    hits.push((left, row_y, width, LINE, Hit::Drive(*drive)));
                }
                // Under the pointer, a row that opens its drive says so, lightly (the rows are close, and a band
                // across one would crowd them): its words in full ink, its name marked as its skin marks what opens,
                // and a folder after it.
                let hovered = opens.is_some_and(|drive| scene.hover == Some(Hit::Drive(drive)));
                let words = if hovered { theme.text } else { theme.text2 };
                let name = if hovered && theme.skin == Skin::Fluent { super::theme::accent(theme.dark) } else { words };
                if hovered {
                    opens_marked(frame, theme, label, left, row_y);
                }
                frame.text(label, theme.small, name, left, row_y, LABEL, Align::Start);
                let value_width = frame.measure(value, theme.small);
                frame.text(value, theme.small, words, left, row_y, width, Align::End);
                let bar_width = left + width - value_width - LABEL_GAP - plot_left;
                let bar_y = row_y + (LINE - theme.meter_height) / 2.0;
                let radius = theme.meter_radius;
                // (Its box on whole pixels once, what is filled cut out of it: see `filled_part`.)
                let whole = frame.on_pixels(plot_left, bar_y, bar_width, theme.meter_height);
                let (bx, by, bw, bh) = whole;
                frame.fill_rounded(theme.track, bx, by, bw, bh, radius);
                if filled_part(frame, whole, *fraction, false).2 > 0.0 {
                    frame.clip_part(plot_left, bar_y, bar_width, theme.meter_height, *fraction, false);
                    frame.fill_rounded(if *hot { theme.signal } else { ink.trace }, bx, by, bw, bh, radius);
                    frame.unclip();
                }
            }
            (Block::Facts { rows, gap }, Pass::Content) => {
                for (i, (label, value, hot)) in rows.iter().enumerate() {
                    let row_y = y + gap + i as f32 * (LINE + FACT_GAP);
                    part(&i.to_string());
                    frame.text(label, theme.small, theme.text2, left, row_y, LABEL, Align::Start);
                    let color = if *hot { theme.signal } else { theme.text };
                    frame.text(value, theme.small, color, plot_left, row_y, left + width - plot_left, Align::Start);
                }
            }
            (Block::Table { headings, sort, rows, visible, scrolls, widths }, Pass::Content) => {
                part("rows");
                // The headings share the lane's head row, after the title.
                let head_y = y - HEAD - HEAD_GAP;
                let columns = table_columns(left, width, *widths);
                // Every heading's ink centred on the title's: Chinese, from
                // another face than the Latin, and a heavier weight each sit
                // differently in their line boxes.
                let title_top = head_y + (HEAD - theme.title.size) / 2.0 - 1.0;
                let middle = |text: &str, font: Font| {
                    let (top, bottom) = frame.ink(text, font);
                    (top + bottom) / 2.0
                };
                let centre = title_top + middle(scene.lang.pick("进程", "Processes"), theme.title);
                for ((label, key), (cx, cw)) in headings.iter().zip(&columns[1..]) {
                    let chosen = key.is_some() && key == sort;
                    let font = if chosen { Font { weight: 650.0, ..theme.small } } else { theme.small };
                    let lit = chosen || key.is_some_and(|key| scene.hover == Some(Hit::Sort(key)));
                    frame.text(label, font, if lit { theme.text } else { theme.text2 }, *cx, centre - middle(label, font), *cw, Align::End);
                    if let Some(key) = key {
                        hits.push((*cx, head_y, *cw, HEAD, Hit::Sort(*key)));
                    }
                }
                let list_height = *visible as f32 * TABLE_ROW;
                let max_scroll = if *scrolls { (rows.len().saturating_sub(*visible)) as f32 * TABLE_ROW } else { 0.0 };
                if *scrolls {
                    hits.push((left, y, width, list_height, Hit::List(List::Processes, max_scroll.to_bits())));
                }
                let scroll = scene.scrolls.at(List::Processes).clamp(0.0, max_scroll);
                frame.clip(left, y, width, list_height);
                for (i, row) in rows.iter().enumerate() {
                    let row_y = y + i as f32 * TABLE_ROW - scroll;
                    if row_y + TABLE_ROW < y || row_y > y + list_height {
                        continue;
                    }
                    let text_y = row_y + (TABLE_ROW - LINE) / 2.0;
                    for (j, (cx, cw)) in columns.iter().enumerate() {
                        let (font, color, align) = if j == 0 { (theme.body, theme.text, Align::Start) } else { (theme.small, theme.text2, Align::End) };
                        frame.text(&row[j], font, color, *cx, text_y, *cw, align);
                    }
                }
                frame.unclip();
                // A thin thumb shows where in the list the view is.
                if max_scroll > 0.0 {
                    let thumb = (list_height * *visible as f32 / rows.len() as f32).max(12.0);
                    let at = y + (list_height - thumb) * scroll / max_scroll;
                    frame.fill_rounded(theme.text3.alpha(0.6), left + width + 6.0, at, 2.0, thumb, 1.0);
                }
            }
            (Block::Tabs { tab, count, copies, recent, view }, Pass::Content) => {
                paint_tabs(frame, scene, (left, y, width), *tab, *count, *copies, *recent, *view, hits);
            }
            (Block::Bench(what), Pass::Content) => {
                let room = (left, y, width, (area.y + area.h - pad_bottom - y).max(0.0));
                paint_bench(frame, scene, ink, room, area, what, Shape::of((room.2, room.3)), hits);
            }
            (Block::Notes { notes, view, tall }, Pass::Content) => {
                paint_notes(frame, scene, (left, y + SHELF_GAP, width), notes, *view, *tall + spare, hits);
            }
            // (All the room it is given, whatever its size. Written in where it is, drawn by what writes it. Not written
            // in, its words scrolled by the wheel as far as they reach past their box.)
            (Block::StuckNote(note), Pass::Content) => {
                let room = (left, y, width, (area.y + area.h - pad_bottom - y).max(0.0));
                let most = (!note.written).then(|| super::note::stuck_scrolls(frame, &note.text, (room.2, room.3)));
                if let Some(most) = most {
                    let scrolled = scene.scrolls.at(List::Words).clamp(0.0, most);
                    super::note::paint_stuck(frame, theme, scene.lang, &note.text, note.colour, &note.when, &note.name, room, scrolled);
                }
                let (wx, wy, ww, wh) = super::note::words_in(room.2, room.3);
                hits.push((room.0 + wx, room.1 + wy, ww, wh, Hit::Words(note.number)));
                hits.push((room.0, room.1, room.2, room.3, Hit::Note(note.number)));
                if let Some(most) = most.filter(|most| *most > 0.0) {
                    hits.push((room.0 + wx, room.1 + wy, ww, wh, Hit::List(List::Words, most.to_bits())));
                }
            }
            (Block::Apps { apps, rows, named, over }, Pass::Content) => {
                paint_apps(frame, scene, (left, y + SHELF_GAP, width), apps, *rows, *named, *over, hits);
            }
            (Block::Recent { rows, filter, view, tall, nowhere }, Pass::Content) => {
                paint_recent(frame, scene, ink, (left, y + SHELF_GAP, width), rows, *filter, *view, *tall + spare, *nowhere, hits);
            }
            (Block::Shelf { tiles, shows, tall, said, over }, Pass::Content) => {
                paint_shelf(frame, scene, ink, (left, y + SHELF_GAP, width), tiles, *shows, *tall + spare, said.as_deref(), *over, hits);
            }
            (Block::Copies { rows, filter, view, tall }, Pass::Content) => {
                paint_copies(frame, scene, ink, (left, y + SHELF_GAP, width), rows, *filter, *view, *tall + spare, hits);
            }
        }
        frame.key("");
        y += height;
    }
}

/// The station's head (see `Block::Tabs`) on the head row at `top`, from `left`, `width` across: its pages' tabs,
/// the one shown marked as its skin marks what is chosen, and its buttons at the end.
#[allow(clippy::too_many_arguments)]
fn paint_tabs(frame: &dyn Canvas, scene: &Scene, (left, top, width): (f32, f32, f32), tab: Tab, count: usize, copies: bool, recent: bool, view: ShelfView, hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    let lang = scene.lang;
    let small = theme.small;
    // (Paper underlines the one chosen under its name, as its rules are drawn: no room round it.)
    let glass = theme.skin == Skin::Glass;
    let pad = if theme.skin == Skin::Paper { 0.0 } else { TAB_PAD };
    let start = left - pad;
    let mut pages = vec![(Tab::Shelf, lang.pick("中转站", "Station"), if count > 0 { count.to_string() } else { String::new() })];
    // (Each chosen again with the filter it has.)
    if copies {
        pages.push((Tab::Copies(Default::default()).again(tab), lang.pick("最近复制", "Copied"), String::new()));
    }
    if recent {
        pages.push((Tab::Recent(Default::default()).again(tab), lang.pick("最近文件", "Recent"), String::new()));
    }
    // (In the room the buttons at the end leave, whichever page shows: names that do not fit there set a size smaller,
    // and shortened, each to a like share of it, only where they still do not. A tab's room never reaches a button's.)
    let gap = if theme.skin == Skin::Paper { 16.0 } else { 2.0 };
    let room = left + width - (MEDIA_ROW + 12.0 + 6.0) - start;
    let whole_in = |font: Font| -> f32 {
        pages.iter().map(|(_, name, aside)| 2.0 * pad + frame.measure(name, font) + if aside.is_empty() { 0.0 } else { 5.0 + frame.measure(aside, small) }).sum::<f32>() + gap * (pages.len() - 1) as f32
    };
    let smaller = Font { size: theme.title.size - 2.0, ..theme.title };
    let font = if whole_in(theme.title) <= room { theme.title } else { smaller };
    // Not side by side even so: the one shown alone, a chevron after it, its menu of them all (cut to a letter each,
    // they said nothing).
    if whole_in(smaller) > room {
        let row = top + (HEAD - TAB_H) / 2.0;
        let (name, aside) = pages.iter().find(|(page, ..)| page.page() == tab.page()).map(|(_, name, aside)| (*name, aside.clone())).unwrap_or_default();
        let name_w = frame.measure(name, theme.title);
        let aside_w = if aside.is_empty() { 0.0 } else { frame.measure(&aside, small) + 5.0 };
        let wide = (pad + name_w + aside_w + 6.0 + 10.0 + pad).min(room);
        let lit = scene.hover == Some(Hit::StationPages);
        if lit {
            frame.fill_rounded(theme.hover, start, row, wide, TAB_H, if glass { TAB_H / 2.0 } else { theme.control_radius.min(TAB_H / 2.0) });
        }
        let name_top = top + (HEAD - theme.title.size) / 2.0 - 1.0;
        frame.text(&fitted(frame, name, theme.title, wide - 2.0 * pad - aside_w - 16.0), theme.title, theme.text, start + pad, name_top, name_w + 1.0, Align::Start);
        if !aside.is_empty() {
            frame.text(&aside, small, theme.text3, start + pad + name_w + 5.0, name_top + 2.0, aside_w, Align::Start);
        }
        frame.icon(Icon::ChevronDown, Point { x: start + wide - pad - 5.0, y: row + TAB_H / 2.0 + 0.5 }, 10.0, if lit { theme.text } else { theme.text2 });
        hits.push((start, row, wide, TAB_H, Hit::StationPages));
        paint_view_button(frame, scene, (left, top, width), view, hits);
        return;
    }
    // The names on the head's line as the other lanes' titles are; their plates and the buttons about the middle of
    // the names' ink (not of their line box: a plate round it stood higher than the words in it), the chosen one's
    // mark so far under it (under the line box's foot, it met the words).
    let name_top = top + (HEAD - font.size) / 2.0 - 1.0;
    let (ink_top, ink_bottom) = frame.ink(lang.pick("中转站最近复制最近文件", "StationCopiedRecent"), font);
    let middle = name_top + (ink_top + ink_bottom) / 2.0;
    let row = middle - TAB_H / 2.0;
    let measured: Vec<(f32, f32)> = pages.iter().map(|(_, name, aside)| (frame.measure(name, font), if aside.is_empty() { 0.0 } else { frame.measure(aside, small) })).collect();
    let whole: f32 = measured.iter().map(|(name_w, aside_w)| 2.0 * pad + name_w + if *aside_w > 0.0 { 5.0 + aside_w } else { 0.0 }).sum::<f32>() + gap * (pages.len() - 1) as f32;
    let share = (room - gap * (pages.len() - 1) as f32) / pages.len() as f32;
    // Each laid out: its name (shortened to its share where they do not all fit), its count, where it is.
    let mut x = start;
    let laid: Vec<(Tab, String, f32, String, f32, f32, f32)> = pages
        .into_iter()
        .zip(measured)
        .map(|((page, name, aside), (name_w, aside_w))| {
            let (name, name_w) = if whole > room {
                let fit = (share - 2.0 * pad - if aside_w > 0.0 { 5.0 + aside_w } else { 0.0 }).min(name_w).max(0.0);
                let name = fitted(frame, name, font, fit).into_owned();
                let name_w = frame.measure(&name, font);
                (name, name_w)
            } else {
                (name.to_string(), name_w)
            };
            let wide = pad + name_w + if aside_w > 0.0 { 5.0 + aside_w } else { 0.0 } + pad;
            let at = x;
            x += wide + gap;
            (page, name, name_w, aside, aside_w, at, wide)
        })
        .collect();
    // The one chosen marked, sliding to it as another is chosen.
    {
        let boxes: Vec<(f32, f32)> = laid.iter().map(|tab| (tab.5, tab.6)).collect();
        let at = |page: Tab| laid.iter().position(|tab| tab.0 == page);
        let chosen = at(tab).unwrap_or(0);
        let slid = slid_at(scene.thumbs.slid(CHOICE_PAGE, Hit::Tab(tab.page())), |from| match from {
            Hit::Tab(from) => laid.iter().position(|one| one.0.page() == from),
            _ => None,
        });
        if glass {
            let hovered = laid.iter().position(|tab| scene.hover == Some(Hit::Tab(tab.0)));
            glass_choice(frame, theme, (row, TAB_H), &boxes, chosen, slid, hovered);
        } else {
            choice_mark(frame, theme, &boxes, chosen, slid, name_top + ink_bottom + 4.0, 16.0, 0.0);
        }
    }
    for (page, name, name_w, aside, aside_w, x, wide) in laid {
        let chosen = page == tab;
        let hovered = scene.hover == Some(Hit::Tab(page));
        if theme.skin == Skin::Fluent && hovered {
            frame.fill_rounded(theme.hover.alpha(0.6), x, row, wide, TAB_H, theme.control_radius.min(TAB_H / 2.0));
        }
        // (Glass's others a shade lighter than the chosen one's, none faint.)
        let words = match (chosen || hovered, glass) {
            (true, _) => theme.text,
            (false, true) => theme.text2,
            (false, false) => theme.text3,
        };
        frame.text(&name, font, words, x + pad, name_top, name_w + 1.0, Align::Start);
        if aside_w > 0.0 {
            frame.text(&aside, small, theme.text3, x + pad + name_w + 5.0, name_top + 2.0, aside_w + 1.0, Align::Start);
        }
        hits.push((x, row, wide, TAB_H, Hit::Tab(page)));
    }
    paint_view_button(frame, scene, (left, top, width), view, hits);
}

/// The station's head's menu button at its end (on the head row at `top`, from `left`, `width` across): how what is on
/// the page shows (its view's mark, and a chevron), and the rest.
fn paint_view_button(frame: &dyn Canvas, scene: &Scene, (left, top, width): (f32, f32, f32), view: ShelfView, hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    let font = theme.title;
    let name_top = top + (HEAD - font.size) / 2.0 - 1.0;
    let (ink_top, ink_bottom) = frame.ink(scene.lang.pick("中转站最近复制最近文件", "StationCopiedRecent"), font);
    let middle = name_top + (ink_top + ink_bottom) / 2.0;
    let row = middle - MEDIA_ROW / 2.0;
    {
        let wide = MEDIA_ROW + 12.0;
        let vx = left + width - wide;
        let lit = scene.hover == Some(Hit::StationView);
        if lit {
            frame.fill_rounded(theme.hover, vx, row, wide, MEDIA_ROW, theme.control_radius.min(MEDIA_ROW / 2.0));
        }
        let color = if lit { theme.text } else { theme.text2 };
        let icon = match view {
            ShelfView::Cards => Icon::LayoutGrid,
            ShelfView::Large => Icon::Image,
            ShelfView::List => Icon::List,
        };
        frame.icon(icon, Point { x: vx + MEDIA_ROW / 2.0, y: row + MEDIA_ROW / 2.0 }, 16.0, color);
        frame.icon(Icon::ChevronDown, Point { x: vx + MEDIA_ROW + 3.0, y: row + MEDIA_ROW / 2.0 + 0.5 }, 10.0, color);
        hits.push((vx, row, wide, MEDIA_ROW, Hit::StationView));
    }
}

/// Glass's choice of one of several, in the panel's own look: the chosen one (`chosen` of `boxes`, each its left and
/// width, all `pills` (top, height) down) on a pill of the sheet's glass, as the lanes lie on it (whiter than what is
/// round it, a rim lit along its top, fainter round its foot), `slid` from the one chosen before so far (eased: see
/// `thumb::Dots::choose`); the one under the pointer, `hovered`, lit faintly. What each says is drawn over it after.
pub(crate) fn glass_choice(frame: &dyn Canvas, theme: &Theme, pills: (f32, f32), boxes: &[(f32, f32)], chosen: usize, slid: Option<SlidAt>, hovered: Option<usize>) {
    let (py, ph) = pills;
    let dark = theme.dark;
    if let Some(&(x, w)) = hovered.filter(|at| *at != chosen).and_then(|at| boxes.get(at)) {
        frame.fill_rounded(if dark { Color::hex(0xFFFFFF, 0.06) } else { Color::hex(0xFFFFFF, 0.24) }, x, py, w, ph, ph / 2.0);
    }
    let (x, w) = slid_box(boxes, chosen, slid);
    glass_pill(frame, theme, (x, py, w, ph), ph / 2.0, false);
}

/// Where the mark of what is chosen (`chosen` of `boxes`, each its left and width) is: on it, or on its way to it from
/// the one chosen before (`slid`, so far: see `thumb::Dots::choose`).
pub(crate) fn slid_box(boxes: &[(f32, f32)], chosen: usize, slid: Option<SlidAt>) -> (f32, f32) {
    let lerp = |a: (f32, f32), b: (f32, f32), t: f32| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    let to = boxes[chosen];
    let Some(slid) = slid else { return to };
    let Some(&from) = boxes.get(slid.from) else { return to };
    // (From mid-way, where it was chosen again on its way.)
    let start = match slid.mid.and_then(|(mid, t)| Some((*boxes.get(mid)?, t))) {
        Some((mid, t)) => lerp(from, mid, t),
        None => from,
    };
    lerp(start, to, slid.done)
}

/// A choice's mark on its way (see `thumb::Dots::slid`), by the places of what it slides from among the boxes it is
/// drawn over: from which, mid-way to which and how far, and how far it has come since.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct SlidAt {
    pub from: usize,
    pub mid: Option<(usize, f32)>,
    pub done: f32,
}

/// `slid` by the places of what it slides from (`at`: where each is among the boxes; none, where one no longer is).
pub(crate) fn slid_at<K: Copy>(slid: Option<(super::thumb::Slid<K>, f32)>, at: impl Fn(K) -> Option<usize>) -> Option<SlidAt> {
    let (slid, done) = slid?;
    let mid = match slid.mid {
        Some((mid, t)) => Some((at(mid)?, t)),
        None => None,
    };
    Some(SlidAt { from: at(slid.from)?, mid, done })
}

/// The mark under what is chosen, as Windows 11 (a short bar of its accent under its middle, `bar` long) and chart
/// paper (a line of ink under all of it) draw it, `y` down, on its way to it from the one before as it slides (see
/// `slid_box`). Glass marks it otherwise (see `glass_choice`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn choice_mark(frame: &dyn Canvas, theme: &Theme, boxes: &[(f32, f32)], chosen: usize, slid: Option<SlidAt>, y: f32, bar: f32, inset: f32) {
    let (x, w) = slid_box(boxes, chosen, slid);
    match theme.skin {
        Skin::Fluent => frame.fill_rounded(super::theme::accent(theme.dark), x + w / 2.0 - bar / 2.0, y, bar, 3.0, 1.5),
        Skin::Paper => frame.fill(theme.text, x + inset, y, w - 2.0 * inset, 2.0),
        Skin::Glass => {}
    }
}

/// A pill of glass's glass at (x, y, w, h), its corners `radius` round, as the lanes lie on the sheet (whiter than what
/// is round it, a rim lit along its top, fainter round its foot): what is chosen, a button; `lit` under the pointer.
pub(crate) fn glass_pill(frame: &dyn Canvas, theme: &Theme, (x, py, w, ph): (f32, f32, f32, f32), radius: f32, lit: bool) {
    let dark = theme.dark;
    // (On a light sheet, white on white: its shadow and a faint dark edge under its lit one tell it from what is round
    // it, as the lanes' tell them from the sheet.)
    if !dark {
        frame.shadow(Color::hex(0, 0.08), x, py, w, ph, radius, 3.0, 1.0);
    }
    let fill = match (dark, lit) {
        (true, false) => Color::hex(0xFFFFFF, 0.12),
        (true, true) => Color::hex(0xFFFFFF, 0.18),
        (false, false) => Color::hex(0xFFFFFF, 0.62),
        (false, true) => Color::hex(0xFFFFFF, 0.85),
    };
    frame.fill_rounded(fill, x, py, w, ph, radius);
    // (Under the pointer, also as everything else on glass is: white on a white sheet, whiter only was not seen.)
    if lit {
        frame.fill_rounded(theme.hover, x, py, w, ph, radius);
    }
    if !dark {
        frame.stroke_rounded(Fill::Solid(Color::hex(0, 0.07)), x, py, w, ph, radius, 1.0);
    }
    let (top, foot) = if dark { (Color::hex(0xFFFFFF, 0.3), Color::hex(0xFFFFFF, 0.08)) } else { (Color::hex(0xFFFFFF, 0.95), Color::hex(0xFFFFFF, 0.4)) };
    frame.stroke_rounded(Fill::Down { top: py + 0.5, from: top, bottom: py + ph, to: foot }, x + 0.5, py + 0.5, w - 1.0, ph - 1.0, radius - 0.5, 1.0);
}

/// The choices whose marks slide (see `thumb::Dots::choose`): the station's pages, the notes' views, the recent files'
/// filters.
pub const CHOICE_PAGE: usize = 0;
pub const CHOICE_NOTES: usize = 1;
pub const CHOICE_FILTER: usize = 2;

/// What is chosen of the station's pages, of the notes' views and of the filter of the page shown (what was copied
/// lately, or the files come lately), for their marks to slide (see `thumb::Dots::choose`).
pub fn choices(tab: Tab) -> Vec<(usize, Hit)> {
    let mut chosen = vec![(CHOICE_PAGE, Hit::Tab(tab.page())), (CHOICE_NOTES, Hit::NotesShow(crate::notes::view()))];
    if matches!(tab, Tab::Copies(_) | Tab::Recent(_)) {
        chosen.push((CHOICE_FILTER, Hit::Tab(tab)));
    }
    chosen
}

/// A count's mark: a pill `BADGE` tall, as wide as its figures and the room either side of them (see `badge_width`).
const BADGE: f32 = 16.0;

/// A count's figures (see `paint_badge`): small and bold, in the skin's face.
fn badge_font(theme: &Theme) -> Font {
    Font { size: 10.5, weight: 700.0, ..theme.small }
}

/// How wide `text`'s mark is (see `paint_badge`): no narrower than tall.
fn badge_width(frame: &dyn Canvas, theme: &Theme, text: &str) -> f32 {
    (frame.measure(text, badge_font(theme)) + 12.0).max(BADGE)
}

/// `text` (a count) on a pill of `ground` from `x`, its middle at `middle` down: small bold figures, white on a dark
/// ground and dark on a light one, in its middle.
fn paint_badge(frame: &dyn Canvas, theme: &Theme, text: &str, ground: Color, x: f32, middle: f32) {
    let (font, wide) = (badge_font(theme), badge_width(frame, theme, text));
    frame.fill_rounded(ground, x, middle - BADGE / 2.0, wide, BADGE, BADGE / 2.0);
    // (Figures stand from their line's top to its base: their middle is so far down it.)
    centred(frame, text, font, ink_on(ground), x, middle - font.size * 0.71, wide);
}

/// What is written or drawn on a mark of `ground`: white on a dark one, near black on a light one (chart paper's dark
/// ink is light).
fn ink_on(ground: Color) -> Color {
    let light = 0.2126 * ground.r + 0.7152 * ground.g + 0.0722 * ground.b;
    if light > 0.6 { Color::hex(0x111111, 1.0) } else { Color::hex(0xFFFFFF, 1.0) }
}

/// Where each of the station's `count` things is laid out as `shows` says, `width` across: each one's room (from the
/// list's top, unscrolled), and how tall they all are.
fn shelf_places(shows: Shows, count: usize, width: f32, tall: f32) -> (Vec<Rect>, f32) {
    let grid = |across: usize, w: f32, h: f32, gap: f32| {
        // (Spread over the width: the gaps share what is left over.)
        let gap_x = if across > 1 { (width - across as f32 * w) / (across - 1) as f32 } else { 0.0 };
        let places: Vec<Rect> = (0..count).map(|at| Rect { x: (at % across) as f32 * (w + gap_x), y: (at / across) as f32 * (h + gap), w, h }).collect();
        let lines = count.div_ceil(across);
        (places, lines as f32 * (h + gap) - if lines > 0 { gap } else { 0.0 })
    };
    match shows {
        Shows::View(ShelfView::Cards) => {
            let across = cards_across(width);
            let w = (width - (across - 1) as f32 * TILE_GAP) / across as f32;
            grid(across, w, w * PLATE + CARD_WORDS, TILE_GAP)
        }
        Shows::Row => {
            let (mut places, _) = grid(tiles_across(width), TILE_W, TILE_H, TILE_GAP);
            // (One row: as many as fit, the rest left out.)
            places.truncate(tiles_across(width));
            (places, TILE_H)
        }
        Shows::View(ShelfView::List) => grid(1, width, LIST_ROW, 0.0),
        Shows::View(ShelfView::Large) => {
            let w = (width - (LARGE_ACROSS - 1) as f32 * TILE_GAP) / LARGE_ACROSS as f32;
            grid(LARGE_ACROSS, w, large_plate(w, tall) + CARD_WORDS, TILE_GAP)
        }
    }
}

/// How round a plate is that lies round something `inner` round, `gap` from it all round: concentric with it (the same
/// centres: its radius the inner's and the gap), as a lane's corners are with the sheet's. A plate rounder than that
/// (glass's used to be a capsule round a row of two lines, round a picture of a smaller radius) showed its corners off
/// what it held. On glass alone: Windows 11's and chart paper's plates keep their own small, square-ish corners.
fn plate_round(theme: &Theme, inner: f32, gap: f32) -> f32 {
    if theme.skin == Skin::Glass { inner + gap } else { theme.control_radius.max(6.0) }
}

/// The plate under a thing kept at hand under the pointer (or held), its box (x, y, side), its picture `gap` in from
/// its edges: as the rail and quick launch's adding page both draw it, its corners concentric with its picture's.
pub(crate) fn app_plate(frame: &dyn Canvas, theme: &Theme, (x, y, side): (f32, f32, f32), gap: f32) {
    frame.fill_rounded(theme.hover, x, y, side, side, plate_round(theme, GLASS_ROW_ROUND, gap));
}

/// How round glass's things on the sheet are where they hold no picture of their own (a row of words): as round as a
/// picture on a tile is, so the two sit alike in a lane (see `face_round`).
const GLASS_ROW_ROUND: f32 = 8.0;

/// How round a tile's face's plate is, its smaller side `small`: glass's as round as its lanes' corners allow within
/// them (the sheet's radius less the lane's padding, rounded up so as not to read as square; see `plate_round`),
/// smaller for a small face; Windows 11's and chart paper's square-cornered.
fn face_round(theme: &Theme, small: f32) -> f32 {
    match theme.skin {
        Skin::Glass => (small / 5.0).clamp(4.0, 10.0),
        Skin::Fluent => 4.0,
        Skin::Paper => 3.0,
    }
}

/// A tile's face (see `Face`) in the box `w` × `h` at (`x`, `y`), on a plate as its skin draws one (glass's round,
/// Windows 11's and chart paper's square-cornered and ruled round): a picture filling it (asked for at as many pixels
/// as it covers), another file's or a folder's icon in its middle, a text's first lines, or a web address's mark; how
/// many more files are with it, and a pin where it is pinned.
#[allow(clippy::too_many_arguments)]
fn paint_face(frame: &dyn Canvas, theme: &Theme, ink: super::theme::Ink, tile: &Tile, x: f32, y: f32, w: f32, h: f32) {
    let small = w.min(h);
    let radius = face_round(theme, small);
    // (A plate under what fills it: a picture, words, a link's mark. A file or folder stands on the card itself, by
    // its icon: on a plate of its own, an icon looked like a picture of the screen.)
    let plate = || {
        frame.fill_rounded(theme.track, x, y, w, h, radius);
        if theme.skin != Skin::Glass {
            frame.stroke_rounded(Fill::Solid(theme.rule), x, y, w, h, radius, 1.0);
        }
    };
    match &tile.face {
        Face::Picture { path, more } => {
            // A picture is shown whole on its plate, as large as fits; a folder, or another file, by Windows' own icon
            // for it.
            let centre = Point { x: x + w / 2.0, y: y + h / 2.0 };
            // (What the count goes with: the picture, its plate's corner; an icon, its own corner.)
            let corner = if shows_itself(path) {
                plate();
                match crate::station::thumbs::of(path, picture_pixels(frame, w.max(h))) {
                    Some(picture) => {
                        let (px, py, pw, ph) = picture.fitted_in(x, y, w, h);
                        frame.picture(&picture, px, py, pw, ph, radius.min(pw.min(ph) / 2.0), 1.0);
                    }
                    None => frame.icon(Icon::Image, centre, small * 0.4, theme.text3),
                }
                (x + w - 5.0, y + h - 5.0)
            } else {
                let side = (small * 0.62).min(96.0);
                paint_shell_icon(frame, theme, path, centre, side);
                (centre.x + side / 2.0 + 4.0, centre.y + side / 2.0)
            };
            // Files put on it together: how many more, on the icon's corner, in the sheet's ink (a mark of the lane's
            // ink was the loudest thing on the panel).
            if *more > 0 && small >= 40.0 {
                let badge = format!("+{more}");
                let wide = badge_width(frame, theme, &badge);
                paint_badge(frame, theme, &badge, theme.text2, (corner.0 - wide).min(x + w - wide), (corner.1 - BADGE / 2.0).min(y + h - BADGE / 2.0));
            }
        }
        Face::Text(text) => {
            plate();
            if small < 40.0 {
                frame.icon(Icon::Clipboard, Point { x: x + w / 2.0, y: y + h / 2.0 }, small / 2.0, theme.text3);
            } else {
                // Its first lines, broken into rows as wide as the tile, as many as it holds (the last cut short where
                // more is left off).
                let font = Font { size: if small > 80.0 { 12.0 } else { 11.0 }, ..theme.small };
                let line = font.size * 1.28;
                let room = w - 10.0;
                // (The first row short of a pin in the corner.)
                let first = room - if tile.pinned { 14.0 } else { 0.0 };
                for (at, row) in face_rows(frame, text, font, first, room, ((h - 10.0) / line) as usize).iter().enumerate() {
                    frame.text(row, font, theme.text2, x + 5.0, y + 5.0 + at as f32 * line, room, Align::Start);
                }
            }
        }
        Face::Link => {
            plate();
            frame.icon(Icon::Link, Point { x: x + w / 2.0, y: y + h / 2.0 }, (small * 0.42).min(28.0), ink.trace);
        }
    }
    // Pinned: a pin in its corner.
    if tile.pinned && small >= 40.0 {
        let corner = Point { x: x + w - 9.0, y: y + 9.0 };
        frame.fill_circle(ink.trace, corner, 8.0);
        frame.icon(Icon::Pin, corner, 10.0, ink_on(ink.trace));
    }
}

/// The file or folder at `path` by Windows' own icon for it, `side` wide round `centre` (asked for at as many pixels as
/// it covers); till
/// it is had, or where there is none (gone), its mark (see `paint_file_mark`), faint where it is gone.
fn paint_shell_icon(frame: &dyn Canvas, theme: &Theme, path: &std::path::Path, centre: Point, side: f32) {
    match crate::station::thumbs::icon(path, picture_pixels(frame, side)) {
        Some(icon) => frame.picture(&icon, centre.x - side / 2.0, centre.y - side / 2.0, side, side, 0.0, 1.0),
        None => {
            let looked = crate::station::thumbs::looked(path);
            let folder = looked.map_or(path.extension().is_none(), |looked| looked == crate::station::thumbs::Looked::Folder);
            paint_file_mark(frame, theme, folder, path, centre, side * 0.8, side >= 40.0, looked == Some(crate::station::thumbs::Looked::Gone));
        }
    }
}

/// A folder's or a file's mark, as the panel's icons are drawn (a line, not the shell's picture of it), `side` wide
/// round `centre`: a folder; a page, its kind (the file's extension) named on a tag of its colour where it is
/// `tagged` (large enough to read). `faint`: as for one no longer where it was.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_file_mark(frame: &dyn Canvas, theme: &Theme, folder: bool, path: &std::path::Path, centre: Point, side: f32, tagged: bool, faint: bool) {
    let shade = if faint { 0.45 } else { 1.0 };
    if folder {
        frame.icon(Icon::Folder, centre, side, theme.text3.alpha(theme.text3.a * shade));
        return;
    }
    frame.icon(Icon::File, centre, side, theme.text3.alpha(theme.text3.a * shade));
    let kind = path.extension().map(|extension| extension.to_string_lossy().to_uppercase()).unwrap_or_default();
    if tagged && !kind.is_empty() && kind.chars().count() <= 4 {
        // (Over the page's foot, from its left edge out, as the mockup has it.)
        let font = Font { size: (side * 0.2).clamp(6.5, 9.0), weight: 700.0, ..theme.small };
        let wide = frame.measure(&kind, font) + font.size * 0.9;
        let tall = font.size + 4.0;
        let (tx, ty) = (centre.x - side * 0.42, centre.y + side * 0.36 - tall);
        frame.fill_rounded(kind_colour(&kind).alpha(shade), tx, ty, wide, tall, 2.5);
        centred(frame, &kind, font, Color::hex(0xFFFFFF, shade), tx, ty + (tall - font.size * 1.33) / 2.0, wide);
    }
}

/// The colour a file's kind is tagged with (see `paint_file_mark`): by what it holds, as the programs that open it mark
/// it (a document's blue, a sheet's green, a slide's orange, a PDF's red), grey for one of no known kind.
fn kind_colour(kind: &str) -> Color {
    let hex = match kind {
        "PDF" => 0xD93A3A,
        "DOC" | "DOCX" | "RTF" | "ODT" => 0x2B65C8,
        "XLS" | "XLSX" | "CSV" | "ODS" => 0x1F8A4C,
        "PPT" | "PPTX" | "ODP" | "KEY" => 0xD2562A,
        "ZIP" | "RAR" | "7Z" | "TAR" | "GZ" | "XZ" | "ZST" | "ISO" => 0xB7861B,
        "MP4" | "MKV" | "MOV" | "AVI" | "WEBM" | "WMV" | "FLV" => 0x7A4FC9,
        "MP3" | "FLAC" | "WAV" | "M4A" | "AAC" | "OGG" | "OPUS" | "APE" => 0xC23F86,
        "EXE" | "MSI" | "BAT" | "CMD" | "PS1" | "LNK" | "DLL" | "SYS" => 0x5D636D,
        "TXT" | "MD" | "LOG" | "INI" | "JSON" | "XML" | "YAML" | "YML" | "TOML" => 0x6B7380,
        "RS" | "PY" | "JS" | "TS" | "C" | "CPP" | "H" | "CS" | "GO" | "JAVA" | "HTML" | "CSS" | "SH" => 0x1F8A8A,
        "PSD" | "AI" | "SVG" | "FIG" | "XD" | "SKETCH" => 0x2F7FD0,
        _ => 0x7B8190,
    };
    Color::hex(hex, 1.0)
}

/// Whether the file at `path` is a picture, shown as itself (its thumbnail filling its plate); any other, or a folder,
/// by its kind's icon.
fn shows_itself(path: &std::path::Path) -> bool {
    let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    ["png", "jpg", "jpeg", "gif", "webp", "bmp", "heic", "heif", "tif", "tiff", "avif", "jfif"].contains(&extension.as_str())
}

/// What is on the station (see `Block::Shelf`) from (`left`, `top`), `width` across, in room `tall` high and scrolled
/// through past it, as `shows` lays it out.
#[allow(clippy::too_many_arguments)]
fn paint_shelf(frame: &dyn Canvas, scene: &Scene, ink: super::theme::Ink, (left, top, width): (f32, f32, f32), tiles: &[Tile], shows: Shows, tall: f32, said: Option<&str>, over: bool, hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    // Something dragged over it: where it goes, lit.
    if over {
        frame.fill_rounded(ink.trace.alpha(0.12), left - 4.0, top - 4.0, width + 8.0, tall + 8.0, 10.0);
        frame.stroke_rounded(Fill::Solid(ink.trace.alpha(0.8)), left - 4.0, top - 4.0, width + 8.0, tall + 8.0, 10.0, 1.5);
    }
    if tiles.is_empty() {
        hits.push(drop_area(left, top, width, tall));
        // Nothing on it: where to drop, and what.
        if !over {
            frame.stroke_rounded(Fill::Solid(theme.text3.alpha(0.5)), left, top, width, tall, 10.0, 1.0);
        }
        let middle = top + tall / 2.0;
        frame.icon(Icon::Inbox, Point { x: left + width / 2.0, y: middle - 12.0 }, 22.0, theme.text3);
        centred(frame, said.unwrap_or_default(), theme.small, theme.text2, left, middle + 4.0, width);
        return;
    }
    paint_tiles(frame, scene, ink, (left, top, width), tiles, shows, tall, List::Shelf, &|tile: &Tile, _| Hit::Shelf(tile.id), hits);
    // Where something dragged may be dropped: all of its lane, its head too (after its things: a press finds the
    // first box it is in, a thing before the list it is on).
    hits.push(drop_area(left, top, width, tall));
    // No room left: said over it.
    if let Some(said) = said {
        let wide = (frame.measure(said, theme.small) + 20.0).min(width);
        frame.fill_rounded(theme.signal, left + (width - wide) / 2.0, top + tall - 24.0, wide, 22.0, 11.0);
        centred(frame, said, theme.small, Color::hex(0xFFFFFF, 1.0), left + (width - wide) / 2.0, top + tall - 21.0, wide);
    }
}

/// A page's things (`tiles`, each pressed as `hit` says, by it and its place) from (`left`, `top`), `width` across,
/// laid out as `shows` says, in room `tall` high, scrolled through past it (as `list`): a card each (its picture,
/// its name and what it is under it; larger, two across), a line each, or (a small lane) one row of pictures, named.
#[allow(clippy::too_many_arguments)]
fn paint_tiles(frame: &dyn Canvas, scene: &Scene, ink: super::theme::Ink, (left, top, width): (f32, f32, f32), tiles: &[Tile], shows: Shows, tall: f32, list: List, hit: &dyn Fn(&Tile, usize) -> Hit, hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    let (places, whole) = shelf_places(shows, tiles.len(), width, tall);
    // (Cards lie so far down their room as the plate under the pointer reaches past them: at the top unscrolled, its
    // top was cut off.)
    let lead = if matches!(shows, Shows::View(ShelfView::Cards | ShelfView::Large)) { CARDS_LEAD } else { 0.0 };
    let whole = whole + lead;
    let most = (whole - tall).max(0.0);
    let scroll = scene.scrolls.at(list).clamp(0.0, most);
    frame.clip(left - 4.0, top - 2.0, width + 8.0, tall + 4.0);
    for (at, (tile, place)) in tiles.iter().zip(&places).enumerate() {
        let (x, y) = (left + place.x, top + lead + place.y - scroll);
        if y + place.h < top || y > top + tall {
            continue;
        }
        let pressed = hit(tile, at);
        // (Named, each, that it moves from where it was as they flow anew: see `morph`; and that what is carried off
        // the lane is drawn as it is here, see `carry`.)
        frame.key(&format!("tile.{pressed:?}"));
        let held = tile_held(pressed);
        let lit = scene.hover == Some(pressed) && held != Held::Away;
        let words = if lit { theme.text } else { theme.text2 };
        let copied = scene.thumbs.copied(pressed);
        // Carried off it: the room it left, outlined (its picture's, or as a row, the row's), nothing of it there.
        if held == Held::Away {
            let room = match shows {
                Shows::View(view @ (ShelfView::Cards | ShelfView::Large)) => {
                    let plate = if view == ShelfView::Large { large_plate(place.w, tall) } else { place.w * PLATE };
                    (x, y, place.w, plate, face_round(theme, place.w.min(plate)))
                }
                Shows::Row => (x + (TILE_W - TILE_PICTURE) / 2.0, y, TILE_PICTURE, TILE_PICTURE, face_round(theme, TILE_PICTURE)),
                Shows::View(ShelfView::List) => (x - 4.0, y + 1.0, place.w + 8.0, place.h - 2.0, plate_round(theme, face_round(theme, LIST_PICTURE), 4.0)),
            };
            held_room(frame, theme, Held::Away, (room.0, room.1, room.2, room.3), room.4);
            hits.extend(shown_in((x, y, place.w, place.h, pressed), (left - 4.0, top, width + 8.0, tall)));
            continue;
        }
        let shown = match shows {
            Shows::View(view @ (ShelfView::Cards | ShelfView::Large)) => {
                let plate = if view == ShelfView::Large { large_plate(place.w, tall) } else { place.w * PLATE };
                if lit {
                    frame.fill_rounded(theme.hover, x - 4.0, y - 4.0, place.w + 8.0, place.h + 6.0, plate_round(theme, face_round(theme, place.w.min(plate)), 4.0));
                }
                let (fx, fy, fw, fh) = tile_face(pressed, (x, y, place.w, plate));
                paint_face(frame, theme, ink, tile, fx, fy, fw, fh);
                // (Its name in the main ink, as the list's is; what it is and when under it, as the list's is.)
                let shown = fitted(frame, &tile.label, theme.small, place.w);
                frame.text(&shown, theme.small, theme.text, x, y + plate + 5.0, place.w, Align::Start);
                frame.text(&fitted(frame, &tile.about, theme.small, place.w), theme.small, theme.text3, x, y + plate + 5.0 + LINE, place.w, Align::Start);
                copied_mark(frame, theme, scene.lang, ink.trace, copied, Mark::Centre(x + place.w / 2.0, y + plate / 2.0, place.w - 8.0));
                shown
            }
            Shows::Row => {
                if lit {
                    frame.fill_rounded(theme.hover, x - 2.0, y - 2.0, place.w + 4.0, place.h + 4.0, 8.0);
                }
                let (fx, fy, fw, fh) = tile_face(pressed, (x + (TILE_W - TILE_PICTURE) / 2.0, y, TILE_PICTURE, TILE_PICTURE));
                paint_face(frame, theme, ink, tile, fx, fy, fw, fh);
                let shown = fitted(frame, &tile.label, theme.small, TILE_W);
                centred(frame, &shown, theme.small, words, x, y + TILE_PICTURE + 4.0, TILE_W);
                copied_mark(frame, theme, scene.lang, ink.trace, copied, Mark::Centre(x + TILE_W / 2.0, y + TILE_PICTURE / 2.0, TILE_W));
                shown
            }
            Shows::View(ShelfView::List) => {
                if lit {
                    frame.fill_rounded(theme.hover, x - 4.0, y + 1.0, place.w + 8.0, place.h - 2.0, plate_round(theme, face_round(theme, LIST_PICTURE), 4.0));
                }
                let (fx, fy, fw, fh) = tile_face(pressed, (x, y + (LIST_ROW - LIST_PICTURE) / 2.0, LIST_PICTURE, LIST_PICTURE));
                paint_face(frame, theme, ink, tile, fx, fy, fw, fh);
                let room = width - LIST_PICTURE - 10.0 - if tile.pinned { 18.0 } else { 0.0 };
                let shown = fitted(frame, &tile.label, theme.body, room);
                frame.text(&shown, theme.body, theme.text, x + LIST_PICTURE + 10.0, y + 4.0, room, Align::Start);
                frame.text(&fitted(frame, &tile.about, theme.small, room), theme.small, theme.text3, x + LIST_PICTURE + 10.0, y + 4.0 + LINE, room, Align::Start);
                if tile.pinned {
                    frame.icon(Icon::Pin, Point { x: x + width - 7.0, y: y + LIST_ROW / 2.0 }, 12.0, ink.trace);
                }
                copied_mark(frame, theme, scene.lang, ink.trace, copied, Mark::Right(x + width, y + LIST_ROW / 2.0));
                shown
            }
        };
        hits.extend(shown_in((x, y, place.w, place.h, pressed), (left - 4.0, top, width + 8.0, tall)));
        name_shown(hits, pressed, &tile.label, &shown);
    }
    frame.key("");
    frame.unclip();
    // What the wheel scrolls (after its things: a press finds a thing before the list it is on).
    if most > 0.0 {
        hits.push((left, top, width, tall, Hit::List(list, most.to_bits())));
    }
    scroll_thumb(frame, theme, left + width + 6.0, top, tall, whole, scroll, most);
}

/// Recent files' page (see `Block::Recent`) from (`left`, `top`), `width` across, in room `tall` high: the filter's
/// choices in a row, the chosen marked as its skin marks what is chosen; under them the files it takes, under the day
/// each came, scrolled through (those come together a row, their count on its picture). With none, what comes here
/// (`nowhere`: that no folder is chosen to list).
#[allow(clippy::too_many_arguments)]
fn paint_recent(frame: &dyn Canvas, scene: &Scene, ink: super::theme::Ink, (left, top, width): (f32, f32, f32), rows: &[Lately], filter: crate::station::recent::Filter, view: ShelfView, tall: f32, nowhere: bool, hits: &mut Vec<HitBox>) {
    use crate::station::recent::Filter;
    let theme = scene.theme;
    let lang = scene.lang;
    let choices = Filter::ALL.map(|choice| {
        let name = match choice {
            Filter::All => lang.pick("全部", "All"),
            Filter::Downloads => lang.pick("下载", "Downloads"),
            Filter::Screenshots => lang.pick("截图", "Screenshots"),
            Filter::Documents => lang.pick("文档", "Documents"),
            Filter::Pictures => lang.pick("图片和视频", "Pictures"),
        };
        (Tab::Recent(choice), name)
    });
    paint_filter(frame, scene, (left, top, width), &choices, Tab::Recent(filter), hits);
    let list_top = top + CHIP_H + 6.0;
    let list_tall = tall - CHIP_H - 6.0;
    if rows.is_empty() {
        let middle = list_top + list_tall / 2.0;
        frame.icon(Icon::File, Point { x: left + width / 2.0, y: middle - 12.0 }, 22.0, theme.text3);
        let said = if nowhere {
            lang.pick("尚未选择要列出的文件夹", "No folders are chosen to list")
        } else if filter == Filter::All {
            lang.pick("下载、截图和新保存的文件会出现在这里", "Files you download, capture or save show here")
        } else {
            lang.pick("这一类没有新文件", "No new files of this kind")
        };
        centred(frame, said, theme.small, theme.text2, left, middle + 4.0, width);
        return;
    }
    // As cards: each its picture, its name, and when it came under it (the newest first).
    if view != ShelfView::List {
        let tiles: Vec<Tile> = rows.iter().map(Lately::tile).collect();
        paint_tiles(frame, scene, ink, (left, list_top, width), &tiles, Shows::View(view), list_tall, List::Recent, &|_, at| Hit::Recent(rows[at].entry), hits);
        return;
    }
    // Each row's top (and each day's heading's, before its first row), from the list's top.
    let mut places = Vec::with_capacity(rows.len());
    let mut headings = Vec::new();
    let mut y = 0.0;
    let mut day = "";
    for row in rows {
        if row.day != day {
            day = row.day;
            headings.push((y, day));
            y += DAY_ROW;
        }
        places.push(y);
        y += LIST_ROW;
    }
    let whole = y;
    let most = (whole - list_tall).max(0.0);
    let scroll = scene.scrolls.at(List::Recent).clamp(0.0, most);
    frame.clip(left - 4.0, list_top, width + 8.0, list_tall);
    for (at, day) in headings {
        let hy = list_top + at - scroll;
        frame.text(day, theme.small, theme.text3, left, hy + (DAY_ROW - LINE) / 2.0 + 1.0, width, Align::Start);
    }
    for (row, at) in rows.iter().zip(places) {
        let y = list_top + at - scroll;
        if y + LIST_ROW < list_top || y > list_top + list_tall {
            continue;
        }
        let pressed = Hit::Recent(row.entry);
        // (Named, as a thing on the station is: see `paint_tiles`.)
        frame.key(&format!("tile.{pressed:?}"));
        let held = tile_held(pressed);
        let round = plate_round(theme, face_round(theme, LIST_PICTURE), 4.0);
        // Carried off it: the row it left, outlined.
        if held == Held::Away {
            held_room(frame, theme, Held::Away, (left - 4.0, y + 1.0, width + 8.0, LIST_ROW - 2.0), round);
            hits.extend(shown_in((left, y, width, LIST_ROW, pressed), (left - 4.0, list_top, width + 8.0, list_tall)));
            continue;
        }
        let lit = scene.hover == Some(pressed);
        if lit {
            frame.fill_rounded(theme.hover, left - 4.0, y + 1.0, width + 8.0, LIST_ROW - 2.0, round);
        }
        let face = row.tile();
        let (fx, fy, fw, fh) = tile_face(pressed, (left, y + (LIST_ROW - LIST_PICTURE) / 2.0, LIST_PICTURE, LIST_PICTURE));
        paint_face(frame, theme, ink, &face, fx, fy, fw, fh);
        let when_w = frame.measure(&row.when, theme.small);
        let room = width - LIST_PICTURE - 10.0 - when_w - 8.0;
        let shown = fitted(frame, &row.label, theme.body, room);
        frame.text(&shown, theme.body, theme.text, left + LIST_PICTURE + 10.0, y + 4.0, room, Align::Start);
        frame.text(&fitted(frame, &row.about, theme.small, room), theme.small, theme.text3, left + LIST_PICTURE + 10.0, y + 4.0 + LINE, room, Align::Start);
        frame.text(&row.when, theme.small, theme.text3, left + width - when_w, y + (LIST_ROW - LINE) / 2.0, when_w + 1.0, Align::Start);
        copied_mark(frame, theme, scene.lang, ink.trace, scene.thumbs.copied(Hit::Recent(row.entry)), Mark::Right(left + width, y + LIST_ROW / 2.0));
        hits.extend(shown_in((left, y, width, LIST_ROW, Hit::Recent(row.entry)), (left - 4.0, list_top, width + 8.0, list_tall)));
        name_shown(hits, Hit::Recent(row.entry), &row.label, &shown);
    }
    frame.key("");
    frame.unclip();
    // (After its rows: a press finds a row before the list it is on.)
    if most > 0.0 {
        hits.push((left, list_top, width, list_tall, Hit::List(List::Recent, most.to_bits())));
    }
    scroll_thumb(frame, theme, left + width + 6.0, list_top, list_tall, whole, scroll, most);
}

/// A page's filter (what was copied lately, the files come lately) at `top` from `left`, `width` across: its
/// `choices` (each the page with that filter, and its name) in a row, `chosen` marked as the station's page over them
/// is, sliding to it.
fn paint_filter(frame: &dyn Canvas, scene: &Scene, (left, top, width): (f32, f32, f32), choices: &[(Tab, &str)], chosen: Tab, hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    // The choices, all in the row: the room round their names narrowed where they do not fit, then their names
    // shortened, each to a like share of it.
    let gaps = 4.0 * (choices.len() - 1) as f32;
    let words: f32 = choices.iter().map(|(_, name)| frame.measure(name, theme.small)).sum();
    let pad = ((width - gaps - words) / (2.0 * choices.len() as f32)).clamp(4.0, CHIP_PAD);
    let share = (width - gaps) / choices.len() as f32;
    let shortened = words + 2.0 * pad * choices.len() as f32 + gaps > width;
    let mut x = left;
    let laid: Vec<(Tab, String, f32, f32)> = choices
        .iter()
        .map(|&(choice, name)| {
            let name = if shortened { fitted(frame, name, theme.small, share - 2.0 * pad).into_owned() } else { name.to_string() };
            let wide = frame.measure(&name, theme.small) + 2.0 * pad;
            let at = x;
            x += wide + 4.0;
            (choice, name, at, wide)
        })
        .collect();
    // The one chosen marked as the station's page over them is, sliding to it (glass's on a pill of its glass, Windows
    // 11's a bar of its accent under it, chart paper's underlined in ink: filled, they were a second way of choosing
    // under the first).
    {
        let boxes: Vec<(f32, f32)> = laid.iter().map(|one| (one.2, one.3)).collect();
        let at = |choice: Tab| laid.iter().position(|one| one.0 == choice);
        let slid = slid_at(scene.thumbs.slid(CHOICE_FILTER, Hit::Tab(chosen)), |from| match from {
            Hit::Tab(from) => at(from),
            _ => None,
        });
        let marked = at(chosen).unwrap_or(0);
        if theme.skin == Skin::Glass {
            let hovered = laid.iter().position(|one| scene.hover == Some(Hit::Tab(one.0)));
            glass_choice(frame, theme, (top, CHIP_H), &boxes, marked, slid, hovered);
        } else {
            let under = if theme.skin == Skin::Paper { top + CHIP_H - 2.0 } else { top + CHIP_H - 3.0 };
            choice_mark(frame, theme, &boxes, marked, slid, under, 14.0, 4.0);
        }
    }
    for (choice, name, x, wide) in laid {
        let is_chosen = choice == chosen;
        let hit = Hit::Tab(choice);
        let hovered = scene.hover == Some(hit);
        if hovered && !is_chosen && theme.skin != Skin::Glass {
            frame.fill_rounded(theme.hover, x, top, wide, CHIP_H, theme.control_radius.min(CHIP_H / 2.0));
        }
        let words = if is_chosen || hovered { theme.text } else { theme.text2 };
        centred(frame, &name, theme.small, words, x, top + (CHIP_H - LINE) / 2.0, wide);
        hits.push((x, top, wide, CHIP_H, hit));
    }
}

/// Where a thing just copied is marked: in the middle of its picture at (x, y), in that much room across; at the end
/// of its row, its right edge at x, its middle at y.
#[derive(Clone, Copy)]
enum Mark {
    Centre(f32, f32, f32),
    Right(f32, f32),
}

/// The mark on a thing just copied (see `copied`), as clear as `clear` (none at 0): a pill of the lane's `ink`, a tick
/// and "Copied" on it in white (the tick alone where the word does not fit).
fn copied_mark(frame: &dyn Canvas, theme: &Theme, lang: Lang, ink: Color, clear: f32, at: Mark) {
    if clear <= 0.0 {
        return;
    }
    const TALL: f32 = 22.0;
    const TICK: f32 = 13.0;
    const PAD: f32 = 9.0;
    let white = Color::hex(0xFFFFFF, clear);
    let word = lang.pick("已复制", "Copied");
    let whole = PAD + TICK + 4.0 + frame.measure(word, theme.small) + PAD;
    let (fits, middle_y) = match at {
        Mark::Centre(_, y, room) => (whole <= room, y),
        Mark::Right(_, y) => (true, y),
    };
    let wide = if fits { whole } else { TALL };
    let left = match at {
        Mark::Centre(x, ..) => x - wide / 2.0,
        Mark::Right(right, _) => right - wide,
    };
    let top = middle_y - TALL / 2.0;
    frame.fill_rounded(ink.alpha(clear), left, top, wide, TALL, TALL / 2.0);
    let tick_x = if fits { left + PAD + TICK / 2.0 } else { left + wide / 2.0 };
    frame.icon(Icon::Check, Point { x: tick_x, y: middle_y }, TICK, white);
    if fits {
        frame.text(word, theme.small, white, left + PAD + TICK + 4.0, middle_y - LINE / 2.0, whole, Align::Start);
    }
}

/// What was just put on the clipboard from where it shows (see `station::run::Shown::copied`): when, and what it is
/// pressed as (a thing on the station or copied lately as either, files come lately by their entry).
pub fn copied() -> Option<(std::time::Instant, Vec<Hit>)> {
    use crate::station::run::Copied;
    let (copied, at) = crate::station::run::shown().copied?;
    Some((at, match copied {
        Copied::Item(id) => vec![Hit::Shelf(id), Hit::Copied(id)],
        Copied::Recent(entry) => vec![Hit::Recent(entry)],
    }))
}

/// A page's filter's choices: their height and the room round each name. A day's heading over its files.
const CHIP_H: f32 = 22.0;
const CHIP_PAD: f32 = 9.0;
const DAY_ROW: f32 = 22.0;

/// `hit` as much of it as shows in a list's room (`x`, `y`, `w`, `h`, what it is drawn clipped to): none where none of
/// it does. (A thing half scrolled out is pressed only where it shows: else under the list's edge, on what lies there.)
fn shown_in(hit: HitBox, (x, y, w, h): (f32, f32, f32, f32)) -> Option<HitBox> {
    let (left, top) = (hit.0.max(x), hit.1.max(y));
    let (right, bottom) = ((hit.0 + hit.2).min(x + w), (hit.1 + hit.3).min(y + h));
    (right > left && bottom > top).then_some((left, top, right - left, bottom - top, hit.4))
}

/// A thin thumb beside a list `tall` high, its whole `whole` tall, scrolled `scroll` of `most`: where in it the view is.
#[allow(clippy::too_many_arguments)]
fn scroll_thumb(frame: &dyn Canvas, theme: &Theme, x: f32, top: f32, tall: f32, whole: f32, scroll: f32, most: f32) {
    if most > 0.0 {
        let thumb = (tall * tall / whole).max(12.0);
        let at = top + (tall - thumb) * scroll / most;
        frame.fill_rounded(theme.text3.alpha(0.6), x, at, 2.0, thumb, 1.0);
    }
}

/// Where something dragged may be dropped on a station whose things are `width` × `tall` from (`left`, `top`): its
/// lane from its head down.
fn drop_area(left: f32, top: f32, width: f32, tall: f32) -> HitBox {
    let head = top - SHELF_GAP - HEAD - HEAD_GAP;
    (left - 4.0, head, width + 8.0, top + tall + 4.0 - head, Hit::Station)
}

/// `text` in the middle of `width` from `x`.
#[allow(clippy::too_many_arguments)]
fn centred(frame: &dyn Canvas, text: &str, font: Font, color: Color, x: f32, y: f32, width: f32) {
    let wide = frame.measure(text, font);
    frame.text(text, font, color, x + (width - wide).max(0.0) / 2.0, y, width, Align::Start);
}

pub(crate) use super::canvas::fitted;

/// What was copied lately (see `Block::Copies`) from (`left`, `top`), `width` across, in room `tall` high: the
/// filter's choices in a row, and under them a row each of those it takes (what it is, its first words, and what and
/// when under them), pressed to be copied again.
#[allow(clippy::too_many_arguments)]
fn paint_copies(frame: &dyn Canvas, scene: &Scene, ink: super::theme::Ink, (left, head, width): (f32, f32, f32), rows: &[Tile], filter: crate::station::shelf::Filter, view: ShelfView, tall: f32, hits: &mut Vec<HitBox>) {
    use crate::station::shelf::Filter;
    let theme = scene.theme;
    let lang = scene.lang;
    let choices = Filter::ALL.map(|choice| {
        let name = match choice {
            Filter::All => lang.pick("全部", "All"),
            Filter::Words => lang.pick("文字", "Text"),
            Filter::Pictures => lang.pick("图片", "Pictures"),
            Filter::Files => lang.pick("文件", "Files"),
        };
        (Tab::Copies(choice), name)
    });
    paint_filter(frame, scene, (left, head, width), &choices, Tab::Copies(filter), hits);
    let top = head + CHIP_H + 6.0;
    let tall = tall - CHIP_H - 6.0;
    if rows.is_empty() {
        let middle = top + tall / 2.0;
        frame.icon(Icon::Clipboard, Point { x: left + width / 2.0, y: middle - 12.0 }, 22.0, theme.text3);
        let said = if filter == Filter::All {
            lang.pick("复制的文字、图片和文件会出现在这里", "What you copy shows here")
        } else {
            lang.pick("这一类没有复制的内容", "Nothing of this kind was copied")
        };
        centred(frame, said, theme.small, theme.text2, left, middle + 4.0, width);
        return;
    }
    // As cards: each its picture (words, their first lines), what it is and when under it.
    if view != ShelfView::List {
        paint_tiles(frame, scene, ink, (left, top, width), rows, Shows::View(view), tall, List::Copies, &|tile, _| Hit::Copied(tile.id), hits);
        return;
    }
    let whole = rows.len() as f32 * LIST_ROW;
    let most = (whole - tall).max(0.0);
    let scroll = scene.scrolls.at(List::Copies).clamp(0.0, most);
    frame.clip(left - 4.0, top, width + 8.0, tall);
    for (at, row) in rows.iter().enumerate() {
        let y = top + at as f32 * LIST_ROW - scroll;
        if y + LIST_ROW < top || y > top + tall {
            continue;
        }
        let lit = scene.hover == Some(Hit::Copied(row.id));
        if lit {
            frame.fill_rounded(theme.hover, left - 4.0, y + 1.0, width + 8.0, LIST_ROW - 2.0, plate_round(theme, face_round(theme, LIST_PICTURE), 4.0));
        }
        paint_face(frame, theme, ink, row, left, y + (LIST_ROW - LIST_PICTURE) / 2.0, LIST_PICTURE, LIST_PICTURE);
        let room = width - LIST_PICTURE - 10.0;
        frame.text(&fitted(frame, &row.label, theme.body, room), theme.body, theme.text, left + LIST_PICTURE + 10.0, y + 4.0, room, Align::Start);
        frame.text(&fitted(frame, &row.about, theme.small, room), theme.small, theme.text3, left + LIST_PICTURE + 10.0, y + 4.0 + LINE, room, Align::Start);
        copied_mark(frame, theme, scene.lang, ink.trace, scene.thumbs.copied(Hit::Copied(row.id)), Mark::Right(left + width, y + LIST_ROW / 2.0));
        hits.extend(shown_in((left, y, width, LIST_ROW, Hit::Copied(row.id)), (left - 4.0, top, width + 8.0, tall)));
    }
    frame.unclip();
    // (After its rows: a press finds a row before the list it is on.)
    if most > 0.0 {
        hits.push((left, top, width, tall, Hit::List(List::Copies, most.to_bits())));
    }
    scroll_thumb(frame, theme, left + width + 6.0, top, tall, whole, scroll, most);
}

/// The widths of the process table's four measures: a share, a size, a
/// rate and a share.
const PROCESS_COLUMNS: [f32; 4] = [40.0, 52.0, 62.0, 36.0];
/// The container table's: a share, a size and two rates.
const CONTAINER_COLUMNS: [f32; 4] = [40.0, 52.0, 62.0, 62.0];

/// x and width of a table's name and its four measures, `widths` wide.
fn table_columns(left: f32, width: f32, widths: [f32; 4]) -> [(f32, f32); 5] {
    let gap = 6.0;
    let measures: f32 = widths.iter().sum::<f32>() + gap * widths.len() as f32;
    let mut columns = [(left, width - measures); 5];
    let mut x = left + width - measures + gap;
    for (i, w) in widths.iter().enumerate() {
        columns[i + 1] = (x, *w);
        x += w + gap;
    }
    columns
}

#[allow(clippy::too_many_arguments)]
fn paint_plot(frame: &dyn Canvas, scene: &Scene, ink: Ink, plot: &Plot, left: f32, top: f32, width: f32, height: f32) {
    let theme = scene.theme;
    let span = scene.prefs.chart_seconds * 1000.0;
    let pen = scene.pen_ms;
    let right = left + width;
    let bottom = top + height;
    let max = plot.max;
    let x = |t: f64| right - ((pen - t) / span) as f32 * width;
    let y = |value: f64| bottom - ((value / max).clamp(0.0, 1.0) as f32) * (height - 2.0);

    // Time rules travel with the paper.
    let rule_ms = span / 6.0;
    let mut t = (pen / rule_ms).floor() * rule_ms;
    while x(t) > left {
        // (Put on whole pixels where it is drawn, as every fill is: rounded to a whole DIP first, it jumped a pixel at a
        // time more than it moved where a DIP is not whole pixels.)
        frame.fill(theme.rule, x(t), top, 1.0, height);
        t -= rule_ms;
    }
    frame.fill(theme.rule, left, bottom, width, 1.0);

    // One sample beyond each end, so the trace runs off both sides.
    let history = scene.history;
    let Some(first) = history.iter().position(|s| s.t as f64 >= pen - span) else { return };
    let visible = &history[first.saturating_sub(1)..];
    let last = visible.last().unwrap();
    frame.clip(left, top - 2.0, width, height + 2.0);
    for (order, read) in plot.series.iter().enumerate() {
        let mut points: Vec<Option<Point>> = visible.iter().map(|s| read(s).map(|value| Point { x: x(s.t as f64), y: y(value) })).collect();
        // The pen holds its position if the next sample is late.
        if (last.t as f64) < pen {
            points.push(read(last).map(|value| Point { x: right, y: y(value) }));
        }
        // A sample without the reading breaks the line: each stretch of
        // readings is drawn on its own, the last one to the pen.
        let stretches: Vec<&[Option<Point>]> = points.split(Option::is_none).collect();
        let last_stretch = stretches.len() - 1;
        for (i, stretch) in stretches.into_iter().enumerate().filter(|(_, stretch)| !stretch.is_empty()) {
            let stretch: Vec<Point> = stretch.iter().flatten().copied().collect();
            let to_pen = i == last_stretch;
            if order == 0 {
                let mut area = stretch.clone();
                let end = stretch.last().unwrap().x;
                area.push(Point { x: if to_pen { end.max(right) } else { end }, y: bottom });
                area.push(Point { x: stretch[0].x, y: bottom });
                // The wash under the trace, fading where the skin's fades.
                let wash = if ink.wash == ink.wash_end { Fill::Solid(ink.wash) } else { Fill::Down { top, from: ink.wash, bottom, to: ink.wash_end } };
                frame.fill_shape(&area, wash);
            }
            let (color, stroke) = if order == 0 { (ink.trace, 1.5) } else { (ink.trace2, 1.0) };
            frame.stroke(&stretch, color, stroke);
            if let (0, Some(hot)) = (order, plot.hot) {
                frame.clip(left, top - 2.0, width, y(hot) - top + 2.0);
                frame.stroke(&stretch, theme.signal, stroke);
                frame.unclip();
            }
        }
    }
    frame.unclip();

    // The pen tip sits at the moment being drawn.
    let read = &plot.series[0];
    let next = visible.iter().position(|s| s.t as f64 >= pen);
    let (before, after) = match next {
        Some(0) | None => (last, last),
        Some(i) => (&visible[i - 1], &visible[i]),
    };
    let progress = if after.t == before.t { 0.0 } else { (pen - before.t as f64) / (after.t - before.t) as f64 };
    // No tip in a gap: the line is not there to have one.
    let (Some(from), Some(to)) = (read(before), read(after)) else { return };
    let value = from + (to - from) * progress;
    let color = if plot.hot.is_some_and(|hot| value > hot) { theme.signal } else { ink.trace };
    frame.fill_circle(color, Point { x: right, y: y(value) }, 2.5);
}

/// A track `track` wide from `from` along `middle`, filled `fraction` of the way in `colour`; with a knob where
/// the filling ends, where it is dragged along (`knob`: its dot's radius in the Windows 11 look, see `thumb`). In
/// that look the knob is Windows 11's own (see `thumb`); in the others, one shape with what is filled. What is filled,
/// its knob and the rest of the track are each beside the next, none over another (their colours are not solid: where
/// two lay over each other, it would show); Windows 11's knob is solid, over both.
#[allow(clippy::too_many_arguments)]
fn slider(frame: &dyn Canvas, theme: &Theme, colour: Color, from: f32, middle: f32, track: f32, fraction: f32, knob: Option<f32>) {
    let radius = theme.meter_radius;
    // The track's box put on whole pixels once, what is filled and what is not cut out of it (each put on pixels alone,
    // they met a pixel apart, and the knob's shape stood off the track's edges).
    let track_box = frame.on_pixels(from, middle - theme.meter_height / 2.0, track, theme.meter_height);
    let ((from, top, track, tall), (_, _, filled, _)) = (track_box, filled_part(frame, track_box, fraction, false));
    let (middle, half) = (top + tall / 2.0, tall / 2.0);
    if let (Some(dot), Skin::Fluent) = (knob, theme.skin) {
        frame.clip_part(from, top, track, tall, fraction, false);
        frame.fill_rounded(colour, from, top, track, tall, radius);
        frame.unclip();
        frame.clip(from + filled, top, track - filled, tall);
        frame.fill_rounded(theme.track, from, top, track, tall, radius);
        frame.unclip();
        let (face, edge) = if theme.dark { (Color::hex(0x454545, 1.0), Color::hex(0xffffff, 0.09)) } else { (Color::hex(0xffffff, 1.0), Color::hex(0x000000, 0.10)) };
        // (Smaller than Windows' own and the settings': the panel's track is thinner.)
        super::thumb::draw(frame, Point { x: from + filled, y: middle }, 8.0, dot, face, edge, colour);
        return;
    }
    // The frosted glass's: a capsule of glass across the end of what is filled, as the system's liquid glass draws its
    // sliders' (a solid dot of the ink, one with the filling, stood apart from the glass round it), a little larger
    // under the pointer and smaller held (see `thumb::Dots`).
    if let (Some(dot), Skin::Glass) = (knob, theme.skin) {
        frame.clip_part(from, top, track, tall, fraction, false);
        frame.fill_rounded(colour, from, top, track, tall, radius);
        frame.unclip();
        frame.clip(from + filled, top, track - filled, tall);
        frame.fill_rounded(theme.track, from, top, track, tall, radius);
        frame.unclip();
        // Its knob as liquid glass's: a white capsule at rest, a little larger under the pointer, and held, a clear
        // lens larger again, the bar through it and light along its rim (`dot` past 1 under the pointer, under 1 held).
        let hover = ((dot - 1.0) / 0.4).clamp(0.0, 1.0);
        let lens = ((1.0 - dot) / 0.15).clamp(0.0, 1.0);
        let grown = 1.0 + 0.1 * hover + 0.3 * lens;
        let (w, h) = (24.0 * grown, 14.0 * grown);
        let (x, y) = (from + filled - w / 2.0, middle - h / 2.0);
        frame.shadow(Color::hex(0x000000, (if theme.dark { 0.4 } else { 0.2 }) * (1.0 - 0.6 * lens)), x, y, w, h, h / 2.0, 5.0, 1.5);
        if lens > 0.0 {
            // (Through the lens the bar is seen larger.)
            let big = tall * (1.0 + 0.6 * lens);
            let (inset, top) = (h / 2.0 - 0.5 * big, middle - big / 2.0);
            frame.clip(x + inset * 0.4, y, w - inset * 0.8, h);
            frame.fill(colour, x, top, from + filled - x, big);
            frame.fill(theme.track, from + filled, top, x + w - from - filled, big);
            frame.unclip();
        }
        frame.fill_rounded(Color::hex(0xFFFFFF, 1.0 - 0.88 * lens), x, y, w, h, h / 2.0);
        let rim = Fill::Down { top: y, from: Color::hex(0xFFFFFF, 0.25 + 0.6 * lens), bottom: y + h, to: Color::hex(0xFFFFFF, 0.1 + 0.25 * lens) };
        frame.stroke_rounded(rim, x, y, w, h, h / 2.0, 1.0);
        if lens > 0.0 {
            // (Its far side's light, inside it below.)
            let inner = Fill::Down { top: y, from: Color::hex(0xFFFFFF, 0.0), bottom: y + h, to: Color::hex(0xFFFFFF, 0.45 * lens) };
            frame.stroke_rounded(inner, x + 1.5, y + 1.5, w - 3.0, h - 3.0, h / 2.0 - 1.5, 1.0);
        }
        return;
    }
    if knob.is_none() {
        frame.fill_rounded(theme.track, from, top, track, tall, radius);
        if filled > 0.0 {
            frame.clip_part(from, top, track, tall, fraction, false);
            frame.fill_rounded(colour, from, top, track, tall, radius);
            frame.unclip();
        }
        return;
    }
    let knob = 5.0;
    // What is filled and its knob are one shape, drawn once: from the track's rounded end along it, round the
    // knob, and back.
    let centre = Point { x: from + filled, y: middle };
    // (Where the track's edges meet the knob.)
    let meets = (knob * knob - half * half).sqrt();
    let mut shape: Vec<Point> = Vec::new();
    if filled > meets + half {
        // Its end at the left, rounded: from its lower corner up round to its upper one.
        shape.extend((0..=8).map(|turn| {
            let angle = std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * turn as f32 / 8.0;
            Point { x: from + half + half * angle.cos(), y: middle + half * angle.sin() }
        }));
        // Round the knob, from where the upper edge meets it, over, to where the lower one does.
        let start = (-half).atan2(-meets);
        let round = 2.0 * std::f32::consts::PI - 2.0 * (half / knob).asin();
        shape.extend((0..=32).map(|turn| {
            let angle = start + round * turn as f32 / 32.0;
            Point { x: centre.x + knob * angle.cos(), y: centre.y + knob * angle.sin() }
        }));
        frame.fill_shape(&shape, Fill::Solid(colour));
    } else {
        // (The knob alone, as the shape above draws it: about the same centre, not put on a pixel as a circle is,
        // which stood it half a pixel off as the level crossed into the shape.)
        let round: Vec<Point> = (0..40).map(|turn| {
            let angle = std::f32::consts::TAU * turn as f32 / 40.0;
            Point { x: centre.x + knob * angle.cos(), y: centre.y + knob * angle.sin() }
        }).collect();
        frame.fill_shape(&round, Fill::Solid(colour));
    }
    if track - filled > knob {
        frame.clip(from + filled + knob, top, track - filled - knob, tall);
        frame.fill_rounded(theme.track, from, top, track, tall, radius);
        frame.unclip();
    }
}

/// The part of `whole` that `fraction` of it fills (see `canvas::filled_part`), on `frame`'s pixels.
pub(crate) fn filled_part(frame: &dyn Canvas, whole: (f32, f32, f32, f32), fraction: f32, up: bool) -> (f32, f32, f32, f32) {
    super::canvas::filled_part(whole, fraction, up, |x, y, w, h| frame.on_pixels(x, y, w, h))
}

/// Something pressed, as the bar's are: its glyph in a square from (`left`,
/// `top`), on a plate while the pointer is over it or it is `lit`.
#[allow(clippy::too_many_arguments)]
fn pressed(frame: &dyn Canvas, scene: &Scene, hits: &mut Vec<HitBox>, hit: Hit, icon: Icon, left: f32, top: f32, lit: bool) {
    let theme = scene.theme;
    let hovered = scene.hover == Some(hit);
    if hovered || lit {
        frame.fill_rounded(theme.hover, left, top, MEDIA_ROW, MEDIA_ROW, theme.control_radius.min(MEDIA_ROW / 2.0));
    }
    frame.icon(icon, Point { x: left + MEDIA_ROW / 2.0, y: top + MEDIA_ROW / 2.0 }, 15.0, if hovered || lit { theme.text } else { theme.text2 });
    hits.push((left, top, MEDIA_ROW, MEDIA_ROW, hit));
}

fn paint_bar(frame: &dyn Canvas, scene: &Scene, layout: &Layout, hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    let bar = layout.bar();
    frame.key("bar");
    // Windows 11 sets the bar off as a slightly darker footer strip.
    if theme.skin == Skin::Fluent {
        frame.fill(theme.footer, bar.x, bar.y, bar.w, bar.h);
        frame.fill(theme.rule, bar.x, bar.y, bar.w, 1.0);
    }
    // Settings at the end, and the pin before it: glyphs of the system's
    // icon font, each centred in its button. A pinned pin stays lit.
    let top = bar.y + (bar.h - BUTTON) / 2.0;
    let settings = bar.x + bar.w - theme.bar_pad.1 - BUTTON;
    let pin = settings - BUTTON_GAP - BUTTON;
    let overlay = pin - BUTTON_GAP - BUTTON;
    let game = overlay - BUTTON_GAP - BUTTON;
    let uptime = scene.lang.duration(scene.latest().system.uptime_s);
    if !scene.buttons {
        let label = if scene.lang == Lang::Zh { format!("已开机 {uptime}") } else { format!("Up {uptime}") };
        let start = bar.x + theme.bar_pad.0;
        frame.text(&label, theme.small, theme.text2, start, bar.y + (bar.h - LINE) / 2.0, bar.w - 2.0 * start, Align::Start);
        frame.key("");
        return;
    }
    let label = if scene.lang == Lang::Zh { format!("已开机 {uptime}") } else { format!("Up {uptime}") };
    let start = bar.x + theme.bar_pad.0;
    frame.text(&label, theme.small, theme.text2, start, bar.y + (bar.h - LINE) / 2.0, game - 8.0 - start, Align::Start);
    // The overlay's: a window with a trace in it, lit while it is on.
    // And the game button before it, lit while a game shows (found, or asked for by hand), as a widget's is.
    for (left, hit) in [(game, Hit::Game), (overlay, Hit::Overlay), (pin, Hit::Pin), (settings, Hit::Settings)] {
        hits.push(panel_button(frame, scene, hit, left, top));
    }
    frame.key("");
}

/// One of the panel's own buttons (the game's, the overlay's, the pin, the settings), as its bar and a rail's foot
/// show them: its glyph (the system's icon font's; the game's Glance's own icon, as on a widget's button) in a square
/// `BUTTON` wide from (`left`, `top`), on a plate while the pointer is over it or it is on (a game shown, the overlay
/// up, pinned, the settings asked from it open); the settings' marked by a dot where a newer Glance is out. Where it is pressed.
fn panel_button(frame: &dyn Canvas, scene: &Scene, hit: Hit, left: f32, top: f32) -> HitBox {
    let theme = scene.theme;
    let (icon, on, grown) = match hit {
        Hit::Game => (Icon::Game, game_shown(scene.history.last().map(std::slice::from_ref).unwrap_or_default()), 1.0),
        Hit::Overlay => (Icon::Activity, scene.overlay, 1.0),
        Hit::Pin => (Icon::Pin, scene.pinned, 1.0),
        _ => (Icon::Settings, scene.settings_lit.is_some(), scene.settings_lit.unwrap_or(1.0)),
    };
    // Under the pointer, on a plate; on (pinned, the overlay up, the game shown), in the accent (chart paper's, its
    // ink and a line under it): on a plate both ways, which was which could not be told under the pointer.
    let hovered = scene.hover == Some(hit);
    if hovered {
        frame.fill_rounded(theme.hover, left, top, BUTTON, BUTTON, theme.control_radius.min(BUTTON / 2.0));
    }
    let colour = match (on, theme.skin) {
        (true, Skin::Paper) => theme.text,
        (true, _) => super::theme::accent(theme.dark),
        (false, _) if hovered => theme.text,
        (false, _) => theme.text2,
    };
    frame.icon(icon, Point { x: left + BUTTON / 2.0, y: top + BUTTON / 2.0 }, 16.0 * grown, colour);
    if on && theme.skin == Skin::Paper {
        frame.fill(theme.text, left + BUTTON / 2.0 - 5.0, top + BUTTON - 3.0, 10.0, 1.5);
    }
    if hit == Hit::Settings && scene.newer {
        frame.fill_circle(super::theme::accent(theme.dark), Point { x: left + BUTTON - 7.0, y: top + 7.0 }, 3.0);
    }
    (left, top, BUTTON, BUTTON, hit)
}

/// What workbench lane `id` holds, for a shape of it (see `Bench`): none for another.
fn bench(scene: &Scene, id: &str) -> Option<Bench> {
    let lang = scene.lang;
    let now = crate::station::now_ms();
    Some(match id {
        "station" => {
            let shown = crate::station::run::shown();
            let folder = crate::station::folder::Folder::own();
            let tab = match scene.tab {
                Tab::Copies(_) if !shown.history => Tab::Shelf,
                Tab::Recent(_) if !crate::station::recent::on() => Tab::Shelf,
                tab => tab,
            };
            let tiles: Vec<(Tile, Hit)> = match tab {
                Tab::Shelf => shown.items.iter().rev().filter(|item| !item.copied).map(|item| (Tile::of(item, &folder, lang, now), Hit::Shelf(item.id))).collect(),
                Tab::Copies(filter) => copied_lately(&shown.items, filter).map(|item| (Tile::of(item, &folder, lang, now), Hit::Copied(item.id))).collect(),
                Tab::Recent(filter) => Lately::rows(&crate::station::recent::shown(), filter, lang, now).iter().map(|row| (row.tile(), Hit::Recent(row.entry))).collect(),
            };
            Bench::Station { tiles, tab }
        }
        "notes" => Bench::Notes(crate::notes::shown().iter().map(|note| note_card(note, lang, now, BENCH_LINES)).collect()),
        "apps" => Bench::Apps(crate::station::apps::shown().iter().map(|app| (app.name.clone(), app.target.clone())).collect()),
        "time" => {
            let now = text::Local::at(scene.latest().t);
            let (figure, unit) = now.time(text::twelve_hours(), lang);
            Bench::Time { figure, unit: unit.to_string(), date: now.date(lang) }
        }
        _ => return None,
    })
}

/// `text` written down, a character under another, each in the middle of `x`, from `top` down as far as `tall` (the
/// rest left off): words in a strip down.
fn text_down(frame: &dyn Canvas, text: &str, font: Font, color: Color, x: f32, top: f32, tall: f32) {
    let step = font.size * 1.18;
    let fits = ((tall / step).floor() as usize).max(1);
    let chars: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    for (at, c) in chars.iter().take(fits).enumerate() {
        let one = c.to_string();
        let w = frame.measure(&one, font);
        frame.text(&one, font, color, x - w / 2.0, top + at as f32 * step, w + 1.0, Align::Start);
    }
}

/// The button that writes a new note, `side` square at (`left`, `top`).
fn paint_add(frame: &dyn Canvas, scene: &Scene, hits: &mut Vec<HitBox>, left: f32, top: f32, side: f32) {
    paint_add_on(frame, scene, hits, left, top, side, scene.theme.text);
}

/// The same, on a ground its words are `ink` on (a note's).
fn paint_add_on(frame: &dyn Canvas, scene: &Scene, hits: &mut Vec<HitBox>, left: f32, top: f32, side: f32, ink: Color) {
    let theme = scene.theme;
    let lit = scene.hover == Some(Hit::NewNote);
    if lit {
        frame.fill_rounded(theme.hover, left, top, side, side, theme.control_radius.min(side / 2.0));
    }
    frame.icon(Icon::Add, Point { x: left + side / 2.0, y: top + side / 2.0 }, (side * 0.62).min(15.0), ink.alpha(ink.a * if lit { 1.0 } else { 0.72 }));
    hits.push((left, top, side, side, Hit::NewNote));
}

/// How far a page of a paged grid is turned in its list's scroll (DIPs): as far as a notch of the wheel turns a list,
/// so a notch turns a page.
pub const PAGE: f32 = TABLE_ROW * crate::panel::WHEEL_ROWS;
/// The room under a grid two rows tall or more: its pages' dots, or the name of the thing under the pointer.
const GRID_FOOT: f32 = 26.0;
/// The room under a cell for its name, where the cells are named.
const GRID_NAME: f32 = 17.0;
const GRID_GAP: f32 = 10.0;

/// A grid of things as a phone's large folder sets them: so many across and down, each cell `cell` square (its name
/// under it where `named`), its corner at (`x`, `y`), and `foot` under it.
struct Grid {
    cols: usize,
    rows: usize,
    cell: f32,
    named: bool,
    x: f32,
    y: f32,
    foot: f32,
}

impl Grid {
    /// The grid in the room `(x, y, w, h)` of cells about `near` square: as many across and down as that comes to,
    /// named where `names` says and it is three across and two down or more (smaller, the name under the pointer is said
    /// in its foot instead).
    fn in_room((x, y, w, h): (f32, f32, f32, f32), near: f32, names: bool) -> Grid {
        let gap = GRID_GAP;
        let fit = |room: f32, each: f32| (((room + gap) / (each + gap)).round().max(1.0)) as usize;
        let cols = fit(w, near);
        let foot = if fit(h, near) >= 2 { GRID_FOOT } else { 0.0 };
        let named = names && cols >= 3 && fit(h - foot, near + GRID_NAME) >= 2;
        let name = if named { GRID_NAME } else { 0.0 };
        let rows = fit(h - foot, near + name);
        let cell = ((w - (cols - 1) as f32 * gap) / cols as f32).min((h - foot - (rows - 1) as f32 * gap) / rows as f32 - name).min(near * 1.3).max(0.0);
        let (wide, tall) = (cols as f32 * cell + (cols - 1) as f32 * gap, rows as f32 * (cell + name) + (rows - 1) as f32 * gap);
        Grid { cols, rows, cell, named, x: x + (w - wide) / 2.0, y: y + (h - foot - tall) / 2.0, foot }
    }

    /// Where the `n`th cell of a page is (its corner).
    fn at(&self, n: usize) -> (f32, f32) {
        let name = if self.named { GRID_NAME } else { 0.0 };
        (self.x + (n % self.cols) as f32 * (self.cell + GRID_GAP), self.y + (n / self.cols) as f32 * (self.cell + name + GRID_GAP))
    }
}

/// How `count` things go on pages of `per` cells: each page's first and how many are on it. Where they do not all go
/// on one, the last cell of each page but the last is the way to the next (holding none of them).
fn pages_of(count: usize, per: usize) -> Vec<(usize, usize)> {
    if per <= 1 {
        return (0..count.max(1)).map(|at| (at, usize::from(at < count))).collect();
    }
    let mut pages = Vec::new();
    let mut from = 0;
    loop {
        let left = count - from;
        if left <= per {
            pages.push((from, left));
            return pages;
        }
        pages.push((from, per - 1));
        from += per - 1;
    }
}

/// How tall a grid's foot's line is (its dots, a name): at the foot of its foot, the rest of it room over it.
const FOOT_LINE: f32 = 18.0;

/// The dots of `count` pages: each dot's side, the long one's length and the gap between them.
const DOT: (f32, f32, f32) = (5.0, 12.0, 4.0);

/// How wide `count` pages' dots are (see `paint_dots`) within `room`: too many to go in it, as wide as which of them
/// it is, said ("3/12"), in `font`.
fn dots_wide(frame: &dyn Canvas, count: usize, font: Font, room: f32) -> f32 {
    let (dot, long, gap) = DOT;
    let wide = count as f32 * (dot + gap) - gap + (long - dot);
    if wide <= room { wide } else { frame.measure(&format!("{count}/{count}"), font) }
}

/// `count` pages' dots in a row, its middle at (`middle`, `y`), the one `at` (between two as the pages slide) drawn
/// long; too many to go in `room`, which of them it is said, in `font`.
#[allow(clippy::too_many_arguments)]
fn paint_dots(frame: &dyn Canvas, count: usize, at: f32, middle: f32, y: f32, ink: Color, font: Font, room: f32) {
    let (dot, long, gap) = DOT;
    let wide = count as f32 * (dot + gap) - gap + (long - dot);
    if wide > room {
        let said = format!("{}/{count}", at.round() as usize + 1);
        let width = frame.measure(&said, font).min(room);
        centred(frame, &said, font, ink, middle - width / 2.0, y - font.size * 0.65, width);
        return;
    }
    let mut x = middle - wide / 2.0;
    for n in 0..count {
        let near = (1.0 - (n as f32 - at).abs()).clamp(0.0, 1.0);
        let w = dot + (long - dot) * near;
        frame.fill_rounded(ink.alpha(ink.a * (0.4 + 0.5 * near)), x, y - dot / 2.0, w, dot, dot / 2.0);
        x += w + gap;
    }
}

/// What draws a thing of a grid: its index, its cell's corner and side, and whether its page is the one shown (only
/// that one's things are pressed, and named for a turn between forms).
type DrawCell<'a> = &'a dyn Fn(&mut Vec<HitBox>, usize, (f32, f32), f32, bool);

/// A grid (see `Grid`) of `count` things in `room`, on pages turned with `list`'s scroll (see `PAGE`): each drawn by
/// `draw`, a page sliding in as it is turned to; the last cell of a page but the last the way to the next, its next
/// four drawn small in it by `small` (as a phone's folder shows what more it holds). In its foot, the name of the thing
/// under the pointer (`hovered`), else the pages' dots.
#[allow(clippy::too_many_arguments)]
fn paint_grid(frame: &dyn Canvas, scene: &Scene, hits: &mut Vec<HitBox>, room: (f32, f32, f32, f32), grid: &Grid, count: usize, list: List, draw: DrawCell, small: &dyn Fn(usize, (f32, f32), f32), hovered: Option<&str>) {
    let theme = scene.theme;
    let (x, y, w, h) = room;
    let pages = pages_of(count, grid.cols * grid.rows);
    let last = (pages.len() - 1) as f32;
    let most = last * PAGE;
    let turned = (scene.scrolls.at(list) / PAGE).clamp(0.0, last);
    let shown = turned.round() as usize;
    let name = if grid.named { GRID_NAME } else { 0.0 };
    let cell = grid.cell;
    frame.clip(x, y, w, h - grid.foot);
    for (page, &(from, on)) in pages.iter().enumerate() {
        let off = page as f32 - turned;
        if off.abs() >= 1.0 {
            continue;
        }
        // (A page as wide as the grid: a swipe moves it as far as the hand moves.)
        let dx = off * w;
        let current = page == shown;
        for n in 0..on {
            let (cx, cy) = grid.at(n);
            draw(hits, from + n, (cx + dx, cy), cell, current);
        }
        // The way to the next page: a plate holding its first four, small. (A grid of one cell has no room for it: its
        // pages are turned by the wheel alone.)
        let Some(&(next, _)) = pages.get(page + 1).filter(|_| grid.cols * grid.rows > 1) else { continue };
        let (cx, cy) = grid.at(on);
        let cx = cx + dx;
        let hit = Hit::NextPage(list, most.to_bits());
        let round = face_round(theme, cell);
        frame.fill_rounded(theme.hover, cx, cy, cell, cell, round);
        if current && scene.hover == Some(hit) {
            frame.fill_rounded(theme.hover, cx, cy, cell, cell, round);
        }
        let inset = cell * 0.14;
        let side = (cell - 2.0 * inset - inset * 0.6) / 2.0;
        for k in 0..(count - next).min(4) {
            small(next + k, (cx + inset + (k % 2) as f32 * (side + inset * 0.6), cy + inset + (k / 2) as f32 * (side + inset * 0.6)), side);
        }
        if grid.named {
            let more = scene.lang.pick("还有 {} 项", "{} more").replace("{}", &(count - next).to_string());
            centred(frame, &fitted(frame, &more, theme.small, cell + GRID_GAP), theme.small, theme.text2, cx - GRID_GAP / 2.0, cy + cell + 2.0, cell + GRID_GAP);
        }
        if current {
            hits.push((cx, cy, cell, cell + name, hit));
        }
    }
    frame.unclip();
    if grid.foot > 0.0 {
        // (Low in it: what is over it apart from the grid.)
        let middle = y + h - FOOT_LINE / 2.0;
        match hovered {
            Some(said) => centred(frame, &fitted(frame, said, theme.small, w), theme.small, theme.text2, x, middle - LINE / 2.0, w),
            None if pages.len() > 1 => paint_dots(frame, pages.len(), turned, x + w / 2.0, middle, theme.text2, theme.small, w),
            None => {}
        }
    }
    // (Last: what is on it is found first; the wheel finds this under it.)
    hits.push((x, y, w, h, Hit::Pages(list, most.to_bits())));
}

/// `text` in rows `width` wide in `font`, at most `most` of them (the last cut short with an ellipsis where it runs
/// on): broken between words, or between characters where a word is too long for a row (and in Chinese).
fn rows_of(frame: &dyn Canvas, text: &str, font: Font, width: f32, most: usize) -> Vec<String> {
    rows_within(frame, text, font, width, width, most)
}

/// `text` in rows as `rows_of` lays it out, its first row `first` wide (short of something in its corner), the rest
/// `width`.
fn rows_within(frame: &dyn Canvas, text: &str, font: Font, first: f32, width: f32, most: usize) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    let mut row = String::new();
    for c in text.chars() {
        let mut longer = row.clone();
        longer.push(c);
        if frame.measure(&longer, font) <= if rows.is_empty() { first } else { width } || row.is_empty() {
            row = longer;
            continue;
        }
        // Back to the space before the word it breaks, where there is one.
        let (kept, carried) = match row.rfind(' ').filter(|_| !c.is_whitespace()) {
            Some(space) if space > 0 => (row[..space].to_string(), row[space + 1..].to_string()),
            _ => (row.clone(), String::new()),
        };
        rows.push(kept);
        // (A row past the most: the rest is not laid out, row by row, to be left off.)
        if rows.len() > most {
            break;
        }
        row = carried.trim_start().to_string();
        if !c.is_whitespace() {
            row.push(c);
        }
    }
    if !row.is_empty() {
        rows.push(row);
    }
    if rows.len() > most {
        rows.truncate(most);
        let room = if rows.len() == 1 { first } else { width };
        if let Some(last) = rows.last_mut() {
            *last = fitted(frame, &format!("{last}…"), font, room).to_string();
        }
    }
    rows
}

/// A text's lines (see `Face::Text`) in at most `most` rows `width` wide, the first `first` wide: each line broken
/// into rows as `rows_of` breaks it, a new line on a new row, the last row cut short with an ellipsis where more of
/// it is left off.
fn face_rows(frame: &dyn Canvas, text: &str, font: Font, first: f32, width: f32, most: usize) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty()).peekable();
    while let Some(line) = lines.next() {
        let left = most - rows.len();
        let mut laid = rows_within(frame, line, font, if rows.is_empty() { first } else { width }, width, left);
        let full = rows.len() + laid.len() == most;
        if full && lines.peek().is_some() && laid.last().is_some_and(|last| !last.ends_with('…')) {
            let room = if rows.is_empty() && laid.len() == 1 { first } else { width };
            let last = laid.last_mut().unwrap();
            *last = fitted(frame, &format!("{last}…"), font, room).to_string();
        }
        rows.append(&mut laid);
        if full {
            break;
        }
    }
    rows
}

/// A workbench lane drawn as `shape` in `area` (x, y, width, height: all of it but its padding; `lane`: the whole of
/// it): the station's things and quick launch's in a grid, as a phone's large folder (see `paint_grid`); the notes,
/// the lane the note shown (it is the note, in its colour), turned through as pages; the time. Each thing is named as
/// in its whole lane, that it moves between the two as the widget is sized (see `morph`).
#[allow(clippy::too_many_arguments)]
fn paint_bench(frame: &dyn Canvas, scene: &Scene, ink: super::theme::Ink, (x, y, w, h): (f32, f32, f32, f32), lane: Rect, what: &Bench, shape: Shape, hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    // What it shows kept off its edge as far as its rounded corners take, and room to breathe round it (at the lane's
    // padding alone, its things ran into the curve and it read crammed); a fifth of it at most, small.
    let radius = theme.radius.min(lane.w.min(lane.h) / 2.0);
    let inset = (radius * 0.6).max(BENCH_BREATH).min(lane.w.min(lane.h) / 5.0);
    let (left, top) = (x.max(lane.x + inset), y.max(lane.y + inset));
    let (x, y, w, h) = (left, top, ((x + w).min(lane.x + lane.w - inset) - left).max(0.0), ((y + h).min(lane.y + lane.h - inset) - top).max(0.0));
    let lit = |hit: Hit| scene.hover == Some(hit);
    match what {
        Bench::Station { tiles, tab } => {
            // Its page's name and how many, its menu of the pages, over the grid where it is roomy.
            let mut room = (x, y, w, h);
            if h >= 150.0 && w >= 110.0 {
                let name = match tab {
                    Tab::Shelf => scene.lang.pick("中转站", "Station"),
                    Tab::Copies(_) => scene.lang.pick("最近复制", "Copied"),
                    Tab::Recent(_) => scene.lang.pick("最近文件", "Recent"),
                };
                let count = tiles.len().to_string();
                let small = theme.small;
                let name_w = frame.measure(name, theme.title).min(w - 40.0);
                let count_w = frame.measure(&count, small);
                let chip = (x - 4.0, y, (name_w + count_w + 30.0).min(w + 4.0), TAB_H);
                if lit(Hit::StationPages) {
                    frame.fill_rounded(theme.hover, chip.0, chip.1, chip.2, chip.3, TAB_H / 2.0);
                }
                let top = y + (TAB_H - theme.title.size * 1.3) / 2.0;
                frame.text(&fitted(frame, name, theme.title, name_w), theme.title, theme.text, x + 4.0, top, name_w + 1.0, Align::Start);
                frame.text(&count, small, theme.text3, x + 4.0 + name_w + 5.0, top + 2.0, count_w + 1.0, Align::Start);
                frame.icon(Icon::ChevronDown, Point { x: x + name_w + count_w + 18.0, y: y + TAB_H / 2.0 + 0.5 }, 9.0, theme.text2);
                hits.push((chip.0, chip.1, chip.2, chip.3, Hit::StationPages));
                room = (x, y + TAB_H + 6.0, w, h - TAB_H - 6.0);
            }
            let grid = Grid::in_room(room, 60.0, true);
            let list = match tab {
                Tab::Shelf => List::Shelf,
                Tab::Copies(_) => List::Copies,
                Tab::Recent(_) => List::Recent,
            };
            let (named, footed) = (grid.named, grid.foot > 0.0);
            let draw = |hits: &mut Vec<HitBox>, at: usize, (cx, cy): (f32, f32), side: f32, current: bool| {
                let (tile, hit) = &tiles[at];
                if current {
                    frame.key(&format!("tile.{hit:?}"));
                    if lit(*hit) {
                        frame.fill_rounded(theme.hover, cx - 3.0, cy - 3.0, side + 6.0, side + 6.0 + if named { GRID_NAME } else { 0.0 }, face_round(theme, side) + 3.0);
                    }
                }
                paint_face(frame, theme, ink, tile, cx, cy, side, side);
                // (Copied by a press: marked so on its picture, as on the station's pages.)
                if current {
                    copied_mark(frame, theme, scene.lang, ink.trace, scene.thumbs.copied(*hit), Mark::Centre(cx + side / 2.0, cy + side / 2.0, side));
                }
                let shown = if named { fitted(frame, &tile.label, theme.small, side + GRID_GAP - 2.0).to_string() } else { String::new() };
                if named {
                    centred(frame, &shown, theme.small, theme.text2, cx - GRID_GAP / 2.0, cy + side + 2.0, side + GRID_GAP);
                }
                if current {
                    frame.key("");
                    hits.push((cx, cy, side, side + if named { GRID_NAME } else { 0.0 }, *hit));
                    // (Its whole name told as the pointer rests on it, where the foot does not say it.)
                    if named || !footed {
                        name_shown(hits, *hit, &tile.label, &shown);
                    }
                }
            };
            let small = |at: usize, (cx, cy): (f32, f32), side: f32| paint_face(frame, theme, ink, &tiles[at].0, cx, cy, side, side);
            let hovered = tiles.iter().find(|(_, hit)| lit(*hit)).map(|(tile, _)| tile.label.as_str()).filter(|_| !named);
            paint_grid(frame, scene, hits, room, &grid, tiles.len(), list, &draw, &small, hovered);
        }
        Bench::Apps(apps) => {
            let grid = Grid::in_room((x, y, w, h), 52.0, false);
            let footed = grid.foot > 0.0;
            let draw = |hits: &mut Vec<HitBox>, at: usize, (cx, cy): (f32, f32), side: f32, current: bool| {
                let (name, target) = &apps[at];
                let hit = Hit::App(crate::station::apps::number_of(target));
                if current {
                    if lit(hit) {
                        frame.fill_rounded(theme.hover, cx - 3.0, cy - 3.0, side + 6.0, side + 6.0, plate_round(theme, GLASS_ROW_ROUND, 3.0));
                    }
                    frame.key(&format!("app.{target}"));
                }
                paint_app(frame, theme, name, target, cx, cy, side);
                if current {
                    frame.key("");
                    hits.push((cx, cy, side, side, hit));
                    if !footed {
                        name_shown(hits, hit, name, "");
                    }
                }
            };
            let small = |at: usize, (cx, cy): (f32, f32), side: f32| paint_app(frame, theme, &apps[at].0, &apps[at].1, cx, cy, side);
            let hovered = apps.iter().find(|(_, target)| lit(Hit::App(crate::station::apps::number_of(target)))).map(|(name, _)| name.as_str());
            paint_grid(frame, scene, hits, (x, y, w, h), &grid, apps.len(), List::Apps, &draw, &small, hovered);
            hits.push((x, y, w, h, Hit::AppsArea));
        }
        Bench::Notes(notes) => {
            if notes.is_empty() {
                let side = MEDIA_ROW.min(w).min(h);
                paint_add(frame, scene, hits, x + (w - side) / 2.0, y + (h - side) / 2.0, side);
                return;
            }
            // The lane is the note shown, in its colour; the others turned to as pages (the wheel, a swipe, its dots),
            // one sliding in as the other goes.
            let last = (notes.len() - 1) as f32;
            let turned = (scene.scrolls.at(List::Notes) / PAGE).clamp(0.0, last);
            let note = &notes[turned.round() as usize];
            let (title_ink, _, faint) = note_inks(theme, note.colour);
            let title = Font { weight: 700.0, ..theme.body };
            let many = notes.len() > 1;
            // What the words of each leave at the end of it for the dots and a new one.
            let foot = MEDIA_ROW + if many { 14.0 } else { 0.0 } + 6.0;
            let dots_room = w / 3.0;
            let dots = if many { dots_wide(frame, notes.len(), theme.small, dots_room) + 12.0 } else { 0.0 };
            // A note's words on `frame` (its page: see below), as its shape lays them out.
            let words_of = |frame: &dyn Canvas, note: &NoteCard| {
                let (title_ink, words, _) = note_inks(theme, note.colour);
                let first = note.lines.first();
                // Its title across in as many rows as go (three at most), then its lines as far as there is room, down
                // to `bottom`.
                let across = |bottom: f32| {
                    let font = Font { size: theme.body.size - 1.0, ..title };
                    let row = font.size * 1.32;
                    let most = (((bottom - y) / row).floor() as usize).clamp(1, 3);
                    let mut top = y;
                    for line in rows_of(frame, &note.title, font, w, most) {
                        frame.text(&line, font, title_ink, x, top, w + 1.0, Align::Start);
                        top += row;
                    }
                    let small_row = theme.small.size * 1.45;
                    top += 4.0;
                    'lines: for line in &note.lines {
                        let left = ((bottom - top) / small_row).floor() as usize;
                        if left == 0 {
                            break;
                        }
                        for part in rows_of(frame, line, theme.small, w, left) {
                            frame.text(&part, theme.small, words, x, top, w + 1.0, Align::Start);
                            top += small_row;
                            if top + small_row > bottom {
                                break 'lines;
                            }
                        }
                    }
                };
                match shape {
                    Shape::Down => {
                        // Its title written down at the right, its first line beside it where two go across. (Written
                        // down only where each word of it is: Latin letters and figures one under another are not
                        // read, so such a title goes across in rows, its lines under it.)
                        let column = title.size * 1.5;
                        let tall = (h - foot).max(0.0);
                        let down = |text: &str| text.split_whitespace().all(crate::ui::music::written_down);
                        if down(&note.title) {
                            let two = first.is_some_and(|first| down(first)) && w >= 2.0 * column + 2.0;
                            let right = if two { x + w - column / 2.0 } else { x + w / 2.0 };
                            text_down(frame, &note.title, title, title_ink, right, y + 2.0, tall);
                            if let (true, Some(first)) = (two, first) {
                                text_down(frame, first, theme.small, words, right - column, y + 4.0, tall);
                            }
                        } else {
                            across(y + tall);
                        }
                    }
                    Shape::Across => {
                        // Its title over its first line.
                        let room = (w - dots - MEDIA_ROW - 8.0).max(0.0);
                        let middle = y + h / 2.0;
                        let both = first.is_some() && h >= 2.0 * LINE + 6.0;
                        let title_y = if both { middle - LINE - 1.0 } else { middle - LINE / 2.0 - 1.0 };
                        frame.text(&fitted(frame, &note.title, title, room), title, title_ink, x + 2.0, title_y, room, Align::Start);
                        if let (true, Some(first)) = (both, first) {
                            frame.text(&fitted(frame, first, theme.small, room), theme.small, words, x + 2.0, middle + 1.0, room, Align::Start);
                        }
                    }
                    // Too small for words: its title's first character, large.
                    Shape::Least if w.min(h) < 44.0 => {
                        let one: String = note.title.chars().take(1).collect();
                        let big = Font { size: (w.min(h) * 0.5).max(10.0), ..title };
                        centred(frame, &one, big, title_ink, x, y + (h - big.size * 1.3) / 2.0, w);
                    }
                    // Its title and lines across.
                    Shape::Least => across(y + h - if many { LINE + 2.0 } else { 0.0 }),
                }
            };
            // Each note a page, the whole lane at rest. Turned, the pages go smaller, each whole, its own rounded edge and
            // rim round it (see `page_place`); at rest again, the one come to is the lane again. (All of it named as
            // the note shown, its clip with it: drawn again as the widget is sized, the clip is where what is in it
            // is.)
            frame.key(&format!("note.{}", note.number));
            frame.clip(lane.x, lane.y, lane.w, lane.h);
            for (page, each) in notes.iter().enumerate() {
                let off = page as f32 - turned;
                if off.abs() >= 1.0 {
                    continue;
                }
                let (scale, along, shows) = page_place(lane.w, off);
                if shows <= 0.0 || scale <= 0.0 {
                    continue;
                }
                let (left, top) = (lane.x + along, lane.y + (lane.h - lane.h * scale) / 2.0);
                // Drawn as at rest, recorded, and drawn back smaller where it is now.
                let page_frame = super::morph::Recorder::new(frame, scale, (left - lane.x * scale, top - lane.y * scale));
                note_card_fill(&page_frame, theme, each.colour, (lane.x, lane.y, lane.w, lane.h), radius, None);
                words_of(&page_frame, each);
                note_card_rim(&page_frame, theme, each.colour, (lane.x, lane.y, lane.w, lane.h), radius, std::ptr::eq(each, note) && lit(Hit::Note(each.number)));
                let parts: Vec<super::morph::Part> = page_frame.parts().iter().map(|part| part.faded(shows)).collect();
                super::morph::draw(frame, &parts);
            }
            frame.unclip();
            frame.key("");
            // Its dots and a new one where they are whichever is shown, in its ink.
            match shape {
                Shape::Down => {
                    if many {
                        paint_dots(frame, notes.len(), turned, x + w / 2.0, y + h - MEDIA_ROW - 9.0, title_ink, theme.small, w);
                    }
                    paint_add_on(frame, scene, hits, x + (w - MEDIA_ROW) / 2.0, y + h - MEDIA_ROW, MEDIA_ROW, title_ink);
                }
                Shape::Across => {
                    let middle = y + h / 2.0;
                    if many {
                        paint_dots(frame, notes.len(), turned, x + w - MEDIA_ROW - 4.0 - dots / 2.0, middle, title_ink, theme.small, dots_room);
                    }
                    paint_add_on(frame, scene, hits, x + w - MEDIA_ROW, middle - MEDIA_ROW / 2.0, MEDIA_ROW, title_ink);
                }
                // (Which of them it is in the corner.)
                Shape::Least if many && w.min(h) >= 44.0 => {
                    let said = format!("{}/{}", turned.round() as usize + 1, notes.len());
                    frame.text(&said, theme.small, faint, x, y + h - LINE, w, Align::End);
                }
                Shape::Least => {}
            }
            hits.push((lane.x, lane.y, lane.w, lane.h, Hit::Note(note.number)));
            hits.push((lane.x, lane.y, lane.w, lane.h, Hit::Pages(List::Notes, (last * PAGE).to_bits())));
        }
        Bench::Time { figure, unit, date } => {
            let big = Font { size: theme.figure.size.min(h * 0.6).min(w / 3.2), ..theme.figure };
            match shape {
                Shape::Down => {
                    // Its hours over its minutes.
                    let (hours, minutes) = figure.split_once(':').unwrap_or((figure.as_str(), ""));
                    let big = Font { size: theme.figure.size.min(w * 0.55), ..theme.figure };
                    // (Named as its whole lane names them, that they move between the two: see `morph`.)
                    frame.key("time.value");
                    centred(frame, hours, big, theme.text, x, y + h / 2.0 - big.size * 1.1, w);
                    centred(frame, minutes, big, theme.text, x, y + h / 2.0, w);
                    if !unit.is_empty() {
                        frame.key("time.unit");
                        centred(frame, unit, theme.small, theme.text3, x, y + h / 2.0 + big.size * 1.15, w);
                    }
                    frame.key("");
                }
                _ => {
                    let shown = if unit.is_empty() { figure.clone() } else { format!("{figure} {unit}") };
                    let fig_w = frame.measure(&shown, big);
                    let date_w = frame.measure(date, theme.small);
                    let with_date = shape == Shape::Across && fig_w + 12.0 + date_w <= w;
                    let start = x + (w - fig_w - if with_date { 12.0 + date_w } else { 0.0 }) / 2.0;
                    frame.key("time.value");
                    frame.text(&shown, big, theme.text, start, y + (h - big.size * 1.2) / 2.0, fig_w + 1.0, Align::Start);
                    if with_date {
                        frame.key("time.detail");
                        frame.text(date, theme.small, theme.text3, start + fig_w + 12.0, y + (h - LINE) / 2.0, date_w + 1.0, Align::Start);
                    }
                    frame.key("");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {

    /// The frosted glass's sliders, at rest, under the pointer and held, on a light and a dark ground:
    /// `cargo test --features studio --lib -- --ignored glass_sliders` writes `target/glass-sliders.png`.
    #[test]
    #[ignore = "writes a picture to look at"]
    #[cfg(feature = "studio")]
    fn glass_sliders() {
        let _device = crate::ui::gfx::held().unwrap();
        use crate::ui::theme::{Skin, Theme};
        let gfx = crate::ui::gfx::Gfx::new().expect("graphics");
        let (w, h) = (360.0f32, 200.0f32);
        let pixels = gfx
            .draw_offscreen(((w * 2.0) as u32, (h * 2.0) as u32), 2.0, |frame| {
                for (row, dark) in [false, true].into_iter().enumerate() {
                    let theme = Theme::new(Skin::Glass, dark);
                    let y0 = row as f32 * 100.0;
                    frame.fill(if dark { Color::hex(0x2A3550, 1.0) } else { Color::hex(0xC9D6E8, 1.0) }, 0.0, y0, w, 100.0);
                    for (i, dot) in [1.0f32, 1.4, 0.85].into_iter().enumerate() {
                        super::slider(frame, &theme, super::super::theme::accent(dark), 20.0, y0 + 25.0 + i as f32 * 25.0, 320.0, 0.3 + 0.2 * i as f32, Some(dot));
                    }
                }
            })
            .expect("drawn");
        crate::studio::save_png(&std::path::PathBuf::from(format!("{}/target/glass-sliders.png", env!("CARGO_MANIFEST_DIR"))), (w * 2.0) as u32, (h * 2.0) as u32, &pixels).unwrap();
    }

    /// The workbench's panel with quick launch as its rail, at the left, right and top edges (the columns), in each
    /// skin, light and dark (the rows): `cargo test --features studio --lib -- --ignored workbench_rails` writes
    /// `target/rails/<row>-<column>.png`.
    #[test]
    #[ignore = "writes a picture to look at"]
    #[cfg(feature = "studio")]
    fn workbench_rails() {
        let _device = crate::ui::gfx::held().unwrap();
        use super::*;
        let _pretending = crate::pretending();
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        crate::station::thumbs::start(|| {});
        let app = |target: &str, name: &str| crate::station::apps::App { target: target.into(), name: name.into() };
        crate::station::apps::pretend(vec![
            app(r"C:\Windows\System32
otepad.exe", "Notepad"),
            app(r"C:\Windows\explorer.exe", "Explorer"),
            app(r"shell:AppsFolder\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App", "Calculator"),
            app("https://github.com/lulu-loopp/glance", "github.com"),
            app(r"C:\Windows\System32\cmd.exe", "Command"),
        ]);
        crate::station::run::pretend(Vec::new(), ShelfView::Cards);
        let sides = [RailSide::Start, RailSide::End, RailSide::Bottom];
        let rows = [(Skin::Glass, false), (Skin::Fluent, false), (Skin::Fluent, true), (Skin::Paper, false), (Skin::Paper, true)];
        let gfx = crate::ui::gfx::Gfx::new().expect("graphics");
        let out = std::path::PathBuf::from(format!("{}/target/rails", env!("CARGO_MANIFEST_DIR")));
        std::fs::create_dir_all(&out).unwrap();
        for pass in 0..2 {
            for (row, (skin, dark)) in rows.iter().enumerate() {
                let theme = Theme::new(*skin, *dark);
                let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["station".to_string()]));
                let history: &'static [Sample] = Box::leak(Box::new([sample(None, None, None)]));
                let seen = Seen::of(history);
                let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::Zh, history, seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: Some(Hit::App(crate::station::apps::number_of(r"C:\Windows\explorer.exe"))), thumbs: &RESTING, pinned: true, overlay: false, buttons: true, newer: false, settings_lit: None };
                let lanes = vec![
                    Lane { id: "station".into(), blocks: station_lane(true, Tab::Shelf, Lang::Zh), ink: theme.ink("station") },
                    Lane { id: "apps".into(), blocks: apps_lane(true, &theme, Lang::Zh), ink: theme.ink("apps") },
                ];
                for (column, side) in sides.iter().enumerate() {
                    let heights: Vec<f32> = lanes.iter().map(|lane| lane.height(&theme)).collect();
                    let layout = Layout::with_columns(vec![heights[0]], 1, &theme).with_rail(1, *side, RAIL_LEAST);
                    let (w, h) = (layout.width(), layout.height());
                    let ground = match skin {
                        Skin::Paper => theme.paper,
                        Skin::Fluent => if *dark { Color::hex(0x24262A, 1.0) } else { Color::hex(0xF3F5F8, 1.0) },
                        Skin::Glass => Color::hex(0xE4EAF2, 1.0),
                    };
                    let pixels = gfx.draw_offscreen(((w * 1.5) as u32, (h * 1.5) as u32), 1.5, |frame| {
                        frame.fill(ground, 0.0, 0.0, w, h);
                        paint(frame, &scene, &lanes, &layout, Pass::Content);
                    }).expect("drawn");
                    if pass == 1 {
                        crate::studio::save_png(&out.join(format!("{row}-{column}.png")), (w * 1.5) as u32, (h * 1.5) as u32, &pixels).unwrap();
                    }
                }
            }
            // (The pictures made meanwhile, on the station's own thread: drawn again with them.)
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
    }

    /// The workbench (the time, the station, the notes, quick launch as its rail) beside the readings' panel (the
    /// processor, memory), each on its skin's ground over a wallpaper, as the panels are drawn: for each skin and its
    /// light and dark (`cargo test --features studio --lib -- --ignored workbench_looks` writes
    /// `target/looks/<skin>-<light|dark>.png`).
    #[test]
    #[ignore = "writes pictures to look at"]
    #[cfg(feature = "studio")]
    fn workbench_looks() {
        let _device = crate::ui::gfx::held().unwrap();
        use super::*;
        let _pretending = crate::pretending();
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        crate::station::thumbs::start(|| {});
        pretend_workbench();
        let (width, height, px) = (900u32, 760u32, 1.5f32);
        let desktop = crate::ui::wallpaper::picture(r"C:\Windows\Web\Wallpaper\Windows\img19.jpg", (width as f32 * px) as u32, (height as f32 * px) as u32).expect("wallpaper");
        let gfx = crate::ui::gfx::Gfx::new().expect("graphics");
        let out = std::path::PathBuf::from(format!("{}/target/looks", env!("CARGO_MANIFEST_DIR")));
        std::fs::create_dir_all(&out).unwrap();
        for pass in 0..2 {
            for skin in [Skin::Glass, Skin::Fluent, Skin::Paper] {
                for dark in [false, true] {
                    let theme = Theme::new(skin, dark);
                    let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["station".to_string()]));
                    let history: &'static [Sample] = Box::leak(Box::new([sample(Some(4200.0), None, None)]));
                    let seen = Seen::of(history);
                    let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::Zh, history, seen: &seen, pen_ms: history[0].t as f64, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
                    let made = |ids: &[&str]| -> Vec<Lane> { ids.iter().filter_map(|id| Some(Lane { id: id.to_string(), blocks: lane(&scene, id, Detail::Full)?, ink: theme.ink(id) })).collect() };
                    let bench = made(&["time", "station", "notes", "apps"]);
                    let readings = made(&["cpu", "memory"]);
                    let heights = |lanes: &[Lane]| -> Vec<f32> { lanes.iter().map(|lane| lane.height(&theme)).collect() };
                    let bench_layout = Layout::with_columns(heights(&bench)[..3].to_vec(), 1, &theme).with_rail(3, RailSide::Start, RAIL_LEAST);
                    let readings_layout = Layout::with_columns(heights(&readings), 1, &theme);
                    let pixels = gfx.draw_offscreen(((width as f32 * px) as u32, (height as f32 * px) as u32), px, |frame| {
                        if let Ok(bitmap) = desktop.bitmap(&frame.dc, px) {
                            unsafe { frame.dc.DrawBitmap(&bitmap, None, 1.0, windows::Win32::Graphics::Direct2D::D2D1_INTERPOLATION_MODE_LINEAR, None, None) };
                        }
                        let behind = desktop.bitmap(&frame.dc, px).ok();
                        for (lanes, layout, at) in [(&bench, &bench_layout, (30.0f32, 30.0f32)), (&readings, &readings_layout, (40.0 + bench_layout.width() + 30.0, 30.0))] {
                            let backdrop = behind.as_ref().map(|bitmap| (bitmap, windows_numerics::Vector2 { X: at.0, Y: at.1 }, desktop.digest, px));
                            let picture = crate::ui::render::Picture { scene: &scene, lanes, layout, small: None, size: (layout.width(), layout.height()), at: (0.0, 0.0), edge: None, backdrop, frost: 0.4, rough: false };
                            let mut layers = crate::ui::render::PanelLayers::default();
                            let _ = layers.draw(frame, &picture, windows_numerics::Matrix3x2::translation(at.0, at.1), px);
                        }
                    }).expect("drawn");
                    if pass == 1 {
                        let name = format!("{}-{}.png", match skin { Skin::Glass => "glass", Skin::Fluent => "fluent", Skin::Paper => "paper" }, if dark { "dark" } else { "light" });
                        crate::studio::save_png(&out.join(name), (width as f32 * px) as u32, (height as f32 * px) as u32, &pixels).unwrap();
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
    }

    /// For pictures: the workbench as the studio films it.
    #[cfg(feature = "studio")]
    fn pretend_workbench() {
        let props = std::path::PathBuf::from(format!("{}/target/props", env!("CARGO_MANIFEST_DIR")));
        crate::studio::pretend_workbench(Lang::Zh, &props).unwrap();
    }

    /// The workbench's lanes each a widget of its own in every shape (a strip down, one across, the least), and all
    /// four as one in each, in each skin, light and dark (`cargo test --features studio --lib -- --ignored
    /// workbench_shapes` writes `target/shapes/<skin>-<light|dark>.png`).
    #[test]
    #[ignore = "writes pictures to look at"]
    #[cfg(feature = "studio")]
    fn workbench_shapes() {
        let _device = crate::ui::gfx::held().unwrap();
        use super::*;
        let _pretending = crate::pretending();
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        crate::station::thumbs::start(|| {});
        pretend_workbench();
        let (width, height, px) = (1600u32, 900u32, 1.5f32);
        let desktop = crate::ui::wallpaper::picture(r"C:\Windows\Web\Wallpaper\Windows\img19.jpg", (width as f32 * px) as u32, (height as f32 * px) as u32).expect("wallpaper");
        let gfx = crate::ui::gfx::Gfx::new().expect("graphics");
        let out = std::path::PathBuf::from(format!("{}/target/shapes", env!("CARGO_MANIFEST_DIR")));
        std::fs::create_dir_all(&out).unwrap();
        let ids = ["station", "notes", "apps", "time"];
        for pass in 0..2 {
            for skin in [Skin::Glass, Skin::Fluent, Skin::Paper] {
                for dark in [false, true] {
                    let mut theme = Theme::new(skin, dark);
                    (theme.bar_gap, theme.bar_height) = (0.0, 0.0);
                    let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["station".to_string()]));
                    let history: &'static [Sample] = Box::leak(Box::new([sample(Some(4200.0), None, None)]));
                    let seen = Seen::of(history);
                    let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::Zh, history, seen: &seen, pen_ms: history[0].t as f64, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
                    // Each widget: its lanes, its shape, its size, where it is, and how far its notes are turned (pages).
                    type Widget<'a> = (Vec<&'a str>, Shape, (f32, f32), (f32, f32), f32);
                    let mut widgets: Vec<Widget> = Vec::new();
                    for (at, id) in ids.iter().enumerate() {
                        let x = 30.0 + at as f32 * 90.0;
                        widgets.push((vec![id], Shape::Down, (70.0, 300.0), (x, 30.0), 0.0));
                        widgets.push((vec![id], Shape::Least, (70.0, 70.0), (x, 360.0), 0.0));
                        widgets.push((vec![id], Shape::Least, (44.0, 44.0), (x + 12.0, 450.0), 0.0));
                        widgets.push((vec![id], Shape::Across, (330.0, 64.0), (420.0, 30.0 + at as f32 * 84.0), 0.0));
                    }
                    widgets.push((ids.to_vec(), Shape::Across, (640.0, 70.0), (420.0, 380.0), 0.0));
                    widgets.push((ids.to_vec(), Shape::Down, (80.0, 220.0), (790.0, 30.0), 0.0));
                    widgets.push((ids.to_vec(), Shape::Least, (120.0, 120.0), (900.0, 30.0), 0.0));
                    // The notes part way to their next page: each a whole rounded page, smaller, side by side.
                    widgets.push((vec!["notes"], Shape::Least, (150.0, 150.0), (1100.0, 30.0), 0.1));
                    widgets.push((vec!["notes"], Shape::Least, (150.0, 150.0), (1100.0, 200.0), 0.25));
                    widgets.push((vec!["notes"], Shape::Least, (150.0, 150.0), (1270.0, 200.0), 0.5));
                    widgets.push((vec!["notes"], Shape::Down, (70.0, 300.0), (1440.0, 30.0), 0.5));
                    widgets.push((vec!["notes"], Shape::Down, (70.0, 300.0), (1530.0, 30.0), 0.0));
                    widgets.push((vec!["notes"], Shape::Across, (330.0, 64.0), (1100.0, 380.0), 0.5));
                    // Squares, as a phone's large folders: two by two, three by three.
                    for (at, id) in ids[..3].iter().enumerate() {
                        widgets.push((vec![id], Shape::Least, (150.0, 150.0), (30.0 + at as f32 * 170.0, 540.0), 0.0));
                        widgets.push((vec![id], Shape::Least, (300.0, 330.0), (560.0 + at as f32 * 330.0, 540.0), 0.0));
                    }
                    let pixels = gfx.draw_offscreen(((width as f32 * px) as u32, (height as f32 * px) as u32), px, |frame| {
                        if let Ok(bitmap) = desktop.bitmap(&frame.dc, px) {
                            unsafe { frame.dc.DrawBitmap(&bitmap, None, 1.0, windows::Win32::Graphics::Direct2D::D2D1_INTERPOLATION_MODE_LINEAR, None, None) };
                        }
                        let behind = desktop.bitmap(&frame.dc, px).ok();
                        for (lane_ids, shape, size, at, turned) in &widgets {
                            let mut scrolls = Scrolls::default();
                            scrolls.hold_pages(List::Notes, turned * PAGE, 2.0 * PAGE);
                            let scene = Scene { scrolls, ..scene };
                            let lanes: Vec<Lane> = lane_ids.iter().filter_map(|id| Some(Lane { id: id.to_string(), blocks: lane(&scene, id, Detail::Shape(*shape))?, ink: theme.ink(id) })).collect();
                            let heights: Vec<f32> = lanes.iter().map(|lane| lane.height(&theme)).collect();
                            let layout = Layout::with_columns(heights, shape.columns(lanes.len()), &theme).fitted(*size);
                            let backdrop = behind.as_ref().map(|bitmap| (bitmap, windows_numerics::Vector2 { X: at.0, Y: at.1 }, desktop.digest, px));
                            let picture = crate::ui::render::Picture { scene: &scene, lanes: &lanes, layout: &layout, small: None, size: (layout.width(), layout.height()), at: (0.0, 0.0), edge: None, backdrop, frost: 0.4, rough: false };
                            let mut layers = crate::ui::render::PanelLayers::default();
                            let _ = layers.draw(frame, &picture, windows_numerics::Matrix3x2::translation(at.0, at.1), px);
                        }
                    }).expect("drawn");
                    if pass == 1 {
                        let name = format!("{}-{}.png", match skin { Skin::Glass => "glass", Skin::Fluent => "fluent", Skin::Paper => "paper" }, if dark { "dark" } else { "light" });
                        crate::studio::save_png(&out.join(name), (width as f32 * px) as u32, (height as f32 * px) as u32, &pixels).unwrap();
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
    }

    /// The notes' lane as a deck, cards and a list (the columns) in each skin, light and dark (the rows):
    /// `cargo test --features studio --lib -- --ignored notes_lanes` writes `target/notes/<row>-<column>.png`.
    #[test]
    #[ignore = "writes pictures to look at"]
    #[cfg(feature = "studio")]
    fn notes_lanes() {
        let _device = crate::ui::gfx::held().unwrap();
        use super::*;
        let _pretending = crate::pretending();
        use crate::notes::{Colour, Note};
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let now = crate::station::now_ms();
        let note = |name: &str, text: &str, colour: Colour, ago_min: u64| Note { name: name.into(), text: text.into(), colour, stamp: crate::notes::store::Stamp { bytes: 0, written_ms: now - ago_min * 60_000, digest: 0 } };
        crate::notes::pretend(vec![
            note("a.md", "# 本周要做的事
- [x] 发布 0.3.13，回复 issue 17
- [ ] 中转站：**三种视图**的设计确认
- [ ] 便签输入法试验
周五前合并 `feature/station`", Colour::Blue, 3),
            note("b.md", "显卡价格记录
5070 Ti 京东 6299", Colour::Yellow, 60 * 20),
            note("c.md", "周末开车听的歌
- 敢爱敢做", Colour::Green, 60 * 50),
            note("d.md", "家里网络
路由器管理地址 192.168.31.1", Colour::Pink, 60 * 200),
            note("e.md", "读书笔记：设计心理学
好的设计让人一眼看出怎么用", Colour::Purple, 60 * 300),
        ]);
        let views = [crate::settings::NotesView::Deck, crate::settings::NotesView::Cards, crate::settings::NotesView::List];
        let rows = [(Skin::Glass, false), (Skin::Fluent, true), (Skin::Paper, false)];
        let gfx = crate::ui::gfx::Gfx::new().expect("graphics");
        let out = std::path::PathBuf::from(format!("{}/target/notes", env!("CARGO_MANIFEST_DIR")));
        std::fs::create_dir_all(&out).unwrap();
        for (row, (skin, dark)) in rows.iter().enumerate() {
            let theme = Theme::new(*skin, *dark);
            let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["station".to_string()]));
            let history: &'static [Sample] = Box::leak(Box::new([sample(None, None, None)]));
            let seen = Seen::of(history);
            let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::Zh, history, seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: Some(Hit::Note(crate::notes::number_of("c.md"))), thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
            for (column, view) in views.iter().enumerate() {
                let mut blocks = notes_lane(true, Lang::Zh);
                if let Some(Block::Notes { view: shown, .. }) = blocks.last_mut() {
                    *shown = *view;
                }
                let lane = Lane { id: "notes".into(), blocks, ink: theme.ink("notes") };
                let (w, h) = (COLUMN_WIDTH, lane.height(&theme));
                let ground = match skin {
                    Skin::Paper => theme.paper,
                    Skin::Fluent => if *dark { Color::hex(0x24262A, 1.0) } else { Color::hex(0xF3F5F8, 1.0) },
                    Skin::Glass => Color::hex(0xE4EAF2, 1.0),
                };
                let pixels = gfx.draw_offscreen(((w * 1.5) as u32, (h * 1.5) as u32), 1.5, |frame| {
                    frame.fill(ground, 0.0, 0.0, w, h);
                    paint_lane(frame, &scene, &lane, Rect { x: 0.0, y: 0.0, w, h }, 0.0, Pass::Content, &mut Vec::new());
                }).expect("drawn");
                crate::studio::save_png(&out.join(format!("{row}-{column}.png")), (w * 1.5) as u32, (h * 1.5) as u32, &pixels).unwrap();
            }
        }
    }

    /// The station's lane in each skin, light and dark (the rows), as cards, a list, pictures, what was copied lately,
    /// and Recent files as pictures and as a list, files come together a row (the columns), holding files of this
    /// repository as Explorer pictures them, text and links:
    /// `cargo test --features studio --lib -- --ignored station_pages` writes `target/station-pages.png`.
    #[test]
    #[ignore = "writes a picture to look at"]
    #[cfg(feature = "studio")]
    fn station_pages() {
        let _device = crate::ui::gfx::held().unwrap();
        use super::*;
        let _pretending = crate::pretending();
        use crate::station::shelf::{Item, What};
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let file = |name: &str| std::path::PathBuf::from(format!("{}/{name}", env!("CARGO_MANIFEST_DIR"))).components().collect::<std::path::PathBuf>();
        let file_at = file;
        let now = crate::station::now_ms();
        let item = |id: u64, what: What, copied: bool, ago_min: u64| Item { id, what, added_ms: now - ago_min * 60_000, pinned: id == 2, copied, stays_here: false, own_bytes: 0 };
        let items = || vec![
            item(10, What::Files { paths: vec![file("README.zh-CN.md")], kept: Vec::new() }, true, 2),
            item(11, What::Link { url: "https://glancepc.com/app".into() }, true, 9),
            item(12, What::Text { text: "cargo build --profile fast".into() }, true, 30),
            item(1, What::Files { paths: vec![file("icons/128x128@2x.png")], kept: Vec::new() }, false, 400),
            item(2, What::Text { text: "会议改到周四下午三点
地点：二楼小会议室
记得带上季度报表".into() }, false, 180),
            item(3, What::Link { url: "https://github.com/lulu-loopp/glance/releases".into() }, false, 60),
            item(4, What::Files { paths: vec![file("Cargo.toml"), file("README.md"), file("LICENSE")], kept: Vec::new() }, false, 20),
            item(5, What::Files { paths: vec![file("docs/images")], kept: Vec::new() }, false, 3),
            item(6, What::Files { paths: vec![file("docs/images/banner-zh.webp")], kept: Vec::new() }, false, 1),
        ];
        crate::station::thumbs::start(|| {});
        {
            use crate::station::recent::{Came, Kind};
            let came = |file: u128, name: &str, ago_min: u64, bytes: u64, kind: Kind, site: Option<&str>| Came { key: crate::station::recent::Key { drive: 'C', file }, path: file_at(name), came_ms: now - ago_min * 60_000, bytes, kind, downloaded: site.is_some(), site: site.map(str::to_string) };
            let mut files = vec![
                came(1, "docs/images/banner-zh.webp", 3, 182_000, Kind::Screenshot, None),
                came(2, "Cargo.toml", 25, 6_100_000, Kind::Archive, Some("github.com")),
                came(3, "README.zh-CN.md", 70, 21_000, Kind::Document, None),
                came(4, "icons/128x128@2x.png", 26 * 60, 5_600_000, Kind::Picture, Some("wallhaven.cc")),
                came(5, "README.md", 4 * 24 * 60, 64_000, Kind::Document, None),
            ];
            // Pictures copied into one folder together, a second apart.
            let names = ["banner-zh.webp", "desktop-zh.webp", "looks-zh.webp", "music.webp", "widgets-zh.webp", "workbench-zh.webp"];
            for (at, name) in names.iter().enumerate() {
                files.push(Came { came_ms: now - 40 * 60_000 + at as u64 * 1_000, ..came(20 + at as u128, &format!("docs/images/{name}"), 0, 300_000, Kind::Picture, None) });
            }
            files.sort_by_key(|came| std::cmp::Reverse(came.came_ms));
            crate::station::recent::pretend(files);
        }
        let (pages, lanes) = ([ShelfView::Cards, ShelfView::List, ShelfView::Large], 6);
        let (lane_w, lane_h) = (COLUMN_WIDTH, 360.0);
        let rows = [(Skin::Glass, false), (Skin::Glass, true), (Skin::Fluent, false), (Skin::Fluent, true), (Skin::Paper, false), (Skin::Paper, true)];
        let (w, h) = (lanes as f32 * (lane_w + 12.0), rows.len() as f32 * (lane_h + 12.0));
        let gfx = crate::ui::gfx::Gfx::new().expect("graphics");
        let draw = || gfx.draw_offscreen(((w * 2.0) as u32, (h * 2.0) as u32), 2.0, |frame| {
            frame.fill(Color::hex(0x7A8CA6, 1.0), 0.0, 0.0, w, h);
            for (row, (skin, dark)) in rows.iter().enumerate() {
                let theme = Theme::new(*skin, *dark);
                for column in 0..lanes {
                    let (x, y) = (column as f32 * (lane_w + 12.0) + 6.0, row as f32 * (lane_h + 12.0) + 6.0);
                    crate::station::run::pretend(items(), pages[if column == 5 { 1 } else { column.min(2) }]);
                    let tab = match column {
                        3 => Tab::Copies(Default::default()),
                        4 | 5 => Tab::Recent(Default::default()),
                        _ => Tab::Shelf,
                    };
                    let ground = match skin {
                        Skin::Paper => theme.paper,
                        Skin::Fluent => if *dark { Color::hex(0x24262A, 1.0) } else { Color::hex(0xF3F5F8, 1.0) },
                        Skin::Glass => if *dark { Color::hex(0x2A3550, 0.92) } else { Color::hex(0xE4EAF2, 0.9) },
                    };
                    frame.fill_rounded(ground, x, y, lane_w, lane_h, theme.radius);
                    let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["station".to_string()]));
                    let history: &'static [Sample] = Box::leak(Box::new([sample(None, None, None)]));
                    let seen = Seen::of(history);
                    let hover = match column {
                        0 => Some(Hit::Shelf(3)),
                        4 => Some(Hit::Recent(crate::station::recent::Entry::File(crate::station::recent::Key { drive: 'C', file: 2 }))),
                        5 => Some(Hit::Recent(crate::station::recent::Entry::Burst(Default::default(), crate::station::recent::Key { drive: 'C', file: 25 }))),
                        _ => None,
                    };
                    let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::Zh, history, seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab, hover, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
                    let lane = Lane { id: "station".into(), blocks: station_lane(true, tab, Lang::Zh), ink: theme.ink("station") };
                    let area = Rect { x, y, w: lane_w, h: lane.height(&theme) };
                    paint_lane(frame, &scene, &lane, area, 0.0, Pass::Content, &mut Vec::new());
                }
            }
        }).expect("drawn");
        // (The pictures made meanwhile, on the station's own thread: drawn again with them.)
        let _ = draw();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let pixels = draw();
        crate::studio::save_png(&std::path::PathBuf::from(format!("{}/target/station-pages.png", env!("CARGO_MANIFEST_DIR"))), (w * 2.0) as u32, (h * 2.0) as u32, &pixels).unwrap();
    }
    use super::*;

    #[test]
    fn splits_lanes_into_the_shortest_columns() {
        let heights = [100.0, 200.0, 50.0, 150.0];
        assert_eq!(split(&heights, 1, 0.0), (vec![0], 500.0));
        // [100, 200] | [50, 150] beats [100] | [200, 50, 150].
        assert_eq!(split(&heights, 2, 0.0), (vec![0, 2], 300.0));
        assert_eq!(split(&heights, 3, 0.0).1, 200.0);
        // Gaps count between the lanes of a column, not after the last.
        assert_eq!(split(&heights, 2, 8.0), (vec![0, 2], 308.0));
        assert_eq!(split(&[], 1, 8.0), (vec![0], 0.0));
    }

    #[test]
    fn lays_out_a_panel_with_no_lanes() {
        let theme = Theme::new(Skin::Paper, false);
        let layout = Layout::new(Vec::new(), 1000.0, 3, &theme);
        assert_eq!(layout.columns, 1);
        assert!(layout.lanes().is_empty());
        assert_eq!(layout.height(), theme.bar_height);
    }

    #[test]
    fn takes_the_fewest_columns_that_fit() {
        let theme = Theme::new(Skin::Paper, false);
        let bar = theme.bar_height;
        let heights = vec![300.0, 300.0, 300.0];
        assert_eq!(Layout::new(heights.clone(), 900.0 + bar, 3, &theme).columns, 1);
        assert_eq!(Layout::new(heights.clone(), 700.0, 3, &theme).columns, 2);
        // Too short for even the most columns: the most, to be zoomed out.
        assert_eq!(Layout::new(heights, 200.0, 2, &theme).columns, 2);
    }

    #[test]
    fn deals_lanes_into_level_columns() {
        let theme = Theme::new(Skin::Glass, false);
        let layout = Layout::new(vec![100.0, 200.0, 50.0, 150.0], 400.0, 2, &theme);
        let boxes = layout.lanes();
        assert_eq!(layout.columns, 2);
        assert_eq!(boxes[2].x, COLUMN_WIDTH + theme.column_gap);
        // Both columns end at the same line.
        let end = |r: &Rect| r.y + r.h;
        assert_eq!(end(&boxes[1]), end(&boxes[3]));
        assert_eq!(layout.width(), 2.0 * COLUMN_WIDTH + theme.column_gap);
    }

    fn sample(clock: Option<f32>, battery: Option<u8>, fan: Option<f32>) -> Sample {
        use crate::reading::*;
        Sample {
            t: 0,
            cpu: Some(10.0),
            threads: vec![Some(10.0); 4],
            ghz: Some(3.0),
            kinds_ghz: Vec::new(),
            threads_ghz: Vec::new(),
            memory: MemorySample { used: 1 << 30, committed: 1 << 30, commit_limit: 1 << 31, cached: 0 },
            gpus: vec![GpuSample { usage: Some(5.0), engines: Some(Vec::new()), mem_used: Some(0), shared_used: Some(0), temp: Some(50.0), hot_spot: None, clock_mhz: clock, fan_rpm: None, power: None }],
            net_down: Some(0.0),
            net_up: Some(0.0),
            net_total_down: 0,
            net_total_up: 0,
            ping_ms: None,
            network: None,
            disk_read: Some(0.0),
            disk_write: Some(0.0),
            disk_active: Some(0.0),
            volumes: Vec::new(),
            processes: Vec::new(),
            system: SystemSample { uptime_s: 60, processes: 1, threads: 1, handles: 1 },
            battery: battery.map(|percent| BatterySample { percent, charging: false, plugged: false, seconds_left: None, watts: None, health: None }),
            cpu_sensors: None,
            board: fan.map(|rpm| BoardSensors { temps: Vec::new(), fans: vec![("1".into(), rpm)] }),
            drive_temps: Vec::new(),
            dimm_temps: Vec::new(),
            mic_muted: None,
            game: None,
            wsl: None,
            docker: None,
        }
    }

    fn info() -> StaticInfo {
        use crate::reading::GpuInfo;
        StaticInfo {
            cpu_name: "CPU".into(),
            memory_modules: None,
            memory_speed: None,
            drives: Vec::new(),
            drive_ids: Vec::new(),
            network_adapter: None,
            board: "Board".into(),
            threads: 4,
            mem_total: 1 << 32,
            gpus: vec![GpuInfo { slot: 0, identity: String::new(), name: "GPU".into(), mem_total: 1 << 30, shared_total: 0 }],
            found: Vec::new(),
        }
    }

    /// The facts of a lane: label and value of each row.
    fn facts(blocks: &[Block]) -> Vec<(String, String)> {
        blocks
            .iter()
            .filter_map(|block| match block {
                Block::Facts { rows, .. } => Some(rows.iter().map(|(label, value, _)| (label.clone(), value.clone()))),
                _ => None,
            })
            .flatten()
            .collect()
    }

    #[test]
    fn tags_the_thread_pointed_at() {
        let (info, prefs, theme) = (info(), Prefs::default(), Theme::new(Skin::Paper, false));
        let leak = |history: Vec<Sample>| -> &'static [Sample] { Box::leak(history.into_boxed_slice()) };
        let mut read = sample(None, None, None);
        read.threads_ghz = vec![Some(5.0), Some(4.5), Some(5.4), Some(4.2)];
        let history = leak(vec![read]);
        let seen = Box::leak(Box::new(Seen::of(history)));
        let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::En, history, seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
        let cpu = lane(&scene, "cpu", Detail::Full).unwrap();
        let tags: Vec<&String> = cpu.iter().filter_map(|block| if let Block::Threads(cells) = block { Some(cells.iter().map(|(_, _, tag)| tag)) } else { None }).flatten().collect();
        assert_eq!(tags[2], "Thread 3 · 5.40 GHz · 10%");
    }

    #[test]
    fn shows_each_kind_of_cores_clock() {
        let (info, prefs, theme) = (info(), Prefs::default(), Theme::new(Skin::Paper, false));
        let leak = |history: Vec<Sample>| -> &'static [Sample] { Box::leak(history.into_boxed_slice()) };
        let scene = |history: &'static [Sample]| Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::En, history, seen: Box::leak(Box::new(Seen::of(history))), pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
        // Big and little cores: each kind's clock, in place of the one clock.
        let mut hybrid = sample(None, None, None);
        hybrid.kinds_ghz = vec![Some(5.2), None];
        let cpu = lane(&scene(leak(vec![hybrid])), "cpu", Detail::Full).unwrap();
        let shown = facts(&cpu);
        assert!(shown.contains(&("P-core clock".into(), "5.20 GHz".into())) && shown.contains(&("E-core clock".into(), UNREAD.into())));
        assert!(!shown.iter().any(|(label, _)| label == "Clock"));
    }

    #[test]
    fn keeps_a_reading_in_place_while_it_is_missing() {
        let (info, prefs, theme) = (info(), Prefs::default(), Theme::new(Skin::Paper, false));
        let scene = |history: &'static [Sample]| Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::En, history, seen: Box::leak(Box::new(Seen::of(history))), pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
        let leak = |history: Vec<Sample>| -> &'static [Sample] { Box::leak(history.into_boxed_slice()) };
        // The GPU's clock, the battery and a fan read a moment ago, and not now.
        let history = leak(vec![sample(Some(1350.0), Some(80), Some(900.0)), sample(None, None, None)]);
        let gpu = lane(&scene(history), "gpu:0", Detail::Full).unwrap();
        assert!(facts(&gpu).contains(&("Clock".into(), UNREAD.into())));
        let battery = lane(&scene(history), "battery", Detail::Full).expect("the battery lane stays");
        assert!(matches!(&battery[1], Block::Readout { figure, .. } if figure == UNREAD));
        let board = lane(&scene(history), "board", Detail::Full).expect("the board lane stays");
        assert_eq!(facts(&board), vec![("Fan 1".to_string(), UNREAD.to_string())]);
        // Read again: shown.
        let history = leak(vec![sample(None, None, None), sample(Some(1350.0), Some(80), Some(900.0))]);
        assert!(facts(&lane(&scene(history), "gpu:0", Detail::Full).unwrap()).contains(&("Clock".into(), "1350 MHz".into())));
        // Never read in the span: no row, no lane.
        let history = leak(vec![sample(None, None, None), sample(None, None, None)]);
        assert!(!facts(&lane(&scene(history), "gpu:0", Detail::Full).unwrap()).iter().any(|(label, _)| label == "Clock"));
        assert!(lane(&scene(history), "battery", Detail::Full).is_none());
        assert!(lane(&scene(history), "board", Detail::Full).is_none());
    }

    #[test]
    fn tells_readings_apart_by_what_they_are_of() {
        use crate::reading::{CpuSensors, DriveTemperature};
        let (info, prefs, theme) = (info(), Prefs::default(), Theme::new(Skin::Paper, false));
        let scene = |history: &'static [Sample]| Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::En, history, seen: Box::leak(Box::new(Seen::of(history))), pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
        let leak = |history: Vec<Sample>| -> &'static [Sample] { Box::leak(history.into_boxed_slice()) };
        let with = |ccds: Vec<(usize, f32)>, drives: Vec<(u32, f32)>| {
            let mut s = sample(None, None, None);
            s.cpu_sensors = Some(CpuSensors { temp: Some(50.0), ccds, power: None, graphics_power: None });
            s.drive_temps = drives.into_iter().map(|(id, celsius)| DriveTemperature { id, name: "SSD".into(), celsius }).collect();
            s
        };
        // Two drives of one model: a row each.
        let history = leak(vec![with(Vec::new(), vec![(0, 35.0), (1, 95.0)])]);
        let disk = facts(&lane(&scene(history), "disk", Detail::Full).unwrap());
        assert_eq!(&disk[..2], [("SSD".to_string(), "35 °C".to_string()), ("SSD".to_string(), "95 °C".to_string())]);
        // A drive that gives no temperature (a SATA one unread) has its row
        // all the same, by its name.
        let mut two = self::info();
        two.drives = vec!["NVMe".into(), "SATA".into()];
        two.drive_ids = vec![0, 1];
        let history = leak(vec![with(Vec::new(), vec![(0, 35.0)])]);
        let scene_two = Scene { music_apart: false, info: &two, prefs: &prefs, theme: &theme, lang: Lang::En, history, seen: Box::leak(Box::new(Seen::of(history))), pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
        let disk = facts(&lane(&scene_two, "disk", Detail::Full).unwrap());
        assert_eq!(&disk[..2], [("NVMe".to_string(), "35 °C".to_string()), ("SATA".to_string(), UNREAD.to_string())]);
        // The first chiplet missed this time: the second's reading stays its own.
        let history = leak(vec![with(vec![(0, 40.0), (1, 95.0)], Vec::new()), with(vec![(1, 95.0)], Vec::new())]);
        let cpu = facts(&lane(&scene(history), "cpu", Detail::Full).unwrap());
        assert!(cpu.contains(&("CCD 1".into(), UNREAD.into())));
        assert!(cpu.contains(&("CCD 2".into(), "95 °C".into())));
    }

    /// The CPU's lane shows its fan, of the board's, once seen turning; others as switched, each by its name.
    #[test]
    fn wsl_and_docker_show_only_while_they_run() {
        use crate::reading::{DockerSample, WslSample};
        let (info, theme) = (info(), Theme::new(Skin::Paper, false));
        let prefs = Prefs::resolve(&serde_json::json!({"modules": [{"id": "wsl", "on": true}, {"id": "docker", "on": true}]}), &["wsl".to_string(), "docker".to_string()]);
        let leak = |history: Vec<Sample>| -> &'static [Sample] { Box::leak(history.into_boxed_slice()) };
        let with = |wsl: WslSample, docker: DockerSample| {
            let mut s = sample(None, None, None);
            (s.wsl, s.docker) = (Some(wsl), Some(docker));
            s
        };
        let running = || with(WslSample::Running { distros: Some(vec!["Ubuntu".into()]), cpu: Some(3.0), used: Some(1 << 30), total: Some(1 << 33), gpu: None }, DockerSample::Running { context: "default".into(), containers: Vec::new() });
        let stopped = || with(WslSample::Stopped, DockerSample::Stopped { context: "default".into() });
        let shown = |seen: &'static Seen, history: &'static [Sample]| {
            let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::En, history, seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
            lanes(&scene).into_iter().map(|lane| lane.id).filter(|id| id == "wsl" || id == "docker").collect::<Vec<_>>()
        };
        let seen_of = |history: &'static [Sample]| -> &'static Seen { Box::leak(Box::new(Seen::of(history))) };
        // Not running, not there (nor installed): no lane.
        let history = leak(vec![stopped()]);
        assert!(shown(seen_of(history), history).is_empty());
        let history = leak(vec![with(WslSample::Missing, DockerSample::Unsupported { context: "remote".into(), host: "ssh://x".into() })]);
        assert!(shown(seen_of(history), history).is_empty());
        // Running: their lanes.
        let history = leak(vec![running()]);
        assert_eq!(shown(seen_of(history), history), ["wsl", "docker"]);
        // Stopped since, a panel up meanwhile keeping them as it was laid out; laid out afresh, none.
        let history = leak(vec![running(), stopped()]);
        let up = Box::leak(Box::new(Seen::of(&history[..1]).join(&Seen::of(history), &info)));
        assert_eq!(shown(up, history), ["wsl", "docker"]);
        assert!(shown(seen_of(history), history).is_empty());
        // Come to run while a panel laid out without them is up: theirs are its news.
        let up = Seen::of(&history[1..]);
        let news: Vec<crate::ui::seen::Item> = ["wsl", "docker"].iter().flat_map(|id| up.news(&Seen::of(&history[..1]), id, &info)).collect();
        assert_eq!(news, [crate::ui::seen::Item::Wsl, crate::ui::seen::Item::Docker]);
    }

    #[test]
    fn the_cpu_shows_its_fan() {
        let (info, theme) = (info(), Theme::new(Skin::Paper, false));
        let leak = |history: Vec<Sample>| -> &'static [Sample] { Box::leak(history.into_boxed_slice()) };
        let fans = |read: &[(&str, f32)]| {
            let mut s = sample(None, None, None);
            s.board = Some(crate::reading::BoardSensors { temps: Vec::new(), fans: read.iter().map(|(n, rpm)| (n.to_string(), *rpm)).collect() });
            s
        };
        let cpu_fans = |prefs: &Prefs, history: &'static [Sample]| {
            let scene = Scene { music_apart: false, info: &info, prefs, theme: &theme, lang: Lang::En, history, seen: Box::leak(Box::new(Seen::of(history))), pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
            facts(&lane(&scene, "cpu", Detail::Full).unwrap()).into_iter().filter(|(_, value)| value.ends_with("RPM") || value == UNREAD).collect::<Vec<_>>()
        };
        let prefs = Prefs::default();
        // Its own, turning: as "Fan"; the case's not.
        assert_eq!(cpu_fans(&prefs, leak(vec![fans(&[("cpu_fan", 1200.0), ("system_fan_1", 800.0)])])), vec![("Fan".to_string(), "1200 RPM".to_string())]);
        // A header with none on it: none.
        assert!(cpu_fans(&prefs, leak(vec![fans(&[("cpu_fan", 0.0), ("system_fan_1", 800.0)])])).is_empty());
        // Seen turning, and stopped since: kept, at 0.
        assert_eq!(cpu_fans(&prefs, leak(vec![fans(&[("cpu_fan", 900.0)]), fans(&[("cpu_fan", 0.0)])])), vec![("Fan".to_string(), "0 RPM".to_string())]);
        // A pump switched on beside it: each by its name.
        let mut prefs = Prefs::resolve(&serde_json::Value::Null, &["cpu".to_string()]);
        prefs.set_item("cpu", "fans:system_fan_4_pump", true);
        let history = leak(vec![fans(&[("cpu_fan", 1200.0), ("system_fan_4_pump", 2800.0)])]);
        assert_eq!(cpu_fans(&prefs, history).len(), 2);
        // The group off: none.
        prefs.set_item("cpu", "fans", false);
        assert!(cpu_fans(&prefs, history).is_empty());
    }

    /// What is on the station as its tiles show it: a web address by its site and path, files by the first's name with
    /// how many more, text by its words in a line and its first lines on its face; under each, what it is and when.
    #[test]
    fn the_station_shows_what_it_holds() {
        use crate::station::shelf::{Item, What};
        let folder = crate::station::folder::Folder::at(std::path::PathBuf::from("C:/station"));
        let now = 1_760_800_000_000;
        let item = |what: What| Item { id: 1, what, added_ms: now - 5 * 60_000, pinned: false, copied: false, stays_here: false, own_bytes: 0 };
        let link = Tile::of(&item(What::Link { url: "https://example.com/a/b?c".into() }), &folder, Lang::En, now);
        assert_eq!((link.label.as_str(), &link.face, link.about.as_str()), ("example.com/a/b?c", &Face::Link, "Link · 5 min ago"));
        let files = Tile::of(&item(What::Files { paths: vec!["D:/x/report.pdf".into(), "D:/x/b.txt".into()], kept: Vec::new() }), &folder, Lang::Zh, now);
        assert_eq!((files.label.as_str(), files.about.as_str()), ("report.pdf", "2 个文件 · 5 分钟前"));
        assert!(matches!(files.face, Face::Picture { more: 1, .. }));
        let text = Tile::of(&item(What::Text { text: "one

 two 
three
four
five
six
seven".into() }), &folder, Lang::Zh, now);
        assert_eq!((text.label.as_str(), text.about.as_str()), ("one two three four five six seven", "27 字 · 5 分钟前"));
        assert_eq!(text.face, Face::Text("one
 two 
three
four
five
six
seven".into()));
    }

    /// A picture is asked for at as many pixels as it covers where it is drawn, rounded up to a step: never fewer (a
    /// card's face, 140 DIPs at 150%, was asked for at 128 px and drawn blurred over 210).
    #[test]
    fn a_picture_is_asked_for_at_the_pixels_it_covers() {
        struct At(f32);
        impl Canvas for At {
            fn text(&self, _: &str, _: Font, _: Color, _: f32, _: f32, _: f32, _: Align) {}
            fn markdown(&self, _: &str, _: Font, _: super::super::canvas::Inks, _: f32, _: f32, _: f32, _: f32, _: f32) {}
            fn markdown_tall(&self, _: &str, _: Font, _: f32) -> f32 {
                0.0
            }
            fn measure(&self, _: &str, _: Font) -> f32 {
                0.0
            }
            fn baseline(&self, font: Font) -> (f32, f32) {
                (font.size, 0.0)
            }
            fn ink(&self, _: &str, _: Font) -> (f32, f32) {
                (0.0, 0.0)
            }
            fn clip(&self, _: f32, _: f32, _: f32, _: f32) {}
            fn unclip(&self) {}
            fn fill(&self, _: Color, _: f32, _: f32, _: f32, _: f32) {}
            fn fill_rounded(&self, _: Color, _: f32, _: f32, _: f32, _: f32, _: f32) {}
            fn fill_circle(&self, _: Color, _: Point, _: f32) {}
            fn stroke(&self, _: &[Point], _: Color, _: f32) {}
            fn stroke_dashed(&self, _: &[Point], _: Color, _: f32) {}
            fn fill_shape(&self, _: &[Point], _: Fill) {}
            fn stroke_rounded(&self, _: Fill, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
            fn shadow(&self, _: Color, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
            fn frosted(&self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32, _: &dyn Fn(&dyn Canvas)) {}
            fn arc(&self, _: Point, _: f32, _: f32, _: f32, _: Color, _: f32) {}
            fn icon(&self, _: Icon, _: Point, _: f32, _: Color) {}
            fn picture(&self, _: &super::super::canvas::Picture, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
            fn pixels_per_dip(&self) -> f32 {
                self.0
            }
        }
        for (scale, side) in [(1.0, 56.0), (1.5, 140.0), (2.0, 140.0), (2.25, 333.0), (3.0, 1.0)] {
            let asked = picture_pixels(&At(scale), side);
            assert!((asked as f32) >= side * scale && (asked as f32) < side * scale + PICTURE_STEP as f32 && asked % PICTURE_STEP == 0, "{side} DIPs at {scale}: {asked}");
        }
    }

    /// A text's face: a line longer than the tile is wide goes on in the rows under it, not cut short where it holds
    /// more rows; the last row the tile holds ends in an ellipsis where more is left off, whether of a line or after it.
    #[test]
    fn a_text_face_is_broken_into_rows() {
        let font = Font::new(super::super::canvas::Family::Segoe, 10.0, 400.0);
        // (Six DIPs a character: ten to a row 61 wide.)
        assert_eq!(face_rows(&Nothing, "把起点钉放开 4/4 都解得出", font, 61.0, 61.0, 4), ["把起点钉放开 4/4", "都解得出"]);
        assert_eq!(face_rows(&Nothing, "abc\n\n  def  \nghi", font, 61.0, 61.0, 4), ["abc", "def", "ghi"]);
        assert_eq!(face_rows(&Nothing, "abc\ndef\nghi", font, 61.0, 61.0, 2), ["abc", "def…"]);
        assert_eq!(face_rows(&Nothing, "aaaaaaaaaabbbbbbbbbbcccccccccc", font, 61.0, 61.0, 2), ["aaaaaaaaaa", "bbbbbbbbb…"]);
        // (Short of a pin, the first row only.)
        assert_eq!(face_rows(&Nothing, "aaaaaaaaaabbbbbbbbbb", font, 37.0, 61.0, 3), ["aaaaaa", "aaaabbbbbb", "bbbb"]);
        assert!(face_rows(&Nothing, "abc", font, 61.0, 61.0, 0).is_empty());
    }

    /// A canvas that draws nothing, measuring text as so many DIPs a character: for what is drawn's places alone.
    struct Nothing;

    impl Canvas for Nothing {
        fn text(&self, _: &str, _: Font, _: Color, _: f32, _: f32, _: f32, _: Align) {}
        fn markdown(&self, _: &str, _: Font, _: super::super::canvas::Inks, _: f32, _: f32, _: f32, _: f32, _: f32) {}
        fn markdown_tall(&self, _: &str, _: Font, _: f32) -> f32 {
            0.0
        }
        fn measure(&self, text: &str, font: Font) -> f32 {
            MEASURED.with(|measured| measured.set(measured.get() + 1));
            MEASURED_CHARACTERS.with(|measured| measured.set(measured.get() + text.chars().count()));
            text.chars().count() as f32 * font.size * 0.6
        }
        fn baseline(&self, font: Font) -> (f32, f32) {
            (font.size, font.size * 0.25)
        }
        fn ink(&self, _: &str, font: Font) -> (f32, f32) {
            (font.size * 0.2, font.size)
        }
        fn clip(&self, _: f32, _: f32, _: f32, _: f32) {}
        fn unclip(&self) {}
        fn fill(&self, _: Color, _: f32, _: f32, _: f32, _: f32) {}
        fn fill_rounded(&self, _: Color, _: f32, _: f32, _: f32, _: f32, _: f32) {}
        fn fill_circle(&self, _: Color, _: Point, _: f32) {}
        fn stroke(&self, _: &[Point], _: Color, _: f32) {}
        fn stroke_dashed(&self, _: &[Point], _: Color, _: f32) {}
        fn fill_shape(&self, _: &[Point], _: Fill) {}
        fn stroke_rounded(&self, _: Fill, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
        fn shadow(&self, _: Color, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
        fn frosted(&self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32, _: &dyn Fn(&dyn Canvas)) {}
        fn arc(&self, _: Point, _: f32, _: f32, _: f32, _: Color, _: f32) {}
        fn icon(&self, _: super::super::icons::Icon, _: Point, _: f32, _: Color) {}
        fn picture(&self, _: &super::super::canvas::Picture, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    }

    thread_local! {
        /// How many texts `Nothing` was asked to measure.
        static MEASURED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        /// How many characters they held in all.
        static MEASURED_CHARACTERS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// What `f` gives, and how many texts it had `Nothing` measure, and of how many characters in all.
    fn measuring<T>(f: impl FnOnce() -> T) -> (T, usize, usize) {
        MEASURED.with(|measured| measured.set(0));
        MEASURED_CHARACTERS.with(|measured| measured.set(0));
        let made = f();
        (made, MEASURED.with(|measured| measured.get()), MEASURED_CHARACTERS.with(|measured| measured.get()))
    }

    /// A text cut short to fit, as it was cut one character at a time: the longest start, its spaces at the end left
    /// off, that fits with the ellipsis after it.
    fn fitted_one_by_one(frame: &dyn Canvas, text: &str, font: Font, room: f32) -> String {
        if frame.measure(text, font) <= room {
            return text.to_string();
        }
        let mut kept: Vec<char> = text.chars().collect();
        loop {
            kept.pop();
            let cut = format!("{}…", kept.iter().collect::<String>().trim_end());
            if kept.is_empty() || frame.measure(&cut, font) <= room {
                return cut;
            }
        }
    }

    /// A long text copied is cut short to a row in a few measures, each of little more than the row holds (each
    /// measure a layout of what it measures: one by one, 88 copied took 3.4 s to draw, and the station's pages as long
    /// to show), as it was cut one by one.
    #[test]
    fn a_long_text_is_cut_short_in_a_few_measures() {
        let font = Font::new(super::super::canvas::Family::Segoe, 12.0, 400.0);
        let long = "电磁波 transmission line 的不同结构 Two-wire parallel plate ".repeat(120);
        let (cut, measured, characters) = measuring(|| fitted(&Nothing, &long, font, 260.0).into_owned());
        assert_eq!(cut, fitted_one_by_one(&Nothing, &long, font, 260.0));
        assert!(measured <= 16, "{measured} measures for {} characters", long.chars().count());
        assert!(characters <= 20 * cut.chars().count(), "{characters} characters measured for a row of {}", cut.chars().count());
        for (text, room) in [("", 10.0), ("", -1.0), ("short", 260.0), ("short", 36.0), ("a b  c   d", 30.0), ("a b  c   d", 7.2), ("a b  c   d", 50.0), ("  lead", 25.0), ("字字字字字", 3.0), ("x", 0.0), ("abcdefgh  i", 70.0)] {
            assert_eq!(fitted(&Nothing, text, font, room), fitted_one_by_one(&Nothing, text, font, room), "{text:?} in {room}");
        }
    }

    /// A long text in rows, at most so many: laid out as far as those rows, not row by row to its end.
    #[test]
    fn rows_of_a_long_text_stop_at_the_most() {
        let font = Font::new(super::super::canvas::Family::Segoe, 12.0, 400.0);
        let long = "a note with a great many words in it ".repeat(200);
        let (rows, measured, _) = measuring(|| rows_of(&Nothing, &long, font, 120.0, 2));
        assert_eq!(rows.len(), 2);
        assert!(rows[1].ends_with('…'), "{rows:?}");
        assert!(measured < 200, "{measured} measures for {} characters", long.chars().count());
    }

    /// A canvas that draws nothing, keeping the box all it drew reaches (a shadow as far as its blur shows, what is
    /// clipped no further than its clip).
    #[derive(Default)]
    struct Reach {
        /// The clips begun and not ended.
        clips: std::cell::RefCell<Vec<(f32, f32, f32, f32)>>,
        reach: std::cell::Cell<Option<(f32, f32, f32, f32)>>,
        /// What it was given to write.
        said: std::cell::RefCell<Vec<String>>,
    }

    impl Reach {
        /// The box (left, top, right, bottom) drawn over.
        fn drew(&self, (left, top, right, bottom): (f32, f32, f32, f32)) {
            let (left, top, right, bottom) = self.clips.borrow().iter().fold((left, top, right, bottom), |(l, t, r, b), &(cx, cy, cw, ch)| (l.max(cx), t.max(cy), r.min(cx + cw), b.min(cy + ch)));
            if right <= left || bottom <= top {
                return;
            }
            self.reach.set(Some(match self.reach.get() {
                Some((l, t, r, b)) => (l.min(left), t.min(top), r.max(right), b.max(bottom)),
                None => (left, top, right, bottom),
            }));
        }

        fn drew_box(&self, x: f32, y: f32, w: f32, h: f32) {
            self.drew((x, y, x + w, y + h));
        }

        fn drew_points(&self, points: &[Point], width: f32) {
            let fold = |pick: fn(&Point) -> f32, f: fn(f32, f32) -> f32, from: f32| points.iter().map(pick).fold(from, f);
            self.drew((fold(|p| p.x, f32::min, f32::MAX) - width / 2.0, fold(|p| p.y, f32::min, f32::MAX) - width / 2.0, fold(|p| p.x, f32::max, f32::MIN) + width / 2.0, fold(|p| p.y, f32::max, f32::MIN) + width / 2.0));
        }
    }

    impl Canvas for Reach {
        fn text(&self, text: &str, font: Font, _: Color, x: f32, y: f32, width: f32, align: Align) {
            self.said.borrow_mut().push(text.to_string());
            let wide = Nothing.measure(text, font).min(width);
            let from = match align {
                Align::Start => x,
                Align::End => x + width - wide,
            };
            if !text.trim().is_empty() {
                self.drew_box(from, y, wide, font.size * 1.3);
            }
        }
        fn markdown(&self, text: &str, _: Font, _: super::super::canvas::Inks, x: f32, y: f32, w: f32, h: f32, _: f32) {
            if !text.trim().is_empty() {
                self.drew_box(x, y, w, h);
            }
        }
        fn markdown_tall(&self, _: &str, _: Font, _: f32) -> f32 {
            0.0
        }
        fn measure(&self, text: &str, font: Font) -> f32 {
            Nothing.measure(text, font)
        }
        fn baseline(&self, font: Font) -> (f32, f32) {
            Nothing.baseline(font)
        }
        fn ink(&self, text: &str, font: Font) -> (f32, f32) {
            Nothing.ink(text, font)
        }
        fn clip(&self, x: f32, y: f32, w: f32, h: f32) {
            self.clips.borrow_mut().push((x, y, w, h));
        }
        fn unclip(&self) {
            self.clips.borrow_mut().pop();
        }
        fn fill(&self, _: Color, x: f32, y: f32, w: f32, h: f32) {
            self.drew_box(x, y, w, h);
        }
        fn fill_rounded(&self, _: Color, x: f32, y: f32, w: f32, h: f32, _: f32) {
            self.drew_box(x, y, w, h);
        }
        fn fill_circle(&self, _: Color, centre: Point, radius: f32) {
            self.drew_box(centre.x - radius, centre.y - radius, 2.0 * radius, 2.0 * radius);
        }
        fn stroke(&self, points: &[Point], _: Color, width: f32) {
            self.drew_points(points, width);
        }
        fn stroke_dashed(&self, points: &[Point], _: Color, width: f32) {
            self.drew_points(points, width);
        }
        fn fill_shape(&self, points: &[Point], _: Fill) {
            self.drew_points(points, 0.0);
        }
        fn stroke_rounded(&self, _: Fill, x: f32, y: f32, w: f32, h: f32, _: f32, line: f32) {
            self.drew_box(x - line / 2.0, y - line / 2.0, w + line, h + line);
        }
        fn shadow(&self, _: Color, x: f32, y: f32, w: f32, h: f32, _: f32, blur: f32, drop: f32) {
            self.drew_box(x - 2.0 * blur, y - 2.0 * blur + drop, w + 4.0 * blur, h + 4.0 * blur);
        }
        fn frosted(&self, x: f32, y: f32, w: f32, h: f32, _: f32, _: f32, _: &dyn Fn(&dyn Canvas)) {
            self.drew_box(x, y, w, h);
        }
        fn arc(&self, centre: Point, radius: f32, _: f32, _: f32, _: Color, width: f32) {
            let r = radius + width / 2.0;
            self.drew_box(centre.x - r, centre.y - r, 2.0 * r, 2.0 * r);
        }
        fn icon(&self, _: super::super::icons::Icon, centre: Point, size: f32, _: Color) {
            self.drew_box(centre.x - size / 2.0, centre.y - size / 2.0, size, size);
        }
        fn picture(&self, _: &super::super::canvas::Picture, x: f32, y: f32, w: f32, h: f32, _: f32, _: f32) {
            self.drew_box(x, y, w, h);
        }
    }

    /// The panel's settings' button is lit (in the accent, as the pin pinned is) while the settings asked from it are open, the
    /// pointer away from it: they pour back into a button that shows where they came from. Else it is drawn plain.
    #[test]
    fn the_settings_button_is_lit_while_the_settings_asked_from_it_are_open() {
        struct Icons(std::cell::RefCell<Vec<Color>>, std::cell::Cell<usize>);
        impl Canvas for Icons {
            fn text(&self, _: &str, _: Font, _: Color, _: f32, _: f32, _: f32, _: Align) {}
            fn markdown(&self, _: &str, _: Font, _: super::super::canvas::Inks, _: f32, _: f32, _: f32, _: f32, _: f32) {}
        fn markdown_tall(&self, _: &str, _: Font, _: f32) -> f32 {
            0.0
        }
            fn measure(&self, text: &str, font: Font) -> f32 {
                text.chars().count() as f32 * font.size * 0.6
            }
            fn baseline(&self, font: Font) -> (f32, f32) {
                (font.size, font.size * 0.25)
            }
            fn ink(&self, _: &str, font: Font) -> (f32, f32) {
                (font.size * 0.2, font.size)
            }
            fn clip(&self, _: f32, _: f32, _: f32, _: f32) {}
            fn unclip(&self) {}
            fn fill(&self, _: Color, _: f32, _: f32, _: f32, _: f32) {}
            fn fill_rounded(&self, _: Color, _: f32, _: f32, _: f32, _: f32, _: f32) {
                self.1.set(self.1.get() + 1);
            }
            fn fill_circle(&self, _: Color, _: Point, _: f32) {}
            fn stroke(&self, _: &[Point], _: Color, _: f32) {}
            fn stroke_dashed(&self, _: &[Point], _: Color, _: f32) {}
            fn fill_shape(&self, _: &[Point], _: Fill) {}
            fn stroke_rounded(&self, _: Fill, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
            fn shadow(&self, _: Color, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
            fn frosted(&self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32, _: &dyn Fn(&dyn Canvas)) {}
            fn arc(&self, _: Point, _: f32, _: f32, _: f32, _: Color, _: f32) {}
            fn icon(&self, _: super::super::icons::Icon, _: Point, _: f32, color: Color) {
                self.0.borrow_mut().push(color);
            }
            fn picture(&self, _: &super::super::canvas::Picture, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
        }
        let theme = Theme::new(Skin::Fluent, true);
        let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["time".to_string()]));
        let history: &'static [Sample] = Box::leak(Box::new([sample(None, None, None)]));
        let seen = Seen::of(history);
        let drawn = |settings_open: bool| {
            let settings_lit = settings_open.then_some(1.0);
            let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::Zh, history, seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit };
            let icons = Icons(Default::default(), Default::default());
            panel_button(&icons, &scene, Hit::Settings, 0.0, 0.0);
            (icons.0.into_inner(), icons.1.get())
        };
        let accent = super::super::theme::accent(true);
        assert_eq!(drawn(true), (vec![accent], 0));
        assert_eq!(drawn(false), (vec![theme.text2], 0));
    }

    /// A line sung drawn gliding into its place says so to the window that drew it (drawn again at the next frame);
    /// drawn at rest, or by a window that shows none, it says nothing: only what shows the line is drawn as often as
    /// it glides (every window was, while any line glided anywhere).
    #[test]
    fn only_a_gliding_line_drawn_asks_for_frames() {
        let theme = Theme::new(Skin::Fluent, false);
        let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["time".to_string()]));
        let history: &'static [Sample] = Box::leak(Box::new([sample(None, None, None)]));
        let seen = Seen::of(history);
        let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::En, history, seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
        let drawn = |done: f32| {
            let glide = Glide { done, was: "the line before".into() };
            let lane = Lane { id: "time".into(), blocks: vec![Block::Lyric { line: "the line sung".into(), under: String::new(), said: false, glide }], ink: theme.ink("time") };
            gliding_drawn();
            paint_lane(&Nothing, &scene, &lane, Rect { x: 0.0, y: 0.0, w: 300.0, h: 100.0 }, 0.0, Pass::Content, &mut Vec::new());
            gliding_drawn()
        };
        assert!(drawn(0.4), "gliding");
        assert!(!drawn(1.0), "at rest");
        // (Nothing drawn since: nothing glides.)
        assert!(!gliding_drawn());
    }

    /// A thing on the station whose name does not fit says its whole name as the pointer comes over it, in every view
    /// (its name as the station holds it now, none kept of what was drawn: renamed, it says its new name, gone, none);
    /// one whose name fits says nothing, and a press on either still finds it.
    #[test]
    fn a_shortened_name_is_said_whole() {
        let _pretending = crate::pretending();
        let theme = Theme::new(Skin::Fluent, false);
        let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["station".to_string()]));
        let history: &'static [Sample] = Box::leak(Box::new([sample(None, None, None)]));
        let seen = Seen::of(history);
        let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::En, history, seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
        let long = "Bliss_v2.1.1_(Chocapic13_Shaders_edit).zip";
        let saved = |id: u64, name: &str| crate::station::shelf::Item {
            id,
            what: crate::station::shelf::What::Saved { file: format!("{id}.zip"), name: name.into(), words: false },
            added_ms: 0,
            pinned: false,
            copied: false,
            stays_here: false,
            own_bytes: 0,
        };
        let held = vec![saved(1, long), saved(2, "a.md")];
        crate::station::run::pretend(held.clone(), ShelfView::Cards);
        let folder = crate::station::folder::Folder::own();
        let tiles: Vec<Tile> = held.iter().map(|item| Tile::of(item, &folder, Lang::En, 0)).collect();
        for shows in [Shows::View(ShelfView::Cards), Shows::View(ShelfView::List), Shows::View(ShelfView::Large), Shows::Row] {
            let mut hits = Vec::new();
            paint_shelf(&Nothing, &scene, theme.ink("station"), (0.0, 0.0, 324.0), &tiles, shows, STATION_TALL, None, false, &mut hits);
            assert_eq!(named(&hits, Hit::Shelf(1), Lang::En).as_deref(), Some(long), "{shows:?}");
            assert_eq!(named(&hits, Hit::Shelf(2), Lang::En), None, "{shows:?}");
            let (x, y, w, h, _) = *hits.iter().find(|hit| hit.4 == Hit::Shelf(1)).unwrap();
            assert_eq!(hits.iter().find(|(hx, hy, hw, hh, _)| x + w / 2.0 >= *hx && x + w / 2.0 < hx + hw && y + h / 2.0 >= *hy && y + h / 2.0 < hy + hh).map(|hit| hit.4), Some(Hit::Shelf(1)), "{shows:?}");
            let renamed = format!("{long}.renamed");
            crate::station::run::pretend(vec![saved(1, &renamed), saved(2, "a.md")], ShelfView::Cards);
            assert_eq!(named(&hits, Hit::Shelf(1), Lang::En), Some(renamed), "{shows:?}");
            crate::station::run::pretend(vec![saved(2, "a.md")], ShelfView::Cards);
            assert_eq!(named(&hits, Hit::Shelf(1), Lang::En), None, "{shows:?}");
            crate::station::run::pretend(held.clone(), ShelfView::Cards);
        }
    }

    /// Files come together on Recent files' page: one row, "N files · folder", in the folder (and how large together),
    /// with the newest's time and picture and how many more, as a list and as cards. A press on it finds them as the
    /// page's filter shows them. A file come alone is its own row.
    #[test]
    fn files_come_together_are_one_row() {
        use crate::station::recent::{Came, Entry, Filter, Key, Kind};
        let now = 1_760_800_000_000;
        let came = |file: u128, path: &str, ago_s: u64, kind: Kind| Came { key: Key { drive: 'D', file }, path: std::path::PathBuf::from(path), came_ms: now - ago_s * 1000, bytes: 1_000_000, kind, downloaded: false, site: None };
        let mut files: Vec<Came> = (1..=6).map(|file| came(file, &format!(r"D:\Phone\DCIM\{file}.jpg"), 600 - file as u64, Kind::Picture)).collect();
        files.push(came(9, r"D:\Documents\a.pdf", 60, Kind::Document));
        files.sort_by_key(|came| std::cmp::Reverse(came.came_ms));
        let rows = Lately::rows(&files, Filter::All, Lang::En, now);
        assert_eq!(rows.iter().map(|row| row.label.as_str()).collect::<Vec<_>>(), ["a.pdf", "6 files · DCIM"]);
        let together = &rows[1];
        assert_eq!((together.entry, together.more, together.path.clone()), (Entry::Burst(Filter::All, Key { drive: 'D', file: 6 }), 5, std::path::PathBuf::from(r"D:\Phone\DCIM\6.jpg")));
        assert_eq!(together.about, format!(r"Phone\DCIM · {}", text::bytes(6_000_000.0)));
        assert_eq!(together.when, text::since(now - 594_000, now, Lang::En));
        assert_eq!(Lately::rows(&files, Filter::All, Lang::Zh, now)[1].label, "6 个文件 · DCIM");
        assert_eq!(rows[0].entry, Entry::File(Key { drive: 'D', file: 9 }));
        let theme = Theme::new(Skin::Fluent, false);
        let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["station".to_string()]));
        let history: &'static [Sample] = Box::leak(Box::new([sample(None, None, None)]));
        let seen = Seen::of(history);
        let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::En, history, seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
        for view in [ShelfView::List, ShelfView::Cards] {
            let mut hits = Vec::new();
            paint_recent(&Nothing, &scene, theme.ink("station"), (0.0, 0.0, 324.0), &rows, Filter::All, view, STATION_TALL, false, &mut hits);
            let found: Vec<Hit> = hits.iter().map(|hit| hit.4).filter(|hit| matches!(hit, Hit::Recent(_))).collect();
            assert_eq!(found, [Hit::Recent(rows[0].entry), Hit::Recent(together.entry)], "{view:?}");
        }
        assert_eq!(rows[1].tile().face, Face::Picture { path: std::path::PathBuf::from(r"D:\Phone\DCIM\6.jpg"), more: 5 });
    }

    /// On a station, a list of what was copied, Recent files, long enough to scroll: a press on a thing finds the
    /// thing, not the list it is on.
    #[test]
    fn a_press_on_a_list_finds_its_thing() {
        let theme = Theme::new(Skin::Paper, false);
        let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["station".to_string()]));
        let history: &'static [Sample] = Box::leak(Box::new([sample(None, None, None)]));
        let seen = Seen::of(history);
        let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::En, history, seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
        let ink = theme.ink("station");
        let tile = |id: u64| Tile { id, label: format!("{id}"), about: String::new(), face: Face::Link, pinned: false };
        let first = |hits: &[HitBox], x: f32, y: f32| hits.iter().find(|(hx, hy, w, h, _)| x >= *hx && x < hx + w && y >= *hy && y < hy + h).map(|hit| hit.4);
        let many: Vec<Tile> = (1..=40).map(tile).collect();
        for shows in [Shows::View(ShelfView::Cards), Shows::View(ShelfView::List), Shows::View(ShelfView::Large)] {
            let mut hits = Vec::new();
            paint_shelf(&Nothing, &scene, ink, (0.0, 0.0, 324.0), &many, shows, STATION_TALL, None, false, &mut hits);
            assert_eq!(first(&hits, 10.0, 10.0), Some(Hit::Shelf(1)), "{shows:?}");
        }
        let mut hits = Vec::new();
        paint_copies(&Nothing, &scene, ink, (0.0, 0.0, 324.0), &many, Default::default(), ShelfView::List, STATION_TALL, &mut hits);
        let row = hits.iter().find(|hit| hit.4 == Hit::Copied(1)).copied().unwrap();
        assert_eq!(first(&hits, row.0 + 5.0, row.1 + 5.0), Some(Hit::Copied(1)));
        let rows: Vec<Lately> = (1..=40).map(|file| Lately { entry: crate::station::recent::Entry::File(crate::station::recent::Key { drive: 'C', file }), day: "Today", label: String::new(), about: String::new(), when: String::new(), path: std::path::PathBuf::new(), more: 0 }).collect();
        let mut hits = Vec::new();
        paint_recent(&Nothing, &scene, ink, (0.0, 0.0, 324.0), &rows, Default::default(), ShelfView::List, STATION_TALL, false, &mut hits);
        let row = hits.iter().find(|hit| hit.4 == Hit::Recent(crate::station::recent::Entry::File(crate::station::recent::Key { drive: 'C', file: 1 }))).copied().unwrap();
        assert_eq!(first(&hits, row.0 + 5.0, row.1 + 5.0), Some(Hit::Recent(crate::station::recent::Entry::File(crate::station::recent::Key { drive: 'C', file: 1 }))));
    }

    /// A list scrolled part way, each of its things pressed only where it shows: none reaches past the list's room
    /// (onto what lies under it), on the station's pages, the notes' views and quick launch.
    #[test]
    fn a_thing_half_shown_is_pressed_only_where_it_shows() {
        let theme = Theme::new(Skin::Fluent, false);
        let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["station".to_string()]));
        let history: &'static [Sample] = Box::leak(Box::new([sample(None, None, None)]));
        let seen = Seen::of(history);
        let mut scrolls = Scrolls::default();
        for list in [List::Shelf, List::Copies, List::Recent, List::Notes, List::Apps] {
            scrolls.turn(list, 17.0, 1000.0);
        }
        for _ in 0..200 {
            scrolls.ease(1.0, 0.1);
        }
        let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::En, history, seen: &seen, pen_ms: 0.0, scrolls, tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
        let ink = theme.ink("station");
        let tile = |id: u64| Tile { id, label: format!("{id}"), about: String::new(), face: Face::Link, pinned: false };
        let many: Vec<Tile> = (1..=40).map(tile).collect();
        let within = |hits: &[HitBox], top: f32, tall: f32, what: &str| {
            for hit in hits.iter().filter(|hit| matches!(hit.4, Hit::Shelf(_) | Hit::Copied(_) | Hit::Recent(_) | Hit::Note(_) | Hit::App(_) | Hit::AddApp)) {
                assert!(hit.1 >= top - 0.01 && hit.1 + hit.3 <= top + tall + 0.01, "{what}: {hit:?} past {top}..{}", top + tall);
            }
        };
        let (top, width) = (100.0, 320.0);
        for shows in [Shows::View(ShelfView::Cards), Shows::View(ShelfView::List), Shows::View(ShelfView::Large)] {
            let mut hits = Vec::new();
            paint_shelf(&Nothing, &scene, ink, (0.0, top, width), &many, shows, STATION_TALL, None, false, &mut hits);
            within(&hits, top, STATION_TALL, &format!("{shows:?}"));
        }
        let mut hits = Vec::new();
        paint_copies(&Nothing, &scene, ink, (0.0, top, width), &many, Default::default(), ShelfView::List, STATION_TALL, &mut hits);
        within(&hits, top, STATION_TALL, "copies");
        let rows: Vec<Lately> = (1..=40).map(|file| Lately { entry: crate::station::recent::Entry::File(crate::station::recent::Key { drive: 'C', file }), day: "Today", label: String::new(), about: String::new(), when: String::new(), path: std::path::PathBuf::new(), more: 0 }).collect();
        let mut hits = Vec::new();
        paint_recent(&Nothing, &scene, ink, (0.0, top, width), &rows, Default::default(), ShelfView::List, STATION_TALL, false, &mut hits);
        within(&hits, top + CHIP_H + 6.0, STATION_TALL - CHIP_H - 6.0, "recent");
        let notes: Vec<NoteCard> = (0..13).map(|at| NoteCard { number: at, title: format!("{at}"), lines: vec!["a".into()], when: String::new(), colour: Default::default(), held: super::Held::Not }).collect();
        for view in [crate::settings::NotesView::Deck, crate::settings::NotesView::Cards, crate::settings::NotesView::List] {
            let mut hits = Vec::new();
            paint_notes(&Nothing, &scene, (0.0, top, width), &notes, view, NOTES_TALL, &mut hits);
            within(&hits, top, NOTES_TALL, &format!("{view:?}"));
        }
        let apps: Vec<(String, String)> = (0..14).map(|at| (format!("{at}"), format!("C:/{at}.exe"))).collect();
        let mut hits = Vec::new();
        paint_apps(&Nothing, &scene, (0.0, top, width), &apps, APP_ROWS, true, false, &mut hits);
        within(&hits, top, APP_ROWS as f32 * APP_CELL_H + (APP_ROWS - 1) as f32 * APP_GAP, "apps");
    }

    /// The station's pages: side by side where they fit; where they do not (a narrow widget), one with its menu of
    /// them all, not each cut to a letter.
    #[test]
    fn the_station_s_pages_are_one_menu_where_they_do_not_fit() {
        for skin in [Skin::Glass, Skin::Fluent, Skin::Paper] {
            let theme = Theme::new(skin, false);
            let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["station".to_string()]));
            let history: &'static [Sample] = Box::leak(Box::new([sample(None, None, None)]));
            let seen = Seen::of(history);
            let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::Zh, history, seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Copies(Default::default()), hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
            let (mut wide, mut narrow) = (Vec::new(), Vec::new());
            paint_tabs(&Nothing, &scene, (0.0, 0.0, COLUMN_WIDTH - 2.0 * theme.pad_x), Tab::Copies(Default::default()), 3, true, true, ShelfView::List, &mut wide);
            // (Measured here as so many DIPs a character, narrower than Chinese is: narrower than the least, for that.)
            paint_tabs(&Nothing, &scene, (0.0, 0.0, 130.0), Tab::Copies(Default::default()), 3, true, true, ShelfView::List, &mut narrow);
            assert_eq!(wide.iter().filter(|hit| matches!(hit.4, Hit::Tab(_))).count(), 3, "{skin:?}");
            assert!(narrow.iter().any(|hit| hit.4 == Hit::StationPages) && !narrow.iter().any(|hit| matches!(hit.4, Hit::Tab(_))), "{skin:?}");
            assert!(narrow.iter().any(|hit| hit.4 == Hit::StationView), "{skin:?}");
        }
    }

    /// A workbench laid out again for a size (a widget sized): as large as that, narrower and shorter than designed
    /// too; its station's things flowing across the width it has, its list shown in the height left, down to its least.
    #[test]
    fn a_workbench_is_laid_out_again_for_its_size() {
        let theme = Theme::new(Skin::Glass, false);
        let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["station".to_string()]));
        let history: &'static [Sample] = Box::leak(Box::new([sample(None, None, None)]));
        let seen = Seen::of(history);
        let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::Zh, history, seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
        let heights: Vec<f32> = heights(&scene).iter().map(|(_, h)| h[0]).collect();
        let designed = Layout::with_columns(heights.clone(), 1, &theme).giving(gives(&scene));
        assert!(designed.least_height() < designed.height());
        for size in [(designed.width() - 100.0, designed.height() - 80.0), (designed.width() + 150.0, designed.height() + 120.0)] {
            let layout = designed.clone().fitted(size);
            assert!((layout.width() - size.0).abs() < 0.01 && (layout.height() - size.1).abs() < 0.01, "{size:?}");
            assert!((layout.lanes()[0].w - size.0 + 2.0 * 0.0).abs() < 60.0);
        }
        let narrow = designed.clone().fitted((240.0, designed.height()));
        let wide = designed.clone().fitted((designed.width() + 150.0, designed.height()));
        assert!(narrow.flow(&theme).0 < wide.flow(&theme).0, "{:?} {:?}", narrow.flow(&theme), wide.flow(&theme));
    }

    /// A workbench made shorter or taller than designed: only its lanes with a list at their foot (the station's, the
    /// notes') are drawn shorter or taller, each by no more than it gives; the time's and quick launch's are as
    /// designed (made shorter evenly, the time's music was drawn over the station's tabs). Its least is each list given
    /// way, and lanes never overlap.
    #[test]
    fn a_workbench_made_shorter_takes_it_from_its_lists() {
        let theme = Theme::new(Skin::Paper, false);
        let heights = vec![200.0, 300.0, 250.0, 120.0];
        let gives = vec![0.0, 150.0, 100.0, 0.0];
        let designed = Layout::with_columns(heights.clone(), 1, &theme).giving(gives.clone());
        assert!((designed.height() - designed.least_height() - 250.0).abs() < 0.01, "least {}", designed.least_height());
        for change in [-200.0, -240.0, -250.0, 0.0, 90.0] {
            let layout = designed.clone().fitted((designed.width(), designed.height() + change));
            let lanes = layout.lanes();
            assert_eq!(lanes[0].h, 200.0, "{change}: the time's lane");
            assert_eq!(lanes[3].h, 120.0, "{change}: quick launch's lane");
            for at in [1, 2] {
                assert!(lanes[at].h >= heights[at] - gives[at] - 0.01, "{change}: lane {at} past its least ({})", lanes[at].h);
            }
            assert!((lanes.iter().map(|lane| lane.h).sum::<f32>() - heights.iter().sum::<f32>() - change).abs() < 0.01, "{change}");
            for pair in lanes.windows(2) {
                assert!(pair[0].y + pair[0].h <= pair[1].y + 0.01, "{change}: lanes overlap");
            }
        }
        // Made shorter by more than the notes give in an even share: they give all of theirs, the station the rest.
        let lanes = designed.clone().fitted((designed.width(), designed.height() - 240.0)).lanes();
        assert!((lanes[2].h - 150.0).abs() < 0.01 && (lanes[1].h - 160.0).abs() < 0.01, "{} {}", lanes[1].h, lanes[2].h);
        // Lanes not told what they give (the panel's) take an even share, as before.
        let even = Layout::with_columns(heights.clone(), 1, &theme);
        let lanes = even.clone().fitted((even.width(), even.height() + 40.0)).lanes();
        assert!(lanes.iter().zip(&heights).all(|(lane, h)| (lane.h - h - 10.0).abs() < 0.01));
    }

    /// A grid's pages: as many as go on one, else each but the last a cell short (that cell the way to the next).
    #[test]
    fn a_grid_s_things_go_on_pages() {
        assert_eq!(pages_of(9, 9), [(0, 9)]);
        assert_eq!(pages_of(13, 9), [(0, 8), (8, 5)]);
        assert_eq!(pages_of(10, 4), [(0, 3), (3, 3), (6, 4)]);
        assert_eq!(pages_of(3, 1), [(0, 1), (1, 1), (2, 1)]);
        assert_eq!(pages_of(0, 4), [(0, 0)]);
    }

    /// Pages turned by the wheel come to a page: a notch turns one, a touchpad's small turns go on to the next once
    /// they come to half a page, and never past the last.
    #[test]
    fn pages_are_turned_to_a_page() {
        let mut scrolls = Scrolls::default();
        scrolls.turn_pages(List::Apps, PAGE, PAGE, 2.0 * PAGE);
        assert_eq!(scrolls.to[List::Apps as usize], PAGE);
        scrolls.turn_pages(List::Apps, PAGE * 0.3, PAGE, 2.0 * PAGE);
        assert_eq!(scrolls.to[List::Apps as usize], PAGE);
        scrolls.turn_pages(List::Apps, PAGE * 0.3, PAGE, 2.0 * PAGE);
        assert_eq!(scrolls.to[List::Apps as usize], 2.0 * PAGE);
        scrolls.turn_pages(List::Apps, 5.0 * PAGE, PAGE, 2.0 * PAGE);
        assert_eq!(scrolls.to[List::Apps as usize], 2.0 * PAGE);
        scrolls.turn_pages(List::Apps, -PAGE, PAGE, 2.0 * PAGE);
        assert_eq!(scrolls.to[List::Apps as usize], PAGE);
    }

    /// Pages swiped: held where the hand is, then let go, on to the page nearest; flicked, the next that way (from
    /// a little way along, either way); never past the first or the last.
    #[test]
    fn pages_swiped_come_to_a_page() {
        let mut scrolls = Scrolls::default();
        let most = 3.0 * PAGE;
        scrolls.hold_pages(List::Notes, 0.4 * PAGE, most);
        assert_eq!(scrolls.at(List::Notes), 0.4 * PAGE);
        scrolls.let_go_pages(List::Notes, 0.0, PAGE, most);
        assert_eq!(scrolls.to[List::Notes as usize], 0.0);
        scrolls.hold_pages(List::Notes, 0.6 * PAGE, most);
        scrolls.let_go_pages(List::Notes, 0.0, PAGE, most);
        assert_eq!(scrolls.to[List::Notes as usize], PAGE);
        scrolls.hold_pages(List::Notes, 1.1 * PAGE, most);
        scrolls.let_go_pages(List::Notes, 1.0, PAGE, most);
        assert_eq!(scrolls.to[List::Notes as usize], 2.0 * PAGE);
        scrolls.hold_pages(List::Notes, 1.9 * PAGE, most);
        scrolls.let_go_pages(List::Notes, -1.0, PAGE, most);
        assert_eq!(scrolls.to[List::Notes as usize], PAGE);
        scrolls.hold_pages(List::Notes, 9.0 * PAGE, most);
        assert_eq!(scrolls.at(List::Notes), most);
        scrolls.let_go_pages(List::Notes, 1.0, PAGE, most);
        assert_eq!(scrolls.to[List::Notes as usize], most);
        scrolls.hold_pages(List::Notes, -PAGE, most);
        scrolls.let_go_pages(List::Notes, -1.0, PAGE, most);
        assert_eq!(scrolls.to[List::Notes as usize], 0.0);
    }

    /// A press on a note, or on a program kept at hand, names it, not its place: the list changed before the press is
    /// let go (a note written in elsewhere, a program moved), it is still the one pressed.
    #[test]
    fn a_press_names_what_it_was_on_as_the_list_changes() {
        use crate::notes::{Colour, Note};
        let _pretending = crate::pretending();
        let note = |name: &str| Note { name: name.into(), text: name.into(), colour: Colour::Blue, stamp: crate::notes::store::Stamp { bytes: 0, written_ms: 0, digest: 0 } };
        crate::notes::pretend(vec![note("pressed-a.md"), note("pressed-b.md")]);
        let card = note_card(&crate::notes::shown()[0], Lang::En, 0, 1);
        crate::notes::pretend(vec![note("pressed-b.md"), note("pressed-a.md")]);
        assert_eq!(crate::notes::named(card.number).as_deref(), Some("pressed-a.md"));
        crate::notes::pretend(vec![note("pressed-b.md")]);
        assert_eq!(crate::notes::named(card.number), None, "gone: nothing opened");
        let app = |target: &str| crate::station::apps::App { target: target.into(), name: target.into() };
        let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["apps".to_string()]));
        let history: &'static [Sample] = Box::leak(Box::new([sample(None, None, None)]));
        let seen = Seen::of(history);
        let theme = Theme::new(Skin::Glass, false);
        let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::En, history, seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
        let apps: Vec<(String, String)> = ["C:/pressed-a.exe", "C:/pressed-b.exe"].iter().map(|target| (target.to_string(), target.to_string())).collect();
        let mut hits = Vec::new();
        paint_apps(&Nothing, &scene, (0.0, 0.0, 300.0), &apps, APP_ROWS, true, false, &mut hits);
        let first = hits.iter().find_map(|hit| if let Hit::App(number) = hit.4 { Some(number) } else { None }).unwrap();
        crate::station::apps::pretend(vec![app("C:/pressed-b.exe"), app("C:/pressed-a.exe")]);
        assert_eq!(crate::station::apps::numbered(first).map(|app| app.target).as_deref(), Some("C:/pressed-a.exe"));
    }

    /// The notes turned through by pages: the note shown stays shown as their order changes (written in, it comes
    /// first), wherever it goes; turned by hand, the note turned to is; one opened beside them is turned to.
    #[test]
    fn the_note_shown_stays_as_their_order_changes() {
        use crate::notes::{Colour, Note};
        let note = |name: &str| Note { name: name.into(), text: name.into(), colour: Colour::Blue, stamp: crate::notes::store::Stamp { bytes: 0, written_ms: 0, digest: 0 } };
        let order = |names: &[&str]| names.iter().map(|name| note(name)).collect::<Vec<Note>>();
        let (mut scrolls, mut shown) = (Scrolls::default(), NoteShown::default());
        let shows = |scrolls: &Scrolls| (scrolls.at(List::Notes) / PAGE, scrolls.to(List::Notes) / PAGE);
        let abc = order(&["a", "b", "c"]);
        keep_note(&mut scrolls, &mut shown, &abc, None);
        // Turned by hand to the third.
        scrolls.turn_pages(List::Notes, 2.0 * PAGE, PAGE, 2.0 * PAGE);
        keep_note(&mut scrolls, &mut shown, &abc, None);
        assert_eq!(shows(&scrolls).1, 2.0);
        // It written in: first now, and shown there at once.
        scrolls.ease(10.0, 0.1);
        keep_note(&mut scrolls, &mut shown, &order(&["c", "a", "b"]), None);
        assert_eq!(shows(&scrolls), (0.0, 0.0));
        // Another written in elsewhere: it stays shown, second now.
        keep_note(&mut scrolls, &mut shown, &order(&["b", "c", "a"]), None);
        assert_eq!(shows(&scrolls), (1.0, 1.0));
        // One opened beside: turned to (sliding there).
        keep_note(&mut scrolls, &mut shown, &order(&["b", "c", "a"]), Some("a"));
        assert_eq!(shows(&scrolls).1, 2.0);
        // A new one written beside, first kept: turned to.
        keep_note(&mut scrolls, &mut shown, &order(&["new", "b", "c", "a"]), Some("new"));
        assert_eq!(shows(&scrolls).1, 0.0);
        // The one shown taken away: the page it was on.
        scrolls.ease(10.0, 0.1);
        keep_note(&mut scrolls, &mut shown, &order(&["b", "c", "a"]), None);
        assert_eq!(shows(&scrolls).1, 0.0);
    }

    /// A lane's pages as they are turned: the one turned to whole at rest, the lane; all along, the two turned
    /// between each whole within the lane and apart (the gap between them and round them half way); the one turned from smaller as
    /// it goes, the one turned to larger as it comes.
    #[test]
    fn pages_turned_stay_whole_within_the_lane() {
        for wide in [44.0, 70.0, 150.0, 330.0] {
            assert_eq!(page_place(wide, 0.0), (1.0, 0.0, 1.0));
            let mut last: Option<(f32, f32)> = None;
            for step in 0..=40 {
                let turned = step as f32 / 40.0;
                let (from, to) = (page_place(wide, -turned), page_place(wide, 1.0 - turned));
                for (scale, left, _) in [from, to] {
                    assert!(scale <= 1.0 + 1e-6 && left >= -1e-3 && left + scale * wide <= wide + 1e-3, "{wide} at {turned}: {scale} from {left}");
                }
                // (One gone to nothing is nowhere.)
                if from.0 > 0.0 && to.0 > 0.0 {
                    assert!(from.1 + from.0 * wide <= to.1 + 1e-3, "{wide} at {turned}: they meet");
                }
                if let Some((was_from, was_to)) = last {
                    assert!(from.0 <= was_from + 1e-6 && to.0 >= was_to - 1e-6, "{wide} at {turned}");
                }
                last = Some((from.0, to.0));
            }
            let ((a, a_left, _), (b, b_left, _)) = (page_place(wide, -0.5), page_place(wide, 0.5));
            assert!((a - b).abs() < 1e-6 && (b_left - (a_left + a * wide) - PAGE_GAP).abs() < 1e-3);
            assert!((a_left - PAGE_GAP).abs() < 1e-3 && (wide - b_left - b * wide - PAGE_GAP).abs() < 1e-3);
        }
    }

    /// Pages' dots too many for their room: which of them it is said instead, within it.
    #[test]
    fn too_many_dots_are_said() {
        let font = Theme::new(Skin::Glass, false).small;
        for (count, room) in [(3, 120.0), (23, 120.0), (113, 200.0), (40, 30.0)] {
            let reach = Reach::default();
            paint_dots(&reach, count, 0.0, room / 2.0, 10.0, Color::hex(0, 1.0), font, room);
            let (left, _, right, _) = reach.reach.get().unwrap();
            assert!(left >= -0.01 && right <= room + 0.01, "{count} in {room}: {left} to {right}");
        }
    }

    /// A workbench widget made smaller than all of it fits in, at any size down to the least, in every skin: what it
    /// shows (each lane in the shape its room is) all within it, and a form of its own saved and read back alike.
    #[test]
    #[cfg(feature = "studio")]
    fn a_workbench_in_a_shape_keeps_within_it() {
        let _pretending = crate::pretending();
        pretend_workbench();
        let ids = ["station", "notes", "apps", "time"];
        for skin in [Skin::Glass, Skin::Fluent, Skin::Paper] {
            let mut theme = Theme::new(skin, false);
            (theme.bar_gap, theme.bar_height) = (0.0, 0.0);
            let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["station".to_string()]));
            let history: &'static [Sample] = Box::leak(Box::new([sample(None, None, None)]));
            let seen = Seen::of(history);
            let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::Zh, history, seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
            for held in [&ids[..1], &ids[1..2], &ids[2..3], &ids[3..], &ids[..]] {
                for w in [40.0, 56.0, 80.0, 130.0, 200.0, 420.0] {
                    for h in [40.0, 56.0, 80.0, 130.0, 260.0, 500.0] {
                        let shape = Shape::of((w, h));
                        assert_eq!(crate::ui::arrange::Form::from_key(&crate::ui::arrange::Form::Lanes { detail: Detail::Shape(shape), columns: 2 }.key()), Some(crate::ui::arrange::Form::Lanes { detail: Detail::Shape(shape), columns: 2 }));
                        let lanes: Vec<Lane> = held.iter().filter_map(|id| Some(Lane { id: id.to_string(), blocks: lane(&scene, id, Detail::Shape(shape))?, ink: theme.ink(id) })).collect();
                        assert_eq!(lanes.len(), held.len());
                        let heights: Vec<f32> = lanes.iter().map(|lane| lane.height(&theme)).collect();
                        let layout = Layout::with_columns(heights, shape.columns(lanes.len()), &theme).fitted((w, h));
                        assert!((layout.width() - w).abs() < 0.01 && (layout.height() - h).abs() < 0.01);
                        let reach = Reach::default();
                        for (x, y, hw, hh, hit) in paint(&reach, &scene, &lanes, &layout, Pass::Content) {
                            assert!(x >= -0.01 && y >= -0.01 && x + hw <= w + 0.01 && y + hh <= h + 0.01, "{skin:?} {held:?} {w}×{h}: {hit:?} at {x},{y} {hw}×{hh}");
                        }
                        // (Nor what is drawn: a shadow past its edge is cut off square there. A line on its edge goes
                        // half a line over.)
                        if let Some((left, top, right, bottom)) = reach.reach.get() {
                            assert!(left >= -1.0 && top >= -1.0 && right <= w + 1.0 && bottom <= h + 1.0, "{skin:?} {held:?} {w}×{h}: drawn over {left},{top} to {right},{bottom}");
                        }
                    }
                }
            }
        }
    }

    /// The station's head's menu button, on each page, found first by a press on it (a menu opens), in every skin.
    #[test]
    fn the_station_s_menu_button_is_found_first() {
        for skin in [Skin::Glass, Skin::Fluent, Skin::Paper] {
            for tab in [Tab::Shelf, Tab::Copies(Default::default()), Tab::Recent(Default::default())] {
                let theme = Theme::new(skin, false);
                let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["station".to_string()]));
                let history: &'static [Sample] = Box::leak(Box::new([sample(None, None, None)]));
                let seen = Seen::of(history);
                let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::Zh, history, seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
                let lanes: Vec<Lane> = ["station"].iter().filter_map(|id| Some(Lane { id: id.to_string(), blocks: lane(&scene, id, Detail::Full)?, ink: theme.ink(id) })).collect();
                let heights: Vec<f32> = lanes.iter().map(|lane| lane.height(&theme)).collect();
                let layout = Layout::with_columns(heights, 1, &theme);
                let hits = paint(&Nothing, &scene, &lanes, &layout, Pass::Content);
                let Some(&(x, y, w, h, _)) = hits.iter().find(|hit| hit.4 == Hit::StationView) else { panic!("{skin:?} {tab:?}: no button") };
                let (px, py) = (x + w / 2.0, y + h / 2.0);
                let first = hits.iter().find(|(hx, hy, hw, hh, _)| px >= *hx && px < hx + hw && py >= *hy && py < hy + hh).map(|hit| hit.4);
                assert_eq!(first, Some(Hit::StationView), "{skin:?} {tab:?}");
            }
        }
    }

    /// The panel's rail: a point at each of its places finds that place (what the panel takes one held to go to),
    /// whatever else lies there, held or not (the places are there before the first frame drawn with one held).
    #[test]
    #[cfg(feature = "studio")]
    fn a_rail_s_places_are_found_while_one_is_dragged() {
        let _pretending = crate::pretending();
        let theme = Theme::new(Skin::Fluent, false);
        let app = |target: &str| crate::station::apps::App { target: target.into(), name: target.into() };
        crate::station::apps::pretend((0..5).map(|at| app(&format!("C:/{at}.exe"))).collect());
        let order: Vec<String> = (0..5).map(|at| format!("C:/{at}.exe")).collect();
        let mut moving = super::super::thumb::Dots::new();
        moving.apps(std::time::Instant::now(), &order, Some((2, 2, Some((30.0, 30.0)))));
        let moving: &'static super::super::thumb::Dots<Hit> = Box::leak(Box::new(moving));
        let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["time".to_string(), "apps".to_string()]));
        let history: &'static [Sample] = Box::leak(Box::new([sample(None, None, None)]));
        let seen = Seen::of(history);
        let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::En, history, seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: moving, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
        let lanes: Vec<Lane> = ["time", "apps"].iter().filter_map(|id| Some(Lane { id: id.to_string(), blocks: lane(&scene, id, Detail::Full)?, ink: theme.ink(id) })).collect();
        let heights: Vec<f32> = lanes.iter().map(|lane| lane.height(&theme)).collect();
        let layout = Layout::with_columns(vec![heights[0].max(600.0)], 1, &theme).with_rail(1, RailSide::Start, RAIL_LEAST);
        let hits = paint(&Nothing, &scene, &lanes, &layout, Pass::Content);
        for at in 0..5u16 {
            let Some(&(x, y, w, h, _)) = hits.iter().find(|hit| hit.4 == Hit::AppSlot(at)) else { panic!("no place {at}: {hits:?}") };
            let (px, py) = (x + w / 2.0, y + h.min(RAIL_CELL) / 2.0 + if at == 0 { h - RAIL_CELL } else { 0.0 });
            // (As the panel finds it: the first place among all that lies there, whatever else is drawn there.)
            let place = hits.iter().filter(|(hx, hy, hw, hh, _)| px >= *hx && px < hx + hw && py >= *hy && py < hy + hh).find_map(|hit| if let Hit::AppSlot(slot) = hit.4 { Some(slot) } else { None });
            assert_eq!(place, Some(at), "at {at}");
        }
    }

    /// One kept at hand held and dragged: the places it may go to stand still where the things were, whatever the
    /// others do to make room (what the pointer is over is not moved from under it: dragged to the top, it went no
    /// further than the next, which moved away under it), and the one held finds none of its own.
    #[test]
    fn places_stand_still_while_one_is_dragged() {
        let theme = Theme::new(Skin::Glass, false);
        let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["apps".to_string()]));
        let history: &'static [Sample] = Box::leak(Box::new([sample(None, None, None)]));
        let seen = Seen::of(history);
        let apps: Vec<(String, String)> = (0..4).map(|at| (format!("{at}"), format!("C:/{at}.exe"))).collect();
        let order: Vec<String> = apps.iter().map(|(_, target)| target.clone()).collect();
        let still: &'static super::super::thumb::Dots<Hit> = Box::leak(Box::new(super::super::thumb::Dots::new()));
        let scene = |thumbs: &'static super::super::thumb::Dots<Hit>| Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::En, history, seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
        let mut hits = Vec::new();
        paint_apps(&Nothing, &scene(still), (0.0, 0.0, 300.0), &apps, APP_ROWS, true, false, &mut hits);
        let at_rest: Vec<(f32, f32)> = (0..4).map(|at| hits.iter().find(|hit| hit.4 == Hit::App(crate::station::apps::number_of(&format!("C:/{at}.exe")))).map(|hit| (hit.0, hit.1)).unwrap()).collect();
        // The last held over the first's place, the others on their way down.
        let mut moving = super::super::thumb::Dots::new();
        let start = std::time::Instant::now();
        moving.apps(start, &order, Some((3, 0, Some((10.0, 10.0)))));
        moving.apps(start + std::time::Duration::from_millis(60), &order, Some((3, 0, Some((10.0, 10.0)))));
        let moving: &'static super::super::thumb::Dots<Hit> = Box::leak(Box::new(moving));
        let mut hits = Vec::new();
        paint_apps(&Nothing, &scene(moving), (0.0, 0.0, 300.0), &apps, APP_ROWS, true, false, &mut hits);
        for (at, place) in at_rest.iter().enumerate() {
            let slot = hits.iter().find(|hit| hit.4 == Hit::AppSlot(at as u16)).map(|hit| (hit.0, hit.1));
            assert_eq!(slot, Some(*place), "place {at}");
        }
        assert!(!hits.iter().any(|hit| matches!(hit.4, Hit::App(_))));
    }

    /// The deck's strips scroll to every note there is (thirteen: more than fit over the last written's card); the
    /// first not among them.
    #[test]
    fn the_deck_reaches_every_note() {
        let theme = Theme::new(Skin::Fluent, false);
        let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["notes".to_string()]));
        let history: &'static [Sample] = Box::leak(Box::new([sample(None, None, None)]));
        let seen = Seen::of(history);
        let notes: Vec<NoteCard> = (0..13).map(|at| NoteCard { number: at, title: format!("{at}"), lines: vec!["a".into()], when: String::new(), colour: Default::default(), held: super::Held::Not }).collect();
        let pressed = |scrolls: Scrolls| {
            let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::En, history, seen: &seen, pen_ms: 0.0, scrolls, tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
            let mut hits = Vec::new();
            paint_notes(&Nothing, &scene, (0.0, 0.0, 320.0), &notes, crate::settings::NotesView::Deck, NOTES_TALL, &mut hits);
            hits.iter().filter_map(|hit| if let Hit::Note(at) = hit.4 { Some(at) } else { None }).collect::<Vec<_>>()
        };
        let at_top = pressed(Scrolls::default());
        assert!(at_top.contains(&0) && at_top.contains(&1) && !at_top.contains(&12), "{at_top:?}");
        let mut scrolls = Scrolls::default();
        scrolls.turn(List::Notes, 10_000.0, 10_000.0);
        for _ in 0..200 {
            scrolls.ease(1.0, 0.1);
        }
        let at_end = pressed(scrolls);
        assert!(at_end.contains(&0) && at_end.contains(&12), "{at_end:?}");
    }

    /// A note stuck on the desktop is drawn as its page is as it is written in, in every skin, at any size it is made:
    /// its words, all of them, written in Markdown as the page's box writes them, just where that box is and as large;
    /// its colour's dot and when it was written where the page has them, pressed on its words, written in there. (It
    /// was drawn as its card on the panel, its title bold and small over a few of its lines cut short: opened to be
    /// written in, its page looked another thing altogether.)
    #[test]
    fn a_note_stuck_on_the_desktop_is_drawn_as_its_page() {
        use crate::ui::morph::{Op, Recorder};
        let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["notes".to_string()]));
        let history: &'static [Sample] = Box::leak(Box::new([sample(None, None, None)]));
        let seen = Seen::of(history);
        let text = "# A note\nIts first line, long enough to be written on two of the page's lines\n- [ ] something to do\n";
        let note = crate::notes::Note { name: "a.md".into(), text: text.into(), colour: crate::notes::Colour::Green, stamp: crate::notes::store::Stamp { bytes: 0, written_ms: 0, digest: 0 } };
        for skin in [Skin::Paper, Skin::Glass, Skin::Fluent] {
            // (Dressed as a widget dresses it: no room round its lane, its page all of it.)
            let mut theme = Theme::new(skin, false);
            (theme.pad_x, theme.pad_top, theme.pad_bottom, theme.outer) = (0.0, 0.0, 0.0, (0.0, 0.0, 0.0));
            let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::Zh, history, seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
            for (w, h) in [STUCK_NOTE, (300.0, 520.0), STUCK_NOTE_LEAST] {
                let lane = Lane { id: format!("{STUCK_NOTE_LANE}a.md"), blocks: vec![stuck_note(&note, Lang::Zh, 60_000)], ink: theme.ink("notes") };
                let recorder = Recorder::new(&Nothing, 1.0, (0.0, 0.0));
                let mut hits = Vec::new();
                paint_lane(&recorder, &scene, &lane, Rect { x: 0.0, y: 0.0, w, h }, 0.0, Pass::Content, &mut hits);
                let ops: Vec<Op> = recorder.parts().into_iter().flat_map(|part| part.ops).collect();
                // Its words where the page's box writes them (at twice the pixels, its box's edges on whole ones).
                let scale = 2.0;
                let page = super::super::note::words_box(0, 0, (w * scale) as i32, (h * scale) as i32, scale);
                let words = ops.iter().find_map(|op| match op {
                    Op::Markdown { text, font, x, y, w, h, .. } => Some((text.clone(), font.size, (*x, *y, *w, *h))),
                    _ => None,
                });
                let boxed = (page.left as f32 / scale, page.top as f32 / scale, (page.right - page.left) as f32 / scale, (page.bottom - page.top) as f32 / scale);
                assert_eq!(words, Some((text.to_string(), super::super::note::WORDS, boxed)), "{skin:?} {w}x{h}");
                // Its dot, and when it was written after it.
                assert!(ops.iter().any(|op| matches!(op, Op::Circle { centre, radius, .. } if (centre.x, centre.y, *radius) == (25.0, 26.0, 5.0))), "{skin:?} {w}x{h}: its dot");
                assert!(ops.iter().any(|op| matches!(op, Op::Text { text, x, y, .. } if text.ends_with("修改") && (*x, *y) == (38.0, 18.0))), "{skin:?} {w}x{h}: when it was written");
                // Its words pressed, written in there, elsewhere, it is the note's.
                let number = crate::notes::number_of("a.md");
                assert_eq!(hits.iter().map(|hit| (hit.0, hit.1, hit.2, hit.3, hit.4)).collect::<Vec<_>>(), vec![(boxed.0, boxed.1, boxed.2, boxed.3, Hit::Words(number)), (0.0, 0.0, w, h, Hit::Note(number))], "{skin:?} {w}x{h}");
            }
            // Written in where it is: nothing of it drawn here (what writes it draws it all), what is pressed on it as
            // ever, no longer, drawn here again.
            for written in [true, false] {
                written_in_place("a.md", written);
                let lane = Lane { id: format!("{STUCK_NOTE_LANE}a.md"), blocks: vec![stuck_note(&note, Lang::Zh, 60_000)], ink: theme.ink("notes") };
                let recorder = Recorder::new(&Nothing, 1.0, (0.0, 0.0));
                let mut hits = Vec::new();
                paint_lane(&recorder, &scene, &lane, Rect { x: 0.0, y: 0.0, w: STUCK_NOTE.0, h: STUCK_NOTE.1 }, 0.0, Pass::Content, &mut hits);
                let drew = recorder.parts().into_iter().flat_map(|part| part.ops).any(|op| matches!(op, Op::Markdown { .. }));
                assert_eq!((drew, hits.len()), (!written, 2), "{skin:?} written {written}");
            }
        }
    }

    /// A note's card carried off the panel leaves the room it took outlined, none of its words written; pressed, all of
    /// it is drawn within its box drawn in a little; as a row, alike.
    #[test]
    fn a_card_held_from_the_panel_is_drawn_as_it_is_held() {
        let theme = Theme::new(Skin::Paper, false);
        let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["notes".to_string()]));
        let history: &'static [Sample] = Box::leak(Box::new([sample(None, None, None)]));
        let seen = Seen::of(history);
        let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::En, history, seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
        let card = |held| NoteCard { number: 1, title: "Title".into(), lines: vec!["line".into()], when: "now".into(), colour: Default::default(), held };
        let room = (10.0, 20.0, 170.0, 104.0);
        type Paint = fn(&dyn Canvas, &Scene, &NoteCard, (f32, f32, f32, f32), bool);
        for (paint, what) in [(paint_note_card as Paint, "card"), (paint_note_row as Paint, "row")] {
            let drawn = |held| {
                let reach = Reach::default();
                paint(&reach, &scene, &card(held), room, false);
                (reach.reach.get().unwrap(), reach.said.take())
            };
            let (away, said) = drawn(Held::Away);
            assert!(said.is_empty(), "{what}: {said:?}");
            assert!((away.0 - 10.0).abs() < 1.0 && (away.1 - 20.0).abs() < 1.0 && (away.2 - 180.0).abs() < 1.0 && (away.3 - 124.0).abs() < 1.0, "{what}: {away:?}");
            let ((pressed, said), (whole, _)) = (drawn(Held::Pressed), drawn(Held::Not));
            assert!(said.contains(&"Title".to_string()), "{what}: {said:?}");
            let (dx, dy) = (170.0 * (1.0 - PRESSED_IN) / 2.0, 104.0 * (1.0 - PRESSED_IN) / 2.0);
            // (Within the box drawn in, but for as far as it reaches past its box unpressed: its rim.)
            let past = (10.0 - whole.0).max(20.0 - whole.1).max(whole.2 - 180.0).max(whole.3 - 124.0).max(0.0) + 0.01;
            assert!(pressed.0 >= 10.0 + dx - past && pressed.1 >= 20.0 + dy - past && pressed.2 <= 180.0 - dx + past && pressed.3 <= 124.0 - dy + past, "{what}: {pressed:?} in {whole:?}");
        }
    }

    /// A thing on the station held to be dragged off it: carried off, its room is outlined, nothing of it written, still
    /// there to press; pressed, its picture is drawn in a little; put back, it springs back from smaller to its size. A
    /// file come lately alike, carried off its row.
    #[test]
    fn a_thing_on_the_station_is_drawn_as_it_is_held() {
        let _holding = HOLDING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let theme = Theme::new(Skin::Paper, false);
        let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["station".to_string()]));
        let history: &'static [Sample] = Box::leak(Box::new([sample(None, None, None)]));
        let seen = Seen::of(history);
        let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::En, history, seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
        let ink = theme.ink("station");
        // (Its own number: none other drawn meanwhile is held so.)
        let id = 0x5EED_0001;
        let hit = Hit::Shelf(id);
        let tiles = [Tile { id, label: "Held".into(), about: String::new(), face: Face::Link, pinned: false }];
        let drawn = || {
            let (reach, mut hits) = (Reach::default(), Vec::new());
            paint_shelf(&reach, &scene, ink, (0.0, 0.0, 324.0), &tiles, Shows::View(ShelfView::Cards), STATION_TALL, None, false, &mut hits);
            (reach.reach.get().unwrap(), reach.said.take(), hits.iter().any(|one| one.4 == hit))
        };
        let (whole, said, pressed_there) = drawn();
        assert!(said.contains(&"Held".to_string()) && pressed_there, "{said:?}");
        carry_tile(Some(hit));
        let (away, said, pressed_there) = drawn();
        carry_tile(None);
        assert!(!said.contains(&"Held".to_string()) && pressed_there, "{said:?}");
        assert!((away.0 - whole.0).abs() < 1.0 && (away.1 - whole.1).abs() < 1.0, "{away:?} of {whole:?}");
        press_tile(Some(hit));
        let (pressed, ..) = drawn();
        press_tile(None);
        let plate = (324.0 - SHELF_GAP * 2.0) / 3.0 * PLATE;
        assert!(pressed.1 > whole.1 + plate * (1.0 - PRESSED_IN) / 2.0 * 0.5 && pressed.0 > whole.0, "{pressed:?} of {whole:?}");
        land_tile(hit, std::time::Instant::now());
        let (landed, ..) = drawn();
        assert!(landed.1 > whole.1 + plate * (1.0 - LANDED_FROM) / 2.0 * 0.5, "{landed:?} of {whole:?}");
        land_tile(hit, std::time::Instant::now() - LANDED_FOR);
        let (rested, ..) = drawn();
        assert_eq!(rested, whole);
        // A file come lately, carried off its row.
        let entry = crate::station::recent::Entry::File(crate::station::recent::Key { drive: 'Q', file: id as u128 });
        let rows = [Lately { entry, day: "Today", label: "Lately".into(), about: String::new(), when: String::new(), path: std::path::PathBuf::new(), more: 0 }];
        let lately = || {
            let (reach, mut hits) = (Reach::default(), Vec::new());
            paint_recent(&reach, &scene, ink, (0.0, 0.0, 324.0), &rows, Default::default(), ShelfView::List, STATION_TALL, false, &mut hits);
            (reach.said.take(), hits.iter().any(|one| one.4 == Hit::Recent(entry)))
        };
        assert!(lately().0.contains(&"Lately".to_string()));
        carry_tile(Some(Hit::Recent(entry)));
        let (said, pressed_there) = lately();
        carry_tile(None);
        assert!(!said.contains(&"Lately".to_string()) && pressed_there, "{said:?}");
    }

    /// A lane given more room than its blocks take (its column taller): the notes' list at its foot goes down to the
    /// lane's foot, not stopping part way.
    #[test]
    fn a_list_at_a_lanes_foot_fills_it() {
        let theme = Theme::new(Skin::Paper, false);
        let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["notes".to_string()]));
        let history: &'static [Sample] = Box::leak(Box::new([sample(None, None, None)]));
        let seen = Seen::of(history);
        let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::En, history, seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
        let notes: Vec<NoteCard> = (0..30).map(|at| NoteCard { number: at, title: format!("{at}"), lines: vec!["a".into()], when: String::new(), colour: Default::default(), held: super::Held::Not }).collect();
        let lane = Lane { id: "notes".into(), blocks: vec![head("Notes", String::from("15"), "", false), Block::Notes { notes, view: crate::settings::NotesView::List, tall: NOTES_TALL }], ink: theme.ink("notes") };
        let area = Rect { x: 0.0, y: 0.0, w: COLUMN_WIDTH, h: 700.0 };
        let mut hits = Vec::new();
        paint_lane(&Nothing, &scene, &lane, area, 0.0, Pass::Content, &mut hits);
        let lowest = hits.iter().filter(|hit| matches!(hit.4, Hit::Note(_))).map(|hit| hit.1 + hit.3).fold(0.0, f32::max);
        assert!(lowest > area.h - theme.pad_bottom - NOTE_ROW, "the notes end at {lowest} in a lane {} tall", area.h);
    }

    /// A list shown another way, or under another filter, starts at its top; shown as before, it stays where it was.
    #[test]
    fn a_list_shown_another_way_starts_at_its_top() {
        let mut scrolls = Scrolls::default();
        scrolls.follow([0, 0, 0, 0, 0, 1, 0]);
        scrolls.turn(List::Notes, 80.0, 500.0);
        scrolls.turn(List::Recent, 40.0, 500.0);
        for _ in 0..200 {
            scrolls.ease(1.0, 0.1);
        }
        scrolls.follow([0, 0, 0, 0, 0, 1, 0]);
        assert!(scrolls.at(List::Notes) > 79.0 && scrolls.at(List::Recent) > 39.0);
        scrolls.follow([0, 0, 0, 0, 0, 2, 0]);
        assert_eq!(scrolls.at(List::Notes), 0.0);
        assert!(scrolls.at(List::Recent) > 39.0);
        scrolls.ease(1.0, 0.1);
        assert_eq!(scrolls.at(List::Notes), 0.0);
    }

    /// The station's tabs, in English with counts, all three: none reaches its buttons (the view's, the menu's), in
    /// each skin; and each of Recent files' five choices is there to press.
    #[test]
    fn the_station_head_leaves_its_buttons_their_room() {
        for skin in [Skin::Fluent, Skin::Glass, Skin::Paper] {
            let theme = Theme::new(skin, false);
            let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["station".to_string()]));
            let history: &'static [Sample] = Box::leak(Box::new([sample(None, None, None)]));
            let seen = Seen::of(history);
            let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::En, history, seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
            let width = COLUMN_WIDTH - 2.0 * theme.pad_x;
            let mut hits = Vec::new();
            paint_tabs(&Nothing, &scene, (0.0, 0.0, width), Tab::Shelf, 128, true, true, ShelfView::Cards, &mut hits);
            let buttons: Vec<HitBox> = hits.iter().filter(|hit| matches!(hit.4, Hit::StationView)).copied().collect();
            assert_eq!(buttons.len(), 1, "{skin:?}");
            for tab in hits.iter().filter(|hit| matches!(hit.4, Hit::Tab(_))) {
                for button in &buttons {
                    assert!(tab.0 + tab.2 <= button.0, "{skin:?}: {tab:?} over {button:?}");
                }
            }
            let mut hits = Vec::new();
            paint_recent(&Nothing, &scene, theme.ink("station"), (0.0, 0.0, width), &[], Default::default(), ShelfView::List, STATION_TALL, false, &mut hits);
            let chips: Vec<HitBox> = hits.iter().filter(|hit| matches!(hit.4, Hit::Tab(Tab::Recent(_)))).copied().collect();
            assert_eq!(chips.len(), 5, "{skin:?}");
            assert!(chips.iter().all(|chip| chip.0 + chip.2 <= width + 0.01), "{skin:?}: {chips:?}");
            // And Recently copied's four, in Chinese too, in a lane as narrow as a workbench's gets.
            for (lang, wide) in [(Lang::En, width), (Lang::Zh, width), (Lang::Zh, LANE_LEAST)] {
                let scene = Scene { lang, ..scene };
                let mut hits = Vec::new();
                paint_copies(&Nothing, &scene, theme.ink("station"), (0.0, 0.0, wide), &[], Default::default(), ShelfView::List, STATION_TALL, &mut hits);
                let chips: Vec<Tab> = hits.iter().filter_map(|hit| if let Hit::Tab(tab) = hit.4 { Some(tab) } else { None }).collect();
                assert_eq!(chips, crate::station::shelf::Filter::ALL.map(Tab::Copies), "{skin:?} {}", lang.pick("zh", "en"));
                assert!(hits.iter().all(|chip| chip.0 + chip.2 <= wide + 0.01), "{skin:?} {}: {hits:?}", lang.pick("zh", "en"));
            }
        }
    }

    /// Files copied that are gone from where they were, with no copy of them kept, are said to be gone; one still
    /// there, or one whose copy is kept, is a file as before.
    #[test]
    fn a_file_copied_and_gone_is_said_to_be() {
        use crate::station::shelf::{Item, What};
        crate::station::thumbs::start(|| {});
        let here = std::env::current_exe().unwrap();
        let gone = std::env::temp_dir().join("glance-no-such-file.png");
        let item = |paths: Vec<std::path::PathBuf>, kept: Vec<String>| Item { id: 1, what: What::Files { paths, kept }, added_ms: 0, pinned: false, copied: true, stays_here: false, own_bytes: 0 };
        let folder = crate::station::folder::Folder::own();
        let about = |item: &Item| {
            // (Looked at on a thread of its own: asked till it answers.)
            for _ in 0..200 {
                let both = [&here, &gone].iter().all(|path| crate::station::thumbs::looked(path).is_some());
                if both {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Tile::of(item, &folder, Lang::Zh, 0).about
        };
        assert!(about(&item(vec![gone.clone()], Vec::new())).starts_with("文件已不存在 · "));
        assert!(about(&item(vec![gone.clone(), gone.clone()], Vec::new())).starts_with("2 个文件已不存在 · "));
        assert!(about(&item(vec![gone.clone(), here.clone()], Vec::new())).starts_with("2 个文件 · "));
        assert!(about(&item(vec![here.clone()], Vec::new())).starts_with("文件 · "));
        assert!(about(&item(vec![gone.clone()], vec!["kept.png".into()])).starts_with("文件 · "));
    }

    /// Recently copied with a filter chosen shows only what it takes, the newest first, and its page is chosen again
    /// with it (a tab or the pages' menu), while another page shown chooses it from the start.
    #[test]
    fn recently_copied_shows_what_its_filter_takes() {
        use crate::station::shelf::{Filter, Item, What};
        let item = |id: u64, what: What, copied: bool| Item { id, what, added_ms: id, pinned: false, copied, stays_here: false, own_bytes: 0 };
        let items = [
            item(1, What::Text { text: "a".into() }, true),
            item(2, What::Picture { file: "2.png".into() }, true),
            item(3, What::Link { url: "https://glancepc.com".into() }, true),
            item(4, What::Text { text: "put by hand".into() }, false),
            item(5, What::Files { paths: vec![std::path::PathBuf::from(r"C:\a.txt")], kept: Vec::new() }, true),
        ];
        let ids = |filter: Filter| copied_lately(&items, filter).map(|item| item.id).collect::<Vec<_>>();
        assert_eq!(ids(Filter::All), [5, 3, 2, 1]);
        assert_eq!(ids(Filter::Words), [3, 1]);
        assert_eq!(ids(Filter::Pictures), [2]);
        assert_eq!(ids(Filter::Files), [5]);
        let words = Tab::Copies(Filter::Words);
        assert_eq!(Tab::Copies(Filter::All).again(words), words);
        assert_eq!(Tab::Copies(Filter::All).again(Tab::Recent(Default::default())), Tab::Copies(Filter::All));
        assert_eq!(Tab::Recent(Default::default()).again(words), Tab::Recent(Default::default()));
        assert_eq!(words.page(), Tab::Copies(Filter::All));
        // (A filter chosen starts its list from the top: it scrolls for each alone.)
        assert_ne!(lists_show(words)[List::Copies as usize], lists_show(Tab::Copies(Filter::All))[List::Copies as usize]);
    }

    /// Quick launch whole in a column: its rows as many as its things take across the room its skin leaves (the place
    /// to add more within them, never under them).
    #[test]
    fn quick_launch_takes_the_rows_it_is_laid_out_in() {
        for skin in [Skin::Fluent, Skin::Glass, Skin::Paper] {
            let theme = Theme::new(skin, false);
            let (info, prefs) = (info(), Prefs::resolve(&serde_json::Value::Null, &["apps".to_string()]));
            let history: &'static [Sample] = Box::leak(Box::new([sample(None, None, None)]));
            let seen = Seen::of(history);
            let scene = Scene { music_apart: false, info: &info, prefs: &prefs, theme: &theme, lang: Lang::En, history, seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::Shelf, hover: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
            let width = COLUMN_WIDTH - 2.0 * theme.pad_x;
            let across = ((width / APP_CELL_W) as usize).max(1);
            // As many as fill the first row but the place to add more: it on the same row; one more: on a second.
            for count in [across - 1, across] {
                let apps: Vec<(String, String)> = (0..count).map(|at| (format!("{at}"), format!("C:/{at}.exe"))).collect();
                let rows = apps_rows(count, &theme);
                let block = Block::Apps { apps: apps.clone(), rows, named: true, over: false };
                let mut hits = Vec::new();
                paint_apps(&Nothing, &scene, (0.0, 0.0, width), &apps, rows, true, false, &mut hits);
                let adder = hits.iter().find(|hit| hit.4 == Hit::AddApp).copied().expect("the place to add more");
                assert!(adder.1 + adder.3 <= block.height() - SHELF_GAP + 0.01, "{skin:?} {count}: {adder:?}");
            }
        }
    }

    /// The station's places as each view lays them out: cards three across a column, filling it, each a plate as wide
    /// and its words under it; a list a row each; large cards two across; a small lane's one row, as many as fit.
    #[test]
    fn lays_the_station_out_as_its_view_says() {
        let width = COLUMN_WIDTH - 32.0;
        let (cards, tall) = shelf_places(Shows::View(ShelfView::Cards), 9, width, STATION_TALL);
        let across = cards_across(width);
        assert_eq!(across, 3);
        let card = cards[0];
        assert!((card.w * 3.0 + 2.0 * TILE_GAP - width).abs() < 0.01);
        assert!((card.h - (card.w * PLATE + CARD_WORDS)).abs() < 0.01);
        assert_eq!((cards[across].x, cards[across].y), (0.0, card.h + TILE_GAP));
        assert!((cards[across - 1].x + card.w - width).abs() < 0.01);
        assert!((tall - (3.0 * card.h + 2.0 * TILE_GAP)).abs() < 0.01);
        let (list, tall) = shelf_places(Shows::View(ShelfView::List), 3, width, STATION_TALL);
        assert_eq!((list[2].y, list[2].w, tall), (2.0 * LIST_ROW, width, 3.0 * LIST_ROW));
        // Large cards: two across, filling it, each a plate and its words under it, two rows of them whole in the room,
        // on the station's page and on Recent files' (under its filters).
        for room in [STATION_TALL, STATION_TALL - CHIP_H - 6.0] {
            let (large, _) = shelf_places(Shows::View(ShelfView::Large), 4, width, room);
            assert!((large[0].w * 2.0 + TILE_GAP - width).abs() < 0.01);
            assert_eq!((large[2].x, large[3].x), (0.0, large[1].x));
            assert!(CARDS_LEAD + large[3].y + large[3].h <= room + 0.01, "{room}: {:?}", large[3]);
        }
        let (row, tall) = shelf_places(Shows::Row, 9, width, STATION_TALL);
        assert_eq!((row.len(), tall), (tiles_across(width), TILE_H));
    }

    #[test]
    fn keeps_the_order_of_what_it_has_seen() {
        let fans = |names: &[&str]| {
            let mut s = sample(None, None, None);
            s.board = Some(crate::reading::BoardSensors { temps: Vec::new(), fans: names.iter().map(|n| (n.to_string(), 900.0)).collect() });
            s
        };
        let order = |history: Vec<Sample>| Seen::of(&history).board.unwrap().fans;
        // Fan 2 alone at first, then both: 1 goes before 2, as read.
        assert_eq!(order(vec![fans(&["2"]), fans(&["1", "2"])]), ["1", "2"]);
        // One missing later keeps its place, beside what came before it.
        assert_eq!(order(vec![fans(&["1", "2", "3"]), fans(&["1", "3"])]), ["1", "2", "3"]);
        assert_eq!(order(vec![fans(&["1", "2", "3"]), fans(&["2", "3"])]), ["1", "2", "3"]);
    }

    #[test]
    fn rounds_rates_up_to_a_clean_scale() {
        assert_eq!(round_up_rate(1500.0 * 1024.0), 2.0 * 1024.0 * 1024.0);
        assert_eq!(round_up_rate(30.0 * 1024.0), 50.0 * 1024.0);
        assert_eq!(round_up_rate(10.0 * 1024.0), 10.0 * 1024.0);
    }
}
