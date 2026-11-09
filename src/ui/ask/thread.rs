//! A report's thread in a window of its own, as a conversation. At its top, what the report is and how it stands.
//! Below, everything said since in a column that scrolls, the newest last, the user's own on the right, the
//! developer's on the left and each change of its status between them. At its foot, room to add to it (words,
//! pictures, a fresh copy of the diagnostics), and the report closed or opened again, as an issue on GitHub is by the
//! one who opened it. It is kept up beside the window it is opened from, which takes nothing meanwhile: that window is
//! told as the user sends what they wrote, closes or opens the report again, or closes the window, and gives it the
//! thread again as it changes.

use super::*;

/// How wide it is (DIPs).
pub(super) const WIDTH: f32 = 640.0;
/// Its measures (DIPs): the room at its sides, the close button, the status chip, the box for words, a
/// picture's tile in the box and in a message, the row of buttons under the box, a message's padding, its corners,
/// the line over it, how much of the column's width it may take, the space between messages, and the room above
/// and below what was said.
const SIDE: f32 = 24.0;
const CLOSE: f32 = 32.0;
const CHIP: f32 = 24.0;
const WORDS_TALL: f32 = 64.0;
const COMPOSED_TILE: (f32, f32) = (88.0, 56.0);
const SAID_TILE: (f32, f32) = (132.0, 84.0);
const TOOLS: f32 = 32.0;
const BUBBLE_PAD: (f32, f32) = (14.0, 10.0);
const BUBBLE_RADIUS: f32 = 10.0;
const SIGNED: f32 = 20.0;
const BUBBLE_SHARE: f32 = 0.8;
const BETWEEN: f32 = 16.0;
const ENDS: f32 = 20.0;
/// An event's line, and what is said with a message besides its words (the diagnostics).
const EVENT: f32 = 26.0;
const ATTACHED: f32 = 18.0;

const STRONG_SMALL: Font = Font::new(Family::Segoe, 12.0, 600.0);

/// How a report stands, as its chip and its changes of status are coloured.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Open,
    Answered,
    Fixed,
    Closed,
}

impl Tone {
    fn icon(self) -> Icon {
        match self {
            Tone::Open => Icon::CircleDot,
            Tone::Answered => Icon::MessageCircle,
            Tone::Fixed => Icon::CircleCheck,
            Tone::Closed => Icon::CircleX,
        }
    }

    fn colour(self, c: &Colours) -> Color {
        match self {
            Tone::Open => c.text2,
            Tone::Answered => c.accent_text,
            Tone::Fixed => c.success,
            Tone::Closed => c.text3,
        }
    }
}

/// A report's thread as the window shows it: its title, what is said under it (which report, and when it was sent),
/// how it stands (in a few words, and what that means for this Glance, if anything), what was said, in order, and
/// whether the user may close it (else open it again).
#[derive(Clone, Debug, PartialEq)]
pub struct Thread {
    pub title: String,
    pub about: String,
    pub status: String,
    pub tone: Tone,
    pub note: String,
    pub items: Vec<Item>,
    pub closable: bool,
}

/// One thing said in a thread.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// The user's own: when, their words, their pictures (JPEGs), and what went with them besides (the diagnostics),
    /// if anything.
    Mine { when: String, words: String, pictures: Vec<Vec<u8>>, attached: String },
    /// The developer's: who, when, and their words.
    Theirs { who: String, when: String, words: String },
    /// A change of its status, in a few words.
    Event { tone: Tone, words: String },
}

/// What the window says besides the thread, and whether it is dark.
pub struct Talk<'a> {
    /// In the box for words while it is empty.
    pub hint: &'a str,
    /// On the button that adds a picture.
    pub add: &'a str,
    /// On the button that pastes the clipboard's picture.
    pub paste: &'a str,
    /// Pictures, as the file chooser lists them.
    pub kinds: &'a str,
    /// Beside the choice to attach the diagnostics.
    pub option: &'a str,
    /// On the button that sends.
    pub send: &'a str,
    /// On the button that closes the report, and on the one that opens it again.
    pub close_report: &'a str,
    pub reopen: &'a str,
    /// Of closing it (the close button, as it is named).
    pub close: &'a str,
    /// The user, as their own messages are signed.
    pub you: &'a str,
    pub dark: bool,
}

/// One thing said, as the window holds it: its pictures read, each with a likeness the size of its tile.
enum Shown {
    Mine { when: String, words: String, pictures: Vec<Picture>, attached: String },
    Theirs { who: String, when: String, words: String },
    Event { tone: Tone, words: String },
}

/// What is being done with what the user wrote: nothing yet, being sent (what is said meanwhile), or not sent (why).
#[derive(Clone, PartialEq, Debug)]
enum State {
    Writing,
    Sending(String),
    Failed(String),
}

/// A thread, as the window shows it while it is up: the window it is kept beside and what that window is told (see
/// `open`), the thread's heading and what was said, the user as their messages are signed, what is being done with
/// what they wrote, and how far down what was said starts (the heading's height, as last drawn).
pub(super) struct Conversation {
    owner: HWND,
    told: u32,
    about: String,
    status: String,
    tone: Tone,
    note: String,
    items: Vec<Shown>,
    you: String,
    /// Whether the report may be closed (else opened again), and the words on the button that does either.
    closable: bool,
    turns: (String, String),
    state: State,
    pub(super) header: f32,
    /// Shown till it is taken down (see `life`).
    _showing: super::super::life::Showing,
}

/// `items` as the window holds them, their pictures read with likenesses for tiles at `scale`.
fn shown(items: Vec<Item>, scale: f32) -> Vec<Shown> {
    let small = ((SAID_TILE.0 * scale).ceil() as u32, (SAID_TILE.1 * scale).ceil() as u32);
    items
        .into_iter()
        .map(|item| match item {
            Item::Mine { when, words, pictures, attached } => Shown::Mine { when, words, pictures: pictures.iter().filter_map(|jpeg| picture::from_jpeg(jpeg, small)).collect(), attached },
            Item::Theirs { who, when, words } => Shown::Theirs { who, when, words },
            Item::Event { tone, words } => Shown::Event { tone, words },
        })
        .collect()
}

/// How wide `text` is laid out in lines no wider than `most` (DIPs): its widest line's width, or `most`.
fn natural_width(gfx: &Gfx, text: &str, font: Font, most: f32) -> f32 {
    text.split('\n').map(|line| gfx.measure(line, font).ceil() + 1.0).fold(0.0, f32::max).min(most)
}

/// Where one message's parts go (DIPs, from its bubble's corner): its bubble's size, its words' width, and where its
/// pictures and what went with it start.
struct Laid {
    size: (f32, f32),
    words: f32,
    pictures: f32,
    attached: f32,
}

/// A message of `words`, `pictures` many pictures, and what went with it (`attached`), laid out in a bubble no wider
/// than `most`.
fn laid(gfx: &Gfx, words: &str, pictures: usize, attached: &str, most: f32) -> Laid {
    let inner_most = most - 2.0 * BUBBLE_PAD.0;
    let row = if pictures == 0 { 0.0 } else { pictures as f32 * (SAID_TILE.0 + TILE_GAP) - TILE_GAP };
    let attached_wide = if attached.is_empty() { 0.0 } else { 20.0 + gfx.measure(attached, SMALL).ceil() };
    let wide = natural_width(gfx, words, BODY, inner_most).max(row).max(attached_wide).min(inner_most).max(24.0);
    let mut y = BUBBLE_PAD.1;
    if !words.is_empty() {
        y += gfx.wrapped_height(words, BODY, wide);
    }
    let at_pictures = if pictures == 0 {
        y
    } else {
        if !words.is_empty() {
            y += 8.0;
        }
        let at = y;
        y += SAID_TILE.1;
        at
    };
    let at_attached = if attached.is_empty() {
        y
    } else {
        if !words.is_empty() || pictures > 0 {
            y += 6.0;
        }
        let at = y;
        y += ATTACHED;
        at
    };
    Laid { size: (wide + 2.0 * BUBBLE_PAD.0, y + BUBBLE_PAD.1), words: wide, pictures: at_pictures, attached: at_attached }
}

impl Dialog {
    /// The window's owner told what the user did: sent what they wrote (`send`), or closed it.
    pub(super) fn tell(&self, send: bool) {
        let Some(conversation) = &self.conversation else { return };
        if send && !self.sendable() {
            return;
        }
        unsafe {
            let _ = PostMessageW(Some(conversation.owner), conversation.told, WPARAM(usize::from(send)), LPARAM(self.hwnd.0 as isize));
        }
    }

    /// The window's owner told the user closes the report, or opens it again: as it stands, unless something is being
    /// sent.
    pub(super) fn tell_turn(&self) {
        let Some(conversation) = &self.conversation else { return };
        if matches!(conversation.state, State::Sending(_)) {
            return;
        }
        unsafe {
            let _ = PostMessageW(Some(conversation.owner), conversation.told, WPARAM(2), LPARAM(self.hwnd.0 as isize));
        }
    }

    /// Whether what the user wrote stands as it is, being sent (or the report closed or opened again): what is sent is
    /// what is taken away once it is, nothing written meanwhile.
    pub(super) fn written_fixed(&self) -> bool {
        self.conversation.as_ref().is_some_and(|conversation| matches!(conversation.state, State::Sending(_)))
    }

    /// Whether what the user wrote may be sent: there is something to send, and nothing is being sent.
    fn sendable(&self) -> bool {
        self.conversation.as_ref().is_some_and(|conversation| self.sendable_while(&conversation.state))
    }

    /// Whether what the user wrote may be sent, as what was written before stands (`state`).
    fn sendable_while(&self, state: &State) -> bool {
        let option = self.option.as_ref().is_some_and(|(_, ticked)| *ticked);
        !matches!(state, State::Sending(_)) && (!self.typing.string().trim().is_empty() || !self.pictures.is_empty() || option)
    }

    /// What the keyboard can go to in a thread's window, in order: what the user writes, closing the report or opening
    /// it again, sending it, what was said, and closing the window.
    pub(super) fn thread_stops(&self) -> Vec<Hit> {
        let mut stops = vec![Hit::Words];
        stops.extend((0..self.pictures.len()).map(Hit::Remove));
        if self.pictures.len() < PICTURES_MOST {
            stops.push(Hit::Add);
            if self.chip_shown {
                stops.push(Hit::Paste);
            }
        }
        stops.extend([Hit::Option, Hit::Turn, Hit::Yes, Hit::Sent, Hit::No]);
        stops
    }

    /// The thread drawn whole, `width` × `height`: its heading, what was said, and the room to add to it.
    pub(super) fn paint_thread(&mut self, frame: &Frame, (width, height): (f32, f32)) {
        let Some(mut conversation) = self.conversation.take() else { return };
        let colours = Colours::new(self.dark);
        let c = &colours;
        let inner = width - 2.0 * SIDE;
        frame.fill(c.content, 0.0, 0.0, width, height);

        // Its heading: the title with the close button beside it, then how it stands, which report it is and when it
        // was sent, and what that means for this Glance.
        let close = Rect { x: width - 12.0 - CLOSE, y: 12.0, w: CLOSE, h: CLOSE };
        self.close_button(frame, c, close);
        let title = self.title.clone();
        frame.text(&title, TITLE_FONT, c.text, SIDE, 18.0, close.x - 8.0 - SIDE, Align::Start);
        let row = 56.0;
        let chip = self.chip(frame, c, SIDE, row, &conversation.status, conversation.tone);
        let (ascent, descent) = frame.gfx.baseline(SMALL);
        frame.text(&conversation.about, SMALL, c.text2, SIDE + chip + 10.0, row + (CHIP - ascent - descent) / 2.0, inner - chip - 10.0, Align::Start);
        let mut header = row + CHIP + 14.0;
        if !conversation.note.is_empty() {
            frame.text_wrapped(&conversation.note, SMALL, c.text2, SIDE, header - 4.0, inner);
            header += frame.gfx.wrapped_height(&conversation.note, SMALL, inner) + 8.0;
        }
        conversation.header = header;
        frame.fill(c.field_stroke, 0.0, header, width, 1.0);

        // The room to add to it, from the foot up: the box for words, the pictures added, and a row of buttons.
        let composed = if self.pictures.is_empty() { 0.0 } else { COMPOSED_TILE.1 + 10.0 };
        let foot = height - (16.0 + WORDS_TALL + 10.0 + composed + TOOLS + 16.0);
        frame.fill(c.field_stroke, 0.0, foot, width, 1.0);

        // What was said, between them.
        self.paint_said(frame, c, &conversation, Rect { x: 0.0, y: header + 1.0, w: width, h: foot - header - 1.0 });

        let mut y = foot + 16.0;
        let hint = self.own.as_ref().map(|own| own.hint.clone()).unwrap_or_default();
        self.paint_words(frame, c, Rect { x: SIDE, y, w: inner, h: WORDS_TALL }, &hint);
        y += WORDS_TALL + 10.0;
        if !self.pictures.is_empty() {
            self.paint_tiles(frame, c, SIDE, y, COMPOSED_TILE);
            y += COMPOSED_TILE.1 + 10.0;
        }
        self.paint_tools(frame, c, &conversation, Rect { x: SIDE, y, w: inner, h: TOOLS });
        self.conversation = Some(conversation);
    }

    /// The button that closes it: its icon, on a light ground while pointed at.
    fn close_button(&mut self, frame: &Frame, c: &Colours, r: Rect) {
        let hover = self.hovered() == Some(Hit::No);
        if hover {
            frame.fill_rounded(if self.pressed == Some(Hit::No) { c.shade } else { c.subtle }, r.x, r.y, r.w, r.h, 6.0);
        }
        frame.icon(Icon::Close, Point { x: r.x + r.w / 2.0, y: r.y + r.h / 2.0 }, 16.0, c.text2);
        self.ring(frame, c, r, 6.0, Hit::No);
        self.drawn.push((r, Hit::No));
    }

    /// How the report stands, in a chip at `(x, y)` tinted as it stands: how wide it is.
    fn chip(&self, frame: &Frame, c: &Colours, x: f32, y: f32, words: &str, tone: Tone) -> f32 {
        let colour = tone.colour(c);
        let wide = 10.0 + 14.0 + 6.0 + frame.gfx.measure(words, STRONG_SMALL).ceil() + 12.0;
        frame.fill_rounded(colour.alpha(0.14), x, y, wide, CHIP, CHIP / 2.0);
        frame.icon(tone.icon(), Point { x: x + 17.0, y: y + CHIP / 2.0 }, 14.0, colour);
        let (ascent, descent) = frame.gfx.baseline(STRONG_SMALL);
        frame.text(words, STRONG_SMALL, colour, x + 30.0, y + (CHIP - ascent - descent) / 2.0, wide - 30.0, Align::Start);
        wide
    }

    /// Everything said, in `r`, scrolled as far as it is (from its end, when it opens), with its scroll bar.
    fn paint_said(&mut self, frame: &Frame, c: &Colours, conversation: &Conversation, r: Rect) {
        frame.fill(c.footer, r.x, r.y, r.w, r.h);
        let column = Rect { x: r.x + SIDE, y: r.y, w: r.w - 2.0 * SIDE, h: r.h };
        let most = (column.w * BUBBLE_SHARE).floor();
        // Each one's height, then all of them, to know how far they scroll.
        let heights: Vec<f32> = conversation
            .items
            .iter()
            .map(|item| match item {
                Shown::Mine { words, pictures, attached, .. } => SIGNED + laid(frame.gfx, words, pictures.len(), attached, most).size.1,
                Shown::Theirs { words, .. } => SIGNED + laid(frame.gfx, words, 0, "", most).size.1,
                Shown::Event { .. } => EVENT,
            })
            .collect();
        let whole = 2.0 * ENDS + heights.iter().sum::<f32>() + BETWEEN * heights.len().saturating_sub(1) as f32;
        self.scroll_most = (whole - r.h).max(0.0);
        self.scroll = self.scroll.clamp(0.0, self.scroll_most);
        frame.clip(r.x, r.y, r.w, r.h);
        let mut y = r.y + ENDS - self.scroll;
        for (item, height) in conversation.items.iter().zip(&heights) {
            // (Only what can be seen is drawn.)
            if y + height >= r.y && y <= r.y + r.h {
                self.paint_item(frame, c, conversation, item, column, y, most);
            }
            y += height + BETWEEN;
        }
        frame.unclip();
        if self.keyboard && self.focus == Hit::Sent {
            frame.stroke_rounded(Fill::Solid(c.ring), r.x + 2.0, r.y + 2.0, r.w - 4.0, r.h - 4.0, 4.0, 2.0);
        }
        self.drawn.push((r, Hit::Sent));
        if self.scroll_most > 0.0 {
            let inside = (r.y + 6.0, r.h - 12.0);
            let thumb = (inside.1 * r.h / whole).max(24.0);
            let at = inside.0 + (inside.1 - thumb) * self.scroll / self.scroll_most;
            self.track = (inside.0, inside.1, thumb);
            let wide = matches!(self.hovered(), Some(Hit::Thumb)) || self.held.is_some();
            let across = if wide { 6.0 } else { 3.0 };
            frame.fill_rounded(c.scroll, r.x + r.w - 5.0 - across, at, across, thumb, across / 2.0);
            self.drawn.push((Rect { x: r.x + r.w - 16.0, y: at, w: 16.0, h: thumb }, Hit::Thumb));
        } else {
            self.track = (r.y, r.h, r.h);
        }
    }

    /// One thing said, its top at `y` in `column`: a message in its bubble (the user's own on the right, tinted, the
    /// developer's on the left), signed above it, or a change of status in the middle.
    #[allow(clippy::too_many_arguments)]
    fn paint_item(&self, frame: &Frame, c: &Colours, conversation: &Conversation, item: &Shown, column: Rect, y: f32, most: f32) {
        let (ascent, descent) = frame.gfx.baseline(SMALL);
        let signed = y + (SIGNED - 4.0 - ascent - descent);
        match item {
            Shown::Event { tone, words } => {
                let wide = (14.0 + 6.0 + frame.gfx.measure(words, SMALL).ceil() + 24.0).min(column.w);
                let x = column.x + (column.w - wide) / 2.0;
                frame.fill_rounded(c.subtle, x, y, wide, EVENT, EVENT / 2.0);
                frame.icon(tone.icon(), Point { x: x + 19.0, y: y + EVENT / 2.0 }, 14.0, tone.colour(c));
                frame.text(words, SMALL, c.text2, x + 32.0, y + (EVENT - ascent - descent) / 2.0, wide - 44.0, Align::Start);
            }
            Shown::Theirs { who, when, words } => {
                let name = frame.gfx.measure(who, STRONG_SMALL).ceil();
                frame.text(who, STRONG_SMALL, c.text2, column.x + 2.0, signed, name + 1.0, Align::Start);
                frame.text(&format!(" · {when}"), SMALL, c.text3, column.x + 2.0 + name, signed, column.w - name, Align::Start);
                let laid = laid(frame.gfx, words, 0, "", most);
                let bubble = Rect { x: column.x, y: y + SIGNED, w: laid.size.0, h: laid.size.1 };
                frame.fill_rounded(c.content, bubble.x, bubble.y, bubble.w, bubble.h, BUBBLE_RADIUS);
                frame.stroke_rounded(Fill::Solid(c.button_stroke), bubble.x, bubble.y, bubble.w, bubble.h, BUBBLE_RADIUS, 1.0);
                frame.text_wrapped(words, BODY, c.text, bubble.x + BUBBLE_PAD.0, bubble.y + BUBBLE_PAD.1, laid.words);
            }
            Shown::Mine { when, words, pictures, attached } => {
                frame.text(&format!("{} · {when}", conversation.you), SMALL, c.text3, column.x, signed, column.w - 2.0, Align::End);
                let laid = laid(frame.gfx, words, pictures.len(), attached, most);
                let bubble = Rect { x: column.x + column.w - laid.size.0, y: y + SIGNED, w: laid.size.0, h: laid.size.1 };
                frame.fill_rounded(c.content, bubble.x, bubble.y, bubble.w, bubble.h, BUBBLE_RADIUS);
                frame.fill_rounded(c.mine, bubble.x, bubble.y, bubble.w, bubble.h, BUBBLE_RADIUS);
                let x = bubble.x + BUBBLE_PAD.0;
                if !words.is_empty() {
                    frame.text_wrapped(words, BODY, c.text, x, bubble.y + BUBBLE_PAD.1, laid.words);
                }
                for (index, picture) in pictures.iter().enumerate() {
                    let tile = Rect { x: x + index as f32 * (SAID_TILE.0 + TILE_GAP), y: bubble.y + laid.pictures, w: SAID_TILE.0, h: SAID_TILE.1 };
                    frame.fill_rounded(c.sent, tile.x, tile.y, tile.w, tile.h, 6.0);
                    covering(frame, picture, tile, 6.0, self.scale);
                    frame.stroke_rounded(Fill::Solid(c.field_stroke), tile.x, tile.y, tile.w, tile.h, 6.0, 1.0);
                }
                if !attached.is_empty() {
                    let middle = bubble.y + laid.attached + ATTACHED / 2.0;
                    frame.icon(Icon::Paperclip, Point { x: x + 7.0, y: middle }, 14.0, c.text2);
                    frame.text(attached, SMALL, c.text2, x + 20.0, middle - (ascent + descent) / 2.0, laid.words - 20.0, Align::Start);
                }
            }
        }
    }

    /// The row of buttons under the box for words: adding a picture (and pasting the clipboard's, where there is
    /// room), attaching the diagnostics, how the sending goes, closing the report (or opening it again), and sending.
    fn paint_tools(&mut self, frame: &Frame, c: &Colours, conversation: &Conversation, r: Rect) {
        let send = self.yes.clone();
        let send_wide = (frame.gfx.measure(&send, BODY).ceil() + 56.0).max(96.0);
        let send_r = Rect { x: r.x + r.w - send_wide, y: r.y, w: send_wide, h: r.h };
        // (The conversation is held apart while it is drawn: its state is asked of it as given.)
        let enabled = self.sendable_while(&conversation.state);
        self.send_button(frame, c, send_r, &send, enabled);
        let (turn, icon) = if conversation.closable { (&conversation.turns.0, Icon::CircleX) } else { (&conversation.turns.1, Icon::CircleDot) };
        let turn_wide = frame.gfx.measure(turn, BODY).ceil() + 46.0;
        let turn_r = Rect { x: send_r.x - 8.0 - turn_wide, y: r.y, w: turn_wide, h: r.h };
        let turnable = !matches!(conversation.state, State::Sending(_));
        self.tool_while(frame, c, turn_r, icon, turn, Hit::Turn, turnable);
        // (What is left of the row, before them.)
        let end = turn_r.x;
        let (said, colour) = match &conversation.state {
            State::Writing => (String::new(), c.text2),
            State::Sending(said) => (said.clone(), c.text2),
            State::Failed(said) => (said.clone(), c.danger),
        };
        let said_wide = if said.is_empty() { 0.0 } else { frame.gfx.measure(&said, SMALL).ceil() + 12.0 };
        let mut x = r.x;
        self.chip_shown = false;
        if self.pictures.len() < PICTURES_MOST {
            let (add, paste) = self.own.as_ref().map(|own| (own.add.clone(), own.paste.clone())).unwrap_or_default();
            let wide = frame.gfx.measure(&add, BODY).ceil() + 46.0;
            self.tool(frame, c, Rect { x, y: r.y, w: wide, h: r.h }, Icon::ImagePlus, &add, Hit::Add);
            x += wide + 8.0;
            let chip = frame.gfx.measure(&paste, BODY).ceil() + 28.0;
            let option = self.option.as_ref().map_or(0.0, |(words, _)| OPTION_BOX + 10.0 + frame.gfx.measure(words, BODY).ceil() + 16.0);
            if picture::on_clipboard() && x + chip + 8.0 + option + said_wide <= end - 12.0 {
                self.chip_shown = true;
                self.button(frame, c, Rect { x, y: r.y, w: chip, h: r.h }, &paste, Hit::Paste, false, true);
                x += chip + 8.0;
            }
        }
        x += 6.0;
        if let Some((words, ticked)) = self.option.clone() {
            let wide = OPTION_BOX + 10.0 + frame.gfx.measure(&words, BODY).ceil();
            self.paint_option(frame, c, Rect { x, y: r.y + (r.h - OPTION) / 2.0, w: wide.min(end - 12.0 - x), h: OPTION }, &words, ticked);
            x += wide + 16.0;
        }
        if !said.is_empty() {
            let (ascent, descent) = frame.gfx.baseline(SMALL);
            let room = (end - 12.0 - x).max(0.0);
            frame.text(&said, SMALL, colour, end - 12.0 - room, r.y + (r.h - ascent - descent) / 2.0, room, Align::End);
        }
    }

    /// A button of an icon and words, plain.
    fn tool(&mut self, frame: &Frame, c: &Colours, r: Rect, icon: Icon, words: &str, hit: Hit) {
        self.tool_while(frame, c, r, icon, words, hit, true);
    }

    /// A button of an icon and words, plain, greyed and not to be pressed unless `enabled`.
    #[allow(clippy::too_many_arguments)]
    fn tool_while(&mut self, frame: &Frame, c: &Colours, r: Rect, icon: Icon, words: &str, hit: Hit, enabled: bool) {
        let hover = enabled && self.hovered() == Some(hit);
        let down = hover && self.pressed == Some(hit);
        frame.fill_rounded(if hover { c.button_hover } else { c.button }, r.x, r.y, r.w, r.h, 6.0);
        frame.stroke_rounded(Fill::Solid(c.button_stroke), r.x, r.y, r.w, r.h, 6.0, 1.0);
        let (icon_ink, ink) = match (enabled, hover, down) {
            (false, ..) => (c.text3, c.text3),
            (true, true, true) => (c.accent_text, c.text2),
            (true, true, false) => (c.accent_text, c.text),
            (true, false, _) => (c.text2, c.text),
        };
        frame.icon(icon, Point { x: r.x + 20.0, y: r.y + r.h / 2.0 }, 16.0, icon_ink);
        let (ascent, descent) = frame.gfx.baseline(BODY);
        frame.text(words, BODY, ink, r.x + 34.0, r.y + (r.h - ascent - descent) / 2.0, r.w - 40.0, Align::Start);
        self.ring(frame, c, r, 6.0, hit);
        if enabled {
            self.drawn.push((r, hit));
        }
    }

    /// The button that sends, in the accent with its icon (greyed while there is nothing to send, or it is being sent).
    fn send_button(&mut self, frame: &Frame, c: &Colours, r: Rect, words: &str, enabled: bool) {
        let hover = enabled && self.hovered() == Some(Hit::Yes);
        let down = hover && self.pressed == Some(Hit::Yes);
        let ink = if enabled {
            frame.fill_rounded(c.accent.alpha(if down { 0.8 } else if hover { 0.9 } else { 1.0 }), r.x, r.y, r.w, r.h, 6.0);
            Color::hex(0xFFFFFF, 1.0)
        } else {
            frame.fill_rounded(c.subtle, r.x, r.y, r.w, r.h, 6.0);
            c.text3
        };
        let wide = 16.0 + 8.0 + frame.gfx.measure(words, BODY).ceil();
        let x = r.x + (r.w - wide) / 2.0;
        frame.icon(Icon::Send, Point { x: x + 8.0, y: r.y + r.h / 2.0 }, 15.0, ink);
        let (ascent, descent) = frame.gfx.baseline(BODY);
        frame.text(words, BODY, ink, x + 24.0, r.y + (r.h - ascent - descent) / 2.0, wide - 23.0, Align::Start);
        self.ring(frame, c, r, 6.0, Hit::Yes);
        if enabled {
            self.drawn.push((r, Hit::Yes));
        }
    }
}

/// Opens `thread` in a window over `owner` (`talk` says what it says besides), kept up beside it till it is taken
/// down (see `close`), `owner` taking nothing meanwhile. `owner` is told `told`, its WPARAM 1 as the user sends what
/// they wrote (see `said`), 2 as they close the report or open it again (as `thread` says it may be), and 0 as they
/// close the window, its LPARAM the window. The window, if it was made.
pub fn open(owner: HWND, told: u32, thread: Thread, talk: &Talk) -> Option<HWND> {
    let gfx = super::device_over(owner).ok()?;
    let showing = super::super::life::Showing::of(&gfx);
    let own = OwnText {
        words: String::new(),
        hint: talk.hint.into(),
        pictures: String::new(),
        add: talk.add.into(),
        ways: String::new(),
        paste: talk.paste.into(),
        waiting: String::new(),
        kinds: talk.kinds.into(),
    };
    let mut dialog = Dialog::new(gfx, talk.dark, &thread.title, "", Some(own));
    dialog.option = Some((talk.option.into(), false));
    dialog.yes = talk.send.into();
    dialog.no = talk.close.into();
    // (From its end: the newest, once it is laid out.)
    dialog.scroll = f32::MAX;
    let Thread { about, status, tone, note, items, closable, .. } = thread;
    let turns = (talk.close_report.into(), talk.reopen.into());
    dialog.conversation = Some(Conversation { owner, told, about, status, tone, note, items: Vec::new(), you: talk.you.into(), closable, turns, state: State::Writing, header: CLOSE + 24.0, _showing: showing });
    unsafe {
        put_up(owner, dialog, |dialog| {
            let scale = dialog.scale;
            if let Some(conversation) = &mut dialog.conversation {
                conversation.items = shown(items, scale);
            }
        })
    }
}

/// The thread in `window` given again, as it is now: kept at its newest if it was there.
pub fn update(window: HWND, thread: Thread) {
    with(window, |dialog| {
        let at_end = dialog.scroll >= dialog.scroll_most - 1.0;
        let scale = dialog.scale;
        dialog.title = thread.title;
        if let Some(conversation) = &mut dialog.conversation {
            conversation.about = thread.about;
            conversation.status = thread.status;
            conversation.tone = thread.tone;
            conversation.note = thread.note;
            conversation.items = shown(thread.items, scale);
            conversation.closable = thread.closable;
        }
        if at_end {
            dialog.scroll = f32::MAX;
        }
        dialog.redraw();
    });
}

/// What the user wrote in `window`, to be sent: none if there is nothing, or it is being sent already.
pub fn said(window: HWND) -> Option<Said> {
    with(window, |dialog| {
        dialog.sendable().then(|| Said {
            words: dialog.typing.string().trim().to_string(),
            pictures: dialog.pictures.iter().map(|picture| picture.jpeg.clone()).collect(),
            option: dialog.option.as_ref().is_some_and(|(_, ticked)| *ticked),
        })
    })
    .flatten()
}

/// What the user wrote in `window` shown as being sent (or the report being closed or opened again), `doing` said of
/// it meanwhile: nothing else is sent till it is over (see `sent`, `failed`, `writing`).
pub fn sending(window: HWND, doing: &str) {
    set_state(window, State::Sending(doing.into()));
}

/// What the user wrote in `window` sent: the room to write in empty again.
pub fn sent(window: HWND) {
    with(window, |dialog| {
        dialog.typing = Typing::most(WORDS_MOST);
        dialog.pictures.clear();
        if let Some((_, ticked)) = &mut dialog.option {
            *ticked = false;
        }
    });
    set_state(window, State::Writing);
}

/// What the user wrote in `window` not sent (`why`), kept to be sent again.
pub fn failed(window: HWND, why: &str) {
    set_state(window, State::Failed(why.into()));
}

/// What the user wrote in `window` not sent, as they chose: kept as it was.
pub fn writing(window: HWND) {
    set_state(window, State::Writing);
}

fn set_state(window: HWND, state: State) {
    with(window, |dialog| {
        if let Some(conversation) = &mut dialog.conversation {
            conversation.state = state;
        }
        dialog.redraw();
    });
}

/// The window `window` taken down, its owner given back what it takes.
pub fn close(window: HWND) {
    let owner = with(window, |dialog| dialog.conversation.as_ref().map(|conversation| conversation.owner)).flatten();
    if let Some(owner) = owner {
        take_down(window, owner);
    }
}

/// `thread` as its window draws it, `talk` saying what it says, at `scale`, scrolled to its newest, on no window: its
/// size (pixels) and its pixels, to be pictured.
#[cfg(all(test, feature = "studio"))]
pub fn pictured(gfx: &Rc<Gfx>, thread: Thread, talk: &Talk, scale: f32) -> ((u32, u32), Vec<u8>) {
    let own = OwnText {
        words: String::new(),
        hint: talk.hint.into(),
        pictures: String::new(),
        add: talk.add.into(),
        ways: String::new(),
        paste: talk.paste.into(),
        waiting: String::new(),
        kinds: talk.kinds.into(),
    };
    let mut dialog = Dialog::new(gfx.clone(), talk.dark, &thread.title, "", Some(own));
    dialog.scale = scale;
    dialog.option = Some((talk.option.into(), false));
    dialog.yes = talk.send.into();
    dialog.no = talk.close.into();
    dialog.scroll = f32::MAX;
    let Thread { about, status, tone, note, items, closable, .. } = thread;
    let items = shown(items, scale);
    let turns = (talk.close_report.into(), talk.reopen.into());
    dialog.conversation = Some(Conversation { owner: HWND::default(), told: 0, about, status, tone, note, items, you: talk.you.into(), closable, turns, state: State::Writing, header: 0.0, _showing: super::super::life::Showing::of(gfx) });
    let area = (WIDTH, TALL);
    let size = ((area.0 * scale).round() as u32, (area.1 * scale).round() as u32);
    // (Laid out once, to know how far it scrolls, then drawn at its end.)
    let mut pixels = Vec::new();
    for _ in 0..2 {
        pixels = gfx.draw_offscreen(size, scale, |frame| dialog.paint(frame, area)).expect("drawn");
    }
    (size, pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the user wrote stands while it is sent: what is taken away once it is sent is what was sent, nothing
    /// written meanwhile.
    #[test]
    fn what_is_written_stands_while_it_is_sent() {
        let _device = crate::ui::gfx::held().unwrap();
        let gfx = Rc::new(Gfx::new().expect("graphics"));
        let own = OwnText { words: String::new(), hint: String::new(), pictures: String::new(), add: String::new(), ways: String::new(), paste: String::new(), waiting: String::new(), kinds: String::new() };
        let mut dialog = Dialog::new(gfx.clone(), false, "", "", Some(own));
        dialog.option = Some((String::new(), false));
        let state = State::Writing;
        let turns = (String::new(), String::new());
        dialog.conversation = Some(Conversation { owner: HWND::default(), told: 0, about: String::new(), status: String::new(), tone: Tone::Open, note: String::new(), items: Vec::new(), you: String::new(), closable: true, turns, state, header: 0.0, _showing: crate::ui::life::Showing::of(&gfx) });
        dialog.focus = Hit::Words;
        let write = |dialog: &mut Dialog, text: &str| text.encode_utf16().for_each(|unit| dialog.char(unit));
        write(&mut dialog, "ab");
        let set = |dialog: &mut Dialog, state: State| dialog.conversation.as_mut().unwrap().state = state;
        set(&mut dialog, State::Sending(String::new()));
        write(&mut dialog, "c");
        dialog.type_key(VK_BACK, false, false);
        dialog.type_key(VK_RETURN, false, false);
        dialog.activate(Hit::Option);
        assert_eq!((dialog.typing.string(), dialog.option.as_ref().unwrap().1), ("ab".to_string(), false));
        // The caret still goes where it is sent.
        dialog.type_key(VK_LEFT, false, false);
        assert_eq!(dialog.typing.caret, 1);
        set(&mut dialog, State::Writing);
        write(&mut dialog, "c");
        dialog.activate(Hit::Option);
        assert_eq!((dialog.typing.string(), dialog.option.as_ref().unwrap().1), ("acb".to_string(), true));
    }
}
