//! Reminders of to-dos: each to-do not done that has a day is told of once, by a notification of Windows' (see
//! `toast`), while Glance runs. One with a time of day is told of so many minutes before it (`Settings::todo_lead`),
//! one of the whole day at a time of its day (`Settings::todo_day_at`, 09:00 till the user chooses another).
//!
//! What is told of is kept by the to-do as written (its words and its day and time: see `key`), so that a to-do whose
//! time is changed is told of again at its new time, and one ticked has its notification taken back. A notification
//! may tick its to-do, or tell of it again later (in ten minutes, an hour, or the next day); a press on it opens the
//! workbench where the to-dos are.
//!
//! Glance may not have been watching as a to-do came due: the PC asleep, or Glance not running. What came due
//! meanwhile is told of as Glance watches again, that of today alone (not what came due days before), several together
//! in one notification. A to-do whose time had gone by already as it was written, Glance watching, is not told of as
//! missed: where its time is still to come, it is told of then (its reminder's time gone, its own not).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::todo::{Day, Due, Item};

/// A moment as this PC's clock has it: seconds from 1970-01-01 00:00 in its own time (its time zone's, whatever it
/// is): to-dos are written in the PC's own time, and so are compared with it.
pub type Local = i64;

const DAY: Local = 86_400;

/// How late a reminder may be told of and still be told as it comes (s): beyond that, it was missed.
const LATE: Local = 120;

/// When to-dos are told of, as the settings have it: how many minutes before its time one with a time of day, and
/// when on its day one of the whole day.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rules {
    pub lead: u32,
    pub day_at: (u32, u32),
}

impl Rules {
    /// When `due` is told of.
    pub fn told_at(self, due: Due) -> Local {
        match due.time {
            Some(time) => at(due.day, time) - Local::from(self.lead) * 60,
            None => at(due.day, self.day_at),
        }
    }
}

/// The moment `time` (hour, minute) of `day` begins.
pub fn at(day: Day, (hour, minute): (u32, u32)) -> Local {
    day.number() * DAY + Local::from(hour) * 3600 + Local::from(minute) * 60
}

/// The day of the moment `now`.
pub fn day_of(now: Local) -> Day {
    Day::from_number(now.div_euclid(DAY))
}

/// When a to-do is due: the beginning of its day, where it is of the whole day.
fn due_at(due: Due) -> Local {
    at(due.day, due.time.unwrap_or((0, 0)))
}

/// A to-do as it is told of, and what is kept of it: its words and when it is due, as the note writes them. None,
/// where it has no day.
pub fn key(item: &Item) -> Option<String> {
    item.due.map(|due| format!("{} @{}", item.text, due.written()))
}

/// What is kept between one look at the to-dos and the next, and from one run of Glance to the next: the to-dos told
/// of (by `key`), those to be told of again later and when, when Glance last looked, and those the notification of
/// several missed together tells of.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Kept {
    pub told: BTreeSet<String>,
    pub later: BTreeMap<String, Local>,
    pub last: Option<Local>,
    pub missed: Vec<String>,
}

/// A to-do to be told of: by its `key`, as it is now.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Tell {
    pub key: String,
    pub item: Item,
}

/// A notification to be shown: of one to-do, or of several whose reminders were missed together.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Notice {
    One(Tell),
    Missed(Vec<Tell>),
}

/// What looking at the to-dos at `now` found: the notifications to show; the keys of those told of before that are
/// told of no more (done, taken out, or changed), whose notifications are taken back; when to look next for something
/// due (none: nothing is); and, where some of those the notification of several missed together tells of are told of
/// no more, those of them left (none left: it is taken back).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Looked {
    pub notices: Vec<Notice>,
    pub gone: Vec<String>,
    pub next: Option<Local>,
    pub missed_left: Option<Vec<Tell>>,
}

/// The to-dos `items` looked at `now`, as `rules` tell them, `kept` as told: each not done with a day told of once, as
/// it comes due (see the module's words); those to be told of again later, as that comes. What is kept is changed as
/// they are told of. None, where the to-dos are not to be read now (their note not there, or its folder not to be
/// reached): nothing told of, and nothing of what is kept forgotten, those to be told of again later told of once
/// they are read again, as they come then.
pub fn look(items: Option<&[Item]>, kept: &mut Kept, now: Local, rules: Rules) -> Looked {
    let Some(items) = items else {
        kept.last = Some(now);
        return Looked::default();
    };
    let today = now.div_euclid(DAY);
    // (Never looked before: what came due today, from its beginning.)
    let last = kept.last.unwrap_or(today * DAY - 1);
    let mut looked = Looked::default();
    let mut on_time = Vec::new();
    let mut missed = Vec::new();
    let mut live = BTreeMap::new();
    let sooner = |next: &mut Option<Local>, at: Local| *next = Some(next.map_or(at, |next: Local| next.min(at)));
    for item in items.iter().filter(|item| !item.done) {
        let (Some(key), Some(due)) = (key(item), item.due) else { continue };
        if live.insert(key.clone(), item.clone()).is_some() {
            continue;
        }
        let tell = Tell { key: key.clone(), item: item.clone() };
        if let Some(&again) = kept.later.get(&key) {
            if again <= now {
                kept.later.remove(&key);
                kept.told.insert(key);
                on_time.push(tell);
            } else {
                sooner(&mut looked.next, again);
            }
            continue;
        }
        if kept.told.contains(&key) {
            continue;
        }
        let reminder = rules.told_at(due);
        // Its reminder's time, where it came since Glance last looked; else its own, where that is still to come then.
        let when = if reminder > last {
            Some(reminder)
        } else if due.time.is_some() && due_at(due) > last {
            Some(due_at(due))
        } else {
            None
        };
        match when {
            // Both gone by while Glance looked (written after its time, or its reminder's time moved back past now):
            // not told of, nor kept as told, so that one moved on again past now is told of then.
            None => {}
            Some(when) if when > now => sooner(&mut looked.next, when),
            Some(when) => {
                kept.told.insert(key);
                if now - when <= LATE {
                    on_time.push(tell);
                } else if when.div_euclid(DAY) == today || due.day.number() == today {
                    missed.push(tell);
                }
            }
        }
    }
    looked.gone = kept.told.iter().chain(kept.later.keys()).filter(|key| !live.contains_key(*key)).cloned().collect::<BTreeSet<_>>().into_iter().collect();
    kept.told.retain(|key| live.contains_key(key));
    kept.later.retain(|key, _| live.contains_key(key));
    kept.last = Some(now);
    // The notification of several missed together, some of them told of no more: of those left.
    if kept.missed.iter().any(|key| !live.contains_key(key)) {
        let left: Vec<Tell> = kept.missed.iter().filter_map(|key| Some(Tell { key: key.clone(), item: live.get(key)?.clone() })).collect();
        kept.missed = left.iter().map(|tell| tell.key.clone()).collect();
        looked.missed_left = Some(left);
    }
    looked.notices = on_time.into_iter().map(Notice::One).collect();
    match missed.len() {
        0 => {}
        1 => looked.notices.push(Notice::One(missed.remove(0))),
        _ => {
            // (In its place: as Windows shows one notification by its tag.)
            kept.missed = missed.iter().map(|tell| tell.key.clone()).collect();
            looked.missed_left = None;
            looked.notices.push(Notice::Missed(missed));
        }
    }
    looked
}

/// When a to-do is told of again, asked from its notification.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Later {
    Minutes(u32),
    /// The next day: at this time, or (one of the whole day) when its reminder is told.
    Tomorrow,
}

impl Later {
    /// As a notification's choice names it (see `toast`).
    pub fn named(name: &str) -> Option<Later> {
        match name {
            "tomorrow" => Some(Later::Tomorrow),
            minutes => minutes.parse().ok().filter(|minutes| (1..=24 * 60).contains(minutes)).map(Later::Minutes),
        }
    }
}

/// The to-do `key` (of `item`) told of again `later`, from `now`.
pub fn tell_later(kept: &mut Kept, key: &str, item: &Item, later: Later, now: Local, rules: Rules) {
    let again = match (later, item.due) {
        (Later::Minutes(minutes), _) => now + Local::from(minutes) * 60,
        (Later::Tomorrow, Some(Due { time: None, .. })) => at(day_of(now).plus(1), rules.day_at),
        (Later::Tomorrow, _) => now + DAY,
    };
    kept.later.insert(key.to_string(), again);
}

/// What a notification of `notice` says at `now`: its title, and the line under it.
pub fn says(notice: &Notice, now: Local, zh: bool) -> (String, String) {
    let today = day_of(now);
    let pick = |zh_said: String, en: String| if zh { zh_said } else { en };
    match notice {
        Notice::One(tell) => {
            let Some(due) = tell.item.due else { return (tell.item.text.clone(), String::new()) };
            let label = due.label(today, zh);
            let when = match due.time {
                None => pick("全天".into(), "All day".into()),
                Some(_) => {
                    let left = due_at(due) - now;
                    if left >= 60 {
                        let minutes = (left + 59) / 60;
                        let (hours, minutes) = (minutes / 60, minutes % 60);
                        match (hours, minutes) {
                            (0, minutes) => pick(format!("还有 {minutes} 分钟"), format!("In {minutes} min")),
                            (hours, 0) => pick(format!("还有 {hours} 小时"), format!("In {hours} h")),
                            (hours, minutes) => pick(format!("还有 {hours} 小时 {minutes} 分钟"), format!("In {hours} h {minutes} min")),
                        }
                    } else if left > -60 {
                        pick("现在".into(), "Now".into())
                    } else {
                        pick("已过时间".into(), "Overdue".into())
                    }
                }
            };
            (tell.item.text.clone(), format!("{label} · {when}"))
        }
        Notice::Missed(tells) => {
            let count = tells.len();
            let all_past = tells.iter().all(|tell| tell.item.due.is_some_and(|due| due.time.is_none() || due_at(due) <= now));
            let title = match all_past {
                true => pick(format!("今天有 {count} 项待办已过时间"), format!("{count} to-dos today are past their time")),
                false => pick(format!("错过了 {count} 项待办的提醒"), format!("{count} to-do reminders were missed")),
            };
            let names: Vec<&str> = tells.iter().map(|tell| tell.item.text.as_str()).collect();
            (title, names.join(if zh { "、" } else { ", " }))
        }
    }
}

/// Now, as this PC's clock has it (see `Local`).
#[cfg(windows)]
pub fn now() -> Local {
    let now = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    let Some(day) = Day::of(now.wYear as i32, now.wMonth as u32, now.wDay as u32) else { return 0 };
    at(day, (now.wHour as u32, now.wMinute as u32)) + Local::from(now.wSecond)
}

/// The group Glance's notifications of to-dos are in, and the tag of the one of those whose reminders were missed.
#[cfg(windows)]
const GROUP: &str = "todo";
#[cfg(windows)]
const MISSED: &str = "missed";

/// The tag of the notification of the to-do `key`: no longer than Windows takes.
#[cfg(windows)]
fn tag(key: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// The notification of `notice` at `now`, as XML: of one to-do, a choice of when to tell of it again, a button that
/// does, one that ticks it (the notification itself opening the to-dos), staying till it is answered; of several
/// missed, a button that opens the to-dos.
#[cfg(windows)]
fn xml(notice: &Notice, now: Local, rules: Rules, zh: bool) -> String {
    use crate::toast::escaped;
    let (title, line) = says(notice, now, zh);
    let pick = |zh_said: &str, en: &str| escaped(if zh { zh_said } else { en });
    let text = format!(r#"<visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual>"#, escaped(&title), escaped(&line));
    match notice {
        Notice::One(tell) => {
            let key = escaped(&tell.key);
            let tomorrow = match tell.item.due.and_then(|due| due.time) {
                Some(_) => pick("明天这个时间", "This time tomorrow"),
                None => {
                    let (hour, minute) = rules.day_at;
                    escaped(&format!("{} {hour:02}:{minute:02}", if zh { "明天" } else { "Tomorrow" }))
                }
            };
            format!(
                r#"<toast launch="open:{key}" activationType="foreground" scenario="reminder">{text}<actions><input id="later" type="selection" defaultInput="10"><selection id="10" content="{}"/><selection id="60" content="{}"/><selection id="tomorrow" content="{tomorrow}"/></input><action content="{}" arguments="later:{key}" activationType="background"/><action content="{}" arguments="done:{key}" activationType="background"/></actions></toast>"#,
                pick("10 分钟后", "In 10 minutes"),
                pick("1 小时后", "In 1 hour"),
                pick("稍后提醒", "Remind me later"),
                pick("完成", "Done"),
            )
        }
        Notice::Missed(_) => format!(r#"<toast launch="open:" activationType="foreground">{text}<actions><action content="{}" arguments="open:" activationType="foreground"/></actions></toast>"#, pick("打开待办", "Open to-dos")),
    }
}

/// What was pressed on Glance's notifications, waiting to be done, and woken for as it is.
#[cfg(windows)]
static ASKED: std::sync::Mutex<Vec<crate::toast::Asked>> = std::sync::Mutex::new(Vec::new());
#[cfg(windows)]
static WAKE: std::sync::Condvar = std::sync::Condvar::new();

/// What is kept (see `Kept`) is in this file, where Glance keeps what it can make again.
#[cfg(windows)]
pub const KEPT: &str = "reminders.json";

/// The to-dos looked at as Glance runs, every ten seconds, and told of as they come due (see `look`); what is pressed on
/// their notifications done. On a thread of its own, which answers Windows for the notifications (see `toast`).
#[cfg(windows)]
pub fn start() {
    let _ = std::thread::Builder::new().name("glance: reminders".into()).spawn(|| {
        let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED) };
        if let Err(error) = crate::toast::register(|asked| {
            ASKED.lock().unwrap().push(asked);
            WAKE.notify_one();
        }) {
            crate::journal::note(format!("reminders: not named to Windows for notifications ({error})"));
        }
        let folder = crate::settings::local_dir();
        let mut kept: Kept = std::fs::read(folder.join(KEPT)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
        let mut saved = (kept.clone(), std::time::Instant::now());
        // (Not written: told in the journal once, till it is written again.)
        let mut unwritten = false;
        loop {
            // (What was pressed is done once the to-dos are read: pressed as Windows started Glance for it, it came first.)
            let asked = {
                let waiting = ASKED.lock().unwrap();
                let (mut waiting, _) = WAKE.wait_timeout_while(waiting, std::time::Duration::from_secs(10), |asked| asked.is_empty() || !crate::notes::read_yet()).unwrap();
                if crate::notes::read_yet() { std::mem::take(&mut *waiting) } else { Vec::new() }
            };
            let (on, rules, zh) = {
                let settings = crate::app().settings.lock().unwrap();
                let day_at = crate::todo::clock(&settings.todo_day_at).unwrap_or((9, 0));
                (settings.todo_remind, Rules { lead: settings.todo_lead, day_at }, crate::update::language_in(&settings.view) == crate::ui::text::Lang::Zh)
            };
            let now = now();
            for asked in asked {
                act(&asked, &mut kept, now, rules);
            }
            // (Not before the notes are read: the to-dos would be none, and what came due meanwhile taken as written after.)
            if crate::notes::read_yet() {
                if on {
                    let items: Option<Vec<Item>> = crate::todo::read().map(|list| list.items().into_iter().cloned().collect());
                    let looked = look(items.as_deref(), &mut kept, now, rules);
                    for key in &looked.gone {
                        let _ = crate::toast::take_back(&tag(key), GROUP);
                    }
                    // Several missed told of together, some of them told of no more: told of those left, in its
                    // place and without coming up again (where it is still among the notifications), else taken back.
                    match &looked.missed_left {
                        Some(left) if left.is_empty() => {
                            let _ = crate::toast::take_back(MISSED, GROUP);
                        }
                        Some(left) if crate::toast::shown(MISSED, GROUP) => {
                            if let Err(error) = crate::toast::show(&xml(&Notice::Missed(left.clone()), now, rules, zh), MISSED, GROUP, true) {
                                crate::journal::note(format!("reminders: a notification not shown ({error})"));
                            }
                        }
                        _ => {}
                    }
                    for notice in &looked.notices {
                        let tagged = match notice {
                            Notice::One(tell) => tag(&tell.key),
                            Notice::Missed(_) => MISSED.into(),
                        };
                        if let Err(error) = crate::toast::show(&xml(notice, now, rules, zh), &tagged, GROUP, false) {
                            crate::journal::note(format!("reminders: a notification not shown ({error})"));
                        }
                    }
                } else {
                    // (Off: what comes due meanwhile is not told of as missed once it is on again.)
                    kept.last = Some(now);
                }
            }
            // Written as what is told changes, and now and then for when Glance last looked; not written, tried again at
            // the next look.
            let (was, at) = &saved;
            if kept != *was && (kept.told != was.told || kept.later != was.later || kept.missed != was.missed || at.elapsed() >= std::time::Duration::from_secs(600)) {
                let written = serde_json::to_vec(&kept).map_err(std::io::Error::other).and_then(|bytes| {
                    std::fs::create_dir_all(&folder)?;
                    std::fs::write(folder.join(KEPT), bytes)
                });
                match written {
                    Ok(()) => (saved, unwritten) = ((kept.clone(), std::time::Instant::now()), false),
                    Err(error) if !unwritten => {
                        crate::journal::note(format!("reminders: not kept ({error})"));
                        unwritten = true;
                    }
                    Err(_) => {}
                }
            }
        }
    });
}

/// What was pressed on a notification done: its to-do ticked, or told of again later; the to-dos opened.
#[cfg(windows)]
fn act(asked: &crate::toast::Asked, kept: &mut Kept, now: Local, rules: Rules) {
    let (what, key) = asked.args.split_once(':').unwrap_or((asked.args.as_str(), ""));
    let list = crate::todo::list();
    let found = list.items().into_iter().enumerate().find(|(_, item)| !item.done && self::key(item).as_deref() == Some(key)).map(|(n, item)| (n, item.clone()));
    match what {
        "done" => {
            if let Some((n, item)) = found {
                crate::todo::tick(n, &item, true);
            }
        }
        "later" => {
            if let (Some((_, item)), Some(later)) = (found, asked.input("later").and_then(Later::named)) {
                tell_later(kept, key, &item, later, now, rules);
            }
        }
        "open" => crate::app().controller.open_todo((!key.is_empty()).then(|| key.to_string())),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULES: Rules = Rules { lead: 10, day_at: (9, 0) };

    fn day() -> Day {
        Day::of(2026, 11, 21).unwrap()
    }

    fn time(hour: u32, minute: u32) -> Local {
        at(day(), (hour, minute))
    }

    fn item(text: &str, due: Option<(i64, Option<(u32, u32)>)>) -> Item {
        Item { text: text.into(), done: false, due: due.map(|(days, time)| Due { day: day().plus(days), time }) }
    }

    fn told(notices: &[Notice]) -> Vec<Vec<&str>> {
        notices
            .iter()
            .map(|notice| match notice {
                Notice::One(tell) => vec![tell.item.text.as_str()],
                Notice::Missed(tells) => tells.iter().map(|tell| tell.item.text.as_str()).collect(),
            })
            .collect()
    }

    /// Looked at every ten seconds from `from` to `to`, as Glance looks: what was told, and when.
    fn watched(items: &[Item], kept: &mut Kept, from: Local, to: Local) -> Vec<(Local, Vec<Vec<String>>)> {
        let mut shown = Vec::new();
        let mut now = from;
        while now <= to {
            let looked = look(Some(items), kept, now, RULES);
            if !looked.notices.is_empty() {
                shown.push((now, told(&looked.notices).into_iter().map(|names| names.into_iter().map(String::from).collect()).collect()));
            }
            now += 10;
        }
        shown
    }

    #[test]
    fn a_to_do_is_told_of_once_before_its_time_and_one_of_the_whole_day_in_the_morning() {
        let items = [item("回复邮件", Some((0, Some((15, 0))))), item("交水电费", Some((0, None))), item("没有日期", None), Item { done: true, ..item("已完成", Some((0, Some((16, 0))))) }];
        let mut kept = Kept { last: Some(time(8, 0)), ..Kept::default() };
        let shown = watched(&items, &mut kept, time(8, 0), time(17, 0));
        assert_eq!(shown, vec![(time(9, 0), vec![vec!["交水电费".to_string()]]), (time(14, 50), vec![vec!["回复邮件".to_string()]])]);
        // As the settings have it: 30 minutes before, and at 07:00.
        let mut kept = Kept { last: Some(time(6, 0)), ..Kept::default() };
        let rules = Rules { lead: 30, day_at: (7, 0) };
        assert_eq!(look(Some(&items), &mut kept, time(7, 0), rules).notices.len(), 1);
        assert_eq!(look(Some(&items), &mut kept, time(14, 30), rules).notices.len(), 1);
        assert_eq!(look(Some(&items), &mut kept, time(14, 31), rules).notices.len(), 0);
    }

    #[test]
    fn the_next_look_is_when_the_soonest_comes_due() {
        let items = [item("a", Some((0, Some((15, 0))))), item("b", Some((1, None)))];
        let mut kept = Kept { last: Some(time(8, 0)), ..Kept::default() };
        assert_eq!(look(Some(&items), &mut kept, time(8, 0), RULES).next, Some(time(14, 50)));
        assert_eq!(look(Some(&items), &mut kept, time(14, 50), RULES).next, Some(at(day().plus(1), (9, 0))));
    }

    #[test]
    fn what_came_due_while_glance_was_not_watching_is_told_of_today_together() {
        let items = [
            item("整理周报", Some((0, Some((14, 0))))),
            item("预约体检", Some((0, Some((16, 30))))),
            item("交水电费", Some((0, None))),
            item("昨天的", Some((-1, Some((20, 0))))),
            item("明天的", Some((1, Some((9, 0))))),
        ];
        // Asleep from 13:00 to 16:42.
        let mut kept = Kept { last: Some(time(13, 0)), told: ["交水电费 @2026-11-21".to_string()].into(), ..Kept::default() };
        let looked = look(Some(&items), &mut kept, time(16, 42), RULES);
        assert_eq!(told(&looked.notices), vec![vec!["整理周报", "预约体检"]]);
        assert_eq!(says(&looked.notices[0], time(16, 42), true), ("今天有 2 项待办已过时间".into(), "整理周报、预约体检".into()));
        // Not again.
        assert!(look(Some(&items), &mut kept, time(16, 43), RULES).notices.is_empty());
        // Glance first run this morning: what came due today, and not what came due yesterday.
        let mut first = Kept::default();
        assert_eq!(told(&look(Some(&items), &mut first, time(10, 0), RULES).notices), vec![vec!["交水电费"]]);
        // One alone missed: told of alone, as it is.
        let mut kept = Kept { last: Some(time(14, 0)), told: ["整理周报 @2026-11-21 14:00".to_string(), "交水电费 @2026-11-21".to_string()].into(), ..Kept::default() };
        let looked = look(Some(&items), &mut kept, time(16, 25), RULES);
        assert_eq!(told(&looked.notices), vec![vec!["预约体检"]]);
        assert_eq!(says(&looked.notices[0], time(16, 25), true), ("预约体检".into(), "今天 16:30 · 还有 5 分钟".into()));
    }

    #[test]
    fn the_notification_of_those_missed_together_tells_of_those_left_as_they_are_done() {
        let mut items = vec![item("整理周报", Some((0, Some((14, 0))))), item("预约体检", Some((0, Some((15, 0))))), item("交水电费", Some((0, None)))];
        let mut kept = Kept { last: Some(time(13, 0)), told: ["交水电费 @2026-11-21".to_string()].into(), ..Kept::default() };
        assert_eq!(told(&look(Some(&items), &mut kept, time(16, 0), RULES).notices), vec![vec!["整理周报", "预约体检"]]);
        assert_eq!(look(Some(&items), &mut kept, time(16, 1), RULES).missed_left, None);
        // One ticked: of the other alone. Both: taken back. Then nothing more.
        items[0].done = true;
        let left = look(Some(&items), &mut kept, time(16, 2), RULES).missed_left.unwrap();
        assert_eq!(left.iter().map(|tell| tell.item.text.as_str()).collect::<Vec<_>>(), vec!["预约体检"]);
        items[1].due = Some(Due { day: day(), time: Some((18, 0)) });
        assert_eq!(look(Some(&items), &mut kept, time(16, 3), RULES).missed_left, Some(Vec::new()));
        assert_eq!(look(Some(&items), &mut kept, time(16, 4), RULES).missed_left, None);
        // (Another ticked, not among them: nothing to change in it.)
        let mut kept = Kept { last: Some(time(13, 0)), ..Kept::default() };
        look(Some(&items[1..]), &mut kept, time(16, 0), RULES);
        items[2].done = true;
        assert_eq!(look(Some(&items[1..]), &mut kept, time(16, 1), RULES).missed_left, None);
    }

    #[test]
    fn nothing_is_forgotten_while_the_to_dos_are_not_to_be_read() {
        let items = [item("回复邮件", Some((0, Some((15, 0))))), item("交水电费", Some((0, None)))];
        let mut kept = Kept { last: Some(time(8, 0)), ..Kept::default() };
        assert_eq!(look(Some(&items), &mut kept, time(9, 0), RULES).notices.len(), 1);
        tell_later(&mut kept, &key(&items[1]).unwrap(), &items[1], Later::Minutes(60), time(9, 0), RULES);
        // The notes' drive taken out from 9:30 to 11:00: nothing told, nothing forgotten.
        let was = kept.clone();
        for now in [time(9, 30), time(10, 0), time(10, 30)] {
            assert_eq!(look(None, &mut kept, now, RULES), Looked::default());
        }
        assert_eq!((&kept.told, &kept.later), (&was.told, &was.later));
        // Back: what was to be told again told then, and the other at its time.
        assert_eq!(told(&look(Some(&items), &mut kept, time(11, 0), RULES).notices), vec![vec!["交水电费"]]);
        assert_eq!(told(&look(Some(&items), &mut kept, time(14, 50), RULES).notices), vec![vec!["回复邮件"]]);
    }

    #[test]
    fn a_to_do_written_after_its_reminder_is_told_of_at_its_own_time_and_one_written_after_that_not_at_all() {
        let mut kept = Kept { last: Some(time(14, 0)), ..Kept::default() };
        assert!(look(Some(&[]), &mut kept, time(14, 55), RULES).notices.is_empty());
        // Written at 14:55, due at 15:00: its reminder (14:50) gone by, it is told of at 15:00.
        let soon = [item("开会", Some((0, Some((15, 0))))), item("早就过了", Some((0, Some((11, 0))))), item("今天全天", Some((0, None)))];
        let shown = watched(&soon, &mut kept, time(14, 55) + 10, time(16, 0));
        assert_eq!(shown, vec![(time(15, 0), vec![vec!["开会".to_string()]])]);
        assert_eq!(says(&Notice::One(Tell { key: String::new(), item: soon[0].clone() }), time(15, 0), true).1, "今天 15:00 · 现在");
    }

    #[test]
    fn a_to_do_changed_is_told_of_at_its_new_time_and_its_notification_taken_back() {
        let mut items = vec![item("回复邮件", Some((0, Some((15, 0)))))];
        let mut kept = Kept { last: Some(time(14, 0)), ..Kept::default() };
        assert_eq!(look(Some(&items), &mut kept, time(14, 50), RULES).notices.len(), 1);
        // Moved to 17:00: the old one's notification taken back, the new told of at 16:50.
        items[0].due = Some(Due { day: day(), time: Some((17, 0)) });
        let looked = look(Some(&items), &mut kept, time(14, 51), RULES);
        assert_eq!(looked.gone, vec!["回复邮件 @2026-11-21 15:00".to_string()]);
        assert_eq!(looked.next, Some(time(16, 50)));
        assert_eq!(look(Some(&items), &mut kept, time(16, 50), RULES).notices.len(), 1);
        // Ticked: taken back. Unticked, its reminder's time gone by: told of at its own.
        items[0].done = true;
        assert_eq!(look(Some(&items), &mut kept, time(16, 51), RULES).gone, vec!["回复邮件 @2026-11-21 17:00".to_string()]);
        items[0].done = false;
        let looked = look(Some(&items), &mut kept, time(16, 52), RULES);
        assert_eq!((looked.notices.len(), looked.next), (0, Some(time(17, 0))));
    }

    /// The hour all-day to-dos are told at moved back past now and on again (07:00 to 08:00 at 08:30, then back to
    /// 09:00), a to-do of the whole day not yet told of is told of at 09:00: moved back, it was kept as told without
    /// being told, and never told of that day.
    #[test]
    fn an_all_day_to_do_is_told_of_at_the_hour_moved_back_and_on_again() {
        let items = [item("交水电费", Some((0, None)))];
        let mut kept = Kept { last: Some(time(8, 0)), ..Kept::default() };
        let earlier = Rules { day_at: (8, 0), ..RULES };
        assert!(look(Some(&items), &mut kept, time(8, 30), RULES).notices.is_empty());
        assert!(look(Some(&items), &mut kept, time(8, 31), earlier).notices.is_empty());
        let shown = watched(&items, &mut kept, time(8, 32), time(10, 0));
        assert_eq!(shown, vec![(time(9, 0), vec![vec!["交水电费".to_string()]])]);
    }

    #[test]
    fn told_again_later_as_asked() {
        let items = [item("回复邮件", Some((0, Some((15, 0))))), item("交水电费", Some((0, None)))];
        let key = |n: usize| key(&items[n]).unwrap();
        let mut kept = Kept { last: Some(time(8, 0)), ..Kept::default() };
        tell_later(&mut kept, &key(0), &items[0], Later::Minutes(10), time(14, 50), RULES);
        tell_later(&mut kept, &key(1), &items[1], Later::Tomorrow, time(9, 0), RULES);
        assert_eq!(kept.later[&key(1)], at(day().plus(1), (9, 0)));
        let shown = watched(&items, &mut kept, time(14, 50), time(15, 30));
        assert_eq!(shown, vec![(time(15, 0), vec![vec!["回复邮件".to_string()]])]);
        // An hour, and the next day at this time.
        tell_later(&mut kept, &key(0), &items[0], Later::Minutes(60), time(15, 0), RULES);
        assert_eq!(kept.later[&key(0)], time(16, 0));
        tell_later(&mut kept, &key(0), &items[0], Later::Tomorrow, time(15, 0), RULES);
        assert_eq!(kept.later[&key(0)], at(day().plus(1), (15, 0)));
        assert_eq!(Later::named("60"), Some(Later::Minutes(60)));
        assert_eq!(Later::named("tomorrow"), Some(Later::Tomorrow));
        assert_eq!(Later::named("0"), None);
        assert_eq!(Later::named("x"), None);
    }

    #[test]
    fn kept_reads_back_as_written() {
        let kept = Kept { told: ["a @2026-11-21".into()].into(), later: [("b @2026-11-21 15:00".into(), time(15, 10))].into(), last: Some(time(14, 0)), missed: vec!["c @2026-11-21".into()] };
        assert_eq!(serde_json::from_str::<Kept>(&serde_json::to_string(&kept).unwrap()).unwrap(), kept);
        assert_eq!(serde_json::from_str::<Kept>("{}").unwrap(), Kept::default());
    }

    #[test]
    fn what_a_notification_says() {
        let one = |due| Notice::One(Tell { key: String::new(), item: item("回复邮件", due) });
        assert_eq!(says(&one(Some((0, Some((15, 0))))), time(14, 50), true), ("回复邮件".into(), "今天 15:00 · 还有 10 分钟".into()));
        assert_eq!(says(&one(Some((0, Some((15, 0))))), time(13, 50), true).1, "今天 15:00 · 还有 1 小时 10 分钟");
        assert_eq!(says(&one(Some((0, Some((15, 0))))), time(13, 0), true).1, "今天 15:00 · 还有 2 小时");
        assert_eq!(says(&one(Some((0, Some((15, 0))))), time(16, 0), true).1, "今天 15:00 · 已过时间");
        assert_eq!(says(&one(Some((0, None))), time(9, 0), true).1, "今天 · 全天");
        assert_eq!(says(&one(Some((0, Some((15, 0))))), time(14, 50), false).1, "Today 15:00 · In 10 min");
        let missed = Notice::Missed(vec![Tell { key: String::new(), item: item("a", Some((0, Some((17, 0))))) }, Tell { key: String::new(), item: item("b", Some((0, None))) }]);
        assert_eq!(says(&missed, time(16, 55), true).0, "错过了 2 项待办的提醒");
    }
}
