//! What Glance asks of Windows for its look.

use windows::core::w;
use windows::UI::ViewManagement::{UIColorType, UISettings};
use windows::Win32::Globalization::GetUserDefaultUILanguage;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

/// Whether the user's interface language is Chinese, in any of its variants
/// (by its primary language id).
pub fn speaks_chinese() -> bool {
    (unsafe { GetUserDefaultUILanguage() } & 0x3FF) == 0x04
}

/// Whether the system's apps are dark: as last read, read again as it is changed (it is asked for at every frame of
/// every widget; read each time, the registry was).
pub fn apps_dark() -> bool {
    use std::sync::atomic::{AtomicU8, Ordering};
    // 0: not read yet; 1 light; 2 dark.
    static DARK: AtomicU8 = AtomicU8::new(0);
    match DARK.load(Ordering::Relaxed) {
        0 => {
            let dark = read_apps_dark();
            if DARK.compare_exchange(0, if dark { 2 } else { 1 }, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
                std::thread::Builder::new().name("glance: theme watched".into()).spawn(move || watch_apps_dark(&DARK)).expect("a thread started");
            }
            dark
        }
        known => known == 2,
    }
}

/// Keeps `dark` as the system has it, told by the registry as the key changes (watched before it is read again:
/// no change goes unseen).
fn watch_apps_dark(dark: &std::sync::atomic::AtomicU8) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Registry::{RegCloseKey, RegNotifyChangeKeyValue, RegOpenKeyExW, HKEY, KEY_NOTIFY, KEY_READ, REG_NOTIFY_CHANGE_LAST_SET};
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject, INFINITE};
    let mut key = HKEY::default();
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"), None, KEY_NOTIFY | KEY_READ, &mut key) }.is_err() {
        return;
    }
    let Ok(changed) = (unsafe { CreateEventW(None, false, false, None) }) else {
        let _ = unsafe { RegCloseKey(key) };
        return;
    };
    loop {
        if unsafe { RegNotifyChangeKeyValue(key, false, REG_NOTIFY_CHANGE_LAST_SET, Some(changed), true) }.is_err() {
            break;
        }
        dark.store(if read_apps_dark() { 2 } else { 1 }, std::sync::atomic::Ordering::Relaxed);
        unsafe { WaitForSingleObject(changed, INFINITE) };
    }
    unsafe {
        let _ = CloseHandle(changed);
        let _ = RegCloseKey(key);
    }
}

/// Whether the system's apps are dark, read now.
fn read_apps_dark() -> bool {
    let mut light = 1u32;
    let mut size = 4u32;
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut light as *mut _ as *mut _),
            Some(&mut size),
        )
    };
    read.is_ok() && light == 0
}

/// The user's accent colour as 0xRRGGBB, in the shade the system pairs with
/// a dark or a light theme; `None` where the system will not say.
pub fn accent(dark: bool) -> Option<u32> {
    let shade = if dark { UIColorType::AccentLight2 } else { UIColorType::AccentDark1 };
    // In the process's multithreaded apartment, kept for as long as the process runs: the system's colour settings'
    // maker is kept by the bindings for the whole process, and an apartment that came and went with a thread (the
    // tests', one after another) took its library with it, the next asked faulting the process in it.
    static KEPT: std::sync::Once = std::sync::Once::new();
    KEPT.call_once(|| {
        let _ = unsafe { windows::Win32::System::Com::CoIncrementMTAUsage() };
    });
    let color = UISettings::new().and_then(|settings| settings.GetColorValue(shade)).ok()?;
    Some(((color.R as u32) << 16) | ((color.G as u32) << 8) | color.B as u32)
}

/// Whether two names are one to the file system: alike case aside, as NTFS compares them (`CompareStringOrdinal`).
pub fn names_alike(a: &str, b: &str) -> bool {
    use windows::Win32::Globalization::{CompareStringOrdinal, CSTR_EQUAL};
    let (a, b): (Vec<u16>, Vec<u16>) = (a.encode_utf16().collect(), b.encode_utf16().collect());
    (unsafe { CompareStringOrdinal(&a, &b, true) }) == CSTR_EQUAL
}

/// The file or folder at `from` given the name `to`, where nothing has it (one that does: `AlreadyExists`): moved, not
/// copied, on the same drive.
pub fn move_new(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVE_FILE_FLAGS};
    unsafe { MoveFileExW(&windows::core::HSTRING::from(from.as_os_str()), &windows::core::HSTRING::from(to.as_os_str()), MOVE_FILE_FLAGS(0)) }.map_err(|error| std::io::Error::from_raw_os_error(error.code().0 & 0xFFFF))
}
