//! A name written in where it shows (see `ui::naming`): drawn in its own name's stead, on a field's ground, in the
//! font and at the place its name is drawn at rest, scrolled for the caret to show, what is chosen marked, the caret in
//! the accent; why what was written was refused said under it in the signal's colour.

use super::*;

/// What a name written in where it shows is the name of: a thing on the station or among what was copied (by its
/// number, on whichever of the two it shows), a file come lately, a tag (its choice over the page), what is kept at
/// hand (by its number: see `station::apps::number_of`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Named {
    Item(u64),
    /// Part of what a thing holds together (see `Hit::Part`).
    Part(u64, usize),
    Recent(crate::station::recent::Entry),
    Tag(u32),
    App(u64),
}

impl Named {
    /// Whether it is what `hit` is drawn for (a thing's line or card, where its name is drawn).
    pub fn names(self, hit: Hit) -> bool {
        match (self, hit) {
            (Named::Item(id), Hit::Shelf(of) | Hit::Copied(of)) => id == of,
            (Named::Part(id, at), Hit::Part(of, part)) => (id, at) == (of, part),
            (Named::Recent(entry), Hit::Recent(of)) => entry == of,
            (Named::App(number), Hit::App(of)) => number == of,
            _ => false,
        }
    }
}

/// A name being written where it shows, as its window draws it: what it names, what is written (UTF-16), the caret and
/// where the selection began, whether the caret shows now, and why what was written was refused (till it changes).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct NameWritten {
    pub of: Named,
    pub text: Vec<u16>,
    pub caret: usize,
    pub anchor: usize,
    pub caret_on: bool,
    pub said: Option<String>,
}

/// Where the name being written was last drawn, from its field's corner (DIPs): in what font, where its words begin
/// (as far as they are scrolled), and its caret (its left, top and height).
#[derive(Clone, Copy)]
pub struct Drawn {
    pub font: Font,
    pub words: f32,
    pub caret: (f32, f32, f32),
}

thread_local! {
    static DRAWN: std::cell::Cell<Option<Drawn>> = const { std::cell::Cell::new(None) };
}

/// Where the name being written was last drawn (see `Drawn`).
pub fn drawn() -> Option<Drawn> {
    DRAWN.with(std::cell::Cell::get)
}

/// How far a name's field stands out round its words, across and up and down.
pub(super) const FIELD_OUT: f32 = 5.0;
pub(super) const FIELD_UP: f32 = 2.0;

/// The ground a name's field is drawn on: as round as a control is, no rounder than half its height.
fn field_round(theme: &Theme, tall: f32) -> f32 {
    theme.control_radius.min(tall / 2.0)
}

/// `written` drawn in `font` in the field (x, y, w, h), its words `inset` in from its left (in its middle where
/// `centred`, while they fit): on the ground of a field where `grounded` (not where what it is on is that ground
/// already: a chip, a plate of its own), scrolled for the caret to show, what is chosen marked, the caret in the
/// accent while it shows. A press there is its own (see `Hit::Naming`), as much of it as shows in `view`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_written(frame: &dyn Canvas, theme: &Theme, written: &NameWritten, font: Font, (x, y, w, h): (f32, f32, f32, f32), inset: f32, centred: bool, grounded: bool, view: (f32, f32, f32, f32), hits: &mut Vec<HitBox>) {
    if grounded {
        frame.fill_rounded(theme.track, x, y, w, h, field_round(theme, h));
    }
    let room = (w - 2.0 * inset).max(0.0);
    let text = String::from_utf16_lossy(&written.text);
    let before = |at: usize| frame.measure(&String::from_utf16_lossy(&written.text[..at.min(written.text.len())]), font);
    let (whole, caret_x) = (before(written.text.len()), before(written.caret));
    let (start, shift) = if centred && whole <= room { (x + (w - whole) / 2.0, 0.0) } else { (x + inset, (caret_x - room + 2.0).max(0.0)) };
    let middle = y + h / 2.0;
    let (caret_top, caret_tall) = (middle - font.size * 0.65, font.size * 1.3);
    frame.clip(x, y, w, h);
    if written.caret != written.anchor {
        let (from, to) = (before(written.caret.min(written.anchor)), before(written.caret.max(written.anchor)));
        frame.fill(todo::mark(theme).0.alpha(0.3), start + from - shift, caret_top, to - from, caret_tall);
    }
    frame.text(&text, font, theme.text, start - shift, frame.line_in(font, y, h), whole + 2.0, Align::Start);
    if written.caret_on {
        frame.fill(todo::soon(theme), start + caret_x - shift, caret_top, 1.2, caret_tall);
    }
    frame.unclip();
    DRAWN.with(|drawn| drawn.set(Some(Drawn { font, words: start - shift - x, caret: (start + caret_x - shift - x, caret_top - y, caret_tall) })));
    hits.extend(shown_in((x, y, w, h, Hit::Naming), view));
}

/// Why what was written was refused (`said`), on the line from (`x`, `top`), `room` across, in the signal's colour, as
/// much of it as fits.
pub(crate) fn paint_said(frame: &dyn Canvas, theme: &Theme, said: &str, x: f32, top: f32, room: f32) {
    let font = Font { size: theme.small.size - 1.0, ..theme.small };
    frame.text(&fitted(frame, said, font, room), font, theme.signal, x, top, room + 1.0, Align::Start);
}

/// Why what was written was refused (`said`), where there is no line for it (a thing in a row, its name under its
/// picture): on a plate at the foot of its picture (`face`), as the preview's shortcut is said there.
pub(super) fn paint_said_on(frame: &dyn Canvas, theme: &Theme, said: &str, (x, y, w, h): (f32, f32, f32, f32)) {
    let colours = super::super::menu::Colours::new(super::super::menu::Look::of(theme.skin, theme.dark), false);
    let font = hint_font(theme);
    let shown = fitted(frame, said, font, (w - 2.0 * HINT_PAD).max(0.0));
    let wide = (frame.measure(&shown, font) + 2.0 * HINT_PAD).min(w);
    let (left, top) = (x + (w - wide) / 2.0, y + h - HINT_TALL - 2.0);
    frame.fill_rounded(colours.sheet, left, top, wide, HINT_TALL, mark_round(theme, &colours, HINT_TALL));
    frame.text(&shown, font, theme.signal, left + HINT_PAD, frame.line_in(font, top, HINT_TALL), wide, Align::Start);
}

/// The field a name written in a line's or a card's stead is drawn in, round its words at (`x`, `top`) (the top of their
/// line, as `frame.text` takes it), `room` across: standing out round them.
pub(super) fn field_round_words(frame: &dyn Canvas, font: Font, x: f32, top: f32, room: f32) -> (f32, f32, f32, f32) {
    let tall = (font.size * 1.3).ceil() + 2.0 * FIELD_UP;
    let middle = top - frame.line_top(font, 0.0);
    (x - FIELD_OUT, middle - tall / 2.0, room + 2.0 * FIELD_OUT, tall)
}
