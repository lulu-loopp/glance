//! The panel: how it is laid out and what it shows, on any platform, and how
//! each platform draws it (on Windows, Direct2D and DirectWrite on a
//! DirectComposition surface of the panel's own window).

pub mod arrange;
#[cfg(windows)]
pub mod backdrop;
pub mod beside;
pub mod buttons;
pub mod canvas;
pub mod editor;
pub mod note;
#[cfg(windows)]
pub mod genie;
pub mod gfx;
pub mod life;
pub mod forms;
#[cfg(windows)]
pub mod glass;
pub mod icons;
#[cfg(windows)]
pub mod keys;
#[cfg(windows)]
pub mod menu;
pub mod morph;
pub mod motion;
pub mod overlay;
pub mod phone_art;
pub mod prefs;
pub mod seen;
#[cfg(windows)]
pub mod render;
pub mod scene;
#[cfg(windows)]
pub mod search;
#[cfg(windows)]
pub(crate) mod ask;
pub(crate) mod picture;
#[cfg(windows)]
pub mod music;
#[cfg(windows)]
pub mod settings_window;
#[cfg(windows)]
pub mod skins;
pub mod smoke;
pub mod text;
pub mod theme;
pub mod thumb;
#[cfg(windows)]
pub mod tip;
pub mod typing;
pub mod view;
#[cfg(windows)]
pub mod wallpaper;
#[cfg(windows)]
pub mod window;
