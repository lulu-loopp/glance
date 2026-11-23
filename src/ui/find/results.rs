//! What search shows for what is typed, in the view chosen: the programs, Settings' pages and the system's tools, the
//! folders and files, and Glance's own (notes, what is on the station, what was copied lately) whose names meet it,
//! the best of all first, then each kind under its heading, a few of each. A view shows one kind alone, as many as
//! fit. Nothing typed, it shows what came lately (the files, Recent files'), the programs (A to Z), or Glance's own.
//! The view of all shows what is typed itself too: a sum's answer, the address it is, or a web search of it.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::address::{self, Address};
use super::pages::{Page, PAGES};
use super::sum;
use crate::notes::Note;
use crate::search::{Only, Order, Query};
use crate::settings::WebSearch;
use crate::station::found::Found as Program;
use crate::station::recent::{self, Came, Kind as FileKind};
use crate::station::shelf::{Item, Tag, What as Held};
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

    /// Whether what it shows can be put in another order than how well it meets what was typed: files and folders
    /// (a view of apps and settings, or of Glance's own, has none of them).
    pub fn sorts(self) -> bool {
        !matches!(self, View::Programs | View::Glance)
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
    /// A sum typed, worked out.
    Sum,
    /// A web address typed.
    Address,
    /// A web search of what was typed.
    Web,
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
    /// A sum's answer, its figures as they are copied.
    Answer(String),
    /// A web address, opened in the browser.
    Url(String),
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
    /// Files and folders together, in the order asked for (see `search::Order`).
    Found,
    Glance,
    /// What came lately (nothing typed).
    Lately,
    /// What opens in the browser: the address typed, a web search.
    Web,
}

impl Group {
    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            Group::Programs => lang.pick("应用", "Apps"),
            Group::Pages => lang.pick("设置", "Settings"),
            Group::Folders => lang.pick("文件夹", "Folders"),
            Group::Files => lang.pick("文件", "Files"),
            Group::Found => lang.pick("文件和文件夹", "Files and folders"),
            Group::Glance => "Glance",
            Group::Lately => lang.pick("最近的文件", "Recent files"),
            Group::Web => lang.pick("网页", "Web"),
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

/// What is shown, and how many names were found in all (a to-do to add counted; a sum, an address and a web search
/// not: they were not found).
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
/// the drives' files and folders (as `search::find` finds them), in the language the user reads; and what a web search
/// is made with.
pub struct Sources<'a> {
    pub programs: &'a [Program],
    pub notes: &'a [Note],
    pub held: &'a [Item],
    /// The tags the station's pages are sorted by.
    pub tags: &'a [Tag],
    pub lately: &'a [Came],
    pub files: &'a dyn Fn(&Query, usize) -> crate::search::Found,
    pub lang: Lang,
    pub web: WebSearch,
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

/// What is shown for `typed` in `view` (the files all, `expanded`), its files and folders in `order`, from `sources`.
/// First of all, larger, what is typed itself, where it is something: what adds it as a to-do (in every view), else (in
/// the view of all) its sum's answer, or what opens the address it is. Last of all, in the view of all, a web search
/// of it (nothing else shown: it alone, larger), after the address, where that is not first.
pub fn gather(typed: &str, view: View, expanded: bool, order: Order, sources: &Sources) -> Shown {
    let lang = sources.lang;
    let of_all = view == View::All && !Query::read(typed).is_empty();
    let todo = todo_row(typed, lang);
    let sum = if of_all && todo.is_none() { sum_row(typed, lang) } else { None };
    let address = if of_all && todo.is_none() && sum.is_none() { address::typed(typed) } else { None };
    // (What may be a file's name as well gives way to a file there is.)
    let (opens_first, opens_last) = match address {
        Some(address) if address.sure || !named(typed, sources) => (Some(address_row(typed, address, lang)), None),
        Some(address) => (None, Some(address_row(typed, address, lang))),
        None => (None, None),
    };
    let todos = usize::from(todo.is_some());
    let first = todo.or(sum).or(opens_first);
    let mut shown = found(typed, view, expanded, order, first.is_none(), sources);
    shown.total += todos;
    if let Some(row) = first {
        shown.lines.insert(0, Line::Row(row, true));
    }
    if of_all {
        let last: Vec<Row> = opens_last.into_iter().chain(web_row(typed, sources.web, lang)).collect();
        if shown.lines.is_empty() {
            shown.lines.extend(last.into_iter().map(|row| Line::Row(row, true)));
        } else if !last.is_empty() {
            shown.lines.push(Line::Head(Group::Web, More::No));
            shown.lines.extend(last.into_iter().map(|row| Line::Row(row, false)));
        }
    }
    shown
}

/// Whether a file, a folder or a program there is has a name ending in what was typed (`main.rs`, a file of that name
/// or `domain.rs`).
fn named(typed: &str, sources: &Sources) -> bool {
    let typed = typed.trim();
    let end = typed.to_lowercase();
    sources.programs.iter().any(|program| program.name.to_lowercase().ends_with(&end)) || (sources.files)(&Query::read(&format!("*{typed}")), 1).total > 0
}

/// A row for what is typed itself, shown as `name`, `detail` under it.
fn own_row(kind: Kind, name: String, detail: String, what: What) -> Row {
    Row { kind, name, marks: Vec::new(), detail, what, rank: f32::MAX, bytes: None, changed: None }
}

/// What was typed, worked out, where it is a sum.
fn sum_row(typed: &str, lang: Lang) -> Option<Row> {
    let answer = sum::worked_out(typed)?;
    let detail = format!("{} · {}", answer.said, lang.pick("按 Enter 复制结果", "Enter copies the result"));
    Some(own_row(Kind::Sum, answer.shown, detail, What::Answer(answer.plain)))
}

fn address_row(typed: &str, address: Address, lang: Lang) -> Row {
    let typed = typed.trim();
    let name = match lang {
        Lang::Zh => format!("打开 {typed}"),
        Lang::En => format!("Open {typed}"),
    };
    own_row(Kind::Address, name, in_browser(lang).into(), What::Url(address.url))
}

fn in_browser(lang: Lang) -> &'static str {
    lang.pick("在默认浏览器中打开", "In the default browser")
}

/// A web search of what was typed, where one is made with anything.
pub fn web_row(typed: &str, web: WebSearch, lang: Lang) -> Option<Row> {
    let typed = typed.trim();
    let url = web.url(typed).filter(|_| !typed.is_empty())?;
    let engine = web.name(lang);
    let name = match lang {
        // (A space between a name in letters and the words round it.)
        Lang::Zh if engine.is_ascii() => format!("在 {engine} 中搜索“{typed}”"),
        Lang::Zh => format!("在{engine}中搜索“{typed}”"),
        Lang::En => format!("Search {engine} for \u{201C}{typed}\u{201D}"),
    };
    Some(own_row(Kind::Web, name, in_browser(lang).into(), What::Url(url)))
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
    Some(own_row(Kind::Todo, name, detail, What::Todo(rest.to_string())))
}

/// What was found of what was typed (see `gather`). In another order than how well they meet it, the files and folders
/// are the first of all of them in it, together (the view of all shows them under one heading, and none first larger);
/// what else is found keeps its place and its order. The best of all is shown first, larger, unless something else is
/// (`best_first` not): it is under its heading then, the first there.
fn found(typed: &str, view: View, expanded: bool, order: Order, best_first: bool, sources: &Sources) -> Shown {
    let query = Query::read(typed).ordered(order);
    let ordered = order.other() && view.sorts();
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
        glance.extend(sources.held.iter().filter_map(|item| held_row(item, sources.tags, &query, lang)));
        glance.extend(sources.lately.iter().filter_map(|came| tagged_row(came, sources.tags, &query, lang)));
    }
    let most = match view {
        View::All if expanded => FILES_ALL,
        View::All if ordered => FILES,
        View::All => FILES.max(FOLDERS),
        _ => LISTED,
    };
    let found = view.files(query).map(|query| (sources.files)(&query, most)).unwrap_or_default();
    let in_order: Vec<Row> = found.hits.into_iter().map(file_row).collect();
    // (A file that came lately found by its name too: once, among the files.)
    let named: std::collections::HashSet<&What> = in_order.iter().map(|row| &row.what).collect();
    glance.retain(|row| !named.contains(&row.what));
    drop(named);
    let (mut folders, mut files): (Vec<Row>, Vec<Row>) = if ordered { (Vec::new(), in_order) } else { in_order.into_iter().partition(|row| row.kind == Kind::Folder) };
    for rows in [&mut programs, &mut pages, &mut glance] {
        rows.sort_by(|a, b| b.rank.total_cmp(&a.rank));
    }
    if !ordered {
        for rows in [&mut folders, &mut files] {
            rows.sort_by(|a, b| b.rank.total_cmp(&a.rank));
        }
    }
    let total = programs.len() + pages.len() + glance.len() + found.total;
    if view != View::All {
        // One kind: as many as fit, a program or page first shown larger. (In another order, files and folders are
        // all it shows: see `View::sorts`.)
        let mut rows: Vec<Row> = programs.into_iter().chain(pages).chain(glance).chain(folders).chain(files).collect();
        if !ordered {
            rows.sort_by(|a, b| b.rank.total_cmp(&a.rank));
        }
        let others = total.saturating_sub(LISTED.min(rows.len()));
        rows.truncate(LISTED);
        let mut lines: Vec<Line> = rows.into_iter().enumerate().map(|(at, row)| { let best = best_first && at == 0 && matches!(row.kind, Kind::Program | Kind::Page | Kind::Tool); Line::Row(row, best) }).collect();
        if others > 0 {
            lines.push(Line::Others(others));
        }
        return Shown { lines, total };
    }
    let files_most = if expanded { FILES_ALL } else { FILES };
    let mut lines = Vec::new();
    if ordered {
        // Each kind under its heading, the files and folders together under theirs: none first, larger (the best of
        // what is in another order is not first).
        let files_total = found.total;
        let heads = [(Group::Programs, PROGRAMS, programs.len(), programs), (Group::Pages, PAGES_SHOWN, pages.len(), pages), (Group::Found, files_most, files_total, files), (Group::Glance, GLANCE, glance.len(), glance)];
        for (group, most, size, rows) in heads {
            headed(&mut lines, group, rows, most, (size, size), expanded);
        }
        return Shown { lines, total };
    }
    // The best of all first, larger, then each kind under its heading (how many of each were found, the best one too).
    let sizes = [programs.len(), pages.len(), found.folders, found.total - found.folders, glance.len()];
    let mut groups = [programs, pages, folders, files, glance];
    let best = groups.iter().enumerate().filter_map(|(at, rows)| rows.first().map(|row| (at, row.rank))).max_by(|a, b| a.1.total_cmp(&b.1)).map(|(at, _)| at).filter(|_| best_first);
    if let Some(at) = best {
        lines.push(Line::Row(groups[at].remove(0), true));
    }
    let heads = [(Group::Programs, PROGRAMS), (Group::Pages, PAGES_SHOWN), (Group::Folders, FOLDERS), (Group::Files, files_most), (Group::Glance, GLANCE)];
    for (at, ((group, most), rows)) in heads.into_iter().zip(groups).enumerate() {
        headed(&mut lines, group, rows, most, (sizes[at] - usize::from(best == Some(at)), sizes[at]), expanded);
    }
    Shown { lines, total }
}

/// `rows` of a kind under its heading, `most` of them, and how many more there are of the `left` found (not counting
/// one shown first, of `all`): the files, not all shown (`expanded` not), to be shown all.
fn headed(lines: &mut Vec<Line>, group: Group, rows: Vec<Row>, most: usize, (left, all): (usize, usize), expanded: bool) {
    if rows.is_empty() {
        return;
    }
    let shown = most.min(rows.len());
    let more = match left.saturating_sub(shown) {
        0 => More::No,
        _ if matches!(group, Group::Files | Group::Found) && !expanded => More::ShowAll(all),
        more => More::Others(more),
    };
    lines.push(Line::Head(group, more));
    lines.extend(rows.into_iter().take(shown).map(|row| Line::Row(row, false)));
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
            let rows = notes.into_iter().map(|note| note_shown(note, 0.0, Vec::new(), lang)).chain(held.into_iter().map(|item| held_shown(item, sources.tags, 0.0, Vec::new(), lang))).collect();
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

/// What the station holds, or what was copied lately, meeting `query` by its name, by a tag of its (of `tags`), or (text)
/// by what it says.
fn held_row(item: &Item, tags: &[Tag], query: &Query, lang: Lang) -> Option<Row> {
    let name = held_name(&item.what, lang);
    if let Some((score, marks)) = query.meets_name(&name) {
        return Some(held_shown(item, tags, score, marks, lang));
    }
    if let Some(score) = meets_tag(&item.tags, tags, query) {
        return Some(held_shown(item, tags, score, Vec::new(), lang));
    }
    let text = match &item.what {
        Held::Text { text } => text.as_str(),
        Held::Link { url } => url.as_str(),
        _ => return None,
    };
    query.in_text(text).then(|| held_shown(item, tags, IN_TEXT, Vec::new(), lang))
}

/// How well one of the tags `ids` (of `tags`) meets `query` by its name, the best of them.
fn meets_tag(ids: &[u32], tags: &[Tag], query: &Query) -> Option<f32> {
    tags.iter().filter(|tag| ids.contains(&tag.id)).filter_map(|tag| query.meets_name(&tag.name)).map(|(score, _)| score).max_by(f32::total_cmp)
}

/// A file that came lately meeting `query` by a tag of its (of `tags`): said to be one of those, with its tags.
fn tagged_row(came: &Came, tags: &[Tag], query: &Query, lang: Lang) -> Option<Row> {
    let score = meets_tag(&came.marks.tags, tags, query)?;
    Some(Row { detail: with_tags(lang.pick("最近文件", "Recent files"), &came.marks.tags, tags), ..path_shown(came.path.clone(), false, score, Vec::new()) })
}

/// `place`, and the tags `ids` (of `tags`) after it.
fn with_tags(place: &str, ids: &[u32], tags: &[Tag]) -> String {
    std::iter::once(place).chain(tags.iter().filter(|tag| ids.contains(&tag.id)).map(|tag| tag.name.as_str())).collect::<Vec<_>>().join(" · ")
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
        Held::Came { files } => {
            let entries = crate::station::shelf::entries(files);
            let first = match entries.first() {
                Some(crate::station::shelf::Entry::File(arrived)) => arrived.name.clone(),
                Some(crate::station::shelf::Entry::Folder { name, .. }) => name.to_string(),
                None => String::new(),
            };
            first_of(first, entries.len())
        }
        Held::Text { text } => text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default().chars().take(120).collect(),
        Held::Link { url } => url.clone(),
        Held::Picture { .. } => lang.pick("图片", "Picture").into(),
        Held::Saved { name, .. } => name.clone(),
    }
}

fn held_shown(item: &Item, tags: &[Tag], rank: f32, marks: Vec<(usize, usize)>, lang: Lang) -> Row {
    let (kind, detail) = if item.copied { (Kind::Copied, lang.pick("最近复制", "Recently copied")) } else { (Kind::Held, lang.pick("中转站", "Station")) };
    // (Where it is, and its tags.)
    let detail = with_tags(detail, &item.tags, tags);
    Row { kind, name: held_name(&item.what, lang), marks, detail, what: What::Item(item.id), rank, bytes: None, changed: None }
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
                    Listed { name: name.encode_utf16().collect(), id: next, attributes: if *folder { 0x10 } else { 0x20 }, tag: 0, stat: Default::default() }
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
        "Code\\main.rs",
        "Code\\main.rs.bak",
    ];

    /// What is shown for `typed` in `view`, a line each: `#` a heading (and how many more there are, or `all`, how
    /// many files all are), `*` the best.
    fn shown(typed: &str, view: View, expanded: bool, notes: &[Note], held: &[Item], lately: &[Came]) -> Vec<String> {
        shown_in(typed, view, expanded, Order::default(), notes, held, lately)
    }

    /// As `shown`, the files and folders in `order`.
    fn shown_in(typed: &str, view: View, expanded: bool, order: Order, notes: &[Note], held: &[Item], lately: &[Came]) -> Vec<String> {
        shown_with(typed, view, expanded, order, notes, held, lately, WebSearch::Off)
    }

    /// As `shown_in`, a web search made with `web`.
    #[allow(clippy::too_many_arguments)]
    fn shown_with(typed: &str, view: View, expanded: bool, order: Order, notes: &[Note], held: &[Item], lately: &[Came], web: WebSearch) -> Vec<String> {
        let programs = [program("Steam"), program("网易云音乐"), program("QQ音乐"), program("Visual Studio Code")];
        let index = drive(PATHS);
        let files = |query: &Query, most: usize| {
            let (total, folders, hits) = query.find(&index, "D:\\", most);
            crate::search::Found { total, folders, hits }
        };
        let sources = Sources { programs: &programs, notes, held, tags: &[], lately, files: &files, lang: Lang::Zh, web };
        gather(typed, view, expanded, order, &sources)
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

    /// In another order, the view of all shows the files and folders together under one heading (none first, larger),
    /// the first of all of them in it, as many as files are shown, all to be shown; what else is found keeps its
    /// heading and its order. A view of files and folders shows them in it; a view of other things, as it did.
    #[test]
    fn in_another_order_files_and_folders_are_shown_together_in_it() {
        use crate::search::Sort;
        let by_name = Order { by: Sort::Name, reversed: false };
        let in_order = |typed: &str, view: View, order: Order| shown_in(typed, view, false, order, &[], &[], &[]);
        assert_eq!(in_order("music", View::All, by_name), ["# 文件和文件夹 all 9", "Music", "music 1.txt", "music 2.txt", "music 3.txt", "music 4.txt", "music 5.txt"]);
        assert_eq!(in_order("music", View::All, Order { reversed: true, ..by_name }), ["# 文件和文件夹 all 9", "Music", "music cover.png", "music 7.txt", "music 6.txt", "music 5.txt", "music 4.txt"]);
        assert_eq!(shown_in("music", View::All, true, by_name, &[], &[], &[]).len(), 1 + 9);
        assert_eq!(in_order("steam", View::All, by_name), ["# 应用", "Steam", "# 文件和文件夹", "Steam"]);
        let notes = [Note { name: "a.md".into(), text: "music to buy".into(), colour: Default::default(), stamp: Default::default() }];
        assert_eq!(shown_in("cover", View::All, false, by_name, &notes, &[], &[]), ["# 设置", "恢复", "# 文件和文件夹", "music cover.png"]);
        assert_eq!(shown_in("music", View::All, false, by_name, &notes, &[], &[]).last().map(String::as_str), Some("music to buy"));
        assert_eq!(in_order("music", View::Files, Order { reversed: true, ..by_name })[..3], ["music cover.png", "music 7.txt", "music 6.txt"]);
        assert_eq!(in_order("yinyue", View::Programs, by_name), ["* 网易云音乐", "QQ音乐"]);
    }

    #[test]
    fn glance_s_own_are_found_by_name_and_by_what_they_say() {
        let notes = [Note { name: "a.md".into(), text: "下周要做的事\n回复 Steam 的邮件".into(), colour: Default::default(), stamp: Default::default() }];
        let held = [
            Item { id: 7, what: Held::Text { text: "https://store.steampowered.com".into() }, added_ms: 1, pinned: false, copied: true, stays_here: false, own_bytes: 0, from: None, used_ms: 0, tags: Vec::new(), held: false, freed_ms: 0 },
            Item { id: 8, what: Held::Files { paths: vec!["D:\\a\\季度计划.docx".into(), "D:\\a\\b.txt".into()], kept: Vec::new() }, added_ms: 2, pinned: false, copied: false, stays_here: false, own_bytes: 0, from: None, used_ms: 0, tags: Vec::new(), held: false, freed_ms: 0 },
        ];
        assert_eq!(shown("steam", View::Glance, false, &notes, &held, &[]), ["https://store.steampowered.com", "下周要做的事"]);
        assert_eq!(shown("xiazhou", View::Glance, false, &notes, &held, &[]), ["下周要做的事"]);
        assert_eq!(shown("jidu", View::Glance, false, &notes, &held, &[]), ["季度计划.docx 等 2 项"]);
        // Nothing typed: the newest first.
        assert_eq!(shown("", View::Glance, false, &notes, &held, &[]), ["# Glance", "下周要做的事", "季度计划.docx 等 2 项", "https://store.steampowered.com"]);
    }

    #[test]
    fn nothing_typed_shows_what_came_lately() {
        let came = |path: &str, ms: u64, kind: FileKind| Came::of(recent::Key { drive: 'D', file: u128::from(ms) }, path.into(), ms, 0, kind);
        let lately = [came("D:\\a.png", 1, FileKind::Screenshot), came("D:\\b.pdf", 2, FileKind::Document)];
        assert_eq!(shown("", View::All, false, &[], &[], &lately), ["# 最近的文件", "b.pdf", "a.png"]);
        assert_eq!(shown("", View::Pictures, false, &[], &[], &lately), ["# 最近的文件", "a.png"]);
        assert_eq!(shown("", View::Folders, false, &[], &[], &lately), Vec::<String>::new());
        assert_eq!(shown(" ", View::Programs, false, &[], &[], &lately), ["# 应用", "QQ音乐", "Steam", "Visual Studio Code", "网易云音乐"]);
    }

    /// What is on any page of the station is found by its tag, pinned or not, which its line says: what was copied, what
    /// is on the station, and a file that came lately (once, where its name meets what is typed too).
    #[test]
    fn what_is_on_the_station_s_pages_is_found_by_its_tag() {
        let tags = [Tag { id: 3, name: "常用回复".into() }, Tag { id: 4, name: "命令".into() }];
        let item = |id: u64, copied: bool, tag: u32| Item { id, what: Held::Text { text: "您好".into() }, added_ms: 1, pinned: false, copied, stays_here: false, own_bytes: 0, from: None, used_ms: 0, tags: vec![tag], held: false, freed_ms: 0 };
        let row = held_row(&item(7, true, 4), &tags, &Query::read("命令"), Lang::Zh).unwrap();
        assert_eq!((row.name.as_str(), row.detail.as_str(), row.marks), ("您好", "最近复制 · 命令", Vec::new()));
        assert_eq!(held_row(&item(7, false, 4), &tags, &Query::read("命令"), Lang::Zh).unwrap().detail, "中转站 · 命令");
        assert!(held_row(&item(7, true, 4), &tags, &Query::read("常用"), Lang::Zh).is_none());
        let came = |path: &str, marks: &[u32]| Came::of(recent::Key { drive: 'D', file: 1 }, path.into(), 1, 0, FileKind::Document).marked(recent::Marks { tags: marks.to_vec(), ..Default::default() });
        let row = tagged_row(&came("D:\\a\\报价.pdf", &[4, 3]), &tags, &Query::read("命令"), Lang::Zh).unwrap();
        assert_eq!((row.name.as_str(), row.detail.as_str(), row.what), ("报价.pdf", "最近文件 · 常用回复 · 命令", What::Path("D:\\a\\报价.pdf".into())));
        assert!(tagged_row(&came("D:\\a\\命令.pdf", &[3]), &tags, &Query::read("命令"), Lang::Zh).is_none());
        let found_in = |files: &dyn Fn(&Query, usize) -> crate::search::Found| {
            let lately = [came("D:\\a\\报价.pdf", &[4]), came("D:\\a\\命令.txt", &[4])];
            let sources = Sources { programs: &[], notes: &[], held: &[], tags: &tags, lately: &lately, files, lang: Lang::Zh, web: WebSearch::Off };
            let shown = super::found("命令", View::All, false, Order::default(), true, &sources);
            let mut names: Vec<String> = shown.lines.iter().filter_map(|line| if let Line::Row(row, _) = line { Some(row.name.clone()) } else { None }).collect();
            names.sort();
            names
        };
        assert_eq!(found_in(&|_, _| Default::default()), ["命令.txt", "报价.pdf"]);
        let hit = || crate::search::Hit { path: "D:\\a\\命令.txt".into(), name: "命令.txt".into(), folder: false, rank: 5.0, marks: Vec::new(), stat: Default::default() };
        assert_eq!(found_in(&|_, _| crate::search::Found { hits: vec![hit()], total: 1, ..Default::default() }), ["命令.txt", "报价.pdf"]);
    }

    /// In the view of all, a sum's answer comes first, larger, what else is found under its headings (the best too), and
    /// a web search last; nothing found, the search comes first, larger. No other view shows them.
    #[test]
    fn what_is_typed_itself_is_shown_with_what_is_found() {
        let web = |typed: &str, view: View| shown_with(typed, view, false, Order::default(), &[], &[], &[], WebSearch::Bing);
        assert_eq!(web("128*6.5+40", View::All), ["* = 872", "# 网页", "在必应中搜索“128*6.5+40”"]);
        assert_eq!(web("steam", View::All), ["* Steam", "# 文件夹", "Steam", "# 网页", "在必应中搜索“steam”"]);
        assert_eq!(web("zzqq", View::All), ["* 在必应中搜索“zzqq”"]);
        assert_eq!(web(" ", View::All), Vec::<String>::new());
        for typed in ["128*6.5+40", "github.com", "zzqq"] {
            assert!(web(typed, View::Files).is_empty() && web(typed, View::Programs).is_empty(), "{typed}");
        }
        // (A search engine not used: none of it.)
        assert_eq!(found("128*6.5+40", View::All), ["* = 872"]);
        assert_eq!(found("zzqq", View::All), Vec::<String>::new());
        // In another order too.
        let by_name = Order { by: crate::search::Sort::Name, reversed: false };
        assert_eq!(shown_with("music 1", View::All, false, by_name, &[], &[], &[], WebSearch::Google), ["# 文件和文件夹", "music 1.txt", "# 网页", "在 Google 中搜索“music 1”"]);
        // A to-do to add comes before them, the web search last all the same.
        assert_eq!(web("待办 steam", View::All), ["* 添加为待办：steam", "# 网页", "在必应中搜索“待办 steam”"]);
    }

    /// An address typed comes first, what else is found after it; where it may be the name of a file there is, the file
    /// first, the address before the web search.
    #[test]
    fn an_address_gives_way_to_a_file_of_that_name() {
        let web = |typed: &str| shown_with(typed, View::All, false, Order::default(), &[], &[], &[], WebSearch::Baidu);
        assert_eq!(web("github.com"), ["* 打开 github.com", "# 网页", "在百度中搜索“github.com”"]);
        assert_eq!(web("main.rs"), ["* main.rs", "# 文件", "main.rs.bak", "# 网页", "打开 main.rs", "在百度中搜索“main.rs”"]);
        // (Only to a name ending in it: this one goes on.)
        assert_eq!(web("in.rs.ba"), ["* 打开 in.rs.ba", "# 文件", "main.rs.bak", "# 网页", "在百度中搜索“in.rs.ba”"]);
        assert_eq!(web("https://main.rs")[0], "* 打开 https://main.rs");
        // Found in whatever order the files are put in.
        let by_size = Order { by: crate::search::Sort::Size, reversed: false };
        assert_eq!(shown_with("main.rs", View::All, false, by_size, &[], &[], &[], WebSearch::Off).last().map(String::as_str), Some("打开 main.rs"));
        // What is copied already of it does not hold it back.
        let held = [Item { id: 7, what: Held::Link { url: "https://github.com".into() }, added_ms: 1, pinned: false, copied: true, stays_here: false, own_bytes: 0, from: None, used_ms: 0, tags: Vec::new(), held: false, freed_ms: 0 }];
        assert_eq!(shown_with("github.com", View::All, false, Order::default(), &[], &held, &[], WebSearch::Off), ["* 打开 github.com", "# Glance", "https://github.com"]);
    }

    /// What was found is counted, not the sum, the address or the web search beside it.
    #[test]
    fn only_what_is_found_is_counted() {
        let index = drive(PATHS);
        let files = |query: &Query, most: usize| {
            let (total, folders, hits) = query.find(&index, "D:\\", most);
            crate::search::Found { total, folders, hits }
        };
        let sources = Sources { programs: &[], notes: &[], held: &[], tags: &[], lately: &[], files: &files, lang: Lang::Zh, web: WebSearch::Bing };
        let total = |typed: &str| gather(typed, View::All, false, Order::default(), &sources).total;
        assert_eq!((total("1+1"), total("github.com"), total("main.rs"), total("zzqq")), (0, 0, 2, 0));
    }

    /// Each is what it does: the sum's answer copied in figures alone, the address opened, the words looked for in the
    /// browser by the engine chosen.
    #[test]
    fn what_is_typed_itself_does_what_it_says() {
        assert_eq!(sum_row("1000*2", Lang::Zh).map(|row| (row.name, row.detail, row.what)), Some(("= 2,000".into(), "1000 × 2 · 按 Enter 复制结果".into(), What::Answer("2000".into()))));
        let row = address_row(" github.com ", address::typed(" github.com ").unwrap(), Lang::En);
        assert_eq!((row.name.as_str(), row.what), ("Open github.com", What::Url("https://github.com".into())));
        let row = web_row(" a&b ", WebSearch::DuckDuckGo, Lang::En).unwrap();
        assert_eq!((row.name.as_str(), row.what), ("Search DuckDuckGo for \u{201C}a&b\u{201D}", What::Url("https://duckduckgo.com/?q=a%26b".into())));
        assert!(web_row("a", WebSearch::Off, Lang::En).is_none() && web_row(" ", WebSearch::Bing, Lang::En).is_none());
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
        let sources = Sources { programs: &programs, notes: &[], held: &[], tags: &[], lately: &[], files: &files, lang: Lang::Zh, web: WebSearch::Off };
        let mut shown = gather("music", View::All, false, Order::default(), &sources);
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
        let mut again = gather("music", View::All, false, Order::default(), &sources);
        again.told_as(&shown);
        assert_eq!(told(&again, "Music"), Some((None, Some(when))));
        assert_eq!(told(&again, "music 1.txt"), Some((Some(12), Some(when))));
    }
}
