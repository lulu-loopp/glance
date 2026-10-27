//! What is typed into search, read and looked for in a drive's index: words each name must hold (in any case, or
//! spelt in pinyin), `ext:` the kinds of file asked for, words with `\` folders the name must be within, `*` and `?`
//! a name's pattern. Every name meeting them all is found; the best are ranked by how well their names meet the
//! words, and how near the user's own files they are.

use super::index::{flag, Index, ROOT};
use super::readings;
use std::collections::HashMap;
use std::path::PathBuf;

/// A word typed.
#[derive(Debug, PartialEq)]
enum Word {
    /// Part of a name (case folded); spelt in pinyin too where it is all letters and figures.
    Plain(String),
    /// A pattern a whole name meets (case folded): `*` anything, `?` any one character.
    Pattern(Vec<char>),
}

/// What is typed, read.
#[derive(Debug, Default, PartialEq)]
pub struct Query {
    words: Vec<Word>,
    /// Kinds of file (extensions, folded, without their dot).
    kinds: Vec<String>,
    /// Folders the name must be within (each part of one of the folders it is in, folded).
    within: Vec<String>,
    /// The kinds of file a view of them allows (none: any), besides those typed; and whether only folders, or only
    /// files, are found.
    allowed: Option<Vec<String>>,
    only: Option<Only>,
}

/// Which names alone are found: folders', or files'.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Only {
    Folders,
    Files,
}

/// A name found.
#[derive(Debug, Clone)]
pub struct Hit {
    pub path: PathBuf,
    pub name: String,
    pub folder: bool,
    /// Where the words met its name, as characters of `name` (from, to).
    pub marks: Vec<(usize, usize)>,
    pub rank: f32,
}

/// How many of the names found are ranked by their paths too (the best by their names alone).
const RANKED: usize = 2000;

/// Folders whose files are rarely the ones looked for: Windows', programs', caches and builds.
const AWAY: &[&str] = &[
    "windows", "program files", "program files (x86)", "programdata", "appdata", "$recycle.bin", "system volume information", "node_modules", ".git", "target",
    "__pycache__", ".cargo", ".rustup", "site-packages", ".gradle", ".nuget", "winsxs",
];

impl Query {
    pub fn read(typed: &str) -> Query {
        let mut query = Query::default();
        for word in typed.split_whitespace() {
            let folded = word.to_lowercase();
            if let Some(kinds) = folded.strip_prefix("ext:") {
                query.kinds.extend(kinds.split([',', ';']).map(|kind| kind.trim_start_matches('.').to_string()).filter(|kind| !kind.is_empty()));
            } else if folded.contains('\\') {
                query.within.extend(folded.split('\\').filter(|part| !part.is_empty()).map(String::from));
            } else if folded.contains(['*', '?']) {
                query.words.push(Word::Pattern(folded.chars().collect()));
            } else {
                query.words.push(Word::Plain(folded));
            }
        }
        query
    }

    /// Whether nothing was typed to be looked for (what a view allows is not looked for alone).
    pub fn is_empty(&self) -> bool {
        self.words.is_empty() && self.kinds.is_empty() && self.within.is_empty()
    }

    /// The same, only files of `kinds` (extensions, folded, without their dot) found, of those typed too.
    pub fn allowing(self, kinds: &[&str]) -> Query {
        Query { allowed: Some(kinds.iter().map(|kind| kind.to_string()).collect()), only: Some(Only::Files), ..self }
    }

    /// The same, only folders', or files', names found.
    pub fn only(self, only: Only) -> Query {
        Query { only: Some(only), ..self }
    }

    /// The names in `index` (of the drive whose root is `root`) meeting it: how many there are, how many of them are
    /// folders', and the best `most` folders and the best `most` files, best first.
    pub fn find(&self, index: &Index, root: &str, most: usize) -> (usize, usize, Vec<Hit>) {
        let threads = (index.len() as usize).div_ceil(1 << 20).clamp(1, 4);
        let candidates = self.candidates(index, threads);
        // (Many, they are met on several threads at once.)
        let meet = |places: &[u32]| places.iter().filter_map(|&place| self.meets(index, place).map(|score| (score, place))).collect::<Vec<_>>();
        let met: Vec<(f32, u32)> = if candidates.len() < 1 << 16 {
            meet(&candidates)
        } else {
            let share = candidates.len().div_ceil(threads);
            std::thread::scope(|scope| {
                let meeting: Vec<_> = candidates.chunks(share).map(|places| scope.spawn(move || meet(places))).collect();
                meeting.into_iter().flat_map(|met| met.join().unwrap()).collect()
            })
        };
        let total = met.len();
        // The best by their names, folders and files apart, then those ranked by where they are too.
        let (folders, files): (Vec<_>, Vec<_>) = met.into_iter().partition(|&(_, place)| index.entry(place).folder());
        let folders_total = folders.len();
        let rank = |mut met: Vec<(f32, u32)>| -> Vec<Hit> {
            let ranked = RANKED.min(met.len());
            if ranked < met.len() {
                met.select_nth_unstable_by(ranked, |a, b| b.0.total_cmp(&a.0));
                met.truncate(ranked);
            }
            let mut hits: Vec<Hit> = met
                .into_iter()
                .filter_map(|(score, place)| {
                    let entry = index.entry(place);
                    let path = index.path(place, root)?;
                    let rank = score - away(index, place) - if entry.flags & (flag::HIDDEN | flag::SYSTEM) != 0 { 2.0 } else { 0.0 };
                    Some(Hit { name: index.name(place).to_string_lossy().into_owned(), path, folder: entry.folder(), marks: Vec::new(), rank })
                })
                .collect();
            sort(&mut hits);
            hits.truncate(most);
            hits
        };
        let mut hits = rank(folders);
        hits.extend(rank(files));
        sort(&mut hits);
        // (Where the words met is found for those shown alone.)
        for hit in &mut hits {
            hit.marks = self.marks(&hit.name);
        }
        (total, folders_total, hits)
    }

    /// The live entries that may meet it: those holding its longest word (or spelling it in pinyin), else a part of
    /// its pattern, else its kinds, else those within its folders.
    fn candidates(&self, index: &Index, threads: usize) -> Vec<u32> {
        let plain = self.words.iter().filter_map(|word| if let Word::Plain(word) = word { Some(word) } else { None }).max_by_key(|word| word.len());
        if let Some(word) = plain {
            let mut found = index.holding(word, threads);
            if spellable(word) {
                found.extend(index.chinese().filter(|&place| !index.folded_name(place).contains(word.as_str())));
            }
            return found;
        }
        let part = self.words.iter().filter_map(|word| if let Word::Pattern(pattern) = word { Some(pattern) } else { None }).flat_map(|pattern| pattern.split(|&c| c == '*' || c == '?')).max_by_key(|part| part.len());
        if let Some(part) = part.filter(|part| !part.is_empty()) {
            return index.holding(&part.iter().collect::<String>(), threads);
        }
        if !self.words.is_empty() {
            return live(index);
        }
        let kinds = if self.kinds.is_empty() { self.allowed.as_deref().unwrap_or_default() } else { &self.kinds };
        if !kinds.is_empty() {
            // (A name may end in one kind and hold another: each found once.)
            let mut found: Vec<u32> = kinds.iter().flat_map(|kind| index.holding(&format!(".{kind}"), threads)).collect();
            found.sort_unstable();
            found.dedup();
            return found;
        }
        if let Some(last) = self.within.last() {
            // Whatever is in the folders named so.
            let folders: std::collections::HashSet<u32> = index.holding(last, threads).into_iter().filter(|&place| index.entry(place).folder()).collect();
            let mut under: HashMap<u32, bool> = folders.iter().map(|&folder| (folder, true)).collect();
            return live(index).into_iter().filter(|&place| place != ROOT && within(index, index.entry(place).parent, &mut under, &|folder| folders.contains(&folder))).collect();
        }
        Vec::new()
    }

    /// How well the entry at `place` meets it; none where it does not.
    fn meets(&self, index: &Index, place: u32) -> Option<f32> {
        let entry = index.entry(place);
        if place == ROOT || !entry.alive() {
            return None;
        }
        let folded = index.folded_name(place);
        match self.only {
            Some(Only::Folders) if !entry.folder() => return None,
            Some(Only::Files) if entry.folder() => return None,
            _ => {}
        }
        for kinds in [Some(&self.kinds).filter(|kinds| !kinds.is_empty()), self.allowed.as_ref()].into_iter().flatten() {
            let kind = folded.rsplit_once('.').map(|(_, kind)| kind)?;
            if entry.folder() || !kinds.iter().any(|asked| asked == kind) {
                return None;
            }
        }
        let score = self.words_meet(folded, entry.flags & flag::CHINESE != 0)?;
        if !self.within.iter().all(|part| ancestors_hold(index, entry.parent, part)) {
            return None;
        }
        // (Its words' score counts most.)
        Some(score * 2.0 + if entry.folder() { 0.5 } else { 0.0 })
    }

    /// How well its words meet `folded` (a name, case folded; `chinese`, holding Chinese characters, spelt in pinyin
    /// too): each 4 for the whole name, 3 its start, 2 a word's start in it, 1 within it. None where one does not.
    fn words_meet(&self, folded: &str, chinese: bool) -> Option<f32> {
        let mut score = 0.0;
        let mut chars: Option<Vec<char>> = None;
        for word in &self.words {
            match word {
                Word::Plain(word) => {
                    if let Some(at) = folded.find(word.as_str()) {
                        score += if at == 0 && word.len() == folded.len() {
                            4.0
                        } else if at == 0 {
                            3.0
                        } else if starts_word(folded, at) {
                            2.0
                        } else {
                            1.0
                        };
                    } else if chinese && spellable(word) {
                        let chars = chars.get_or_insert_with(|| folded.chars().collect());
                        let (from, to) = readings::spelt(chars, word.as_bytes())?;
                        score += if from == 0 && to == chars.len() {
                            4.0
                        } else if from == 0 {
                            3.0
                        } else if readings::read(chars[from]) || !chars[from - 1].is_alphanumeric() {
                            2.0
                        } else {
                            1.0
                        };
                    } else {
                        return None;
                    }
                }
                Word::Pattern(pattern) => {
                    let chars = chars.get_or_insert_with(|| folded.chars().collect());
                    if !matches_pattern(chars, pattern) {
                        return None;
                    }
                    score += 3.0;
                }
            }
        }
        Some(score)
    }

    /// How well a name that is not a file's (a program's, a settings page's, a note's) meets it, as a file's name would
    /// (its words' score doubled, as `find` ranks), and where its words met it; none where it does not, or it asks for
    /// what only files have (kinds, folders).
    pub fn meets_name(&self, name: &str) -> Option<(f32, Vec<(usize, usize)>)> {
        if self.words.is_empty() || !self.kinds.is_empty() || !self.within.is_empty() {
            return None;
        }
        let folded: String = name.chars().flat_map(char::to_lowercase).collect();
        let score = self.words_meet(&folded, name.chars().any(readings::read))?;
        Some((score * 2.0, self.marks(name)))
    }

    /// Whether it asks for what only files have: kinds of file, or folders they are within.
    pub fn of_files(&self) -> bool {
        !self.kinds.is_empty() || !self.within.is_empty()
    }

    /// Whether `text` (a note's, something copied) holds each of its words, in any case: met within what is written,
    /// not by a name. (A pattern is a name's: it meets no text.)
    pub fn in_text(&self, text: &str) -> bool {
        if self.words.is_empty() || self.of_files() {
            return false;
        }
        let folded = text.to_lowercase();
        self.words.iter().all(|word| matches!(word, Word::Plain(word) if folded.contains(word.as_str())))
    }

    /// Where its words meet `name`, as its characters (from, to) in order: as they are written in any case, or spelt
    /// in pinyin. None where folding the name changes how many characters it has (they would not line up).
    fn marks(&self, name: &str) -> Vec<(usize, usize)> {
        let folded: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();
        if folded.len() != name.chars().count() {
            return Vec::new();
        }
        let text: String = folded.iter().collect();
        let mut marks = Vec::new();
        for word in &self.words {
            let Word::Plain(word) = word else { continue };
            if let Some(at) = text.find(word.as_str()) {
                let from = text[..at].chars().count();
                marks.push((from, from + word.chars().count()));
            } else if let Some(spelt) = readings::spelt(&folded, word.as_bytes()) {
                marks.push(spelt);
            }
        }
        marks.sort_unstable();
        marks
    }
}

/// Hits best first, the shorter path first of two as good.
pub fn sort(hits: &mut [Hit]) {
    hits.sort_by(|a, b| b.rank.total_cmp(&a.rank).then_with(|| a.path.as_os_str().len().cmp(&b.path.as_os_str().len())));
}

/// Every live entry but the root.
fn live(index: &Index) -> Vec<u32> {
    (1..index.len()).filter(|&place| index.entry(place).alive()).collect()
}

/// Whether a word can be a name spelt in pinyin: letters and figures alone.
fn spellable(word: &str) -> bool {
    word.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// Whether the match at byte `at` of `folded` starts a word in it: after a space or a mark, or a Chinese character.
fn starts_word(folded: &str, at: usize) -> bool {
    let before = folded[..at].chars().next_back();
    let here = folded[at..].chars().next();
    match (before, here) {
        (Some(before), Some(here)) => !before.is_alphanumeric() || readings::read(before) != readings::read(here),
        _ => true,
    }
}

/// Whether `name` (characters, folded) meets `pattern` whole: `*` any run of characters, `?` any one.
fn matches_pattern(name: &[char], pattern: &[char]) -> bool {
    let (mut n, mut p) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[n]) {
            n += 1;
            p += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, n));
            p += 1;
        } else if let Some((star_p, star_n)) = star {
            p = star_p + 1;
            n = star_n + 1;
            star = Some((star_p, star_n + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

/// Whether one of the folders `folder` is, or is within, holds `part` in its name (folded).
fn ancestors_hold(index: &Index, folder: u32, part: &str) -> bool {
    let mut at = folder;
    let mut steps = 0;
    while at != ROOT && steps < 4096 {
        if index.folded_name(at).contains(part) {
            return true;
        }
        at = index.entry(at).parent;
        steps += 1;
    }
    false
}

/// Whether `folder` is, or is within, one that `is` names, memoised in `known`.
fn within(index: &Index, folder: u32, known: &mut HashMap<u32, bool>, is: &dyn Fn(u32) -> bool) -> bool {
    let mut path = Vec::new();
    let mut at = folder;
    let found = loop {
        if let Some(&found) = known.get(&at) {
            break found;
        }
        if is(at) {
            break true;
        }
        if at == ROOT || path.len() > 4096 {
            break false;
        }
        path.push(at);
        at = index.entry(at).parent;
    };
    for folder in path {
        known.insert(folder, found);
    }
    found
}

/// How far from the user's own files the entry is: in Windows', programs', caches' or builds' folders, and deep.
fn away(index: &Index, place: u32) -> f32 {
    let mut away = 0.0;
    let mut depth = 0;
    let mut at = index.entry(place).parent;
    while at != ROOT && depth < 4096 {
        if AWAY.contains(&index.folded_name(at)) {
            away = 3.0;
        }
        depth += 1;
        at = index.entry(at).parent;
    }
    away + depth as f32 * 0.05
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_typed_is_read() {
        let query = Query::read("Glance ext:PNG,.jpg developer\\ *.md");
        assert_eq!(query.words, [Word::Plain("glance".into()), Word::Pattern("*.md".chars().collect())]);
        assert_eq!(query.kinds, ["png", "jpg"]);
        assert_eq!(query.within, ["developer"]);
    }

    #[test]
    fn patterns_meet_whole_names() {
        let meets = |name: &str, pattern: &str| matches_pattern(&name.chars().collect::<Vec<_>>(), &pattern.chars().collect::<Vec<_>>());
        assert!(meets("readme.md", "*.md"));
        assert!(meets("readme.md", "read??.md"));
        assert!(!meets("readme.md.bak", "*.md"));
        assert!(meets("a.b.md", "*.*.md"));
        assert!(meets("", "*"));
        assert!(!meets("readme", "?"));
    }

    /// C:\Docs\{Readme.md, 年度计划.docx, my readme.txt}, C:\Windows\readme.txt, C:\readme (a folder).
    fn drive() -> Index {
        use super::super::index::{Changes, Item};
        let item = |name: &str, id: u64, folder: bool| Item { name: name.encode_utf16().collect(), id, attributes: if folder { 0x10 } else { 0x20 }, tag: 0 };
        let mut index = Index::new(5, true);
        let mut changes = Changes::default();
        index.list(ROOT, &[item("Docs", 10, true), item("Windows", 11, true), item("readme", 12, true)], &[], &mut changes);
        index.list(1, &[item("Readme.md", 20, false), item("年度计划.docx", 21, false), item("my readme.txt", 22, false)], &[], &mut changes);
        index.list(2, &[item("readme.txt", 30, false)], &[], &mut changes);
        index.finish(changes);
        index
    }

    /// A path found, and where the words met its name.
    type Shown = (String, Vec<(usize, usize)>);

    fn found(typed: &str) -> (usize, Vec<Shown>) {
        found_as(Query::read(typed))
    }

    fn found_as(query: Query) -> (usize, Vec<Shown>) {
        let (total, _, hits) = query.find(&drive(), "C:\\", 10);
        (total, hits.into_iter().map(|hit| (hit.path.display().to_string(), hit.marks)).collect())
    }

    #[test]
    fn names_are_found_and_ranked() {
        assert_eq!(
            found("README"),
            (
                4,
                vec![
                    ("C:\\readme".into(), vec![(0, 6)]),
                    ("C:\\Docs\\Readme.md".into(), vec![(0, 6)]),
                    ("C:\\Docs\\my readme.txt".into(), vec![(3, 9)]),
                    ("C:\\Windows\\readme.txt".into(), vec![(0, 6)]),
                ]
            )
        );
        assert_eq!(found("jihua"), (1, vec![("C:\\Docs\\年度计划.docx".into(), vec![(2, 4)])]));
        assert_eq!(found("ndjh docx"), (1, vec![("C:\\Docs\\年度计划.docx".into(), vec![(0, 4), (5, 9)])]));
        assert_eq!(found("ext:txt,md").0, 3);
        assert_eq!(found("docs\\ readme").0, 2);
        assert_eq!(found("docs\\").0, 3);
        assert_eq!(found("read*.md"), (1, vec![("C:\\Docs\\Readme.md".into(), vec![])]));
        assert_eq!(found("readme zz").0, 0);
    }

    #[test]
    fn a_view_allows_what_it_shows() {
        assert_eq!(found_as(Query::read("readme").only(Only::Folders)).1, [("C:\\readme".into(), vec![(0, 6)])]);
        assert_eq!(found_as(Query::read("readme").only(Only::Files)).0, 3);
        assert_eq!(found_as(Query::read("readme").allowing(&["md", "pdf"])).1, [("C:\\Docs\\Readme.md".into(), vec![(0, 6)])]);
        // (What is typed and what the view allows, both.)
        assert_eq!(found_as(Query::read("ext:txt").allowing(&["md"])).0, 0);
        assert_eq!(found_as(Query::read("ext:txt,md").allowing(&["md", "docx"])).0, 1);
        // (Nothing typed, nothing is looked for, whatever the view.)
        assert!(Query::read("  ").allowing(&["md"]).is_empty());
        // A name holding two kinds is found once.
        let (total, folders, _) = Query::read("ext:txt,md").find(&drive(), "C:\\", 10);
        assert_eq!((total, folders), (3, 0));
        let (total, folders, hits) = Query::read("readme").find(&drive(), "C:\\", 1);
        assert_eq!((total, folders, hits.len()), (4, 1, 2), "the best folder and the best file");
    }

    #[test]
    fn other_names_are_met_as_files_are() {
        let meets = |typed: &str, name: &str| Query::read(typed).meets_name(name);
        assert_eq!(meets("wyy", "网易云音乐"), Some((6.0, vec![(0, 3)])));
        assert_eq!(meets("yinyue", "网易云音乐"), Some((4.0, vec![(3, 5)])));
        assert_eq!(meets("code", "Visual Studio Code"), Some((4.0, vec![(14, 18)])));
        assert_eq!(meets("STEAM", "Steam"), Some((8.0, vec![(0, 5)])));
        assert_eq!(meets("qq", "QQ音乐"), Some((6.0, vec![(0, 2)])));
        assert_eq!(meets("lanya", "蓝牙和其他设备"), Some((6.0, vec![(0, 2)])));
        assert_eq!(meets("note*", "Notepad"), Some((6.0, vec![])));
        assert!(meets("zz", "Steam").is_none());
        // (Asking for what only files have, no other name meets it.)
        assert!(meets("steam ext:exe", "Steam").is_none());
        assert!(meets("app\\ steam", "Steam").is_none());
    }
}
