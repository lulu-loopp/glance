//! Claude's limits as Claude Code tells them to its status line (the way its documentation gives): Glance, asked
//! (`glance.exe --statusline`), is the status line Claude Code runs. It keeps the limits it is told in a folder of
//! Glance's own (see `folder`), read as they are written (see `watch`), and passes what it is told on to the status
//! line it took the place of (another tool's, as claude-hud's), run by the same shell, whose words are shown as they
//! were; with none, it shows nothing.
//!
//! It is put in Claude Code's settings, and taken out, only as the user asks it on the AI component's page: its
//! `statusLine` alone is written there, the rest of the file as it was, and what was there before is kept to be put
//! back.

use std::io::{Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;

use super::{Limit, Tool, Window};

/// The flag Claude Code runs Glance with as its status line.
pub const ASKED: &str = "--statusline";
/// Glance's folder for it (in its local folder): the limits last told, and the status line it took the place of.
const FOLDER: &str = "claude-status";
pub(super) const LIMITS: &str = "limits.json";
const REPLACED: &str = "replaced.json";
/// The setting Claude Code reads its status line from.
const KEY: &str = "statusLine";
/// The id of the limit it tells (of Claude's: see `Limit`).
pub(super) const ID: &str = "claude";

/// Glance's folder for what the status line is told.
pub fn folder() -> PathBuf {
    crate::settings::local_dir().join(FOLDER)
}

/// What Claude Code gives its status line, as far as the limits go.
#[derive(Deserialize)]
struct Given {
    rate_limits: Option<Rates>,
}

#[derive(Deserialize)]
struct Rates {
    five_hour: Option<Rate>,
    seven_day: Option<Rate>,
}

/// How much of a window is used (0 to 100) and when it starts again (seconds since 1970).
#[derive(Deserialize)]
struct Rate {
    used_percentage: f32,
    resets_at: Option<f64>,
}

/// The limit `given` tells at `now`: none where it tells none (an account with no subscription's limits, or a session
/// before its first answer). Each window may be left out (Claude Code drops one as it starts again).
fn limit_of(given: &[u8], now: u64) -> Option<Limit> {
    let rates = serde_json::from_slice::<Given>(given).ok()?.rate_limits?;
    let windows: Vec<Window> = [(300, rates.five_hour), (10080, rates.seven_day)]
        .into_iter()
        .filter_map(|(minutes, rate)| rate.map(|rate| Window { minutes, used: rate.used_percentage, resets: rate.resets_at.map(|seconds| (seconds * 1000.0) as u64) }))
        .collect();
    (!windows.is_empty()).then(|| Limit { tool: Tool::Claude, id: ID.into(), name: None, plan: None, windows, told: now })
}

/// The limit last told, as kept in `folder` (none where none was, or it cannot be read).
pub(super) fn told_in(folder: &Path) -> Option<Limit> {
    serde_json::from_slice(&std::fs::read(folder.join(LIMITS)).ok()?).ok()
}

/// Glance run as Claude Code's status line: the limits it is told kept, and what it is told passed on to the status
/// line it took the place of. Its exit code, that one's (or 0).
pub fn run() -> i32 {
    let mut given = Vec::new();
    if std::io::stdin().read_to_end(&mut given).is_err() {
        return 1;
    }
    let folder = folder();
    keep_told(&given, &folder, crate::station::now_ms());
    match (replaced_command(&folder), parent_program()) {
        (Some(command), Some(shell)) => pass_on(&shell, &command, &given),
        _ => 0,
    }
}

/// The limit `given` tells at `now` kept in `folder`, where it tells one.
fn keep_told(given: &[u8], folder: &Path, now: u64) {
    if let Some(limit) = limit_of(given, now) {
        if std::fs::create_dir_all(folder).is_ok() {
            let _ = super::write_whole(folder, LIMITS, &serde_json::to_vec(&limit).unwrap_or_default());
        }
    }
}

/// The status line it took the place of, as it was in Claude Code's settings (its JSON as written there; none where
/// there was none).
#[derive(serde::Serialize, Deserialize)]
struct Replaced {
    value: Option<String>,
}

/// The command of the status line it took the place of, where there was one.
fn replaced_command(folder: &Path) -> Option<String> {
    let replaced: Replaced = serde_json::from_slice(&std::fs::read(folder.join(REPLACED)).ok()?).ok()?;
    command_of(&replaced.value?)
}

/// The command a status line (its setting's JSON) runs, where it runs one.
fn command_of(value: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(value).ok()?;
    (value.get("type")?.as_str()? == "command").then_some(())?;
    Some(value.get("command")?.as_str()?.to_string())
}

/// `command` run by `shell`, the one that ran Glance (Claude Code runs a status line through Git Bash, or PowerShell
/// where there is none), given `given`, its words going where Glance's would and ending with it (Claude Code ends a
/// status line still at work as a newer is asked for). Its exit code (0 for a shell Claude Code does not run it by).
fn pass_on(shell: &Path, command: &str, given: &[u8]) -> i32 {
    use std::os::windows::process::CommandExt;
    use windows::Win32::System::Threading::CREATE_NO_WINDOW;
    let name = shell.file_stem().map(|stem| stem.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut run = Command::new(shell);
    match name.as_str() {
        "bash" | "sh" => run.arg("-c").arg(command),
        "powershell" | "pwsh" => run.args(["-NoProfile", "-Command"]).arg(command),
        "cmd" => run.args(["/d", "/s", "/c"]).raw_arg(format!("\"{command}\"")),
        _ => return 0,
    };
    // (Glance has no console of its own: the shell is given none, its words going where Glance's go.)
    run.creation_flags(CREATE_NO_WINDOW.0);
    let Ok(mut child) = run.stdin(Stdio::piped()).stdout(Stdio::inherit()).stderr(Stdio::inherit()).spawn() else { return 1 };
    ends_with_this(&child);
    if let Some(mut input) = child.stdin.take() {
        let _ = input.write_all(given);
    }
    child.wait().ok().and_then(|status| status.code()).unwrap_or(1)
}

/// `child` (and what it starts) ended as this process ends, however it does.
fn ends_with_this(child: &std::process::Child) {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE};
    unsafe {
        // (Its handle held till this process ends, which closes it.)
        let Ok(job) = CreateJobObjectW(None, windows::core::PCWSTR::null()) else { return };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(job, JobObjectExtendedLimitInformation, (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(), size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32).is_ok() {
            let _ = AssignProcessToJobObject(job, HANDLE(child.as_raw_handle()));
        }
    }
}

/// The program of the process that ran this one.
fn parent_program() -> Option<PathBuf> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    use windows::Win32::System::Threading::{GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
    let own = unsafe { GetCurrentProcessId() };
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.ok()?;
    let mut each = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut parent = None;
    let mut more = unsafe { Process32FirstW(snapshot, &mut each) }.is_ok();
    while more {
        if each.th32ProcessID == own {
            parent = Some(each.th32ParentProcessID);
            break;
        }
        more = unsafe { Process32NextW(snapshot, &mut each) }.is_ok();
    }
    let _ = unsafe { CloseHandle(snapshot) };
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, parent?) }.ok()?;
    let mut name = [0u16; 1024];
    let mut length = name.len() as u32;
    let found = unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, windows::core::PWSTR(name.as_mut_ptr()), &mut length) };
    let _ = unsafe { CloseHandle(process) };
    found.ok()?;
    Some(PathBuf::from(String::from_utf16_lossy(&name[..length as usize])))
}

/// Claude Code's settings file: in the first of `CLAUDE_CONFIG_DIR`'s folders where it is set, else in `.claude` in
/// the user's folder.
#[cfg(not(test))]
fn settings_file() -> Option<PathBuf> {
    let dir = match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dirs) => std::env::split_paths(&dirs).next()?,
        None => PathBuf::from(std::env::var_os("USERPROFILE")?).join(".claude"),
    };
    Some(dir.join("settings.json"))
}

/// In a test, one in a folder of the test process's own (a test never reads or writes the user's).
#[cfg(test)]
fn settings_file() -> Option<PathBuf> {
    Some(std::env::temp_dir().join(format!("glance-test-{}", std::process::id())).join("claude").join("settings.json"))
}

/// Whether Claude Code has Git Bash to run its status line with (as it finds it: where `CLAUDE_CODE_GIT_BASH_PATH`
/// says, else beside the Git on the path), else PowerShell.
fn git_bash() -> bool {
    if std::env::var_os("CLAUDE_CODE_GIT_BASH_PATH").is_some() {
        return true;
    }
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("git.exe").is_file() && dir.parent().is_some_and(|git| git.join("bin").join("bash.exe").is_file())))
}

/// The command that runs `exe` as the status line, written for the shell that runs it: its path with forward slashes
/// (Git Bash takes backslashes away), quoted where it holds a space (PowerShell runs a quoted path with `&`).
fn command_for(exe: &Path, git_bash: bool) -> String {
    let path = exe.to_string_lossy().replace('\\', "/");
    match (path.contains(' '), git_bash) {
        (false, _) => format!("{path} {ASKED}"),
        (true, true) => format!("\"{path}\" {ASKED}"),
        (true, false) => format!("& \"{path}\" {ASKED}"),
    }
}

/// Whether a status line's setting (its JSON) is this one.
fn is_ours(value: &str) -> bool {
    command_of(value).is_some_and(|command| command.contains(&format!(" {ASKED}")) && command.to_lowercase().contains("glance"))
}

/// How it stands in Claude Code's settings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Standing {
    /// Claude Code is not on this PC (it has no settings folder).
    NoClaude,
    Out,
    In,
}

/// How it stands in Claude Code's settings now.
pub fn standing() -> Standing {
    let Some(file) = settings_file() else { return Standing::NoClaude };
    if !file.parent().is_some_and(Path::is_dir) {
        return Standing::NoClaude;
    }
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    match members(&text).and_then(|(members, _)| members.into_iter().find(|member| member.key == KEY)) {
        Some(member) if is_ours(&text[member.value.clone()]) => Standing::In,
        _ => Standing::Out,
    }
}

/// Puts it in Claude Code's settings as its status line, what was there kept to be passed on and put back.
pub fn put_in() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let file = settings_file().ok_or("no settings folder")?;
    // (Written where it is, where it is a link.)
    let file = std::fs::canonicalize(&file).unwrap_or(file);
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "{}\n".to_string(),
        Err(error) => return Err(error.to_string()),
    };
    let (members, _) = members(&text).ok_or("its settings are not a JSON object")?;
    let had = members.iter().find(|member| member.key == KEY).map(|member| text[member.value.clone()].to_string());
    if had.as_deref().is_some_and(is_ours) {
        return Ok(());
    }
    let ours = ours_from(had.as_deref(), &command_for(&exe, git_bash()));
    let written = with_member(&text, KEY, Some(&ours)).ok_or("its settings could not be written")?;
    let folder = folder();
    std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    // (What was there kept first: the status line is never left without it.)
    super::write_whole(&folder, REPLACED, &serde_json::to_vec(&Replaced { value: had }).unwrap_or_default()).map_err(|error| error.to_string())?;
    write_settings(&file, &written)
}

/// Takes it out of Claude Code's settings, the status line it took the place of put back (where another has not taken
/// its place since).
pub fn take_out() -> Result<(), String> {
    let folder = folder();
    let replaced: Option<Replaced> = std::fs::read(folder.join(REPLACED)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let file = settings_file().ok_or("no settings folder")?;
    let file = std::fs::canonicalize(&file).unwrap_or(file);
    if let Ok(text) = std::fs::read_to_string(&file) {
        let (members, _) = members(&text).ok_or("its settings are not a JSON object")?;
        if members.iter().any(|member| member.key == KEY && is_ours(&text[member.value.clone()])) {
            let back = replaced.and_then(|replaced| replaced.value);
            let written = with_member(&text, KEY, back.as_deref()).ok_or("its settings could not be written")?;
            write_settings(&file, &written)?;
        }
    }
    let _ = std::fs::remove_file(folder.join(REPLACED));
    let _ = std::fs::remove_file(folder.join(LIMITS));
    Ok(())
}

/// Its status line's setting, written as the setting there was writes it (another's padding and how often it is run
/// again kept), its command `command`: indented as a member of the settings' object is.
fn ours_from(had: Option<&str>, command: &str) -> String {
    let mut value = had.and_then(|had| serde_json::from_str::<serde_json::Value>(had).ok()).filter(serde_json::Value::is_object).unwrap_or_else(|| serde_json::json!({}));
    value["type"] = "command".into();
    value["command"] = command.into();
    serde_json::to_string_pretty(&value).unwrap_or_default().replace('\n', "\n  ")
}

/// `text` written as Claude Code's settings at `file`, checked to be JSON first.
fn write_settings(file: &Path, text: &str) -> Result<(), String> {
    serde_json::from_str::<serde_json::Value>(text).map_err(|error| error.to_string())?;
    let (dir, name) = (file.parent().ok_or("no folder")?, file.file_name().ok_or("no name")?.to_string_lossy());
    super::write_whole(dir, &name, text.as_bytes()).map_err(|error| error.to_string())
}

/// A member of a JSON object as it is written: where it begins (its key's quote), its key, and where its value is.
#[derive(Debug, PartialEq)]
struct Member {
    start: usize,
    key: String,
    value: Range<usize>,
}

/// The members of the JSON object `text` is (whitespace round it), as written, and where its closing brace is; none
/// where it is not one.
fn members(text: &str) -> Option<(Vec<Member>, usize)> {
    let bytes = text.as_bytes();
    let space = |mut at: usize| {
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        at
    };
    // (A byte-order mark before it, as some editors write.)
    let mut at = space(if text.starts_with('\u{feff}') { 3 } else { 0 });
    (bytes.get(at) == Some(&b'{')).then_some(())?;
    at = space(at + 1);
    let mut members = Vec::new();
    if bytes.get(at) == Some(&b'}') {
        return Some((members, at));
    }
    loop {
        let start = at;
        let key_end = value_end(bytes, at)?;
        let key: String = serde_json::from_str(&text[start..key_end]).ok()?;
        at = space(key_end);
        (bytes.get(at) == Some(&b':')).then_some(())?;
        let value_start = space(at + 1);
        let end = value_end(bytes, value_start)?;
        members.push(Member { start, key, value: value_start..end });
        at = space(end);
        match bytes.get(at) {
            Some(b',') => at = space(at + 1),
            Some(b'}') => return Some((members, at)),
            _ => return None,
        }
    }
}

/// Where the JSON value that begins at `at` ends.
fn value_end(bytes: &[u8], at: usize) -> Option<usize> {
    let string_end = |mut at: usize| {
        at += 1;
        while at < bytes.len() {
            match bytes[at] {
                b'\\' => at += 2,
                b'"' => return Some(at + 1),
                _ => at += 1,
            }
        }
        None
    };
    match bytes.get(at)? {
        b'"' => string_end(at),
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut at = at;
            while at < bytes.len() {
                match bytes[at] {
                    b'"' => {
                        at = string_end(at)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(at + 1);
                        }
                    }
                    _ => {}
                }
                at += 1;
            }
            None
        }
        _ => {
            let mut end = at;
            while end < bytes.len() && !matches!(bytes[end], b',' | b'}' | b']') && !bytes[end].is_ascii_whitespace() {
                end += 1;
            }
            (end > at).then_some(end)
        }
    }
}

/// `text` (a JSON object as written) with its member `key` given `value` (JSON as it is to be written), added at its
/// end where it has none, or (no value) taken out; all else as it was written.
fn with_member(text: &str, key: &str, value: Option<&str>) -> Option<String> {
    let (members, close) = members(text)?;
    let at = members.iter().position(|member| member.key == key);
    let written = match (at, value) {
        (Some(at), Some(value)) => format!("{}{value}{}", &text[..members[at].value.start], &text[members[at].value.end..]),
        // Taken out with the comma between it and the next, or else the last's before it.
        (Some(at), None) => match (members.get(at + 1), at.checked_sub(1).map(|before| &members[before])) {
            (Some(next), _) => format!("{}{}", &text[..members[at].start], &text[next.start..]),
            (None, Some(before)) => format!("{}{}", &text[..before.value.end], &text[members[at].value.end..]),
            (None, None) => format!("{}{}", &text[..members[at].start], &text[close..]),
        },
        (None, None) => text.to_string(),
        // Added after the last, indented as the first is (else by two spaces).
        (None, Some(value)) => {
            let key = serde_json::to_string(key).ok()?;
            match members.last() {
                Some(last) => {
                    let line_start = text[..members[0].start].rfind('\n').map_or(0, |at| at + 1);
                    let indent = &text[line_start..members[0].start];
                    let indent = if indent.chars().all(char::is_whitespace) { indent } else { "  " };
                    format!("{},\n{indent}{key}: {value}{}", &text[..last.value.end], &text[last.value.end..])
                }
                None => format!("{}\n  {key}: {value}\n{}", &text[..close].trim_end(), &text[close..]),
            }
        }
    };
    Some(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What Claude Code gives its status line: each window's share and when it starts again (seconds, as milliseconds
    /// here); a window left out, left out; none told, none.
    #[test]
    fn reads_the_limits_claude_code_tells() {
        let given = br#"{"model":{"display_name":"Opus"},"rate_limits":{"five_hour":{"used_percentage":23.5,"resets_at":1738425600},"seven_day":{"used_percentage":41.2,"resets_at":1738857600}}}"#;
        let limit = limit_of(given, 7).unwrap();
        assert_eq!((limit.tool, limit.id.as_str(), limit.told), (Tool::Claude, "claude", 7));
        assert_eq!(limit.windows, [Window { minutes: 300, used: 23.5, resets: Some(1_738_425_600_000) }, Window { minutes: 10080, used: 41.2, resets: Some(1_738_857_600_000) }]);
        let given = br#"{"rate_limits":{"seven_day":{"used_percentage":41.2,"resets_at":1738857600}}}"#;
        assert_eq!(limit_of(given, 7).unwrap().windows.len(), 1);
        assert!(limit_of(br#"{"model":{}}"#, 7).is_none() && limit_of(br#"{"rate_limits":{}}"#, 7).is_none() && limit_of(b"not json", 7).is_none());
    }

    /// The limit told kept in Glance's folder for it, and read from there as it was told; none told, the one kept before
    /// left as it was.
    #[test]
    fn keeps_the_limits_it_is_told() {
        let folder = folder().join("keeps");
        keep_told(br#"{"rate_limits":{"five_hour":{"used_percentage":12,"resets_at":1738425600}}}"#, &folder, 9);
        let kept = told_in(&folder).unwrap();
        assert_eq!((kept.told, kept.windows.as_slice()), (9, [Window { minutes: 300, used: 12.0, resets: Some(1_738_425_600_000) }].as_slice()));
        keep_told(br#"{"model":{}}"#, &folder, 10);
        assert_eq!(told_in(&folder).unwrap().told, 9);
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// What it is told passed on whole to the status line it took the place of, by each shell Claude Code may run it
    /// with, that one's exit code its own (the command telling by its exit code that it was given it).
    #[test]
    fn passes_what_it_is_told_on_by_the_shell_that_ran_it() {
        let given = br#"{"model":{"display_name":"Opus"}}"#;
        let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
        let powershell = system.join(r"WindowsPowerShell\v1.0\powershell.exe");
        assert_eq!(pass_on(&powershell, "if ([Console]::In.ReadToEnd() -match 'Opus') { exit 7 } else { exit 3 }", given), 7);
        assert_eq!(pass_on(&system.join("cmd.exe"), "findstr /c:\"Opus\" >nul && exit 7 || exit 3", given), 7);
        // (Git Bash, where it is on this PC.)
        let bash = std::env::var_os("PATH").and_then(|path| std::env::split_paths(&path).map(|dir| dir.join("bash.exe")).find(|bash| bash.is_file() && !bash.starts_with(&system)));
        if let Some(bash) = bash {
            assert_eq!(pass_on(&bash, "grep -q '\"Opus\"' && exit 7 || exit 3", given), 7);
        }
        assert_eq!(pass_on(Path::new("node.exe"), "exit 7", given), 0);
    }

    /// The command that runs Glance as the status line, as each shell takes it.
    #[test]
    fn writes_the_command_for_the_shell_that_runs_it() {
        assert_eq!(command_for(Path::new(r"D:\Glance\glance.exe"), true), "D:/Glance/glance.exe --statusline");
        assert_eq!(command_for(Path::new(r"C:\Program Files\Glance\glance.exe"), true), "\"C:/Program Files/Glance/glance.exe\" --statusline");
        assert_eq!(command_for(Path::new(r"C:\Program Files\Glance\glance.exe"), false), "& \"C:/Program Files/Glance/glance.exe\" --statusline");
        assert!(is_ours(r#"{"type":"command","command":"D:/Glance/glance.exe --statusline"}"#));
        assert!(!is_ours(r#"{"type":"command","command":"bash -c 'node hud.js'"}"#));
        assert_eq!(command_of(r#"{"type":"command","command":"bash -c 'node \"hud.js\"'"}"#).as_deref(), Some("bash -c 'node \"hud.js\"'"));
    }

    /// The settings' status line put in, replacing what was there (its padding kept), or added at the end, and taken
    /// out again; the rest of the file written as it was, to its order, its spacing and what is in its strings.
    #[test]
    fn writes_only_the_status_line_in_claude_codes_settings() {
        let text = "{\n  \"env\": { \"A\": \"{,}\\\"\" },\n  \"statusLine\": {\n    \"type\": \"command\",\n    \"command\": \"bash -c 'hud'\",\n    \"padding\": 1\n  },\n  \"model\": \"opus\"\n}\n";
        let ours = ours_from(Some("{\n    \"type\": \"command\",\n    \"command\": \"bash -c 'hud'\",\n    \"padding\": 1\n  }"), "D:/Glance/glance.exe --statusline");
        let written = with_member(text, KEY, Some(&ours)).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert_eq!(parsed["statusLine"]["command"], "D:/Glance/glance.exe --statusline");
        assert_eq!(parsed["statusLine"]["padding"], 1);
        assert!(written.starts_with("{\n  \"env\": { \"A\": \"{,}\\\"\" },\n  \"statusLine\": {") && written.ends_with("},\n  \"model\": \"opus\"\n}\n"), "{written}");
        // (Put back as it was written.)
        let had = &text[members(text).unwrap().0[1].value.clone()];
        assert_eq!(with_member(&written, KEY, Some(had)).unwrap(), text);
        // None there: added after the last, as indented; taken out again, as it was.
        let none = "{\n    \"model\": \"opus\"\n}\n";
        let added = with_member(none, KEY, Some("{ \"type\": \"command\", \"command\": \"x --statusline\" }")).unwrap();
        assert_eq!(added, "{\n    \"model\": \"opus\",\n    \"statusLine\": { \"type\": \"command\", \"command\": \"x --statusline\" }\n}\n");
        assert_eq!(with_member(&added, KEY, None).unwrap(), none);
        // First of several, taken out with the comma after it.
        assert_eq!(with_member("{\"statusLine\": 1, \"a\": 2}", KEY, None).unwrap(), "{\"a\": 2}");
        // An empty object, and one that is not an object.
        let empty = with_member("{}\n", KEY, Some("1")).unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&empty).unwrap()["statusLine"], 1);
        assert!(members("[1]").is_none() && members("{\"a\" 1}").is_none());
    }
}
