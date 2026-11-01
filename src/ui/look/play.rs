//! What plays in a look: a video or a sound, played by Windows' own media engine (Media Foundation's), a video's frames
//! drawn on the sheet itself (so they grow out of a tile and fade with it, as a picture does), and a sound's loudness
//! read through beforehand, to be drawn as the whole of it (see `loudness`).
//!
//! The engine decodes on a Direct3D device of its own, made on the graphics card the look draws on with what decoding
//! on the card asks for (which the look's own device is not made with). Each new frame is written into a texture the
//! two devices share, handed across by its keyed mutex (so the look's device reads it only once it is written), and
//! copied there into one of the look's own, which Direct2D draws.

use std::path::Path;
use std::sync::OnceLock;

use windows::core::{implement, Interface, BSTR, HSTRING};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_IGNORE, D2D1_PIXEL_FORMAT};
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap1, ID2D1DeviceContext, D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_PROPERTIES1};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11Multithread, ID3D11Texture2D, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
    D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{IDXGIDevice, IDXGIKeyedMutex, IDXGIResource, IDXGISurface};
use windows::Win32::Media::MediaFoundation::{
    IMFAttributes, IMFDXGIDeviceManager, IMFMediaEngine, IMFMediaEngineClassFactory, IMFMediaEngineNotify, IMFMediaEngineNotify_Impl, MFAudioFormat_Float, MFCreateAttributes, MFCreateDXGIDeviceManager,
    MFCreateMediaType, MFCreateSourceReaderFromURL, MFMediaType_Audio, MFStartup, CLSID_MFMediaEngineClassFactory, MFSTARTUP_FULL, MF_MEDIA_ENGINE_CALLBACK, MF_MEDIA_ENGINE_DXGI_MANAGER,
    MF_MEDIA_ENGINE_EVENT_ERROR, MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, MF_MT_AUDIO_NUM_CHANNELS, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_PD_DURATION, MF_SOURCE_READERF_ENDOFSTREAM,
    MF_SOURCE_READER_ALL_STREAMS, MF_SOURCE_READER_FIRST_AUDIO_STREAM, MF_SOURCE_READER_MEDIASOURCE, MF_VERSION,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

/// Media Foundation started, once for the process (whether it could be).
fn started() -> bool {
    static STARTED: OnceLock<bool> = OnceLock::new();
    *STARTED.get_or_init(|| unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }.is_ok())
}

/// What the engine says happens (its metadata read, playing, paused, ended, a moment reached, failed): told to the
/// window `hwnd` as `message`, WPARAM the event, from whichever thread the engine says it on.
#[implement(IMFMediaEngineNotify)]
struct Told {
    hwnd: isize,
    message: u32,
}

impl IMFMediaEngineNotify_Impl for Told_Impl {
    fn EventNotify(&self, event: u32, _param1: usize, _param2: u32) -> windows::core::Result<()> {
        let _ = unsafe { PostMessageW(Some(HWND(self.hwnd as *mut _)), self.message, WPARAM(event as usize), LPARAM(0)) };
        Ok(())
    }
}

/// Whether what the engine told (an event, as `Told` passes it on) is that it failed.
pub fn failed_event(event: usize) -> bool {
    event == MF_MEDIA_ENGINE_EVENT_ERROR.0 as usize
}

/// A file played: its engine, the device it decodes on, and where its frames go (made once its frames' size is known).
pub struct Player {
    engine: IMFMediaEngine,
    device: ID3D11Device,
    _manager: IMFDXGIDeviceManager,
    frames: Option<Frames>,
    /// The time of the frame last copied (100 ns), and whether one has been.
    last: Option<i64>,
}

/// Where a video's frames are written (the texture the devices share, the decoding device's hold on it) and read (the
/// look's device's hold on it, the texture of its own they are copied into, and that as Direct2D draws it).
struct Frames {
    size: (u32, u32),
    shared: ID3D11Texture2D,
    written: IDXGIKeyedMutex,
    read: Option<Read>,
}

struct Read {
    device: ID3D11Device,
    shared: ID3D11Texture2D,
    held: IDXGIKeyedMutex,
    own: ID3D11Texture2D,
    bitmap: Option<ID2D1Bitmap1>,
}

impl Player {
    /// The file at `path` made ready to play (not playing yet), `volume` loud (0 to 1), on the graphics card the device
    /// `drawn_on` is made on; what it tells, told to `hwnd` as `message`.
    pub fn new(path: &Path, drawn_on: &ID3D11Device, volume: f32, hwnd: HWND, message: u32) -> windows::core::Result<Player> {
        if !started() {
            return Err(windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL));
        }
        unsafe {
            let adapter = drawn_on.cast::<IDXGIDevice>()?.GetAdapter()?;
            let mut device: Option<ID3D11Device> = None;
            D3D11CreateDevice(&adapter, D3D_DRIVER_TYPE_UNKNOWN, Default::default(), D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, None)?;
            let device = device.ok_or_else(|| windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL))?;
            // (The engine decodes on it from threads of its own.)
            let _ = device.cast::<ID3D11Multithread>()?.SetMultithreadProtected(true);
            let mut token = 0;
            let mut manager: Option<IMFDXGIDeviceManager> = None;
            MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
            let manager = manager.ok_or_else(|| windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL))?;
            manager.ResetDevice(&device, token)?;
            let mut attributes: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut attributes, 3)?;
            let attributes = attributes.ok_or_else(|| windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL))?;
            let told: IMFMediaEngineNotify = Told { hwnd: hwnd.0 as isize, message }.into();
            attributes.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &told)?;
            attributes.SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &manager)?;
            attributes.SetUINT32(&MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32)?;
            let factory: IMFMediaEngineClassFactory = CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)?;
            let engine = factory.CreateInstance(0, &attributes)?;
            engine.SetAutoPlay(false)?;
            engine.SetVolume(f64::from(volume))?;
            engine.SetSource(&BSTR::from(path.to_string_lossy().as_ref()))?;
            Ok(Player { engine, device, _manager: manager, frames: None, last: None })
        }
    }

    pub fn play(&self) {
        let _ = unsafe { self.engine.Play() };
    }

    pub fn pause(&self) {
        let _ = unsafe { self.engine.Pause() };
    }

    /// Whether it plays now (asked to, and not at its end).
    pub fn playing(&self) -> bool {
        unsafe { !self.engine.IsPaused().as_bool() && !self.engine.IsEnded().as_bool() }
    }

    pub fn ended(&self) -> bool {
        unsafe { self.engine.IsEnded().as_bool() }
    }

    /// Where it is (seconds from its beginning).
    pub fn at(&self) -> f64 {
        unsafe { self.engine.GetCurrentTime() }
    }

    /// How long it is (seconds), once known (a stream with no end: never).
    pub fn length(&self) -> Option<f64> {
        let length = unsafe { self.engine.GetDuration() };
        (length.is_finite() && length > 0.0).then_some(length)
    }

    /// It goes to `seconds` from its beginning (playing on from there, where it plays).
    pub fn seek(&self, seconds: f64) {
        let _ = unsafe { self.engine.SetCurrentTime(seconds.max(0.0)) };
    }

    pub fn set_volume(&self, volume: f32) {
        let _ = unsafe { self.engine.SetVolume(f64::from(volume)) };
    }

    /// How large its frames are (px), where it is a video and that is known.
    pub fn video_size(&self) -> Option<(u32, u32)> {
        unsafe {
            if !self.engine.HasVideo().as_bool() {
                return None;
            }
            let (mut w, mut h) = (0, 0);
            self.engine.GetNativeVideoSize(Some(&mut w), Some(&mut h)).ok()?;
            (w > 0 && h > 0).then_some((w, h))
        }
    }

    /// Its frame for now copied to where the look's device `drawn_on` reads it, where there is a new one. Whether there
    /// was.
    pub fn advance(&mut self, drawn_on: &ID3D11Device) -> bool {
        let Some(size) = self.video_size() else { return false };
        // (A frame the engine has for this moment: its time; one already copied, none.)
        let Ok(time) = (unsafe { self.engine.OnVideoStreamTick() }) else { return false };
        if self.last == Some(time) {
            return false;
        }
        if self.frames.as_ref().is_none_or(|frames| frames.size != size) {
            self.frames = self.made_frames(size).ok();
        }
        let Some(frames) = &mut self.frames else { return false };
        if frames.read.as_ref().is_none_or(|read| read.device != *drawn_on) {
            frames.read = read_on(drawn_on, &frames.shared, size).ok();
        }
        let Some(read) = &mut frames.read else { return false };
        unsafe {
            // (Written by the engine under key 0, handed over as key 1; read under 1, handed back as 0: each step taken
            // whatever the one before came to, so the keys stay in turn.)
            if frames.written.AcquireSync(0, u32::MAX).is_err() {
                return false;
            }
            let whole = RECT { left: 0, top: 0, right: size.0 as i32, bottom: size.1 as i32 };
            let transferred = self.engine.TransferVideoFrame(&frames.shared, None, &whole, None).is_ok();
            let _ = frames.written.ReleaseSync(1);
            if read.held.AcquireSync(1, u32::MAX).is_err() {
                return false;
            }
            if transferred {
                if let Ok(context) = drawn_on.GetImmediateContext() {
                    context.CopyResource(&read.own, &read.shared);
                }
            }
            let _ = read.held.ReleaseSync(0);
            if transferred {
                self.last = Some(time);
            }
            transferred
        }
    }

    /// Its latest frame copied (see `advance`), as Direct2D draws it on `dc`, and how large it is (px); none before the
    /// first.
    pub fn frame(&mut self, dc: &ID2D1DeviceContext) -> Option<(ID2D1Bitmap1, (u32, u32))> {
        self.last?;
        let frames = self.frames.as_mut()?;
        let read = frames.read.as_mut()?;
        if read.bitmap.is_none() {
            let properties = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_IGNORE },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
                ..Default::default()
            };
            let surface = read.own.cast::<IDXGISurface>().ok()?;
            read.bitmap = unsafe { dc.CreateBitmapFromDxgiSurface(&surface, Some(&properties)) }.ok();
        }
        Some((read.bitmap.clone()?, frames.size))
    }

    /// The texture its frames are written into, `size` px, shared with another device.
    fn made_frames(&self, size: (u32, u32)) -> windows::core::Result<Frames> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: size.0,
            Height: size.1,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX.0 as u32,
        };
        let mut shared: Option<ID3D11Texture2D> = None;
        unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut shared))? };
        let shared = shared.ok_or_else(|| windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL))?;
        let written = shared.cast()?;
        Ok(Frames { size, shared, written, read: None })
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = unsafe { self.engine.Shutdown() };
    }
}

/// The shared texture `shared` (`size` px) opened on the device `device`, and a texture of its own to copy it into.
fn read_on(device: &ID3D11Device, shared: &ID3D11Texture2D, size: (u32, u32)) -> windows::core::Result<Read> {
    unsafe {
        let handle: HANDLE = shared.cast::<IDXGIResource>()?.GetSharedHandle()?;
        let mut opened: Option<ID3D11Texture2D> = None;
        device.OpenSharedResource(handle, &mut opened)?;
        let opened = opened.ok_or_else(|| windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL))?;
        let held = opened.cast()?;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: size.0,
            Height: size.1,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut own: Option<ID3D11Texture2D> = None;
        device.CreateTexture2D(&desc, None, Some(&mut own))?;
        let own = own.ok_or_else(|| windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL))?;
        Ok(Read { device: device.clone(), shared: opened, held, own, bitmap: None })
    }
}

/// How loud the sound in the file at `path` is along it, in `parts` equal parts of its length: each part's loudest
/// sample, made relative to the loudest of all (1), read through while it is wanted still (`still`). None where it cannot
/// be read through (or holds no sound), or it was wanted no more.
pub fn loudness(path: &Path, parts: usize, still: &dyn Fn() -> bool) -> Option<Vec<f32>> {
    if !started() || parts == 0 {
        return None;
    }
    unsafe {
        let reader = MFCreateSourceReaderFromURL(&HSTRING::from(path.as_os_str()), None).ok()?;
        let audio = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
        reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false).ok()?;
        reader.SetStreamSelection(audio, true).ok()?;
        let wanted = MFCreateMediaType().ok()?;
        wanted.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).ok()?;
        wanted.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float).ok()?;
        reader.SetCurrentMediaType(audio, None, &wanted).ok()?;
        let channels = reader.GetCurrentMediaType(audio).ok()?.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).ok()?.max(1) as usize;
        let length = u64::try_from(&reader.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION).ok()?).ok()? as f64;
        if length <= 0.0 {
            return None;
        }
        let mut peaks = vec![0f32; parts];
        loop {
            if !still() {
                return None;
            }
            let (mut flags, mut time) = (0u32, 0i64);
            let mut sample = None;
            reader.ReadSample(audio, 0, None, Some(&mut flags), Some(&mut time), Some(&mut sample)).ok()?;
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }
            let Some(sample) = sample else { continue };
            let lasts = sample.GetSampleDuration().unwrap_or(0) as f64;
            let Ok(buffer) = sample.ConvertToContiguousBuffer() else { continue };
            let (mut data, mut bytes) = (std::ptr::null_mut(), 0u32);
            if buffer.Lock(&mut data, None, Some(&mut bytes)).is_err() {
                continue;
            }
            let values = std::slice::from_raw_parts(data as *const f32, bytes as usize / 4);
            let frames = values.len() / channels;
            for (at, frame) in values.chunks_exact(channels).enumerate() {
                let when = time as f64 + lasts * at as f64 / frames.max(1) as f64;
                let part = ((when / length) * parts as f64).clamp(0.0, (parts - 1) as f64) as usize;
                let loud = frame.iter().fold(0f32, |most, value| most.max(value.abs()));
                peaks[part] = peaks[part].max(loud);
            }
            let _ = buffer.Unlock();
        }
        let loudest = peaks.iter().copied().fold(0f32, f32::max);
        (loudest > 0.0).then(|| peaks.iter().map(|peak| peak / loudest).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A video is played on the graphics card and its frames are copied to where the look's device reads them; a
    /// sound is read through for how loud it is along it. (Files on the owner's machine: run by hand.)
    #[test]
    #[ignore = "plays files of the machine it runs on: GLANCE_VIDEO, GLANCE_SOUND"]
    fn files_play() {
        let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) };
        let _device = crate::ui::gfx::held().unwrap();
        let gfx = crate::ui::gfx::current().unwrap();
        let d3d = gfx.d3d().unwrap();
        let video = std::path::PathBuf::from(std::env::var("GLANCE_VIDEO").unwrap());
        let mut player = Player::new(&video, &d3d, 0.0, HWND::default(), 0).unwrap();
        player.play();
        let mut copied = 0;
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(2) {
            copied += usize::from(player.advance(&d3d));
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        println!("video {:?} length {:?} at {:.2} frames copied {copied}", player.video_size(), player.length(), player.at());
        assert!(copied > 10 && player.video_size().is_some() && player.at() > 1.0);
        let sound = std::path::PathBuf::from(std::env::var("GLANCE_SOUND").unwrap());
        let start = std::time::Instant::now();
        let loud = loudness(&sound, 400, &|| true).unwrap();
        println!("sound read in {} ms, loudest parts {:?}", start.elapsed().as_millis(), &loud[..8]);
        assert_eq!(loud.len(), 400);
        assert!(loud.iter().any(|part| *part == 1.0));
    }
}
