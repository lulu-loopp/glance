//! Whether the microphone is muted: the system's default recording device,
//! as Windows' own sound settings switch it.

use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eCapture, eConsole, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

pub struct Microphone {
    devices: IMMDeviceEnumerator,
}

// The enumerator lives in the process's multithreaded apartment (see lib.rs)
// and is used from one thread at a time (the sampler's).
unsafe impl Send for Microphone {}

impl Microphone {
    pub fn open() -> Option<Self> {
        let devices = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok()?;
        Some(Microphone { devices })
    }

    /// Whether the default recording device is muted now; none without one.
    /// Looked up each time: the default can change while Glance runs.
    pub fn muted(&self) -> Option<bool> {
        unsafe {
            let device = self.devices.GetDefaultAudioEndpoint(eCapture, eConsole).ok()?;
            let volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None).ok()?;
            volume.GetMute().ok().map(|muted| muted.as_bool())
        }
    }
}

#[cfg(test)]
mod tests {
    /// Prints whether this machine's default microphone is muted.
    #[test]
    fn reads_this_machines_microphone() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let microphone = super::Microphone::open().expect("the audio device enumerator");
        println!("default microphone muted: {:?}", microphone.muted());
    }
}
