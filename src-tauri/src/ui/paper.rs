//! 记录纸's slab: one sheet flush with the screen edge, rounded only where
//! it is free, lifted off the desktop by a soft shadow and ringed by a rule.

use windows::core::{Interface, Result};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COMPOSITE_MODE_SOURCE_OVER, D2D_RECT_F};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Effect, ID2D1Image, CLSID_D2D1Shadow, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_PROPERTY_TYPE_FLOAT, D2D1_PROPERTY_TYPE_VECTOR4,
    D2D1_ROUNDED_RECT, D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION, D2D1_SHADOW_PROP_COLOR,
};
use windows_numerics::{Matrix3x2, Vector2};

use super::gfx::{Color, Frame};
use super::theme::Theme;
use crate::settings::Edge;

/// Room the shadow needs around the slab (DIPs).
pub const MARGIN: f32 = 40.0;

/// A box shadow as CSS writes one: offset down, blur radius, spread, colour.
struct Shadow {
    y: f32,
    blur: f32,
    spread: f32,
    color: Color,
}

/// The slab's outline: a rounded rectangle whose corners on the attached side
/// lie beyond the screen edge, so only the free side shows rounded.
fn outline(edge: Edge, width: f32, height: f32, radius: f32, grow: f32) -> D2D1_ROUNDED_RECT {
    let (mut left, mut top, mut right, bottom) = (-grow, -grow, width + grow, height + grow);
    match edge {
        Edge::Left => left -= radius,
        Edge::Right => right += radius,
        Edge::Top => top -= radius,
    }
    let radius = (radius + grow).max(0.0);
    D2D1_ROUNDED_RECT { rect: D2D_RECT_F { left, top, right, bottom }, radiusX: radius, radiusY: radius }
}

/// The slab's shadows, blurred once for each size and kept while it lasts.
#[derive(Default)]
pub struct Slab {
    shadows: Option<(ShadowKey, Vec<(ID2D1Effect, f32)>)>,
}

type ShadowKey = (Edge, u32, u32, [u32; 4]);

impl Slab {
    /// Draws the slab, `width` × `height` DIPs, at the frame's origin.
    pub fn draw(&mut self, frame: &Frame, theme: &Theme, edge: Edge, width: f32, height: f32) -> Result<()> {
        let c = theme.shadow;
        let key = (edge, width.to_bits(), height.to_bits(), [c.r, c.g, c.b, c.a].map(f32::to_bits));
        if self.shadows.as_ref().is_none_or(|(at, _)| *at != key) {
            self.shadows = Some((key, shadows(frame, theme, edge, width, height)?));
        }
        let dc = &frame.dc;
        unsafe {
            for (effect, y) in &self.shadows.as_ref().unwrap().1 {
                dc.DrawImage(
                    &effect.GetOutput()?,
                    Some(&Vector2 { X: 0.0, Y: *y }),
                    None,
                    D2D1_INTERPOLATION_MODE_LINEAR,
                    D2D1_COMPOSITE_MODE_SOURCE_OVER,
                );
            }
            // The rule rings the slab just outside it.
            dc.FillRoundedRectangle(&outline(edge, width, height, theme.radius, 1.0), frame.brush(theme.rule));
            dc.FillRoundedRectangle(&outline(edge, width, height, theme.radius, 0.0), frame.brush(theme.surface));
        }
        Ok(())
    }
}

/// The slab's two shadows as CSS draws them, each with how far down it falls.
fn shadows(frame: &Frame, theme: &Theme, edge: Edge, width: f32, height: f32) -> Result<Vec<(ID2D1Effect, f32)>> {
    let specs = [
        Shadow { y: 18.0, blur: 40.0, spread: -12.0, color: theme.shadow },
        Shadow { y: 2.0, blur: 6.0, spread: 0.0, color: theme.shadow.alpha(0.12 / 0.35) },
    ];
    let dc = &frame.dc;
    let mut made = Vec::new();
    for shadow in &specs {
        // The shape, in the shadow's colour, blurred: CSS's blur radius is
        // twice the Gaussian's standard deviation.
        let shape = unsafe { dc.CreateCommandList()? };
        let (target, transform) = unsafe {
            let target = dc.GetTarget()?;
            let mut transform = Matrix3x2::default();
            dc.GetTransform(&mut transform);
            (target, transform)
        };
        unsafe {
            dc.SetTarget(&shape);
            dc.SetTransform(&Matrix3x2::identity());
            dc.FillRoundedRectangle(&outline(edge, width, height, theme.radius, shadow.spread), frame.brush(Color { a: 1.0, ..shadow.color }));
            shape.Close()?;
            dc.SetTarget(&target);
            dc.SetTransform(&transform);
        }
        let effect = unsafe { dc.CreateEffect(&CLSID_D2D1Shadow)? };
        let c = shadow.color;
        let color = [c.r, c.g, c.b, c.a];
        let deviation = shadow.blur / 2.0;
        unsafe {
            effect.SetInput(0, &shape.cast::<ID2D1Image>()?, true);
            effect.SetValue(D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &deviation.to_ne_bytes())?;
            effect.SetValue(
                D2D1_SHADOW_PROP_COLOR.0 as u32,
                D2D1_PROPERTY_TYPE_VECTOR4,
                std::slice::from_raw_parts(color.as_ptr().cast::<u8>(), size_of_val(&color)),
            )?;
        }
        made.push((effect, shadow.y));
    }
    Ok(made)
}
