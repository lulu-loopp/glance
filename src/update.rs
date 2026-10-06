//! Update checks: once a day, while the setting allows, Glance asks GitHub
//! for its latest release; nothing else of the user's is sent. A newer one
//! is offered in the settings (and once by the tray), and taken only when
//! the user asks: its installer is downloaded into a folder of
//! administrators' alone and run only if it is signed by whoever signed the
//! Glance that is running.

use std::path::Path;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetOption, INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
    WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2, WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3, WINHTTP_OPTION_SECURE_PROTOCOLS, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_STATUS_CODE,
};
use windows::Win32::Security::Cryptography::{CertCompareCertificateName, CERT_CONTEXT, PKCS_7_ASN_ENCODING, X509_ASN_ENCODING};
use windows::Win32::Security::WinTrust::{
    WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData, WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA,
    WINTRUST_DATA_0, WINTRUST_FILE_INFO, WTD_CHOICE_FILE, WTD_REVOKE_WHOLECHAIN, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_ProgramData, SHGetKnownFolderPath, ShellExecuteW, KF_FLAG_DEFAULT};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::ui::prefs::LanguagePref;
use crate::ui::text::Lang;

const HOST: &str = "api.github.com";
const LATEST: &str = "/repos/lulu-loopp/glance/releases/latest";
/// How often to ask, and how soon after Glance starts (not while the
/// desktop is still coming up); an ask that got no answer (offline, or
/// GitHub out of reach) is made again sooner.
const EVERY: Duration = Duration::from_secs(24 * 60 * 60);
const RETRY: Duration = Duration::from_secs(60 * 60);
const FIRST: Duration = Duration::from_secs(60);
/// More than any installer of Glance's: an answer this large is not one.
const MOST: usize = 64 << 20;

/// A release newer than this Glance.
#[derive(Clone)]
pub struct Release {
    pub version: String,
    installer: String,
}

/// Where taking an update stands.
#[derive(Clone, Copy, PartialEq)]
pub enum State {
    Ready,
    Downloading,
    /// It could not be downloaded; it may be tried again.
    Failed,
    /// What was downloaded is not signed as this Glance is: not run.
    Unsigned,
}

static FOUND: Mutex<Option<(Release, State)>> = Mutex::new(None);

/// The newer release found, if any, and where taking it stands.
pub fn available() -> Option<(String, State)> {
    FOUND.lock().unwrap().as_ref().map(|(release, state)| (release.version.clone(), *state))
}

/// Asks for the latest release now and then, for as long as Glance runs.
pub fn watch() {
    // When to ask next.
    let mut due = Instant::now();
    thread::sleep(FIRST);
    loop {
        let wanted = crate::app().settings.lock().unwrap().check_updates;
        if wanted && Instant::now() >= due {
            let answer = latest();
            due = Instant::now() + if answer.is_some() { EVERY } else { RETRY };
            if answer.is_none() {
                crate::journal::note("update check: no answer from GitHub");
            }
            if let Some(release) = answer.filter(|release| newer(&release.version, env!("CARGO_PKG_VERSION"))) {
                let mut found = FOUND.lock().unwrap();
                let new = found.as_ref().is_none_or(|(known, _)| known.version != release.version);
                if new {
                    let version = release.version.clone();
                    crate::journal::note(format!("update available: {version}"));
                    *found = Some((release, State::Ready));
                    drop(found);
                    let (title, text) = match language() {
                        Lang::Zh => (format!("Glance {version} 可用"), "右键单击托盘图标，在设置中更新。".to_string()),
                        Lang::En => (format!("Glance {version} is available"), "Right-click the tray icon to update from the settings.".to_string()),
                    };
                    crate::tray::notify(&title, &text);
                }
            }
        }
        // A minute at a time, so that turning the setting on is soon heard.
        thread::sleep(Duration::from_secs(60));
    }
}

/// Downloads the newer release's installer and runs it, on a thread of its
/// own; the installer closes Glance as it replaces it.
pub fn install() {
    let release = {
        let mut found = FOUND.lock().unwrap();
        match found.as_mut() {
            Some((release, state)) if *state != State::Downloading => {
                *state = State::Downloading;
                release.clone()
            }
            _ => return,
        }
    };
    thread::spawn(move || {
        let outcome = fetch_and_run(&release);
        if let Some((_, state)) = FOUND.lock().unwrap().as_mut() {
            *state = outcome;
        }
    });
}

fn fetch_and_run(release: &Release) -> State {
    let note = |what: &str| crate::journal::note(format!("update to {}: {what}", release.version));
    let Some(bytes) = get(&release.installer) else {
        note("the installer could not be downloaded");
        return State::Failed;
    };
    // The one downloaded before, if it is still there.
    crate::elevation::forget_download();
    // In ProgramData, where no one but administrators can move anything
    // aside: a folder made there new, administrators' only, named afresh
    // each time, stays where it is while the installer is written, checked
    // and started, and after, while the installer reads itself again.
    let Some(program_data) = program_data() else {
        note("ProgramData could not be found");
        return State::Failed;
    };
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let name = format!("Glance-update-{}-{stamp}", std::process::id());
    let Some(folder) = crate::elevation::private_folder(&program_data, &name) else {
        note("no folder of administrators' alone could be made in ProgramData");
        return State::Failed;
    };
    let installer = folder.path().join(format!("Glance_{}_x64-setup.exe", release.version));
    if !crate::elevation::remember_download(&installer) || std::fs::write(&installer, bytes).is_err() {
        note("the installer could not be written");
        return State::Failed;
    }
    let Ok(running) = std::env::current_exe() else { return State::Failed };
    if !same_signer(&installer, &running) {
        note("the installer is not signed as this Glance is; not run");
        return State::Unsigned;
    }
    let path = HSTRING::from(installer.as_os_str());
    let started = unsafe { ShellExecuteW(None::<HWND>, w!("open"), &path, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    // Above 32: the installer is running.
    if started.0 as usize > 32 {
        note("installer started");
        State::Ready
    } else {
        note("the installer could not be started");
        State::Failed
    }
}

/// The machine's ProgramData folder.
fn program_data() -> Option<std::path::PathBuf> {
    let path = unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramData, KF_FLAG_DEFAULT, None) }.ok()?;
    let text = unsafe { path.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(path.0 as *const _)) };
    text.map(std::path::PathBuf::from)
}

/// The language the user reads Glance in.
fn language() -> Lang {
    let settings = crate::app().settings.lock().unwrap();
    Lang::resolve(serde_json::from_value::<LanguagePref>(settings.view["language"].clone()).unwrap_or_default())
}

/// The latest release, as GitHub describes it, and its installer.
fn latest() -> Option<Release> {
    let body = get(&format!("https://{HOST}{LATEST}"))?;
    let release: serde_json::Value = serde_json::from_slice(&body).ok()?;
    let version = release["tag_name"].as_str()?.trim_start_matches('v').to_string();
    let name = format!("Glance_{version}_x64-setup.exe");
    let installer = release["assets"]
        .as_array()?
        .iter()
        .find(|asset| asset["name"].as_str() == Some(name.as_str()))?["browser_download_url"]
        .as_str()?
        .to_string();
    installer.starts_with("https://").then_some(Release { version, installer })
}

/// Whether version `a` comes after `b`, compared number by number (1.10
/// after 1.9; a missing number counts as 0).
fn newer(a: &str, b: &str) -> bool {
    let numbers = |v: &str| -> Option<Vec<u64>> { v.split('.').map(|part| part.parse().ok()).collect() };
    let (Some(mut a), Some(mut b)) = (numbers(a), numbers(b)) else { return false };
    let length = a.len().max(b.len());
    a.resize(length, 0);
    b.resize(length, 0);
    a > b
}

/// A WinHTTP handle, closed when dropped.
struct Handle(*mut core::ffi::c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            let _ = unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

/// The body of an HTTPS GET of `url`, if it answered 200 (redirects
/// followed, never from HTTPS to HTTP).
fn get(url: &str) -> Option<Vec<u8>> {
    let rest = url.strip_prefix("https://")?;
    let (host, path) = rest.split_at(rest.find('/')?);
    let agent = HSTRING::from(format!("Glance/{}", env!("CARGO_PKG_VERSION")));
    unsafe {
        let session = Handle(WinHttpOpen(&agent, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0));
        if session.0.is_null() {
            return None;
        }
        let protocols = (WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2 | WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3).to_ne_bytes();
        // Older systems know no TLS 1.3: then 1.2 alone.
        if WinHttpSetOption(Some(session.0), WINHTTP_OPTION_SECURE_PROTOCOLS, Some(&protocols)).is_err() {
            let _ = WinHttpSetOption(Some(session.0), WINHTTP_OPTION_SECURE_PROTOCOLS, Some(&WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2.to_ne_bytes()));
        }
        let connection = Handle(WinHttpConnect(session.0, &HSTRING::from(host), INTERNET_DEFAULT_HTTPS_PORT, 0));
        if connection.0.is_null() {
            return None;
        }
        let request = Handle(WinHttpOpenRequest(connection.0, w!("GET"), &HSTRING::from(path), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), WINHTTP_FLAG_SECURE));
        if request.0.is_null() {
            return None;
        }
        let headers: Vec<u16> = "Accept: application/vnd.github+json, application/octet-stream\r\n".encode_utf16().collect();
        WinHttpSendRequest(request.0, Some(&headers), None, 0, 0, 0).ok()?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).ok()?;
        let mut status = 0u32;
        let mut size = size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status as *mut u32).cast()),
            &mut size,
            std::ptr::null_mut(),
        )
        .ok()?;
        if status != 200 {
            return None;
        }
        let mut body = Vec::new();
        let mut chunk = vec![0u8; 64 << 10];
        loop {
            let mut read = 0u32;
            WinHttpReadData(request.0, chunk.as_mut_ptr().cast(), chunk.len() as u32, &mut read).ok()?;
            if read == 0 {
                return Some(body);
            }
            body.extend_from_slice(&chunk[..read as usize]);
            if body.len() > MOST {
                return None;
            }
        }
    }
}

/// A file's Authenticode signature, verified (the whole chain, revocation
/// included), held open while its signer's certificate is looked at.
struct Signature {
    data: WINTRUST_DATA,
    _file: Box<WINTRUST_FILE_INFO>,
    _path: HSTRING,
}

impl Signature {
    fn verify(file: &Path) -> Option<Self> {
        let path = HSTRING::from(file.as_os_str());
        let mut info = Box::new(WINTRUST_FILE_INFO {
            cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
            pcwszFilePath: PCWSTR(path.as_ptr()),
            hFile: HANDLE::default(),
            pgKnownSubject: std::ptr::null_mut(),
        });
        let mut signature = Signature {
            data: WINTRUST_DATA {
                cbStruct: size_of::<WINTRUST_DATA>() as u32,
                dwUIChoice: WTD_UI_NONE,
                fdwRevocationChecks: WTD_REVOKE_WHOLECHAIN,
                dwUnionChoice: WTD_CHOICE_FILE,
                Anonymous: WINTRUST_DATA_0 { pFile: &mut *info },
                dwStateAction: WTD_STATEACTION_VERIFY,
                ..Default::default()
            },
            _file: info,
            _path: path,
        };
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        let trusted = unsafe { WinVerifyTrust(HWND::default(), &mut action, (&mut signature.data as *mut WINTRUST_DATA).cast()) } == 0;
        // Closed by Drop whether or not it was trusted.
        trusted.then_some(signature)
    }

    /// The certificate it was signed with.
    fn signer(&self) -> Option<&CERT_CONTEXT> {
        unsafe {
            let provider = WTHelperProvDataFromStateData(self.data.hWVTStateData);
            if provider.is_null() {
                return None;
            }
            let signer = WTHelperGetProvSignerFromChain(provider, 0, false, 0);
            if signer.is_null() || (*signer).csCertChain == 0 {
                return None;
            }
            (*(*signer).pasCertChain).pCert.as_ref()
        }
    }
}

impl Drop for Signature {
    fn drop(&mut self) {
        self.data.dwStateAction = WTD_STATEACTION_CLOSE;
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        unsafe { WinVerifyTrust(HWND::default(), &mut action, (&mut self.data as *mut WINTRUST_DATA).cast()) };
    }
}

/// Whether `file` and `reference` both carry valid signatures, by
/// certificates issued to the same subject (a signing service's
/// certificates last days; the subject they vouch for stays).
fn same_signer(file: &Path, reference: &Path) -> bool {
    let (Some(a), Some(b)) = (Signature::verify(file), Signature::verify(reference)) else { return false };
    let (Some(a), Some(b)) = (a.signer(), b.signer()) else { return false };
    unsafe { CertCompareCertificateName(X509_ASN_ENCODING | PKCS_7_ASN_ENCODING, &(*a.pCertInfo).Subject, &(*b.pCertInfo).Subject) }.as_bool()
}

#[cfg(test)]
mod tests {
    use super::{get, latest, newer, same_signer};
    use std::path::Path;

    /// Over the network: the latest release is found, its installer
    /// downloaded, and its signer matched to the Glance installed here
    /// (`GLANCE_INSTALLED`, a signed glance.exe), not to another publisher's
    /// program (`OTHER_SIGNED`).
    #[test]
    #[ignore]
    fn finds_downloads_and_checks_the_latest_release() {
        let release = latest().expect("latest release");
        let bytes = get(&release.installer).expect("installer");
        let file = std::env::temp_dir().join(format!("glance-update-test-{}.exe", std::process::id()));
        std::fs::write(&file, bytes).unwrap();
        let installed = std::env::var("GLANCE_INSTALLED").unwrap();
        let other = std::env::var("OTHER_SIGNED").unwrap();
        let ours = same_signer(&file, Path::new(&installed));
        let theirs = same_signer(&file, Path::new(&other));
        std::fs::remove_file(&file).unwrap();
        assert!(ours, "{} not signed as {installed}", release.version);
        assert!(!theirs);
    }

    #[test]
    fn compares_versions_number_by_number() {
        assert!(newer("0.1.4", "0.1.3"));
        assert!(newer("0.1.10", "0.1.9"));
        assert!(newer("1.0", "0.9.9"));
        assert!(!newer("0.1.3", "0.1.3"));
        assert!(!newer("0.1", "0.1.0"));
        assert!(!newer("0.1.2", "0.1.3"));
        assert!(!newer("0.2.0-beta", "0.1.3"));
    }
}
