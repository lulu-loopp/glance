//! The phone app's first screen, drawn for the settings' phone remote page:
//! this PC as the phone pictures it (its case, every part a block in it,
//! seen from one corner), with what the phone would be told of it now, and
//! what the phone may ask of it, to be asked from here too: every tile,
//! switch and slider is pressed and slid as on the phone (see `Part`).
//!
//! The case is drawn as the app and the remote's web page draw it (see
//! `remote/src/web.html`): the same blocks, the same corner.

use serde_json::Value;
use windows::core::Interface;
use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN, D2D1_PIXEL_FORMAT, D2D_SIZE_U};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Direct2D::{
    ID2D1BitmapBrush1, ID2D1Geometry, ID2D1StrokeStyle, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_BRUSH_PROPERTIES1, D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_PROPERTIES1, D2D1_BRUSH_PROPERTIES, D2D1_EXTEND_MODE_CLAMP,
    D2D1_INTERPOLATION_MODE_LINEAR, D2D1_CAP_STYLE_ROUND, D2D1_ELLIPSE, D2D1_LAYER_OPTIONS1_NONE, D2D1_LAYER_PARAMETERS1, D2D1_LINE_JOIN_ROUND, D2D1_QUADRATIC_BEZIER_SEGMENT,
    D2D1_ROUNDED_RECT, D2D1_STROKE_STYLE_PROPERTIES1,
};
use windows_numerics::{Matrix3x2, Vector2};

use super::canvas::{Align, Canvas, Color, Family, Font};
use super::gfx::{rect, Frame};
use super::text::Lang;
use super::view::Rect;

/// A phone's screen, as wide and as tall as the app lays itself out for (dp).
pub const SCREEN: (f32, f32) = (360.0, 760.0);
/// The case: how wide, deep and tall, and how round its corners.
const CASE: (f32, f32, f32) = (56.0, 124.0, 128.0);
const ROUND: f32 = 9.0;
/// The room the case is drawn in, as the app has it: where it starts, and its size.
const ROOM: (f32, f32, f32, f32) = (-190.0, -148.0, 320.0, 254.0);
/// The heat at which a part is drawn amber, and red (°C).
const WARM: i64 = 62;
const HOT: i64 = 75;
/// How many turns of a fan one turn of its picture stands for.
const SLOWED: f32 = 16.0;

const PAGE: Color = Color::hex(0x0E1116, 1.0);
const CARD: Color = Color::hex(0x171B22, 1.0);
const TILE: Color = Color::hex(0x1F242C, 1.0);
const INK: Color = Color::hex(0xF2F4F7, 1.0);
const DIM: Color = Color::hex(0x98A2AE, 1.0);
const FAINT: Color = Color::hex(0x5B6470, 1.0);
const GOOD: Color = Color::hex(0x4CC38A, 1.0);
const ACCENT: Color = Color::hex(0xFF7A45, 1.0);
const LIT: Color = Color::hex(0x2E211D, 1.0);
const STEEL: Color = Color::hex(0x8EA2C0, 1.0);

type Point = [f32; 3];
/// A figure of a path: where it starts, its curves (each by a corner, to
/// an end), and whether it closes.
type Figure = (Vector2, Vec<(Vector2, Vector2)>, bool);

/// A point of the case as it is seen: x to the right and down, y to the
/// left and down, z up.
fn seen(point: Point) -> Vector2 {
    let lean = (std::f32::consts::PI / 6.0).cos();
    Vector2 { X: (point[0] - point[1]) * lean, Y: (point[0] + point[1]) * 0.5 - point[2] }
}

/// What lays a flat drawing on a face of the case: from its corner `o`,
/// across `u` and along `v`.
fn face(o: Point, u: Point, v: Point) -> Matrix3x2 {
    let (across, along, corner) = (seen(u), seen(v), seen(o));
    Matrix3x2 { M11: across.X, M12: across.Y, M21: along.X, M22: along.Y, M31: corner.X, M32: corner.Y }
}

/// `color`, `percent` of it over the card's own ground.
fn shade(color: Color, percent: f32) -> Color {
    let k = percent / 100.0;
    Color { r: CARD.r + (color.r - CARD.r) * k, g: CARD.g + (color.g - CARD.g) * k, b: CARD.b + (color.b - CARD.b) * k, a: 1.0 }
}

/// What the phone is told of a part: how warm it is, and how busy.
fn read(reads: &Value, part: &str) -> (Option<i64>, Option<i64>) {
    let figure = |name: &str| reads.get(part).and_then(|part| part.get(name)).and_then(Value::as_i64);
    (figure("temp"), figure("load"))
}

/// A part's one figure: how warm it is; where that is not known, how busy.
fn figure((temp, load): (Option<i64>, Option<i64>)) -> String {
    temp.map(|temp| format!("{temp}°")).or(load.map(|load| format!("{load}%"))).unwrap_or_else(|| "—".into())
}

/// The picture being drawn: where, and with what.
struct Picture<'a> {
    frame: &'a Frame<'a>,
    /// From the screen's own measure (dp) to the window's.
    screen: Matrix3x2,
    /// From the case's room to the screen's measure.
    room: Matrix3x2,
    round: ID2D1StrokeStyle,
}

impl Picture<'_> {
    /// A path through `figures` (each a start, then curves by a corner and
    /// an end; closed or left open), as the case's room has it.
    fn path(&self, figures: &[Figure], through: Matrix3x2) -> Option<ID2D1Geometry> {
        unsafe {
            let path = self.frame.gfx.factory.CreatePathGeometry().ok()?;
            let sink = path.Open().ok()?;
            for (start, curves, closed) in figures {
                sink.BeginFigure(*start, if *closed { D2D1_FIGURE_BEGIN_FILLED } else { D2D1_FIGURE_BEGIN_HOLLOW });
                for (corner, end) in curves {
                    sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT { point1: *corner, point2: *end });
                }
                sink.EndFigure(if *closed { D2D1_FIGURE_END_CLOSED } else { D2D1_FIGURE_END_OPEN });
            }
            sink.Close().ok()?;
            self.frame.gfx.factory.CreateTransformedGeometry(&path, &(through * self.room)).ok()?.cast().ok()
        }
    }

    fn draw(&self, shape: Option<ID2D1Geometry>, fill: Option<Color>, stroke: Option<(Color, f32)>) {
        let Some(shape) = shape else { return };
        unsafe {
            if let Some(fill) = fill {
                self.frame.dc.FillGeometry(&shape, self.frame.brush(fill), None);
            }
            if let Some((color, width)) = stroke {
                self.frame.dc.DrawGeometry(&shape, self.frame.brush(color), width, &self.round);
            }
        }
    }

    /// A loop of corners in space, each rounded by `r` in the plane of its
    /// two edges.
    fn ring(&self, corners: &[Point], r: f32) -> Option<ID2D1Geometry> {
        let count = corners.len();
        let toward = |from: Point, to: Point| {
            let along = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
            let length = (along[0] * along[0] + along[1] * along[1] + along[2] * along[2]).sqrt();
            let k = r.min(length / 2.0) / length;
            seen([from[0] + along[0] * k, from[1] + along[1] * k, from[2] + along[2] * k])
        };
        // Each corner: straight to where its rounding begins, then round it.
        let rounded: Vec<(Vector2, Vector2, Vector2)> = (0..count).map(|at| (toward(corners[at], corners[(at + count - 1) % count]), seen(corners[at]), toward(corners[at], corners[(at + 1) % count]))).collect();
        let mut curves = Vec::new();
        for (at, (begins, corner, ends)) in rounded.iter().enumerate() {
            if at > 0 {
                // (A straight edge, as a curve whose corner is on it.)
                curves.push((*begins, *begins));
            }
            curves.push((*corner, *ends));
        }
        self.path(&[(rounded[0].0, curves, true)], Matrix3x2::identity())
    }

    /// A straight line or several, drawn flat (on a face, through `through`).
    fn lines(&self, lines: &[(Vector2, Vector2)], through: Matrix3x2, color: Color, width: f32) {
        let figures: Vec<_> = lines.iter().map(|(from, to)| (*from, vec![(*to, *to)], false)).collect();
        self.draw(self.path(&figures, through), None, Some((color, width)));
    }

    fn circle(&self, r: f32, through: Matrix3x2) -> Option<ID2D1Geometry> {
        unsafe {
            let circle = self.frame.gfx.factory.CreateEllipseGeometry(&D2D1_ELLIPSE { point: Vector2 { X: 0.0, Y: 0.0 }, radiusX: r, radiusY: r }).ok()?;
            self.frame.gfx.factory.CreateTransformedGeometry(&circle, &(through * self.room)).ok()?.cast().ok()
        }
    }

    fn sheet(&self, size: (f32, f32), r: f32, through: Matrix3x2) -> Option<ID2D1Geometry> {
        unsafe {
            let sheet = self.frame.gfx.factory.CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT { rect: rect(0.0, 0.0, size.0, size.1), radiusX: r, radiusY: r }).ok()?;
            self.frame.gfx.factory.CreateTransformedGeometry(&sheet, &(through * self.room)).ok()?.cast().ok()
        }
    }

    /// A block with soft corners: what it hides first, then its side
    /// towards x, its side towards y and its top.
    fn block(&self, from: Point, to: Point, r: f32, color: Color) {
        let ([x0, y0, z0], [x1, y1, z1]) = (from, to);
        let behind: Vec<Vector2> = [[x0, y0, z1], [x1, y0, z1], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0], [x0, y1, z1]].map(seen).to_vec();
        self.draw(self.path(&[(behind[0], behind[1..].iter().map(|corner| (*corner, *corner)).collect(), true)], Matrix3x2::identity()), Some(shade(color, 8.0)), None);
        for (side, percent) in [([[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]], 10.0), ([[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]], 5.0), ([[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]], 17.0)] {
            self.draw(self.ring(&side, r), Some(shade(color, percent)), Some((color, 1.0)));
        }
    }

    /// A fan lying in a face: its ring, its blades (turned by `turned`,
    /// radians) and its hub.
    fn fan(&self, centre: Point, u: Point, v: Point, r: f32, color: Color, turned: f32) {
        let through = face(centre, u, v);
        self.draw(self.circle(r, through), Some(shade(color, 6.0)), Some((color, 1.0)));
        let blades: Vec<_> = (0..7)
            .map(|blade| {
                let (sin, cos) = (turned + blade as f32 * std::f32::consts::TAU / 7.0).sin_cos();
                let at = |x: f32, y: f32| Vector2 { X: x * cos - y * sin, Y: x * sin + y * cos };
                (at(r * 0.2, 0.0), vec![(at(r * 0.55, -r * 0.42), at(r * 0.86, -r * 0.1))], false)
            })
            .collect();
        self.draw(self.path(&blades, through), None, Some((color, 1.0)));
        self.draw(self.circle(r * 0.2, through), Some(CARD), Some((color, 1.0)));
    }

    /// Words with their baseline at `y`, from `x` on or (`end`) up to it.
    fn words(&self, text: &str, font: Font, color: Color, x: f32, y: f32, end: bool) {
        let (width, top) = (400.0, y - self.frame.gfx.baseline(font).0);
        if end {
            self.frame.text(text, font, color, x - width, top, width, Align::End);
        } else {
            self.frame.text(text, font, color, x, top, width, Align::Start);
        }
    }

    /// Words with their middle at `x`.
    fn middled(&self, text: &str, font: Font, color: Color, x: f32, y: f32) {
        let width = self.frame.gfx.measure(text, font);
        self.words(text, font, color, x - width / 2.0, y, false);
    }

    fn plate(&self, at: Rect, r: f32, color: Color) {
        unsafe { self.frame.dc.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: rect(at.x, at.y, at.w, at.h), radiusX: r, radiusY: r }, self.frame.brush(color)) };
    }

    /// What is said of a part, out beside the picture on a line from the
    /// part (`from`, in space) to its words (at `to`, in the room; ending
    /// there, `end`, or beginning): a figure, and a line or two under it.
    fn say(&self, from: Point, to: (f32, f32), end: bool, lines: [&str; 3]) {
        let at = |x: f32, y: f32| Vector2 { X: x, Y: y };
        let (large, small) = (Font::new(Family::Segoe, 15.0, 600.0), Font::new(Family::Segoe, 9.5, 400.0));
        let (from, elbow) = (seen(from), to.0 + if end { 12.0 } else { -12.0 });
        unsafe {
            self.frame.dc.DrawLine(from, at(elbow, to.1), self.frame.brush(FAINT), 0.8, &self.round);
            self.frame.dc.DrawLine(at(elbow, to.1), at(to.0, to.1), self.frame.brush(FAINT), 0.8, &self.round);
            self.frame.dc.FillEllipse(&D2D1_ELLIPSE { point: from, radiusX: 2.0, radiusY: 2.0 }, self.frame.brush(INK));
        }
        let x = to.0 + if end { -5.0 } else { 5.0 };
        self.words(lines[0], large, INK, x, to.1 + 1.0, end);
        self.words(lines[1], small, DIM, x, to.1 + 13.0, end);
        self.words(lines[2], small, DIM, x, to.1 + 25.0, end);
    }

    /// A laptop, drawn as the case is and as one with a clear shell would
    /// look: its screen up at the back; in its body, its board with the
    /// processor and the card on it, a pipe from each to its fan, its
    /// memory, its drive and its battery; and over them, faintly, its keys
    /// and its pad. In the middle of the room, as the case is. `light` is
    /// the colour of the light under its keys while that is on.
    #[allow(clippy::too_many_arguments)]
    fn laptop(&mut self, x: f32, y: f32, width: f32, reads: &Value, lang: Lang, seconds: f32, light: Option<Color>) {
        let k = width / ROOM.2;
        self.room = Matrix3x2::scale(k, k) * Matrix3x2::translation(x - ROOM.0 * k, y - ROOM.1 * k);
        // Its body: from (X0, Y0), W wide and D deep, T thick; its screen S up from its back edge, leaning back by LEAN.
        // As a laptop of fourteen inches is: its body 104 by 74 (about 312 mm by 222), its lid as deep as its
        // body (shut, it covers it), opened 18 degrees past upright; its screen 16 by 10 in the lid, the wider
        // edge of the lid under it. Its body and its lid drawn thicker than they are, to be seen at this size:
        // the body twice the lid.
        const U: f32 = 1.0;
        const X0: f32 = -52.0;
        const Y0: f32 = 9.0;
        const W: f32 = 104.0 * U;
        const D: f32 = 74.0 * U;
        const T: f32 = 6.0;
        const S: f32 = D * 0.951;
        const LEAN: f32 = D * 0.309;
        /// How thick the lid is, and how far up in the body what is in it lies.
        const LID: f32 = 3.0;
        const IN: f32 = 1.5;
        let at = |x: f32, y: f32| Vector2 { X: x, Y: y };
        let lit = |color: Color, much: f32| light.map_or(color, |light| Color { r: color.r + (light.r - color.r) * much, g: color.g + (light.g - color.g) * much, b: color.b + (light.b - color.b) * much, a: color.a });
        let turn = |rpm: Option<i64>| seconds * rpm.unwrap_or(0).max(0) as f32 / 60.0 / SLOWED * std::f32::consts::TAU;
        let cpu_fan = reads.get("cpu_fan").and_then(Value::as_i64);
        let shell = lit(Color::hex(0x8F99A6, 1.0), 0.22);
        let traced = Color::hex(0x46505C, 1.0);
        // The screen, at the back: its lid, and the picture's place in it.
        // (As thick as LID: its back is that far behind its face, of which its top and its side towards the eye show.)
        let lid = [[X0, Y0, T], [X0 + W, Y0, T], [X0 + W, Y0 - LEAN, T + S], [X0, Y0 - LEAN, T + S]];
        let slant = (S * S + LEAN * LEAN).sqrt();
        let behind = |corner: Point| [corner[0], corner[1] - LID * S / slant, corner[2] - LID * LEAN / slant];
        for edge in [[lid[1], lid[2], behind(lid[2]), behind(lid[1])], [lid[3], lid[2], behind(lid[2]), behind(lid[3])]] {
            self.draw(self.ring(&edge, 1.0), Some(shade(shell, 12.0)), Some((shell, 1.0)));
        }
        self.draw(self.ring(&lid, 4.0), Some(Color::hex(0x12161B, 1.0)), Some((shell, 1.4)));
        // (Up the lid, a step as long as one across it; the screen 3.5 in from its sides and its top.)
        let up = [0.0, -LEAN / D, S / D];
        let (across, edge) = (W - 7.0, 3.5);
        let tall = across * 10.0 / 16.0;
        let from = D - edge - tall;
        self.draw(self.sheet((across, tall), 2.5, face([X0 + edge, Y0 + up[1] * from, T + up[2] * from], [1.0, 0.0, 0.0], up)), Some(Color::hex(0x0B0E12, 1.0)), Some((Color::hex(0x3A424D, 1.0), 1.0)));
        // The body, clear: what is in it lies on its floor, seen through its deck.
        self.block([X0, Y0, 0.0], [X0 + W, Y0 + D, T], 2.5, shell);
        let inside = |x: f32, y: f32| face([X0 + x * U, Y0 + y * U, IN], [U, 0.0, 0.0], [0.0, U, 0.0]);
        let colour = |part: &str| match read(reads, part).0 {
            Some(temp) if temp >= HOT => Color::hex(0xFF6B5E, 1.0),
            Some(temp) if temp >= WARM => Color::hex(0xFFB057, 1.0),
            _ => lit(STEEL, 0.42),
        };
        // The board across the back, and the battery before it.
        self.draw(self.sheet((90.0, 33.0), 3.0, inside(7.0, 4.0)), Some(Color::hex(0x161B21, 1.0)), Some((traced, 1.0)));
        self.draw(self.sheet((74.0, 21.0), 3.0, inside(15.0, 43.0)), Some(Color::hex(0x141920, 1.0)), Some((traced, 1.0)));
        let cells: Vec<_> = [1.0, 2.0].iter().map(|cell| (at(cell * 74.0 / 3.0, 3.0), at(cell * 74.0 / 3.0, 18.0))).collect();
        self.lines(&cells, inside(15.0, 43.0), traced.alpha(0.7), 1.0);
        // A pipe from the processor and from the card, each out to its fan.
        let pipes = [(at(25.0, 17.0), at(45.0, 17.0)), (at(25.0, 20.0), at(45.0, 20.0)), (at(64.0, 17.0), at(79.0, 17.0)), (at(64.0, 20.0), at(79.0, 20.0))];
        self.lines(&pipes, inside(0.0, 0.0), Color::hex(0x59636F, 1.0), 1.6);
        let fans = lit(Color::hex(0x59636F, 1.0), 1.0);
        for across in [20.0, 84.0] {
            self.fan([X0 + across * U, Y0 + 19.0 * U, IN + 0.2], [U, 0.0, 0.0], [0.0, U, 0.0], 11.5, fans, turn(cpu_fan));
        }
        let chip = |picture: &Picture, part: &str, x: f32, y: f32, size: (f32, f32)| {
            let color = colour(part);
            picture.draw(picture.sheet(size, 1.6, inside(x, y)), Some(shade(color, 16.0)), Some((color, 1.0)));
        };
        chip(self, "cpu", 37.0, 11.0, (14.0, 14.0));
        chip(self, "gpu", 54.0, 11.0, (15.0, 14.0));
        chip(self, "ram", 34.0, 28.0, (17.0, 6.0));
        chip(self, "disk", 54.0, 28.0, (22.0, 6.0));
        // Over them, faintly: the keys in their rows, and the pad.
        let deck = |x: f32, y: f32| face([X0 + x * U, Y0 + y * U, T], [U, 0.0, 0.0], [0.0, U, 0.0]);
        let keys = lit(Color::hex(0x8F99A6, 1.0), 0.5).alpha(if light.is_some() { 0.55 } else { 0.34 });
        let (wide, deep, rows, across) = (90.0, 31.0, 5, 13);
        self.draw(self.sheet((wide, deep), 2.0, deck(7.0, 6.0)), None, Some((keys, 1.0)));
        let mut grid: Vec<(Vector2, Vector2)> = (1..rows).map(|row| (at(0.0, deep * row as f32 / rows as f32), at(wide, deep * row as f32 / rows as f32))).collect();
        // (Each row's keys set off from the next row's, as keys are; the last row with its long key in the middle.)
        for row in 0..rows {
            let (top, bottom) = (deep * row as f32 / rows as f32, deep * (row + 1) as f32 / rows as f32);
            let shift = [0.0, 0.5, 0.25, 0.75, 0.0][row] * wide / across as f32;
            grid.extend((1..across).map(|key| key as f32 * wide / across as f32 + shift).filter(|x| *x < wide - 1.0 && (row + 1 < rows || !(wide * 0.3..wide * 0.7).contains(x))).map(|x| (at(x, top), at(x, bottom))));
        }
        self.lines(&grid, deck(7.0, 6.0), keys, 0.8);
        self.draw(self.sheet((30.0, 19.0), 2.5, deck(37.0, 44.0)), None, Some((keys, 1.0)));

        // What is said of each part. (In the room's own measure, as the laptop is.)
        self.frame.place(self.room * self.screen);
        let named = |part: &str, name: &str| match read(reads, part) {
            (Some(_), Some(load)) => format!("{name} · {load}%"),
            _ => name.to_string(),
        };
        let fan = cpu_fan.map(|rpm| lang.pick(&format!("风扇 {rpm} 转"), &format!("Fan {rpm} rpm")).to_string()).unwrap_or_default();
        self.say([X0 + 44.0 * U, Y0 + 18.0 * U, IN], (-122.0, -104.0), true, [&figure(read(reads, "cpu")), &named("cpu", "CPU"), &fan]);
        self.say([X0 + 61.5 * U, Y0 + 18.0 * U, IN], (74.0, -34.0), false, [&figure(read(reads, "gpu")), &named("gpu", lang.pick("显卡", "Graphics")), ""]);
        self.say([X0 + 42.5 * U, Y0 + 31.0 * U, IN], (-134.0, 64.0), true, [&figure(read(reads, "ram")), &named("ram", lang.pick("内存", "Memory")), ""]);
        self.say([X0 + 65.0 * U, Y0 + 31.0 * U, IN], (74.0, 66.0), false, [&figure(read(reads, "disk")), &named("disk", lang.pick("硬盘", "Disk")), ""]);
        self.frame.place(self.screen);
    }

    /// The case and what is in it, with what is said of each part out
    /// beside it, on a line to it: in a room `width` wide whose corner is
    /// at `(x, y)`.
    /// `light` is the lights' colour while they are on, which the whole
    /// inside takes a little of: the walls, the fans, the card's edge.
    #[allow(clippy::too_many_arguments)]
    fn rig(&mut self, x: f32, y: f32, width: f32, reads: &Value, lang: Lang, seconds: f32, light: Option<Color>) {
        let k = width / ROOM.2;
        self.room = Matrix3x2::scale(k, k) * Matrix3x2::translation(x - ROOM.0 * k, y - ROOM.1 * k);
        let (w, d, h) = CASE;
        let at = |x: f32, y: f32| Vector2 { X: x, Y: y };
        let turn = |rpm: Option<i64>| seconds * rpm.unwrap_or(0).max(0) as f32 / 60.0 / SLOWED * std::f32::consts::TAU;
        let (cpu_fan, case_fan) = (reads.get("cpu_fan").and_then(Value::as_i64), reads.get("case_fan").and_then(Value::as_i64));
        // So much of the lights' colour in `color`; none of it while they are out.
        let lit = |color: Color, much: f32| light.map_or(color, |light| Color { r: color.r + (light.r - color.r) * much, g: color.g + (light.g - color.g) * much, b: color.b + (light.b - color.b) * much, a: color.a });
        // The fans are lit as the lights are; out, they are the case's grey.
        let fans = lit(Color::hex(0x59636F, 1.0), 1.0);
        // The walls behind: the floor, the back and the far side.
        for wall in [[[0.0, 0.0, 0.0], [w, 0.0, 0.0], [w, d, 0.0], [0.0, d, 0.0]], [[0.0, 0.0, 0.0], [0.0, d, 0.0], [0.0, d, h], [0.0, 0.0, h]], [[0.0, 0.0, 0.0], [w, 0.0, 0.0], [w, 0.0, h], [0.0, 0.0, h]]] {
            self.draw(self.ring(&wall, ROUND), Some(lit(Color::hex(0x12161B, 1.0), 0.09)), Some((Color::hex(0x3A424D, 1.0), 1.0)));
        }
        // The board on the far side.
        let board = face([0.5, 10.0, 32.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]);
        let traced = Color::hex(0x46505C, 1.0);
        self.draw(self.sheet((86.0, 86.0), 3.0, board), Some(Color::hex(0x161B21, 1.0)), Some((traced, 1.0)));
        self.lines(&[(at(8.0, 12.0), at(78.0, 12.0)), (at(8.0, 20.0), at(78.0, 20.0))], board, traced.alpha(0.55), 1.0);
        self.fan([w / 2.0, 0.6, 98.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], 16.0, fans, turn(case_fan));
        // The cover over the power supply, and its vents.
        let cover = Color::hex(0x4A5461, 1.0);
        self.block([2.0, 2.0, 2.0], [w - 2.0, d - 2.0, 24.0], 3.0, cover);
        let vents: Vec<_> = (0..9).map(|vent| (at(vent as f32 * 6.0, 0.0), at(vent as f32 * 6.0, 10.0))).collect();
        self.lines(&vents, face([w - 2.0, 18.0, 8.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]), cover.alpha(0.7), 1.0);
        // A part is the case's own steel till it runs warm: then amber, then red.
        let colour = |part: &str| match read(reads, part).0 {
            Some(temp) if temp >= HOT => Color::hex(0xFF6B5E, 1.0),
            Some(temp) if temp >= WARM => Color::hex(0xFFB057, 1.0),
            _ => lit(STEEL, 0.42),
        };
        let disk = colour("disk");
        self.block([16.0, 86.0, 24.0], [46.0, 114.0, 28.0], 1.6, disk);
        self.draw(self.sheet((22.0, 20.0), 2.0, face([20.0, 90.0, 28.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0])), None, Some((disk.alpha(0.5), 1.0)));
        let gpu = colour("gpu");
        self.block([0.5, 10.0, 58.0], [42.0, 102.0, 72.0], 2.4, gpu);
        self.draw(self.sheet((30.0, 76.0), 3.0, face([6.0, 18.0, 72.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0])), None, Some((gpu.alpha(0.5), 1.0)));
        // (Its light along the edge towards the glass.)
        if let Some(light) = light {
            self.lines(&[(seen([42.0, 20.0, 65.0]), seen([42.0, 92.0, 65.0]))], Matrix3x2::identity(), light, 2.5);
        }
        let cpu = colour("cpu");
        self.block([0.5, 28.0, 86.0], [22.0, 52.0, 110.0], 2.4, cpu);
        self.fan([11.5, 40.0, 110.3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], 8.5, cpu, turn(cpu_fan));
        // The cooler's little display, and how warm it says the processor is.
        let display = face([22.0, 49.0, 107.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]);
        self.draw(self.sheet((18.0, 18.0), 3.5, display), Some(Color::hex(0x0B0E12, 1.0)), Some((cpu, 1.0)));
        if let Some(temp) = read(reads, "cpu").0 {
            self.frame.place(display * self.room * self.screen);
            self.middled(&temp.to_string(), Font::new(Family::Segoe, 8.5, 600.0), cpu, 9.0, 12.4);
            self.frame.place(self.screen);
        }
        let ram = colour("ram");
        for stick in [60.0, 66.0, 72.0, 78.0] {
            self.block([0.5, stick, 84.0], [15.0, stick + 2.4, 116.0], 0.8, ram);
        }
        for z in [38.0, 70.0, 102.0] {
            self.fan([w / 2.0, d - 0.6, z], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], 15.0, fans, turn(case_fan));
        }
        // The case's own shell, over all: its top, its near side and its front.
        for panel in [[[0.0, 0.0, h], [w, 0.0, h], [w, d, h], [0.0, d, h]], [[w, 0.0, 0.0], [w, d, 0.0], [w, d, h], [w, 0.0, h]], [[0.0, d, 0.0], [w, d, 0.0], [w, d, h], [0.0, d, h]]] {
            self.draw(self.ring(&panel, ROUND), None, Some((lit(Color::hex(0x8F99A6, 1.0), 0.22), 1.4)));
        }

        // What is said of each part. (In the room's own measure, as the case is.)
        self.frame.place(self.room * self.screen);
        let say = |from: Point, to: (f32, f32), end: bool, lines: [&str; 3]| self.say(from, to, end, lines);
        let named = |part: &str, name: &str| match read(reads, part) {
            (Some(_), Some(load)) => format!("{name} · {load}%"),
            _ => name.to_string(),
        };
        let fan = cpu_fan.map(|rpm| lang.pick(&format!("风扇 {rpm} 转"), &format!("Fan {rpm} rpm")).to_string()).unwrap_or_default();
        say([22.0, 36.0, 100.0], (66.0, -100.0), false, [&figure(read(reads, "cpu")), &named("cpu", "CPU"), &fan]);
        say([42.0, 36.0, 68.0], (74.0, -30.0), false, [&figure(read(reads, "gpu")), &named("gpu", lang.pick("显卡", "Graphics")), ""]);
        say([40.0, 98.0, 28.0], (74.0, 62.0), false, [&figure(read(reads, "disk")), &named("disk", lang.pick("硬盘", "Disk")), ""]);
        say([15.0, 70.0, 116.0], (-122.0, -124.0), true, [&figure(read(reads, "ram")), &named("ram", lang.pick("内存", "Memory")), ""]);
        if let Some(rpm) = case_fan {
            say([w / 2.0, d, 38.0], (-134.0, 64.0), true, [&rpm.to_string(), lang.pick("机箱风扇", "Case fans"), lang.pick("转/分", "rpm")]);
        }
        self.frame.place(self.screen);
    }
}

/// What on the phone's page takes a press or a slide.
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Night,
    Screen,
    Lights,
    /// One of the PC's power modes, by its id.
    Power(String),
    /// A colour for the lights (RRGGBB).
    Colour(String),
    /// A colour kept to choose from, taken away; and the taking away ended.
    Remove(String),
    Done,
    /// What the lights do.
    Effect(&'static str),
    /// How bright the lights are: slid.
    Brightness,
    /// The ring a colour is made on, opened or shut; on it, the hue (slid
    /// round its middle) and how much colour (slid); and the colour made
    /// given up, or kept.
    Make,
    Hue,
    Depth,
    GiveUp,
    Use,
    /// A monitor by itself, on or off; and how bright it is: slid.
    Monitor(usize),
    MonitorBrightness(usize),
    /// How bright the light under a laptop's keys is: one of its steps.
    Keys(u8),
    Mute,
    /// How loud: slid.
    Volume,
    /// What is playing: "previous", "toggle" or "next".
    Media(&'static str),
    Sleep,
}

impl Part {
    /// Whether it is slid (its value from 0 to 1 along it) rather than pressed.
    pub fn slides(&self) -> bool {
        matches!(self, Part::Brightness | Part::MonitorBrightness(_) | Part::Volume | Part::Hue | Part::Depth)
    }

    /// How far along it (0 to 1) a point is, the part being at `track`:
    /// round the ring's middle from three o'clock for the hue, across the
    /// track for every other.
    pub fn along(&self, track: Rect, x: f32, y: f32) -> f32 {
        match self {
            Part::Hue => (y - track.y - track.h / 2.0).atan2(x - track.x - track.w / 2.0).to_degrees().rem_euclid(360.0) / 360.0,
            _ => ((x - track.x) / track.w).clamp(0.0, 1.0),
        }
    }
}

/// A colour being made on the ring: the one the lights had as the ring was
/// opened (to go back to), its hue (degrees) and how much colour it has
/// (0 to 1). It is the colour at its brightest: how bright the lights are
/// is set for them all, by their own slider.
#[derive(Clone, Debug, PartialEq)]
pub struct Making {
    pub was: String,
    pub hue: f32,
    pub depth: f32,
}

impl Making {
    /// Begun from the colour in use (RRGGBB).
    pub fn from(was: &str) -> Making {
        let Color { r, g, b, .. } = colour(was);
        let (most, least) = (r.max(g).max(b), r.min(g).min(b));
        let spread = most - least;
        let hue = if spread == 0.0 {
            0.0
        } else if most == r {
            60.0 * ((g - b) / spread).rem_euclid(6.0)
        } else if most == g {
            60.0 * ((b - r) / spread + 2.0)
        } else {
            60.0 * ((r - g) / spread + 4.0)
        };
        Making { was: was.to_uppercase(), hue, depth: if most == 0.0 { 0.0 } else { spread / most } }
    }

    /// The colour as made so far (RRGGBB).
    pub fn hex(&self) -> String {
        let Color { r, g, b, .. } = hued(self.hue, self.depth);
        let part = |of: f32| (of * 255.0).round() as u32;
        format!("{:06X}", part(r) << 16 | part(g) << 8 | part(b))
    }
}

/// Every hue round a middle, from three o'clock, as a brush: what a ring
/// drawn with it is coloured by, `across` wide about `centre`.
fn hues(frame: &Frame, centre: Vector2, across: f32) -> Option<ID2D1BitmapBrush1> {
    const SIDE: u32 = 512;
    let bitmap = frame
        .gfx
        .made([u32::from_be_bytes(*b"HUES"), SIDE, 0, 0], || {
            let mut pixels = vec![0u8; (SIDE * SIDE * 4) as usize];
            for row in 0..SIDE {
                for column in 0..SIDE {
                    let (x, y) = (column as f32 + 0.5 - SIDE as f32 / 2.0, row as f32 + 0.5 - SIDE as f32 / 2.0);
                    let Color { r, g, b, .. } = hued(y.atan2(x).to_degrees().rem_euclid(360.0), 1.0);
                    let at = ((row * SIDE + column) * 4) as usize;
                    // BGRA, opaque.
                    pixels[at..at + 4].copy_from_slice(&[(b * 255.0).round() as u8, (g * 255.0).round() as u8, (r * 255.0).round() as u8, 255]);
                }
            }
            let properties = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
                ..Default::default()
            };
            unsafe { frame.dc.CreateBitmap(D2D_SIZE_U { width: SIDE, height: SIDE }, Some(pixels.as_ptr().cast()), SIDE * 4, &properties) }
        })
        .ok()?;
    let k = across / SIDE as f32;
    let placed = D2D1_BRUSH_PROPERTIES { opacity: 1.0, transform: Matrix3x2::scale(k, k) * Matrix3x2::translation(centre.X - across / 2.0, centre.Y - across / 2.0) };
    let smooth = D2D1_BITMAP_BRUSH_PROPERTIES1 { extendModeX: D2D1_EXTEND_MODE_CLAMP, extendModeY: D2D1_EXTEND_MODE_CLAMP, interpolationMode: D2D1_INTERPOLATION_MODE_LINEAR };
    unsafe { frame.dc.CreateBitmapBrush(&bitmap, Some(&smooth), Some(&placed)).ok() }
}

/// The colour of a hue (degrees) with so much colour in it (0 to 1), at its brightest.
fn hued(hue: f32, depth: f32) -> Color {
    let part = |shift: f32| {
        let k = (shift + hue / 60.0).rem_euclid(6.0);
        1.0 - depth * k.min(4.0 - k).clamp(0.0, 1.0)
    };
    Color { r: part(5.0), g: part(3.0), b: part(1.0), a: 1.0 }
}

/// What was drawn: where each thing to press or slide is (in the window's
/// measure; a slider's is its track), and how tall the page is (the
/// screen's measure), for scrolling it.
pub struct Painted {
    pub parts: Vec<(Rect, Part)>,
    pub tall: f32,
}

/// The colours the app starts with.
const GIVEN: [&str; 6] = ["FF7B4A", "FFFFFF", "70B3FF", "59C98A", "C68CFF", "FF5E7E"];
/// How far apart the colours to choose from are, each way (the screen's measure).
const SWATCH: f32 = 42.0;

/// The colours kept to choose from, as the remote says (see the remote's
/// `Config::colours`); none kept yet, those the app starts with.
pub fn kept_colours(state: &Value) -> Vec<String> {
    match state.get("colours").and_then(Value::as_array) {
        Some(kept) => kept.iter().filter_map(Value::as_str).map(str::to_uppercase).collect(),
        None => GIVEN.iter().map(|given| given.to_string()).collect(),
    }
}
/// How much taller the lights' card is with the ring open: the ring, how much colour, and the two buttons.
const RING: f32 = 4.0 + 196.0 + 16.0 + 28.0 + 16.0 + 44.0 + 16.0;
/// Where the page under its head begins (the screen's measure).
pub const HEAD: f32 = 60.0;

fn colour(hex: &str) -> Color {
    Color::hex(u32::from_str_radix(hex, 16).unwrap_or(0xFFFFFF), 1.0)
}

/// Draws the app's page in `at` (a phone's screen: `SCREEN`, to whatever
/// size), for the PC called `name`: what the phone would be told of it now
/// (`reads`: see `remote::figures`), and how the remote says it is
/// (`state`: what a phone is answered; none, while the remote has not
/// answered: the page is then a picture, with nothing to press; `off`,
/// where the remote is not turned on at all), `seconds` into the fans'
/// turning, the page under its head moved up by `scroll`,
/// and a slider in hand shown where it is held (`held`: which, and how far
/// along); the ring a colour is made on is open while one is `making`;
/// the colours kept are being taken away while `removing`.
#[allow(clippy::too_many_arguments)]
pub fn paint(frame: &Frame, at: Rect, lang: Lang, name: &str, reads: &Value, state: Option<&Value>, off: bool, seconds: f32, scroll: f32, held: Option<(&Part, f32)>, making: Option<&Making>, removing: bool) -> Painted {
    let nothing = Painted { parts: Vec::new(), tall: 0.0 };
    let k = at.w / SCREEN.0;
    let screen = Matrix3x2::scale(k, k) * Matrix3x2::translation(at.x, at.y);
    let Ok(round) = (unsafe { frame.gfx.factory.CreateStrokeStyle(&D2D1_STROKE_STYLE_PROPERTIES1 { startCap: D2D1_CAP_STYLE_ROUND, endCap: D2D1_CAP_STYLE_ROUND, lineJoin: D2D1_LINE_JOIN_ROUND, ..Default::default() }, None) }) else { return nothing };
    let mut picture = Picture { frame, screen, room: Matrix3x2::identity(), round: round.into() };
    // Whatever runs past the screen's end is not drawn: a page that goes on below.
    let Ok(glass) = (unsafe { frame.gfx.factory.CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT { rect: rect(0.0, 0.0, SCREEN.0, at.h / k), radiusX: 26.0, radiusY: 26.0 }) }) else { return nothing };
    frame.place(screen);
    let layer = D2D1_LAYER_PARAMETERS1 {
        contentBounds: rect(0.0, 0.0, SCREEN.0, at.h / k),
        geometricMask: std::mem::ManuallyDrop::new(glass.cast().ok()),
        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
        maskTransform: Matrix3x2::identity(),
        opacity: 1.0,
        opacityBrush: std::mem::ManuallyDrop::new(None),
        layerOptions: D2D1_LAYER_OPTIONS1_NONE,
    };
    unsafe { frame.dc.PushLayer(&layer, None) };
    picture.plate(Rect { x: 0.0, y: 0.0, w: SCREEN.0, h: at.h / k }, 0.0, PAGE);

    let (side, wide) = (16.0, SCREEN.0 - 32.0);
    // Whose it is, and whether it answers.
    picture.words(name, Font::new(Family::Segoe, 21.0, 600.0), INK, side, 50.0, false);
    let answering = match (state.is_some(), off) {
        (true, _) => lang.pick("已连接", "Connected"),
        (false, true) => lang.pick("遥控未开启", "The remote is off"),
        (false, false) => lang.pick("正在连接…", "Connecting…"),
    };
    let small = Font::new(Family::Segoe, 12.5, 400.0);
    picture.words(answering, small, DIM, SCREEN.0 - side, 48.0, true);
    unsafe { frame.dc.FillEllipse(&D2D1_ELLIPSE { point: Vector2 { X: SCREEN.0 - side - frame.gfx.measure(answering, small) - 11.0, Y: 43.5 }, radiusX: 4.0, radiusY: 4.0 }, frame.brush(if state.is_some() { GOOD } else { FAINT })) };

    // Under its head the page moves: what is there is in the page's own measure from here on.
    unsafe { frame.dc.PushAxisAlignedClip(&rect(0.0, HEAD, SCREEN.0, at.h / k - HEAD), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
    picture.screen = Matrix3x2::translation(0.0, -scroll) * screen;
    frame.place(picture.screen);
    let mut parts: Vec<(Rect, Part)> = Vec::new();
    // Where a part of the page is in the window, as much of it as shows under the head.
    let shown = |r: Rect| {
        let (top, bottom) = ((r.y - scroll).max(HEAD), (r.y + r.h - scroll).min(at.h / k));
        (bottom > top).then(|| Rect { x: at.x + r.x * k, y: at.y + top * k, w: r.w * k, h: (bottom - top) * k })
    };
    // A row of chips in a card scrolls across in the app, cut at the card's sides inside it: drawn so (see `in_row`),
    // and pressed only where it shows.
    let in_row = |r: Rect| {
        let (left, right) = (r.x.max(side + 16.0), (r.x + r.w).min(side + wide - 16.0));
        (right > left).then(|| Rect { x: left, w: right - left, ..r }).and_then(shown)
    };

    // The PC drawn, and its four parts under it.
    let top = 68.0;
    let drawing = (wide - 16.0) * ROOM.3 / ROOM.2;
    picture.plate(Rect { x: side, y: top, w: wide, h: 8.0 + drawing + 8.0 + 52.0 + 12.0 }, 20.0, CARD);
    // (Lit as the lights are: in their colour while they are on; in the one being made, while one is.)
    let lit = state.filter(|state| state["lights"].as_bool() == Some(true) && state["night"].as_bool() != Some(true) && state["colours_there"].as_bool() != Some(false));
    let light = lit.and_then(|state| match making {
        Some(making) => Some(hued(held.filter(|(held, _)| **held == Part::Hue).map_or(making.hue, |(_, at)| at * 360.0), held.filter(|(held, _)| **held == Part::Depth).map_or(making.depth, |(_, at)| at))),
        None => state["light"]["color"].as_str().filter(|color| color.len() == 6).map(colour),
    });
    // (A laptop is drawn as one; its light is the one under its keys, which has no colour to say: the lights' own.)
    if state.is_some_and(|state| state["laptop"].as_bool() == Some(true)) {
        // (In its colour where it takes one, else white.)
        let keys = state.filter(|state| state["lights"].as_bool() == Some(true) && state["night"].as_bool() != Some(true)).map(|_| light.unwrap_or(INK));
        picture.laptop(side + 8.0, top + 8.0, wide - 16.0, reads, lang, seconds, keys);
    } else {
        picture.rig(side + 8.0, top + 8.0, wide - 16.0, reads, lang, seconds, light);
    }
    let named = [("cpu", "CPU"), ("gpu", lang.pick("显卡", "Graphics")), ("ram", lang.pick("内存", "Memory")), ("disk", lang.pick("硬盘", "Disk"))];
    let each = (wide - 24.0 - 3.0 * 8.0) / 4.0;
    for (place, (part, called)) in named.into_iter().enumerate() {
        let (x, y) = (side + 12.0 + place as f32 * (each + 8.0), top + 8.0 + drawing + 8.0);
        picture.plate(Rect { x, y, w: each, h: 52.0 }, 12.0, TILE);
        picture.words(&figure(read(reads, part)), Font::new(Family::Segoe, 15.0, 600.0), INK, x + 10.0, y + 23.0, false);
        let under = match read(reads, part) {
            (Some(_), Some(load)) => format!("{called} {load}%"),
            _ => called.to_string(),
        };
        picture.words(&under, Font::new(Family::Segoe, 10.0, 400.0), DIM, x + 10.0, y + 40.0, false);
    }
    let mut y = top + 8.0 + drawing + 8.0 + 52.0 + 12.0 + 12.0;

    // How the remote says the PC is (as a phone is answered); before it has answered, as a PC with everything on.
    let none = Value::Null;
    let said = state.unwrap_or(&none);
    let flag = |name: &str| said.get(name).and_then(Value::as_bool);
    let night = flag("night") == Some(true);
    let (screens, lights) = (flag("screen"), flag("lights"));
    let (lights_there, colours_there) = (flag("lights_there").unwrap_or(true), flag("colours_there").unwrap_or(true));
    // A laptop whose lights are the one under its keys: called that.
    let keys_alone = flag("laptop") == Some(true) && said.get("keys").is_some_and(|keys| !keys.is_null());
    let text = |value: &Value, name: &str| value.get(name).and_then(Value::as_str).unwrap_or_default().to_string();
    let effect = said.get("light").map_or("steady".to_string(), |light| text(light, "effect"));
    let effects = [("steady", lang.pick("常亮", "Steady")), ("breathing", lang.pick("呼吸", "Breathing")), ("rainbow", lang.pick("彩虹", "Rainbow")), ("heat", lang.pick("跟随温度", "By heat"))];
    let effect_name = effects.iter().find(|(id, _)| *id == effect).map_or(effects[0].1, |(_, name)| *name);
    // A slider's place along its track: as it is held, else as the PC says.
    let along = |part: &Part, said: f32| held.filter(|(held, _)| *held == part).map_or(said, |(_, at)| at).clamp(0.0, 1.0);
    let (title, under, label, chip) = (Font::new(Family::Segoe, 16.0, 600.0), Font::new(Family::Segoe, 12.0, 400.0), Font::new(Family::Segoe, 12.5, 400.0), Font::new(Family::Segoe, 12.5, 600.0));
    let live = state.is_some();
    // A tile: lit or not, its name and what it says of itself; pressed, where the remote answers.
    let tile = |picture: &Picture, parts: &mut Vec<(Rect, Part)>, r: Rect, lit: bool, called: &str, how: &str, part: Part| {
        picture.plate(r, 20.0, if lit { LIT } else { CARD });
        picture.words(called, title, if lit { ACCENT } else { INK }, r.x + 16.0, r.y + r.h / 2.0 - 5.0, false);
        frame.text_wrapped(how, under, DIM, r.x + 16.0, r.y + r.h / 2.0 + 4.0, r.w - 32.0);
        if live {
            parts.extend(shown(r).map(|shown| (shown, part)));
        }
    };
    // A slider: its track, filled as far as it is along, and its knob.
    let slider = |picture: &Picture, parts: &mut Vec<(Rect, Part)>, x: f32, middle: f32, width: f32, at: f32, part: Part, enabled: bool| {
        picture.plate(Rect { x, y: middle - 2.5, w: width, h: 5.0 }, 2.5, TILE);
        picture.plate(Rect { x, y: middle - 2.5, w: width * at, h: 5.0 }, 2.5, if enabled { ACCENT } else { FAINT });
        unsafe { frame.dc.FillEllipse(&D2D1_ELLIPSE { point: Vector2 { X: x + width * at, Y: middle }, radiusX: 8.0, radiusY: 8.0 }, frame.brush(if enabled { INK } else { FAINT })) };
        if live && enabled {
            parts.extend(shown(Rect { x, y: middle - 14.0, w: width, h: 28.0 }).map(|shown| (shown, part)));
        }
    };

    // Night, the screens, the lights.
    tile(&picture, &mut parts, Rect { x: side, y, w: wide, h: 76.0 }, night, lang.pick("夜间模式", "Night mode"), if night { lang.pick("已开启，电脑仍在运行。再次点按即可结束", "On. The PC keeps running. Tap again to turn it off") } else { lang.pick("电脑保持运行，关闭屏幕与灯光、静音并切换至节能", "Keeps the PC running with screens and lights off, sound muted and power saving on") }, Part::Night);
    y += 76.0 + 12.0;
    let half = if lights_there { (wide - 12.0) / 2.0 } else { wide };
    tile(&picture, &mut parts, Rect { x: side, y, w: half, h: 88.0 }, screens != Some(false), lang.pick("屏幕", "Screens"), if screens == Some(false) { lang.pick("已关闭，点按开启", "Off. Tap to turn on") } else { lang.pick("已开启，点按关闭全部屏幕", "On. Tap to turn all off") }, Part::Screen);
    if lights_there {
        let how = match lights {
            Some(true) if colours_there => format!("{} · {effect_name}", lang.pick("已开启", "On")),
            Some(true) => lang.pick("已开启", "On").to_string(),
            Some(false) => lang.pick("已关闭，点按开启", "Off. Tap to turn on").to_string(),
            None => lang.pick("点按关闭灯光", "Tap to turn off").to_string(),
        };
        tile(&picture, &mut parts, Rect { x: side + half + 12.0, y, w: half, h: 88.0 }, lights != Some(false), if keys_alone { lang.pick("键盘背光", "Keyboard light") } else { lang.pick("灯光", "Lights") }, &how, Part::Lights);
    }
    y += 88.0 + 12.0;

    // The power modes, the one in use chosen.
    let modes: Vec<(String, String, bool)> = match said.get("power").and_then(Value::as_array) {
        Some(modes) => modes.iter().map(|mode| (text(mode, "id"), text(mode, "name"), mode.get("on").and_then(Value::as_bool) == Some(true))).collect(),
        None => vec![("efficiency".into(), String::new(), false), ("balanced".into(), String::new(), true), ("performance".into(), String::new(), false)],
    };
    if modes.len() > 1 {
        picture.plate(Rect { x: side, y, w: wide, h: 92.0 }, 20.0, CARD);
        picture.words(lang.pick("电源模式", "Power plan"), label, DIM, side + 16.0, y + 27.0, false);
        let mut x = side + 16.0;
        let _row = frame.clipped(side + 16.0, y, wide - 32.0, 92.0);
        for (id, called, chosen) in &modes {
            let called = match id.as_str() {
                "efficiency" => lang.pick("最佳能效", "Best efficiency"),
                "balanced" => lang.pick("平衡", "Balanced"),
                "performance" => lang.pick("最佳性能", "Best performance"),
                _ => called.as_str(),
            };
            let across = frame.gfx.measure(called, chip) + 28.0;
            let r = Rect { x, y: y + 42.0, w: across, h: 34.0 };
            picture.plate(r, 17.0, if *chosen { INK } else { TILE });
            picture.words(called, chip, if *chosen { PAGE } else { INK }, x + 14.0, y + 63.5, false);
            if live && !chosen {
                parts.extend(in_row(r).map(|shown| (shown, Part::Power(id.clone()))));
            }
            x += across + 8.0;
        }
        y += 92.0 + 12.0;
    }

    // The lights' colour, what they do, and how bright they are: while they are on.
    if colours_there && lights == Some(true) && !night {
        let light = said.get("light").cloned().unwrap_or(Value::Null);
        let now = text(&light, "color").to_uppercase();
        // (A line under the colours says how one is taken away, as the app says it there.)
        const SAYS: f32 = 22.0;
        let tall = SAYS + if making.is_some() { 196.0 + RING } else { 196.0 };
        // The colours kept to choose from (those it starts with, till any were kept), and before them the one in
        // use where it is none of them (while one is made, the one in use before).
        let kept = kept_colours(said);
        let mut colours = kept.clone();
        let own = making.map_or(&now, |making| &making.was);
        if own.len() == 6 && !colours.contains(own) {
            colours.insert(0, own.clone());
        }
        // In rows of as many as its width holds; after them, what opens the ring to make another.
        let (across, apart) = (((wide - 32.0) / SWATCH) as usize, SWATCH);
        let rows = (colours.len() + 1).div_ceil(across);
        let more = (rows - 1) as f32 * apart;
        let at = |place: usize| Vector2 { X: side + 16.0 + 18.0 + (place % across) as f32 * apart, Y: y + 62.0 + (place / across) as f32 * apart };
        picture.plate(Rect { x: side, y, w: wide, h: tall + more }, 20.0, CARD);
        picture.words(lang.pick("灯光效果与颜色", "Lighting effect and color"), label, DIM, side + 16.0, y + 27.0, false);
        // While colours are taken away, what ends that, at the card's other end.
        if removing {
            let word = lang.pick("完成", "Done");
            picture.words(word, label, ACCENT, side + wide - 16.0, y + 27.0, true);
            if live {
                let across = frame.gfx.measure(word, label);
                parts.extend(shown(Rect { x: side + wide - 16.0 - across - 8.0, y: y + 8.0, w: across + 16.0, h: 30.0 }).map(|shown| (shown, Part::Done)));
            }
        }
        let opens = at(colours.len());
        if let Some(hues) = hues(frame, opens, 26.0) {
            unsafe { frame.dc.DrawEllipse(&D2D1_ELLIPSE { point: opens, radiusX: 9.0, radiusY: 9.0 }, &hues, 8.0, None) };
        }
        if making.is_some() {
            unsafe { frame.dc.DrawEllipse(&D2D1_ELLIPSE { point: opens, radiusX: 16.75, radiusY: 16.75 }, frame.brush(INK), 2.5, None) };
        }
        if live && !removing {
            parts.extend(shown(Rect { x: opens.X - 18.0, y: opens.Y - 18.0, w: 36.0, h: 36.0 }).map(|shown| (shown, Part::Make)));
        }
        for (place, one) in colours.iter().enumerate() {
            let centre = at(place);
            // (Only one that is kept can be taken away.)
            let goes = removing && kept.contains(one);
            unsafe {
                if making.is_none() && !removing && *one == now && effect != "rainbow" {
                    frame.dc.DrawEllipse(&D2D1_ELLIPSE { point: centre, radiusX: 17.0, radiusY: 17.0 }, frame.brush(INK), 2.0, None);
                }
                frame.dc.FillEllipse(&D2D1_ELLIPSE { point: centre, radiusX: 13.0, radiusY: 13.0 }, frame.brush(colour(one)));
                if goes {
                    // A minus at its upper right corner.
                    let minus = Vector2 { X: centre.X + 10.0, Y: centre.Y - 10.0 };
                    frame.dc.FillEllipse(&D2D1_ELLIPSE { point: minus, radiusX: 8.5, radiusY: 8.5 }, frame.brush(CARD));
                    frame.dc.FillEllipse(&D2D1_ELLIPSE { point: minus, radiusX: 7.0, radiusY: 7.0 }, frame.brush(INK));
                    frame.dc.DrawLine(Vector2 { X: minus.X - 3.5, Y: minus.Y }, Vector2 { X: minus.X + 3.5, Y: minus.Y }, frame.brush(PAGE), 1.8, &picture.round);
                }
            }
            if live && (goes || !removing) {
                let part = if goes { Part::Remove(one.clone()) } else { Part::Colour(one.clone()) };
                parts.extend(shown(Rect { x: centre.X - 18.0, y: centre.Y - 18.0, w: 36.0, h: 36.0 }).map(|shown| (shown, part)));
            }
        }
        // How one is taken away, under the colours' last row; while they are, how.
        let says = if removing { lang.pick("点按带减号的颜色即可将其删除", "Tap a color with a minus to delete it") } else { lang.pick("长按颜色可删除", "Press and hold a color to delete it") };
        picture.words(says, label, FAINT, side + 16.0, y + more + 97.5, false);
        // (What follows is under them.)
        y += more + SAYS;
        let mut x = side + 16.0;
        let row = frame.clipped(side + 16.0, y + 96.0, wide - 32.0, 34.0);
        for (id, called) in effects {
            let across = frame.gfx.measure(called, chip) + 24.0;
            let r = Rect { x, y: y + 96.0, w: across, h: 34.0 };
            picture.plate(r, 17.0, if id == effect { INK } else { TILE });
            picture.words(called, chip, if id == effect { PAGE } else { INK }, x + 12.0, y + 117.5, false);
            if live && id != effect {
                parts.extend(in_row(r).map(|shown| (shown, Part::Effect(id))));
            }
            x += across + 8.0;
        }
        drop(row);
        // (Every colour in turn has its own brightness: not slid.)
        let bright = light.get("brightness").and_then(Value::as_f64).unwrap_or(100.0) as f32;
        let at = along(&Part::Brightness, (bright - 5.0) / 95.0);
        // (Each slider after its name, the longer of the two names: they start where the other's does.)
        let (brightness, saturation) = (lang.pick("亮度", "Brightness"), lang.pick("饱和度", "Saturation"));
        let named = 16.0 + frame.gfx.measure(brightness, label).max(frame.gfx.measure(saturation, label)).ceil() + 12.0;
        picture.words(brightness, label, DIM, side + 16.0, y + 168.0, false);
        slider(&picture, &mut parts, side + named, y + 164.0, wide - named - 16.0 - 44.0, at, Part::Brightness, effect != "rainbow");
        picture.words(&format!("{:.0}%", 5.0 + at * 95.0), label, DIM, side + wide - 16.0, y + 168.0, true);
        // The ring: every hue round it, the colour itself in its middle; under it how much colour, then given up or kept.
        if let Some(making) = making {
            let (band, across) = (26.0, 196.0);
            let centre = Vector2 { X: side + wide / 2.0, Y: y + 196.0 + 4.0 + across / 2.0 };
            let middle = across / 2.0 - band / 2.0;
            let on = |angle: f32, r: f32| Vector2 { X: centre.X + r * angle.cos(), Y: centre.Y + r * angle.sin() };
            if let Some(hues) = hues(frame, centre, across) {
                unsafe { frame.dc.DrawEllipse(&D2D1_ELLIPSE { point: centre, radiusX: middle, radiusY: middle }, &hues, band, None) };
            }
            let hue = held.filter(|(held, _)| **held == Part::Hue).map_or(making.hue, |(_, at)| at * 360.0);
            let depth = along(&Part::Depth, making.depth);
            let made = Making { was: String::new(), hue, depth };
            let knob = on(hue.to_radians(), middle);
            unsafe {
                frame.dc.FillEllipse(&D2D1_ELLIPSE { point: centre, radiusX: across / 2.0 - 42.0, radiusY: across / 2.0 - 42.0 }, frame.brush(hued(hue, depth)));
                frame.dc.FillEllipse(&D2D1_ELLIPSE { point: Vector2 { X: knob.X, Y: knob.Y + 1.0 }, radiusX: 17.0, radiusY: 17.0 }, frame.brush(Color::hex(0x000000, 0.6)));
                frame.dc.FillEllipse(&D2D1_ELLIPSE { point: knob, radiusX: 16.0, radiusY: 16.0 }, frame.brush(Color::hex(0xFFFFFF, 1.0)));
                frame.dc.FillEllipse(&D2D1_ELLIPSE { point: knob, radiusX: 13.0, radiusY: 13.0 }, frame.brush(hued(hue, 1.0)));
            }
            let ink = if depth < 0.75 { Color::hex(0x000000, 1.0) } else { Color::hex(0xFFFFFF, 1.0) };
            picture.middled(&format!("#{}", made.hex()), Font::new(Family::Segoe, 15.0, 600.0), ink, centre.X, centre.Y + 1.0);
            picture.middled(lang.pick("当前颜色", "Current color"), Font::new(Family::Segoe, 11.0, 400.0), Color { a: 0.75, ..ink }, centre.X, centre.Y + 18.0);
            if live {
                parts.extend(shown(Rect { x: centre.X - across / 2.0, y: centre.Y - across / 2.0, w: across, h: across }).map(|shown| (shown, Part::Hue)));
            }
            // How much colour: from none of it to all of it.
            let row = centre.Y + across / 2.0 + 16.0 + 14.0;
            let (x, width) = (side + named, wide - named - 16.0 - 44.0);
            picture.words(saturation, label, DIM, side + 16.0, row + 4.0, false);
            if let Some(fading) = frame.gradient(Vector2 { X: x, Y: 0.0 }, hued(hue, 0.0), Vector2 { X: x + width, Y: 0.0 }, hued(hue, 1.0)) {
                unsafe { frame.dc.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: rect(x, row - 4.0, width, 8.0), radiusX: 4.0, radiusY: 4.0 }, &fading) };
            }
            unsafe { frame.dc.FillEllipse(&D2D1_ELLIPSE { point: Vector2 { X: x + width * depth, Y: row }, radiusX: 8.0, radiusY: 8.0 }, frame.brush(INK)) };
            if live {
                parts.extend(shown(Rect { x, y: row - 14.0, w: width, h: 28.0 }).map(|shown| (shown, Part::Depth)));
            }
            picture.words(&format!("{:.0}%", depth * 100.0), label, INK, side + wide - 16.0, row + 4.0, true);
            let (top, each) = (row + 14.0 + 16.0, (wide - 32.0 - 12.0) / 2.0);
            for (place, (word, part)) in [(lang.pick("取消", "Cancel"), Part::GiveUp), (lang.pick("使用此颜色", "Use this color"), Part::Use)].into_iter().enumerate() {
                let r = Rect { x: side + 16.0 + place as f32 * (each + 12.0), y: top, w: each, h: 44.0 };
                let kept = part == Part::Use;
                picture.plate(r, 14.0, if kept { INK } else { TILE });
                picture.middled(word, Font::new(Family::Segoe, 15.0, if kept { 600.0 } else { 400.0 }), if kept { PAGE } else { INK }, r.x + r.w / 2.0, r.y + 27.0);
                if live {
                    parts.extend(shown(r).map(|shown| (shown, part)));
                }
            }
        }
        y += tall + 12.0;
    }

    // A laptop's keys: how bright the light under them is, with the lights.
    if let Some(keys) = said.get("keys").filter(|keys| !keys.is_null() && lights != Some(false) && !night) {
        let (level, most) = (keys.get("level").and_then(Value::as_u64), keys.get("most").and_then(Value::as_u64).unwrap_or(3));
        picture.plate(Rect { x: side, y, w: wide, h: 92.0 }, 20.0, CARD);
        picture.words(if keys_alone { lang.pick("背光亮度", "Backlight") } else { lang.pick("键盘背光", "Keyboard light") }, label, DIM, side + 16.0, y + 27.0, false);
        // Out, then each step up to the brightest: named where there are the usual three, numbered otherwise.
        let names: Vec<String> = if most == 3 {
            [lang.pick("关", "Off"), lang.pick("低", "Low"), lang.pick("中", "Medium"), lang.pick("高", "High")].iter().map(|name| name.to_string()).collect()
        } else {
            std::iter::once(lang.pick("关", "Off").to_string()).chain((1..=most).map(|step| step.to_string())).collect()
        };
        let mut x = side + 16.0;
        let _row = frame.clipped(side + 16.0, y, wide - 32.0, 92.0);
        for (step, called) in names.iter().enumerate() {
            let across = frame.gfx.measure(called, chip) + 28.0;
            let r = Rect { x, y: y + 42.0, w: across, h: 34.0 };
            let chosen = level == Some(step as u64);
            picture.plate(r, 17.0, if chosen { INK } else { TILE });
            picture.words(called, chip, if chosen { PAGE } else { INK }, x + 14.0, y + 63.5, false);
            if live && !chosen {
                parts.extend(in_row(r).map(|shown| (shown, Part::Keys(step as u8))));
            }
            x += across + 8.0;
        }
        y += 92.0 + 12.0;
    }

    // Each monitor by itself, where there are several.
    let monitors = said.get("monitors").and_then(Value::as_array).cloned().unwrap_or_default();
    if monitors.len() > 1 {
        let rows: f32 = monitors.iter().map(|monitor| if monitor.get("brightness").is_some_and(|b| !b.is_null()) { 82.0 } else { 52.0 }).sum();
        picture.plate(Rect { x: side, y, w: wide, h: 40.0 + rows + 8.0 }, 20.0, CARD);
        picture.words(lang.pick("显示器", "Monitors"), label, DIM, side + 16.0, y + 27.0, false);
        let mut row = y + 40.0;
        for (index, monitor) in monitors.iter().enumerate() {
            let is_on = monitor.get("on").and_then(Value::as_bool) != Some(false) && screens != Some(false);
            let brightness = monitor.get("brightness").and_then(Value::as_f64);
            picture.plate(Rect { x: side + 16.0, y: row + 6.0, w: 24.0, h: 24.0 }, 7.0, TILE);
            picture.words(&(index + 1).to_string(), Font::new(Family::Segoe, 12.0, 600.0), DIM, side + 28.0 - frame.gfx.measure(&(index + 1).to_string(), Font::new(Family::Segoe, 12.0, 600.0)) / 2.0, row + 22.5, false);
            picture.words(&text(monitor, "name"), Font::new(Family::Segoe, 14.0, 500.0), INK, side + 50.0, row + 17.0, false);
            let size = format!("{} × {}", monitor.get("width").and_then(Value::as_i64).unwrap_or(0), monitor.get("height").and_then(Value::as_i64).unwrap_or(0));
            let part = Part::MonitorBrightness(index);
            let at = along(&part, brightness.unwrap_or(0.0) as f32 / 100.0);
            let detail = if brightness.is_some() { format!("{size} · {} {:.0}%", lang.pick("亮度", "brightness"), at * 100.0) } else { size };
            picture.words(&detail, under, DIM, side + 50.0, row + 36.0, false);
            // Its switch.
            let switch = Rect { x: side + wide - 16.0 - 44.0, y: row + 7.0, w: 44.0, h: 26.0 };
            picture.plate(switch, 13.0, if is_on { ACCENT } else { TILE });
            unsafe { frame.dc.FillEllipse(&D2D1_ELLIPSE { point: Vector2 { X: if is_on { switch.x + 31.0 } else { switch.x + 13.0 }, Y: switch.y + 13.0 }, radiusX: 9.0, radiusY: 9.0 }, frame.brush(INK)) };
            if live && screens != Some(false) {
                parts.extend(shown(Rect { x: switch.x - 6.0, y: switch.y - 6.0, w: 56.0, h: 38.0 }).map(|shown| (shown, Part::Monitor(index))));
            }
            row += 52.0;
            if brightness.is_some() {
                slider(&picture, &mut parts, side + 50.0, row + 8.0, wide - 50.0 - 16.0 - 8.0, at, part, is_on);
                row += 30.0;
            }
        }
        y += 40.0 + rows + 8.0 + 12.0;
    }

    // The sound: how loud, and what is playing.
    if let Some(volume) = said.get("volume").and_then(Value::as_f64) {
        let muted = flag("muted") == Some(true);
        let media = said.get("media").filter(|media| !media.is_null());
        let tall = if media.is_some() { 152.0 } else { 92.0 };
        picture.plate(Rect { x: side, y, w: wide, h: tall }, 20.0, CARD);
        picture.words(lang.pick("声音", "Sound"), label, DIM, side + 16.0, y + 27.0, false);
        let word = if muted { lang.pick("取消静音", "Unmute") } else { lang.pick("静音", "Mute") };
        picture.words(word, label, if muted { ACCENT } else { DIM }, side + wide - 16.0, y + 27.0, true);
        if live {
            let across = frame.gfx.measure(word, label);
            parts.extend(shown(Rect { x: side + wide - 16.0 - across - 8.0, y: y + 8.0, w: across + 16.0, h: 30.0 }).map(|shown| (shown, Part::Mute)));
        }
        let at = along(&Part::Volume, volume as f32 / 100.0);
        slider(&picture, &mut parts, side + 16.0, y + 62.0, wide - 32.0 - 52.0, at, Part::Volume, true);
        picture.words(&if muted { lang.pick("静音", "Muted").to_string() } else { format!("{:.0}%", at * 100.0) }, Font::new(Family::Segoe, 14.0, 600.0), INK, side + wide - 16.0, y + 67.0, true);
        if let Some(media) = media {
            picture.plate(Rect { x: side + 16.0, y: y + 88.0, w: wide - 32.0, h: 1.0 }, 0.0, TILE);
            let playing = media.get("playing").and_then(Value::as_bool) == Some(true);
            let (title_said, app) = (text(media, "title"), text(media, "app"));
            let about = [if playing { lang.pick("正在播放", "Playing") } else { lang.pick("已暂停", "Paused") }.to_string(), text(media, "artist"), if title_said.is_empty() { String::new() } else { app.clone() }];
            // (As wide as the room before the three buttons.)
            unsafe { frame.dc.PushAxisAlignedClip(&rect(side + 16.0, y + 92.0, wide - 32.0 - 124.0, 56.0), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
            picture.words(if title_said.is_empty() { &app } else { &title_said }, Font::new(Family::Segoe, 14.0, 500.0), INK, side + 16.0, y + 116.0, false);
            picture.words(&about.iter().filter(|one| !one.is_empty()).cloned().collect::<Vec<_>>().join(" · "), under, DIM, side + 16.0, y + 135.0, false);
            unsafe { frame.dc.PopAxisAlignedClip() };
            // Before, play or pause, after: each a mark of its own.
            let middle = y + 120.0;
            for (place, what) in ["previous", "toggle", "next"].into_iter().enumerate() {
                let centre = side + wide - 16.0 - 100.0 + place as f32 * 40.0 + 10.0;
                let mark = |points: &[(f32, f32)]| {
                    let points: Vec<super::canvas::Point> = points.iter().map(|(x, y)| super::canvas::Point { x: centre + x, y: middle + y }).collect();
                    frame.fill_shape(&points, super::canvas::Fill::Solid(INK));
                };
                match what {
                    "previous" => {
                        mark(&[(6.0, -7.0), (6.0, 7.0), (-4.0, 0.0)]);
                        picture.plate(Rect { x: centre - 7.0, y: middle - 7.0, w: 2.5, h: 14.0 }, 1.0, INK);
                    }
                    "next" => {
                        mark(&[(-6.0, -7.0), (-6.0, 7.0), (4.0, 0.0)]);
                        picture.plate(Rect { x: centre + 4.5, y: middle - 7.0, w: 2.5, h: 14.0 }, 1.0, INK);
                    }
                    _ => {
                        picture.plate(Rect { x: centre - 17.0, y: middle - 17.0, w: 34.0, h: 34.0 }, 17.0, TILE);
                        if playing {
                            picture.plate(Rect { x: centre - 6.0, y: middle - 7.0, w: 4.0, h: 14.0 }, 1.0, INK);
                            picture.plate(Rect { x: centre + 2.0, y: middle - 7.0, w: 4.0, h: 14.0 }, 1.0, INK);
                        } else {
                            mark(&[(-4.0, -8.0), (-4.0, 8.0), (8.0, 0.0)]);
                        }
                    }
                }
                if live {
                    parts.extend(shown(Rect { x: centre - 19.0, y: middle - 19.0, w: 38.0, h: 38.0 }).map(|shown| (shown, Part::Media(what))));
                }
            }
        }
        y += tall + 12.0;
    }

    // Sleep says what it means where it is pressed (and is asked about again before it is done).
    tile(&picture, &mut parts, Rect { x: side, y, w: wide, h: 96.0 }, false, lang.pick("睡眠", "Sleep"), lang.pick("最省电，手机无法唤醒", "Uses the least power. The phone can't wake it"), Part::Sleep);
    y += 96.0 + 24.0;

    unsafe { frame.dc.PopAxisAlignedClip() };
    unsafe { frame.dc.PopLayer() };
    drop(std::mem::ManuallyDrop::into_inner(layer.geometricMask));
    frame.origin(0.0, 0.0);
    Painted { parts, tall: y }
}
