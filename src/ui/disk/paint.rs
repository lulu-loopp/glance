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
use crate::ui::canvas::{Align, Canvas, Color, Family, Font, Point};
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
const TABS_H: f32 = 44.0;
const SUM_H: f32 = 28.0;
const CARD_RADIUS: f32 = 8.0;
const CHIP_H: f32 = 24.0;
const BUTTON_H: f32 = 32.0;
const PLATE_W: f32 = 320.0;
/// The mark under the tab shown (its width), as the settings' own tabs' is.
const TAB_MARK: f32 = 16.0;
const TAG_W: f32 = 280.0;
/// How dim what is not lit is drawn, and a ground then.
const DIM: f32 = 0.28;
const GROUND_DIM: f32 = 0.5;

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
    Rect { x: side.x + 8.0, y: side.y + TABS_H + SUM_H, w: side.w - 16.0, h: side.h - TABS_H - SUM_H - 8.0 }
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
                    Some(Light::Kind(kind)) if !self.filter.on() || self.peek.is_some() => node.kind() == *kind && (!self.filter.on() || self.lit_place(shown, at)),
                    _ => self.lit_place(shown, at),
                },
            }
        };
        match item {
            Item::Node(at) => node_lit(*at),
            Item::Many { members, .. } => members.iter().any(|&member| node_lit(member)),
        }
    }

    fn lit_place(&self, shown: &Shown, at: usize) -> bool {
        let node = &shown.tree.nodes[at];
        match node.what {
            What::Rest => shown.detail.lit_rests.contains(&node.place),
            _ => shown.detail.lit_folders.contains(&node.place),
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

    /// Everything drawn; whether anything moves still.
    pub(super) fn paint(&mut self, frame: &Frame, now: Instant, dt: f32) -> bool {
        let palette = self.palette.clone();
        let (width, height) = self.client();
        let g = geo(width, height);
        let hot = self.hot();
        // The legend's kind under the pointer lights the map while it is there.
        self.peek = match &hot {
            Some(Target::Legend(kind)) => Some(Light::Kind(*kind)),
            _ => None,
        };
        self.targets.clear();
        let mut moving = false;
        self.paint_head(frame, &palette, g, dt, &mut moving);
        if self.root.is_empty() {
            self.paint_no_drive(frame, &palette, g);
            return moving;
        }
        moving |= self.paint_drives(frame, &palette, g, dt, &hot);
        self.paint_tools(frame, &palette, g, &hot);
        moving |= self.paint_map_card(frame, &palette, g, now, dt, &hot);
        moving |= self.paint_side(frame, &palette, g, dt, &hot);
        moving |= self.paint_foot(frame, &palette, g, now, &hot);
        self.paint_over(frame, &palette, g, now, &hot);
        moving
    }

    fn paint_no_drive(&mut self, frame: &Frame, p: &Palette, g: Geo) {
        let lang = self.lang;
        let top = g.height / 2.0 - 30.0;
        frame.icon(Icon::Disk, Point { x: PAGE_X + g.page_w / 2.0, y: top }, 28.0, p.text3);
        centred(frame, lang.pick("没有可以显示的磁盘", "No drive to show"), body(16.0, 600.0), p.text, PAGE_X, top + 28.0, g.page_w);
        centred(frame, lang.pick("磁盘整理读取搜索的索引，开启搜索后即可在这里查看磁盘。", "The organizer reads search's index: drives are shown here once search is on."), body(13.0, 400.0), p.text2, PAGE_X, top + 56.0, g.page_w);
    }

    /// The page's title, as the settings' pages have theirs, and at its right the use of the drive shown.
    fn paint_head(&mut self, frame: &Frame, p: &Palette, g: Geo, dt: f32, moving: &mut bool) {
        let lang = self.lang;
        let title_font = display(28.0, 600.0);
        frame.text(lang.pick("磁盘整理", "Disk Organizer"), title_font, p.text, PAGE_X, frame.line_in(title_font, TITLE_Y, 44.0), g.page_w - 300.0, Align::Start);
        let Some(drive) = self.drives.iter().find(|drive| drive.root.eq_ignore_ascii_case(&self.root)).cloned() else { return };
        let used = drive.total.saturating_sub(drive.free);
        // The meter eased toward what is used (what went shows going).
        if self.used_shown.0 != drive.root {
            self.used_shown = (drive.root.clone(), used as f64);
        }
        let k = 1.0 - (-dt * 1000.0 / 120.0).exp();
        self.used_shown.1 += (used as f64 - self.used_shown.1) * k as f64;
        if (self.used_shown.1 - used as f64).abs() > drive.total as f64 * 0.0005 {
            *moving = true;
        } else {
            self.used_shown.1 = used as f64;
        }
        let bin = self.shown.as_ref().filter(|shown| shown.root == drive.root).map_or(0, |shown| shown.bin.0);
        let pick: u64 = self.gathered.iter().map(|item| item.bytes).sum::<u64>() + self.pending.as_ref().map_or(0, |pending| pending.items.iter().map(|item| item.bytes).sum());
        let block_w = 500.0f32.min(g.page_w - 320.0).max(240.0);
        let right = PAGE_X + g.page_w;
        let mut runs: Vec<(String, bool)> = vec![(lang.pick("已用 ", "Used ").into(), false), (size_text(used, lang), true), (format!("{}{}{}{}", lang.pick("，共 ", ", of "), size_text(drive.total, lang), lang.pick("，可用 ", ", free "), size_text(drive.free, lang)), false)];
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
        // The meter: what is used (the Recycle Bin's part and what is gathered striped, at its end).
        let bar = Rect { x: right - block_w, y: 56.0, w: block_w, h: 6.0 };
        fill(frame, p.meter, bar.x, bar.y, bar.w, bar.h, 3.0);
        if drive.total > 0 {
            let of = |bytes: f64| (bytes / drive.total as f64).clamp(0.0, 1.0) as f32 * bar.w;
            let used_w = of(self.used_shown.1);
            let pick_w = of(pick as f64).min(used_w);
            let bin_w = of(bin as f64).min(used_w - pick_w);
            within(frame, bar, 3.0, 1.0, || {
                fill(frame, p.accent, bar.x, bar.y, used_w - pick_w - bin_w, bar.h, 0.0);
                if bin_w > 0.0 {
                    let r = Rect { x: bar.x + used_w - pick_w - bin_w, w: bin_w, ..bar };
                    fill(frame, p.text3.alpha(p.text3.a * 0.6), r.x, r.y, r.w, r.h, 0.0);
                    stripes(frame, r, p.stripe);
                }
                if pick_w > 0.0 {
                    let r = Rect { x: bar.x + used_w - pick_w, w: pick_w, ..bar };
                    fill(frame, p.accent.alpha(0.5), r.x, r.y, r.w, r.h, 0.0);
                    stripes(frame, r, p.stripe);
                }
            });
        }
    }

    /// The drives, a tab each, as the settings' own tabs are: the one shown marked under its middle by a short bar in
    /// the accent, a drive nearly out of space by a dot after its name (told of under the pointer). Those that do not
    /// fit in a menu, the one shown never among them. Whether the mark moves still.
    fn paint_drives(&mut self, frame: &Frame, p: &Palette, g: Geo, dt: f32, hot: &Option<Target>) -> bool {
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
            if *hot == Some(target.clone()) && !is_shown {
                fill(frame, p.hover, r.x, r.y + 4.0, r.w, r.h - 8.0, 6.0);
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
            if *hot == Some(target.clone()) {
                fill(frame, p.hover, r.x, r.y, r.w, r.h, 6.0);
            }
            frame.icon(Icon::Ellipsis, Point { x: r.x + r.w / 2.0, y: cy }, 16.0, p.text2);
            // (A drive among them nearly out of space: its dot on the button too.)
            if hidden.iter().any(|&at| drives[at].low()) {
                frame.fill_circle(p.signal, Point { x: r.x + r.w - 6.0, y: r.y + 6.0 }, 3.0);
            }
            self.spot(r, target);
        }
        // The mark goes to the drive chosen, as the list's tabs' does.
        let Some(middle) = middle else { return false };
        let to = middle - TAB_MARK / 2.0;
        let at = self.drive_mark.get_or_insert(to);
        let k = 1.0 - (-dt * 1000.0 / 60.0).exp();
        *at += (to - *at) * k;
        let moving = (to - *at).abs() > 0.3;
        if !moving {
            *at = to;
        }
        fill(frame, p.accent, *at, TABS_Y + h - 2.0 - 3.0, TAB_MARK, 3.0, 1.5);
        moving
    }

    /// Back, the path to the folder shown, and the filters.
    fn paint_tools(&mut self, frame: &Frame, p: &Palette, g: Geo, hot: &Option<Target>) {
        let lang = self.lang;
        let top = TOOLS_Y;
        let right = PAGE_X + g.page_w;
        // The filters from the right.
        let preset = self.filter.old && self.filter.least == 1 << 30;
        let size_label = match self.filter.least {
            0 => lang.pick("大小", "Size").to_string(),
            least => least_text(least, lang),
        };
        let buttons: [(Target, Icon, String, bool, bool); 3] = [
            (Target::FilterOld, Icon::Clock, lang.pick("超过一年未修改", "Not changed in a year").into(), self.filter.old && !preset, false),
            (Target::FilterSize, Icon::BySize, size_label, self.filter.least > 0 && !preset, true),
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
            let hovered = *hot == Some(target.clone());
            button(frame, p, r, on, hovered);
            let ink = if on { p.on_accent } else { p.text };
            frame.icon(icon, Point { x: r.x + 11.0 + 7.0, y: r.y + r.h / 2.0 }, 14.0, ink);
            frame.text(&label, font, ink, r.x + 11.0 + 14.0 + 8.0, frame.line_in(font, r.y, r.h), r.w, Align::Start);
            if menu {
                frame.icon(Icon::ChevronDown, Point { x: r.x + r.w - 11.0 - 6.0, y: r.y + r.h / 2.0 }, 12.0, ink);
            }
            self.spot(r, target);
        }
        // Back, then the path.
        let back = Rect { x: PAGE_X, y: top, w: 32.0, h: 32.0 };
        let can_go_up = !self.at_root();
        if can_go_up && *hot == Some(Target::Back) {
            fill(frame, p.hover, back.x, back.y, back.w, back.h, 6.0);
        }
        frame.icon(Icon::ArrowLeft, Point { x: back.x + 16.0, y: back.y + 16.0 }, 16.0, if can_go_up { p.text } else { p.text3.alpha(p.text3.a * 0.6) });
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
            if *hot == Some(target.clone()) {
                fill(frame, p.hover, r.x, r.y, r.w, r.h, 6.0);
            }
            frame.icon(Icon::Ellipsis, Point { x: r.x + 16.0, y: r.y + 16.0 }, 16.0, p.text2);
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
            if !last && *hot == Some(target.clone()) {
                fill(frame, p.hover, r.x, r.y, r.w, r.h, 6.0);
            }
            let font = crumb_font(last);
            frame.text(&crumbs[at].0, font, if last { p.text } else { p.text2 }, r.x + 6.0, frame.line_in(font, r.y, r.h), r.w - 12.0, Align::Start);
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

    /// The map's card: the map, and its legend under it. Whether it moves still.
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
        let inner_w = g.card.w - 24.0;
        let mut lines: Vec<Vec<(Kind, String, f32)>> = vec![Vec::new()];
        let mut line_w = 0.0;
        for (kind, bytes) in kinds {
            let text = format!("{} {}", kind_name(kind, lang), size_text(bytes, lang));
            let w = 8.0 + 10.0 + 6.0 + frame.measure(&text, chip_font) + 8.0;
            if line_w + w > inner_w && !lines.last().unwrap().is_empty() {
                lines.push(Vec::new());
                line_w = 0.0;
            }
            line_w += w + 4.0;
            lines.last_mut().unwrap().push((kind, text, w));
        }
        let legend_h = lines.len() as f32 * CHIP_H + (lines.len() as f32 - 1.0) * 6.0;
        let map = Rect { x: g.card.x + 12.0, y: g.card.y + 12.0, w: inner_w, h: (g.card.h - 12.0 - 12.0 - legend_h - 12.0).max(40.0) };
        let lighting = self.peek.clone().or(self.light.clone());
        let mut y = map.y + map.h + 12.0;
        for line in lines {
            let mut x = map.x;
            for (kind, text, w) in line {
                let r = Rect { x, y, w, h: CHIP_H };
                let on = self.light == Some(Light::Kind(kind));
                let target = Target::Legend(kind);
                if on {
                    fill(frame, p.control, r.x, r.y, r.w, r.h, 4.0);
                    stroke_inside(frame, r, 4.0, p.accent);
                } else if *hot == Some(target.clone()) {
                    fill(frame, p.hover, r.x, r.y, r.w, r.h, 4.0);
                }
                let faded = lighting.as_ref().is_some_and(|light| *light != Light::Kind(kind)) && matches!(lighting, Some(Light::Kind(_)));
                let alpha = if faded { 0.5 } else { 1.0 };
                fill(frame, p.fill[kind.at()].alpha(alpha), r.x + 8.0, r.y + 7.0, 10.0, 10.0, 2.0);
                frame.text(&text, chip_font, p.text2.alpha(p.text2.a * alpha), r.x + 24.0, frame.line_in(chip_font, r.y, r.h), r.w - 28.0, Align::Start);
                self.spot(r, target);
                x += w + 4.0;
            }
            y += CHIP_H + 6.0;
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
        let moving = self.paint_map(frame, p, map, now, dt, lighting.as_ref());
        // The tools on the block under the pointer.
        if let (Some(at), None, None) = (self.hover, &self.drag, &self.anim) {
            self.paint_block_tools(frame, p, map, at, hot);
        }
        moving
    }

    /// The map's blocks, as they are now (moving, if they do). Whether anything moves still.
    fn paint_map(&mut self, frame: &Frame, p: &Palette, map: Rect, now: Instant, dt: f32, lighting: Option<&Light>) -> bool {
        let lang = self.lang;
        let blocks = self.now_rects(now);
        let animating = self.anim.as_ref().is_some_and(|anim| now < anim.began + anim.lasts);
        if !animating {
            self.anim = None;
        }
        let lights = self.filter.on() || lighting.is_some();
        let k = 1.0 - (-dt * 1000.0 / 70.0).exp();
        let mut dims_moving = false;
        frame.clip(map.x - 2.0, map.y - 2.0, map.w + 4.0, map.h + 4.0);
        let hover_key = self.hover.and_then(|at| self.laid.get(at)).map(|placed| placed.key);
        let chosen = self.chosen.clone();
        let any_gathered = !self.gathered.is_empty() || !self.going.is_empty() || self.pending.is_some();
        // A ground's outline (under the pointer, or gathered), over what it holds: drawn once the blocks are.
        let mut grounds_over: Vec<(Rect, Color, bool)> = Vec::new();
        for (placed, at, alpha, group) in blocks {
            let r = Rect { x: map.x + at.x, y: map.y + at.y, w: at.w, h: at.h };
            if r.w < 0.5 || r.h < 0.5 || alpha <= 0.01 {
                continue;
            }
            let shown = &placed.shown;
            let tree = &shown.tree;
            let item = &placed.block.item;
            let kind = item.kind(tree);
            let target = if !lights {
                1.0
            } else if group {
                GROUND_DIM
            } else if self.lit(shown, item, lighting) {
                1.0
            } else {
                DIM
            };
            let dim = self.dims.entry(placed.key).or_insert(target);
            *dim += (target - *dim) * k;
            if (target - *dim).abs() > 0.004 {
                dims_moving = true;
            } else {
                *dim = target;
            }
            let dim = *dim;
            let about = self.about(shown, item);
            let hovered = hover_key == Some(placed.key) && !animating;
            let gathered = !matches!(about.what, Some(What::Unlisted)) && about.many_in.is_none() && self.gathered_at(&about.path);
            if group {
                // A ground: its name above what it holds, its size at the right where there is room.
                fill(frame, p.ground.alpha(p.ground.a * alpha * dim), r.x, r.y, r.w, r.h, crate::disk::map::RADIUS);
                if hovered {
                    grounds_over.push((r, p.edge.alpha(p.edge.a * alpha), false));
                }
                let font = body(12.0, 600.0);
                let size = size_text(about.bytes, lang);
                let size_w = frame.measure(&size, body(12.0, 400.0));
                let ink = p.text.alpha(p.text.a * alpha * dim.max(0.5));
                let y = frame.line_in(font, r.y, crate::disk::map::LABEL);
                let room = r.w - 16.0;
                if room > size_w + 48.0 {
                    frame.text(&about.name, font, ink, r.x + 8.0, y, room - size_w - 8.0, Align::Start);
                    frame.text(&size, body(12.0, 400.0), p.text2.alpha(p.text2.a * alpha * dim.max(0.5)), r.x + 8.0, y, room, Align::End);
                } else {
                    frame.text(&about.name, font, ink, r.x + 8.0, y, room, Align::Start);
                }
                if gathered {
                    within(frame, r, crate::disk::map::RADIUS, alpha, || stripes(frame, r, p.stripe));
                    grounds_over.push((r, p.text2.alpha(p.text2.a * alpha), true));
                }
                continue;
            }
            let fills = if hovered { &p.fill_hot } else { &p.fill };
            let pieces = &placed.block.pieces;
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
                for fold in &placed.block.folds {
                    mark(&fold.item, &fold.at, true);
                }
                for piece in pieces {
                    mark(&piece.item, &piece.at, false);
                }
            }
            within(frame, r, crate::disk::map::RADIUS, alpha * dim, || {
                if pieces.is_empty() {
                    fill(frame, fills[kind.at()], r.x, r.y, r.w, r.h, 0.0);
                } else {
                    fill(frame, p.ring, r.x, r.y, r.w, r.h, 0.0);
                    for piece in pieces {
                        let pr = piece_rect(&piece.at);
                        let piece_kind = piece.item.kind(tree);
                        let piece_lit = !lights || self.lit(shown, &piece.item, lighting);
                        let fill_color = fills[piece_kind.at()];
                        fill(frame, if piece_lit { fill_color } else { fill_color.alpha(0.45) }, pr.x, pr.y, pr.w, pr.h, 0.0);
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
                        frame.text(&about.name, name_font, p.text, r.x + 8.0, r.y + 6.0, r.w - 14.0, Align::Start);
                        frame.text(&size, body(12.0, 400.0), p.text2, r.x + 8.0, r.y + 22.0, r.w - 14.0, Align::Start);
                    } else {
                        frame.text(&about.name, name_font, p.text, r.x + 6.0, frame.line_in(name_font, r.y, r.h), r.w - 10.0, Align::Start);
                    }
                }
            });
            if gathered {
                frame.stroke_rounded_dashed(p.text.alpha(p.text.a * alpha * 0.7), r.x, r.y, r.w, r.h, crate::disk::map::RADIUS, 1.0);
            }
            for at in &outlined {
                frame.stroke_rounded_dashed(p.text.alpha(p.text.a * alpha * dim * 0.7), at.x, at.y, at.w, at.h, 0.0, 1.0);
            }
            if chosen.as_ref().is_some_and(|chosen| *chosen == about.path) && about.many_in.is_none() && !matches!(about.what, Some(What::Rest | What::Unlisted)) {
                stroke_inside(frame, r, crate::disk::map::RADIUS, p.accent.alpha(alpha));
                stroke_inside(frame, Rect { x: r.x + 1.0, y: r.y + 1.0, w: r.w - 2.0, h: r.h - 2.0 }, crate::disk::map::RADIUS - 1.0, p.accent.alpha(alpha));
                stroke_inside(frame, Rect { x: r.x + 2.0, y: r.y + 2.0, w: r.w - 4.0, h: r.h - 4.0 }, (crate::disk::map::RADIUS - 2.0).max(0.0), p.ring.alpha(alpha));
            } else if hovered {
                Canvas::stroke_rounded(frame, crate::ui::canvas::Fill::Solid(p.edge), r.x + 0.75, r.y + 0.75, r.w - 1.5, r.h - 1.5, crate::disk::map::RADIUS, 1.5);
            }
        }
        for (r, color, dashed) in grounds_over {
            if dashed {
                frame.stroke_rounded_dashed(color, r.x, r.y, r.w, r.h, crate::disk::map::RADIUS, 1.0);
            } else {
                stroke_inside(frame, r, crate::disk::map::RADIUS, color);
            }
        }
        frame.unclip();
        // A folder holding nothing that takes room: told so where its map would be.
        if !animating && self.laid.is_empty() && self.shown.is_some() {
            centred(frame, lang.pick("这个文件夹没有占用空间的内容", "Nothing in this folder takes up space"), body(13.0, 400.0), p.text3, map.x, map.y + map.h / 2.0 - 10.0, map.w);
        }
        // (What is dimmed no longer drawn is let go of.)
        if !animating {
            let keys: std::collections::HashSet<Key> = self.laid.iter().map(|placed| placed.key).collect();
            self.dims.retain(|key, _| keys.contains(key));
        }
        animating || dims_moving
    }

    /// The tools of the block hovered: gathered to go (or, a system item, told of), and freed to OneDrive.
    fn paint_block_tools(&mut self, frame: &Frame, p: &Palette, map: Rect, at: usize, hot: &Option<Target>) {
        let Some(placed) = self.laid.get(at).cloned() else { return };
        let about = self.about(&placed.shown, &placed.block.item);
        let b = placed.block.at;
        let r = Rect { x: map.x + b.x, y: map.y + b.y, w: b.w, h: b.h };
        if r.w < 40.0 || r.h < 32.0 {
            return;
        }
        let mut right = r.x + r.w - 4.0;
        let tool = |frame: &Frame, x: f32, icon: Icon, target: &Target| {
            let t = Rect { x: x - 24.0, y: r.y + 4.0, w: 24.0, h: 24.0 };
            let hovered = *hot == Some(target.clone());
            frame.shadow(Color::hex(0, if p.dark { 0.3 } else { 0.12 }), t.x, t.y, t.w, t.h, 6.0, 3.0, 1.0);
            fill(frame, if hovered { p.fly } else { p.fly.alpha(0.94) }, t.x, t.y, t.w, t.h, 6.0);
            stroke_inside(frame, t, 6.0, p.fly_stroke);
            frame.icon(icon, Point { x: t.x + 12.0, y: t.y + 12.0 }, 14.0, p.text);
            t
        };
        let info_like = about.info.is_some() || matches!(about.what, Some(What::File { system: true }));
        if info_like {
            let target = Target::AddTool(at);
            let t = tool(frame, right, Icon::Info, &target);
            self.spot(t, target);
            return;
        }
        if about.why_not.is_none() {
            let target = Target::AddTool(at);
            let t = tool(frame, right, Icon::Add, &target);
            self.spot(t, target);
            right -= 28.0;
        }
        let releasable = about.onedrive && about.bytes > 0 && matches!(about.what, Some(What::File { system: false } | What::Folder(Role::Plain)));
        if releasable && r.w >= 72.0 {
            let target = Target::ReleaseTool(at);
            let t = tool(frame, right, Icon::Cloud, &target);
            self.spot(t, target);
        }
    }

    /// The lists beside the map. Whether they move still.
    fn paint_side(&mut self, frame: &Frame, p: &Palette, g: Geo, dt: f32, hot: &Option<Target>) -> bool {
        let lang = self.lang;
        let side = g.side;
        card(frame, p, side);
        let mut moving = false;
        // The tabs, the chosen one's mark sliding to it.
        let tabs = [(Tab::Largest, lang.pick("最大的文件", "Largest files")), (Tab::Types, lang.pick("按类型", "By type")), (Tab::Ages, lang.pick("按时间", "By time"))];
        let font = body(13.0, 600.0);
        let mut x = side.x + 8.0;
        let mut mark_to = (0.0, 0.0);
        for (tab, label) in tabs {
            let w = frame.measure(label, font) + 24.0;
            let r = Rect { x, y: side.y + 6.0, w, h: TABS_H - 12.0 };
            let target = Target::Tab(tab);
            let on = self.tab == tab;
            if !on && *hot == Some(target.clone()) {
                fill(frame, p.hover, r.x, r.y, r.w, r.h, 6.0);
            }
            frame.text(label, font, if on { p.text } else { p.text2 }, r.x + 12.0, frame.line_in(font, r.y, r.h), w, Align::Start);
            if on {
                mark_to = (r.x + r.w / 2.0 - 8.0, 16.0);
            }
            self.spot(r, target);
            x += w + 2.0;
        }
        if self.tab_mark.1 == 0.0 {
            self.tab_mark = mark_to;
        }
        let k = 1.0 - (-dt * 1000.0 / 60.0).exp();
        self.tab_mark.0 += (mark_to.0 - self.tab_mark.0) * k;
        if (mark_to.0 - self.tab_mark.0).abs() > 0.3 {
            moving = true;
        } else {
            self.tab_mark.0 = mark_to.0;
        }
        fill(frame, p.accent, self.tab_mark.0, side.y + TABS_H - 5.0, 16.0, 3.0, 1.5);
        let Some(shown) = self.shown.clone().filter(|shown| shown.root.eq_ignore_ascii_case(&self.root)) else { return moving };
        let detail = &shown.detail;
        // The sum line.
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
        frame.text(&sum, sum_font, p.text3, side.x + 16.0, sum_y, side.w - 32.0, Align::Start);
        let addable: Vec<usize> = detail.rows.iter().enumerate().filter(|(_, row)| !self.gathered_at(&row.path)).map(|(at, _)| at).collect();
        if self.tab == Tab::Largest && (self.filter.on() || self.light.is_some()) && !addable.is_empty() {
            let label = lang.pick("全部加入", "Gather all");
            let w = frame.measure(label, sum_font) + 12.0;
            let r = Rect { x: side.x + side.w - 10.0 - w, y: side.y + TABS_H + 2.0, w, h: SUM_H - 4.0 };
            if *hot == Some(Target::AddAll) {
                fill(frame, p.hover, r.x, r.y, r.w, r.h, 4.0);
            }
            frame.text(label, sum_font, p.accent, r.x + 6.0, sum_y, w, Align::Start);
            self.spot(r, Target::AddAll);
        }
        let area = list_area(side);
        let count = match self.tab {
            Tab::Largest => detail.rows.len(),
            Tab::Types => detail.extensions.len(),
            Tab::Ages => AGES,
        };
        let most_scroll = (count as f32 * ROW_H - area.h).max(0.0);
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
        frame.clip(area.x, area.y, area.w, area.h);
        let first = (self.scroll / ROW_H).floor().max(0.0) as usize;
        let last = (((self.scroll + area.h) / ROW_H).ceil() as usize).min(count);
        let name_font = body(13.0, 400.0);
        let sub_font = body(12.0, 400.0);
        let biggest = match self.tab {
            Tab::Largest => detail.rows.first().map_or(1, |row| row.bytes),
            Tab::Types => detail.extensions.first().map_or(1, |extension| extension.bytes),
            Tab::Ages => detail.ages.iter().map(|age| age.1).max().unwrap_or(1),
        }
        .max(1);
        for at in first..last {
            let r = Rect { x: area.x, y: area.y + at as f32 * ROW_H - self.scroll, w: area.w, h: ROW_H };
            let (target, icon, hue, name, sub, bytes, on) = match self.tab {
                Tab::Largest => {
                    let row = &detail.rows[at];
                    let parent = row.path.parent().map(|parent| parent.to_string_lossy().into_owned()).unwrap_or_default();
                    (Target::Row(at), kind_icon(row.kind), p.hue[row.kind.at()], row.name.clone(), parent, row.bytes, self.chosen.as_ref() == Some(&row.path))
                }
                Tab::Types => {
                    let extension = &detail.extensions[at];
                    let name = if extension.extension.is_empty() { lang.pick("无扩展名", "No extension").to_string() } else { format!(".{}", extension.extension) };
                    let sub = match lang {
                        Lang::Zh => format!("{} · {} 个文件", kind_name(extension.kind, lang), count_text(extension.files)),
                        Lang::En => format!("{} · {} files", kind_name(extension.kind, lang), count_text(extension.files)),
                    };
                    (Target::TypeRow(extension.extension.clone()), kind_icon(extension.kind), p.hue[extension.kind.at()], name, sub, extension.bytes, self.light == Some(Light::Extension(extension.extension.clone())))
                }
                Tab::Ages => {
                    let (files, bytes) = detail.ages[at];
                    let sub = match lang {
                        Lang::Zh => format!("{} 个文件", count_text(files)),
                        Lang::En => format!("{} files", count_text(files)),
                    };
                    (Target::AgeRow(at), Icon::Clock, p.text2, age_name(at, lang).to_string(), sub, bytes, self.light == Some(Light::Age(at)))
                }
            };
            let hovered = self.row_hover.as_ref() == Some(&target) || *hot == Some(target.clone()) || matches!(hot, Some(Target::RowAdd(n) | Target::RowRelease(n)) if *n == at && self.tab == Tab::Largest);
            if on {
                fill(frame, p.chosen, r.x, r.y + 1.0, r.w, r.h - 2.0, 6.0);
            } else if hovered {
                fill(frame, p.hover, r.x, r.y + 1.0, r.w, r.h - 2.0, 6.0);
            }
            frame.icon(icon, Point { x: r.x + 18.0, y: r.y + r.h / 2.0 }, 16.0, hue);
            let size = size_text(bytes, lang);
            let size_w = frame.measure(&size, sub_font) + 4.0;
            let mut text_right = r.x + r.w - 10.0;
            let gathered_row = self.tab == Tab::Largest && self.gathered_at(&detail.rows[at.min(detail.rows.len().saturating_sub(1))].path);
            if hovered && self.tab == Tab::Largest && !gathered_row {
                // The row's tools in place of its size.
                let row = &detail.rows[at];
                let add = Rect { x: r.x + r.w - 8.0 - 24.0, y: r.y + 10.0, w: 24.0, h: 24.0 };
                let add_target = Target::RowAdd(at);
                if *hot == Some(add_target.clone()) {
                    fill(frame, p.control, add.x, add.y, add.w, add.h, 4.0);
                }
                frame.icon(Icon::Add, Point { x: add.x + 12.0, y: add.y + 12.0 }, 14.0, p.text);
                self.spot(add, add_target);
                text_right = add.x - 4.0;
                if self.in_onedrive(&row.path) {
                    let release = Rect { x: r.x + r.w - 36.0 - 24.0, y: r.y + 10.0, w: 24.0, h: 24.0 };
                    let release_target = Target::RowRelease(at);
                    if *hot == Some(release_target.clone()) {
                        fill(frame, p.control, release.x, release.y, release.w, release.h, 4.0);
                    }
                    frame.icon(Icon::Cloud, Point { x: release.x + 12.0, y: release.y + 12.0 }, 14.0, p.text);
                    self.spot(release, release_target);
                    text_right = release.x - 4.0;
                }
            } else {
                frame.text(&size, sub_font, p.text2, text_right - size_w, frame.line_in(sub_font, r.y, r.h), size_w, Align::End);
                text_right -= size_w + 8.0;
                if gathered_row {
                    frame.icon(Icon::Inbox, Point { x: text_right - 6.0, y: r.y + r.h / 2.0 }, 12.0, p.text3);
                    text_right -= 16.0;
                }
            }
            let text_x = r.x + 36.0;
            frame.text(&name, name_font, p.text, text_x, r.y + 5.0, text_right - text_x, Align::Start);
            frame.text(&sub, sub_font, p.text3, text_x, r.y + 23.0, text_right - text_x, Align::Start);
            // Its share of the largest.
            let share = (bytes as f64 / biggest as f64) as f32;
            fill(frame, hue.alpha(0.5), text_x, r.y + r.h - 4.0, (r.x + r.w - 10.0 - text_x) * share, 2.0, 1.0);
            self.spot(Rect { x: r.x, y: r.y.max(area.y), w: r.w, h: (r.y + r.h).min(area.y + area.h) - r.y.max(area.y) }, target);
        }
        frame.unclip();
        // The tools spotted before their row: put after it again, to be found first.
        let tools: Vec<(Rect, Target)> = self.targets.iter().filter(|(_, target)| matches!(target, Target::RowAdd(_) | Target::RowRelease(_))).cloned().collect();
        self.targets.retain(|(_, target)| !matches!(target, Target::RowAdd(_) | Target::RowRelease(_)));
        self.targets.extend(tools.into_iter().filter(|(r, _)| r.y >= area.y - 1.0 && r.y + r.h <= area.y + area.h + 1.0));
        moving
    }

    /// Whether `path` is in one of this PC's OneDrive folders.
    pub(super) fn in_onedrive(&self, path: &Path) -> bool {
        self.clouds.iter().any(|cloud| path.starts_with(cloud))
    }

    /// The place what is to go is gathered in. Whether it moves still.
    fn paint_foot(&mut self, frame: &Frame, p: &Palette, g: Geo, now: Instant, hot: &Option<Target>) -> bool {
        let lang = self.lang;
        let foot = g.foot;
        self.foot_at = foot;
        self.spot(foot, Target::Foot);
        let dragging_in = self.drag.as_ref().is_some_and(|drag| foot.contains(drag.at.0, drag.at.1));
        let title_font = body(13.0, 600.0);
        let sub_font = body(12.0, 400.0);
        if let Some(pending) = &self.pending {
            // About to go: its time to be undone draining.
            card(frame, p, foot);
            let left = (UNDO_FOR.as_secs_f32() - now.duration_since(pending.since).as_secs_f32()).max(0.0);
            let centre = Point { x: foot.x + 16.0 + 14.0, y: foot.y + foot.h / 2.0 };
            frame.arc(centre, 12.0, 0.0, TAU, p.meter, 2.5);
            frame.arc(centre, 12.0, 0.0, TAU * left / UNDO_FOR.as_secs_f32(), p.accent, 2.5);
            centred(frame, &format!("{}", left.ceil() as u32), body(11.0, 600.0), p.text, centre.x - 14.0, frame.line_in(body(11.0, 600.0), centre.y - 10.0, 20.0), 28.0);
            let bytes: u64 = pending.items.iter().map(|item| item.bytes).sum();
            let count = pending.items.len();
            frame.text(lang.pick("即将移至回收站", "Moving to the Recycle Bin"), title_font, p.text, foot.x + 56.0, foot.y + 9.0, foot.w - 200.0, Align::Start);
            let sub = match lang {
                Lang::Zh => format!("{} 项，共 {}", count, size_text(bytes, lang)),
                Lang::En => format!("{} items, {}", count, size_text(bytes, lang)),
            };
            frame.text(&sub, sub_font, p.text2, foot.x + 56.0, foot.y + 29.0, foot.w - 200.0, Align::Start);
            let label = lang.pick("撤销", "Undo");
            let w = frame.measure(label, body(13.0, 400.0)) + 22.0 + 22.0;
            let r = Rect { x: foot.x + foot.w - 12.0 - w, y: foot.y + 12.0, w, h: BUTTON_H };
            button(frame, p, r, false, *hot == Some(Target::Undo));
            frame.icon(Icon::Undo, Point { x: r.x + 11.0 + 7.0, y: r.y + 16.0 }, 14.0, p.text);
            frame.text(label, body(13.0, 400.0), p.text, r.x + 33.0, frame.line_in(body(13.0, 400.0), r.y, r.h), w, Align::Start);
            self.spot(r, Target::Undo);
            return true;
        }
        if self.gathered.is_empty() && !self.going.is_empty() {
            // Going now: the shell moves it.
            card(frame, p, foot);
            let bytes: u64 = self.going.iter().map(|item| item.bytes).sum();
            frame.icon(Icon::Trash, Point { x: foot.x + 30.0, y: foot.y + foot.h / 2.0 }, 18.0, p.text2);
            frame.text(lang.pick("正在移至回收站", "Moving to the Recycle Bin"), title_font, p.text, foot.x + 56.0, foot.y + 9.0, foot.w - 72.0, Align::Start);
            let sub = match lang {
                Lang::Zh => format!("{} 项，共 {}", self.going.len(), size_text(bytes, lang)),
                Lang::En => format!("{} items, {}", self.going.len(), size_text(bytes, lang)),
            };
            frame.text(&sub, sub_font, p.text2, foot.x + 56.0, foot.y + 29.0, foot.w - 72.0, Align::Start);
            return false;
        }
        if self.gathered.is_empty() {
            if let Some(went) = &self.went {
                card(frame, p, foot);
                frame.icon(Icon::CircleCheck, Point { x: foot.x + 30.0, y: foot.y + foot.h / 2.0 }, 20.0, p.accent);
                let title = match lang {
                    Lang::Zh => format!("已移至回收站：{} 项，{}", went.count, size_text(went.bytes, lang)),
                    Lang::En => format!("Moved to the Recycle Bin: {} items, {}", went.count, size_text(went.bytes, lang)),
                };
                frame.text(&title, title_font, p.text, foot.x + 56.0, foot.y + 9.0, foot.w - 220.0, Align::Start);
                frame.text(lang.pick("回收站仍在这个盘上，清空后才释放空间", "The Recycle Bin is on this drive: emptied, its space is freed"), sub_font, p.text2, foot.x + 56.0, foot.y + 29.0, foot.w - 220.0, Align::Start);
                let label = lang.pick("清空回收站", "Empty Recycle Bin");
                let w = frame.measure(label, body(13.0, 400.0)) + 22.0 + 22.0;
                let r = Rect { x: foot.x + foot.w - 12.0 - w, y: foot.y + 12.0, w, h: BUTTON_H };
                button(frame, p, r, false, *hot == Some(Target::EmptyNow));
                frame.icon(Icon::Trash, Point { x: r.x + 11.0 + 7.0, y: r.y + 16.0 }, 14.0, p.text);
                frame.text(label, body(13.0, 400.0), p.text, r.x + 33.0, frame.line_in(body(13.0, 400.0), r.y, r.h), w, Align::Start);
                self.spot(r, Target::EmptyNow);
                return false;
            }
            // Nothing gathered: where to put it.
            let ink = if dragging_in { p.accent } else { p.text3 };
            if dragging_in {
                fill(frame, p.accent.alpha(0.08), foot.x, foot.y, foot.w, foot.h, CARD_RADIUS);
            }
            frame.stroke_rounded_dashed(ink, foot.x, foot.y, foot.w, foot.h, CARD_RADIUS, 1.0);
            frame.icon(Icon::Inbox, Point { x: foot.x + 30.0, y: foot.y + foot.h / 2.0 }, 18.0, ink);
            frame.text(lang.pick("将文件或文件夹拖到这里，集中移至回收站", "Drag files or folders here to move them to the Recycle Bin together"), title_font, if dragging_in { p.accent } else { p.text2 }, foot.x + 56.0, foot.y + 9.0, foot.w - 72.0, Align::Start);
            frame.text(
                lang.pick("也可以点方块或列表右侧的加号。删除以当前用户的权限执行，没有权限的项目会保留并列出。", "Or press the plus on a block or a row. They are deleted as you may: what you may not delete stays, and is listed."),
                sub_font,
                p.text3,
                foot.x + 56.0,
                foot.y + 29.0,
                foot.w - 72.0,
                Align::Start,
            );
            return false;
        }
        // What is gathered: as many as fit, the rest in a menu; how much it frees, and its going.
        card(frame, p, foot);
        if dragging_in {
            stroke_inside(frame, foot, CARD_RADIUS, p.accent);
        }
        let bytes: u64 = self.gathered.iter().map(|item| item.bytes).sum();
        let button_font = body(13.0, 400.0);
        let commit_label = lang.pick("移至回收站", "Move to Recycle Bin");
        let clear_label = lang.pick("全部移出", "Remove all");
        let commit_w = frame.measure(commit_label, button_font) + 22.0 + 22.0;
        let clear_w = frame.measure(clear_label, button_font) + 22.0;
        let commit = Rect { x: foot.x + foot.w - 12.0 - commit_w, y: foot.y + 12.0, w: commit_w, h: BUTTON_H };
        let clear = Rect { x: commit.x - 8.0 - clear_w, y: foot.y + 12.0, w: clear_w, h: BUTTON_H };
        button(frame, p, commit, true, *hot == Some(Target::Commit));
        frame.icon(Icon::Trash, Point { x: commit.x + 11.0 + 7.0, y: commit.y + 16.0 }, 14.0, p.on_accent);
        frame.text(commit_label, button_font, p.on_accent, commit.x + 33.0, frame.line_in(button_font, commit.y, commit.h), commit_w, Align::Start);
        self.spot(commit, Target::Commit);
        button(frame, p, clear, false, *hot == Some(Target::UngatherAll));
        frame.text(clear_label, button_font, p.text, clear.x + 11.0, frame.line_in(button_font, clear.y, clear.h), clear_w, Align::Start);
        self.spot(clear, Target::UngatherAll);
        let frees = format!("{} {}", lang.pick("将释放", "Frees"), size_text(bytes, lang));
        let later = lang.pick("清空回收站后生效", "once the Recycle Bin is emptied");
        let summary_w = frame.measure(&frees, title_font).max(frame.measure(later, sub_font)) + 4.0;
        let summary_x = clear.x - 16.0 - summary_w;
        frame.text(&frees, title_font, p.text, summary_x, foot.y + 9.0, summary_w, Align::End);
        frame.text(later, sub_font, p.text3, summary_x, foot.y + 29.0, summary_w, Align::End);
        // The chips.
        let chip_font = body(12.0, 400.0);
        let room_right = summary_x - 12.0;
        let mut x = foot.x + 12.0;
        let items = self.gathered.clone();
        let mut hidden: Vec<usize> = Vec::new();
        for (at, item) in items.iter().enumerate() {
            let name_w = frame.measure(&item.name, chip_font).min(140.0);
            let size = size_text(item.bytes, lang);
            let size_w = frame.measure(&size, chip_font);
            let w = 10.0 + 14.0 + 6.0 + name_w + 6.0 + size_w + 4.0 + 20.0 + 4.0;
            let more_room = if at + 1 < items.len() { 90.0 } else { 0.0 };
            if !hidden.is_empty() || x + w + more_room > room_right {
                hidden.push(at);
                continue;
            }
            let r = Rect { x, y: foot.y + 12.0, w, h: 32.0 };
            fill(frame, p.control, r.x, r.y, r.w, r.h, 6.0);
            frame.icon(if item.folder { Icon::Folder } else { kind_icon(item.kind) }, Point { x: r.x + 10.0 + 7.0, y: r.y + 16.0 }, 14.0, if item.folder { p.text2 } else { p.hue[item.kind.at()] });
            frame.text(&item.name, chip_font, p.text, r.x + 30.0, frame.line_in(chip_font, r.y, r.h), name_w + 2.0, Align::Start);
            frame.text(&size, chip_font, p.text3, r.x + 30.0 + name_w + 6.0, frame.line_in(chip_font, r.y, r.h), size_w + 2.0, Align::Start);
            let close = Rect { x: r.x + r.w - 24.0, y: r.y + 6.0, w: 20.0, h: 20.0 };
            if *hot == Some(Target::Ungather(at)) {
                fill(frame, p.hover, close.x, close.y, close.w, close.h, 4.0);
            }
            frame.icon(Icon::Close, Point { x: close.x + 10.0, y: close.y + 10.0 }, 12.0, p.text2);
            self.spot(close, Target::Ungather(at));
            x += w + 6.0;
        }
        if !hidden.is_empty() {
            let label = match lang {
                Lang::Zh => format!("另外 {} 项", hidden.len()),
                Lang::En => format!("{} more", hidden.len()),
            };
            let w = frame.measure(&label, chip_font) + 20.0 + 16.0;
            let r = Rect { x, y: foot.y + 12.0, w, h: 32.0 };
            let target = Target::GatheredMore(hidden);
            fill(frame, if *hot == Some(target.clone()) { p.chosen } else { p.control }, r.x, r.y, r.w, r.h, 6.0);
            frame.text(&label, chip_font, p.text, r.x + 10.0, frame.line_in(chip_font, r.y, r.h), w, Align::Start);
            frame.icon(Icon::ChevronDown, Point { x: r.x + r.w - 14.0, y: r.y + 16.0 }, 12.0, p.text2);
            self.spot(r, target);
        }
        false
    }

    /// What is over the window: a dragged thing, a block's tag, a card, a menu, a word said.
    fn paint_over(&mut self, frame: &Frame, p: &Palette, g: Geo, now: Instant, hot: &Option<Target>) {
        let lang = self.lang;
        if let Some(drag) = &self.drag {
            let about = match drag.what {
                Draggable::Block(at) => self.laid.get(at).map(|placed| self.about(&placed.shown, &placed.block.item)),
                Draggable::Row(at) => self.shown.as_ref().and_then(|shown| shown.detail.rows.get(at)).map(|row| About {
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
                    onedrive: false,
                    many_in: None,
                    why_not: None,
                }),
            };
            if let Some(about) = about {
                let font = body(12.0, 600.0);
                let text = format!("{}  {}", about.name, size_text(about.bytes, lang));
                let w = (frame.measure(&text, font) + 40.0).min(300.0);
                let (x, y) = (drag.at.0 + 12.0, drag.at.1 + 8.0);
                let tall = if drag.why_not.is_some() { 50.0 } else { 30.0 };
                frame.shadow(Color::hex(0, 0.18), x, y, w, tall, 6.0, 6.0, 2.0);
                fill(frame, p.fly, x, y, w, tall, 6.0);
                stroke_inside(frame, Rect { x, y, w, h: tall }, 6.0, p.fly_stroke);
                frame.icon(if about.folder { Icon::Folder } else { kind_icon(about.kind) }, Point { x: x + 16.0, y: y + 15.0 }, 14.0, p.hue[about.kind.at()]);
                frame.text(&text, font, p.text, x + 30.0, frame.line_in(font, y, 30.0), w - 38.0, Align::Start);
                if let Some(why) = &drag.why_not {
                    frame.text(why, body(12.0, 400.0), p.signal, x + 10.0, y + 28.0, w - 18.0, Align::Start);
                }
            }
        } else if self.plate.is_none() && self.menu.is_none() && self.press.is_none() && self.anim.is_none() {
            if let Some(at) = self.hover.filter(|_| matches!(hot, Some(Target::Map))) {
                self.paint_tag(frame, p, g, at);
            }
            if let Some(Target::Drive(at)) = hot {
                if let Some(drive) = self.drives.get(*at).filter(|drive| drive.low()) {
                    let (title, sub) = match lang {
                        Lang::Zh => (format!("{}: 可用空间不足", drive.letter()), format!("仅余 {}，约为总容量的 {:.0}%", size_text(drive.free, lang), drive.free as f64 * 100.0 / drive.total.max(1) as f64)),
                        Lang::En => (format!("{}: is running out of space", drive.letter()), format!("Only {} free, about {:.0}% of it", size_text(drive.free, lang), drive.free as f64 * 100.0 / drive.total.max(1) as f64)),
                    };
                    let under = self.target_rect(&Target::Drive(*at));
                    let r = Rect { x: under.x, y: under.y + under.h, w: 240.0, h: 52.0 };
                    flyout(frame, p, r);
                    frame.text(&title, body(13.0, 600.0), p.text, r.x + 12.0, r.y + 8.0, r.w - 24.0, Align::Start);
                    frame.text(&sub, body(12.0, 400.0), p.text2, r.x + 12.0, r.y + 28.0, r.w - 24.0, Align::Start);
                }
            }
        }
        self.paint_plate(frame, p, g, hot);
        self.paint_menu(frame, p, g, hot);
        if let Some(said) = &self.said {
            let font = body(13.0, 400.0);
            let w = frame.measure(&said.words, font) + 16.0 + 16.0 + 24.0;
            let r = Rect { x: PAGE_X + (g.page_w - w) / 2.0, y: g.height - 96.0 - 40.0, w, h: 40.0 };
            let t = now.duration_since(said.since).as_secs_f32();
            let alpha = (t / 0.15).min(1.0).min((SAID_FOR.as_secs_f32() - t) / 0.25).clamp(0.0, 1.0);
            frame.shadow(Color::hex(0, 0.16 * alpha), r.x, r.y, r.w, r.h, 8.0, 8.0, 3.0);
            fill(frame, p.fly.alpha(alpha), r.x, r.y, r.w, r.h, 8.0);
            stroke_inside(frame, r, 8.0, p.fly_stroke.alpha(p.fly_stroke.a * alpha));
            frame.icon(said.icon, Point { x: r.x + 16.0 + 8.0, y: r.y + 20.0 }, 16.0, p.accent.alpha(alpha));
            frame.text(&said.words, font, p.text.alpha(p.text.a * alpha), r.x + 40.0, frame.line_in(font, r.y, r.h), r.w - 48.0, Align::Start);
        }
    }

    /// The tag of the block at `at` (its place in the layout): what it is, and what can be done with it.
    fn paint_tag(&mut self, frame: &Frame, p: &Palette, g: Geo, at: usize) {
        let lang = self.lang;
        let Some(placed) = self.laid.get(at).cloned() else { return };
        let Some((x, y)) = self.pointer else { return };
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
        let mut r = Rect { x: x + 16.0, y: y + 20.0, w: TAG_W, h: tall };
        if r.x + r.w > g.width - 8.0 {
            r.x = x - 16.0 - r.w;
        }
        if r.y + r.h > g.height - 8.0 {
            r.y = (y - 12.0 - r.h).max(8.0);
        }
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

    /// The card over the window, if there is one.
    fn paint_plate(&mut self, frame: &Frame, p: &Palette, g: Geo, hot: &Option<Target>) {
        let lang = self.lang;
        let Some(plate) = &self.plate else { return };
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
        self.spot(r, Target::Over);
        let mut y = r.y + 16.0;
        frame.text(&title, body(16.0, 600.0), p.text, r.x + 16.0, y, inner - 28.0, Align::Start);
        let close = Rect { x: r.x + r.w - 12.0 - 24.0, y: r.y + 12.0, w: 24.0, h: 24.0 };
        let close_target = Target::PlateAct(PlateAct::Close);
        if *hot == Some(close_target.clone()) {
            fill(frame, p.hover, close.x, close.y, close.w, close.h, 4.0);
        }
        frame.icon(Icon::Close, Point { x: close.x + 12.0, y: close.y + 12.0 }, 14.0, p.text2);
        self.spot(close, close_target);
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
            self.spot(lr, target);
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
                button(frame, p, br, primary, *hot == Some(target.clone()));
                centred(frame, &label, body_font, if primary { p.on_accent } else { p.text }, br.x, frame.line_in(body_font, br.y, br.h), br.w);
                self.spot(br, target);
                x -= 8.0;
            }
        }
    }

    /// The menu over the window, if there is one.
    fn paint_menu(&mut self, frame: &Frame, p: &Palette, g: Geo, hot: &Option<Target>) {
        let lang = self.lang;
        let Some(menu) = &self.menu else { return };
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
        self.spot(r, Target::Over);
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
            self.spot(ir, target);
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

/// Windows 11's button: standard, or the accent's (`on`).
fn button(frame: &Frame, p: &Palette, r: Rect, on: bool, hovered: bool) {
    if on {
        fill(frame, if hovered { p.accent.alpha(0.9) } else { p.accent }, r.x, r.y, r.w, r.h, 6.0);
        return;
    }
    fill(frame, if hovered { p.button_hover } else { p.button }, r.x, r.y, r.w, r.h, 6.0);
    stroke_inside(frame, r, 6.0, p.button_stroke);
    within(frame, r, 6.0, 1.0, || fill(frame, p.button_edge, r.x, r.y + r.h - 1.0, r.w, 1.0, 0.0));
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
