//! What Glance asks of the system it runs on, each system answered its own
//! way: the user's language, whether their apps are dark, their accent.

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::*;

#[cfg(not(windows))]
mod unix;
#[cfg(not(windows))]
pub use self::unix::*;

/// `text` as a file's name may hold it on Windows: what a name may not hold left out (and spaces and dots where it
/// ends), at most `most` characters; empty where nothing is left, or what is left names a device (CON, NUL, COM1, a
/// name such as "con.txt" too).
pub fn file_name_of(text: &str, most: usize) -> String {
    let kept: String = text.chars().filter(|c| !NOT_IN_NAMES.contains(*c) && !c.is_control()).take(most).collect();
    let kept = kept.trim().trim_end_matches(['.', ' ']);
    if names_device(kept) {
        return String::new();
    }
    kept.to_string()
}

/// What a file's name may not hold on Windows (a control character neither).
const NOT_IN_NAMES: &str = r#"\/:*?"<>|"#;

/// Whether `name` names a device (CON, NUL, COM1, a name such as "con.txt" too): no file may be named so.
fn names_device(name: &str) -> bool {
    let device = name.split('.').next().unwrap_or_default().trim_end().to_uppercase();
    let numbered = |name: &str| device.strip_prefix(name).is_some_and(|number| matches!(number, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"));
    matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$") || numbered("COM") || numbered("LPT")
}

/// Why what was written may not be a file's name (see `name_written`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NameRefused {
    /// It holds what a name may not (see `NOT_IN_NAMES`).
    Holds,
    /// It names a device.
    Device,
}

/// `written` as a file's name, as Explorer takes a name written: its spaces at its ends, and dots where it ends, left
/// out (none where nothing is left); refused where it holds what a name may not, or names a device.
pub fn name_written(written: &str) -> Result<Option<String>, NameRefused> {
    let name = written.trim().trim_end_matches(['.', ' ']);
    if name.is_empty() {
        return Ok(None);
    }
    if name.chars().any(|c| NOT_IN_NAMES.contains(c) || c.is_control()) {
        return Err(NameRefused::Holds);
    }
    if names_device(name) {
        return Err(NameRefused::Device);
    }
    Ok(Some(name.to_string()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_name_holds_what_windows_lets_it() {
        assert_eq!(super::file_name_of("  a/b:c?.txt. . ", 60), "abc?.txt".replace('?', ""));
        assert_eq!(super::file_name_of("报告\u{7}「终稿」", 60), "报告「终稿」");
        assert_eq!(super::file_name_of("abcdef", 3), "abc");
        for device in ["CON", "nul.txt", "Com1.tar.gz", "lpt²", "aux .md", "CONIN$"] {
            assert_eq!(super::file_name_of(device, 60), "", "{device}");
        }
        for name in ["console.txt", "COM10", "con-notes.md", "LPT"] {
            assert_eq!(super::file_name_of(name, 60), name);
        }
        assert_eq!(super::file_name_of("...", 60), "");
    }

    /// A name written is taken as Explorer takes it: its ends' spaces and its last dots left out, nothing left none,
    /// what a name may not hold, or a device's name, refused.
    #[test]
    fn a_name_written_is_taken_as_explorer_takes_it() {
        use super::{name_written, NameRefused};
        assert_eq!(name_written("  周报.docx . "), Ok(Some("周报.docx".into())));
        assert_eq!(name_written(" .. "), Ok(None));
        assert_eq!(name_written("a/b.txt"), Err(NameRefused::Holds));
        assert_eq!(name_written("a\u{7}b"), Err(NameRefused::Holds));
        assert_eq!(name_written("nul.txt"), Err(NameRefused::Device));
        assert_eq!(name_written("console.txt"), Ok(Some("console.txt".into())));
    }
}
