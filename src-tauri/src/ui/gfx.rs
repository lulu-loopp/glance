//! Drawing: Direct2D on a DirectComposition surface, text with DirectWrite.
//! Coordinates are in device-independent pixels (DIPs); the surface is in
//! physical pixels and the context scales between them.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::core::{w, Interface, Result, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_COMPOSITE_MODE_SOURCE_OVER, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext, ID2D1Factory1, ID2D1Image, ID2D1SolidColorBrush,
    D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_DRAW_TEXT_OPTIONS_CLIP,
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_MULTI_THREADED, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice2, IDCompositionDesktopDevice, IDCompositionSurface, IDCompositionTarget, IDCompositionVisual2,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory6, IDWriteFontCollection, IDWriteFontSetBuilder1, IDWriteTextFormat3, IDWriteTextLayout,
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_AXIS_TAG_WEIGHT, DWRITE_FONT_AXIS_TAG_WIDTH, DWRITE_FONT_AXIS_VALUE,
    DWRITE_FONT_FAMILY_MODEL_TYPOGRAPHIC, DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM};
use windows::Win32::Graphics::Dxgi::{IDXGIDevice, IDXGIDevice3};
use windows_numerics::Matrix3x2;

/// Archivo, the chart-paper skin's face, shipped inside the program.
const ARCHIVO: &[u8] = include_bytes!("../../fonts/Archivo.ttf");

/// A colour as a CSS-style straight (not premultiplied) RGBA.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
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

    /// For Direct2D, which takes colours straight and premultiplies itself.
    pub fn d2d(self) -> D2D1_COLOR_F {
        D2D1_COLOR_F { r: self.r, g: self.g, b: self.b, a: self.a }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    Archivo,
    Segoe,
    SegoeDisplay,
    /// The system's icon font: Segoe Fluent Icons on Windows 11, Segoe MDL2
    /// Assets before it; both put the same glyphs at the same code points.
    Icons,
}

/// A text style: face, size in DIPs, weight (100–900) and width (% of normal).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Font {
    pub family: Family,
    pub size: f32,
    pub weight: f32,
    pub width: f32,
}

impl Font {
    pub const fn new(family: Family, size: f32, weight: f32) -> Self {
        Font { family, size, weight, width: 100.0 }
    }

    pub const fn width(self, width: f32) -> Self {
        Font { width, ..self }
    }

    fn key(&self) -> (Family, u32, u32, u32) {
        (self.family, (self.size * 100.0) as u32, self.weight as u32, self.width as u32)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Start,
    End,
}

/// Devices and factories shared by every surface.
pub struct Gfx {
    pub factory: ID2D1Factory1,
    device: ID2D1Device,
    dxgi: IDXGIDevice3,
    pub dcomp: IDCompositionDesktopDevice,
    write: IDWriteFactory6,
    archivo: IDWriteFontCollection,
    icons: PCWSTR,
    formats: RefCell<HashMap<(Family, u32, u32, u32), IDWriteTextFormat3>>,
    /// Laid-out text, kept while frames keep drawing it: most of a panel's
    /// words and figures are the same from one frame to the next.
    layouts: RefCell<HashMap<LayoutKey, (IDWriteTextLayout, bool)>>,
}

type LayoutKey = (String, (Family, u32, u32, u32), u32, bool);

impl Gfx {
    pub fn new() -> Result<Self> {
        let mut d3d: Option<ID3D11Device> = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                Default::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut d3d),
                None,
                None,
            )?
        };
        let dxgi: IDXGIDevice = d3d.unwrap().cast()?;
        let factory: ID2D1Factory1 = unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_MULTI_THREADED, None)? };
        let device: ID2D1Device = unsafe { factory.CreateDevice(&dxgi)? };
        // Made from the Direct2D device, composition surfaces hand out device
        // contexts already aimed at themselves.
        let dcomp: IDCompositionDesktopDevice = unsafe { DCompositionCreateDevice2(&device)? };
        let write: IDWriteFactory6 = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let archivo = unsafe {
            let loader = write.CreateInMemoryFontFileLoader()?;
            write.RegisterFontFileLoader(&loader)?;
            let file = loader.CreateInMemoryFontFileReference(&write, ARCHIVO.as_ptr() as *const _, ARCHIVO.len() as u32, None)?;
            let builder = write.CreateFontSetBuilder()?;
            IDWriteFontSetBuilder1::AddFontFile(&builder, &file)?;
            write.CreateFontCollectionFromFontSet(&builder.CreateFontSet()?, DWRITE_FONT_FAMILY_MODEL_TYPOGRAPHIC)?.cast()?
        };
        let icons = unsafe {
            let system = write.GetSystemFontCollection(false, DWRITE_FONT_FAMILY_MODEL_TYPOGRAPHIC)?;
            let (mut index, mut fluent) = (0, BOOL(0));
            system.FindFamilyName(w!("Segoe Fluent Icons"), &mut index, &mut fluent)?;
            if fluent.as_bool() { w!("Segoe Fluent Icons") } else { w!("Segoe MDL2 Assets") }
        };
        Ok(Gfx { factory, device, dxgi: dxgi.cast()?, dcomp, write, archivo, icons, formats: RefCell::new(HashMap::new()), layouts: RefCell::new(HashMap::new()) })
    }

    fn format(&self, font: Font) -> IDWriteTextFormat3 {
        if let Some(format) = self.formats.borrow().get(&font.key()) {
            return format.clone();
        }
        let axes = [
            DWRITE_FONT_AXIS_VALUE { axisTag: DWRITE_FONT_AXIS_TAG_WEIGHT, value: font.weight },
            DWRITE_FONT_AXIS_VALUE { axisTag: DWRITE_FONT_AXIS_TAG_WIDTH, value: font.width },
        ];
        let (name, collection): (PCWSTR, Option<&IDWriteFontCollection>) = match font.family {
            Family::Archivo => (w!("Archivo"), Some(&self.archivo)),
            Family::Segoe => (w!("Segoe UI Variable Text"), None),
            Family::SegoeDisplay => (w!("Segoe UI Variable Display"), None),
            Family::Icons => (self.icons, None),
        };
        let format = unsafe {
            self.write
                .CreateTextFormat(name, collection, &axes, font.size, w!("zh-cn"))
                .expect("text format")
        };
        unsafe {
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP).unwrap();
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR).unwrap();
            // Text too long for its box ends in an ellipsis.
            let trimming = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, ..Default::default() };
            let sign = self.write.CreateEllipsisTrimmingSign(&format).unwrap();
            format.SetTrimming(&trimming, &sign).unwrap();
        }
        self.formats.borrow_mut().insert(font.key(), format.clone());
        format
    }

    /// Lays `text` out on one line, `width` DIPs wide at most.
    pub fn layout(&self, text: &str, font: Font, width: f32, align: Align) -> IDWriteTextLayout {
        let key = (text.to_string(), font.key(), width.max(0.0).to_bits(), align == Align::End);
        if let Some((layout, used)) = self.layouts.borrow_mut().get_mut(&key) {
            *used = true;
            return layout.clone();
        }
        let wide: Vec<u16> = text.encode_utf16().collect();
        let layout = unsafe { self.write.CreateTextLayout(&wide, &self.format(font), width.max(0.0), font.size * 2.0) }
            .expect("text layout");
        let alignment = if align == Align::End { DWRITE_TEXT_ALIGNMENT_TRAILING } else { DWRITE_TEXT_ALIGNMENT_LEADING };
        unsafe { layout.SetTextAlignment(alignment).unwrap() };
        self.layouts.borrow_mut().insert(key, (layout.clone(), true));
        layout
    }

    /// Gives back what drawing holds on to while nothing is on screen.
    pub fn trim(&self) {
        self.layouts.borrow_mut().clear();
        unsafe {
            self.device.ClearResources(0);
            self.dxgi.Trim();
        }
    }

    /// Forgets the text no frame has drawn since the last sweep.
    pub fn sweep(&self) {
        self.layouts.borrow_mut().retain(|_, (_, used)| std::mem::take(used));
    }

    /// How wide `text` is in `font`, in DIPs.
    pub fn measure(&self, text: &str, font: Font) -> f32 {
        let mut metrics = DWRITE_TEXT_METRICS::default();
        unsafe { self.layout(text, font, 10_000.0, Align::Start).GetMetrics(&mut metrics).unwrap() };
        metrics.widthIncludingTrailingWhitespace
    }
}

/// One frame being drawn onto a surface.
pub struct Frame<'a> {
    pub gfx: &'a Gfx,
    pub dc: ID2D1DeviceContext,
    brush: ID2D1SolidColorBrush,
    /// Where the surface's region starts (DIPs).
    base: Matrix3x2,
}

impl<'a> Frame<'a> {
    /// Draws from here on with `(x, y)`, in DIPs from the surface's corner, as the origin.
    pub fn origin(&self, x: f32, y: f32) {
        unsafe { self.dc.SetTransform(&(Matrix3x2::translation(x, y) * self.base)) };
    }

    pub fn brush(&self, color: Color) -> &ID2D1SolidColorBrush {
        unsafe { self.brush.SetColor(&color.d2d()) };
        &self.brush
    }

    /// Draws `text` with its first baseline-box at `(x, y)`, in a box `width` wide.
    pub fn text(&self, text: &str, font: Font, color: Color, x: f32, y: f32, width: f32, align: Align) {
        let layout = self.gfx.layout(text, font, width, align);
        unsafe {
            self.dc.DrawTextLayout(
                windows_numerics::Vector2 { X: x, Y: y },
                &layout,
                self.brush(color),
                D2D1_DRAW_TEXT_OPTIONS_CLIP | D2D1_DRAW_TEXT_OPTIONS_NONE,
            )
        };
    }
}

/// A window's content: a composition surface the size of the window.
pub struct Surface {
    _target: IDCompositionTarget,
    visual: IDCompositionVisual2,
    surface: Option<IDCompositionSurface>,
    size: (u32, u32),
}

impl Surface {
    pub fn new(gfx: &Gfx, hwnd: HWND) -> Result<Self> {
        let target = unsafe { gfx.dcomp.CreateTargetForHwnd(hwnd, true)? };
        let visual: IDCompositionVisual2 = unsafe { gfx.dcomp.CreateVisual()? };
        unsafe { target.SetRoot(&visual)? };
        Ok(Surface { _target: target, visual, surface: None, size: (0, 0) })
    }

    /// Draws one frame, at `scale` physical pixels per DIP, on a surface
    /// `size` physical pixels large.
    pub fn draw(&mut self, gfx: &Gfx, size: (u32, u32), scale: f32, paint: impl FnOnce(&Frame)) -> Result<()> {
        if self.surface.is_none() || self.size != size {
            let surface = unsafe {
                gfx.dcomp.CreateSurface(size.0.max(1), size.1.max(1), DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_ALPHA_MODE_PREMULTIPLIED)?
            };
            unsafe { self.visual.SetContent(&surface)? };
            self.surface = Some(surface);
            self.size = size;
        }
        let surface = self.surface.as_ref().unwrap();
        let mut offset = POINT::default();
        let dc: ID2D1DeviceContext = unsafe { surface.BeginDraw(None, &mut offset)? };
        // The surface may hand out a region of a larger atlas.
        let base = Matrix3x2::translation(offset.x as f32 / scale, offset.y as f32 / scale);
        unsafe {
            dc.SetDpi(96.0 * scale, 96.0 * scale);
            dc.SetTransform(&base);
            // A transparent surface takes no ClearType.
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            dc.Clear(Some(&D2D1_COLOR_F::default()));
        }
        let brush = unsafe { dc.CreateSolidColorBrush(&D2D1_COLOR_F::default(), None)? };
        paint(&Frame { gfx, dc, brush, base });
        unsafe {
            surface.EndDraw()?;
            gfx.dcomp.Commit()?;
        }
        Ok(())
    }

    /// Lets go of the drawing memory while the window is hidden.
    pub fn release(&mut self, gfx: &Gfx) {
        if self.surface.take().is_some() {
            unsafe {
                let _ = self.visual.SetContent(None);
                let _ = gfx.dcomp.Commit();
            }
        }
    }
}

/// Drawing kept in a bitmap of its own and painted again only when what it
/// shows changes, told by a key.
#[derive(Default)]
pub struct Layer {
    bitmap: Option<(u64, ID2D1Bitmap1)>,
}

impl Layer {
    /// Draws the layer, `size` DIPs at `scale`, at the frame's current
    /// origin, painting it again first if `key` is not what it shows.
    pub fn draw(&mut self, frame: &Frame, key: u64, size: (f32, f32), scale: f32, paint: impl FnOnce(&Frame)) -> Result<()> {
        let dc = &frame.dc;
        if self.bitmap.as_ref().is_none_or(|(shown, _)| *shown != key) {
            let pixels = D2D_SIZE_U { width: (size.0 * scale).ceil() as u32, height: (size.1 * scale).ceil() as u32 };
            // The same bitmap serves while the size holds.
            let reusable = self.bitmap.take().map(|(_, bitmap)| bitmap).filter(|bitmap| {
                let have = unsafe { bitmap.GetPixelSize() };
                let mut dpi = (0.0, 0.0);
                unsafe { bitmap.GetDpi(&mut dpi.0, &mut dpi.1) };
                have == pixels && dpi.0 == 96.0 * scale
            });
            let bitmap = match reusable {
                Some(bitmap) => bitmap,
                None => unsafe {
                    let properties = D2D1_BITMAP_PROPERTIES1 {
                        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                        dpiX: 96.0 * scale,
                        dpiY: 96.0 * scale,
                        bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
                        ..Default::default()
                    };
                    dc.CreateBitmap(pixels, None, 0, &properties)?
                },
            };
            unsafe {
                let target = dc.GetTarget()?;
                let mut transform = Matrix3x2::default();
                dc.GetTransform(&mut transform);
                dc.SetTarget(&bitmap);
                dc.SetTransform(&Matrix3x2::identity());
                dc.Clear(Some(&D2D1_COLOR_F::default()));
                paint(frame);
                dc.SetTarget(&target);
                dc.SetTransform(&transform);
            }
            self.bitmap = Some((key, bitmap));
        }
        let bitmap = &self.bitmap.as_ref().unwrap().1;
        unsafe {
            dc.DrawImage(&bitmap.cast::<ID2D1Image>()?, None, None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER)
        };
        Ok(())
    }

    pub fn release(&mut self) {
        self.bitmap = None;
    }
}

pub fn rect(left: f32, top: f32, width: f32, height: f32) -> D2D_RECT_F {
    D2D_RECT_F { left, top, right: left + width, bottom: top + height }
}
