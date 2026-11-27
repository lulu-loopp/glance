use super::*;
use crate::search::index::{Changes, Item, Stat};

const FILE: u32 = 0x20;
const FOLDER: u32 = 0x10;
const HIDDEN_SYSTEM: u32 = 0x2 | 0x4;
const CLOUD: u32 = 0x40_0000;

const MB: u64 = 1 << 20;
const GB: u64 = 1 << 30;
/// Now, as a file time (late 2025), and a time two years before.
const NOW: i64 = 133_800_000_000_000_000;
const OLD: i64 = NOW - 2 * 365 * 864_000_000_000;

/// A thing a folder holds: a file (its size, when written, its attributes) or a folder (what it holds).
enum Thing {
    File(&'static str, u64, i64, u32),
    Folder(&'static str, Vec<Thing>),
}
use Thing::{File, Folder};

/// The index of a drive holding `things` at its root.
fn drive(things: Vec<Thing>) -> Index {
    let mut index = Index::new(1, true);
    let mut id = 10;
    let mut folders = vec![(ROOT, things)];
    while !folders.is_empty() {
        let mut items_of = Vec::new();
        let mut inner = Vec::new();
        for (folder, things) in folders.drain(..) {
            let mut items = Vec::new();
            for thing in things {
                id += 1;
                match thing {
                    File(name, size, written, attributes) => items.push(Item { name: name.encode_utf16().collect(), id, attributes, tag: 0, stat: Stat { written, size } }),
                    Folder(name, held) => {
                        items.push(Item { name: name.encode_utf16().collect(), id, attributes: FOLDER, tag: 0, stat: Stat::default() });
                        inner.push((folder, name, held));
                    }
                }
            }
            items_of.push((folder, items));
        }
        let before = index.children(&items_of.iter().map(|(folder, _)| *folder).collect());
        let mut changes = Changes::default();
        for (folder, items) in &items_of {
            index.list(*folder, items, before.get(folder).map_or(&[], Vec::as_slice), &mut changes);
        }
        index.finish(changes);
        for (folder, name, held) in inner {
            let mut place = None;
            index.each_in(folder, |at, entry| {
                if entry.folder() && index.name(at) == std::ffi::OsStr::new(name) {
                    place = Some(at);
                }
            });
            folders.push((place.unwrap(), held));
        }
    }
    index
}

fn pc() -> Index {
    drive(vec![
        Folder("Windows", vec![File("notepad.exe", MB, NOW, FILE), Folder("Temp", vec![File("log.txt", 100 << 10, NOW, FILE)]), Folder("System32", vec![File("kernel.dll", 50 * MB, OLD, FILE)])]),
        Folder(
            "Users",
            vec![Folder(
                "me",
                vec![
                    Folder("Videos", vec![File("a.mp4", 2 * GB, OLD, FILE)]),
                    Folder("Downloads", vec![File("setup.exe", 300 * MB, NOW, FILE), File("old.zip", 3 * GB / 2, OLD, FILE)]),
                    Folder("OneDrive", vec![File("cloud.docx", 10 * MB, NOW, FILE | CLOUD), File("local.pdf", 5 * MB, NOW, FILE)]),
                    Folder("AppData", vec![Folder("Local", vec![Folder("Temp", vec![File("t.tmp", 10 * MB, NOW, FILE)])])]),
                ],
            )],
        ),
        Folder("Games", vec![Folder("X", vec![File("data.bin", 4 * GB, NOW, FILE)])]),
        Folder("$Recycle.Bin", vec![Folder("S-1", vec![File("$R1.zip", 5 * GB, OLD, FILE)])]),
        File("pagefile.sys", 16 * GB, NOW, FILE | HIDDEN_SYSTEM),
    ])
}

fn places() -> Places {
    Places {
        windows: Some(PathBuf::from("C:\\Windows")),
        kept: ["C:\\Users", "C:\\Users\\me", "C:\\Users\\me\\Downloads", "C:\\Users\\me\\Videos", "C:\\Users\\me\\AppData\\Local"].iter().map(PathBuf::from).collect(),
        onedrive: vec![PathBuf::from("C:\\Users\\me\\OneDrive")],
    }
}

fn at(index: &Index, path: &str) -> u32 {
    index.place_of(Path::new(path)).unwrap()
}

/// What is listed: every file but those in the cloud alone.
const LISTED: u64 = MB + (100 << 10) + 50 * MB + 2 * GB + 300 * MB + 3 * GB / 2 + 5 * MB + 10 * MB + 4 * GB + 5 * GB + 16 * GB;

fn survey(index: &Index) -> Survey {
    Survey::take(index, "C:\\", &places(), (100 * GB, 100 * GB - LISTED - 3 * GB), NOW)
}

#[test]
fn a_path_on_no_drive_is_not_on_the_drive_whatever_its_characters() {
    assert_eq!(on_drive(Path::new("C:\\Users\\me"), "C:\\"), Some(Path::new("Users\\me")));
    assert_eq!(on_drive(Path::new("c:\\"), "C:\\"), Some(Path::new("")));
    // (Where the root would end, in the middle of a character.)
    assert_eq!(on_drive(Path::new("\\\\服务器\\share\\Documents"), "C:\\"), None);
    assert_eq!(on_drive(Path::new("D:\\x"), "C:\\"), None);
    assert_eq!(on_drive(Path::new("C"), "C:\\"), None);
}

#[test]
fn kinds_are_told_by_extension() {
    assert_eq!(kind_of("a.mp4"), Kind::Video);
    assert_eq!(kind_of("x.tar.gz"), Kind::Archive);
    assert_eq!(kind_of("main.ts"), Kind::Program);
    assert_eq!(kind_of(".gitignore"), Kind::Other);
    assert_eq!(kind_of("makefile"), Kind::Other);
    assert_eq!(extension_of("report.final.docx"), "docx");
    assert_eq!(extension_of("readme"), "");
    assert_eq!(extension_of("v1. draft"), "");
}

#[test]
fn a_drive_is_surveyed_whole_with_what_is_in_no_file_apart() {
    let index = pc();
    let survey = survey(&index);
    let root = survey.folder(ROOT).unwrap();
    assert_eq!(root.held.bytes, LISTED);
    assert_eq!(root.held.files, 11);
    assert_eq!(root.held.cloud, 10 * MB);
    assert_eq!(survey.other, 3 * GB);
    assert_eq!(root.held.written, NOW);
    let onedrive = survey.folder(at(&index, "Users\\me\\OneDrive")).unwrap();
    assert_eq!((onedrive.held.bytes, onedrive.held.cloud), (5 * MB, 10 * MB));
    assert!(survey.onedrive.contains(&at(&index, "Users\\me\\OneDrive")));
    assert!(!survey.onedrive.contains(&at(&index, "Users\\me")));
}

#[test]
fn what_is_kept_and_what_is_the_system_s_is_told_apart() {
    let index = pc();
    let survey = survey(&index);
    let role = |path: &str| survey.folder(at(&index, path)).unwrap().role;
    assert_eq!(role("Windows"), Role::Kept);
    assert_eq!(role("Windows\\System32"), Role::System);
    assert_eq!(role("Windows\\Temp"), Role::Plain);
    assert_eq!(role("Users\\me"), Role::Kept);
    assert_eq!(role("Users\\me\\Downloads"), Role::Kept);
    assert_eq!(role("Users\\me\\OneDrive"), Role::Kept);
    assert_eq!(role("Users\\me\\AppData\\Local\\Temp"), Role::Plain);
    assert_eq!(role("Games"), Role::Plain);
    assert_eq!(role("$Recycle.Bin"), Role::Bin);
    let guarded = |path: &str| survey.folder(at(&index, path)).unwrap().held.guarded;
    assert!(guarded("Users") && guarded("Windows") && guarded("$Recycle.Bin"));
    assert!(!guarded("Games") && !guarded("Windows\\Temp") && !guarded("Users\\me\\AppData\\Local\\Temp"));
    // Windows' files are the system's (but its Temp's), a game library's a game's, a system file the system's.
    let windows = survey.folder(at(&index, "Windows")).unwrap().held;
    assert_eq!(windows.kinds[Kind::System.at()], MB + 50 * MB);
    assert_eq!(windows.kinds[Kind::Document.at()], 100 << 10);
    assert_eq!(survey.folder(at(&index, "Games")).unwrap().held.kind(), Kind::Game);
    assert_eq!(survey.folder(ROOT).unwrap().held.kinds[Kind::System.at()], MB + 50 * MB + 16 * GB);
}

#[test]
fn the_largest_files_above_the_recycle_bin_are_not_in_it() {
    let index = pc();
    let survey = survey(&index);
    let root = survey.folder(ROOT).unwrap().held;
    // (The bin's 5 GB file, and the system's 16 GB, are none of them.)
    assert_eq!((root.largest, root.largest_old), (4 * GB, 2 * GB));
    let bin = survey.folder(at(&index, "$Recycle.Bin")).unwrap().held;
    assert_eq!(bin.largest, 5 * GB);
    assert!(survey.in_bin(at(&index, "$Recycle.Bin\\S-1")));
    assert!(!survey.in_bin(at(&index, "Games")));
    let filter = Filter { old: true, least: GB };
    assert!(filter.passes_within(root.largest, root.largest_old));
    assert!(!Filter { old: true, least: 3 * GB }.passes_within(root.largest, root.largest_old));
}

#[test]
fn a_tree_is_drawn_down_as_far_as_its_map_can_draw_it() {
    let index = pc();
    let survey = survey(&index);
    // About 31 MB a square DIP: what is smaller goes into its folder's rest, a folder under 25 times that is drawn as one.
    let tree = Tree::take(&index, &survey, ROOT, None, 1000.0);
    let root = tree.root().unwrap();
    assert_eq!(root.held.bytes, LISTED + 3 * GB);
    // (What is in no file listed is the system's, in the root's kinds as in its block's.)
    let unlisted = root.children.iter().find(|&&kid| tree.nodes[kid].what == What::Unlisted).map(|&kid| tree.nodes[kid].held.kinds[Kind::System.at()]).unwrap();
    assert_eq!(unlisted, 3 * GB);
    assert_eq!(root.held.kinds.iter().sum::<u64>(), root.held.bytes);
    let names: Vec<(&str, What)> = root.children.iter().map(|&kid| (tree.nodes[kid].name.as_str(), tree.nodes[kid].what)).collect();
    assert_eq!(
        names,
        [
            ("pagefile.sys", What::File { system: true }),
            ("$Recycle.Bin", What::Folder(Role::Bin)),
            ("Games", What::Folder(Role::Plain)),
            ("Users", What::Folder(Role::Kept)),
            ("", What::Unlisted),
            ("Windows", What::Folder(Role::Kept)),
        ]
    );
    let windows = tree.by_place[&at(&index, "Windows")];
    assert!(!tree.nodes[windows].opened && tree.nodes[windows].children.is_empty());
    let games = tree.by_place[&at(&index, "Games")];
    assert!(tree.nodes[games].opened);
    // Downloads' setup.exe is drawn, OneDrive's small file is in its rest.
    let downloads = tree.by_place[&at(&index, "Users\\me\\Downloads")];
    assert_eq!(tree.nodes[downloads].children.len(), 2);
    let me = at(&index, "Users\\me");
    let me_at = tree.by_place[&me];
    let rest = tree.rests[&me];
    assert_eq!(tree.nodes[rest].what, What::Rest);
    assert_eq!((tree.nodes[rest].items, tree.nodes[rest].only_files), (2, false));
    assert_eq!(*tree.nodes[me_at].children.last().unwrap(), rest);
    // Where each is drawn: itself, a folder drawn as one, a rest.
    let chain = |path: &str| survey.chain(at(&index, path));
    assert_eq!(tree.drawn_in(&chain("Games\\X")), Some(tree.by_place[&at(&index, "Games\\X")]));
    assert_eq!(tree.drawn_in(&chain("Windows\\System32")), Some(windows));
    assert_eq!(tree.drawn_in(&chain("Users\\me\\OneDrive")), Some(rest));
}

#[test]
fn the_lists_hold_what_passes_and_is_lit_never_the_system_s_nor_the_bin_s() {
    let index = pc();
    let survey = survey(&index);
    let tree = Tree::take(&index, &survey, ROOT, None, 1000.0);
    let names = |detail: &Detail| detail.rows.iter().map(|row| row.name.clone()).collect::<Vec<_>>();
    let all = Detail::take(&index, &survey, ROOT, Filter::default(), None, &tree);
    assert_eq!(names(&all)[..3], ["data.bin", "a.mp4", "old.zip"]);
    assert!(!names(&all).iter().any(|name| name == "pagefile.sys" || name == "$R1.zip" || name == "cloud.docx"));
    assert_eq!(all.rows[0].path, PathBuf::from("C:\\Games\\X\\data.bin"));
    // The lists by type and by time count the system's files too (not the bin's).
    assert_eq!(all.extensions[0].extension, "sys");
    assert_eq!(all.ages.iter().map(|age| age.0).sum::<u64>(), 10);

    let old_and_large = Detail::take(&index, &survey, ROOT, Filter { old: true, least: GB }, None, &tree);
    assert_eq!(names(&old_and_large), ["a.mp4", "old.zip"]);
    assert_eq!(old_and_large.passing, (2, 2 * GB + 3 * GB / 2));
    assert_eq!(old_and_large.ages.iter().map(|age| age.0).sum::<u64>(), 2);

    let zips = Detail::take(&index, &survey, ROOT, Filter::default(), Some(&Light::Extension("zip".into())), &tree);
    assert_eq!(names(&zips), ["old.zip"]);
    let lit: QuickSet<u32> = ["", "Users", "Users\\me"].iter().map(|path| if path.is_empty() { ROOT } else { at(&index, path) }).collect();
    assert!(lit.is_subset(&zips.lit_folders));
    assert!(zips.lit_rests.is_empty());
    let pdfs = Detail::take(&index, &survey, ROOT, Filter::default(), Some(&Light::Extension("pdf".into())), &tree);
    assert_eq!(pdfs.lit_rests, QuickSet::from_iter([at(&index, "Users\\me")]));
    // A row of the lists by type and by time lights the map as being lit by it would, among the folders drawn: read
    // with no light at all.
    let drawn = |lit: &QuickSet<u32>| lit.iter().copied().filter(|place| tree.by_place.contains_key(place)).collect::<QuickSet<u32>>();
    assert_eq!(all.by_extension["zip"].folders, drawn(&zips.lit_folders));
    assert_eq!(all.by_extension["zip"].rests, zips.lit_rests);
    assert_eq!(all.by_extension["pdf"].rests, pdfs.lit_rests);
    for age in 0..AGES {
        let aged = Detail::take(&index, &survey, ROOT, Filter::default(), Some(&Light::Age(age)), &tree);
        assert_eq!(all.by_age[age].folders, drawn(&aged.lit_folders), "{age}");
        assert_eq!(all.by_age[age].rests, aged.lit_rests, "{age}");
    }
    assert!(all.by_extension.keys().all(|extension| all.extensions.iter().any(|listed| listed.extension == *extension)));

    // Shown in the bin, what is in it is listed.
    let bin = at(&index, "$Recycle.Bin");
    let in_bin = Detail::take(&index, &survey, bin, Filter::default(), None, &Tree::take(&index, &survey, bin, None, 1000.0));
    assert_eq!(names(&in_bin), ["$R1.zip"]);
}

#[test]
fn a_folder_shown_from_past_one_of_what_it_holds_has_only_what_comes_after_it() {
    let index = pc();
    let survey = survey(&index);
    let rank = |path: &str| (survey.folder(at(&index, path)).unwrap().held.bytes, at(&index, path));
    // Past Games: Users, what is in no file listed, Windows (the page file, the bin and Games none of it).
    let games = rank("Games");
    let tree = Tree::take(&index, &survey, ROOT, Some(games), 1000.0);
    let root = tree.root().unwrap();
    let held = |path: &str| survey.folder(at(&index, path)).unwrap().held.bytes;
    assert_eq!(root.held.bytes, held("Users") + 3 * GB + held("Windows"));
    let names: Vec<&str> = root.children.iter().map(|&kid| tree.nodes[kid].name.as_str()).collect();
    assert_eq!(names, ["Users", "", "Windows"]);
    assert_eq!(count_past(&index, &survey, ROOT, games), (3, false));
    let detail = Detail::take(&index, &survey, ROOT, Filter::default(), None, &tree);
    assert_eq!(detail.rows.iter().map(|row| row.name.as_str()).take(3).collect::<Vec<_>>(), ["a.mp4", "old.zip", "setup.exe"]);
    // (Of Windows, the files in Temp alone: the system's are none to list.)
    assert_eq!(detail.passing.0, 6);
    assert!(!detail.extensions.iter().any(|extension| extension.extension == "bin" || extension.extension == "sys"));
    // Further past: what is in no file listed, and Windows.
    let users = rank("Users");
    let tree = Tree::take(&index, &survey, ROOT, Some(users), 1000.0);
    assert_eq!(tree.root().unwrap().children.iter().map(|&kid| tree.nodes[kid].what).collect::<Vec<_>>(), [What::Unlisted, What::Folder(Role::Kept)]);
    assert_eq!(count_past(&index, &survey, ROOT, users), (2, false));
    // Past what is in no file listed, a folder alone; in a folder, files alone.
    assert_eq!(count_past(&index, &survey, ROOT, (3 * GB, crate::search::index::NONE)), (1, false));
    let downloads = at(&index, "Users\\me\\Downloads");
    let mut old_zip = None;
    index.each_in(downloads, |place, _| {
        if index.name(place) == std::ffi::OsStr::new("old.zip") {
            old_zip = Some(place);
        }
    });
    assert_eq!(count_past(&index, &survey, downloads, (3 * GB / 2, old_zip.unwrap())), (1, true));
}

#[test]
fn ages_go_by_week_month_half_year_year_three_years() {
    let day = 864_000_000_000;
    assert_eq!(age_of(NOW - day, NOW), 0);
    assert_eq!(age_of(NOW - 20 * day, NOW), 1);
    assert_eq!(age_of(NOW - 100 * day, NOW), 2);
    assert_eq!(age_of(NOW - 300 * day, NOW), 3);
    assert_eq!(age_of(NOW - 800 * day, NOW), 4);
    assert_eq!(age_of(NOW - 2000 * day, NOW), 5);
}

/// The files met under a folder, by their names, each with its kind, whether it is the system's and whether it is in
/// the Recycle Bin (the bin's walked too or not).
fn walked(index: &Index, path: &str, bin: bool) -> Vec<(String, Kind, bool, bool)> {
    let mut met = Vec::new();
    let folder = if path.is_empty() { ROOT } else { at(index, path) };
    walk(index, "C:\\", &places(), folder, bin, &mut || true, &mut |file| met.push((index.name(file.place).to_string_lossy().into_owned(), file.is.kind, file.is.system, file.in_bin))).unwrap();
    met.sort_by(|a, b| a.0.cmp(&b.0));
    met
}

/// A walk tells each file as the survey does, wherever it starts: below Windows' own folder its files are the system's
/// (but its Temp's), below a game library's they are a game's. The Recycle Bin is walked only where asked, or where the
/// walk starts in it.
#[test]
fn a_walk_tells_files_by_the_folders_above_where_it_starts() {
    let index = pc();
    assert_eq!(walked(&index, r"Windows\System32", false), [("kernel.dll".to_string(), Kind::System, true, false)]);
    assert_eq!(walked(&index, r"Windows\Temp", false), [("log.txt".to_string(), Kind::Document, false, false)]);
    assert_eq!(walked(&index, r"Games\X", false), [("data.bin".to_string(), Kind::Game, false, false)]);
    let all = walked(&index, "", false);
    assert_eq!(all.len(), 11, "every file but the bin's (the one in the cloud alone among them)");
    assert!(!all.iter().any(|(name, ..)| name == "$R1.zip"));
    let with_bin = walked(&index, "", true);
    assert!(with_bin.contains(&("$R1.zip".to_string(), Kind::Archive, false, true)));
    assert_eq!(walked(&index, r"$Recycle.Bin\S-1", false), [("$R1.zip".to_string(), Kind::Archive, false, true)]);
}

/// A walk asked to stop stops, and tells it did: asked once a few thousand entries were met (the folder among them),
/// nothing more given after.
#[test]
fn a_walk_asked_to_stop_stops() {
    let things = (0..10_000).map(|_| File("a.txt", 1, NOW, FILE)).collect();
    let index = drive(vec![Folder("many", things)]);
    let mut asked = 0;
    let mut met = 0;
    assert!(walk(&index, "C:\\", &places(), ROOT, false, &mut || { asked += 1; false }, &mut |_| met += 1).is_none());
    assert_eq!((asked, met), (1, ASKED_EVERY as usize - 2));
}
