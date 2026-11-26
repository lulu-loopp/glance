//! The disk organizer drawn, as a page of the settings window (right of its pages' list): its title with the drive's
//! use, the drives as its tabs, the map and its legend, the lists beside it, the place what is to go is gathered in
//! under them, and what is over them (a block's tag, a card, a menu, a word said), as the mockup draws them.

use std::f32::consts::TAU;
use std::path::{Path, PathBuf};
use std::time::Instant;

use windows::Win32::Graphics::Direct2D::{D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_LAYER_PARAMETERS1, D2D1_ROUNDED_RECT};
use windows_numerics::Matrix3x2;

use super::*;
use crate::disk::{Role, AGES};
use crate::ui::canvas::{Align, Canvas, Color, Family, Fill, Font, Point};
use crate::ui::gfx::{rect, Frame};
use crate::ui::icons::Icon;

/// Where the page begins, as the settings' pages do (their list's width and the page's margin), and its rows from the
/// top: the title (the drive's use at its right), the drives' tabs, the way to the folder shown with the filters, the
/// cards.
pub(super) const PAGE_X: f32 = 236.0;
const TITLE_Y: f32 = 28.0;
const TABS_Y: f32 = 80.0;
const TABS_ROW: f32 = 48.0;
const TOOLS_Y: f32 = 136.0;
const CARDS_Y: f32 = 180.0;
pub(super) const SIDE_W: f32 = 272.0;
pub(super) const ROW_H: f32 = 44.0;
/// A row of the lists by type and by time: taller, its share of the largest under it.
const PICK_ROW_H: f32 = 50.0;
const TABS_H: f32 = 44.0;
const SUM_H: f32 = 28.0;
const CARD_RADIUS: f32 = 8.0;
const CHIP_H: f32 = 24.0;
const BUTTON_H: f32 = 32.0;
const PLATE_W: f32 = 320.0;
const TAG_W: f32 = 280.0;
/// How much of a block (or a piece of one) not lit is gone, dimmed wholly; how dimmed a ground is then (its words a
/// half as much as that).
const DIMS: f32 = 0.65;
const GROUND_DIM: f32 = 0.5;
/// A change of the drive's use its words count to (bytes): less is told at once.
const HALF_GB: f64 = (1u64 << 29) as f64;

fn body(size: f32, weight: f32) -> Font {
    Font::new(Family::Segoe, size, weight)
}

fn display(size: f32, weight: f32) -> Font {
    Font::new(Family::SegoeDisplay, size, weight)
}

/// Where the window's parts are, for its size.
#[derive(Clone, Copy, Debug)]
pub(super) struct Geo {
    pub width: f32,
    pub height: f32,
    pub page_w: f32,
    pub card: Rect,
    pub side: Rect,
    pub foot: Rect,
}

pub(super) fn geo(width: f32, height: f32) -> Geo {
    let page_w = (width - PAGE_X - 36.0).max(480.0);
    let card_h = (height - CARDS_Y - 96.0).max(220.0);
    Geo {
        width,
        height,
        page_w,
        card: Rect { x: PAGE_X, y: CARDS_Y, w: page_w - SIDE_W - 16.0, h: card_h },
        side: Rect { x: PAGE_X + page_w - SIDE_W, y: CARDS_Y, w: SIDE_W, h: card_h },
        foot: Rect { x: PAGE_X, y: height - 84.0, w: page_w, h: 56.0 },
    }
}

/// What the side list shows of its rows: where they start, how tall they may be.
pub(super) fn list_area(side: Rect) -> Rect {
    Rect { x: side.x + 6.0, y: side.y + TABS_H + SUM_H, w: side.w - 12.0, h: side.h - TABS_H - SUM_H - 6.0 }
}

/// How tall a row of the list `tab` is.
pub(super) fn row_height(tab: Tab) -> f32 {
    match tab {
        Tab::Largest => ROW_H,
        Tab::Types | Tab::Ages => PICK_ROW_H,
    }
}

/// The tools on the block under the pointer (its place in the layout): its plus (or the button telling of it, a
/// system item's), where it is; its button freeing it to the cloud; the room they take at a ground's name line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct BlockTools {
    pub at: usize,
    pub add: Option<(Rect, Icon)>,
    pub release: Option<Rect>,
    pub room: f32,
}

pub(super) fn kind_name(kind: Kind, lang: Lang) -> &'static str {
    match kind {
        Kind::Video => lang.pick("视频", "Videos"),
        Kind::Picture => lang.pick("图片", "Pictures"),
        Kind::Music => lang.pick("音乐", "Music"),
        Kind::Document => lang.pick("文档", "Documents"),
        Kind::Archive => lang.pick("压缩包", "Archives"),
        Kind::Program => lang.pick("程序", "Programs"),
        Kind::Game => lang.pick("游戏", "Games"),
        Kind::System => lang.pick("系统", "System"),
        Kind::Other => lang.pick("其他", "Other"),
    }
}

pub(super) fn kind_icon(kind: Kind) -> Icon {
    match kind {
        Kind::Video => Icon::Film,
        Kind::Picture => Icon::Image,
        Kind::Music => Icon::Music,
        Kind::Document => Icon::FileText,
        Kind::Archive => Icon::Archive,
        Kind::Program => Icon::AppWindow,
        Kind::Game => Icon::Game,
        Kind::System => Icon::Settings,
        Kind::Other => Icon::File,
    }
}

pub(super) fn age_name(age: usize, lang: Lang) -> &'static str {
    const ZH: [&str; AGES] = ["一周内", "一个月内", "半年内", "一年内", "三年内", "三年以上"];
    const EN: [&str; AGES] = ["Past week", "Past month", "Past six months", "Past year", "Past three years", "Over three years"];
    match lang {
        Lang::Zh => ZH[age],
        Lang::En => EN[age],
    }
}

/// What the organizer tells of a block (or of what a press or a drag is on): its name and path, its size, what it is,
/// and the reason it may not be gathered to go (none: it may).
#[derive(Clone, Debug)]
pub(super) struct About {
    pub name: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub files: u64,
    pub cloud: u64,
    pub written: i64,
    pub kind: Kind,
    pub folder: bool,
    pub what: Option<What>,
    pub info: Option<Info>,
    pub onedrive: bool,
    /// A block of several small things: the folder they are in (its path), and whether it is the one shown.
    pub many_in: Option<(PathBuf, bool)>,
    pub why_not: Option<String>,
}

impl Organizer {
    /// What a block of `shown` (its item `item`) is.
    pub(super) fn about(&self, shown: &Shown, item: &Item) -> About {
        let lang = self.lang;
        let tree = &shown.tree;
        let held = item.held(tree);
        let (node, many) = match item {
            Item::Node(node) => (*node, false),
            Item::Many { folder, .. } => (*folder, true),
        };
        let path = shown.paths[node].clone();
        let what = (!many).then_some(tree.nodes[node].what);
        let info = if many { None } else { shown.infos[node] };
        let (count, only_files) = item.count(tree);
        let name = if many {
            if only_files { format!("{} {}", count_text(count), lang.pick("个小文件", "small files")) } else { format!("{} {}", count_text(count), lang.pick("个小项目", "small items")) }
        } else {
            match (info, what) {
                (Some(Info::Unlisted), _) => lang.pick("其他系统数据", "Other system data").into(),
                (Some(Info::Bin), _) => lang.pick("回收站", "Recycle Bin").into(),
                (_, Some(What::Rest)) => {
                    let node = &tree.nodes[node];
                    if node.only_files { format!("{} {}", count_text(node.items), lang.pick("个小文件", "small files")) } else { format!("{} {}", count_text(node.items), lang.pick("个小项目", "small items")) }
                }
                _ => tree.nodes[node].name.clone(),
            }
        };
        let folder = matches!(what, Some(What::Folder(_)));
        let onedrive = !many && shown.onedrive[node];
        let many_in = many.then(|| (path.clone(), node == 0));
        let why_not = self.why_not(shown, node, many, what, info, &path, held.guarded);
        About { name, path, bytes: held.bytes, files: held.files, cloud: held.cloud, written: held.written, kind: held.kind(), folder, what, info, onedrive, many_in, why_not }
    }

    fn why_not(&self, shown: &Shown, node: usize, many: bool, what: Option<What>, info: Option<Info>, path: &Path, guarded: bool) -> Option<String> {
        let lang = self.lang;
        if many {
            return Some(lang.pick("若干小项目的合计，请在其所在的文件夹中逐个选择", "Several small items together: choose them one by one in their folder").into());
        }
        let said = |zh: &str, en: &str| Some(lang.pick(zh, en).to_string());
        match (what, info) {
            (_, Some(Info::Unlisted)) => return said("系统项目，不可删除", "Used by the system: it cannot be deleted"),
            (_, Some(Info::Bin)) => return said("回收站请使用「清空回收站」", "Empty the Recycle Bin instead"),
            (Some(What::Rest), _) => return said("若干小文件的合计，请选择其所在的文件夹", "Several small files together: choose the folder they are in"),
            _ => {}
        }
        if shown.binned[node] {
            return said("已在回收站中", "Already in the Recycle Bin");
        }
        match what {
            Some(What::File { system: true }) => return said("系统项目，不可删除", "Used by the system: it cannot be deleted"),
            Some(What::Folder(Role::Kept | Role::System | Role::Unread)) => return said("系统文件夹，不可删除", "A system folder: it cannot be deleted"),
            Some(What::Folder(_)) if guarded => return said("其中含有系统项目，不可整体删除", "It holds system items: it cannot be deleted whole"),
            _ => {}
        }
        if self.gathered.iter().chain(&self.going).any(|gathered| gathered.path == path) || self.pending.as_ref().is_some_and(|pending| pending.items.iter().any(|item| item.path == path)) {
            return said("已在待清理中", "Already gathered");
        }
        if let Some(holder) = self.gathered.iter().find(|gathered| gathered.folder && path.starts_with(&gathered.path)) {
            return Some(match lang {
                Lang::Zh => format!("已包含在待清理的「{}」中", holder.name),
                Lang::En => format!("Already in “{}”, gathered", holder.name),
            });
        }
        None
    }

    /// Whether a block's item is lit by what lights the map (`light`, peeked at or chosen) and the filter.
    fn lit(&self, shown: &Shown, item: &Item, light: Option<&Light>) -> bool {
        let tree = &shown.tree;
        let node_lit = |at: usize| -> bool {
            let node = &tree.nodes[at];
            match node.what {
                What::Unlisted => false,
                What::File { system } => {
                    (!self.filter.on() || self.filter.passes(node.held.bytes, node.held.written, system, shown.old_before)) && light.is_none_or(|light| light.lights(&node.folded, node.kind(), node.held.written, shown.at))
                }
                What::Folder(_) | What::Rest => match light {
                    // A kind alone (or one peeked at in the legend) lights what is mostly of it.
                    Some(Light::Kind(kind)) if !self.filter.on() || self.peek.is_some() => node.kind() == *kind && (!self.filter.on() || self.lit_place(shown, at, light)),
                    _ => self.lit_place(shown, at, light),
                },
            }
        };
        match item {
            Item::Node(at) => node_lit(*at),
            Item::Many { members, .. } => members.iter().any(|&member| node_lit(member)),
        }
    }

    /// Whether the folder (or the rest) at `at` holds a file lit by `light` that passes the filter: read with the
    /// lists, for what lights the map (or for a type's row or a time's peeked at, apart from it).
    fn lit_place(&self, shown: &Shown, at: usize, light: Option<&Light>) -> bool {
        let node = &shown.tree.nodes[at];
        let rest = node.what == What::Rest;
        let peeked = match light {
            Some(Light::Extension(extension)) if light != self.light.as_ref() => Some(shown.detail.by_extension.get(extension)),
            Some(Light::Age(age)) if light != self.light.as_ref() => Some(shown.detail.by_age.get(*age)),
            _ => None,
        };
        match peeked {
            Some(lit) => lit.is_some_and(|lit| if rest { lit.rests.contains(&node.place) } else { lit.folders.contains(&node.place) }),
            None if rest => shown.detail.lit_rests.contains(&node.place),
            None => shown.detail.lit_folders.contains(&node.place),
        }
    }

    /// Whether the file or folder at `path` is gathered to go, or in a folder that is (or is about to go).
    fn gathered_at(&self, path: &Path) -> bool {
        let pending = self.pending.iter().flat_map(|pending| pending.items.iter());
        self.gathered.iter().chain(pending).chain(&self.going).any(|gathered| path == gathered.path || (gathered.folder && under(path, &gathered.path)))
    }

    /// The spot under the pointer as last drawn.
    pub(super) fn hot(&self) -> Option<Target> {
        let (x, y) = self.pointer?;
        self.targets.iter().rev().find(|(at, _)| at.contains(x, y)).map(|(_, target)| target.clone())
    }

    fn spot(&mut self, at: Rect, target: Target) {
        self.targets.push((at, target));
    }

    /// How far the look `look` of `target` has faded to `on` (0 to 1), over `lasts` seconds on CSS's own ease, as the
    /// mockup's transitions do: kept from frame to frame while the spot is drawn. A spot's look under the pointer comes
    /// up from none; whether it is on is as it is where the spot is first drawn.
    fn fade(&mut self, target: &Target, look: Look, on: bool, lasts: f32) -> f32 {
        let to = if on { 1.0 } else { 0.0 };
        if crate::panel::reduced_motion() {
            return to as f32;
        }
        let now = self.now;
        match self.fades.iter_mut().find(|(spot, of, _, _)| spot == target && *of == look) {
            Some((_, _, tween, seen)) => {
                *seen = true;
                tween.go(to, now);
                self.fading |= tween.moving(now);
                tween.at(now) as f32
            }
            None if look == Look::On => {
                self.fades.push((target.clone(), look, Tween::new(to, lasts, EASE), true));
                to as f32
            }
            None if on => {
                let mut tween = Tween::new(0.0, lasts, EASE);
                tween.go(1.0, now);
                self.fades.push((target.clone(), look, tween, true));
                self.fading = true;
                0.0
            }
            None => 0.0,
        }
    }

    /// Everything drawn; whether anything moves still.
    pub(super) fn paint(&mut self, frame: &Frame, now: Instant, dt: f32) -> bool {
        let palette = self.palette.clone();
        let (width, height) = self.client();
        let g = geo(width, height);
        let hot = self.hot();
        self.now = now;
        self.fading = false;
        for fade in &mut self.fades {
            fade.3 = false;
        }
        // The legend's kind under the pointer lights the map while it is there, as a type's row or a time's does.
        self.peek = match &hot {
            Some(Target::Legend(kind)) => Some(Light::Kind(*kind)),
            Some(Target::TypeRow(extension)) => Some(Light::Extension(extension.clone())),
            Some(Target::AgeRow(age)) => Some(Light::Age(*age)),
            _ => None,
        };
        self.targets.clear();
        let mut moving = self.paint_head(frame, &palette, g, now);
        if self.root.is_empty() {
            self.paint_no_drive(frame, &palette, g);
        } else {
            moving |= self.paint_drives(frame, &palette, g, now, &hot);
            self.paint_tools(frame, &palette, g, &hot);
            moving |= self.paint_map_card(frame, &palette, g, now, dt, &hot);
            moving |= self.paint_side(frame, &palette, g, dt, &hot);
            moving |= self.paint_foot(frame, &palette, g, now, &hot);
            moving |= self.paint_over(frame, &palette, g, now, &hot);
        }
        // (The looks of spots no longer drawn let go of.)
        self.fades.retain(|fade| fade.3);
        moving || self.fading
    }

    fn paint_no_drive(&mut self, frame: &Frame, p: &Palette, g: Geo) {
        let lang = self.lang;
        let top = g.height / 2.0 - 30.0;
        frame.icon(Icon::Disk, Point { x: PAGE_X + g.page_w / 2.0, y: top }, 28.0, p.text3);
        centred(frame, lang.pick("没有可以显示的磁盘", "No drive to show"), body(16.0, 600.0), p.text, PAGE_X, top + 28.0, g.page_w);
        centred(frame, lang.pick("磁盘整理读取搜索的索引，开启搜索后即可在这里查看磁盘。", "The organizer reads search's index: drives are shown here once search is on."), body(13.0, 400.0), p.text2, PAGE_X, top + 56.0, g.page_w);
    }

    /// The page's title, as the settings' pages have theirs, and at its right the use of the drive shown: its words
    /// counting to a change of half a GB or more, its meter's parts sliding to theirs. Whether they move still.
    fn paint_head(&mut self, frame: &Frame, p: &Palette, g: Geo, now: Instant) -> bool {
        let lang = self.lang;
        let title_font = display(28.0, 600.0);
        frame.text(lang.pick("磁盘整理", "Disk Organizer"), title_font, p.text, PAGE_X, frame.line_in(title_font, TITLE_Y, 44.0), g.page_w - 300.0, Align::Start);
        let Some(drive) = self.drives.iter().find(|drive| drive.root.eq_ignore_ascii_case(&self.root)).cloned() else { return false };
        let used = drive.total.saturating_sub(drive.free);
        let bin = self.shown.as_ref().filter(|shown| shown.root == drive.root).map_or(0, |shown| shown.bin.0);
        let pick: u64 = self.gathered.iter().map(|item| item.bytes).sum::<u64>() + self.pending.as_ref().map_or(0, |pending| pending.items.iter().map(|item| item.bytes).sum());
        // The meter's parts: what is used but for the Recycle Bin (what is gathered at its end), the Recycle Bin's.
        let parts = [used.saturating_sub(bin), pick, bin];
        if self.meter_of != drive.root || crate::panel::reduced_motion() {
            self.meter_of = drive.root.clone();
            self.used_told.set(used as f64);
            for (tween, part) in self.meter.iter_mut().zip(parts) {
                tween.set(part as f64);
            }
        } else {
            if (used as f64 - self.used_told.to).abs() < HALF_GB {
                self.used_told.set(used as f64);
            } else {
                self.used_told.go(used as f64, now);
            }
            for (tween, part) in self.meter.iter_mut().zip(parts) {
                tween.go(part as f64, now);
            }
        }
        let moving = self.used_told.moving(now) || self.meter.iter().any(|tween| tween.moving(now));
        let told = self.used_told.at(now).max(0.0) as u64;
        let block_w = 500.0f32.min(g.page_w - 320.0).max(240.0);
        let right = PAGE_X + g.page_w;
        let mut runs: Vec<(String, bool)> = vec![(lang.pick("已用 ", "Used ").into(), false), (size_text(told, lang), true), (format!("{}{}{}{}", lang.pick("，共 ", ", of "), size_text(drive.total, lang), lang.pick("，可用 ", ", free "), size_text(drive.total.saturating_sub(told), lang)), false)];
        if bin > 0 {
            runs.push((format!("{}{}", lang.pick("，回收站 ", ", Recycle Bin "), size_text(bin, lang)), false));
        }
        if pick > 0 {
            runs.push((format!("{}{}", lang.pick("，待清理 ", ", gathered "), size_text(pick, lang)), false));
        }
        let font = |bold: bool| body(12.0, if bold { 600.0 } else { 400.0 });
        let widths: Vec<f32> = runs.iter().map(|(text, bold)| frame.measure(text, font(*bold))).collect();
        let mut x = right - widths.iter().sum::<f32>();
        let y = frame.line_in(font(false), 30.0, 18.0);
        for ((text, bold), w) in runs.iter().zip(&widths) {
            frame.text(text, font(*bold), if *bold { p.text } else { p.text2 }, x, y, w + 2.0, Align::Start);
            x += w;
        }
        // The meter: what is used, what is gathered striped at its end, the Recycle Bin's striped after it.
        let bar = Rect { x: right - block_w, y: 56.0, w: block_w, h: 6.0 };
        fill(frame, p.meter, bar.x, bar.y, bar.w, bar.h, 3.0);
        if drive.total > 0 {
            let of = |bytes: f64| (bytes / drive.total as f64).clamp(0.0, 1.0) as f32 * bar.w;
            let [own, gathered, binned] = self.meter.map(|tween| tween.at(now));
            let used_w = of(own);
            let pick_w = of(gathered).min(used_w);
            let bin_w = of(binned).min(bar.w - used_w);
            within(frame, bar, 3.0, 1.0, || {
                fill(frame, p.accent, bar.x, bar.y, used_w, bar.h, 0.0);
                if pick_w > 0.0 {
                    let r = Rect { x: bar.x + used_w - pick_w, w: pick_w, ..bar };
                    let _clip = frame.clipped(r.x, r.y, r.w, r.h);
                    fill(frame, p.meter, r.x, r.y, r.w, r.h, 0.0);
                    stripes(frame, r, p.accent);
                }
                if bin_w > 0.0 {
                    let r = Rect { x: bar.x + used_w, w: bin_w, ..bar };
                    let _clip = frame.clipped(r.x, r.y, r.w, r.h);
                    stripes(frame, r, p.text3);
                }
            });
        }
        moving
    }

    /// The drives, a tab each, as the settings' own tabs are: the one shown marked under its middle by a short bar in
    /// the accent (going to it as it is chosen), a drive nearly out of space by a dot after its name (told of under the
    /// pointer). Those that do not fit in a menu, the one shown never among them. Whether the mark moves still.
    fn paint_drives(&mut self, frame: &Frame, p: &Palette, g: Geo, now: Instant, hot: &Option<Target>) -> bool {
        const BETWEEN: f32 = 28.0;
        const DOT: f32 = 12.0;
        const MORE: f32 = 32.0;
        let font = |shown: bool| body(14.0, if shown { 600.0 } else { 400.0 });
        let drives = self.drives.clone();
        let chosen = drives.iter().position(|drive| drive.root.eq_ignore_ascii_case(&self.root));
        let names: Vec<String> = drives.iter().map(|drive| format!("{} ({}:)", drive.label, drive.letter())).collect();
        // (Each as wide as it is shown, bold: none moves as another is chosen.)
        let widths: Vec<f32> = names.iter().zip(&drives).map(|(name, drive)| frame.measure(name, font(true)) + if drive.low() { DOT } else { 0.0 }).collect();
        let room = g.page_w;
        let mut shown: Vec<usize> = (0..drives.len()).collect();
        let taken = |shown: &[usize], more: bool| shown.iter().map(|&at| widths[at] + BETWEEN).sum::<f32>() + if more { MORE } else { 0.0 };
        while shown.len() > 1 && taken(&shown, shown.len() < drives.len()) > room {
            // (The last that is not the one shown goes into the menu.)
            let Some(last) = shown.iter().rposition(|&at| Some(at) != chosen) else { break };
            shown.remove(last);
        }
        let hidden: Vec<usize> = (0..drives.len()).filter(|at| !shown.contains(at)).collect();
        let h = TABS_ROW - 8.0;
        let cy = TABS_Y + h / 2.0;
        let mut x = PAGE_X;
        let mut middle = None;
        for &at in &shown {
            let is_shown = Some(at) == chosen;
            let target = Target::Drive(at);
            let r = Rect { x: x - 12.0, y: TABS_Y, w: widths[at] + 24.0, h };
            let lit = self.fade(&target, Look::Hot, *hot == Some(target.clone()) && !is_shown, 0.12);
            if lit > 0.0 {
                fill(frame, p.hover.alpha(lit), r.x, r.y + 4.0, r.w, r.h - 8.0, 6.0);
            }
            let f = font(is_shown);
            let name_w = frame.measure(&names[at], f);
            frame.text(&names[at], f, if is_shown { p.text } else { p.text2 }, x, frame.line_in(f, TABS_Y, h), name_w + 4.0, Align::Start);
            if drives[at].low() {
                frame.fill_circle(p.signal, Point { x: x + name_w + DOT / 2.0 + 2.0, y: cy }, 3.0);
            }
            if is_shown {
                middle = Some(x + name_w / 2.0);
            }
            self.spot(r, target);
            x += widths[at] + BETWEEN;
        }
        if !hidden.is_empty() {
            let r = Rect { x: x - 12.0, y: TABS_Y + 4.0, w: MORE, h: h - 8.0 };
            let target = Target::DriveMore(hidden.clone());
            let lit = self.fade(&target, Look::Hot, *hot == Some(target.clone()), 0.12);
            if lit > 0.0 {
                fill(frame, p.hover.alpha(lit), r.x, r.y, r.w, r.h, 6.0);
            }
            frame.icon(Icon::Ellipsis, Point { x: r.x + r.w / 2.0, y: cy }, 16.0, p.text2);
            // (A drive among them nearly out of space: its dot on the button too.)
            if hidden.iter().any(|&at| drives[at].low()) {
                frame.fill_circle(p.signal, Point { x: r.x + r.w - 6.0, y: r.y + 6.0 }, 3.0);
            }
            self.spot(r, target);
        }
        // The mark goes to the drive chosen, as the settings' own tabs' does.
        let Some(middle) = middle else { return false };
        let mark = self.drive_mark.get_or_insert_with(|| Mark::new(middle));
        mark.go(middle, now);
        let (from, to) = mark.at(now);
        fill(frame, p.accent, from, TABS_Y + h - 2.0 - 3.0, to - from, 3.0, 1.5);
        mark.moving(now)
    }

    /// Back, the path to the folder shown, and the filters.
    fn paint_tools(&mut self, frame: &Frame, p: &Palette, g: Geo, hot: &Option<Target>) {
        let lang = self.lang;
        let top = TOOLS_Y;
        let right = PAGE_X + g.page_w;
        // The filters from the right: each pressed while its own filter is on (the preset's two by it too).
        let preset = self.filter.old && self.filter.least == 1 << 30;
        let buttons: [(Target, Icon, String, bool, bool); 3] = [
            (Target::FilterOld, Icon::Clock, lang.pick("超过一年未修改", "Not changed in a year").into(), self.filter.old, false),
            (Target::FilterSize, Icon::BySize, least_text(self.filter.least, lang), self.filter.least > 0, true),
            (Target::FilterPreset, Icon::Archive, lang.pick("大且旧的文件", "Large and old").into(), preset, false),
        ];
        let font = body(13.0, 400.0);
        let mut x = right;
        let mut placed = Vec::new();
        for (target, icon, label, on, menu) in buttons.iter().rev() {
            let w = 11.0 + 14.0 + 8.0 + frame.measure(label, font) + if *menu { 8.0 + 12.0 } else { 0.0 } + 11.0;
            x -= w;
            placed.push((Rect { x, y: top, w, h: BUTTON_H }, target.clone(), *icon, label.clone(), *on, *menu));
            x -= 8.0;
        }
        let tools_left = x;
        for (r, target, icon, label, on, menu) in placed.into_iter().rev() {
            let lit = self.fade(&target, Look::Hot, *hot == Some(target.clone()), 0.12);
            let pressed = self.fade(&target, Look::On, on, 0.12);
            button(frame, p, r, 6.0, pressed, 1.06, lit);
            let ink = mix(p.text, p.on_accent, pressed);
            frame.icon(icon, Point { x: r.x + 11.0 + 7.0, y: r.y + r.h / 2.0 }, 14.0, ink);
            frame.text(&label, font, ink, r.x + 11.0 + 14.0 + 8.0, frame.line_in(font, r.y, r.h), r.w, Align::Start);
            if menu {
                frame.icon(Icon::ChevronDown, Point { x: r.x + r.w - 11.0 - 6.0, y: r.y + r.h / 2.0 }, 12.0, mix(p.text2, p.on_accent, pressed));
            }
            self.spot(r, target);
        }
        // Back (faint where there is no folder above), then the path.
        let back = Rect { x: PAGE_X, y: top, w: 32.0, h: 32.0 };
        let can_go_up = !self.at_root();
        let usable = self.fade(&Target::Back, Look::On, can_go_up, 0.2);
        let lit = self.fade(&Target::Back, Look::Hot, can_go_up && *hot == Some(Target::Back), 0.12);
        if lit > 0.0 {
            fill(frame, p.hover.alpha(lit), back.x, back.y, back.w, back.h, 6.0);
        }
        frame.icon(Icon::ArrowLeft, Point { x: back.x + 16.0, y: back.y + 16.0 }, 16.0, p.text.alpha(0.36 + 0.64 * usable));
        if can_go_up {
            self.spot(back, Target::Back);
        }
        let Some(shown) = self.shown.clone() else { return };
        let crumbs = shown.crumbs.clone();
        let room = tools_left - 16.0 - (back.x + back.w + 8.0);
        let crumb_font = |last: bool| body(14.0, if last { 600.0 } else { 400.0 });
        let width_of = |at: usize| -> f32 {
            let last = at + 1 == crumbs.len();
            (frame.measure(&crumbs[at].0, crumb_font(last)) + 12.0).min(if last { 340.0 } else { 150.0 })
        };
        const SEPARATOR: f32 = 16.0;
        const MORE: f32 = 32.0;
        // As many from the end as fit, the root kept where it can be; those between in a menu.
        let mut shown_at: Vec<usize> = Vec::new();
        let mut used = 0.0;
        for at in (0..crumbs.len()).rev() {
            let w = width_of(at) + if shown_at.is_empty() { 0.0 } else { SEPARATOR };
            let hidden_left = at > 0;
            if used + w + if hidden_left { MORE + SEPARATOR } else { 0.0 } > room && !shown_at.is_empty() {
                break;
            }
            used += w;
            shown_at.push(at);
        }
        shown_at.reverse();
        let first_shown = shown_at.first().copied().unwrap_or(0);
        let mut x = back.x + back.w + 8.0;
        if first_shown > 0 {
            let hidden: Vec<(String, PathBuf)> = crumbs[..first_shown].iter().rev().cloned().collect();
            let r = Rect { x, y: top, w: MORE, h: 32.0 };
            let target = Target::CrumbMore(hidden);
            let lit = self.fade(&target, Look::Hot, *hot == Some(target.clone()), 0.12);
            if lit > 0.0 {
                fill(frame, p.hover.alpha(lit), r.x, r.y, r.w, r.h, 6.0);
            }
            frame.icon(Icon::Ellipsis, Point { x: r.x + 16.0, y: r.y + 16.0 }, 16.0, mix(p.text2, p.text, lit));
            self.spot(r, target);
            x += MORE;
            frame.icon(Icon::ChevronRight, Point { x: x + SEPARATOR / 2.0, y: top + 16.0 }, 12.0, p.text3);
            x += SEPARATOR;
        }
        for (n, &at) in shown_at.iter().enumerate() {
            let last = at + 1 == crumbs.len();
            let w = width_of(at);
            let r = Rect { x, y: top, w, h: 32.0 };
            let target = Target::Crumb(crumbs[at].1.clone());
            let lit = self.fade(&target, Look::Hot, !last && *hot == Some(target.clone()), 0.12);
            if lit > 0.0 {
                fill(frame, p.hover.alpha(lit), r.x, r.y, r.w, r.h, 6.0);
            }
            let font = crumb_font(last);
            frame.text(&crumbs[at].0, font, if last { p.text } else { mix(p.text2, p.text, lit) }, r.x + 6.0, frame.line_in(font, r.y, r.h), r.w - 12.0, Align::Start);
            if !last {
                self.spot(r, target);
            }
            x += w;
            if n + 1 < shown_at.len() {
                frame.icon(Icon::ChevronRight, Point { x: x + SEPARATOR / 2.0, y: top + 16.0 }, 12.0, p.text3);
                x += SEPARATOR;
            }
        }
    }

    /// The map's card: the map, and its legend under it (each kind a button, pressed where it lights the map). Whether
    /// it moves still.
    fn paint_map_card(&mut self, frame: &Frame, p: &Palette, g: Geo, now: Instant, dt: f32, hot: &Option<Target>) -> bool {
        let lang = self.lang;
        card(frame, p, g.card);
        let shown = self.shown.clone().filter(|shown| shown.root.eq_ignore_ascii_case(&self.root));
        let Some(shown) = shown else {
            let words = if self.listing { lang.pick("正在读取此盘的文件…", "Reading this drive's files…") } else { lang.pick("正在读取…", "Reading…") };
            centred(frame, words, body(13.0, 400.0), p.text2, g.card.x, frame.line_in(body(13.0, 400.0), g.card.y, g.card.h), g.card.w);
            return false;
        };
        // The legend: each kind the folder shown holds, the largest first, in as many lines as it takes.
        let root = &shown.tree.nodes[0].held;
        let mut kinds: Vec<(Kind, u64)> = crate::disk::KINDS.iter().map(|&kind| (kind, root.kinds[kind.at()])).filter(|(_, bytes)| *bytes > 0).collect();
        kinds.sort_by(|a, b| b.1.cmp(&a.1));
        let chip_font = body(12.0, 400.0);
        let legend_x = g.card.x + 6.0;
        let legend_w = g.card.w - 12.0;
        let mut lines: Vec<Vec<(Kind, String, String, f32)>> = vec![Vec::new()];
        let mut line_w = 0.0;
        for (kind, bytes) in kinds {
            let name = kind_name(kind, lang).to_string();
            let value = size_text(bytes, lang);
            let w = 8.0 + 10.0 + 6.0 + frame.measure(&name, chip_font) + 6.0 + frame.measure(&value, chip_font) + 8.0;
            if line_w + w > legend_w && !lines.last().unwrap().is_empty() {
                lines.push(Vec::new());
                line_w = 0.0;
            }
            line_w += w + 2.0;
            lines.last_mut().unwrap().push((kind, name, value, w));
        }
        let legend_h = lines.len() as f32 * CHIP_H + (lines.len() as f32 - 1.0) * 4.0;
        let map = Rect { x: g.card.x + 12.0, y: g.card.y + 12.0, w: g.card.w - 24.0, h: (g.card.h - 12.0 - 8.0 - legend_h - 8.0).max(40.0) };
        let lighting = self.peek.clone().or(self.light.clone());
        let mut y = map.y + map.h + 8.0;
        for line in lines {
            let mut x = legend_x;
            for (kind, name, value, w) in line {
                let r = Rect { x, y, w, h: CHIP_H };
                let target = Target::Legend(kind);
                let lit = self.fade(&target, Look::Hot, *hot == Some(target.clone()), 0.12);
                let pressed = self.fade(&target, Look::On, self.light == Some(Light::Kind(kind)), 0.12);
                button(frame, p, r, 4.0, pressed, 1.0, lit);
                let swatch = Rect { x: r.x + 8.0, y: r.y + 7.0, w: 10.0, h: 10.0 };
                fill(frame, p.fill[kind.at()], swatch.x, swatch.y, swatch.w, swatch.h, 3.0);
                if pressed < 1.0 {
                    stroke_inside(frame, swatch, 3.0, Color::hex(0, 0.1 * (1.0 - pressed)));
                }
                if pressed > 0.0 {
                    Canvas::stroke_rounded(frame, Fill::Solid(p.on_accent.alpha(pressed)), swatch.x - 1.0, swatch.y - 1.0, swatch.w + 2.0, swatch.h + 2.0, 4.0, 1.0);
                }
                let name_w = frame.measure(&name, chip_font);
                let line_y = frame.line_in(chip_font, r.y, r.h);
                frame.text(&name, chip_font, mix(p.text, p.on_accent, pressed), r.x + 24.0, line_y, name_w + 2.0, Align::Start);
                frame.text(&value, chip_font, mix(p.text2, p.on_accent, pressed), r.x + 24.0 + name_w + 6.0, line_y, r.w, Align::Start);
                self.spot(r, target);
                x += w + 2.0;
            }
            y += CHIP_H + 4.0;
        }
        // Laid out anew for a map of another size (at once: a window being sized is not a change of what it shows).
        if (map.w - self.laid_size.0).abs() > 0.5 || (map.h - self.laid_size.1).abs() > 0.5 {
            let area_before = self.laid_size.0 * self.laid_size.1;
            self.laid_size = (map.w, map.h);
            self.relay(None);
            if area_before > 0.0 && ((map.w * map.h) / area_before - 1.0).abs() > 0.25 {
                self.ask();
            }
        }
        self.map_at = map;
        self.spot(map, Target::Map);
        // The tools on the block under the pointer (while the map is still and nothing is dragged), and the room they
        // take at a ground's name.
        let tools = match (self.hover, &self.drag, &self.anim) {
            (Some(at), None, None) => self.block_tools(frame, map, at),
            _ => None,
        };
        let moving = self.paint_map(frame, p, map, now, dt, lighting.as_ref(), hot);
        self.paint_block_tools(frame, p, now, tools.as_ref(), hot) || moving
    }

    /// The map's blocks, as they are now (moving, if they do): a block becoming a folder's ground going in (or a ground
    /// a block going out) crossing from the one look to the other; each block, and each piece of one, dimmed as the
    /// filters and what lights the map say (eased), a ground's words with it; what is gathered striped. Once the map
    /// is still, the file chosen outlined, and the block under the pointer (or the file whose row is). Whether anything
    /// moves still.
    #[allow(clippy::too_many_arguments)]
    fn paint_map(&mut self, frame: &Frame, p: &Palette, map: Rect, now: Instant, dt: f32, lighting: Option<&Light>, hot: &Option<Target>) -> bool {
        let lang = self.lang;
        let blocks = self.now_rects(now);
        let animating = self.anim.as_ref().is_some_and(|anim| now < anim.began + anim.lasts);
        if !animating {
            self.anim = None;
        }
        let radius = crate::disk::map::RADIUS;
        let lights = self.filter.on() || lighting.is_some();
        let k = 1.0 - (-dt * 1000.0 / 70.0).exp();
        let mut dims_moving = false;
        frame.clip(map.x - 2.0, map.y - 2.0, map.w + 4.0, map.h + 4.0);
        let hover_key = self.hover.and_then(|at| self.laid.get(at)).map(|placed| placed.key).filter(|_| !animating);
        let any_gathered = !self.gathered.is_empty() || !self.going.is_empty() || self.pending.is_some();
        // The tools up on a block (coming or going), how far they are seen: a ground's size gives way to its plus.
        let tools_up = self.tools_shown.as_ref().and_then(|shown| self.laid.get(shown.what.at).map(|placed| (placed.key, shown.shown(now, 0.12, EASE), shown.what)));
        for seen in blocks {
            let Seen { placed, at, alpha, ground, was } = seen;
            let r = Rect { x: map.x + at.x, y: map.y + at.y, w: at.w, h: at.h };
            if r.w < 0.5 || r.h < 0.5 || alpha <= 0.01 {
                continue;
            }
            // How it looks as a block: itself, or (a ground now, going in) what it was before.
            let block = if placed.block.group { was.as_ref().filter(|was| !was.block.group).unwrap_or(&placed) } else { &placed };
            let shown = &block.shown;
            let tree = &shown.tree;
            let item = &block.block.item;
            let kind = item.kind(tree);
            let dim_to = if !lights {
                0.0
            } else if ground > 0.5 {
                GROUND_DIM
            } else if self.lit(shown, item, lighting) {
                0.0
            } else {
                1.0
            };
            let dim = eased(&mut self.dims, placed.key, dim_to, k, &mut dims_moving);
            let pieces = &block.block.pieces;
            let mut piece_dims = Vec::with_capacity(pieces.len());
            for piece in pieces {
                let to = if lights && !self.lit(shown, &piece.item, lighting) { 1.0 } else { 0.0 };
                piece_dims.push(eased(&mut self.dims, key_of(shown, &piece.item), to, k, &mut dims_moving));
            }
            let hovered = hover_key == Some(placed.key);
            let as_block = alpha * (1.0 - ground);
            if as_block > 0.01 {
                let about = self.about(shown, item);
                let gathered = !matches!(about.what, Some(What::Unlisted)) && about.many_in.is_none() && self.gathered_at(&about.path);
                let piece_rect = |at: &Rect| Rect { x: r.x + at.x * r.w, y: r.y + at.y * r.h, w: at.w * r.w, h: at.h * r.h };
                // What is gathered among its pieces (the block not gathered whole): striped as the block would be, each
                // folder split into pieces outlined with all it holds, as deep as it is.
                let (mut marked, mut outlined): (Vec<Rect>, Vec<Rect>) = (Vec::new(), Vec::new());
                if any_gathered && !gathered {
                    let mut taken: Vec<&Path> = Vec::new();
                    let mut mark = |item: &Item, at: &Rect, fold: bool| {
                        let Item::Node(node) = item else { return };
                        let path = shown.paths[*node].as_path();
                        if taken.iter().any(|top| under(path, top)) || !self.gathered_at(path) {
                            return;
                        }
                        taken.push(path);
                        marked.push(piece_rect(at));
                        if fold {
                            outlined.push(piece_rect(at));
                        }
                    };
                    for fold in &block.block.folds {
                        mark(&fold.item, &fold.at, true);
                    }
                    for piece in pieces {
                        mark(&piece.item, &piece.at, false);
                    }
                }
                let seen_as = 1.0 - dim * DIMS;
                within(frame, r, radius, as_block, || {
                    if pieces.is_empty() {
                        let fills = if hovered { &p.fill_hot } else { &p.fill };
                        fill(frame, fills[kind.at()].alpha(seen_as), r.x, r.y, r.w, r.h, 0.0);
                    } else {
                        fill(frame, p.ring, r.x, r.y, r.w, r.h, 0.0);
                        for (piece, piece_dim) in pieces.iter().zip(&piece_dims) {
                            let pr = piece_rect(&piece.at);
                            fill(frame, p.fill[piece.item.kind(tree).at()].alpha(1.0 - piece_dim * DIMS), pr.x, pr.y, pr.w, pr.h, 0.0);
                        }
                        if hovered {
                            fill(frame, p.hover, r.x, r.y, r.w, r.h, 0.0);
                        }
                    }
                    if gathered {
                        stripes(frame, r, p.stripe);
                    }
                    for at in &marked {
                        let _clip = frame.clipped(at.x, at.y, at.w, at.h);
                        stripes(frame, *at, p.stripe);
                    }
                    // Its name and size, where there is room.
                    if r.w >= 36.0 && r.h >= 18.0 {
                        let name_font = body(12.0, 600.0);
                        let size = size_text(about.bytes, lang);
                        if r.h >= 42.0 && r.w >= 56.0 {
                            frame.text(&about.name, name_font, p.text.alpha(seen_as), r.x + 8.0, r.y + 6.0, r.w - 14.0, Align::Start);
                            frame.text(&size, body(12.0, 400.0), p.text2.alpha(seen_as), r.x + 8.0, r.y + 22.0, r.w - 14.0, Align::Start);
                        } else {
                            frame.text(&about.name, name_font, p.text.alpha(seen_as), r.x + 6.0, frame.line_in(name_font, r.y, r.h), r.w - 10.0, Align::Start);
                        }
                    }
                });
                if gathered {
                    frame.stroke_rounded_dashed(p.text2.alpha(as_block), r.x, r.y, r.w, r.h, radius, 1.0);
                }
                for at in &outlined {
                    frame.stroke_rounded_dashed(p.text.alpha(as_block * seen_as * 0.7), at.x, at.y, at.w, at.h, 0.0, 1.0);
                }
            }
            let as_ground = alpha * ground;
            if as_ground > 0.01 {
                // A ground: its name above what it holds, its size at the right where there is room (and the tools are
                // not up), quiet under the pointer.
                let about = self.about(&placed.shown, &placed.block.item);
                fill(frame, p.ground.alpha(as_ground), r.x, r.y, r.w, r.h, radius);
                if hovered {
                    fill(frame, p.hover.alpha(as_ground), r.x, r.y, r.w, r.h, radius);
                }
                if r.h >= 20.0 && r.w >= 40.0 {
                    let font = body(12.0, 600.0);
                    let size_font = body(12.0, 400.0);
                    let size = size_text(about.bytes, lang);
                    let size_w = frame.measure(&size, size_font);
                    let room = r.w - 16.0;
                    let (tools_seen, keep, plus) = match tools_up {
                        Some((key, seen, tools)) if key == placed.key => (seen, tools.room, tools.add.is_some()),
                        _ => (0.0, 0.0, false),
                    };
                    let show_size = room - size_w - 8.0 > 40.0;
                    let size_seen = if plus { 1.0 - tools_seen } else { 1.0 };
                    let name_w = if show_size && size_seen > 0.0 { room - size_w - 8.0 } else { room };
                    let name_w = if tools_seen > 0.0 { name_w.min(room - keep) } else { name_w };
                    let seen_as = as_ground * (1.0 - dim * 0.5);
                    let y = frame.line_in(font, r.y, crate::disk::map::LABEL);
                    frame.text(&about.name, font, p.text.alpha(seen_as), r.x + 8.0, y, name_w, Align::Start);
                    if show_size && size_seen > 0.0 {
                        frame.text(&size, size_font, p.text2.alpha(seen_as * size_seen), r.x + 8.0, y, room, Align::End);
                    }
                }
                if !matches!(about.what, Some(What::Unlisted)) && self.gathered_at(&about.path) {
                    within(frame, r, radius, as_ground, || stripes(frame, r, p.stripe));
                    frame.stroke_rounded_dashed(p.text2.alpha(as_ground), r.x, r.y, r.w, r.h, radius, 1.0);
                }
            }
        }
        // Once still: the file chosen, ringed in the accent; the block under the pointer (not a ground), or the file
        // whose row is under it, outlined.
        if !animating {
            let at_map = |e: Rect| Rect { x: map.x + e.x, y: map.y + e.y, ..e };
            let chosen = self.chosen.clone();
            if let Some((e, piece)) = chosen.as_deref().and_then(|path| self.exact_rect(path)) {
                outline(frame, at_map(e), piece, p.accent, 2.0, Some(p.ring));
            }
            let hovered = match self.hover.and_then(|at| self.laid.get(at)) {
                Some(placed) => (!placed.block.group).then(|| (placed.block.at, false, self.about(&placed.shown, &placed.block.item).path)),
                None => match hot {
                    Some(Target::Row(at) | Target::RowAdd(at) | Target::RowRelease(at)) if self.tab == Tab::Largest => {
                        let path = self.shown.as_ref().and_then(|shown| shown.detail.rows.get(*at)).map(|row| row.path.clone());
                        path.and_then(|path| self.exact_rect(&path).map(|(e, piece)| (e, piece, path)))
                    }
                    _ => None,
                },
            };
            if let Some((e, piece, path)) = hovered {
                if chosen.as_ref() != Some(&path) {
                    outline(frame, at_map(e), piece, p.edge, 1.5, None);
                }
            }
        }
        frame.unclip();
        // A folder holding nothing that takes room: told so where its map would be.
        if !animating && self.laid.is_empty() && self.shown.is_some() {
            centred(frame, lang.pick("这个文件夹没有占用空间的内容", "Nothing in this folder takes up space"), body(13.0, 400.0), p.text3, map.x, map.y + map.h / 2.0 - 10.0, map.w);
        }
        // (What is dimmed no longer drawn is let go of.)
        if !animating {
            let mut keys: std::collections::HashSet<Key> = std::collections::HashSet::new();
            for placed in &self.laid {
                keys.insert(placed.key);
                keys.extend(placed.block.pieces.iter().map(|piece| key_of(&placed.shown, &piece.item)));
            }
            self.dims.retain(|key, _| keys.contains(key));
        }
        animating || dims_moving
    }

    /// The tools of the block at `at` (its place in the layout) on the map at `map`, as the mockup places them: at its
    /// top right (on a ground's name line), the plus (or, a system item, the button telling of it) where the block is
    /// wide enough, freeing it to the cloud beside it where its words fit.
    fn block_tools(&self, frame: &Frame, map: Rect, at: usize) -> Option<BlockTools> {
        let placed = self.laid.get(at)?;
        let about = self.about(&placed.shown, &placed.block.item);
        let b = placed.block.at;
        let e = Rect { x: map.x + b.x, y: map.y + b.y, w: b.w, h: b.h };
        let group = placed.block.group;
        let label = crate::disk::map::LABEL;
        if e.h < if group { label } else { 30.0 } {
            return None;
        }
        let info_like = about.info.is_some() || matches!(about.what, Some(What::File { system: true }));
        let can = if info_like {
            Some(Icon::Info)
        } else if about.why_not.is_none() {
            Some(Icon::Add)
        } else {
            None
        };
        let top = e.y + if group { (label - 24.0) / 2.0 } else { 4.0 };
        let fits = can.is_some() && e.w >= 48.0;
        let mut right = e.x + e.w - 4.0;
        if fits {
            right -= 28.0;
        }
        let mut room = 0.0;
        let mut release = None;
        let releasable = about.onedrive && about.bytes > 0 && matches!(about.what, Some(What::File { system: false } | What::Folder(Role::Plain)));
        if releasable {
            let w = release_width(frame, self.lang);
            if e.w >= w + if can.is_some() { 40.0 } else { 12.0 } {
                release = Some(Rect { x: right - w, y: top, w, h: 24.0 });
                room = w + 4.0;
            }
        }
        let add = match can {
            Some(icon) if fits => {
                room += 28.0;
                Some((Rect { x: e.x + e.w - 28.0, y: top, w: 24.0, h: 24.0 }, icon))
            }
            _ => None,
        };
        (add.is_some() || release.is_some()).then_some(BlockTools { at, add, release, room })
    }

    /// The tools of the block under the pointer, popping up as they come (and not again as the pointer goes from one
    /// block to the next), and going as they came. Whether they move still.
    fn paint_block_tools(&mut self, frame: &Frame, p: &Palette, now: Instant, tools: Option<&BlockTools>, hot: &Option<Target>) -> bool {
        let moving = keep(&mut self.tools_shown, tools, |_, _| true, now, 0.16);
        let Some(shown) = self.tools_shown.clone() else { return moving };
        let opacity = shown.shown(now, 0.12, EASE);
        let scale = 0.8 + 0.2 * shown.shown(now, 0.16, SLIDE);
        let live = shown.went.is_none();
        let tools = shown.what;
        if let Some(r) = tools.release {
            let label = self.lang.pick("释放空间", "Free up space");
            self.paint_tool(frame, p, r, Icon::Cloud, Some(label), Target::ReleaseTool(tools.at), (opacity, scale, live), hot);
        }
        if let Some((r, icon)) = tools.add {
            self.paint_tool(frame, p, r, icon, None, Target::AddTool(tools.at), (opacity, scale, live), hot);
        }
        moving
    }

    /// A tool on a block: a flyout's button, the accent's under the pointer; seen as much as `opacity` says, `scale`
    /// times as large, pressable where `live`.
    #[allow(clippy::too_many_arguments)]
    fn paint_tool(&mut self, frame: &Frame, p: &Palette, r: Rect, icon: Icon, label: Option<&str>, target: Target, (opacity, scale, live): (f32, f32, bool), hot: &Option<Target>) {
        let lit = live && *hot == Some(target.clone());
        {
            let _faint = faint(frame, opacity);
            let _moved = shifted(frame, 0.0, 0.0, Some((r.x + r.w / 2.0, r.y + r.h / 2.0, scale)));
            frame.shadow(Color::hex(0, 0.16), r.x, r.y, r.w, r.h, 4.0, 3.0, 2.0);
            fill(frame, if lit { p.accent } else { p.fly }, r.x, r.y, r.w, r.h, 4.0);
            Canvas::stroke_rounded(frame, Fill::Solid(p.fly_stroke), r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0, 5.0, 1.0);
            let ink = if lit { p.on_accent } else { p.text };
            match label {
                None => frame.icon(icon, Point { x: r.x + r.w / 2.0, y: r.y + r.h / 2.0 }, 14.0, ink),
                Some(label) => {
                    frame.icon(icon, Point { x: r.x + 6.0 + 7.0, y: r.y + r.h / 2.0 }, 14.0, ink);
                    let font = body(12.0, 400.0);
                    frame.text(label, font, ink, r.x + 6.0 + 14.0 + 6.0, frame.line_in(font, r.y, r.h), r.w, Align::Start);
                }
            }
        }
        if live {
            self.spot(r, target);
        }
    }

    /// The row of the list shown that is the file under the pointer on the map (or its type, or its time).
    fn row_of_hovered(&self, shown: &Shown) -> Option<usize> {
        if self.drag.is_some() || self.anim.is_some() {
            return None;
        }
        let placed = self.laid.get(self.hover?)?;
        let Item::Node(at) = placed.block.item else { return None };
        let node = &placed.shown.tree.nodes[at];
        if !matches!(node.what, What::File { .. }) {
            return None;
        }
        match self.tab {
            Tab::Largest => shown.detail.rows.iter().position(|row| row.path == placed.shown.paths[at]),
            Tab::Types => shown.detail.extensions.iter().position(|listed| listed.extension == crate::disk::extension_of(&node.folded)),
            Tab::Ages => Some(crate::disk::age_of(node.held.written, shown.at)),
        }
    }

    /// How many rows the list shown has.
    pub(super) fn list_count(&self) -> usize {
        self.shown.as_ref().map_or(0, |shown| match self.tab {
            Tab::Largest => shown.detail.rows.len(),
            Tab::Types => shown.detail.extensions.len(),
            Tab::Ages => AGES,
        })
    }

    /// The list's scroll thumb, in the list's `area` (its rows `count`), as it is scrolled now; and how far the list
    /// scrolls. None where it all fits.
    pub(super) fn thumb(&self, area: Rect, count: usize) -> Option<(Rect, f32)> {
        let whole = count as f32 * row_height(self.tab);
        let most = whole - area.h;
        if most <= 0.5 {
            return None;
        }
        let h = (area.h * area.h / whole).max(24.0);
        let y = area.y + (self.scroll / most).clamp(0.0, 1.0) * (area.h - h);
        Some((Rect { x: area.x + area.w + 1.0, y, w: 4.0, h }, most))
    }

    /// The lists beside the map: the largest files (each with its tools under the pointer), the files by type and by
    /// time (each lighting the map under the pointer, pressed to light it); the row of the file under the pointer on
    /// the map lit too. Whether they move still.
    fn paint_side(&mut self, frame: &Frame, p: &Palette, g: Geo, dt: f32, hot: &Option<Target>) -> bool {
        let lang = self.lang;
        let now = self.now;
        let side = g.side;
        card(frame, p, side);
        let mut moving = false;
        // The tabs, the mark under the one shown sliding to it.
        let tabs = [(Tab::Largest, lang.pick("最大的文件", "Largest files")), (Tab::Types, lang.pick("按类型", "By type")), (Tab::Ages, lang.pick("按时间", "By time"))];
        let font = body(13.0, 600.0);
        let mut x = side.x + 8.0;
        let mut mark_to = None;
        for (tab, label) in tabs {
            let w = frame.measure(label, font) + 24.0;
            let r = Rect { x, y: side.y + 6.0, w, h: TABS_H - 12.0 };
            let target = Target::Tab(tab);
            let on = self.tab == tab;
            let lit = self.fade(&target, Look::Hot, !on && *hot == Some(target.clone()), 0.12);
            if lit > 0.0 {
                fill(frame, p.hover.alpha(lit), r.x, r.y, r.w, r.h, 6.0);
            }
            frame.text(label, font, if on { p.text } else { p.text2 }, r.x + 12.0, frame.line_in(font, r.y, r.h), w, Align::Start);
            if on {
                mark_to = Some(r.x + r.w / 2.0);
            }
            self.spot(r, target);
            x += w + 2.0;
        }
        if let Some(middle) = mark_to {
            let mark = self.tab_mark.get_or_insert_with(|| Mark::new(middle));
            mark.go(middle, now);
            moving |= mark.moving(now);
            let (from, to) = mark.at(now);
            fill(frame, p.accent, from, side.y + TABS_H - 5.0, to - from, 3.0, 1.5);
        }
        let Some(shown) = self.shown.clone().filter(|shown| shown.root.eq_ignore_ascii_case(&self.root)) else { return moving };
        let detail = &shown.detail;
        // The sum line, and gathering all that passes where the list is filtered or lit.
        let sum_font = body(12.0, 400.0);
        let sum_y = frame.line_in(sum_font, side.y + TABS_H, SUM_H);
        let sum = match self.tab {
            Tab::Largest => match lang {
                Lang::Zh => format!("{} 个文件，共 {}", count_text(detail.passing.0), size_text(detail.passing.1, lang)),
                Lang::En => format!("{} files, {}", count_text(detail.passing.0), size_text(detail.passing.1, lang)),
            },
            Tab::Types => match lang {
                Lang::Zh => format!("{} 种扩展名", detail.extensions.len()),
                Lang::En => format!("{} extensions", detail.extensions.len()),
            },
            Tab::Ages => lang.pick("按最后修改的时间", "By when last changed").into(),
        };
        let mut sum_right = side.x + side.w - 16.0;
        let addable = detail.rows.iter().any(|row| !self.gathered_at(&row.path));
        if self.tab == Tab::Largest && (self.filter.on() || self.light.is_some()) && addable {
            let label = lang.pick("全部加入", "Gather all");
            let link_font = body(13.0, 400.0);
            let content = 12.0 + 4.0 + frame.measure(label, link_font);
            let r = Rect { x: side.x + side.w - 16.0 - content - 4.0, y: side.y + TABS_H + 3.0, w: content + 8.0, h: SUM_H - 6.0 };
            let lit = self.fade(&Target::AddAll, Look::Hot, *hot == Some(Target::AddAll), 0.12);
            if lit > 0.0 {
                fill(frame, p.hover.alpha(lit), r.x, r.y, r.w, r.h, 4.0);
            }
            frame.icon(Icon::Add, Point { x: r.x + 4.0 + 6.0, y: r.y + r.h / 2.0 }, 12.0, p.accent);
            frame.text(label, link_font, p.accent, r.x + 4.0 + 16.0, frame.line_in(link_font, r.y, r.h), content, Align::Start);
            self.spot(r, Target::AddAll);
            sum_right = r.x - 8.0;
        }
        frame.text(&sum, sum_font, p.text2, side.x + 16.0, sum_y, sum_right - side.x - 16.0, Align::Start);
        let area = list_area(side);
        let row_h = row_height(self.tab);
        let count = self.list_count();
        let most_scroll = (count as f32 * row_h - area.h).max(0.0);
        self.scroll_to = self.scroll_to.clamp(0.0, most_scroll);
        let k = 1.0 - (-dt * 1000.0 / 50.0).exp();
        self.scroll += (self.scroll_to - self.scroll) * k;
        if (self.scroll_to - self.scroll).abs() > 0.3 {
            moving = true;
        } else {
            self.scroll = self.scroll_to;
        }
        if count == 0 {
            let none = if self.filter.on() || self.light.is_some() { lang.pick("没有符合条件的文件", "No file matches") } else { lang.pick("这个文件夹没有文件", "No files in this folder") };
            centred(frame, none, body(13.0, 400.0), p.text3, area.x, area.y + 40.0, area.w);
            return moving;
        }
        let from_map = self.row_of_hovered(&shown);
        frame.clip(area.x, area.y, area.w, area.h);
        let first = (self.scroll / row_h).floor().max(0.0) as usize;
        let last = (((self.scroll + area.h) / row_h).ceil() as usize).min(count);
        let name_font = body(13.0, 400.0);
        let sub_font = body(12.0, 400.0);
        let biggest = match self.tab {
            Tab::Largest => 1,
            Tab::Types => detail.extensions.first().map_or(1, |extension| extension.bytes),
            Tab::Ages => detail.ages.iter().map(|age| age.1).max().unwrap_or(1),
        }
        .max(1);
        for at in first..last {
            let r = Rect { x: area.x, y: area.y + at as f32 * row_h - self.scroll, w: area.w, h: row_h };
            // Each row: what it is, its icon and colour, its name and what is under it, its size, whether it is chosen
            // (or lights the map), whether nothing is in it; a file's tools (gathered, freed to the cloud).
            let (target, icon, hue, name, sub, bytes, on, empty, add, release) = match self.tab {
                Tab::Largest => {
                    let row = &detail.rows[at];
                    let gathered = self.gathered_at(&row.path);
                    let sub = if gathered { lang.pick("已在待清理中", "Already gathered").to_string() } else { row.path.parent().map(|parent| parent.to_string_lossy().into_owned()).unwrap_or_default() };
                    let release = self.in_onedrive(&row.path) && row.bytes > 0;
                    (Target::Row(at), kind_icon(row.kind), p.hue[row.kind.at()], row.name.clone(), sub, row.bytes, self.chosen.as_ref() == Some(&row.path), false, !gathered, release)
                }
                Tab::Types => {
                    let extension = &detail.extensions[at];
                    let name = if extension.extension.is_empty() { lang.pick("其他扩展名", "Other extensions").to_string() } else { format!(".{}", extension.extension) };
                    let sub = match lang {
                        Lang::Zh => format!("{} · {} 个文件", kind_name(extension.kind, lang), count_text(extension.files)),
                        Lang::En => format!("{} · {} files", kind_name(extension.kind, lang), count_text(extension.files)),
                    };
                    (Target::TypeRow(extension.extension.clone()), kind_icon(extension.kind), p.hue[extension.kind.at()], name, sub, extension.bytes, self.light == Some(Light::Extension(extension.extension.clone())), false, false, false)
                }
                Tab::Ages => {
                    let (files, bytes) = detail.ages[at];
                    let sub = match lang {
                        Lang::Zh => format!("{} 个文件", count_text(files)),
                        Lang::En => format!("{} files", count_text(files)),
                    };
                    (Target::AgeRow(at), Icon::Clock, p.text2, age_name(at, lang).to_string(), sub, bytes, self.light == Some(Light::Age(at)), files == 0, false, false)
                }
            };
            let row_hot = *hot == Some(target.clone()) || matches!(hot, Some(Target::RowAdd(n) | Target::RowRelease(n)) if *n == at && self.tab == Tab::Largest);
            let lit = self.fade(&target, Look::Hot, row_hot || from_map == Some(at), 0.12);
            let has_tools = add || release;
            let tools = if has_tools { self.fade(&target, Look::Tools, row_hot, 0.12) } else { 0.0 };
            let add_r = Rect { x: r.x + r.w - 8.0 - 24.0, y: r.y + 10.0, w: 24.0, h: 24.0 };
            let release_r = Rect { x: if add { r.x + r.w - 36.0 - 24.0 } else { add_r.x }, ..add_r };
            {
                let _faint = faint(frame, if empty { 0.45 } else { 1.0 });
                if on {
                    fill(frame, p.chosen, r.x, r.y, r.w, r.h, 6.0);
                    fill(frame, p.accent, r.x, r.y + 14.0, 3.0, r.h - 28.0, 1.5);
                } else if lit > 0.0 {
                    fill(frame, p.hover.alpha(lit), r.x, r.y, r.w, r.h, 6.0);
                }
                frame.icon(icon, Point { x: r.x + 10.0 + 7.0, y: r.y + r.h / 2.0 }, 14.0, hue);
                let size = size_text(bytes, lang);
                let size_w = frame.measure(&size, sub_font) + 2.0;
                let size_right = r.x + r.w - 10.0;
                let size_seen = 1.0 - tools;
                if size_seen > 0.0 {
                    frame.text(&size, sub_font, p.text2.alpha(size_seen), size_right - size_w, frame.line_in(sub_font, r.y, r.h), size_w, Align::End);
                }
                let text_x = r.x + 36.0;
                let text_right = size_right - size_w - 12.0;
                let top = r.y + (r.h - 34.0) / 2.0;
                frame.text(&name, name_font, p.text, text_x, top, text_right - text_x, Align::Start);
                frame.text(&sub, sub_font, p.text2, text_x, top + 18.0, text_right - text_x, Align::Start);
                // A type's or a time's share of the largest.
                if self.tab != Tab::Largest {
                    let track = Rect { x: r.x + 34.0, y: r.y + r.h - 3.0 - 2.0, w: r.w - 34.0 - 10.0, h: 2.0 };
                    fill(frame, p.rule, track.x, track.y, track.w, track.h, 1.0);
                    fill(frame, p.accent.alpha(0.7), track.x, track.y, track.w * (bytes as f64 / biggest as f64) as f32, track.h, 1.0);
                }
                if tools > 0.0 {
                    let _faint = faint(frame, tools);
                    if release {
                        row_button(frame, p, release_r, Icon::Cloud, *hot == Some(Target::RowRelease(at)));
                    }
                    if add {
                        row_button(frame, p, add_r, Icon::Add, *hot == Some(Target::RowAdd(at)));
                    }
                }
            }
            self.spot(Rect { x: r.x, y: r.y.max(area.y), w: r.w, h: (r.y + r.h).min(area.y + area.h) - r.y.max(area.y) }, target);
            // (The tools found before their row: where they are whole in the list.)
            if row_hot {
                let inside = |t: &Rect| t.y >= area.y - 1.0 && t.y + t.h <= area.y + area.h + 1.0;
                if release && inside(&release_r) {
                    self.spot(release_r, Target::RowRelease(at));
                }
                if add && inside(&add_r) {
                    self.spot(add_r, Target::RowAdd(at));
                }
            }
        }
        frame.unclip();
        // A thin scroll thumb, as the mockup's list has (held and dragged to scroll).
        if let Some((thumb, _)) = self.thumb(area, count) {
            let held = matches!(self.press, Some(Press { on: Pressed::Thumb(_), .. }));
            let lit = self.fade(&Target::Thumb, Look::Hot, held || *hot == Some(Target::Thumb), 0.12);
            fill(frame, mix(p.rule, p.text3, lit), thumb.x, thumb.y, thumb.w, thumb.h, 2.0);
            self.spot(Rect { x: thumb.x - 2.0, w: thumb.w + 4.0, ..thumb }, Target::Thumb);
        }
        moving
    }

    /// Whether `path` is in one of this PC's OneDrive folders.
    pub(super) fn in_onedrive(&self, path: &Path) -> bool {
        self.clouds.iter().any(|cloud| path.starts_with(cloud))
    }

    /// Whether (`x`, `y`) is over the foot, where what is dragged may be let go of (as far as 8 DIPs above it).
    pub(super) fn over_foot(&self, x: f32, y: f32) -> bool {
        let foot = self.foot_at;
        x >= foot.x && x <= foot.x + foot.w && y >= foot.y - 8.0 && y <= foot.y + foot.h
    }

    /// What the foot shows, told by a sum of it (see `foot_seen`).
    fn foot_sum(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut sum = std::collections::hash_map::DefaultHasher::new();
        for item in &self.gathered {
            item.path.hash(&mut sum);
        }
        self.pending.as_ref().map(|pending| pending.items.len()).hash(&mut sum);
        self.going.len().hash(&mut sum);
        self.went.as_ref().map(|went| (went.count, went.bytes)).hash(&mut sum);
        sum.finish()
    }

    /// The place what is to go is gathered in, as the mockup's collector: what it holds as chips (as many as fit, the
    /// rest in a menu), how much it frees, its buttons; its going, undoable, a ring draining; that it went; or, holding
    /// nothing, where to drag things. Drawn anew, it comes up from faint; something that may go held over it marks it
    /// ready. Whether it moves still.
    fn paint_foot(&mut self, frame: &Frame, p: &Palette, g: Geo, now: Instant, hot: &Option<Target>) -> bool {
        let lang = self.lang;
        let foot = g.foot;
        self.foot_at = foot;
        self.spot(foot, Target::Foot);
        let seen = self.foot_sum();
        if seen != self.foot_seen.0 {
            self.foot_seen = (seen, now);
        }
        let since = now.saturating_duration_since(self.foot_seen.1).as_secs_f32();
        let flash = if crate::panel::reduced_motion() { 1.0 } else { 0.3 + 0.7 * EASE_OUT.at(since / 0.18) };
        let moving = flash < 1.0;
        let ready_now = self.drag.as_ref().is_some_and(|drag| drag.why_not.is_none() && self.pending.is_none() && self.over_foot(drag.at.0, drag.at.1));
        let ready = self.fade(&Target::Foot, Look::Hot, ready_now, 0.16);
        let _faint = faint(frame, flash);
        let title_font = body(13.0, 400.0);
        let sub_font = body(12.0, 400.0);
        let words_x = foot.x + 16.0 + 20.0 + 12.0;
        let title_y = frame.line_in(title_font, foot.y + 11.0, 18.0);
        let sub_y = frame.line_in(sub_font, foot.y + 29.0, 16.0);
        let mark = Point { x: foot.x + 16.0 + 10.0, y: foot.y + foot.h / 2.0 };
        let empty = self.pending.is_none() && self.gathered.is_empty() && self.going.is_empty() && self.went.is_none();
        if empty {
            fill(frame, p.chosen.alpha(ready), foot.x, foot.y, foot.w, foot.h, CARD_RADIUS);
            if ready < 1.0 {
                frame.stroke_rounded_dashed(p.text3.alpha(1.0 - ready), foot.x, foot.y, foot.w, foot.h, CARD_RADIUS, 1.0);
            }
            if ready > 0.0 {
                Canvas::stroke_rounded(frame, Fill::Solid(p.accent.alpha(ready)), foot.x, foot.y, foot.w, foot.h, CARD_RADIUS, 2.0);
            }
            frame.icon(Icon::Inbox, mark, 18.0, mix(p.text2, p.accent, ready));
            frame.text(lang.pick("将文件或文件夹拖到这里，集中移至回收站", "Drag files or folders here to move them to the Recycle Bin together"), title_font, p.text, words_x, title_y, foot.w - 64.0, Align::Start);
            frame.text(
                lang.pick("也可以点方块或列表右侧的加号。删除以当前用户的权限执行，没有权限的项目会保留并列出。", "Or press the plus on a block or a row. They are deleted as you may: what you may not delete stays, and is listed."),
                sub_font,
                p.text2,
                words_x,
                sub_y,
                foot.w - 64.0,
                Align::Start,
            );
            return moving;
        }
        card(frame, p, foot);
        if ready > 0.0 {
            fill(frame, p.chosen.alpha(ready), foot.x, foot.y, foot.w, foot.h, CARD_RADIUS);
            Canvas::stroke_rounded(frame, Fill::Solid(p.accent.alpha(ready)), foot.x, foot.y, foot.w, foot.h, CARD_RADIUS, 2.0);
        }
        let right = foot.x + foot.w - 12.0;
        let top = foot.y + 12.0;
        if let Some(pending) = self.pending.clone() {
            // About to go: the button undoing it, what goes, its time to be undone draining from the ring's top, the
            // seconds left in it, and what goes as it was gathered (faint: no way out of it now).
            let undo = self.foot_button(frame, p, right, top, Some(Icon::Undo), lang.pick("撤销", "Undo"), Target::Undo, false, hot);
            let bytes: u64 = pending.items.iter().map(|item| item.bytes).sum();
            let title = lang.pick("即将移至回收站", "Moving to the Recycle Bin");
            let sub = match lang {
                Lang::Zh => format!("{} 项，共 {}", pending.items.len(), size_text(bytes, lang)),
                Lang::En => format!("{} items, {}", pending.items.len(), size_text(bytes, lang)),
            };
            let words_w = frame.measure(title, title_font).max(frame.measure(&sub, sub_font)) + 2.0;
            let words_at = undo.x - 12.0 - words_w;
            frame.text(title, title_font, p.text, words_at, title_y, words_w, Align::Start);
            frame.text(&sub, sub_font, p.text2, words_at, sub_y, words_w, Align::Start);
            let left = (UNDO_FOR.as_secs_f32() - now.saturating_duration_since(pending.since).as_secs_f32()).max(0.0);
            let centre = Point { x: words_at - 12.0 - 14.0, y: foot.y + foot.h / 2.0 };
            frame.arc(centre, 11.0, 0.0, TAU, p.rule, 2.5);
            frame.arc(centre, 11.0, 0.0, TAU * left / UNDO_FOR.as_secs_f32(), p.accent, 2.5);
            let number = body(12.0, 600.0);
            centred(frame, &format!("{}", left.ceil() as u32), number, p.text, centre.x - 14.0, frame.line_in(number, centre.y - 10.0, 20.0), 28.0);
            self.paint_chips(frame, p, &pending.items, (foot.x + 16.0, centre.x - 14.0 - 12.0), top, false, hot);
            return true;
        }
        if self.gathered.is_empty() && !self.going.is_empty() {
            // Going now: the shell moves it.
            let bytes: u64 = self.going.iter().map(|item| item.bytes).sum();
            frame.icon(Icon::Trash, mark, 18.0, p.text2);
            frame.text(lang.pick("正在移至回收站", "Moving to the Recycle Bin"), title_font, p.text, words_x, title_y, foot.w - 64.0, Align::Start);
            let sub = match lang {
                Lang::Zh => format!("{} 项，共 {}", self.going.len(), size_text(bytes, lang)),
                Lang::En => format!("{} items, {}", self.going.len(), size_text(bytes, lang)),
            };
            frame.text(&sub, sub_font, p.text2, words_x, sub_y, foot.w - 64.0, Align::Start);
            return moving;
        }
        if self.gathered.is_empty() {
            if let Some(went) = self.went.as_ref().map(|went| (went.count, went.bytes)) {
                let button = self.foot_button(frame, p, right, top, Some(Icon::Trash), lang.pick("清空回收站", "Empty Recycle Bin"), Target::EmptyNow, false, hot);
                frame.icon(Icon::CircleCheck, mark, 18.0, p.accent);
                let title = match lang {
                    Lang::Zh => format!("已移至回收站：{} 项，{}", went.0, size_text(went.1, lang)),
                    Lang::En => format!("Moved to the Recycle Bin: {} items, {}", went.0, size_text(went.1, lang)),
                };
                frame.text(&title, title_font, p.text, words_x, title_y, button.x - 12.0 - words_x, Align::Start);
                frame.text(lang.pick("回收站仍在这个盘上，清空后才释放空间", "The Recycle Bin is on this drive: emptied, its space is freed"), sub_font, p.text2, words_x, sub_y, button.x - 12.0 - words_x, Align::Start);
                return moving;
            }
        }
        // What is gathered: its chips, how much it frees, all of it out, its going.
        let commit = self.foot_button(frame, p, right, top, None, lang.pick("移至回收站", "Move to Recycle Bin"), Target::Commit, true, hot);
        let clear = self.foot_button(frame, p, commit.x - 12.0, top, None, lang.pick("全部移出", "Remove all"), Target::UngatherAll, false, hot);
        let bytes: u64 = self.gathered.iter().map(|item| item.bytes).sum();
        let frees = format!("{} {}", lang.pick("将释放", "Frees"), size_text(bytes, lang));
        let later = lang.pick("清空回收站后生效", "once the Recycle Bin is emptied");
        let total_font = body(14.0, 600.0);
        let total_w = frame.measure(&frees, total_font).max(frame.measure(later, sub_font)) + 2.0;
        let total_x = clear.x - 12.0 - total_w;
        frame.text(&frees, total_font, p.text, total_x, frame.line_in(total_font, foot.y + 11.0, 18.0), total_w, Align::End);
        frame.text(later, sub_font, p.text2, total_x, sub_y, total_w, Align::End);
        let items = self.gathered.clone();
        self.paint_chips(frame, p, &items, (foot.x + 16.0, total_x - 12.0), top, true, hot);
        moving
    }

    /// A button of the foot's (Windows 11's, the accent's where `primary`) ending at `right`: as wide as its words (and
    /// icon), 96 DIPs at the least.
    #[allow(clippy::too_many_arguments)]
    fn foot_button(&mut self, frame: &Frame, p: &Palette, right: f32, y: f32, icon: Option<Icon>, label: &str, target: Target, primary: bool, hot: &Option<Target>) -> Rect {
        let font = body(14.0, 400.0);
        let content = icon.map_or(0.0, |_| 14.0 + 8.0) + frame.measure(label, font);
        let w = (content + 24.0).max(96.0);
        let r = Rect { x: right - w, y, w, h: BUTTON_H };
        let lit = self.fade(&target, Look::Hot, *hot == Some(target.clone()), 0.12);
        button(frame, p, r, 6.0, if primary { 1.0 } else { 0.0 }, 1.08, lit);
        let ink = if primary { p.on_accent } else { p.text };
        let mut x = r.x + (w - content) / 2.0;
        if let Some(icon) = icon {
            frame.icon(icon, Point { x: x + 7.0, y: r.y + r.h / 2.0 }, 14.0, ink);
            x += 14.0 + 8.0;
        }
        frame.text(label, font, ink, x, frame.line_in(font, r.y, r.h), content + 2.0, Align::Start);
        self.spot(r, target);
        r
    }

    /// The chips of what is gathered (or about to go, faint and with no way out, where not `live`) from `left` to
    /// `right` at `y`: as many as fit whole, the rest under one more that lists them.
    #[allow(clippy::too_many_arguments)]
    fn paint_chips(&mut self, frame: &Frame, p: &Palette, items: &[Gathered], (left, right): (f32, f32), y: f32, live: bool, hot: &Option<Target>) {
        let lang = self.lang;
        let name_font = body(13.0, 400.0);
        let size_font = body(12.0, 400.0);
        let more_label = |count: usize| match lang {
            Lang::Zh => format!("另外 {} 项", count),
            Lang::En => format!("{} more", count),
        };
        let more_w = |count: usize| 12.0 + frame.measure(&more_label(count), name_font) + 12.0;
        // Each chip's name's width and its own (230 DIPs at the most, its name cut to fit).
        let widths: Vec<(f32, f32)> = items
            .iter()
            .map(|item| {
                let tail = if live { 8.0 + 24.0 + 4.0 } else { 8.0 + 6.0 + 4.0 };
                let fixed = 10.0 + 14.0 + 8.0 + 8.0 + frame.measure(&size_text(item.bytes, lang), size_font) + tail;
                let name_w = frame.measure(&item.name, name_font).min(230.0 - fixed).max(0.0);
                (name_w, fixed + name_w)
            })
            .collect();
        let taken = |count: usize| widths[..count].iter().map(|w| w.1).sum::<f32>() + 6.0 * count.saturating_sub(1) as f32;
        let mut shown = items.len();
        if taken(shown) > right - left {
            while shown > 0 && taken(shown) + if shown > 0 { 6.0 } else { 0.0 } + more_w(items.len() - shown) > right - left {
                shown -= 1;
            }
        }
        let mut x = left;
        for (at, item) in items.iter().enumerate().take(shown) {
            let (name_w, w) = widths[at];
            let r = Rect { x, y, w, h: 32.0 };
            {
                let _faint = faint(frame, if live { 1.0 } else { 0.55 });
                fill(frame, p.control, r.x, r.y, r.w, r.h, 6.0);
                frame.icon(if item.folder { Icon::Folder } else { kind_icon(item.kind) }, Point { x: r.x + 10.0 + 7.0, y: r.y + 16.0 }, 14.0, p.text2);
                frame.text(&item.name, name_font, p.text, r.x + 32.0, frame.line_in(name_font, r.y, r.h), name_w, Align::Start);
                let size = size_text(item.bytes, lang);
                frame.text(&size, size_font, p.text2, r.x + 32.0 + name_w + 8.0, frame.line_in(size_font, r.y, r.h), frame.measure(&size, size_font) + 2.0, Align::Start);
            }
            if live {
                let close = Rect { x: r.x + r.w - 4.0 - 24.0, y: r.y + 4.0, w: 24.0, h: 24.0 };
                let target = Target::Ungather(at);
                let lit = self.fade(&target, Look::Hot, *hot == Some(target.clone()), 0.12);
                if lit > 0.0 {
                    fill(frame, p.hover.alpha(lit), close.x, close.y, close.w, close.h, 4.0);
                }
                frame.icon(Icon::Close, Point { x: close.x + 12.0, y: close.y + 12.0 }, 12.0, mix(p.text2, p.text, lit));
                self.spot(close, target);
            }
            x += w + 6.0;
        }
        let reopen = std::mem::take(&mut self.reopen_gathered);
        if shown < items.len() {
            let hidden: Vec<usize> = (shown..items.len()).collect();
            let r = Rect { x, y, w: more_w(hidden.len()), h: 32.0 };
            let label = more_label(hidden.len());
            if live {
                let target = Target::GatheredMore(hidden.clone());
                let lit = self.fade(&target, Look::Hot, *hot == Some(target.clone()), 0.12);
                fill(frame, mix(p.control, p.rule, lit), r.x, r.y, r.w, r.h, 6.0);
                frame.text(&label, name_font, p.text, r.x + 12.0, frame.line_in(name_font, r.y, r.h), r.w, Align::Start);
                self.spot(r, target);
                // (One taken out of its menu: the menu again, of those not shown now.)
                if reopen {
                    self.menu = Some(Menu { of: MenuOf::Gathered(hidden), under: r, above: true });
                }
            } else {
                let _faint = faint(frame, 0.55);
                fill(frame, p.control, r.x, r.y, r.w, r.h, 6.0);
                frame.text(&label, name_font, p.text, r.x + 12.0, frame.line_in(name_font, r.y, r.h), r.w, Align::Start);
            }
        }
    }

    /// What is dragged, as its ghost tells it.
    fn ghost_of(&self, drag: &Drag) -> Option<Ghost> {
        let about = match drag.what {
            Draggable::Block(at) => self.laid.get(at).map(|placed| self.about(&placed.shown, &placed.block.item)),
            Draggable::Row(at) => self.row_about(at),
        }?;
        let icon = if drag.why_not.is_some() {
            Icon::Lock
        } else if about.folder {
            Icon::Folder
        } else {
            kind_icon(about.kind)
        };
        Some(Ghost { icon, name: about.name, size: size_text(about.bytes, self.lang), why_not: drag.why_not.clone(), at: drag.at })
    }

    /// What is over the window, each coming and going as the mockup's do: what is dragged, by the pointer; a block's
    /// tag (or a drive's); a card, rising into place; a menu, dropping into it; a word said. Whether any moves still.
    fn paint_over(&mut self, frame: &Frame, p: &Palette, g: Geo, now: Instant, hot: &Option<Target>) -> bool {
        let mut moving = false;
        let ghost = self.drag.as_ref().and_then(|drag| self.ghost_of(drag));
        moving |= keep(&mut self.ghost, ghost.as_ref(), |_, _| true, now, 0.16);
        if let Some(shown) = self.ghost.clone() {
            paint_ghost(frame, p, &shown.what, shown.shown(now, 0.12, EASE), 0.92 + 0.08 * shown.shown(now, 0.16, SLIDE));
        }
        // A block's tag, while the map is still, nothing is held, and no card or menu is up.
        let quiet = self.drag.is_none() && self.plate.is_none() && self.menu.is_none() && self.press.is_none() && self.anim.is_none();
        let tag = self.hover.filter(|_| quiet && matches!(hot, Some(Target::Map | Target::AddTool(_) | Target::ReleaseTool(_)))).zip(self.pointer);
        moving |= keep(&mut self.tag_shown, tag.as_ref(), |_, _| true, now, 0.1);
        if let Some(shown) = self.tag_shown.clone() {
            let _faint = faint(frame, shown.shown(now, 0.1, EASE));
            self.paint_tag(frame, p, g, shown.what.0, shown.what.1);
        }
        // A drive's, nearly out of space, under its tab.
        let drive_tag = match hot {
            Some(Target::Drive(at)) if quiet && self.drives.get(*at).is_some_and(|drive| drive.low()) => Some(*at),
            _ => None,
        };
        moving |= keep(&mut self.drive_tag, drive_tag.as_ref(), |_, _| true, now, 0.1);
        if let Some(shown) = self.drive_tag.clone() {
            let _faint = faint(frame, shown.shown(now, 0.1, EASE));
            self.paint_drive_tag(frame, p, shown.what);
        }
        moving |= keep(&mut self.plate_shown, self.plate.as_ref(), |_, _| true, now, 0.22);
        if let Some(shown) = self.plate_shown.clone() {
            let _faint = faint(frame, shown.shown(now, 0.14, EASE));
            let _moved = shifted(frame, 0.0, 6.0 * (1.0 - shown.shown(now, 0.22, SETTLE)), None);
            self.paint_plate(frame, p, g, &shown.what, shown.went.is_none(), hot);
        }
        moving |= keep(&mut self.menu_shown, self.menu.as_ref(), |_, _| true, now, 0.2);
        if let Some(shown) = self.menu_shown.clone() {
            let _faint = faint(frame, shown.shown(now, 0.12, EASE));
            let _moved = shifted(frame, 0.0, -6.0 * (1.0 - shown.shown(now, 0.2, SETTLE)), None);
            self.paint_menu(frame, p, g, &shown.what, shown.went.is_none(), hot);
        }
        moving |= keep(&mut self.said_shown, self.said.as_ref(), |_, _| true, now, 0.22);
        if let Some(shown) = self.said_shown.clone() {
            let _faint = faint(frame, shown.shown(now, 0.16, EASE));
            let _moved = shifted(frame, 0.0, 8.0 * (1.0 - shown.shown(now, 0.22, SETTLE)), None);
            let said = &shown.what;
            let font = body(13.0, 400.0);
            let w = 12.0 + 16.0 + 10.0 + frame.measure(&said.words, font) + 16.0;
            let r = Rect { x: PAGE_X + (g.page_w - w) / 2.0, y: g.height - 96.0 - 40.0, w, h: 40.0 };
            flyout(frame, p, r);
            frame.icon(said.icon, Point { x: r.x + 12.0 + 8.0, y: r.y + r.h / 2.0 }, 16.0, p.accent);
            frame.text(&said.words, font, p.text, r.x + 12.0 + 16.0 + 10.0, frame.line_in(font, r.y, r.h), r.w - 38.0, Align::Start);
        }
        moving
    }

    /// The tag of a drive nearly out of space (its place among the drives), under its tab.
    fn paint_drive_tag(&mut self, frame: &Frame, p: &Palette, at: usize) {
        let lang = self.lang;
        let Some(drive) = self.drives.get(at) else { return };
        let (title, sub) = match lang {
            Lang::Zh => (format!("{}: 可用空间不足", drive.letter()), format!("仅余 {}，约为总容量的 {:.0}%", size_text(drive.free, lang), drive.free as f64 * 100.0 / drive.total.max(1) as f64)),
            Lang::En => (format!("{}: is running out of space", drive.letter()), format!("Only {} free, about {:.0}% of it", size_text(drive.free, lang), drive.free as f64 * 100.0 / drive.total.max(1) as f64)),
        };
        let under = self.target_rect(&Target::Drive(at));
        let r = Rect { x: under.x, y: under.y + under.h, w: 240.0, h: 52.0 };
        flyout(frame, p, r);
        frame.text(&title, body(13.0, 600.0), p.text, r.x + 12.0, r.y + 8.0, r.w - 24.0, Align::Start);
        frame.text(&sub, body(12.0, 400.0), p.text2, r.x + 12.0, r.y + 28.0, r.w - 24.0, Align::Start);
    }

    /// The tag of the block at `at` (its place in the layout): what it is, and what can be done with it.
    fn paint_tag(&mut self, frame: &Frame, p: &Palette, g: Geo, at: usize, (x, y): (f32, f32)) {
        let lang = self.lang;
        let Some(placed) = self.laid.get(at).cloned() else { return };
        let about = self.about(&placed.shown, &placed.block.item);
        let label_font = body(12.0, 400.0);
        let mut lines: Vec<(String, Font, Color)> = Vec::new();
        let unlisted = about.info == Some(Info::Unlisted);
        if !unlisted && about.many_in.is_none() && !matches!(about.what, Some(What::Rest)) {
            lines.push((about.path.to_string_lossy().into_owned(), label_font, p.text3));
        }
        let mut facts: Vec<(String, String)> = vec![(lang.pick("大小", "Size").into(), size_text(about.bytes, lang))];
        if about.folder || about.many_in.is_some() || matches!(about.what, Some(What::Rest)) {
            facts.push((lang.pick("文件数", "Files").into(), count_text(about.files)));
        }
        if !unlisted && about.written > 0 {
            facts.push((lang.pick("修改时间", "Changed").into(), date_text(about.written, lang)));
        }
        let mut notes: Vec<(String, Color)> = Vec::new();
        if about.many_in.is_some() || matches!(about.what, Some(What::Rest)) {
            notes.push((lang.pick("这些项目各自太小，图上画不出，合为一块显示", "Each of these is too small to draw: they are shown as one block").into(), p.text2));
        }
        if about.cloud > 0 {
            notes.push((format!("{}{}{}", lang.pick("云端另有 ", "A further "), size_text(about.cloud, lang), lang.pick("，不占用本机空间", " is in the cloud alone, taking no space here")), p.text2));
        }
        let releasable = about.onedrive && about.bytes > 0 && matches!(about.what, Some(What::File { system: false } | What::Folder(Role::Plain)));
        if releasable {
            notes.push((lang.pick("可以释放空间，文件只保留在 OneDrive 中", "Space can be freed: the files kept in OneDrive alone").into(), p.text2));
        }
        let info_like = about.info.is_some() || matches!(about.what, Some(What::File { system: true }));
        if let Some(why) = &about.why_not {
            if !info_like {
                notes.push((why.clone(), p.text3));
            }
        }
        let hint = if info_like {
            lang.pick("单击查看说明", "Click for what it is")
        } else if let Some((_, focus)) = &about.many_in {
            if *focus { lang.pick("可以在右侧列表中逐个选择", "Choose them one by one in the list") } else { lang.pick("单击进入所在文件夹，再逐个选择", "Click to open their folder, then choose them one by one") }
        } else if about.folder {
            lang.pick("单击进入，双击在资源管理器中显示", "Click to open, double-click to show in Explorer")
        } else if matches!(about.what, Some(What::Rest)) {
            ""
        } else {
            lang.pick("双击在资源管理器中显示", "Double-click to show in Explorer")
        };
        let inner = TAG_W - 24.0;
        let note_font = body(12.0, 400.0);
        let mut tall = 12.0 + 20.0;
        tall += lines.len() as f32 * 18.0;
        tall += 6.0 + facts.len() as f32 * 18.0;
        let note_heights: Vec<f32> = notes.iter().map(|(note, _)| frame.gfx.wrapped_height(note, note_font, inner)).collect();
        tall += note_heights.iter().map(|h| h + 4.0).sum::<f32>();
        if !hint.is_empty() {
            tall += 6.0 + 18.0;
        }
        tall += 10.0;
        let mut r = Rect { x: x + 14.0, y: y + 20.0, w: TAG_W, h: tall };
        if r.x + r.w > g.width - 8.0 {
            r.x = x - 14.0 - r.w;
        }
        if r.y + r.h > g.height - 8.0 {
            r.y = y - 12.0 - r.h;
        }
        r.x = r.x.clamp(8.0, (g.width - 8.0 - r.w).max(8.0));
        r.y = r.y.clamp(8.0, (g.height - 8.0 - r.h).max(8.0));
        flyout(frame, p, r);
        let mut at_y = r.y + 10.0;
        frame.text(&about.name, body(13.0, 600.0), p.text, r.x + 12.0, at_y, inner, Align::Start);
        at_y += 22.0;
        for (line, font, color) in &lines {
            frame.text(line, *font, *color, r.x + 12.0, at_y, inner, Align::Start);
            at_y += 18.0;
        }
        at_y += 6.0;
        for (label, value) in &facts {
            frame.text(label, label_font, p.text3, r.x + 12.0, at_y, 72.0, Align::Start);
            frame.text(value, label_font, p.text, r.x + 84.0, at_y, inner - 72.0, Align::Start);
            at_y += 18.0;
        }
        for ((note, color), h) in notes.iter().zip(&note_heights) {
            at_y += 4.0;
            frame.text_wrapped(note, note_font, *color, r.x + 12.0, at_y, inner);
            at_y += h;
        }
        if !hint.is_empty() {
            at_y += 6.0;
            frame.text(hint, label_font, p.text3, r.x + 12.0, at_y, inner, Align::Start);
        }
    }

    /// A card over the window (its buttons and links pressable where `live`: not as it goes).
    fn paint_plate(&mut self, frame: &Frame, p: &Palette, g: Geo, plate: &Plate, live: bool, hot: &Option<Target>) {
        let lang = self.lang;
        let none = None;
        let hot = if live { hot } else { &none };
        let inner = PLATE_W - 32.0;
        let body_font = body(13.0, 400.0);
        // Its title, its words, its links, its buttons.
        let (title, words, command, links, buttons): (String, String, Option<&str>, Vec<(String, PlateAct)>, Vec<(String, PlateAct, bool)>) = match &plate.of {
            PlateOf::System { info, name, bytes, files, .. } => match info {
                Some(Info::Pagefile) => (
                    lang.pick("页面文件", "Page file").into(),
                    lang.pick("Windows 在内存不足时使用的空间，大小通常由系统自动管理。可以在虚拟内存设置中调整它的大小或位置。", "Space Windows uses when memory runs short, usually sized by the system. Its size and drive can be changed in the virtual memory settings.").into(),
                    None,
                    vec![(lang.pick("调整虚拟内存", "Adjust virtual memory").into(), PlateAct::Open(crate::disk::act::Page::Performance))],
                    vec![],
                ),
                Some(Info::Hiberfil) => (
                    lang.pick("休眠文件", "Hibernation file").into(),
                    lang.pick("休眠和快速启动用它保存内存中的内容，大小约为内存的四成。关闭休眠后，Windows 会删除这个文件。", "Hibernation and fast startup keep what is in memory in it, about two-fifths of the memory's size. With hibernation off, Windows deletes it.").into(),
                    None,
                    vec![(lang.pick("关闭休眠", "Turn hibernation off").into(), PlateAct::Hibernation)],
                    vec![],
                ),
                Some(Info::Restore) => (
                    lang.pick("系统还原点", "Restore points").into(),
                    lang.pick("系统还原点和卷影副本占用的空间。可以在系统保护设置中减少它的最大用量，或删除此盘的还原点。", "Space taken by restore points and shadow copies. Their most can be lowered, or this drive's restore points deleted, in the system protection settings.").into(),
                    None,
                    vec![(lang.pick("管理还原点", "Manage restore points").into(), PlateAct::Open(crate::disk::act::Page::Protection))],
                    vec![],
                ),
                Some(Info::ComponentStore) => (
                    lang.pick("组件存储", "Component store").into(),
                    lang.pick("Windows 组件的存储（WinSxS）。其中许多文件与 Windows 文件夹其他位置的文件共用同一份数据，这里每处都计入一次，所以显示的大小大于它实际占用的空间。可以在“存储”设置的清理建议中清理。", "Where Windows keeps its components (WinSxS). Many of its files share their data with files elsewhere in the Windows folder, counted here at each, so it shows larger than the space it really takes. It can be cleaned up in Storage's cleanup recommendations.").into(),
                    None,
                    vec![(lang.pick("打开存储设置", "Open Storage settings").into(), PlateAct::Open(crate::disk::act::Page::Storage))],
                    vec![],
                ),
                Some(Info::Unlisted) => (
                    lang.pick("其他系统数据", "Other system data").into(),
                    lang.pick("文件系统自身的数据（如主文件表和日志），以及无法读取的项目所占的空间，不对应任何单独的文件。", "The file system's own data (its master file table, its log) and what cannot be read: no file of its own.").into(),
                    None,
                    vec![],
                    vec![],
                ),
                Some(Info::Bin) => (
                    lang.pick("回收站", "Recycle Bin").into(),
                    match lang {
                        Lang::Zh => format!("此盘的回收站中有 {} 个文件，共 {}。清空后才会释放这些空间。", count_text(*files), size_text(*bytes, lang)),
                        Lang::En => format!("This drive's Recycle Bin holds {} files, {}. Its space is freed once it is emptied.", count_text(*files), size_text(*bytes, lang)),
                    },
                    None,
                    vec![],
                    vec![(lang.pick("查看内容", "Show what is in it").into(), PlateAct::OpenBin, false), (lang.pick("清空回收站", "Empty Recycle Bin").into(), PlateAct::AskEmpty, true)],
                ),
                None => (name.clone(), lang.pick("系统使用的文件，不可删除。", "A file the system uses: it cannot be deleted.").into(), None, vec![], vec![]),
            },
            PlateOf::Hibernation { .. } => (
                lang.pick("关闭休眠", "Turn hibernation off").into(),
                lang.pick("在以管理员身份打开的终端中运行下面的命令，即可关闭休眠并删除休眠文件。快速启动也会随之关闭。", "Run the command below in a terminal opened as administrator: hibernation is turned off and its file deleted. Fast startup goes off with it.").into(),
                Some("powercfg /hibernate off"),
                vec![],
                vec![(lang.pick("打开电源选项", "Open Power Options").into(), PlateAct::Open(crate::disk::act::Page::PowerButtons), false), (lang.pick("复制命令", "Copy command").into(), PlateAct::CopyCommand, true)],
            ),
            PlateOf::Empty => {
                let (bytes, files) = self.shown.as_ref().map_or((0, 0), |shown| shown.bin);
                (
                    lang.pick("清空回收站", "Empty Recycle Bin").into(),
                    match lang {
                        Lang::Zh => format!("将永久删除此盘回收站中的 {} 个文件，共 {}。此操作无法撤销。", count_text(files), size_text(bytes, lang)),
                        Lang::En => format!("The {} files in this drive's Recycle Bin, {}, are deleted for good. This cannot be undone.", count_text(files), size_text(bytes, lang)),
                    },
                    None,
                    vec![],
                    vec![(lang.pick("取消", "Cancel").into(), PlateAct::Close, false), (lang.pick("清空", "Empty").into(), PlateAct::Empty, true)],
                )
            }
        };
        let words_h = frame.gfx.wrapped_height(&words, body_font, inner);
        let mut tall = 16.0 + 24.0 + 8.0 + words_h;
        if command.is_some() {
            tall += 12.0 + 32.0;
        }
        tall += links.len() as f32 * 28.0 + if links.is_empty() { 0.0 } else { 8.0 };
        if !buttons.is_empty() {
            tall += 16.0 + BUTTON_H;
        }
        tall += 16.0;
        let (px, py) = plate.at;
        let mut r = Rect { x: px, y: if plate.above { py - tall } else { py }, w: PLATE_W, h: tall };
        r.x = r.x.clamp(8.0, g.width - 8.0 - r.w);
        r.y = r.y.clamp(8.0, (g.height - 8.0 - r.h).max(8.0));
        flyout(frame, p, r);
        if live {
            self.spot(r, Target::Over);
        }
        let mut y = r.y + 16.0;
        frame.text(&title, body(16.0, 600.0), p.text, r.x + 16.0, y, inner - 28.0, Align::Start);
        let close = Rect { x: r.x + r.w - 12.0 - 24.0, y: r.y + 12.0, w: 24.0, h: 24.0 };
        let close_target = Target::PlateAct(PlateAct::Close);
        if *hot == Some(close_target.clone()) {
            fill(frame, p.hover, close.x, close.y, close.w, close.h, 4.0);
        }
        frame.icon(Icon::Close, Point { x: close.x + 12.0, y: close.y + 12.0 }, 14.0, p.text2);
        if live {
            self.spot(close, close_target);
        }
        y += 24.0 + 8.0;
        frame.text_wrapped(&words, body_font, p.text2, r.x + 16.0, y, inner);
        y += words_h;
        if let Some(command) = command {
            y += 12.0;
            fill(frame, p.control, r.x + 16.0, y, inner, 32.0, 4.0);
            let mono = Font::new(Family::Mono, 13.0, 400.0);
            frame.text(command, mono, p.text, r.x + 28.0, frame.line_in(mono, y, 32.0), inner - 24.0, Align::Start);
            y += 32.0;
        }
        if !links.is_empty() {
            y += 8.0;
        }
        for (label, act) in links {
            let target = Target::PlateAct(act);
            let w = frame.measure(&label, body_font) + 20.0;
            let lr = Rect { x: r.x + 16.0, y, w, h: 28.0 };
            let ink = if *hot == Some(target.clone()) { p.accent.alpha(0.8) } else { p.accent };
            frame.text(&label, body_font, ink, lr.x, frame.line_in(body_font, lr.y, lr.h), w, Align::Start);
            // (A page of Windows' opens; hibernation's turning off is told of in a card of its own.)
            let icon = if act == PlateAct::Hibernation { Icon::ChevronRight } else { Icon::ExternalLink };
            frame.icon(icon, Point { x: lr.x + w - 8.0, y: lr.y + 14.0 }, 12.0, ink);
            if live {
                self.spot(lr, target);
            }
            y += 28.0;
        }
        if !buttons.is_empty() {
            y += 16.0;
            let mut x = r.x + r.w - 16.0;
            for (label, act, primary) in buttons.into_iter().rev() {
                let w = frame.measure(&label, body_font) + 24.0;
                x -= w;
                let br = Rect { x, y, w, h: BUTTON_H };
                let target = Target::PlateAct(act);
                let lit = self.fade(&target, Look::Hot, *hot == Some(target.clone()), 0.12);
                button(frame, p, br, 6.0, if primary { 1.0 } else { 0.0 }, 1.08, lit);
                centred(frame, &label, body_font, if primary { p.on_accent } else { p.text }, br.x, frame.line_in(body_font, br.y, br.h), br.w);
                if live {
                    self.spot(br, target);
                }
                x -= 8.0;
            }
        }
    }

    /// A menu over the window (its items pressable where `live`: not as it goes).
    fn paint_menu(&mut self, frame: &Frame, p: &Palette, g: Geo, menu: &Menu, live: bool, hot: &Option<Target>) {
        let lang = self.lang;
        let none = None;
        let hot = if live { hot } else { &none };
        let font = body(13.0, 400.0);
        let items: Vec<(Option<Icon>, String, Option<String>, Target, bool)> = match &menu.of {
            MenuOf::Sizes => [0u64, 100 << 20, 1 << 30, 10 << 30]
                .into_iter()
                .map(|least| {
                    let label = least_text(least, lang);
                    (None, label, None, Target::MenuSize(least), self.filter.least == least)
                })
                .collect(),
            MenuOf::Crumbs(crumbs) => crumbs.iter().map(|(name, path)| (Some(Icon::Folder), name.clone(), None, Target::MenuCrumb(path.clone()), false)).collect(),
            MenuOf::Drives(hidden) => hidden
                .iter()
                .filter_map(|&at| self.drives.get(at).map(|drive| (Some(Icon::Disk), format!("{} ({}:)", drive.label, drive.letter()), drive.low().then(|| lang.pick("空间不足", "Low on space").to_string()), Target::MenuDrive(at), false)))
                .collect(),
            MenuOf::Gathered(hidden) => hidden
                .iter()
                .filter_map(|&at| self.gathered.get(at).map(|item| (Some(if item.folder { Icon::Folder } else { kind_icon(item.kind) }), item.name.clone(), Some(size_text(item.bytes, lang)), Target::MenuUngather(at), false)))
                .collect(),
        };
        let checks = matches!(menu.of, MenuOf::Sizes);
        let gathered = matches!(menu.of, MenuOf::Gathered(_));
        let widest = items.iter().map(|(_, label, size, _, _)| frame.measure(label, font).min(260.0) + size.as_ref().map_or(0.0, |size| frame.measure(size, body(12.0, 400.0)) + 12.0)).fold(0.0, f32::max);
        let w = (widest + 12.0 + 28.0 + 12.0 + if gathered { 24.0 } else { 0.0 }).max(menu.under.w).max(160.0);
        let tall = items.len() as f32 * 32.0 + 8.0;
        let mut r = Rect { x: menu.under.x, y: if menu.above { menu.under.y - 4.0 - tall } else { menu.under.y + menu.under.h + 4.0 }, w, h: tall };
        r.x = r.x.clamp(8.0, g.width - 8.0 - r.w);
        r.y = r.y.clamp(8.0, (g.height - 8.0 - r.h).max(8.0));
        flyout(frame, p, r);
        if live {
            self.spot(r, Target::Over);
        }
        for (n, (icon, label, size, target, on)) in items.into_iter().enumerate() {
            let ir = Rect { x: r.x + 4.0, y: r.y + 4.0 + n as f32 * 32.0, w: r.w - 8.0, h: 32.0 };
            if *hot == Some(target.clone()) {
                fill(frame, p.hover, ir.x, ir.y, ir.w, ir.h, 4.0);
            }
            if on {
                fill(frame, p.accent, ir.x, ir.y + 9.0, 3.0, 14.0, 1.5);
            }
            let text_x = if checks || icon.is_some() { ir.x + 36.0 } else { ir.x + 12.0 };
            if let Some(icon) = icon {
                frame.icon(icon, Point { x: ir.x + 18.0, y: ir.y + 16.0 }, 14.0, p.text2);
            }
            if checks && on {
                frame.icon(Icon::Check, Point { x: ir.x + 18.0, y: ir.y + 16.0 }, 14.0, p.accent);
            }
            let right = ir.x + ir.w - if gathered { 30.0 } else { 12.0 };
            let mut label_right = right;
            if let Some(size) = &size {
                let size_w = frame.measure(size, body(12.0, 400.0)) + 2.0;
                frame.text(size, body(12.0, 400.0), p.text3, right - size_w, frame.line_in(body(12.0, 400.0), ir.y, ir.h), size_w, Align::End);
                label_right = right - size_w - 10.0;
            }
            frame.text(&label, font, p.text, text_x, frame.line_in(font, ir.y, ir.h), label_right - text_x, Align::Start);
            if gathered {
                frame.icon(Icon::Close, Point { x: ir.x + ir.w - 16.0, y: ir.y + 16.0 }, 12.0, p.text2);
            }
            if live {
                self.spot(ir, target);
            }
        }
    }
}

/// `fill`, its box on whole pixels, its radius no more than half its sides.
pub(super) fn fill(frame: &Frame, color: Color, x: f32, y: f32, w: f32, h: f32, radius: f32) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    if radius <= 0.0 {
        Canvas::fill(frame, color, x, y, w, h);
    } else {
        Canvas::fill_rounded(frame, color, x, y, w, h, radius.min(w / 2.0).min(h / 2.0));
    }
}

/// The least size the files shown may be, as the size filter says it ("大于 1 GB"): its figure whole.
fn least_text(least: u64, lang: Lang) -> String {
    if least == 0 {
        return lang.pick("不限大小", "Any size").into();
    }
    let (value, unit) = if least >= 1 << 30 { (least >> 30, "GB") } else { (least >> 20, "MB") };
    format!("{} {value}\u{a0}{unit}", lang.pick("大于", "Over"))
}

/// `text` centred in the `w` from `x`.
fn centred(frame: &Frame, text: &str, font: Font, color: Color, x: f32, y: f32, w: f32) {
    let width = frame.measure(text, font).min(w);
    frame.text(text, font, color, x + (w - width) / 2.0, y, width + 2.0, Align::Start);
}

/// A one-DIP line inside the box.
fn stroke_inside(frame: &Frame, r: Rect, radius: f32, color: Color) {
    Canvas::stroke_rounded(frame, crate::ui::canvas::Fill::Solid(color), r.x, r.y, r.w, r.h, radius, 1.0);
}

/// A card (Windows 11's): filled, a hairline round it.
fn card(frame: &Frame, p: &Palette, r: Rect) {
    fill(frame, p.card, r.x, r.y, r.w, r.h, CARD_RADIUS);
    stroke_inside(frame, r, CARD_RADIUS, p.card_stroke);
}

/// A flyout (a tag, a card, a menu): its shadow, its fill, its hairline.
fn flyout(frame: &Frame, p: &Palette, r: Rect) {
    frame.shadow(Color::hex(0, if p.dark { 0.26 } else { 0.14 }), r.x, r.y, r.w, r.h, 8.0, 8.0, 4.0);
    fill(frame, p.fly, r.x, r.y, r.w, r.h, 8.0);
    stroke_inside(frame, r, 8.0, p.fly_stroke);
}

/// Windows 11's button, its corners rounded by `radius`: standard, or as much the accent's as `pressed` says (0 to 1),
/// as much under the pointer as `hover` says (the accent's then lifted `lift` times as bright).
fn button(frame: &Frame, p: &Palette, r: Rect, radius: f32, pressed: f32, lift: f32, hover: f32) {
    let standard = mix(p.button, p.button_hover, hover);
    let accent = bright(p.accent, 1.0 + (lift - 1.0) * hover);
    fill(frame, mix(standard, accent, pressed), r.x, r.y, r.w, r.h, radius);
    if pressed < 1.0 {
        stroke_inside(frame, r, radius, p.button_stroke.alpha(1.0 - pressed));
        within(frame, r, radius, 1.0 - pressed, || fill(frame, p.button_edge, r.x, r.y + r.h - 1.0, r.w, 1.0, 0.0));
    }
}

/// A row's tool: a standard button, the accent's under the pointer.
fn row_button(frame: &Frame, p: &Palette, r: Rect, icon: Icon, hot: bool) {
    button(frame, p, r, 4.0, if hot { 1.0 } else { 0.0 }, 1.0, 0.0);
    frame.icon(icon, Point { x: r.x + r.w / 2.0, y: r.y + r.h / 2.0 }, 14.0, if hot { p.on_accent } else { p.text });
}

/// How wide a block's button freeing it to the cloud is: its icon and words.
fn release_width(frame: &Frame, lang: Lang) -> f32 {
    6.0 + 14.0 + 6.0 + frame.measure(lang.pick("释放空间", "Free up space"), body(12.0, 400.0)) + 8.0
}

/// `a` going to `b`, as far as `t` says (0 to 1).
fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color { r: a.r + (b.r - a.r) * t, g: a.g + (b.g - a.g) * t, b: a.b + (b.b - a.b) * t, a: a.a + (b.a - a.a) * t }
}

/// `c` `k` times as bright, as CSS's brightness() makes it.
fn bright(c: Color, k: f32) -> Color {
    Color { r: (c.r * k).min(1.0), g: (c.g * k).min(1.0), b: (c.b * k).min(1.0), a: c.a }
}

/// What is drawn while it is held seen through as much as its opacity says (see `faint`).
struct Layer<'f>(&'f Frame<'f>);

impl Drop for Layer<'_> {
    fn drop(&mut self) {
        unsafe { self.0.dc.PopLayer() };
    }
}

/// What is drawn till what this gives is dropped seen through as much as `opacity` says (1: as it is).
fn faint<'f>(frame: &'f Frame<'f>, opacity: f32) -> Option<Layer<'f>> {
    if opacity >= 1.0 {
        return None;
    }
    let everything = windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F { left: -f32::MAX, top: -f32::MAX, right: f32::MAX, bottom: f32::MAX };
    let layer = D2D1_LAYER_PARAMETERS1 { contentBounds: everything, opacity: opacity.max(0.0), ..Default::default() };
    unsafe { frame.dc.PushLayer(&layer, None) };
    Some(Layer(frame))
}

/// The transform drawn through before (see `shifted`), put back as what this holds is dropped.
struct Moved<'f>(&'f Frame<'f>, Matrix3x2);

impl Drop for Moved<'_> {
    fn drop(&mut self) {
        unsafe { self.0.dc.SetTransform(&self.1) };
    }
}

/// What is drawn till what this gives is dropped put `dx`, `dy` further, and (with `scaled`) that many times as large
/// about its point.
fn shifted<'f>(frame: &'f Frame<'f>, dx: f32, dy: f32, scaled: Option<(f32, f32, f32)>) -> Moved<'f> {
    let mut was = Matrix3x2::default();
    unsafe { frame.dc.GetTransform(&mut was) };
    let mut about = Matrix3x2::translation(dx, dy);
    if let Some((x, y, scale)) = scaled {
        about = Matrix3x2::translation(-x, -y) * Matrix3x2 { M11: scale, M12: 0.0, M21: 0.0, M22: scale, M31: 0.0, M32: 0.0 } * Matrix3x2::translation(x + dx, y + dy);
    }
    unsafe { frame.dc.SetTransform(&(about * was)) };
    Moved(frame, was)
}

/// An outline `width` DIPs wide round `e` (inside it), a block's corners or a piece's (all but square), a ring of
/// `inner` inside it; round a block too small for one, a circle about its middle.
fn outline(frame: &Frame, e: Rect, piece: bool, color: Color, width: f32, inner: Option<Color>) {
    let radius = crate::disk::map::RADIUS;
    if e.w < 8.0 || e.h < 8.0 {
        frame.arc(Point { x: e.x + e.w / 2.0, y: e.y + e.h / 2.0 }, 7.0, 0.0, TAU, color, width);
        return;
    }
    Canvas::stroke_rounded(frame, Fill::Solid(color), e.x, e.y, e.w, e.h, if piece { 1.0 } else { radius }, width);
    if let Some(inner) = inner {
        if e.w > 2.0 * width + 4.0 && e.h > 2.0 * width + 4.0 {
            Canvas::stroke_rounded(frame, Fill::Solid(inner), e.x + width, e.y + width, e.w - 2.0 * width, e.h - 2.0 * width, if piece { 0.0 } else { radius - width }, 1.0);
        }
    }
}

/// The dim of what is known as `key` eased toward `to` by `k` (where it had none, at it at once); whether it moves
/// still told in `moving`.
fn eased(dims: &mut HashMap<Key, f32>, key: Key, to: f32, k: f32, moving: &mut bool) -> f32 {
    let dim = dims.entry(key).or_insert(to);
    if (to - *dim).abs() > 0.004 {
        *dim += (to - *dim) * k;
        *moving = true;
    } else {
        *dim = to;
    }
    *dim
}

/// What is dragged, by the pointer (above it, to its right): its icon, its name and size, or the reason it may not
/// go; seen as much as `opacity` says, `scale` times as large.
fn paint_ghost(frame: &Frame, p: &Palette, ghost: &Ghost, opacity: f32, scale: f32) {
    let font = body(13.0, 400.0);
    let size_font = body(12.0, 400.0);
    let (words, size) = match &ghost.why_not {
        Some(why) => (why.as_str(), None),
        None => (ghost.name.as_str(), Some(ghost.size.as_str())),
    };
    let words_w = frame.measure(words, font).min(360.0);
    let size_w = size.map_or(0.0, |size| 8.0 + frame.measure(size, size_font));
    let r = Rect { x: ghost.at.0 + 14.0, y: ghost.at.1 - 40.0, w: 10.0 + 14.0 + 8.0 + words_w + size_w + 12.0, h: 32.0 };
    let _faint = faint(frame, opacity);
    let _moved = shifted(frame, 0.0, 0.0, Some((r.x + r.w / 2.0, r.y + r.h / 2.0, scale)));
    frame.shadow(Color::hex(0, 0.2), r.x, r.y, r.w, r.h, 6.0, 8.0, 8.0);
    fill(frame, p.fly, r.x, r.y, r.w, r.h, 6.0);
    Canvas::stroke_rounded(frame, Fill::Solid(p.fly_stroke), r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0, 7.0, 1.0);
    frame.icon(ghost.icon, Point { x: r.x + 10.0 + 7.0, y: r.y + r.h / 2.0 }, 14.0, p.text2);
    let ink = if ghost.why_not.is_some() { p.text2 } else { p.text };
    frame.text(words, font, ink, r.x + 32.0, frame.line_in(font, r.y, r.h), words_w + 2.0, Align::Start);
    if let Some(size) = size {
        frame.text(size, size_font, p.text2, r.x + 32.0 + words_w + 8.0, frame.line_in(size_font, r.y, r.h), size_w, Align::Start);
    }
}

/// Stripes across `r`, at 45°, every 6 DIPs (cut to what they are drawn within): on the same lines wherever they are
/// drawn, so stripes over pieces side by side run on from one into the next.
fn stripes(frame: &Frame, r: Rect, color: Color) {
    const EVERY: f32 = 6.0;
    // (Each line where x + y is a whole number of strides, from the corner at the top left to the one at the foot.)
    let mut k = ((r.x + r.y) / EVERY).floor() * EVERY;
    while k <= r.x + r.w + r.y + r.h {
        frame.stroke(&[Point { x: k - (r.y + r.h), y: r.y + r.h }, Point { x: k - r.y, y: r.y }], color, 1.5);
        k += EVERY;
    }
}

/// Whether `path` is `top` or in it (as written: both read from the same index, alike in case), by its text: as
/// quick as a comparison, for every piece of the map, each frame.
pub(super) fn under(path: &Path, top: &Path) -> bool {
    let (path, top) = (path.as_os_str().as_encoded_bytes(), top.as_os_str().as_encoded_bytes());
    path.starts_with(top) && (path.len() == top.len() || top.ends_with(b"\\") || path[top.len()] == b'\\')
}

/// `draw` drawn only within `r`, its corners rounded by `radius`, `opacity` of it.
pub(super) fn within(frame: &Frame, r: Rect, radius: f32, opacity: f32, draw: impl FnOnce()) {
    if r.w <= 0.0 || r.h <= 0.0 {
        return;
    }
    let (x, y, w, h) = frame.on_pixels(r.x, r.y, r.w, r.h);
    let radius = radius.min(w / 2.0).min(h / 2.0);
    let Ok(mask) = (unsafe { frame.gfx.factory.CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT { rect: rect(x, y, w, h), radiusX: radius, radiusY: radius }) }) else { return };
    let layer = D2D1_LAYER_PARAMETERS1 {
        contentBounds: rect(x, y, w, h),
        geometricMask: std::mem::ManuallyDrop::new(windows::core::Interface::cast(&mask).ok()),
        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
        maskTransform: Matrix3x2::identity(),
        opacity: opacity.clamp(0.0, 1.0),
        ..Default::default()
    };
    unsafe { frame.dc.PushLayer(&layer, None) };
    draw();
    unsafe { frame.dc.PopLayer() };
    drop(std::mem::ManuallyDrop::into_inner(layer.geometricMask));
}
