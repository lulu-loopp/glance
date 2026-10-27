//! What the panel is drawn with, on any platform: colours, fonts, and the
//! canvas each platform paints them on (Direct2D on Windows).

/// A colour as a CSS-style straight (not premultiplied) RGBA.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const CLEAR: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

    pub const fn hex(rgb: u32, a: f32) -> Self {
        Color {
            r: ((rgb >> 16) & 0xFF) as f32 / 255.0,
            g: ((rgb >> 8) & 0xFF) as f32 / 255.0,
            b: (rgb & 0xFF) as f32 / 255.0,
            a,
        }
    }

    pub fn alpha(self, a: f32) -> Self {
        Color { a: self.a * a, ..self }
    }

}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    Archivo,
    Inter,
    Segoe,
    SegoeDisplay,
    /// Microsoft YaHei UI (the system's, Chinese with Latin): the music full screen's lines, till the serif face
    /// they are drawn in is had.
    YaHei,
    /// Noto Serif SC Black: the music full screen's lines, once taken from Glance's site (see `serif`). Asked for
    /// only once it is had.
    Serif,
}

/// A text style: face, size in DIPs, weight (100–900), width (% of normal)
/// and letter spacing (em).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Font {
    pub family: Family,
    pub size: f32,
    pub weight: f32,
    pub width: f32,
    pub tracking: f32,
}

impl Font {
    pub const fn new(family: Family, size: f32, weight: f32) -> Self {
        Font { family, size, weight, width: 100.0, tracking: 0.0 }
    }

    pub const fn width(self, width: f32) -> Self {
        Font { width, ..self }
    }

    pub const fn tracking(self, tracking: f32) -> Self {
        Font { tracking, ..self }
    }

    pub(super) fn key(&self) -> FontKey {
        (self.family, self.size.to_bits(), self.weight.to_bits(), self.width.to_bits(), self.tracking.to_bits())
    }
}

pub(super) type FontKey = (Family, u32, u32, u32, u32);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Start,
    End,
}

/// A point, in DIPs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// How a shape is filled: one colour, or a colour at `top` fading to another
/// at `bottom` (both heights in DIPs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fill {
    Solid(Color),
    Down { top: f32, from: Color, bottom: f32, to: Color },
}

/// The part of `whole` (a box already on whole pixels, as `on_pixels` puts it) that `fraction` of it fills, from its
/// bottom (`up`) or its left: as many whole pixels as that comes to (as `on_pixels` makes it), ending where the box
/// does at the side it fills from (a part put on pixels alone ended a pixel off the box's own edge, and the box's
/// colour showed there). None of it for a fraction of nothing (or not a number).
pub fn filled_part(whole: (f32, f32, f32, f32), fraction: f32, up: bool, on_pixels: impl Fn(f32, f32, f32, f32) -> (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    let (x, y, w, h) = whole;
    let fraction = if fraction.is_nan() { 0.0 } else { fraction.clamp(0.0, 1.0) };
    if up {
        let filled = if fraction > 0.0 { on_pixels(x, y, w, h * fraction).3.min(h) } else { 0.0 };
        (x, y + h - filled, w, filled)
    } else {
        let filled = if fraction > 0.0 { on_pixels(x, y, w * fraction, h).2.min(w) } else { 0.0 };
        (x, y, filled, h)
    }
}

/// A picture to draw (a track's cover, a file's thumbnail): its pixels, premultiplied BGRA rows, `size` (width,
/// height); known by `key` (the same pixels, the same key: what is made of them on the card is made once).
#[derive(Clone, Debug)]
pub struct Picture {
    pub key: u64,
    pub size: (u32, u32),
    pub pixels: std::sync::Arc<Vec<u8>>,
}

impl Picture {
    /// The box of its shape as large as fits in the box `w` × `h` from (`x`, `y`), in its middle: where it is drawn
    /// whole.
    pub fn fitted_in(&self, x: f32, y: f32, w: f32, h: f32) -> (f32, f32, f32, f32) {
        let (pw, ph) = (self.size.0.max(1) as f32, self.size.1.max(1) as f32);
        let k = (w / pw).min(h / ph);
        let (fw, fh) = (pw * k, ph * k);
        (x + (w - fw) / 2.0, y + (h - fh) / 2.0, fw, fh)
    }
}

impl PartialEq for Picture {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

/// The inks a note's words are written in (see `Canvas::markdown`): its words', faint (Markdown's marks, a quote, a
/// done task), the accent (a link's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inks {
    pub words: Color,
    pub faint: Color,
    pub accent: Color,
}

impl Inks {
    pub fn of(&self, ink: crate::notes::markdown::Ink) -> Color {
        use crate::notes::markdown::Ink;
        match ink {
            Ink::Words => self.words,
            Ink::Faint => self.faint,
            Ink::Accent => self.accent,
        }
    }
}

/// A to-do's box, `side` square from (`x`, `y`), in a note's `inks`: ruled round faintly; done, filled in the accent
/// and ticked in what reads on it (light on a dark accent, dark on a light one).
pub fn task_box(frame: &dyn Canvas, inks: Inks, x: f32, y: f32, side: f32, done: bool) {
    let radius = side * 0.25;
    if done {
        frame.fill_rounded(inks.accent, x, y, side, side, radius);
        let light = 0.2126 * inks.accent.r + 0.7152 * inks.accent.g + 0.0722 * inks.accent.b;
        let tick = if light > 0.6 { Color::hex(0x1A1A1A, 1.0) } else { Color::hex(0xFFFFFF, 1.0) };
        frame.icon(super::icons::Icon::Check, Point { x: x + side / 2.0, y: y + side / 2.0 }, side * 0.78, tick);
    } else {
        let line = (side / 11.0).max(1.0);
        frame.stroke_rounded(Fill::Solid(inks.faint), x + line / 2.0, y + line / 2.0, side - line, side - line, radius, line);
    }
}

/// A list's dot in a note, in the room drawn for it from `left`, by its line's `baseline` (DIPs), its words `size` DIPs
/// high: in its words' ink (see `notes::markdown::BULLET_SIDE`).
pub fn bullet(frame: &dyn Canvas, inks: Inks, left: f32, baseline: f32, size: f32) {
    use crate::notes::markdown::{BOX_RISE, BULLET_IN, BULLET_SIDE};
    frame.fill_circle(inks.words, Point { x: left + BULLET_IN * size, y: baseline - BOX_RISE * size }, BULLET_SIDE * size / 2.0);
}

/// A quote's bars in a note, `deep` of them, in the room drawn for them from `left`, from `top` `tall` down (its line, all
/// its rows: DIPs), its words `size` DIPs high and `per` pixels a DIP: each a whole number of pixels wide (two at least),
/// in its faint ink, square at its ends (a quote's lines' bars one bar: see `notes::markdown::QUOTE_ROOM`).
pub fn quote_bars(frame: &dyn Canvas, inks: Inks, (left, top, tall): (f32, f32, f32), deep: usize, size: f32, per: f32) {
    use crate::notes::markdown::{BAR_IN, BAR_WIDE, QUOTE_ROOM};
    let wide = (BAR_WIDE * size * per).round().max(2.0) / per;
    for nth in 0..deep {
        let x = ((left + (nth as f32 * QUOTE_ROOM + BAR_IN) * size) * per).round() / per;
        frame.fill(Color { a: 0.6, ..inks.faint }, x, top, wide, tall);
    }
}

/// A note's block of code's box, `width` × `tall` from (`x`, `y`) (DIPs), its words `size` DIPs high: tinted in its faint
/// ink, its corners rounded.
pub fn code_box(frame: &dyn Canvas, inks: Inks, x: f32, y: f32, width: f32, tall: f32, size: f32) {
    frame.fill_rounded(Color { a: CODE_TINT, ..inks.faint }, x, y, width, tall, 0.3 * size);
}

/// How strongly a note's block of code's box is tinted in its faint ink.
const CODE_TINT: f32 = 0.1;

/// The fixed face code in a note is written in.
pub const CODE_FACE: &str = "Cascadia Mono";

/// `text`, or as much of its start as is no wider than `room` in `font` with an ellipsis after it: the most of its
/// characters that fit. Each measure lays out what it measures, so no more of it is: its start, twice as long each
/// time, till that alone is too wide, then halving between (one by one, and each time all of it, the station's texts
/// copied took seconds to draw).
pub fn fitted<'t>(frame: &(impl Canvas + ?Sized), text: &'t str, font: Font, room: f32) -> std::borrow::Cow<'t, str> {
    // (Where each character ends: its start so many of them long.)
    let ends: Vec<usize> = text.char_indices().map(|(at, c)| at + c.len_utf8()).collect();
    let start = |count: usize| &text[..count.checked_sub(1).map_or(0, |last| ends[last])];
    let mut over = 1;
    while over < ends.len() && frame.measure(start(over), font) <= room {
        over *= 2;
    }
    if over >= ends.len() {
        if frame.measure(text, font) <= room {
            return text.into();
        }
        over = ends.len();
    }
    // (Cut longer, never narrower: its start too wide alone, or ending in spaces left off that a cut as long leaves
    // off too. None of it, the ellipsis alone, however wide.)
    let cut = |count: usize| format!("{}…", start(count).trim_end());
    let mut fits = 0;
    while over - fits > 1 {
        let middle = (fits + over) / 2;
        if frame.measure(&cut(middle), font) <= room { fits = middle } else { over = middle }
    }
    cut(fits).into()
}

/// What the panel's readings are painted with: each platform paints these
/// with its own graphics (Direct2D and DirectWrite on Windows). Coordinates
/// are in DIPs; text is placed by the top of its line box.
/// How much deeper frosted glass makes the colours under it (see `Canvas::frosted`): blurred, they would go grey.
pub const FROST_SATURATION: f32 = 1.3;

pub trait Canvas {
    /// `text` on one line from (`x`, `y`), at the start or end of `width`,
    /// cut short with an ellipsis if it does not fit.
    #[allow(clippy::too_many_arguments)]
    fn text(&self, text: &str, font: Font, color: Color, x: f32, y: f32, width: f32, align: Align);
    /// `text` laid out in `font` and drawn `scale` times as large about
    /// (`x`, `y`), in `width` as drawn.
    #[allow(clippy::too_many_arguments)]
    fn text_scaled(&self, text: &str, font: Font, color: Color, x: f32, y: f32, width: f32, align: Align, scale: f32) {
        self.text(text, Font { size: font.size * scale, ..font }, color, x, y, width, align);
    }
    /// `text`, a note's words in Markdown, as the box it is written in writes them (see `notes::markdown::dressing`): in
    /// lines `width` wide from (`x`, `y`), in `font`, each stretch dressed as its looks make it, in `inks`, scrolled
    /// `scrolled` down (DIPs: what is above it out of sight); what is past `height` down cut off.
    #[allow(clippy::too_many_arguments)]
    fn markdown(&self, text: &str, font: Font, inks: Inks, x: f32, y: f32, width: f32, height: f32, scrolled: f32);
    /// How tall `text`, a note's words, is written `width` wide in `font` (see `markdown`): all of its lines, to its foot.
    fn markdown_tall(&self, text: &str, font: Font, width: f32) -> f32;
    /// How wide `text` is in `font`.
    fn measure(&self, text: &str, font: Font) -> f32;
    /// The ascent and descent of `font`'s line box.
    fn baseline(&self, font: Font) -> (f32, f32);
    /// How far below the top of its line box `text`'s ink starts and ends:
    /// where its glyphs, from whichever face draws them, actually are.
    fn ink(&self, text: &str, font: Font) -> (f32, f32);
    /// Draws only inside the rectangle until `unclip`; clips nest.
    fn clip(&self, x: f32, y: f32, width: f32, height: f32);
    fn unclip(&self);
    /// The rectangle as a fill of it comes out on whole pixels (see `Frame::on_pixels`): for a control drawn in parts
    /// (a track and what fills it, a bar standing on a floor) to cut its parts out of one box, not each put on pixels
    /// alone, which left them a pixel apart. As it is, where nothing is put on pixels.
    fn on_pixels(&self, x: f32, y: f32, width: f32, height: f32) -> (f32, f32, f32, f32) {
        (x, y, width, height)
    }
    /// Clips to the part of the box `(x, y, width, height)` that `fraction` of it fills, from its bottom (`up`) or its
    /// left: the box put on whole pixels once, the part as many whole pixels as that comes to, ending where the box
    /// does (see `filled_part`). (One call, not the box worked out by the caller: a recording of it, played back as a
    /// layout turns into another, works it out on the pixels it is played on.)
    fn clip_part(&self, x: f32, y: f32, width: f32, height: f32, fraction: f32, up: bool) {
        let whole = self.on_pixels(x, y, width, height);
        let part = filled_part(whole, fraction, up, |x, y, w, h| self.on_pixels(x, y, w, h));
        self.clip(part.0, part.1, part.2, part.3);
    }
    fn fill(&self, color: Color, x: f32, y: f32, width: f32, height: f32);
    fn fill_rounded(&self, color: Color, x: f32, y: f32, width: f32, height: f32, radius: f32);
    fn fill_circle(&self, color: Color, centre: Point, radius: f32);
    /// The line through `points`, `width` DIPs wide.
    fn stroke(&self, points: &[Point], color: Color, width: f32);
    /// The same, dashed.
    fn stroke_dashed(&self, points: &[Point], color: Color, width: f32);
    /// The shape `points` outline, closed.
    fn fill_shape(&self, points: &[Point], fill: Fill);
    /// The outline of the rounded rectangle, a line `line` DIPs wide
    /// centred on it.
    #[allow(clippy::too_many_arguments)]
    fn stroke_rounded(&self, fill: Fill, x: f32, y: f32, width: f32, height: f32, radius: f32, line: f32);
    /// The outline of the rounded rectangle, dashed, a line `line` DIPs wide inside its edge: its corners each a
    /// quarter circle in steps (as `stroke_dashed` draws), where it is not drawn as one.
    #[allow(clippy::too_many_arguments)]
    fn stroke_rounded_dashed(&self, color: Color, x: f32, y: f32, width: f32, height: f32, radius: f32, line: f32) {
        let (x, y, width, height) = (x + line / 2.0, y + line / 2.0, width - line, height - line);
        let radius = (radius - line / 2.0).clamp(0.0, width.min(height) / 2.0);
        const STEPS: usize = 6;
        // Each corner's middle and the angle it starts at (clockwise from the top left's).
        let corners = [
            (x + width - radius, y + radius, -std::f32::consts::FRAC_PI_2),
            (x + width - radius, y + height - radius, 0.0),
            (x + radius, y + height - radius, std::f32::consts::FRAC_PI_2),
            (x + radius, y + radius, std::f32::consts::PI),
        ];
        let mut points: Vec<Point> = corners
            .iter()
            .flat_map(|&(cx, cy, from)| {
                (0..=STEPS).map(move |step| {
                    let angle = from + std::f32::consts::FRAC_PI_2 * step as f32 / STEPS as f32;
                    Point { x: cx + radius * angle.cos(), y: cy + radius * angle.sin() }
                })
            })
            .collect();
        points.push(points[0]);
        self.stroke_dashed(&points, color, line);
    }
    /// The shadow the rounded rectangle casts, `drop` DIPs below it and
    /// blurred by a deviation of `blur` DIPs: outside the rectangle only, so
    /// that what is drawn in it shows through clear.
    #[allow(clippy::too_many_arguments)]
    fn shadow(&self, color: Color, x: f32, y: f32, width: f32, height: f32, radius: f32, blur: f32, drop: f32);
    /// Frosted glass over what `below` draws: that drawn, blurred by a deviation of `blur` DIPs and its colours
    /// deepened by `FROST_SATURATION`, shown only inside the rounded rectangle (what a pane of coloured glass laid on
    /// it shows of it; the pane's own tint is drawn over it as any fill is). What `below` draws is the pane's to
    /// show, not drawn of itself: whoever wants it seen outside the pane draws it too.
    #[allow(clippy::too_many_arguments)]
    fn frosted(&self, x: f32, y: f32, width: f32, height: f32, radius: f32, blur: f32, below: &dyn Fn(&dyn Canvas));
    /// An arc of the circle `radius` round `centre`, from `from` (radians
    /// clockwise from twelve o'clock) through `sweep` more, `width` wide,
    /// its ends round. A sweep of a whole turn or more is the circle.
    fn arc(&self, centre: Point, radius: f32, from: f32, sweep: f32, color: Color, width: f32);
    /// `icon` stroked in `color`, `size` DIPs square, centred on `centre`.
    fn icon(&self, icon: super::icons::Icon, centre: Point, size: f32, color: Color);

    /// `picture` drawn over the box `w` × `h` from (`x`, `y`), filling it (a box not of its shape shows the picture's
    /// middle, the rest cut off: see `Picture::fitted_in` for one drawn whole), its corners rounded by `radius`,
    /// `opacity` of it.
    #[allow(clippy::too_many_arguments)]
    fn picture(&self, picture: &Picture, x: f32, y: f32, w: f32, h: f32, radius: f32, opacity: f32) {
        self.picture_within(picture, (x, y, w, h), (x, y, w, h, radius), opacity);
    }
    /// `picture` drawn over the box `at` (x, y, width, height) as `picture` draws it, shown only where it is within
    /// the box `within` (x, y, width, height) rounded by its fifth: a picture drawn whole on a plate, square-cornered
    /// but where it meets the plate's round corners.
    fn picture_within(&self, picture: &Picture, at: (f32, f32, f32, f32), within: (f32, f32, f32, f32, f32), opacity: f32);
    /// How many of the target's pixels a DIP is, as drawn now: what a picture drawn so many DIPs wide is asked for at,
    /// to be as sharp as the pixels it is drawn on. One where nothing is drawn on pixels.
    fn pixels_per_dip(&self) -> f32 {
        1.0
    }
    /// Names what is drawn from here on (`""`: nothing), for a canvas that
    /// tells one drawing's parts from another's (see `morph`).
    fn key(&self, _key: &str) {}
}
