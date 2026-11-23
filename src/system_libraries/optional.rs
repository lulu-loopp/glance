// Read by build.rs too (`include!`), which has it link each of these to be loaded when first called: no more than
// plain Rust here.

/// The libraries of Windows that Glance calls into and that not every Windows has: the Host Compute Service's (there
/// only with the virtual machine platform turned on), Media Foundation's two (not on the N editions without their media
/// pack) and the wireless one's (not on a Windows without wireless). Each is loaded when first called, so Glance starts
/// on a Windows without it, and is called only where `present` finds it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Optional {
    HostCompute,
    MediaPlatform,
    MediaReadWrite,
    Wireless,
}

impl Optional {
    pub const ALL: [Optional; 4] = [Optional::HostCompute, Optional::MediaPlatform, Optional::MediaReadWrite, Optional::Wireless];

    /// The library's file, in System32.
    pub const fn file(self) -> &'static str {
        match self {
            Optional::HostCompute => "computecore.dll",
            Optional::MediaPlatform => "mfplat.dll",
            Optional::MediaReadWrite => "mfreadwrite.dll",
            Optional::Wireless => "wlanapi.dll",
        }
    }
}
