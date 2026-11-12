//! The phone remote and nearby transfer, as Glance's settings turn them on
//! and off, pair phones with the one and name this PC for the other.
//!
//! Both are another program, `glance-remote.exe` beside this one: it runs as
//! the user (never elevated), answers a paired phone on the local network
//! and devices nearby (see `nearby`), and is the only one of the two to open
//! a port. Glance starts it (through a task of the user's own, which also
//! starts it at sign-in) while either is on, lets it through the firewall as
//! each needs, and asks it for what the settings show: a pairing offered as a
//! QR code, the phones paired, the devices trusted.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use windows::core::{w, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, LocalFree, SetHandleInformation, HANDLE, HANDLE_FLAGS, HANDLE_FLAG_INHERIT, HLOCAL};
use windows::Win32::Security::AppLocker::{SaferCloseLevel, SaferComputeTokenFromLevel, SaferCreateLevel, SAFER_COMPUTE_TOKEN_FROM_LEVEL_FLAGS, SAFER_LEVELID_NORMALUSER, SAFER_LEVEL_OPEN, SAFER_SCOPEID_USER};
use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, ConvertStringSidToSidW, SDDL_REVISION_1};
use windows::Win32::Security::{
    GetLengthSid, GetSecurityDescriptorDacl, SetTokenInformation, TokenDefaultDacl, TokenIntegrityLevel, TokenOwner, ACL, PSECURITY_DESCRIPTOR, PSID, SAFER_LEVEL_HANDLE, SECURITY_ATTRIBUTES, SID_AND_ATTRIBUTES,
    TOKEN_DEFAULT_DACL, TOKEN_MANDATORY_LABEL, TOKEN_OWNER,
};
use windows::Win32::Storage::FileSystem::ReadFile;
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows::Win32::System::SystemServices::SE_GROUP_INTEGRITY;
use windows::Win32::System::Threading::{
    CreateProcessAsUserW, CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess, InitializeProcThreadAttributeList, UpdateProcThreadAttribute, WaitForSingleObject, CREATE_NO_WINDOW, EXTENDED_STARTUPINFO_PRESENT,
    INFINITE, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, STARTF_FORCEOFFFEEDBACK, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

use crate::elevation;

/// The port the remote answers on (its own default: see its configuration).
const PORT: u16 = 47631;
/// The firewall rule that lets phones on the local network reach it.
const RULE: &str = "Glance Remote";
/// The firewall rule that lets devices nearby reach it (see `set`).
const NEARBY_RULE: &str = "Glance Nearby";
/// The port devices nearby say they are there on, to all the network (LocalSend's own).
const NEARBY_PORT: u16 = 53317;
/// A child program started without a window of its own.
const NO_WINDOW: u32 = 0x0800_0000;
/// Where the app is to be had: the page a phone is sent to by a QR code.
pub const APP_PAGE: &str = "https://glancepc.com/app";

/// The remote's program, if this Glance came with it.
pub fn helper() -> Option<PathBuf> {
    let beside = std::env::current_exe().ok()?.parent()?.join("glance-remote.exe");
    beside.is_file().then_some(beside)
}

/// What the remote in `folder` fetched for this user (OpenRGB, in the
/// user's own folder) taken away again, by that remote, as the user: for
/// the uninstaller, which runs from elsewhere.
pub fn forget_fetched(folder: &Path) {
    let helper = folder.join("glance-remote.exe");
    if helper.is_file() {
        // (And monitors it was keeping dark when it was ended, on again.)
        let _ = asked(&helper, &["restore"]);
        let _ = asked(&helper, &["lights", "forget"]);
    }
}

/// What the remote says, asked with `arguments`. It is asked as the user
/// is without an administrator's rights, as it runs: what it reads and
/// keeps is in the user's own folders, which anything of the user's may
/// have changed, and nothing there is to be acted on with Glance's rights.
fn ask(arguments: &[&str]) -> Option<String> {
    asked(&helper()?, arguments)
}

fn asked(helper: &Path, arguments: &[&str]) -> Option<String> {
    said_plainly(helper, arguments)
}

/// A handle closed when this is let go.
struct Open(HANDLE);

impl Drop for Open {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// This program's own rights with an administrator's taken from them:
/// those of the same user's ordinary programs, to start another program
/// with. Their standing too ("medium"), and whose what they make is: the
/// user's, to be opened again by the user's programs (what an
/// administrator's program makes is the administrators', by default, and
/// would be closed to the very program that made it).
fn plain_rights() -> Option<Open> {
    unsafe {
        let mut level = SAFER_LEVEL_HANDLE::default();
        SaferCreateLevel(SAFER_SCOPEID_USER, SAFER_LEVELID_NORMALUSER, SAFER_LEVEL_OPEN, &mut level, None).ok()?;
        let mut token = HANDLE::default();
        let computed = SaferComputeTokenFromLevel(level, None, &mut token, SAFER_COMPUTE_TOKEN_FROM_LEVEL_FLAGS(0), None);
        let _ = SaferCloseLevel(level);
        computed.ok()?;
        let token = Open(token);
        let mut medium = PSID::default();
        ConvertStringSidToSidW(w!("S-1-16-8192"), &mut medium).ok()?;
        let label = TOKEN_MANDATORY_LABEL { Label: SID_AND_ATTRIBUTES { Sid: medium, Attributes: SE_GROUP_INTEGRITY as u32 } };
        let set = SetTokenInformation(token.0, TokenIntegrityLevel, (&label as *const TOKEN_MANDATORY_LABEL).cast(), size_of::<TOKEN_MANDATORY_LABEL>() as u32 + GetLengthSid(medium));
        let _ = LocalFree(Some(HLOCAL(medium.0)));
        set.ok()?;
        let user = elevation::user_sid()?;
        let mut owner = PSID::default();
        ConvertStringSidToSidW(&HSTRING::from(user.as_str()), &mut owner).ok()?;
        let owned = SetTokenInformation(token.0, TokenOwner, (&TOKEN_OWNER { Owner: owner } as *const TOKEN_OWNER).cast(), size_of::<TOKEN_OWNER>() as u32);
        let _ = LocalFree(Some(HLOCAL(owner.0)));
        owned.ok()?;
        // What it makes, open to the user and to Windows itself: as an ordinary program's is.
        let mut described = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(&HSTRING::from(format!("D:(A;;GA;;;{user})(A;;GA;;;SY)")), SDDL_REVISION_1, &mut described, None).ok()?;
        let (mut present, mut defaulted, mut open_to) = (Default::default(), Default::default(), std::ptr::null_mut::<ACL>());
        let opened = GetSecurityDescriptorDacl(described, &mut present, &mut open_to, &mut defaulted).and_then(|()| SetTokenInformation(token.0, TokenDefaultDacl, (&TOKEN_DEFAULT_DACL { DefaultDacl: open_to } as *const TOKEN_DEFAULT_DACL).cast(), size_of::<TOKEN_DEFAULT_DACL>() as u32));
        let _ = LocalFree(Some(HLOCAL(described.0)));
        opened.ok()?;
        Some(token)
    }
}

/// An argument as a command line carries it, for the program started to
/// read back what was meant (as Windows' programs split their command lines).
fn quoted(argument: &str) -> String {
    if !argument.is_empty() && !argument.contains([' ', '\t', '"']) {
        return argument.to_string();
    }
    let mut quoted = String::from('"');
    let mut slashes = 0;
    for letter in argument.chars() {
        match letter {
            '\\' => slashes += 1,
            // (The backslashes before a quote are doubled, and the quote itself escaped.)
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', slashes + 1));
                slashes = 0;
            }
            _ => slashes = 0,
        }
        quoted.push(letter);
    }
    // (As are those before the quote that closes it.)
    quoted.extend(std::iter::repeat_n('\\', slashes));
    quoted.push('"');
    quoted
}

/// The one end of a pipe to hear by, and the other (to be passed on) to say by.
fn pipe() -> Option<(Open, Open)> {
    unsafe {
        let (mut heard, mut said) = (HANDLE::default(), HANDLE::default());
        let passed_on = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, bInheritHandle: true.into(), ..Default::default() };
        CreatePipe(&mut heard, &mut said, Some(&passed_on), 0).ok()?;
        let (heard, said) = (Open(heard), Open(said));
        // (This end stays here.)
        SetHandleInformation(heard.0, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0)).ok()?;
        Some((heard, said))
    }
}

/// All that comes by a pipe, till its other end is closed.
fn all(heard: Open) -> Vec<u8> {
    let mut all = Vec::new();
    let mut some = [0u8; 4096];
    loop {
        let mut read = 0;
        if unsafe { ReadFile(heard.0, Some(&mut some), Some(&mut read), None) }.is_err() || read == 0 {
            return all;
        }
        all.extend_from_slice(&some[..read as usize]);
    }
}

// (A handle is a number: it is the same in whichever thread.)
unsafe impl Send for Open {}

/// What `program` says, run with `arguments` and without an
/// administrator's rights (see `plain_rights`), if it ends well; what it
/// says of why not goes in the log. It is given nothing of this program's
/// but somewhere to say those.
fn said_plainly(program: &Path, arguments: &[&str]) -> Option<String> {
    let (ended, said, wrong) = run_plainly(program, arguments)?;
    if ended != 0 {
        crate::journal::note(format!("the remote, asked to {}: {}", arguments.first().copied().unwrap_or_default(), wrong.trim()));
    }
    (ended == 0).then_some(said)
}

/// `program` run with `arguments` and without an administrator's rights
/// (this program's own, where it has none; else those taken from it): how
/// it ended, what it said, and what it said went wrong. Started so that the
/// pointer does not say a program is starting (as `std::process` would
/// have it), asked again and again as the settings' phone page is used.
fn run_plainly(program: &Path, arguments: &[&str]) -> Option<(u32, String, String)> {
    let rights = if elevation::is_elevated() { Some(plain_rights()?) } else { None };
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let name = wide(&program.display().to_string());
    let mut line = wide(&std::iter::once(quoted(&program.display().to_string())).chain(arguments.iter().map(|argument| quoted(argument))).collect::<Vec<_>>().join(" "));
    let ((heard, said), (heard_wrong, said_wrong)) = (pipe()?, pipe()?);
    unsafe {
        // Of all this program holds open, the program started is given those two ends alone.
        let mut size = 0;
        let _ = InitializeProcThreadAttributeList(None, 1, None, &mut size);
        let mut room = vec![0u64; size.div_ceil(8)];
        let list = LPPROC_THREAD_ATTRIBUTE_LIST(room.as_mut_ptr().cast());
        InitializeProcThreadAttributeList(Some(list), 1, None, &mut size).ok()?;
        let given = [said.0, said_wrong.0];
        let mut process = PROCESS_INFORMATION::default();
        let started = UpdateProcThreadAttribute(list, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize, Some(given.as_ptr().cast()), size_of_val(&given), None, None).and_then(|()| {
            let mut startup = STARTUPINFOEXW::default();
            startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
            // (Asked again and again as the settings' phone page is used: the pointer is not to say a program is starting.)
            startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES | STARTF_FORCEOFFFEEDBACK;
            startup.StartupInfo.hStdOutput = said.0;
            startup.StartupInfo.hStdError = said_wrong.0;
            startup.lpAttributeList = list;
            let flags = CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT;
            match &rights {
                Some(rights) => CreateProcessAsUserW(Some(rights.0), PCWSTR(name.as_ptr()), Some(PWSTR(line.as_mut_ptr())), None, None, true, flags, None, None, &startup.StartupInfo, &mut process),
                None => CreateProcessW(PCWSTR(name.as_ptr()), Some(PWSTR(line.as_mut_ptr())), None, None, true, flags, None, None, &startup.StartupInfo, &mut process),
            }
        });
        DeleteProcThreadAttributeList(list);
        started.ok()?;
        let (process, _thread) = (Open(process.hProcess), Open(process.hThread));
        // (Its ends are its alone now: what it says ends when it does.)
        drop((said, said_wrong));
        // (Both heard at once: neither waits on the other being read.)
        let wrong = std::thread::spawn(move || all(heard_wrong));
        let said = all(heard);
        let wrong = wrong.join().unwrap_or_default();
        WaitForSingleObject(process.0, INFINITE);
        let mut ended = 1;
        GetExitCodeProcess(process.0, &mut ended).ok()?;
        Some((ended, String::from_utf8_lossy(&said).into_owned(), String::from_utf8_lossy(&wrong).into_owned()))
    }
}

/// A QR code: its cells, row by row (true, dark), with no margin.
pub struct Code(pub Vec<Vec<bool>>);

fn code(rows: &mut dyn Iterator<Item = &str>) -> Option<Code> {
    let cells: Vec<Vec<bool>> = rows.filter(|row| !row.is_empty()).map(|row| row.bytes().map(|cell| cell == b'1').collect()).collect();
    (!cells.is_empty() && cells.iter().all(|row| row.len() == cells.len())).then_some(Code(cells))
}

/// A pairing offered to one phone, as the QR code it scans: with the app,
/// or (`web`) with its camera, which opens the remote's page in its browser.
pub fn offer(web: bool) -> Option<Code> {
    let said = ask(if web { &["pair", "web", "cells"] } else { &["pair", "cells"] })?;
    // (The link itself, then the code's rows.)
    code(&mut said.lines().skip(1))
}

/// The QR code that holds `text`.
pub fn code_of(text: &str) -> Option<Code> {
    code(&mut ask(&["qr", text])?.lines())
}

/// The QR code that sends a phone to where the app is to be had.
pub fn app_code() -> Option<Code> {
    code_of(APP_PAGE)
}

/// Whether there is an OpenRGB for the remote to set the lights by.
pub fn lights() -> bool {
    ask(&["lights", "there"]).is_some_and(|said| said.trim() == "there")
}

/// Fetches OpenRGB for the remote (some twenty megabytes, from its own
/// project), or takes the one fetched away: whether there is one then.
/// Takes as long as the fetching does.
pub fn set_lights(on: bool) -> bool {
    ask(&["lights", if on { "fetch" } else { "forget" }]).is_some_and(|said| said.lines().last().is_some_and(|last| last.trim() == "there"))
}

/// Whether the remote asks an OpenRGB run with an administrator's rights (see `openrgb`), and starts none of its own.
pub fn lights_raised() -> bool {
    ask(&["lights", "raised"]).is_some_and(|said| said.trim() == "raised")
}

/// OpenRGB run with an administrator's rights, or as the remote runs it: the release fetched for that first (some
/// twenty megabytes), then the remote told, and the sensors asked. Whether it is so then. Takes as long as that does.
pub fn raise_lights(on: bool) -> bool {
    use crate::link::Switch;
    if !on {
        crate::source::switch(Switch::RaisedLights, false);
        return ask(&["lights", "lower"]).is_none_or(|said| said.trim() != "plain");
    }
    if !ask(&["lights", "raise"]).is_some_and(|said| said.lines().last().is_some_and(|last| last.trim() == "raised")) {
        return false;
    }
    let raised = crate::source::switch(Switch::RaisedLights, true);
    if !raised {
        let _ = ask(&["lights", "lower"]);
    }
    raised
}

/// Whether the remote puts the screens out with their signal kept (each
/// monitor turned off by itself, or covered in black) rather than as
/// Windows does, by stopping the signal to them all.
pub fn screens_kept() -> bool {
    ask(&["screens"]).is_some_and(|said| said.trim() == "kept")
}

/// Has the remote put the screens out that way, or Windows' own: which it then does.
pub fn keep_screens(kept: bool) -> bool {
    ask(&["screens", if kept { "kept" } else { "signal" }]).is_some_and(|said| said.trim() == "kept")
}

/// Asks the remote that listens on this PC what a phone would (`method`
/// and `path`, with `body` where it sends one): how the PC is then, as a
/// phone is answered; none where the remote did not do it or does not
/// answer.
pub fn said(method: &str, path: &str, body: &str) -> Option<serde_json::Value> {
    let answer = ask(&["ask", method, path, body])?;
    let mut lines = answer.lines();
    (lines.next()?.trim() == "200").then(|| serde_json::from_str(lines.next()?).ok()).flatten()
}

/// Where the remote's page for a phone's browser is (a browser paired by
/// it before opens it again there): none while this PC is on no network.
pub fn web_address() -> Option<String> {
    ask(&["address"]).map(|said| said.trim().to_string()).filter(|address| !address.is_empty())
}

/// What the remote finds on this PC (its lights, its monitors, what else
/// it can set), as lines for a report; none where this Glance has no remote.
pub fn report() -> Option<String> {
    ask(&["report"])
}

/// A phone paired with: what the remote tells it from the others by, and what it calls itself.
pub struct Phone {
    pub id: String,
    pub name: String,
}

/// The phones paired with, as the remote keeps them (pairings offered and not yet taken are not among them).
pub fn phones() -> Vec<Phone> {
    let kept = remote_config();
    let config: Option<serde_json::Value> = kept.and_then(|path| std::fs::read_to_string(path).ok()).and_then(|text| serde_json::from_str(&text).ok());
    let Some(list) = config.as_ref().and_then(|config| config.get("phones")).and_then(|phones| phones.as_array()) else { return Vec::new() };
    let text = |phone: &serde_json::Value, name: &str| phone.get(name).and_then(|value| value.as_str()).unwrap_or_default().to_string();
    list.iter().filter(|phone| phone.get("offered_till").is_none_or(|till| till.is_null())).map(|phone| Phone { id: text(phone, "id"), name: text(phone, "name") }).filter(|phone| !phone.id.is_empty()).collect()
}

/// Where the remote keeps what it was told for this user, the phones paired among it.
fn remote_config() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|roaming| PathBuf::from(roaming).join("dev.weiyi.glance.remote").join("config.json"))
}

/// When the remote last wrote what it keeps (a phone paired or unpaired among it); none where it keeps nothing.
pub fn phones_written() -> Option<std::time::SystemTime> {
    std::fs::metadata(remote_config()?).and_then(|meta| meta.modified()).ok()
}

/// Unpairs the phone the remote tells from the others by `id`.
pub fn unpair(id: &str) {
    let _ = ask(&["unpair", id]);
}

/// Windows' own `netsh`, from where Windows keeps it (and from nowhere
/// else that a program of that name might have been put).
fn netsh() -> PathBuf {
    let mut folder = [0u16; 260];
    let length = unsafe { GetSystemDirectoryW(Some(&mut folder)) } as usize;
    PathBuf::from(String::from_utf16_lossy(&folder[..length.min(folder.len())])).join("netsh.exe")
}

fn firewall(arguments: &[&str]) -> bool {
    Command::new(netsh()).args(["advfirewall", "firewall"]).args(arguments).creation_flags(NO_WINDOW).output().is_ok_and(|done| done.status.success())
}

/// The firewall closed to the remote and nearby transfer again: for the uninstaller.
pub fn forget() {
    for rule in [RULE, NEARBY_RULE] {
        firewall(&["delete", "rule", &format!("name={rule}")]);
    }
}

/// The remote running, if it is on: asked of it as Glance starts. On a laptop, where the sensors run no OpenRGB (see
/// `openrgb::runs_on`), a remote told to ask theirs (by an earlier Glance) runs its own again: else it would start none,
/// and set no lights.
pub fn keep() {
    if let Some(helper) = helper() {
        if lowered_as_glance_starts(crate::laptop::this_machine(), lights_raised) {
            let _ = ask(&["lights", "lower"]);
        }
        elevation::run_remote(&helper);
    }
}

/// Whether the remote is told, as Glance starts, to run its own OpenRGB again: on a `laptop`, where it was told to ask
/// the one the sensors run (`raised` says, asked of it on a laptop alone).
fn lowered_as_glance_starts(laptop: bool, raised: impl FnOnce() -> bool) -> bool {
    laptop && raised()
}

/// Whether the remote is on: its helper started at sign-in, by this account's task, for phones to control this PC.
pub fn on() -> bool {
    running() && kept_flags().0
}

/// Whether nearby transfer is on: the helper started at sign-in, by this account's task, for devices nearby.
pub fn nearby_on() -> bool {
    running() && kept_flags().1
}

/// Whether this account's task starts the helper beside this Glance.
fn running() -> bool {
    helper().is_some_and(|helper| elevation::remote_enabled(&helper))
}

/// What the helper keeps of what it runs for, as it starts: phones controlling this PC (unless told otherwise, they
/// do), and nearby transfer (unless told so, not).
fn kept_flags() -> (bool, bool) {
    let config: Option<serde_json::Value> = remote_config().and_then(|path| std::fs::read_to_string(path).ok()).and_then(|text| serde_json::from_str(&text).ok());
    flags_of(config.as_ref())
}

fn flags_of(config: Option<&serde_json::Value>) -> (bool, bool) {
    let flag = |at: &str, unsaid: bool| config.and_then(|config| config.pointer(at)).and_then(serde_json::Value::as_bool).unwrap_or(unsaid);
    (flag("/remote", true), flag("/nearby/on", false))
}

/// Turns the remote on or off (see `set`): whether it is on then.
pub fn turn(on: bool) -> bool {
    set(Some(on), None).0
}

/// Turns nearby transfer on or off (see `set`): whether it is on then.
pub fn turn_nearby(on: bool) -> bool {
    set(None, Some(on)).1
}

/// The helper set to run for phones (`remote`) and for devices nearby (`nearby`), each as asked, or else as it is:
/// told what it runs for, the firewall opened to the local network for it as each needs (and closed as neither does),
/// and its task made and run anew while either is on (so it takes what it was told), else stopped and gone. Whether
/// each is on then.
fn set(remote: Option<bool>, nearby: Option<bool>) -> (bool, bool) {
    let Some(helper) = helper() else { return (false, false) };
    let running = elevation::remote_enabled(&helper);
    let kept = kept_flags();
    let (remote, nearby) = (remote.unwrap_or(running && kept.0), nearby.unwrap_or(running && kept.1));
    let word = |on: bool| if on { "on" } else { "off" };
    let _ = asked(&helper, &["remote", word(remote)]);
    let _ = asked(&helper, &["nearby", word(nearby)]);
    let program = format!("program={}", helper.display());
    for (rule, wanted) in [(RULE, remote), (NEARBY_RULE, nearby)] {
        let name = format!("name={rule}");
        // (Whatever rule of that name there was goes first: one, and current.)
        firewall(&["delete", "rule", &name]);
        if !wanted {
            continue;
        }
        let open = |protocol: &str, port: Option<u16>| {
            let port = port.map(|port| format!("localport={port}"));
            let mut arguments = vec!["add", "rule", &name, "dir=in", "action=allow", protocol];
            arguments.extend(port.as_deref());
            arguments.extend(["remoteip=localsubnet", "profile=any", &program]);
            firewall(&arguments);
        };
        if rule == RULE {
            // (What phones ask of it, and their looking for it when its address has changed.)
            open("protocol=TCP", Some(PORT));
            open("protocol=UDP", Some(PORT));
        } else {
            // (What devices send and ask of it, on the port it receives on, another where that one was taken; and
            // their saying they are there.)
            open("protocol=TCP", None);
            open("protocol=UDP", Some(NEARBY_PORT));
        }
    }
    let is_on = elevation::set_remote(&helper, remote || nearby);
    // Ended while it kept the screens dark its own way, it has not put them back: it is asked to now.
    if !is_on {
        let _ = asked(&helper, &["restore"]);
    }
    let kept = kept_flags();
    (is_on && kept.0, is_on && kept.1)
}

/// Nearby transfer as the helper keeps it (see its `nearby`), what the settings show of it.
#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize)]
#[serde(default)]
pub struct Nearby {
    /// What this PC is called to devices nearby where it is named (none: as its computer is, `name`).
    pub alias: Option<String>,
    pub name: String,
    pub trusted: Vec<Trusted>,
    /// Devices not trusted are turned away, not asked about.
    pub others_refused: bool,
}

impl Nearby {
    /// What devices nearby see this PC called.
    pub fn called(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.name)
    }
}

/// A device whose files are taken without asking: its certificate's fingerprint, what it calls itself, what it is,
/// since when it is trusted (seconds since 1970; none known: 0), and the paired phone it is trusted by, where one told it.
#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize)]
#[serde(default)]
pub struct Trusted {
    pub fingerprint: String,
    pub alias: String,
    pub kind: Option<String>,
    pub since: u64,
    pub phone: Option<String>,
}

/// Nearby transfer as the helper keeps it, once it is told `words` (see its `nearby`: `alias NAME`, `forget
/// FINGERPRINT`, `others ask|refuse`); none where it cannot be asked.
pub fn nearby(words: &[&str]) -> Option<Nearby> {
    let said = ask(&std::iter::once("nearby").chain(words.iter().copied()).collect::<Vec<_>>())?;
    serde_json::from_str(&said).ok()
}

/// Tells the helper the most one sending to this PC may be (bytes; each of its files too): what the station has room
/// for. Whether it was told.
pub fn nearby_most(bytes: u64) -> bool {
    let bytes = bytes.to_string();
    ask(&["nearby", "most", &bytes, &bytes]).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the helper runs for, as it keeps it: phones control this PC unless it was told otherwise, and nearby
    /// transfer is off unless it was told it is on.
    #[test]
    fn the_helper_runs_for_phones_unless_told_otherwise() {
        let flags = |text: &str| flags_of(serde_json::from_str::<serde_json::Value>(text).ok().as_ref());
        assert_eq!(flags_of(None), (true, false));
        assert_eq!(flags("{}"), (true, false));
        assert_eq!(flags(r#"{"remote": false, "nearby": {"on": true}}"#), (false, true));
        assert_eq!(flags(r#"{"nearby": {"alias": "Desk"}}"#), (true, false));
    }

    /// Nearby transfer as the helper says it (see `run::said` there).
    #[test]
    fn nearby_transfer_is_read_as_the_helper_says_it() {
        let said = r#"{"on":true,"alias":null,"name":"DESKTOP-9950X","fingerprint":"AB","port":53317,"trusted":[{"fingerprint":"CD","alias":"Galaxy Z Fold6","kind":"mobile","last":0,"since":1762905600,"phone":"p1"},{"fingerprint":"EF","alias":"MacBook","kind":"desktop","last":0,"since":0}],"others_refused":true}"#;
        let nearby: Nearby = serde_json::from_str(said).unwrap();
        assert_eq!(nearby.called(), "DESKTOP-9950X");
        assert!(nearby.others_refused);
        assert_eq!(nearby.trusted.len(), 2);
        assert_eq!((nearby.trusted[0].phone.as_deref(), nearby.trusted[0].since), (Some("p1"), 1762905600));
        assert_eq!(nearby.trusted[1].phone, None);
        let named: Nearby = serde_json::from_str(r#"{"alias":"书房","name":"DESKTOP-9950X"}"#).unwrap();
        assert_eq!(named.called(), "书房");
    }

    /// On a laptop, a remote told to ask an OpenRGB the sensors run (which they run on no laptop) is told to run its
    /// own again as Glance starts. Elsewhere it is not asked.
    #[test]
    fn a_laptops_remote_runs_its_own_lights_program() {
        assert!(lowered_as_glance_starts(true, || true));
        assert!(!lowered_as_glance_starts(true, || false));
        assert!(!lowered_as_glance_starts(false, || panic!("asked on a desktop")));
    }

    #[test]
    fn arguments_are_carried_as_windows_reads_them_back() {
        assert_eq!(quoted("pair"), "pair");
        assert_eq!(quoted(""), "\"\"");
        assert_eq!(quoted("a b"), "\"a b\"");
        assert_eq!(quoted("C:\\Program Files\\"), "\"C:\\Program Files\\\\\"");
        assert_eq!(quoted("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert!(netsh().is_file());
    }

    /// The remote asked as the user is without an administrator's rights
    /// (to be run elevated, where that is another thing than this
    /// program's own rights): it answers, and what it keeps stays the user's.
    #[test]
    #[ignore = "needs administrator rights and the phone remote installed"]
    fn the_remote_is_asked_without_an_administrators_rights() {
        let helper = std::env::current_exe().unwrap().ancestors().find_map(|folder| Some(folder.join("run").join("glance-remote.exe")).filter(|helper| helper.is_file())).expect("a remote built beside");
        let said = said_plainly(&helper, &["qr", "a b"]).expect("an answer");
        assert!(code(&mut said.lines()).is_some(), "{said}");
        assert!(said_plainly(&helper, &["report"]).is_some_and(|said| said.contains("version")));
        // And what is started so stands as the user's ordinary programs do ("medium", not "high").
        let standing = said_plainly(&netsh().with_file_name("whoami.exe"), &["/groups"]).expect("an answer");
        assert!(standing.contains("S-1-16-8192") && !standing.contains("S-1-16-12288"), "{standing}");
    }

    #[test]
    fn a_code_is_square_or_none() {
        let read = code(&mut "101\n010\n111\n".lines()).unwrap();
        assert_eq!(read.0, [[true, false, true], [false, true, false], [true, true, true]]);
        assert!(code(&mut "101\n01\n".lines()).is_none());
        assert!(code(&mut "".lines()).is_none());
    }
}

#[cfg(test)]
mod quiet_tests {
    /// A program started as the helper is, its output read back.
    #[test]
    fn runs_a_program_quietly() {
        let cmd = std::path::PathBuf::from(std::env::var("ComSpec").unwrap());
        let (ended, said, _) = super::run_plainly(&cmd, &["/c", "echo", "heard"]).expect("started");
        assert_eq!((ended, said.trim()), (0, "heard"));
    }
}
