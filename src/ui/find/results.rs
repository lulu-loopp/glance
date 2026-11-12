//! What search shows for what is typed, in the view chosen: the programs, Settings' pages and the system's tools, the
//! folders and files, and Glance's own (notes, what is on the station, what was copied lately) whose names meet it,
//! the best of all first, then each kind under its heading, a few of each. A view shows one kind alone, as many as
//! fit. Nothing typed, it shows what came lately (the files, Recent files'), the programs (A to Z), or Glance's own.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::pages::{Page, PAGES};
use crate::notes::Note;
use crate::search::{Only, Query};
use crate::station::found::Found as Program;
use crate::station::recent::{self, Came, Kind as FileKind};
use crate::station::shelf::{Item, What as Held};
use crate::ui::text::Lang;

/// Which names a view shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum View {
    #[default]
    All,
    Programs,
    Folders,
    Files,
    Pictures,
    Documents,
    Media,
    Glance,
}

impl View {
    pub const EVERY: [View; 8] = [View::All, View::Programs, View::Folders, View::Files, View::Pictures, View::Documents, View::Media, View::Glance];

    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            View::All => lang.pick("全部", "All"),
            View::Programs => lang.pick("应用和设置", "Apps & settings"),
            View::Folders => lang.pick("文件夹", "Folders"),
            View::Files => lang.pick("文件", "Files"),
            View::Pictures => lang.pick("图片", "Pictures"),
            View::Documents => lang.pick("文档", "Documents"),
            View::Media => lang.pick("音乐和视频", "Music & videos"),
            View::Glance => "Glance",
        }
    }

    /// The query for the files and folders it shows (none for a view of other things).
    fn files(self, query: Query) -> Option<Query> {
        let media: Vec<&str> = recent::AUDIO.iter().chain(recent::VIDEOS).copied().collect();
        match self {
            View::All => Some(query),
            View::Folders => Some(query.only(Only::Folders)),
            View::Files => Some(query.only(Only::Files)),
            View::Pictures => Some(query.allowing(recent::PICTURES)),
            View::Documents => Some(query.allowing(recent::DOCUMENTS)),
            View::Media => Some(query.allowing(&media)),
            View::Programs | View::Glance => None,
        }
    }

    /// Whether it shows a file of `kind` that came lately.
    fn takes(self, kind: FileKind) -> bool {
        match self {
            View::All | View::Files => true,
            View::Pictures => matches!(kind, FileKind::Picture | FileKind::Screenshot),
            View::Documents => kind == FileKind::Document,
            View::Media => matches!(kind, FileKind::Audio | FileKind::Video),
            View::Programs | View::Folders | View::Glance => false,
        }
    }
}

/// What a name found is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Program,
    Page,
    Tool,
    Folder,
    File(FileKind),
    Note,
    /// On the station, put there by hand.
    Held,
    /// Copied lately (the station's Recently copied).
    Copied,
    /// What was typed after "待办 ", to be added as a to-do.
    Todo,
}

/// What opens it.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum What {
    /// A program, as the shell opens it (`shell:AppsFolder\…`).
    Program(String),
    /// A page of Settings, or a tool, by its place in `PAGES`.
    Page(usize),
    Path(PathBuf),
    /// A note, by its file's name.
    Note(String),
    /// A thing on the station, or copied lately, by its number.
    Item(u64),
    /// A to-do to add, its words as typed (see `todo_row`).
    Todo(String),
}

/// A name found: what it is, its name and where its words met it (characters of the name), what is said under it
/// (where it is), what opens it, how well it meets what was typed, and for a file or folder its size and when it was
/// last changed.
#[derive(Clone, Debug)]
pub struct Row {
    pub kind: Kind,
    pub name: String,
    pub marks: Vec<(usize, usize)>,
    pub detail: String,
    pub what: What,
    pub rank: f32,
    pub bytes: Option<u64>,
    pub changed: Option<SystemTime>,
}

/// A heading over names of a kind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    Programs,
    Pages,
    Folders,
    Files,
    Glance,
    /// What came lately (nothing typed).
    Lately,
}

impl Group {
    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            Group::Programs => lang.pick("应用", "Apps"),
            Group::Pages => lang.pick("设置", "Settings"),
            Group::Folders => lang.pick("文件夹", "Folders"),
            Group::Files => lang.pick("文件", "Files"),
            Group::Glance => "Glance",
            Group::Lately => lang.pick("最近的文件", "Recent files"),
        }
    }
}

/// More of a kind than is shown under its heading: how many more, or (files) all of them to be shown, how many.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum More {
    No,
    Others(usize),
    ShowAll(usize),
}

/// A line of what is shown: a heading, a name (the best of all, shown larger, or another), or, ending a view showing
/// fewer than it found, how many more there are.
#[derive(Clone, Debug)]
pub enum Line {
    Head(Group, More),
    Row(Row, bool),
    Others(usize),
}

/// What is shown, and how many names were found in all.
#[derive(Clone, Debug, Default)]
pub struct Shown {
    pub lines: Vec<Line>,
    pub total: usize,
}

impl Shown {
    /// Its names, in order.
    pub fn rows(&self) -> impl Iterator<Item = &Row> {
        self.lines.iter().filter_map(|line| if let Line::Row(row, _) = line { Some(row) } else { None })
    }

    /// The files and folders among its names (the paths, and whether each is a folder), to be told of (see `told`).
    pub fn paths(&self) -> Vec<(PathBuf, bool)> {
        self.rows().filter_map(|row| if let What::Path(path) = &row.what { Some((path.clone(), row.kind == Kind::Folder)) } else { None }).collect()
    }

    /// Its files and folders told of as `was` told of them.
    pub fn told_as(&mut self, was: &Shown) {
        for row in was.rows() {
            if let (What::Path(path), Some(changed)) = (&row.what, row.changed) {
                self.told(path, (row.bytes.unwrap_or_default(), changed));
            }
        }
    }

    /// The file or folder at `path` told of: its size (a file's) and when it changed, as they are now.
    pub fn told(&mut self, path: &Path, (bytes, changed): (u64, SystemTime)) {
        for line in &mut self.lines {
            if let Line::Row(row, _) = line {
                if matches!(&row.what, What::Path(at) if at == path) {
                    row.bytes = (row.kind != Kind::Folder).then_some(bytes);
                    row.changed = Some(changed);
                }
            }
        }
    }
}

/// Where names are found: the Start menu's programs, the notes, what the station holds, the files that came lately,
/// the drives' files and folders (as `search::find` finds them), in the language the user reads.
pub struct Sources<'a> {
    pub programs: &'a [Program],
    pub notes: &'a [Note],
    pub held: &'a [Item],
    pub lately: &'a [Came],
    pub files: &'a dyn Fn(&Query, usize) -> crate::search::Found,
    pub lang: Lang,
}

/// How many of each kind the view of all shows under its heading; how many files once all are asked for; how many a
/// view of one kind shows; how many files that came lately the view of all shows.
const PROGRAMS: usize = 3;
const PAGES_SHOWN: usize = 2;
const FOLDERS: usize = 3;
const FILES: usize = 6;
const FILES_ALL: usize = 40;
const GLANCE: usize = 3;
const LISTED: usize = 60;
const LATELY: usize = 8;

/// What a program's and a page's names count for besides how well they meet what was typed: what is looked for by a
/// name of a few letters is most often a program, or a setting, more than a file of that name.
const PROGRAM_FIRST: f32 = 1.0;
const PAGE_FIRST: f32 = 0.5;

/// What is shown for `typed` in `view` (the files all, `expanded`), from `sources`.
pub fn gather(typed: &str, view: View, expanded: bool, sources: &Sources) -> Shown {
    let mut shown = found(typed, view, expanded, sources);
    // Typed after "待办 ": first of all, larger, what adds it as a to-do.
    if let Some(row) = todo_row(typed, sources.lang) {
        for line in shown.lines.iter_mut() {
            if let Line::Row(_, best) = line {
                *best = false;
            }
        }
        shown.lines.insert(0, Line::Row(row, true));
        shown.total += 1;
    }
    shown
}

/// The words typed after what says they are a to-do ("待办 ", "todo "), where they are: as a row that adds them, what
/// they say of a day and a time said under it.
pub fn todo_row(typed: &str, lang: Lang) -> Option<Row> {
    let typed = typed.trim_start();
    let rest = ["待办 ", "待办：", "待办:", "todo ", "to-do "].iter().find_map(|lead| {
        let head = typed.get(..lead.len())?;
        head.eq_ignore_ascii_case(lead).then(|| &typed[lead.len()..])
    })?;
    let rest = rest.trim();
    let (words, due) = crate::todo::words(rest, crate::todo::Day::today());
    if words.trim().is_empty() {
        return None;
    }
    let name = format!("{}{words}", lang.pick("添加为待办：", "Add as to-do: "));
    let detail = due.map_or_else(|| lang.pick("待办", "To-do").to_string(), |due| due.label(crate::todo::Day::today(), lang == Lang::Zh));
    Some(Row { kind: Kind::Todo, name, marks: Vec::new(), detail, what: What::Todo(rest.to_string()), rank: f32::MAX, bytes: None, changed: None })
}

/// What was found of what was typed (see `gather`).
fn found(typed: &str, view: View, expanded: bool, sources: &Sources) -> Shown {
    let query = Query::read(typed);
    if query.is_empty() {
        return lately(view, sources);
    }
    let lang = sources.lang;
    let mut programs: Vec<Row> = Vec::new();
    let mut pages: Vec<Row> = Vec::new();
    let mut glance: Vec<Row> = Vec::new();
    if matches!(view, View::All | View::Programs) {
        programs = sources.programs.iter().filter_map(|program| program_row(program, &query, lang)).collect();
        pages = PAGES.iter().enumerate().filter_map(|(at, page)| page_row(at, page, &query, lang)).collect();
    }
    if matches!(view, View::All | View::Glance) {
        glance.extend(sources.notes.iter().filter_map(|note| note_row(note, &query, lang)));
        glance.extend(sources.held.iter().filter_map(|item| held_row(item, &query, lang)));
    }
    let most = match view {
        View::All if expanded => FILES_ALL,
        View::All => FILES.max(FOLDERS),
        _ => LISTED,
    };
    let found = view.files(query).map(|query| (sources.files)(&query, most)).unwrap_or_default();
    let (mut folders, mut files): (Vec<Row>, Vec<Row>) = found.hits.into_iter().map(file_row).partition(|row| row.kind == Kind::Folder);
    for rows in [&mut programs, &mut pages, &mut glance, &mut folders, &mut files] {
        rows.sort_by(|a, b| b.rank.total_cmp(&a.rank));
    }
    let total = programs.len() + pages.len() + glance.len() + found.total;
    if view != View::All {
        // One kind: as many as fit, a program or page first shown larger.
        let mut rows: Vec<Row> = programs.into_iter().chain(pages).chain(glance).chain(folders).chain(files).collect();
        rows.sort_by(|a, b| b.rank.total_cmp(&a.rank));
        let others = total.saturating_sub(LISTED.min(rows.len()));
        rows.truncate(LISTED);
        let mut lines: Vec<Line> = rows.into_iter().enumerate().map(|(at, row)| { let best = at == 0 && matches!(row.kind, Kind::Program | Kind::Page | Kind::Tool); Line::Row(row, best) }).collect();
        if others > 0 {
            lines.push(Line::Others(others));
        }
        return Shown { lines, total };
    }
    // The best of all first, larger, then each kind under its heading (how many of each were found, the best one too).
    let sizes = [programs.len(), pages.len(), found.folders, found.total - found.folders, glance.len()];
    let mut groups = [programs, pages, folders, files, glance];
    let best = groups.iter().enumerate().filter_map(|(at, rows)| rows.first().map(|row| (at, row.rank))).max_by(|a, b| a.1.total_cmp(&b.1)).map(|(at, _)| at);
    let mut lines = Vec::new();
    if let Some(at) = best {
        lines.push(Line::Row(groups[at].remove(0), true));
    }
    let heads = [(Group::Programs, PROGRAMS), (Group::Pages, PAGES_SHOWN), (Group::Folders, FOLDERS), (Group::Files, if expanded { FILES_ALL } else { FILES }), (Group::Glance, GLANCE)];
    for (at, ((group, most), rows)) in heads.into_iter().zip(groups).enumerate() {
        if rows.is_empty() {
            continue;
        }
        let left = sizes[at] - usize::from(best == Some(at));
        let shown = most.min(rows.len());
        let more = match left - shown {
            0 => More::No,
            _ if group == Group::Files && !expanded => More::ShowAll(sizes[at]),
            more => More::Others(more),
        };
        lines.push(Line::Head(group, more));
        lines.extend(rows.into_iter().take(shown).map(|row| Line::Row(row, false)));
    }
    Shown { lines, total }
}

/// What is shown with nothing typed: the files that came lately (of the kinds the view shows), newest first; the
/// programs, A to Z; Glance's own, the newest first.
fn lately(view: View, sources: &Sources) -> Shown {
    let lang = sources.lang;
    let (group, rows): (Group, Vec<Row>) = match view {
        View::Programs => {
            let mut programs: Vec<&Program> = sources.programs.iter().collect();
            programs.sort_by_cached_key(|program| program.name.to_lowercase());
            (Group::Programs, programs.into_iter().map(|program| program_shown(program, 0.0, Vec::new(), lang)).collect())
        }
        View::Glance => {
            let mut notes: Vec<&Note> = sources.notes.iter().collect();
            notes.sort_by(|a, b| b.stamp.written_ms.cmp(&a.stamp.written_ms));
            let mut held: Vec<&Item> = sources.held.iter().collect();
            held.sort_by(|a, b| b.added_ms.cmp(&a.added_ms));
            let rows = notes.into_iter().map(|note| note_shown(note, 0.0, Vec::new(), lang)).chain(held.into_iter().map(|item| held_shown(item, 0.0, Vec::new(), lang))).collect();
            (Group::Glance, rows)
        }
        View::Folders => (Group::Lately, Vec::new()),
        _ => {
            let mut came: Vec<&Came> = sources.lately.iter().filter(|came| view.takes(came.kind)).collect();
            came.sort_by(|a, b| b.came_ms.cmp(&a.came_ms));
            came.truncate(if view == View::All { LATELY } else { LISTED });
            let rows = came.into_iter().map(|came| path_shown(came.path.clone(), false, 0.0, Vec::new())).collect();
            (Group::Lately, rows)
        }
    };
    let total = rows.len();
    if rows.is_empty() {
        return Shown::default();
    }
    let mut lines = vec![Line::Head(group, More::No)];
    lines.extend(rows.into_iter().take(LISTED).map(|row| Line::Row(row, false)));
    Shown { lines, total }
}

fn program_row(program: &Program, query: &Query, lang: Lang) -> Option<Row> {
    let (score, marks) = query.meets_name(&program.name)?;
    Some(program_shown(program, score + PROGRAM_FIRST, marks, lang))
}

fn program_shown(program: &Program, rank: f32, marks: Vec<(usize, usize)>, lang: Lang) -> Row {
    Row { kind: Kind::Program, name: program.name.clone(), marks, detail: lang.pick("应用", "App").into(), what: What::Program(program.target.clone()), rank, bytes: None, changed: None }
}

/// A page meeting `query` by its name (where its words met it marked), or by another word it is looked for by.
fn page_row(at: usize, page: &Page, query: &Query, lang: Lang) -> Option<Row> {
    let named = query.meets_name(page.name(lang));
    let also = page.also.iter().chain([&page.zh, &page.en]).filter_map(|word| query.meets_name(word)).map(|(score, _)| score).max_by(f32::total_cmp);
    let (score, marks) = match (named, also) {
        (Some((score, marks)), Some(also)) if also <= score => (score, marks),
        (Some((score, marks)), None) => (score, marks),
        (_, Some(also)) => (also, Vec::new()),
        (None, None) => return None,
    };
    let kind = if page.tool() { Kind::Tool } else { Kind::Page };
    Some(Row { kind, name: page.name(lang).into(), marks, detail: page.at(lang).into(), what: What::Page(at), rank: score + PAGE_FIRST, bytes: None, changed: None })
}

/// A note meeting `query` by its title (where its words met it marked), or by what is written in it.
fn note_row(note: &Note, query: &Query, lang: Lang) -> Option<Row> {
    let title = note.title();
    match query.meets_name(&title) {
        Some((score, marks)) => Some(note_shown(note, score, marks, lang)),
        None => query.in_text(&note.text).then(|| note_shown(note, IN_TEXT, Vec::new(), lang)),
    }
}

/// How well a note or what was copied meets what is typed by what is written in it (as a name meets a word within it).
const IN_TEXT: f32 = 2.0;

fn note_shown(note: &Note, rank: f32, marks: Vec<(usize, usize)>, lang: Lang) -> Row {
    let title = note.title();
    let name = if title.is_empty() { lang.pick("空白便签", "Empty note").to_string() } else { title };
    Row { kind: Kind::Note, name, marks, detail: lang.pick("便签", "Note").into(), what: What::Note(note.name.clone()), rank, bytes: None, changed: None }
}

/// What the station holds, or what was copied lately, meeting `query` by its name, or (text) by what it says.
fn held_row(item: &Item, query: &Query, lang: Lang) -> Option<Row> {
    let name = held_name(&item.what, lang);
    if let Some((score, marks)) = query.meets_name(&name) {
        return Some(held_shown(item, score, marks, lang));
    }
    let text = match &item.what {
        Held::Text { text } => text.as_str(),
        Held::Link { url } => url.as_str(),
        _ => return None,
    };
    query.in_text(text).then(|| held_shown(item, IN_TEXT, Vec::new(), lang))
}

/// What a thing on the station is called in a line: its file's name (and how many more were put there with it), the
/// first line of its text, its web address, or what kind of thing it is.
fn held_name(what: &Held, lang: Lang) -> String {
    let first_of = |first: String, n: usize| match n {
        0 | 1 => first,
        n => match lang {
            Lang::Zh => format!("{first} 等 {n} 项"),
            Lang::En => format!("{first} and {} more", n - 1),
        },
    };
    match what {
        Held::Files { paths, .. } => first_of(paths.first().and_then(|path| path.file_name()).map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(), paths.len()),
        Held::Came { files } => first_of(files.first().map(|arrived| arrived.name.clone()).unwrap_or_default(), files.len()),
        Held::Text { text } => text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default().chars().take(120).collect(),
        Held::Link { url } => url.clone(),
        Held::Picture { .. } => lang.pick("图片", "Picture").into(),
        Held::Saved { name, .. } => name.clone(),
    }
}

fn held_shown(item: &Item, rank: f32, marks: Vec<(usize, usize)>, lang: Lang) -> Row {
    let (kind, detail) = if item.copied { (Kind::Copied, lang.pick("最近复制", "Recently copied")) } else { (Kind::Held, lang.pick("中转站", "Station")) };
    Row { kind, name: held_name(&item.what, lang), marks, detail: detail.into(), what: What::Item(item.id), rank, bytes: None, changed: None }
}

fn file_row(hit: crate::search::Hit) -> Row {
    let mut row = path_shown(hit.path, hit.folder, hit.rank, hit.marks);
    row.name = hit.name;
    row
}

/// The file or folder at `path`, by its name, said to be in the folder it is in (its size and when it changed told
/// after: see `Shown::told`).
fn path_shown(path: PathBuf, folder: bool, rank: f32, marks: Vec<(usize, usize)>) -> Row {
    let name = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
    let detail = path.parent().map(|parent| parent.display().to_string()).unwrap_or_default();
    let kind = if folder {
        Kind::Folder
    } else {
        Kind::File(recent::kind_of_extension(&path.extension().map(|extension| extension.to_string_lossy().to_lowercase()).unwrap_or_default()))
    };
    Row { kind, name, marks, detail, what: What::Path(path), rank, bytes: None, changed: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Typed after "待办 " (or "todo "), the words are a to-do to add, what they say of a day said under it; typed
    /// alone, they are not.
    #[test]
    fn typed_after_todo_is_a_to_do() {
        let row = todo_row("待办 明天交报告", Lang::Zh).unwrap();
        assert_eq!((row.kind, row.name.as_str(), row.detail.as_str()), (Kind::Todo, "添加为待办：交报告", "明天"));
        assert_eq!(row.what, What::Todo("明天交报告".into()));
        let row = todo_row("  TODO call the bank", Lang::En).unwrap();
        assert_eq!((row.name.as_str(), row.detail.as_str()), ("Add as to-do: call the bank", "To-do"));
        assert!(todo_row("交报告", Lang::Zh).is_none());
        assert!(todo_row("待办 ", Lang::Zh).is_none());
        assert!(todo_row("待办事项", Lang::Zh).is_none());
    }
    use crate::search::index::{Changes, Index, Item as Listed, ROOT};
    use std::collections::{BTreeMap, HashMap};

    fn program(name: &str) -> Program {
        Program { name: name.into(), target: format!("shell:AppsFolder\\{name}"), initials: Vec::new() }
    }

    /// A drive (D:) holding `paths` (a folder's ending in `\`), every folder on the way to them too.
    fn drive(paths: &[&str]) -> Index {
        let mut children: BTreeMap<String, Vec<(String, bool)>> = BTreeMap::new();
        for path in paths {
            let parts: Vec<&str> = path.trim_end_matches('\\').split('\\').collect();
            for at in 0..parts.len() {
                let named = (parts[at].to_string(), at + 1 < parts.len() || path.ends_with('\\'));
                let within = children.entry(parts[..at].join("\\")).or_default();
                if !within.contains(&named) {
                    within.push(named);
                }
            }
        }
        let mut index = Index::new(1, true);
        let mut paths_of: HashMap<u64, String> = HashMap::new();
        let mut next = 1u64;
        let mut listing = vec![(String::new(), ROOT)];
        while let Some((path, place)) = listing.pop() {
            let items: Vec<Listed> = children
                .get(&path)
                .into_iter()
                .flatten()
                .map(|(name, folder)| {
                    next += 1;
                    paths_of.insert(next, if path.is_empty() { name.clone() } else { format!("{path}\\{name}") });
                    Listed { name: name.encode_utf16().collect(), id: next, attributes: if *folder { 0x10 } else { 0x20 }, tag: 0 }
                })
                .collect();
            let mut changes = Changes::default();
            index.list(place, &items, &[], &mut changes);
            listing.extend(changes.to_list.iter().map(|&(place, id)| (paths_of[&id].clone(), place)));
            index.finish(changes);
        }
        index
    }

    const PATHS: &[&str] = &[
        "App\\Steam\\",
        "Music\\逍遥叹.flac",
        "Documents\\音乐会.pdf",
        "Pictures\\music cover.png",
        "a\\music 1.txt",
        "a\\music 2.txt",
        "a\\music 3.txt",
        "a\\music 4.txt",
        "a\\music 5.txt",
        "a\\music 6.txt",
        "a\\music 7.txt",
    ];

    /// What is shown for `typed` in `view`, a line each: `#` a heading (and how many more there are, or `all`, how
    /// many files all are), `*` the best.
    fn shown(typed: &str, view: View, expanded: bool, notes: &[Note], held: &[Item], lately: &[Came]) -> Vec<String> {
        let programs = [program("Steam"), program("网易云音乐"), program("QQ音乐"), program("Visual Studio Code")];
        let index = drive(PATHS);
        let files = |query: &Query, most: usize| {
            let (total, folders, hits) = query.find(&index, "D:\\", most);
            crate::search::Found { total, folders, hits }
        };
        let sources = Sources { programs: &programs, notes, held, lately, files: &files, lang: Lang::Zh };
        gather(typed, view, expanded, &sources)
            .lines
            .iter()
            .map(|line| match line {
                Line::Head(group, More::No) => format!("# {}", group.name(Lang::Zh)),
                Line::Head(group, More::Others(n)) => format!("# {} +{n}", group.name(Lang::Zh)),
                Line::Head(group, More::ShowAll(n)) => format!("# {} all {n}", group.name(Lang::Zh)),
                Line::Row(row, true) => format!("* {}", row.name),
                Line::Row(row, false) => row.name.clone(),
                Line::Others(n) => format!("+{n}"),
            })
            .collect()
    }

    fn found(typed: &str, view: View) -> Vec<String> {
        shown(typed, view, false, &[], &[], &[])
    }

    #[test]
    fn the_best_of_all_comes_first_then_a_few_of_each_kind() {
        // (The folder Music is the best: its whole name. Of eight files, six, and all to be shown.)
        assert_eq!(found("music", View::All), ["* Music", "# 文件 all 8", "music 1.txt", "music 2.txt", "music 3.txt", "music 4.txt", "music 5.txt", "music 6.txt"]);
        assert_eq!(shown("music", View::All, true, &[], &[], &[]).len(), 1 + 1 + 8);
        // A program first of what meets it as well: what a name of a few letters looks for most often.
        assert_eq!(found("steam", View::All), ["* Steam", "# 文件夹", "Steam"]);
        assert_eq!(found("wyy", View::All), ["* 网易云音乐"]);
        // (By pinyin: the file's start, better than within the programs' names.)
        assert_eq!(found("yinyue", View::All), ["* 音乐会.pdf", "# 应用", "网易云音乐", "QQ音乐"]);
        // A setting by its name, or by another word it is looked for by.
        assert_eq!(found("lanya", View::All), ["* 蓝牙和其他设备"]);
        assert_eq!(found("亮度", View::All), ["* 显示"]);
        assert_eq!(found("zzqq", View::All), Vec::<String>::new());
    }

    #[test]
    fn a_view_shows_its_kind_alone() {
        assert_eq!(found("music", View::Folders), ["Music"]);
        assert_eq!(found("music", View::Pictures), ["music cover.png"]);
        assert_eq!(found("music", View::Files).len(), 8);
        assert_eq!(found("yinyue", View::Programs), ["* 网易云音乐", "QQ音乐"]);
        assert_eq!(found("yinyue", View::Documents), ["音乐会.pdf"]);
        assert_eq!(found("xiaoyao", View::Media), ["逍遥叹.flac"]);
        // (Only files have kinds: no program meets what asks for one.)
        assert_eq!(found("ext:pdf", View::All), ["* 音乐会.pdf"]);
    }

    #[test]
    fn glance_s_own_are_found_by_name_and_by_what_they_say() {
        let notes = [Note { name: "a.md".into(), text: "下周要做的事\n回复 Steam 的邮件".into(), colour: Default::default(), stamp: Default::default() }];
        let held = [
            Item { id: 7, what: Held::Text { text: "https://store.steampowered.com".into() }, added_ms: 1, pinned: false, copied: true, stays_here: false, own_bytes: 0, from: None },
            Item { id: 8, what: Held::Files { paths: vec!["D:\\a\\季度计划.docx".into(), "D:\\a\\b.txt".into()], kept: Vec::new() }, added_ms: 2, pinned: false, copied: false, stays_here: false, own_bytes: 0, from: None },
        ];
        assert_eq!(shown("steam", View::Glance, false, &notes, &held, &[]), ["https://store.steampowered.com", "下周要做的事"]);
        assert_eq!(shown("xiazhou", View::Glance, false, &notes, &held, &[]), ["下周要做的事"]);
        assert_eq!(shown("jidu", View::Glance, false, &notes, &held, &[]), ["季度计划.docx 等 2 项"]);
        // Nothing typed: the newest first.
        assert_eq!(shown("", View::Glance, false, &notes, &held, &[]), ["# Glance", "下周要做的事", "季度计划.docx 等 2 项", "https://store.steampowered.com"]);
    }

    #[test]
    fn nothing_typed_shows_what_came_lately() {
        let came = |path: &str, ms: u64, kind: FileKind| Came { key: recent::Key { drive: 'D', file: u128::from(ms) }, path: path.into(), came_ms: ms, bytes: 0, kind, downloaded: false, site: None, from: None };
        let lately = [came("D:\\a.png", 1, FileKind::Screenshot), came("D:\\b.pdf", 2, FileKind::Document)];
        assert_eq!(shown("", View::All, false, &[], &[], &lately), ["# 最近的文件", "b.pdf", "a.png"]);
        assert_eq!(shown("", View::Pictures, false, &[], &[], &lately), ["# 最近的文件", "a.png"]);
        assert_eq!(shown("", View::Folders, false, &[], &[], &lately), Vec::<String>::new());
        assert_eq!(shown(" ", View::Programs, false, &[], &[], &lately), ["# 应用", "QQ音乐", "Steam", "Visual Studio Code", "网易云音乐"]);
    }

    /// Files and folders found are shown before anything of them is asked of their drives (a USB drive busy held the
    /// list till each was), told of after (a folder's size never), and found again, told of as before till told anew.
    #[test]
    fn files_found_are_told_of_after() {
        let programs = [program("Steam")];
        let index = drive(PATHS);
        let files = |query: &Query, most: usize| {
            let (total, folders, hits) = query.find(&index, "D:\\", most);
            crate::search::Found { total, folders, hits }
        };
        let sources = Sources { programs: &programs, notes: &[], held: &[], lately: &[], files: &files, lang: Lang::Zh };
        let mut shown = gather("music", View::All, false, &sources);
        assert!(shown.rows().all(|row| row.bytes.is_none() && row.changed.is_none()));
        let paths = shown.paths();
        assert_eq!(paths[0], (PathBuf::from("D:\\Music"), true));
        assert!(paths.contains(&(PathBuf::from("D:\\a\\music 1.txt"), false)));
        let when = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        shown.told(Path::new("D:\\Music"), (4096, when));
        shown.told(Path::new("D:\\a\\music 1.txt"), (12, when));
        let told = |shown: &Shown, name: &str| shown.rows().find(|row| row.name == name).map(|row| (row.bytes, row.changed));
        assert_eq!(told(&shown, "Music"), Some((None, Some(when))));
        assert_eq!(told(&shown, "music 1.txt"), Some((Some(12), Some(when))));
        assert_eq!(told(&shown, "music 2.txt"), Some((None, None)));
        let mut again = gather("music", View::All, false, &sources);
        again.told_as(&shown);
        assert_eq!(told(&again, "Music"), Some((None, Some(when))));
        assert_eq!(told(&again, "music 1.txt"), Some((Some(12), Some(when))));
    }
}
