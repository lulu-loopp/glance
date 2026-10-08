//! A window drawn over a pane of frosted glass: the screen behind it,
//! blurred by the system's compositor as it changes (Windows 11's host
//! backdrop), under what Glance draws with Direct2D. For the overlay, which
//! floats over games and pictures that move.

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
    CompositionDrawingSurface, CompositionEffectSourceParameter, CompositionGraphicsDevice, Compositor, ContainerVisual, RectangleClip, SpriteVisual,
};
use windows::Win32::Foundation::{E_BOUNDS, E_INVALIDARG, HWND, POINT};
use windows::Win32::Graphics::Direct2D::{ID2D1DeviceContext, CLSID_D2D1GaussianBlur, CLSID_D2D1Saturation};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_HOSTBACKDROPBRUSH};
use windows::Win32::System::WinRT::Composition::{ICompositionDrawingSurfaceInterop, ICompositorDesktopInterop, ICompositorInterop};
use windows::Win32::System::WinRT::Graphics::Direct2D::{IGraphicsEffectD2D1Interop, IGraphicsEffectD2D1Interop_Impl, GRAPHICS_EFFECT_PROPERTY_MAPPING};
use windows::Win32::System::WinRT::{CreateDispatcherQueueController, DispatcherQueueOptions, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT};
use windows_numerics::{Vector2, Vector3};

use super::gfx::{Frame, Gfx};

/// How much the glass blurs what is behind beyond the backdrop's own blur
/// (a deviation, DIPs), and how much colour it leaves it (1 as it is).
const BLUR: f32 = 20.0;
pub const SATURATION: f32 = 0.5;
/// How much the system's backdrop is blurred already (a deviation, DIPs;
/// acrylic's).
const HOST_BLUR: f32 = 30.0;

/// How much the glass blurs what is behind in all (a deviation, DIPs): for
/// drawing it where no compositor does (the settings' preview).
pub fn blur() -> f32 {
    (HOST_BLUR * HOST_BLUR + BLUR * BLUR).sqrt()
}

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

/// The pane of glass: the backdrop seen through it, and its corners.
struct Pane {
    sprite: SpriteVisual,
    clip: RectangleClip,
    /// The scale its blur was made for.
    scale: f32,
}

/// What a window shows: a pane of frosted glass (where the system has a
/// backdrop to frost; Windows 10 has none for a desktop window, and the
/// pane is then left clear), and Glance's drawing over it.
pub struct Frosted {
    compositor: Compositor,
    _target: DesktopWindowTarget,
    root: ContainerVisual,
    pane: Option<Pane>,
    content: SpriteVisual,
    /// The device the drawing is made on, and its surface (none while the
    /// window is away).
    gfx: Rc<Gfx>,
    graphics: CompositionGraphicsDevice,
    surface: Option<CompositionDrawingSurface>,
    size: (u32, u32),
}

impl Frosted {
    pub fn new(gfx: &Rc<Gfx>, hwnd: HWND) -> Result<Self> {
        let compositor = compositor()?;
        let target = unsafe { compositor.cast::<ICompositorDesktopInterop>()?.CreateDesktopWindowTarget(hwnd, true)? };
        let root = compositor.CreateContainerVisual()?;
        target.SetRoot(&root)?;
        // The window's backdrop, offered only where the system has one.
        let backdrop = BOOL::from(true);
        let offered = unsafe { DwmSetWindowAttribute(hwnd, DWMWA_USE_HOSTBACKDROPBRUSH, &backdrop as *const _ as *const _, size_of::<BOOL>() as u32) }.is_ok();
        let pane = if offered {
            let sprite = compositor.CreateSpriteVisual()?;
            let clip = compositor.CreateRectangleClip()?;
            sprite.SetClip(&clip)?;
            root.Children()?.InsertAtTop(&sprite)?;
            Some(Pane { sprite, clip, scale: 0.0 })
        } else {
            None
        };
        let content = compositor.CreateSpriteVisual()?;
        root.Children()?.InsertAtTop(&content)?;
        let graphics = graphics(&compositor, gfx)?;
        Ok(Frosted { compositor, _target: target, root, pane, content, gfx: gfx.clone(), graphics, surface: None, size: (0, 0) })
    }

    /// Draws one frame on `gfx` (the thread's device: a new one, after the
    /// last was lost, is taken up), at `scale` physical pixels per DIP, on a
    /// window `size` physical pixels large, with the glass at `pane` (left,
    /// top, width, height and corner radius, physical pixels).
    pub fn draw(&mut self, gfx: &Rc<Gfx>, size: (u32, u32), scale: f32, pane: (f32, f32, f32, f32, f32), paint: impl FnOnce(&Frame)) -> Result<()> {
        if !Rc::ptr_eq(gfx, &self.gfx) {
            self.graphics = graphics(&self.compositor, gfx)?;
            self.gfx = gfx.clone();
            self.surface = None;
        }
        let whole = Vector2 { X: size.0 as f32, Y: size.1 as f32 };
        self.root.SetSize(whole)?;
        self.content.SetSize(whole)?;
        // A pane the compositor will not frost (an effect it cannot run) is
        // let go of, and the glass left clear: that is no lost device.
        if let Some(glass) = &mut self.pane {
            if place(glass, &self.compositor, scale, pane).is_err() {
                let _ = self.root.Children().and_then(|children| children.Remove(&glass.sprite));
                self.pane = None;
            }
        }
        let pixels = SizeInt32 { Width: size.0.max(1) as i32, Height: size.1.max(1) as i32 };
        let surface = match &self.surface {
            Some(surface) => {
                if self.size != size {
                    surface.Resize(pixels)?;
                }
                surface.clone()
            }
            None => {
                let surface = self.graphics.CreateDrawingSurface2(pixels, DirectXPixelFormat::B8G8R8A8UIntNormalized, DirectXAlphaMode::Premultiplied)?;
                self.content.SetBrush(&self.compositor.CreateSurfaceBrushWithSurface(&surface)?)?;
                self.surface = Some(surface.clone());
                surface
            }
        };
        self.size = size;
        let interop: ICompositionDrawingSurfaceInterop = surface.cast()?;
        let mut offset = POINT::default();
        let dc: ID2D1DeviceContext = unsafe { interop.BeginDraw(None, &mut offset)? };
        let frame = Frame::begun(gfx, dc, offset, scale);
        if let Ok(frame) = &frame {
            paint(frame);
        }
        unsafe { interop.EndDraw()? };
        frame.map(|_| ())
    }

    /// Lets go of the drawing memory while the window is away.
    pub fn release(&mut self) {
        if self.surface.take().is_some() {
            let _ = self.content.SetBrush(None);
        }
    }
}

/// The backdrop, frosted: blurred further at `scale`, its colour
/// softened.
fn frost(compositor: &Compositor, scale: f32) -> Result<windows::UI::Composition::CompositionEffectBrush> {
    let single = |v: f32| -> Result<IPropertyValue> { PropertyValue::CreateSingle(v)?.cast() };
    let whole = |v: u32| -> Result<IPropertyValue> { PropertyValue::CreateUInt32(v)?.cast() };
    let backdrop: IGraphicsEffectSource = CompositionEffectSourceParameter::Create(&HSTRING::from("backdrop"))?.cast()?;
    let softened: IGraphicsEffect = Effect { id: CLSID_D2D1Saturation, properties: vec![single(SATURATION)?], source: backdrop }.into();
    // Blurred with its edges hard: the pane's rim is not darkened by
    // the transparent nothing past it.
    let blurred: IGraphicsEffect = Effect { id: CLSID_D2D1GaussianBlur, properties: vec![single(BLUR * scale)?, whole(1)?, whole(1)?], source: softened.cast()? }.into();
    let brush = compositor.CreateEffectFactory(&blurred)?.CreateBrush()?;
    brush.SetSourceParameter(&HSTRING::from("backdrop"), &compositor.CreateHostBackdropBrush()?)?;
    Ok(brush)
}

/// Puts the pane at `at` (left, top, width, height and corner radius,
/// physical pixels), frosted for `scale`.
fn place(glass: &mut Pane, compositor: &Compositor, scale: f32, at: (f32, f32, f32, f32, f32)) -> Result<()> {
    if glass.scale != scale {
        glass.sprite.SetBrush(&frost(compositor, scale)?)?;
        glass.scale = scale;
    }
    let (left, top, width, height, radius) = at;
    glass.sprite.SetOffset(Vector3 { X: left, Y: top, Z: 0.0 })?;
    glass.sprite.SetSize(Vector2 { X: width, Y: height })?;
    glass.clip.SetRight(width)?;
    glass.clip.SetBottom(height)?;
    let corner = Vector2 { X: radius, Y: radius };
    glass.clip.SetTopLeftRadius(corner)?;
    glass.clip.SetTopRightRadius(corner)?;
    glass.clip.SetBottomLeftRadius(corner)?;
    glass.clip.SetBottomRightRadius(corner)
}

/// The compositor's drawing device on `gfx`'s Direct2D device.
fn graphics(compositor: &Compositor, gfx: &Gfx) -> Result<CompositionGraphicsDevice> {
    unsafe { compositor.cast::<ICompositorInterop>()?.CreateGraphicsDevice(&gfx.device) }
}
