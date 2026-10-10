//! Drawing: Direct2D on a DirectComposition surface, text with DirectWrite.
//! Coordinates are in device-independent pixels (DIPs); the surface is in
//! physical pixels and the context scales between them.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// How long laid-out text no window draws is kept.
const TEXT_KEPT: Duration = Duration::from_secs(2);

use windows::core::{w, Interface, Result, BOOL, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_COMPOSITE_MODE_SOURCE_OVER, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext, ID2D1Factory1, ID2D1Image, ID2D1SolidColorBrush,
    CLSID_D2D1DpiCompensation, CLSID_D2D1Shadow, D2D1_BITMAP_OPTIONS_TARGET, D2D1_DPICOMPENSATION_PROP_INPUT_DPI,
    D2D1_PROPERTY_TYPE_VECTOR2, D2D1_BITMAP_PROPERTIES1, D2D1_INTERPOLATION_MODE_LINEAR,
    D2D1_PROPERTY_TYPE_FLOAT, D2D1_PROPERTY_TYPE_VECTOR4, D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION, D2D1_SHADOW_PROP_COLOR, D2D1_DRAW_TEXT_OPTIONS_CLIP,
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_MULTI_THREADED, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice2, IDCompositionDesktopDevice, IDCompositionSurface, IDCompositionTarget, IDCompositionVisual2,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory6, IDWriteFontCollection, IDWriteFontSetBuilder1, IDWriteInMemoryFontFileLoader,
    IDWriteTextLayout1, DWRITE_CONTAINER_TYPE_WOFF2, DWRITE_LINE_METRICS, DWRITE_TEXT_RANGE, IDWriteTextFormat3, IDWriteTextLayout,
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_AXIS_TAG_WEIGHT, DWRITE_FONT_AXIS_TAG_WIDTH, DWRITE_FONT_AXIS_VALUE,
    DWRITE_FONT_FAMILY_MODEL_TYPOGRAPHIC, DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWRITE_WORD_WRAPPING_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM};
use windows::Win32::Graphics::Dxgi::{IDXGIDevice, IDXGIDevice3};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_OPEN, D2D1_GRADIENT_STOP,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1PathGeometry1, ID2D1RenderTarget, D2D1_ANTIALIAS_MODE_ALIASED, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_ELLIPSE, D2D1_EXTEND_MODE_CLAMP,
    D2D1_GAMMA_2_2, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_BEZIER_SEGMENT, D2D1_FIGURE_END_CLOSED, D2D_SIZE_F};
use windows::Win32::Graphics::Direct2D::{
    ID2D1StrokeStyle, D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_LARGE, D2D1_DASH_STYLE_CUSTOM, D2D1_ARC_SIZE_SMALL, D2D1_CAP_STYLE_ROUND, D2D1_LINE_JOIN_ROUND,
    D2D1_STROKE_STYLE_PROPERTIES1, D2D1_SWEEP_DIRECTION_CLOCKWISE, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
};
use windows_numerics::{Matrix3x2, Vector2};

use super::icons::{self, Icon, Segment};

use super::canvas::{Align, Canvas, Color, Family, Fill, Font, FontKey, Point};

/// The faces shipped inside the program, Latin only (the system supplies
/// Chinese): Archivo, with its weight and width axes, for chart paper, and
/// Inter, with its weight axis, for glass.
const ARCHIVO: &[u8] = include_bytes!("../../fonts/Archivo-latin.woff2");
const INTER: &[u8] = include_bytes!("../../fonts/Inter-latin.woff2");

impl Color {
    /// For Direct2D, which takes colours straight and premultiplies itself.
    pub fn d2d(self) -> D2D1_COLOR_F {
        D2D1_COLOR_F { r: self.r, g: self.g, b: self.b, a: self.a }
    }
}

/// Devices and factories shared by every surface.
pub struct Gfx {
    pub factory: ID2D1Factory1,
    pub(super) device: ID2D1Device,
    dxgi: IDXGIDevice3,
    pub dcomp: IDCompositionDesktopDevice,
    write: IDWriteFactory6,
    /// Holds the shipped faces; registered with the shared factory for as
    /// long as this device lives.
    loader: IDWriteInMemoryFontFileLoader,
    /// The shipped faces: their collections and family names.
    archivo: (IDWriteFontCollection, HSTRING),
    inter: (IDWriteFontCollection, HSTRING),
    icons: PCWSTR,
    /// Each font's format, and when it was last asked for.
    formats: RefCell<HashMap<FontKey, (IDWriteTextFormat3, Instant)>>,
    /// Laid-out text, kept while frames keep drawing it: most of a panel's
    /// words and figures are the same from one frame to the next. With when
    /// each was last drawn.
    layouts: RefCell<HashMap<LayoutKey, (IDWriteTextLayout, Instant)>>,
    /// Strokes with round ends and joins, for rings and icons.
    round: ID2D1StrokeStyle,
    /// Dashes as long as the gaps between them, each twice the line's width.
    dashed: ID2D1StrokeStyle,
    /// Each icon's figures as a geometry, made when first drawn.
    icon_paths: RefCell<HashMap<Icon, ID2D1PathGeometry1>>,
    /// Bitmaps made from nothing but their key (the glass's displacement
    /// maps, by size, radius and scale): kept while frames keep drawing
    /// them, as text is.
    made: RefCell<HashMap<[u32; 4], (ID2D1Bitmap1, Instant)>>,
}

/// How much contrast grayscale text's edges are given by `crisp_text` (the
/// system's default is 1).
const CRISP_CONTRAST: f32 = 2.0;

/// Text, font, width, at the end, and broken into lines.
type LayoutKey = (String, FontKey, u32, bool, bool);

impl Drop for Gfx {
    fn drop(&mut self) {
        let _ = unsafe { self.write.UnregisterFontFileLoader(&self.loader) };
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<Gfx>>> = const { RefCell::new(None) };
}

/// This thread's graphics device, which its windows share; made when first
/// asked for, and again after it was lost.
pub fn current() -> Result<Rc<Gfx>> {
    CURRENT.with(|cell| {
        if let Some(gfx) = cell.borrow().as_ref() {
            return Ok(gfx.clone());
        }
        let gfx = Rc::new(Gfx::new()?);
        *cell.borrow_mut() = Some(gfx.clone());
        Ok(gfx)
    })
}

/// Drawing failed: the device is gone (a driver update, a graphics reset).
/// The next `current` makes a new one, and each window, seeing it, makes its
/// surfaces again.
pub fn lost() {
    if CURRENT.with(|cell| cell.borrow_mut().take()).is_some() {
        crate::journal::note("the graphics device was lost; drawing on a new one");
    }
}

impl Gfx {
    pub fn new() -> Result<Self> {
        // The graphics card; without a working one, Windows' software renderer.
        let create = |kind| {
            let mut d3d: Option<ID3D11Device> = None;
            unsafe {
                D3D11CreateDevice(None, kind, Default::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut d3d), None, None)
            }
            .map(|_| d3d.unwrap())
        };
        let d3d = create(D3D_DRIVER_TYPE_HARDWARE).or_else(|_| create(D3D_DRIVER_TYPE_WARP))?;
        let dxgi: IDXGIDevice = d3d.cast()?;
        let factory: ID2D1Factory1 = unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_MULTI_THREADED, None)? };
        let device: ID2D1Device = unsafe { factory.CreateDevice(&dxgi)? };
        // Made from the Direct2D device, composition surfaces hand out device
        // contexts already aimed at themselves.
        let dcomp: IDCompositionDesktopDevice = unsafe { DCompositionCreateDevice2(&device)? };
        let write: IDWriteFactory6 = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let loader = unsafe { write.CreateInMemoryFontFileLoader()? };
        unsafe { write.RegisterFontFileLoader(&loader)? };
        let faces = (|| {
            let archivo = shipped_face(&write, &loader, ARCHIVO)?;
            let inter = shipped_face(&write, &loader, INTER)?;
            let icons = unsafe {
                let system = write.GetSystemFontCollection(false, DWRITE_FONT_FAMILY_MODEL_TYPOGRAPHIC)?;
                let (mut index, mut fluent) = (0, BOOL(0));
                system.FindFamilyName(w!("Segoe Fluent Icons"), &mut index, &mut fluent)?;
                if fluent.as_bool() { w!("Segoe Fluent Icons") } else { w!("Segoe MDL2 Assets") }
            };
            Ok::<_, windows::core::Error>((archivo, inter, icons))
        })();
        // Failing here, the loader is not left registered with nothing to free it.
        let (archivo, inter, icons) = faces.inspect_err(|_| unsafe {
            let _ = write.UnregisterFontFileLoader(&loader);
        })?;
        let round = unsafe {
            factory.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES1 { startCap: D2D1_CAP_STYLE_ROUND, endCap: D2D1_CAP_STYLE_ROUND, dashCap: D2D1_CAP_STYLE_ROUND, lineJoin: D2D1_LINE_JOIN_ROUND, ..Default::default() },
                None,
            )?
        };
        let dashed = unsafe {
            factory.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES1 { lineJoin: D2D1_LINE_JOIN_ROUND, dashStyle: D2D1_DASH_STYLE_CUSTOM, ..Default::default() },
                Some(&[2.0, 2.0]),
            )?
        };
        Ok(Gfx {
            factory,
            device,
            dxgi: dxgi.cast()?,
            dcomp,
            write,
            loader,
            archivo,
            inter,
            icons,
            formats: RefCell::new(HashMap::new()),
            made: RefCell::new(HashMap::new()),
            layouts: RefCell::new(HashMap::new()),
            round: round.into(),
            dashed: dashed.into(),
            icon_paths: RefCell::new(HashMap::new()),
        })
    }

    fn format(&self, font: Font) -> IDWriteTextFormat3 {
        if let Some((format, used)) = self.formats.borrow_mut().get_mut(&font.key()) {
            *used = Instant::now();
            return format.clone();
        }
        let axes = [
            DWRITE_FONT_AXIS_VALUE { axisTag: DWRITE_FONT_AXIS_TAG_WEIGHT, value: font.weight },
            DWRITE_FONT_AXIS_VALUE { axisTag: DWRITE_FONT_AXIS_TAG_WIDTH, value: font.width },
        ];
        let (name, collection): (PCWSTR, Option<&IDWriteFontCollection>) = match font.family {
            Family::Archivo => (PCWSTR(self.archivo.1.as_ptr()), Some(&self.archivo.0)),
            Family::Inter => (PCWSTR(self.inter.1.as_ptr()), Some(&self.inter.0)),
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
        self.formats.borrow_mut().insert(font.key(), (format.clone(), Instant::now()));
        format
    }

    /// Lays `text` out on one line, `width` DIPs wide at most.
    pub fn layout(&self, text: &str, font: Font, width: f32, align: Align) -> IDWriteTextLayout {
        self.laid_out(text, font, width, align, false)
    }

    /// Lays `text` out in as many lines `width` DIPs wide as it takes,
    /// broken between words (or, in Chinese, characters).
    pub fn wrapped(&self, text: &str, font: Font, width: f32) -> IDWriteTextLayout {
        self.laid_out(text, font, width, Align::Start, true)
    }

    /// How tall `text` is laid out in lines `width` wide.
    pub fn wrapped_height(&self, text: &str, font: Font, width: f32) -> f32 {
        let mut metrics = DWRITE_TEXT_METRICS::default();
        unsafe { self.wrapped(text, font, width).GetMetrics(&mut metrics).unwrap() };
        metrics.height
    }

    fn laid_out(&self, text: &str, font: Font, width: f32, align: Align, wrap: bool) -> IDWriteTextLayout {
        let key = (text.to_string(), font.key(), width.max(0.0).to_bits(), align == Align::End, wrap);
        if let Some((layout, used)) = self.layouts.borrow_mut().get_mut(&key) {
            *used = Instant::now();
            return layout.clone();
        }
        let wide: Vec<u16> = text.encode_utf16().collect();
        let layout = unsafe { self.write.CreateTextLayout(&wide, &self.format(font), width.max(0.0), font.size * 2.0) }
            .expect("text layout");
        let alignment = if align == Align::End { DWRITE_TEXT_ALIGNMENT_TRAILING } else { DWRITE_TEXT_ALIGNMENT_LEADING };
        unsafe { layout.SetTextAlignment(alignment).unwrap() };
        if wrap {
            unsafe {
                layout.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP).unwrap();
                layout.SetTrimming(&DWRITE_TRIMMING::default(), None).unwrap();
            }
        }
        if font.tracking != 0.0 {
            // As CSS letter-spacing: added after every character.
            let range = DWRITE_TEXT_RANGE { startPosition: 0, length: wide.len() as u32 };
            let spaced: IDWriteTextLayout1 = layout.cast().unwrap();
            unsafe { spaced.SetCharacterSpacing(0.0, font.tracking * font.size, 0.0, range).unwrap() };
        }
        self.layouts.borrow_mut().insert(key, (layout.clone(), Instant::now()));
        layout
    }

    /// Where the first line's baseline falls below the top of `font`'s
    /// text, and how far its descent reaches below it (DIPs).
    pub fn baseline(&self, font: Font) -> (f32, f32) {
        let layout = self.layout("0", font, 10_000.0, Align::Start);
        let mut line = [DWRITE_LINE_METRICS::default()];
        let mut count = 0;
        unsafe { layout.GetLineMetrics(Some(&mut line), &mut count).unwrap() };
        (line[0].baseline, line[0].height - line[0].baseline)
    }

    /// Lets go of what Direct2D keeps from earlier drawing, such as the
    /// intermediate images of effects.
    pub fn clear_caches(&self) {
        unsafe { self.device.ClearResources(0) };
    }

    /// Gives back what drawing holds on to while nothing is on screen.
    pub fn trim(&self) {
        self.layouts.borrow_mut().clear();
        self.made.borrow_mut().clear();
        unsafe {
            self.device.ClearResources(0);
            self.dxgi.Trim();
        }
    }

    /// Forgets the text, and the fonts, no window has drawn for a while.
    pub fn sweep(&self) {
        self.layouts.borrow_mut().retain(|_, (_, used)| used.elapsed() < TEXT_KEPT);
        self.formats.borrow_mut().retain(|_, (_, used)| used.elapsed() < TEXT_KEPT);
        self.made.borrow_mut().retain(|_, (_, used)| used.elapsed() < TEXT_KEPT);
    }

    /// The bitmap `key` stands for: the one made for it before, else made
    /// now by `make`. Those drawn least lately are let go of past
    /// `MADE_BUDGET` (a widget sized frame by frame makes one for each size).
    pub fn made(&self, key: [u32; 4], make: impl FnOnce() -> Result<ID2D1Bitmap1>) -> Result<ID2D1Bitmap1> {
        let now = Instant::now();
        if let Some((bitmap, used)) = self.made.borrow_mut().get_mut(&key) {
            *used = now;
            return Ok(bitmap.clone());
        }
        let bitmap = make()?;
        let mut made = self.made.borrow_mut();
        made.insert(key, (bitmap.clone(), now));
        let bytes = |bitmap: &ID2D1Bitmap1| {
            let size = unsafe { bitmap.GetPixelSize() };
            size.width as usize * size.height as usize * 4
        };
        let mut total: usize = made.values().map(|(bitmap, _)| bytes(bitmap)).sum();
        while total > MADE_BUDGET && made.len() > 1 {
            let Some(oldest) = made.iter().filter(|(k, _)| **k != key).min_by_key(|(_, (_, used))| *used).map(|(k, _)| *k) else { break };
            if let Some((bitmap, _)) = made.remove(&oldest) {
                total -= bytes(&bitmap);
            }
        }
        Ok(bitmap)
    }

    /// How far below the top of its line box `text`'s ink starts and ends.
    pub fn ink(&self, text: &str, font: Font) -> (f32, f32) {
        // Laid out in a box twice the font's size tall (see `laid_out`); the
        // overhangs say how far the ink reaches beyond it.
        let overhang = unsafe { self.layout(text, font, 10_000.0, Align::Start).GetOverhangMetrics().unwrap() };
        (-overhang.top, font.size * 2.0 + overhang.bottom)
    }

    /// How wide `text` is in `font`, in DIPs.
    pub fn measure(&self, text: &str, font: Font) -> f32 {
        let mut metrics = DWRITE_TEXT_METRICS::default();
        unsafe { self.layout(text, font, 10_000.0, Align::Start).GetMetrics(&mut metrics).unwrap() };
        metrics.widthIncludingTrailingWhitespace
    }
}

/// A shipped face, unpacked from its WOFF2 container, as a collection of
/// its own, and the family name it goes by.
fn shipped_face(write: &IDWriteFactory6, loader: &IDWriteInMemoryFontFileLoader, packed: &[u8]) -> Result<(IDWriteFontCollection, HSTRING)> {
    unsafe {
        let stream = write.UnpackFontFile(DWRITE_CONTAINER_TYPE_WOFF2, packed.as_ptr().cast(), packed.len() as u32)?;
        let size = stream.GetFileSize()?;
        let (mut start, mut context) = (std::ptr::null_mut(), std::ptr::null_mut());
        stream.ReadFileFragment(&mut start, 0, size, &mut context)?;
        // Without an owner, the loader keeps a copy of the data.
        let file = loader.CreateInMemoryFontFileReference(write, start, size as u32, None);
        stream.ReleaseFileFragment(context);
        let builder = write.CreateFontSetBuilder()?;
        IDWriteFontSetBuilder1::AddFontFile(&builder, &file?)?;
        let collection: IDWriteFontCollection =
            write.CreateFontCollectionFromFontSet(&builder.CreateFontSet()?, DWRITE_FONT_FAMILY_MODEL_TYPOGRAPHIC)?.cast()?;
        let names = collection.GetFontFamily(0)?.GetFamilyNames()?;
        let mut name = vec![0u16; names.GetStringLength(0)? as usize + 1];
        names.GetString(0, &mut name)?;
        name.pop();
        Ok((collection, HSTRING::from_wide(&name)))
    }
}

impl Gfx {
    /// `icon`'s figures as one geometry, on its 24-unit grid; made once.
    fn icon_path(&self, icon: Icon) -> Option<ID2D1PathGeometry1> {
        if let Some(path) = self.icon_paths.borrow().get(&icon) {
            return Some(path.clone());
        }
        let point = |(x, y): (f32, f32)| Vector2 { X: x, Y: y };
        let path = (|| -> Result<ID2D1PathGeometry1> {
            unsafe {
                let path = self.factory.CreatePathGeometry()?;
                let sink = path.Open()?;
                for figure in icon.figures() {
                    sink.BeginFigure(point(figure.start), D2D1_FIGURE_BEGIN_HOLLOW);
                    for segment in &figure.segments {
                        match *segment {
                            Segment::Line(to) => sink.AddLine(point(to)),
                            Segment::Arc { to, radii, rotation, large, clockwise } => sink.AddArc(&D2D1_ARC_SEGMENT {
                                point: point(to),
                                size: D2D_SIZE_F { width: radii.0, height: radii.1 },
                                rotationAngle: rotation,
                                sweepDirection: if clockwise { D2D1_SWEEP_DIRECTION_CLOCKWISE } else { D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE },
                                arcSize: if large { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
                            }),
                            Segment::Cubic(c1, c2, to) => sink.AddBezier(&D2D1_BEZIER_SEGMENT { point1: point(c1), point2: point(c2), point3: point(to) }),
                        }
                    }
                    sink.EndFigure(if figure.closed { D2D1_FIGURE_END_CLOSED } else { D2D1_FIGURE_END_OPEN });
                }
                sink.Close()?;
                Ok(path)
            }
        })()
        .ok()?;
        self.icon_paths.borrow_mut().insert(icon, path.clone());
        Some(path)
    }
}

#[cfg(feature = "studio")]
impl Gfx {
    /// Draws `paint` into a picture `size` pixels large, at `scale` pixels
    /// per DIP, off screen; returns its pixels, rows top to bottom,
    /// premultiplied BGRA. For the studio, which films the panel.
    pub fn draw_offscreen(&self, size: (u32, u32), scale: f32, paint: impl FnOnce(&Frame)) -> Result<Vec<u8>> {
        use windows::Win32::Graphics::Direct2D::{
            D2D1_BITMAP_OPTIONS, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_CPU_READ,
            D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_MAP_OPTIONS_READ,
        };
        let properties = |options: D2D1_BITMAP_OPTIONS| D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0 * scale,
            dpiY: 96.0 * scale,
            bitmapOptions: options,
            ..Default::default()
        };
        let pixels = D2D_SIZE_U { width: size.0, height: size.1 };
        unsafe {
            let dc = self.device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let target = dc.CreateBitmap(pixels, None, 0, &properties(D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW))?;
            dc.SetTarget(&target);
            dc.SetDpi(96.0 * scale, 96.0 * scale);
            dc.BeginDraw();
            dc.SetTransform(&Matrix3x2::identity());
            dc.Clear(Some(&D2D1_COLOR_F::default()));
            let brush = dc.CreateSolidColorBrush(&D2D1_COLOR_F::default(), None)?;
            paint(&Frame { gfx: self, dc: dc.clone(), brush, base: Matrix3x2::identity() });
            dc.EndDraw(None, None)?;
            let read = dc.CreateBitmap(pixels, None, 0, &properties(D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW))?;
            read.CopyFromBitmap(None, &target, None)?;
            let mapped = read.Map(D2D1_MAP_OPTIONS_READ)?;
            let row = size.0 as usize * 4;
            let mut out = Vec::with_capacity(row * size.1 as usize);
            for y in 0..size.1 as usize {
                out.extend_from_slice(std::slice::from_raw_parts(mapped.bits.add(y * mapped.pitch as usize), row));
            }
            read.Unmap()?;
            Ok(out)
        }
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

impl Canvas for Frame<'_> {
    fn text(&self, text: &str, font: Font, color: Color, x: f32, y: f32, width: f32, align: Align) {
        Frame::text(self, text, font, color, x, y, width, align);
    }

    fn text_scaled(&self, text: &str, font: Font, color: Color, x: f32, y: f32, width: f32, align: Align, scale: f32) {
        // Laid out at its own size and drawn larger or smaller about its
        // corner: no font, nor glyphs, made for a size passed through.
        let mut parent = Matrix3x2::default();
        unsafe {
            self.dc.GetTransform(&mut parent);
            let about = Matrix3x2::translation(-x, -y) * Matrix3x2::scale(scale, scale) * Matrix3x2::translation(x, y);
            self.dc.SetTransform(&(about * parent));
        }
        Frame::text(self, text, font, color, x, y, width / scale.max(0.01), align);
        unsafe { self.dc.SetTransform(&parent) };
    }

    fn measure(&self, text: &str, font: Font) -> f32 {
        self.gfx.measure(text, font)
    }

    fn baseline(&self, font: Font) -> (f32, f32) {
        self.gfx.baseline(font)
    }

    fn ink(&self, text: &str, font: Font) -> (f32, f32) {
        self.gfx.ink(text, font)
    }

    fn clip(&self, x: f32, y: f32, width: f32, height: f32) {
        unsafe { self.dc.PushAxisAlignedClip(&rect(x, y, width, height), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
    }

    fn unclip(&self) {
        unsafe { self.dc.PopAxisAlignedClip() };
    }

    fn fill(&self, color: Color, x: f32, y: f32, width: f32, height: f32) {
        unsafe { self.dc.FillRectangle(&rect(x, y, width, height), self.brush(color)) };
    }

    fn fill_rounded(&self, color: Color, x: f32, y: f32, width: f32, height: f32, radius: f32) {
        if radius == 0.0 {
            return Canvas::fill(self, color, x, y, width, height);
        }
        let radius = radius.min(width / 2.0).min(height / 2.0);
        let shape = D2D1_ROUNDED_RECT { rect: rect(x, y, width, height), radiusX: radius, radiusY: radius };
        unsafe { self.dc.FillRoundedRectangle(&shape, self.brush(color)) };
    }

    fn fill_circle(&self, color: Color, centre: Point, radius: f32) {
        let dot = D2D1_ELLIPSE { point: Vector2 { X: centre.x, Y: centre.y }, radiusX: radius, radiusY: radius };
        unsafe { self.dc.FillEllipse(&dot, self.brush(color)) };
    }

    fn stroke(&self, points: &[Point], color: Color, width: f32) {
        if let Some(path) = self.path(points, false) {
            unsafe { self.dc.DrawGeometry(&path, self.brush(color), width, None) };
        }
    }

    fn stroke_dashed(&self, points: &[Point], color: Color, width: f32) {
        if let Some(path) = self.path(points, false) {
            unsafe { self.dc.DrawGeometry(&path, self.brush(color), width, &self.gfx.dashed) };
        }
    }

    fn fill_shape(&self, points: &[Point], fill: Fill) {
        let Some(path) = self.path(points, true) else { return };
        if let Some(brush) = self.paint_with(fill) {
            unsafe { self.dc.FillGeometry(&path, &brush, None) };
        }
    }

    fn arc(&self, centre: Point, radius: f32, from: f32, sweep: f32, color: Color, width: f32) {
        let brush = self.brush(color);
        if sweep >= std::f32::consts::TAU {
            let circle = D2D1_ELLIPSE { point: Vector2 { X: centre.x, Y: centre.y }, radiusX: radius, radiusY: radius };
            unsafe { self.dc.DrawEllipse(&circle, brush, width, None) };
            return;
        }
        if sweep <= 0.0 {
            return;
        }
        let at = |angle: f32| Vector2 { X: centre.x + radius * angle.sin(), Y: centre.y - radius * angle.cos() };
        let made = (|| -> Result<ID2D1PathGeometry1> {
            unsafe {
                let path = self.gfx.factory.CreatePathGeometry()?;
                let sink = path.Open()?;
                sink.BeginFigure(at(from), D2D1_FIGURE_BEGIN_HOLLOW);
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: at(from + sweep),
                    size: D2D_SIZE_F { width: radius, height: radius },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                    arcSize: if sweep > std::f32::consts::PI { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
                });
                sink.EndFigure(D2D1_FIGURE_END_OPEN);
                sink.Close()?;
                Ok(path)
            }
        })();
        if let Ok(path) = made {
            unsafe { self.dc.DrawGeometry(&path, brush, width, &self.gfx.round) };
        }
    }

    fn icon(&self, icon: Icon, centre: Point, size: f32, color: Color) {
        let Some(path) = self.gfx.icon_path(icon) else { return };
        let scale = size / 24.0;
        let mut parent = Matrix3x2::default();
        unsafe {
            self.dc.GetTransform(&mut parent);
            let placed = Matrix3x2::scale(scale, scale) * Matrix3x2::translation(centre.x - size / 2.0, centre.y - size / 2.0) * parent;
            self.dc.SetTransform(&placed);
            self.dc.DrawGeometry(&path, self.brush(color), icons::STROKE, &self.gfx.round);
            self.dc.SetTransform(&parent);
        }
    }

    fn stroke_rounded(&self, fill: Fill, x: f32, y: f32, width: f32, height: f32, radius: f32, line: f32) {
        let radius = radius.max(0.0).min(width / 2.0).min(height / 2.0);
        let shape = D2D1_ROUNDED_RECT { rect: rect(x, y, width, height), radiusX: radius, radiusY: radius };
        if let Some(brush) = self.paint_with(fill) {
            unsafe { self.dc.DrawRoundedRectangle(&shape, &brush, line, None) };
        }
    }

    fn shadow(&self, color: Color, x: f32, y: f32, width: f32, height: f32, radius: f32, blur: f32, drop: f32) {
        use windows::Win32::Graphics::Direct2D::{CLSID_D2D1Composite, D2D1_COMPOSITE_PROP_MODE, D2D1_PROPERTY_TYPE_ENUM};
        use windows::Win32::Graphics::Direct2D::Common::D2D1_COMPOSITE_MODE_SOURCE_OUT;
        let dc = &self.dc;
        // The rectangle as a picture of its own, recorded in the frame's
        // DIPs (drawn back through the frame's transform), where `down`
        // below its place.
        let shape = |down: f32| -> Result<ID2D1Image> {
            unsafe {
                let list = dc.CreateCommandList()?;
                let (target, mut transform) = (dc.GetTarget()?, Matrix3x2::default());
                dc.GetTransform(&mut transform);
                dc.SetTarget(&list);
                dc.SetTransform(&Matrix3x2::identity());
                Canvas::fill_rounded(self, Color::hex(0, 1.0), x, y + down, width, height, radius);
                dc.SetTarget(&target);
                dc.SetTransform(&transform);
                list.Close()?;
                list.cast()
            }
        };
        // Without its pictures (the device going, which the frame's end
        // tells), no shadow this time.
        let _ = (|| -> Result<()> {
            unsafe {
                let cast = dc.CreateEffect(&CLSID_D2D1Shadow)?;
                cast.SetInput(0, &shape(drop)?, true);
                cast.SetValue(D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &blur.to_ne_bytes())?;
                let rgba = [color.r, color.g, color.b, color.a];
                cast.SetValue(D2D1_SHADOW_PROP_COLOR.0 as u32, D2D1_PROPERTY_TYPE_VECTOR4, std::slice::from_raw_parts(rgba.as_ptr().cast(), 16))?;
                // The shadow where the rectangle is not: the rectangle as the
                // destination, the shadow kept outside it.
                let outside = dc.CreateEffect(&CLSID_D2D1Composite)?;
                outside.SetInput(0, &shape(0.0)?, true);
                outside.SetInput(1, &cast.GetOutput()?, true);
                outside.SetValue(D2D1_COMPOSITE_PROP_MODE.0 as u32, D2D1_PROPERTY_TYPE_ENUM, &(D2D1_COMPOSITE_MODE_SOURCE_OUT.0 as u32).to_ne_bytes())?;
                dc.DrawImage(&outside.GetOutput()?, None, None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
            }
            Ok(())
        })();
    }
}

impl<'a> Frame<'a> {
    /// A frame on a composition surface's context just handed out for
    /// drawing, at `offset` (physical pixels) in it: cleared, at `scale`
    /// physical pixels per DIP.
    pub(super) fn begun(gfx: &'a Gfx, dc: ID2D1DeviceContext, offset: POINT, scale: f32) -> Result<Self> {
        // The surface may hand out a region of a larger atlas.
        let base = Matrix3x2::translation(offset.x as f32 / scale, offset.y as f32 / scale);
        unsafe {
            dc.SetDpi(96.0 * scale, 96.0 * scale);
            dc.SetTransform(&base);
            // A transparent surface takes no ClearType.
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            // The system's text rendering, unless the frame asks for crisp
            // text: the context may be one another surface had it set on.
            dc.SetTextRenderingParams(None);
            dc.Clear(Some(&D2D1_COLOR_F::default()));
        }
        let brush = unsafe { dc.CreateSolidColorBrush(&D2D1_COLOR_F::default(), None)? };
        Ok(Frame { gfx, dc, brush, base })
    }

    /// A brush for `fill`; none if Direct2D cannot make it.
    fn paint_with(&self, fill: Fill) -> Option<windows::Win32::Graphics::Direct2D::ID2D1Brush> {
        match fill {
            Fill::Solid(color) => self.brush(color).cast().ok(),
            Fill::Down { top, from, bottom, to } => self.gradient(Vector2 { X: 0.0, Y: top }, from, Vector2 { X: 0.0, Y: bottom }, to),
        }
    }

    /// A brush fading from `from` at `start` to `to` at `end`.
    fn gradient(&self, start: Vector2, from: Color, end: Vector2, to: Color) -> Option<windows::Win32::Graphics::Direct2D::ID2D1Brush> {
        let stops = [D2D1_GRADIENT_STOP { position: 0.0, color: from.d2d() }, D2D1_GRADIENT_STOP { position: 1.0, color: to.d2d() }];
        unsafe {
            let collection = ID2D1RenderTarget::CreateGradientStopCollection(&self.dc, &stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP).ok()?;
            let line = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: start, endPoint: end };
            self.dc.CreateLinearGradientBrush(&line, None, &collection).ok()?.cast().ok()
        }
    }

    /// The path through `points`, closed as a shape or open as a line;
    /// `None` without points, or if Direct2D cannot make it.
    fn path(&self, points: &[Point], closed: bool) -> Option<ID2D1PathGeometry1> {
        let (first, rest) = points.split_first()?;
        let rest: Vec<Vector2> = rest.iter().map(|p| Vector2 { X: p.x, Y: p.y }).collect();
        unsafe {
            let path = self.gfx.factory.CreatePathGeometry().ok()?;
            let sink = path.Open().ok()?;
            sink.BeginFigure(Vector2 { X: first.x, Y: first.y }, if closed { D2D1_FIGURE_BEGIN_FILLED } else { D2D1_FIGURE_BEGIN_HOLLOW });
            // A lone point (a reading between two gaps) has no lines: an empty
            // slice's pointer is a dangling placeholder, which Direct2D reads
            // all the same, and faults on.
            if !rest.is_empty() {
                sink.AddLines(&rest);
            }
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            sink.Close().ok()?;
            Some(path)
        }
    }

    /// Draws text from here on as sharply as grayscale antialiasing allows
    /// (a surface with nothing opaque beneath its text has no ClearType):
    /// stems fitted to the pixel grid, edges given more contrast.
    pub fn crisp_text(&self) {
        let params = unsafe {
            use windows::Win32::Graphics::DirectWrite::{DWRITE_GRID_FIT_MODE_ENABLED, DWRITE_PIXEL_GEOMETRY_FLAT, DWRITE_RENDERING_MODE1_GDI_CLASSIC};
            // GDI's own rendering: stems on whole pixels, smoothed across only,
            // as the system draws small text; sharpest at 100% scale.
            self.gfx.write.CreateCustomRenderingParams(1.8, 0.5, CRISP_CONTRAST, 0.0, DWRITE_PIXEL_GEOMETRY_FLAT, DWRITE_RENDERING_MODE1_GDI_CLASSIC, DWRITE_GRID_FIT_MODE_ENABLED)
        };
        if let Ok(params) = params {
            unsafe { self.dc.SetTextRenderingParams(&params) };
        }
    }

    /// The picture `bitmap`, placed by `placed` from the surface's corner,
    /// frosted as the glass frosts what is behind it (see `glass`), inside
    /// the rounded rectangle `pane` (left, top, width, height, radius)
    /// placed by `at`, as much of it as `frost` (1, all; 0, none: the
    /// picture as it is). For the glass where no compositor draws it.
    pub fn frosted(&self, bitmap: &ID2D1Bitmap1, placed: Matrix3x2, pane: (f32, f32, f32, f32, f32), at: Matrix3x2, frost: f32) {
        use windows::Win32::Graphics::Direct2D::{CLSID_D2D1GaussianBlur, CLSID_D2D1Saturation, D2D1_GAUSSIANBLUR_PROP_BORDER_MODE, D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION, D2D1_LAYER_PARAMETERS1, D2D1_PROPERTY_TYPE_ENUM, D2D1_SATURATION_PROP_SATURATION};
        let (x, y, width, height, radius) = pane;
        let dc = &self.dc;
        let _ = (|| -> Result<()> {
            unsafe {
                use windows::Win32::Graphics::Direct2D::Common::D2D1_BORDER_MODE_HARD;
                let shape = D2D1_ROUNDED_RECT { rect: rect(x, y, width, height), radiusX: radius, radiusY: radius };
                let mask = self.gfx.factory.CreateRoundedRectangleGeometry(&shape)?;
                let softened = dc.CreateEffect(&CLSID_D2D1Saturation)?;
                softened.SetInput(0, &effect_input(dc, bitmap)?, true);
                softened.SetValue(D2D1_SATURATION_PROP_SATURATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &super::glass::SATURATION.to_ne_bytes())?;
                let blurred = dc.CreateEffect(&CLSID_D2D1GaussianBlur)?;
                blurred.SetInput(0, &softened.GetOutput()?, true);
                blurred.SetValue(D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &super::glass::blur().to_ne_bytes())?;
                blurred.SetValue(D2D1_GAUSSIANBLUR_PROP_BORDER_MODE.0 as u32, D2D1_PROPERTY_TYPE_ENUM, &(D2D1_BORDER_MODE_HARD.0 as u32).to_ne_bytes())?;
                let mut before = Matrix3x2::default();
                dc.GetTransform(&mut before);
                let layer = D2D1_LAYER_PARAMETERS1 {
                    contentBounds: rect(-1e6, -1e6, 2e6, 2e6),
                    geometricMask: std::mem::ManuallyDrop::new(Some(mask.cast()?)),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: at * self.base,
                    opacity: frost,
                    ..Default::default()
                };
                dc.SetTransform(&Matrix3x2::identity());
                dc.PushLayer(&layer, None);
                std::mem::ManuallyDrop::into_inner(layer.geometricMask);
                dc.SetTransform(&(placed * self.base));
                dc.DrawImage(&blurred.GetOutput()?, None, None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
                dc.PopLayer();
                dc.SetTransform(&before);
            }
            Ok(())
        })();
    }

    /// Draws from here on with `(x, y)`, in DIPs from the surface's corner, as the origin.
    pub fn origin(&self, x: f32, y: f32) {
        self.place(Matrix3x2::translation(x, y));
    }

    /// Draws from here on through `transform`, from the surface's corner.
    pub fn place(&self, transform: Matrix3x2) {
        unsafe { self.dc.SetTransform(&(transform * self.base)) };
    }

    pub fn brush(&self, color: Color) -> &ID2D1SolidColorBrush {
        unsafe { self.brush.SetColor(&color.d2d()) };
        &self.brush
    }

    /// Draws `text` in lines `width` wide, the first line's box at `(x, y)`.
    pub fn text_wrapped(&self, text: &str, font: Font, color: Color, x: f32, y: f32, width: f32) {
        let layout = self.gfx.wrapped(text, font, width);
        unsafe { self.dc.DrawTextLayout(windows_numerics::Vector2 { X: x, Y: y }, &layout, self.brush(color), D2D1_DRAW_TEXT_OPTIONS_NONE) };
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

/// How much the bitmaps made from their keys alone may hold (bytes; see
/// `Gfx::made`).
const MADE_BUDGET: usize = 32 << 20;

/// The step a surface's size is made in (physical px; see `Surface::draw`).
const SURFACE_STEP: u32 = 128;

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
        // Made a little larger than asked, in steps, and kept while it holds
        // what is asked without being much larger: a window sized frame by
        // frame (a widget settling, sticking) draws on one surface, not a
        // new one each frame. What lies past the window is not shown.
        let step = |v: u32| v.max(1).div_ceil(SURFACE_STEP) * SURFACE_STEP;
        let holds = self.size.0 >= size.0 && self.size.1 >= size.1 && self.size.0 <= step(size.0) + 2 * SURFACE_STEP && self.size.1 <= step(size.1) + 2 * SURFACE_STEP;
        if self.surface.is_none() || !holds {
            let made = (step(size.0), step(size.1));
            let surface = unsafe { gfx.dcomp.CreateSurface(made.0, made.1, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_ALPHA_MODE_PREMULTIPLIED)? };
            unsafe { self.visual.SetContent(&surface)? };
            self.surface = Some(surface);
            self.size = made;
        }
        let surface = self.surface.as_ref().unwrap();
        let mut offset = POINT::default();
        let dc: ID2D1DeviceContext = unsafe { surface.BeginDraw(None, &mut offset)? };
        paint(&Frame::begun(gfx, dc, offset, scale)?);
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
    /// Draws the layer: what `paint` draws in its own coordinates, inside
    /// `area` of them, placed by `local` within the frame's current
    /// transform, and kept at `scale` physical pixels per unit. It is painted
    /// again first only if `key` is not what it shows. A `halo` colour rings
    /// what is drawn with a faint one-unit glow, as CSS's `text-shadow: 0 0 1px`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        frame: &Frame,
        key: u64,
        local: Matrix3x2,
        area: (f32, f32, f32, f32),
        scale: f32,
        halo: Option<Color>,
        paint: impl FnOnce(&Frame) -> Result<()>,
    ) -> Result<()> {
        let (left, top, width, height) = area;
        let size = (width, height);
        let dc = &frame.dc;
        if self.bitmap.as_ref().is_none_or(|(shown, _)| *shown != key) {
            let pixels = D2D_SIZE_U { width: (size.0 * scale).ceil() as u32, height: (size.1 * scale).ceil() as u32 };
            // The same bitmap serves while it holds the size without being
            // much larger: made in steps (as surfaces are), a layer sized
            // frame by frame keeps one bitmap, not one for each size.
            let step = |v: u32| v.max(1).div_ceil(SURFACE_STEP) * SURFACE_STEP;
            let reusable = self.bitmap.take().map(|(_, bitmap)| bitmap).filter(|bitmap| {
                let have = unsafe { bitmap.GetPixelSize() };
                let mut dpi = (0.0, 0.0);
                unsafe { bitmap.GetDpi(&mut dpi.0, &mut dpi.1) };
                let holds = have.width >= pixels.width && have.height >= pixels.height;
                let near = have.width <= step(pixels.width) + 2 * SURFACE_STEP && have.height <= step(pixels.height) + 2 * SURFACE_STEP;
                holds && near && dpi.0 == 96.0 * scale
            });
            let pixels = D2D_SIZE_U { width: step(pixels.width), height: step(pixels.height) };
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
                // A new target brings its own DPI to the context; the
                // frame's is put back with the frame's target.
                let target = dc.GetTarget()?;
                let mut transform = Matrix3x2::default();
                dc.GetTransform(&mut transform);
                let mut dpi = (0.0, 0.0);
                dc.GetDpi(&mut dpi.0, &mut dpi.1);
                dc.SetTarget(&bitmap);
                dc.SetDpi(96.0 * scale, 96.0 * scale);
                dc.SetTransform(&Matrix3x2::translation(-left, -top));
                dc.Clear(Some(&D2D1_COLOR_F::default()));
                // Its bitmap may be larger than asked (see above): what is
                // drawn stays inside what was.
                dc.PushAxisAlignedClip(&rect(left, top, width, height), D2D1_ANTIALIAS_MODE_ALIASED);
                let painted = paint(frame);
                dc.PopAxisAlignedClip();
                dc.SetTarget(&target);
                dc.SetDpi(dpi.0, dpi.1);
                dc.SetTransform(&transform);
                painted?;
            }
            self.bitmap = Some((key, bitmap));
        }
        let image: ID2D1Image = self.bitmap.as_ref().unwrap().1.cast()?;
        let at = windows_numerics::Vector2 { X: left, Y: top };
        let mut parent = Matrix3x2::default();
        unsafe { dc.GetTransform(&mut parent) };
        unsafe {
            dc.SetTransform(&(local * parent));
            if let Some(color) = halo {
                let glow = dc.CreateEffect(&CLSID_D2D1Shadow)?;
                glow.SetInput(0, &effect_input(dc, &self.bitmap.as_ref().unwrap().1)?, true);
                glow.SetValue(D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &0.5f32.to_ne_bytes())?;
                let rgba = [color.r, color.g, color.b, color.a];
                glow.SetValue(D2D1_SHADOW_PROP_COLOR.0 as u32, D2D1_PROPERTY_TYPE_VECTOR4, std::slice::from_raw_parts(rgba.as_ptr().cast(), 16))?;
                dc.DrawImage(&glow.GetOutput()?, Some(&at), None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
            }
            dc.DrawImage(&image, Some(&at), None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
            dc.SetTransform(&parent);
        }
        Ok(())
    }

    pub fn release(&mut self) {
        self.bitmap = None;
    }
}

/// A bitmap as an effect's input, at its own DPI: effects read their input
/// bitmaps' pixels at the context's DPI otherwise.
pub fn effect_input(dc: &ID2D1DeviceContext, bitmap: &ID2D1Bitmap1) -> Result<ID2D1Image> {
    unsafe {
        let mut dpi = (0.0f32, 0.0f32);
        bitmap.GetDpi(&mut dpi.0, &mut dpi.1);
        let compensation = dc.CreateEffect(&CLSID_D2D1DpiCompensation)?;
        compensation.SetInput(0, &bitmap.cast::<ID2D1Image>()?, true);
        let input_dpi = [dpi.0, dpi.1];
        compensation.SetValue(
            D2D1_DPICOMPENSATION_PROP_INPUT_DPI.0 as u32,
            D2D1_PROPERTY_TYPE_VECTOR2,
            std::slice::from_raw_parts(input_dpi.as_ptr().cast(), 8),
        )?;
        compensation.GetOutput()
    }
}

pub fn rect(left: f32, top: f32, width: f32, height: f32) -> D2D_RECT_F {
    D2D_RECT_F { left, top, right: left + width, bottom: top + height }
}
