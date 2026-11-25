//! The disk organizer's pointer: what is under it, a press there (a click on a block or a row, a button's, a drag to
//! the place things are gathered in), the wheel over the list, and what each spot does.

use std::path::PathBuf;
use std::time::Instant;

use super::paint::{list_area, About, ROW_H};
use super::*;

/// How far a press moves before it is a drag (DIPs).
const DRAG_FROM: f32 = 5.0;

impl Ui {
    pub(super) fn moved(&mut self, x: f32, y: f32) {
        self.pointer = Some((x, y));
        self.next_frame = Instant::now();
        if let Some(drag) = &mut self.drag {
            drag.at = (x, y);
            return;
        }
        if let Some(press) = self.press.clone() {
            let (dx, dy) = (x - press.at.0, y - press.at.1);
            if dx * dx + dy * dy > DRAG_FROM * DRAG_FROM {
                self.start_drag(&press, (x, y));
                return;
            }
        }
        self.hover = if self.menu.is_none() && self.plate.is_none() { self.block_at(x, y) } else { None };
    }

    pub(super) fn left(&mut self) {
        self.pointer = None;
        self.hover = None;
        self.row_hover = None;
        self.next_frame = Instant::now();
    }

    pub(super) fn pressed(&mut self, x: f32, y: f32, double: bool) {
        self.pointer = Some((x, y));
        // (The block under the press itself: a press may come with no move to it before.)
        if self.menu.is_none() && self.plate.is_none() {
            self.hover = self.block_at(x, y);
        }
        self.next_frame = Instant::now();
        let target = self.hot();
        // A press off a menu or a card closes it (and does nothing else).
        if (self.menu.is_some() || self.plate.is_some()) && !matches!(target, Some(Target::Over | Target::MenuSize(_) | Target::MenuCrumb(_) | Target::MenuUngather(_) | Target::PlateAct(_))) {
            self.menu = None;
            self.plate = None;
            return;
        }
        match target {
            Some(Target::Map) => {
                // (The second press of a double-click shows what the first was on: that one may have opened a folder,
                // and what is under the pointer now is in it.)
                if double {
                    self.press = None;
                    match self.clicked.take() {
                        Some(path) => crate::disk::act::reveal(&[path]),
                        None => {
                            if let Some(at) = self.hover {
                                self.reveal_block(at);
                            }
                        }
                    }
                    return;
                }
                self.clicked = None;
                let Some(at) = self.hover else { return };
                self.press = Some(Press { at: (x, y), on: Pressed::Block(at) });
            }
            Some(Target::Row(at)) => {
                if double {
                    self.press = None;
                    if let Some(row) = self.shown.as_ref().and_then(|shown| shown.detail.rows.get(at)) {
                        crate::disk::act::reveal(std::slice::from_ref(&row.path));
                    }
                    return;
                }
                self.press = Some(Press { at: (x, y), on: Pressed::Row(at) });
            }
            Some(target) => self.press = Some(Press { at: (x, y), on: Pressed::Spot(target) }),
            None => self.press = None,
        }
    }

    pub(super) fn released(&mut self, x: f32, y: f32) {
        self.pointer = Some((x, y));
        self.next_frame = Instant::now();
        if let Some(drag) = self.drag.take() {
            if self.foot_at.contains(x, y) && drag.why_not.is_none() && self.pending.is_none() {
                let about = match drag.what {
                    Draggable::Block(at) => self.laid.get(at).map(|placed| self.about(&placed.shown, &placed.block.item)),
                    Draggable::Row(at) => self.row_about(at),
                };
                if let Some(about) = about {
                    self.gather(&about);
                }
            }
            self.press = None;
            return;
        }
        let Some(press) = self.press.take() else { return };
        match press.on {
            Pressed::Block(at) => self.click_block(at),
            Pressed::Row(at) => self.choose_row(at),
            Pressed::Spot(target) => {
                if self.hot() == Some(target.clone()) {
                    self.act(target);
                }
            }
        }
    }

    pub(super) fn wheel(&mut self, delta: i16) {
        let Some((x, y)) = self.pointer else { return };
        let (width, height) = self.client();
        let area = list_area(super::paint::geo(width, height).side);
        if area.contains(x, y) {
            self.scroll_to = (self.scroll_to - delta as f32 / 120.0 * ROW_H * 2.0).max(0.0);
            self.next_frame = Instant::now();
        }
    }

    fn start_drag(&mut self, press: &Press, at: (f32, f32)) {
        let about = match press.on {
            Pressed::Block(block) => self.laid.get(block).map(|placed| (Draggable::Block(block), self.about(&placed.shown, &placed.block.item))),
            Pressed::Row(row) => self.row_about(row).map(|about| (Draggable::Row(row), about)),
            Pressed::Spot(_) => None,
        };
        self.press = None;
        if let Some((what, about)) = about {
            self.drag = Some(Drag { what, why_not: about.why_not, at });
            self.hover = None;
        }
    }

    /// What a row of the largest files is, as a block of it would be.
    fn row_about(&self, at: usize) -> Option<About> {
        let row = self.shown.as_ref()?.detail.rows.get(at)?;
        let mut about = About {
            name: row.name.clone(),
            path: row.path.clone(),
            bytes: row.bytes,
            files: 1,
            cloud: 0,
            written: row.written,
            kind: row.kind,
            folder: false,
            what: Some(What::File { system: false }),
            info: None,
            onedrive: self.in_onedrive(&row.path),
            many_in: None,
            why_not: None,
        };
        let lang = self.lang;
        if self.gathered.iter().any(|gathered| gathered.path == row.path) {
            about.why_not = Some(lang.pick("已在待清理中", "Already gathered").into());
        } else if let Some(holder) = self.gathered.iter().find(|gathered| gathered.folder && row.path.starts_with(&gathered.path)) {
            about.why_not = Some(match lang {
                Lang::Zh => format!("已包含在待清理的「{}」中", holder.name),
                Lang::En => format!("Already in “{}”, gathered", holder.name),
            });
        }
        Some(about)
    }

    /// `about` gathered to go: what is gathered in it (a folder) no longer apart.
    fn gather(&mut self, about: &About) {
        if about.why_not.is_some() {
            return;
        }
        if about.folder {
            self.gathered.retain(|gathered| !gathered.path.starts_with(&about.path));
        }
        self.gathered.push(Gathered { path: about.path.clone(), name: about.name.clone(), bytes: about.bytes, folder: about.folder, kind: about.kind });
        self.went = None;
        self.next_frame = Instant::now();
    }

    /// A block clicked: a folder opened, a file chosen (and found in the list), a system item told of.
    fn click_block(&mut self, at: usize) {
        let Some(placed) = self.laid.get(at).cloned() else { return };
        let about = self.about(&placed.shown, &placed.block.item);
        if about.info.is_some() || matches!(about.what, Some(What::File { system: true })) {
            self.tell_of(&about);
            return;
        }
        self.clicked = matches!(about.what, Some(What::Folder(_) | What::File { .. })).then(|| about.path.clone());
        if let Some((folder, focus)) = &about.many_in {
            if !focus {
                self.go_to(folder.clone());
            }
            return;
        }
        match about.what {
            Some(What::Folder(_)) => self.go_to(about.path.clone()),
            Some(What::Rest) => {
                if about.path != self.focus {
                    self.go_to(about.path.clone());
                }
            }
            Some(What::File { .. }) => {
                self.chosen = Some(about.path.clone());
                self.find_in_list(&about.path);
            }
            _ => {}
        }
    }

    /// The row of the file at `path` shown in the list of the largest, where it is in it.
    fn find_in_list(&mut self, path: &std::path::Path) {
        let Some(at) = self.shown.as_ref().and_then(|shown| shown.detail.rows.iter().position(|row| row.path == path)) else { return };
        self.tab = Tab::Largest;
        let (width, height) = self.client();
        let area = list_area(super::paint::geo(width, height).side);
        let top = at as f32 * ROW_H;
        if top < self.scroll_to || top + ROW_H > self.scroll_to + area.h {
            self.scroll_to = (top - area.h / 2.0 + ROW_H / 2.0).max(0.0);
        }
    }

    /// A row of the largest files chosen: the file chosen on the map too.
    fn choose_row(&mut self, at: usize) {
        if self.hot() != Some(Target::Row(at)) {
            return;
        }
        if let Some(row) = self.shown.as_ref().and_then(|shown| shown.detail.rows.get(at)) {
            self.chosen = Some(row.path.clone());
        }
    }

    /// A block shown in Explorer.
    fn reveal_block(&mut self, at: usize) {
        let Some(placed) = self.laid.get(at).cloned() else { return };
        let about = self.about(&placed.shown, &placed.block.item);
        match (about.what, &about.many_in) {
            (Some(What::Unlisted), _) => {}
            (Some(What::Rest), _) | (_, Some(_)) => crate::disk::act::open_folder(&about.path),
            _ => crate::disk::act::reveal(&[about.path.clone()]),
        }
    }

    /// The card telling of a system item.
    fn tell_of(&mut self, about: &About) {
        let at = self.pointer.unwrap_or((self.map_at.x + self.map_at.w / 2.0, self.map_at.y + self.map_at.h / 2.0));
        let (bytes, files) = match about.info {
            Some(Info::Bin) => self.shown.as_ref().map_or((about.bytes, about.files), |shown| shown.bin),
            _ => (about.bytes, about.files),
        };
        self.plate = Some(Plate { of: PlateOf::System { info: about.info, name: about.name.clone(), path: about.path.clone(), bytes, files }, at: (at.0 + 8.0, at.1 + 8.0), above: false });
        self.next_frame = Instant::now();
    }

    /// The OneDrive file or folder `about` left in the cloud alone.
    fn release(&mut self, about: &About) {
        let path = about.path.clone();
        let bytes = about.bytes;
        super::act(self.hwnd, move || Acted::Released { bytes, marked: crate::disk::act::release(&path) > 0 });
    }

    /// What a spot pressed does.
    fn act(&mut self, target: Target) {
        self.next_frame = Instant::now();
        match target {
            Target::Drive(at) => {
                if let Some(root) = self.drives.get(at).map(|drive| drive.root.clone()) {
                    if !root.eq_ignore_ascii_case(&self.root) {
                        self.to_drive(&root);
                    } else {
                        self.go_to(PathBuf::from(&root));
                    }
                }
            }
            Target::Back => self.up_a_folder(),
            Target::Crumb(path) | Target::MenuCrumb(path) => {
                self.menu = None;
                self.go_to(path);
            }
            Target::CrumbMore(hidden) => {
                let under = self.target_rect(&Target::CrumbMore(hidden.clone()));
                self.menu = Some(Menu { of: MenuOf::Crumbs(hidden), under, above: false });
            }
            Target::FilterOld => {
                let preset = self.filter.old && self.filter.least == 1 << 30;
                if preset {
                    self.filter.least = 0;
                } else {
                    self.filter.old = !self.filter.old;
                }
                self.ask();
            }
            Target::FilterSize => {
                let under = self.target_rect(&Target::FilterSize);
                self.menu = Some(Menu { of: MenuOf::Sizes, under, above: false });
            }
            Target::FilterPreset => {
                let preset = self.filter.old && self.filter.least == 1 << 30;
                self.filter = if preset { Filter::default() } else { Filter { old: true, least: 1 << 30 } };
                self.ask();
            }
            Target::MenuSize(least) => {
                self.menu = None;
                // (Of the preset's two, the one asked for kept alone.)
                if self.filter.old && self.filter.least == 1 << 30 {
                    self.filter.old = false;
                }
                self.filter.least = least;
                self.ask();
            }
            Target::Legend(kind) => {
                self.light = if self.light == Some(Light::Kind(kind)) { None } else { Some(Light::Kind(kind)) };
                self.ask();
            }
            Target::Tab(tab) => {
                if self.tab != tab {
                    self.tab = tab;
                    self.scroll = 0.0;
                    self.scroll_to = 0.0;
                }
            }
            Target::TypeRow(extension) => {
                let light = Light::Extension(extension);
                self.light = if self.light.as_ref() == Some(&light) { None } else { Some(light) };
                self.ask();
            }
            Target::AgeRow(age) => {
                self.light = if self.light == Some(Light::Age(age)) { None } else { Some(Light::Age(age)) };
                self.ask();
            }
            Target::AddTool(at) => {
                let Some(placed) = self.laid.get(at).cloned() else { return };
                let about = self.about(&placed.shown, &placed.block.item);
                if about.info.is_some() || matches!(about.what, Some(What::File { system: true })) {
                    self.tell_of(&about);
                } else {
                    self.gather(&about);
                }
            }
            Target::ReleaseTool(at) => {
                if let Some(placed) = self.laid.get(at).cloned() {
                    let about = self.about(&placed.shown, &placed.block.item);
                    self.release(&about);
                }
            }
            Target::RowAdd(at) => {
                if let Some(about) = self.row_about(at) {
                    self.gather(&about);
                }
            }
            Target::RowRelease(at) => {
                if let Some(about) = self.row_about(at) {
                    self.release(&about);
                }
            }
            Target::AddAll => {
                let count = self.shown.as_ref().map_or(0, |shown| shown.detail.rows.len());
                for at in 0..count {
                    if let Some(about) = self.row_about(at) {
                        self.gather(&about);
                    }
                }
            }
            Target::Ungather(at) | Target::MenuUngather(at) => {
                if at < self.gathered.len() {
                    self.gathered.remove(at);
                }
                // (The menu's places are the gathered's: it closes as they move.)
                self.menu = None;
            }
            Target::GatheredMore(hidden) => {
                let under = self.target_rect(&Target::GatheredMore(hidden.clone()));
                self.menu = Some(Menu { of: MenuOf::Gathered(hidden), under, above: true });
            }
            Target::UngatherAll => self.gathered.clear(),
            Target::Commit => self.start_commit(),
            Target::Undo => self.undo(),
            Target::EmptyNow => {
                let foot = self.foot_at;
                self.plate = Some(Plate { of: PlateOf::Empty, at: (foot.x + foot.w - PLATE_W_OVER, foot.y - 8.0), above: true });
            }
            Target::PlateAct(act) => self.plate_act(act),
            Target::Row(_) | Target::Map | Target::Foot | Target::Over => {}
        }
    }

    /// Where a spot was last drawn.
    fn target_rect(&self, target: &Target) -> Rect {
        self.targets.iter().rev().find(|(_, spot)| spot == target).map_or(Rect::default(), |(at, _)| *at)
    }

    fn plate_act(&mut self, act: PlateAct) {
        let lang = self.lang;
        match act {
            PlateAct::Open(page) => {
                self.plate = None;
                crate::disk::act::open_page(page);
            }
            PlateAct::Hibernation => {
                let Some(plate) = &self.plate else { return };
                let bytes = match &plate.of {
                    PlateOf::System { bytes, .. } => *bytes,
                    _ => 0,
                };
                let (at, above) = (plate.at, plate.above);
                self.plate = Some(Plate { of: PlateOf::Hibernation { bytes }, at, above });
            }
            PlateAct::CopyCommand => {
                let copied = crate::clipboard::put(self.hwnd, &[(crate::clipboard::TEXT, crate::clipboard::utf16_z("powercfg /hibernate off"))]);
                if copied {
                    self.say(lang.pick("已复制命令", "Command copied").into(), crate::ui::icons::Icon::Copy);
                }
            }
            PlateAct::AskEmpty => {
                let Some(plate) = &self.plate else { return };
                let (at, above) = (plate.at, plate.above);
                self.plate = Some(Plate { of: PlateOf::Empty, at, above });
            }
            PlateAct::Empty => {
                self.plate = None;
                let root = self.root.clone();
                let bytes = self.shown.as_ref().map_or(0, |shown| shown.bin.0);
                super::act(self.hwnd, move || Acted::Emptied { bytes, done: crate::disk::act::empty_bin(&root) });
            }
            PlateAct::OpenBin => {
                self.plate = None;
                if let Some(bin) = self.shown.as_ref().and_then(|shown| shown.infos.iter().position(|info| *info == Some(Info::Bin)).map(|at| shown.paths[at].clone())) {
                    self.go_to(bin);
                }
            }
            PlateAct::Close => self.plate = None,
        }
        self.next_frame = Instant::now();
    }
}

/// How far left of the foot's right end the card asking to empty the Recycle Bin starts.
const PLATE_W_OVER: f32 = 320.0;
