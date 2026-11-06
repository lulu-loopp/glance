//! Update checks: once a day, while the setting allows, Glance asks GitHub,
//! and its mirror on Gitee (which mainland China can reach), for the latest
//! release; nothing else of the user's is sent (Glance's site is asked
//! besides what is new in the reports this PC sent: see `reports`). A newer one
//! is offered in the settings (and once by the tray), and taken only when
//! the user asks: its installer is downloaded into a folder of
//! administrators' alone and run only if it carries Glance's own signature for
//! the version asked for (see `own_signature`), or Authenticode's by Glance's
//! publisher (see `PUBLISHER`) on an installer that carries that version, and
//! the version is newer than this one; it is held open, no one else writing it,
//! till it is done.

use std::cmp::Ordering;
use std::io::{Read, Seek};
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
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Security::Cryptography::{
    CertGetCertificateContextProperty, CertGetEnhancedKeyUsage, CertNameToStrW, CERT_CONTEXT, CERT_FIND_EXT_ONLY_ENHKEY_USAGE_FLAG, CERT_STRING_TYPE, CTL_USAGE,
    CERT_NAME_STR_REVERSE_FLAG, CERT_SHA256_HASH_PROP_ID, CERT_X500_NAME_STR, X509_ASN_ENCODING,
};
use windows::Win32::Security::WinTrust::{
    WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData, WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA,
    WINTRUST_DATA_0, WINTRUST_FILE_INFO, WTD_CHOICE_FILE, WTD_REVOKE_WHOLECHAIN, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
};
use windows::Win32::Storage::FileSystem::{FILE_SHARE_READ, GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Threading::{WaitForSingleObject, INFINITE};
use windows::Win32::UI::Shell::{FOLDERID_ProgramData, SHGetKnownFolderPath, ShellExecuteExW, KF_FLAG_DEFAULT, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::ui::prefs::LanguagePref;
use crate::ui::text::Lang;

/// Where each site describes the latest release, in the same terms.
/// Glance's own site, asked first of all (`LATEST`, there): it says the same
/// of the latest release as the two below do, serves the installer itself,
/// and counts how many ask (see `site`).
const LATEST: &str = "/api/latest";
const SOURCES: [&str; 2] = [
    "https://api.github.com/repos/lulu-loopp/glance/releases/latest",
    "https://gitee.com/api/v5/repos/lulu-loopp/glance/releases/latest",
];
/// Glance's own site, where Glance asks for the latest release and sends its reports. A debug build may be pointed
/// at another (`GLANCE_SITE`, a copy of the site run on this PC, see `on_this_pc`) to be tried against: a release
/// build has no such switch.
pub(crate) fn own_site() -> String {
    #[cfg(debug_assertions)]
    if let Ok(site) = std::env::var("GLANCE_SITE") {
        return site.trim_end_matches('/').to_string();
    }
    "https://glancepc.com".to_string()
}

/// Where an installer is taken from: Glance's site, and the releases of its own repositories (each followed
/// wherever it redirects: what is downloaded is believed only for its signature).
const INSTALLERS_FROM: [&str; 3] = [
    "https://glancepc.com/",
    "https://github.com/lulu-loopp/glance/releases/download/",
    "https://gitee.com/lulu-loopp/glance/releases/download/",
];

/// Glance's publisher, as its signing certificates say it (a signing service's certificates last days; these
/// stay): this subject, its identity validated by Microsoft's Artifact Signing (its identity-validated mark), under
/// Microsoft's root for it. Not by one certificate profile's own mark: a profile made anew for the same identity has
/// another, and installers signed under it were refused (the first profile was suspended, 2026-10).
const PUBLISHER: &str = "CN=Weiyi Shi, O=Weiyi Shi, L=Ann Arbor, S=mi, C=US";
const PUBLISHER_MARK: &str = "1.3.6.1.4.1.311.97.1.0";
/// Microsoft Identity Verification Root Certificate Authority 2020, its SHA-256.
const PUBLISHER_ROOT: [u8; 32] = [
    0x53, 0x67, 0xF2, 0x0C, 0x7A, 0xDE, 0x0E, 0x2B, 0xCA, 0x79, 0x09, 0x15, 0x05, 0x6D, 0x08, 0x6B, 0x72, 0x0C, 0x33, 0xC1, 0xFA, 0x2A, 0x26, 0x61,
    0xAC, 0xF7, 0x87, 0xE3, 0x29, 0x2E, 0x12, 0x70,
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
/// The slowest line that what is sent is waited for on (bytes a millisecond: a quarter of a megabit a second).
const SLOWEST: usize = 32;
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
    /// The tray asked while an ask was under way: told its outcome.
    tray_waits: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Asker {
    Settings,
    Tray,
}

static ASKING: Mutex<Asking> = Mutex::new(Asking { check: Check::Idle, now: None, tray_waits: false });
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
        asking.tray_waits |= asker == Asker::Tray;
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

/// A release found, `version`, its taking standing as `state` (none: none found), for pictures of the settings.
#[cfg(all(test, feature = "studio"))]
pub fn pretend(found: Option<(&str, State)>) {
    *FOUND.lock().unwrap() = found.map(|(version, state)| (Release { version: version.to_string(), installers: Vec::new() }, state));
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
                let until_due = if wanted { due.saturating_duration_since(Instant::now()) } else { Duration::from_secs(60) };
                let wait = until_due.min(Duration::from_secs(60)).max(Duration::from_secs(1));
                asking = WAKE.wait_timeout(asking, wait).unwrap().0;
            }
        };
        // What is new in the reports this PC sent, asked of Glance's site as it is asked for the latest release.
        thread::spawn(crate::reports::ask);
        let asked = Instant::now();
        let answer = latest();
        let lang = language();
        // A notice of the site's not told before, told as it came or now: told in place of the release found (clicked,
        // it opens the settings at it, where the release is too).
        let told = crate::notice::tell(lang) || crate::notice::told_since(asked);
        if answer.is_some() {
            unanswered = 0;
            due = Instant::now() + EVERY;
        } else {
            due = Instant::now() + RETRIES[unanswered.min(RETRIES.len() - 1)];
            unanswered += 1;
        }
        let answered = answer.is_some();
        let asker = {
            let mut asking = ASKING.lock().unwrap();
            asking.check = if answered { Check::Answered } else { Check::Unanswered };
            if std::mem::take(&mut asking.tray_waits) { Some(Asker::Tray) } else { asker }
        };
        let newer_release = answer.filter(|release| newer(&release.version, env!("CARGO_PKG_VERSION")));
        if let Some(release) = newer_release {
            let mut found = FOUND.lock().unwrap();
            let new = found.as_ref().is_none_or(|(known, _)| known.version != release.version);
            if new {
                let version = release.version.clone();
                crate::journal::note(format!("update available: {version}"));
                *found = Some((release, State::Ready));
                drop(found);
                // Asked from the settings, the settings show it.
                if asker != Some(Asker::Settings) && !told {
                    let (title, text) = match lang {
                        Lang::Zh => (format!("Glance {version} 可用"), "点这里，在设置中更新。".to_string()),
                        Lang::En => (format!("Glance {version} is available"), "Click here to update from the settings.".to_string()),
                    };
                    crate::tray::notify_updates(&title, &text);
                }
            }
        } else if asker == Some(Asker::Tray) && !told {
            // Asked from the tray: the answer, either way.
            let version = env!("CARGO_PKG_VERSION");
            let (title, text) = match (lang, answered) {
                (Lang::Zh, true) => ("Glance 已是最新版本".to_string(), format!("当前版本 {version}。")),
                (Lang::En, true) => ("Glance is up to date".to_string(), format!("You have {version}.")),
                (Lang::Zh, false) => ("没能检查更新".to_string(), "GitHub 和 Gitee 都没有回应，请稍后再试。".to_string()),
                (Lang::En, false) => ("Couldn't check for updates".to_string(), "Neither GitHub nor Gitee answered; try again later.".to_string()),
            };
            crate::tray::notify_updates(&title, &text);
        }
    }
}

/// Whether Glance found settings of its own as it started (see `note_settings_found`).
static SETTINGS_FOUND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Notes whether there were settings as Glance started (`found`), looked at before anything it does as it starts
/// writes them: on a first install, what it settles first (each GPU's number) is written before it says whether it
/// was updated.
pub fn note_settings_found(found: bool) {
    let _ = SETTINGS_FOUND.set(found);
}

/// Says so once if this is the first start since Glance was updated, and
/// notes the version that runs. Settings from before versions were noted
/// (0.1.8 and earlier) count as an update; none as Glance started, as a first install.
pub fn tell_if_updated() {
    let app = crate::app();
    let version = env!("CARGO_PKG_VERSION");
    let last = app.settings.lock().unwrap().last_version.clone();
    let updated = match last.as_deref() {
        Some(last) => newer(version, last),
        None => *SETTINGS_FOUND.get().expect("noted as Glance started"),
    };
    if last.as_deref() == Some(version) {
        return;
    }
    app.change(|settings| settings.last_version = Some(version.to_string()));
    if updated {
        let (title, text) = match language() {
            Lang::Zh => (format!("Glance 已更新到 {version}"), "设置都已保留。".to_string()),
            Lang::En => (format!("Glance is updated to {version}"), "Your settings are as they were.".to_string()),
        };
        crate::tray::notify(&title, &text);
    }
}

/// Downloads the newer release's installer and runs it, on a thread of its
/// own; the installer closes Glance as it replaces it. With the sensors in a
/// process of their own, they do it (with the rights to run it unasked).
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
    if crate::source::linked() {
        crate::source::install(crate::link::ReleaseSent { version: release.version.clone(), installers: release.installers.clone() });
        return;
    }
    // Without the sensors' process, the update is taken by this program run elevated for it (the user is asked,
    // having asked for the update), unless this one is elevated already.
    thread::spawn(move || {
        let outcome = if crate::elevation::is_elevated() { fetch_and_run(&release) } else { fetch_and_run_elevated(&release) };
        if let Some((_, state)) = FOUND.lock().unwrap().as_mut() {
            *state = outcome;
        }
    });
}

/// The sensors' process went: an update it was taking (its outcome never to be told) may be asked for again.
pub fn link_lost() {
    if let Some((_, state)) = FOUND.lock().unwrap().as_mut() {
        if *state == State::Downloading {
            *state = State::Failed;
        }
    }
}

/// What `--update` is given: the release's version, then each installer's version and where it is.
const UPDATE: &str = "--update";

/// Takes `release` in this program run elevated (`UPDATE`), the user asked to let it; how that went.
fn fetch_and_run_elevated(release: &Release) -> State {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    let Ok(exe) = std::env::current_exe() else { return State::Failed };
    let arguments = std::iter::once(UPDATE.to_string())
        .chain(std::iter::once(release.version.clone()))
        .chain(release.installers.iter().flat_map(|(version, url)| [version.clone(), url.clone()]))
        .map(|argument| format!("\"{argument}\""))
        .collect::<Vec<_>>()
        .join(" ");
    let (file, arguments) = (HSTRING::from(exe.as_os_str()), HSTRING::from(arguments));
    let mut started = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(arguments.as_ptr()),
        nShow: windows::Win32::UI::WindowsAndMessaging::SW_HIDE.0,
        ..Default::default()
    };
    // (Refused by the user, or not started: as one that could not be downloaded, to be tried again.)
    if unsafe { ShellExecuteExW(&mut started) }.is_err() || started.hProcess.is_invalid() {
        return State::Failed;
    }
    let mut code = 1u32;
    unsafe {
        WaitForSingleObject(started.hProcess, INFINITE);
        let _ = GetExitCodeProcess(started.hProcess, &mut code);
        let _ = CloseHandle(started.hProcess);
    }
    match code {
        0 => State::Ready,
        2 => State::Rejected,
        _ => State::Failed,
    }
}

/// Run as `UPDATE` (elevated, see `fetch_and_run_elevated`): takes the release its arguments name, and ends saying
/// how that went (0 the installer runs, 2 none was as it should be, 1 none could be had). None: not so run.
pub fn run_asked(arguments: &[String]) -> Option<i32> {
    let [flag, version, rest @ ..] = arguments else { return None };
    if flag != UPDATE || rest.len() % 2 != 0 {
        return None;
    }
    let installers = rest.chunks(2).map(|pair| (pair[0].clone(), pair[1].clone())).collect();
    Some(match fetch_and_run(&Release { version: version.clone(), installers }) {
        State::Ready => 0,
        State::Rejected => 2,
        State::Failed | State::Downloading => 1,
    })
}

/// Where an update asked of the sensors stands, as they tell it.
pub fn set_state(state: crate::link::UpdateState) {
    if let Some((_, kept)) = FOUND.lock().unwrap().as_mut() {
        *kept = match state {
            crate::link::UpdateState::Ready => State::Ready,
            crate::link::UpdateState::Downloading => State::Downloading,
            crate::link::UpdateState::Failed => State::Failed,
            crate::link::UpdateState::Rejected => State::Rejected,
        };
    }
}

/// The release the rest of Glance asks the sensors to update to, downloaded and run here (see `fetch_and_run`).
pub fn fetch_and_run_sent(release: &crate::link::ReleaseSent) -> crate::link::UpdateState {
    match fetch_and_run(&Release { version: release.version.clone(), installers: release.installers.clone() }) {
        State::Ready => crate::link::UpdateState::Ready,
        State::Downloading => crate::link::UpdateState::Downloading,
        State::Failed => crate::link::UpdateState::Failed,
        State::Rejected => crate::link::UpdateState::Rejected,
    }
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
    // One file whichever site's it is (each tried in turn written over the one before): the one noted, and taken away.
    let installer = folder.path().join("Glance-setup.exe");
    if !crate::elevation::remember_download(&installer) {
        note("the installer could not be written");
        return State::Failed;
    }
    // From each site in turn (only Glance's own: see `INSTALLERS_FROM`), until one gives an installer that is
    // what it should be: signed as Glance's, of the version that site announced, and newer than this one. No site
    // is trusted for any of it: an older signed installer under a newer name would take Glance back, and a site
    // claiming a version that is not out keeps no other from being tried. The installer is checked as held open,
    // no one else writing it, and so held till it is done.
    let mut chosen = None;
    let mut outcome = State::Failed;
    for (version, url) in &release.installers {
        if !INSTALLERS_FROM.iter().any(|from| url.starts_with(from)) {
            note(&format!("{url} is no place of Glance's; not downloaded"));
            continue;
        }
        let host = url.trim_start_matches("https://").split('/').next().unwrap_or(url);
        // (Glance's site is told which version takes it: its count of updates.)
        let from = if host == "glancepc.com" { format!("{url}?u={}", env!("CARGO_PKG_VERSION")) } else { url.clone() };
        let Some(bytes) = get(&from) else {
            note(&format!("the installer could not be downloaded from {host}"));
            continue;
        };
        let held = std::fs::write(&installer, bytes).and_then(|()| hold(&installer));
        let Ok(held) = held else {
            note("the installer could not be written");
            return State::Failed;
        };
        // Glance's own signature says the version with the bytes. Authenticode's says neither: the version is what
        // the installer carries, whole (its numbers alone are a preview's and its release's alike, and an older
        // preview, signed, would pass for a later one).
        let own = own_signature_on(url, version, &held);
        if !own && !by_publisher(&installer, &held) {
            note(&format!("the installer from {host} is signed neither by Glance nor by its publisher; not run"));
            outcome = State::Rejected;
            continue;
        }
        let carried = installer_version(&installer);
        let announced = own || carried.as_deref().is_some_and(|carried| same_version(carried, version));
        if !announced {
            note(&format!("the installer from {host} is version {}, not {version}; not run", carried.as_deref().unwrap_or("unknown")));
            outcome = State::Rejected;
            continue;
        }
        if !newer(version, env!("CARGO_PKG_VERSION")) {
            note(&format!("the installer from {host} is version {version}, not newer than this one; not run"));
            outcome = State::Rejected;
            continue;
        }
        chosen = Some((installer, held));
        break;
    }
    let Some((installer, held)) = chosen else { return outcome };
    let path = HSTRING::from(installer.as_os_str());
    // Silent, and Glance started again once it is in: the one press in the
    // settings is all an update asks. (Glance runs elevated, and so does the
    // installer it starts, unasked.) Not for a Glance installed somewhere
    // unprotected all the same: a silent installer cannot ask whether to
    // install there again, and refuses; it shows its pages instead.
    let arguments = if crate::elevation::may_start_unasked() { w!("/S /RESTART") } else { PCWSTR::null() };
    let mut started = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("open"),
        lpFile: PCWSTR(path.as_ptr()),
        lpParameters: arguments,
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    if unsafe { ShellExecuteExW(&mut started) }.is_err() || started.hProcess.is_invalid() {
        note("the installer could not be started");
        return State::Failed;
    }
    note("installer started");
    // Held till it is done (or till this process is ended by it, which lets it go all the same).
    let process = started.hProcess.0 as isize;
    thread::spawn(move || {
        let process = HANDLE(process as *mut _);
        unsafe {
            WaitForSingleObject(process, INFINITE);
            let _ = CloseHandle(process);
        }
        drop(held);
    });
    State::Ready
}

/// Whether the installer held as `held`, downloaded from `url`, has Glance's own signature as `version`, in the file
/// beside it there.
fn own_signature_on(url: &str, version: &str, held: &std::fs::File) -> bool {
    let Some(signature) = get(&format!("{url}.sig")) else { return false };
    let mut bytes = Vec::new();
    let mut reading = held;
    let read = reading.read_to_end(&mut bytes).is_ok() && reading.rewind().is_ok();
    read && crate::own_signature::is_glances(version, &bytes, &signature)
}

/// `file` opened to be read, as long as it is held: by others too, but neither written nor deleted.
fn hold(file: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ.0).open(file)
}

/// Glance's download page on its site, in `lang`: where a release is had whatever this Glance can check of it.
pub fn download_page(lang: Lang) -> String {
    match lang {
        Lang::Zh => format!("{}/#download", own_site()),
        Lang::En => format!("{}/en/#download", own_site()),
    }
}

/// Whether two versions are one, by the order versions go in (see `order`): never what is not a version.
fn same_version(a: &str, b: &str) -> bool {
    order(a, b) == Some(Ordering::Equal)
}

/// The product version an installer carries in its version resource, as it was
/// released ("0.4.3-preview": the resource's numbers say only 0.4.3.0).
fn installer_version(file: &Path) -> Option<String> {
    let path = HSTRING::from(file.as_os_str());
    let size = unsafe { GetFileVersionInfoSizeW(&path, None) };
    if size == 0 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    unsafe { GetFileVersionInfoW(&path, None, size, data.as_mut_ptr().cast()) }.ok()?;
    // Under the language and code page it names first.
    let (mut named, mut length) = (std::ptr::null_mut(), 0u32);
    if !unsafe { VerQueryValueW(data.as_ptr().cast(), w!("\\VarFileInfo\\Translation"), &mut named, &mut length) }.as_bool() || (length as usize) < 2 * size_of::<u16>() {
        return None;
    }
    let (language, page) = unsafe { (*(named as *const u16), *(named as *const u16).add(1)) };
    let key = HSTRING::from(format!("\\StringFileInfo\\{language:04x}{page:04x}\\ProductVersion"));
    let (mut text, mut characters) = (std::ptr::null_mut(), 0u32);
    if !unsafe { VerQueryValueW(data.as_ptr().cast(), &key, &mut text, &mut characters) }.as_bool() || characters == 0 {
        return None;
    }
    let text = unsafe { std::slice::from_raw_parts(text as *const u16, characters as usize) };
    let version = String::from_utf16_lossy(text);
    Some(version.trim_end_matches('\0').to_string())
}

/// The machine's ProgramData folder.
pub(crate) fn program_data() -> Option<std::path::PathBuf> {
    let path = unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramData, KF_FLAG_DEFAULT, None) }.ok()?;
    let text = unsafe { path.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(path.0 as *const _)) };
    text.map(std::path::PathBuf::from)
}

/// The language the user reads Glance in.
pub(crate) fn language() -> Lang {
    language_in(&crate::app().settings.lock().unwrap().view)
}

/// The language the user reads Glance in, as `view` (the settings' preferences, held already) says.
pub(crate) fn language_in(view: &serde_json::Value) -> Lang {
    Lang::resolve(serde_json::from_value::<LanguagePref>(view["language"].clone()).unwrap_or_default())
}

/// Where Glance's site is asked for the latest release, with what it is
/// told for its counting: this version; that this is the first asking of a
/// Glance new here (`n`); which version this one was updated from, where
/// the last to ask was another (`f`: "pre" for one from before versions
/// said); and how many weeks Glance has been here (`w`). Nothing that tells
/// one PC from another: no name, no number kept for it.
fn site() -> String {
    let (began, asked_as) = {
        let settings = crate::app().settings.lock().unwrap();
        (settings.began, settings.asked_as.clone())
    };
    let version = env!("CARGO_PKG_VERSION");
    let mut url = format!("{}{LATEST}?v={version}", own_site());
    match (&asked_as, began) {
        (None, Some(_)) => url.push_str("&n=1"),
        (None, None) => url.push_str("&f=pre"),
        (Some(last), _) if last != version => url.push_str(&format!("&f={last}")),
        (Some(_), _) => {}
    }
    if let Some(began) = began {
        let today = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs() / 86400);
        url.push_str(&format!("&w={}", today.saturating_sub(began) / 7));
    }
    url
}

/// The latest release each site knows of, the newest first. A site that
/// does not answer (out of reach from where the user is) is noted, and the
/// others serve.
fn latest() -> Option<Release> {
    // Both sites at once, each on a thread of its own; their answers in the
    // order they come, a site out of reach answering nothing. Once one has
    // answered, the other is waited for a moment more, not for as long as
    // an unreachable site takes to give up.
    let (sender, answers) = std::sync::mpsc::channel();
    let sources: Vec<String> = std::iter::once(site()).chain(SOURCES.iter().map(|url| url.to_string())).collect();
    let asked = sources.len();
    for url in sources {
        let sender = sender.clone();
        thread::spawn(move || {
            let body = get(&url);
            // Glance's own site answered: what it has to tell besides (see `notice`).
            let own_site = url.starts_with(&format!("{}{LATEST}", own_site()));
            if let Some(body) = body.as_deref().filter(|_| own_site) {
                crate::notice::heard(body);
            }
            let answer = body.and_then(|body| release(&body));
            if answer.is_none() {
                let host = url.trim_start_matches("https://").split('/').next().unwrap_or(&url);
                crate::journal::note(format!("update check: no answer from {host}"));
            } else if own_site {
                // Answered: the next asking is this version's, not a new Glance's nor one just updated.
                crate::app().change(|settings| settings.asked_as = Some(env!("CARGO_PKG_VERSION").to_string()));
            }
            let _ = sender.send(answer);
        });
    }
    drop(sender);
    let mut installers: Vec<(String, String)> = Vec::new();
    let mut heard = 0;
    while heard < asked {
        let next = if installers.is_empty() { answers.recv().ok() } else { answers.recv_timeout(STRAGGLER).ok() };
        let Some(answer) = next else { break };
        heard += 1;
        installers.extend(answer);
    }
    // The newest first; of the same version, Glance's own site's, then the sooner of the others.
    let own = |url: &str| url.starts_with("https://glancepc.com/");
    installers.sort_by(|(a, from_a), (b, from_b)| if newer(a, b) { Ordering::Less } else if newer(b, a) { Ordering::Greater } else { own(from_b).cmp(&own(from_a)) });
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

/// Whether version `a` comes after `b`: number by number (1.10 after 1.9; a missing number counts as 0), and of the
/// same numbers, as semantic versions are ordered, a preview before its release and previews by what follows the dash
/// (0.4.3-preview, 0.4.3-preview.2, 0.4.3). What is not a version comes after nothing.
///
/// Glance before 0.4.3-preview read numbers alone and took a version with a dash for no version: such a release is
/// never offered to it (`an_older_glance_is_offered_no_preview`).
pub(crate) fn newer(a: &str, b: &str) -> bool {
    order(a, b) == Some(Ordering::Greater)
}

/// How version `a` stands to `b` (see `newer`): nothing where either is not a version, which is neither before, after
/// nor the same as any.
fn order(a: &str, b: &str) -> Option<Ordering> {
    let ((mut numbers_a, preview_a), (mut numbers_b, preview_b)) = (version_parts(a)?, version_parts(b)?);
    let length = numbers_a.len().max(numbers_b.len());
    numbers_a.resize(length, 0);
    numbers_b.resize(length, 0);
    Some(numbers_a.cmp(&numbers_b).then_with(|| match (preview_a.is_empty(), preview_b.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => preview_order(&preview_a, &preview_b),
    }))
}

/// A version's numbers, and what follows its dash, field by field (none for a release): nothing, if it is not one.
fn version_parts(version: &str) -> Option<(Vec<u64>, Vec<&str>)> {
    let (numbers, preview) = version.split_once('-').map_or((version, None), |(numbers, preview)| (numbers, Some(preview)));
    let numbers = numbers.split('.').map(|part| part.parse().ok()).collect::<Option<Vec<u64>>>()?;
    let preview: Vec<&str> = preview.map_or(Vec::new(), |preview| preview.split('.').collect());
    let well_made = preview.iter().all(|field| !field.is_empty() && field.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'));
    well_made.then_some((numbers, preview))
}

/// Two previews' fields in order, as semantic versions have them: numbers by value and before words, words as written,
/// and the one that ends first before the other.
fn preview_order(a: &[&str], b: &[&str]) -> Ordering {
    for (a, b) in a.iter().zip(b) {
        let order = match (a.parse::<u64>(), b.parse::<u64>()) {
            (Ok(a), Ok(b)) => a.cmp(&b),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            (Err(_), Err(_)) => a.cmp(b),
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    a.len().cmp(&b.len())
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
pub(crate) fn get(url: &str) -> Option<Vec<u8>> {
    ask(url, None, RELEASES)
}

/// What a release's places are asked for as: GitHub's API's JSON, or a file itself.
const RELEASES: &str = "application/vnd.github+json, application/octet-stream";

/// What `url` answers to JSON sent to it (an HTTPS POST), if it answered 200.
pub fn post(url: &str, json: &[u8]) -> Option<Vec<u8>> {
    ask(url, Some(json), "application/json")
}

/// What `url` answered to an HTTPS GET asked for as JSON (any other site's API), whatever its status: its status and body (none, where it
/// was not reached, or its answer was cut short).
pub fn answer_json(url: &str) -> Option<(u32, Vec<u8>)> {
    answered(url, None, "application/json, */*", None)
}

/// The same, asked as from the page `referer` (a site whose API answers only its own pages).
pub fn answer_json_from(url: &str, referer: &str) -> Option<(u32, Vec<u8>)> {
    answered(url, None, "application/json, */*", Some(referer))
}

fn ask(url: &str, sent: Option<&[u8]>, accept: &str) -> Option<Vec<u8>> {
    answered(url, sent, accept, None).filter(|(status, _)| *status == 200).map(|(_, body)| body)
}

fn answered(url: &str, sent: Option<&[u8]>, accept: &str, referer: Option<&str>) -> Option<(u32, Vec<u8>)> {
    // Over HTTPS, or (a debug build only) over plain HTTP to a site run on this PC (see `own_site`).
    let (secure, rest) = match url.strip_prefix("https://") {
        Some(rest) => (true, rest),
        None => (false, on_this_pc(url)?),
    };
    let (host, path) = rest.split_at(rest.find('/')?);
    let (host, port) = match host.rsplit_once(':') {
        Some((name, port)) if !secure => (name, port.parse().ok()?),
        _ => (host, INTERNET_DEFAULT_HTTPS_PORT),
    };
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
        // (Longer for the sending by what is sent, were the line that slow.)
        let _ = WinHttpSetTimeouts(session.0, RESOLVE_MS, CONNECT_MS, ANSWER_MS + (sent.map_or(0, <[u8]>::len) / SLOWEST) as i32, ANSWER_MS);
        let connection = Handle(WinHttpConnect(session.0, &HSTRING::from(host), port, 0));
        if connection.0.is_null() {
            return None;
        }
        let method = if sent.is_some() { w!("POST") } else { w!("GET") };
        let flags = if secure { WINHTTP_FLAG_SECURE } else { Default::default() };
        let request = Handle(WinHttpOpenRequest(connection.0, method, &HSTRING::from(path), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), flags));
        if request.0.is_null() {
            return None;
        }
        match sent {
            None => {
                let from = referer.map(|page| format!("Referer: {page}\r\n")).unwrap_or_default();
                let headers: Vec<u16> = format!("Accept: {accept}\r\n{from}").encode_utf16().collect();
                WinHttpSendRequest(request.0, Some(&headers), None, 0, 0, 0).ok()?;
            }
            Some(body) => {
                let headers: Vec<u16> = format!("Content-Type: application/json\r\nAccept: {accept}\r\n").encode_utf16().collect();
                WinHttpSendRequest(request.0, Some(&headers), Some(body.as_ptr().cast()), body.len() as u32, body.len() as u32, 0).ok()?;
            }
        }
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
        let mut body = Vec::new();
        let mut chunk = vec![0u8; 64 << 10];
        loop {
            let mut read = 0u32;
            WinHttpReadData(request.0, chunk.as_mut_ptr().cast(), chunk.len() as u32, &mut read).ok()?;
            if read == 0 {
                return Some((status, body));
            }
            body.extend_from_slice(&chunk[..read as usize]);
            if body.len() > MOST {
                return None;
            }
        }
    }
}

/// What follows `http://` in `url`, where it is a site run on this PC (its address and port), for a debug build to
/// be tried against (see `own_site`). A release build asks nothing but over HTTPS.
#[cfg(debug_assertions)]
fn on_this_pc(url: &str) -> Option<&str> {
    url.strip_prefix("http://").filter(|rest| rest.starts_with("127.0.0.1:") || rest.starts_with("localhost:"))
}

#[cfg(not(debug_assertions))]
fn on_this_pc(_url: &str) -> Option<&str> {
    None
}

/// A file's Authenticode signature, verified (the whole chain, revocation
/// included), held open while its signer's certificates are looked at.
struct Signature {
    data: WINTRUST_DATA,
    _file: Box<WINTRUST_FILE_INFO>,
    _path: HSTRING,
}

impl Signature {
    /// That of `file`, read through `opened` (the file as held open).
    fn verify(file: &Path, opened: &std::fs::File) -> Option<Self> {
        use std::os::windows::io::AsRawHandle;
        let path = HSTRING::from(file.as_os_str());
        let mut info = Box::new(WINTRUST_FILE_INFO {
            cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
            pcwszFilePath: PCWSTR(path.as_ptr()),
            hFile: HANDLE(opened.as_raw_handle()),
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

    /// The chain it was signed with, as it was verified: the signer's certificate first, the root last.
    fn chain(&self) -> Vec<&CERT_CONTEXT> {
        unsafe {
            let provider = WTHelperProvDataFromStateData(self.data.hWVTStateData);
            if provider.is_null() {
                return Vec::new();
            }
            let signer = WTHelperGetProvSignerFromChain(provider, 0, false, 0);
            if signer.is_null() || (*signer).pasCertChain.is_null() {
                return Vec::new();
            }
            std::slice::from_raw_parts((*signer).pasCertChain, (*signer).csCertChain as usize).iter().filter_map(|element| element.pCert.as_ref()).collect()
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

/// Whether `file` (held open as `opened`) carries a valid signature by Glance's publisher (see `PUBLISHER`).
fn by_publisher(file: &Path, opened: &std::fs::File) -> bool {
    let Some(signature) = Signature::verify(file, opened) else { return false };
    let chain = signature.chain();
    let (Some(signer), Some(root)) = (chain.first(), chain.last()) else { return false };
    chain.len() > 1 && publishers(subject(signer).as_deref(), &usages(signer), sha256(root))
}

/// Whether a signer of `subject`, for `usages`, under the root of SHA-256 `root`, is Glance's publisher.
fn publishers(subject: Option<&str>, usages: &[String], root: Option<[u8; 32]>) -> bool {
    subject == Some(PUBLISHER) && usages.iter().any(|usage| usage == PUBLISHER_MARK) && root == Some(PUBLISHER_ROOT)
}

/// A certificate's subject, written as `PUBLISHER` is (its most particular part first).
fn subject(certificate: &CERT_CONTEXT) -> Option<String> {
    let kind = CERT_STRING_TYPE(CERT_X500_NAME_STR.0 | CERT_NAME_STR_REVERSE_FLAG);
    let name = unsafe { &(*certificate.pCertInfo).Subject };
    let length = unsafe { CertNameToStrW(X509_ASN_ENCODING, name, kind, None) };
    let mut text = vec![0u16; length as usize];
    let written = unsafe { CertNameToStrW(X509_ASN_ENCODING, name, kind, Some(&mut text)) } as usize;
    (written > 0).then(|| String::from_utf16_lossy(&text[..written - 1]))
}

/// The uses a certificate is for (its enhanced key usage), as identifiers.
fn usages(certificate: &CERT_CONTEXT) -> Vec<String> {
    let mut size = 0u32;
    unsafe {
        if CertGetEnhancedKeyUsage(certificate, CERT_FIND_EXT_ONLY_ENHKEY_USAGE_FLAG.0, None, &mut size).is_err() {
            return Vec::new();
        }
        // (Aligned as the structure that heads it.)
        let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
        let usage = buffer.as_mut_ptr() as *mut CTL_USAGE;
        if CertGetEnhancedKeyUsage(certificate, CERT_FIND_EXT_ONLY_ENHKEY_USAGE_FLAG.0, Some(usage), &mut size).is_err() || (*usage).rgpszUsageIdentifier.is_null() {
            return Vec::new();
        }
        std::slice::from_raw_parts((*usage).rgpszUsageIdentifier, (*usage).cUsageIdentifier as usize).iter().filter_map(|id: &windows::core::PSTR| id.to_string().ok()).collect()
    }
}

/// A certificate's SHA-256.
fn sha256(certificate: &CERT_CONTEXT) -> Option<[u8; 32]> {
    let mut hash = [0u8; 32];
    let mut size = hash.len() as u32;
    unsafe { CertGetCertificateContextProperty(certificate, CERT_SHA256_HASH_PROP_ID, Some(hash.as_mut_ptr().cast()), &mut size) }.ok()?;
    (size == 32).then_some(hash)
}

#[cfg(test)]
mod tests {
    use super::{by_publisher, get, hold, installer_version, newer, publishers, release, same_version, INSTALLERS_FROM, PUBLISHER, PUBLISHER_ROOT, SOURCES};
    use std::path::Path;

    /// Over the network: the latest release is found, its installer
    /// downloaded, and found signed by Glance's publisher; another
    /// publisher's program (`OTHER_SIGNED`) is not.
    #[test]
    #[ignore = "downloads the latest release over the network"]
    fn finds_downloads_and_checks_the_latest_release() {
        // (As `latest` asks GitHub and Gitee; Glance's own site is asked only by a running Glance.)
        let releases: Vec<(String, String)> = SOURCES.iter().map(|url| release(&get(url).expect("an answer")).expect("a release")).collect();
        assert_eq!(releases[0].0, releases[1].0, "both sites offer the same version");
        let (_, mirror) = releases.iter().find(|(_, url)| url.starts_with("https://gitee.com/")).expect("the mirror's installer");
        assert!(releases.iter().all(|(_, url)| INSTALLERS_FROM.iter().any(|from| url.starts_with(from))));
        let bytes = get(mirror).expect("installer from the mirror");
        let file = std::env::temp_dir().join(format!("glance-update-test-{}.exe", std::process::id()));
        std::fs::write(&file, bytes).unwrap();
        let other = std::env::var("OTHER_SIGNED").unwrap();
        let ours = by_publisher(&file, &hold(&file).unwrap());
        let theirs = by_publisher(Path::new(&other), &hold(Path::new(&other)).unwrap());
        std::fs::remove_file(&file).unwrap();
        assert!(ours, "{} not signed by Glance's publisher", releases[0].0);
        assert!(!theirs);
    }

    /// Glance's publisher is its identity as Microsoft validated it, under any certificate profile of it: one made
    /// anew (another profile's mark) is still it, and another subject, an identity not validated, or another root
    /// is not.
    #[test]
    fn its_publisher_is_its_identity_under_any_profile() {
        let marks = |marks: &[&str]| marks.iter().map(|mark| mark.to_string()).collect::<Vec<_>>();
        let first = marks(&["1.3.6.1.4.1.311.97.1.0","1.3.6.1.4.1.311.97.921771326.840678362.737210289.98417136", "1.3.6.1.5.5.7.3.3"]);
        let anew = marks(&["1.3.6.1.4.1.311.97.1.0", "1.3.6.1.4.1.311.97.12345.678.910.1112", "1.3.6.1.5.5.7.3.3"]);
        let root = Some(PUBLISHER_ROOT);
        assert!(publishers(Some(PUBLISHER), &first, root));
        assert!(publishers(Some(PUBLISHER), &anew, root));
        assert!(!publishers(Some("CN=Someone Else, O=Someone Else, L=Ann Arbor, S=mi, C=US"), &anew, root));
        assert!(!publishers(Some(PUBLISHER), &marks(&["1.3.6.1.5.5.7.3.3"]), root));
        assert!(!publishers(Some(PUBLISHER), &anew, Some([0; 32])));
        assert!(!publishers(None, &anew, root));
    }

    /// A released installer (`GLANCE_INSTALLER`) is signed by Glance's publisher; held, it is not written, but
    /// read by others meanwhile (as its version is).
    #[test]
    #[ignore = "needs a released installer, named by GLANCE_INSTALLER"]
    fn knows_glances_publisher() {
        let installer = std::env::var("GLANCE_INSTALLER").unwrap();
        let held = hold(Path::new(&installer)).unwrap();
        assert!(by_publisher(Path::new(&installer), &held));
        assert!(std::fs::OpenOptions::new().append(true).open(&installer).is_err());
        assert!(installer_version(Path::new(&installer)).is_some());
    }

    /// Authenticode says nothing of the version: an older preview, signed, with the same numbers, does not pass for a
    /// later one, nor for the release, nor the release for a preview before it.
    #[test]
    fn a_signed_installer_is_only_the_version_it_carries() {
        assert!(same_version("0.4.3-preview", "0.4.3-preview"));
        assert!(same_version("0.4.3", "0.4.3"));
        assert!(!same_version("0.4.3-preview", "0.4.3-preview.2"));
        assert!(!same_version("0.4.3-preview", "0.4.3"));
        assert!(!same_version("0.4.3", "0.4.3-preview"));
        assert!(!same_version("0.4.3", "0.4.4"));
        // What is not a version is the same as none: neither newer nor older is not the same.
        for not_one in ["", "garbage", "0.4.2+build", "0.4.4-", "v0.4.4"] {
            assert!(!same_version(not_one, "0.4.4"), "{not_one}");
            assert!(!same_version("0.4.4", not_one), "{not_one}");
            assert!(!same_version(not_one, not_one), "{not_one}");
        }
    }

    /// The version an installer (`GLANCE_INSTALLER`, a released one) carries.
    #[test]
    #[ignore = "needs a released installer, named by GLANCE_INSTALLER"]
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
    }

    /// In the order a release comes after its previews: each after all before it.
    #[test]
    fn a_preview_comes_before_its_release() {
        let order = ["0.4.2", "0.4.3-preview", "0.4.3-preview.2", "0.4.3-preview.10", "0.4.3-rc", "0.4.3", "0.4.4-preview", "0.4.4", "0.5"];
        for (at, earlier) in order.iter().enumerate() {
            assert!(!newer(earlier, earlier), "{earlier}");
            for later in &order[at + 1..] {
                assert!(newer(later, earlier), "{later} after {earlier}");
                assert!(!newer(earlier, later), "{earlier} before {later}");
            }
        }
    }

    #[test]
    fn what_is_not_a_version_comes_after_nothing() {
        for not_one in ["", "preview", "0.4.x", "0.4.3-", "0.4.3-preview..2", "0.4.3-预览"] {
            assert!(!newer(not_one, "0.1"), "{not_one}");
            assert!(!newer("0.1", not_one), "{not_one}");
        }
    }

    /// Glance from 0.4.2 and before, as it compares (its code, kept here): a preview is no version to it, and is never
    /// offered. What it is told of a later release stands.
    #[test]
    fn an_older_glance_is_offered_no_preview() {
        let older_newer = |a: &str, b: &str| {
            let numbers = |v: &str| -> Option<Vec<u64>> { v.split('.').map(|part| part.parse().ok()).collect() };
            let (Some(mut a), Some(mut b)) = (numbers(a), numbers(b)) else { return false };
            let length = a.len().max(b.len());
            a.resize(length, 0);
            b.resize(length, 0);
            a > b
        };
        assert!(!older_newer("0.4.3-preview", "0.4.2"));
        assert!(!older_newer("0.4.4-preview.2", "0.4.0"));
        assert!(older_newer("0.4.3", "0.4.2"));
    }

    /// Every place a version is written says this one: nothing else ties the installer's to the program's.
    #[test]
    fn the_installer_and_the_remote_are_of_this_version() {
        let version = env!("CARGO_PKG_VERSION");
        assert!(include_str!("../installer/glance.nsi").contains(&format!("!define VERSION \"{version}\"")));
        assert!(include_str!("../remote/Cargo.toml").contains(&format!("\nversion = \"{version}\"")));
    }
}
