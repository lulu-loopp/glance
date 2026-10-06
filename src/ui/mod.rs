//! The panel: how it is laid out and what it shows, on any platform, and how
//! each platform draws it (on Windows, Direct2D and DirectWrite on a
//! DirectComposition surface of the panel's own window).

pub mod arrange;
#[cfg(windows)]
pub mod backdrop;
pub mod canvas;
#[cfg(windows)]
pub mod gfx;
pub mod motion;
pub mod prefs;
pub mod seen;
#[cfg(windows)]
pub mod render;
#[cfg(windows)]
pub mod settings_window;
#[cfg(windows)]
pub mod skins;
pub mod text;
pub mod theme;
pub mod view;
#[cfg(windows)]
pub mod wallpaper;
#[cfg(windows)]
pub mod window;
