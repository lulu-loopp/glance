//! How long reading every NTFS volume's files takes on this PC, how much they hold, and how long finding names in
//! them takes, on one thread and on all (run as administrator: `cargo run --example search_probe -- out.txt`).

use glance_lib::search::volume;
use std::fmt::Write;
use std::time::{Duration, Instant};

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "search-probe.txt".into());
    let mut said = String::new();
    let began = Instant::now();
    // Each volume read on its own thread, all at once.
    let read: Vec<_> = std::thread::scope(|scope| {
        let reads: Vec<_> = volume::ntfs_letters().into_iter().map(|letter| (letter, scope.spawn(move || volume::read(letter)))).collect();
        reads.into_iter().map(|(letter, read)| (letter, read.join().unwrap())).collect()
    });
    let all_in = began.elapsed();
    let mut volumes = Vec::new();
    for (letter, read) in read {
        match read {
            Ok(read) => {
                let directories = read.entries.iter().filter(|entry| entry.directory).count();
                let in_order = read.entries.windows(2).all(|two| two[0].id & ((1 << 48) - 1) < two[1].id & ((1 << 48) - 1));
                writeln!(
                    said,
                    "{letter}: {} entries ({directories} folders, {} written apart, in order {in_order}) in {:?}, {:.1} MB",
                    read.entries.len(),
                    read.written(),
                    read.read_in,
                    read.bytes() as f64 / 1e6
                )
                .unwrap();
                volumes.push(read);
            }
            Err(error) => writeln!(said, "{letter}: {error}").unwrap(),
        }
    }
    let bytes: usize = volumes.iter().map(|read| read.bytes()).sum();
    writeln!(said, "all read in {all_in:?}, {:.1} MB", bytes as f64 / 1e6).unwrap();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    for words in ["a", "readme", "glance", ".rs", "季度", "zzqqxx"] {
        let mut line = format!("{words:?}:");
        for threads in [1, 4, threads] {
            // The fastest of five.
            let (mut found, mut best) = (0, Duration::MAX);
            for _ in 0..5 {
                let began = Instant::now();
                found = volumes.iter().map(|read| read.find(words, threads).len()).sum::<usize>();
                best = best.min(began.elapsed());
            }
            write!(line, " {found} found in {best:?} on {threads} threads,").unwrap();
        }
        writeln!(said, "{}", line.trim_end_matches(',')).unwrap();
    }
    for read in &volumes {
        for place in read.find("glance.exe", threads).into_iter().take(3) {
            writeln!(said, "  {}", read.path(&read.entries[place as usize])).unwrap();
        }
    }
    std::fs::write(&out, said).unwrap();
}
