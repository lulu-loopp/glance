//! The reports this PC sent, followed as threads till their problems are fixed. Each was sent with a secret of its
//! own (random, made for it alone), of which Glance's site keeps only a hash: with the report's id it proves that
//! whoever asks sent it. Along with each update check, Glance asks the site what is new in the reports it sent (the
//! owner's replies, and changes of their status), with nothing but their ids, their secrets and how far it has
//! heard, and tells the user once of each report with news. The user reads a thread, adds to it, and closes it or
//! opens it again, from the settings. Kept beside the settings (`FILE`), with small copies of the pictures sent (`PICTURES`), and forgotten 90
//! days after anything last happened in it.

use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::ui::text::Lang;

/// Where they are kept, in the settings' folder, and the most of it that is read.
pub const FILE: &str = "reports.json";
const MOST: u64 = 4 << 20;
/// How long a report is followed after anything last happened in it (milliseconds).
const FOLLOWED_FOR: u64 = 90 * 86_400_000;
/// How often the site is asked while a report is still talked over (not fixed, not closed: see `talking`), beside the
/// update check's daily asking: a reply heard within the hour, the user can answer it while it matters.
const TALKING_EVERY: std::time::Duration = std::time::Duration::from_secs(60 * 60);
/// How many reports one asking names, the latest first (the site answers no more).
const ASKED_MOST: usize = 20;
/// The most of the user's own words kept (characters), as the window that takes them takes no more.
const WORDS_MOST: usize = 500;
/// The folder beside them where small copies of the pictures sent are kept, to be shown in their threads (see `keep`):
/// how large a copy is made (its longer side, pixels), and the most of one that is read.
pub const PICTURES: &str = "reports";
const PICTURE_SIDE: u32 = 480;
const PICTURE_MOST: u64 = 1 << 20;

/// The reports sent from this PC.
#[derive(Clone, Default, Serialize, Deserialize, Debug, PartialEq)]
pub struct Kept {
    pub reports: Vec<Report>,
}

/// A report sent: its id as the site gave it, the secret sent with it, when it was sent (ms since 1970), the user's
/// own words with it and how many pictures, its thread since, the highest number of the site's that asking has heard
/// (what is asked past), and the highest the user has read.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Report {
    pub id: String,
    secret: String,
    pub sent: u64,
    #[serde(default)]
    pub words: String,
    #[serde(default)]
    pub pictures: usize,
    #[serde(default)]
    pub thread: Vec<Message>,
    #[serde(default)]
    heard: u64,
    #[serde(default)]
    read: u64,
}

/// One message of a thread, numbered as the site numbers them: who said it (or the status it was set to), when
/// (ms), its words, a version (the Glance a user's addition came from, or the one with the fix), for the user's
/// own, how many pictures and whether the diagnostics went with it, and, of a status, whether the user set it.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Message {
    pub n: u64,
    pub ts: u64,
    pub kind: Kind,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub pictures: usize,
    #[serde(default)]
    pub diagnostics: bool,
    #[serde(default)]
    pub mine: bool,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// The user's own addition.
    User,
    /// The owner's reply.
    Reply,
    /// The status set: open again, answered, fixed (in the message's version), closed.
    Open,
    Answered,
    Fixed,
    Closed,
}

/// Where a report stands, as the owner or the user last set it.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Open,
    Answered,
    Fixed(String),
    Closed,
}

impl Report {
    /// The latest status set, or open, none set.
    pub fn status(&self) -> Status {
        self.thread
            .iter()
            .rev()
            .find_map(|message| match message.kind {
                Kind::Open => Some(Status::Open),
                Kind::Answered => Some(Status::Answered),
                Kind::Fixed => Some(Status::Fixed(message.version.clone())),
                Kind::Closed => Some(Status::Closed),
                Kind::User | Kind::Reply => None,
            })
            .unwrap_or(Status::Open)
    }

    /// Whether the site has told something the user has not read.
    pub fn unread(&self) -> bool {
        self.thread.iter().any(|message| message.kind != Kind::User && !message.mine && message.n > self.read)
    }

    /// Whether the user may close it (it is still talked over), or else open it again (it is fixed or closed: the fix
    /// did not help, or more is to be said).
    pub fn closable(&self) -> bool {
        matches!(self.status(), Status::Open | Status::Answered)
    }

    /// When anything last happened in it.
    pub fn active(&self) -> u64 {
        self.thread.iter().map(|message| message.ts).fold(self.sent, u64::max)
    }

    /// The first line of the user's words.
    pub fn first_line(&self) -> &str {
        self.words.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("")
    }
}

/// What the site says is new, as it says it.
#[derive(Deserialize)]
struct News {
    news: Vec<Told>,
}

#[derive(Deserialize)]
struct Told {
    id: String,
    n: u64,
    ts: u64,
    kind: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    version: String,
}

/// The reports followed no longer: nothing has happened in them for `FOLLOWED_FOR` by `now`. Those that were.
fn forget_old(kept: &mut Kept, now: u64) -> Vec<Report> {
    let (old, followed) = std::mem::take(&mut kept.reports).into_iter().partition(|report| report.active().saturating_add(FOLLOWED_FOR) < now);
    kept.reports = followed;
    old
}

/// What the site is asked, of the reports kept that `of` takes (the latest first), in as many askings as it takes, each
/// naming as many as the site answers: each one's id, its secret and the highest number heard of it. Nothing else.
/// None with no report to ask about.
fn asking(kept: &Kept, of: impl Fn(&Report) -> bool) -> Vec<String> {
    let mut latest: Vec<&Report> = kept.reports.iter().filter(|report| of(report)).collect();
    latest.sort_by_key(|report| std::cmp::Reverse(report.active()));
    latest
        .chunks(ASKED_MOST)
        .map(|some| {
            let asked: Vec<serde_json::Value> = some.iter().map(|report| serde_json::json!({ "id": report.id, "secret": report.secret, "after": report.heard })).collect();
            serde_json::json!({ "reports": asked }).to_string()
        })
        .collect()
}

/// The site's answer (`body`) taken into `kept`: each message of a report kept, of a kind Glance knows and not heard
/// before, put in its thread. The reports with news, each once, with what is newest in it.
fn heard(kept: &mut Kept, body: &[u8]) -> Vec<(String, Message)> {
    let Ok(news) = serde_json::from_slice::<News>(body) else { return Vec::new() };
    let mut told: Vec<(String, Message)> = Vec::new();
    for item in news.news {
        let kind = match item.kind.as_str() {
            "reply" => Kind::Reply,
            "open" => Kind::Open,
            "answered" => Kind::Answered,
            "fixed" => Kind::Fixed,
            "closed" => Kind::Closed,
            _ => continue,
        };
        let Some(report) = kept.reports.iter_mut().find(|report| report.id == item.id) else { continue };
        if item.n <= report.heard || report.thread.iter().any(|message| message.n == item.n) {
            continue;
        }
        report.heard = item.n;
        let message = Message { n: item.n, ts: item.ts, kind, text: item.text, version: item.version, pictures: 0, diagnostics: false, mine: false };
        report.thread.push(message.clone());
        report.thread.sort_by_key(|message| message.n);
        // A reply is told over a status set with it (the site marks a report answered as it is replied to).
        match told.iter_mut().find(|(id, _)| *id == item.id) {
            Some((_, newest)) if newest.kind == Kind::Reply && kind != Kind::Reply => {}
            Some((_, newest)) => *newest = message,
            None => told.push((item.id, message)),
        }
    }
    told
}

/// What the user is told of `id`'s news, `newest`: its title and words.
fn notice(lang: Lang, id: &str, newest: &Message) -> (String, String) {
    let first = newest.text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("");
    let first: String = first.chars().take(120).collect();
    let see = lang.pick("单击以查看。", "Click to view.").to_string();
    match (newest.kind, lang) {
        (Kind::Reply, Lang::Zh) => (format!("你的反馈 {id} 有新回复"), if first.is_empty() { see } else { first }),
        (Kind::Reply, Lang::En) => (format!("New reply to your report {id}"), if first.is_empty() { see } else { first }),
        (Kind::Fixed, _) => {
            let version = &newest.version;
            let words = match (crate::update::newer(version, env!("CARGO_PKG_VERSION")), lang) {
                (true, Lang::Zh) => format!("更新到 {version} 即可获得此修复。"),
                (true, Lang::En) => format!("Update to {version} to get the fix."),
                (false, Lang::Zh) => "当前版本已包含此修复。".to_string(),
                (false, Lang::En) => "This version includes the fix.".to_string(),
            };
            (lang.pick(&format!("你的反馈 {id} 已在 {version} 修复"), &format!("Your report {id} was fixed in {version}")).to_string(), words)
        }
        (Kind::Closed, _) => (lang.pick(&format!("你的反馈 {id} 已关闭"), &format!("Your report {id} was closed")).to_string(), see),
        (Kind::Answered, _) => (lang.pick(&format!("你的反馈 {id} 已回复"), &format!("Your report {id} was answered")).to_string(), see),
        (Kind::Open | Kind::User, _) => (lang.pick(&format!("你的反馈 {id} 已重新打开"), &format!("Your report {id} was reopened")).to_string(), see),
    }
}

/// The reports, as read from `dir`: none where there are none, or what is there does not read.
fn load(dir: &Path) -> Kept {
    crate::elevation::read_in_place(dir, FILE, MOST).and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

/// The reports written to `dir`, whole or not at all.
fn save(kept: &Kept, dir: &Path) -> std::io::Result<()> {
    crate::elevation::ensure_folder(dir)?;
    crate::elevation::write_in_place(dir, FILE, serde_json::to_string_pretty(kept).expect("the reports are written").as_bytes())
}

/// The reports as held while Glance runs (read the first time they are wanted).
static KEPT: Mutex<Option<Kept>> = Mutex::new(None);
/// The latest of them still to be written, and whether they are being written (see `save_later`).
static UNSAVED: Mutex<(Option<Kept>, bool)> = Mutex::new((None, false));

/// `act` done with the reports held (`changes`: they are written after, on a thread of their own).
fn with<R>(changes: bool, act: impl FnOnce(&mut Kept) -> R) -> R {
    with_changing(|kept| (act(kept), changes))
}

/// `act` done with the reports held, written after if it says they changed.
fn with_changing<R>(act: impl FnOnce(&mut Kept) -> (R, bool)) -> R {
    let mut held = KEPT.lock().unwrap();
    let kept = held.get_or_insert_with(|| load(&crate::settings::config_dir()));
    let (result, changed) = act(kept);
    if changed {
        save_later(kept.clone());
    }
    result
}

/// `kept` written on the thread that writes them, after those before (as the settings are: see `save_later` in lib.rs).
fn save_later(kept: Kept) {
    let mut unsaved = UNSAVED.lock().unwrap();
    let idle = unsaved.0.is_none() && !unsaved.1;
    unsaved.0 = Some(kept);
    if !idle {
        return;
    }
    std::thread::Builder::new()
        .name("glance: reports saved".into())
        .spawn(|| loop {
            let kept = {
                let mut unsaved = UNSAVED.lock().unwrap();
                match unsaved.0.take() {
                    Some(next) => {
                        unsaved.1 = true;
                        next
                    }
                    None => {
                        unsaved.1 = false;
                        return;
                    }
                }
            };
            if let Err(error) = save(&kept, &crate::settings::config_dir()) {
                crate::journal::note(format!("could not save the reports sent: {error}"));
            }
        })
        .expect("a thread started");
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_millis() as u64)
}

/// A new secret to send a report with: 32 bytes from the system's random number generator, in hex.
pub fn secret() -> Option<String> {
    use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
    let mut bytes = [0u8; 32];
    unsafe { BCryptGenRandom(None, &mut bytes, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }.ok().ok()?;
    Some(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// The report the site took as `id`, sent with `secret`, the user's `words` and their `pictures` (JPEGs), remembered
/// to be followed. On a thread of the caller's (see `keep`).
pub fn remember(id: &str, secret: String, words: &str, pictures: &[Vec<u8>]) {
    let words: String = words.chars().take(WORDS_MOST).collect();
    let report = Report { id: id.to_string(), secret, sent: now(), words, pictures: pictures.len(), thread: Vec::new(), heard: 0, read: 0 };
    with(true, |kept| kept.reports.push(report));
    keep(id, 0, pictures);
}

/// The name a picture sent with the report `id` is kept under: the `k`th of the report itself (`n` 0), or of the
/// user's addition numbered `n`. None for an id that is no name the site gives.
fn picture_name(id: &str, n: u64, k: usize) -> Option<String> {
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())).then(|| format!("{id}-{n}-{k}.jpg"))
}

/// Small copies of `pictures` (JPEGs) sent with the report `id` (`n`: 0 for the report itself, else the number of the
/// user's addition that carried them), kept to be shown in its thread. On a thread of the caller's, which joins the
/// process's apartment for the codecs it uses.
fn keep(id: &str, n: u64, pictures: &[Vec<u8>]) {
    if pictures.is_empty() {
        return;
    }
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
    }
    let own = crate::settings::config_dir();
    let folder = own.join(PICTURES);
    if let Err(error) = crate::elevation::ensure_folder(&own).and_then(|()| crate::elevation::ensure_folder(&folder)) {
        crate::journal::note(format!("could not keep the pictures of a report: {error}"));
        return;
    }
    for (k, picture) in pictures.iter().enumerate() {
        let (Some(name), Some(small)) = (picture_name(id, n, k), crate::ui::picture::shrunk(picture, PICTURE_SIDE)) else { continue };
        if let Err(error) = crate::elevation::write_in_place(&folder, &name, &small) {
            crate::journal::note(format!("could not keep a picture of a report: {error}"));
        }
    }
}

/// The copies kept of the `count` pictures sent with the report `id` (`n` as for `keep`): those that are there.
pub fn pictures(id: &str, n: u64, count: usize) -> Vec<Vec<u8>> {
    let folder = crate::settings::config_dir().join(PICTURES);
    (0..count).filter_map(|k| crate::elevation::read_bytes_in_place(&folder, &picture_name(id, n, k)?, PICTURE_MOST)).collect()
}

/// The copies kept of the pictures sent with `report`, gone with it.
fn forget_pictures(report: &Report) {
    let folder = crate::settings::config_dir().join(PICTURES);
    let carried = std::iter::once((0, report.pictures)).chain(report.thread.iter().filter(|message| message.kind == Kind::User).map(|message| (message.n, message.pictures)));
    for (n, count) in carried {
        for name in (0..count).filter_map(|k| picture_name(&report.id, n, k)) {
            crate::elevation::remove_in_place(&folder, &name);
        }
    }
}

/// Asks Glance's site what is new in the reports this PC sent, and tells the user once of each with news (on the
/// update check's thread, as it asks for the latest release).
pub fn ask() {
    let told = hear();
    if told.is_empty() {
        return;
    }
    let lang = crate::update::language();
    for (id, newest) in told {
        crate::journal::note(format!("reports: news of {id}"));
        let (title, text) = notice(lang, &id, &newest);
        crate::tray::notify_reports(&title, &text);
    }
}

/// Whether a report in `kept` is still talked over: neither fixed nor closed.
fn talking(kept: &Kept) -> bool {
    kept.reports.iter().any(|report| matches!(report.status(), Status::Open | Status::Answered))
}

/// Asks the site every `TALKING_EVERY` what is new in the reports this PC sent, while one is still talked over and
/// updates are checked for (asking is as the update check's, which asks once a day of them all: see `ask`). Its own
/// thread, till Glance ends.
pub fn follow() {
    loop {
        std::thread::sleep(TALKING_EVERY);
        let wanted = crate::app().settings.lock().unwrap().check_updates;
        if wanted && with(false, |kept| talking(kept)) {
            ask();
        }
    }
}

/// What is new in the reports followed, asked of the site and kept: each report with news, and what is newest in it.
/// Then, all of them heard, those quiet too long forgotten (a reply that came near their end is heard first: asked
/// of no more, it never would be).
fn hear() -> Vec<(String, Message)> {
    let (told, all_heard) = hear_of(|_| true);
    if all_heard {
        let now = now();
        let forgotten = with_changing(|kept| {
            let forgotten = forget_old(kept, now);
            let changed = !forgotten.is_empty();
            (forgotten, changed)
        });
        forgotten.iter().for_each(forget_pictures);
    }
    told
}

/// What is new in the reports followed that `of` takes, asked of the site and kept: each report with news, and what is
/// newest in it; and whether the site answered every asking.
fn hear_of(of: impl Fn(&Report) -> bool) -> (Vec<(String, Message)>, bool) {
    let askings = with(false, |kept| asking(kept, of));
    let mut told = Vec::new();
    let mut all_heard = true;
    for asked in askings {
        let Some(answer) = crate::update::post(&format!("{}/api/report/news", crate::update::own_site()), asked.as_bytes()) else {
            crate::journal::note("reports: no answer from the site");
            all_heard = false;
            continue;
        };
        told.extend(with_changing(|kept| {
            let told = heard(kept, &answer);
            let changed = !told.is_empty();
            (told, changed)
        }));
    }
    (told, all_heard)
}

/// Adds the user's `words` (and `diagnostics`, if they agreed to send them), with their `pictures`, to the report
/// `id`, in its thread on the site, and to the thread kept here. Whether it got there. On a thread of the caller's
/// (see `keep`).
pub fn add(id: &str, words: &str, diagnostics: Option<&str>, pictures: &[Vec<u8>]) -> bool {
    let Some(secret) = with(false, |kept| kept.reports.iter().find(|report| report.id == id).map(|report| report.secret.clone())) else { return false };
    let text = match diagnostics {
        Some(diagnostics) if words.is_empty() => diagnostics.to_string(),
        Some(diagnostics) => format!("The user says: {words}\n\n{diagnostics}"),
        None => words.to_string(),
    };
    let encoded: Vec<String> = pictures.iter().map(|picture| crate::diagnostics::base64(picture)).collect();
    let sent = serde_json::json!({ "id": id, "secret": secret, "version": env!("CARGO_PKG_VERSION"), "text": text, "pictures": encoded }).to_string();
    let Some(answer) = crate::update::post(&format!("{}/api/report/add", crate::update::own_site()), sent.as_bytes()) else { return false };
    let Ok(answer) = serde_json::from_slice::<serde_json::Value>(&answer) else { return false };
    let (Some(n), Some(ts)) = (answer["n"].as_u64(), answer["ts"].as_u64()) else { return false };
    let message = Message {
        n,
        ts,
        kind: Kind::User,
        text: words.chars().take(WORDS_MOST).collect(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        pictures: pictures.len(),
        diagnostics: diagnostics.is_some(),
        mine: false,
    };
    with(true, |kept| {
        if let Some(report) = kept.reports.iter_mut().find(|report| report.id == id) {
            report.thread.push(message);
            report.thread.sort_by_key(|message| message.n);
        }
    });
    keep(id, n, pictures);
    true
}

/// The report `id` closed by the user (`close`), or opened again, on the site and in the thread kept here: whether
/// that got there. One the site says stands so already (the owner set it so meanwhile, to be heard as the site is
/// next asked) is left as it is here. On a thread of the caller's.
pub fn turn(id: &str, close: bool) -> bool {
    let Some(secret) = with(false, |kept| kept.reports.iter().find(|report| report.id == id).map(|report| report.secret.clone())) else { return false };
    let status = if close { "closed" } else { "open" };
    let sent = serde_json::json!({ "id": id, "secret": secret, "status": status, "version": env!("CARGO_PKG_VERSION") }).to_string();
    let Some(answer) = crate::update::post(&format!("{}/api/report/status", crate::update::own_site()), sent.as_bytes()) else { return false };
    let Ok(answer) = serde_json::from_slice::<serde_json::Value>(&answer) else { return false };
    // (Standing so already, set so by the owner meanwhile: what was set heard at once, its thread shown as it is.)
    if answer["same"] == true {
        let _ = hear_of(|report| report.id == id);
        return true;
    }
    let (Some(n), Some(ts)) = (answer["n"].as_u64(), answer["ts"].as_u64()) else { return false };
    let kind = if close { Kind::Closed } else { Kind::Open };
    let message = Message { n, ts, kind, text: String::new(), version: String::new(), pictures: 0, diagnostics: false, mine: true };
    with(true, |kept| {
        if let Some(report) = kept.reports.iter_mut().find(|report| report.id == id) {
            report.thread.push(message);
            report.thread.sort_by_key(|message| message.n);
        }
    });
    true
}

/// The reports followed, the latest activity first.
pub fn list() -> Vec<Report> {
    let mut reports = with(false, |kept| kept.reports.clone());
    reports.sort_by_key(|report| std::cmp::Reverse(report.active()));
    reports
}

/// The report `id` as the user has now read it whole.
pub fn read(id: &str) {
    with(true, |kept| {
        if let Some(report) = kept.reports.iter_mut().find(|report| report.id == id) {
            report.read = report.thread.iter().map(|message| message.n).max().unwrap_or(0).max(report.read);
        }
    });
}

/// Reports as pictures of the settings need them, held and never written (their pictures, `pictures`, kept in the
/// test's own settings folder): one whose thread went on a while, with pictures, a reply, the user's addition and the
/// fix, its latest unread, then one fixed and read, and one waiting. Sent `now` (ms) and before.
#[cfg(all(test, feature = "studio"))]
pub fn pretend(now: u64, pictures: &[Vec<u8>]) {
    const HOUR: u64 = 3_600_000;
    let message = |n: u64, ts: u64, kind: Kind, text: &str, version: &str| Message { n, ts, kind, text: text.into(), version: version.into(), pictures: 0, diagnostics: false, mine: false };
    let report = |id: &str, sent: u64, words: &str, thread: Vec<Message>, read: u64| Report {
        id: id.into(),
        secret: String::new(),
        sent,
        words: words.into(),
        pictures: 0,
        heard: thread.iter().filter(|message| message.kind != Kind::User).map(|message| message.n).max().unwrap_or(0),
        thread,
        read,
    };
    let (first, added) = (&pictures[..pictures.len().min(2)], &pictures[pictures.len().min(2)..]);
    let addition = Message { n: 14, ts: now - 6 * HOUR, kind: Kind::User, text: "开启后仍然没有显示。附上新的截图与诊断信息。".into(), version: "0.4.1".into(), pictures: added.len(), diagnostics: true, mine: false };
    let mut followed = report(
        "KQRH9F",
        now - 30 * HOUR,
        "主板温度未显示\n华硕 B650 主板，其他软件可以读到",
        vec![
            message(11, now - 26 * HOUR, Kind::Reply, "已收到。请问开启“以管理员身份读取传感器”后是否仍然没有显示？", ""),
            message(12, now - 26 * HOUR, Kind::Answered, "", ""),
            addition,
            message(17, now - 2 * HOUR, Kind::Reply, "已找到原因：这块主板的温度需要从 Super I/O 芯片读取，下一版本将加入支持。", ""),
            message(18, now - HOUR, Kind::Fixed, "", "0.4.2"),
        ],
        14,
    );
    followed.pictures = first.len();
    // (The test's own settings folder, as Glance's is where it runs.)
    std::fs::create_dir_all(crate::settings::config_dir()).unwrap();
    keep("KQRH9F", 0, first);
    keep("KQRH9F", 14, added);
    let reports = vec![
        followed,
        report("7MPA2D", now - 300 * HOUR, "风扇转速与其他软件不一致", vec![message(3, now - 250 * HOUR, Kind::Reply, "已找到原因，下个版本修复。", ""), message(5, now - 100 * HOUR, Kind::Fixed, "", "0.4.0")], 5),
        report("ZX3W8N", now - 5 * HOUR, "", Vec::new(), 0),
    ];
    *KEPT.lock().unwrap() = Some(Kept { reports });
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86_400_000;

    fn report(id: &str, sent: u64) -> Report {
        Report { id: id.into(), secret: format!("{:0>64}", id.to_lowercase()), sent, words: "主板温度未显示\n第二行".into(), pictures: 0, thread: Vec::new(), heard: 0, read: 0 }
    }

    fn news(items: &[(&str, u64, &str, &str)]) -> Vec<u8> {
        let news: Vec<serde_json::Value> = items.iter().map(|(id, n, kind, text)| serde_json::json!({ "id": id, "n": n, "ts": 1000 + n, "kind": kind, "text": text, "version": if *kind == "fixed" { "9.0.0" } else { "" } })).collect();
        serde_json::json!({ "news": news }).to_string().into_bytes()
    }

    /// A report is talked over till it is fixed or closed (open again: talked over again), and with none of them so,
    /// the site is not asked each hour.
    #[test]
    fn asked_each_hour_while_a_report_is_talked_over() {
        let status = |kind: Kind, n: u64| Message { n, ts: 1000 + n, kind, text: String::new(), version: if kind == Kind::Fixed { "9.0.0".into() } else { String::new() }, pictures: 0, diagnostics: false, mine: false };
        let mut kept = Kept { reports: vec![report("KQRH9F", 10 * DAY), report("ABCDEF", 20 * DAY)] };
        assert!(talking(&kept));
        kept.reports[0].thread.push(status(Kind::Fixed, 1));
        kept.reports[1].thread.extend([status(Kind::Reply, 1), status(Kind::Answered, 2)]);
        assert!(talking(&kept));
        kept.reports[1].thread.push(status(Kind::Closed, 3));
        assert!(!talking(&kept));
        kept.reports[0].thread.push(status(Kind::Open, 2));
        assert!(talking(&kept));
        assert!(!talking(&Kept::default()));
    }

    /// A change of status the user made sets where the report stands as the owner's do, but is nothing to read.
    #[test]
    fn the_user_s_own_status_is_not_news() {
        let mut followed = report("KQRH9F", 10 * DAY);
        let fixed = Message { n: 3, ts: 3, kind: Kind::Fixed, text: String::new(), version: "0.4.2".into(), pictures: 0, diagnostics: false, mine: false };
        followed.thread.push(fixed);
        followed.read = 3;
        assert!(!followed.closable());
        followed.thread.push(Message { n: 4, ts: 4, kind: Kind::Open, version: String::new(), mine: true, ..followed.thread[0].clone() });
        assert_eq!(followed.status(), Status::Open);
        assert!(followed.closable());
        assert!(!followed.unread());
        followed.thread.push(Message { n: 5, ts: 5, kind: Kind::Closed, mine: true, ..followed.thread[1].clone() });
        assert_eq!(followed.status(), Status::Closed);
        assert!(!followed.closable());
        assert!(!followed.unread());
    }

    #[test]
    fn secrets_are_long_random_and_never_the_same() {
        let (a, b) = (secret().unwrap(), secret().unwrap());
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(a, b);
    }

    /// What is asked carries each report's id, its secret and how far it was heard, and nothing else: no version, no
    /// time, no words, nothing of the PC.
    #[test]
    fn the_asking_names_nothing_but_the_reports_and_their_proof() {
        let mut kept = Kept { reports: vec![report("KQRH9F", 10 * DAY), report("ABCDEF", 20 * DAY)] };
        kept.reports[1].heard = 7;
        let asked: serde_json::Value = serde_json::from_str(&asking(&kept, |_| true)[0]).unwrap();
        let fields: Vec<&String> = asked.as_object().unwrap().keys().collect();
        assert_eq!(fields, ["reports"]);
        let reports = asked["reports"].as_array().unwrap();
        // (The latest first.)
        assert_eq!(reports[0], serde_json::json!({ "id": "ABCDEF", "secret": kept.reports[1].secret, "after": 7 }));
        assert_eq!(reports[1], serde_json::json!({ "id": "KQRH9F", "secret": kept.reports[0].secret, "after": 0 }));
        for one in reports {
            let mut keys: Vec<&String> = one.as_object().unwrap().keys().collect();
            keys.sort();
            assert_eq!(keys, ["after", "id", "secret"]);
        }
        let text = asking(&kept, |_| true).concat();
        for absent in [env!("CARGO_PKG_VERSION"), "主板", "sent", "words"] {
            assert!(!text.contains(absent), "{absent} in {text}");
        }
        // None kept: nothing asked at all.
        assert!(asking(&Kept::default(), |_| true).is_empty());
        // No more in one asking than the site answers, and all of them asked of: the latest first.
        let many = Kept { reports: (0..30).map(|at| report(&format!("R{at:05}"), at * DAY)).collect() };
        let askings: Vec<serde_json::Value> = asking(&many, |_| true).iter().map(|text| serde_json::from_str(text).unwrap()).collect();
        let sizes: Vec<usize> = askings.iter().map(|asked| asked["reports"].as_array().unwrap().len()).collect();
        assert_eq!(sizes, [ASKED_MOST, 30 - ASKED_MOST]);
        assert_eq!((askings[0]["reports"][0]["id"].as_str(), askings[1]["reports"][9]["id"].as_str()), (Some("R00029"), Some("R00000")));
        // One alone, where one is asked of.
        let one: serde_json::Value = serde_json::from_str(&asking(&many, |report| report.id == "R00003")[0]).unwrap();
        assert_eq!(one["reports"].as_array().unwrap().len(), 1);
    }

    /// A report is followed for 90 days after anything last happened in it, then forgotten.
    #[test]
    fn reports_quiet_for_90_days_are_forgotten() {
        let now = 200 * DAY;
        let mut active = report("ACTIVE", 50 * DAY);
        active.thread.push(Message { n: 1, ts: 150 * DAY, kind: Kind::Reply, text: "x".into(), version: String::new(), pictures: 0, diagnostics: false, mine: false });
        let mut kept = Kept { reports: vec![report("RECENT", 120 * DAY), report("STALEX", 109 * DAY), active, report("EDGEXX", 110 * DAY)] };
        forget_old(&mut kept, now);
        let ids: Vec<&str> = kept.reports.iter().map(|report| report.id.as_str()).collect();
        assert_eq!(ids, ["RECENT", "ACTIVE", "EDGEXX"]);
    }

    /// News is heard once: the same answer again (or a number already in the thread) tells nothing more, and what
    /// is asked next goes past it.
    #[test]
    fn each_piece_of_news_is_told_once() {
        let mut kept = Kept { reports: vec![report("KQRH9F", 0), report("ABCDEF", 0)] };
        let answer = news(&[("KQRH9F", 3, "reply", "已收到，正在排查"), ("KQRH9F", 4, "answered", ""), ("ABCDEF", 5, "fixed", ""), ("NOTOUR", 6, "reply", "x"), ("ABCDEF", 7, "someday", "")]);
        let told = heard(&mut kept, &answer);
        assert_eq!(told.len(), 2);
        // A reply is told over the status set with it.
        assert_eq!((told[0].0.as_str(), told[0].1.kind), ("KQRH9F", Kind::Reply));
        assert_eq!((told[1].0.as_str(), told[1].1.kind), ("ABCDEF", Kind::Fixed));
        assert!(heard(&mut kept, &answer).is_empty());
        assert_eq!(kept.reports[0].thread.len(), 2);
        let asked: serde_json::Value = serde_json::from_str(&asking(&kept, |_| true)[0]).unwrap();
        assert!(asked["reports"].as_array().unwrap().iter().any(|one| one["id"] == "KQRH9F" && one["after"] == 4));
        // Unread till read, and the status as last set.
        assert!(kept.reports.iter().all(Report::unread));
        assert_eq!(kept.reports[1].status(), Status::Fixed("9.0.0".into()));
        assert_eq!(kept.reports[0].status(), Status::Answered);
        // The next reply is told.
        let told = heard(&mut kept, &news(&[("KQRH9F", 9, "reply", "请更新后再试")]));
        assert_eq!(told.len(), 1);
        assert_eq!(told[0].1.text, "请更新后再试");
    }

    /// The user's own addition, numbered past a reply not yet heard, does not keep that reply from being heard.
    #[test]
    fn an_addition_does_not_hide_a_reply_not_yet_heard() {
        let mut kept = Kept { reports: vec![report("KQRH9F", 0)] };
        kept.reports[0].thread.push(Message { n: 8, ts: 5, kind: Kind::User, text: "补充".into(), version: String::new(), pictures: 1, diagnostics: false, mine: false });
        let told = heard(&mut kept, &news(&[("KQRH9F", 6, "reply", "先前的回复")]));
        assert_eq!(told.len(), 1);
        let ns: Vec<u64> = kept.reports[0].thread.iter().map(|message| message.n).collect();
        assert_eq!(ns, [6, 8]);
        // The user's own messages are never unread.
        kept.reports[0].read = 6;
        assert!(!kept.reports[0].unread());
    }

    #[test]
    fn a_fix_is_told_as_this_version_has_it_or_not() {
        let fixed = |version: &str| Message { n: 1, ts: 1, kind: Kind::Fixed, text: String::new(), version: version.into(), pictures: 0, diagnostics: false, mine: false };
        let (title, words) = notice(Lang::Zh, "KQRH9F", &fixed("999.0"));
        assert_eq!(title, "你的反馈 KQRH9F 已在 999.0 修复");
        assert_eq!(words, "更新到 999.0 即可获得此修复。");
        let (_, words) = notice(Lang::En, "KQRH9F", &fixed("0.0.1"));
        assert_eq!(words, "This version includes the fix.");
        let reply = Message { kind: Kind::Reply, text: "\n  请问是哪块主板？\n更多".into(), ..fixed("") };
        assert_eq!(notice(Lang::Zh, "KQRH9F", &reply), ("你的反馈 KQRH9F 有新回复".to_string(), "请问是哪块主板？".to_string()));
    }

    /// Over a copy of Glance's site run on this PC (`GLANCE_SITE`, a debug build's): a report is sent and followed.
    /// Its id is written to the file `REPORT_ID_FILE` names, for whoever answers as the site's owner: a reply is
    /// heard, and told once, an addition with pictures and diagnostics goes into the same thread, and then the fix.
    #[test]
    #[ignore = "needs a copy of Glance's site run on this PC (GLANCE_SITE) and its owner to answer"]
    fn follows_a_report_on_a_site_run_here() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let waited = |what: &str| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(180);
            loop {
                let told = hear();
                if !told.is_empty() {
                    println!("told {what}: {told:?}");
                    return told;
                }
                assert!(std::time::Instant::now() < until, "no {what} heard");
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        };
        let id = crate::diagnostics::send("Glance (a test of following a report)", "风扇转速不显示\n其余正常", &[]).expect("the report sent");
        std::fs::write(std::env::var("REPORT_ID_FILE").unwrap(), &id).unwrap();
        let told = waited("a reply");
        assert_eq!(told.len(), 1);
        assert_eq!((told[0].0.as_str(), told[0].1.kind), (id.as_str(), Kind::Reply));
        assert!(hear().is_empty(), "told twice");
        let picture = std::mem::take(&mut crate::ui::picture::from_file(concat!(env!("CARGO_MANIFEST_DIR"), "/icons/128x128@2x.png"), (84, 52)).expect("a picture").jpeg);
        assert!(add(&id, "更新后仍然存在", Some("Diagnostics, again"), &[picture]));
        println!("ADDED");
        let told = waited("the fix");
        assert_eq!(told[0].1.kind, Kind::Fixed);
        let report = list().into_iter().find(|report| report.id == id).unwrap();
        assert_eq!(report.status(), Status::Fixed(told[0].1.version.clone()));
        let kinds: Vec<Kind> = report.thread.iter().map(|message| message.kind).collect();
        assert_eq!(kinds, [Kind::Reply, Kind::Answered, Kind::User, Kind::Fixed]);
        assert!(report.unread());
        read(&id);
        assert!(!list().into_iter().find(|report| report.id == id).unwrap().unread());
        // Fixed, the user opens it again, then closes it: theirs, never news, nothing to read.
        assert!(turn(&id, false));
        assert_eq!(list().into_iter().find(|report| report.id == id).unwrap().status(), Status::Open);
        assert!(turn(&id, true));
        let report = list().into_iter().find(|report| report.id == id).unwrap();
        assert_eq!(report.status(), Status::Closed);
        assert!(!report.closable());
        let mine: Vec<(Kind, bool)> = report.thread.iter().map(|message| (message.kind, message.mine)).collect();
        assert_eq!(mine[4..], [(Kind::Open, true), (Kind::Closed, true)]);
        assert!(!report.unread());
        assert!(hear().is_empty());
        // (Closed already: closing it again changes nothing.)
        assert!(turn(&id, true));
        assert_eq!(list().into_iter().find(|report| report.id == id).unwrap().thread.len(), 6);
    }

    /// The pictures sent are kept small, under the report and the message that carried them, read again as they
    /// were kept, and gone with the report.
    #[test]
    fn pictures_sent_are_kept_small_and_go_with_their_report() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        // (The test's own settings folder, as Glance's is where it runs.)
        std::fs::create_dir_all(crate::settings::config_dir()).unwrap();
        let picture = std::mem::take(&mut crate::ui::picture::from_file(concat!(env!("CARGO_MANIFEST_DIR"), "/icons/icon.png"), (8, 8)).expect("a picture").jpeg);
        keep("PICTRS", 0, &[picture.clone(), picture.clone()]);
        keep("PICTRS", 9, std::slice::from_ref(&picture));
        let kept = pictures("PICTRS", 0, 2);
        assert_eq!(kept.len(), 2);
        let again = crate::ui::picture::from_jpeg(&kept[0], (8, 8)).expect("read again");
        assert!(again.size.0.max(again.size.1) <= PICTURE_SIDE);
        assert_eq!(pictures("PICTRS", 9, 1).len(), 1);
        // (Asked for more than there are: those there are.)
        assert_eq!(pictures("PICTRS", 9, 3).len(), 1);
        // No file is named after what is no id.
        assert_eq!(picture_name("../x", 0, 0), None);
        let mut report = report("PICTRS", 0);
        report.pictures = 2;
        report.thread.push(Message { n: 9, ts: 1, kind: Kind::User, text: String::new(), version: String::new(), pictures: 1, diagnostics: false, mine: false });
        forget_pictures(&report);
        assert!(pictures("PICTRS", 0, 2).is_empty());
        assert!(pictures("PICTRS", 9, 1).is_empty());
    }

    /// Written and read again as they were, in a folder of the test's own.
    #[test]
    fn reports_are_kept_as_they_were() {
        let dir = std::env::temp_dir().join(format!("glance-reports-test-{}", std::process::id()));
        let mut kept = Kept { reports: vec![report("KQRH9F", 3)] };
        heard(&mut kept, &news(&[("KQRH9F", 2, "reply", "好的")]));
        save(&kept, &dir).unwrap();
        assert_eq!(load(&dir), kept);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(load(&dir), Kept::default());
    }
}
