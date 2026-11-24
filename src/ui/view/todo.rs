//! The to-dos' lane (see `crate::todo`): its head (how many are left, and a plus to add one), the field to add one in,
//! and the items, those not done first, then a line for those done (folded or open) and them, the last done first.
//!
//! An item ticked is ticked where it is, its box filled and its tick drawn, its words struck through; a moment after,
//! it sinks to the top of those done. Each item moves from where it was drawn to where it now goes (a new one drops
//! from the field to the top), as what it says changes: they move at the frame they are drawn in, in the lane of every
//! window that shows them alike (see `PLACES`).
//!
//! An item written in where it shows (see `ui::todo::edit`) is drawn as it was, its words those written, the caret in
//! them in the accent, what is chosen marked, its buttons put away (its time shown as it was).

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::*;
use crate::todo::{Day, Item};

/// An item's row, the field, the line of those done and the line saying how many more there are (DIPs).
const ROW: f32 = 32.0;
const FIELD: f32 = FIELD_TALL;
const FIELD_GAP: f32 = 6.0;
const DONE_HEAD: f32 = 30.0;
const MORE: f32 = 26.0;
/// A box to tick (DIPs, square), and the gap after it.
const BOX: f32 = 18.0;
const BOX_GAP: f32 = 10.0;
/// What a press on an item's time, or its buttons as the pointer is on it, takes (DIPs).
const ACT: f32 = 24.0;
/// How much room the lane's list is laid out in, whole: room for six items.
pub(super) const TODO_TALL: f32 = 6.0 * ROW;

/// How long an item ticked stays where it was (its tick drawn) before it sinks among those done; how long its tick
/// takes to draw; how long an item takes to move to its new place.
const SINK: Duration = Duration::from_millis(600);
const TICK: Duration = Duration::from_millis(260);
const MOVE: Duration = Duration::from_millis(340);

/// What a press on the lane lands on: an item (by its place among the note's and what it is: see `key_of`), its box,
/// its words (written in where they show, one not done), its time (or the button that sets one), the button that takes
/// it out; the field, the button that sets the time of the one being written, the head's plus; the line of those done
/// (folded or opened by it) and what clears them.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TodoHit {
    Row(u32, u64),
    Words(u32, u64),
    Tick(u32, u64),
    Due(u32, u64),
    Remove(u32, u64),
    Field,
    FieldDue,
    Add,
    Done,
    Clear,
}

impl TodoHit {
    /// The item it is on, where it is on one: its place among the note's, and what it is.
    pub fn item(self) -> Option<(usize, u64)> {
        match self {
            TodoHit::Row(n, key) | TodoHit::Words(n, key) | TodoHit::Tick(n, key) | TodoHit::Due(n, key) | TodoHit::Remove(n, key) => Some((n as usize, key)),
            _ => None,
        }
    }
}

/// What an item is known by while it moves: its words, and which of the items with those words it is.
pub fn key_of(items: &[&Item], n: usize) -> u64 {
    let text = &items[n].text;
    let nth = items[..n].iter().filter(|item| &item.text == text).count();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (text, nth).hash(&mut hasher);
    hasher.finish()
}

/// The field to add an item in, as it is being written in its window (see `ui::todo`): whether it is, its words
/// (UTF-16), the caret and where the selection began, whether the caret shows now, and the time set for it (its label);
/// and the item written in where it shows in its stead, where one is.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct Field {
    pub writing: bool,
    pub text: Vec<u16>,
    pub caret: usize,
    pub anchor: usize,
    pub caret_on: bool,
    pub due: Option<String>,
    pub edit: Option<Editing>,
}

/// An item written in where it shows (see `ui::todo::edit`): the `n`th, keyed `key`, its words as written (UTF-16), the
/// caret and where the selection began, and whether the caret shows now.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Editing {
    pub n: u32,
    pub key: u64,
    pub text: Vec<u16>,
    pub caret: usize,
    pub anchor: usize,
    pub caret_on: bool,
}

/// An item as its row shows it.
#[derive(Clone, PartialEq, Debug)]
struct Row {
    n: u32,
    key: u64,
    text: String,
    done: bool,
    /// Its time's label, and whether it is due by today (said in the accent).
    label: Option<String>,
    soon: bool,
    /// How far its tick is drawn (0 not ticked, 1 ticked).
    tick: f32,
    /// How lit it is, opened from its notification (see `ui::todo::lit`).
    lit: f32,
}

impl Hash for Row {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.n, self.key, &self.text, self.done, &self.label, self.soon, self.tick.to_bits(), self.lit.to_bits()).hash(state);
    }
}

/// The lane's items as it shows them: those not done (and those just ticked, still where they were), in their order;
/// those done, the last done first; whether those are shown (opened) or folded into their line; the field (none in a
/// small lane), as its window has it; and, while anything moves, the time (for the lane to be drawn anew each frame).
/// The item written in where it shows, where one is (see `Editing`).
#[derive(Clone, PartialEq, Debug)]
pub struct TodoLane {
    undone: Vec<Row>,
    done: Vec<Row>,
    open: bool,
    field: Option<Field>,
    moving: u64,
    edit: Option<Editing>,
}

impl Hash for TodoLane {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (&self.undone, &self.done, self.open, &self.field, self.moving, &self.edit).hash(state);
    }
}

thread_local! {
    /// Items ticked here, each drawn where it was among those not done (its place there) till `SINK` after it was ticked.
    static TICKED: RefCell<Vec<(u64, usize, Instant)>> = const { RefCell::new(Vec::new()) };
    /// Where each item was last drawn going (its key's): from where, to where (DIPs down the list), since when.
    static PLACES: RefCell<HashMap<u64, (f32, f32, Instant)>> = RefCell::new(HashMap::new());
    /// An item added here just now (its key): it drops from the field, not from nowhere.
    static ADDED: RefCell<Option<u64>> = const { RefCell::new(None) };
    /// Whether the last drawing of the lane had anything moving.
    static MOVING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Whether those done are shown, not folded into their line.
    static OPEN: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    /// Where the caret was drawn in the field (or the words of the item written in where it shows), from their corner
    /// (DIPs: across, down, how tall), and how large its words are, for the input method's words to be put there as
    /// they are.
    static CARET: std::cell::Cell<(f32, f32, f32, f32)> = const { std::cell::Cell::new((0.0, 0.0, 0.0, 0.0)) };
    /// How far the words of the item written in where it shows were last drawn scrolled (DIPs), for its caret to show.
    static EDIT_SHIFT: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
}

/// The item `key` ticked here, `slot`th among those not done: drawn there a moment (see `SINK`).
pub fn ticked(key: u64, slot: usize) {
    TICKED.with_borrow_mut(|ticked| {
        ticked.retain(|(each, ..)| *each != key);
        ticked.push((key, slot, Instant::now()));
    });
}

/// The item `key` added here: it drops from the field.
pub fn added(key: u64) {
    ADDED.with_borrow_mut(|added| *added = Some(key));
}

/// Those done shown, or folded into their line.
pub fn toggle_open() {
    OPEN.with(|open| open.set(!open.get()));
}

/// Where the caret was last drawn in the field (see `CARET`).
pub fn caret() -> (f32, f32, f32, f32) {
    CARET.with(std::cell::Cell::get)
}

/// How far the words of the item written in where it shows were last drawn scrolled (see `EDIT_SHIFT`).
pub fn edit_shift() -> f32 {
    EDIT_SHIFT.with(std::cell::Cell::get)
}

/// The colours a box ticked is filled with and its tick drawn in (and a choice marked, in the time's picker): glass's
/// white on dark glass, a clear blue on light; chart paper's its signal; Windows 11's its accent.
pub fn mark(theme: &Theme) -> (Color, Color) {
    match theme.skin {
        Skin::Glass if theme.dark => (Color::hex(0xFFFFFF, 1.0), Color::hex(0x1B1B22, 1.0)),
        Skin::Glass => (Color::hex(0x0A7AFF, 1.0), Color::hex(0xFFFFFF, 1.0)),
        Skin::Paper => (theme.signal, if theme.dark { Color::hex(0x14231F, 1.0) } else { Color::hex(0xFFFFFF, 1.0) }),
        Skin::Fluent => (super::super::theme::accent(theme.dark), if theme.dark { Color::hex(0x000000, 1.0) } else { Color::hex(0xFFFFFF, 1.0) }),
    }
}

/// The colour a time due by today is said in: chart paper's signal; else the system's accent.
pub fn soon(theme: &Theme) -> Color {
    match theme.skin {
        Skin::Paper => theme.signal,
        _ => super::super::theme::accent(theme.dark),
    }
}

/// The to-dos' lane: whole (`full`), the head, the field and the items; small, the head and the item due soonest.
pub(super) fn todo_lane(full: bool, lang: Lang) -> Vec<Block> {
    let list = crate::todo::list();
    let items = list.items();
    let today = Day::today();
    let zh = lang == Lang::Zh;
    let now = Instant::now();
    let row = |n: usize, tick: f32| {
        let item = items[n];
        Row {
            n: n as u32,
            key: key_of(&items, n),
            text: item.text.clone(),
            done: item.done,
            label: item.due.map(|due| due.label(today, zh)),
            soon: item.due.is_some_and(|due| due.by(today)) && !item.done,
            tick,
            lit: crate::ui::todo::lit(item, now),
        }
    };
    // Those just ticked here stay where they were (and those unticked since are no longer).
    let keys: Vec<u64> = (0..items.len()).map(|n| key_of(&items, n)).collect();
    // (Where those gone were drawn forgotten.)
    PLACES.with_borrow_mut(|places| places.retain(|key, _| keys.contains(key)));
    let ticked: Vec<(usize, usize, f32)> = TICKED.with_borrow_mut(|ticked| {
        ticked.retain(|(key, _, since)| now.duration_since(*since) < SINK && keys.iter().position(|each| each == key).is_some_and(|n| items[n].done));
        ticked.iter().map(|(key, slot, since)| (keys.iter().position(|each| each == key).unwrap(), *slot, (now.duration_since(*since).as_secs_f32() / TICK.as_secs_f32()).min(1.0))).collect()
    });
    let mut undone: Vec<Row> = (0..items.len()).filter(|n| !items[*n].done).map(|n| row(n, 0.0)).collect();
    let mut placed: Vec<(usize, usize, f32)> = ticked.clone();
    placed.sort_by_key(|(_, slot, _)| *slot);
    for (n, slot, tick) in placed {
        undone.insert(slot.min(undone.len()), row(n, tick));
    }
    let done: Vec<Row> = (0..items.len()).filter(|n| items[*n].done && !ticked.iter().any(|(t, ..)| t == n)).map(|n| row(n, 1.0)).collect();
    let lit = undone.iter().any(|row| row.lit > 0.0);
    let moving = if MOVING.with(std::cell::Cell::get) || !ticked.is_empty() || lit { crate::station::now_ms() } else { 0 };
    let count = match undone.iter().filter(|row| !row.done).count() {
        0 => String::new(),
        n => n.to_string(),
    };
    let head = head(lang.pick("待办", "To-do"), count, "", false);
    if full {
        return vec![head, Block::Todo { lane: TodoLane { undone, done, open: OPEN.with(std::cell::Cell::get), field: Some(Field::default()), moving, edit: None }, tall: TODO_TALL }];
    }
    // Small: the one shown first, alone.
    let lits: Vec<f32> = items.iter().map(|item| crate::ui::todo::lit(item, now)).collect();
    let shown = shown_first(&items, &lits).map(|n| row(n, 0.0));
    vec![head, Block::Todo { lane: TodoLane { undone: shown.into_iter().collect(), done: Vec::new(), open: false, field: None, moving, edit: None }, tall: ROW }]
}

/// Of `items`, the one not done that is due soonest (else the first not done).
fn soonest(items: &[&Item]) -> Option<usize> {
    let undone = || (0..items.len()).filter(|n| !items[*n].done);
    undone().filter(|n| items[*n].due.is_some()).min_by_key(|n| items[*n].due).or_else(|| undone().next())
}

/// Of `items` (each as lit as `lits` has it), the one a small lane shows, or shows first: the one lit, opened from its
/// notification, else the one due soonest (else the first not done).
fn shown_first(items: &[&Item], lits: &[f32]) -> Option<usize> {
    (0..items.len()).find(|&n| !items[n].done && lits[n] > 0.0).or_else(|| soonest(items))
}

/// The field of `lanes`' to-do lane shown as it is being written (see `Field`), and the item written in where it shows
/// as it is, in the lane whole, small or in a shape.
pub fn write_in(lanes: &mut [Lane], field: &Field) {
    for lane in lanes.iter_mut().filter(|lane| lane.id == "todo") {
        for block in &mut lane.blocks {
            match block {
                Block::Todo { lane, .. } => {
                    if lane.field.is_some() {
                        lane.field = Some(field.clone());
                    }
                    lane.edit = field.edit.clone();
                }
                Block::Bench(Bench::Todo(_, edit)) => *edit = field.edit.clone(),
                _ => {}
            }
        }
    }
}

/// How tall the to-do block is: the field (in a whole lane) and the list's room.
pub(super) fn height(lane: &TodoLane, tall: f32) -> f32 {
    SHELF_GAP + if lane.field.is_some() { FIELD + FIELD_GAP } else { 0.0 } + tall
}

/// The to-dos' head: its title, how many are left, and the plus that adds one (the field taken to write in).
pub(super) fn paint_head(frame: &dyn Canvas, scene: &Scene, (left, y, width): (f32, f32, f32), title: &str, count: &str, hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    let title_width = frame.measure(title, theme.title);
    let row = y + (HEAD - theme.title.size) / 2.0 - 1.0;
    frame.text(title, theme.title, theme.text, left, row, width, Align::Start);
    frame.text(count, theme.small, theme.text3, left + title_width + 8.0, row + 1.0, width, Align::Start);
    let button = y + (HEAD - MEDIA_ROW) / 2.0;
    pressed(frame, scene, hits, Hit::Todo(TodoHit::Add), Icon::Add, left + width - MEDIA_ROW, button, false);
}

/// The field and the items, from (`left`, `y`), `width` across (`pad` in from the lane's edges), in a list room `tall`
/// high.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint(frame: &dyn Canvas, scene: &Scene, (left, y, width): (f32, f32, f32), pad: f32, lane: &TodoLane, tall: f32, hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    let mut top = y + SHELF_GAP;
    if let Some(field) = &lane.field {
        // (Concentric with the lane's corners: see `field_fit`.)
        let (out, round) = field_fit(theme, theme.radius, pad);
        paint_field(frame, scene, (left - out, top, width + 2.0 * out), round, field, hits);
        top += FIELD + FIELD_GAP;
    }
    let lang = scene.lang;
    let undone = lane.undone.len();
    let done = lane.done.len();
    // Where each goes, as the room takes them: those not done as many as fit (the last place saying how many more,
    // where not all do), the line of those done under them and, opened, as many of those as fit after.
    let head_room = if done > 0 { DONE_HEAD } else { 0.0 };
    let fit_undone = if undone as f32 * ROW + head_room <= tall { undone } else { ((tall - head_room - MORE) / ROW).floor().max(0.0) as usize };
    let more = undone - fit_undone.min(undone);
    // (One lit, opened from its notification, past those that fit: in the last place of them while it is.)
    let mut shown: Vec<&Row> = lane.undone.iter().take(fit_undone).collect();
    if let (Some(lit), Some(last)) = (lane.undone.iter().skip(fit_undone).find(|row| row.lit > 0.0), shown.last_mut()) {
        *last = lit;
    }
    let mut targets: Vec<(&Row, f32)> = shown.into_iter().enumerate().map(|(at, row)| (row, at as f32 * ROW)).collect();
    let after_undone = fit_undone as f32 * ROW + if more > 0 { MORE } else { 0.0 };
    let done_head = after_undone;
    let mut done_more = 0;
    if lane.open && done > 0 {
        let room = tall - done_head - DONE_HEAD;
        let fit = if done as f32 * ROW <= room { done } else { ((room - MORE) / ROW).floor().max(0.0) as usize };
        done_more = done - fit.min(done);
        targets.extend(lane.done.iter().take(fit).enumerate().map(|(at, row)| (row, done_head + DONE_HEAD + at as f32 * ROW)));
    }
    // Each moved from where it was drawn going.
    let now = Instant::now();
    let added = ADDED.with_borrow_mut(Option::take);
    let mut moving = false;
    let drawn: Vec<(&Row, f32, f32)> = PLACES.with_borrow_mut(|places| {
        targets
            .iter()
            .map(|(row, to)| {
                let place = places.entry(row.key).or_insert_with(|| if added == Some(row.key) { (-(FIELD + FIELD_GAP), *to, now) } else { (*to, *to, now) });
                let at = |place: &(f32, f32, Instant)| {
                    let t = (now.duration_since(place.2).as_secs_f32() / MOVE.as_secs_f32()).min(1.0);
                    place.0 + (place.1 - place.0) * ease_out(t)
                };
                if place.1 != *to {
                    *place = (at(place), *to, now);
                }
                let y = at(place);
                let fade = if place.0 < 0.0 { ((y - place.0) / (place.1 - place.0).max(1.0)).clamp(0.0, 1.0) } else { 1.0 };
                moving |= (y - to).abs() > 0.01;
                (*row, y, fade)
            })
            .collect()
    });
    moving |= lane.undone.iter().any(|row| row.tick > 0.0);
    MOVING.with(|was| was.set(moving));
    if moving {
        GLIDING.with(|gliding| gliding.set(true));
    }
    frame.clip(left - 4.0, top, width + 8.0, tall);
    for (row, at, fade) in drawn {
        paint_row(frame, scene, (left, top + at, width), row, fade, lane.edit.as_ref(), hits);
    }
    let small = theme.small;
    let said_more = |frame: &dyn Canvas, n: usize, y: f32| {
        let says = if lang == Lang::Zh { format!("还有 {n} 项") } else { format!("{n} more") };
        frame.text(&says, small, theme.text3, left + BOX + BOX_GAP, frame.line_in(small, y, MORE), width, Align::Start);
    };
    if more > 0 {
        said_more(frame, more, top + fit_undone as f32 * ROW);
    }
    if done > 0 {
        paint_done_head(frame, scene, (left, top + done_head, width), done, lane.open, hits);
        if done_more > 0 {
            said_more(frame, done_more, top + tall - MORE);
        }
    }
    frame.unclip();
    // Nothing to do: said where the items would be.
    if undone == 0 && done == 0 {
        let says = lang.pick("没有待办事项", "Nothing to do");
        frame.text(says, small, theme.text3, left, frame.line_in(small, top, ROW), width, Align::Start);
    }
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

/// The field at (`left`, `y`), `width` across: a plus, the words written (or what to write), the time set for them, and
/// the button that sets it; while it is written in, ringed in the mark's colour (Windows 11's a bar of its accent under
/// it), its caret and selection shown.
fn paint_field(frame: &dyn Canvas, scene: &Scene, (left, y, width): (f32, f32, f32), round: f32, field: &Field, hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    let (mark, _) = mark(theme);
    let lit = scene.hover == Some(Hit::Todo(TodoHit::Field)) && !field.writing;
    paint_field_ground(frame, theme, (left, y, width), round, lit, field.writing);
    let middle = y + FIELD / 2.0;
    frame.icon(Icon::Add, Point { x: left + FIELD_ICON_AT, y: middle }, FIELD_ICON, theme.text3);
    // The time's button, and the time set before it.
    let button = left + width - ACT - (FIELD - ACT) / 2.0;
    let lit = scene.hover == Some(Hit::Todo(TodoHit::FieldDue));
    if lit {
        frame.fill_rounded(theme.hover, button, y + (FIELD - ACT) / 2.0, ACT, ACT, theme.control_radius.min(ACT / 2.0));
    }
    frame.icon(Icon::Calendar, Point { x: button + ACT / 2.0, y: y + FIELD / 2.0 }, 15.0, if lit { theme.text } else { theme.text2 });
    hits.push((button, y + (FIELD - ACT) / 2.0, ACT, ACT, Hit::Todo(TodoHit::FieldDue)));
    let mut words_right = button - 4.0;
    let small = theme.small;
    if let Some(due) = &field.due {
        let chip = frame.measure(due, small) + 14.0;
        let x = words_right - chip;
        frame.fill_rounded(theme.track, x, y + (FIELD - 22.0) / 2.0, chip, 22.0, (round - (FIELD - 22.0) / 2.0).max(4.0));
        frame.text(due, small, theme.text, x + 7.0, frame.line_in(small, y, FIELD), chip, Align::Start);
        words_right = x - 6.0;
    }
    let words_left = left + FIELD_WORDS;
    let room = (words_right - words_left).max(0.0);
    let font = theme.body;
    let line_y = frame.line_in(font, y, FIELD);
    // (The caret, and what is chosen, as tall as a line of it, across the middle.)
    let (caret_top, caret_tall) = (middle - font.size * 0.65, font.size * 1.3);
    if field.text.is_empty() {
        let says = scene.lang.pick("添加待办，回车保存", "Add a to-do, Enter to keep");
        frame.text(&fitted(frame, says, font, room), font, theme.text3, words_left, line_y, room + 1.0, Align::Start);
    }
    // Scrolled so that the caret shows.
    let text = String::from_utf16_lossy(&field.text);
    let before = |at: usize| frame.measure(&String::from_utf16_lossy(&field.text[..at.min(field.text.len())]), font);
    let caret_x = before(field.caret);
    let shift = (caret_x - room + 2.0).max(0.0);
    frame.clip(words_left, y, room, FIELD);
    if field.caret != field.anchor {
        let (from, to) = (before(field.caret.min(field.anchor)), before(field.caret.max(field.anchor)));
        frame.fill(mark.alpha(0.3), words_left + from - shift, caret_top, to - from, caret_tall);
    }
    frame.text(&text, font, theme.text, words_left - shift, line_y, room + shift + 200.0, Align::Start);
    if field.writing && field.caret_on {
        frame.fill(theme.text, words_left + caret_x - shift, caret_top, 1.2, caret_tall);
    }
    frame.unclip();
    if field.writing {
        CARET.with(|caret| caret.set((words_left + caret_x - shift - left, caret_top - y, caret_tall, font.size)));
    }
    hits.push((left, y, width, FIELD, Hit::Todo(TodoHit::Field)));
}

/// An item's row at (`left`, `y`), `width` across, `fade` faded in (dropping from the field): its box, its words (struck
/// through, done), its time; as the pointer is on it, the buttons that set its time (where it has none) and take it out.
/// Where it is the one written in where it shows (`edit`), the words written there, scrolled for its caret to show, the
/// caret in the accent and what is chosen marked, and no buttons.
#[allow(clippy::too_many_arguments)]
fn paint_row(frame: &dyn Canvas, scene: &Scene, (left, y, width): (f32, f32, f32), row: &Row, fade: f32, edit: Option<&Editing>, hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    let (n, key) = (row.n, row.key);
    let edit = edit.filter(|edit| (edit.n, edit.key) == (n, key) && !row.done);
    let on = edit.is_none() && matches!(scene.hover, Some(Hit::Todo(hit)) if hit.item() == Some((n as usize, key)));
    let (mark, on_mark) = mark(theme);
    let alpha = |color: Color| color.alpha(fade);
    // Lit, opened from its notification: a wash of the accent behind it all.
    if row.lit > 0.0 {
        frame.fill_rounded(soon(theme).alpha(0.24 * row.lit * fade), left - 6.0, y + 1.0, width + 12.0, ROW - 2.0, theme.control_radius.min((ROW - 2.0) / 2.0));
    }
    // Its box: ruled round; ticked, filled and its tick drawn left to right as it is ticked.
    let bx = left;
    let by = y + (ROW - BOX) / 2.0;
    let radius = match theme.skin {
        Skin::Glass => BOX / 2.0,
        Skin::Paper => 5.0,
        Skin::Fluent => 4.0,
    };
    let tick_lit = scene.hover == Some(Hit::Todo(TodoHit::Tick(n, key)));
    let filled = (row.tick * 2.0).min(1.0);
    if filled < 1.0 {
        let edge = if tick_lit { mark } else { theme.text3 };
        frame.stroke_rounded(Fill::Solid(alpha(edge)), bx + 0.8, by + 0.8, BOX - 1.6, BOX - 1.6, radius, 1.6);
    }
    if row.tick > 0.0 {
        frame.fill_rounded(alpha(mark.alpha(filled)), bx, by, BOX, BOX, radius);
        frame.clip_part(bx, by, BOX, BOX, ease_out(row.tick), false);
        frame.icon(Icon::Check, Point { x: bx + BOX / 2.0, y: by + BOX / 2.0 }, BOX * 0.72, alpha(on_mark));
        frame.unclip();
    }
    hits.push((bx - 4.0, y, BOX + 8.0, ROW, Hit::Todo(TodoHit::Tick(n, key))));
    // Its time, at the end (a button where the pointer is on it), and its buttons before it as it is.
    let small = theme.small;
    let mut right = left + width;
    let remove = Hit::Todo(TodoHit::Remove(n, key));
    let due = Hit::Todo(TodoHit::Due(n, key));
    let act = |frame: &dyn Canvas, hits: &mut Vec<HitBox>, x: f32, hit: Hit, icon: Icon| {
        let lit = scene.hover == Some(hit);
        if lit {
            frame.fill_rounded(theme.hover, x, y + (ROW - ACT) / 2.0, ACT, ACT, theme.control_radius.min(ACT / 2.0));
        }
        frame.icon(icon, Point { x: x + ACT / 2.0, y: y + ROW / 2.0 }, 14.0, if lit { theme.text } else { theme.text2 });
        hits.push((x, y + (ROW - ACT) / 2.0, ACT, ACT, hit));
    };
    if on {
        right -= ACT;
        act(frame, hits, right, remove, Icon::Close);
        right -= 2.0;
        if row.label.is_none() && !row.done {
            right -= ACT;
            act(frame, hits, right, due, Icon::Calendar);
        }
    }
    if let Some(label) = &row.label {
        let bold = Font { weight: 600.0, ..small };
        let font = if row.soon { bold } else { small };
        let color = if row.done { theme.text3 } else if row.soon { soon(theme) } else { theme.text2 };
        let wide = frame.measure(label, font) + 12.0;
        let x = right - wide;
        if scene.hover == Some(due) {
            frame.fill_rounded(theme.hover, x, y + (ROW - ACT) / 2.0, wide, ACT, 6.0);
        }
        frame.text(label, font, alpha(color), x + 6.0, frame.line_in(font, y, ROW), wide, Align::Start);
        if !row.done {
            hits.push((x, y + (ROW - ACT) / 2.0, wide, ACT, due));
        }
        right = x - 4.0;
    }
    // Its words, struck through as it is ticked; written in where they show, those written (and pressed there, the caret
    // put where they were pressed: see `ui::todo::caret_at`).
    let font = theme.body;
    let words_left = left + BOX + BOX_GAP;
    let room = (right - words_left).max(0.0);
    if !row.done && row.tick == 0.0 {
        hits.push((words_left, y, room, ROW, Hit::Todo(TodoHit::Words(n, key))));
    }
    if let Some(edit) = edit {
        paint_edit(frame, scene, (words_left, y, room), edit);
        hits.push((left, y, width, ROW, Hit::Todo(TodoHit::Row(n, key))));
        return;
    }
    let shown = fitted(frame, &row.text, font, room);
    let ink = if row.done || row.tick > 0.0 { theme.text3 } else { theme.text };
    let text_y = frame.line_in(font, y, ROW);
    frame.text(&shown, font, alpha(ink), words_left, text_y, room + 1.0, Align::Start);
    let struck = if row.done { 1.0 } else { ease_out(row.tick) };
    if struck > 0.0 {
        let wide = frame.measure(&shown, font).min(room);
        frame.fill(alpha(ink), words_left, y + ROW / 2.0 + 0.5, wide * struck, 1.2);
    }
    hits.push((left, y, width, ROW, Hit::Todo(TodoHit::Row(n, key))));
}

/// The words of an item written in where it shows, in its row's room for them at (`left`, `y`), `room` across: as its
/// words are drawn at rest, scrolled for the caret to show, what is chosen marked, the caret in the accent while it
/// shows. Where the caret is (and how far they are scrolled) kept for the input method and for presses on them.
fn paint_edit(frame: &dyn Canvas, scene: &Scene, (left, y, room): (f32, f32, f32), edit: &Editing) {
    let theme = scene.theme;
    let font = theme.body;
    let middle = y + ROW / 2.0;
    let (caret_top, caret_tall) = (middle - font.size * 0.65, font.size * 1.3);
    let before = |at: usize| frame.measure(&String::from_utf16_lossy(&edit.text[..at.min(edit.text.len())]), font);
    let caret_x = before(edit.caret);
    let shift = (caret_x - room + 2.0).max(0.0);
    frame.clip(left, y, room, ROW);
    if edit.caret != edit.anchor {
        let (from, to) = (before(edit.caret.min(edit.anchor)), before(edit.caret.max(edit.anchor)));
        frame.fill(mark(theme).0.alpha(0.3), left + from - shift, caret_top, to - from, caret_tall);
    }
    frame.text(&String::from_utf16_lossy(&edit.text), font, theme.text, left - shift, frame.line_in(font, y, ROW), room + shift + 200.0, Align::Start);
    if edit.caret_on {
        frame.fill(soon(theme), left + caret_x - shift, caret_top, 1.2, caret_tall);
    }
    frame.unclip();
    CARET.with(|caret| caret.set((caret_x - shift, caret_top - y, caret_tall, font.size)));
    EDIT_SHIFT.with(|kept| kept.set(shift));
}

/// The line of those done at (`left`, `y`), `width` across: how many, folded or open (a press on it turns it), and what
/// clears them.
fn paint_done_head(frame: &dyn Canvas, scene: &Scene, (left, y, width): (f32, f32, f32), done: usize, open: bool, hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    frame.fill(theme.rule, left, y + 2.0, width, 1.0);
    let small = theme.small;
    let says = format!("{} {done}", scene.lang.pick("已完成", "Done"));
    let font = Font { weight: 600.0, ..small };
    let lit = scene.hover == Some(Hit::Todo(TodoHit::Done));
    let text_y = frame.line_top(font, y + 2.0 + (DONE_HEAD - 2.0) / 2.0);
    frame.text(&says, font, if lit { theme.text } else { theme.text2 }, left, text_y, width, Align::Start);
    let wide = frame.measure(&says, font);
    frame.icon(if open { Icon::ChevronUp } else { Icon::ChevronDown }, Point { x: left + wide + 12.0, y: y + 2.0 + (DONE_HEAD - 2.0) / 2.0 }, 12.0, theme.text2);
    let clear = scene.lang.pick("清除", "Clear");
    let clear_w = frame.measure(clear, small) + 16.0;
    let x = left + width - clear_w;
    let clear_lit = scene.hover == Some(Hit::Todo(TodoHit::Clear));
    if clear_lit {
        frame.fill_rounded(theme.hover, x, y + 2.0 + (DONE_HEAD - 2.0 - ACT) / 2.0, clear_w, ACT, 6.0);
    }
    frame.text(clear, small, if clear_lit { theme.text } else { theme.text3 }, x + 8.0, frame.line_top(small, y + 2.0 + (DONE_HEAD - 2.0) / 2.0), clear_w, Align::Start);
    hits.push((x, y + 2.0 + (DONE_HEAD - 2.0 - ACT) / 2.0, clear_w, ACT, Hit::Todo(TodoHit::Clear)));
    hits.push((left, y, width, DONE_HEAD, Hit::Todo(TodoHit::Done)));
}

/// The to-dos for a lane in a shape: those not done, the one shown first (see `shown_first`) first, each as its row
/// says it.
pub(super) fn bench(lang: Lang) -> Bench {
    let list = crate::todo::list();
    let items = list.items();
    let now = Instant::now();
    let lits: Vec<f32> = items.iter().map(|item| crate::ui::todo::lit(item, now)).collect();
    Bench::Todo(bench_rows(&items, &lits, Day::today(), lang == Lang::Zh), None)
}

/// The rows of `items` (each as lit as `lits` has it) in a shape on `today` (see `bench`).
fn bench_rows(items: &[&Item], lits: &[f32], today: Day, zh: bool) -> Vec<BenchRow> {
    let first = shown_first(items, lits);
    let order = first.into_iter().chain((0..items.len()).filter(|n| !items[*n].done && Some(*n) != first));
    order.map(|n| (items[n].text.clone(), items[n].due.map(|due| due.label(today, zh)), items[n].due.is_some_and(|due| due.by(today)), n as u32, key_of(items, n), lits[n].to_bits())).collect()
}

/// A to-do's row in a shape (see `Bench::Todo`).
pub type BenchRow = (String, Option<String>, bool, u32, u64, u32);

/// The to-dos in a shape (`x`, `y`, `w`, `h`: within its lane's padding): as many of those not done as fit, a row each,
/// the one due soonest first (the one written in where it shows, `edit`, as it is); nothing to do, said.
pub(super) fn paint_bench(frame: &dyn Canvas, scene: &Scene, (x, y, w, h): (f32, f32, f32, f32), rows: &[BenchRow], edit: Option<&Editing>, hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    if rows.is_empty() {
        let says = scene.lang.pick("没有待办事项", "Nothing to do");
        let font = theme.small;
        frame.text(&fitted(frame, says, font, w), font, theme.text3, x, frame.line_in(font, y, h), w + 1.0, Align::Start);
        return;
    }
    let fit = ((h / ROW).floor() as usize).max(1);
    let tall = fit as f32 * ROW;
    let top = y + ((h - tall) / 2.0).max(0.0).min(if fit > 1 { 0.0 } else { h });
    frame.clip(x - 4.0, y, w + 8.0, h);
    for (at, (text, label, soon, n, key, lit)) in rows.iter().take(fit).enumerate() {
        let row = Row { n: *n, key: *key, text: text.clone(), done: false, label: label.clone(), soon: *soon, tick: 0.0, lit: f32::from_bits(*lit) };
        paint_row(frame, scene, (x, top + at as f32 * ROW, w), &row, 1.0, edit, hits);
    }
    frame.unclip();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The search's lane and a to-do field in each skin, at rest and under the pointer, over a picture of a desktop:
    /// `GLANCE_BACKDROP=<png> GLANCE_OUT=<png> cargo test --features studio --lib -- --ignored fields_pictured` draws it.
    #[test]
    #[ignore = "writes a picture to look at"]
    #[cfg(feature = "studio")]
    fn fields_pictured() {
        use crate::ui::theme::Skin;
        use windows::Win32::Graphics::Imaging::{CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnLoad};
        use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
        let _device = crate::ui::gfx::held().unwrap();
        let gfx = crate::ui::gfx::Gfx::new().expect("graphics");
        let info = crate::reading::StaticInfo { cpu_name: String::new(), memory_modules: None, memory_speed: None, drives: Vec::new(), drive_ids: Vec::new(), network_adapter: None, board: String::new(), threads: 1, mem_total: 0, gpus: Vec::new(), found: Vec::new() };
        let prefs = crate::ui::prefs::Prefs::default();
        let seen = crate::ui::seen::Seen::default();
        let path = std::env::var("GLANCE_BACKDROP").expect("GLANCE_BACKDROP");
        let (w, h) = (900.0f32, 600.0f32);
        let scale = 2.0;
        let pixels = gfx
            .draw_offscreen(((w * scale) as u32, (h * scale) as u32), scale, |frame| unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).unwrap();
                let decoder = wic.CreateDecoderFromFilename(&windows::core::HSTRING::from(path.as_str()), None, windows::Win32::Foundation::GENERIC_READ, WICDecodeMetadataCacheOnLoad).unwrap();
                let converter = wic.CreateFormatConverter().unwrap();
                converter.Initialize(&decoder.GetFrame(0).unwrap(), &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom).unwrap();
                let desktop = frame.dc.CreateBitmapFromWicBitmap(&converter, None).unwrap();
                frame.dc.DrawImage(&desktop, None, None, Default::default(), Default::default());
                for (row, (skin, dark)) in [(Skin::Glass, true), (Skin::Fluent, true), (Skin::Paper, false)].into_iter().enumerate() {
                    let theme = Theme::new(skin, dark);
                    let y0 = 20.0 + row as f32 * 195.0;
                    for (column, hover) in [None, Some(Hit::Search)].into_iter().enumerate() {
                        let scene = Scene { info: &info, prefs: &prefs, theme: &theme, lang: Lang::Zh, history: &[], seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::default(), hover, hint: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None, music_apart: false, sound_open: false };
                        let lane = Rect { x: 20.0 + column as f32 * 300.0, y: y0, w: 280.0, h: 84.0 };
                        let todo = Rect { x: 20.0 + column as f32 * 300.0, y: y0 + 92.0, w: 280.0, h: 86.0 };
                        for piece in [lane, todo] {
                            let radius = theme.radius.min(piece.h / 2.0);
                            match skin {
                                Skin::Glass => crate::ui::skins::glass_sheet(frame, &theme, piece, radius, 1.0).unwrap(),
                                Skin::Fluent => frame.fill_rounded(Color::hex(0x202020, 0.92), piece.x, piece.y, piece.w, piece.h, radius),
                                Skin::Paper => {
                                    frame.fill_rounded(Color { a: 1.0, ..theme.paper }, piece.x, piece.y, piece.w, piece.h, radius);
                                    frame.stroke_rounded(Fill::Solid(theme.stroke), piece.x, piece.y, piece.w, piece.h, radius, 1.0);
                                }
                            }
                        }
                        let mut hits = Vec::new();
                        paint_search(frame, &scene, lane, "Ctrl+Alt+S", &mut hits);
                        let pad = theme.pad_x;
                        frame.text("待办", theme.title, theme.text, todo.x + pad, frame.line_top(theme.title, todo.y + 22.0), 100.0, Align::Start);
                        let (out, round) = field_fit(&theme, theme.radius, pad);
                        let field = Field { writing: column == 1, ..Field::default() };
                        let scene = Scene { hover: None, ..scene };
                        paint_field(frame, &scene, (todo.x + pad - out, todo.y + 40.0, todo.w - 2.0 * pad + 2.0 * out), round, &field, &mut hits);
                    }
                    frame.text(&format!("{skin:?}"), theme.title, Color::hex(0xFFFFFF, 1.0), 620.0, y0 + 30.0, 200.0, Align::Start);
                }
            })
            .expect("drawn");
        crate::studio::save_png(&std::path::PathBuf::from(std::env::var("GLANCE_OUT").expect("GLANCE_OUT")), (w * scale) as u32, (h * scale) as u32, &pixels).unwrap();
    }

    /// A to-do lit, opened from its notification, shows where the lane has no room for all: past those that fit, it
    /// takes the last place of them while it is lit (it was among those the lane only counted, lit where nothing showed).
    #[test]
    fn a_to_do_lit_past_those_that_fit_shows() {
        use crate::ui::theme::Skin;
        let theme = Theme::new(Skin::Glass, true);
        let info = crate::reading::StaticInfo { cpu_name: String::new(), memory_modules: None, memory_speed: None, drives: Vec::new(), drive_ids: Vec::new(), network_adapter: None, board: String::new(), threads: 1, mem_total: 0, gpus: Vec::new(), found: Vec::new() };
        let prefs = crate::ui::prefs::Prefs::default();
        let seen = crate::ui::seen::Seen::default();
        let scene = Scene { info: &info, prefs: &prefs, theme: &theme, lang: Lang::Zh, history: &[], seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::default(), hover: None, hint: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None, music_apart: false, sound_open: false };
        let row = |n: u32, lit: f32| Row { n, key: 1000 + u64::from(n), text: format!("{n}"), done: false, label: None, soon: false, tick: 0.0, lit };
        let ticks = |lit: Option<u32>| {
            let lane = TodoLane { undone: (0..8).map(|n| row(n, if Some(n) == lit { 1.0 } else { 0.0 })).collect(), done: Vec::new(), open: false, field: None, moving: 0, edit: None };
            let mut hits = Vec::new();
            paint(&crate::ui::view::tests::Nothing, &scene, (0.0, 0.0, 300.0), 16.0, &lane, 3.0 * ROW + MORE, &mut hits);
            let mut ticked: Vec<u32> = hits.iter().filter_map(|hit| match hit.4 { Hit::Todo(TodoHit::Tick(n, _)) => Some(n), _ => None }).collect();
            ticked.sort();
            ticked
        };
        assert_eq!(ticks(None), [0, 1, 2]);
        assert_eq!(ticks(Some(6)), [0, 1, 6]);
        assert_eq!(ticks(Some(1)), [0, 1, 2]);
    }

    /// The words of a to-do not done are pressed to write in it where it shows, not those of one done. Written in, its
    /// row keeps its time but shows no buttons, the pointer on it or not, in the lane and in a shape alike.
    #[test]
    fn a_to_do_written_in_where_it_shows_puts_its_buttons_away() {
        use crate::ui::theme::Skin;
        let theme = Theme::new(Skin::Glass, true);
        let info = crate::reading::StaticInfo { cpu_name: String::new(), memory_modules: None, memory_speed: None, drives: Vec::new(), drive_ids: Vec::new(), network_adapter: None, board: String::new(), threads: 1, mem_total: 0, gpus: Vec::new(), found: Vec::new() };
        let prefs = crate::ui::prefs::Prefs::default();
        let seen = crate::ui::seen::Seen::default();
        let scene_on = |hover: Option<Hit>| Scene { info: &info, prefs: &prefs, theme: &theme, lang: Lang::Zh, history: &[], seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::default(), hover, hint: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None, music_apart: false, sound_open: false };
        let row = |n: u32, done: bool, label: Option<&str>| Row { n, key: 500 + u64::from(n), text: format!("item {n}"), done, label: label.map(Into::into), soon: false, tick: if done { 1.0 } else { 0.0 }, lit: 0.0 };
        let undone = vec![row(0, false, Some("18:00")), row(1, false, None)];
        let done = vec![row(2, true, None)];
        let editing = Editing { n: 0, key: 500, text: "item 0 written".encode_utf16().collect(), caret: 3, anchor: 3, caret_on: true };
        let hits_of = |edit: Option<Editing>, hover: Option<Hit>| {
            let lane = TodoLane { undone: undone.clone(), done: done.clone(), open: true, field: None, moving: 0, edit };
            let mut hits = Vec::new();
            paint(&crate::ui::view::tests::Nothing, &scene_on(hover), (0.0, 0.0, 300.0), 16.0, &lane, TODO_TALL, &mut hits);
            hits.into_iter().filter_map(|hit| match hit.4 { Hit::Todo(hit) => Some(hit), _ => None }).collect::<Vec<_>>()
        };
        let rest = hits_of(None, None);
        assert!(rest.contains(&TodoHit::Words(0, 500)) && rest.contains(&TodoHit::Words(1, 501)));
        assert!(!rest.contains(&TodoHit::Words(2, 502)), "one done is unticked first");
        let on = hits_of(None, Some(Hit::Todo(TodoHit::Row(0, 500))));
        assert!(on.contains(&TodoHit::Remove(0, 500)), "its button to take it out, the pointer on it");
        let written = hits_of(Some(editing.clone()), Some(Hit::Todo(TodoHit::Row(0, 500))));
        assert!(!written.contains(&TodoHit::Remove(0, 500)), "no buttons, written in");
        assert!(written.contains(&TodoHit::Due(0, 500)), "its time shown");
        assert!(written.contains(&TodoHit::Words(0, 500)), "its words pressed again move the caret");
        let other = hits_of(Some(editing.clone()), Some(Hit::Todo(TodoHit::Row(1, 501))));
        assert!(other.contains(&TodoHit::Remove(1, 501)), "another's buttons as they were");
        // In a shape.
        let scene = scene_on(Some(Hit::Todo(TodoHit::Row(0, 500))));
        let rows: Vec<BenchRow> = vec![("item 0".into(), Some("18:00".into()), false, 0, 500, 0f32.to_bits())];
        for (edit, buttons) in [(None, true), (Some(&editing), false)] {
            let mut hits = Vec::new();
            paint_bench(&crate::ui::view::tests::Nothing, &scene, (0.0, 0.0, 300.0, ROW), &rows, edit, &mut hits);
            assert_eq!(hits.iter().any(|hit| hit.4 == Hit::Todo(TodoHit::Remove(0, 500))), buttons, "in a shape, written in: {}", edit.is_some());
            assert!(hits.iter().any(|hit| hit.4 == Hit::Todo(TodoHit::Words(0, 500))));
        }
    }

    /// A to-do lit, opened from its notification, comes first in a shape too, and is drawn lit there.
    #[test]
    fn a_to_do_lit_comes_first_in_a_shape() {
        let today = Day::of(2026, 11, 21).unwrap();
        let item = |text: &str, hour: u32| Item { text: text.into(), done: false, due: Some(crate::todo::Due { day: today, time: Some((hour, 0)) }) };
        let items = [item("a", 18), item("b", 15), item("c", 20)];
        let items: Vec<&Item> = items.iter().collect();
        let order = |lits: &[f32]| bench_rows(&items, lits, today, true).into_iter().map(|row| (row.0, f32::from_bits(row.5))).collect::<Vec<_>>();
        assert_eq!(order(&[0.0, 0.0, 0.0]), [("b".to_string(), 0.0), ("a".to_string(), 0.0), ("c".to_string(), 0.0)]);
        assert_eq!(order(&[0.0, 0.0, 0.5]), [("c".to_string(), 0.5), ("a".to_string(), 0.0), ("b".to_string(), 0.0)]);
    }

    /// The field's words stand in its middle in every skin: their ink as far below its top as above its foot (they
    /// were placed by a line height guessed for each, up to two DIPs off in some skins, and off by as much in each).
    #[test]
    fn the_field_s_words_stand_in_its_middle() {
        use crate::ui::theme::Skin;
        let _device = crate::ui::gfx::held().unwrap();
        let gfx = crate::ui::gfx::Gfx::new().expect("graphics");
        let info = crate::reading::StaticInfo { cpu_name: String::new(), memory_modules: None, memory_speed: None, drives: Vec::new(), drive_ids: Vec::new(), network_adapter: None, board: String::new(), threads: 1, mem_total: 0, gpus: Vec::new(), found: Vec::new() };
        let prefs = crate::ui::prefs::Prefs::default();
        let seen = crate::ui::seen::Seen::default();
        let scale = 4.0;
        let (wide, tall) = (300.0f32, FIELD);
        for skin in [Skin::Glass, Skin::Fluent, Skin::Paper] {
            for dark in [false, true] {
                let theme = Theme::new(skin, dark);
                let scene = Scene { info: &info, prefs: &prefs, theme: &theme, lang: Lang::Zh, history: &[], seen: &seen, pen_ms: 0.0, scrolls: Scrolls::default(), tab: Tab::default(), hover: None, hint: None, thumbs: &RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None, music_apart: false, sound_open: false };
                let (w, h) = ((wide * scale) as usize, (tall * scale) as usize);
                let paint = |words: bool| {
                    gfx.draw_offscreen((w as u32, h as u32), scale, |frame| {
                        let field = Field::default();
                        let mut hits = Vec::new();
                        if words {
                            paint_field(frame, &scene, (0.0, 0.0, wide), 8.0, &field, &mut hits);
                        } else {
                            paint_field_ground(frame, &theme, (0.0, 0.0, wide), 8.0, false, false);
                        }
                    })
                    .expect("drawn")
                };
                let (with, without) = (paint(true), paint(false));
                // The rows where its words are: where, across them, it differs from its ground alone.
                let from = ((FIELD_WORDS + 2.0) * scale) as usize;
                let to = ((FIELD_WORDS + 110.0) * scale) as usize;
                let inked: Vec<usize> = (0..h).filter(|row| (from..to).any(|column| (0..4).any(|c| with[(row * w + column) * 4 + c].abs_diff(without[(row * w + column) * 4 + c]) > 24))).collect();
                let (first, last) = (*inked.first().expect("words drawn") as f32, *inked.last().unwrap() as f32 + 1.0);
                let off = ((first + last) / 2.0 / scale) - tall / 2.0;
                assert!(off.abs() <= 0.5, "{skin:?} dark {dark}: words {off:+.2} DIPs off the middle");
            }
        }
    }
}
