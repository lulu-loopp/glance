//! How long finding every drive's names takes as the user (or reading them as kept), how much they hold, and how
//! long searching them takes (`cargo run --example search_index -- out.txt`, not elevated).

use glance_lib::search;
use std::fmt::Write;
use std::time::{Duration, Instant};

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "search-index.txt".into());
    let mut said = String::new();
    let began = Instant::now();
    search::start();
    std::thread::sleep(Duration::from_millis(200));
    loop {
        let standing = search::standing();
        if !standing.is_empty() && standing.iter().all(|drive| drive.ready) {
            for drive in &standing {
                writeln!(said, "{}: {} names", drive.root, drive.names).unwrap();
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    writeln!(said, "all ready in {:?}", began.elapsed()).unwrap();
    for typed in ["a", "readme", "glance", ".rs", "季度", "jidu", "ext:png", "*.md", "developer\\ rs", "zzqqxx"] {
        let began = Instant::now();
        let found = search::find(typed, 50);
        let took = began.elapsed();
        writeln!(said, "{typed:?}: {} found in {took:?}", found.total).unwrap();
        for hit in found.hits.iter().take(3) {
            writeln!(said, "  {} {:?} {:.2}", hit.path.display(), hit.marks, hit.rank).unwrap();
        }
    }
    search::keep_all();
    writeln!(said, "kept in {:?}", began.elapsed()).unwrap();
    std::fs::write(&out, said).unwrap();
}
