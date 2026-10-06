//! What Glance asks of macOS and Linux for its look. The ports fill these
//! in as they come (AppKit's appearance and accent on macOS; the desktop's
//! settings portal on Linux); until then, what the environment says.

/// Whether the user's language is Chinese: the locale the session gives
/// programs, as LC_ALL, LC_MESSAGES and LANG are consulted, in that order.
pub fn speaks_chinese() -> bool {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
        .is_some_and(|locale| locale.starts_with("zh"))
}

/// Whether the user's apps are dark. Not asked yet: light.
pub fn apps_dark() -> bool {
    false
}

/// The user's accent colour. Not asked yet: none.
pub fn accent(_dark: bool) -> Option<u32> {
    None
}
