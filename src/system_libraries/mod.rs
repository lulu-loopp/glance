//! The libraries of Windows Glance calls into that not every Windows has (see `Optional`), and whether this one has
//! each.

use std::sync::OnceLock;

use windows::core::HSTRING;
use windows::Win32::System::LibraryLoader::{LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};

include!("optional.rs");

impl Optional {
    /// Whether this Windows has the library: loaded from System32 the first time asked, and kept loaded, so its
    /// functions called after it are found in it.
    pub fn present(self) -> bool {
        static FOUND: [OnceLock<bool>; Optional::ALL.len()] = [const { OnceLock::new() }; Optional::ALL.len()];
        *FOUND[self as usize].get_or_init(|| unsafe { LoadLibraryExW(&HSTRING::from(self.file()), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }.is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::Optional;
    use windows::core::PCSTR;
    use windows::Win32::System::Diagnostics::Debug::{ImageDirectoryEntryToData, IMAGE_DIRECTORY_ENTRY, IMAGE_DIRECTORY_ENTRY_DELAY_IMPORT, IMAGE_DIRECTORY_ENTRY_IMPORT};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::SystemServices::IMAGE_IMPORT_DESCRIPTOR;
    use windows::Win32::System::WindowsProgramming::IMAGE_DELAYLOAD_DESCRIPTOR;

    /// The libraries every Windows 10 and 11 for desktops has, whatever its edition (N ones too) and the features
    /// turned on in it: the ones a program may have Windows load as it starts.
    const ON_EVERY_WINDOWS: [&str; 45] = [
        "advapi32.dll",
        "api-ms-win-core-realtime-l1-1-1.dll",
        "api-ms-win-core-synch-l1-2-0.dll",
        "api-ms-win-core-winrt-error-l1-1-0.dll",
        "api-ms-win-core-winrt-l1-1-0.dll",
        "api-ms-win-shcore-scaling-l1-1-1.dll",
        "api-ms-win-shcore-stream-winrt-l1-1-0.dll",
        "bcrypt.dll",
        "bcryptprimitives.dll",
        "cfgmgr32.dll",
        "combase.dll",
        "comdlg32.dll",
        "coremessaging.dll",
        "crypt32.dll",
        "d2d1.dll",
        "d3d11.dll",
        "d3dcompiler_47.dll",
        "dbghelp.dll",
        "dcomp.dll",
        "dwmapi.dll",
        "dwrite.dll",
        "dxgi.dll",
        "gdi32.dll",
        "hid.dll",
        "imm32.dll",
        "iphlpapi.dll",
        "kernel32.dll",
        "ntdll.dll",
        "ole32.dll",
        "oleaut32.dll",
        "pdh.dll",
        "powrprof.dll",
        "propsys.dll",
        "psapi.dll",
        "setupapi.dll",
        "shell32.dll",
        "shlwapi.dll",
        "tdh.dll",
        "user32.dll",
        "userenv.dll",
        "version.dll",
        "wevtapi.dll",
        "winhttp.dll",
        "wintrust.dll",
        "ws2_32.dll",
    ];

    /// The libraries this program (the tests', holding all of Glance's code, as its programs do) has Windows load as it
    /// starts, and the ones it has loaded when first called, from its tables of each: their names, in small letters.
    fn imported() -> (Vec<String>, Vec<String>) {
        let base = unsafe { GetModuleHandleW(None) }.unwrap().0;
        // The library names a table's entries (each `size` long, its name `name_at` into it) give, up to the empty one
        // that ends it.
        let names = |entry: IMAGE_DIRECTORY_ENTRY, size: usize, name_at: usize| -> Vec<String> {
            let mut length = 0u32;
            let mut at = unsafe { ImageDirectoryEntryToData(base, true, entry, &mut length) } as *const u8;
            let mut names = Vec::new();
            if at.is_null() {
                return names;
            }
            loop {
                let name = unsafe { at.add(name_at).cast::<u32>().read_unaligned() };
                if name == 0 {
                    return names;
                }
                let text = unsafe { PCSTR(base.cast::<u8>().add(name as usize)).to_string() }.unwrap();
                names.push(text.to_lowercase());
                at = unsafe { at.add(size) };
            }
        };
        (
            names(IMAGE_DIRECTORY_ENTRY_IMPORT, size_of::<IMAGE_IMPORT_DESCRIPTOR>(), std::mem::offset_of!(IMAGE_IMPORT_DESCRIPTOR, Name)),
            names(IMAGE_DIRECTORY_ENTRY_DELAY_IMPORT, size_of::<IMAGE_DELAYLOAD_DESCRIPTOR>(), std::mem::offset_of!(IMAGE_DELAYLOAD_DESCRIPTOR, DllNameRVA)),
        )
    }

    #[test]
    fn glance_starts_on_a_windows_without_the_libraries_not_every_windows_has() {
        let (at_start, when_called) = imported();
        assert!(at_start.iter().any(|name| name == "kernel32.dll"), "the table read is not the program's: {at_start:?}");
        let not_everywhere: Vec<&String> = at_start.iter().filter(|name| !ON_EVERY_WINDOWS.contains(&name.as_str())).collect();
        assert!(not_everywhere.is_empty(), "loaded as Glance starts, though not every Windows has them: {not_everywhere:?}");
        for optional in Optional::ALL {
            assert!(when_called.iter().any(|name| name == optional.file()), "{} not loaded when first called: {when_called:?}", optional.file());
        }
    }
}
