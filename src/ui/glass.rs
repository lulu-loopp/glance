//! A window drawn over panes of frosted glass: the screen behind it,
//! blurred by the system's compositor as it changes (Windows 11's host
//! backdrop), under what Glance draws with Direct2D. For the overlay, which
//! floats over games and pictures that move, the menus and what floats over
//! the screen as they do (the notice at its edge, what is carried off the
//! station), each pane under its own drawing and placed as it is: turned,
//! made larger, faded.

use std::cell::RefCell;
use std::rc::Rc;

use windows::core::{implement, Interface, Result, BOOL, GUID, HSTRING, PCWSTR};
use windows::Foundation::{IPropertyValue, PropertyValue};
use windows::Graphics::DirectX::{DirectXAlphaMode, DirectXPixelFormat};
use windows::Graphics::Effects::{IGraphicsEffect, IGraphicsEffect_Impl, IGraphicsEffectSource, IGraphicsEffectSource_Impl};
use windows::Graphics::SizeInt32;
use windows::System::DispatcherQueueController;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{
    CompositionDrawingSurface, CompositionEffectBrush, CompositionEffectSourceParameter, CompositionGraphicsDevice, Compositor, ContainerVisual, RectangleClip, SpriteVisual,
};
use windows::Win32::Foundation::{E_BOUNDS, E_INVALIDARG, HWND, POINT};
use windows::Win32::Graphics::Direct2D::{ID2D1DeviceContext, CLSID_D2D1GaussianBlur, CLSID_D2D1Saturation};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_HOSTBACKDROPBRUSH};
use windows::Win32::System::WinRT::Composition::{ICompositionDrawingSurfaceInterop, ICompositorDesktopInterop, ICompositorInterop};
use windows::Win32::System::WinRT::Graphics::Direct2D::{IGraphicsEffectD2D1Interop, IGraphicsEffectD2D1Interop_Impl, GRAPHICS_EFFECT_PROPERTY_MAPPING};
use windows::Win32::System::WinRT::{CreateDispatcherQueueController, DispatcherQueueOptions, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT};
use windows_numerics::{Matrix3x2, Matrix4x4, Vector2, Vector3};

use super::gfx::{Frame, Gfx};

/// How much the overlay's glass blurs what is behind beyond the backdrop's own blur (a deviation, DIPs), and how much
/// colour it leaves it (1 as it is): grey, for its words over a game's colours.
pub const BLUR: f32 = 20.0;
pub const SATURATION: f32 = 0.5;

thread_local! {
    /// This thread's compositor, with the dispatcher queue it needs.
    static COMPOSITOR: RefCell<Option<(DispatcherQueueController, Compositor)>> = const { RefCell::new(None) };
}

fn compositor() -> Result<Compositor> {
    COMPOSITOR.with(|cell| {
        if let Some((_, compositor)) = cell.borrow().as_ref() {
            return Ok(compositor.clone());
        }
        let options = DispatcherQueueOptions { dwSize: size_of::<DispatcherQueueOptions>() as u32, threadType: DQTYPE_THREAD_CURRENT, apartmentType: DQTAT_COM_NONE };
        let queue = unsafe { CreateDispatcherQueueController(options)? };
        let compositor = Compositor::new()?;
        *cell.borrow_mut() = Some((queue, compositor.clone()));
        Ok(compositor)
    })
}

/// A Direct2D effect, as the compositor takes one: its class, its
/// properties by index, and the one source it works on.
#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
struct Effect {
    id: GUID,
    properties: Vec<IPropertyValue>,
    source: IGraphicsEffectSource,
}

impl IGraphicsEffect_Impl for Effect_Impl {
    fn Name(&self) -> Result<HSTRING> {
        Ok(HSTRING::new())
    }
    fn SetName(&self, _: &HSTRING) -> Result<()> {
        Ok(())
    }
}

impl IGraphicsEffectSource_Impl for Effect_Impl {}

impl IGraphicsEffectD2D1Interop_Impl for Effect_Impl {
    fn GetEffectId(&self) -> Result<GUID> {
        Ok(self.id)
    }
    fn GetNamedPropertyMapping(&self, _: &PCWSTR, _: *mut u32, _: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING) -> Result<()> {
        Err(E_INVALIDARG.into())
    }
    fn GetPropertyCount(&self) -> Result<u32> {
        Ok(self.properties.len() as u32)
    }
    fn GetProperty(&self, index: u32) -> Result<IPropertyValue> {
        self.properties.get(index as usize).cloned().ok_or_else(|| E_BOUNDS.into())
    }
    fn GetSource(&self, index: u32) -> Result<IGraphicsEffectSource> {
        if index == 0 {
            Ok(self.source.clone())
        } else {
            Err(E_BOUNDS.into())
        }
    }
    fn GetSourceCount(&self) -> Result<u32> {
        Ok(1)
    }
}

/// A pane of frosted glass in a window: `size` large and its corners `radius` round (physical px), placed from its
/// corner onto the window by `at` (physical px: moved, and turned and made larger or smaller as what is drawn over it
/// is), `seen` of it shown (0 none, 1 all).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pane {
    pub size: (f32, f32),
    pub radius: f32,
    pub at: Matrix3x2,
    pub seen: f32,
}

impl Pane {
    /// The pane under a box (`x`, `y`, `w`, `h` and its corners `radius` round, DIPs) placed onto the window by `placed`
    /// (DIPs), `px` physical px a DIP, `seen` of it shown: made as large as it is in physical px, so that it is
    /// frosted as the rest of the window's glass is (its blur made for the window's scale).
    pub fn placed(placed: Matrix3x2, (x, y, w, h): (f32, f32, f32, f32), radius: f32, px: f32, seen: f32) -> Pane {
        let at = Matrix3x2::scale(1.0 / px, 1.0 / px) * Matrix3x2::translation(x, y) * placed * Matrix3x2::scale(px, px);
        Pane { size: (w * px, h * px), radius: radius * px, at, seen }
    }
}

/// One of what a window shows, each over the one before: a pane of frosted glass (none, where nothing of it is
/// glass), and what is drawn over it, on as much of the window as `reach` says (left, top, width, height: physical px).
/// What is drawn of one is under the next's glass, as one sheet of glass is under another laid on it.
#[derive(Clone, Copy, Debug)]
pub struct Sheet {
    pub pane: Option<Pane>,
    pub reach: (i32, i32, u32, u32),
}

/// A sheet as the compositor shows it: its pane (once it has had one, where the system has a backdrop to frost) and
/// that pane's corners and how much of it shows, and what is drawn over it, on its surface (none while the window is
/// away), as large as it has been at most.
struct Layer {
    pane: Option<(SpriteVisual, RectangleClip)>,
    seen: f32,
    content: SpriteVisual,
    surface: Option<CompositionDrawingSurface>,
    size: (u32, u32),
}

/// What a window shows: sheets, each a pane of frosted glass (where the system has a backdrop to frost; Windows 10
/// has none for a desktop window, and the panes are then left clear) under Glance's drawing.
pub struct Frosted {
    compositor: Compositor,
    _target: DesktopWindowTarget,
    root: ContainerVisual,
    layers: Vec<Layer>,
    /// Whether the system frosts what is behind the window, and the frosting its panes are drawn with, for the scale
    /// it was made for.
    offered: bool,
    brush: Option<(f32, CompositionEffectBrush)>,
    /// The device the drawing is made on.
    gfx: Rc<Gfx>,
    graphics: CompositionGraphicsDevice,
    /// Its window, whether the compositor is asked for what is behind it, and how frosted its panes are (see
    /// `set_frost`).
    hwnd: HWND,
    frosting: bool,
    frost: f32,
    /// How much it blurs what is behind beyond the backdrop's own blur (DIPs), and how much colour it leaves it.
    blur: f32,
    saturation: f32,
}

impl Frosted {
    /// The glass of `hwnd`, blurring what is behind by `blur` (DIPs) beyond the backdrop's own blur and leaving it
    /// `saturation` of its colour (1 as it is): the overlay's (`BLUR`, `SATURATION`), or the skin's of what a menu is
    /// opened on.
    pub fn new(gfx: &Rc<Gfx>, hwnd: HWND, (blur, saturation): (f32, f32)) -> Result<Self> {
        let compositor = compositor()?;
        let target = unsafe { compositor.cast::<ICompositorDesktopInterop>()?.CreateDesktopWindowTarget(hwnd, true)? };
        let root = compositor.CreateContainerVisual()?;
        target.SetRoot(&root)?;
        // The window's backdrop, offered only where the system has one.
        let backdrop = BOOL::from(true);
        let offered = unsafe { DwmSetWindowAttribute(hwnd, DWMWA_USE_HOSTBACKDROPBRUSH, &backdrop as *const _ as *const _, size_of::<BOOL>() as u32) }.is_ok();
        let graphics = graphics(&compositor, gfx)?;
        Ok(Frosted { compositor, _target: target, root, layers: Vec::new(), offered, brush: None, gfx: gfx.clone(), graphics, hwnd, frosting: true, frost: 1.0, blur, saturation })
    }

    /// Draws one frame on `gfx` (the thread's device: a new one, after the
    /// last was lost, is taken up), at `scale` physical pixels per DIP, on a
    /// window `size` physical pixels large, with the glass at `pane` (left,
    /// top, width, height and corner radius, physical pixels).
    pub fn draw(&mut self, gfx: &Rc<Gfx>, size: (u32, u32), scale: f32, (left, top, width, height, radius): (f32, f32, f32, f32, f32), paint: impl FnOnce(&Frame)) -> Result<()> {
        let pane = Pane { size: (width, height), radius, at: Matrix3x2::translation(left, top), seen: 1.0 };
        let mut paint = Some(paint);
        self.draw_sheets(gfx, size, scale, &[Sheet { pane: Some(pane), reach: (0, 0, size.0, size.1) }], |_, frame| {
            if let Some(paint) = paint.take() {
                paint(frame);
            }
        })
    }

    /// Draws one frame of `sheets` on `gfx` (as `draw` does), the first at the bottom, on a window `size` physical
    /// pixels large: each its pane, and over it what `paint` draws for it (told which it is), from the corner of its
    /// reach.
    pub fn draw_sheets(&mut self, gfx: &Rc<Gfx>, size: (u32, u32), scale: f32, sheets: &[Sheet], mut paint: impl FnMut(usize, &Frame)) -> Result<()> {
        if !Rc::ptr_eq(gfx, &self.gfx) {
            self.graphics = graphics(&self.compositor, gfx)?;
            self.gfx = gfx.clone();
            // (Their surfaces were the device's that is gone.)
            for layer in &mut self.layers {
                layer.surface = None;
            }
        }
        self.root.SetSize(Vector2 { X: size.0 as f32, Y: size.1 as f32 })?;
        let children = self.root.Children()?;
        while self.layers.len() > sheets.len() {
            let Some(layer) = self.layers.pop() else { break };
            // (Its pane is out of what is composed already while nothing is frosted.)
            if let (Some((sprite, _)), true) = (&layer.pane, self.frosting) {
                children.Remove(sprite)?;
            }
            children.Remove(&layer.content)?;
        }
        while self.layers.len() < sheets.len() {
            let content = self.compositor.CreateSpriteVisual()?;
            children.InsertAtTop(&content)?;
            self.layers.push(Layer { pane: None, seen: 0.0, content, surface: None, size: (0, 0) });
        }
        // A pane the compositor will not frost (an effect it cannot run) is
        // let go of, and the glass left clear: that is no lost device.
        if self.offered && self.place(scale, sheets).is_err() {
            for layer in &mut self.layers {
                if let (Some((sprite, _)), true) = (layer.pane.take(), self.frosting) {
                    let _ = children.Remove(&sprite);
                }
            }
            self.offered = false;
            self.brush = None;
        }
        for (at, (layer, sheet)) in self.layers.iter_mut().zip(sheets).enumerate() {
            let (left, top, width, height) = sheet.reach;
            // (Grown as it needs, never made smaller: what it draws moves and turns from frame to frame.)
            let size = (layer.size.0.max(width.max(1)), layer.size.1.max(height.max(1)));
            let pixels = SizeInt32 { Width: size.0 as i32, Height: size.1 as i32 };
            let surface = match &layer.surface {
                Some(surface) => {
                    if layer.size != size {
                        surface.Resize(pixels)?;
                    }
                    surface.clone()
                }
                None => {
                    let surface = self.graphics.CreateDrawingSurface2(pixels, DirectXPixelFormat::B8G8R8A8UIntNormalized, DirectXAlphaMode::Premultiplied)?;
                    layer.content.SetBrush(&self.compositor.CreateSurfaceBrushWithSurface(&surface)?)?;
                    layer.surface = Some(surface.clone());
                    surface
                }
            };
            layer.size = size;
            layer.content.SetOffset(Vector3 { X: left as f32, Y: top as f32, Z: 0.0 })?;
            layer.content.SetSize(Vector2 { X: size.0 as f32, Y: size.1 as f32 })?;
            let interop: ICompositionDrawingSurfaceInterop = surface.cast()?;
            // Drawn apart first, and put on the surface only once drawn whole (see `Frame::apart`).
            let frame = Frame::whole(gfx, size, scale, |frame| paint(at, frame))?;
            let mut offset = POINT::default();
            let dc: ID2D1DeviceContext = unsafe { interop.BeginDraw(None, &mut offset)? };
            // (The surface's drawing ended whatever became of the putting: left begun, nothing more was drawn on it.)
            let put = frame.put(&dc, offset);
            let ended = unsafe { interop.EndDraw() };
            put?;
            ended?;
        }
        Ok(())
    }

    /// Puts each sheet's pane where it says, frosted for `scale` (see `frost`), as much of it shown as it says and the
    /// glass is frosted (see `set_frost`); a sheet with none, its pane (had before) hidden.
    fn place(&mut self, scale: f32, sheets: &[Sheet]) -> Result<()> {
        if self.brush.as_ref().is_none_or(|(made, _)| *made != scale) {
            let brush = frost(&self.compositor, scale, (self.blur, self.saturation))?;
            for (sprite, _) in self.layers.iter().filter_map(|layer| layer.pane.as_ref()) {
                sprite.SetBrush(&brush)?;
            }
            self.brush = Some((scale, brush));
        }
        let Some((_, brush)) = &self.brush else { return Ok(()) };
        let children = self.root.Children()?;
        for (layer, sheet) in self.layers.iter_mut().zip(sheets) {
            let Some(pane) = sheet.pane else {
                if let Some((sprite, _)) = &layer.pane {
                    sprite.SetIsVisible(false)?;
                }
                continue;
            };
            let (sprite, clip) = match &layer.pane {
                Some(made) => made,
                None => {
                    let sprite = self.compositor.CreateSpriteVisual()?;
                    let clip = self.compositor.CreateRectangleClip()?;
                    sprite.SetClip(&clip)?;
                    sprite.SetBrush(brush)?;
                    if self.frosting {
                        children.InsertBelow(&sprite, &layer.content)?;
                    }
                    layer.pane.insert((sprite, clip))
                }
            };
            layer.seen = pane.seen.clamp(0.0, 1.0);
            sprite.SetIsVisible(true)?;
            sprite.SetOpacity(self.frost * layer.seen)?;
            sprite.SetSize(Vector2 { X: pane.size.0, Y: pane.size.1 })?;
            sprite.SetTransformMatrix(four_by_four(pane.at))?;
            clip.SetRight(pane.size.0)?;
            clip.SetBottom(pane.size.1)?;
            let corner = Vector2 { X: pane.radius, Y: pane.radius };
            clip.SetTopLeftRadius(corner)?;
            clip.SetTopRightRadius(corner)?;
            clip.SetBottomLeftRadius(corner)?;
            clip.SetBottomRightRadius(corner)?;
        }
        Ok(())
    }

    /// How frosted the glass is: 1, the screen behind blurred; 0, as it
    /// is (the frosting gone, what is drawn over it standing).
    ///
    /// With no frosting at all the compositor is not asked for what is
    /// behind the window, and the panes are out of what it composes: asked,
    /// it must draw the game behind into the window every frame, which
    /// costs the game frames whether or not any of it then shows.
    pub fn set_frost(&mut self, frost: f32) {
        if !self.offered {
            return;
        }
        let frost = frost.clamp(0.0, 1.0);
        self.frost = frost;
        let wanted = frost > 0.0;
        if wanted != self.frosting {
            let asked = BOOL::from(wanted);
            let _ = unsafe { DwmSetWindowAttribute(self.hwnd, DWMWA_USE_HOSTBACKDROPBRUSH, &asked as *const _ as *const _, size_of::<BOOL>() as u32) };
            if let Ok(children) = self.root.Children() {
                for layer in &self.layers {
                    if let Some((sprite, _)) = &layer.pane {
                        let _ = if wanted { children.InsertBelow(sprite, &layer.content) } else { children.Remove(sprite) };
                    }
                }
            }
            self.frosting = wanted;
        }
        for layer in &self.layers {
            if let Some((sprite, _)) = &layer.pane {
                let _ = sprite.SetOpacity(frost * layer.seen);
            }
        }
    }

    /// Lets go of the drawing memory while the window is away.
    pub fn release(&mut self) {
        for layer in &mut self.layers {
            if layer.surface.take().is_some() {
                let _ = layer.content.SetBrush(None);
            }
        }
    }
}

/// The backdrop, frosted: blurred further by `blur` (DIPs) at `scale`, `saturation` of its colour left.
fn frost(compositor: &Compositor, scale: f32, (blur, saturation): (f32, f32)) -> Result<CompositionEffectBrush> {
    let single = |v: f32| -> Result<IPropertyValue> { PropertyValue::CreateSingle(v)?.cast() };
    let whole = |v: u32| -> Result<IPropertyValue> { PropertyValue::CreateUInt32(v)?.cast() };
    let backdrop: IGraphicsEffectSource = CompositionEffectSourceParameter::Create(&HSTRING::from("backdrop"))?.cast()?;
    let softened: IGraphicsEffect = Effect { id: CLSID_D2D1Saturation, properties: vec![single(saturation)?], source: backdrop }.into();
    // Blurred with its edges hard: the pane's rim is not darkened by
    // the transparent nothing past it.
    let blurred: IGraphicsEffect = Effect { id: CLSID_D2D1GaussianBlur, properties: vec![single(blur * scale)?, whole(1)?, whole(1)?], source: softened.cast()? }.into();
    let brush = compositor.CreateEffectFactory(&blurred)?.CreateBrush()?;
    brush.SetSourceParameter(&HSTRING::from("backdrop"), &compositor.CreateHostBackdropBrush()?)?;
    Ok(brush)
}

/// A placing in the plane (`Matrix3x2`) as the compositor places a visual: its z left as it is.
fn four_by_four(m: Matrix3x2) -> Matrix4x4 {
    Matrix4x4 {
        M11: m.M11,
        M12: m.M12,
        M13: 0.0,
        M14: 0.0,
        M21: m.M21,
        M22: m.M22,
        M23: 0.0,
        M24: 0.0,
        M31: 0.0,
        M32: 0.0,
        M33: 1.0,
        M34: 0.0,
        M41: m.M31,
        M42: m.M32,
        M43: 0.0,
        M44: 1.0,
    }
}

/// The compositor's drawing device on `gfx`'s Direct2D device.
fn graphics(compositor: &Compositor, gfx: &Gfx) -> Result<CompositionGraphicsDevice> {
    unsafe { compositor.cast::<ICompositorInterop>()?.CreateGraphicsDevice(&gfx.device) }
}
