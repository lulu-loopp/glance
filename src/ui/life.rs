//! How long what Glance draws with lives: the same for every window that draws (the panel, a widget, the settings,
//! the overlay, the lyrics' windows, a dialog, a menu, a tip, where a widget would stick).
//!
//! - The device's own (one a thread for each adapter its windows' screens are driven by: see `gfx::on_screen`), its
//!   caches among them, live as long as the thread, or till no window of the thread draws on that adapter, and are
//!   given back as they fall due by the device itself: what painting layers took once they stay as they are, and
//!   everything once no window shows anything (see `Showing`).
//! - What a window draws with while it is shown (its surface, its layers, a bitmap of what is behind it) is let go of
//!   as it hides, and made again as it shows.
//! - What a window keeps for as long as it is there (the scene of what is behind it, see `scene`: each capture made
//!   is memory the system keeps till Glance ends) is kept while it is hidden, at rest, and let go of only as it goes,
//!   or as its device does.
//! - What one action needs (sizing by hand, a turn from one layout into another) goes as it ends (see
//!   `gfx::Gfx::settle`).

use std::rc::Rc;

use super::gfx::Gfx;

/// A window shown with a device: counted by it while held (see `gfx::Gfx::hide_one`).
pub struct Showing {
    gfx: Rc<Gfx>,
}

impl Showing {
    pub fn of(gfx: &Rc<Gfx>) -> Self {
        gfx.show_one();
        Showing { gfx: gfx.clone() }
    }
}

impl Drop for Showing {
    fn drop(&mut self) {
        self.gfx.hide_one();
    }
}

/// A window that draws: what it draws with while shown, and what it keeps for its life. Shown, hidden, gone, or
/// moved to another device, the same is done for every one.
pub trait Lives {
    /// The device it draws with now.
    fn device(&self) -> &Rc<Gfx>;
    /// Where it holds its being shown (see `Showing`).
    fn showing(&mut self) -> &mut Option<Showing>;
    /// Lets go of what it draws with while shown.
    fn let_go_of_shown(&mut self);
    /// Lets go of what it keeps for its life (what it keeps while hidden).
    fn let_go_of_kept(&mut self) {}

    /// It is shown (as it draws): counted as shown on its device, once.
    fn shown(&mut self) {
        if self.showing().is_none() {
            let showing = Showing::of(self.device());
            *self.showing() = Some(showing);
        }
    }

    /// It is hidden, to be shown again: what it draws with goes, what it keeps stays. (Not shown since it last was,
    /// it holds nothing to let go of.)
    fn hidden(&mut self) {
        if self.showing().is_none() {
            return;
        }
        self.let_go_of_shown();
        *self.showing() = None;
    }

    /// It is gone, or its device is (what it keeps was made on it): all of it goes.
    fn gone(&mut self) {
        self.let_go_of_shown();
        self.let_go_of_kept();
        *self.showing() = None;
    }
}
