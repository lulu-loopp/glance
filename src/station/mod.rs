//! The station: a shelf to put things on for a while (files, pictures, text, links) and take them off again into
//! another program, and what was copied lately.

pub mod apps;
pub mod catch;
pub mod clipboard;
pub mod drag;
pub mod drop;
pub mod found;
pub mod folder;
pub mod keep;
pub mod recent;
pub mod run;
pub mod select;
pub mod shelf;
pub mod thumbs;
#[cfg(test)]
mod spike;

/// A press on what a station's lane shows (see `view::Hit`), in the panel and in a widget alike: done; whether it was
/// one. A thing on it, or copied lately, is put on the clipboard (pressed again within a double click's time, a thing
/// on it is opened).
pub fn pressed(hit: crate::ui::view::Hit) -> bool {
    use crate::ui::view::Hit;
    /// The thing on it last pressed, and when.
    static LAST: std::sync::Mutex<Option<(u64, std::time::Instant)>> = std::sync::Mutex::new(None);
    match hit {
        // A thing on it, or copied lately: put on the clipboard (pressed again within a double click's time, opened).
        Hit::Shelf(id) | Hit::Copied(id) => {
            let double = std::time::Duration::from_millis(u64::from(unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() }));
            let mut last = LAST.lock().unwrap();
            let again = last.is_some_and(|(was, at)| was == id && at.elapsed() <= double);
            *last = (!again).then(|| (id, std::time::Instant::now()));
            run::ask(if again { run::Ask::Open(id) } else { run::Ask::Copy(id) });
        }
        // The notes shown another way (their head's views).
        Hit::NotesShow(view) => {
            std::thread::spawn(move || crate::app().change(|settings| settings.notes_view = view));
        }
        // What is kept at hand: opened.
        Hit::App(number) => {
            if let Some(app) = apps::numbered(number) {
                run::ask(run::Ask::Launch(app.target.clone()));
            }
        }
        // A file come lately: put on the clipboard as a file (pressed again within a double click's time, opened).
        // Files come together: put on the clipboard (pressed again, their folder opened, showing them chosen: opened,
        // a phone's photos would open hundreds of windows).
        Hit::Recent(entry) => {
            let double = std::time::Duration::from_millis(u64::from(unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() }));
            static LAST_FILE: std::sync::Mutex<Option<(recent::Entry, std::time::Instant)>> = std::sync::Mutex::new(None);
            let mut last = LAST_FILE.lock().unwrap();
            let again = last.is_some_and(|(was, at)| was == entry && at.elapsed() <= double);
            *last = (!again).then(|| (entry, std::time::Instant::now()));
            if let Some(paths) = recent::paths_of(entry) {
                let opened = if matches!(entry, recent::Entry::Burst(..)) { run::FilesAct::Show } else { run::FilesAct::Open };
                run::ask(run::Ask::Files(paths, if again { opened } else { run::FilesAct::Copy(Some(entry)) }));
            }
        }
        _ => return false,
    }
    true
}

/// What a thing on the station's menu does: put it on the clipboard, open it, show it in Explorer, pin it (or not),
/// take it off.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Choice {
    Copy,
    Open,
    Preview,
    Show,
    Pin,
    Remove,
}

/// The menu of the thing `id` on the station (a right click on it), at `at` on the screen (`over_the_panel`: as
/// `Menu::open_at` says): up, with what its rows do.
pub fn menu(id: u64, at: windows::Win32::Foundation::POINT, over_the_panel: bool, look: crate::ui::menu::Look) -> Option<(crate::ui::menu::Menu, Vec<Choice>)> {
    use crate::ui::icons::Icon;
    use crate::ui::menu::{Item, Menu};
    use shelf::What;
    let shown = run::shown();
    let item = shown.items.iter().find(|item| item.id == id)?;
    let lang = crate::update::language();
    let (opens, in_explorer) = match &item.what {
        What::Text { .. } | What::Link { .. } => (true, false),
        What::Files { .. } | What::Picture { .. } | What::Saved { .. } | What::Came { .. } => (true, true),
    };
    let mut rows: Vec<(Choice, &'static str, Icon, bool)> = vec![(Choice::Copy, lang.pick("复制", "Copy"), Icon::Clipboard, false)];
    if opens {
        rows.push((Choice::Open, lang.pick("打开", "Open"), Icon::File, false));
    }
    rows.push((Choice::Preview, lang.pick("预览", "Preview"), Icon::Image, false));
    if in_explorer {
        rows.push((Choice::Show, lang.pick("在资源管理器中显示", "Show in Explorer"), Icon::Panel, false));
    }
    rows.push((Choice::Pin, if item.pinned { lang.pick("取消固定", "Unpin") } else { lang.pick("固定", "Pin") }, Icon::Pin, false));
    rows.push((Choice::Remove, lang.pick("移除", "Remove"), Icon::Trash, true));
    let items = rows.iter().map(|(_, label, icon, rule_before)| Item { label, mark: Some(crate::ui::menu::Mark::Icon(*icon)), checked: false, rule_before: *rule_before }).collect();
    let menu = Menu::open_at(items, look, at, false, over_the_panel)?;
    Some((menu, rows.into_iter().map(|(choice, ..)| choice).collect()))
}

/// `choice` made in the menu of the thing `id` on the station (see `menu`): done, on the station's thread.
pub fn chose(id: u64, choice: Choice) {
    let pinned = run::shown().items.iter().find(|item| item.id == id).is_some_and(|item| item.pinned);
    run::ask(match choice {
        Choice::Copy => run::Ask::Copy(id),
        Choice::Open => run::Ask::Open(id),
        // (Opened beside what it was chosen on, by that: see `Controller::end_menu`.)
        Choice::Preview => return,
        Choice::Show => run::Ask::Show(id),
        Choice::Pin => run::Ask::Pin(id, !pinned),
        Choice::Remove => run::Ask::Remove(id),
    });
}

/// What things pressed on a station's lane and moved far enough (see `run::drag_begins`) drag off, as one (the thing
/// pressed, and what is chosen with it: see `select`): things on it, or copied lately, as the station gives them, or
/// files come lately (or files come together), where they are (pressed on a window of the thread numbered `thread`).
/// The drag's number, as it is followed (see `drag::going`), where it is one.
pub fn drag_off(hits: &[crate::ui::view::Hit], thread: u32) -> Option<u64> {
    let given = given(hits);
    if given.is_empty() {
        return None;
    }
    let drag = drag::announce();
    run::ask(run::Ask::DragOut(given, thread));
    Some(drag)
}

/// What the station is asked to give of `hits` (see `run::Given`): things on it, or copied lately, by their numbers,
/// files come lately where they are (those no longer listed, nothing).
fn given(hits: &[crate::ui::view::Hit]) -> Vec<run::Given> {
    use crate::ui::view::Hit;
    hits.iter()
        .filter_map(|hit| match *hit {
            Hit::Shelf(id) | Hit::Copied(id) => Some(run::Given::Item(id)),
            Hit::Recent(entry) => recent::paths_of(entry).map(run::Given::Files),
            _ => None,
        })
        .collect()
}

/// What the menu of things chosen together does with all of them (see `select`): puts them on the clipboard as one,
/// shows those that are files in Explorer, puts files come lately on the station, pins them on it or not, takes them
/// off it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ManyChoice {
    Copy,
    Show,
    Station,
    Pin(bool),
    Remove,
}

/// What the menu of `hits`, chosen together, offers, as the station holds `items`: putting them on the clipboard,
/// showing them in Explorer, where any is a file (files come lately are), files come lately put on the station, things
/// on it, or copied lately, pinned (unpinned, where all are) and taken off.
pub fn many_choices(hits: &[crate::ui::view::Hit], items: &[shelf::Item]) -> Vec<ManyChoice> {
    use crate::ui::view::Hit;
    let item = |id: u64| items.iter().find(|item| item.id == id);
    let ids: Vec<u64> = hits.iter().filter_map(|hit| if let Hit::Shelf(id) | Hit::Copied(id) = hit { Some(*id) } else { None }).collect();
    let recent = hits.iter().any(|hit| matches!(hit, Hit::Recent(_)));
    let files = recent || ids.iter().filter_map(|id| item(*id)).any(|item| matches!(item.what, shelf::What::Files { .. } | shelf::What::Picture { .. } | shelf::What::Saved { words: false, .. } | shelf::What::Came { .. }));
    let mut choices = vec![ManyChoice::Copy];
    if files {
        choices.push(ManyChoice::Show);
    }
    if recent {
        choices.push(ManyChoice::Station);
    }
    if !ids.is_empty() {
        let pinned = ids.iter().all(|id| item(*id).is_some_and(|item| item.pinned));
        choices.extend([ManyChoice::Pin(!pinned), ManyChoice::Remove]);
    }
    choices
}

/// The menu of `hits`, chosen together (a right click on one of them), at `at` on the screen (`over_the_panel`: as
/// `Menu::open_at` says): up, with what its rows do (see `many_choices`).
pub fn many_menu(hits: &[crate::ui::view::Hit], at: windows::Win32::Foundation::POINT, over_the_panel: bool, look: crate::ui::menu::Look) -> Option<(crate::ui::menu::Menu, Vec<ManyChoice>)> {
    use crate::ui::icons::Icon;
    use crate::ui::menu::{Item, Menu};
    let lang = crate::update::language();
    let choices = many_choices(hits, &run::shown().items);
    let rows: Vec<(&str, Icon, bool)> = choices
        .iter()
        .map(|choice| match choice {
            ManyChoice::Copy => (lang.pick("全部复制", "Copy all"), Icon::Clipboard, false),
            ManyChoice::Show => (lang.pick("在资源管理器中显示", "Show in Explorer"), Icon::Panel, false),
            ManyChoice::Station => (lang.pick("全部放入中转站", "Put them all on the station"), Icon::Inbox, false),
            ManyChoice::Pin(true) => (lang.pick("全部固定", "Pin all"), Icon::Pin, false),
            ManyChoice::Pin(false) => (lang.pick("全部取消固定", "Unpin all"), Icon::Pin, false),
            ManyChoice::Remove => (lang.pick("全部移除", "Remove all"), Icon::Trash, true),
        })
        .collect();
    let items = rows.iter().map(|(label, icon, rule_before)| Item { label, mark: Some(crate::ui::menu::Mark::Icon(*icon)), checked: false, rule_before: *rule_before }).collect();
    let menu = Menu::open_at(items, look, at, false, over_the_panel)?;
    Some((menu, choices))
}

/// `choice` made in the menu of `hits`, chosen together (see `many_menu`): done for all of them. Taken off, nothing is
/// chosen any more.
pub fn many_chose(hits: &[crate::ui::view::Hit], choice: ManyChoice) {
    use crate::ui::view::Hit;
    let ids = || hits.iter().filter_map(|hit| if let Hit::Shelf(id) | Hit::Copied(id) = hit { Some(*id) } else { None });
    match choice {
        ManyChoice::Copy => run::ask(run::Ask::Together(given(hits), run::Together::Copy)),
        ManyChoice::Show => run::ask(run::Ask::Together(given(hits), run::Together::Show)),
        // (Each its own thing on the station, as each is put there from its own menu.)
        ManyChoice::Station => {
            for paths in hits.iter().filter_map(|hit| if let Hit::Recent(entry) = hit { recent::paths_of(*entry) } else { None }) {
                run::ask(run::Ask::Take(clipboard::Content::Files(paths)));
            }
        }
        ManyChoice::Pin(pinned) => ids().for_each(|id| run::ask(run::Ask::Pin(id, pinned))),
        ManyChoice::Remove => {
            ids().for_each(|id| run::ask(run::Ask::Remove(id)));
            select::change(select::Chosen::clear);
        }
    }
}

/// What a file come lately's menu does: opens it, shows it in Explorer, puts it on the station, copies its path, lists
/// no more of its folder's files. (Files come together: their folder opened showing them, all put on the station, the
/// folder's path copied.)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecentChoice {
    Open,
    Show,
    Station,
    CopyPath,
    Hide,
}

/// The menu of `entry` among the files come lately (a right click on it), at `at` on the screen: up, with what its
/// rows do.
pub fn recent_menu(entry: recent::Entry, at: windows::Win32::Foundation::POINT, over_the_panel: bool, look: crate::ui::menu::Look) -> Option<(crate::ui::menu::Menu, Vec<RecentChoice>)> {
    use crate::ui::icons::Icon;
    use crate::ui::menu::{Item, Menu};
    recent::paths_of(entry)?;
    let lang = crate::update::language();
    let rows: Vec<(RecentChoice, &str, Icon, bool)> = match entry {
        recent::Entry::File(_) => vec![
            (RecentChoice::Open, lang.pick("打开", "Open"), Icon::File, false),
            (RecentChoice::Show, lang.pick("在资源管理器中显示", "Show in Explorer"), Icon::Panel, false),
            (RecentChoice::Station, lang.pick("放入中转站", "Put on the station"), Icon::Inbox, false),
            (RecentChoice::CopyPath, lang.pick("复制路径", "Copy its path"), Icon::Clipboard, false),
            (RecentChoice::Hide, lang.pick("不再列出这个文件夹", "List this folder no more"), Icon::EyeOff, true),
        ],
        recent::Entry::Burst(..) => vec![
            (RecentChoice::Show, lang.pick("打开所在文件夹", "Open their folder"), Icon::Panel, false),
            (RecentChoice::Station, lang.pick("全部放入中转站", "Put them all on the station"), Icon::Inbox, false),
            (RecentChoice::CopyPath, lang.pick("复制文件夹路径", "Copy the folder's path"), Icon::Clipboard, false),
            (RecentChoice::Hide, lang.pick("不再列出这个文件夹", "List this folder no more"), Icon::EyeOff, true),
        ],
    };
    let items = rows.iter().map(|(_, label, icon, rule_before)| Item { label, mark: Some(crate::ui::menu::Mark::Icon(*icon)), checked: false, rule_before: *rule_before }).collect();
    let menu = Menu::open_at(items, look, at, false, over_the_panel)?;
    Some((menu, rows.into_iter().map(|(choice, ..)| choice).collect()))
}

/// `choice` made in the menu of `entry` among the files come lately (see `recent_menu`): done.
pub fn recent_chose(entry: recent::Entry, choice: RecentChoice) {
    let Some(paths) = recent::paths_of(entry) else { return };
    // (The files come together are in one folder.)
    let folder = paths.first().and_then(|path| path.parent()).map(std::path::Path::to_path_buf);
    match choice {
        RecentChoice::Open => run::ask(run::Ask::Files(paths, run::FilesAct::Open)),
        RecentChoice::Show => run::ask(run::Ask::Files(paths, run::FilesAct::Show)),
        RecentChoice::Station => run::ask(run::Ask::Take(clipboard::Content::Files(paths))),
        RecentChoice::CopyPath => {
            let copied = match entry {
                recent::Entry::File(_) => paths.first().cloned(),
                recent::Entry::Burst(..) => folder,
            };
            if let Some(copied) = copied {
                run::ask(run::Ask::CopyText(copied.display().to_string(), Some(run::Copied::Recent(entry))));
            }
        }
        RecentChoice::Hide => {
            if let Some(folder) = folder {
                std::thread::spawn(move || crate::app().change(|settings| if !settings.recent_hidden.contains(&folder) { settings.recent_hidden.push(folder) }));
            }
        }
    }
}

/// What the menu of a thing kept at hand does: opens it, shows where it is in Explorer, takes it off.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AppChoice {
    Open,
    Show,
    Remove,
}

/// The menu of the thing kept at hand at `at` among them (a right click on it), at `point` on the screen: up, with
/// what its rows do (where it is is shown only for what is on the disk).
pub fn app_menu(number: u64, point: windows::Win32::Foundation::POINT, over_the_panel: bool, look: crate::ui::menu::Look) -> Option<(crate::ui::menu::Menu, Vec<AppChoice>, apps::App)> {
    use crate::ui::icons::Icon;
    use crate::ui::menu::{Item, Menu};
    let app = apps::numbered(number)?;
    let lang = crate::update::language();
    let mut rows = vec![(AppChoice::Open, lang.pick("打开", "Open"), Icon::File, false)];
    if app.location().is_some() {
        rows.push((AppChoice::Show, lang.pick("打开文件所在的位置", "Open its location"), Icon::Panel, false));
    }
    rows.push((AppChoice::Remove, lang.pick("从快捷 App 移除", "Take off quick launch"), Icon::PinOff, true));
    let items = rows.iter().map(|(_, label, icon, rule_before)| Item { label, mark: Some(crate::ui::menu::Mark::Icon(*icon)), checked: false, rule_before: *rule_before }).collect();
    let menu = Menu::open_at(items, look, point, false, over_the_panel)?;
    Some((menu, rows.into_iter().map(|(choice, ..)| choice).collect(), app))
}

/// `choice` made in the menu of `app`, kept at hand (see `app_menu`: the one its menu was opened on, whatever is
/// kept now): done.
pub fn app_chose(app: apps::App, choice: AppChoice) {
    match choice {
        AppChoice::Open => run::ask(run::Ask::Launch(app.target)),
        AppChoice::Show => {
            if let Some(path) = app.location() {
                run::ask(run::Ask::Files(vec![path], run::FilesAct::Show));
            }
        }
        AppChoice::Remove => apps::change(move |apps| apps.retain(|kept| *kept != app)),
    }
}

/// What a row of the station's head's menus does: shows what is on it so; takes off what is not pinned, of what is on
/// it or of what was copied lately; keeps no more of what is copied; opens the settings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HeadChoice {
    /// One of its pages shown (from the menu of them all, where their names do not fit side by side).
    Page(crate::ui::view::Tab),
    View(crate::settings::StationPage, crate::settings::ShelfView),
    Clear,
    ClearCopies,
    StopCopies,
    StopRecent,
    Settings,
}

/// The menu a press on the station's head opens (`hit`: the button of how what is on it shows, or of the rest, the
/// page `tab` shown), at `at` on the screen (`over_the_panel`: as `Menu::open_at` says): up, with what its rows do.
pub fn head_menu(hit: crate::ui::view::Hit, tab: crate::ui::view::Tab, at: windows::Win32::Foundation::POINT, over_the_panel: bool, look: crate::ui::menu::Look) -> Option<(crate::ui::menu::Menu, Vec<HeadChoice>)> {
    use crate::settings::ShelfView;
    use crate::ui::icons::Icon;
    use crate::ui::menu::{Item, Menu};
    use crate::ui::view::{Hit, Tab};
    let lang = crate::update::language();
    let view = run::shown().view;
    // Its pages, as one: each, the one shown ticked.
    if hit == Hit::StationPages {
        let shown = run::shown();
        let mut pages = vec![(Tab::Shelf, lang.pick("中转站", "Station"), Icon::Inbox)];
        if shown.history {
            pages.push((Tab::Copies(Default::default()).again(tab), lang.pick("最近复制", "Copied"), Icon::Clipboard));
        }
        if recent::on() {
            pages.push((Tab::Recent(Default::default()).again(tab), lang.pick("最近文件", "Recent"), Icon::File));
        }
        let items = pages.iter().map(|(page, label, icon)| Item { label, mark: Some(crate::ui::menu::Mark::Icon(*icon)), checked: page.page() == tab.page(), rule_before: false }).collect();
        let menu = Menu::open_at(items, look, at, false, over_the_panel)?;
        return Some((menu, pages.into_iter().map(|(page, ..)| HeadChoice::Page(page)).collect()));
    }
    if hit != Hit::StationView {
        return None;
    }
    // Each row: what it does, its name, its icon, whether it is ticked, and whether a rule sets it off. How the page
    // shows (each page its own way); under a rule, what else is done with it; the settings last.
    let (page, view) = match tab {
        Tab::Shelf => (crate::settings::StationPage::Shelf, view),
        Tab::Copies(_) => (crate::settings::StationPage::Copies, run::shown().copies_view),
        Tab::Recent(_) => (crate::settings::StationPage::Recent, run::shown().recent_view),
    };
    let mut rows: Vec<(HeadChoice, &'static str, Icon, bool, bool)> = [(ShelfView::Cards, lang.pick("小卡片", "Small cards"), Icon::LayoutGrid), (ShelfView::Large, lang.pick("大卡片", "Large cards"), Icon::Image), (ShelfView::List, lang.pick("列表", "List"), Icon::List)]
        .into_iter()
        .map(|(shows, label, icon)| (HeadChoice::View(page, shows), label, icon, shows == view, false))
        .collect();
    rows.extend(match tab {
        Tab::Shelf => vec![(HeadChoice::Clear, lang.pick("全部清空（保留固定的项目）", "Clear all but the pinned"), Icon::Trash, false, true)],
        Tab::Recent(_) => vec![(HeadChoice::StopRecent, lang.pick("不再列出最近文件", "Stop listing recent files"), Icon::EyeOff, false, true)],
        Tab::Copies(_) => vec![
            (HeadChoice::ClearCopies, lang.pick("清空最近复制（保留固定的项目）", "Clear Recently copied but the pinned"), Icon::Trash, false, true),
            (HeadChoice::StopCopies, lang.pick("不再记录复制的内容", "Stop keeping what is copied"), Icon::Clipboard, false, false),
        ],
    });
    rows.push((HeadChoice::Settings, lang.pick("中转站设置", "Station settings"), Icon::Settings, false, true));
    let items = rows.iter().map(|(_, label, icon, checked, rule_before)| Item { label, mark: Some(crate::ui::menu::Mark::Icon(*icon)), checked: *checked, rule_before: *rule_before }).collect();
    let menu = Menu::open_at(items, look, at, false, over_the_panel)?;
    Some((menu, rows.into_iter().map(|(choice, ..)| choice).collect()))
}

/// `choice` made in a menu of the station's head (see `head_menu`): done (the settings changed off the calling thread,
/// which they tell to restyle).
pub fn head_chose(choice: HeadChoice, place: Option<windows::Win32::Foundation::RECT>) {
    match choice {
        HeadChoice::View(page, view) => {
            std::thread::spawn(move || {
                crate::app().change(|settings| match page {
                    crate::settings::StationPage::Shelf => settings.station.view = view,
                    crate::settings::StationPage::Copies => settings.station.copies_view = view,
                    crate::settings::StationPage::Recent => settings.station.recent_view = view,
                })
            });
        }
        HeadChoice::Clear => run::ask(run::Ask::Clear(false)),
        HeadChoice::ClearCopies => run::ask(run::Ask::Clear(true)),
        HeadChoice::StopCopies => {
            std::thread::spawn(|| crate::app().change(|settings| settings.station.history = false));
        }
        HeadChoice::StopRecent => {
            std::thread::spawn(|| crate::app().change(|settings| settings.station.recent = false));
        }
        HeadChoice::Settings => crate::show_workbench_settings(place),
        // (Shown by what it was chosen on: see the panel's `end_menu`.)
        HeadChoice::Page(_) => {}
    }
}

/// Now, in ms since 1970 (as the shelf keeps when things were put on it).
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::view::Hit;

    /// The menu of things chosen together offers what may be done with all of them: put on the clipboard ever, shown
    /// in Explorer where any is a file, files come lately put on the station, things on it pinned (unpinned where all
    /// are) and taken off.
    #[test]
    fn the_menu_of_things_chosen_together_offers_what_all_may_do() {
        use shelf::{Item, What};
        let item = |id: u64, what: What, pinned: bool| Item { id, what, added_ms: id, pinned, copied: false, stays_here: false, own_bytes: 0, from: None };
        let items = vec![
            item(1, What::Text { text: "a".into() }, true),
            item(2, What::Link { url: "https://a.b".into() }, false),
            item(3, What::Picture { file: "3.png".into() }, true),
            item(4, What::Saved { file: "4".into(), name: "a.txt".into(), words: true }, false),
        ];
        assert_eq!(many_choices(&[Hit::Shelf(1), Hit::Shelf(2)], &items), [ManyChoice::Copy, ManyChoice::Pin(true), ManyChoice::Remove]);
        assert_eq!(many_choices(&[Hit::Shelf(1), Hit::Copied(4)], &items), [ManyChoice::Copy, ManyChoice::Pin(true), ManyChoice::Remove]);
        assert_eq!(many_choices(&[Hit::Shelf(1), Hit::Shelf(3)], &items), [ManyChoice::Copy, ManyChoice::Show, ManyChoice::Pin(false), ManyChoice::Remove]);
        let entry = recent::Entry::File(recent::Key { drive: 'C', file: 1 });
        let other = recent::Entry::File(recent::Key { drive: 'C', file: 2 });
        assert_eq!(many_choices(&[Hit::Recent(entry), Hit::Recent(other)], &items), [ManyChoice::Copy, ManyChoice::Show, ManyChoice::Station]);
    }
}
