//! The disk organizer's pointer: what is under it, a press there (a click on a block or a row, a button's, a drag to
//! the place things are gathered in), the wheel over the list, and what each spot does.

use std::path::PathBuf;
use std::time::Instant;

use super::paint::{list_area, row_height, About};
use super::*;

/// How far a press moves before it is a drag (DIPs).
const DRAG_FROM: f32 = 5.0;

impl Organizer {
    pub fn moved(&mut self, x: f32, y: f32) {
        self.pointer = Some((x, y));
        self.next_frame = Instant::now();
        if let Some(drag) = &mut self.drag {
            drag.at = (x, y);
            return;
        }
        // The list's thumb held: the list scrolled as far as it is dragged.
        if let Some(Press { on: Pressed::Thumb(held), .. }) = self.press {
            let area = self.list_area();
            if let Some((thumb, most)) = self.thumb(area, self.list_count()) {
                let scrolled = ((y - held - area.y) / (area.h - thumb.h).max(1.0)).clamp(0.0, 1.0) * most;
                self.scroll = scrolled;
                self.scroll_to = scrolled;
            }
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

    pub fn left(&mut self) {
        self.pointer = None;
        self.hover = None;
        self.next_frame = Instant::now();
    }

    pub fn pressed(&mut self, x: f32, y: f32, double: bool) {
        self.pointer = Some((x, y));
        // (The block under the press itself: a press may come with no move to it before.)
        if self.menu.is_none() && self.plate.is_none() {
            self.hover = self.block_at(x, y);
        }
        self.next_frame = Instant::now();
        let target = self.hot();
        // A press off a menu or a card closes it (and does nothing else).
        if (self.menu.is_some() || self.plate.is_some()) && !matches!(target, Some(Target::Over | Target::MenuSize(_) | Target::MenuCrumb(_) | Target::MenuDrive(_) | Target::MenuUngather(_) | Target::PlateAct(_))) {
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
                // (A press on the map off every block: the file chosen let go of.)
                let Some(at) = self.hover else {
                    self.chosen = None;
                    self.press = None;
                    return;
                };
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
            Some(Target::Thumb) => {
                let area = self.list_area();
                let held = self.thumb(area, self.list_count()).map_or(0.0, |(thumb, _)| y - thumb.y);
                self.press = Some(Press { at: (x, y), on: Pressed::Thumb(held) });
            }
            Some(target) => self.press = Some(Press { at: (x, y), on: Pressed::Spot(target) }),
            None => self.press = None,
        }
    }

    pub fn released(&mut self, x: f32, y: f32) {
        self.pointer = Some((x, y));
        self.next_frame = Instant::now();
        if let Some(drag) = self.drag.take() {
            if self.over_foot(x, y) && drag.why_not.is_none() && self.pending.is_none() {
                self.gather(&drag.about);
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
            Pressed::Thumb(_) => {}
        }
    }

    pub fn wheel(&mut self, delta: i16) {
        let Some((x, y)) = self.pointer else { return };
        let area = self.list_area();
        // (A menu open over the pointer scrolled, as far as it goes: see `paint_menu`.)
        if self.menu.is_some() && self.menu_at.contains(x, y) {
            self.menu_scroll = (self.menu_scroll - delta as f32 / 120.0 * 32.0 * 2.0).max(0.0);
            self.next_frame = Instant::now();
        } else if area.contains(x, y) {
            self.scroll_to = (self.scroll_to - delta as f32 / 120.0 * row_height(self.tab) * 2.0).max(0.0);
            self.next_frame = Instant::now();
        }
    }

    /// Where the list's rows are drawn, for the window's size now.
    fn list_area(&self) -> Rect {
        let (width, height) = self.client();
        list_area(super::paint::geo(width, height).side)
    }

    fn start_drag(&mut self, press: &Press, at: (f32, f32)) {
        // (Nothing is dragged while what was gathered waits to go.)
        let about = match press.on {
            _ if self.pending.is_some() => None,
            Pressed::Block(block) => self.laid.get(block).map(|placed| self.about(&placed.shown, &placed.block.item)),
            Pressed::Row(row) => self.row_about(row),
            Pressed::Spot(_) | Pressed::Thumb(_) => None,
        };
        self.press = None;
        if let Some(about) = about {
            self.drag = Some(Drag { why_not: about.why_not.clone(), about, at });
            self.hover = None;
        }
    }

    /// What a row of the largest files is, as a block of it would be.
    pub(super) fn row_about(&self, at: usize) -> Option<About> {
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
            about.why_not = Some(lang.pick("已在待清理中", "Already queued").into());
        } else if let Some(holder) = self.gathered.iter().find(|gathered| gathered.folder && row.path.starts_with(&gathered.path)) {
            about.why_not = Some(match lang {
                Lang::Zh => format!("已包含在待清理的「{}」中", holder.name),
                Lang::En => format!("Already in “{}”, queued", holder.name),
            });
        }
        Some(about)
    }

    /// `about` gathered to go: what is gathered in it (a folder) no longer apart.
    fn gather(&mut self, about: &About) {
        if about.why_not.is_some() {
            return;
        }
        self.put_gathered(Gathered { path: about.path.clone(), name: about.name.clone(), bytes: about.bytes, folder: about.folder, kind: about.kind });
        self.went = None;
        self.next_frame = Instant::now();
    }

    /// A block clicked: a folder opened, a file chosen (and found in the list) or let go of again, a system item told
    /// of.
    fn click_block(&mut self, at: usize) {
        let Some(placed) = self.laid.get(at).cloned() else { return };
        let about = self.about(&placed.shown, &placed.block.item);
        if about.info.is_some() || matches!(about.what, Some(What::File { system: true })) {
            self.tell_of(&about);
            return;
        }
        self.clicked = matches!(about.what, Some(What::Folder(_) | What::File { .. })).then(|| about.path.clone());
        // Several too small to draw: what they are shown, from where they start in their folder's order on (each
        // way taken into the folder shown kept); all their folder holds, its own.
        if let Some((folder, last)) = &about.many_in {
            if *last {
                return;
            }
            let Item::Many { folder: node, .. } = &placed.block.item else { return };
            let after = match placed.block.item.bound() {
                Some(bound) if *node == 0 => [placed.shown.after.as_slice(), &[bound]].concat(),
                Some(bound) => vec![bound],
                None => Vec::new(),
            };
            self.change = Some(Change::In(placed.block.at));
            self.go_within(folder.clone(), after);
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
                if self.chosen.as_ref() == Some(&about.path) {
                    self.chosen = None;
                } else {
                    self.chosen = Some(about.path.clone());
                    self.find_in_list(&about.path);
                }
            }
            _ => {}
        }
    }

    /// The row of the file at `path` scrolled into view (to the nearer edge) where the largest files are listed.
    fn find_in_list(&mut self, path: &std::path::Path) {
        if self.tab != Tab::Largest {
            return;
        }
        let Some(at) = self.shown.as_ref().and_then(|shown| shown.detail.rows.iter().position(|row| row.path == path)) else { return };
        let area = self.list_area();
        let row_h = row_height(self.tab);
        let top = at as f32 * row_h;
        if top < self.scroll_to {
            self.scroll_to = top;
        } else if top + row_h > self.scroll_to + area.h {
            self.scroll_to = top + row_h - area.h;
        }
    }

    /// A row of the largest files chosen (the file chosen on the map too), or let go of again.
    fn choose_row(&mut self, at: usize) {
        if self.hot() != Some(Target::Row(at)) {
            return;
        }
        if let Some(path) = self.shown.as_ref().and_then(|shown| shown.detail.rows.get(at)).map(|row| row.path.clone()) {
            self.chosen = if self.chosen.as_ref() == Some(&path) { None } else { Some(path) };
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
            Target::DriveMore(hidden) => {
                let under = self.target_rect(&Target::DriveMore(hidden.clone()));
                self.menu = Some(Menu { of: MenuOf::Drives(hidden), under, above: false });
                self.menu_scroll = 0.0;
            }
            Target::Drive(at) | Target::MenuDrive(at) => {
                self.menu = None;
                if let Some(root) = self.drives.get(at).map(|drive| drive.root.clone()) {
                    if !root.eq_ignore_ascii_case(&self.root) {
                        self.to_drive(&root);
                    } else {
                        self.go_to(PathBuf::from(&root));
                    }
                }
            }
            Target::Back => self.up_a_folder(),
            Target::Crumb(step) | Target::MenuCrumb(step) => {
                self.menu = None;
                let after = if step.path == self.focus { self.after[..step.within.min(self.after.len())].to_vec() } else { Vec::new() };
                self.go_within(step.path, after);
            }
            Target::CrumbMore(hidden) => {
                let under = self.target_rect(&Target::CrumbMore(hidden.clone()));
                self.menu = Some(Menu { of: MenuOf::Crumbs(hidden), under, above: false });
                self.menu_scroll = 0.0;
            }
            Target::FilterOld => {
                self.filter.old = !self.filter.old;
                self.ask();
            }
            Target::FilterSize => {
                let under = self.target_rect(&Target::FilterSize);
                self.menu = Some(Menu { of: MenuOf::Sizes, under, above: false });
                self.menu_scroll = 0.0;
            }
            Target::FilterPreset => {
                let preset = self.filter.old && self.filter.least == 1 << 30;
                self.filter = if preset { Filter::default() } else { Filter { old: true, least: 1 << 30 } };
                self.ask();
            }
            Target::MenuSize(least) => {
                self.menu = None;
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
                    // (A type or a time lit by the list left: the map lit by them no longer.)
                    if matches!(self.light, Some(Light::Extension(_) | Light::Age(_))) {
                        self.light = None;
                        self.ask();
                    }
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
                // (The menu's places are the gathered's: taken out of it, it is opened again as the foot is drawn anew,
                // of those not shown then.)
                self.reopen_gathered = matches!(target, Target::MenuUngather(_));
                self.menu = None;
            }
            Target::GatheredMore(hidden) => {
                let under = self.target_rect(&Target::GatheredMore(hidden.clone()));
                self.menu = Some(Menu { of: MenuOf::Gathered(hidden), under, above: true });
                self.menu_scroll = 0.0;
            }
            Target::UngatherAll => self.gathered.clear(),
            Target::Commit => self.start_commit(),
            Target::Undo => self.undo(),
            Target::EmptyNow => {
                let foot = self.foot_at;
                self.plate = Some(Plate { of: PlateOf::Empty, at: (foot.x + foot.w - PLATE_W_OVER, foot.y - 8.0), above: true });
            }
            Target::PlateAct(act) => self.plate_act(act),
            Target::Row(_) | Target::Map | Target::Foot | Target::Over | Target::Thumb => {}
        }
    }

    /// Where a spot was last drawn.
    pub(super) fn target_rect(&self, target: &Target) -> Rect {
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
