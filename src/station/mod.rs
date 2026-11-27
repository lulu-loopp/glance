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
pub mod send;
pub mod shelf;
pub mod thumbs;
#[cfg(test)]
mod spike;

/// What a station's lane is drawn from: the station as its thread shows it, and the files come lately. Drawn otherwise
/// than before (see `moved_from`), the lane turns from how it was drawn (see `as_was`).
#[derive(Clone)]
pub struct Drawn {
    station: std::sync::Arc<run::Shown>,
    files: std::sync::Arc<Vec<recent::Came>>,
}

impl Drawn {
    /// As it is now.
    pub fn now() -> Drawn {
        Drawn { station: run::shown(), files: recent::shown() }
    }

    /// Whether it is other than `before` (see `run::Shown::moved_from`), or what is pinned of the files come lately is
    /// (pinned or not, in another order).
    pub fn moved_from(&self, before: &Drawn) -> bool {
        self.station.moved_from(&before.station) || recent::pinned_rows(&self.files) != recent::pinned_rows(&before.files)
    }

    /// What `draw` gives, the lane drawn as it was.
    pub fn as_was<T>(self, draw: impl FnOnce() -> T) -> T {
        let Drawn { station, files } = self;
        run::as_was(station, || recent::as_was(files, draw))
    }
}

/// A thing that may be pinned and tagged, on any of the station's pages: one on it, or copied lately, by its number, part
/// of what one holds together (by its number and the part's place: pinned or tagged, it is taken out of it first, a thing
/// of its own, see `keep::Keep::take_part`), or a file come lately (or files come together, as one: all of them).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Thing {
    Item(u64),
    Part(u64, usize),
    Recent(recent::Entry),
}

impl Thing {
    /// The thing `hit` is, if one.
    pub fn of(hit: crate::ui::view::Hit) -> Option<Thing> {
        use crate::ui::view::Hit;
        match hit {
            Hit::Shelf(id) | Hit::Copied(id) => Some(Thing::Item(id)),
            Hit::Part(id, at) => Some(Thing::Part(id, at)),
            Hit::Recent(entry) => Some(Thing::Recent(entry)),
            _ => None,
        }
    }
}

/// Whether all of `things` are pinned, and the tags all of them have, the station holding `items` and the files come
/// lately being `files` (those gone are not counted, none left: not pinned, of no tag). Part of what one holds is marked as
/// it is, as it is when taken out.
fn marks_among(things: &[Thing], items: &[shelf::Item], files: &[recent::Came]) -> (bool, Vec<u32>) {
    let mut marks: Vec<(bool, &[u32])> = Vec::new();
    for thing in things {
        match *thing {
            Thing::Item(id) | Thing::Part(id, _) => marks.extend(items.iter().filter(|item| item.id == id).map(|item| (item.pinned, item.tags.as_slice()))),
            Thing::Recent(entry) => marks.extend(recent::files_of(files, entry).into_iter().map(|came| (came.marks.pinned, came.marks.tags.as_slice()))),
        }
    }
    let Some((_, first)) = marks.first() else { return (false, Vec::new()) };
    (marks.iter().all(|(pinned, _)| *pinned), first.iter().copied().filter(|tag| marks.iter().all(|(_, tags)| tags.contains(tag))).collect())
}

/// Whether all of `things` are pinned now, and the tags all of them have.
pub fn marks(things: &[Thing]) -> (bool, Vec<u32>) {
    marks_among(things, &run::shown().items, &recent::shown())
}

/// `things` taken apart: those on the station or copied lately, by their numbers, the files come lately, by theirs, and
/// parts of what they hold together.
fn apart(things: &[Thing]) -> (Vec<u64>, Vec<recent::Key>, Vec<(u64, usize)>) {
    let files = recent::shown();
    let ids = things.iter().filter_map(|thing| if let Thing::Item(id) = thing { Some(*id) } else { None }).collect();
    let keys = things.iter().flat_map(|thing| if let Thing::Recent(entry) = thing { recent::files_of(&files, *entry) } else { Vec::new() }).map(|came| came.key).collect();
    let parts = things.iter().filter_map(|thing| if let Thing::Part(id, at) = thing { Some((*id, *at)) } else { None }).collect();
    (ids, keys, parts)
}

/// The files come lately `keys` marked with `mark` (what holds them on the station told as the list is: see `lib`).
fn mark_files(keys: Vec<recent::Key>, mark: recent::Mark) {
    if !keys.is_empty() {
        recent::mark(keys, mark);
    }
}

/// `things` pinned, or not any more.
pub fn pin(things: &[Thing], pinned: bool) {
    let (ids, keys, parts) = apart(things);
    ids.into_iter().for_each(|id| run::ask(run::Ask::Pin(id, pinned)));
    mark_parts(parts, run::Marked::Pin(pinned));
    mark_files(keys, recent::Mark::Pinned(pinned));
}

/// `things` of the tag `tag`, or, where all are, not of it any more.
pub fn turn_tag(things: &[Thing], tag: u32) {
    let on = !marks(things).1.contains(&tag);
    let (ids, keys, parts) = apart(things);
    if !ids.is_empty() {
        run::ask(run::Ask::Tag(ids, tag, on));
    }
    mark_parts(parts, run::Marked::Tag(tag, on));
    mark_files(keys, recent::Mark::Tag(tag, on));
}

/// Parts of what things hold together (`parts`) each taken out of it, a thing of its own, and marked so.
fn mark_parts(parts: Vec<(u64, usize)>, marked: run::Marked) {
    if !parts.is_empty() {
        run::ask(run::Ask::Marked(parts, marked));
    }
}

/// The tags a page shows to sort it by (`tags`: those there are, in their order): those its own things have (`here`), those
/// nothing has on any page, for them to be taken away somewhere (`items`, `files`: all there is), and the one it shows
/// (`lit`).
pub fn tags_of<'a>(tags: &'a [shelf::Tag], here: impl Fn(u32) -> bool, lit: Option<u32>, items: &[shelf::Item], files: &[recent::Came]) -> Vec<&'a shelf::Tag> {
    let used = |tag: u32| items.iter().any(|item| item.tags.contains(&tag)) || files.iter().any(|came| came.marks.tags.contains(&tag));
    tags.iter().filter(|tag| lit == Some(tag.id) || here(tag.id) || !used(tag.id)).collect()
}

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
        // Part of what one holds together: as a thing on it is.
        Hit::Part(id, part) => {
            static LAST_PART: std::sync::Mutex<Option<((u64, usize), std::time::Instant)>> = std::sync::Mutex::new(None);
            let double = std::time::Duration::from_millis(u64::from(unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() }));
            let mut last = LAST_PART.lock().unwrap();
            let again = last.is_some_and(|(was, at)| was == (id, part) && at.elapsed() <= double);
            *last = (!again).then(|| ((id, part), std::time::Instant::now()));
            run::ask(run::Ask::Part(id, part, if again { run::PartAct::Open } else { run::PartAct::Copy }));
        }
        // The count on what holds things together: they shown under it, or no longer.
        Hit::Unfold(thing) => unfold(thing),
        // Its pin: pinned, or not any more. What is pinned, where it is listed first: folded away, or shown.
        Hit::PinThing(thing) => pin(&[thing], !marks(&[thing]).0),
        Hit::PinsHead(page) => {
            std::thread::spawn(move || crate::app().change(|settings| settings.station.folded.turn(page)));
        }
        // A device on the strip a sending to goes on: that sending cancelled.
        Hit::SendTo(key) => {
            send::cancel(key);
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

/// What holds things together (`thing`: one on the station or copied lately, files come lately together) shown with them
/// under it, each its own, or no longer (see `run::Shown::unfolded`).
pub fn unfold(thing: Thing) {
    let shown = run::shown();
    run::ask(match thing {
        Thing::Item(id) => run::Ask::Unfold(run::Unfolded::Item(id), !shown.unfolds(id)),
        Thing::Recent(entry) => {
            let keys: Vec<recent::Key> = recent::files_of(&recent::shown(), entry).iter().map(|came| came.key).collect();
            let on = !shown.unfolds_files(keys.iter());
            run::Ask::Unfold(run::Unfolded::Files(keys), on)
        }
        // (Holding nothing together.)
        Thing::Part(..) => return,
    });
}

/// What a thing on the station's menu does: put it on the clipboard, open it, show it in Explorer, pin it (or not), tag
/// it (`Tags`, the row opening the tags, `Tag` one of them by its number, `NewTag` one made: see `tags_row`), send it to a device nearby (`Send`, the row opening the devices, `SendTo` one of them by its key: see
/// `send::menu_row`), take it off.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Choice {
    Copy,
    Open,
    Preview,
    Show,
    Rename,
    Pin,
    Tags,
    Tag(u32),
    NewTag,
    Send,
    SendTo(u64),
    /// All it holds together each a thing of its own (see `keep::Keep::split`).
    Split,
    Remove,
}

/// What the menu of part of what a thing holds together does (see `part_menu`): put it on the clipboard, open it, look at
/// it, show it in Explorer, rename it, send it to a device nearby (`Send`, the row opening the devices, `SendTo` one of
/// them by its key), take it out of what holds it as a thing of its own, take it off.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PartChoice {
    Copy,
    Open,
    Preview,
    Show,
    Rename,
    Send,
    SendTo(u64),
    TakeOut,
    Remove,
}

/// The tags there are, as `things` may be of them: those all of them have ticked (taken off them as they are chosen
/// again), and a new one made, and what each is chosen as.
fn tag_rows<C>(things: &[Thing], tag: fn(u32) -> C, new: C) -> (Vec<crate::ui::menu::Item<'static>>, Vec<C>) {
    use crate::ui::icons::Icon;
    use crate::ui::menu::{Item, Mark};
    let shown = run::shown();
    let (tags, (_, on)) = (&shown.tags, marks(things));
    let mut rows: Vec<Item<'static>> = tags.iter().map(|tag| Item::new(tag.name.clone(), Some(Mark::Icon(Icon::Tag)), on.contains(&tag.id), false)).collect();
    rows.push(Item::new(crate::update::language().pick("新建标签", "New tag"), Some(Mark::Icon(Icon::Add)), false, !tags.is_empty()));
    (rows, tags.iter().map(|one| tag(one.id)).chain([new]).collect())
}

/// A menu's "标签" of `things`: the row, opening the tags there are (see `tag_rows`), and what each of those is chosen
/// as.
fn tags_row<C>(things: &[Thing], tag: fn(u32) -> C, new: C) -> (crate::ui::menu::Item<'static>, Vec<C>) {
    use crate::ui::icons::Icon;
    use crate::ui::menu::{Item, Mark};
    let mut row = Item::new(crate::update::language().pick("标签", "Tags"), Some(Mark::Icon(Icon::Tag)), false, false);
    let (more, chosen) = tag_rows(things, tag, new);
    row.more = more;
    (row, chosen)
}

/// What the menu of a thing's tags does (see `tags_menu`): the thing of one, or not any more, or a new one made.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TagsChoice {
    Tag(u32),
    New,
}

/// The menu of the tags of `thing` (its button pressed), at `at` on the screen: up, with what its rows do.
pub fn tags_menu(thing: Thing, at: windows::Win32::Foundation::POINT, over_the_panel: bool, look: crate::ui::menu::Look) -> Option<(crate::ui::menu::Menu, Vec<TagsChoice>)> {
    let (rows, chosen) = tag_rows(&[thing], TagsChoice::Tag, TagsChoice::New);
    Some((crate::ui::menu::Menu::open_at(rows, look, at, false, over_the_panel)?, chosen))
}

/// A new tag made at once, `things` of it, named as Windows names a new folder (see `new_tag_name`): its name to be
/// written in where it shows (see `ui::naming`).
pub fn new_tag(things: &[Thing]) -> crate::ui::naming::Naming {
    let name = new_tag_name(&run::shown().tags, crate::update::language());
    let (ids, keys, parts) = apart(things);
    run::ask(run::Ask::NewTag(name.clone(), ids, keys.clone(), parts));
    crate::ui::naming::Naming::new_tag(name, keys)
}

/// What a new tag is named among `tags`: "新建标签", or where a tag has that name, "新建标签 2", and so on, the first no
/// tag has.
fn new_tag_name(tags: &[shelf::Tag], lang: crate::ui::text::Lang) -> String {
    let base = lang.pick("新建标签", "New tag");
    (1..).map(|n| if n == 1 { base.to_string() } else { format!("{base} {n}") }).find(|name| !tags.iter().any(|tag| tag.name == *name)).unwrap()
}

/// What a tag's menu does (see `tag_menu`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TagChoice {
    Rename,
    Remove,
}

/// The menu of the tag `id` (a right click on it, or its button), at `at` on the screen: up, with what its rows do, and
/// its name.
pub fn tag_menu(id: u32, at: windows::Win32::Foundation::POINT, over_the_panel: bool, look: crate::ui::menu::Look) -> Option<(crate::ui::menu::Menu, Vec<TagChoice>, String)> {
    use crate::ui::icons::Icon;
    use crate::ui::menu::{Item, Mark, Menu};
    let name = run::shown().tags.iter().find(|tag| tag.id == id)?.name.clone();
    let lang = crate::update::language();
    let items = vec![Item::new(lang.pick("重命名", "Rename"), Some(Mark::Icon(Icon::SquarePen)), false, false), Item::new(lang.pick("删除标签", "Delete the tag"), Some(Mark::Icon(Icon::Trash)), false, true)];
    Some((Menu::open_at(items, look, at, false, over_the_panel)?, vec![TagChoice::Rename, TagChoice::Remove], name))
}

/// How `thing` is renamed where it shows (see `ui::naming`), where it has a name to rename (see
/// `shelf::What::renamed_by`; a file come lately alone): a file or folder where it is renamed there (see
/// `rename_there`), one the station keeps itself given out by the name written from now on (a picture never renamed
/// written in as what it is, of its kind).
pub fn renaming(thing: Thing) -> Option<crate::ui::naming::Naming> {
    renamed(match thing {
        Thing::Item(id) => Renaming::Item(id),
        Thing::Part(id, at) => Renaming::Part(id, at),
        Thing::Recent(entry) => Renaming::Recent(entry),
    })
}

/// What is renamed where it shows (see `renamed`): a thing on the station, part of what one holds together, a file come
/// lately.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Renaming {
    Item(u64),
    Part(u64, usize),
    Recent(recent::Entry),
}

/// How `what` is renamed where it shows (see `ui::naming`), where it has a name to rename (see `renaming`; part of what a
/// thing holds together as `shelf::What::part_renamed_by` says).
pub fn renamed(what: Renaming) -> Option<crate::ui::naming::Naming> {
    use crate::ui::naming::Naming;
    use crate::ui::view::naming::Named;
    let lang = crate::update::language();
    let of = match what {
        Renaming::Item(id) => Named::Item(id),
        Renaming::Part(id, at) => Named::Part(id, at),
        Renaming::Recent(entry) => Named::Recent(entry),
    };
    let there = |path: std::path::PathBuf| {
        let folder = thumbs::looked(&path) == Some(thumbs::Looked::Folder);
        let name = path.file_name()?.to_string_lossy().into_owned();
        Some(Naming::file(of, name, folder, lang, move |written| rename_there(&path, written, lang)))
    };
    match what {
        Renaming::Part(id, at) => {
            let shown = run::shown();
            let item = shown.items.iter().find(|item| item.id == id)?;
            match item.what.part_renamed_by(at)? {
                shelf::Renamed::There(path) => there(path.to_path_buf()),
                shelf::Renamed::Own { name, folder } => Some(Naming::file(of, name, folder, lang, move |written| {
                    run::ask(run::Ask::Part(id, at, run::PartAct::Rename(written.to_string())));
                    Ok(())
                })),
            }
        }
        Renaming::Item(id) => {
            let shown = run::shown();
            let item = shown.items.iter().find(|item| item.id == id)?;
            match item.what.renamed_by()? {
                shelf::Renamed::There(path) => there(path.to_path_buf()),
                shelf::Renamed::Own { name, folder } => {
                    let name = match &item.what {
                        shelf::What::Picture { file, .. } if name.is_empty() => {
                            let kind = std::path::Path::new(file).extension().map_or_else(String::new, |kind| format!(".{}", kind.to_string_lossy()));
                            format!("{}{kind}", lang.pick("图片", "Picture"))
                        }
                        _ => name,
                    };
                    Some(Naming::file(of, name, folder, lang, move |written| {
                        run::ask(run::Ask::Rename(id, written.to_string()));
                        Ok(())
                    }))
                }
            }
        }
        Renaming::Recent(entry @ recent::Entry::File(_)) => there(recent::paths_of(entry)?.into_iter().next()?),
        Renaming::Recent(recent::Entry::Burst(..)) => None,
    }
}

/// The file or folder at `path` named `name` where it is (where nothing there has that name): what holds it on the
/// station holds it so (see `shelf::Shelf::moved`; the files come lately follow it themselves). Why not, where not.
fn rename_there(path: &std::path::Path, name: &str, lang: crate::ui::text::Lang) -> Result<(), String> {
    let to = path.with_file_name(name);
    match crate::os::move_new(path, &to) {
        Ok(()) => {
            run::ask(run::Ask::Moved(path.to_path_buf(), to));
            Ok(())
        }
        Err(error) => {
            crate::journal::note(format!("station: not renamed ({error})"));
            // (winerror.h: a file not found, its path not found; access denied, a sharing violation; a file there, one
            // already there.)
            Err(match error.raw_os_error() {
                Some(2 | 3) => lang.pick("此项目已不存在", "This item no longer exists"),
                Some(5 | 32) => lang.pick("此项目正在使用，或没有修改权限", "This item is in use, or can't be changed"),
                Some(80 | 183) => lang.pick("此位置已有同名的项目", "An item with this name already exists here"),
                _ => lang.pick("未能重命名", "Couldn't rename"),
            }
            .to_string())
        }
    }
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
        rows.push((Choice::Show, lang.pick("在文件资源管理器中显示", "Show in File Explorer"), Icon::Panel, false));
    }
    if item.what.renamed_by().is_some() {
        rows.push((Choice::Rename, lang.pick("重命名", "Rename"), Icon::SquarePen, false));
    }
    rows.push((Choice::Pin, if item.pinned { lang.pick("取消固定", "Unpin") } else { lang.pick("固定", "Pin") }, Icon::Pin, false));
    // (What holds things together: each made a thing of its own, after "发送到".)
    let splits = item.what.parts() > 1;
    rows.push((Choice::Remove, lang.pick("移除", "Remove"), Icon::Trash, true));
    let stays_here = item.stays_here;
    let mut items: Vec<Item<'static>> = rows.iter().map(|(_, label, icon, rule_before)| Item::new(*label, Some(crate::ui::menu::Mark::Icon(*icon)), false, *rule_before)).collect();
    let mut choices: Vec<Choice> = rows.into_iter().map(|(choice, ..)| choice).collect();
    // "标签" and "发送到" before the rule, their tags and devices chosen after all its rows (see `Menu::sub_choice`).
    let (tags, tagged) = tags_row(&[Thing::Item(id)], Choice::Tag, Choice::NewTag);
    items.insert(items.len() - 1, tags);
    choices.insert(choices.len() - 1, Choice::Tags);
    let (send, keys) = send::menu_row(stays_here);
    items.insert(items.len() - 1, send);
    choices.insert(choices.len() - 1, Choice::Send);
    if splits {
        items.insert(items.len() - 1, Item::new(lang.pick("拆分为单独项目", "Split into separate items"), Some(crate::ui::menu::Mark::Icon(Icon::Ungroup)), false, false));
        choices.insert(choices.len() - 1, Choice::Split);
    }
    choices.extend(tagged.into_iter().chain(keys.into_iter().map(Choice::SendTo)));
    let menu = Menu::open_at(items, look, at, false, over_the_panel)?;
    Some((menu, choices))
}

/// The menu of part `part` of what the thing `id` on the station holds together (a right click on it, shown under it),
/// at `at` on the screen (`over_the_panel`: as `Menu::open_at` says): up, with what its rows do.
pub fn part_menu(id: u64, part: usize, at: windows::Win32::Foundation::POINT, over_the_panel: bool, look: crate::ui::menu::Look) -> Option<(crate::ui::menu::Menu, Vec<PartChoice>)> {
    use crate::ui::icons::Icon;
    use crate::ui::menu::{Item, Menu};
    let shown = run::shown();
    let item = shown.items.iter().find(|item| item.id == id).filter(|item| part < item.what.parts() && item.what.parts() > 1)?;
    let lang = crate::update::language();
    let mut rows: Vec<(PartChoice, &'static str, Icon, bool)> = vec![
        (PartChoice::Copy, lang.pick("复制", "Copy"), Icon::Clipboard, false),
        (PartChoice::Open, lang.pick("打开", "Open"), Icon::File, false),
        (PartChoice::Preview, lang.pick("预览", "Preview"), Icon::Image, false),
        (PartChoice::Show, lang.pick("在文件资源管理器中显示", "Show in File Explorer"), Icon::Panel, false),
    ];
    if item.what.part_renamed_by(part).is_some() {
        rows.push((PartChoice::Rename, lang.pick("重命名", "Rename"), Icon::SquarePen, false));
    }
    rows.push((PartChoice::TakeOut, lang.pick("移出此组", "Take out of group"), Icon::ArrowUpFromLine, true));
    rows.push((PartChoice::Remove, lang.pick("移除", "Remove"), Icon::Trash, false));
    let mut items: Vec<Item<'static>> = rows.iter().map(|(_, label, icon, rule_before)| Item::new(*label, Some(crate::ui::menu::Mark::Icon(*icon)), false, *rule_before)).collect();
    let mut choices: Vec<PartChoice> = rows.into_iter().map(|(choice, ..)| choice).collect();
    // "发送到" before the rule, its devices chosen after all its rows (see `Menu::sub_choice`).
    let (send, keys) = send::menu_row(item.stays_here);
    let before = choices.len() - 2;
    items.insert(before, send);
    choices.insert(before, PartChoice::Send);
    choices.extend(keys.into_iter().map(PartChoice::SendTo));
    let menu = Menu::open_at(items, look, at, false, over_the_panel)?;
    Some((menu, choices))
}

/// What the menu of a file the look shows does (see `given_menu`): put it on the clipboard, open it, show it in
/// Explorer, send it to a device nearby (`Send`, the row opening the devices, `SendTo` one of them by its key).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GivenChoice {
    Copy,
    Open,
    Show,
    Send,
    SendTo(u64),
}

/// The menu of `given`, a file the look shows (one of files together, or the one it shows: see `ui::look`), at `at`
/// on the screen (`over_the_panel`: as `Menu::open_at` says): up, with what its rows do.
pub fn given_menu(given: &run::Given, at: windows::Win32::Foundation::POINT, over_the_panel: bool, look: crate::ui::menu::Look) -> Option<(crate::ui::menu::Menu, Vec<GivenChoice>)> {
    use crate::ui::icons::Icon;
    use crate::ui::menu::{Item, Mark, Menu};
    let lang = crate::update::language();
    let stays_here = match given {
        run::Given::Item(id) | run::Given::Part(id, _) => run::shown().items.iter().any(|item| item.id == *id && item.stays_here),
        run::Given::Files(_) => false,
    };
    let rows = [
        (GivenChoice::Copy, lang.pick("复制", "Copy"), Icon::Clipboard),
        (GivenChoice::Open, lang.pick("打开", "Open"), Icon::File),
        (GivenChoice::Show, lang.pick("在文件资源管理器中显示", "Show in File Explorer"), Icon::Panel),
    ];
    let mut items: Vec<Item<'static>> = rows.iter().map(|(_, label, icon)| Item::new(*label, Some(Mark::Icon(*icon)), false, false)).collect();
    let mut choices: Vec<GivenChoice> = rows.iter().map(|(choice, ..)| *choice).collect();
    // "发送到" last, its devices chosen after all its rows (see `Menu::sub_choice`).
    let (send, keys) = send::menu_row(stays_here);
    items.push(send);
    choices.push(GivenChoice::Send);
    choices.extend(keys.into_iter().map(GivenChoice::SendTo));
    let menu = Menu::open_at(items, look, at, false, over_the_panel)?;
    Some((menu, choices))
}

/// `choice` made in the menu of `given`, a file the look shows (see `given_menu`): done, on the station's thread.
/// Opened, the look goes, as its own button to open it does.
pub fn given_chose(given: run::Given, choice: GivenChoice) {
    use run::{FilesAct, Given, PartAct};
    let act = |given: Given, files: FilesAct, part: PartAct, item: fn(u64) -> run::Ask| match given {
        Given::Item(id) => item(id),
        Given::Part(id, at) => run::Ask::Part(id, at, part),
        Given::Files(paths) => run::Ask::Files(paths, files),
    };
    run::ask(match choice {
        GivenChoice::Copy => act(given, FilesAct::Copy(None), PartAct::Copy, run::Ask::Copy),
        GivenChoice::Open => {
            crate::ui::look::close();
            act(given, FilesAct::Open, PartAct::Open, run::Ask::Open)
        }
        GivenChoice::Show => act(given, FilesAct::Show, PartAct::Show, run::Ask::Show),
        GivenChoice::SendTo(key) => run::Ask::SendTo(vec![given], key),
        GivenChoice::Send => return,
    });
}

/// `choice` made in the menu of part `part` of the thing `id` (see `part_menu`): done, on the station's thread.
pub fn part_chose(id: u64, part: usize, choice: PartChoice) {
    use run::PartAct;
    run::ask(match choice {
        PartChoice::Copy => run::Ask::Part(id, part, PartAct::Copy),
        PartChoice::Open => run::Ask::Part(id, part, PartAct::Open),
        PartChoice::Show => run::Ask::Part(id, part, PartAct::Show),
        PartChoice::TakeOut => run::Ask::Part(id, part, PartAct::TakeOut),
        PartChoice::Remove => run::Ask::Part(id, part, PartAct::Remove),
        PartChoice::SendTo(key) => run::Ask::SendTo(vec![run::Given::Part(id, part)], key),
        // (Looked at, and renamed where it shows, by what the menu was up on: see `Controller::end_menu`.)
        PartChoice::Preview | PartChoice::Rename | PartChoice::Send => return,
    });
}

/// `choice` made in the menu of the thing `id` on the station (see `menu`): done, on the station's thread.
pub fn chose(id: u64, choice: Choice) {
    let thing = Thing::Item(id);
    run::ask(match choice {
        Choice::Copy => run::Ask::Copy(id),
        Choice::Open => run::Ask::Open(id),
        // (Opened beside what it was chosen on, by that: see `Controller::end_menu`.)
        Choice::Preview => return,
        Choice::Show => run::Ask::Show(id),
        Choice::Pin => return pin(&[thing], !marks(&[thing]).0),
        // (A new one named in its box first, and it renamed in its own, opened by the thread the menu was up on: see
        // `ui::rename`.)
        Choice::Tags | Choice::NewTag | Choice::Rename => return,
        Choice::Tag(tag) => return turn_tag(&[thing], tag),
        Choice::Send => return,
        Choice::SendTo(key) => run::Ask::SendTo(vec![run::Given::Item(id)], key),
        Choice::Split => run::Ask::Split(id),
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
    // (The devices nearby looked for, to rise at the station's foot as it is dragged: see `send`.)
    crate::nearby::want_to_send();
    // (What is carried still, flying back into its room, put back there at once: lifted from there again before it
    // landed, it is drawn as it is in its room, not as the outline of the room it left.)
    crate::carry::put_back(hits);
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
            Hit::Part(id, at) => Some(run::Given::Part(id, at)),
            Hit::Recent(entry) => recent::paths_of(entry).map(run::Given::Files),
            _ => None,
        })
        .collect()
}

/// What the menu of things chosen together does with all of them (see `select`): puts them on the clipboard as one,
/// shows those that are files in Explorer, puts files come lately on the station, pins them or not, tags them, sends
/// them to a device nearby (as `Choice` does), takes them off it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ManyChoice {
    Copy,
    Show,
    Station,
    Pin(bool),
    Tags,
    Tag(u32),
    NewTag,
    Send,
    SendTo(u64),
    Remove,
}

/// What the menu of `hits`, chosen together, offers, as the station holds `items` and the files come lately are `came`:
/// putting them on the clipboard, showing them in Explorer, where any is a file (files come lately are), files come
/// lately put on the station, all pinned (unpinned, where all are), tagged, and sent to a device nearby (the devices: see
/// `many_menu`), and things on the station, or copied lately, taken off.
pub fn many_choices(hits: &[crate::ui::view::Hit], items: &[shelf::Item], came: &[recent::Came]) -> Vec<ManyChoice> {
    use crate::ui::view::Hit;
    let item = |id: u64| items.iter().find(|item| item.id == id);
    let ids: Vec<u64> = hits.iter().filter_map(|hit| if let Hit::Shelf(id) | Hit::Copied(id) = hit { Some(*id) } else { None }).collect();
    let recent = hits.iter().any(|hit| matches!(hit, Hit::Recent(_)));
    let parts = hits.iter().any(|hit| matches!(hit, Hit::Part(..)));
    let files = recent || parts || ids.iter().filter_map(|id| item(*id)).any(|item| matches!(item.what, shelf::What::Files { .. } | shelf::What::Picture { .. } | shelf::What::Saved { words: false, .. } | shelf::What::Came { .. }));
    let mut choices = vec![ManyChoice::Copy];
    if files {
        choices.push(ManyChoice::Show);
    }
    if recent {
        choices.push(ManyChoice::Station);
    }
    let things: Vec<Thing> = hits.iter().filter_map(|hit| Thing::of(*hit)).collect();
    choices.push(ManyChoice::Pin(!marks_among(&things, items, came).0));
    choices.extend([ManyChoice::Tags, ManyChoice::Send]);
    if !ids.is_empty() || parts {
        choices.push(ManyChoice::Remove);
    }
    choices
}

/// The menu of `hits`, chosen together (a right click on one of them), at `at` on the screen (`over_the_panel`: as
/// `Menu::open_at` says): up, with what its rows do (see `many_choices`).
pub fn many_menu(hits: &[crate::ui::view::Hit], at: windows::Win32::Foundation::POINT, over_the_panel: bool, look: crate::ui::menu::Look) -> Option<(crate::ui::menu::Menu, Vec<ManyChoice>)> {
    use crate::ui::icons::Icon;
    use crate::ui::menu::{Item, Menu};
    use crate::ui::view::Hit;
    let lang = crate::update::language();
    let shown = run::shown();
    let items_held = &shown.items;
    let mut choices = many_choices(hits, items_held, &recent::shown());
    // (Sent all together: none where one of them may not leave this PC.)
    let stays_here = hits.iter().any(|hit| matches!(hit, Hit::Shelf(id) | Hit::Copied(id) | Hit::Part(id, _) if items_held.iter().any(|item| item.id == *id && item.stays_here)));
    let things: Vec<Thing> = hits.iter().filter_map(|hit| Thing::of(*hit)).collect();
    let (tags, tagged) = tags_row(&things, ManyChoice::Tag, ManyChoice::NewTag);
    let (send, keys) = send::menu_row(stays_here);
    let row = |label: &'static str, icon: Icon, rule_before: bool| Item::new(label, Some(crate::ui::menu::Mark::Icon(icon)), false, rule_before);
    let items: Vec<Item<'static>> = choices
        .iter()
        .map(|choice| match choice {
            ManyChoice::Copy => row(lang.pick("全部复制", "Copy all"), Icon::Clipboard, false),
            ManyChoice::Show => row(lang.pick("在文件资源管理器中显示", "Show in File Explorer"), Icon::Panel, false),
            ManyChoice::Station => row(lang.pick("全部放入中转站", "Add all to Station"), Icon::Inbox, false),
            ManyChoice::Pin(true) => row(lang.pick("全部固定", "Pin all"), Icon::Pin, false),
            ManyChoice::Pin(false) => row(lang.pick("全部取消固定", "Unpin all"), Icon::Pin, false),
            ManyChoice::Tags | ManyChoice::Tag(_) | ManyChoice::NewTag => tags.clone(),
            ManyChoice::Send | ManyChoice::SendTo(_) => send.clone(),
            ManyChoice::Remove => row(lang.pick("全部移除", "Remove all"), Icon::Trash, true),
        })
        .collect();
    // (Its tags and its devices chosen after all its rows: see `Menu::sub_choice`.)
    choices.extend(tagged);
    choices.extend(keys.into_iter().map(ManyChoice::SendTo));
    let menu = Menu::open_at(items, look, at, false, over_the_panel)?;
    Some((menu, choices))
}

/// `choice` made in the menu of `hits`, chosen together (see `many_menu`): done for all of them. Taken off, nothing is
/// chosen any more.
pub fn many_chose(hits: &[crate::ui::view::Hit], choice: ManyChoice) {
    use crate::ui::view::Hit;
    let ids = || hits.iter().filter_map(|hit| if let Hit::Shelf(id) | Hit::Copied(id) = hit { Some(*id) } else { None });
    let things = || hits.iter().filter_map(|hit| Thing::of(*hit)).collect::<Vec<Thing>>();
    match choice {
        ManyChoice::Copy => run::ask(run::Ask::Together(given(hits), run::Together::Copy)),
        ManyChoice::Show => run::ask(run::Ask::Together(given(hits), run::Together::Show)),
        // (Each its own thing on the station, as each is put there from its own menu.)
        ManyChoice::Station => {
            for paths in hits.iter().filter_map(|hit| if let Hit::Recent(entry) = hit { recent::paths_of(*entry) } else { None }) {
                run::ask(run::Ask::Take(clipboard::Content::Files(paths)));
            }
        }
        ManyChoice::Pin(pinned) => pin(&things(), pinned),
        ManyChoice::Tags | ManyChoice::NewTag => {}
        ManyChoice::Tag(tag) => turn_tag(&things(), tag),
        ManyChoice::Send => {}
        ManyChoice::SendTo(key) => run::ask(run::Ask::SendTo(given(hits), key)),
        ManyChoice::Remove => {
            // (Parts of one, the last first: each taken off leaves those before it where they were.)
            let mut parts: Vec<(u64, usize)> = hits.iter().filter_map(|hit| if let Hit::Part(id, at) = hit { Some((*id, *at)) } else { None }).collect();
            parts.sort_by(|one, other| other.cmp(one));
            parts.into_iter().for_each(|(id, at)| run::ask(run::Ask::Part(id, at, run::PartAct::Remove)));
            ids().for_each(|id| run::ask(run::Ask::Remove(id)));
            select::change(select::Chosen::clear);
        }
    }
}

/// What a file come lately's menu does: opens it, shows it in Explorer, puts it on the station, copies its path, pins it
/// (or not), tags it and sends it to a device nearby (as `Choice` does), lists no more of its folder's files. (Files
/// come together: their folder opened showing them, all put on the station, the folder's path copied, all pinned, tagged
/// and sent.)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecentChoice {
    Open,
    Show,
    Station,
    CopyPath,
    Rename,
    Pin,
    Tags,
    Tag(u32),
    NewTag,
    Send,
    SendTo(u64),
    Hide,
}

/// The menu of `entry` among the files come lately (a right click on it), at `at` on the screen: up, with what its
/// rows do.
pub fn recent_menu(entry: recent::Entry, at: windows::Win32::Foundation::POINT, over_the_panel: bool, look: crate::ui::menu::Look) -> Option<(crate::ui::menu::Menu, Vec<RecentChoice>)> {
    use crate::ui::icons::Icon;
    use crate::ui::menu::{Item, Menu};
    recent::paths_of(entry)?;
    let lang = crate::update::language();
    let thing = Thing::Recent(entry);
    let mut rows: Vec<(RecentChoice, &str, Icon, bool)> = match entry {
        recent::Entry::File(_) => vec![
            (RecentChoice::Open, lang.pick("打开", "Open"), Icon::File, false),
            (RecentChoice::Show, lang.pick("在文件资源管理器中显示", "Show in File Explorer"), Icon::Panel, false),
            (RecentChoice::Station, lang.pick("放入中转站", "Add to Station"), Icon::Inbox, false),
            (RecentChoice::CopyPath, lang.pick("复制路径", "Copy path"), Icon::Clipboard, false),
            (RecentChoice::Rename, lang.pick("重命名", "Rename"), Icon::SquarePen, false),
        ],
        recent::Entry::Burst(..) => vec![
            (RecentChoice::Show, lang.pick("打开所在文件夹", "Open containing folder"), Icon::Panel, false),
            (RecentChoice::Station, lang.pick("全部放入中转站", "Add all to Station"), Icon::Inbox, false),
            (RecentChoice::CopyPath, lang.pick("复制文件夹路径", "Copy folder path"), Icon::Clipboard, false),
        ],
    };
    rows.push((RecentChoice::Pin, if marks(&[thing]).0 { lang.pick("取消固定", "Unpin") } else { lang.pick("固定", "Pin") }, Icon::Pin, false));
    rows.push((RecentChoice::Hide, lang.pick("不再列出这个文件夹", "Stop listing this folder"), Icon::EyeOff, true));
    let mut items: Vec<Item<'static>> = rows.iter().map(|(_, label, icon, rule_before)| Item::new(*label, Some(crate::ui::menu::Mark::Icon(*icon)), false, *rule_before)).collect();
    let mut choices: Vec<RecentChoice> = rows.into_iter().map(|(choice, ..)| choice).collect();
    // "标签" and "发送到" before the rule, their tags and devices chosen after all its rows (see `Menu::sub_choice`).
    let (tags, tagged) = tags_row(&[thing], RecentChoice::Tag, RecentChoice::NewTag);
    items.insert(items.len() - 1, tags);
    choices.insert(choices.len() - 1, RecentChoice::Tags);
    let (send, keys) = send::menu_row(false);
    items.insert(items.len() - 1, send);
    choices.insert(choices.len() - 1, RecentChoice::Send);
    choices.extend(tagged);
    choices.extend(keys.into_iter().map(RecentChoice::SendTo));
    let menu = Menu::open_at(items, look, at, false, over_the_panel)?;
    Some((menu, choices))
}

/// `choice` made in the menu of `entry` among the files come lately (see `recent_menu`): done.
pub fn recent_chose(entry: recent::Entry, choice: RecentChoice) {
    let Some(paths) = recent::paths_of(entry) else { return };
    let thing = Thing::Recent(entry);
    // (The files come together are in one folder.)
    let folder = paths.first().and_then(|path| path.parent()).map(std::path::Path::to_path_buf);
    match choice {
        RecentChoice::Open => run::ask(run::Ask::Files(paths, run::FilesAct::Open)),
        RecentChoice::Show => run::ask(run::Ask::Files(paths, run::FilesAct::Show)),
        RecentChoice::Station => run::ask(run::Ask::Take(clipboard::Content::Files(paths))),
        RecentChoice::Pin => pin(&[thing], !marks(&[thing]).0),
        RecentChoice::Tag(tag) => turn_tag(&[thing], tag),
        // (A new one named in its box first, opened by the thread the menu was up on: see `ui::rename`.)
        RecentChoice::Tags | RecentChoice::NewTag | RecentChoice::Rename | RecentChoice::Send => {}
        RecentChoice::SendTo(key) => run::ask(run::Ask::SendTo(vec![run::Given::Files(paths)], key)),
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

/// What the menu of a thing kept at hand does: opens it, shows where it is in Explorer, renames it (in quick launch
/// alone: see `ui::rename`), gives it its own name again, takes it off.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AppChoice {
    Open,
    Show,
    Rename,
    Restore,
    Remove,
}

/// The rows of the menu of `app`, kept at hand: what each does, its words, its mark, whether a rule sets it off. Where
/// it is is offered only for what is on the disk, its own name again only where it was given another.
fn app_rows(app: &apps::App, lang: crate::ui::text::Lang) -> Vec<(AppChoice, &'static str, crate::ui::icons::Icon, bool)> {
    use crate::ui::icons::Icon;
    let mut rows = vec![(AppChoice::Open, lang.pick("打开", "Open"), Icon::File, false)];
    if app.location().is_some() {
        rows.push((AppChoice::Show, lang.pick("打开文件所在的位置", "Open file location"), Icon::Panel, false));
    }
    rows.push((AppChoice::Rename, lang.pick("重命名", "Rename"), Icon::SquarePen, false));
    if app.alias.is_some() {
        rows.push((AppChoice::Restore, lang.pick("恢复原名", "Restore original name"), Icon::Restore, false));
    }
    rows.push((AppChoice::Remove, lang.pick("从快捷启动移除", "Remove from Quick launch"), Icon::PinOff, true));
    rows
}

/// The menu of the thing kept at hand at `at` among them (a right click on it), at `point` on the screen: up, with
/// what its rows do (where it is is shown only for what is on the disk).
pub fn app_menu(number: u64, point: windows::Win32::Foundation::POINT, over_the_panel: bool, look: crate::ui::menu::Look) -> Option<(crate::ui::menu::Menu, Vec<AppChoice>, apps::App)> {
    use crate::ui::menu::{Item, Menu};
    let app = apps::numbered(number)?;
    let rows = app_rows(&app, crate::update::language());
    let items = rows.iter().map(|(_, label, icon, rule_before)| Item::new(*label, Some(crate::ui::menu::Mark::Icon(*icon)), false, *rule_before)).collect();
    let menu = Menu::open_at(items, look, point, false, over_the_panel)?;
    Some((menu, rows.into_iter().map(|(choice, ..)| choice).collect(), app))
}

/// `choice` made in the menu of `app`, kept at hand (see `app_menu`: the one its menu was opened on, whatever is
/// kept now): done. (Renaming it opens a box where its menu was, opened by the thread the menu was up on: see
/// `ui::rename`.)
pub fn app_chose(app: apps::App, choice: AppChoice) {
    match choice {
        AppChoice::Open => run::ask(run::Ask::Launch(app.target)),
        AppChoice::Show => {
            if let Some(path) = app.location() {
                run::ask(run::Ask::Files(vec![path], run::FilesAct::Show));
            }
        }
        AppChoice::Rename => {}
        AppChoice::Restore => apps::rename(app, None),
        AppChoice::Remove => apps::change(move |apps| apps.retain(|kept| *kept != app)),
    }
}

/// What a row of the station's head's menus does: shows what is on it so; takes off what does not stay, of what is on it
/// or of what was copied lately; keeps no more of what is copied; opens the settings.
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
        let mut pages = vec![(Tab::Shelf(Default::default()).again(tab), lang.pick("中转站", "Station"), Icon::Inbox)];
        if shown.history {
            pages.push((Tab::Copies(Default::default()).again(tab), lang.pick("最近复制", "Copied"), Icon::Clipboard));
        }
        if recent::on() {
            pages.push((Tab::Recent(Default::default()).again(tab), lang.pick("最近文件", "Recent"), Icon::File));
        }
        let items = pages.iter().map(|(page, label, icon)| Item::new(*label, Some(crate::ui::menu::Mark::Icon(*icon)), page.page() == tab.page(), false)).collect();
        let menu = Menu::open_at(items, look, at, false, over_the_panel)?;
        return Some((menu, pages.into_iter().map(|(page, ..)| HeadChoice::Page(page)).collect()));
    }
    if hit != Hit::StationView {
        return None;
    }
    // Each row: what it does, its name, its icon, whether it is ticked, and whether a rule sets it off. How the page
    // shows (each page its own way); under a rule, what else is done with it; the settings last.
    let (page, view) = match tab {
        Tab::Shelf(_) => (crate::settings::StationPage::Shelf, view),
        Tab::Copies(_) => (crate::settings::StationPage::Copies, run::shown().copies_view),
        Tab::Recent(_) => (crate::settings::StationPage::Recent, run::shown().recent_view),
    };
    let mut rows: Vec<(HeadChoice, &'static str, Icon, bool, bool)> = [(ShelfView::Cards, lang.pick("小卡片", "Small cards"), Icon::LayoutGrid), (ShelfView::Large, lang.pick("大卡片", "Large cards"), Icon::Image), (ShelfView::List, lang.pick("列表", "List"), Icon::List)]
        .into_iter()
        .map(|(shows, label, icon)| (HeadChoice::View(page, shows), label, icon, shows == view, false))
        .collect();
    rows.extend(match tab {
        Tab::Shelf(_) => vec![(HeadChoice::Clear, lang.pick("全部清空", "Clear all"), Icon::Trash, false, true)],
        Tab::Recent(_) => vec![(HeadChoice::StopRecent, lang.pick("不再列出最近文件", "Stop listing recent files"), Icon::EyeOff, false, true)],
        Tab::Copies(_) => vec![
            (HeadChoice::ClearCopies, lang.pick("清空最近复制", "Clear copied items"), Icon::Trash, false, true),
            (HeadChoice::StopCopies, lang.pick("不再记录复制的内容", "Stop recording copied items"), Icon::Clipboard, false, false),
        ],
    });
    rows.push((HeadChoice::Settings, lang.pick("中转站设置", "Station settings"), Icon::Settings, false, true));
    let items = rows.iter().map(|(_, label, icon, checked, rule_before)| Item::new(*label, Some(crate::ui::menu::Mark::Icon(*icon)), *checked, *rule_before)).collect();
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

/// Now, in ms since 1970 (as the shelf keeps when things were put on it). (The studio's: as it films, see `pretend_now`.)
pub fn now_ms() -> u64 {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    #[cfg(feature = "studio")]
    let now = now.saturating_add_signed(SHIFT.load(std::sync::atomic::Ordering::Relaxed));
    now
}

/// How far the studio's moment is from the clock's (ms).
#[cfg(feature = "studio")]
static SHIFT: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// Now is `ms` (since 1970) from here on, the clock going on from it: what the studio films is shown at the moment it
/// says, not at the moment it is filmed.
#[cfg(feature = "studio")]
pub fn pretend_now(ms: u64) {
    let clock = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
    SHIFT.store(ms as i64 - clock, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::view::Hit;

    /// A new tag is named as Windows names a new folder: "新建标签", or where a tag has that name, the first of
    /// "新建标签 2", "新建标签 3" and on that none has.
    #[test]
    fn a_new_tag_is_named_as_a_new_folder_is() {
        let tags = |names: &[&str]| names.iter().enumerate().map(|(at, name)| shelf::Tag { id: at as u32, name: (*name).into() }).collect::<Vec<_>>();
        let zh = crate::ui::text::Lang::Zh;
        assert_eq!(new_tag_name(&tags(&[]), zh), "新建标签");
        assert_eq!(new_tag_name(&tags(&["工作"]), zh), "新建标签");
        assert_eq!(new_tag_name(&tags(&["新建标签"]), zh), "新建标签 2");
        assert_eq!(new_tag_name(&tags(&["新建标签", "新建标签 2", "新建标签 4"]), zh), "新建标签 3");
        assert_eq!(new_tag_name(&tags(&["New tag"]), crate::ui::text::Lang::En), "New tag 2");
    }

    /// The menu of a thing kept at hand offers to rename it, and to give it its own name again only where it was given
    /// another. (It offered neither.)
    #[test]
    fn a_thing_kept_at_hand_is_offered_its_renaming() {
        let mut app = apps::App { target: "https://example.com/".into(), name: "example.com".into(), alias: None };
        let choices = |app: &apps::App| app_rows(app, crate::ui::text::Lang::Zh).into_iter().map(|(choice, label, ..)| (choice, label)).collect::<Vec<_>>();
        assert_eq!(choices(&app), [(AppChoice::Open, "打开"), (AppChoice::Rename, "重命名"), (AppChoice::Remove, "从快捷启动移除")]);
        app.alias = Some("样例".into());
        assert_eq!(choices(&app), [(AppChoice::Open, "打开"), (AppChoice::Rename, "重命名"), (AppChoice::Restore, "恢复原名"), (AppChoice::Remove, "从快捷启动移除")]);
    }

    /// The menu of things chosen together offers what may be done with all of them: put on the clipboard ever, shown
    /// in Explorer where any is a file, files come lately put on the station, all pinned (unpinned where all are) and
    /// tagged, and things on the station taken off.
    #[test]
    fn the_menu_of_things_chosen_together_offers_what_all_may_do() {
        use shelf::{Item, What};
        let item = |id: u64, what: What, pinned: bool| Item { id, what, added_ms: id, pinned, copied: false, stays_here: false, own_bytes: 0, from: None, used_ms: 0, tags: Vec::new(), held: false, freed_ms: 0 };
        let items = vec![
            item(1, What::Text { text: "a".into() }, true),
            item(2, What::Link { url: "https://a.b".into() }, false),
            item(3, What::Picture { file: "3.png".into(), name: String::new() }, true),
            item(4, What::Saved { file: "4".into(), name: "a.txt".into(), words: true }, false),
        ];
        let choices = |hits: &[Hit], files: &[recent::Came]| many_choices(hits, &items, files);
        assert_eq!(choices(&[Hit::Shelf(1), Hit::Shelf(2)], &[]), [ManyChoice::Copy, ManyChoice::Pin(true), ManyChoice::Tags, ManyChoice::Send, ManyChoice::Remove]);
        assert_eq!(choices(&[Hit::Shelf(1), Hit::Copied(4)], &[]), [ManyChoice::Copy, ManyChoice::Pin(true), ManyChoice::Tags, ManyChoice::Send, ManyChoice::Remove]);
        assert_eq!(choices(&[Hit::Shelf(1), Hit::Shelf(3)], &[]), [ManyChoice::Copy, ManyChoice::Show, ManyChoice::Pin(false), ManyChoice::Tags, ManyChoice::Send, ManyChoice::Remove]);
        let key = |file: u128| recent::Key { drive: 'C', file };
        let hits = [1, 2].map(|file| Hit::Recent(recent::Entry::File(key(file))));
        let mut files: Vec<recent::Came> = [1, 2].map(|file| recent::Came::of(key(file), format!(r"C:\{file}.pdf").into(), 1, 1, recent::Kind::Document)).into();
        assert_eq!(choices(&hits, &files), [ManyChoice::Copy, ManyChoice::Show, ManyChoice::Station, ManyChoice::Pin(true), ManyChoice::Tags, ManyChoice::Send]);
        files.iter_mut().for_each(|came| came.marks.pinned = true);
        assert!(choices(&hits, &files).contains(&ManyChoice::Pin(false)));
    }

    /// Things marked together: pinned where all are, of the tags all have, files come together each of its own. Those
    /// gone are not counted.
    #[test]
    fn things_are_marked_as_all_of_them_are() {
        use shelf::{Item, What};
        let item = |id: u64, pinned: bool, tags: &[u32]| Item { id, what: What::Text { text: "a".into() }, added_ms: id, pinned, copied: false, stays_here: false, own_bytes: 0, from: None, used_ms: 0, tags: tags.to_vec(), held: false, freed_ms: 0 };
        let items = [item(1, true, &[1, 2]), item(2, true, &[2, 3]), item(3, false, &[2])];
        let mut files: Vec<recent::Came> = (1..=5).rev().map(|file| recent::Came::of(recent::Key { drive: 'D', file }, format!(r"D:\Phone\{file}.jpg").into(), file as u64, 1, recent::Kind::Picture)).collect();
        files.iter_mut().for_each(|came| came.marks = recent::Marks { pinned: true, tags: vec![2], freed_ms: 0 });
        let burst = Thing::Recent(recent::Entry::Burst(recent::Filter::Pinned, files[0].key));
        let marks = |things: &[Thing], files: &[recent::Came]| marks_among(things, &items, files);
        assert_eq!(marks(&[Thing::Item(1), Thing::Item(2), Thing::Item(9)], &files), (true, vec![2]));
        assert_eq!(marks(&[Thing::Item(1), Thing::Item(3)], &files), (false, vec![2]));
        assert_eq!(marks(&[Thing::Item(1), burst], &files), (true, vec![2]));
        files[3].marks.tags.clear();
        assert_eq!(marks(&[burst], &files), (true, vec![]));
        assert_eq!(marks(&[Thing::Item(9)], &files), (false, vec![]));
    }

    /// A page is sorted by the tags its own things have, those nothing has anywhere, and the one it shows.
    #[test]
    fn a_page_shows_its_own_tags() {
        let tags: Vec<shelf::Tag> = (1..=4).map(|id| shelf::Tag { id, name: id.to_string() }).collect();
        let mut file = recent::Came::of(recent::Key { drive: 'D', file: 1 }, r"D:\a.pdf".into(), 1, 1, recent::Kind::Document);
        file.marks.tags = vec![2];
        let item = shelf::Item { id: 1, what: shelf::What::Text { text: "a".into() }, added_ms: 1, pinned: false, copied: true, stays_here: false, own_bytes: 0, from: None, used_ms: 0, tags: vec![1, 4], held: false, freed_ms: 0 };
        let shown = |here: &[u32], lit: Option<u32>| tags_of(&tags, |tag| here.contains(&tag), lit, &[item.clone()], &[file.clone()]).iter().map(|tag| tag.id).collect::<Vec<_>>();
        assert_eq!(shown(&[1, 4], None), [1, 3, 4]);
        assert_eq!(shown(&[2], None), [2, 3]);
        assert_eq!(shown(&[], Some(4)), [3, 4]);
    }
}
