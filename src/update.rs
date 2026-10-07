//! Update checks: once a day, while the setting allows, Glance asks GitHub,
//! and its mirror on Gitee (which mainland China can reach), for the latest
//! release; nothing else of the user's is sent. A newer one
//! is offered in the settings (and once by the tray), and taken only when
//! the user asks: its installer is downloaded into a folder of
//! administrators' alone and run only if it is signed by whoever signed the
//! Glance that is running.

use std::cmp::Ordering;
use std::path::Path;
use std::sync::{Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts, INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
    WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2, WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3, WINHTTP_OPTION_SECURE_PROTOCOLS, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_STATUS_CODE,
};
use windows::Win32::Security::Cryptography::{CertCompareCertificateName, CERT_CONTEXT, PKCS_7_ASN_ENCODING, X509_ASN_ENCODING};
use windows::Win32::Security::WinTrust::{
    WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData, WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA,
    WINTRUST_DATA_0, WINTRUST_FILE_INFO, WTD_CHOICE_FILE, WTD_REVOKE_WHOLECHAIN, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_ProgramData, SHGetKnownFolderPath, ShellExecuteW, KF_FLAG_DEFAULT};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::ui::prefs::LanguagePref;
use crate::ui::text::Lang;

/// Where each site describes the latest release, in the same terms.
const SOURCES: [&str; 2] = [
    "https://api.github.com/repos/lulu-loopp/glance/releases/latest",
    "https://gitee.com/api/v5/repos/lulu-loopp/glance/releases/latest",
];
/// How often to ask. Glance asks as soon as it starts; an ask that got no
/// answer (offline, the network not yet up at sign-in, both sites out of
/// reach) is made again after each of these in turn, then every last one.
const EVERY: Duration = Duration::from_secs(24 * 60 * 60);
const RETRIES: [Duration; 3] = [Duration::from_secs(60), Duration::from_secs(5 * 60), Duration::from_secs(60 * 60)];
/// How long the second site is waited for once the first has answered.
const STRAGGLER: Duration = Duration::from_secs(3);
/// How long a request waits to find, reach, and hear from a site: a site
/// out of reach is given up in seconds, not the minute and more Windows
/// waits by itself. Downloading the installer reads on as long as data
/// comes.
const RESOLVE_MS: i32 = 10_000;
const CONNECT_MS: i32 = 10_000;
const ANSWER_MS: i32 = 15_000;
/// More than any installer of Glance's: an answer this large is not one.
const MOST: usize = 64 << 20;

/// A release newer than this Glance.
#[derive(Clone)]
pub struct Release {
    pub version: String,
    /// Each site's latest version and where its installer is: the newest
    /// first and, of the same version, the site that answered soonest (the
    /// one most easily reached from where the user is). `version` is the
    /// first's.
    installers: Vec<(String, String)>,
}

/// Where taking an update stands.
#[derive(Clone, Copy, PartialEq)]
pub enum State {
    Ready,
    Downloading,
    /// It could not be downloaded; it may be tried again.
    Failed,
    /// What was downloaded is not signed as this Glance is, or is not the
    /// version announced: not run.
    Rejected,
}

static FOUND: Mutex<Option<(Release, State)>> = Mutex::new(None);

/// How the last ask went, for the settings to tell.
#[derive(Clone, Copy, PartialEq)]
pub enum Check {
    /// Not asked yet.
    Idle,
    Checking,
    /// The sites answered; nothing newer, or what `available` tells.
    Answered,
    /// Neither site answered.
    Unanswered,
}

/// The last ask, and whether one has been asked for now (and by whom: the
/// tray's asker is told the outcome in a notification).
struct Asking {
    check: Check,
    now: Option<Asker>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Asker {
    Settings,
    Tray,
}

static ASKING: Mutex<Asking> = Mutex::new(Asking { check: Check::Idle, now: None });
static WAKE: Condvar = Condvar::new();

/// How the last ask for a newer release went.
pub fn check() -> Check {
    ASKING.lock().unwrap().check
}

/// Asks for the latest release at once, whatever the daily asking is set
/// to.
pub fn check_now(asker: Asker) {
    let mut asking = ASKING.lock().unwrap();
    if asking.check == Check::Checking {
        return;
    }
    asking.now = Some(asker);
    asking.check = Check::Checking;
    WAKE.notify_all();
}

/// The newer release found, if any, and where taking it stands.
pub fn available() -> Option<(String, State)> {
    FOUND.lock().unwrap().as_ref().map(|(release, state)| (release.version.clone(), *state))
}

/// Asks for the latest release now and then, for as long as Glance runs,
/// and whenever asked to.
pub fn watch() {
    // When to ask next, and how many asks in a row went unanswered.
    let mut due = Instant::now();
    let mut unanswered = 0usize;
    loop {
        // Asked for now, or due and wanted. Otherwise a minute's wait at
        // most, so that turning the daily asking on is soon heard.
        let asker = {
            let mut asking = ASKING.lock().unwrap();
            loop {
                if let Some(asker) = asking.now.take() {
                    break Some(asker);
                }
                let wanted = crate::app().settings.lock().unwrap().check_updates;
                if wanted && Instant::now() >= due {
                    asking.check = Check::Checking;
                    break None;
                }
                let wait = due.saturating_duration_since(Instant::now()).min(Duration::from_secs(60)).max(Duration::from_secs(1));
                asking = WAKE.wait_timeout(asking, wait).unwrap().0;
            }
        };
        let answer = latest();
        if answer.is_some() {
            unanswered = 0;
            due = Instant::now() + EVERY;
        } else {
            due = Instant::now() + RETRIES[unanswered.min(RETRIES.len() - 1)];
            unanswered += 1;
        }
        let answered = answer.is_some();
        ASKING.lock().unwrap().check = if answered { Check::Answered } else { Check::Unanswered };
        let newer_release = answer.filter(|release| newer(&release.version, env!("CARGO_PKG_VERSION")));
        let lang = language();
        if let Some(release) = newer_release {
            let mut found = FOUND.lock().unwrap();
            let new = found.as_ref().is_none_or(|(known, _)| known.version != release.version);
            if new {
                let version = release.version.clone();
                crate::journal::note(format!("update available: {version}"));
                *found = Some((release, State::Ready));
                drop(found);
                // Asked from the settings, the settings show it.
                if asker != Some(Asker::Settings) {
                    let (title, text) = match lang {
                        Lang::Zh => (format!("Glance {version} 可用"), "点这里，在设置中更新。".to_string()),
                        Lang::En => (format!("Glance {version} is available"), "Click here to update from the settings.".to_string()),
                    };
                    crate::tray::notify(&title, &text);
                }
            }
        } else if asker == Some(Asker::Tray) {
            // Asked from the tray: the answer, either way.
            let version = env!("CARGO_PKG_VERSION");
            let (title, text) = match (lang, answered) {
                (Lang::Zh, true) => ("Glance 已是最新版本".to_string(), format!("当前版本 {version}。")),
                (Lang::En, true) => ("Glance is up to date".to_string(), format!("You have {version}.")),
                (Lang::Zh, false) => ("没能检查更新".to_string(), "GitHub 和 Gitee 都没有回应，请稍后再试。".to_string()),
                (Lang::En, false) => ("Couldn't check for updates".to_string(), "Neither GitHub nor Gitee answered; try again later.".to_string()),
            };
            crate::tray::notify(&title, &text);
        }
    }
}

/// Says so once if this is the first start since Glance was updated, and
/// notes the version that runs. Settings from before versions were noted
/// (0.1.8 and earlier) count as an update; none at all, as a first install.
pub fn tell_if_updated() {
    let app = crate::app();
    let version = env!("CARGO_PKG_VERSION");
    let mut settings = app.settings.lock().unwrap().clone();
    let updated = match settings.last_version.as_deref() {
        Some(last) => newer(version, last),
        None => crate::settings::saved(&app.config),
    };
    if settings.last_version.as_deref() == Some(version) {
        return;
    }
    settings.last_version = Some(version.to_string());
    app.save(settings);
    if updated {
        let (title, text) = match language() {
            Lang::Zh => (format!("Glance 已更新到 {version}"), "设置都已保留。".to_string()),
            Lang::En => (format!("Glance is updated to {version}"), "Your settings are as they were.".to_string()),
        };
        crate::tray::notify(&title, &text);
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
    let path_for = |version: &str| folder.path().join(format!("Glance_{version}_x64-setup.exe"));
    if !crate::elevation::remember_download(&path_for(&release.version)) {
        note("the installer could not be written");
        return State::Failed;
    }
    let Ok(running) = std::env::current_exe() else { return State::Failed };
    // From each site in turn, until one gives an installer that is what it
    // should be: signed as this Glance is, of the version that site
    // announced, and newer than this one. No site is trusted for any of it:
    // an older signed installer under a newer name would take Glance back,
    // and a site claiming a version that is not out keeps no other from
    // being tried.
    let mut chosen = None;
    let mut outcome = State::Failed;
    for (version, url) in &release.installers {
        let host = url.trim_start_matches("https://").split('/').next().unwrap_or(url);
        let Some(bytes) = get(url) else {
            note(&format!("the installer could not be downloaded from {host}"));
            continue;
        };
        let installer = path_for(version);
        if std::fs::write(&installer, bytes).is_err() {
            note("the installer could not be written");
            return State::Failed;
        }
        if !same_signer(&installer, &running) {
            note(&format!("the installer from {host} is not signed as this Glance is; not run"));
            outcome = State::Rejected;
            continue;
        }
        let carried = installer_version(&installer);
        let announced = carried.as_deref().is_some_and(|carried| !newer(carried, version) && !newer(version, carried));
        if !announced || !newer(version, env!("CARGO_PKG_VERSION")) {
            note(&format!("the installer from {host} is version {}, not {version}; not run", carried.as_deref().unwrap_or("unknown")));
            outcome = State::Rejected;
            continue;
        }
        chosen = Some(installer);
        break;
    }
    let Some(installer) = chosen else { return outcome };
    let path = HSTRING::from(installer.as_os_str());
    // Silent, and Glance started again once it is in: the one press in the
    // settings is all an update asks. (Glance runs elevated, and so does the
    // installer it starts, unasked.)
    let started = unsafe { ShellExecuteW(None::<HWND>, w!("open"), &path, w!("/S /RESTART"), PCWSTR::null(), SW_SHOWNORMAL) };
    // Above 32: the installer is running.
    if started.0 as usize > 32 {
        note("installer started");
        State::Ready
    } else {
        note("the installer could not be started");
        State::Failed
    }
}

/// The product version an installer carries in its version resource
/// ("0.1.7" for 0.1.7.0).
fn installer_version(file: &Path) -> Option<String> {
    let path = HSTRING::from(file.as_os_str());
    let size = unsafe { GetFileVersionInfoSizeW(&path, None) };
    if size == 0 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    unsafe { GetFileVersionInfoW(&path, None, size, data.as_mut_ptr().cast()) }.ok()?;
    let (mut fixed, mut length) = (std::ptr::null_mut(), 0u32);
    if !unsafe { VerQueryValueW(data.as_ptr().cast(), w!("\\"), &mut fixed, &mut length) }.as_bool() || (length as usize) < size_of::<VS_FIXEDFILEINFO>() {
        return None;
    }
    let fixed = unsafe { &*(fixed as *const VS_FIXEDFILEINFO) };
    let parts = [fixed.dwProductVersionMS >> 16, fixed.dwProductVersionMS & 0xFFFF, fixed.dwProductVersionLS >> 16, fixed.dwProductVersionLS & 0xFFFF];
    let mut parts: Vec<String> = parts.iter().map(u32::to_string).collect();
    while parts.len() > 1 && parts.last().is_some_and(|part| part == "0") {
        parts.pop();
    }
    Some(parts.join("."))
}

/// The machine's ProgramData folder.
fn program_data() -> Option<std::path::PathBuf> {
    let path = unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramData, KF_FLAG_DEFAULT, None) }.ok()?;
    let text = unsafe { path.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(path.0 as *const _)) };
    text.map(std::path::PathBuf::from)
}

/// The language the user reads Glance in.
pub(crate) fn language() -> Lang {
    let settings = crate::app().settings.lock().unwrap();
    Lang::resolve(serde_json::from_value::<LanguagePref>(settings.view["language"].clone()).unwrap_or_default())
}

/// The latest release each site knows of, the newest first. A site that
/// does not answer (out of reach from where the user is) is noted, and the
/// other one serves.
fn latest() -> Option<Release> {
    // Both sites at once, each on a thread of its own; their answers in the
    // order they come, a site out of reach answering nothing. Once one has
    // answered, the other is waited for a moment more, not for as long as
    // an unreachable site takes to give up.
    let (sender, answers) = std::sync::mpsc::channel();
    for url in SOURCES {
        let sender = sender.clone();
        thread::spawn(move || {
            let answer = get(url).and_then(|body| release(&body));
            if answer.is_none() {
                let host = url.trim_start_matches("https://").split('/').next().unwrap_or(url);
                crate::journal::note(format!("update check: no answer from {host}"));
            }
            let _ = sender.send(answer);
        });
    }
    drop(sender);
    let mut installers: Vec<(String, String)> = Vec::new();
    let mut heard = 0;
    while heard < SOURCES.len() {
        let next = if installers.is_empty() { answers.recv().ok() } else { answers.recv_timeout(STRAGGLER).ok() };
        let Some(answer) = next else { break };
        heard += 1;
        installers.extend(answer);
    }
    // The newest first; of the same version, still the sooner site first.
    installers.sort_by(|(a, _), (b, _)| if newer(a, b) { Ordering::Less } else if newer(b, a) { Ordering::Greater } else { Ordering::Equal });
    let version = installers.first()?.0.clone();
    Some(Release { version, installers })
}

/// A site's description of a release (GitHub's and Gitee's read alike):
/// its version, and where its installer is.
fn release(body: &[u8]) -> Option<(String, String)> {
    let release: serde_json::Value = serde_json::from_slice(body).ok()?;
    let version = release["tag_name"].as_str()?.trim_start_matches('v').to_string();
    let name = format!("Glance_{version}_x64-setup.exe");
    let installer = release["assets"]
        .as_array()?
        .iter()
        .find(|asset| asset["name"].as_str() == Some(name.as_str()))?["browser_download_url"]
        .as_str()?
        .to_string();
    installer.starts_with("https://").then_some((version, installer))
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
        let _ = WinHttpSetTimeouts(session.0, RESOLVE_MS, CONNECT_MS, ANSWER_MS, ANSWER_MS);
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
    use super::{get, installer_version, latest, newer, release, same_signer};
    use std::path::Path;

    /// Over the network: the latest release is found, its installer
    /// downloaded, and its signer matched to the Glance installed here
    /// (`GLANCE_INSTALLED`, a signed glance.exe), not to another publisher's
    /// program (`OTHER_SIGNED`).
    #[test]
    #[ignore]
    fn finds_downloads_and_checks_the_latest_release() {
        let release = latest().expect("latest release");
        assert_eq!(release.installers.len(), 2, "both sites offer {}", release.version);
        let (_, mirror) = release.installers.iter().find(|(_, url)| url.starts_with("https://gitee.com/")).expect("the mirror's installer");
        let bytes = get(mirror).expect("installer from the mirror");
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

    /// The version an installer (`GLANCE_INSTALLER`, a released one) carries.
    #[test]
    #[ignore]
    fn reads_an_installers_version() {
        let installer = std::env::var("GLANCE_INSTALLER").unwrap();
        let version = std::env::var("GLANCE_INSTALLER_VERSION").unwrap();
        assert_eq!(installer_version(Path::new(&installer)).as_deref(), Some(version.as_str()));
        // A file with no version resource.
        assert_eq!(installer_version(Path::new("Cargo.toml")), None);
    }

    #[test]
    fn reads_either_sites_release() {
        // As Gitee answers: its own archives besides the files uploaded.
        let gitee = br#"{"tag_name":"v0.1.6","assets":[
            {"name":"Glance_0.1.6_x64-setup.exe","browser_download_url":"https://gitee.com/lulu-loopp/glance/releases/download/v0.1.6/Glance_0.1.6_x64-setup.exe"},
            {"name":"v0.1.6.zip","browser_download_url":"https://gitee.com/lulu-loopp/glance/archive/refs/tags/v0.1.6.zip"}]}"#;
        let (version, installer) = release(gitee).unwrap();
        assert_eq!(version, "0.1.6");
        assert!(installer.ends_with("/v0.1.6/Glance_0.1.6_x64-setup.exe"));
        // No installer for the version, or one not over HTTPS: nothing.
        assert!(release(br#"{"tag_name":"v0.1.7","assets":[{"name":"SHA256SUMS.txt","browser_download_url":"https://x/y"}]}"#).is_none());
        assert!(release(br#"{"tag_name":"v0.1.7","assets":[{"name":"Glance_0.1.7_x64-setup.exe","browser_download_url":"http://x/y"}]}"#).is_none());
        // An error page.
        assert!(release(br#"{"message":"Not Found"}"#).is_none());
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
