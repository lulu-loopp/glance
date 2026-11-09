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
    MF_MEDIA_ENGINE_EVENT_ERROR, MF_MEDIA_ENGINE_READY_HAVE_METADATA, MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, MF_MT_AUDIO_NUM_CHANNELS, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_PD_DURATION, MF_SOURCE_READERF_ENDOFSTREAM,
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
/// What it plays and where it tells what happens, for it to be played anew on another card (see `follow`).
pub struct Player {
    engine: IMFMediaEngine,
    device: ID3D11Device,
    _manager: IMFDXGIDeviceManager,
    frames: Option<Frames>,
    path: std::path::PathBuf,
    told: (HWND, u32),
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
            let device = decoding_on(drawn_on)?;
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
            Ok(Player { engine, device, _manager: manager, frames: None, path: path.to_path_buf(), told: (hwnd, message), last: None })
        }
    }

    /// Its frames read from now on where the look's device `drawn_on` reads them, that device another than the one it
    /// read them on (one made anew after that was lost, or another adapter's, the look moved onto a screen that one
    /// drives), and its frame for now copied again there as it next advances, a paused one's too (one already copied
    /// is not, see `advance`). Till then it has no frame to draw: the one before was copied for the device left, and
    /// drawn as that device's. Decoded on another adapter, or on a device lost, it is played anew on `drawn_on`'s
    /// adapter from where it was, as loud, playing or paused as it was: a frame of one adapter's is not to be opened on
    /// another's, and an engine handed a device of another adapter as it plays decodes nothing more.
    pub fn follow(&mut self, drawn_on: &ID3D11Device) -> windows::core::Result<()> {
        let removed = unsafe { self.device.GetDeviceRemovedReason() }.is_err();
        if removed || crate::ui::gfx::adapter_of(&self.device) != crate::ui::gfx::adapter_of(drawn_on) {
            let (at, playing, volume) = (self.at(), self.playing(), unsafe { self.engine.GetVolume() });
            let anew = Player::new(&self.path, drawn_on, volume as f32, self.told.0, self.told.1)?;
            anew.seek(at);
            if playing {
                anew.play();
            }
            *self = anew;
        }
        if let Some(frames) = &mut self.frames {
            frames.read = None;
        }
        self.last = None;
        Ok(())
    }

    /// Whether it is a video whose frame for now has not been copied yet (see `advance`), or may be one, its engine not
    /// having read yet what it plays (one made anew, see `follow`): to be advanced till it is, paused or not. Taken for
    /// none till its size was known, a paused video played anew on another card had its frame timer stopped as its
    /// engine read, and its frame was never drawn. (One whose engine fails to read it is let go of as that is told.)
    pub fn owes_frame(&self) -> bool {
        let read = unsafe { self.engine.GetReadyState() } >= MF_MEDIA_ENGINE_READY_HAVE_METADATA.0 as u16;
        self.last.is_none() && (!read || self.video_size().is_some())
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

/// A device to decode on, made on the adapter the device `drawn_on` is of, with what decoding on the card asks for
/// (which the look's own device is not made with).
fn decoding_on(drawn_on: &ID3D11Device) -> windows::core::Result<ID3D11Device> {
    unsafe {
        let adapter = drawn_on.cast::<IDXGIDevice>()?.GetAdapter()?;
        let mut device: Option<ID3D11Device> = None;
        D3D11CreateDevice(&adapter, D3D_DRIVER_TYPE_UNKNOWN, Default::default(), D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, None)?;
        let device = device.ok_or_else(|| windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL))?;
        // (The engine decodes on it from threads of its own.)
        let _ = device.cast::<ID3D11Multithread>()?.SetMultithreadProtected(true);
        Ok(device)
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
    use std::time::{Duration, Instant};
    use windows::Win32::Graphics::Direct2D::{D2D1CreateDevice, D2D1_DEVICE_CONTEXT_OPTIONS_NONE};
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory6, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_GPU_PREFERENCE_MINIMUM_POWER};
    use windows::Win32::Media::MediaFoundation::{
        IMFSinkWriter, MFCreateMemoryBuffer, MFCreateSample, MFCreateSinkWriterFromURL, MFMediaType_Video, MFVideoFormat_H264, MFVideoFormat_RGB32, MFVideoInterlace_Progressive, MF_MT_AVG_BITRATE,
        MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_PIXEL_ASPECT_RATIO,
    };

    /// The graphics cards there are (Windows' renderer in software left out), the one taking least power first.
    fn cards() -> Vec<IDXGIAdapter1> {
        let factory: IDXGIFactory6 = unsafe { CreateDXGIFactory1() }.unwrap();
        (0..)
            .map_while(|index| unsafe { factory.EnumAdapterByGpuPreference::<IDXGIAdapter1>(index, DXGI_GPU_PREFERENCE_MINIMUM_POWER) }.ok())
            .filter(|adapter| unsafe { adapter.GetDesc1() }.is_ok_and(|desc| desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 == 0))
            .collect()
    }

    /// A device on `adapter` as the look's is made (with what Direct2D asks for), and a Direct2D context drawing on it.
    fn drawn_on(adapter: &IDXGIAdapter1) -> (ID3D11Device, ID2D1DeviceContext) {
        let mut device = None;
        unsafe { D3D11CreateDevice(adapter, D3D_DRIVER_TYPE_UNKNOWN, Default::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, None) }.unwrap();
        let device: ID3D11Device = device.unwrap();
        let dc = unsafe { D2D1CreateDevice(&device.cast::<IDXGIDevice>().unwrap(), None) }.and_then(|d2d| unsafe { d2d.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE) }).unwrap();
        (device, dc)
    }

    /// A video of two seconds written at `path`, 320 by 240, thirty frames a second, each a colour of its own.
    fn video(path: &Path) {
        const SIZE: (u32, u32) = (320, 240);
        const FRAME: i64 = 10_000_000 / 30;
        assert!(started(), "Media Foundation not started");
        let framed = |kind: &windows::core::GUID| -> windows::core::Result<windows::Win32::Media::MediaFoundation::IMFMediaType> {
            let made = unsafe { MFCreateMediaType()? };
            unsafe {
                made.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                made.SetGUID(&MF_MT_SUBTYPE, kind)?;
                made.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                made.SetUINT64(&MF_MT_FRAME_SIZE, (u64::from(SIZE.0) << 32) | u64::from(SIZE.1))?;
                made.SetUINT64(&MF_MT_FRAME_RATE, (30 << 32) | 1)?;
                made.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1 << 32) | 1)?;
            }
            Ok(made)
        };
        let written = (|| -> windows::core::Result<()> {
            let writer: IMFSinkWriter = unsafe { MFCreateSinkWriterFromURL(&HSTRING::from(path.as_os_str()), None, None)? };
            let out = framed(&MFVideoFormat_H264)?;
            unsafe { out.SetUINT32(&MF_MT_AVG_BITRATE, 800_000)? };
            let stream = unsafe { writer.AddStream(&out)? };
            unsafe { writer.SetInputMediaType(stream, &framed(&MFVideoFormat_RGB32)?, None)? };
            unsafe { writer.BeginWriting()? };
            let length = SIZE.0 * SIZE.1 * 4;
            for frame in 0..60u32 {
                let buffer = unsafe { MFCreateMemoryBuffer(length)? };
                let mut data = std::ptr::null_mut();
                unsafe { buffer.Lock(&mut data, None, None)? };
                let pixels = unsafe { std::slice::from_raw_parts_mut(data.cast::<u32>(), (SIZE.0 * SIZE.1) as usize) };
                pixels.fill(0xFF00_0000 | (frame * 4) << 16 | (255 - frame * 4) << 8 | 0x80);
                unsafe {
                    buffer.Unlock()?;
                    buffer.SetCurrentLength(length)?;
                }
                let sample = unsafe { MFCreateSample()? };
                unsafe {
                    sample.AddBuffer(&buffer)?;
                    sample.SetSampleTime(i64::from(frame) * FRAME)?;
                    sample.SetSampleDuration(FRAME)?;
                    writer.WriteSample(stream, &sample)?;
                }
            }
            unsafe { writer.Finalize() }
        })();
        written.expect("a video written");
    }

    /// Whether `player` copies a frame to where `drawn_on` reads it within two seconds.
    fn copies(player: &mut Player, drawn_on: &ID3D11Device) -> bool {
        let began = Instant::now();
        while began.elapsed() < Duration::from_secs(2) {
            if player.advance(drawn_on) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    /// The device the frame `player` gives to draw on `dc` is on (none: it gives none).
    fn frame_on(player: &mut Player, dc: &ID2D1DeviceContext) -> Option<ID3D11Device> {
        let (bitmap, _) = player.frame(dc)?;
        let surface = unsafe { bitmap.GetSurface() }.ok()?;
        unsafe { surface.cast::<ID3D11Texture2D>().ok()?.GetDevice() }.ok()
    }

    /// A video paused, the look gone onto another device of the same card (one made anew after the last was lost, say),
    /// has its frame copied there and drawn as that device's: it was drawn from the bitmap made on the device left, and
    /// copied again only as a new frame came, which none did while it was paused.
    #[test]
    fn a_paused_video_follows_the_looks_device() {
        let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED) };
        let path = std::env::temp_dir().join(format!("glance-test-{}-paused.mp4", std::process::id()));
        video(&path);
        let card = cards().into_iter().next().expect("a graphics card");
        let (first, first_dc) = drawn_on(&card);
        let mut player = Player::new(&path, &first, 0.0, HWND::default(), 0).expect("played");
        player.play();
        assert!(copies(&mut player, &first), "no frame copied");
        player.pause();
        while copies(&mut player, &first) {}
        assert_eq!(frame_on(&mut player, &first_dc), Some(first.clone()), "a frame not drawn as the device's it was copied for");
        let (second, second_dc) = drawn_on(&card);
        player.follow(&second).expect("followed");
        assert!(player.owes_frame(), "a paused video gone onto another device not drawn on till its frame is copied there");
        assert!(copies(&mut player, &second), "a paused video's frame not copied to the device gone onto");
        assert_eq!(frame_on(&mut player, &second_dc), Some(second), "a frame drawn as the device's left");
        drop(player);
        let _ = std::fs::remove_file(&path);
    }

    /// A video made anew paused, as one is that the look takes onto another card (see `follow`), owes its frame from
    /// the first, before its engine has read what it plays, till that frame is copied: taken for none till its size was
    /// known, the look stopped its frame timer meanwhile, and the frame was never drawn.
    #[test]
    fn a_paused_video_made_anew_owes_its_frame_till_it_is_copied() {
        let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED) };
        let path = std::env::temp_dir().join(format!("glance-test-{}-anew.mp4", std::process::id()));
        video(&path);
        let card = cards().into_iter().next().expect("a graphics card");
        let (device, _) = drawn_on(&card);
        let mut player = Player::new(&path, &device, 0.0, HWND::default(), 0).expect("played");
        player.seek(0.5);
        let began = Instant::now();
        let (mut owed, mut copied) = (true, false);
        while !copied && began.elapsed() < Duration::from_secs(2) {
            owed &= player.owes_frame();
            copied = player.advance(&device);
            std::thread::sleep(Duration::from_millis(5));
        }
        let paid = !player.owes_frame();
        drop(player);
        let _ = std::fs::remove_file(&path);
        assert!(owed, "a paused video made anew owed no frame before it was copied");
        assert!(copied, "a paused video made anew never had its frame copied");
        assert!(paid, "a frame copied still owed");
    }

    /// A video played, the look gone onto a screen another card drives (a laptop's own screen, its discrete card's
    /// unplugged), is decoded on that card from then on and its frames copied there: decoded on the card left, a frame
    /// of one card's could not be opened on the other's, and the video stood still as its sound played on.
    #[test]
    #[ignore = "needs two graphics cards"]
    fn a_video_follows_the_look_onto_another_card() {
        let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED) };
        let path = std::env::temp_dir().join(format!("glance-test-{}-carded.mp4", std::process::id()));
        video(&path);
        let cards = cards();
        assert!(cards.len() >= 2, "fewer than two graphics cards");
        let (first, _) = drawn_on(&cards[0]);
        let (second, second_dc) = drawn_on(&cards[1]);
        let mut player = Player::new(&path, &first, 0.0, HWND::default(), 0).expect("played");
        player.play();
        assert!(copies(&mut player, &first), "no frame copied");
        std::thread::sleep(Duration::from_millis(300));
        player.follow(&second).expect("followed");
        assert_eq!(crate::ui::gfx::adapter_of(&player.device), crate::ui::gfx::adapter_of(&second), "decoded on the card left");
        let copied = (0..10).filter(|_| copies(&mut player, &second)).count();
        assert_eq!(copied, 10, "frames not copied to the card gone onto");
        assert!(player.playing() && player.at() > 0.3, "played from its beginning again ({:.2})", player.at());
        assert_eq!(frame_on(&mut player, &second_dc), Some(second.clone()), "a frame drawn as the card's left");
        // Paused, and gone back: its frame for now copied there, where it was.
        player.pause();
        while copies(&mut player, &second) {}
        let at = player.at();
        let (back, back_dc) = drawn_on(&cards[0]);
        player.follow(&back).expect("followed");
        assert!(copies(&mut player, &back), "a paused video's frame not copied to the card gone onto");
        assert!(!player.playing() && (player.at() - at).abs() < 0.1, "not paused where it was ({:.2}, {at:.2})", player.at());
        assert_eq!(frame_on(&mut player, &back_dc), Some(back), "a paused frame drawn as the card's left");
        drop(player);
        let _ = std::fs::remove_file(&path);
    }

    /// A video is played on the graphics card and its frames are copied to where the look's device reads them; a
    /// sound is read through for how loud it is along it. (Files on the owner's machine: run by hand.)
    #[test]
    #[ignore = "plays files of the machine it runs on: GLANCE_VIDEO, GLANCE_SOUND"]
    fn files_play() {
        let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) };
        let _device = crate::ui::gfx::held().unwrap();
        let gfx = crate::ui::gfx::for_main_screen().unwrap();
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
