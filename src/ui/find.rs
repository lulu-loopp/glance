//! Search: a window called up anywhere by its shortcut (Win+Alt+Space unless another is chosen), or from the
//! workbench, that finds programs, Settings' pages, files and folders on every drive, and Glance's own notes and
//! what its station holds, by any part of their names (or their pinyin) as it is typed.

pub mod pages;
pub mod results;
mod window;

pub use window::{in_front, preview, toggle};
