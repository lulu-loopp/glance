//! One layout turning into another, as a slide show's morph does: each
//! drawing recorded as named parts (the CPU's figure, the network's chart),
//! and between two drawings each part they share moved and resized from the
//! one to the other, what only one has faded in or out where it is.

use std::cell::RefCell;

use super::canvas::{Align, Canvas, Color, Fill, Font, Point};
use super::icons::Icon;

/// One thing drawn, in the coordinates it was recorded at.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// `inked`: how wide the text drew (for where it is); `scale`: how much
    /// larger than its font it is drawn (moving between two sizes, it is
    /// drawn larger or smaller rather than set in a font for each).
    Text { text: String, font: Font, color: Color, x: f32, y: f32, width: f32, align: Align, inked: f32, scale: f32 },
    Clip { x: f32, y: f32, w: f32, h: f32 },
    Unclip,
    Fill { color: Color, x: f32, y: f32, w: f32, h: f32 },
    Rounded { color: Color, x: f32, y: f32, w: f32, h: f32, radius: f32 },
    Circle { color: Color, centre: Point, radius: f32 },
    Stroke { points: Vec<Point>, color: Color, width: f32, dashed: bool },
    Shape { points: Vec<Point>, fill: Fill },
    Outline { fill: Fill, x: f32, y: f32, w: f32, h: f32, radius: f32, line: f32 },
    Shadow { color: Color, x: f32, y: f32, w: f32, h: f32, radius: f32, blur: f32, drop: f32 },
    Arc { centre: Point, radius: f32, from: f32, sweep: f32, color: Color, width: f32 },
    Icon { icon: Icon, centre: Point, size: f32, color: Color },
}

/// A part of a drawing: what was drawn under one name, or one thing drawn
/// under none.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub key: Option<String>,
    pub ops: Vec<Op>,
}

/// A box: left, top, width, height.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Bounds {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Bounds {
    fn lerp(self, to: Bounds, t: f32) -> Bounds {
        Bounds { x: lerp(self.x, to.x, t), y: lerp(self.y, to.y, t), w: lerp(self.w, to.w, t), h: lerp(self.h, to.h, t) }
    }

    fn union(self, other: Bounds) -> Bounds {
        let (x, y) = (self.x.min(other.x), self.y.min(other.y));
        Bounds { x, y, w: (self.x + self.w).max(other.x + other.w) - x, h: (self.y + self.h).max(other.y + other.h) - y }
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    Color { r: lerp(a.r, b.r, t), g: lerp(a.g, b.g, t), b: lerp(a.b, b.b, t), a: lerp(a.a, b.a, t) }
}

fn lerp_point(a: Point, b: Point, t: f32) -> Point {
    Point { x: lerp(a.x, b.x, t), y: lerp(a.y, b.y, t) }
}

fn lerp_fill(a: Fill, b: Fill, t: f32) -> Fill {
    match (a, b) {
        (Fill::Solid(a), Fill::Solid(b)) => Fill::Solid(lerp_color(a, b, t)),
        (Fill::Down { top: at, from: af, bottom: ab, to: ato }, Fill::Down { top: bt, from: bf, bottom: bb, to: bto }) => {
            Fill::Down { top: lerp(at, bt, t), from: lerp_color(af, bf, t), bottom: lerp(ab, bb, t), to: lerp_color(ato, bto, t) }
        }
        (_, b) => b,
    }
}

/// From one box to another: each point scaled about the first's corner,
/// then moved to the second's; sizes (a font's, a line's width, a ring's
/// radius) scaled by `sizes`, so nothing is drawn squashed: a whole drawing
/// scaled evenly scales them with it, a part moved into another's box
/// keeps its own.
#[derive(Clone, Copy)]
struct Map {
    from: Bounds,
    to: Bounds,
    sizes: f32,
}

impl Map {
    fn sx(&self) -> f32 {
        if self.from.w > 0.01 { self.to.w / self.from.w } else { 1.0 }
    }

    fn sy(&self) -> f32 {
        if self.from.h > 0.01 { self.to.h / self.from.h } else { 1.0 }
    }

    fn point(&self, p: Point) -> Point {
        Point { x: self.to.x + (p.x - self.from.x) * self.sx(), y: self.to.y + (p.y - self.from.y) * self.sy() }
    }

    fn length(&self, l: f32) -> f32 {
        l * self.sizes
    }
}

impl Op {
    /// Where it draws; none for a clip's end.
    fn bounds(&self) -> Option<Bounds> {
        let of_points = |points: &[Point]| {
            let first = points.first()?;
            let b = points.iter().fold(Bounds { x: first.x, y: first.y, w: 0.0, h: 0.0 }, |b, p| b.union(Bounds { x: p.x, y: p.y, w: 0.0, h: 0.0 }));
            Some(b)
        };
        Some(match self {
            Op::Text { font, x, y, width, align, inked, scale, .. } => {
                let left = if *align == Align::End { x + width - inked } else { *x };
                Bounds { x: left, y: *y, w: *inked, h: font.size * scale * 1.25 }
            }
            Op::Clip { x, y, w, h } | Op::Fill { x, y, w, h, .. } | Op::Rounded { x, y, w, h, .. } | Op::Outline { x, y, w, h, .. } | Op::Shadow { x, y, w, h, .. } => {
                Bounds { x: *x, y: *y, w: *w, h: *h }
            }
            Op::Circle { centre, radius, .. } | Op::Arc { centre, radius, .. } => Bounds { x: centre.x - radius, y: centre.y - radius, w: 2.0 * radius, h: 2.0 * radius },
            Op::Icon { centre, size, .. } => Bounds { x: centre.x - size / 2.0, y: centre.y - size / 2.0, w: *size, h: *size },
            Op::Stroke { points, .. } | Op::Shape { points, .. } => of_points(points)?,
            Op::Unclip => return None,
        })
    }

    /// Drawn `alpha` as strong.
    fn faded(mut self, alpha: f32) -> Op {
        let fade = |c: &mut Color| *c = c.alpha(alpha);
        let fade_fill = |f: &mut Fill| match f {
            Fill::Solid(c) => *c = c.alpha(alpha),
            Fill::Down { from, to, .. } => {
                *from = from.alpha(alpha);
                *to = to.alpha(alpha);
            }
        };
        match &mut self {
            Op::Text { color, .. } | Op::Fill { color, .. } | Op::Rounded { color, .. } | Op::Circle { color, .. } | Op::Stroke { color, .. } => fade(color),
            Op::Shadow { color, .. } | Op::Arc { color, .. } | Op::Icon { color, .. } => fade(color),
            Op::Shape { fill, .. } | Op::Outline { fill, .. } => fade_fill(fill),
            Op::Clip { .. } | Op::Unclip => {}
        }
        self
    }

    /// Whether it shows at all: faded out past what a pixel can show, it
    /// does not (a clip, which shows nothing itself, always counts).
    fn shows(&self) -> bool {
        const LEAST: f32 = 0.5 / 255.0;
        let fill = |f: &Fill| match f {
            Fill::Solid(c) => c.a >= LEAST,
            Fill::Down { from, to, .. } => from.a >= LEAST || to.a >= LEAST,
        };
        match self {
            Op::Text { color, .. } | Op::Fill { color, .. } | Op::Rounded { color, .. } | Op::Circle { color, .. } | Op::Stroke { color, .. } => color.a >= LEAST,
            Op::Shadow { color, .. } | Op::Arc { color, .. } | Op::Icon { color, .. } => color.a >= LEAST,
            Op::Shape { fill: f, .. } | Op::Outline { fill: f, .. } => fill(f),
            Op::Clip { .. } | Op::Unclip => true,
        }
    }

    /// Moved from one box to another (see `Map`).
    fn mapped(&self, map: &Map) -> Op {
        let rect = |x: f32, y: f32, w: f32, h: f32| {
            let p = map.point(Point { x, y });
            (p.x, p.y, w * map.sx(), h * map.sy())
        };
        match self.clone() {
            Op::Text { text, font, color, x, y, width, align, inked, scale } => {
                // Its box's start kept where the map puts it, its size kept true.
                let start = map.point(Point { x: if align == Align::End { x + width - inked } else { x }, y });
                let inked = map.length(inked);
                let width = width * map.sx();
                let x = if align == Align::End { start.x + inked - width } else { start.x };
                Op::Text { text, font, color, x, y: start.y, width, align, inked, scale: map.length(scale) }
            }
            Op::Clip { x, y, w, h } => {
                let (x, y, w, h) = rect(x, y, w, h);
                Op::Clip { x, y, w, h }
            }
            Op::Unclip => Op::Unclip,
            Op::Fill { color, x, y, w, h } => {
                let (x, y, w, h) = rect(x, y, w, h);
                Op::Fill { color, x, y, w, h }
            }
            Op::Rounded { color, x, y, w, h, radius } => {
                let (x, y, w, h) = rect(x, y, w, h);
                Op::Rounded { color, x, y, w, h, radius: map.length(radius) }
            }
            Op::Outline { fill, x, y, w, h, radius, line } => {
                let (x, y, w, h) = rect(x, y, w, h);
                Op::Outline { fill, x, y, w, h, radius: map.length(radius), line: map.length(line) }
            }
            Op::Shadow { color, x, y, w, h, radius, blur, drop } => {
                let (x, y, w, h) = rect(x, y, w, h);
                Op::Shadow { color, x, y, w, h, radius: map.length(radius), blur: map.length(blur), drop: map.length(drop) }
            }
            Op::Circle { color, centre, radius } => Op::Circle { color, centre: map.point(centre), radius: map.length(radius) },
            Op::Arc { centre, radius, from, sweep, color, width } => Op::Arc { centre: map.point(centre), radius: map.length(radius), from, sweep, color, width: map.length(width) },
            Op::Icon { icon, centre, size, color } => Op::Icon { icon, centre: map.point(centre), size: map.length(size), color },
            Op::Stroke { points, color, width, dashed } => Op::Stroke { points: points.iter().map(|p| map.point(*p)).collect(), color, width: map.length(width), dashed },
            Op::Shape { points, fill } => {
                let fill = match fill {
                    Fill::Down { top, from, bottom, to } => Fill::Down { top: map.point(Point { x: 0.0, y: top }).y, from, bottom: map.point(Point { x: 0.0, y: bottom }).y, to },
                    solid => solid,
                };
                Op::Shape { points: points.iter().map(|p| map.point(*p)).collect(), fill }
            }
        }
    }

    /// Between `self` (at 0) and `to` (at 1), where the two are the same
    /// thing at two places and sizes: none where they are not. Words that
    /// changed fade from the old to the new on the box moving between.
    fn between(&self, to: &Op, t: f32) -> Option<Vec<Op>> {
        Some(match (self, to) {
            (
                Op::Text { text: at, font: af, color: ac, x: ax, y: ay, width: aw, align: aa, inked: ai, scale: asc },
                Op::Text { text: bt, font: bf, color: bc, x: bx, y: by, width: bw, align: ba, inked: bi, scale: bsc },
            ) if aa == ba => {
                let (x, y, width) = (lerp(*ax, *bx, t), lerp(*ay, *by, t), lerp(*aw, *bw, t));
                // The size it has on the way, each drawn at its own font
                // scaled to it.
                let size = lerp(af.size * asc, bf.size * bsc, t);
                if at == bt && af.family == bf.family && af.weight == bf.weight {
                    // In the font of the end it is nearer: either end drawn as it is.
                    let (color, inked) = (lerp_color(*ac, *bc, t), lerp(*ai, *bi, t));
                    let font = if t < 0.5 { *af } else { *bf };
                    vec![Op::Text { text: bt.clone(), font, color, x, y, width, align: *ba, inked, scale: size / font.size }]
                } else {
                    // Both on the box moving between, at the size it has there.
                    let old = Op::Text { text: at.clone(), font: *af, color: *ac, x, y, width, align: *aa, inked: *ai, scale: size / af.size };
                    let new = Op::Text { text: bt.clone(), font: *bf, color: *bc, x, y, width, align: *ba, inked: *bi, scale: size / bf.size };
                    vec![old.faded(1.0 - t), new.faded(t)]
                }
            }
            (Op::Clip { x: ax, y: ay, w: aw, h: ah }, Op::Clip { x: bx, y: by, w: bw, h: bh }) => {
                vec![Op::Clip { x: lerp(*ax, *bx, t), y: lerp(*ay, *by, t), w: lerp(*aw, *bw, t), h: lerp(*ah, *bh, t) }]
            }
            (Op::Unclip, Op::Unclip) => vec![Op::Unclip],
            (Op::Fill { color: ac, x: ax, y: ay, w: aw, h: ah }, Op::Fill { color: bc, x: bx, y: by, w: bw, h: bh }) => {
                vec![Op::Fill { color: lerp_color(*ac, *bc, t), x: lerp(*ax, *bx, t), y: lerp(*ay, *by, t), w: lerp(*aw, *bw, t), h: lerp(*ah, *bh, t) }]
            }
            (Op::Rounded { color: ac, x: ax, y: ay, w: aw, h: ah, radius: ar }, Op::Rounded { color: bc, x: bx, y: by, w: bw, h: bh, radius: br }) => vec![Op::Rounded {
                color: lerp_color(*ac, *bc, t),
                x: lerp(*ax, *bx, t),
                y: lerp(*ay, *by, t),
                w: lerp(*aw, *bw, t),
                h: lerp(*ah, *bh, t),
                radius: lerp(*ar, *br, t),
            }],
            (Op::Circle { color: ac, centre: acen, radius: ar }, Op::Circle { color: bc, centre: bcen, radius: br }) => {
                vec![Op::Circle { color: lerp_color(*ac, *bc, t), centre: lerp_point(*acen, *bcen, t), radius: lerp(*ar, *br, t) }]
            }
            (Op::Arc { centre: acen, radius: ar, from: af, sweep: asw, color: ac, width: aw }, Op::Arc { centre: bcen, radius: br, from: bf, sweep: bsw, color: bc, width: bw }) => {
                vec![Op::Arc {
                    centre: lerp_point(*acen, *bcen, t),
                    radius: lerp(*ar, *br, t),
                    from: lerp(*af, *bf, t),
                    sweep: lerp(*asw, *bsw, t),
                    color: lerp_color(*ac, *bc, t),
                    width: lerp(*aw, *bw, t),
                }]
            }
            (Op::Icon { icon: ai, centre: ac, size: asz, color: acol }, Op::Icon { icon: bi, centre: bc, size: bsz, color: bcol }) if ai == bi => {
                vec![Op::Icon { icon: *bi, centre: lerp_point(*ac, *bc, t), size: lerp(*asz, *bsz, t), color: lerp_color(*acol, *bcol, t) }]
            }
            (Op::Stroke { points: ap, color: ac, width: aw, dashed: ad }, Op::Stroke { points: bp, color: bc, width: bw, dashed: bd }) if ad == bd => {
                let color = lerp_color(*ac, *bc, t);
                let width = lerp(*aw, *bw, t);
                if ap.len() == bp.len() {
                    vec![Op::Stroke { points: ap.iter().zip(bp).map(|(a, b)| lerp_point(*a, *b, t)).collect(), color, width, dashed: *bd }]
                } else {
                    // The same line, sampled otherwise: the new one, on its way
                    // from where the old one was.
                    let (from, to) = (self.bounds()?, to.bounds()?);
                    let map = Map { from: to, to: from.lerp(to, t), sizes: 1.0 };
                    vec![Op::Stroke { points: bp.iter().map(|p| map.point(*p)).collect(), color, width, dashed: *bd }]
                }
            }
            (Op::Shape { points: ap, fill: af }, Op::Shape { points: bp, fill: bf }) => {
                let fill = lerp_fill(*af, *bf, t);
                if ap.len() == bp.len() {
                    vec![Op::Shape { points: ap.iter().zip(bp).map(|(a, b)| lerp_point(*a, *b, t)).collect(), fill }]
                } else {
                    let (from, to) = (self.bounds()?, to.bounds()?);
                    let map = Map { from: to, to: from.lerp(to, t), sizes: 1.0 };
                    vec![Op::Shape { points: bp.iter().map(|p| map.point(*p)).collect(), fill }]
                }
            }
            (Op::Outline { fill: af, x: ax, y: ay, w: aw, h: ah, radius: ar, line: al }, Op::Outline { fill: bf, x: bx, y: by, w: bw, h: bh, radius: br, line: bl }) => {
                vec![Op::Outline {
                    fill: lerp_fill(*af, *bf, t),
                    x: lerp(*ax, *bx, t),
                    y: lerp(*ay, *by, t),
                    w: lerp(*aw, *bw, t),
                    h: lerp(*ah, *bh, t),
                    radius: lerp(*ar, *br, t),
                    line: lerp(*al, *bl, t),
                }]
            }
            _ => return None,
        })
    }

    fn draw(&self, canvas: &dyn Canvas) {
        match self {
            Op::Text { text, font, color, x, y, width, align, scale, .. } if (scale - 1.0).abs() < 1e-3 => canvas.text(text, *font, *color, *x, *y, *width, *align),
            Op::Text { text, font, color, x, y, width, align, scale, .. } => canvas.text_scaled(text, *font, *color, *x, *y, *width, *align, *scale),
            Op::Clip { x, y, w, h } => canvas.clip(*x, *y, *w, *h),
            Op::Unclip => canvas.unclip(),
            Op::Fill { color, x, y, w, h } => canvas.fill(*color, *x, *y, *w, *h),
            Op::Rounded { color, x, y, w, h, radius } => canvas.fill_rounded(*color, *x, *y, *w, *h, *radius),
            Op::Circle { color, centre, radius } => canvas.fill_circle(*color, *centre, *radius),
            Op::Stroke { points, color, width, dashed: false } => canvas.stroke(points, *color, *width),
            Op::Stroke { points, color, width, dashed: true } => canvas.stroke_dashed(points, *color, *width),
            Op::Shape { points, fill } => canvas.fill_shape(points, *fill),
            Op::Outline { fill, x, y, w, h, radius, line } => canvas.stroke_rounded(*fill, *x, *y, *w, *h, *radius, *line),
            Op::Shadow { color, x, y, w, h, radius, blur, drop } => canvas.shadow(*color, *x, *y, *w, *h, *radius, *blur, *drop),
            Op::Arc { centre, radius, from, sweep, color, width } => canvas.arc(*centre, *radius, *from, *sweep, *color, *width),
            Op::Icon { icon, centre, size, color } => canvas.icon(*icon, *centre, *size, *color),
        }
    }
}

impl Part {
    fn bounds(&self) -> Option<Bounds> {
        self.ops.iter().filter_map(Op::bounds).reduce(Bounds::union)
    }

    /// What of it shows (see `Op::shows`), clips left round nothing
    /// dropped; nothing, if none of it does. A turn broken off by another
    /// starts from what showed, so what has faded out is not carried on
    /// from turn to turn.
    fn shown(self) -> Option<Part> {
        let mut ops: Vec<Op> = Vec::with_capacity(self.ops.len());
        for op in self.ops.into_iter().filter(Op::shows) {
            if matches!(op, Op::Unclip) && matches!(ops.last(), Some(Op::Clip { .. })) {
                ops.pop();
                continue;
            }
            ops.push(op);
        }
        ops.iter().any(|op| !matches!(op, Op::Clip { .. } | Op::Unclip)).then_some(Part { key: self.key, ops })
    }

    fn faded(&self, alpha: f32) -> Part {
        Part { key: self.key.clone(), ops: self.ops.iter().map(|op| op.clone().faded(alpha)).collect() }
    }

    fn mapped(&self, map: &Map) -> Part {
        Part { key: self.key.clone(), ops: self.ops.iter().map(|op| op.mapped(map)).collect() }
    }

    /// Between `self` (at 0) and `to` (at 1): thing by thing where the two
    /// are drawn alike; else the whole of each on its way from its box to
    /// the other's, the one fading out as the other fades in.
    fn between(&self, to: &Part, t: f32) -> Part {
        if self.ops.len() == to.ops.len() {
            let alike: Option<Vec<Vec<Op>>> = self.ops.iter().zip(&to.ops).map(|(a, b)| a.between(b, t)).collect();
            if let Some(ops) = alike {
                return Part { key: to.key.clone(), ops: ops.into_iter().flatten().collect() };
            }
        }
        let (Some(from), Some(into)) = (self.bounds(), to.bounds()) else { return to.faded(t) };
        let midway = from.lerp(into, t);
        let mut ops = self.mapped(&Map { from, to: midway, sizes: 1.0 }).faded(1.0 - t).ops;
        ops.extend(to.mapped(&Map { from: into, to: midway, sizes: 1.0 }).faded(t).ops);
        Part { key: to.key.clone(), ops }
    }
}

/// The drawing between `from` (at 0) and `to` (at 1): each part `to` has
/// on its way from where `from` had it, or fading in; each part only
/// `from` had fading out where it was.
pub fn between(from: &[Part], to: &[Part], t: f32) -> Vec<Part> {
    let t = t.clamp(0.0, 1.0);
    let mut used = vec![false; from.len()];
    let mut parts: Vec<Part> = to
        .iter()
        .map(|part| {
            let mine = part.key.as_ref().and_then(|key| (0..from.len()).find(|&i| !used[i] && from[i].key.as_ref() == Some(key)));
            match mine {
                Some(i) => {
                    used[i] = true;
                    from[i].between(part, t)
                }
                None => part.faded(t),
            }
        })
        .collect();
    parts.extend(from.iter().zip(&used).filter(|(_, used)| !**used).map(|(part, _)| part.faded(1.0 - t)));
    parts.into_iter().filter_map(Part::shown).collect()
}

/// Draws `parts`, every clip ended.
pub fn draw(canvas: &dyn Canvas, parts: &[Part]) {
    let mut clips = 0usize;
    for op in parts.iter().flat_map(|part| &part.ops) {
        match op {
            Op::Clip { .. } => clips += 1,
            Op::Unclip if clips == 0 => continue,
            Op::Unclip => clips -= 1,
            _ => {}
        }
        op.draw(canvas);
    }
    for _ in 0..clips {
        canvas.unclip();
    }
}

/// An ease that starts at once and settles gently, as what is let go
/// settles: what a widget shows moves with its frame.
pub fn ease(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(4)
}

/// A canvas that draws nothing but records what is drawn on it, by the
/// names it is told (see `Canvas::key`), scaled by `scale` and moved by
/// `at`; it measures with `measure`.
pub struct Recorder<'a> {
    measure: &'a dyn Canvas,
    scale: f32,
    at: (f32, f32),
    key: RefCell<Option<String>>,
    parts: RefCell<Vec<Part>>,
}

impl<'a> Recorder<'a> {
    pub fn new(measure: &'a dyn Canvas, scale: f32, at: (f32, f32)) -> Self {
        Recorder { measure, scale, at, key: RefCell::new(None), parts: RefCell::new(Vec::new()) }
    }

    /// What was drawn: each name's things together, as one part, where the
    /// name was first used.
    pub fn parts(self) -> Vec<Part> {
        self.parts.into_inner()
    }

    fn push(&self, op: Op) {
        let map = Map { from: Bounds { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, to: Bounds { x: self.at.0, y: self.at.1, w: self.scale, h: self.scale }, sizes: self.scale };
        let op = op.mapped(&map);
        let key = self.key.borrow().clone();
        let mut parts = self.parts.borrow_mut();
        match key.as_ref().and_then(|key| parts.iter_mut().find(|part| part.key.as_ref() == Some(key))) {
            Some(part) => part.ops.push(op),
            None => parts.push(Part { key, ops: vec![op] }),
        }
    }
}

impl Canvas for Recorder<'_> {
    fn text(&self, text: &str, font: Font, color: Color, x: f32, y: f32, width: f32, align: Align) {
        let inked = self.measure.measure(text, font).min(width);
        self.push(Op::Text { text: text.into(), font, color, x, y, width, align, inked, scale: 1.0 });
    }

    fn text_scaled(&self, text: &str, font: Font, color: Color, x: f32, y: f32, width: f32, align: Align, scale: f32) {
        let inked = (self.measure.measure(text, font) * scale).min(width);
        self.push(Op::Text { text: text.into(), font, color, x, y, width, align, inked, scale });
    }

    fn measure(&self, text: &str, font: Font) -> f32 {
        self.measure.measure(text, font)
    }

    fn baseline(&self, font: Font) -> (f32, f32) {
        self.measure.baseline(font)
    }

    fn ink(&self, text: &str, font: Font) -> (f32, f32) {
        self.measure.ink(text, font)
    }

    fn clip(&self, x: f32, y: f32, w: f32, h: f32) {
        self.push(Op::Clip { x, y, w, h });
    }

    fn unclip(&self) {
        self.push(Op::Unclip);
    }

    fn fill(&self, color: Color, x: f32, y: f32, w: f32, h: f32) {
        self.push(Op::Fill { color, x, y, w, h });
    }

    fn fill_rounded(&self, color: Color, x: f32, y: f32, w: f32, h: f32, radius: f32) {
        self.push(Op::Rounded { color, x, y, w, h, radius });
    }

    fn fill_circle(&self, color: Color, centre: Point, radius: f32) {
        self.push(Op::Circle { color, centre, radius });
    }

    fn stroke(&self, points: &[Point], color: Color, width: f32) {
        self.push(Op::Stroke { points: points.to_vec(), color, width, dashed: false });
    }

    fn stroke_dashed(&self, points: &[Point], color: Color, width: f32) {
        self.push(Op::Stroke { points: points.to_vec(), color, width, dashed: true });
    }

    fn fill_shape(&self, points: &[Point], fill: Fill) {
        self.push(Op::Shape { points: points.to_vec(), fill });
    }

    fn stroke_rounded(&self, fill: Fill, x: f32, y: f32, w: f32, h: f32, radius: f32, line: f32) {
        self.push(Op::Outline { fill, x, y, w, h, radius, line });
    }

    fn shadow(&self, color: Color, x: f32, y: f32, w: f32, h: f32, radius: f32, blur: f32, drop: f32) {
        self.push(Op::Shadow { color, x, y, w, h, radius, blur, drop });
    }

    fn arc(&self, centre: Point, radius: f32, from: f32, sweep: f32, color: Color, width: f32) {
        self.push(Op::Arc { centre, radius, from, sweep, color, width });
    }

    fn icon(&self, icon: Icon, centre: Point, size: f32, color: Color) {
        self.push(Op::Icon { icon, centre, size, color });
    }

    fn key(&self, key: &str) {
        *self.key.borrow_mut() = (!key.is_empty()).then(|| key.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::canvas::Family;

    const INK: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };

    fn text(key: &str, text: &str, x: f32, y: f32, size: f32) -> Part {
        let font = Font::new(Family::Inter, size, 600.0);
        Part { key: Some(key.into()), ops: vec![Op::Text { text: text.into(), font, color: INK, x, y, width: 100.0, align: Align::Start, inked: size * text.len() as f32 * 0.5, scale: 1.0 }] }
    }

    #[test]
    fn a_part_both_have_moves_and_resizes_between_them() {
        let from = [text("cpu.value", "18", 10.0, 20.0, 40.0)];
        let to = [text("cpu.value", "18", 110.0, 60.0, 14.0)];
        let half = between(&from, &to, 0.5);
        let Op::Text { x, y, font, color, scale, .. } = &half[0].ops[0] else { panic!() };
        // Drawn at 27, from the font of an end made larger or smaller.
        assert_eq!((*x, *y, font.size * scale, color.a), (60.0, 40.0, 27.0, 1.0));
        assert_eq!(between(&from, &to, 1.0), to);
        assert_eq!(between(&from, &to, 0.0)[0].ops[0], from[0].ops[0]);
    }

    #[test]
    fn turns_broken_off_again_and_again_carry_on_only_what_shows() {
        // Two drawings of one part that do not match thing by thing: each
        // turn between them holds both; one broken off at its middle again
        // and again starts from what showed, the faded out let go of.
        let one = text("cpu.value", "18", 0.0, 0.0, 20.0);
        let two = Part { key: one.key.clone(), ops: vec![one.ops[0].clone(), Op::Fill { color: INK, x: 0.0, y: 30.0, w: 40.0, h: 4.0 }] };
        let mut drawn = vec![one.clone()];
        let mut most = 0;
        for turn in 0..200 {
            let to = if turn % 2 == 0 { &two } else { &one };
            drawn = between(&drawn, std::slice::from_ref(to), 0.5);
            most = most.max(drawn.iter().map(|part| part.ops.len()).sum::<usize>());
        }
        assert!(most < 40, "{most} things drawn");
    }

    #[test]
    fn what_only_one_has_fades_where_it_is() {
        let from = [text("cpu.label", "CPU", 0.0, 0.0, 12.0)];
        let to = [text("gpu.label", "GPU", 50.0, 0.0, 12.0)];
        let quarter = between(&from, &to, 0.25);
        let alpha = |part: &Part| match &part.ops[0] {
            Op::Text { color, x, .. } => (color.a, *x),
            _ => panic!(),
        };
        assert_eq!(alpha(&quarter[0]), (0.25, 50.0));
        assert_eq!(alpha(&quarter[1]), (0.75, 0.0));
    }

    #[test]
    fn changed_words_fade_from_one_to_the_other_as_they_move() {
        let from = [text("cpu.detail", "52 °C", 0.0, 0.0, 12.0)];
        let to = [text("cpu.detail", "52°", 40.0, 10.0, 12.0)];
        let half = between(&from, &to, 0.5);
        assert_eq!(half[0].ops.len(), 2);
        let place = |op: &Op| match op {
            Op::Text { text, color, x, y, .. } => (text.clone(), color.a, *x, *y),
            _ => panic!(),
        };
        assert_eq!(place(&half[0].ops[0]), ("52 °C".into(), 0.5, 20.0, 5.0));
        assert_eq!(place(&half[0].ops[1]), ("52°".into(), 0.5, 20.0, 5.0));
    }

    #[test]
    fn parts_drawn_otherwise_move_whole_from_box_to_box() {
        let line = |points: &[(f32, f32)]| Op::Stroke { points: points.iter().map(|&(x, y)| Point { x, y }).collect(), color: INK, width: 2.0, dashed: false };
        let from = [Part { key: Some("cpu.chart".into()), ops: vec![Op::Fill { color: INK, x: 0.0, y: 0.0, w: 1.0, h: 100.0 }, line(&[(0.0, 0.0), (100.0, 100.0)])] }];
        let to = [Part { key: Some("cpu.chart".into()), ops: vec![line(&[(200.0, 0.0), (220.0, 10.0), (240.0, 20.0)])] }];
        let half = between(&from, &to, 0.5);
        // The old (the rule and the line) and the new, each mapped onto the
        // box halfway between: from (0, 0, 100, 100) and (200, 0, 40, 20).
        let Op::Stroke { points, .. } = &half[0].ops.last().unwrap() else { panic!() };
        assert_eq!(points[0], Point { x: 100.0, y: 0.0 });
        assert_eq!(points[2], Point { x: 170.0, y: 60.0 });
    }

    #[test]
    fn words_moved_with_a_part_into_another_box_keep_their_size() {
        let font = Font::new(Family::Inter, 12.0, 500.0);
        let words = |text: &str, x: f32| Op::Text { text: text.into(), font, color: INK, x, y: 0.0, width: 40.0, align: Align::Start, inked: 10.0, scale: 1.0 };
        let from = [Part { key: Some("network.a.mark".into()), ops: vec![words("↓", 0.0)] }];
        let to = [Part { key: Some("network.a.mark".into()), ops: vec![Op::Fill { color: INK, x: 100.0, y: 6.0, w: 10.0, h: 2.0 }, words("下载", 115.0)] }];
        for part in between(&from, &to, 0.7) {
            for op in part.ops {
                if let Op::Text { font, scale, .. } = op {
                    assert_eq!(font.size * scale, 12.0);
                }
            }
        }
    }

    #[test]
    fn records_each_name_as_one_part_scaled_and_moved() {
        struct Measure;
        impl Canvas for Measure {
            fn text(&self, _: &str, _: Font, _: Color, _: f32, _: f32, _: f32, _: Align) {}
            fn measure(&self, text: &str, _: Font) -> f32 {
                text.len() as f32
            }
            fn baseline(&self, _: Font) -> (f32, f32) {
                (10.0, 2.0)
            }
            fn ink(&self, _: &str, _: Font) -> (f32, f32) {
                (2.0, 10.0)
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
            fn arc(&self, _: Point, _: f32, _: f32, _: f32, _: Color, _: f32) {}
            fn icon(&self, _: Icon, _: Point, _: f32, _: Color) {}
        }
        let recorder = Recorder::new(&Measure, 2.0, (5.0, 5.0));
        recorder.key("cpu.chart");
        recorder.fill(INK, 1.0, 1.0, 10.0, 10.0);
        recorder.key("");
        recorder.fill(INK, 0.0, 0.0, 1.0, 1.0);
        recorder.key("cpu.chart");
        recorder.fill_circle(INK, Point { x: 1.0, y: 1.0 }, 3.0);
        let parts = recorder.parts();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].ops[0], Op::Fill { color: INK, x: 7.0, y: 7.0, w: 20.0, h: 20.0 });
        assert_eq!(parts[0].ops[1], Op::Circle { color: INK, centre: Point { x: 7.0, y: 7.0 }, radius: 6.0 });
        assert_eq!(parts[1].key, None);
    }
}
