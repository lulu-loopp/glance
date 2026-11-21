//! Notifications of Windows' own (toasts), shown as Glance's: Glance is named to Windows as an app that shows them
//! (its name and icon under its own id, for the user alone: `HKCU\Software\Classes\AppUserModelId`), and as what is
//! started for what is pressed on them (a class of its own, `CustomActivator`), which Glance, running, answers itself.
//! Pressed with Glance not running, Windows starts it (`-Embedding`), and it answers once it has started.

use std::sync::OnceLock;

use windows::core::{implement, IUnknown, Interface, Ref, BOOL, GUID, HSTRING, PCWSTR};
use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows::Win32::Foundation::CLASS_E_NOAGGREGATION;
use windows::Win32::System::Com::{CoRegisterClassObject, IClassFactory, IClassFactory_Impl, CLSCTX_LOCAL_SERVER, REGCLS_MULTIPLEUSE};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{RegDeleteTreeW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
use windows::Win32::UI::Notifications::{INotificationActivationCallback, INotificationActivationCallback_Impl, NOTIFICATION_USER_INPUT_DATA};

/// The id Glance shows its notifications by (a debug build's its own, as its settings are: see `settings::config_dir`).
pub const APP: &str = if cfg!(debug_assertions) { "dev.weiyi.glance.debug" } else { "dev.weiyi.glance" };

/// The class Windows starts, or asks of Glance running, for what is pressed on a notification.
const ACTIVATOR: GUID = if cfg!(debug_assertions) { GUID::from_u128(0x90e8f494_9135_4478_8d45_9007509a0c6b) } else { GUID::from_u128(0x6509c47c_56a8_4170_88c8_8b714b0774d4) };

/// What was pressed on a notification: the arguments of what was pressed (a button's, or the notification's own), and
/// what was chosen in its boxes (by their ids).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Asked {
    pub args: String,
    pub inputs: Vec<(String, String)>,
}

impl Asked {
    /// What was chosen in the box `id`.
    pub fn input(&self, id: &str) -> Option<&str> {
        self.inputs.iter().find(|(key, _)| key == id).map(|(_, value)| value.as_str())
    }
}

/// What is told of what is pressed (see `register`).
static TOLD: OnceLock<Box<dyn Fn(Asked) + Send + Sync>> = OnceLock::new();

#[implement(INotificationActivationCallback)]
struct Activator;

impl INotificationActivationCallback_Impl for Activator_Impl {
    fn Activate(&self, _app: &PCWSTR, args: &PCWSTR, data: *const NOTIFICATION_USER_INPUT_DATA, count: u32) -> windows::core::Result<()> {
        let args = if args.is_null() { String::new() } else { unsafe { args.to_string() }.unwrap_or_default() };
        let inputs = if data.is_null() { &[][..] } else { unsafe { std::slice::from_raw_parts(data, count as usize) } };
        let inputs = inputs.iter().filter_map(|input| Some((unsafe { input.Key.to_string() }.ok()?, unsafe { input.Value.to_string() }.ok()?))).collect();
        if let Some(told) = TOLD.get() {
            told(Asked { args, inputs });
        }
        Ok(())
    }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, outer: Ref<IUnknown>, iid: *const GUID, object: *mut *mut core::ffi::c_void) -> windows::core::Result<()> {
        if !outer.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let activator: INotificationActivationCallback = Activator.into();
        unsafe { activator.query(iid, object) }.ok()
    }

    fn LockServer(&self, _lock: BOOL) -> windows::core::Result<()> {
        Ok(())
    }
}

/// Glance named to Windows as the app its notifications are of, and as what answers what is pressed on them: `told`
/// of each press. Called once, on a thread in the process's multithreaded apartment, which answers as long as Glance
/// runs.
pub fn register(told: impl Fn(Asked) + Send + Sync + 'static) -> windows::core::Result<()> {
    let _ = TOLD.set(Box::new(told));
    let exe = std::env::current_exe()?;
    let class = format!("{{{ACTIVATOR:?}}}");
    let app = format!(r"Software\Classes\AppUserModelId\{APP}");
    set(&app, "DisplayName", "Glance")?;
    if let Some(icon) = icon() {
        set(&app, "IconUri", &icon.to_string_lossy())?;
    }
    set(&app, "CustomActivator", &class)?;
    set(&format!(r"Software\Classes\CLSID\{class}\LocalServer32"), "", &format!("\"{}\"", exe.to_string_lossy()))?;
    let factory: IClassFactory = Factory.into();
    // (Kept registered as long as Glance runs: the cookie that would take it back is not needed.)
    unsafe { CoRegisterClassObject(&ACTIVATOR, &factory, CLSCTX_LOCAL_SERVER, REGCLS_MULTIPLEUSE) }?;
    Ok(())
}

/// The value `name` of the user's key `key` (made where it is not there) set to `value`.
fn set(key: &str, name: &str, value: &str) -> windows::core::Result<()> {
    let wide: Vec<u16> = value.encode_utf16().chain([0]).collect();
    let name = (!name.is_empty()).then(|| HSTRING::from(name));
    let name = name.as_ref().map_or(PCWSTR::null(), |name| PCWSTR(name.as_ptr()));
    unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key), name, REG_SZ.0, Some(wide.as_ptr().cast()), (wide.len() * 2) as u32) }.ok()
}

/// Glance named to Windows no more (as `register` names it), and the notifications it showed taken back: as it is
/// uninstalled. Whether nothing of it is left.
pub fn forget() -> bool {
    let _ = ToastNotificationManager::History().and_then(|history| history.ClearWithId(&HSTRING::from(APP)));
    let class = format!(r"Software\Classes\CLSID\{{{ACTIVATOR:?}}}");
    [format!(r"Software\Classes\AppUserModelId\{APP}"), class].iter().all(|key| {
        let deleted = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(key.as_str())) };
        deleted == ERROR_SUCCESS || deleted == ERROR_FILE_NOT_FOUND
    })
}

/// The picture of Glance's icon its notifications show (see `icon`).
pub const ICON_FILE: &str = "notification-icon.png";

/// Glance's icon as a picture its notifications show, written where Glance keeps what it can make again (where it is
/// not there as it is).
fn icon() -> Option<std::path::PathBuf> {
    const ICON: &[u8] = include_bytes!("../icons/128x128@2x.png");
    let path = crate::settings::local_dir().join(ICON_FILE);
    if std::fs::read(&path).ok().as_deref() != Some(ICON) {
        std::fs::create_dir_all(path.parent()?).ok()?;
        std::fs::write(&path, ICON).ok()?;
    }
    Some(path)
}

/// The notification `xml` shown, as `tag` of `group` (another of the same taking its place).
pub fn show(xml: &str, tag: &str, group: &str) -> windows::core::Result<()> {
    let document = XmlDocument::new()?;
    document.LoadXml(&HSTRING::from(xml))?;
    let toast = ToastNotification::CreateToastNotification(&document)?;
    toast.SetTag(&HSTRING::from(tag))?;
    toast.SetGroup(&HSTRING::from(group))?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP))?.Show(&toast)
}

/// The notification `tag` of `group` taken back, on the screen or among the notifications kept.
pub fn take_back(tag: &str, group: &str) -> windows::core::Result<()> {
    ToastNotificationManager::History()?.RemoveGroupedTagWithId(&HSTRING::from(tag), &HSTRING::from(group), &HSTRING::from(APP))
}

/// `text` as it is written in a notification's XML, an attribute's value or words.
pub fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // (Characters XML does not take: left out.)
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_written_as_xml_takes_it() {
        assert_eq!(escaped(r#"a<b>&"c" 'd'"#), "a&lt;b&gt;&amp;&quot;c&quot; &apos;d&apos;");
        assert_eq!(escaped("x\u{1}y\tz"), "xy\tz");
        let document = XmlDocument::new().unwrap();
        document.LoadXml(&HSTRING::from(format!("<toast launch=\"{}\"><visual><binding template=\"ToastGeneric\"><text>{}</text></binding></visual></toast>", escaped("open:\"<&>"), escaped("a & b < c")))).unwrap();
    }

    #[test]
    fn the_class_is_named_as_the_registry_writes_it() {
        assert_eq!(format!("{{{:?}}}", GUID::from_u128(0x6509c47c_56a8_4170_88c8_8b714b0774d4)), "{6509C47C-56A8-4170-88C8-8B714B0774D4}");
    }
}
