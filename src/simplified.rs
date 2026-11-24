//! Chinese text in traditional characters written in simplified ones, as OpenCC's own conversion (t2s) writes it: at
//! each place the longest of its phrases written there taken first, else the character, each as the first OpenCC
//! gives for it (`simplified.tsv`, made by `scripts/simplified.py` from OpenCC's tables, Apache License 2.0). A phrase
//! keeps its characters from being written otherwise one by one (乾隆 stays, 乾淨 is 干净). Every text keeps its number
//! of characters.

use std::collections::HashMap;
use std::sync::OnceLock;

/// OpenCC's phrases and characters, each as it is written in simplified characters, and how many characters the
/// longest has.
struct Table {
    written: HashMap<&'static str, &'static str>,
    longest: usize,
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        let written: HashMap<&str, &str> = include_str!("simplified.tsv").lines().filter_map(|line| line.split_once('\t')).collect();
        let longest = written.keys().map(|key| key.chars().count()).max().unwrap_or(1);
        Table { written, longest }
    })
}

/// `text` in simplified characters; none where it is so already.
pub fn simplified(text: &str) -> Option<String> {
    let table = table();
    // Where each character begins, and where the text ends.
    let starts: Vec<usize> = text.char_indices().map(|(at, _)| at).chain([text.len()]).collect();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at + 1 < starts.len() {
        let most = table.longest.min(starts.len() - 1 - at);
        let (taken, written) = (1..=most)
            .rev()
            .find_map(|length| table.written.get(&text[starts[at]..starts[at + length]]).map(|written| (length, *written)))
            .unwrap_or((1, &text[starts[at]..starts[at + 1]]));
        out.push_str(written);
        at += taken;
    }
    (out != text).then_some(out)
}

/// Whether `text` has kana (hiragana, katakana, their half-width forms): Japanese, whose kanji are not traditional
/// Chinese to be written otherwise.
pub fn kana(text: &str) -> bool {
    text.chars().any(|c| matches!(c, '\u{3040}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}' | '\u{FF66}'..='\u{FF9F}'))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Traditional characters in simplified ones, phrase before character (a report's 汽水音乐 lyrics, JXVZUN: "而我受過
    /// 的傷都是我的成長"), the same number of characters; what is simplified already, and letters, left as they are.
    #[test]
    fn writes_traditional_characters_as_simplified_ones() {
        assert_eq!(simplified("而我受過的傷都是我的成長").as_deref(), Some("而我受过的伤都是我的成长"));
        assert_eq!(simplified("Whoa 我會像奧德修斯一樣").as_deref(), Some("Whoa 我会像奥德修斯一样"));
        assert_eq!(simplified("乾淨的乾隆").as_deref(), Some("干净的乾隆"));
        assert_eq!(simplified("又站在你家的门口"), None);
        assert_eq!(simplified("Medals"), None);
        let long = "頭髮亂了 後來 著急";
        assert_eq!(simplified(long).map(|text| text.chars().count()), Some(long.chars().count()));
        assert!(kana("君の名は") && kana("ｶﾀｶﾅ") && !kana("我會像奧德修斯一樣"));
    }
}
