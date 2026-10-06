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
