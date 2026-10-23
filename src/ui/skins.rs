//! What each skin puts behind the readings: chart paper's slab, Windows 11's
//! acrylic sheet, or glass pieces bending the desktop at their rims.

use windows::core::{Interface, Result};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_BLEND_MODE_LUMINOSITY, D2D1_COMPOSITE_MODE_SOURCE_OVER, D2D1_FILL_MODE_ALTERNATE, D2D1_GRADIENT_STOP,
    D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    CLSID_D2D12DAffineTransform, CLSID_D2D1Blend, CLSID_D2D1Border, CLSID_D2D1ColorMatrix, CLSID_D2D1Crop,
    CLSID_D2D1DisplacementMap, CLSID_D2D1Flood, CLSID_D2D1GaussianBlur, CLSID_D2D1Shadow, ID2D1Bitmap1, ID2D1Effect,
    ID2D1Geometry, ID2D1Image, D2D1_2DAFFINETRANSFORM_PROP_TRANSFORM_MATRIX, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
    D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1, ID2D1RenderTarget,
    D2D1_BLEND_PROP_MODE, D2D1_BORDER_EDGE_MODE_MIRROR, D2D1_BORDER_PROP_EDGE_MODE_X, D2D1_BORDER_PROP_EDGE_MODE_Y,
    D2D1_CHANNEL_SELECTOR_G, D2D1_CHANNEL_SELECTOR_R, D2D1_COLORMATRIX_PROP_COLOR_MATRIX, D2D1_CROP_PROP_RECT,
    D2D1_DISPLACEMENTMAP_PROP_SCALE, D2D1_DISPLACEMENTMAP_PROP_X_CHANNEL_SELECT, D2D1_DISPLACEMENTMAP_PROP_Y_CHANNEL_SELECT,
    D2D1_EXTEND_MODE_CLAMP, D2D1_FLOOD_PROP_COLOR, D2D1_GAMMA_2_2,
    D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_LAYER_PARAMETERS1,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_PROPERTY_TYPE, D2D1_PROPERTY_TYPE_ENUM, D2D1_PROPERTY_TYPE_FLOAT,
    D2D1_PROPERTY_TYPE_MATRIX_3X2, D2D1_PROPERTY_TYPE_MATRIX_5X4, D2D1_PROPERTY_TYPE_VECTOR4, D2D1_ROUNDED_RECT,
    D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION, D2D1_SHADOW_PROP_COLOR,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows_numerics::{Matrix3x2, Vector2};

use super::canvas::{Canvas, Color, Fill};
use super::gfx::{effect_input, Frame};
use super::theme::{Shadow, Skin, Theme};
use super::view::Rect;
use crate::settings::Edge;

/// Width of the curved rim of a piece of glass, and how far at most it bends
/// what is behind (DIPs). The bend falls off as (1 - depth/RIM)², whose
/// steepest slope, at the very edge, is 2·BEND/RIM DIPs of shift per DIP.
/// Kept below one, the image behind is squeezed toward the edge as a convex
/// rim does; above one it folds back on itself and reads as a hard break.
const RIM: f32 = 40.0;
const BEND: f32 = 16.0;
/// Backdrop luminance (0–1) each theme's glass reads well over without
/// help, and the distance from it at which glass is fully frosted. Light
/// glass carries dark text and wants a light backdrop, dark glass the reverse.
const READS_WELL: (f32, f32) = (0.6, 0.4);
const CONTRAST_SPAN: f32 = 0.35;
/// Spread of backdrop luminance (standard deviation) at which glass is fully
/// frosted, and below which it stays clear. Text, icons and photos behind
/// the glass spread wide; an empty page or a plain wallpaper hardly.
const BUSY: f32 = 0.14;
const CALM: f32 = 0.03;
/// Blur of fully frosted glass (DIPs).
pub(crate) const FROST_BLUR: f32 = 9.0;
/// Windows 11's acrylic: a wide blur and a lift in saturation.
pub(crate) const ACRYLIC_BLUR: f32 = 30.0;
pub(crate) const ACRYLIC_SATURATION: f32 = 1.25;
pub(crate) const GLASS_SATURATION: f32 = 1.3;

/// How frosted glass over a backdrop of luminance `mean` and spread
/// `spread` should be, 0–1: more when what is behind is busy, and more when
/// it clashes with the theme (bright under dark glass, dark under light).
pub fn frost(mean: f32, spread: f32, dark: bool) -> f32 {
    let busy = ((spread - CALM) / (BUSY - CALM)).clamp(0.0, 1.0);
    let against = if dark { mean - READS_WELL.1 } else { READS_WELL.0 - mean };
    busy.max((against / CONTRAST_SPAN).clamp(0.0, 1.0))
}

/// What the surface is drawn over and around.
pub struct Ground<'a> {
    pub theme: &'a Theme,
    /// The screen edge it is against; none for a panel moved away from it.
    pub edge: Option<Edge>,
    /// The panel's size (DIPs).
    pub size: (f32, f32),
    /// Each lane's box and the bar's, from the panel's corner.
    pub lanes: &'a [Rect],
    pub bar: Rect,
    /// The desktop behind the window, with the panel's corner at `at` in it
    /// (DIPs), for the skins that see it; and how many of the desktop's
    /// pixels a DIP of the panel covers (not the raster's scale, which may be
    /// coarser while it moves, or smaller in a preview).
    pub backdrop: Option<(&'a ID2D1Bitmap1, Vector2, f32)>,
    pub frost: f32,
}

/// The parts a surface is drawn in, one over the other: what lies under
/// the desktop it shows (its shadows), the desktop as it shows it, and
/// what lies over that (its tint, the light on its edges). Only the middle
/// one changes as the surface is moved over the desktop: the others are
/// drawn once for a shape and kept.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Part {
    Under,
    Desktop,
    Over,
}

/// The parts a skin's surface has, from the lowest.
pub fn parts(skin: Skin) -> &'static [Part] {
    match skin {
        Skin::Paper => &[Part::Under],
        Skin::Fluent | Skin::Glass => &[Part::Under, Part::Desktop, Part::Over],
    }
}

/// Draws `part` of the surface with the panel's corner at the frame's origin.
pub fn draw(frame: &Frame, ground: &Ground, part: Part) -> Result<()> {
    match ground.theme.skin {
        Skin::Paper => paper(frame, ground),
        Skin::Fluent => fluent(frame, ground, part),
        Skin::Glass => glass(frame, ground, part),
    }
}

fn rounded(r: Rect, radius: f32) -> D2D1_ROUNDED_RECT {
    let radius = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    D2D1_ROUNDED_RECT { rect: D2D_RECT_F { left: r.x, top: r.y, right: r.x + r.w, bottom: r.y + r.h }, radiusX: radius, radiusY: radius }
}

fn grow(r: Rect, by: f32) -> Rect {
    Rect { x: r.x - by, y: r.y - by, w: r.w + 2.0 * by, h: r.h + 2.0 * by }
}

fn geometry(frame: &Frame, r: Rect, radius: f32) -> Result<ID2D1Geometry> {
    unsafe { frame.gfx.factory.CreateRoundedRectangleGeometry(&rounded(r, radius))?.cast() }
}

fn prop<T>(effect: &ID2D1Effect, index: i32, kind: D2D1_PROPERTY_TYPE, value: &T) -> Result<()> {
    let bytes = unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) };
    unsafe { effect.SetValue(index as u32, kind, bytes) }
}

fn vector(c: Color) -> [f32; 4] {
    [c.r, c.g, c.b, c.a]
}

/// Draws whatever `paint` draws as a picture of its own, to be fed to effects.
fn picture(frame: &Frame, paint: impl FnOnce()) -> Result<ID2D1Image> {
    let dc = &frame.dc;
    unsafe {
        let list = dc.CreateCommandList()?;
        let target = dc.GetTarget()?;
        let mut transform = Matrix3x2::default();
        dc.GetTransform(&mut transform);
        // Setting a target can change the context's DPI; it is put back.
        let mut dpi = (0.0, 0.0);
        dc.GetDpi(&mut dpi.0, &mut dpi.1);
        dc.SetTarget(&list);
        dc.SetDpi(dpi.0, dpi.1);
        dc.SetTransform(&Matrix3x2::identity());
        paint();
        list.Close()?;
        dc.SetTarget(&target);
        dc.SetDpi(dpi.0, dpi.1);
        dc.SetTransform(&transform);
        list.cast()
    }
}

fn effect(frame: &Frame, class: &windows::core::GUID, input: &ID2D1Image) -> Result<ID2D1Effect> {
    let effect = unsafe { frame.dc.CreateEffect(class)? };
    unsafe { effect.SetInput(0, input, true) };
    Ok(effect)
}

fn output(effect: &ID2D1Effect) -> Result<ID2D1Image> {
    unsafe { effect.GetOutput() }
}

fn draw_image(frame: &Frame, image: &ID2D1Image, offset: Vector2) {
    unsafe {
        frame.dc.DrawImage(image, Some(&offset), None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
    }
}

/// Box shadows cast by `shapes`, as CSS draws them outside a box. (Each
/// shape's own: where two meet, they lie over each other as two shadows do,
/// a shade lighter than one blur of both would be.)
pub fn shadows(frame: &Frame, shapes: &[(Rect, f32)], specs: &[Shadow]) -> Result<()> {
    for shadow in specs {
        for (r, radius) in shapes {
            let (grown, radius) = (grow(*r, shadow.spread), radius + shadow.spread);
            // CSS's blur radius is twice the Gaussian's standard deviation.
            let deviation = shadow.blur / 2.0;
            let reach = reach(deviation);
            // Along an edge, past the rounded corner and as far again as the
            // blur reaches, the shadow is the same all along (see `sliced`).
            let cut = radius + 2.0 * reach;
            let at = Rect { x: grown.x - reach, y: grown.y - reach + shadow.y, w: grown.w + 2.0 * reach, h: grown.h + 2.0 * reach };
            let key = (*b"SHDW", [radius.to_bits(), deviation.to_bits(), 0, color_bits(shadow.color)]);
            let drawn = sliced(frame, at, (cut, cut), key, |frame, at| outer_blurred(frame, shrink(at, reach), radius, deviation, 0.0, shadow.color))?;
            if !drawn {
                outer_blurred(frame, grown, radius, deviation, shadow.y, shadow.color)?;
            }
        }
    }
    Ok(())
}

/// `shadows`, cast only outside each shape, as CSS casts a box's shadow: under a shape that lets what is behind it
/// show (glass), the shadow under it would darken all that shows through.
fn shadows_outside(frame: &Frame, shapes: &[(Rect, f32)], specs: &[Shadow]) -> Result<()> {
    let furthest = specs.iter().map(|shadow| reach(shadow.blur / 2.0) + shadow.spread.max(0.0) + shadow.y.abs() + 1.0).fold(0.0, f32::max);
    for shape in shapes {
        let (r, radius) = *shape;
        let around = grow(r, furthest);
        let ring: ID2D1Geometry = unsafe {
            let factory = &frame.gfx.factory;
            let outer: ID2D1Geometry = factory.CreateRectangleGeometry(&D2D_RECT_F { left: around.x, top: around.y, right: around.x + around.w, bottom: around.y + around.h })?.cast()?;
            factory.CreateGeometryGroup(D2D1_FILL_MODE_ALTERNATE, &[Some(outer), Some(geometry(frame, r, radius)?)])?.cast()?
        };
        let layer = D2D1_LAYER_PARAMETERS1 {
            contentBounds: D2D_RECT_F { left: around.x, top: around.y, right: around.x + around.w, bottom: around.y + around.h },
            geometricMask: std::mem::ManuallyDrop::new(Some(ring)),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: Matrix3x2::identity(),
            opacity: 1.0,
            ..Default::default()
        };
        unsafe { frame.dc.PushLayer(&layer, None) };
        let drawn = shadows(frame, std::slice::from_ref(shape), specs);
        unsafe { frame.dc.PopLayer() };
        drop(std::mem::ManuallyDrop::into_inner(layer.geometricMask));
        drawn?;
    }
    Ok(())
}

/// How far a Gaussian of `deviation` reaches before what it adds no longer shows (DIPs, whole).
fn reach(deviation: f32) -> f32 {
    (3.0 * deviation).ceil()
}

fn shrink(r: Rect, by: f32) -> Rect {
    grow(r, -by)
}

/// A colour as bits, for a key.
fn color_bits(c: Color) -> u32 {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    byte(c.r) << 24 | byte(c.g) << 16 | byte(c.b) << 8 | byte(c.a)
}

/// The shadow of the rounded box `r`, blurred as it is (a Gaussian of
/// `deviation`), moved down by `y`.
fn outer_blurred(frame: &Frame, r: Rect, radius: f32, deviation: f32, y: f32, color: Color) -> Result<()> {
    let shape = picture(frame, || unsafe { frame.dc.FillRoundedRectangle(&rounded(r, radius), frame.brush(Color { a: 1.0, ..color })) })?;
    let blur = effect(frame, &CLSID_D2D1Shadow, &shape)?;
    prop(&blur, D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION.0, D2D1_PROPERTY_TYPE_FLOAT, &deviation)?;
    prop(&blur, D2D1_SHADOW_PROP_COLOR.0, D2D1_PROPERTY_TYPE_VECTOR4, &vector(color))?;
    draw_image(frame, &output(&blur)?, Vector2 { X: 0.0, Y: y });
    Ok(())
}

/// A shadow cast inward from a box's edge, as CSS's `inset` box shadow:
/// what lies outside the box, offset and blurred, seen through the box.
fn inset_shadow(frame: &Frame, r: Rect, radius: f32, y: f32, blur: f32, color: Color) -> Result<()> {
    let deviation = blur / 2.0;
    let reach = reach(deviation);
    // Inside an edge, past the rounded corner, the blur's reach and the
    // offset, the shadow is the same all along (see `sliced`).
    let cut = (radius + reach, radius + reach + y.abs().ceil());
    let key = (*b"INST", [radius.to_bits(), deviation.to_bits(), y.to_bits(), color_bits(color)]);
    if !sliced(frame, r, cut, key, |frame, at| inset_blurred(frame, at, radius, y, deviation, color))? {
        inset_blurred(frame, r, radius, y, deviation, color)?;
    }
    Ok(())
}

/// `inset_shadow`, blurred as it is.
fn inset_blurred(frame: &Frame, r: Rect, radius: f32, y: f32, deviation: f32, color: Color) -> Result<()> {
    let outside = picture(frame, || unsafe {
        let factory = &frame.gfx.factory;
        let reach = grow(r, 2.0 * deviation + y.abs() + 1.0);
        let around: ID2D1Geometry = factory.CreateRectangleGeometry(&D2D_RECT_F { left: reach.x, top: reach.y, right: reach.x + reach.w, bottom: reach.y + reach.h }).unwrap().cast().unwrap();
        let inside = geometry(frame, r, radius).unwrap();
        let ring = factory.CreateGeometryGroup(D2D1_FILL_MODE_ALTERNATE, &[Some(around), Some(inside)]).unwrap();
        frame.dc.FillGeometry(&ring, frame.brush(Color { a: 1.0, ..color }), None);
    })?;
    let blurred = effect(frame, &CLSID_D2D1Shadow, &outside)?;
    prop(&blurred, D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION.0, D2D1_PROPERTY_TYPE_FLOAT, &deviation)?;
    prop(&blurred, D2D1_SHADOW_PROP_COLOR.0, D2D1_PROPERTY_TYPE_VECTOR4, &vector(color))?;
    within(frame, r, radius, || draw_image(frame, &output(&blurred).unwrap(), Vector2 { X: 0.0, Y: y }))
}

/// The steps of scale (physical pixels a DIP) the pieces of shadows are made at while a window is sized by hand (see
/// `sliced`): a sixteenth either way at most, at 1.5, stretched.
const SIZED_STEP: f32 = 0.125;

/// Draws over `at` what `paint` draws over a rectangle (given it, in its own
/// DIPs) that is the same all along its middle: past `cut` DIPs in from each
/// side (across, down), every column (row) is the same as the next (a box's
/// shadow past its corners). It is drawn once, small, into a bitmap kept by
/// `key` (see `Gfx::made`); its corners are drawn as they are, the strips
/// between drawn out along `at`'s sides, and its middle over `at`'s middle.
/// Made a few pixels across its middle, whatever `at`'s size, it costs the
/// same for a window of any size, and needs nothing made again as a window
/// is sized but itself.
///
/// It is made at the frame's scale, and starting at the same fraction of a
/// pixel as `at` does, and ending as `at` does: each of its pixels is one of
/// the frame's, nothing in it moved by a fraction of a pixel (a rim a pixel
/// wide is drawn as sharp as if it were drawn at `at` itself; sized by hand,
/// near it: see below). Whether it was drawn: not where `at` is too small to
/// have a middle, or the frame is turned or stretched; then the caller draws
/// it as it is.
fn sliced(frame: &Frame, at: Rect, cut: (f32, f32), key: ([u8; 4], [u32; 4]), paint: impl FnOnce(&Frame, Rect) -> Result<()>) -> Result<bool> {
    let dc = &frame.dc;
    let mut transform = Matrix3x2::default();
    let mut dpi = (0.0, 0.0);
    unsafe {
        dc.GetTransform(&mut transform);
        dc.GetDpi(&mut dpi.0, &mut dpi.1);
    }
    if at.w < 2.0 * cut.0 + 2.0 || at.h < 2.0 * cut.1 + 2.0 || transform.M12 != 0.0 || transform.M21 != 0.0 || transform.M11 != transform.M22 || transform.M11 <= 0.0 {
        return Ok(false);
    }
    // Physical pixels a DIP, and where `at`'s sides fall on the frame's pixels. Sized by hand, a panel zoomed as it is
    // sized is drawn at a scale of its own each frame, and made for it, its pieces were made again each frame (tens a
    // frame: a sized glass panel's frames twice as long), never drawn again: they are made at the nearest
    // `SIZED_STEP` instead, and drawn stretched the little that is left over, on no pixel in particular, each made
    // once for the frames sized through its step; at rest, made again as they are.
    let sized = super::gfx::sized_by_hand();
    let drawn = dpi.0 / 96.0 * transform.M11;
    let px = if sized { (drawn / SIZED_STEP).round().max(1.0) * SIZED_STEP } else { drawn };
    let device = |v: f32, offset: f32| (v * transform.M11 + offset) * dpi.0 / 96.0;
    let phase = |v: f32| if sized { 0.0 } else { v.rem_euclid(1.0) };
    let (left, top) = (phase(device(at.x, transform.M31)), phase(device(at.y, transform.M32)));
    let (right, bottom) = (phase(device(at.x + at.w, transform.M31)), phase(device(at.y + at.h, transform.M32)));
    // Its size in pixels: room for the corners and a few pixels between, and
    // ending at the fraction of a pixel `at` ends at.
    let length = |cut: f32, start: f32, end: f32| (2.0 * cut * px).ceil() + 2.0 + (end - start).rem_euclid(1.0);
    let size = (length(cut.0, left, right), length(cut.1, top, bottom));
    let (tag, parts) = key;
    let mut mixed = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&(parts, px.to_bits(), [left, top, right, bottom].map(f32::to_bits), cut.0.to_bits(), cut.1.to_bits()), &mut mixed);
    let mixed = std::hash::Hasher::finish(&mixed);
    let made_key = [u32::from_be_bytes(tag), (size.0 as u32) << 16 | size.1 as u32, mixed as u32, (mixed >> 32) as u32];
    // In its own DIPs (at `px`): where what stands for `at` is in it.
    let inner = Rect { x: left / px, y: top / px, w: size.0 / px, h: size.1 / px };
    let sample = frame.gfx.made(made_key, || unsafe {
        let properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0 * px,
            dpiY: 96.0 * px,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
            ..Default::default()
        };
        let pixels = D2D_SIZE_U { width: (left + size.0).ceil() as u32, height: (top + size.1).ceil() as u32 };
        let bitmap = frame.gfx.drawn_on(dc, pixels, properties.dpiX)?;
        // Drawn on it, the frame's target, scale and place put back after.
        let target = dc.GetTarget()?;
        dc.SetTarget(&bitmap);
        dc.SetDpi(96.0 * px, 96.0 * px);
        dc.SetTransform(&Matrix3x2::identity());
        dc.Clear(Some(&windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F::default()));
        let painted = paint(frame, inner);
        dc.SetTarget(&target);
        dc.SetDpi(dpi.0, dpi.1);
        dc.SetTransform(&transform);
        painted.map(|_| bitmap)
    })?;
    // Its pieces and `at`'s, cut at the same distances in from each side; the
    // outer ones out to whole pixels (the pixels `at` covers a part of are
    // drawn, as they were drawn into it, each in part).
    let from = |start: f32, length: f32, cut: f32, before: f32, after: f32| [start - before, start + cut, start + length - cut, start + length + after];
    let after = |start: f32, length: f32| ((start + length).ceil() - (start + length)) / px;
    let (sx, sy) = (from(inner.x, inner.w, cut.0, left / px, after(left, size.0)), from(inner.y, inner.h, cut.1, top / px, after(top, size.1)));
    let (dx, dy) = (from(at.x, at.w, cut.0, left / px, after(left, size.0)), from(at.y, at.h, cut.1, top / px, after(top, size.1)));
    unsafe {
        // Pieces meeting edge to edge: each pixel drawn by one of them, none
        // twice (at a seam's half-covered pixels) or not at all.
        let mode = dc.GetAntialiasMode();
        dc.SetAntialiasMode(windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED);
        for row in 0..3 {
            for column in 0..3 {
                let source = D2D_RECT_F { left: sx[column], top: sy[row], right: sx[column + 1], bottom: sy[row + 1] };
                let target = D2D_RECT_F { left: dx[column], top: dy[row], right: dx[column + 1], bottom: dy[row + 1] };
                dc.DrawBitmap(&sample, Some(&target), 1.0, D2D1_INTERPOLATION_MODE_LINEAR, Some(&source), None);
            }
        }
        dc.SetAntialiasMode(mode);
    }
    Ok(true)
}

/// Draws what `paint` draws, seen only inside the rounded box.
fn within(frame: &Frame, r: Rect, radius: f32, paint: impl FnOnce()) -> Result<()> {
    let mask = geometry(frame, r, radius)?;
    let layer = D2D1_LAYER_PARAMETERS1 {
        contentBounds: D2D_RECT_F { left: r.x, top: r.y, right: r.x + r.w, bottom: r.y + r.h },
        geometricMask: std::mem::ManuallyDrop::new(Some(mask)),
        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
        maskTransform: Matrix3x2::identity(),
        opacity: 1.0,
        ..Default::default()
    };
    unsafe { frame.dc.PushLayer(&layer, None) };
    paint();
    unsafe { frame.dc.PopLayer() };
    drop(std::mem::ManuallyDrop::into_inner(layer.geometricMask));
    Ok(())
}

/// The matrix CSS and SVG use to change saturation by `amount` (1 unchanged).
fn saturation(amount: f32) -> [f32; 20] {
    let s = amount;
    // Rows of the SVG `saturate` matrix, as Direct2D's columns.
    let m = [
        [0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s],
        [0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s],
        [0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s],
    ];
    // Direct2D multiplies a row vector by the matrix: element [input][output].
    [
        m[0][0], m[1][0], m[2][0], 0.0,
        m[0][1], m[1][1], m[2][1], 0.0,
        m[0][2], m[1][2], m[2][2], 0.0,
        0.0, 0.0, 0.0, 1.0,
        0.0, 0.0, 0.0, 0.0,
    ]
}

/// The backdrop inside `area` (DIPs from the panel's corner), mirrored past
/// its edges. Where `area` reaches past the backdrop (beside the screen's
/// edge), what is beyond is the backdrop seen in the edge as in a mirror,
/// not the nothing there; nor the edge's last pixels drawn out, which make a
/// thin line along the screen's edge (a window's border) a broad band.
fn backdrop_within(frame: &Frame, backdrop: (&ID2D1Bitmap1, Vector2, f32), area: Rect) -> Result<ID2D1Effect> {
    let (bitmap, at, shown) = backdrop;
    // The backdrop placed so the panel's corner is at the origin, and as
    // large as the screen is where it is drawn: a bitmap made at the
    // screen's own scale serves a panel drawn larger or smaller (a widget
    // at its zoom) without being made again.
    let image = effect_input(&frame.dc, bitmap)?;
    // At the scale it is shown at, whatever the raster's: the desktop keeps
    // its place and size under a panel drawn coarser while it moves.
    let (mut made, mut unused) = (0.0, 0.0);
    unsafe { bitmap.GetDpi(&mut made, &mut unused) };
    let k = made / (96.0 * shown);
    let placed = effect(frame, &CLSID_D2D12DAffineTransform, &image)?;
    prop(&placed, D2D1_2DAFFINETRANSFORM_PROP_TRANSFORM_MATRIX.0, D2D1_PROPERTY_TYPE_MATRIX_3X2, &(Matrix3x2::scale(k, k) * Matrix3x2::translation(-at.X, -at.Y)))?;
    let size = unsafe { bitmap.GetSize() };
    let size = windows::Win32::Graphics::Direct2D::Common::D2D_SIZE_F { width: size.width * k, height: size.height * k };
    // What of `area` the backdrop holds; past it altogether, the backdrop's
    // nearest edge (one DIP of it), which mirrored is that edge carried on.
    // (A backdrop narrower than a DIP, a sliver of a screen, is carried on
    // whole.)
    let (x0, y0, x1, y1) = (-at.X, -at.Y, size.width - at.X, size.height - at.Y);
    let left = area.x.max(x0).min((x1 - 1.0).max(x0));
    let top = area.y.max(y0).min((y1 - 1.0).max(y0));
    let right = (area.x + area.w).min(x1).max((left + 1.0).min(x1));
    let bottom = (area.y + area.h).min(y1).max((top + 1.0).min(y1));
    let crop = effect(frame, &CLSID_D2D1Crop, &output(&placed)?)?;
    prop(&crop, D2D1_CROP_PROP_RECT.0, D2D1_PROPERTY_TYPE_VECTOR4, &[left, top, right, bottom])?;
    let border = effect(frame, &CLSID_D2D1Border, &output(&crop)?)?;
    prop(&border, D2D1_BORDER_PROP_EDGE_MODE_X.0, D2D1_PROPERTY_TYPE_ENUM, &D2D1_BORDER_EDGE_MODE_MIRROR.0)?;
    prop(&border, D2D1_BORDER_PROP_EDGE_MODE_Y.0, D2D1_PROPERTY_TYPE_ENUM, &D2D1_BORDER_EDGE_MODE_MIRROR.0)?;
    Ok(border)
}

fn blurred(frame: &Frame, input: &ID2D1Effect, deviation: f32) -> Result<ID2D1Effect> {
    let blur = effect(frame, &CLSID_D2D1GaussianBlur, &output(input)?)?;
    prop(&blur, D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0, D2D1_PROPERTY_TYPE_FLOAT, &deviation)?;
    Ok(blur)
}

fn saturated(frame: &Frame, input: &ID2D1Effect, amount: f32) -> Result<ID2D1Effect> {
    let matrix = effect(frame, &CLSID_D2D1ColorMatrix, &output(input)?)?;
    prop(&matrix, D2D1_COLORMATRIX_PROP_COLOR_MATRIX.0, D2D1_PROPERTY_TYPE_MATRIX_5X4, &saturation(amount))?;
    Ok(matrix)
}

/// 记录纸: one slab flush with the screen edge, rounded only where it is
/// free: its corners on the attached side lie beyond the edge. Moved away
/// from the edge, it is free all round.
fn paper(frame: &Frame, ground: &Ground) -> Result<()> {
    let theme = ground.theme;
    let (w, h) = ground.size;
    let radius = theme.radius;
    let slab = match ground.edge {
        Some(Edge::Left) => Rect { x: -radius, y: 0.0, w: w + radius, h },
        Some(Edge::Right) => Rect { x: 0.0, y: 0.0, w: w + radius, h },
        Some(Edge::Top) => Rect { x: 0.0, y: -radius, w, h: h + radius },
        None => Rect { x: 0.0, y: 0.0, w, h },
    };
    shadows(frame, &[(slab, radius)], &theme.shadows)?;
    // The rule rings the slab just outside it. (Through the frame's fills, on whole pixels as everything is.)
    let ring = grow(slab, 1.0);
    Canvas::fill_rounded(frame, theme.stroke, ring.x, ring.y, ring.w, ring.h, radius + 1.0);
    Canvas::fill_rounded(frame, theme.paper, slab.x, slab.y, slab.w, slab.h, radius);
    Ok(())
}

/// Windows 11: one acrylic sheet. The desktop behind it, blurred and a
/// little more saturated; a luminosity layer that evens out how bright it
/// is; and a thin tint. (Windows adds a faint grain; on a dark sheet it
/// read as noise, so the sheet goes without.)
fn fluent(frame: &Frame, ground: &Ground, part: Part) -> Result<()> {
    let theme = ground.theme;
    let sheet = Rect { x: 0.0, y: 0.0, w: ground.size.0, h: ground.size.1 };
    let area = D2D_RECT_F { left: 0.0, top: 0.0, right: sheet.w, bottom: sheet.h };
    match part {
        Part::Under => {
            shadows(frame, &[(sheet, theme.radius)], &theme.shadows)?;
            let ring = grow(sheet, 1.0);
            Canvas::fill_rounded(frame, theme.stroke, ring.x, ring.y, ring.w, ring.h, theme.radius + 1.0);
            Ok(())
        }
        Part::Desktop => within(frame, sheet, theme.radius, || match ground.backdrop.map(|backdrop| acrylic(frame, backdrop, sheet, theme.luminosity)) {
            Some(Ok((acrylic, luminous))) => {
                draw_image(frame, &acrylic, Vector2::zero());
                // The luminosity layer, at its own opacity, over the acrylic:
                // CSS's mix-blend-mode, (1 - a) * backdrop + a * blend.
                let layer = D2D1_LAYER_PARAMETERS1 { contentBounds: area, opacity: theme.luminosity.a, ..Default::default() };
                unsafe {
                    frame.dc.PushLayer(&layer, None);
                    draw_image(frame, &luminous, Vector2::zero());
                    frame.dc.PopLayer();
                }
            }
            // Nothing captured: the luminosity layer alone, as if over a
            // backdrop of its own colour.
            _ => unsafe { frame.dc.FillRectangle(&area, frame.brush(Color { a: 1.0, ..theme.luminosity })) },
        }),
        Part::Over => within(frame, sheet, theme.radius, || {
            unsafe { frame.dc.FillRectangle(&area, frame.brush(theme.tint)) };
            // Each lane on its card (not a small layout's one piece: the sheet
            // is its card).
            if theme.card.a > 0.0 {
                for lane in ground.lanes.iter().filter(|lane| lane.w < sheet.w || lane.h < sheet.h) {
                    // (Through the frame's fill and outline, put on whole pixels as everything is: drawn straight,
                    // a card's border on a pixel's edge was spread over two at a fractional scale.)
                    Canvas::fill_rounded(frame, theme.card, lane.x, lane.y, lane.w, lane.h, CARD_RADIUS);
                    Canvas::stroke_rounded(frame, Fill::Solid(theme.card_stroke), lane.x, lane.y, lane.w, lane.h, CARD_RADIUS, 1.0);
                }
            }
        }),
    }
}

/// Windows 11's cards' corners.
const CARD_RADIUS: f32 = 4.0;

/// The desktop behind the sheet blurred and a little more saturated, and
/// the same with the luminosity layer's brightness in place of its own:
/// its hue and saturation, `luminosity`'s lightness.
fn acrylic(frame: &Frame, backdrop: (&ID2D1Bitmap1, Vector2, f32), sheet: Rect, luminosity: Color) -> Result<(ID2D1Image, ID2D1Image)> {
    let desktop = backdrop_within(frame, backdrop, sheet)?;
    let acrylic = saturated(frame, &blurred(frame, &desktop, ACRYLIC_BLUR)?, ACRYLIC_SATURATION)?;
    // Cropped back to the sheet: the blur spreads past it, and the flood
    // below is endless.
    let crop = effect(frame, &CLSID_D2D1Crop, &output(&acrylic)?)?;
    prop(&crop, D2D1_CROP_PROP_RECT.0, D2D1_PROPERTY_TYPE_VECTOR4, &[sheet.x, sheet.y, sheet.x + sheet.w, sheet.y + sheet.h])?;
    // An opaque flood, so its colour reads the same straight or premultiplied.
    let flood = unsafe { frame.dc.CreateEffect(&CLSID_D2D1Flood)? };
    prop(&flood, D2D1_FLOOD_PROP_COLOR.0, D2D1_PROPERTY_TYPE_VECTOR4, &vector(Color { a: 1.0, ..luminosity }))?;
    let blend = unsafe { frame.dc.CreateEffect(&CLSID_D2D1Blend)? };
    unsafe {
        // Destination first: the acrylic, under the luminosity layer.
        blend.SetInput(0, &output(&crop)?, true);
        blend.SetInput(1, &output(&flood)?, true);
    }
    prop(&blend, D2D1_BLEND_PROP_MODE.0, D2D1_PROPERTY_TYPE_ENUM, &D2D1_BLEND_MODE_LUMINOSITY.0)?;
    let clipped = effect(frame, &CLSID_D2D1Crop, &output(&blend)?)?;
    prop(&clipped, D2D1_CROP_PROP_RECT.0, D2D1_PROPERTY_TYPE_VECTOR4, &[sheet.x, sheet.y, sheet.x + sheet.w, sheet.y + sheet.h])?;
    Ok((output(&crop)?, output(&clipped)?))
}

/// 磨砂玻璃: every lane and the bar a piece of glass. Each bends the desktop
/// at its rim, frosts it as much as legibility needs, takes the theme's
/// tint, glows faintly inside its edge and catches light along it.
fn glass(frame: &Frame, ground: &Ground, part: Part) -> Result<()> {
    let theme = ground.theme;
    let pieces: Vec<(Rect, f32)> = ground.lanes.iter().chain([&ground.bar]).filter(|r| r.w > 0.0 && r.h > 0.0).map(|r| (*r, theme.radius)).collect();
    if part == Part::Under {
        return shadows_outside(frame, &pieces, &theme.shadows);
    }
    let tint = glass_tint(theme, ground.frost, ground.backdrop.is_some());
    for (piece, radius) in &pieces {
        if part == Part::Desktop {
            if let Some(backdrop) = ground.backdrop {
                let lens = lens(frame, backdrop, *piece, *radius, ground.frost * FROST_BLUR)?;
                within(frame, *piece, *radius, || draw_image(frame, &lens, Vector2::zero()))?;
            }
            continue;
        }
        unsafe { frame.dc.FillRoundedRectangle(&rounded(*piece, *radius), frame.brush(tint)) };
        // Light caught just inside the edge: strongest from above.
        let white = |a| Color::hex(0xFFFFFF, a);
        // (Lit along the top, a little light back along the foot: the owner's "适中", of three looked at; the first,
        // bright all round, read as a tube of metal.)
        inset_shadow(frame, *piece, *radius, 1.0, 2.0, white(0.5))?;
        inset_shadow(frame, *piece, *radius, -1.0, 2.0, white(0.12))?;
        inset_shadow(frame, *piece, *radius, 0.0, 18.0, white(0.07))?;
        rim(frame, *piece, *radius)?;
    }
    Ok(())
}

/// The glass's tint, `frost` asked for (as the desktop behind needs it: see `frost`), and whether a picture of the
/// desktop is had to frost.
pub(crate) fn glass_tint(theme: &Theme, frost: f32, behind: bool) -> Color {
    // With none, as at its most frosted: the desktop shows through sharp, and how busy it is is not known.
    let frost = if behind { frost } else { 1.0 };
    theme.glass.alpha(theme.glass_clear + theme.glass_frosted * frost)
}

/// The desktop behind a piece as its glass shows it: frosted by `blur`, bent
/// outward across the rim, and a little more saturated.
fn lens(frame: &Frame, backdrop: (&ID2D1Bitmap1, Vector2, f32), piece: Rect, radius: f32, blur: f32) -> Result<ID2D1Image> {
    // The rim bends in what lies up to BEND beyond the piece.
    let desktop = backdrop_within(frame, backdrop, grow(piece, BEND))?;
    let frosted = blurred(frame, &desktop, blur)?;
    let map = displacement_map(frame, piece, radius)?;
    let placed = effect(frame, &CLSID_D2D12DAffineTransform, &effect_input(&frame.dc, &map)?)?;
    prop(&placed, D2D1_2DAFFINETRANSFORM_PROP_TRANSFORM_MATRIX.0, D2D1_PROPERTY_TYPE_MATRIX_3X2, &Matrix3x2::translation(piece.x, piece.y))?;
    let bent = unsafe { frame.dc.CreateEffect(&CLSID_D2D1DisplacementMap)? };
    unsafe {
        bent.SetInput(0, &output(&frosted)?, true);
        bent.SetInput(1, &output(&placed)?, true);
    }
    prop(&bent, D2D1_DISPLACEMENTMAP_PROP_SCALE.0, D2D1_PROPERTY_TYPE_FLOAT, &(2.0 * BEND))?;
    prop(&bent, D2D1_DISPLACEMENTMAP_PROP_X_CHANNEL_SELECT.0, D2D1_PROPERTY_TYPE_ENUM, &D2D1_CHANNEL_SELECTOR_R.0)?;
    prop(&bent, D2D1_DISPLACEMENTMAP_PROP_Y_CHANNEL_SELECT.0, D2D1_PROPERTY_TYPE_ENUM, &D2D1_CHANNEL_SELECTOR_G.0)?;
    output(&saturated(frame, &bent, GLASS_SATURATION)?)
}

/// For every point of a rounded rectangle, where its glass makes it look:
/// unchanged in the middle, pulled outward across the rim, most at the very
/// edge, as a convex edge of glass bends light. Red is the x shift and green
/// the y shift, half meaning none.
fn displacement_map(frame: &Frame, piece: Rect, radius: f32) -> Result<ID2D1Bitmap1> {
    let mut drawn = (0.0, 0.0);
    unsafe { frame.dc.GetDpi(&mut drawn.0, &mut drawn.1) };
    // Made at a whole number of pixels a DIP, no coarser than it is drawn at
    // (it is a smooth field: drawn finer or coarser, it is sampled), so that
    // a panel drawn at every zoom as it is sized makes one or two, not one
    // for each zoom.
    let px = (drawn.0 / 96.0).ceil().max(1.0);
    let dpi = (96.0 * px, 96.0 * px);
    let (w, h) = ((piece.w * px).round() as usize, (piece.h * px).round() as usize);
    // The same for every piece of this size, wherever it is: made once
    // (not for each frame of a widget moved over the desktop).
    let key = [piece.w.to_bits(), piece.h.to_bits(), radius.to_bits(), dpi.0.to_bits()];
    frame.gfx.made(key, || made_displacement_map(frame, piece, radius, dpi, (w, h)))
}

/// The displacement map's pixel at (`column`, `row`) of a piece's, `px` pixels a DIP (see `displacement_map`): BGRA,
/// opaque.
fn displaced(piece: Rect, radius: f32, px: f32, column: usize, row: usize) -> [u8; 4] {
    let (hx, hy) = (piece.w / 2.0, piece.h / 2.0);
    let r = radius.min(hx).min(hy);
    // The pixel's centre, from the piece's centre (DIPs).
    let (x, y) = ((column as f32 + 0.5) / px - hx, (row as f32 + 0.5) / px - hy);
    let (qx, qy) = (x.abs() - (hx - r), y.abs() - (hy - r));
    // Distance in from the edge, and the outward direction there.
    let (depth, nx, ny) = if qx > 0.0 && qy > 0.0 {
        let length = qx.hypot(qy);
        (r - length, qx / length, qy / length)
    } else if qx > qy {
        (hx - x.abs(), 1.0, 0.0)
    } else {
        (hy - y.abs(), 0.0, 1.0)
    };
    let strength = (1.0 - (depth / RIM).clamp(0.0, 1.0)).powi(2);
    [128, (128.0 + ny * y.signum() * strength * 127.0).round() as u8, (128.0 + nx * x.signum() * strength * 127.0).round() as u8, 255]
}

/// A piece's displacement map, `size` pixels at `px` a DIP, as `displaced` has each pixel; worked out only where it
/// differs from row to row and column to column. Between the corners (further in from the ends than the rounding
/// and the rim both reach: `RIM`), each row is as far from the sides as the next and bent only by them, alike all
/// down the middle; each column, only by the top and the bottom. Only the corners are worked out pixel by pixel (a
/// wide piece's map a few times sooner: a glass panel made its lanes' maps in a third of its first frame).
fn displacement_pixels(piece: Rect, radius: f32, px: f32, (w, h): (usize, usize)) -> Vec<u8> {
    let (hx, hy) = (piece.w / 2.0, piece.h / 2.0);
    let reach = radius.min(hx).min(hy).max(RIM);
    let middle = |index: usize, half: f32| ((index as f32 + 0.5) / px - half).abs() <= half - reach;
    let mut pixels = vec![0u8; w * h * 4];
    let mut middle_row = None;
    for row in 0..h {
        let line = row * w * 4..(row + 1) * w * 4;
        let row_middle = middle(row, hy);
        if row_middle {
            if let Some(first) = middle_row {
                pixels.copy_within(first * w * 4..(first + 1) * w * 4, line.start);
                continue;
            }
            middle_row = Some(row);
        }
        let mut across = None;
        for (column, pixel) in pixels[line].chunks_exact_mut(4).enumerate() {
            let value = match !row_middle && middle(column, hx) {
                true => *across.get_or_insert_with(|| displaced(piece, radius, px, column, row)),
                false => displaced(piece, radius, px, column, row),
            };
            pixel.copy_from_slice(&value);
        }
    }
    pixels
}

/// A piece's displacement map, `size` pixels at `dpi` (see
/// `displacement_map`).
fn made_displacement_map(frame: &Frame, piece: Rect, radius: f32, dpi: (f32, f32), (w, h): (usize, usize)) -> Result<ID2D1Bitmap1> {
    let pixels = displacement_pixels(piece, radius, dpi.0 / 96.0, (w, h));
    let properties = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
        dpiX: dpi.0,
        dpiY: dpi.1,
        bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
        ..Default::default()
    };
    let size = D2D_SIZE_U { width: w as u32, height: h as u32 };
    unsafe { frame.dc.CreateBitmap(size, Some(pixels.as_ptr().cast()), (w * 4) as u32, &properties) }
}

/// The light along a piece's edge: a 1.5 DIP ring, brightest where light
/// would strike the top-left, again faintly at the bottom-right.
/// How wide glass's rim is (DIPs).
const RIM_WIDTH: f32 = 1.25;

fn rim(frame: &Frame, piece: Rect, radius: f32) -> Result<()> {
    let white = |position, a| D2D1_GRADIENT_STOP { position, color: Color::hex(0xFFFFFF, a).d2d() };
    let stops = [white(0.0, 0.75), white(0.22, 0.2), white(0.5, 0.05), white(0.78, 0.08), white(1.0, 0.22)];
    // CSS's 135deg: toward the bottom-right, over a line long enough for the
    // gradient to reach the corners.
    let length = (piece.w + piece.h) * std::f32::consts::FRAC_1_SQRT_2;
    let (cx, cy) = (piece.x + piece.w / 2.0, piece.y + piece.h / 2.0);
    let step = length / 2.0 * std::f32::consts::FRAC_1_SQRT_2;
    unsafe {
        let collection = ID2D1RenderTarget::CreateGradientStopCollection(&frame.dc, &stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
        let line = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
            startPoint: Vector2 { X: cx - step, Y: cy - step },
            endPoint: Vector2 { X: cx + step, Y: cy + step },
        };
        let brush = frame.dc.CreateLinearGradientBrush(&line, None, &collection)?;
        let outer = geometry(frame, piece, radius)?;
        let inner = geometry(frame, grow(piece, -RIM_WIDTH), radius - RIM_WIDTH)?;
        let ring = frame.gfx.factory.CreateGeometryGroup(D2D1_FILL_MODE_ALTERNATE, &[Some(outer), Some(inner)])?;
        frame.dc.FillGeometry(&ring, &brush, None);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Glass with no picture of the desktop to frost (none taken yet, or taken on a device since lost) is tinted as
    /// at its most frosted: the desktop shows through it sharp, and how busy it is is not known; tinted as the
    /// picture last had it, a busy desktop came through all but clear, and the readings over it were lost.
    /// A box's shadow, while the window it is in is sized by hand, is made once for the scales near one another its
    /// frames are drawn at (a panel zoomed as it is sized: a scale of its own each frame), not once for each; at
    /// rest, at each scale as it is.
    #[test]
    fn a_shadow_sized_by_hand_is_made_once_for_scales_near_one_another() {
        let _device = crate::ui::gfx::held().unwrap();
        let gfx = super::super::gfx::Gfx::new().expect("graphics");
        let shapes = [(Rect { x: 10.0, y: 10.0, w: 200.0, h: 120.0 }, 8.0)];
        let specs = [Shadow { y: 2.0, blur: 8.0, spread: 0.0, color: Color::hex(0, 0.2) }];
        let draw = |scale: f32| {
            Frame::whole(&gfx, (400, 300), scale, |frame| shadows(frame, &shapes, &specs).expect("drawn")).expect("frame");
            gfx.made_count()
        };
        {
            let _sizing = super::super::gfx::sizing(true);
            let made = draw(1.5);
            assert!(made > 0);
            assert_eq!(draw(1.53), made, "near: the same one");
            assert_eq!(draw(1.47), made, "near: the same one");
            assert!(draw(1.75) > made, "further: one of its own");
        }
        let made = draw(1.5);
        assert!(draw(1.53) > made, "at rest: each scale's own");
    }

    /// A displacement map worked out only where it changes is the one worked out pixel by pixel, for pieces long and
    /// short, rounded more and less than the rim reaches, at whole and fractional sizes and scales.
    #[test]
    fn a_displacement_map_is_the_same_worked_out_pixel_by_pixel() {
        for (pw, ph) in [(356.3, 189.7), (356.0, 44.5), (90.0, 400.25), (20.0, 10.0), (81.0, 81.0), (500.0, 500.0)] {
            for radius in [0.0, 8.0, 60.0] {
                for px in [1.0, 1.5, 2.0, 3.0] {
                    let piece = Rect { x: 3.25, y: 7.5, w: pw, h: ph };
                    let size = ((pw * px).round() as usize, (ph * px).round() as usize);
                    let fast = displacement_pixels(piece, radius, px, size);
                    for row in 0..size.1 {
                        for column in 0..size.0 {
                            let i = (row * size.0 + column) * 4;
                            assert_eq!(fast[i..i + 4], displaced(piece, radius, px, column, row), "{pw}x{ph} r{radius} at {px}: ({column}, {row})");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn glass_without_a_picture_is_tinted_as_most_frosted() {
        for dark in [false, true] {
            let theme = Theme::new(Skin::Glass, dark);
            let most = glass_tint(&theme, 1.0, true);
            assert_eq!(glass_tint(&theme, 0.0, false).a, most.a);
            assert_eq!(glass_tint(&theme, 0.4, false).a, most.a);
            assert!(glass_tint(&theme, 0.0, true).a < most.a);
        }
    }
}
