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
    D2D1_BITMAP_BRUSH_PROPERTIES1, D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_PROPERTIES1, ID2D1RenderTarget,
    D2D1_BLEND_PROP_MODE, D2D1_BORDER_EDGE_MODE_CLAMP, D2D1_BORDER_PROP_EDGE_MODE_X, D2D1_BORDER_PROP_EDGE_MODE_Y,
    D2D1_CHANNEL_SELECTOR_G, D2D1_CHANNEL_SELECTOR_R, D2D1_COLORMATRIX_PROP_COLOR_MATRIX, D2D1_CROP_PROP_RECT,
    D2D1_DISPLACEMENTMAP_PROP_SCALE, D2D1_DISPLACEMENTMAP_PROP_X_CHANNEL_SELECT, D2D1_DISPLACEMENTMAP_PROP_Y_CHANNEL_SELECT,
    D2D1_EXTEND_MODE_CLAMP, D2D1_EXTEND_MODE_WRAP, D2D1_FLOOD_PROP_COLOR, D2D1_GAMMA_2_2,
    D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_LAYER_PARAMETERS1,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_PROPERTY_TYPE, D2D1_PROPERTY_TYPE_ENUM, D2D1_PROPERTY_TYPE_FLOAT,
    D2D1_PROPERTY_TYPE_MATRIX_3X2, D2D1_PROPERTY_TYPE_MATRIX_5X4, D2D1_PROPERTY_TYPE_VECTOR4, D2D1_ROUNDED_RECT,
    D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION, D2D1_SHADOW_PROP_COLOR,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows_numerics::{Matrix3x2, Vector2};

use super::gfx::{effect_input, Color, Frame};
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
const FROST_BLUR: f32 = 9.0;
/// Windows 11's acrylic: a wide blur and a lift in saturation.
const ACRYLIC_BLUR: f32 = 30.0;
const ACRYLIC_SATURATION: f32 = 1.25;
const GLASS_SATURATION: f32 = 1.3;
/// The grain acrylic carries to keep large blurs from banding: one tile, and
/// how strongly it shows.
const GRAIN: u32 = 64;
const GRAIN_ALPHA: f32 = 6.0 / 255.0;

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
    pub edge: Edge,
    /// The panel's size (DIPs).
    pub size: (f32, f32),
    /// Each lane's box and the bar's, from the panel's corner.
    pub lanes: &'a [Rect],
    pub bar: Rect,
    /// The desktop behind the window, with the panel's corner at `at` in it
    /// (DIPs), for the skins that see it.
    pub backdrop: Option<(&'a ID2D1Bitmap1, Vector2)>,
    pub frost: f32,
}

/// Draws the surface with the panel's corner at the frame's origin.
pub fn draw(frame: &Frame, ground: &Ground) -> Result<()> {
    match ground.theme.skin {
        Skin::Paper => paper(frame, ground),
        Skin::Fluent => fluent(frame, ground),
        Skin::Glass => glass(frame, ground),
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

/// Box shadows cast by `shapes`, as CSS draws them outside a box.
pub fn shadows(frame: &Frame, shapes: &[(Rect, f32)], specs: &[Shadow]) -> Result<()> {
    for shadow in specs {
        let shape = picture(frame, || {
            for (r, radius) in shapes {
                let grown = grow(*r, shadow.spread);
                unsafe { frame.dc.FillRoundedRectangle(&rounded(grown, radius + shadow.spread), frame.brush(Color { a: 1.0, ..shadow.color })) };
            }
        })?;
        let blur = effect(frame, &CLSID_D2D1Shadow, &shape)?;
        // CSS's blur radius is twice the Gaussian's standard deviation.
        prop(&blur, D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION.0, D2D1_PROPERTY_TYPE_FLOAT, &(shadow.blur / 2.0))?;
        prop(&blur, D2D1_SHADOW_PROP_COLOR.0, D2D1_PROPERTY_TYPE_VECTOR4, &vector(shadow.color))?;
        draw_image(frame, &output(&blur)?, Vector2 { X: 0.0, Y: shadow.y });
    }
    Ok(())
}

/// A shadow cast inward from a box's edge, as CSS's `inset` box shadow:
/// what lies outside the box, offset and blurred, seen through the box.
fn inset_shadow(frame: &Frame, r: Rect, radius: f32, y: f32, blur: f32, color: Color) -> Result<()> {
    let outside = picture(frame, || unsafe {
        let factory = &frame.gfx.factory;
        let reach = grow(r, blur + y.abs() + 1.0);
        let around: ID2D1Geometry = factory.CreateRectangleGeometry(&D2D_RECT_F { left: reach.x, top: reach.y, right: reach.x + reach.w, bottom: reach.y + reach.h }).unwrap().cast().unwrap();
        let inside = geometry(frame, r, radius).unwrap();
        let ring = factory.CreateGeometryGroup(D2D1_FILL_MODE_ALTERNATE, &[Some(around), Some(inside)]).unwrap();
        frame.dc.FillGeometry(&ring, frame.brush(Color { a: 1.0, ..color }), None);
    })?;
    let blurred = effect(frame, &CLSID_D2D1Shadow, &outside)?;
    prop(&blurred, D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION.0, D2D1_PROPERTY_TYPE_FLOAT, &(blur / 2.0))?;
    prop(&blurred, D2D1_SHADOW_PROP_COLOR.0, D2D1_PROPERTY_TYPE_VECTOR4, &vector(color))?;
    within(frame, r, radius, || draw_image(frame, &output(&blurred).unwrap(), Vector2 { X: 0.0, Y: y }))
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

/// The backdrop inside `area` (DIPs from the panel's corner), its edges
/// carried on outward, as SVG filters do with `edgeMode="duplicate"`.
fn backdrop_within(frame: &Frame, backdrop: (&ID2D1Bitmap1, Vector2), area: Rect) -> Result<ID2D1Effect> {
    let (bitmap, at) = backdrop;
    // The backdrop placed so the panel's corner is at the origin.
    let image = effect_input(&frame.dc, bitmap)?;
    let placed = effect(frame, &CLSID_D2D12DAffineTransform, &image)?;
    prop(&placed, D2D1_2DAFFINETRANSFORM_PROP_TRANSFORM_MATRIX.0, D2D1_PROPERTY_TYPE_MATRIX_3X2, &Matrix3x2::translation(-at.X, -at.Y))?;
    let crop = effect(frame, &CLSID_D2D1Crop, &output(&placed)?)?;
    prop(&crop, D2D1_CROP_PROP_RECT.0, D2D1_PROPERTY_TYPE_VECTOR4, &[area.x, area.y, area.x + area.w, area.y + area.h])?;
    let border = effect(frame, &CLSID_D2D1Border, &output(&crop)?)?;
    prop(&border, D2D1_BORDER_PROP_EDGE_MODE_X.0, D2D1_PROPERTY_TYPE_ENUM, &D2D1_BORDER_EDGE_MODE_CLAMP.0)?;
    prop(&border, D2D1_BORDER_PROP_EDGE_MODE_Y.0, D2D1_PROPERTY_TYPE_ENUM, &D2D1_BORDER_EDGE_MODE_CLAMP.0)?;
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
/// free: its corners on the attached side lie beyond the edge.
fn paper(frame: &Frame, ground: &Ground) -> Result<()> {
    let theme = ground.theme;
    let (w, h) = ground.size;
    let radius = theme.radius;
    let slab = match ground.edge {
        Edge::Left => Rect { x: -radius, y: 0.0, w: w + radius, h },
        Edge::Right => Rect { x: 0.0, y: 0.0, w: w + radius, h },
        Edge::Top => Rect { x: 0.0, y: -radius, w, h: h + radius },
    };
    shadows(frame, &[(slab, radius)], &theme.shadows)?;
    unsafe {
        // The rule rings the slab just outside it.
        frame.dc.FillRoundedRectangle(&rounded(grow(slab, 1.0), radius + 1.0), frame.brush(theme.stroke));
        frame.dc.FillRoundedRectangle(&rounded(slab, radius), frame.brush(theme.paper));
    }
    Ok(())
}

/// Windows 11: one acrylic sheet. The desktop behind it, blurred and a
/// little more saturated; a luminosity layer that evens out how bright it
/// is; a thin tint; and a faint grain.
fn fluent(frame: &Frame, ground: &Ground) -> Result<()> {
    let theme = ground.theme;
    let sheet = Rect { x: 0.0, y: 0.0, w: ground.size.0, h: ground.size.1 };
    shadows(frame, &[(sheet, theme.radius)], &theme.shadows)?;
    unsafe { frame.dc.FillRoundedRectangle(&rounded(grow(sheet, 1.0), theme.radius + 1.0), frame.brush(theme.stroke)) };
    within(frame, sheet, theme.radius, || {
        let area = D2D_RECT_F { left: 0.0, top: 0.0, right: sheet.w, bottom: sheet.h };
        match ground.backdrop.map(|backdrop| acrylic(frame, backdrop, sheet, theme.luminosity)) {
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
        }
        unsafe {
            frame.dc.FillRectangle(&area, frame.brush(theme.tint));
            if let Ok(brush) = grain(frame) {
                frame.dc.FillRectangle(&area, &brush);
            }
        }
    })
}

/// The desktop behind the sheet blurred and a little more saturated, and
/// the same with the luminosity layer's brightness in place of its own:
/// its hue and saturation, `luminosity`'s lightness.
fn acrylic(frame: &Frame, backdrop: (&ID2D1Bitmap1, Vector2), sheet: Rect, luminosity: Color) -> Result<(ID2D1Image, ID2D1Image)> {
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

/// A tile of faint grey grain, repeated.
fn grain(frame: &Frame) -> Result<windows::Win32::Graphics::Direct2D::ID2D1BitmapBrush1> {
    // The same grain every time: a fixed sequence, not the clock.
    let mut seed = 0x2545_F491u32;
    let mut pixels = Vec::with_capacity((GRAIN * GRAIN * 4) as usize);
    for _ in 0..GRAIN * GRAIN {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        // Premultiplied grey at the grain's strength.
        let value = ((seed >> 24) as f32 * GRAIN_ALPHA).round() as u8;
        pixels.extend_from_slice(&[value, value, value, (GRAIN_ALPHA * 255.0).round() as u8]);
    }
    let properties = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
        ..Default::default()
    };
    unsafe {
        let tile = frame.dc.CreateBitmap(D2D_SIZE_U { width: GRAIN, height: GRAIN }, Some(pixels.as_ptr().cast()), GRAIN * 4, &properties)?;
        let brush = D2D1_BITMAP_BRUSH_PROPERTIES1 {
            extendModeX: D2D1_EXTEND_MODE_WRAP,
            extendModeY: D2D1_EXTEND_MODE_WRAP,
            interpolationMode: windows::Win32::Graphics::Direct2D::D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
        };
        frame.dc.CreateBitmapBrush(&tile, Some(&brush), None)
    }
}

/// 磨砂玻璃: every lane and the bar a piece of glass. Each bends the desktop
/// at its rim, frosts it as much as legibility needs, takes the theme's
/// tint, glows faintly inside its edge and catches light along it.
fn glass(frame: &Frame, ground: &Ground) -> Result<()> {
    let theme = ground.theme;
    let pieces: Vec<(Rect, f32)> = ground.lanes.iter().chain([&ground.bar]).map(|r| (*r, theme.radius)).collect();
    shadows(frame, &pieces, &theme.shadows)?;
    let tint = theme.glass.alpha(theme.glass_clear + theme.glass_frosted * ground.frost);
    for (piece, radius) in &pieces {
        if let Some(backdrop) = ground.backdrop {
            let lens = lens(frame, backdrop, *piece, *radius, ground.frost * FROST_BLUR)?;
            within(frame, *piece, *radius, || draw_image(frame, &lens, Vector2::zero()))?;
        }
        unsafe { frame.dc.FillRoundedRectangle(&rounded(*piece, *radius), frame.brush(tint)) };
        // Light caught just inside the edge: strongest from above.
        let white = |a| Color::hex(0xFFFFFF, a);
        inset_shadow(frame, *piece, *radius, 1.0, 2.0, white(0.7))?;
        inset_shadow(frame, *piece, *radius, -1.0, 2.0, white(0.25))?;
        inset_shadow(frame, *piece, *radius, 0.0, 18.0, white(0.12))?;
        rim(frame, *piece, *radius)?;
    }
    Ok(())
}

/// The desktop behind a piece as its glass shows it: frosted by `blur`, bent
/// outward across the rim, and a little more saturated.
fn lens(frame: &Frame, backdrop: (&ID2D1Bitmap1, Vector2), piece: Rect, radius: f32, blur: f32) -> Result<ID2D1Image> {
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
    let mut dpi = (0.0, 0.0);
    unsafe { frame.dc.GetDpi(&mut dpi.0, &mut dpi.1) };
    let px = dpi.0 / 96.0;
    let (w, h) = ((piece.w * px).round() as usize, (piece.h * px).round() as usize);
    let (hx, hy) = (piece.w / 2.0, piece.h / 2.0);
    let r = radius.min(hx).min(hy);
    let mut pixels = vec![0u8; w * h * 4];
    for row in 0..h {
        for column in 0..w {
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
            let i = (row * w + column) * 4;
            // BGRA, opaque.
            pixels[i] = 128;
            pixels[i + 1] = (128.0 + ny * y.signum() * strength * 127.0).round() as u8;
            pixels[i + 2] = (128.0 + nx * x.signum() * strength * 127.0).round() as u8;
            pixels[i + 3] = 255;
        }
    }
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
fn rim(frame: &Frame, piece: Rect, radius: f32) -> Result<()> {
    let white = |position, a| D2D1_GRADIENT_STOP { position, color: Color::hex(0xFFFFFF, a).d2d() };
    let stops = [white(0.0, 0.95), white(0.22, 0.25), white(0.5, 0.06), white(0.78, 0.2), white(1.0, 0.7)];
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
        let inner = geometry(frame, grow(piece, -1.5), radius - 1.5)?;
        let ring = frame.gfx.factory.CreateGeometryGroup(D2D1_FILL_MODE_ALTERNATE, &[Some(outer), Some(inner)])?;
        frame.dc.FillGeometry(&ring, &brush, None);
    }
    Ok(())
}
