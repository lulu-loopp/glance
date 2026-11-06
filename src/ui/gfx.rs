//! Drawing: Direct2D on a DirectComposition surface, text with DirectWrite.
//! Coordinates are in device-independent pixels (DIPs); the surface is in
//! physical pixels and the context scales between them.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// How long laid-out text no window draws is kept.
const TEXT_KEPT: Duration = Duration::from_secs(2);
/// How long a picture not drawn again is kept on the card (a track's cover: drawn each second while it plays).
const PICTURE_KEPT: Duration = Duration::from_secs(10);

use windows::core::{w, Interface, Result, HSTRING, PCWSTR};
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
    DWriteCreateFactory, IDWriteFactory, IDWriteFactory6, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_ITALIC, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_BOLD, IDWriteFontCollection, IDWriteFontSetBuilder1, IDWriteInMemoryFontFileLoader,
    IDWriteTextLayout1, DWRITE_CONTAINER_TYPE_WOFF2, DWRITE_LINE_METRICS, DWRITE_TEXT_RANGE, IDWriteTextFormat3, IDWriteTextLayout,
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_AXIS_TAG_WEIGHT, DWRITE_FONT_AXIS_TAG_WIDTH, DWRITE_FONT_AXIS_VALUE,
    DWRITE_FONT_FAMILY_MODEL_TYPOGRAPHIC, DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWRITE_WORD_WRAPPING_WRAP, DWRITE_HIT_TEST_METRICS, IDWriteInlineObject, IDWriteInlineObject_Impl,
    DWRITE_LINE_SPACING_METHOD_UNIFORM, DWRITE_LINE_SPACING_METHOD_DEFAULT, IDWriteTextFormat,
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

use super::canvas::{Align, Canvas, Color, Family, Fill, Font, FontKey, Inks, Point};

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
    /// Whether its device's immediate context may be used from several threads (see `new`).
    pub guarded: bool,
    /// Drawn with Windows' software renderer, the graphics card having failed to make a device as it was made: when
    /// the card is to be asked again (None: not, there being no card that draws; see `current`). None on the card.
    software: Option<Cell<Option<Instant>>>,
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
    /// The serif face, once it is had and first asked for (see `serif`).
    serif: std::cell::OnceCell<Option<(IDWriteFontCollection, HSTRING)>>,
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
    /// The pictures drawn (see `Canvas::picture`), as bitmaps on the card, by key, and when each was last drawn.
    pictures: RefCell<HashMap<u64, (ID2D1Bitmap1, Instant)>>,
    /// Bitmaps made from nothing but their key (the glass's displacement maps, by size, radius and scale; the pieces
    /// of shadows): kept for as long as the device, while they fit in `MADE_BUDGET`, with when each was last drawn.
    /// What a window's layers are painted from, the same each time they are painted for the same shape: let go of as
    /// it hid, the panel made them all again as it next opened (tens of them, a glass panel's first frame three
    /// times as long).
    made: RefCell<HashMap<[u32; 4], (ID2D1Bitmap1, Instant)>>,
    /// Bitmaps drawn on that were let go of (see `made`), to be drawn on again for what is made next (see
    /// `drawn_on`), as much as `MADE_BUDGET` of them.
    spare: RefCell<Vec<ID2D1Bitmap1>>,
    /// When a window last drew with it, and whether what drawing holds has been given back since (see
    /// `trim_when_idle`).
    last_drawn: Cell<Option<Instant>>,
    trimmed: Cell<bool>,
    /// When a layer was last painted anew, and whether what painting it took has been given back since (see
    /// `let_go_once_settled`).
    last_painted: Cell<Option<Instant>>,
    settled: Cell<bool>,
    /// How many windows show what they draw with it now (see `life::Showing`).
    showing: Cell<usize>,
    /// The thread's timer that gives back what drawing holds once it is due (see `tend`), while one runs.
    tending: Cell<usize>,
    /// The two colours each gradient fades between, as made for them (a resource of the device, any of its
    /// contexts draws with), and when each was last used.
    stops: RefCell<HashMap<[u32; 8], (windows::Win32::Graphics::Direct2D::ID2D1GradientStopCollection, Instant)>>,
    /// What each frame is drawn on first (see `Frame::apart`), as large as the largest frame yet.
    scratch: RefCell<Option<(ID2D1Bitmap1, (u32, u32))>>,
    /// The context that draws on it: Glance's own, made once (the compositor's, its target changed to the bitmap and
    /// back, failed now and then, and the window showed nothing for a frame, or another window's drawing).
    scratch_context: RefCell<Option<ID2D1DeviceContext>>,
}

/// How much contrast grayscale text's edges are given by `crisp_text` (the
/// system's default is 1).
const CRISP_CONTRAST: f32 = 2.0;

/// Text, font, width, at the end, and broken into lines.
type LayoutKey = (String, FontKey, u32, bool, bool);

/// How many devices are made and not let go of, for a report (see `alive`): one a thread that draws, as it draws.
static ALIVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// How many graphics devices Glance holds now.
pub fn alive() -> usize {
    ALIVE.load(std::sync::atomic::Ordering::Relaxed)
}

impl Drop for Gfx {
    fn drop(&mut self) {
        ALIVE.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        // What its windows' surfaces were let go of in the compositor too: till a commit tells it, the composition
        // device holds what they showed, and through that the whole device, its driver's threads and its memory
        // (a settings window opened and closed left some 100 threads and 70 MB each time).
        unsafe {
            let _ = self.dcomp.Commit();
            if let Ok(device) = self.dcomp.cast::<windows::Win32::Graphics::DirectComposition::IDCompositionDevice>() {
                let _ = device.WaitForCommitCompletion();
            }
        }
        if self.tending.get() != 0 {
            let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::KillTimer(None, self.tending.get()) };
        }
        let _ = unsafe { self.write.UnregisterFontFileLoader(&self.loader) };
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<Gfx>>> = const { RefCell::new(None) };
}

/// What the device last made (on any thread) draws on: for a report (see `drawn_on`).
static DRAWN_ON: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// What Glance draws on, as its last device made does: the adapter, its driver's version, and whether in software
/// (none made yet).
pub fn drawn_on() -> Option<String> {
    DRAWN_ON.lock().unwrap().clone()
}

/// The adapter `dxgi` is a device of, as a report says it: its name and its driver's version (the user-mode
/// driver's, as DXGI gives it: four parts of 16 bits).
fn adapter(dxgi: &IDXGIDevice, software: bool) -> String {
    let Ok(adapter) = (unsafe { dxgi.GetAdapter() }) else { return "an adapter not asked".into() };
    let name = unsafe { adapter.GetDesc() }.map_or_else(|_| "an adapter not named".to_string(), |desc| String::from_utf16_lossy(desc.Description.split(|unit| *unit == 0).next().unwrap_or_default()));
    let driver = unsafe { adapter.CheckInterfaceSupport(&IDXGIDevice::IID) }.map_or_else(|_| "not said".to_string(), |version| {
        let version = version as u64;
        format!("{}.{}.{}.{}", version >> 48, (version >> 32) & 0xFFFF, (version >> 16) & 0xFFFF, version & 0xFFFF)
    });
    format!("{name} (driver {driver}){}", if software { ", in software" } else { "" })
}

/// How often a device drawn in software for a card's moment's failure asks the card again.
const CARD_ASKED_AGAIN: Duration = Duration::from_secs(2);

/// This thread's graphics device, which its windows share; made when first asked for, and again after it was lost.
/// One drawn in software for a card's moment's failure (see `software_instead`) gives way to one on the card as soon
/// as the card makes one: each window, seeing it, makes its surfaces again on it, as after a loss.
pub fn current() -> Result<Rc<Gfx>> {
    CURRENT.with(|cell| {
        let held = cell.borrow().clone();
        if let Some(gfx) = held {
            let asked = gfx.software.as_ref().and_then(Cell::get);
            if asked.is_none_or(|at| Instant::now() < at) {
                return Ok(gfx);
            }
            match Gfx::new() {
                Ok(made) if made.software.is_none() => {
                    crate::journal::note("drawing on the graphics card again, not in software");
                    let made = Rc::new(made);
                    *cell.borrow_mut() = Some(made.clone());
                    return Ok(made);
                }
                // (Not yet: asked again later.)
                _ => {
                    if let Some(software) = &gfx.software {
                        software.set(Some(Instant::now() + CARD_ASKED_AGAIN));
                    }
                    return Ok(gfx);
                }
            }
        }
        let gfx = Rc::new(Gfx::new()?);
        *cell.borrow_mut() = Some(gfx.clone());
        Ok(gfx)
    })
}

/// This thread's graphics device (see `current`), let go of as what is given is: before the thread ends, not as its
/// own storage is let go of after (on Windows, under the loader's lock, where the device let go of waits on threads
/// of its own that wait on that lock, and the thread never ends). For a thread of its own that draws a while and ends
/// (a test's; one that asks something, see `station::run`).
pub fn held() -> Result<Held> {
    current().map(Held)
}

/// See `held`.
pub struct Held(Rc<Gfx>);

impl std::ops::Deref for Held {
    type Target = Rc<Gfx>;
    fn deref(&self) -> &Rc<Gfx> {
        &self.0
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        CURRENT.with(|cell| cell.borrow_mut().take());
    }
}

thread_local! {
    /// When drawing that failed with the device still there was last noted, for each window that failed so.
    static FAILED_NOTED: RefCell<HashMap<&'static str, Instant>> = RefCell::new(HashMap::new());
}

/// How often a window failing to draw with the device still there is noted.
const FAILED_NOTED_EVERY: Duration = Duration::from_secs(60);

/// This thread's device let go of now, by a thread that drew and is to end (once what it drew with is let go of: its
/// windows' surfaces). Left to the thread's end, it would be let go of as the thread's own are, under the loader's
/// lock, where the driver's threads it waits on cannot end: the thread hung there, and the device stayed with all it
/// held.
pub fn let_go() {
    drop(CURRENT.with(|cell| cell.borrow_mut().take()));
}
/// Drawing failed for `what` (a window's name), with `error` where it is had. Only where the device is gone (a
/// driver update, a graphics reset: removed, as it says; or the compositor's device no longer valid) is it let go
/// of: the next `current` makes a new one, and each window, seeing it, makes its surfaces again. Drawing that failed
/// on a device still there (short of memory for a moment, a window's sized) is drawn again on it as the window next
/// draws: the frame not shown meanwhile (see `Frame::apart`), and the window left as it showed, not emptied as its
/// surfaces are made anew. Either is noted, the reason a device was removed with it. Whether the device is gone (a
/// window that keeps its surfaces till then makes them anew).
pub fn failed(what: &'static str, error: Option<&windows::core::Error>) -> bool {
    let Some(gfx) = CURRENT.with(|cell| cell.borrow().clone()) else { return true };
    let kind = if gfx.software.is_some() { "in software" } else { "on the graphics card" };
    let said = error.map_or_else(String::new, |error| format!(" ({error}, {:#010X})", error.code().0));
    let removed = gfx.d3d().ok().and_then(|d3d| unsafe { d3d.GetDeviceRemovedReason() }.err());
    let composing = gfx
        .dcomp
        .cast::<windows::Win32::Graphics::DirectComposition::IDCompositionDevice>()
        .ok()
        .is_none_or(|device| unsafe { device.CheckDeviceState() }.is_ok_and(|valid| valid.as_bool()));
    if removed.is_none() && composing {
        let now = Instant::now();
        let due = FAILED_NOTED.with(|noted| {
            let mut noted = noted.borrow_mut();
            let due = noted.get(what).is_none_or(|at| now.duration_since(*at) >= FAILED_NOTED_EVERY);
            if due {
                noted.insert(what, now);
            }
            due
        });
        if due {
            crate::journal::note(format!("{what}: a frame not drawn{said}, the graphics device ({kind}) still there; drawn again on it"));
        }
        return false;
    }
    CURRENT.with(|cell| cell.borrow_mut().take());
    crate::journal::note(match removed {
        Some(why) => format!("{what}: the graphics device ({kind}) was lost{said}: {why} ({:#010X}); drawing on a new one", why.code().0),
        None => format!("{what}: the compositor's device is no longer valid{said}; drawing on a new one"),
    });
    true
}

/// Whether hardware failing to make a device with `error` leaves Windows' software renderer to draw on: only where
/// no graphics card can (none there, or none that does Direct3D 11). Any other failure is the card's for the moment
/// (being reset, out of memory): the device is asked for again shortly (see `current`), on the card; drawn in
/// software meanwhile, everything would be drawn slowly on the processor till Glance started again.
fn software_instead(error: windows::core::HRESULT) -> bool {
    error == windows::Win32::Graphics::Dxgi::DXGI_ERROR_UNSUPPORTED
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
        // A card that failed for a moment (as it is reset) is drawn on in software meanwhile, and asked again (see
        // `current`): its windows are not left undrawn, nor drawn slowly for good.
        let made = Instant::now();
        let (d3d, software) = match create(D3D_DRIVER_TYPE_HARDWARE) {
            Ok(d3d) => (d3d, None),
            Err(error) => {
                let again = (!software_instead(error.code())).then(|| Instant::now() + CARD_ASKED_AGAIN);
                crate::journal::note(format!("the graphics card made no device ({error}); drawing in software{}", if again.is_some() { " till it does" } else { "" }));
                (create(D3D_DRIVER_TYPE_WARP)?, Some(Cell::new(again)))
            }
        };
        let took = made.elapsed();
        if took > Duration::from_millis(500) {
            crate::journal::note(format!("a graphics device took {} ms to make", took.as_millis()));
        }
        // Its immediate context guarded (the context, not the device, answers for that): the frames of what is
        // behind are read back on it from other threads (see `scene`), which they are not unless it is.
        let guarded = unsafe { d3d.GetImmediateContext() }
            .and_then(|context| context.cast::<windows::Win32::Graphics::Direct3D11::ID3D11Multithread>())
            .is_ok_and(|guard| unsafe {
                let _ = guard.SetMultithreadProtected(true);
                guard.GetMultithreadProtected().as_bool()
            });
        let dxgi: IDXGIDevice = d3d.cast()?;
        *DRAWN_ON.lock().unwrap() = Some(adapter(&dxgi, software.is_some()));
        let factory: ID2D1Factory1 = unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_MULTI_THREADED, None)? };
        let device: ID2D1Device = unsafe { factory.CreateDevice(&dxgi)? };
        // What Direct2D keeps of the textures its effects draw through (the frost's blur, shadows), past which it
        // lets go of those not in use: each size a window is drawn at has its own, and a window sized pixel by pixel
        // would otherwise keep one of each.
        unsafe { device.SetMaximumTextureMemory(EFFECTS_KEPT) };
        // Made from the Direct2D device, composition surfaces hand out device
        // contexts already aimed at themselves.
        let dcomp: IDCompositionDesktopDevice = unsafe { DCompositionCreateDevice2(&device)? };
        let write: IDWriteFactory6 = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let loader = unsafe { write.CreateInMemoryFontFileLoader()? };
        unsafe { write.RegisterFontFileLoader(&loader)? };
        let faces = (|| {
            let archivo = shipped_face(&write, &loader, ARCHIVO)?;
            let inter = shipped_face(&write, &loader, INTER)?;
            Ok::<_, windows::core::Error>((archivo, inter))
        })();
        // Failing here, the loader is not left registered with nothing to free it.
        let (archivo, inter) = faces.inspect_err(|_| unsafe {
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
        ALIVE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(Gfx {
            software,
            scratch: RefCell::new(None),
            scratch_context: RefCell::new(None),
            guarded,
            factory,
            device,
            dxgi: dxgi.cast()?,
            dcomp,
            write,
            loader,
            archivo,
            inter,
            serif: std::cell::OnceCell::new(),
            formats: RefCell::new(HashMap::new()),
            made: RefCell::new(HashMap::new()),
            spare: RefCell::new(Vec::new()),
            last_drawn: Cell::new(None),
            trimmed: Cell::new(true),
            last_painted: Cell::new(None),
            settled: Cell::new(true),
            showing: Cell::new(0),
            tending: Cell::new(0),
            stops: RefCell::new(HashMap::new()),
            layouts: RefCell::new(HashMap::new()),
            round: round.into(),
            dashed: dashed.into(),
            icon_paths: RefCell::new(HashMap::new()),
            pictures: RefCell::new(HashMap::new()),
        })
    }

    /// The Direct3D device it draws on.
    pub fn d3d(&self) -> Result<ID3D11Device> {
        self.dxgi.cast()
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
        let (name, collection) = self.face(font.family);
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
            // A line as tall as the face's own, its baseline where the face's own is, whatever face draws what is in it:
            // else a line in Chinese (drawn in Windows' Chinese face, taller) sat lower than one in the face's own
            // letters, set at the face's baseline (see `baseline`), and was not in the middle of what it was put in.
            // (Lines broken between words are as tall as what is in them: see `laid_out`.)
            let own = self.write.CreateTextLayout(&[u16::from(b'0')], &format, 10_000.0, font.size * 2.0).expect("text layout");
            let mut line = [DWRITE_LINE_METRICS::default()];
            let mut count = 0;
            own.GetLineMetrics(Some(&mut line), &mut count).unwrap();
            let plain: IDWriteTextFormat = format.cast().unwrap();
            plain.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, line[0].height, line[0].baseline).unwrap();
        }
        self.formats.borrow_mut().insert(font.key(), (format.clone(), Instant::now()));
        format
    }

    /// Whether the system has the face named `name` (asked once: faces are not put in while Glance runs, as far as it
    /// minds).
    fn has_face(&self, name: &str) -> bool {
        static HAS: std::sync::Mutex<Vec<(String, bool)>> = std::sync::Mutex::new(Vec::new());
        if let Some((_, has)) = HAS.lock().unwrap().iter().find(|(named, _)| named == name) {
            return *has;
        }
        let mut system: Option<IDWriteFontCollection> = None;
        let has = unsafe { self.write.cast::<windows::Win32::Graphics::DirectWrite::IDWriteFactory>().and_then(|factory| factory.GetSystemFontCollection(&mut system, false)) }.is_ok()
            && system.is_some_and(|system| {
                let (mut at, mut found) = (0u32, windows::core::BOOL::from(false));
                unsafe { system.FindFamilyName(&HSTRING::from(name), &mut at, &mut found) }.is_ok() && found.as_bool()
            });
        HAS.lock().unwrap().push((name.to_string(), has));
        has
    }

    /// `family`'s name, and the collection it is in (none: the system's).
    fn face(&self, family: Family) -> (PCWSTR, Option<&IDWriteFontCollection>) {
        match family {
            Family::Archivo => (PCWSTR(self.archivo.1.as_ptr()), Some(&self.archivo.0)),
            Family::Inter => (PCWSTR(self.inter.1.as_ptr()), Some(&self.inter.0)),
            Family::Segoe => (w!("Segoe UI Variable Text"), None),
            Family::SegoeDisplay => (w!("Segoe UI Variable Display"), None),
            Family::YaHei => (w!("Microsoft YaHei UI"), None),
            Family::Mono => (if self.has_face(super::canvas::CODE_FACE) { w!("Cascadia Mono") } else { w!("Consolas") }, None),
            // (Asked for only once it is had: see `serif::family`.)
            Family::Serif => match self.serif.get_or_init(|| crate::serif::face().and_then(|face| face_of(&self.write, &self.loader, face).ok())) {
                Some((collection, name)) => (PCWSTR(name.as_ptr()), Some(collection)),
                None => (w!("Microsoft YaHei UI"), None),
            },
        }
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

    /// Where `text` laid out in lines `width` wide ends: how far along its last line (DIPs), and that line's top.
    pub fn wrapped_end(&self, text: &str, font: Font, width: f32) -> (f32, f32) {
        let units = text.encode_utf16().count() as u32;
        if units == 0 {
            return (0.0, 0.0);
        }
        let (mut x, mut y) = (0.0f32, 0.0f32);
        let mut metrics = windows::Win32::Graphics::DirectWrite::DWRITE_HIT_TEST_METRICS::default();
        let hit = unsafe { self.wrapped(text, font, width).HitTestTextPosition(units - 1, true, &mut x, &mut y, &mut metrics) };
        if hit.is_err() {
            return (width, 0.0);
        }
        (x, metrics.top)
    }

    /// How tall `text` is laid out in lines `width` wide.
    pub fn wrapped_height(&self, text: &str, font: Font, width: f32) -> f32 {
        let mut metrics = DWRITE_TEXT_METRICS::default();
        unsafe { self.wrapped(text, font, width).GetMetrics(&mut metrics).unwrap() };
        metrics.height
    }

    /// A note's table's cell's words laid out (see `Frame::table`): in `family`, `pixels` high (whole) at `per` pixels a
    /// DIP, bold or not, on lines `wide` pixels across (none: on one line), set in them as `align` says. Laid out as a
    /// note's lines are (see `Frame::markdown`), GDI's way, as the text services lay out theirs.
    #[allow(clippy::too_many_arguments)]
    fn cell(&self, words: &str, family: Family, pixels: f32, per: f32, bold: bool, wide: Option<f32>, align: crate::notes::table::Align) -> Result<IDWriteTextLayout> {
        use crate::notes::table::Align;
        use windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT_CENTER;
        let factory: IDWriteFactory = self.write.cast()?;
        let (face, collection) = self.face(family);
        let weight = if bold { DWRITE_FONT_WEIGHT_BOLD } else { DWRITE_FONT_WEIGHT(400) };
        let format = unsafe { factory.CreateTextFormat(face, collection, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, pixels / per, w!("zh-cn"))? };
        let wide_units: Vec<u16> = words.encode_utf16().collect();
        // (On one line: as wide as words go.)
        let across = wide.map_or(1.0e6, |wide| wide.max(0.0) / per);
        let layout = unsafe { factory.CreateGdiCompatibleTextLayout(&wide_units, &format, across, f32::MAX, per, None, false)? };
        // Its lines set as a note's are, as tall as its plain words' line (whole pixels), the baseline as far above each
        // line's foot as its plain words reach below theirs: as the text services set a cell's words written in it (see
        // `ui::note`), drawn alike.
        let plain_format = unsafe { factory.CreateTextFormat(face, collection, DWRITE_FONT_WEIGHT(400), DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, pixels / per, w!("zh-cn"))? };
        let plain = unsafe { factory.CreateGdiCompatibleTextLayout(&[u16::from(b' ')], &plain_format, f32::MAX, f32::MAX, per, None, false)? };
        let (mut count, mut row) = (0, [DWRITE_LINE_METRICS::default()]);
        unsafe { plain.GetLineMetrics(Some(&mut row), &mut count)? };
        let below = ((row[0].height - row[0].baseline) * per).round();
        let line = (pixels * crate::notes::markdown::LEADING).round();
        unsafe {
            layout.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, line / per, (line - below) / per)?;
            layout.SetWordWrapping(if wide.is_some() { DWRITE_WORD_WRAPPING_WRAP } else { DWRITE_WORD_WRAPPING_NO_WRAP })?;
            layout.SetTextAlignment(match align {
                Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Centre => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            })?;
        }
        Ok(layout)
    }

    /// A note's `table` laid out (see `notes::table::lay`), its words in `family` `pixels` high (whole) at `per` pixels a
    /// DIP, in `room` pixels across: in pixels, each cell's words laid out in it to be drawn (see `Frame::table`). (Its
    /// words not to be laid out, none of them takes room, and none is drawn.)
    pub fn table_laid(&self, table: &crate::notes::table::Table, family: Family, pixels: f32, per: f32, room: f32) -> TableLaid {
        let measure = |words: &str, bold: bool, wide: Option<f32>| {
            let mut metrics = DWRITE_TEXT_METRICS::default();
            match self.cell(words, family, pixels, per, bold, wide, crate::notes::table::Align::Left).and_then(|layout| unsafe { layout.GetMetrics(&mut metrics) }) {
                Ok(()) => (metrics.width * per, metrics.height * per),
                Err(_) => (0.0, 0.0),
            }
        };
        let laid = crate::notes::table::lay(table, pixels, room, measure);
        let side = (crate::notes::table::PAD_SIDE * pixels).round();
        let starts = |lengths: &[f32]| {
            let mut at = 0.0;
            let mut starts: Vec<f32> = lengths
                .iter()
                .map(|length| {
                    let start = at;
                    at += length;
                    start
                })
                .collect();
            starts.push(at);
            starts
        };
        let cells = table
            .rows
            .iter()
            .enumerate()
            .map(|(row, cells)| {
                cells
                    .iter()
                    .enumerate()
                    .zip(&laid.columns)
                    .filter_map(|((column, cell), wide)| {
                        let layout = self.cell(&cell.reads, family, pixels, per, row == 0, Some(wide - 1.0 - 2.0 * side), table.aligns[column]).ok()?;
                        let mut metrics = DWRITE_TEXT_METRICS::default();
                        unsafe { layout.GetMetrics(&mut metrics) }.ok()?;
                        Some((column, layout, metrics.height * per))
                    })
                    .collect()
            })
            .collect();
        TableLaid { columns_at: starts(&laid.columns), rows_at: starts(&laid.rows), laid, side, cells }
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
                layout.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_DEFAULT, 0.0, 0.0).unwrap();
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

    /// Where the first line's baseline of `text`, wrapped `width` wide in `font`, falls below its top (DIPs): its own,
    /// as the faces that draw it set it (a line in Chinese, in a face apart from the font's own, sits lower than
    /// `baseline` says).
    pub fn wrapped_baseline(&self, text: &str, font: Font, width: f32) -> f32 {
        first_baseline(&self.wrapped(text, font, width))
    }

    /// Lets go of what Direct2D keeps from earlier drawing, such as the
    /// intermediate images of effects.
    pub fn clear_caches(&self) {
        unsafe { self.device.ClearResources(0) };
    }

    /// A window sized by hand has come to rest, and been drawn so (call after that frame): what was made for it
    /// frame by frame as it was sized is let go, and the graphics driver gives back the memory let go of meanwhile
    /// (a few hundred megabytes, else held a while, or for good).
    pub fn settle(&self) {
        self.scratch.borrow_mut().take();
        self.made.borrow_mut().clear();
        self.spare.borrow_mut().clear();
        unsafe { self.dxgi.Trim() };
    }

    /// One more window shows what it draws with this device (see `life::Showing`).
    pub(super) fn show_one(&self) {
        self.showing.set(self.showing.get() + 1);
    }

    /// One window fewer shows what it draws with this device. The last gone, what drawing holds is given back now,
    /// and again once nothing has drawn for `IDLE` (what the windows let go of is the driver's to give back only once
    /// the card has done with it).
    pub(super) fn hide_one(&self) {
        self.showing.set(self.showing.get().saturating_sub(1));
        if self.showing.get() == 0 {
            self.trim_now_and_again();
            self.tend();
        }
    }

    /// What drawing holds given back as it falls due, on a timer of the thread's (see `tended`): what painting
    /// layers took, once they have stayed as they are (`let_go_once_settled`); everything, once nothing is shown
    /// (`trim_when_idle`). The timer runs while either is still due.
    fn tend(&self) {
        if self.tending.get() != 0 {
            return;
        }
        let timer = unsafe { windows::Win32::UI::WindowsAndMessaging::SetTimer(None, 0, TEND_EVERY.as_millis() as u32, Some(tended)) };
        self.tending.set(timer);
    }

    /// Gives back what drawing holds on to (see `trim`) once no window has drawn with it for `IDLE`, if one has
    /// since it was last given back: whatever was drawn last (the panel as it went, a widget closed, the settings
    /// closed, what was measured as Glance started), the graphics driver keeps what it let go of till it is told to
    /// give it back.
    pub fn trim_when_idle(&self) {
        if self.showing.get() == 0 && !self.trimmed.get() && self.last_drawn.get().is_some_and(|at| at.elapsed() >= IDLE) {
            self.trim();
        }
    }

    /// What painting a layer anew took (its effects' textures) given back once no layer has been painted anew for
    /// `IDLE`, if one has since it was last given back: a panel or a widget that stays up redraws each second only
    /// what is not kept in its layers, and what painted those is otherwise held for good (a large glass widget's,
    /// tens of megabytes). Painted anew, they are made again. (What is made from its key alone stays: see `made`.)
    pub fn let_go_once_settled(&self) {
        if self.settled.get() || self.last_painted.get().is_none_or(|at| at.elapsed() < IDLE) {
            return;
        }
        self.settled.set(true);
        self.scratch.borrow_mut().take();
        self.spare.borrow_mut().clear();
        unsafe {
            self.device.ClearResources(0);
            if let Ok(context) = self.d3d().and_then(|device| device.GetImmediateContext()) {
                context.Flush();
            }
            self.dxgi.Trim();
        }
    }

    /// Gives back what drawing holds on to while nothing is on screen (but what is made from its key alone: see
    /// `made`).
    pub fn trim(&self) {
        self.trimmed.set(true);
        self.scratch.borrow_mut().take();
        self.scratch_context.borrow_mut().take();
        self.pictures.borrow_mut().clear();
        self.layouts.borrow_mut().clear();
        self.spare.borrow_mut().clear();
        self.stops.borrow_mut().clear();
        unsafe {
            self.device.ClearResources(0);
            // What was let go of is destroyed only once the card has done what was asked of it before: asked to be
            // done now, so that the driver has it to give back.
            if let Ok(context) = self.d3d().and_then(|device| device.GetImmediateContext()) {
                context.Flush();
            }
            self.dxgi.Trim();
        }
    }

    /// As `trim`, now, and again once nothing has drawn for a while (see `trim_when_idle`): for a window that has
    /// just gone, what it let go of may still be in the card's hands now.
    pub fn trim_now_and_again(&self) {
        self.trim();
        self.trimmed.set(false);
    }

    /// Forgets the text, and the fonts, no window has drawn for a while.
    pub fn sweep(&self) {
        self.pictures.borrow_mut().retain(|_, (_, used)| used.elapsed() < PICTURE_KEPT);
        self.layouts.borrow_mut().retain(|_, (_, used)| used.elapsed() < TEXT_KEPT);
        self.formats.borrow_mut().retain(|_, (_, used)| used.elapsed() < TEXT_KEPT);
        self.stops.borrow_mut().retain(|_, (_, used)| used.elapsed() < TEXT_KEPT);
    }

    /// How many bitmaps made from their keys alone are kept (see `made`).
    #[cfg(test)]
    pub fn made_count(&self) -> usize {
        self.made.borrow().len()
    }

    /// The bitmap `key` stands for, if one was made for it and is kept (see `made`).
    fn kept(&self, key: [u32; 4]) -> Option<ID2D1Bitmap1> {
        let mut made = self.made.borrow_mut();
        let (bitmap, used) = made.get_mut(&key)?;
        *used = Instant::now();
        Some(bitmap.clone())
    }

    /// The bitmap `key` stands for: the one made for it before, else made
    /// now by `make`. Those drawn least lately are let go of past
    /// `MADE_BUDGET` (a widget sized frame by frame makes one for each size).
    pub fn made(&self, key: [u32; 4], make: impl FnOnce() -> Result<ID2D1Bitmap1>) -> Result<ID2D1Bitmap1> {
        if let Some(bitmap) = self.kept(key) {
            return Ok(bitmap);
        }
        let now = Instant::now();
        let bitmap = make()?;
        let mut made = self.made.borrow_mut();
        made.insert(key, (bitmap.clone(), now));
        let bytes = |bitmap: &ID2D1Bitmap1| {
            let size = unsafe { bitmap.GetPixelSize() };
            size.width as usize * size.height as usize * 4
        };
        let mut total: usize = made.values().map(|(bitmap, _)| bytes(bitmap)).sum();
        let mut gone = Vec::new();
        while total > MADE_BUDGET && made.len() > 1 {
            let Some(oldest) = made.iter().filter(|(k, _)| **k != key).min_by_key(|(_, (_, used))| *used).map(|(k, _)| *k) else { break };
            if let Some((bitmap, _)) = made.remove(&oldest) {
                total -= bytes(&bitmap);
                gone.push(bitmap);
            }
        }
        drop(made);
        gone.into_iter().for_each(|bitmap| self.let_go(bitmap));
        Ok(bitmap)
    }

    /// Keeps `bitmap`, let go of, to be drawn on again (see `drawn_on`), if it is one drawn on and there is room
    /// for it within `MADE_BUDGET`.
    fn let_go(&self, bitmap: ID2D1Bitmap1) {
        if unsafe { bitmap.GetOptions() }.0 & D2D1_BITMAP_OPTIONS_TARGET.0 == 0 {
            return;
        }
        let bytes = |bitmap: &ID2D1Bitmap1| {
            let size = unsafe { bitmap.GetPixelSize() };
            size.width as usize * size.height as usize * 4
        };
        let mut spare = self.spare.borrow_mut();
        if spare.iter().map(bytes).sum::<usize>() + bytes(&bitmap) <= MADE_BUDGET {
            spare.push(bitmap);
        }
    }

    /// A bitmap to draw on, at least `pixels` large, at `dpi`: one let go of before (see `made`) that holds that
    /// many pixels and no more than twice as many each way, its memory taken at `dpi`; else a new one. What is
    /// made for keys anew frame by frame (a window sized by hand) draws on the same memory, not on new memory
    /// each frame (each new one is the graphics card's, given back only a while after it is let go).
    /// `picture` as a bitmap on the card: made the first time it is drawn, and kept while it is drawn again (see
    /// `PICTURE_KEPT`).
    fn picture_bitmap(&self, dc: &ID2D1DeviceContext, picture: &super::canvas::Picture) -> Option<ID2D1Bitmap1> {
        if let Some((bitmap, used)) = self.pictures.borrow_mut().get_mut(&picture.key) {
            *used = Instant::now();
            return Some(bitmap.clone());
        }
        if picture.pixels.len() < (picture.size.0 * picture.size.1 * 4) as usize {
            return None;
        }
        let properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
            ..Default::default()
        };
        let size = D2D_SIZE_U { width: picture.size.0, height: picture.size.1 };
        let bitmap = unsafe { dc.CreateBitmap(size, Some(picture.pixels.as_ptr().cast()), picture.size.0 * 4, &properties) }.ok()?;
        self.pictures.borrow_mut().insert(picture.key, (bitmap.clone(), Instant::now()));
        Some(bitmap)
    }

    pub fn drawn_on(&self, dc: &ID2D1DeviceContext, pixels: D2D_SIZE_U, dpi: f32) -> Result<ID2D1Bitmap1> {
        let properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: dpi,
            dpiY: dpi,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
            ..Default::default()
        };
        let found = {
            let mut spare = self.spare.borrow_mut();
            let serves = |bitmap: &ID2D1Bitmap1| {
                let have = unsafe { bitmap.GetPixelSize() };
                have.width >= pixels.width && have.height >= pixels.height && have.width <= 2 * pixels.width.max(16) && have.height <= 2 * pixels.height.max(16)
            };
            spare.iter().position(serves).map(|index| spare.swap_remove(index))
        };
        unsafe {
            match found {
                Some(bitmap) => {
                    let mut had = (0.0, 0.0);
                    bitmap.GetDpi(&mut had.0, &mut had.1);
                    if had.0 == dpi {
                        Ok(bitmap)
                    } else {
                        dc.CreateBitmapFromDxgiSurface(&bitmap.GetSurface()?, Some(&properties))
                    }
                }
                None => dc.CreateBitmap(pixels, None, 0, &properties),
            }
        }
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
        let face = face_of(write, loader, std::slice::from_raw_parts(start.cast::<u8>(), size as usize));
        stream.ReleaseFileFragment(context);
        face
    }
}

/// A face from its file's bytes (OpenType), as a collection of its own, and the family name it goes by.
fn face_of(write: &IDWriteFactory6, loader: &IDWriteInMemoryFontFileLoader, bytes: &[u8]) -> Result<(IDWriteFontCollection, HSTRING)> {
    unsafe {
        // Without an owner, the loader keeps a copy of the data.
        let file = loader.CreateInMemoryFontFileReference(write, bytes.as_ptr().cast(), bytes.len() as u32, None);
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

#[cfg(any(test, feature = "studio"))]
impl Gfx {
    /// Draws `paint` into a picture `size` pixels large, at `scale` pixels
    /// per DIP, off screen; returns its pixels, rows top to bottom,
    /// premultiplied BGRA. For the studio, which films the panel, and the tests.
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
            paint(&Frame { gfx: self, dc: dc.clone(), brush, base: Cell::new(Matrix3x2::identity()), on: None, failed: Default::default(), moving: Cell::new(false) });
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

    /// As `draw_offscreen`, the picture drawn as a frame handed a region `size` large at `offset` (physical pixels)
    /// of a larger atlas is (see `Frame::apart`): that region's pixels, as `draw_offscreen` gives them. (For tests.)
    #[cfg(all(test, feature = "studio"))]
    pub fn draw_in_atlas(&self, size: (u32, u32), offset: (i32, i32), scale: f32, paint: impl FnOnce(&Frame)) -> Result<Vec<u8>> {
        use windows::Win32::Graphics::Direct2D::{
            D2D1_BITMAP_OPTIONS, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_CPU_READ, D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_MAP_OPTIONS_READ,
        };
        let properties = |options: D2D1_BITMAP_OPTIONS| D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: options,
            ..Default::default()
        };
        let whole = D2D_SIZE_U { width: size.0 + offset.0 as u32, height: size.1 + offset.1 as u32 };
        unsafe {
            let dc = self.device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let atlas = dc.CreateBitmap(whole, None, 0, &properties(D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW))?;
            let frame = Frame::whole(self, size, scale, paint)?;
            dc.SetTarget(&atlas);
            dc.BeginDraw();
            frame.put(&dc, POINT { x: offset.0, y: offset.1 })?;
            dc.EndDraw(None, None)?;
            let read = dc.CreateBitmap(whole, None, 0, &properties(D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW))?;
            read.CopyFromBitmap(None, &atlas, None)?;
            let mapped = read.Map(D2D1_MAP_OPTIONS_READ)?;
            let row = size.0 as usize * 4;
            let mut out = Vec::with_capacity(row * size.1 as usize);
            for y in 0..size.1 as usize {
                let line = mapped.bits.add((y + offset.1 as usize) * mapped.pitch as usize + offset.0 as usize * 4);
                out.extend_from_slice(std::slice::from_raw_parts(line, row));
            }
            read.Unmap()?;
            Ok(out)
        }
    }
}

/// Drawing what moves (see `Frame::moving_while`), till this goes.
pub struct Moving<'f, 'a> {
    frame: &'f Frame<'a>,
    was: bool,
}

impl Drop for Moving<'_, '_> {
    fn drop(&mut self) {
        self.frame.set_moving(self.was);
    }
}

/// The ink what is chosen of a table is filled with, and which of its cells (row, column) are chosen (see `Frame::table`).
pub type ChosenCells<'a> = (Color, &'a dyn Fn(usize, usize) -> bool);

/// A note's table laid out to be drawn (see `Gfx::table_laid`, `Frame::table`): its columns' widths and its rows'
/// heights, where each begins (and where the last ends), how far its cells' words are in from their sides, and each
/// cell's words laid out (its column, and how tall they are), all in pixels.
#[derive(Clone)]
pub struct TableLaid {
    pub laid: crate::notes::table::Laid,
    columns_at: Vec<f32>,
    rows_at: Vec<f32>,
    side: f32,
    cells: Vec<Vec<(usize, IDWriteTextLayout, f32)>>,
}

impl TableLaid {
    /// Where the words of its cell `row`, `column` are drawn (see `Frame::table`), from its corner, in pixels: across,
    /// down, how wide they may go and how tall they are. None where it has no such cell.
    pub fn words_at(&self, row: usize, column: usize) -> Option<(f32, f32, f32, f32)> {
        let (_, _, tall) = self.cells.get(row)?.iter().find(|(at, ..)| *at == column)?;
        let down = self.rows_at[row] + 1.0 + ((self.laid.rows[row] - 1.0 - tall) / 2.0).round();
        let along = self.columns_at[column] + 1.0 + self.side;
        Some((along, down, self.laid.columns[column] - 1.0 - 2.0 * self.side, *tall))
    }

    /// The cell (row, column) the point `x`, `y` from its corner (pixels) is in: none off it.
    pub fn cell_at(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        if x < 0.0 || y < 0.0 || x >= self.laid.width() || y >= self.laid.height() {
            return None;
        }
        // (The one whose room it is in, a line counted with the one after it: the last's, past the end.)
        let nth = |starts: &[f32], count: usize, along: f32| starts[..count].partition_point(|start| *start <= along).saturating_sub(1);
        Some((nth(&self.rows_at, self.laid.rows.len(), y), nth(&self.columns_at, self.laid.columns.len(), x)))
    }

    /// The column `x` from its corner (pixels) is over: before its first, the first, past its last, the last.
    pub fn column_at(&self, x: f32) -> usize {
        self.columns_at[..self.laid.columns.len()].partition_point(|start| *start <= x).saturating_sub(1)
    }
}

/// One frame being drawn onto a surface.
pub struct Frame<'a> {
    pub gfx: &'a Gfx,
    pub dc: ID2D1DeviceContext,
    brush: ID2D1SolidColorBrush,
    /// Where the surface's region starts (DIPs), and through what it is drawn besides (see `within`).
    base: Cell<Matrix3x2>,
    /// Drawn apart (see `apart`): the bitmap drawn on, and how much of it (physical pixels).
    on: Option<(ID2D1Bitmap1, (u32, u32))>,
    /// What failed of the frame's drawing as it was drawn (see `fail`): the first.
    failed: std::cell::RefCell<Option<windows::core::Error>>,
    /// Drawing what moves (see `set_moving`): nothing put on whole pixels meanwhile.
    moving: Cell<bool>,
}

impl Canvas for Frame<'_> {
    fn markdown(&self, text: &str, font: Font, inks: Inks, x: f32, y: f32, width: f32, height: f32, scrolled: f32) {
        Frame::markdown(self, text, font, inks, x, y, width, height, scrolled);
    }

    fn markdown_tall(&self, text: &str, font: Font, width: f32) -> f32 {
        Frame::markdown_tall(self, text, font, width)
    }

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
        // (On whole pixels, as what is filled in it is: else a fill to its edge stopped half a pixel short of the same
        // fill unclipped, and what was under showed there.)
        let (x, y, width, height) = self.on_pixels(x, y, width, height);
        unsafe { self.dc.PushAxisAlignedClip(&rect(x, y, width, height), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
    }

    fn on_pixels(&self, x: f32, y: f32, width: f32, height: f32) -> (f32, f32, f32, f32) {
        Frame::on_pixels(self, x, y, width, height)
    }

    fn unclip(&self) {
        unsafe { self.dc.PopAxisAlignedClip() };
    }

    fn fill(&self, color: Color, x: f32, y: f32, width: f32, height: f32) {
        let (x, y, width, height) = self.on_pixels(x, y, width, height);
        unsafe { self.dc.FillRectangle(&rect(x, y, width, height), self.brush(color)) };
    }

    fn fill_rounded(&self, color: Color, x: f32, y: f32, width: f32, height: f32, radius: f32) {
        if radius == 0.0 {
            return Canvas::fill(self, color, x, y, width, height);
        }
        // Round (a key's plate under the pointer): placed by its centre, as a circle is, so as to be one with the glyph
        // on it (placed by its box, it stood half a pixel off the glyph).
        if (width - height).abs() < 1e-3 && radius >= width / 2.0 - 1e-3 {
            return Canvas::fill_circle(self, color, Point { x: x + width / 2.0, y: y + height / 2.0 }, width / 2.0);
        }
        let (x, y, width, height) = self.on_pixels(x, y, width, height);
        let radius = radius.min(width / 2.0).min(height / 2.0);
        let shape = D2D1_ROUNDED_RECT { rect: rect(x, y, width, height), radiusX: radius, radiusY: radius };
        unsafe { self.dc.FillRoundedRectangle(&shape, self.brush(color)) };
    }

    fn fill_circle(&self, color: Color, centre: Point, radius: f32) {
        // (Its centre on a whole pixel: drawn with a ring round it or a glyph in it, which are put on whole pixels, it
        // kept its own place between pixels, and they moved apart. By its centre, not its box: circles about one centre,
        // a thumb's face and dot, stay one in another whatever their radii.)
        let (x, y) = self.on_pixel(centre.x, centre.y);
        let dot = D2D1_ELLIPSE { point: Vector2 { X: x, Y: y }, radiusX: radius, radiusY: radius };
        unsafe { self.dc.FillEllipse(&dot, self.brush(color)) };
    }

    fn stroke(&self, points: &[Point], color: Color, width: f32) {
        if let Some(path) = self.path(points, false) {
            // (Its corners round: a sharp one is drawn out to a point, which on a chart's line turning at nothing reaches below nothing.)
            unsafe { self.dc.DrawGeometry(&path, self.brush(color), width, &self.gfx.round) };
        }
    }

    fn stroke_rounded_dashed(&self, color: Color, x: f32, y: f32, width: f32, height: f32, radius: f32, line: f32) {
        Frame::stroke_rounded_dashed(self, color, x, y, width, height, radius, line);
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
        // (About its centre put as a circle's is: see `fill_circle`.)
        let (x, y) = self.on_pixel(centre.x, centre.y);
        let centre = Point { x, y };
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
        self.placed_icon(icon, centre, size, color, false);
    }

    fn pixels_per_dip(&self) -> f32 {
        let mut t = Matrix3x2::default();
        let mut dpi = (0.0f32, 0.0f32);
        unsafe {
            self.dc.GetTransform(&mut t);
            self.dc.GetDpi(&mut dpi.0, &mut dpi.1);
        }
        // (How much the transform grows a box, turned or not, by the target's DIPs to its pixels.)
        (t.M11 * t.M22 - t.M12 * t.M21).abs().sqrt() * dpi.0 / 96.0
    }

    fn picture_within(&self, picture: &super::canvas::Picture, at: (f32, f32, f32, f32), within: (f32, f32, f32, f32, f32), opacity: f32) {
        if let Some(bitmap) = self.gfx.picture_bitmap(&self.dc, picture) {
            self.bitmap_within(&bitmap, picture.size, at, within, opacity);
        }
    }

    fn stroke_rounded(&self, fill: Fill, x: f32, y: f32, width: f32, height: f32, radius: f32, line: f32) {
        let Some(brush) = self.paint_with(fill) else { return };
        let line = self.line(line);
        // A circle (a button's ring): about its centre, put as a circle's is, so as to be one with the plate in it.
        if (width - height).abs() < 1e-3 && radius >= width / 2.0 - 1e-3 {
            let (cx, cy) = self.on_pixel(x + width / 2.0, y + height / 2.0);
            let r = (width - line) / 2.0;
            let ring = D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: r, radiusY: r };
            unsafe { self.dc.DrawEllipse(&ring, &brush, line, None) };
            return;
        }
        let (x, y, width, height) = self.stroked(x, y, width, height, line);
        let radius = (radius - line / 2.0).max(0.0).min(width / 2.0).min(height / 2.0);
        let shape = D2D1_ROUNDED_RECT { rect: rect(x, y, width, height), radiusX: radius, radiusY: radius };
        unsafe { self.dc.DrawRoundedRectangle(&shape, &brush, line, None) };
    }

    fn shadow(&self, color: Color, x: f32, y: f32, width: f32, height: f32, radius: f32, blur: f32, drop: f32) {
        use windows::Win32::Graphics::Direct2D::{CLSID_D2D1Composite, D2D1_COMPOSITE_PROP_MODE, D2D1_PROPERTY_TYPE_ENUM};
        use windows::Win32::Graphics::Direct2D::Common::D2D1_COMPOSITE_MODE_SOURCE_OUT;
        let dc = &self.dc;
        let mut transform = Matrix3x2::default();
        unsafe { dc.GetTransform(&mut transform) };
        // The rectangle as a picture of its own, recorded through the frame's transform (so put on the pixels it is
        // drawn on, as the fill it shades is: recorded untransformed and drawn back through it, its edge fell between
        // pixels, a fringe of shadow showing along the fill's), drawn back untransformed; where `down` below its place.
        let shape = |down: f32| -> Result<ID2D1Image> {
            unsafe {
                let list = dc.CreateCommandList()?;
                let target = dc.GetTarget()?;
                dc.SetTarget(&list);
                Canvas::fill_rounded(self, Color::hex(0, 1.0), x, y + down, width, height, radius);
                dc.SetTarget(&target);
                list.Close()?;
                list.cast()
            }
        };
        // (Its blur as large as it was through the transform.)
        let blur = blur * stretch(&transform);
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
                dc.SetTransform(&Matrix3x2::identity());
                dc.DrawImage(&outside.GetOutput()?, None, None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
                dc.SetTransform(&transform);
            }
            Ok(())
        })();
        unsafe { dc.SetTransform(&transform) };
    }

    fn frosted(&self, x: f32, y: f32, width: f32, height: f32, radius: f32, blur: f32, below: &dyn Fn(&dyn Canvas)) {
        // (Nothing, where the device is going: the frame's end tells.)
        let mut transform = Matrix3x2::default();
        unsafe { self.dc.GetTransform(&mut transform) };
        let _ = self.frost(x, y, width, height, radius, blur, below);
        unsafe { self.dc.SetTransform(&transform) };
    }
}

impl Frame<'_> {
    /// `icon` filled, as a button's glyph that is the thing pressed (played, paused) is drawn: see `Canvas::icon`.
    pub fn icon_solid(&self, icon: Icon, centre: Point, size: f32, color: Color) {
        self.placed_icon(icon, centre, size, color, true);
    }

    /// `icon` drawn about `centre`, `size` across, in `color`: its lines, or (`solid`) filled too.
    fn placed_icon(&self, icon: Icon, centre: Point, size: f32, color: Color, solid: bool) {
        let Some(path) = self.gfx.icon_path(icon) else { return };
        // Its middle where a circle's is put (on a whole pixel: a button's plate, its ring and its glyph one about
        // another), and as many pixels across as that leaves its box's corner on a whole pixel too (an even number:
        // drawn between two, every stroke of it was spread over both, and blurred).
        let (cx, cy) = self.on_pixel(centre.x, centre.y);
        let size = match self.grid() {
            Some((t, device)) => {
                let per = t.M11 * device;
                (2.0 * whole_pixel(size * per / 2.0)).max(2.0) / per
            }
            None => size,
        };
        let scale = size / 24.0;
        let (left, top) = (cx - size / 2.0, cy - size / 2.0);
        let mut parent = Matrix3x2::default();
        unsafe {
            self.dc.GetTransform(&mut parent);
            let placed = Matrix3x2::scale(scale, scale) * Matrix3x2::translation(left, top) * parent;
            self.dc.SetTransform(&placed);
            // (Solid: filled, and its edge drawn round as ever, its corners rounded as a stroked one's.)
            if solid {
                self.dc.FillGeometry(&path, self.brush(color), None);
            }
            self.dc.DrawGeometry(&path, self.brush(color), icons::STROKE, &self.gfx.round);
            self.dc.SetTransform(&parent);
        }
    }

    /// `bitmap` (`size` px) drawn as a picture is (see `Canvas::picture_within`): covering the box `at`, its middle on
    /// the box's, within the box `within` (its corners that round), `opacity` clear.
    pub fn bitmap_within(&self, bitmap: &ID2D1Bitmap1, size: (u32, u32), at: (f32, f32, f32, f32), within: (f32, f32, f32, f32, f32), opacity: f32) {
        // (Its box on whole pixels, as a rectangle's: its edges sharp, all four. So the box it is within.)
        let (x, y, w, h) = self.on_pixels(at.0, at.1, at.2, at.3);
        let (wx, wy, ww, wh) = self.on_pixels(within.0, within.1, within.2, within.3);
        // (As large as covers the box, its middle on the box's.)
        let (pw, ph) = (size.0.max(1) as f32, size.1.max(1) as f32);
        let scale = (w / pw).max(h / ph);
        let (from_x, from_y) = (x - (pw * scale - w) / 2.0, y - (ph * scale - h) / 2.0);
        unsafe {
            let properties = windows::Win32::Graphics::Direct2D::D2D1_BITMAP_BRUSH_PROPERTIES1 {
                extendModeX: windows::Win32::Graphics::Direct2D::D2D1_EXTEND_MODE_CLAMP,
                extendModeY: windows::Win32::Graphics::Direct2D::D2D1_EXTEND_MODE_CLAMP,
                interpolationMode: windows::Win32::Graphics::Direct2D::D2D1_INTERPOLATION_MODE_LINEAR,
            };
            let placed = windows::Win32::Graphics::Direct2D::D2D1_BRUSH_PROPERTIES { opacity, transform: Matrix3x2::scale(scale, scale) * Matrix3x2::translation(from_x, from_y) };
            let Ok(brush) = self.dc.CreateBitmapBrush(bitmap, Some(&properties), Some(&placed)) else { return };
            let radius = within.4.max(0.0).min(ww.min(wh) / 2.0);
            // (Within its own box too, where the box it is within is larger: the brush goes on past its edges.)
            let own = (x, y, w, h) != (wx, wy, ww, wh);
            if own {
                self.dc.PushAxisAlignedClip(&rect(x, y, w, h), D2D1_ANTIALIAS_MODE_ALIASED);
            }
            self.dc.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: rect(wx, wy, ww, wh), radiusX: radius, radiusY: radius }, &brush);
            if own {
                self.dc.PopAxisAlignedClip();
            }
        }
    }


    /// See `Canvas::frosted`. What `below` draws is recorded through the frame's transform (on the pixels it would be
    /// drawn on), blurred and deepened, and drawn back untransformed inside the rectangle.
    #[allow(clippy::too_many_arguments)]
    fn frost(&self, x: f32, y: f32, width: f32, height: f32, radius: f32, blur: f32, below: &dyn Fn(&dyn Canvas)) -> Result<()> {
        use windows::Win32::Graphics::Direct2D::Common::{D2D_MATRIX_5X4_F, D2D_MATRIX_5X4_F_0, D2D_MATRIX_5X4_F_0_0};
        use windows::Win32::Graphics::Direct2D::{
            CLSID_D2D1ColorMatrix, CLSID_D2D1GaussianBlur, D2D1_COLORMATRIX_PROP_CLAMP_OUTPUT, D2D1_COLORMATRIX_PROP_COLOR_MATRIX,
            D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION, D2D1_LAYER_OPTIONS1_NONE, D2D1_LAYER_PARAMETERS1, D2D1_PROPERTY_TYPE_BOOL,
            D2D1_PROPERTY_TYPE_MATRIX_5X4,
        };
        let dc = &self.dc;
        let mut transform = Matrix3x2::default();
        let mut dpi = (0.0, 0.0);
        unsafe {
            dc.GetTransform(&mut transform);
            dc.GetDpi(&mut dpi.0, &mut dpi.1);
            let list = dc.CreateCommandList()?;
            let target = dc.GetTarget()?;
            dc.SetTarget(&list);
            // (Setting a target can change the context's DPI: what is under is recorded at the frame's.)
            dc.SetDpi(dpi.0, dpi.1);
            below(self);
            dc.SetTarget(&target);
            dc.SetDpi(dpi.0, dpi.1);
            dc.SetTransform(&transform);
            list.Close()?;
            let blurred = dc.CreateEffect(&CLSID_D2D1GaussianBlur)?;
            blurred.SetInput(0, &list.cast::<ID2D1Image>()?, true);
            let deviation = blur * stretch(&transform);
            blurred.SetValue(D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &deviation.to_ne_bytes())?;
            // Deepened about its lightness (a colour matrix: each colour keeps more of itself, and takes as much less
            // of the others).
            let deepened = dc.CreateEffect(&CLSID_D2D1ColorMatrix)?;
            deepened.SetInput(0, &blurred.GetOutput()?, true);
            let s = super::canvas::FROST_SATURATION;
            let light = [0.2126f32, 0.7152, 0.0722];
            let share = |from: usize, to: usize| light[from] * (1.0 - s) + if from == to { s } else { 0.0 };
            let matrix = D2D_MATRIX_5X4_F {
                Anonymous: D2D_MATRIX_5X4_F_0 {
                    Anonymous: D2D_MATRIX_5X4_F_0_0 {
                        _11: share(0, 0), _12: share(0, 1), _13: share(0, 2), _14: 0.0,
                        _21: share(1, 0), _22: share(1, 1), _23: share(1, 2), _24: 0.0,
                        _31: share(2, 0), _32: share(2, 1), _33: share(2, 2), _34: 0.0,
                        _41: 0.0, _42: 0.0, _43: 0.0, _44: 1.0,
                        _51: 0.0, _52: 0.0, _53: 0.0, _54: 0.0,
                    },
                },
            };
            let bytes = std::slice::from_raw_parts((&matrix as *const D2D_MATRIX_5X4_F).cast::<u8>(), std::mem::size_of::<D2D_MATRIX_5X4_F>());
            deepened.SetValue(D2D1_COLORMATRIX_PROP_COLOR_MATRIX.0 as u32, D2D1_PROPERTY_TYPE_MATRIX_5X4, bytes)?;
            deepened.SetValue(D2D1_COLORMATRIX_PROP_CLAMP_OUTPUT.0 as u32, D2D1_PROPERTY_TYPE_BOOL, &1u32.to_ne_bytes())?;
            let image = deepened.GetOutput()?;
            // Inside the rectangle only: it the mask of a layer, through the frame's transform.
            let (x, y, width, height) = self.on_pixels(x, y, width, height);
            let radius = radius.max(0.0).min(width / 2.0).min(height / 2.0);
            let shape = self.gfx.factory.CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT { rect: rect(x, y, width, height), radiusX: radius, radiusY: radius })?;
            let layer = D2D1_LAYER_PARAMETERS1 {
                contentBounds: D2D_RECT_F { left: f32::MIN, top: f32::MIN, right: f32::MAX, bottom: f32::MAX },
                geometricMask: std::mem::ManuallyDrop::new(Some(shape.cast()?)),
                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                maskTransform: Matrix3x2::identity(),
                opacity: 1.0,
                opacityBrush: std::mem::ManuallyDrop::new(None),
                layerOptions: D2D1_LAYER_OPTIONS1_NONE,
            };
            dc.PushLayer(&layer, None);
            dc.SetTransform(&Matrix3x2::identity());
            dc.DrawImage(&image, None, None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
            dc.SetTransform(&transform);
            dc.PopLayer();
            drop(std::mem::ManuallyDrop::into_inner(layer.geometricMask));
        }
        Ok(())
    }
}

impl<'a> Frame<'a> {
    /// A frame `size` physical pixels large, cleared, at `scale` physical pixels per DIP, drawn apart: on the device's
    /// own bitmap (see `Gfx::scratch`), from its corner, through Glance's own context. Drawn whole (see `finish`), it
    /// is put where it shows (see `put`); one whose drawing failed is not, and what showed there stays.
    ///
    /// Put where it shows pixel for pixel: a surface's region of the compositor's atlas is rarely at a whole DIP
    /// (355 px at 1.5 is 236.666…), and drawn straight into it, text's baselines, which DirectWrite puts on whole
    /// pixels, and its glyphs' places along the line went to one pixel or the other as the region did from frame to
    /// frame: the overlay's words shook by a pixel as its readings changed. And drawn straight onto a surface, a frame
    /// that failed part way showed as far as it was drawn (a panel sized on a card short of memory showed its ground
    /// with a reading or two on it), a new surface's whatever its memory held.
    pub(super) fn apart(gfx: &'a Gfx, size: (u32, u32), scale: f32) -> Result<Self> {
        let own = {
            let mut own = gfx.scratch_context.borrow_mut();
            if own.is_none() {
                *own = Some(unsafe { gfx.device.CreateDeviceContext(windows::Win32::Graphics::Direct2D::D2D1_DEVICE_CONTEXT_OPTIONS_NONE)? });
            }
            own.clone().unwrap()
        };
        let scratch = {
            let mut kept = gfx.scratch.borrow_mut();
            let fits = kept.as_ref().is_some_and(|(_, has)| has.0 >= size.0 && has.1 >= size.1);
            if !fits {
                let has = kept.as_ref().map_or((0, 0), |(_, has)| *has);
                let made = (has.0.max(size.0).max(1), has.1.max(size.1).max(1));
                let properties = D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                    dpiX: 96.0,
                    dpiY: 96.0,
                    bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
                    ..Default::default()
                };
                // (The one it had let go of first: at the largest, two windows' worth of the card's memory held.)
                *kept = None;
                let bitmap = unsafe { own.CreateBitmap(D2D_SIZE_U { width: made.0, height: made.1 }, None, 0, &properties)? };
                *kept = Some((bitmap, made));
            }
            kept.as_ref().unwrap().0.clone()
        };
        let dc = own;
        unsafe {
            dc.SetTarget(&scratch);
            dc.BeginDraw();
        }
        let base = Matrix3x2::identity();
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
        let brush = match unsafe { dc.CreateSolidColorBrush(&D2D1_COLOR_F::default(), None) } {
            Ok(brush) => brush,
            Err(error) => {
                // (Its drawing ended, its bitmap let go: left begun, the next frame could not begin it.)
                unsafe {
                    let _ = dc.EndDraw(None, None);
                    dc.SetTarget(None);
                }
                return Err(error);
            }
        };
        Ok(Frame { gfx, dc, brush, base: Cell::new(base), on: Some((scratch, size)), failed: Default::default(), moving: Cell::new(false) })
    }

    /// A frame drawn apart by `paint` (see `apart`) and finished, ready to be put where it shows; failed, if anything
    /// of it did, and then the surface it was for left as it was.
    pub(super) fn whole(gfx: &'a Gfx, size: (u32, u32), scale: f32, paint: impl FnOnce(&Frame)) -> Result<Self> {
        let frame = Frame::apart(gfx, size, scale)?;
        paint(&frame);
        frame.finish()?;
        Ok(frame)
    }

    /// Something of the frame failed as it was drawn (a layer, a bitmap not made): the frame is not shown (see
    /// `finish`).
    pub fn fail(&self, error: windows::core::Error) {
        self.failed.borrow_mut().get_or_insert(error);
    }

    /// `draw` done on its context counted in physical pixels (as the text services count: see `editor`), at the
    /// frame's place, grown and turned as it is there (a note lifted and leaning: see `widget::Widget::carry`); its DIPs
    /// as they were after.
    pub fn in_pixels(&self, draw: impl FnOnce(&windows::Win32::Graphics::Direct2D::ID2D1RenderTarget)) {
        let (mut x, mut y) = (96.0, 96.0);
        let mut was = Matrix3x2::default();
        unsafe {
            self.dc.GetDpi(&mut x, &mut y);
            self.dc.GetTransform(&mut was);
            self.dc.SetDpi(96.0, 96.0);
            self.dc.SetTransform(&in_pixels(was, (x / 96.0, y / 96.0)));
        }
        if let Ok(target) = self.dc.cast() {
            draw(&target);
        }
        unsafe {
            self.dc.SetDpi(x, y);
            self.dc.SetTransform(&was);
        }
    }

    /// Its drawing done (handed to the card), as drawn apart (see `apart`): failed, if anything of it did (see
    /// `fail`), and then not to be put where it shows.
    pub(super) fn finish(&self) -> Result<()> {
        if self.on.is_some() {
            unsafe {
                let ended = self.dc.EndDraw(None, None);
                self.dc.SetTarget(None);
                ended?;
            }
        }
        match self.failed.borrow_mut().take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// What was drawn apart (see `apart`), finished, put on `dc` (a surface's context, handed out for drawing) at
    /// `offset`, pixel for pixel: its place given in pixels, no DIPs between.
    pub(super) fn put(&self, dc: &ID2D1DeviceContext, offset: POINT) -> Result<()> {
        use windows::Win32::Graphics::Direct2D::Common::D2D1_COMPOSITE_MODE_SOURCE_COPY;
        use windows::Win32::Graphics::Direct2D::{D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_UNIT_MODE_PIXELS};
        let Some((scratch, size)) = &self.on else { return Ok(()) };
        unsafe {
            let image: ID2D1Image = scratch.cast()?;
            dc.SetUnitMode(D2D1_UNIT_MODE_PIXELS);
            dc.SetTransform(&Matrix3x2::identity());
            let from = D2D_RECT_F { left: 0.0, top: 0.0, right: size.0 as f32, bottom: size.1 as f32 };
            dc.DrawImage(&image, Some(&Vector2 { X: offset.x as f32, Y: offset.y as f32 }), Some(&from), D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_COMPOSITE_MODE_SOURCE_COPY);
            // Handed to the card now, before the next frame (another window's, on the same bitmap) draws on it: else
            // the compositor's context may keep the copying a while, and the next window's drawing be done first.
            dc.Flush(None, None)?;
        }
        Ok(())
    }

    /// A brush for `fill`; none if Direct2D cannot make it.
    fn paint_with(&self, fill: Fill) -> Option<windows::Win32::Graphics::Direct2D::ID2D1Brush> {
        match fill {
            Fill::Solid(color) => self.brush(color).cast().ok(),
            Fill::Down { top, from, bottom, to } => self.gradient(Vector2 { X: 0.0, Y: top }, from, Vector2 { X: 0.0, Y: bottom }, to),
        }
    }

    /// A brush fading from `from` at `start` to `to` at `end`.
    pub(super) fn gradient(&self, start: Vector2, from: Color, end: Vector2, to: Color) -> Option<windows::Win32::Graphics::Direct2D::ID2D1Brush> {
        // The colours' stops, made once for them while they are drawn with.
        let key = [from.r, from.g, from.b, from.a, to.r, to.g, to.b, to.a].map(f32::to_bits);
        let now = Instant::now();
        let collection = match self.gfx.stops.borrow_mut().get_mut(&key) {
            Some((collection, used)) => {
                *used = now;
                Some(collection.clone())
            }
            None => None,
        };
        let collection = match collection {
            Some(collection) => collection,
            None => {
                let stops = [D2D1_GRADIENT_STOP { position: 0.0, color: from.d2d() }, D2D1_GRADIENT_STOP { position: 1.0, color: to.d2d() }];
                let made = unsafe { ID2D1RenderTarget::CreateGradientStopCollection(&self.dc, &stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) }.ok()?;
                self.gfx.stops.borrow_mut().insert(key, (made.clone(), now));
                made
            }
        };
        unsafe {
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

    /// The transform drawn through now and how many of the target's pixels a DIP is, where the transform keeps axes
    /// as they are (no turn, skew or mirror): what whole pixels are reckoned by.
    fn grid(&self) -> Option<(Matrix3x2, f32)> {
        if self.moving.get() {
            return None;
        }
        self.axes()
    }

    /// The transform drawn through now and how many of the target's pixels a DIP is, where the transform keeps axes
    /// as they are (no turn, skew or mirror), whether or not what is drawn moves.
    fn axes(&self) -> Option<(Matrix3x2, f32)> {
        let mut t = Matrix3x2::default();
        let mut dpi = (0.0f32, 0.0f32);
        unsafe {
            self.dc.GetTransform(&mut t);
            self.dc.GetDpi(&mut dpi.0, &mut dpi.1);
        }
        (t.M12 == 0.0 && t.M21 == 0.0 && t.M11 > 0.0 && t.M22 > 0.0 && dpi.0 > 0.0).then_some((t, dpi.0 / 96.0))
    }

    /// Where `(x, y)` (DIPs, as drawn now) lands on a whole pixel of what is drawn on, moved the least to (see `grid`);
    /// as it is where the transform turns, skews or mirrors, or for a place too far to count (as a whole frame's
    /// bounds are given).
    pub fn on_pixel(&self, x: f32, y: f32) -> (f32, f32) {
        let Some((t, device)) = self.grid() else { return (x, y) };
        let whole = |v: f32, scale: f32, offset: f32| {
            if !v.is_finite() || v.abs() > 1.0e6 {
                return v;
            }
            (whole_pixel((v * scale + offset) * device) / device - offset) / scale
        };
        (whole(x, t.M11, t.M31), whole(y, t.M22, t.M32))
    }

    /// The path a line `line` wide (as `line` makes it) is drawn along, round the box `(x, y, width, height)` (DIPs)
    /// whose outer edge it is: the box put on whole pixels as a fill of it is (see `on_pixels`), so an outline and
    /// the fill it rings come out as one, and two outlines as large as large wherever they are; the path half the
    /// line in from it, so the line covers whole pixels inside it (one drawn on the box's edge itself was spread over
    /// two pixels, at half strength).
    pub fn stroked(&self, x: f32, y: f32, width: f32, height: f32, line: f32) -> (f32, f32, f32, f32) {
        let (x, y, width, height) = self.on_pixels(x, y, width, height);
        let half = line / 2.0;
        (x + half, y + half, (width - line).max(0.0), (height - line).max(0.0))
    }

    /// A line `line` (DIPs) wide made whole pixels wide, one at least (one 1.25 pixels wide was a pixel and a soft
    /// quarter of one).
    pub fn line(&self, line: f32) -> f32 {
        let Some((t, device)) = self.grid() else { return line };
        let per = t.M11 * device;
        whole_pixel(line * per).max(1.0) / per
    }

    /// Where words drawn from `y` (DIPs, as drawn now), their baseline `baseline` below it, have that baseline on a whole
    /// pixel of what is drawn on, moved the least to: whether or not what is drawn moves, for words that glide to rest
    /// there drawn where they fall all along (put on whole pixels only once at rest, they stepped by up to a pixel as
    /// they came to rest: see `set_moving`), crisp there.
    pub fn baseline_on_pixel(&self, y: f32, baseline: f32) -> f32 {
        let Some((t, device)) = self.axes() else { return y };
        let at = ((y + baseline) * t.M22 + t.M32) * device;
        if !at.is_finite() || at.abs() > 1.0e6 {
            return y;
        }
        y + (whole_pixel(at) - at) / (t.M22 * device)
    }

    /// The rectangle `(x, y, width, height)` (DIPs) with its edges on whole pixels (see `on_pixel`), none shrunk away:
    /// a side that had length keeps at least a pixel's.
    pub fn on_pixels(&self, x: f32, y: f32, width: f32, height: f32) -> (f32, f32, f32, f32) {
        let Some((t, device)) = self.grid() else { return (x, y, width, height) };
        // Its corner put on a whole pixel, and its size made whole pixels apart: as large wherever it is. (Its far
        // edges put on whole pixels as its near ones are, two bars as tall came out a pixel apart in height where a
        // DIP is not whole pixels, at a panel size of 90%, as the screen's pixels fell under each.)
        let (left, top) = self.on_pixel(x, y);
        let whole = |length: f32, scale: f32| {
            let pixels = length * scale * device;
            if !pixels.is_finite() || pixels.abs() > 1.0e6 {
                return length;
            }
            // (A side that had length keeps at least a pixel's.)
            let pixels = if length > 0.0 { whole_pixel(pixels).max(1.0) } else { whole_pixel(pixels) };
            pixels / (scale * device)
        };
        (left, top, whole(width, t.M11), whole(height, t.M22))
    }

    /// Draws from here on (`on`) what moves by less than a pixel a frame (a lyric gliding along its rail) where it
    /// falls, nothing put on whole pixels, its text's glyphs and baselines neither (put on whole pixels, it stood
    /// still, then jumped a pixel at a time); or no longer. What it was before, to be put back. At rest, what is drawn
    /// is put on pixels as everything else is.
    pub fn set_moving(&self, on: bool) -> bool {
        self.moving.replace(on)
    }

    /// As `set_moving`, while what it gives is kept (`on`: what is drawn moves), then as it was.
    pub fn moving_while(&self, on: bool) -> Moving<'_, 'a> {
        Moving { frame: self, was: self.set_moving(on || self.moving.get()) }
    }

    /// How text is drawn: put on whole pixels, or where it falls while what is drawn moves (see `set_moving`), and in
    /// its colours (see `text_options`).
    fn text_snap(&self) -> windows::Win32::Graphics::Direct2D::D2D1_DRAW_TEXT_OPTIONS {
        text_options(if self.moving.get() { windows::Win32::Graphics::Direct2D::D2D1_DRAW_TEXT_OPTIONS_NO_SNAP } else { D2D1_DRAW_TEXT_OPTIONS_NONE })
    }

    /// Draws from here on with `(x, y)`, in DIPs from the surface's corner, as the origin.
    pub fn origin(&self, x: f32, y: f32) {
        self.place(Matrix3x2::translation(x, y));
    }

    /// Draws from here on through `transform`, from the surface's corner.
    pub fn place(&self, transform: Matrix3x2) {
        unsafe { self.dc.SetTransform(&(transform * self.base.get())) };
    }

    /// Draws what `paint` draws through `transform` as well, from the surface's corner: what it places (see `place`)
    /// placed through `transform` too, as all it draws is.
    pub fn within(&self, transform: Matrix3x2, paint: impl FnOnce(&Frame)) {
        let (base, mut was) = (self.base.get(), Matrix3x2::default());
        unsafe { self.dc.GetTransform(&mut was) };
        self.base.set(transform * base);
        unsafe { self.dc.SetTransform(&(transform * base)) };
        paint(self);
        self.base.set(base);
        unsafe { self.dc.SetTransform(&was) };
    }

    /// Draws what `paint` draws `scale` times as large about `(x, y)` (DIPs), placed as it is now otherwise.
    pub fn scaled(&self, x: f32, y: f32, scale: f32, paint: impl FnOnce(&Frame)) {
        let mut was = Matrix3x2::default();
        unsafe { self.dc.GetTransform(&mut was) };
        let about = Matrix3x2::translation(-x, -y) * Matrix3x2 { M11: scale, M12: 0.0, M21: 0.0, M22: scale, M31: 0.0, M32: 0.0 } * Matrix3x2::translation(x, y);
        unsafe { self.dc.SetTransform(&(about * was)) };
        paint(self);
        unsafe { self.dc.SetTransform(&was) };
    }

    /// Draws what `paint` draws turned by `angle` (radians, clockwise) and stretched `sx`, `sy` times about `(x, y)`
    /// (DIPs), placed as it is now otherwise.
    pub fn turned(&self, x: f32, y: f32, angle: f32, (sx, sy): (f32, f32), paint: impl FnOnce(&Frame)) {
        let mut was = Matrix3x2::default();
        unsafe { self.dc.GetTransform(&mut was) };
        let (sin, cos) = angle.sin_cos();
        let turn = Matrix3x2 { M11: sx * cos, M12: sx * sin, M21: -sy * sin, M22: sy * cos, M31: 0.0, M32: 0.0 };
        unsafe { self.dc.SetTransform(&(Matrix3x2::translation(-x, -y) * turn * Matrix3x2::translation(x, y) * was)) };
        paint(self);
        unsafe { self.dc.SetTransform(&was) };
    }

    pub fn brush(&self, color: Color) -> &ID2D1SolidColorBrush {
        unsafe { self.brush.SetColor(&color.d2d()) };
        &self.brush
    }

    /// A glow of `color` round what `paint` draws (which is not itself drawn): its shape blurred by a deviation
    /// of `blur` DIPs. Nothing, where the device is going.
    pub fn glow(&self, color: Color, blur: f32, paint: impl FnOnce(&Frame)) {
        let dc = &self.dc;
        let _ = (|| -> Result<()> {
            unsafe {
                // What is drawn as a picture of its own, recorded through the frame's transform (put on the pixels
                // it is drawn on, as what it glows under is: recorded untransformed and drawn back through it, the glow
                // fell a fraction of a pixel off the words it rings), drawn back untransformed, its blur as large.
                let list = dc.CreateCommandList()?;
                let (target, mut transform) = (dc.GetTarget()?, Matrix3x2::default());
                dc.GetTransform(&mut transform);
                dc.SetTarget(&list);
                paint(self);
                dc.SetTarget(&target);
                dc.SetTransform(&transform);
                list.Close()?;
                let blur = blur * stretch(&transform);
                let cast = dc.CreateEffect(&CLSID_D2D1Shadow)?;
                cast.SetInput(0, &list.cast::<ID2D1Image>()?, true);
                cast.SetValue(D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &blur.to_ne_bytes())?;
                let rgba = [color.r, color.g, color.b, color.a];
                cast.SetValue(D2D1_SHADOW_PROP_COLOR.0 as u32, D2D1_PROPERTY_TYPE_VECTOR4, std::slice::from_raw_parts(rgba.as_ptr().cast(), 16))?;
                dc.SetTransform(&Matrix3x2::identity());
                dc.DrawImage(&cast.GetOutput()?, None, None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
                dc.SetTransform(&transform);
            }
            Ok(())
        })();
    }

    /// The outline of the rounded rectangle, dashed, a line `width` DIPs wide inside its edge (see `stroked`).
    #[allow(clippy::too_many_arguments)]
    pub fn stroke_rounded_dashed(&self, color: Color, x: f32, y: f32, w: f32, h: f32, radius: f32, width: f32) {
        let width = self.line(width);
        let (x, y, w, h) = self.stroked(x, y, w, h, width);
        let radius = (radius - width / 2.0).max(0.0);
        let shape = D2D1_ROUNDED_RECT { rect: rect(x, y, w, h), radiusX: radius, radiusY: radius };
        unsafe { self.dc.DrawRoundedRectangle(&shape, self.brush(color), width, &self.gfx.dashed) };
    }

    /// Draws `text` in lines `width` wide, the first line's box at `(x, y)` (on a whole pixel: see `text`).
    pub fn text_wrapped(&self, text: &str, font: Font, color: Color, x: f32, y: f32, width: f32) {
        let (x, y) = self.on_pixel(x, y);
        let layout = self.gfx.wrapped(text, font, width);
        unsafe { self.dc.DrawTextLayout(windows_numerics::Vector2 { X: x, Y: y }, &layout, self.brush(color), self.text_snap()) };
    }

    /// Draws a note's words as the box they are written in does (see `Canvas::markdown`): laid out in a face of the
    /// font's family, each stretch's weight, slant, size, strike, face and ink set on it (the faces chosen by weight
    /// and slant, as the text services choose them, slanted where the face has no italic); each line set as tall as
    /// `markdown::line_heights` says, its words as far above its foot as plain words reach below their baseline, as the text
    /// services set a line exactly so tall. Its blocks of code's boxes drawn under them, reaching out past its sides.
    /// Scrolled `scrolled` down (DIPs, put on whole pixels as the box scrolls by them): its lines from there drawn.
    #[allow(clippy::too_many_arguments)]
    pub fn markdown(&self, text: &str, font: Font, inks: Inks, x: f32, y: f32, width: f32, height: f32, scrolled: f32) {
        // (Clipped, its blocks of code's boxes' sides within, and let go of whatever fails in between: the clip left on,
        // the whole frame was lost.)
        let out = crate::notes::markdown::CODE_OUT * font.size + 1.0;
        Canvas::clip(self, x - out, y, width + 2.0 * out, height);
        let drawn = self.markdown_laid(text, font, inks, (x, y, width, height), Some(scrolled));
        Canvas::unclip(self);
        if let Err(error) = drawn {
            self.fail(error);
        }
    }

    /// How tall a note's words `text` are, written `width` wide in `font` as `markdown` writes them: all their lines
    /// laid out (nothing drawn), to the foot of the last, in DIPs on whole pixels.
    pub fn markdown_tall(&self, text: &str, font: Font, width: f32) -> f32 {
        let clear = Inks { words: Color::CLEAR, faint: Color::CLEAR, accent: Color::CLEAR };
        match self.markdown_laid(text, font, clear, (0.0, 0.0, width, f32::INFINITY), None) {
            Ok(tall) => tall,
            Err(error) => {
                self.fail(error);
                0.0
            }
        }
    }

    /// A note's words laid out as `markdown` lays them out in the box `(x, y, width, height)`, and, `scrolled` that far
    /// (DIPs), drawn there (none: laid out only, every line). How far down the lines laid out reach.
    fn markdown_laid(&self, text: &str, font: Font, inks: Inks, (x, y, width, height): (f32, f32, f32, f32), scrolled: Option<f32>) -> Result<f32> {
        use crate::notes::markdown::{line_heights, Drawn, BOX_IN, BOX_RISE, BOX_ROOM, BOX_SIDE, CODE_OUT};
        {
            let wide: Vec<u16> = text.encode_utf16().collect();
            let factory: IDWriteFactory = self.gfx.write.cast()?;
            let weight = |bold: bool| if bold { DWRITE_FONT_WEIGHT_BOLD } else { DWRITE_FONT_WEIGHT(font.weight.round() as i32) };
            let (face, collection) = self.gfx.face(font.family);
            let format = unsafe { factory.CreateTextFormat(face, collection, weight(false), DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, font.size, w!("zh-cn"))? };
            let brush = |ink| unsafe { self.dc.CreateSolidColorBrush(&inks.of(ink).d2d(), None) };
            // (Each size made whole pixels high, as the text services make a face's height, a font's height in whole
            // pixels as Windows' fonts are asked for: else its words came out a little smaller, and its lines closer.)
            let per = self.axes().map_or(1.0, |(t, device)| t.M11 * device);
            let whole = |size: f32| (size * per).round().max(1.0) / per;
            let size = whole(font.size);
            let out = CODE_OUT * size;
            let dressing = crate::notes::markdown::dressing(text);
            // What its faces have no glyphs for, in the faces the system falls back on (as the text services are given).
            let code: Vec<_> = dressing.iter().filter(|(_, dress)| dress.code).map(|(range, _)| range.clone()).collect();
            let fallen = fallen_back(&wide, &code, &unsafe { face.to_string() }.unwrap_or_default(), super::canvas::CODE_FACE);
            // Its marks put away, as a note shows (see `notes::markdown::Shown`; none, its source asked for): each
            // stood in for by nothing; a to-do's list mark, box and the space after them, a bullet and the space after
            // it, a quote's marks and the space after them, by the room what stands in for them is drawn in, as wide as
            // the text services make that space where it is written.
            let shown = crate::notes::shown_as(text);
            let hidden = shown.hidden();
            let nothing: IDWriteInlineObject = Room { width: 0.0 }.into();
            let room = |wide: f32| -> IDWriteInlineObject { Room { width: (wide * size * per).round() / per }.into() };
            let (left, top) = self.on_pixel(x, y);
            // (Scrolled by whole pixels, as the box is: its glyphs stay on them.)
            let scrolled_by = scrolled.map_or(0.0, |scrolled| (scrolled.max(0.0) * per).round() / per);
            let top = top - scrolled_by;
            // Laid out as far as is seen (all of it, where only its height is asked).
            let until = if scrolled.is_some() { scrolled_by + height } else { f32::INFINITY };
            // Its tables laid out in its room (in pixels), their lines as tall as they take, and its rules.
            let pixels = size * per;
            let tables: Vec<_> = shown.tables.iter().map(|table| (table, self.gfx.table_laid(table, font.family, pixels, per, (width * per).floor()))).collect();
            let drawn = shown.drawn_lines(pixels, tables.iter().map(|(table, laid)| (table.span.clone(), laid.laid.height())));
            // Each line laid out on its own, as tall as it is set, under the one before: all laid out first, its blocks
            // of code's boxes drawn under them.
            // (Each line's words, how far down it is and how tall each of its rows, it laid out whole (where what stands
            // in for its marks is), and its words as drawn (from `indent` in, under each other: see `lay`), on `rows`.)
            struct Line {
                words: std::ops::Range<usize>,
                down: f32,
                tall: f32,
                whole_line: IDWriteTextLayout,
                layout: IDWriteTextLayout,
                indent: f32,
                rows: Vec<DWRITE_LINE_METRICS>,
            }
            // Each line's baseline as far above its foot as plain words reach below theirs, in whole pixels, whatever is
            // on it (a heading's larger words, bold ones): as the text services set a line exactly so tall, by the words
            // the box is written in, not the line's own.
            let plain = unsafe { factory.CreateGdiCompatibleTextLayout(&[u16::from(b' ')], &format, f32::MAX, f32::MAX, per, None, false)? };
            unsafe { plain.SetFontSize(size, DWRITE_TEXT_RANGE { startPosition: 0, length: 1 })? };
            let mut count = 0;
            let mut row = [DWRITE_LINE_METRICS::default()];
            unsafe { plain.GetLineMetrics(Some(&mut row), &mut count)? };
            let below = ((row[0].height - row[0].baseline) * per).round() / per;
            // The words `words` (of a line, without its line break) laid out `room` wide, dressed, their marks put away.
            // (Laid out and drawn as the text services lay out and draw words, GDI's way, glyphs on whole pixels: else the
            // same words, written in and not, were drawn apart, those not written in softer.)
            let lay = |words: std::ops::Range<usize>, room_wide: f32| -> Result<IDWriteTextLayout> {
                // What of `range` (in its note) is in them, as counted in them.
                let on_line = |range: &std::ops::Range<usize>| {
                    let (from, to) = (range.start.max(words.start), range.end.min(words.end));
                    (from < to).then(|| DWRITE_TEXT_RANGE { startPosition: (from - words.start) as u32, length: (to - from) as u32 })
                };
                let layout = unsafe { factory.CreateGdiCompatibleTextLayout(&wide[words.clone()], &format, room_wide.max(0.0), f32::MAX, per, None, false)? };
                unsafe {
                    layout.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
                    // (Tabs as far apart as the text services set them: else what follows one, and how lines wrap, moved.)
                    layout.SetIncrementalTabStop(super::editor::TAB_PIXELS / per)?;
                }
                for (range, dress) in &dressing {
                    let Some(range) = on_line(range) else { continue };
                    unsafe {
                        layout.SetFontWeight(weight(dress.bold), range)?;
                        layout.SetFontStyle(if dress.italic { DWRITE_FONT_STYLE_ITALIC } else { DWRITE_FONT_STYLE_NORMAL }, range)?;
                        layout.SetFontSize(whole(font.size * dress.size), range)?;
                        layout.SetStrikethrough(dress.struck, range)?;
                        if dress.code {
                            layout.SetFontFamilyName(&HSTRING::from(super::canvas::CODE_FACE), range)?;
                        }
                        layout.SetDrawingEffect(&brush(dress.ink)?, range)?;
                    }
                }
                for (range, family) in &fallen {
                    let Some(range) = on_line(range) else { continue };
                    unsafe { layout.SetFontFamilyName(&HSTRING::from(family.as_str()), range)? };
                }
                for range in hidden.iter().filter_map(on_line) {
                    unsafe { layout.SetInlineObject(&nothing, range)? };
                }
                let rooms = shown.boxes.iter().map(|boxed| (boxed.marks.start..boxed.room + 1, BOX_ROOM)).chain(shown.roomed.iter().map(|roomed| (roomed.marks.start..roomed.room + 1, roomed.drawn.room())));
                for (range, wide) in rooms.filter(|(range, _)| words.contains(&range.start)) {
                    if let Some(range) = on_line(&range) {
                        unsafe { layout.SetInlineObject(&room(wide), range)? };
                    }
                }
                Ok(layout)
            };
            let mut laid: Vec<Line> = Vec::new();
            let mut down = 0.0;
            for (line, tall) in line_heights(text, pixels, &drawn) {
                if down >= until {
                    break;
                }
                // (Its words, without its line break: a `\r` before it too, as a file written on Windows has.)
                let breaks = [u16::from(b'\n'), u16::from(b'\r')];
                let end = (line.start..line.end).rev().find(|&at| !breaks.contains(&wide[at])).map_or(line.start, |at| at + 1);
                let words = line.start..end;
                let whole_line = lay(words.clone(), width)?;
                // With marks put away at its start (a list's item's, a quote's), its rows after its first as far in as its
                // words begin on its first, as the box sets them: its words laid out from there, in the room left.
                let lead = shown.leads.iter().find(|lead| lead.start == words.start).map(|lead| -> Result<(usize, f32)> {
                    let (mut at_x, mut at_y) = (0.0, 0.0);
                    let mut start = DWRITE_HIT_TEST_METRICS::default();
                    unsafe { whole_line.HitTestTextPosition((lead.end - words.start) as u32, false, &mut at_x, &mut at_y, &mut start)? };
                    Ok((lead.end, start.left))
                });
                let (layout, indent) = match lead.transpose()? {
                    Some((from, indent)) if indent > 0.0 => (lay(from..words.end, width - indent)?, indent),
                    _ => (whole_line.clone(), 0.0),
                };
                // The rows it is laid out on (a line too long for its room wrapped onto more than one: asked for its
                // first row alone, DirectWrite said there was not room for them and nothing after it was drawn).
                let mut count = 0;
                let _ = unsafe { layout.GetLineMetrics(None, &mut count) };
                let mut rows = vec![DWRITE_LINE_METRICS::default(); count.max(1) as usize];
                unsafe { layout.GetLineMetrics(Some(&mut rows), &mut count)? };
                let tall = tall / per;
                unsafe { layout.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, tall, tall - below)? };
                let lines = rows.len().max(1) as f32;
                laid.push(Line { words, down, tall, whole_line, layout, indent, rows });
                down += lines * tall;
            }
            // How far down each line's foot is.
            let foot = |line: &Line| line.down + line.rows.len().max(1) as f32 * line.tall;
            if scrolled.is_none() {
                return Ok(down);
            }
            for code in &shown.codes {
                let first = laid.iter().find(|line| line.words.start == code.span.start);
                let last = laid.iter().rev().find(|line| line.words.start <= code.span.end);
                if let (Some(first), Some(last)) = (first, last) {
                    super::canvas::code_box(self, inks, left - out, top + first.down, width + 2.0 * out, foot(last) - first.down, size);
                }
            }
            // (Those scrolled out of sight above left undrawn.)
            for line in laid.iter().filter(|line| foot(line) > scrolled_by) {
                let (words, down, tall) = (&line.words, line.down, line.tall);
                if let Some((_, laid)) = tables.iter().find(|(table, _)| table.span.start == words.start) {
                    self.table(laid, inks, left, top + down, (y, y + height), None, None);
                }
                if shown.rules.iter().any(|rule| rule.start == words.start) {
                    self.rule(inks, left, top + down, width, tall * per);
                }
                unsafe { self.dc.DrawTextLayout(windows_numerics::Vector2 { X: left + line.indent, Y: top + down }, &line.layout, self.brush(inks.words), self.text_snap()) };
                // Each to-do's box, list's dot and quote's bars in its room, on its first row (what stands in for marks
                // at a line's start begins it), by its baseline (`tall - below` down it).
                let place = |at: usize| -> Result<f32> {
                    let (mut at_x, mut at_y) = (0.0, 0.0);
                    let mut start = DWRITE_HIT_TEST_METRICS::default();
                    unsafe { line.whole_line.HitTestTextPosition((at - words.start) as u32, false, &mut at_x, &mut at_y, &mut start)? };
                    Ok(left + start.left)
                };
                let row = top + down;
                for boxed in shown.boxes.iter().filter(|boxed| words.contains(&boxed.marks.start)) {
                    let at_x = place(boxed.marks.start)?;
                    let side = BOX_SIDE * size;
                    let middle = row + tall - below - BOX_RISE * size;
                    super::canvas::task_box(self, inks, at_x + BOX_IN * size, middle - side / 2.0, side, boxed.done);
                }
                for roomed in shown.roomed.iter().filter(|roomed| words.contains(&roomed.marks.start)) {
                    let at_x = place(roomed.marks.start)?;
                    match roomed.drawn {
                        Drawn::Bullet => super::canvas::bullet(self, inks, at_x, row + tall - below, size),
                        Drawn::Bars(deep) => super::canvas::quote_bars(self, inks, (at_x, row, top + foot(line) - row), deep, size, per),
                    }
                }
            }
            Ok(down)
        }
    }

    /// Draws a note's table as `laid` out (see `Gfx::table_laid`), from (`x`, `y`) (DIPs, put on a whole pixel), in its
    /// `inks`: its lines a pixel wide (`rule_ink`), its head's row tinted and its words bold, each cell's words set in it
    /// as its column says, in the middle of its row's height. Only its rows within `within` (from, to: DIPs down) are
    /// drawn: those out of it would not show. The same for a note written in and one shown (see `Writing::paint`,
    /// `markdown`): pixel for pixel alike. The words of the cell `written_in` (row, column) are not drawn: they are being
    /// written in, in a box of their own over it. The cells `chosen` says are chosen (row, column) are filled in its
    /// ink within their lines, under their words: what is chosen of a note written in as its words show it chosen.
    #[allow(clippy::too_many_arguments)]
    pub fn table(&self, laid: &TableLaid, inks: Inks, x: f32, y: f32, within: (f32, f32), written_in: Option<(usize, usize)>, chosen: Option<ChosenCells>) {
        let per = self.axes().map_or(1.0, |(t, device)| t.M11 * device);
        let (left, top) = self.on_pixel(x, y);
        let (width, height) = (laid.laid.width(), laid.laid.height());
        let line = rule_ink(inks);
        // (The rows that show: from the last beginning above `within`, to the first beginning below it.)
        let rows = laid.laid.rows.len();
        let first = laid.rows_at[..rows].partition_point(|start| top + start / per <= within.0).saturating_sub(1);
        let last = laid.rows_at[..rows].partition_point(|start| top + start / per < within.1);
        if let Some(head) = laid.laid.rows.first().filter(|_| first == 0) {
            self.fill(Color { a: HEAD_TINT, ..inks.faint }, left, top, width / per, (head + 1.0) / per);
        }
        if let Some((ink, chosen)) = chosen {
            for row in first..last.min(rows) {
                for column in (0..laid.laid.columns.len()).filter(|column| chosen(row, *column)) {
                    let (along, down) = (laid.columns_at[column] + 1.0, laid.rows_at[row] + 1.0);
                    self.fill(ink, left + along / per, top + down / per, (laid.laid.columns[column] - 1.0) / per, (laid.laid.rows[row] - 1.0) / per);
                }
            }
        }
        let (from, to) = (laid.rows_at[first], laid.rows_at[last].min(height - 1.0) + 1.0);
        for start in &laid.columns_at {
            self.fill(line, left + start / per, top + from / per, 1.0 / per, (to - from) / per);
        }
        for start in &laid.rows_at[first..=last] {
            self.fill(line, left, top + start / per, width / per, 1.0 / per);
        }
        for (row, cells) in laid.cells.iter().enumerate().take(last).skip(first) {
            for (column, layout, tall) in cells.iter().filter(|(column, ..)| written_in != Some((row, *column))) {
                // (In the middle of its row, under the row's line, on a whole pixel.)
                let down = laid.rows_at[row] + 1.0 + ((laid.laid.rows[row] - 1.0 - tall) / 2.0).round();
                let along = laid.columns_at[*column] + 1.0 + laid.side;
                unsafe { self.dc.DrawTextLayout(windows_numerics::Vector2 { X: left + along / per, Y: top + down / per }, layout, self.brush(inks.words), self.text_snap()) };
            }
        }
    }

    /// Draws a note's rule (`---`) across `width` from `x` (DIPs), on its line from `y` `tall` pixels high: a line a
    /// pixel wide through its middle, in its `inks` (see `rule_ink`).
    pub fn rule(&self, inks: Inks, x: f32, y: f32, width: f32, tall: f32) {
        let per = self.axes().map_or(1.0, |(t, device)| t.M11 * device);
        let (left, top) = self.on_pixel(x, y);
        self.fill(rule_ink(inks), left, top + (tall / 2.0).floor() / per, width, 1.0 / per);
    }

    /// Draws `text` with its first baseline-box at `(x, y)`, in a box `width` wide.
    /// As `text`, its glyphs where they fall, not put on whole pixels: words moving by less than a pixel a frame
    /// (put on whole pixels, they stood still, then jumped).
    pub fn text_moving(&self, text: &str, font: Font, color: Color, x: f32, y: f32, width: f32, align: Align) {
        let layout = self.gfx.layout(text, font, width, align);
        unsafe {
            self.dc.DrawTextLayout(
                windows_numerics::Vector2 { X: x, Y: y },
                &layout,
                self.brush(color),
                text_options(windows::Win32::Graphics::Direct2D::D2D1_DRAW_TEXT_OPTIONS_NO_SNAP),
            )
        };
    }

    /// Draws `text` with its first baseline-box at `(x, y)`, put on a whole pixel (still text: drawn between two,
    /// its stems were spread over both, and it looked softer than the same words elsewhere), in a box `width` wide.
    /// Its baseline is what is put on the pixel, once: the top put on one, the baseline below it (Direct2D puts it
    /// on one too) came to a pixel again, and the words stood up to a pixel and a half from where they were placed.
    pub fn text(&self, text: &str, font: Font, color: Color, x: f32, y: f32, width: f32, align: Align) {
        let layout = self.gfx.layout(text, font, width, align);
        let baseline = first_baseline(&layout);
        let (x, line) = self.on_pixel(x, y + baseline);
        let y = line - baseline;
        unsafe {
            self.dc.DrawTextLayout(
                windows_numerics::Vector2 { X: x, Y: y },
                &layout,
                self.brush(color),
                D2D1_DRAW_TEXT_OPTIONS_CLIP | self.text_snap(),
            )
        };
    }
}

/// Where the first line's baseline of `layout` falls below its top (DIPs), as the faces that draw it set it.
fn first_baseline(layout: &IDWriteTextLayout) -> f32 {
    // (A list too short for all its lines is not filled: one of more lines is asked again with room for them all.)
    let (mut count, mut line) = (0, [DWRITE_LINE_METRICS::default()]);
    if unsafe { layout.GetLineMetrics(Some(&mut line), &mut count) }.is_ok() {
        return line[0].baseline;
    }
    let mut lines = vec![DWRITE_LINE_METRICS::default(); count.max(1) as usize];
    let _ = unsafe { layout.GetLineMetrics(Some(&mut lines), &mut count) };
    lines[0].baseline
}

/// `options` for drawing text, with its glyphs in their own colours where a font has them (an emoji's): without it,
/// drawn in the text's one colour, as outlines.
fn text_options(options: windows::Win32::Graphics::Direct2D::D2D1_DRAW_TEXT_OPTIONS) -> windows::Win32::Graphics::Direct2D::D2D1_DRAW_TEXT_OPTIONS {
    options | windows::Win32::Graphics::Direct2D::D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT
}

/// What stands in a note's words for what of them is put away (see `Frame::markdown`): room `width` wide (none for a
/// mark hidden), drawn as nothing, and no taller than nothing (its line as tall as its words make it).
#[windows::core::implement(IDWriteInlineObject)]
struct Room {
    width: f32,
}

impl IDWriteInlineObject_Impl for Room_Impl {
    fn Draw(&self, _: *const core::ffi::c_void, _: windows::core::Ref<windows::Win32::Graphics::DirectWrite::IDWriteTextRenderer>, _: f32, _: f32, _: windows::core::BOOL, _: windows::core::BOOL, _: windows::core::Ref<windows::core::IUnknown>) -> Result<()> {
        Ok(())
    }

    fn GetMetrics(&self) -> Result<windows::Win32::Graphics::DirectWrite::DWRITE_INLINE_OBJECT_METRICS> {
        Ok(windows::Win32::Graphics::DirectWrite::DWRITE_INLINE_OBJECT_METRICS { width: self.width, height: 0.0, baseline: 0.0, supportsSideways: false.into() })
    }

    fn GetOverhangMetrics(&self) -> Result<windows::Win32::Graphics::DirectWrite::DWRITE_OVERHANG_METRICS> {
        Ok(Default::default())
    }

    fn GetBreakConditions(&self, before: *mut windows::Win32::Graphics::DirectWrite::DWRITE_BREAK_CONDITION, after: *mut windows::Win32::Graphics::DirectWrite::DWRITE_BREAK_CONDITION) -> Result<()> {
        unsafe {
            *before = windows::Win32::Graphics::DirectWrite::DWRITE_BREAK_CONDITION_NEUTRAL;
            *after = windows::Win32::Graphics::DirectWrite::DWRITE_BREAK_CONDITION_NEUTRAL;
        }
        Ok(())
    }
}

/// What of a note's words, `words`, its faces have no glyphs for, and the face the system falls back on for each: its
/// words in the face `face`, those in `code` (UTF-16 units) in the face `code_face`, all of the system's. Both the text
/// services and a note's face are given these faces (the services, their font binding turned off, drew boxes, as for
/// Chinese in a block of code): what is drawn written in and not is drawn alike.
pub fn fallen_back(words: &[u16], code: &[std::ops::Range<usize>], face: &str, code_face: &str) -> Vec<(std::ops::Range<usize>, String)> {
    use windows::Win32::Graphics::DirectWrite::{IDWriteFactory2, IDWriteFont, IDWriteTextAnalysisSource, DWRITE_FONT_WEIGHT_NORMAL};
    let mut fallen = Vec::new();
    let Ok(factory) = (unsafe { DWriteCreateFactory::<IDWriteFactory2>(DWRITE_FACTORY_TYPE_SHARED) }) else { return fallen };
    let Ok(fallback) = (unsafe { factory.GetSystemFontFallback() }) else { return fallen };
    // (The faces asked of among the system's: else none is, and every word is given the one fallen back on.)
    let mut system = None;
    if unsafe { factory.GetSystemFontCollection(&mut system, false) }.is_err() {
        return fallen;
    }
    let source: IDWriteTextAnalysisSource = Words { words: words.to_vec(), locale: "zh-cn\0".encode_utf16().collect() }.into();
    // (Each stretch in one face asked of on its own: from where it begins, how much of it the face, or the one fallen
    // back on, has.)
    let in_code = |at: usize| code.iter().any(|range| range.contains(&at));
    let mut start = 0;
    while start < words.len() {
        let coded = in_code(start);
        let end = (start..words.len()).find(|&at| in_code(at) != coded).unwrap_or(words.len());
        let base = if coded { code_face } else { face };
        let family = HSTRING::from(base);
        let mut at = start;
        while at < end {
            let (mut length, mut font, mut scale): (u32, Option<IDWriteFont>, f32) = (0, None, 1.0);
            let asked = unsafe { fallback.MapCharacters(&source, at as u32, (end - at) as u32, system.as_ref(), &family, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, &mut length, &mut font, &mut scale) };
            if asked.is_err() || length == 0 {
                break;
            }
            let name = font.and_then(|font| family_name(&font));
            if let Some(name) = name.filter(|name| !name.eq_ignore_ascii_case(base)) {
                fallen.push((at..at + length as usize, name));
            }
            at += length as usize;
        }
        start = end;
    }
    fallen
}

/// The name of `font`'s family, as the system's fonts know it (in English, else its first).
fn family_name(font: &windows::Win32::Graphics::DirectWrite::IDWriteFont) -> Option<String> {
    let names = unsafe { font.GetFontFamily().and_then(|family| family.GetFamilyNames()).ok()? };
    let (mut index, mut found) = (0, windows::core::BOOL(0));
    unsafe { names.FindLocaleName(w!("en-us"), &mut index, &mut found).ok()? };
    let index = if found.as_bool() { index } else { 0 };
    let length = unsafe { names.GetStringLength(index).ok()? };
    let mut name = vec![0u16; length as usize + 1];
    unsafe { names.GetString(index, &mut name).ok()? };
    Some(String::from_utf16_lossy(&name[..length as usize]))
}

/// A note's words, as DirectWrite asks for them to find the faces that have them (see `fallen_back`): all of them one
/// paragraph, left to right, in the locale `locale` (ended by a nul).
#[windows::core::implement(windows::Win32::Graphics::DirectWrite::IDWriteTextAnalysisSource)]
struct Words {
    words: Vec<u16>,
    locale: Vec<u16>,
}

impl windows::Win32::Graphics::DirectWrite::IDWriteTextAnalysisSource_Impl for Words_Impl {
    fn GetTextAtPosition(&self, at: u32, text: *mut *mut u16, length: *mut u32) -> Result<()> {
        let at = (at as usize).min(self.words.len());
        unsafe {
            *text = if at < self.words.len() { self.words.as_ptr().add(at).cast_mut() } else { std::ptr::null_mut() };
            *length = (self.words.len() - at) as u32;
        }
        Ok(())
    }

    fn GetTextBeforePosition(&self, at: u32, text: *mut *mut u16, length: *mut u32) -> Result<()> {
        let at = (at as usize).min(self.words.len());
        unsafe {
            *text = if at > 0 { self.words.as_ptr().cast_mut() } else { std::ptr::null_mut() };
            *length = at as u32;
        }
        Ok(())
    }

    fn GetParagraphReadingDirection(&self) -> windows::Win32::Graphics::DirectWrite::DWRITE_READING_DIRECTION {
        windows::Win32::Graphics::DirectWrite::DWRITE_READING_DIRECTION_LEFT_TO_RIGHT
    }

    fn GetLocaleName(&self, at: u32, length: *mut u32, locale: *mut *mut u16) -> Result<()> {
        unsafe {
            *length = (self.words.len() as u32).saturating_sub(at);
            *locale = self.locale.as_ptr().cast_mut();
        }
        Ok(())
    }

    fn GetNumberSubstitution(&self, at: u32, length: *mut u32, substitution: windows::core::OutRef<windows::Win32::Graphics::DirectWrite::IDWriteNumberSubstitution>) -> Result<()> {
        unsafe { *length = (self.words.len() as u32).saturating_sub(at) };
        substitution.write(None)
    }
}

/// The ink a note's tables' lines and its rules are drawn in, of its `inks`: its faint, fainter.
fn rule_ink(inks: Inks) -> Color {
    Color { a: 0.5, ..inks.faint }
}

/// How strongly a note's table's head's row is tinted in its faint ink (see `Frame::table`).
const HEAD_TINT: f32 = 0.08;

/// How much the bitmaps made from their keys alone may hold (bytes; see `Gfx::made`): a glass panel's on a 4K screen
/// at 150 %, about 13 MB.
const MADE_BUDGET: usize = 32 << 20;

/// How long no window draws before what drawing holds is given back (see `Gfx::trim_when_idle`): longer than
/// anything shown waits between frames (a widget draws each sample, a second apart).
const IDLE: Duration = Duration::from_secs(3);

/// How often the thread's timer looks whether giving back what drawing holds is due (see `Gfx::tend`).
const TEND_EVERY: Duration = Duration::from_secs(1);

/// The thread's timer (see `Gfx::tend`): what is due given back, and the timer stopped once nothing more is.
unsafe extern "system" fn tended(_: HWND, _: u32, timer: usize, _: u32) {
    // (A device lost and made anew has a timer of its own, if any.)
    let Some(gfx) = CURRENT.with(|cell| cell.borrow().clone()).filter(|gfx| gfx.tending.get() == timer) else {
        let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::KillTimer(None, timer) };
        return;
    };
    gfx.let_go_once_settled();
    gfx.trim_when_idle();
    let idle_due = gfx.showing.get() == 0 && !gfx.trimmed.get();
    if gfx.settled.get() && !idle_due {
        let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::KillTimer(None, timer) };
        gfx.tending.set(0);
    }
}

/// How much of the graphics card's memory Direct2D keeps for its effects' textures (bytes; see `Gfx::new`): a few
/// of a large panel's.
const EFFECTS_KEPT: u64 = 96 << 20;

/// The step a surface's size is made in (physical px; see `room`).
const SURFACE_STEP: u32 = 128;

thread_local! {
    /// Whether what this thread draws now is being sized by hand (see `room`).
    static SIZING: Cell<bool> = const { Cell::new(false) };
}

/// What this thread draws while the returned guard is held is being sized by hand (a panel, a widget or the
/// settings held by their edges), or is not; once it is dropped, what is drawn is not (another window drawn on the
/// thread after it is drawn as it is).
#[must_use]
pub fn sizing(on: bool) -> Sizing {
    SIZING.with(|sizing| sizing.set(on));
    Sizing
}

/// Whether what this thread draws now is being sized by hand (see `sizing`).
pub fn sized_by_hand() -> bool {
    SIZING.with(Cell::get)
}

/// See `sizing`.
pub struct Sizing;

impl Drop for Sizing {
    fn drop(&mut self) {
        SIZING.with(|sizing| sizing.set(false));
    }
}

/// How much larger than asked room at rest may be (physical px each way), and the grain it is made in.
const AT_REST_SLACK: u32 = 64;
const AT_REST_GRAIN: u32 = 16;

/// Room for drawing that is sized as it is used (a window's surface, a layer, what is behind a window): whether
/// what was made `have` (physical px) still serves for `want`, and if not, how large to make it anew. While what is
/// drawn is sized by hand (see `sizing`), it serves while it holds `want` and is no more than twice as large, give
/// or take a step, each way, and is made anew with a quarter more than `want`, in steps: a window sized pixel by
/// pixel makes its room anew a few times, not at each size (each made anew is memory of the card's, given back
/// only a while after it is let go). At rest, it is as large as asked, near enough: a large window holds a dozen
/// such, and room to spare in each would be a third as much again of the card's memory.
pub fn room(have: Option<(u32, u32)>, want: (u32, u32)) -> Option<(u32, u32)> {
    let sizing = SIZING.with(Cell::get);
    let serves = |have: u32, want: u32| have >= want && if sizing { have <= 2 * want + SURFACE_STEP } else { have - want < AT_REST_SLACK };
    if have.is_some_and(|have| serves(have.0, want.0) && serves(have.1, want.1)) {
        return None;
    }
    let made = |want: u32| {
        if sizing {
            (want.max(1) + want / 4).div_ceil(SURFACE_STEP) * SURFACE_STEP
        } else {
            want.max(1).div_ceil(AT_REST_GRAIN) * AT_REST_GRAIN
        }
    };
    Some((made(want.0), made(want.1)))
}

/// A window's content: a composition surface the size of the window, or of
/// the part of it drawn in (see `offset`).
pub struct Surface {
    _target: IDCompositionTarget,
    visual: IDCompositionVisual2,
    surface: Option<IDCompositionSurface>,
    size: (u32, u32),
    /// Whether the window shows `surface` (one made anew is shown once it is first drawn on: see `draw`).
    shown: bool,
    /// Where in the window it is drawn (physical px), and where it was last.
    at: (i32, i32),
    placed: (i32, i32),
    /// Shown no further than the size it is drawn at, where its window is larger than that (see `clipped`): the size
    /// it was last cut to.
    clips: bool,
    clipped_to: (u32, u32),
    /// How long the last frame drawn on it took, part by part.
    pub took: Took,
}

/// A frame's time on a surface: till the compositor gave the surface to
/// draw on, what was drawn on it, and till it was handed back and the
/// compositor told.
#[derive(Clone, Copy, Default, Debug)]
pub struct Took {
    pub given: std::time::Duration,
    pub painted: std::time::Duration,
    pub handed: std::time::Duration,
}

impl Surface {
    pub fn new(gfx: &Gfx, hwnd: HWND) -> Result<Self> {
        let target = unsafe { gfx.dcomp.CreateTargetForHwnd(hwnd, true)? };
        let visual: IDCompositionVisual2 = unsafe { gfx.dcomp.CreateVisual()? };
        unsafe { target.SetRoot(&visual)? };
        Ok(Surface { _target: target, visual, surface: None, size: (0, 0), shown: false, at: (0, 0), placed: (0, 0), clips: false, clipped_to: (0, 0), took: Default::default() })
    }

    /// Shown no further than the size each frame is drawn at, for a window larger than what is drawn in it (the
    /// panel's): the room a surface is made with past that, and anything drawn into it, is not shown.
    pub fn clipped(mut self) -> Self {
        self.clips = true;
        self
    }

    /// Draws from now on at `at` (physical px) in its window, with the next
    /// frame: a window larger than what is drawn in it (the panel's) has only
    /// that much surface, and it is moved with what it shows, in the same
    /// frame of the compositor's.
    pub fn offset(&mut self, at: (i32, i32)) {
        self.at = at;
    }

    /// Draws one frame, at `scale` physical pixels per DIP, on a surface
    /// `size` physical pixels large.
    pub fn draw(&mut self, gfx: &Gfx, size: (u32, u32), scale: f32, paint: impl FnOnce(&Frame)) -> Result<()> {
        // Made with room to spare, and kept while it serves (see `room`): a
        // window sized frame by frame (a widget settling, sticking, sized)
        // draws on one surface, not a new one each frame. What lies past the
        // window is not shown.
        // (A surface made anew is shown only once it is drawn on: shown at once, anything committed meanwhile, by
        // this window or another on the device, showed it empty for a frame, and the window flashed out.)
        if let Some(made) = room(self.surface.as_ref().map(|_| self.size), size) {
            let surface = unsafe { gfx.dcomp.CreateSurface(made.0, made.1, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_ALPHA_MODE_PREMULTIPLIED)? };
            self.surface = Some(surface);
            self.size = made;
            self.shown = false;
        }
        if self.at != self.placed {
            unsafe {
                self.visual.SetOffsetX2(self.at.0 as f32)?;
                self.visual.SetOffsetY2(self.at.1 as f32)?;
            }
            self.placed = self.at;
        }
        if self.clips && self.clipped_to != size {
            let clip = D2D_RECT_F { left: 0.0, top: 0.0, right: size.0 as f32, bottom: size.1 as f32 };
            unsafe { self.visual.SetClip2(&clip)? };
            self.clipped_to = size;
        }
        gfx.last_drawn.set(Some(Instant::now()));
        gfx.trimmed.set(false);
        let surface = self.surface.as_ref().unwrap();
        let began = std::time::Instant::now();
        // Drawn apart first, and put on the surface only once drawn whole (see `Frame::apart`): failing, the surface
        // is not touched, and shows what it showed.
        let mut given = std::time::Duration::ZERO;
        let frame = Frame::whole(gfx, self.size, scale, |frame| {
            given = began.elapsed();
            paint(frame);
        })?;
        let painted = began.elapsed();
        let mut offset = POINT::default();
        let dc: ID2D1DeviceContext = unsafe { surface.BeginDraw(None, &mut offset)? };
        // (The surface's drawing ended whatever became of the putting: left begun, nothing more was drawn on it.)
        let put = frame.put(&dc, offset);
        let ended = unsafe { surface.EndDraw() };
        put?;
        ended?;
        unsafe {
            if !self.shown {
                self.visual.SetContent(surface)?;
                self.shown = true;
            }
            gfx.dcomp.Commit()?;
        }
        self.took = Took { given, painted: painted - given, handed: began.elapsed() - painted };
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

/// The whole pixel `at` (physical px) is put on: the nearest, a half going up; first taken to a 64th of a pixel,
/// so that what is drawn at the same place in a window comes to the same pixel wherever in the compositor's atlas
/// the frame is drawn (its place there, in DIPs, is rarely exact: 355 px at 1.5 is 236.666…, and a place half
/// way between two pixels went to one or the other as the frames went, the text shaking by a pixel).
fn whole_pixel(at: f32) -> f32 {
    ((at * 64.0).round() / 64.0 + 0.5).floor()
}

/// `transform` (counted in DIPs, `k` physical pixels a DIP each way) counted in physical pixels: what it grows and
/// turns by kept, its place as many pixels away.
fn in_pixels(transform: Matrix3x2, k: (f32, f32)) -> Matrix3x2 {
    Matrix3x2 { M11: transform.M11, M12: transform.M12 * k.1 / k.0, M21: transform.M21 * k.0 / k.1, M22: transform.M22, M31: transform.M31 * k.0, M32: transform.M32 * k.1 }
}

/// How much larger `transform` makes what it draws, on average each way (turned or not: its first entry alone shrank a
/// turned shadow's blur to nothing as the record's arm swung).
fn stretch(transform: &Matrix3x2) -> f32 {
    (transform.M11 * transform.M22 - transform.M12 * transform.M21).abs().sqrt().max(1e-3)
}

/// The faint one-unit glow `halo` rings what `drawn` draws with (as CSS's `text-shadow: 0 0 1px`; see `Layer::draw`).
unsafe fn glow(dc: &ID2D1DeviceContext, drawn: &windows::Win32::Graphics::Direct2D::ID2D1CommandList, halo: Color) -> Result<ID2D1Image> {
    let glow = dc.CreateEffect(&CLSID_D2D1Shadow)?;
    glow.SetInput(0, drawn, true);
    glow.SetValue(D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &0.5f32.to_ne_bytes())?;
    let rgba = [halo.r, halo.g, halo.b, halo.a];
    glow.SetValue(D2D1_SHADOW_PROP_COLOR.0 as u32, D2D1_PROPERTY_TYPE_VECTOR4, std::slice::from_raw_parts(rgba.as_ptr().cast(), 16))?;
    glow.GetOutput()
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
        // Being sized by hand (see `sizing`), what it shows changes each frame, and keeping it keeps nothing: it is
        // drawn where it shows, and its bitmap let go (a window's size of the card's memory, each layer), to be made
        // again at the size it comes to rest at. What shows as it did (the settings' library, while the window
        // around it is sized) is drawn from what is kept, as at rest.
        let kept = self.bitmap.as_ref().is_some_and(|(shown, _)| *shown == key);
        if SIZING.with(Cell::get) && !kept {
            self.bitmap = None;
            return unsafe { Self::draw_through(frame, local, area, halo, paint) };
        }
        if self.bitmap.as_ref().is_none_or(|(shown, _)| *shown != key) {
            let pixels = D2D_SIZE_U { width: (size.0 * scale).ceil() as u32, height: (size.1 * scale).ceil() as u32 };
            // The same pixels serve while they serve (see `room`): a layer
            // sized frame by frame keeps one bitmap's memory, not one for
            // each size; at another scale (a panel sized is drawn larger),
            // the same memory is taken at the new one.
            let had = self.bitmap.take().map(|(_, bitmap)| bitmap);
            let made = room(had.as_ref().map(|bitmap| unsafe { bitmap.GetPixelSize() }).map(|have| (have.width, have.height)), (pixels.width, pixels.height));
            let reusable = had.filter(|_| made.is_none());
            let pixels = made.map_or(pixels, |(width, height)| D2D_SIZE_U { width, height });
            let properties = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0 * scale,
                dpiY: 96.0 * scale,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
                ..Default::default()
            };
            let bitmap = match reusable {
                Some(bitmap) => {
                    let mut dpi = (0.0, 0.0);
                    unsafe { bitmap.GetDpi(&mut dpi.0, &mut dpi.1) };
                    if dpi.0 == 96.0 * scale {
                        bitmap
                    } else {
                        unsafe { dc.CreateBitmapFromDxgiSurface(&bitmap.GetSurface()?, Some(&properties))? }
                    }
                }
                None => {
                    unsafe { dc.CreateBitmap(pixels, None, 0, &properties)? }
                }
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
                // Ringed by a halo, the halo is painted in with it, once (not each time the layer is drawn): what is
                // drawn recorded, then drawn over its glow.
                let painted = match halo {
                    None => paint(frame),
                    Some(color) => {
                        let list = dc.CreateCommandList()?;
                        dc.SetTarget(&list);
                        // (Setting a target can change the context's DPI: the layer's, as it is painted at.)
                        dc.SetDpi(96.0 * scale, 96.0 * scale);
                        let painted = paint(frame);
                        list.Close()?;
                        dc.SetTarget(&bitmap);
                        // (Recorded through the layer's own place already: drawn back as it is, not moved by it again.)
                        dc.SetTransform(&Matrix3x2::identity());
                        dc.DrawImage(&glow(dc, &list, color)?, None, None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
                        dc.DrawImage(&list, None, None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
                        painted
                    }
                };
                dc.PopAxisAlignedClip();
                dc.SetTarget(&target);
                dc.SetDpi(dpi.0, dpi.1);
                dc.SetTransform(&transform);
                painted?;
            }
            self.bitmap = Some((key, bitmap));
            frame.gfx.last_painted.set(Some(Instant::now()));
            frame.gfx.settled.set(false);
            frame.gfx.tend();
        }
        let image: ID2D1Image = self.bitmap.as_ref().unwrap().1.cast()?;
        let at = windows_numerics::Vector2 { X: left, Y: top };
        let mut parent = Matrix3x2::default();
        unsafe { dc.GetTransform(&mut parent) };
        unsafe {
            let mut placed = local * parent;
            // Drawn pixel for pixel (its pixels as many to a DIP where it lands as it was kept at: moved only, or made
            // smaller or larger as it was kept, as the settings' previews are), it is put on whole pixels: a bitmap put
            // between them is resampled, and what is in it (its text) blurred. (A widget sized by its left or top
            // edge keeps what it shows to its right and bottom ones: rarely on a whole pixel.)
            let mut dpi = (0.0, 0.0);
            dc.GetDpi(&mut dpi.0, &mut dpi.1);
            let device = dpi.0 / 96.0;
            let pixel_for_pixel = placed.M12 == 0.0 && placed.M21 == 0.0 && (placed.M11 * device - scale).abs() < 1e-4 && (placed.M22 * device - scale).abs() < 1e-4;
            if pixel_for_pixel {
                placed.M31 = whole_pixel((left * placed.M11 + placed.M31) * device) / device - left * placed.M11;
                placed.M32 = whole_pixel((top * placed.M22 + placed.M32) * device) / device - top * placed.M22;
            }
            dc.SetTransform(&placed);
            dc.DrawImage(&image, Some(&at), None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
            dc.SetTransform(&parent);
        }
        Ok(())
    }

    /// As `draw`, kept nowhere: drawn where it shows, through what `paint` draws recorded (a command list, not a
    /// bitmap) when it is ringed by a `halo`.
    unsafe fn draw_through(frame: &Frame, local: Matrix3x2, area: (f32, f32, f32, f32), halo: Option<Color>, paint: impl FnOnce(&Frame) -> Result<()>) -> Result<()> {
        let dc = &frame.dc;
        let (left, top, width, height) = area;
        let mut parent = Matrix3x2::default();
        dc.GetTransform(&mut parent);
        let placed = local * parent;
        let painted = match halo {
            None => {
                dc.SetTransform(&placed);
                dc.PushAxisAlignedClip(&rect(left, top, width, height), D2D1_ANTIALIAS_MODE_ALIASED);
                let painted = paint(frame);
                dc.PopAxisAlignedClip();
                painted
            }
            Some(color) => {
                let list = dc.CreateCommandList()?;
                let target = dc.GetTarget()?;
                // (Setting a target can change the context's DPI; it is put back.)
                let mut dpi = (0.0, 0.0);
                dc.GetDpi(&mut dpi.0, &mut dpi.1);
                dc.SetTarget(&list);
                dc.SetDpi(dpi.0, dpi.1);
                // (Recorded through where it is drawn, so put on the pixels it lands on, as what is drawn straight is;
                // drawn back as it is.)
                dc.SetTransform(&placed);
                dc.PushAxisAlignedClip(&rect(left, top, width, height), D2D1_ANTIALIAS_MODE_ALIASED);
                let painted = paint(frame);
                dc.PopAxisAlignedClip();
                list.Close()?;
                dc.SetTarget(&target);
                dc.SetDpi(dpi.0, dpi.1);
                dc.SetTransform(&Matrix3x2::identity());
                dc.DrawImage(&glow(dc, &list, color)?, None, None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
                dc.DrawImage(&list, None, None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
                dc.SetTransform(&placed);
                painted
            }
        };
        dc.SetTransform(&parent);
        painted
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

/// (Tests: this process's threads now, for what a device leaves running.)
#[cfg(test)]
pub(crate) fn threads_now() -> usize {
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32};
    let pid = std::process::id();
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }.unwrap();
    let mut entry = THREADENTRY32 { dwSize: size_of::<THREADENTRY32>() as u32, ..Default::default() };
    let mut count = 0;
    let mut more = unsafe { Thread32First(snap, &mut entry) }.is_ok();
    while more {
        count += usize::from(entry.th32OwnerProcessID == pid);
        more = unsafe { Thread32Next(snap, &mut entry) }.is_ok();
    }
    let _ = unsafe { windows::Win32::Foundation::CloseHandle(snap) };
    count
}

#[cfg(test)]
mod tests {
    use super::room;

    /// A line of words set in a box stands with its ink across the middle of the box as it is drawn, at any scale and
    /// wherever on a pixel the box falls, to under a pixel: half a pixel, the most a baseline put on a whole pixel can
    /// be off, and what the face's hinting moves its strokes by to fit them to pixels. Its top put on a pixel, then
    /// its baseline again, and centred on the middle asked rather than the box drawn, it stood up to a pixel and a
    /// half off (the picker's buttons, high so on a screen at 150% with the panel made smaller).
    #[test]
    fn words_stand_in_the_middle_of_their_box_as_drawn() {
        use crate::ui::canvas::{Align, Canvas, Color};
        let _device = super::held().unwrap();
        let gfx = super::Gfx::new().expect("graphics");
        let theme = crate::ui::theme::Theme::new(crate::ui::theme::Skin::Glass, true);
        let font = crate::ui::canvas::Font { size: 13.0, weight: 400.0, ..theme.body };
        for scale in [1.0f32, 1.1, 1.143, 1.2, 1.25, 1.3, 1.4, 1.5, 1.6, 1.75, 1.9, 2.0, 2.25, 2.5] {
            for text in ["中8", "今天"] {
                for quarter in 0..4 {
                    let (w, h) = ((120.0 * scale).ceil() as usize, (40.0 * scale).ceil() as usize);
                    let top = 6.0 + quarter as f32 * 0.25 / scale;
                    let draw = |words: bool| {
                        gfx.draw_offscreen((w as u32, h as u32), scale, |frame| {
                            frame.fill_rounded(Color::hex(0x404040, 1.0), 4.0, top, 100.0, 28.0, 8.0);
                            if words {
                                frame.text(text, font, Color::hex(0xFFFFFF, 1.0), 10.0, frame.line_in(font, top, 28.0), 100.0, Align::Start);
                            }
                        })
                        .expect("drawn")
                    };
                    let (with, without) = (draw(true), draw(false));
                    let ground: Vec<usize> = (0..h).filter(|row| without[(row * w + (50.0 * scale) as usize) * 4 + 3] > 0).collect();
                    // How much of each row the words' ink covers at most (its strokes across, partly in a row at
                    // their edge): their top and foot found to a part of a pixel.
                    let covered: Vec<f32> = (0..h).map(|row| (0..w).map(|column| with[(row * w + column) * 4].abs_diff(without[(row * w + column) * 4]) as f32 / 255.0).fold(0.0, f32::max)).collect();
                    let inked: Vec<usize> = (0..h).filter(|row| covered[*row] > 0.05).collect();
                    let (first, last) = (inked[0], inked[inked.len() - 1]);
                    let ink = (first as f32 + 1.0 - covered[first] + last as f32 + covered[last]) / 2.0;
                    let off = ink - (ground[0] + ground[ground.len() - 1] + 1) as f32 / 2.0;
                    assert!(off.abs() <= 0.85, "{text} at {scale}, {quarter}/4 of a pixel down: {off:+.2} px off the box's middle");
                }
            }
        }
    }

    /// What is drawn counted in pixels (a note's words written in where it is) comes where the same drawn in DIPs
    /// would, grown and turned as it would be: a note lifted and leaning had its words drawn as large and as straight
    /// as at rest, its page lifted and leaning round them.
    #[test]
    fn a_place_counted_in_pixels_grows_and_turns_as_in_dips() {
        use windows_numerics::{Matrix3x2, Vector2};
        let k = (1.5f32, 1.25f32);
        let dips = Matrix3x2::translation(-30.0, -10.0) * Matrix3x2::scale(1.04, 1.04) * Matrix3x2::rotation_around(4.0, Vector2 { X: 0.0, Y: 0.0 }) * Matrix3x2::translation(42.0, 17.0);
        let pixels = super::in_pixels(dips, k);
        let through = |m: &Matrix3x2, (x, y): (f32, f32)| (x * m.M11 + y * m.M21 + m.M31, x * m.M12 + y * m.M22 + m.M32);
        for (x, y) in [(0.0f32, 0.0f32), (100.0, 0.0), (0.0, 60.0), (73.0, 41.0)] {
            let (dx, dy) = through(&dips, (x, y));
            let (px, py) = through(&pixels, (x * k.0, y * k.1));
            assert!((px - dx * k.0).abs() < 1e-3 && (py - dy * k.1).abs() < 1e-3, "({x}, {y}): {px},{py} for {},{}", dx * k.0, dy * k.1);
        }
    }

    /// A frame something of which failed as it was drawn is not had to be shown: what it was for shows what it
    /// showed. (Drawn straight onto its surface, a panel sized on a card short of memory showed its ground with a
    /// reading or two on it.)
    /// A note's words drawn as its face (see `Frame::markdown`) with a tab in them: what follows it as far on as the
    /// text services set it as the note is written in (`editor::TAB_PIXELS`, in the pixels drawn in, at any scale).
    #[test]
    #[cfg(feature = "studio")]
    fn a_note_s_tabs_are_as_far_apart_as_its_editor_s() {
        let _device = crate::ui::gfx::held().unwrap();
        use super::super::canvas::{Color, Family, Font, Inks};
        let gfx = super::Gfx::new().expect("graphics");
        let white = Color::hex(0xffffff, 1.0);
        let inks = Inks { words: white, faint: white, accent: white };
        for scale in [1.0f32, 1.5, 2.0] {
            let (wide, tall) = ((200.0 * scale) as u32, (40.0 * scale) as u32);
            let first = |text: &str| {
                let pixels = gfx.draw_offscreen((wide, tall), scale, |frame| frame.markdown(text, Font::new(Family::Segoe, 15.0, 400.0), inks, 0.0, 0.0, 200.0, 40.0, 0.0)).expect("drawn");
                (0..wide).find(|&x| (0..tall).any(|y| pixels[((y * wide + x) * 4 + 3) as usize] > 0)).expect("words drawn") as f32
            };
            let (alone, tabbed) = (first("X"), first("	X"));
            assert!((tabbed - alone - super::super::editor::TAB_PIXELS).abs() <= 1.0, "at {scale}: after a tab at {tabbed}, alone at {alone}");
        }
    }

    /// A note's words drawn as its face with a line too long for its room: that line wrapped onto rows enough, the
    /// lines after it drawn under them, and nothing failed. (A note stuck on the desktop showed its lines only as far as
    /// the first one that wrapped, and its frame was lost.)
    #[test]
    #[cfg(feature = "studio")]
    fn a_note_s_line_too_long_for_its_room_wraps_and_what_follows_is_drawn() {
        use super::super::canvas::{Color, Family, Font, Inks};
        let gfx = super::Gfx::new().expect("graphics");
        let white = Color::hex(0xffffff, 1.0);
        let inks = Inks { words: white, faint: white, accent: white };
        let (wide, tall) = (200u32, 300u32);
        let long = "English first 你好世界我们再见 then English again and on";
        for text in [format!("first\n{long}\nlast\n"), format!("# Head\n- [ ] {long}\nlast")] {
            let mut failed = None;
            let pixels = gfx
                .draw_offscreen((wide, tall), 1.0, |frame| {
                    frame.markdown(&text, Font::new(Family::YaHei, 15.0, 400.0), inks, 0.0, 0.0, wide as f32, tall as f32, 0.0);
                    failed = frame.failed.borrow().clone();
                })
                .expect("drawn");
            assert!(failed.is_none(), "{text:?}: {failed:?}");
            // Rows with ink, one under another: the first line, the long one on two or more, the last.
            let inked = |y: u32| (0..wide).any(|x| pixels[((y * wide + x) * 4 + 3) as usize] > 0);
            let rows = (0..tall).filter(|&y| inked(y) && (y == 0 || !inked(y - 1))).count();
            assert!(rows >= 4, "{text:?}: {rows} rows drawn");
        }
    }

    /// An emoji is drawn in its own colours however text is drawn (still or moving, in a line, wrapped, in a note's
    /// words): drawn in the text's one colour, it was an outline.
    #[test]
    #[cfg(feature = "studio")]
    fn an_emoji_is_drawn_in_its_colours() {
        use super::super::canvas::{Align, Canvas, Color, Family, Font, Inks};
        let gfx = super::Gfx::new().expect("graphics");
        let white = Color::hex(0xffffff, 1.0);
        let font = Font::new(Family::YaHei, 24.0, 400.0);
        let size = 48u32;
        let in_colour = |way: &str, paint: &dyn Fn(&super::Frame)| {
            let pixels = gfx.draw_offscreen((size, size), 1.0, paint).expect("drawn");
            let coloured = pixels.chunks(4).filter(|p| p[3] > 128 && p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2]) > 60).count();
            assert!(coloured > 20, "{way}: {coloured} pixels in colour");
        };
        in_colour("text", &|frame| Canvas::text(frame, "😂", font, white, 4.0, 4.0, 40.0, Align::Start));
        in_colour("moving", &|frame| frame.text_moving("😂", font, white, 4.0, 4.0, 40.0, Align::Start));
        in_colour("moving_while", &|frame| {
            let _moving = frame.moving_while(true);
            Canvas::text(frame, "😂", font, white, 4.0, 4.0, 40.0, Align::Start);
        });
        in_colour("wrapped", &|frame| frame.text_wrapped("😂", font, white, 4.0, 4.0, 40.0));
        in_colour("a note's", &|frame| frame.markdown("😂", font, Inks { words: white, faint: white, accent: white }, 4.0, 4.0, 40.0, 40.0, 0.0));
    }

    /// A layer drawn while a window is sized by hand is painted again only where what it shows has changed: one that
    /// shows as it did (the settings' library, as the window around it is sized) is drawn from what it kept, one that
    /// changed is drawn straight where it shows, and none is kept for it till the sizing ends.
    #[test]
    fn a_layer_sized_by_hand_is_painted_again_only_as_it_changes() {
        let _device = crate::ui::gfx::held().unwrap();
        let gfx = super::Gfx::new().expect("graphics");
        let mut layer = super::Layer::default();
        let painted = std::cell::Cell::new(0);
        let mut draw = |key: u64| {
            super::Frame::whole(&gfx, (32, 32), 1.0, |frame| {
                layer.draw(frame, key, windows_numerics::Matrix3x2::identity(), (0.0, 0.0, 16.0, 16.0), 1.0, None, |_| {
                    painted.set(painted.get() + 1);
                    Ok(())
                })
                .expect("drawn");
            })
            .expect("frame");
            painted.get()
        };
        assert_eq!(draw(1), 1);
        {
            let _sizing = super::sizing(true);
            assert_eq!(draw(1), 1, "as it was: drawn from what was kept");
            assert_eq!(draw(2), 2, "changed: painted where it shows");
            assert_eq!(draw(2), 3, "nothing kept while sized");
        }
        assert_eq!(draw(2), 4);
        assert_eq!(draw(2), 4);
    }

    #[test]
    fn a_frame_failed_is_not_shown() {
        let _device = crate::ui::gfx::held().unwrap();
        use super::Canvas;
        let gfx = super::Gfx::new().expect("graphics");
        let failed = super::Frame::whole(&gfx, (8, 8), 1.0, |frame| {
            frame.fill(super::Color::hex(0xFF0000, 1.0), 0.0, 0.0, 8.0, 8.0);
            frame.fail(windows::Win32::Foundation::E_FAIL.into());
        });
        assert!(failed.is_err());
        // (One not failed, drawn on the same bitmap after it, as ever.)
        assert!(super::Frame::whole(&gfx, (8, 8), 1.0, |_| {}).is_ok());
    }

    /// Windows' software renderer is drawn on only where there is no graphics card to draw on at all; a card that
    /// failed for a moment (as it is reset) is asked again, not given up for good.
    #[test]
    fn software_only_without_a_card() {
        use windows::Win32::Foundation::{E_FAIL, E_OUTOFMEMORY};
        use windows::Win32::Graphics::Dxgi::{DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET, DXGI_ERROR_UNSUPPORTED};
        assert!(super::software_instead(DXGI_ERROR_UNSUPPORTED));
        for passing in [DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET, E_FAIL, E_OUTOFMEMORY] {
            assert!(!super::software_instead(passing), "{passing:?}");
        }
    }

    /// A device made, drawn with, and let go of leaves nothing of itself running: the driver's threads it started
    /// end with it.
    #[test]
    #[ignore = "counts the graphics driver's threads: alone in its process"]
    fn a_device_let_go_of_leaves_no_threads() {
        let _device = crate::ui::gfx::held().unwrap();
        let before = super::threads_now();
        let held: Vec<_> = (0..3).map(|_| super::Gfx::new().expect("graphics")).collect();
        println!("threads: {before} before, {} with three devices held", super::threads_now());
        drop(held);
        for _ in 0..5 {
            let gfx = super::Gfx::new().expect("graphics");
            assert!(super::Frame::whole(&gfx, (64, 64), 1.0, |frame| { use super::Canvas; frame.fill_rounded(super::Color::hex(0xFF0000, 1.0), 0.0, 0.0, 64.0, 64.0, 4.0) }).is_ok());
            drop(gfx);
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        let after = super::threads_now();
        println!("threads: {before} before, {after} after five devices");
        assert!(after <= before + 2, "{before} -> {after}");
    }

    /// A window's thread (as the settings' is) that drew on a device of its own and ended, letting go of it as it
    /// did, leaves nothing of the device running, however often that is done.
    #[test]
    #[ignore = "makes windows, and counts the graphics driver's threads: alone in its process"]
    fn a_windows_thread_lets_its_device_go() {
        let before = super::threads_now();
        for _ in 0..3 {
            std::thread::spawn(|| {
                use super::Canvas;
                unsafe {
                    let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
                }
                let gfx = super::current().expect("graphics");
                let window = crate::ui::window::Window::new().expect("a window");
                let mut surface = super::Surface::new(&gfx, window.hwnd).expect("a surface");
                surface.draw(&gfx, (200, 100), 1.0, |frame| frame.fill_rounded(super::Color::hex(0xFF0000, 1.0), 0.0, 0.0, 200.0, 100.0, 4.0)).expect("drawn");
                drop(surface);
                drop(window);
                drop(gfx);
                super::let_go();
            })
            .join()
            .unwrap();
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
        let after = super::threads_now();
        println!("threads: {before} before, {after} after three windows' threads");
        assert!(after <= before + 4, "{before} -> {after}");
    }

    /// The graphics card's memory this process uses now (bytes), as the driver counts it.
    fn card_memory(gfx: &super::Gfx) -> u64 {
        use windows::core::Interface;
        use windows::Win32::Graphics::Dxgi::{IDXGIAdapter3, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO};
        let dxgi: windows::Win32::Graphics::Dxgi::IDXGIDevice = gfx.d3d().unwrap().cast().unwrap();
        let adapter: IDXGIAdapter3 = unsafe { dxgi.GetAdapter() }.unwrap().cast().unwrap();
        let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
        unsafe { adapter.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut info) }.unwrap();
        info.CurrentUsage
    }

    /// Windows drawn whole screens large and let go of, again and again, on one device (widgets torn off and closed,
    /// the settings opened and closed): the card's memory held after each is the same, not more each time.
    #[test]
    #[ignore = "makes windows, and reads the graphics card's memory: alone in its process"]
    fn windows_let_go_of_take_no_more_each_time() {
        let _device = crate::ui::gfx::held().unwrap();
        use super::Canvas;
        let gfx = super::Gfx::new().expect("graphics");
        let mut held = Vec::new();
        for _ in 0..6 {
            let window = crate::ui::window::Window::new().expect("a window");
            let mut surface = super::Surface::new(&gfx, window.hwnd).expect("a surface");
            surface.draw(&gfx, (3000, 3000), 1.0, |frame| frame.fill_rounded(super::Color::hex(0xFF0000, 1.0), 0.0, 0.0, 3000.0, 3000.0, 4.0)).expect("drawn");
            drop(surface);
            drop(window);
            gfx.trim();
            std::thread::sleep(std::time::Duration::from_millis(300));
            held.push(card_memory(&gfx) >> 20);
        }
        println!("card memory after each (MB): {held:?}");
        assert!(held[5] <= held[1] + 4, "{held:?}");
    }

    /// A report says what Glance draws on: the adapter by name, and its driver's version.
    #[test]
    fn says_what_it_draws_on() {
        let _device = crate::ui::gfx::held().unwrap();
        let _gfx = super::Gfx::new().expect("graphics");
        let said = super::drawn_on().expect("not said");
        println!("{said}");
        assert!(!said.starts_with(' ') && !said.contains('\u{0}') && said.contains(" (driver ") && !said.contains("not said"), "{said}");
    }

    /// Drawn off screen at 100%, `size` pixels square: each pixel's alpha, rows top to bottom.
    #[cfg(feature = "studio")]
    fn alphas(size: u32, paint: impl FnOnce(&super::Frame)) -> Vec<Vec<u8>> {
        let gfx = super::Gfx::new().expect("graphics");
        let pixels = gfx.draw_offscreen((size, size), 1.0, paint).expect("drawn");
        pixels.chunks(size as usize * 4).map(|row| row.chunks(4).map(|pixel| pixel[3]).collect()).collect()
    }

    /// An outline's line covers whole pixels inside the box given as its outer edge, as a fill of that box does: not
    /// two pixels at half strength; and lines as wide come out as wide at any scale (a 1-DIP line at 125% was a pixel
    /// and a soft quarter).
    #[test]
    #[cfg(feature = "studio")]
    fn an_outline_covers_whole_pixels_inside_its_box() {
        use super::super::canvas::{Canvas, Color, Fill};
        for scale in [1.0f32, 1.25, 1.5] {
            let seen = alphas_at(32, scale, |frame| frame.stroke_rounded(Fill::Solid(Color::hex(0xffffff, 1.0)), 2.0, 2.0, 16.0, 16.0, 0.0, 1.0));
            let fill = alphas_at(32, scale, |frame| frame.fill(Color::hex(0xffffff, 1.0), 2.0, 2.0, 16.0, 16.0));
            let row = 10usize;
            let first = fill[row].iter().position(|a| *a > 0).unwrap();
            let last = fill[row].iter().rposition(|a| *a > 0).unwrap();
            let lit: Vec<usize> = (0..32).filter(|x| seen[row][*x] > 0).collect();
            // (A DIP's line, as many whole pixels as it rounds to: one at 100% and 125%, two at 150%.)
            let wide = super::whole_pixel(scale) as usize;
            let want: Vec<usize> = (first..first + wide).chain(last + 1 - wide..=last).collect();
            assert_eq!(lit, want, "at {scale}: the line's pixels {lit:?}, the fill's edges {first}, {last}");
            assert!(lit.iter().all(|x| seen[row][*x] == 255), "at {scale}: the line at full strength");
        }
    }

    /// What is put on a whole pixel comes to the same pixel of its surface wherever in the compositor's atlas the
    /// surface's region is (its place there, in DIPs, rarely exact: 355 px at 1.5 is 236.666…): a place half way
    /// between two pixels went to one or the other as the frames went, and the overlay's words shook by a pixel.
    #[test]
    #[cfg(feature = "studio")]
    fn whole_pixels_do_not_follow_the_atlas() {
        let _device = crate::ui::gfx::held().unwrap();
        let gfx = super::Gfx::new().expect("graphics");
        for scale in [1.25f32, 1.5, 1.75] {
            gfx.draw_offscreen((4, 4), scale, |frame| {
                // Places half way between two pixels of the surface, and others.
                for local in [1.0 / (2.0 * scale), 7.0 / (2.0 * scale), 0.3, 13.37] {
                    let landed: std::collections::BTreeSet<i64> = (0..800)
                        .map(|offset| {
                            let base = offset as f32 / scale;
                            frame.place(windows_numerics::Matrix3x2::translation(base, base));
                            let (x, _) = frame.on_pixel(local, local);
                            // (Which pixel of the surface's own region, in 64ths: the atlas's offset taken off.)
                            (((x + base) * scale - offset as f32) * 64.0).round() as i64
                        })
                        .collect();
                    assert_eq!(landed.len(), 1, "at {scale}, {local} DIPs went to {landed:?} (64ths of a pixel)");
                }
            })
            .expect("drawn");
        }
    }

    /// Bars as tall come out as many pixels tall wherever they are, where a DIP is not whole pixels (a 150% screen,
    /// the panel at 90%): the storage's two bars were a pixel apart in height.
    #[test]
    #[cfg(feature = "studio")]
    fn bars_as_tall_are_as_tall_anywhere() {
        use super::super::canvas::{Canvas, Color};
        for scale in [1.35f32, 1.125, 1.575] {
            let tall: std::collections::BTreeSet<usize> = [3.0f32, 3.2, 3.37, 3.5, 3.71, 3.9]
                .into_iter()
                .map(|y| {
                    let seen = alphas_at(24, scale, |frame| frame.fill_rounded(Color::hex(0xffffff, 1.0), 2.0, y, 10.0, 4.0, 2.0));
                    seen.iter().filter(|row| row[8] > 128).count()
                })
                .collect();
            assert_eq!(tall.len(), 1, "at {scale}, bars 4 DIPs tall came out {tall:?} pixels tall");
        }
    }

    /// A button's plate, its ring and its glyph, drawn about one centre, come out about one centre, wherever that is
    /// and at any scale (the ring and glyph were put on pixels by their boxes and the plate by its centre, and moved
    /// apart by up to a pixel).
    #[test]
    #[cfg(feature = "studio")]
    fn a_plate_its_ring_and_its_glyph_are_one_about_another() {
        use super::super::canvas::{Canvas, Color, Fill, Point};
        use super::super::icons::Icon;
        let middle = |seen: &[Vec<u8>]| {
            let (mut sx, mut sy, mut n) = (0.0f32, 0.0f32, 0.0f32);
            for (y, row) in seen.iter().enumerate() {
                for (x, a) in row.iter().enumerate() {
                    let a = *a as f32;
                    sx += x as f32 * a;
                    sy += y as f32 * a;
                    n += a;
                }
            }
            (sx / n, sy / n)
        };
        for scale in [1.0f32, 1.25, 1.5, 1.35] {
            for c in [20.0f32, 20.2, 20.4, 20.5, 20.75] {
                let centre = Point { x: c, y: c + 0.3 };
                let white = Color::hex(0xffffff, 1.0);
                let plate = middle(&alphas_at(64, scale, |frame| frame.fill_circle(white, centre, 11.0)));
                let ring = middle(&alphas_at(64, scale, |frame| frame.stroke_rounded(Fill::Solid(white), c - 11.0, c + 0.3 - 11.0, 22.0, 22.0, 11.0, 1.0)));
                let glyph = middle(&alphas_at(64, scale, |frame| frame.icon(Icon::Close, centre, 13.0, white)));
                for (name, at) in [("ring", ring), ("glyph", glyph)] {
                    assert!((at.0 - plate.0).abs() < 0.15 && (at.1 - plate.1).abs() < 0.15, "at {scale}, centre {c}: the {name}'s middle {at:?}, the plate's {plate:?}");
                }
            }
        }
    }

    /// As `alphas`, at `scale`.
    #[cfg(feature = "studio")]
    fn alphas_at(size: u32, scale: f32, paint: impl FnOnce(&super::Frame)) -> Vec<Vec<u8>> {
        let gfx = super::Gfx::new().expect("graphics");
        let pixels = gfx.draw_offscreen((size, size), scale, paint).expect("drawn");
        pixels.chunks(size as usize * 4).map(|row| row.chunks(4).map(|pixel| pixel[3]).collect()).collect()
    }

    /// A fill clipped to a box half a pixel off the grid covers what the same fill unclipped does at that edge (both
    /// put on whole pixels): nothing of what is under shows along it.
    #[test]
    #[cfg(feature = "studio")]
    fn a_clipped_fill_meets_its_clip() {
        use super::super::canvas::{Canvas, Color};
        let seen = alphas(16, |frame| {
            frame.clip(2.0, 2.0, 10.0, 7.5);
            frame.fill(Color::hex(0xffffff, 1.0), 2.0, 2.0, 10.0, 7.5);
            frame.unclip();
        });
        assert_eq!(seen[9][5], 255, "the fill's last row, inside the clip");
        assert_eq!(seen[10][5], 0, "past both");
    }

    #[test]
    fn room_is_made_anew_a_few_times_as_a_window_is_sized() {
        // Grown pixel by pixel from 300 to 2000 wide, then back: made anew a few times each way, not at each size.
        let sizing = super::sizing(true);
        let mut have = None;
        let mut made = 0;
        for width in (300..=2000).chain((300..2000).rev()) {
            if let Some(new) = room(have, (width, 400)) {
                have = Some(new);
                made += 1;
            }
            let (w, h) = have.unwrap();
            assert!(w >= width && h >= 400, "it holds what is asked");
            assert!(w <= 2 * width + 128, "it is never much larger than asked");
        }
        assert!(made <= 16, "made {made} times");
        // At rest: as large as asked, near enough.
        drop(sizing);
        let (w, h) = room(have, (1000, 400)).expect("made anew at rest");
        assert!(w >= 1000 && w < 1000 + 64 && h >= 400 && h < 400 + 64);
        assert_eq!(room(Some((w, h)), (1000, 400)), None);
    }

    /// What a note's faces have no glyphs for is given the face the system falls back on, and only that: Chinese in a
    /// block of code (its face has none), nothing in words its own face has.
    #[test]
    fn what_a_face_has_not_falls_back() {
        let words: Vec<u16> = "#include 你好 正文 words".encode_utf16().collect();
        let fallen = super::fallen_back(&words, std::slice::from_ref(&(0..11)), "Microsoft YaHei UI", super::super::canvas::CODE_FACE);
        assert_eq!(fallen.iter().map(|(range, _)| range.clone()).collect::<Vec<_>>(), vec![9..11]);
        assert!(!fallen[0].1.eq_ignore_ascii_case(super::super::canvas::CODE_FACE));
    }
}

