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
    /// The system's icon font: Segoe Fluent Icons on Windows 11, Segoe MDL2
    /// Assets before it; both put the same glyphs at the same code points.
    Icons,
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

#[derive(Clone, Copy, PartialEq, Eq)]
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

/// What the panel's readings are painted with: each platform paints these
/// with its own graphics (Direct2D and DirectWrite on Windows). Coordinates
/// are in DIPs; text is placed by the top of its line box.
pub trait Canvas {
    /// `text` on one line from (`x`, `y`), at the start or end of `width`,
    /// cut short with an ellipsis if it does not fit.
    #[allow(clippy::too_many_arguments)]
    fn text(&self, text: &str, font: Font, color: Color, x: f32, y: f32, width: f32, align: Align);
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
    fn fill(&self, color: Color, x: f32, y: f32, width: f32, height: f32);
    fn fill_rounded(&self, color: Color, x: f32, y: f32, width: f32, height: f32, radius: f32);
    fn fill_circle(&self, color: Color, centre: Point, radius: f32);
    /// The line through `points`, `width` DIPs wide.
    fn stroke(&self, points: &[Point], color: Color, width: f32);
    /// The shape `points` outline, closed.
    fn fill_shape(&self, points: &[Point], fill: Fill);
    /// The outline of the rounded rectangle, a line `line` DIPs wide
    /// centred on it.
    #[allow(clippy::too_many_arguments)]
    fn stroke_rounded(&self, fill: Fill, x: f32, y: f32, width: f32, height: f32, radius: f32, line: f32);
    /// The shadow the rounded rectangle casts, `drop` DIPs below it and
    /// blurred by a deviation of `blur` DIPs: outside the rectangle only, so
    /// that what is drawn in it shows through clear.
    #[allow(clippy::too_many_arguments)]
    fn shadow(&self, color: Color, x: f32, y: f32, width: f32, height: f32, radius: f32, blur: f32, drop: f32);
}
