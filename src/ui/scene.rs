//! What is behind one of Glance's windows, as it is now, without Glance
//! hiding from captures: the windows behind it (the desktop's own among
//! them) are put together again, live, in a window of Glance's own that no
//! one sees, out of the system compositor's thumbnails of them, in their
//! order; and that window is captured (Windows.Graphics.Capture), a frame
//! each time something in it changes, none while nothing does.
//!
//! The window is cloaked (no one sees it), and owned, with no surface of
//! its own and no other extended style: the capture refuses to take a tool
//! window, one that is not to be activated, a disabled one, or one owned by
//! a tool window (on Windows 10 and on Windows 11 before 26300, measured).
//! It looks only as it is taken: its owner is made a tool window after, which
//! keeps the window out of the task switcher and the task view (which list
//! a window owned by one that is not shown, cloaked or not), and its frames
//! come as before. The thumbnails show what the
//! windows show, those of elevated programs too, and live (a video plays
//! on in them); not the shadows the system draws around windows.
//!
//! Where the desktop is drawn in windows of its own under the desktop's window
//! (a live wallpaper's, on Windows 11 24H2 and later, where they are its
//! children), a thumbnail of the desktop's window leaves them out: it holds
//! nothing there. Then the desktop's window itself is captured (one capture
//! for every scene, without the border a capture is shown by), each scene's
//! part of it put in a window of the scene's own (cloaked, owned as its window
//! is), and that window's thumbnail put in in the desktop's place, under the
//! others: the thumbnails come into a frame together, and a frame put together
//! before them is told as before (see `keep`). Put in the scene's window's own
//! visual, the desktop came in before the windows over it, and a frame of it
//! alone was taken (see `Feed`).
//!
//! A frame is taken only when one is wanted, on a thread other than the one
//! that draws: as it comes, or at once if one has come meanwhile.
//!
//! While a frame is met as it comes, its pool is held: on Windows 10, closing
//! a frame of it or making it anew waits till the frame has been met (on 11
//! neither does). So a frame is met quickly, and read on a thread of its own
//! (one for every scene's, see `reader`);
//! and no frame is closed, nor the pool made anew, while a lock that meeting
//! a frame takes is held (the thread that draws held one and closed a frame,
//! a frame met meanwhile waited on that lock, and the panel and the widgets
//! hung for good). It stays
//! on the graphics card, copied out of the capture's own (which it takes
//! back) into a texture that is drawn from as it is; only how light it is,
//! a value for each square of two by two pixels (the card averages them),
//! is read back, for the theme and the frost. The frames are made on the
//! device Glance draws with (one device: each costs the graphics driver's
//! memory), whose use from several threads is guarded.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use windows::core::{w, Interface, BOOL};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11ShaderResourceView, ID3D11Texture2D, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_BOX,
    D3D11_CPU_ACCESS_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_FLAG_DO_NOT_WAIT, D3D11_MAP_READ, D3D11_RESOURCE_MISC_GENERATE_MIPS, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::DXGI_ERROR_WAS_STILL_DRAWING;
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DwmRegisterThumbnail, DwmSetWindowAttribute, DwmUnregisterThumbnail, DwmUpdateThumbnailProperties, DWMWA_CLOAK, DWMWA_CLOAKED,
    DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION, DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::Graphics::DirectComposition::{DCompositionCreateDevice2, IDCompositionDesktopDevice, IDCompositionSurface, IDCompositionTarget, IDCompositionVisual2};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, EnumWindows, GetShellWindow, GetWindowRect, IsIconic, IsWindowVisible, RegisterClassW, SetWindowPos, ShowWindow, SWP_NOACTIVATE,
    SWP_NOSIZE, SWP_NOZORDER, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WNDCLASSW, WS_EX_NOREDIRECTIONBITMAP, WS_POPUP, GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WM_CLOSE,
    WS_EX_TOOLWINDOW,
};

use super::backdrop::{Capture, Spare, Texture};

/// The device a scene's frames are made and read back on: the one Glance draws with (see `gfx`), its use from
/// several threads guarded by the device itself.
struct Device {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    direct3d: IDirect3DDevice,
}

// (A Direct3D 11 device may be used from any thread; its context is guarded: see `gfx::Gfx::new`.)
unsafe impl Send for Device {}
unsafe impl Sync for Device {}

impl Device {
    /// `gfx`'s, if its context may be used from the threads frames are read on.
    fn of(gfx: &super::gfx::Gfx) -> Option<Device> {
        if !gfx.guarded {
            return None;
        }
        Device::on(&gfx.d3d().ok()?)
    }

    /// `device`'s (one whose context is guarded).
    fn on(device: &ID3D11Device) -> Option<Device> {
        let device = device.clone();
        let context = unsafe { device.GetImmediateContext() }.ok()?;
        let dxgi: IDXGIDevice = device.cast().ok()?;
        let direct3d: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }.ok()?.cast().ok()?;
        Some(Device { device, context, direct3d })
    }
}

/// (Tests: how long a read is made to take, see `Shared::read`.)
#[cfg(test)]
static READ_DELAY_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// (Tests: how long closing a frame is made to take, see `close`: as long as on Windows 10 while a frame is met.)
#[cfg(test)]
static CLOSE_DELAY_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// What a scene's frames meet as they come, on whatever thread they come on.
struct Shared {
    /// What its frames are of: the area, when it was last arranged anew (the system's clock of frames, in 100 ns:
    /// a frame from before is of what was there before), and where in it nothing may be clear once what is behind
    /// is put in (see `keep`).
    of: Mutex<(RECT, i64, Vec<RECT>)>,
    /// The latest frame come and not read (none was wanted as it came).
    latest: Mutex<Option<Direct3D11CaptureFrame>>,
    /// A frame is wanted: read as it comes.
    wanted: AtomicBool,
    /// It rests (see `Scene::rest`): what comes meanwhile is of nothing.
    resting: AtomicBool,
    /// What was read, to be taken.
    ready: Mutex<Option<Capture>>,
    /// Tells the thread that draws (see `Scene::new`).
    wake: Arc<dyn Fn() + Send + Sync>,
    device: Device,
    /// What a frame's lightness is made and read back through, kept for the next of the same size.
    halves: Mutex<Option<Halves>>,
    /// Textures frames were kept in, let go of: written again for the next (see `backdrop::Texture`).
    spare: Arc<Spare>,
    /// How many frames have been taken: what tells one from another.
    taken: std::sync::atomic::AtomicU64,
    /// How many frames are being read now, or are handed on to be (see `read_apart`).
    reading: std::sync::atomic::AtomicU32,
}

/// For a frame `size`: a texture it is copied into whose second level the card makes of it (each texel the
/// average of a square of two by two), that level's view, and a texture the processor reads the level from.
struct Halves {
    size: (u32, u32),
    levels: ID3D11Texture2D,
    view: ID3D11ShaderResourceView,
    readable: ID3D11Texture2D,
}

// (Used under its lock only.)
unsafe impl Send for Halves {}

impl Shared {
    /// Reads `frame` back, if it is of the area as it is now: what it holds, and when the area was arranged as it
    /// was read (to be handed on, see `hand_on`).
    fn read(&self, frame: &Direct3D11CaptureFrame) -> Option<(Capture, i64)> {
        // (Tests: a read as slow as on a slow card, to widen what a frame met while it is read overlaps.)
        #[cfg(test)]
        std::thread::sleep(std::time::Duration::from_millis(READ_DELAY_MS.load(Ordering::Relaxed)));
        let (area, arranged, covered) = self.of.lock().unwrap().clone();
        (|| {
            if frame.SystemRelativeTime().ok()?.Duration < arranged {
                crate::widget::note_sparingly("scene-early", || "scene: a frame from before it was arranged passed over".into());
                return None;
            }
            let size = frame.ContentSize().ok()?;
            let (width, height) = (area.right - area.left, area.bottom - area.top);
            if size.Width < width || size.Height < height {
                crate::widget::note_sparingly("scene-small", || format!("scene: a frame of {}x{} passed over, {width}x{height} wanted", size.Width, size.Height));
                return None;
            }
            let texture: ID3D11Texture2D = unsafe { frame.Surface().ok()?.cast::<IDirect3DDxgiInterfaceAccess>().ok()?.GetInterface() }.ok()?;
            // (Its texture as large too: a pool not made larger as its window was keeps smaller ones, and what lies
            // past a texture is not copied.)
            let mut made = D3D11_TEXTURE2D_DESC::default();
            unsafe { texture.GetDesc(&mut made) };
            if made.Width < width as u32 || made.Height < height as u32 {
                crate::widget::note_sparingly("scene-texture", || format!("scene: a texture of {}x{} passed over, {width}x{height} wanted", made.Width, made.Height));
                return None;
            }
            let (kept, luma) = keep(&self.device, &mut self.halves.lock().unwrap(), &self.spare, &texture, width as u32, height as u32, area, &covered)?;
            let taken = self.taken.fetch_add(1, Ordering::Relaxed) + 1;
            Some((Capture::from_texture(area, Texture { texture: kept, spare: self.spare.clone() }, luma, taken), arranged))
        })()
    }

    /// `capture`, read of the area as it was arranged at `arranged`, handed on to be taken (the thread that draws to
    /// be told: see `reader`); whether it was.
    fn hand_on(&self, capture: Capture, arranged: i64) -> bool {
        // Arranged anew (or rested) while it was read: of what was there before, not taken. (Looked at and handed on
        // under one lock: what arranges or rests it marks it so before it clears what was handed on, so that one
        // handed on meanwhile is cleared too, never left behind.)
        let of = self.of.lock().unwrap();
        if of.1 != arranged {
            return false;
        }
        *self.ready.lock().unwrap() = Some(capture);
        drop(of);
        true
    }
}

/// The top left `width` × `height` of `texture` (a frame of the capture's, taken back by it), kept in a texture of
/// its own to be drawn from (one let go of before, of that size, from `spare`; else a new one); and how light it is,
/// a value (0–255) for each square of two by two pixels, as `Capture::from_pixels` makes it (the card averaging each
/// square, through `halves`: made, or made anew, for that size).
///
/// None where the frame was put together before the windows behind were (the compositor puts them in a frame or
/// so after it is asked, and says nothing of it, and a frame from just before holds the scene's own window: nothing,
/// clear). Told by `opaque` (screen coordinates; the frame's top left at `area`'s): where the desktop's own window is
/// put and a screen is, which is wholly clear in such a frame and wholly opaque once it is put in (whatever the
/// windows over it are; a live wallpaper is drawn in it too). Wholly clear there, it is passed over: what the card
/// made of each square of two by two pixels is clear only where all four are, wherever its squares fall.
#[allow(clippy::too_many_arguments)]
fn keep(device: &Device, halves: &mut Option<Halves>, spare: &Arc<Spare>, texture: &ID3D11Texture2D, width: u32, height: u32, area: RECT, opaque: &[RECT]) -> Option<(ID3D11Texture2D, Vec<u8>)> {
    unsafe {
        let make = |description: &D3D11_TEXTURE2D_DESC| -> Option<ID3D11Texture2D> {
            let mut made = None;
            device.device.CreateTexture2D(description, None, Some(&mut made)).ok()?;
            made
        };
        let plain = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };
        // (Those of another size are let go of: the scene's size has changed.)
        let reused = {
            let mut spare = spare.0.lock().unwrap();
            spare.retain(|texture| {
                let mut description = D3D11_TEXTURE2D_DESC::default();
                texture.GetDesc(&mut description);
                description.Width == width && description.Height == height
            });
            spare.pop()
        };
        let kept = match reused {
            Some(texture) => texture,
            None => {
                make(&plain)?
            }
        };
        if halves.as_ref().is_none_or(|halves| halves.size != (width, height)) {
            let levels = make(&D3D11_TEXTURE2D_DESC {
                MipLevels: 2,
                BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
                MiscFlags: D3D11_RESOURCE_MISC_GENERATE_MIPS.0 as u32,
                ..plain
            })?;
            let mut view = None;
            device.device.CreateShaderResourceView(&levels, None, Some(&mut view)).ok()?;
            let readable = make(&D3D11_TEXTURE2D_DESC {
                Width: (width / 2).max(1),
                Height: (height / 2).max(1),
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                ..plain
            })?;
            *halves = Some(Halves { size: (width, height), levels, view: view?, readable });
        }
        let Halves { levels, view, readable, .. } = halves.as_ref()?;
        let region = D3D11_BOX { left: 0, top: 0, front: 0, right: width, bottom: height, back: 1 };
        let context = &device.context;
        context.CopySubresourceRegion(&kept, 0, 0, 0, 0, texture, 0, Some(&region));
        context.CopySubresourceRegion(levels, 0, 0, 0, 0, texture, 0, Some(&region));
        context.GenerateMips(view);
        // (The second level: subresource 1.)
        context.CopySubresourceRegion(readable, 0, 0, 0, 0, levels, 1, None);
        context.Flush();
        let staging = readable;
        let (row_width, rows) = ((width / 2).max(1) as usize, (height / 2).max(1) as usize);
        // Read once the copy is done, not waited for in the call (the device is the drawing's too, and held
        // meanwhile): tried again till it is, a second at most.
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        let began = std::time::Instant::now();
        loop {
            match context.Map(staging, 0, D3D11_MAP_READ, D3D11_MAP_FLAG_DO_NOT_WAIT.0 as u32, Some(&mut mapped)) {
                Ok(()) => break,
                Err(error) if error.code() == DXGI_ERROR_WAS_STILL_DRAWING && began.elapsed() < std::time::Duration::from_secs(1) => {
                    std::thread::sleep(std::time::Duration::from_millis(1))
                }
                Err(_) => return None,
            }
        }
        // As `Capture::from_pixels` reads it: a square for each two pixels each way, rounded up.
        let (columns, down) = ((width as usize).div_ceil(2), (height as usize).div_ceil(2));
        let mut luma = Vec::with_capacity(columns * down);
        // (Whether any square there was looked at, and whether any held anything.)
        let (mut looked, mut held) = (false, false);
        for y in 0..down {
            let line = std::slice::from_raw_parts((mapped.pData as *const u8).add(y.min(rows - 1) * mapped.RowPitch as usize), row_width * 4);
            // (Each square's middle, on the screen.)
            let screen_y = area.top + (2 * y.min(rows - 1)) as i32 + 1;
            for x in 0..columns {
                let i = x.min(row_width - 1) * 4;
                let lightness = (0.2126 * line[i + 2] as f64 + 0.7152 * line[i + 1] as f64 + 0.0722 * line[i] as f64) / 255.0;
                luma.push((lightness * 255.0).round() as u8);
                if !held {
                    let screen_x = area.left + (2 * x.min(row_width - 1)) as i32;
                    if opaque.iter().any(|rect| screen_x >= rect.left && screen_x + 2 <= rect.right && screen_y > rect.top && screen_y < rect.bottom) {
                        looked = true;
                        held = line[i + 3] > 0;
                    }
                }
            }
        }
        context.Unmap(staging, 0);
        if looked && !held {
            crate::widget::note_sparingly("scene-clear", || format!("scene: a frame of {area:?} with the desktop not yet in it passed over"));
            // (The texture it was copied into, spare again.)
            drop(Texture { texture: kept, spare: spare.clone() });
            return None;
        }
        Some((kept, luma))
    }
}

/// A capture of `item` (a scene's window) at `size`: its frame pool, made on the scene's device, each frame met as it
/// comes (see `Shared`), and its session, started; whether its frames come no oftener than `LIVE`.
fn capture(shared: &Arc<Shared>, item: &GraphicsCaptureItem, size: SizeInt32) -> Option<(Direct3D11CaptureFramePool, GraphicsCaptureSession, i64, bool)> {
    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&shared.device.direct3d, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, size).ok()?;
    let arrived = pool.FrameArrived(&TypedEventHandler::new({
        let shared = shared.clone();
        move |pool: windows::core::Ref<Direct3D11CaptureFramePool>, _| {
            let Ok(pool) = pool.ok() else { return Ok(()) };
            let mut newest: Option<Direct3D11CaptureFrame> = None;
            while let Ok(frame) = pool.TryGetNextFrame() {
                // (Those passed over closed.)
                close(newest.replace(frame));
            }
            let Some(frame) = newest else { return Ok(()) };
            if shared.resting.load(Ordering::Acquire) {
                return Ok(());
            }
            // Wanted, read now, on a thread of its own (see the top); else kept, the thread that draws told once.
            if shared.wanted.swap(false, Ordering::AcqRel) {
                read_apart(&shared, frame);
            } else {
                let older = shared.latest.lock().unwrap().replace(frame);
                if older.is_none() {
                    (shared.wake)();
                }
                close(older);
            }
            Ok(())
        }
    }))
    .ok()?;
    let session = pool.CreateCaptureSession(item).ok()?;
    // (Not on every system: a border no one sees is drawn then.)
    let _ = session.SetIsBorderRequired(false);
    // (Debug builds, where GLANCE_UNPACED is set: as on a system that cannot be asked.)
    let unpaced = cfg!(debug_assertions) && std::env::var_os("GLANCE_UNPACED").is_some();
    let paced = !unpaced && session.SetMinUpdateInterval(live_span()).is_ok();
    let _ = session.SetIsCursorCaptureEnabled(false);
    session.StartCapture().ok()?;
    Some((pool, session, arrived, paced))
}

/// The desktop's own window captured itself, for every scene drawn on one adapter that is to show it so (see the top,
/// and `FEEDS`): the adapter, its capture's pool and session, whether it is taken only seldom (no layer shown: a live
/// wallpaper would be taken each frame for no one), and what its frames meet. Made as first wanted, and kept while a
/// scene holds a layer of it (scenes live as long as their windows, see `let_go_of_feed`): a pool made anew keeps some
/// of the card's memory till Glance ends (made larger only, as the desktop grows past it), and a session ended cannot
/// be started again on the same pool (measured): it is slowed while unused, not ended.
struct Feed {
    /// Its number, a feed made anew another (see `let_go_of_feed`).
    id: u64,
    adapter: Option<super::gfx::Luid>,
    shell: HWND,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    seldom: bool,
    shared: Arc<FeedShared>,
    /// How many scenes hold a layer of it (see `let_go_of_feed`).
    scenes: usize,
}

/// How often what is behind is taken anew at most, for a live backdrop (the panel's, a widget's): thirty times a
/// second (at fifteen, what moves behind the glass, a game's picture, was seen to step). No frame comes oftener,
/// the scenes' nor the desktop's own (where the system can be asked: Windows 11 24H2 and later): a live wallpaper
/// was taken, put in and put together at every refresh of the screen, for a third of it to be read.
pub const LIVE: std::time::Duration = std::time::Duration::from_millis(33);

/// `LIVE` as the capture is told it (100 ns).
fn live_span() -> windows::Foundation::TimeSpan {
    windows::Foundation::TimeSpan { Duration: (LIVE.as_nanos() / 100) as i64 }
}

/// How often the desktop is taken while no layer is shown (100 ns): seldom, its newest frame kept for a layer shown
/// again (shown with it at once, then as it comes).
const SELDOM: i64 = 10 * 10_000_000;

// (Its capture's objects are free-threaded; its window's handle only names it.)
unsafe impl Send for Feed {}

/// What a feed's frames meet as they come: its newest frame (held till the next comes; put in a layer only under its
/// lock, so that it is not closed meanwhile), the layers it is put in, how large its pool's frames are, the device it
/// is copied with, and the composition device the layers are made on.
struct FeedShared {
    latest: Mutex<Option<Direct3D11CaptureFrame>>,
    layers: Mutex<Vec<std::sync::Weak<Layer>>>,
    capacity: Mutex<SizeInt32>,
    device: Device,
    composer: Mutex<Composer>,
}

struct Composer(IDCompositionDesktopDevice);
// (Used only under its lock.)
unsafe impl Send for Composer {}

/// A scene's part of the desktop, in a window of its own (see the top): where it is (the scene's area), the surface it
/// is copied into (as large as the area), and whether the scene rests (nothing is put in it then).
struct Layer {
    held: Mutex<LayerHeld>,
}

struct LayerHeld {
    area: RECT,
    resting: bool,
    /// (Held: let go of, its visual leaves the window.)
    _target: IDCompositionTarget,
    visual: IDCompositionVisual2,
    surface: Option<(IDCompositionSurface, (i32, i32))>,
}

// (Used only under its lock.)
unsafe impl Send for LayerHeld {}

/// The feeds there are, one an adapter that scenes are drawn on (see `gfx::on_screen`): a feed is captured and copied
/// on a device of its adapter, which it holds while a scene holds a layer of it, so that one of a card no window draws
/// on any longer (all moved onto screens the other drives) does not keep that card awake, nor is a scene's layer copied
/// into on a card other than its own.
static FEEDS: Mutex<Vec<Feed>> = Mutex::new(Vec::new());
/// The number the next feed made takes.
static NEXT_FEED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// The desktop's window a feed could not be made for (no borderless capture here, say): shown by its thumbnail.
static REFUSED: Mutex<Option<isize>> = Mutex::new(None);

/// Whether a thumbnail of the desktop's window `shell` leaves out what it shows: a window under it, shown, drawn
/// through a surface of its own (a live wallpaper's), which its thumbnail does not hold.
fn drawn_apart(shell: HWND) -> bool {
    unsafe extern "system" fn each(window: HWND, found: LPARAM) -> BOOL {
        let apart = unsafe { IsWindowVisible(window).as_bool() && GetWindowLongPtrW(window, GWL_EXSTYLE) as u32 & WS_EX_NOREDIRECTIONBITMAP.0 != 0 };
        if apart {
            unsafe { *(found.0 as *mut bool) = true };
        }
        (!apart).into()
    }
    let mut found = false;
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::EnumChildWindows(Some(shell), Some(each), LPARAM(&mut found as *mut bool as isize));
    }
    found
}

impl FeedShared {
    /// `frame` (the desktop's window, its top left at the window's, `content` of it held) put in `layer`, where it
    /// does not rest: the part of it in the layer's area copied into its surface (made anew as the area's size
    /// changes), what of the area is past it clear.
    fn put(&self, layer: &Layer, frame: &Direct3D11CaptureFrame, content: SizeInt32, desktop: RECT) -> Option<()> {
        use windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext1;
        use windows::Win32::Graphics::Dxgi::Common::DXGI_ALPHA_MODE_PREMULTIPLIED;
        let mut held = layer.held.lock().unwrap();
        if held.resting {
            return Some(());
        }
        let area = held.area;
        let size = ((area.right - area.left).max(1), (area.bottom - area.top).max(1));
        let composer = self.composer.lock().unwrap();
        unsafe {
            if held.surface.as_ref().is_none_or(|(_, made)| *made != size) {
                let surface = composer.0.CreateSurface(size.0 as u32, size.1 as u32, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_ALPHA_MODE_PREMULTIPLIED).ok()?;
                held.visual.SetContent(&surface).ok()?;
                held.surface = Some((surface, size));
            }
            let (surface, _) = held.surface.as_ref()?;
            let texture: ID3D11Texture2D = frame.Surface().ok()?.cast::<IDirect3DDxgiInterfaceAccess>().ok()?.GetInterface().ok()?;
            let mut description = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut description);
            let mut offset = windows::Win32::Foundation::POINT::default();
            let into: ID3D11Texture2D = surface.BeginDraw(None, &mut offset).ok()?;
            // Clear first (only its own part: a surface may share its texture with others).
            let mut view = None;
            if self.device.device.CreateRenderTargetView(&into, None, Some(&mut view)).is_ok() {
                if let (Some(view), Ok(context)) = (view, self.device.context.cast::<ID3D11DeviceContext1>()) {
                    let whole = RECT { left: offset.x, top: offset.y, right: offset.x + size.0, bottom: offset.y + size.1 };
                    context.ClearView(&view, &[0.0; 4], Some(&[whole]));
                }
            }
            // What of the desktop is in the area, from what the frame holds of it (its pool's frames may be larger).
            let (held_w, held_h) = ((content.Width as u32).min(description.Width) as i32, (content.Height as u32).min(description.Height) as i32);
            let frame_rect = RECT { left: desktop.left, top: desktop.top, right: desktop.left + held_w, bottom: desktop.top + held_h };
            if let Some(part) = meeting(area, frame_rect) {
                let region = D3D11_BOX { left: (part.left - desktop.left) as u32, top: (part.top - desktop.top) as u32, front: 0, right: (part.right - desktop.left) as u32, bottom: (part.bottom - desktop.top) as u32, back: 1 };
                self.device.context.CopySubresourceRegion(&into, 0, (offset.x + part.left - area.left) as u32, (offset.y + part.top - area.top) as u32, 0, &texture, 0, Some(&region));
            }
            surface.EndDraw().ok()?;
            composer.0.Commit().ok()?;
        }
        Some(())
    }

    /// The layers that are, the gone forgotten.
    fn live_layers(&self) -> Vec<Arc<Layer>> {
        let mut layers = self.layers.lock().unwrap();
        layers.retain(|layer| layer.strong_count() > 0);
        layers.iter().filter_map(std::sync::Weak::upgrade).collect()
    }

    /// The newest frame put in `layer` (as it is arranged anew: no frame may come while the desktop is as it was),
    /// under the newest's lock (not closed meanwhile, as the next comes).
    fn show(&self, layer: &Layer, shell: HWND) {
        let latest = self.latest.lock().unwrap();
        if let Some(frame) = latest.as_ref() {
            let content = frame.ContentSize().unwrap_or_default();
            let _ = self.put(layer, frame, content, desktop_rect(shell));
        }
    }
}

/// Where the desktop's window `shell` is (physical screen coordinates).
fn desktop_rect(shell: HWND) -> RECT {
    let mut rect = RECT::default();
    let _ = unsafe { GetWindowRect(shell, &mut rect) };
    rect
}

/// Whether Glance may capture without the border a capture is shown by (asked of the system once: a capture of the
/// desktop's window with a border would ring every screen). Asked and waited for on a thread of its own, in the
/// multithreaded apartment, whichever thread wants it: the answer comes to the apartment it was asked from, and a
/// thread in one of its own (a note's page's, for its text and what is dropped on it) that waits for it there takes no
/// messages meanwhile, so it never came, and that thread hung (its page never shown) where it was the first to ask.
fn borderless() -> bool {
    static ASKED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ASKED.get_or_init(|| {
        std::thread::Builder::new()
            .name("glance: capture access".into())
            .spawn(|| {
                use windows::Graphics::Capture::{GraphicsCaptureAccess, GraphicsCaptureAccessKind};
                use windows::Security::Authorization::AppCapabilityAccess::AppCapabilityAccessStatus;
                unsafe {
                    let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
                }
                GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Borderless).and_then(|asked| asked.join()).is_ok_and(|status| status == AppCapabilityAccessStatus::Allowed)
            })
            .ok()
            .and_then(|asking| asking.join().ok())
            .unwrap_or(false)
    })
}

impl Feed {
    /// The desktop's window `shell` captured on the device `d3d`, each frame put in the layers as it comes (seldom
    /// while none is shown, see `follow_use`); none where it may not be captured without a border.
    fn new(shell: HWND, d3d: &ID3D11Device) -> Option<Feed> {
        let id = NEXT_FEED.fetch_add(1, Ordering::Relaxed);
        if !borderless() {
            return None;
        }
        let device = Device::on(d3d)?;
        let composer = unsafe { DCompositionCreateDevice2::<_, IDCompositionDesktopDevice>(&device.device.cast::<windows::Win32::Graphics::Dxgi::IDXGIDevice>().ok()?) }.ok()?;
        let item = unsafe { windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>().ok()?.CreateForWindow::<GraphicsCaptureItem>(shell) }.ok()?;
        let size = item.Size().ok()?;
        let shared = Arc::new(FeedShared { latest: Mutex::new(None), layers: Mutex::new(Vec::new()), capacity: Mutex::new(size), device, composer: Mutex::new(Composer(composer)) });
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&shared.device.direct3d, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, size).ok()?;
        let told = shared.clone();
        let shell_handle = shell.0 as isize;
        pool.FrameArrived(&TypedEventHandler::new(move |pool: windows::core::Ref<Direct3D11CaptureFramePool>, _| {
            let Ok(pool) = pool.ok() else { return Ok(()) };
            let mut newest: Option<Direct3D11CaptureFrame> = None;
            while let Ok(frame) = pool.TryGetNextFrame() {
                close(newest.replace(frame));
            }
            let Some(frame) = newest else { return Ok(()) };
            let content = frame.ContentSize().unwrap_or_default();
            // The desktop grown past the pool's frames (a screen came): made larger for the frames after (only
            // larger: see `Feed`). What this one holds is put in meanwhile.
            {
                let mut capacity = told.capacity.lock().unwrap();
                if content.Width > capacity.Width || content.Height > capacity.Height {
                    let larger = SizeInt32 { Width: content.Width.max(capacity.Width), Height: content.Height.max(capacity.Height) };
                    if pool.Recreate(&told.device.direct3d, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, larger).is_ok() {
                        *capacity = larger;
                    }
                }
            }
            let desktop = desktop_rect(HWND(shell_handle as *mut _));
            for layer in told.live_layers() {
                let _ = told.put(&layer, &frame, content, desktop);
            }
            // Held as the newest (the one before closed once no lock is held).
            let older = told.latest.lock().unwrap().replace(frame);
            close(older);
            Ok(())
        }))
        .ok()?;
        let session = pool.CreateCaptureSession(&item).ok()?;
        session.SetIsBorderRequired(false).ok()?;
        let _ = session.SetIsCursorCaptureEnabled(false);
        let _ = session.SetMinUpdateInterval(live_span());
        session.StartCapture().ok()?;
        crate::perf::mark(|| format!("desktop feed made {}x{}", size.Width, size.Height));
        Some(Feed { id, adapter: super::gfx::adapter_of(d3d), shell, pool, session, seldom: false, shared, scenes: 0 })
    }

    /// Taken as often as it changes (`LIVE` at most) while a layer does not rest, seldom while none does (where the system can be
    /// asked to: Windows 11 24H2 and later).
    fn follow_use(&mut self) {
        let seldom = !self.shared.live_layers().iter().any(|layer| !layer.held.lock().unwrap().resting);
        if seldom != self.seldom {
            let every = if seldom { windows::Foundation::TimeSpan { Duration: SELDOM } } else { live_span() };
            if self.session.SetMinUpdateInterval(every).is_ok() {
                self.seldom = seldom;
            }
        }
    }

    /// A layer in `window` (a scene's desktop window) for `area`, the newest frame put in it at once.
    fn layer(&mut self, window: HWND, area: RECT) -> Option<Arc<Layer>> {
        let (target, visual) = {
            let composer = self.shared.composer.lock().unwrap();
            unsafe {
                let target = composer.0.CreateTargetForHwnd(window, false).ok()?;
                let visual: IDCompositionVisual2 = composer.0.CreateVisual().ok()?;
                target.SetRoot(&visual).ok()?;
                composer.0.Commit().ok()?;
                (target, visual)
            }
        };
        let layer = Arc::new(Layer { held: Mutex::new(LayerHeld { area, resting: false, _target: target, visual, surface: None }) });
        self.shared.layers.lock().unwrap().push(Arc::downgrade(&layer));
        self.follow_use();
        self.shared.show(&layer, self.shell);
        Some(layer)
    }

    /// Whether the device it captures and copies with is lost (the graphics driver started again, the card gone): no
    /// frame comes, nor can one be put in a layer, again.
    fn lost(&self) -> bool {
        unsafe { self.shared.device.device.GetDeviceRemovedReason() }.is_err()
    }

    /// Its capture ended, its newest frame let go of (Explorer started again: another desktop window; its device lost).
    fn end(self) {
        let _ = self.session.Close();
        let held = self.shared.latest.lock().unwrap().take();
        close(held);
        let _ = self.pool.Close();
        crate::perf::mark(|| "desktop feed ended".into());
    }
}

/// A layer of the desktop for a scene's `window` at `area`, from the feed of `d3d`'s adapter for the desktop's window
/// `shell` (made on `d3d` as first wanted, and made anew where it was another's, as Explorer started again, or its
/// device was lost), and the feed's number. None where none can be made for it (it is not asked again for that window).
fn desktop_layer(shell: HWND, window: HWND, area: RECT, d3d: &ID3D11Device) -> Option<(Arc<Layer>, u64)> {
    if *REFUSED.lock().unwrap() == Some(shell.0 as isize) {
        return None;
    }
    let adapter = super::gfx::adapter_of(d3d);
    let mut feeds = FEEDS.lock().unwrap();
    if let Some(stale) = feeds.iter().position(|feed| feed.adapter == adapter && (feed.shell != shell || feed.lost())) {
        feeds.swap_remove(stale).end();
    }
    if !feeds.iter().any(|feed| feed.adapter == adapter) {
        match Feed::new(shell, d3d) {
            Some(feed) => feeds.push(feed),
            None => {
                *REFUSED.lock().unwrap() = Some(shell.0 as isize);
                return None;
            }
        }
    }
    let feed = feeds.iter_mut().find(|feed| feed.adapter == adapter)?;
    let layer = feed.layer(window, area)?;
    feed.scenes += 1;
    crate::perf::mark(|| format!("desktop feed held by {}", feed.scenes));
    Some((layer, feed.id))
}

/// The feed numbered `id` of `feeds`, where it is still one there is.
fn numbered(feeds: &mut [Feed], id: u64) -> Option<&mut Feed> {
    feeds.iter_mut().find(|feed| feed.id == id)
}

/// Whether the feed numbered `id` is one there is, its device not lost: else a layer of it is of one ended, or soon to
/// be, and nothing comes into it.
fn feed_goes_on(id: u64) -> bool {
    numbered(&mut FEEDS.lock().unwrap(), id).is_some_and(|feed| !feed.lost())
}

/// A scene's layer of the feed numbered `id` let go of: the feed ended as no scene holds one any longer (each scene
/// gone, or let go of as its window's skin no longer shows the desktop, or made again on another adapter's device): its
/// frames are the desktop's whole size. (Counted, not by the layers that are: the thread a frame comes on holds them a
/// moment as it puts it in them, and a last let go of meanwhile left the feed for good. Scenes live as long as their
/// windows, so it is not made again and again: see `Feed`. One of a feed made anew since, Explorer started again or its
/// device lost, is not counted.)
fn let_go_of_feed(id: u64) {
    let mut feeds = FEEDS.lock().unwrap();
    let Some(held) = numbered(&mut feeds, id) else { return };
    held.scenes = held.scenes.saturating_sub(1);
    crate::perf::mark(|| format!("desktop feed held by {}", held.scenes));
    if held.scenes == 0 {
        if let Some(at) = feeds.iter().position(|feed| feed.id == id) {
            feeds.swap_remove(at).end();
        }
    }
}

/// Whether a feed may be made for the desktop's window `shell` (none could before).
fn feed_refused(shell: HWND) -> bool {
    *REFUSED.lock().unwrap() == Some(shell.0 as isize)
}

/// `layer` (of the feed numbered `feed`) at `area` and shown (no longer resting): the newest frame put in it where it
/// was elsewhere or resting.
fn move_layer(layer: &Layer, feed: u64, area: RECT) {
    let changed = {
        let mut held = layer.held.lock().unwrap();
        let changed = held.area != area || held.resting;
        held.area = area;
        held.resting = false;
        changed
    };
    if changed {
        if let Some(feed) = numbered(&mut FEEDS.lock().unwrap(), feed) {
            feed.follow_use();
            feed.shared.show(layer, feed.shell);
        }
    }
}

/// `layer` (of the feed numbered `feed`) resting (its scene rests): nothing put in it till it is shown again, and the
/// capture slowed where no layer of its feed is shown.
fn rest_layer(layer: &Layer, feed: u64) {
    layer.held.lock().unwrap().resting = true;
    if let Some(feed) = numbered(&mut FEEDS.lock().unwrap(), feed) {
        feed.follow_use();
    }
}

/// `layer` (of the feed numbered `feed`) emptied (the desktop shown as a thumbnail again, or its scene gone): nothing
/// in it from now on.
fn empty_layer(layer: &Layer, feed: u64) {
    // (The feeds' lock not taken under the layer's: a frame put in the layers takes them the other way round.)
    let shared = numbered(&mut FEEDS.lock().unwrap(), feed).map(|feed| feed.shared.clone());
    {
        let mut held = layer.held.lock().unwrap();
        held.resting = true;
        held.surface = None;
        if let Some(shared) = &shared {
            let composer = shared.composer.lock().unwrap();
            unsafe {
                let _ = held.visual.SetContent(None);
                let _ = composer.0.Commit();
            }
        }
    }
    if let Some(feed) = numbered(&mut FEEDS.lock().unwrap(), feed) {
        feed.follow_use();
    }
}

/// A window of Glance's own that no one sees, the windows behind an area put together in it, and its capture.
pub struct Scene {
    owner: HWND,
    window: HWND,
    /// The area it stands for (physical screen coordinates).
    area: RECT,
    /// The windows shown in it, bottom first: each with its thumbnail, where it was, and how opaque.
    shown: Vec<(HWND, isize, RECT, u8)>,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    /// Its handler of frames as they come, as the pool has it.
    arrived: i64,
    /// The size the frames are made at.
    size: SizeInt32,
    /// Its frames come no oftener than `LIVE` (see `paced`).
    paced: bool,
    shared: Arc<Shared>,
    /// The desktop, captured itself and put under the windows behind, where a thumbnail of its window would leave out
    /// what it shows (see `Feed`): the window it is put in, its layer, the desktop's window it is of and the feed's
    /// number; the device to start that on.
    desktop: Option<(HWND, Arc<Layer>, HWND, u64)>,
    d3d: ID3D11Device,
}

/// `frame` read on the thread frames are read on (see the top), then closed (one of what was there before is not
/// read, and the next is wanted still). Counted as being read from now (see `Scene::want`).
fn read_apart(shared: &Arc<Shared>, frame: Direct3D11CaptureFrame) {
    shared.reading.fetch_add(1, Ordering::AcqRel);
    let _ = reader().send((shared.clone(), frame));
}

/// What takes the frames to be read, for every scene, and reads them one after another, on a thread of its own,
/// started as the first is (one started for each frame was some thirty a second for a live backdrop).
fn reader() -> &'static std::sync::mpsc::Sender<(Arc<Shared>, Direct3D11CaptureFrame)> {
    static READER: std::sync::OnceLock<std::sync::mpsc::Sender<(Arc<Shared>, Direct3D11CaptureFrame)>> = std::sync::OnceLock::new();
    READER.get_or_init(|| {
        let (sender, frames) = std::sync::mpsc::channel::<(Arc<Shared>, Direct3D11CaptureFrame)>();
        std::thread::Builder::new()
            .name("glance: frames read".into())
            .spawn(move || {
                for (shared, frame) in frames {
                    let read = {
                        let _timed = crate::perf::Timed::start("scene read", String::new);
                        shared.read(&frame)
                    };
                    close(Some(frame));
                    // No longer being read before what was read is handed on: what is wanted once it is taken is
                    // wanted, not taken for served by this read (wanted while the frame was closed, it was never read).
                    shared.reading.fetch_sub(1, Ordering::AcqRel);
                    if read.is_some_and(|(capture, arranged)| shared.hand_on(capture, arranged)) {
                        (shared.wake)();
                    } else {
                        shared.wanted.store(true, Ordering::Release);
                    }
                }
            })
            .expect("a thread started");
        sender
    })
}

/// A frame done with, closed: its buffer back in its pool at once, not once the last reference to it goes. Never
/// with a lock held that meeting a frame takes (see the top).
fn close(frame: Option<Direct3D11CaptureFrame>) {
    if let Some(frame) = frame {
        #[cfg(test)]
        std::thread::sleep(std::time::Duration::from_millis(CLOSE_DELAY_MS.load(Ordering::Relaxed)));
        let _ = frame.Close();
    }
}

/// The system's clock of frames, now (100 ns).
fn now_100ns() -> i64 {
    let (mut count, mut frequency) = (0i64, 0i64);
    unsafe {
        let _ = QueryPerformanceCounter(&mut count);
        let _ = QueryPerformanceFrequency(&mut frequency);
    }
    (count as i128 * 10_000_000 / frequency.max(1) as i128) as i64
}

unsafe extern "system" fn procedure(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // Closed only with its scene (see `Drop`): asked to close by another program, it stays.
    if message == WM_CLOSE {
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// The window a scene is put together in, for `area`, cloaked and shown, its owner, and the capture of it: in no
/// task switcher (see the top).
fn windows(area: RECT) -> Option<(HWND, HWND, GraphicsCaptureItem)> {
    unsafe {
        let module = GetModuleHandleW(None).ok()?;
        let class = WNDCLASSW { lpfnWndProc: Some(procedure), hInstance: module.into(), lpszClassName: w!("GlanceScene"), ..Default::default() };
        RegisterClassW(&class);
        let (width, height) = ((area.right - area.left).max(1), (area.bottom - area.top).max(1));
        let owner = CreateWindowExW(WINDOW_EX_STYLE(0), class.lpszClassName, w!(""), WS_POPUP, 0, 0, 0, 0, None, None, Some(module.into()), None).ok()?;
        // With no surface of its own (nothing draws it): where no window behind is put, it holds nothing (clear),
        // never what its surface would hold, which no one wrote (what was in that memory before, on some cards).
        let Ok(window) = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP, class.lpszClassName, w!("Glance scene"), WS_POPUP, area.left, area.top, width, height, Some(owner), None, Some(module.into()), None) else {
            let _ = DestroyWindow(owner);
            return None;
        };
        let item = (|| {
            let cloak: BOOL = true.into();
            DwmSetWindowAttribute(window, DWMWA_CLOAK, &cloak as *const BOOL as *const _, 4).ok()?;
            let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
            windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>().ok()?.CreateForWindow::<GraphicsCaptureItem>(window).ok()
        })();
        let Some(item) = item else {
            let _ = DestroyWindow(window);
            let _ = DestroyWindow(owner);
            return None;
        };
        // Taken: now out of the task switcher.
        SetWindowLongPtrW(owner, GWL_EXSTYLE, GetWindowLongPtrW(owner, GWL_EXSTYLE) | WS_EX_TOOLWINDOW.0 as isize);
        Some((owner, window, item))
    }
}

unsafe extern "system" fn each(window: HWND, found: LPARAM) -> BOOL {
    unsafe { (*(found.0 as *mut Vec<HWND>)).push(window) };
    true.into()
}

/// How opaque `window` is drawn as a whole (0 to 255): a layered window as its opacity is set.
fn opacity_of(window: HWND) -> u8 {
    use windows::Win32::UI::WindowsAndMessaging::{GetLayeredWindowAttributes, LAYERED_WINDOW_ATTRIBUTES_FLAGS, LWA_ALPHA, WS_EX_LAYERED};
    let layered = unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) } as u32 & WS_EX_LAYERED.0 != 0;
    let (mut alpha, mut flags) = (255u8, LAYERED_WINDOW_ATTRIBUTES_FLAGS(0));
    // (One drawn whole by its program, its opacity its pixels', tells none: whole.)
    if layered && unsafe { GetLayeredWindowAttributes(window, None, Some(&mut alpha), Some(&mut flags)) }.is_ok() && flags.contains(LWA_ALPHA) {
        alpha
    } else {
        255
    }
}

fn cloaked(window: HWND) -> bool {
    let mut cloaked = 0u32;
    unsafe { DwmGetWindowAttribute(window, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, 4) }.is_ok() && cloaked != 0
}

/// Whether the system can capture a window (Windows.Graphics.Capture's capture of a window by its handle: Windows 10
/// 1903 and later, the Universal API contract 8). (Its capture alone came in 1803: there, asked whether it captures,
/// 1809 says it does, and a scene could not be made, again and again, with no other way to what is behind.)
pub fn supported() -> bool {
    static SUPPORTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        let by_window = windows::Foundation::Metadata::ApiInformation::IsApiContractPresentByMajor(&windows::core::HSTRING::from("Windows.Foundation.UniversalApiContract"), 8).unwrap_or(false);
        by_window && GraphicsCaptureSession::IsSupported().unwrap_or(false)
    })
}

/// Glance's windows left out of every scene, while they are: what is not part of what is behind (the panel as a
/// widget torn off it takes its place, the hint of where a widget would stick).
static LEFT_OUT: Mutex<Vec<isize>> = Mutex::new(Vec::new());

/// Leaves `window` out of every scene from now on (`out`), or no longer.
pub fn leave_out(window: HWND, out: bool) {
    let mut left_out = LEFT_OUT.lock().unwrap();
    left_out.retain(|kept| *kept != window.0 as isize);
    if out {
        left_out.push(window.0 as isize);
    }
}

/// The least box holding the screens (`monitors`) `area` meets: what of the desktop is behind it, past which there
/// is no screen and nothing (the desktop's box reaches past a screen where screens of other heights stand beside it).
/// None where it meets none.
fn screens_of(area: RECT, monitors: &[RECT]) -> Option<RECT> {
    monitors.iter().filter(|monitor| meet(**monitor, area)).copied().reduce(|a, b| RECT { left: a.left.min(b.left), top: a.top.min(b.top), right: a.right.max(b.right), bottom: a.bottom.max(b.bottom) })
}

fn meet(a: RECT, b: RECT) -> bool {
    a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
}

/// Where `a` and `b` meet, if they do.
fn meeting(a: RECT, b: RECT) -> Option<RECT> {
    meet(a, b).then(|| RECT { left: a.left.max(b.left), top: a.top.max(b.top), right: a.right.min(b.right), bottom: a.bottom.min(b.bottom) })
}

impl Scene {
    /// A scene standing for `area`, its frames made on `gfx`'s device, empty till it is arranged; `wake` is called
    /// (on another thread) as a frame wanted has been read (see `ready`), and as one comes unwanted after none had
    /// (see `fresh`).
    pub fn new(area: RECT, wake: Arc<dyn Fn() + Send + Sync>, gfx: &super::gfx::Gfx) -> Option<Scene> {
        let device = Device::of(gfx)?;
        unsafe {
            let (owner, window, item) = windows(area)?;
            let shared = Arc::new(Shared { of: Mutex::new((area, now_100ns(), Vec::new())), latest: Mutex::new(None), wanted: AtomicBool::new(false), ready: Mutex::new(None), wake, device, halves: Mutex::new(None), spare: Arc::default(), resting: AtomicBool::new(false), taken: std::sync::atomic::AtomicU64::new(0), reading: std::sync::atomic::AtomicU32::new(0) });
            let made = (|| {
                let size = item.Size().ok()?;
                let (pool, session, arrived, paced) = capture(&shared, &item, size)?;
                Some((pool, session, arrived, size, paced))
            })();
            let Some((pool, session, arrived, size, paced)) = made else {
                let _ = DestroyWindow(window);
                let _ = DestroyWindow(owner);
                return None;
            };
            let d3d = shared.device.device.clone();
            Some(Scene { owner, window, area, shown: Vec::new(), pool, session, arrived, size, paced, shared, desktop: None, d3d })
        }
    }

    /// Puts together in it, for `area`, the windows behind `over` (one of Glance's: those under it, the desktop's
    /// own among them) that meet the area, as they are now; what has not changed is left as it is. Whether anything
    /// had (then a frame of it as it is now comes). What it stands for, and its frames, may reach past `area` (see
    /// `gfx::room`): a capture says where it was taken.
    pub fn arrange(&mut self, area: RECT, over: HWND) -> bool {
        // What of the area is on the screens it meets (a widget stuck to a screen's top reaches past it, its shadow's
        // room: above a screen lower than the one beside it, inside the desktop's box, there is nothing, which the
        // glass would draw in as a black band). Its frames hold that only; the glass mirrors on past their edges.
        let screen = Some(screens_of(area, &crate::screens::monitors()).unwrap_or_else(crate::mouse::desktop));
        let area = screen.and_then(|screen| meeting(area, screen)).unwrap_or(area);
        // As large as it is while that serves, else with room to spare (see `gfx::room`): a window sized pixel by
        // pixel makes its frames, and what they are read through and kept in, anew a few times, not at each size.
        let want = ((area.right - area.left).max(1) as u32, (area.bottom - area.top).max(1) as u32);
        let have = (self.size.Width as u32, self.size.Height as u32);
        let (width, height) = super::gfx::room(Some(have), want).unwrap_or(have);
        // The room to spare on the desktop (what reaches past it is of no screen): on whichever side there is room,
        // all of what was asked held. Against the desktop's edge (a panel, a widget stuck to it), it spares room
        // towards the middle. Not on one screen alone: what is behind a window across two is behind it on both.
        let span = |from: i32, to: i32, length: i32, low: Option<i32>, high: Option<i32>| {
            // As far as it goes from `from`, moved back from the screen's far side, never past where it begins.
            let start = high.map_or(from, |high| from.min(high - length));
            let start = low.map_or(start, |low| start.max(low)).min(from);
            // (Larger than the screen: as far as the screen goes.)
            let end = high.map_or(start + length, |high| (start + length).min(high.max(to)));
            (start, end.max(to))
        };
        let (left, right) = span(area.left, area.right, width as i32, screen.map(|s| s.left), screen.map(|s| s.right));
        let (top, bottom) = span(area.top, area.bottom, height as i32, screen.map(|s| s.top), screen.map(|s| s.bottom));
        let area = RECT { left, top, right, bottom };
        let mut all: Vec<HWND> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(each), LPARAM(&mut all as *mut _ as isize));
        }
        // Top first: those after `over`; then bottom first.
        let left_out = LEFT_OUT.lock().unwrap().clone();
        let mut behind: Vec<(HWND, RECT)> = all
            .into_iter()
            .skip_while(|window| *window != over)
            .skip(1)
            .filter(|window| !left_out.contains(&(window.0 as isize)))
            .filter(|window| unsafe { IsWindowVisible(*window).as_bool() && !IsIconic(*window).as_bool() } && !cloaked(*window) && opacity_of(*window) > 0)
            .filter_map(|window| {
                let mut rect = RECT::default();
                unsafe { GetWindowRect(window, &mut rect) }.ok()?;
                meet(rect, area).then_some((window, rect))
            })
            .collect();
        behind.reverse();
        // The desktop, where its window's thumbnail would leave out what it shows (see `Feed`): captured itself and put
        // in this scene's own layer, not as a thumbnail.
        let shell = unsafe { GetShellWindow() };
        let desktop_at = behind.iter().find(|(window, _)| *window == shell).map(|(_, rect)| *rect);
        let apart = desktop_at.is_some() && !feed_refused(shell) && drawn_apart(shell);
        // Its layer, of the desktop's window as it is now (one of another, as Explorer started again, or of a feed
        // ended or whose device was lost, let go of: a feed made anew), put where it is to be before anything else is,
        // and its window in the desktop's place (the bottom); the desktop's own thumbnail where none could be made.
        if self.desktop.as_ref().is_some_and(|(_, _, of, feed)| !apart || *of != shell || !feed_goes_on(*feed)) {
            self.let_desktop_go();
        }
        let own = match (&self.desktop, apart) {
            (Some((window, layer, _, feed)), true) => {
                unsafe {
                    let _ = SetWindowPos(*window, None, area.left, area.top, (area.right - area.left).max(1), (area.bottom - area.top).max(1), SWP_NOACTIVATE | SWP_NOZORDER);
                }
                move_layer(layer, *feed, area);
                Some(*window)
            }
            (None, true) => self.desktop_window(area).and_then(|window| match desktop_layer(shell, window, area, &self.d3d) {
                Some((layer, feed)) => {
                    self.desktop = Some((window, layer, shell, feed));
                    Some(window)
                }
                None => {
                    unsafe {
                        let _ = DestroyWindow(window);
                    }
                    None
                }
            }),
            (_, false) => None,
        };
        if let Some(own) = own {
            for entry in behind.iter_mut().filter(|(window, _)| *window == shell) {
                *entry = (own, area);
            }
        }
        let same_windows = behind.len() == self.shown.len() && behind.iter().zip(&self.shown).all(|((a, _), (b, ..))| a == b);
        // (Nor how opaque each is: one drawn more or less opaque since is put in at its opacity now.)
        if same_windows && area == self.area && behind.iter().zip(&self.shown).all(|((window, a), (_, _, b, opacity))| a == b && opacity_of(*window) == *opacity) {
            return false;
        }
        // Where the desktop's own window will be put, on a screen (see `keep`): worked out before anything is put in
        // place, so that nothing comes between the last of that and when frames count from. (Between screens it
        // holds nothing. Not behind, as while Explorer is not running: nothing then tells such a frame.)
        let opaque: Vec<RECT> = desktop_at.map_or_else(Vec::new, |desktop| {
            crate::screens::monitors().into_iter().filter_map(|monitor| meeting(meeting(desktop, monitor)?, area)).collect()
        });
        let (width, height) = ((area.right - area.left).max(1), (area.bottom - area.top).max(1));
        if area != self.area {
            // (Only moved, where its size is as it was: sizing a window costs more.)
            let same_size = (area.right - area.left, area.bottom - area.top) == (self.area.right - self.area.left, self.area.bottom - self.area.top);
            let flags = SWP_NOACTIVATE | SWP_NOZORDER | if same_size { SWP_NOSIZE } else { Default::default() };
            unsafe {
                let _ = SetWindowPos(self.window, None, area.left, area.top, width, height, flags);
            }
            self.area = area;
        }
        let screen_size = screen.map(|s| SizeInt32 { Width: s.right - s.left, Height: s.bottom - s.top });
        // (Not made larger, the frames that come are too small, and are not taken: see `Shared::read`.)
        let _ = self.fit(SizeInt32 { Width: width, Height: height }, screen_size);
        // The thumbnails are drawn in the order they were made in: another order, or other windows, and they are
        // made anew, bottom first; the same windows, only moved, are moved.
        if !same_windows {
            self.forget();
            for (window, rect) in behind {
                if let Ok(thumbnail) = unsafe { DwmRegisterThumbnail(self.window, window) } {
                    self.shown.push((window, thumbnail, RECT::default(), 255));
                    self.place(self.shown.len() - 1, rect);
                }
            }
        } else {
            for (index, (_, rect)) in behind.into_iter().enumerate() {
                self.place(index, rect);
            }
        }
        // Frames from before all of it was in place are not of what is there now (one may be put together while it
        // is being put in place); nor what was read of it before and not taken yet. Cleared and made so under one
        // lock, as what reads a frame looks and hands on under it: one read meanwhile is of before, or kept.
        let from = now_100ns();
        let stale = {
            let mut of = self.shared.of.lock().unwrap();
            *self.shared.ready.lock().unwrap() = None;
            let stale = self.shared.latest.lock().unwrap().take();
            *of = (area, from, opaque);
            self.shared.resting.store(false, Ordering::Release);
            stale
        };
        close(stale);
        true
    }

    /// A window of its own for the desktop's part (see `Feed`), at `area`: cloaked and shown, owned by its owner (in
    /// no task switcher), with no surface of its own.
    fn desktop_window(&self, area: RECT) -> Option<HWND> {
        unsafe {
            let module = GetModuleHandleW(None).ok()?;
            let (width, height) = ((area.right - area.left).max(1), (area.bottom - area.top).max(1));
            let window = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP, w!("GlanceScene"), w!("Glance desktop"), WS_POPUP, area.left, area.top, width, height, Some(self.owner), None, Some(module.into()), None).ok()?;
            let cloak: BOOL = true.into();
            if DwmSetWindowAttribute(window, DWMWA_CLOAK, &cloak as *const BOOL as *const _, 4).is_err() {
                let _ = DestroyWindow(window);
                return None;
            }
            let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
            Some(window)
        }
    }

    /// Its desktop's part let go of, and the window it was in (its layer first).
    fn let_desktop_go(&mut self) {
        if let Some((window, layer, _, feed)) = self.desktop.take() {
            empty_layer(&layer, feed);
            drop(layer);
            let_go_of_feed(feed);
            unsafe {
                let _ = DestroyWindow(window);
            }
        }
    }

    /// Holds nothing till it is arranged again: no thumbnails (the compositor puts nothing together in it), its window
    /// a pixel, and what its frames were read through given back (its pool of frames kept: see `fit`). For whoever
    /// wants what is behind only as it was when last looked at, not as it changes, or is hidden (see `life`);
    /// arranged again, it is as it was, and as quickly.
    pub fn rest(&mut self) {
        let least = RECT { left: self.area.left, top: self.area.top, right: self.area.left + 1, bottom: self.area.top + 1 };
        // (A frame being read meanwhile is of what was there: marked so first, see `Shared::read`.)
        *self.shared.of.lock().unwrap() = (least, now_100ns(), Vec::new());
        self.shared.resting.store(true, Ordering::Release);
        if let Some((_, layer, _, feed)) = &self.desktop {
            rest_layer(layer, *feed);
        }
        self.forget();
        self.shared.wanted.store(false, Ordering::Release);
        let stale = self.shared.latest.lock().unwrap().take();
        close(stale);
        *self.shared.ready.lock().unwrap() = None;
        // (Not while a frame is read through them on another thread, which can wait a while on the card: the
        // thread that draws never waits on it. Let go of at the next rest, or with the scene.)
        if let Ok(mut halves) = self.shared.halves.try_lock() {
            *halves = None;
        }
        self.shared.spare.0.lock().unwrap().clear();
        unsafe {
            let _ = SetWindowPos(self.window, None, least.left, least.top, 1, 1, SWP_NOACTIVATE | SWP_NOZORDER);
        }
        self.area = least;
        // (Its pool of frames stays as it is, see `fit`: made smaller, and larger again as it is arranged, would be
        // two more the system keeps.)
    }

    /// The thumbnail `index` shown where its window is, `rect`, within the area.
    fn place(&mut self, index: usize, rect: RECT) {
        let (window, thumbnail, at, shown_at) = &mut self.shown[index];
        let (x, y) = (self.area.left, self.area.top);
        let opacity = opacity_of(*window);
        let properties = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_OPACITY | DWM_TNP_SOURCECLIENTAREAONLY,
            rcDestination: RECT { left: rect.left - x, top: rect.top - y, right: rect.right - x, bottom: rect.bottom - y },
            fVisible: true.into(),
            // (A thumbnail is drawn whole, whatever the window's opacity: at it, as the window is.)
            opacity,
            fSourceClientAreaOnly: false.into(),
            ..Default::default()
        };
        let _ = unsafe { DwmUpdateThumbnailProperties(*thumbnail, &properties) };
        *at = rect;
        *shown_at = opacity;
    }

    fn forget(&mut self) {
        for (_, thumbnail, ..) in self.shown.drain(..) {
            let _ = unsafe { DwmUnregisterThumbnail(thumbnail) };
        }
    }

    /// Its pool's handler taken away, and what it holds of its frames let go of: the handler holds `shared`, which
    /// may hold a frame of the pool's (which holds the pool).
    fn unhook(&mut self) {
        let _ = self.pool.RemoveFrameArrived(self.arrived);
        let stale = self.shared.latest.lock().unwrap().take();
        close(stale);
        *self.shared.ready.lock().unwrap() = None;
        // (Not while a frame is read through them on another thread, which can wait a while on the card: the
        // thread that draws never waits on it. Let go of at the next rest, or with the scene.)
        if let Ok(mut halves) = self.shared.halves.try_lock() {
            *halves = None;
        }
        self.shared.spare.0.lock().unwrap().clear();
    }

    /// Its frames made large enough for `size` from now on, on a desktop of `screen`'s size (none: not known);
    /// whether they are. Each pool of frames made is memory the system keeps for good (till Glance ends, whatever is
    /// let go of: measured, a frame's worth each time): so it is made only larger, never smaller, and a quarter
    /// larger than asked, as large as the desktop at most (what a scene stands for is on it); for a window moved
    /// and sized about, it is made a few times at most, then serves.
    fn fit(&mut self, size: SizeInt32, screen: Option<SizeInt32>) -> bool {
        if size.Width <= self.size.Width && size.Height <= self.size.Height {
            return true;
        }
        let grown = |want: i32, have: i32, most: Option<i32>| {
            let roomy = want + want / 4;
            have.max(most.map_or(roomy, |most| roomy.min(most.max(want))))
        };
        let made = SizeInt32 {
            Width: grown(size.Width, self.size.Width, screen.map(|screen| screen.Width)),
            Height: grown(size.Height, self.size.Height, screen.map(|screen| screen.Height)),
        };
        if self.pool.Recreate(&self.shared.device.direct3d, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, made).is_err() {
            return false;
        }
        self.size = made;
        true
    }

    /// Whether its frames come no oftener than `LIVE`, the system asked (Windows 11 24H2 and later): a live backdrop
    /// can take each as it comes. Else they come as often as the screen is drawn, and are taken as often as `LIVE`.
    pub fn paced(&self) -> bool {
        self.paced
    }

    /// Whether a frame is being read now (to be told as it is: see `ready`).
    pub fn reading(&self) -> bool {
        self.shared.reading.load(Ordering::Acquire) > 0
    }

    /// Whether a frame has come that was not wanted as it came (and is not read yet).
    pub fn fresh(&self) -> bool {
        self.shared.latest.lock().unwrap().is_some()
    }

    /// Wants a frame: one come already, read at once (on another thread); else the next, as it comes. Told as it
    /// has been read (see `ready`). One read, or being read, serves: wanted meanwhile, no other is (one that comes
    /// to nothing wants the next itself).
    pub fn want(&self) {
        if self.reading() || self.shared.ready.lock().unwrap().is_some() {
            return;
        }
        let come = self.shared.latest.lock().unwrap().take();
        match come {
            Some(frame) => read_apart(&self.shared, frame),
            None => self.shared.wanted.store(true, Ordering::Release),
        }
    }

    /// A frame wanted, read: what is behind as it showed (a capture of the area).
    pub fn ready(&self) -> Option<Capture> {
        self.shared.ready.lock().unwrap().take()
    }
}

impl Drop for Scene {
    fn drop(&mut self) {
        self.let_desktop_go();
        self.unhook();
        let _ = self.session.Close();
        let _ = self.pool.Close();
        self.forget();
        unsafe {
            let _ = DestroyWindow(self.window);
            let _ = DestroyWindow(self.owner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window over the desktop is in the first frame taken after a scene is arranged, as the desktop is (a frame of
    /// the desktop alone is not taken): a red window put up on the main screen a moment, a scene arranged over it and
    /// the desktop round it, again and again, each first frame's red counted.
    /// `cargo test --lib -- --ignored --exact ui::scene::tests::scene_first_frame_holds_the_windows --nocapture`.
    #[test]
    #[ignore = "puts a window on this PC's screen and captures it"]
    fn scene_first_frame_holds_the_windows() {
        let _device = crate::ui::gfx::held().unwrap();
        use windows::Win32::Graphics::Gdi::{CreateSolidBrush, HBRUSH};
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
            let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        let gfx = super::super::gfx::Gfx::new().expect("graphics");
        unsafe extern "system" fn answer(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        let red = unsafe {
            let class = WNDCLASSW { lpfnWndProc: Some(answer), lpszClassName: w!("GlanceRedTest"), hbrBackground: HBRUSH(CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x0000FF)).0), ..Default::default() };
            RegisterClassW(&class);
            let window = CreateWindowExW(WINDOW_EX_STYLE(0), class.lpszClassName, w!(""), WS_POPUP, 1300, 700, 300, 200, None, None, None, None).expect("a window");
            let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
            window
        };
        let pump = || unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE};
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        };
        for _ in 0..20 {
            pump();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let mut all: Vec<HWND> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(each), LPARAM(&mut all as *mut _ as isize));
        }
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        let mut scene = Scene::new(RECT { left: 1100, top: 600, right: 1800, bottom: 1000 }, wake, &gfx).expect("scene");
        let mut reds = Vec::new();
        for step in 0..12 {
            let area = RECT { left: 1100 + (step % 3) * 20, top: 600 + (step % 4) * 10, right: 1800, bottom: 1000 };
            if step % 4 == 3 {
                scene.rest();
            }
            scene.arrange(area, all[0]);
            let began = std::time::Instant::now();
            scene.want();
            let mut capture = None;
            while began.elapsed() < std::time::Duration::from_secs(2) && capture.is_none() {
                pump();
                capture = scene.ready();
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let Some(capture) = capture else {
                reds.push(-1);
                continue;
            };
            // The red window's middle, as the frame holds it.
            let seen = capture.luminances(RECT { left: 1400, top: 750, right: 1500, bottom: 850 }, RECT::default(), 10);
            let mean = seen.iter().sum::<f32>() / seen.len().max(1) as f32;
            reds.push((mean * 100.0).round() as i32);
        }
        unsafe {
            let _ = DestroyWindow(red);
        }
        // (Pure red is 21 light.)
        println!("the red window's light in each first frame: {reds:?}");
        assert!(reds.iter().all(|light| (18..=24).contains(light)), "{reds:?}");
    }

    /// What is behind an area reaches as far as the screens it is on, not the desktop's whole box: a widget at the top
    /// of a screen lower than the one beside it has no screen above it.
    #[test]
    fn behind_reaches_the_screens_met() {
        let rect = |left, top, right, bottom| RECT { left, top, right, bottom };
        let monitors = [rect(-2905, -1459, -745, 2381), rect(-745, -1440, 2695, 0), rect(0, 0, 1920, 1080)];
        assert_eq!(screens_of(rect(512, -1440, 2048, -1300), &monitors), Some(monitors[1]));
        // Across two: both.
        assert_eq!(screens_of(rect(-800, -1000, -700, -900), &monitors), Some(rect(-2905, -1459, 2695, 2381)));
        assert_eq!(screens_of(rect(5000, 5000, 5100, 5100), &monitors), None);
    }

    /// A window is put in a scene as opaque as it is drawn: a layered one at its opacity (one at none, as an overlay
    /// kept up and showing nothing is, as none at all), any other whole.
    #[test]
    fn a_window_is_as_opaque_as_it_is_drawn() {
        use windows::Win32::UI::WindowsAndMessaging::{SetLayeredWindowAttributes, LWA_ALPHA, WS_EX_LAYERED};
        let made = |layered: Option<u8>| unsafe {
            unsafe extern "system" fn answer(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
                unsafe { DefWindowProcW(window, message, wparam, lparam) }
            }
            let class = WNDCLASSW { lpfnWndProc: Some(answer), lpszClassName: w!("GlanceOpacityTest"), ..Default::default() };
            RegisterClassW(&class);
            let ex = if layered.is_some() { WS_EX_LAYERED } else { WINDOW_EX_STYLE(0) };
            let window = CreateWindowExW(ex, class.lpszClassName, w!(""), WS_POPUP, -32000, -32000, 10, 10, None, None, None, None).expect("a window");
            if let Some(alpha) = layered {
                SetLayeredWindowAttributes(window, windows::Win32::Foundation::COLORREF(0), alpha, LWA_ALPHA).expect("its opacity set");
            }
            window
        };
        for (layered, opacity) in [(None, 255), (Some(0), 0), (Some(128), 128), (Some(255), 255)] {
            let window = made(layered);
            assert_eq!(opacity_of(window), opacity, "{layered:?}");
            unsafe {
                let _ = DestroyWindow(window);
            }
        }
    }

    /// A scene's window is taken by the capture, and is in no task switcher (Alt+Tab, the task view), which lists a
    /// window shown that is not a tool window and is owned by none or by one that is not a tool window (measured on
    /// Windows 10 and 11, owned by one never shown); it stays as another program asks it to close. (Made a tool
    /// window, or owned by one, before it is taken, it is not taken on Windows 10 nor on Windows 11 before 26300.)
    #[test]
    fn a_scenes_window_is_taken_and_in_no_task_switcher() {
        use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, IsWindow, SendMessageW, GA_ROOTOWNER, WS_EX_APPWINDOW};
        let (owner, window, item) = windows(RECT { left: 100, top: 100, right: 300, bottom: 200 }).expect("its windows, taken by the capture");
        let style = |window: HWND| WINDOW_EX_STYLE(unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) } as u32);
        unsafe {
            assert!(item.Size().is_ok_and(|size| size.Width > 0));
            assert!(IsWindowVisible(window).as_bool() && cloaked(window));
            let root = GetAncestor(window, GA_ROOTOWNER);
            assert_eq!(root, owner);
            assert!(style(root).contains(WS_EX_TOOLWINDOW), "its owner is no tool window: it is listed in the task switcher");
            assert!(!style(window).contains(WS_EX_APPWINDOW));
            SendMessageW(window, WM_CLOSE, None, None);
            assert!(IsWindow(Some(window)).as_bool(), "closed by another program's asking");
            drop(item);
            let _ = DestroyWindow(window);
            let _ = DestroyWindow(owner);
        }
    }

    /// A scene arranged again and again at another place and size (as a widget moved and shown), one frame taken
    /// after each: how many of those were not yet of what is behind (dark, where the windows behind are light).
    /// `cargo test --lib -- --ignored scene_first_frames --nocapture`.
    #[test]
    #[ignore = "captures this PC's desktop"]
    fn scene_first_frames() {
        let _device = crate::ui::gfx::held().unwrap();
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let gfx = super::super::gfx::Gfx::new().expect("graphics");
        let mut all: Vec<HWND> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(each), LPARAM(&mut all as *mut _ as isize));
        }
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        let mut scene = Scene::new(RECT { left: 100, top: 100, right: 400, bottom: 300 }, wake, &gfx).expect("scene");
        let pump = || unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE};
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        };
        let mut means = Vec::new();
        let mut arranged = Vec::new();
        for step in 0..60 {
            let (x, grow) = (100 + (step % 7) * 53, (step % 5) * 61);
            let area = RECT { left: x, top: 100 + grow / 2, right: x + 300 + grow, bottom: 300 + grow };
            if step % 10 == 9 {
                scene.rest();
            }
            let arranging = std::time::Instant::now();
            scene.arrange(area, all[0]);
            arranged.push(arranging.elapsed().as_secs_f64() * 1000.0);
            let began = std::time::Instant::now();
            scene.want();
            let mut capture = None;
            while began.elapsed() < std::time::Duration::from_secs(2) && capture.is_none() {
                pump();
                capture = scene.ready();
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let Some(capture) = capture else {
                means.push(-1.0);
                continue;
            };
            let seen = capture.luminances(area, RECT::default(), 7);
            means.push(seen.iter().sum::<f32>() / seen.len() as f32);
        }
        let dark = means.iter().filter(|mean| (0.0..0.02).contains(*mean)).count();
        arranged.sort_by(f64::total_cmp);
        println!("arranging took {:.2} ms at the median, {:.2} at most", arranged[arranged.len() / 2], arranged[arranged.len() - 1]);
        println!("first frames: {dark} of {} dark; none {}; {:?}", means.len(), means.iter().filter(|m| **m < 0.0).count(), means.iter().map(|m| (m * 100.0).round() as i32).collect::<Vec<_>>());
    }

    /// What a scene with nothing behind it holds (its window's own surface, which nothing draws), beside one with the
    /// desktop behind it: `cargo test --lib -- --ignored scene_shows --nocapture`.
    #[test]
    #[ignore = "captures this PC's desktop"]
    fn scene_shows() {
        let _device = crate::ui::gfx::held().unwrap();
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let gfx = super::super::gfx::Gfx::new().expect("graphics");
        let area = RECT { left: 100, top: 100, right: 700, bottom: 500 };
        for (name, over) in [("bare", HWND(usize::MAX as *mut _)), ("desktop", HWND::default())] {
            let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
            let mut scene = Scene::new(area, wake, &gfx).expect("scene");
            // (Over nothing: the windows after the first are all behind it.)
            let over = if over.is_invalid() {
                let mut all: Vec<HWND> = Vec::new();
                unsafe {
                    let _ = EnumWindows(Some(each), LPARAM(&mut all as *mut _ as isize));
                }
                all[0]
            } else {
                over
            };
            let changed = scene.arrange(area, over);
            println!("{name}: arranged {changed}, {} thumbnails, area {:?}, frames {}x{}", scene.shown.len(), scene.area, scene.size.Width, scene.size.Height);
            for (window, _, rect, _) in &scene.shown {
                let mut title = [0u16; 80];
                let n = unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(*window, &mut title) };
                let mut class = [0u16; 80];
                let c = unsafe { windows::Win32::UI::WindowsAndMessaging::GetClassNameW(*window, &mut class) };
                println!("   behind: {:?} [{}] {:?}", String::from_utf16_lossy(&title[..n as usize]), String::from_utf16_lossy(&class[..c as usize]), rect);
            }
            let arranged = std::time::Instant::now();
            for taken in 0..3 {
                let began = std::time::Instant::now();
                scene.want();
                let mut capture = None;
                while began.elapsed() < std::time::Duration::from_secs(3) && capture.is_none() {
                    capture = scene.ready();
                    // (Its window's messages handled, as the thread that draws handles them.)
                    unsafe {
                        use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE};
                        let mut message = MSG::default();
                        while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                            let _ = TranslateMessage(&message);
                            DispatchMessageW(&message);
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                let Some(capture) = capture else {
                    println!("{name} {taken}: no frame in 3 s");
                    continue;
                };
                let seen = capture.luminances(capture.rect, RECT::default(), 9);
                let mean = seen.iter().sum::<f32>() / seen.len() as f32;
                let spread = (seen.iter().map(|l| (l - mean).powi(2)).sum::<f32>() / seen.len() as f32).sqrt();
                println!("{name} {taken}: at {:?} after arranging, mean {mean:.3}, spread {spread:.3}", arranged.elapsed());
            }
        }
    }

    /// How many times the window `changing_window` puts up has changed.
    static TICK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    /// What changes behind (for a frame to come again and again): a window of its own at (100, 100), 700 × 500,
    /// another colour every 15 ms, on a thread of its own.
    fn changing_window() {
        use windows::Win32::Graphics::Gdi::{BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, InvalidateRect, PAINTSTRUCT};
        use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, SetTimer, TranslateMessage, MSG, WM_PAINT, WM_TIMER, WS_VISIBLE};
        unsafe extern "system" fn painted(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
            unsafe {
                match message {
                    WM_PAINT => {
                        let mut paint = PAINTSTRUCT::default();
                        let dc = BeginPaint(window, &mut paint);
                        let tick = TICK.load(Ordering::Relaxed);
                        let colour = windows::Win32::Foundation::COLORREF((((tick * 37) % 256) | (((tick * 91) % 256) << 8) | (((tick * 53) % 256) << 16)) as u32);
                        let brush = CreateSolidBrush(colour);
                        FillRect(dc, &paint.rcPaint, brush);
                        let _ = DeleteObject(brush.into());
                        let _ = EndPaint(window, &paint);
                        LRESULT(0)
                    }
                    WM_TIMER => {
                        TICK.fetch_add(1, Ordering::Relaxed);
                        let _ = InvalidateRect(Some(window), None, false);
                        LRESULT(0)
                    }
                    _ => DefWindowProcW(window, message, wparam, lparam),
                }
            }
        }
        let (sent, got) = std::sync::mpsc::channel::<isize>();
        std::thread::spawn(move || unsafe {
            let module = GetModuleHandleW(None).unwrap();
            let class = WNDCLASSW { lpfnWndProc: Some(painted), hInstance: module.into(), lpszClassName: w!("GlanceStressBehind"), ..Default::default() };
            RegisterClassW(&class);
            let window = CreateWindowExW(WINDOW_EX_STYLE(0), class.lpszClassName, w!("behind"), WS_POPUP | WS_VISIBLE, 100, 100, 700, 500, None, None, Some(module.into()), None).unwrap();
            SetTimer(Some(window), 1, 15, None);
            sent.send(window.0 as isize).unwrap();
            let mut message = MSG::default();
            while GetMessageW(&mut message, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        });
        got.recv().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    /// What a scene is to be over: a hidden window of its own (class `name`) put on top, every other window behind it.
    fn over_all(name: &str) -> HWND {
        use windows::Win32::UI::WindowsAndMessaging::{HWND_TOP, SWP_NOMOVE};
        unsafe {
            let module = GetModuleHandleW(None).unwrap();
            let name: Vec<u16> = name.encode_utf16().chain([0]).collect();
            let class = WNDCLASSW { lpfnWndProc: Some(procedure), hInstance: module.into(), lpszClassName: windows::core::PCWSTR(name.as_ptr()), ..Default::default() };
            RegisterClassW(&class);
            let over = CreateWindowExW(WINDOW_EX_STYLE(0), class.lpszClassName, w!("over"), WS_POPUP, 0, 0, 1, 1, None, None, Some(module.into()), None).unwrap();
            let _ = SetWindowPos(over, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            over
        }
    }

    /// A frame wanted again and again while one is read (slowly) starts no other read: the one read serves (one
    /// started for each want was some three reads at once behind a live backdrop, each drawn).
    /// `cargo test --lib -- --ignored --exact ui::scene::tests::wanted_while_one_is_read_no_other_is --nocapture`.
    #[test]
    #[ignore = "puts a window on this PC's screen and captures it"]
    fn wanted_while_one_is_read_no_other_is() {
        let _device = crate::ui::gfx::held().unwrap();
        use std::time::{Duration, Instant};
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        READ_DELAY_MS.store(40, Ordering::Relaxed);
        changing_window();
        let gfx = super::super::gfx::Gfx::new().expect("graphics");
        let over = over_all("GlanceWantedOver");
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        let mut scene = Scene::new(RECT { left: 150, top: 150, right: 450, bottom: 350 }, wake, &gfx).expect("scene");
        scene.arrange(RECT { left: 150, top: 150, right: 450, bottom: 350 }, over);
        let (began, mut most, mut ready) = (Instant::now(), 0, 0);
        while began.elapsed() < Duration::from_secs(2) {
            scene.want();
            most = most.max(scene.shared.reading.load(Ordering::Acquire));
            ready += usize::from(scene.ready().is_some());
            unsafe {
                use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE};
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        READ_DELAY_MS.store(0, Ordering::Relaxed);
        println!("{ready} read in 2 s; at most {most} being read at once");
        assert!(ready > 5, "{ready} read");
        assert_eq!(most, 1, "{most} read at once");
    }

    /// A frame wanted again as soon as the one wanted before is taken (the frame after a first, of a scene arranged
    /// anew, wanted to be sure of the windows behind) is read too, however long closing the one read takes: wanted
    /// while that was still being closed, it was taken for wanted while one is read, and never read.
    /// `cargo test --lib -- --ignored --exact ui::scene::tests::wanted_once_one_is_taken_another_is_read --nocapture`.
    #[test]
    #[ignore = "puts a window on this PC's screen and captures it"]
    fn wanted_once_one_is_taken_another_is_read() {
        let _device = crate::ui::gfx::held().unwrap();
        use std::time::{Duration, Instant};
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        CLOSE_DELAY_MS.store(std::env::var("SCENE_CLOSE_DELAY_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(30), Ordering::Relaxed);
        changing_window();
        let gfx = super::super::gfx::Gfx::new().expect("graphics");
        let over = over_all("GlanceWantedAgainOver");
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        let mut scene = Scene::new(RECT { left: 150, top: 150, right: 450, bottom: 350 }, wake, &gfx).expect("scene");
        scene.arrange(RECT { left: 150, top: 150, right: 450, bottom: 350 }, over);
        let pump = || unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE};
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        };
        let taken = |scene: &Scene| {
            let began = Instant::now();
            while began.elapsed() < Duration::from_secs(3) {
                pump();
                if scene.ready().is_some() {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            false
        };
        scene.want();
        let first = taken(&scene);
        // Wanted again at once, as the thread that draws does.
        scene.want();
        let second = taken(&scene);
        CLOSE_DELAY_MS.store(0, Ordering::Relaxed);
        assert!(first, "no first frame read");
        assert!(second, "the frame wanted once the first was taken was never read");
    }

    /// The desktop's feed lives as long as a scene shows the desktop through it, and ends with the last (its frames are
    /// the whole desktop's: kept with no one to show them, a skin chosen that does not see the desktop held them till
    /// Glance ended), the last let go of as a frame is put in the layers too. Needs a desktop drawn apart from its window (a live wallpaper), as on the owner's PC.
    /// `cargo test --lib -- --ignored --exact ui::scene::tests::the_feed_ends_with_the_last_scene_of_it --nocapture`.
    #[test]
    #[ignore = "captures this PC's desktop (needs a live wallpaper)"]
    fn the_feed_ends_with_the_last_scene_of_it() {
        let _device = crate::ui::gfx::held().unwrap();
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let gfx = super::super::gfx::Gfx::new().expect("graphics");
        let over = over_all("GlanceFeedEndsOver");
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        let area = RECT { left: 150, top: 150, right: 450, bottom: 350 };
        let mut first = Scene::new(area, wake.clone(), &gfx).expect("scene");
        let mut second = Scene::new(area, wake, &gfx).expect("scene");
        first.arrange(area, over);
        second.arrange(area, over);
        assert!(!FEEDS.lock().unwrap().is_empty(), "no feed made: the desktop here is not drawn apart (no live wallpaper)");
        drop(first);
        assert!(!FEEDS.lock().unwrap().is_empty(), "the feed ended while a scene still shows the desktop through it");
        second.rest();
        assert!(!FEEDS.lock().unwrap().is_empty(), "the feed ended as its last scene rested");
        // The last let go of while a frame is being put in the layers (the thread it comes on holds them a moment).
        let in_hand = FEEDS.lock().unwrap().first().map(|feed| feed.shared.live_layers()).unwrap_or_default();
        drop(second);
        assert!(FEEDS.lock().unwrap().is_empty(), "the feed kept with no scene left to show it");
        drop(in_hand);
        unsafe {
            let _ = DestroyWindow(over);
        }
    }

    /// A scene arranged at alternating places and sizes over a window that changes all the time (so frames keep
    /// coming), wanted, polled and rested, for a minute, its reads made slow: a watchdog reports any step that takes
    /// more than 5 s (the thread that draws hung on the capture's pool) and ends the test.
    /// `cargo test --lib -- --ignored --exact ui::scene::tests::scene_stress --nocapture`.
    #[test]
    #[ignore = "captures this PC's desktop"]
    fn scene_stress() {
        let _device = crate::ui::gfx::held().unwrap();
        use std::sync::atomic::{AtomicU64, AtomicUsize};
        use std::time::{Duration, Instant};
        use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE};
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        READ_DELAY_MS.store(std::env::var("SCENE_READ_DELAY_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(40), Ordering::Relaxed);
        changing_window();
        let gfx = super::super::gfx::Gfx::new().expect("graphics");
        let over = over_all("GlanceStressOver");
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        let mut scene = Scene::new(RECT { left: 150, top: 150, right: 450, bottom: 350 }, wake, &gfx).expect("scene");
        let pump = || unsafe {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        };
        // The watchdog: what step, in what, since when (ms from the start).
        let start = Instant::now();
        let step_at = Arc::new(AtomicU64::new(0));
        let doing = Arc::new(AtomicUsize::new(0));
        let step_no = Arc::new(AtomicU64::new(0));
        const DOING: [&str; 5] = ["idle", "arrange", "want", "rest", "ready/pump"];
        {
            let (step_at, doing, step_no) = (step_at.clone(), doing.clone(), step_no.clone());
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_millis(250));
                let since = start.elapsed().as_millis() as u64 - step_at.load(Ordering::Acquire);
                if since > 5000 {
                    println!("HANG: step {} stuck in {} for {} ms (at {:.1} s)", step_no.load(Ordering::Acquire), DOING[doing.load(Ordering::Acquire)], since, start.elapsed().as_secs_f64());
                    std::process::exit(2);
                }
            });
        }
        let mark = |what: usize| {
            doing.store(what, Ordering::Release);
            step_at.store(start.elapsed().as_millis() as u64, Ordering::Release);
        };
        let (mut most_arrange, mut most_rest, mut most_want, mut most_step) = (Duration::ZERO, Duration::ZERO, Duration::ZERO, Duration::ZERO);
        let (mut steps, mut taken, mut rests) = (0u64, 0u64, 0u64);
        while start.elapsed() < Duration::from_secs(60) {
            let step = Instant::now();
            step_no.store(steps, Ordering::Release);
            let (x, grow) = (120 + (steps as i32 % 7) * 37, (steps as i32 % 5) * 41);
            let area = RECT { left: x, top: 120 + grow / 2, right: x + 200 + grow, bottom: 300 + grow };
            if steps % 6 == 5 {
                mark(3);
                let t = Instant::now();
                scene.rest();
                most_rest = most_rest.max(t.elapsed());
                rests += 1;
            }
            mark(1);
            let t = Instant::now();
            scene.arrange(area, over);
            most_arrange = most_arrange.max(t.elapsed());
            // Wanted once (a frame come meanwhile read on another thread, else the next as it comes), and polled a
            // while: arranged again, often before a read is done.
            mark(2);
            let t = Instant::now();
            scene.want();
            most_want = most_want.max(t.elapsed());
            let polled = Instant::now();
            while polled.elapsed() < Duration::from_millis(15 + (steps % 4) * 20) {
                mark(4);
                pump();
                if scene.ready().is_some() {
                    taken += 1;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            mark(0);
            most_step = most_step.max(step.elapsed());
            steps += 1;
        }
        let frames = TICK.load(Ordering::Relaxed);
        println!(
            "NO HANG: {steps} steps, {rests} rests, {taken} frames taken, {frames} repaints behind, read delay {} ms; most: arrange {:.1} ms, rest {:.1} ms, want {:.1} ms, step {:.1} ms",
            READ_DELAY_MS.load(Ordering::Relaxed),
            most_arrange.as_secs_f64() * 1000.0,
            most_rest.as_secs_f64() * 1000.0,
            most_want.as_secs_f64() * 1000.0,
            most_step.as_secs_f64() * 1000.0
        );
        drop(scene);
    }
}
