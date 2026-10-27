//! A name spelt in pinyin: each Chinese character met by the start of one of its Mandarin readings (the whole
//! syllable, its initial, or as much of it as was typed: jidu, jdjh, jidjh all meet 季度计划), every reading of a
//! character read more than one way, and any other character by itself.

use std::sync::OnceLock;

/// The first character the table has readings for, and the last.
const FIRST: u32 = 0x3400;
const LAST: u32 = 0x9FFF;

/// Every character's readings, from the Unicode Character Database (see scripts/readings.py).
struct Table {
    syllables: Vec<&'static [u8]>,
    /// Where each character's readings start in `readings`, and the end of the last.
    starts: Vec<u32>,
    readings: Vec<u16>,
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        static BYTES: &[u8] = include_bytes!("readings.bin");
        let count = u16::from_le_bytes([BYTES[0], BYTES[1]]) as usize;
        let mut at = 2;
        let mut syllables = Vec::with_capacity(count);
        for _ in 0..count {
            let len = BYTES[at] as usize;
            syllables.push(&BYTES[at + 1..at + 1 + len]);
            at += 1 + len;
        }
        let characters = (LAST - FIRST + 1) as usize;
        let mut starts = Vec::with_capacity(characters + 1);
        let mut sum = 0u32;
        for &count in &BYTES[at..at + characters] {
            starts.push(sum);
            sum += u32::from(count);
        }
        starts.push(sum);
        at += characters;
        let readings = BYTES[at..].chunks_exact(2).map(|two| u16::from_le_bytes([two[0], two[1]])).collect();
        Table { syllables, starts, readings }
    })
}

/// A character's readings, none for one the table does not have.
fn readings(c: char) -> impl Iterator<Item = &'static [u8]> {
    let table = table();
    let at = (c as u32).wrapping_sub(FIRST) as usize;
    let range = if at < table.starts.len() - 1 { table.starts[at] as usize..table.starts[at + 1] as usize } else { 0..0 };
    table.readings[range].iter().map(|&syllable| table.syllables[syllable as usize])
}

/// Whether `c` has readings (a Chinese character the table knows).
pub fn read(c: char) -> bool {
    readings(c).next().is_some()
}

/// Whether a typed letter meets a reading's (ü, written v, is met by u too).
fn meets(typed: u8, reading: u8) -> bool {
    typed == reading || (reading == b'v' && typed == b'u')
}

/// Where `term` (lower case letters and figures) meets `name` spelt so, from which character to which, each
/// character met whole: the match starting first, and of those the one ending first. None where it does not.
pub fn spelt(name: &[char], term: &[u8]) -> Option<(usize, usize)> {
    if term.is_empty() {
        return None;
    }
    // For each place in the term reached after the characters so far, the earliest character it was reached from.
    let mut reached: Vec<Option<usize>> = vec![None; term.len() + 1];
    let mut next: Vec<Option<usize>> = vec![None; term.len() + 1];
    for (at, &c) in name.iter().enumerate() {
        // (A match may start at any character.)
        reached[0] = Some(at);
        next.fill(None);
        for k in 0..term.len() {
            let Some(from) = reached[k] else { continue };
            let mut step = |to: usize| next[to] = Some(next[to].map_or(from, |was: usize| was.min(from)));
            let mut said = false;
            for reading in readings(c) {
                said = true;
                for (n, &letter) in reading.iter().enumerate() {
                    if k + n >= term.len() || !meets(term[k + n], letter) {
                        break;
                    }
                    step(k + n + 1);
                }
            }
            if !said && c.to_lowercase().eq(std::iter::once(term[k] as char)) {
                step(k + 1);
            }
        }
        if let Some(from) = next[term.len()] {
            return Some((from, at + 1));
        }
        std::mem::swap(&mut reached, &mut next);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spell(name: &str, term: &str) -> Option<(usize, usize)> {
        spelt(&name.chars().collect::<Vec<_>>(), term.as_bytes())
    }

    /// A name is met by its pinyin whole, its initials, both mixed, and the last syllable half typed; every reading
    /// of a character read more than one way; Latin letters in it by themselves.
    #[test]
    fn a_name_is_met_by_its_pinyin() {
        assert_eq!(spell("季度计划.docx", "jidu"), Some((0, 2)));
        assert_eq!(spell("季度计划.docx", "jdjh"), Some((0, 4)));
        assert_eq!(spell("季度计划.docx", "jidujh"), Some((0, 4)));
        assert_eq!(spell("季度计划.docx", "jidjh"), Some((0, 4)));
        assert_eq!(spell("年度计划", "jihua"), Some((2, 4)));
        assert_eq!(spell("网易云音乐", "yinyue"), Some((3, 5)));
        assert_eq!(spell("网易云音乐", "yinle"), Some((3, 5)));
        assert_eq!(spell("QQ音乐", "qqyy"), Some((0, 4)));
        assert_eq!(spell("绿色", "lvse"), Some((0, 2)));
        assert_eq!(spell("绿色", "luse"), Some((0, 2)));
        assert_eq!(spell("季度计划", "jiduz"), None);
        assert_eq!(spell("季度计划", "dujih"), Some((1, 4)));
        assert_eq!(spell("readme", "jidu"), None);
    }
}
