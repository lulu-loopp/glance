//! Nearby sharing's page's sending pane (beside its rows, where the other pages show a preview): what is chosen to
//! send (files and folders from Windows' choosers or dropped there, words written in it, what is on the clipboard),
//! and the devices it can go to (the phones paired first, then the Glance PCs and other LocalSend devices found),
//! each pressed to send all that is chosen to it, and showing how that goes. What is chosen stays after a sending, to
//! go to another device too. See `docs/nearby.md` ("Sending from the PC").
//!
//! While it shows, this PC is found by the devices nearby, receiving or not (see `nearby::want_to_send`): one that
//! sends to it then while it does not receive is turned away.

use super::*;
use crate::nearby::Sending;
use crate::station::clipboard::Content;
use windows::Win32::Graphics::DirectWrite::IDWriteTextLayout;

/// The most of what is chosen listed by itself (the rest counted under it).
const LISTED: usize = 4;
/// How long a sending over is shown (sent, or not) before its device is shown as it was.
const SHOWN_OVER: Duration = Duration::from_secs(4);
/// How long something said of a press stays (a phone not receiving, nothing on the clipboard).
const SAID_FOR: Duration = Duration::from_secs(5);
/// The most written to send (UTF-16 units).
pub(super) const WORDS_MOST: usize = 100_000;
/// Measures (DIPs): the pane's inset, the choosers' tiles, an item chosen, a device, the box words are written in.
const INSET: f32 = 16.0;
const KIND: f32 = 62.0;
const ITEM: f32 = 34.0;
const DEVICE: f32 = 54.0;
const WRITER: f32 = 96.0;
/// How tall the pane's foot is (what this PC is called to devices nearby: it stays as the rest scrolls).
const FOOT: f32 = 52.0;
/// The words written to send are written in.
const WORDS_FONT: Font = Font::new(Family::Segoe, 13.0, 400.0);
/// How often the window looks at what nearby transfer shows while the pane shows or a sending of its goes on (it is
/// drawn again only where that changed).
pub(super) const WATCH_EVERY: Duration = Duration::from_millis(250);

/// What a thing chosen is, as the pane shows it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Kind {
    File,
    Folder,
    Words,
    Picture,
}

/// A thing chosen to send: what it is, its name as listed, and how large (none yet: a folder being measured).
#[derive(Clone, Debug)]
pub(super) struct Chosen {
    id: u64,
    pub content: Content,
    name: String,
    kind: Kind,
    size: Option<u64>,
}

/// A sending begun from the pane: to which device (its fingerprint), its number, and when it was over (as shown).
struct Job {
    to: String,
    job: u64,
    over_at: Option<Instant>,
}

/// The pane: what is chosen (in order), whether something dragged is over it, the sendings begun from it, what was
/// said of the last press and when, the choosers up (on threads of their own), the folders being measured (their sizes
/// come on `sized`), and where it was last drawn (the window's DIPs).
pub(super) struct Pane {
    pub chosen: Vec<Chosen>,
    pub over: bool,
    jobs: Vec<Job>,
    said: Option<(&'static str, Instant)>,
    pub choosing: Option<std::thread::JoinHandle<Vec<std::path::PathBuf>>>,
    /// The words being written as the pane was left (written on as it is opened again), and what nearby transfer showed
    /// as the pane was last looked at.
    pub draft: String,
    seen: Option<crate::nearby::Shown>,
    next: u64,
    sizes: Sizes,
    pub area: Option<Rect>,
    /// How far the pane's content is scrolled down, and the most it can be (DIPs: as last drawn).
    pub scroll: f32,
    pub most: f32,
}

impl Default for Pane {
    fn default() -> Self {
        Pane { chosen: Vec::new(), over: false, jobs: Vec::new(), said: None, choosing: None, draft: String::new(), seen: None, next: 1, sizes: std::sync::mpsc::channel(), area: None, scroll: 0.0, most: 0.0 }
    }
}

/// Where the folders chosen are told measured (a folder's number, how large), and where that is heard.
type Sizes = (std::sync::mpsc::Sender<(u64, u64)>, std::sync::mpsc::Receiver<(u64, u64)>);

/// A device the pane can send to: its fingerprint, what it is called, its icon, what is said of it (its tags), whether
/// it is a phone paired, and whether it is found nearby now (a phone paired not found is not receiving).
pub(super) struct SendDevice {
    fingerprint: String,
    name: String,
    icon: Icon,
    tags: Vec<String>,
    found: bool,
}

/// What dragged into the pane was let go there (see `dragged_in`), till the window takes it.
static DROPPED: std::sync::Mutex<Vec<Content>> = std::sync::Mutex::new(Vec::new());

/// What a window catching drops over the pane of `window` saw (on the station's thread, see `station::catch`): told to
/// the window by message, what was let go kept for it.
pub fn dragged_in(window: HWND, dragged: crate::station::drop::Dragged) {
    use crate::station::drop::Dragged;
    let told = match dragged {
        Dragged::Over(_) => 1,
        Dragged::Gone => 0,
        Dragged::Dropped(given) => {
            DROPPED.lock().unwrap().extend(given);
            2
        }
    };
    let _ = unsafe { PostMessageW(Some(window), SEND_DRAGGED, WPARAM(told), LPARAM(0)) };
}

/// Posted to the settings window as something dragged comes over its sending pane (WPARAM 1), goes (0), or is let go
/// there (2: what it gave kept in `DROPPED`).
pub(super) const SEND_DRAGGED: u32 = WM_APP + 49;
/// Posted to the settings window open, to turn to nearby transfer's page with what was given to send (see
/// `open_to_send`).
pub(super) const TO_SEND_PAGE: u32 = WM_APP + 50;

impl Pane {
    /// `content` chosen, as what it is called `name`, of kind `kind`, `size` large (none: measured on a thread).
    fn add(&mut self, content: Content, name: String, kind: Kind, size: Option<u64>) {
        let id = self.next;
        self.next += 1;
        self.chosen.push(Chosen { id, content, name, kind, size });
    }

    /// The files and folders at `paths` chosen (a folder measured on a thread of its own: it may hold many files).
    pub fn add_paths(&mut self, paths: Vec<std::path::PathBuf>) {
        for path in paths {
            if self.chosen.iter().any(|chosen| chosen.content == Content::Files(vec![path.clone()])) {
                continue;
            }
            let name = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
            let Ok(meta) = std::fs::metadata(&path) else { continue };
            if meta.is_dir() {
                let (id, sizes, folder) = (self.next, self.sizes.0.clone(), path.clone());
                std::thread::spawn(move || {
                    let _ = sizes.send((id, folder_size(&folder)));
                });
                self.add(Content::Files(vec![path]), name, Kind::Folder, None);
            } else {
                self.add(Content::Files(vec![path]), name, Kind::File, Some(meta.len()));
            }
        }
    }

    /// `given` chosen (dropped on the pane, or read off the clipboard): files and folders each by itself, words, a
    /// picture, a file with no path of its own.
    pub fn add_given(&mut self, given: Vec<Content>, lang: Lang) {
        for content in given {
            match content {
                Content::Files(paths) => self.add_paths(paths),
                Content::Text(words) => self.add_words(words),
                Content::Png(png) => {
                    let size = png.len() as u64;
                    self.add(Content::Png(png), pick(lang, "图片.png", "Picture.png").to_string(), Kind::Picture, Some(size));
                }
                Content::Saved { name, bytes } => {
                    let size = bytes.len() as u64;
                    self.add(Content::Saved { name: name.clone(), bytes }, name, Kind::File, Some(size));
                }
            }
        }
    }

    /// `words` chosen (named by their start), unless they are nothing.
    pub fn add_words(&mut self, words: String) {
        if words.trim().is_empty() {
            return;
        }
        let name = words.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(60).collect();
        let size = words.len() as u64;
        self.add(Content::Text(words), name, Kind::Words, Some(size));
    }

    /// The sizes of the folders measured since, taken.
    fn take_sizes(&mut self) -> bool {
        let mut took = false;
        while let Ok((id, size)) = self.sizes.1.try_recv() {
            if let Some(chosen) = self.chosen.iter_mut().find(|chosen| chosen.id == id) {
                chosen.size = Some(size);
                took = true;
            }
        }
        took
    }

    /// Something said of a press, for a while.
    fn say(&mut self, said: &'static str) {
        self.said = Some((said, Instant::now()));
    }

    /// The newest sending begun to `fingerprint`, and how it goes (none yet: being put together).
    fn job_to(&self, fingerprint: &str, shown: &crate::nearby::Shown) -> Option<(u64, Option<Sending>)> {
        self.jobs.iter().rev().find(|job| job.to.eq_ignore_ascii_case(fingerprint)).map(|job| (job.job, shown.sendings.get(&job.job).cloned()))
    }

    /// Whether a folder chosen is still being measured.
    fn measuring(&self) -> bool {
        self.chosen.iter().any(|chosen| chosen.size.is_none())
    }

    /// Whether a sending begun from it is not over yet.
    pub fn sending(&self) -> bool {
        let shown = crate::nearby::shown();
        self.jobs.iter().any(|job| shown.sendings.get(&job.job).is_none_or(|state| !state.over()))
    }
}

/// How large all the files in `folder` are, those within the folders in it too (what only points elsewhere left out,
/// as a sending leaves it out: see `station::send::outgoing`).
fn folder_size(folder: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(folder) else { return 0 };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => folder_size(&entry.path()),
            Ok(kind) if kind.is_file() => entry.metadata().map_or(0, |meta| meta.len()),
            _ => 0,
        })
        .sum()
}

impl Ui {
    /// The pane's devices: the phones paired (by the certificates they told), as found nearby or not; then the devices
    /// found that are no paired phone, Glance's first.
    fn send_devices(&self, shown: &crate::nearby::Shown) -> Vec<SendDevice> {
        let lang = self.lang;
        let trusted = self.nearby.iter().flat_map(|nearby| nearby.trusted.iter());
        let found_of = |fingerprint: &str| shown.devices.iter().find(|device| device.fingerprint.eq_ignore_ascii_case(fingerprint));
        let model = |device: Option<&crate::nearby::Device>| device.and_then(|device| device.model.clone()).filter(|model| !model.trim().is_empty());
        let mut devices: Vec<SendDevice> = Vec::new();
        for phone in self.phones.iter().filter(|phone| !phone.web) {
            let Some(fingerprint) = trusted.clone().find(|trusted| trusted.phone.as_deref() == Some(phone.id.as_str())).map(|trusted| trusted.fingerprint.clone()) else { continue };
            let found = found_of(&fingerprint);
            let name = found.map(|device| device.alias.clone()).filter(|alias| !alias.is_empty()).unwrap_or_else(|| phone.name.clone());
            let tags = std::iter::once(pick(lang, "已配对", "Paired").to_string()).chain(model(found)).collect();
            devices.push(SendDevice { fingerprint, name, icon: Icon::Smartphone, tags, found: found.is_some() });
        }
        let mut others: Vec<&crate::nearby::Device> = shown.devices.iter().filter(|device| !devices.iter().any(|known| known.fingerprint.eq_ignore_ascii_case(&device.fingerprint))).collect();
        others.sort_by_key(|device| (!device.glance, device.alias.to_lowercase()));
        for device in others {
            let name = if device.alias.is_empty() { pick(lang, "未命名的设备", "Unnamed device").to_string() } else { device.alias.clone() };
            let maker = if device.glance { "Glance" } else { "LocalSend" };
            let tags = std::iter::once(maker.to_string()).chain(model(Some(device))).collect();
            devices.push(SendDevice { fingerprint: device.fingerprint.clone(), name, icon: kind_icon(device.kind.as_deref()), tags, found: true });
        }
        devices
    }

    /// The pane taken up as the window ticks: the folders measured, what was let go on it, the choosers closed, the
    /// sendings over a while forgotten, what was said gone after a while, and this PC kept found by the devices nearby
    /// while the pane shows (or a sending of its goes on). Whether it changed.
    pub(super) fn send_tick(&mut self, now: Instant) -> bool {
        let mut changed = self.send.take_sizes();
        let dropped = std::mem::take(&mut *DROPPED.lock().unwrap());
        if !dropped.is_empty() {
            self.send.add_given(dropped, self.lang);
            changed = true;
        }
        if self.send.choosing.as_ref().is_some_and(|choosing| choosing.is_finished()) {
            let chosen = self.send.choosing.take().unwrap().join().unwrap_or_default();
            self.send.add_paths(chosen);
            changed = true;
        }
        let shown = crate::nearby::shown();
        for job in &mut self.send.jobs {
            if job.over_at.is_none() && shown.sendings.get(&job.job).is_some_and(Sending::over) {
                job.over_at = Some(now);
                changed = true;
            }
        }
        let (gone, kept): (Vec<Job>, Vec<Job>) = std::mem::take(&mut self.send.jobs).into_iter().partition(|job| job.over_at.is_some_and(|at| now.duration_since(at) >= SHOWN_OVER));
        self.send.jobs = kept;
        for job in &gone {
            crate::nearby::forget(job.job);
        }
        changed |= !gone.is_empty();
        if self.send.said.is_some_and(|(_, at)| now.duration_since(at) >= SAID_FOR) {
            self.send.said = None;
            changed = true;
        }
        if self.page == Page::Nearby && self.send.seen.as_ref() != Some(&shown) {
            self.send.seen = Some(shown);
            changed = true;
        }
        if self.page == Page::Nearby || self.send.sending() {
            crate::nearby::want_to_send();
        }
        changed
    }

    /// Whether the window looks at the pane again soon (see `WATCH_EVERY`): it shows, a chooser of its is up, a folder
    /// chosen is measured, or a sending of its goes on.
    pub(super) fn send_watching(&self) -> bool {
        self.page == Page::Nearby || self.send.choosing.is_some() || self.send.measuring() || !self.send.jobs.is_empty()
    }

    /// Whether words to send are being written (see `paint_writer`).
    pub(super) fn writing_words(&self) -> bool {
        self.renaming.as_ref().is_some_and(|renaming| renaming.of == Named::Words)
    }

    /// The words being written put by as the pane is left (its page turned from), written on as it is opened again.
    pub(super) fn put_words_by(&mut self) {
        if let Some(renaming) = self.renaming.as_ref().filter(|renaming| renaming.of == Named::Words) {
            self.send.draft = renaming.typing.string();
            self.end_rename(false);
        }
    }

    /// The words being written laid out as they were last drawn: in lines as wide as their box, the box, and how far
    /// they are scrolled down in it (none: not drawn yet).
    fn words_layout(&self) -> Option<(IDWriteTextLayout, Rect, f32)> {
        let renaming = self.renaming.as_ref().filter(|renaming| renaming.of == Named::Words)?;
        let field = renaming.field?;
        Some((self.gfx.wrapped(&renaming.typing.string(), WORDS_FONT, field.w), field, renaming.along))
    }

    /// Where in the words being written a caret put at `(x, y)` (the window's DIPs) goes.
    pub(super) fn words_caret_at(&self, x: f32, y: f32) -> usize {
        let Some((layout, field, along)) = self.words_layout() else { return 0 };
        crate::ui::typing::at_point(&layout, x - field.x, y - field.y + along)
    }

    /// A key down while words to send are written: Ctrl+Enter adds them, Escape leaves them; Enter begins a new line;
    /// the arrows, Home and End move the caret (up and down a line, to its start and end: with Shift, the selection);
    /// Backspace and Delete erase; Ctrl+A chooses all, Ctrl+C and Ctrl+X copy and cut, Ctrl+V pastes (all of it). Every
    /// key is taken (Tab too: it does nothing there), the characters coming as `WM_CHAR` (see `rename_char`).
    pub(super) fn words_key(&mut self, key: VIRTUAL_KEY, shift: bool) -> bool {
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        match key {
            VK_RETURN if ctrl => {
                self.end_rename(true);
                return true;
            }
            VK_ESCAPE => {
                self.end_rename(false);
                return true;
            }
            _ => {}
        }
        let hwnd = self.hwnd;
        let pasted = (ctrl && key == VK_V).then(|| crate::clipboard::text(hwnd).unwrap_or_default());
        let layout = if matches!(key, VK_UP | VK_DOWN | VK_HOME | VK_END) { self.words_layout() } else { None };
        let Some(renaming) = self.renaming.as_mut() else { return false };
        let typing = &mut renaming.typing;
        match key {
            VK_RETURN => typing.put(&[u16::from(b'\n')]),
            VK_LEFT | VK_RIGHT => {
                let forward = key == VK_RIGHT;
                let selection = typing.selection();
                let to = if !shift && !selection.is_empty() { if forward { selection.end } else { selection.start } } else { typing.step(typing.caret, forward) };
                typing.go(to, shift);
            }
            // (Up from the first line, to the start; down from the last, to the end.)
            VK_UP | VK_DOWN => {
                if let Some((layout, ..)) = &layout {
                    let (x, top, tall) = words_caret(layout, &typing.text, typing.caret);
                    let to = if key == VK_UP && top < tall / 2.0 {
                        0
                    } else if key == VK_DOWN && top + tall * 1.5 > crate::ui::typing::height_of(layout) {
                        typing.text.len()
                    } else {
                        crate::ui::typing::at_point(layout, x, if key == VK_UP { top - tall / 2.0 } else { top + tall * 1.5 })
                    };
                    typing.go(to, shift);
                }
            }
            VK_HOME | VK_END => {
                if let Some((layout, ..)) = &layout {
                    let (start, end) = crate::ui::typing::line_of(layout, typing.caret);
                    typing.go(if key == VK_HOME { start } else { end }, shift);
                }
            }
            VK_BACK | VK_DELETE => typing.erase(key == VK_DELETE),
            VK_A if ctrl => {
                typing.go(0, false);
                typing.go(typing.text.len(), true);
            }
            VK_C | VK_X if ctrl => {
                let selection = typing.selection();
                if !selection.is_empty() {
                    let text = String::from_utf16_lossy(&typing.text[selection]);
                    if crate::clipboard::put(hwnd, &[(crate::clipboard::TEXT, crate::clipboard::utf16_z(&text))]) && key == VK_X {
                        typing.put(&[]);
                    }
                }
            }
            VK_V if ctrl => {
                let units: Vec<u16> = pasted.unwrap_or_default().replace("\r\n", "\n").replace('\r', "\n").encode_utf16().collect();
                typing.put(&units);
            }
            _ => {}
        }
        renaming.blink_from = Instant::now();
        self.next_frame = Instant::now();
        true
    }

    /// The pane in `area` (the window's DIPs): what to choose by, what is chosen (or the words being written) and the
    /// devices, scrolled as far as it is where they are taller than it; and at its foot, what this PC is called to them.
    /// The drop's frame over it while something dragged is.
    pub(super) fn paint_send(&mut self, frame: &Frame, palette: &Palette, area: Rect, now: Instant) {
        let lang = self.lang;
        let hovered = self.hovered_now.clone();
        self.send.area = Some(area);
        fill(frame, palette.card, area.x, area.y, area.w, area.h, 8.0);
        stroke_inside(frame, area, 8.0, palette.card_stroke);
        let view = Rect { x: area.x, y: area.y, w: area.w, h: (area.h - FOOT).max(0.0) };
        frame.clip(view.x, view.y, view.w, view.h);
        let (left, right) = (area.x + INSET, area.x + area.w - INSET);
        let width = right - left;
        let small = Font::new(Family::Segoe, 12.0, 600.0);
        let label = Font::new(Family::Segoe, 13.0, 400.0);
        let hint = Font::new(Family::Segoe, HINT, 400.0);
        let targets_from = self.targets.len();
        let mut y = area.y + INSET - self.send.scroll;
        // What to choose by.
        text_centred(frame, pick(lang, "选择", "Choose"), small, palette.text2, left, y + 8.0, width, Align::Start);
        y += 22.0;
        let kinds = [
            (Icon::File, pick(lang, "文件", "Files"), 0u8),
            (Icon::Folder, pick(lang, "文件夹", "Folder"), 1),
            (Icon::Text, pick(lang, "文字", "Words"), 2),
            (Icon::Clipboard, pick(lang, "剪贴板", "Clipboard"), 3),
        ];
        let tile = (width - 3.0 * 8.0) / 4.0;
        for (at, (icon, name, kind)) in kinds.into_iter().enumerate() {
            let rect = Rect { x: left + at as f32 * (tile + 8.0), y, w: tile, h: KIND };
            let target = Target::SendKind(kind);
            let lit = hovered.as_ref() == Some(&target);
            fill(frame, if lit { palette.hover } else { palette.control }, rect.x, rect.y, rect.w, rect.h, 8.0);
            stroke_inside(frame, rect, 8.0, palette.rule);
            frame.icon(icon, Point { x: rect.x + rect.w / 2.0, y: rect.y + 22.0 }, 20.0, palette.selection);
            across(frame, name, label, palette.text, rect.x, rect.y + 46.0, rect.w);
            self.targets.push((rect, target));
        }
        y += KIND + 12.0;
        // The words being written, in their box, with what to do with them under it.
        if self.renaming.as_ref().is_some_and(|renaming| renaming.of == Named::Words) {
            y = self.paint_writer(frame, palette, left, y, width, now) + 12.0;
        }
        // What is chosen: how many and how large, each (as many as are listed) with what takes it away.
        if !self.send.chosen.is_empty() {
            let listed = self.send.chosen.len().min(LISTED);
            let more = self.send.chosen.len() - listed;
            let tall = 40.0 + listed as f32 * ITEM + if more > 0 { 22.0 } else { 0.0 } + 6.0;
            let boxed = Rect { x: left, y, w: width, h: tall };
            stroke_inside(frame, boxed, 8.0, palette.rule);
            let bytes: u64 = self.send.chosen.iter().filter_map(|chosen| chosen.size).sum();
            let count = self.send.chosen.len();
            let summed = match lang {
                Lang::Zh => format!("已选择 {count} 项 · {}", crate::ui::text::bytes(bytes as f64)),
                Lang::En => format!("{count} chosen · {}", crate::ui::text::bytes(bytes as f64)),
            };
            text_centred(frame, &summed, Font::new(Family::Segoe, 13.0, 600.0), palette.text, left + 12.0, y + 20.0, width - 100.0, Align::Start);
            self.button(frame, palette, pick(lang, "清除", "Clear"), right - 6.0, y + 20.0, Target::SendClear, Button::Plain, &hovered);
            let mut row = y + 40.0;
            for chosen in self.send.chosen.iter().take(LISTED).cloned().collect::<Vec<_>>() {
                let cy = row + ITEM / 2.0;
                fill(frame, palette.control, left + 12.0, cy - 14.0, 28.0, 28.0, 6.0);
                let icon = match chosen.kind {
                    Kind::File => Icon::File,
                    Kind::Folder => Icon::Folder,
                    Kind::Words => Icon::Text,
                    Kind::Picture => Icon::Image,
                };
                frame.icon(icon, Point { x: left + 26.0, y: cy }, 16.0, palette.text2);
                let size = chosen.size.map_or_else(|| pick(lang, "正在计算", "Measuring").to_string(), |size| crate::ui::text::bytes(size as f64));
                let size_w = frame.gfx.measure(&size, hint);
                let remove = Rect { x: right - 12.0 - 28.0, y: cy - 14.0, w: 28.0, h: 28.0 };
                let target = Target::SendRemove(chosen.id);
                if hovered.as_ref() == Some(&target) {
                    fill(frame, palette.hover, remove.x, remove.y, remove.w, remove.h, 6.0);
                }
                frame.icon(Icon::Close, Point { x: remove.x + 14.0, y: cy }, 14.0, palette.text2);
                self.targets.push((remove, target));
                let size_x = remove.x - 8.0 - size_w;
                text_centred(frame, &size, hint, palette.text2, size_x, cy, size_w + 1.0, Align::Start);
                let name_x = left + 50.0;
                text_centred(frame, &chosen.name, label, palette.text, name_x, cy, size_x - 12.0 - name_x, Align::Start);
                row += ITEM;
            }
            if more > 0 {
                let said = match lang {
                    Lang::Zh => format!("另有 {more} 项"),
                    Lang::En => format!("{more} more"),
                };
                text_centred(frame, &said, hint, palette.text2, left + 50.0, row + 10.0, width - 62.0, Align::Start);
            }
            y += tall + 12.0;
        }
        // The devices.
        text_centred(frame, pick(lang, "附近的设备", "Devices nearby"), small, palette.text2, left, y + 8.0, width - 40.0, Align::Start);
        let refresh = Rect { x: right - 28.0, y: y - 6.0, w: 28.0, h: 28.0 };
        if hovered.as_ref() == Some(&Target::SendRefresh) {
            fill(frame, palette.hover, refresh.x, refresh.y, refresh.w, refresh.h, 6.0);
        }
        frame.icon(Icon::Update, Point { x: refresh.x + 14.0, y: refresh.y + 14.0 }, 15.0, palette.text2);
        self.targets.push((refresh, Target::SendRefresh));
        y += 24.0;
        let shown = crate::nearby::shown();
        let devices = self.send_devices(&shown);
        if devices.is_empty() {
            across(frame, pick(lang, "正在查找附近的设备", "Looking for devices nearby"), hint, palette.text2, left, y + 18.0, width);
            y += 36.0;
        }
        for device in &devices {
            y = self.paint_send_device(frame, palette, device, &shown, left, y, width, &hovered) + 6.0;
        }
        // What a press said, else what to do.
        let said = self.send.said.map(|(said, _)| said).or_else(|| self.send.chosen.is_empty().then(|| pick(lang, "选择内容后，点设备即可发送", "Choose what to send, then press a device")));
        if let Some(said) = said {
            let tall = frame.gfx.wrapped_height(said, hint, width);
            frame.text_wrapped(said, hint, palette.text2, left, y + 4.0, width);
            y += tall + 8.0;
        }
        frame.unclip();
        // As far as it scrolls (its content taller than where it shows), and what of it is pressed only where it shows.
        let content = y + self.send.scroll - area.y;
        self.send.most = (content - view.h).max(0.0);
        self.send.scroll = self.send.scroll.clamp(0.0, self.send.most);
        let shown_in = |rect: Rect| {
            let (top, bottom) = (rect.y.max(view.y), (rect.y + rect.h).min(view.y + view.h));
            (bottom > top).then_some(Rect { y: top, h: bottom - top, ..rect })
        };
        let scrolled: Vec<(Rect, Target)> = self.targets.drain(targets_from..).filter_map(|(rect, target)| Some((shown_in(rect)?, target))).collect();
        self.targets.extend(scrolled);
        if let Some(renaming) = self.renaming.as_mut().filter(|renaming| renaming.of == Named::Words) {
            renaming.field = renaming.field.and_then(shown_in);
        }
        if self.send.most > 0.0 {
            let thumb = (view.h * view.h / content).max(24.0);
            let at = view.y + (view.h - thumb) * self.send.scroll / self.send.most;
            fill(frame, palette.text3.alpha(0.45), area.x + area.w - 6.0, at + 2.0, 3.0, thumb - 4.0, 1.5);
        }
        // What this PC is called to them, with the way to change it.
        let foot = area.y + area.h - FOOT;
        fill(frame, palette.rule, left, foot, width, 1.0, 0.0);
        let named = match lang {
            Lang::Zh => format!("附近设备上显示为 {}", self.pc_called()),
            Lang::En => format!("Devices nearby see it as {}", self.pc_called()),
        };
        let change_left = self.button(frame, palette, pick(lang, "更改", "Change"), right, foot + FOOT / 2.0, Target::Manage, Button::Plain, &hovered);
        text_centred(frame, &named, hint, palette.text2, left, foot + FOOT / 2.0, change_left - 12.0 - left, Align::Start);
        // Something dragged over it: where it is let go.
        if self.send.over {
            let inner = Rect { x: area.x + 6.0, y: area.y + 6.0, w: area.w - 12.0, h: area.h - 12.0 };
            fill(frame, palette.window.alpha(0.88), inner.x, inner.y, inner.w, inner.h, 8.0);
            frame.stroke_rounded_dashed(palette.selection, inner.x, inner.y, inner.w, inner.h, 8.0, 2.0);
            across(frame, pick(lang, "松开以添加到发送", "Let go to add it"), Font::new(Family::Segoe, 15.0, 600.0), palette.selection, inner.x, inner.y + inner.h / 2.0, inner.w);
        }
    }

    /// A device in the pane at `y`: its icon, its name and its tags, how a sending to it goes (with a bar along its foot
    /// while it goes, and what cancels it while it is waited on), dimmed where it is a phone paired not receiving.
    /// Where the next goes.
    #[allow(clippy::too_many_arguments)]
    fn paint_send_device(&mut self, frame: &Frame, palette: &Palette, device: &SendDevice, shown: &crate::nearby::Shown, left: f32, y: f32, width: f32, hovered: &Option<Target>) -> f32 {
        let lang = self.lang;
        let rect = Rect { x: left, y, w: width, h: DEVICE };
        let target = Target::SendTo(device.fingerprint.clone());
        let lit = hovered.as_ref() == Some(&target) && device.found;
        fill(frame, if lit { palette.hover } else { palette.control }, rect.x, rect.y, rect.w, rect.h, 8.0);
        stroke_inside(frame, rect, 8.0, palette.rule);
        let (ink, ink2) = if device.found { (palette.text, palette.text2) } else { (palette.text3, palette.text3) };
        let cy = y + DEVICE / 2.0;
        fill(frame, palette.window, left + 10.0, cy - 17.0, 34.0, 34.0, 10.0);
        frame.icon(device.icon, Point { x: left + 27.0, y: cy }, 19.0, if device.found { palette.selection } else { palette.text3 });
        // How it goes, at its right.
        let job = self.send.job_to(&device.fingerprint, shown);
        let hint = Font::new(Family::Segoe, HINT, 400.0);
        let mut right = left + width - 12.0;
        let (said, good, cancel, part): (Cow<'static, str>, bool, Option<u64>, Option<f32>) = match (&job, device.found) {
            (None, false) => (Cow::Borrowed(pick(lang, "未开启接收", "Not receiving")), false, None, None),
            (None, true) => (Cow::Borrowed(""), false, None, None),
            (Some((_, None)), _) => (Cow::Borrowed(pick(lang, "正在准备", "Getting ready")), false, None, None),
            (Some((job, Some(state))), _) => match state {
                Sending::Waiting => (Cow::Borrowed(pick(lang, "等待对方接受", "Waiting to be taken")), false, Some(*job), None),
                Sending::Going { done, of } => {
                    let part = if *of == 0 { 0.0 } else { (*done as f32 / *of as f32).clamp(0.0, 1.0) };
                    (Cow::Owned(format!("{}%", (part * 100.0).round() as u32)), false, Some(*job), Some(part))
                }
                Sending::Sent => (Cow::Borrowed(pick(lang, "已发送", "Sent")), true, None, None),
                Sending::Refused => (Cow::Borrowed(pick(lang, "对方已拒绝", "Turned down")), false, None, None),
                Sending::Busy => (Cow::Borrowed(pick(lang, "对方正在接收其他文件", "Receiving something else")), false, None, None),
                Sending::Pin { .. } => (Cow::Borrowed(pick(lang, "需要 PIN 码", "A PIN is needed")), false, Some(*job), None),
                Sending::Cancelled => (Cow::Borrowed(pick(lang, "已取消", "Cancelled")), false, None, None),
                Sending::Failed { .. } => (Cow::Borrowed(pick(lang, "发送失败", "Not sent")), false, None, None),
            },
        };
        if let Some(job) = cancel {
            let x = Rect { x: right - 28.0, y: cy - 14.0, w: 28.0, h: 28.0 };
            let target = Target::SendCancel(job);
            if hovered.as_ref() == Some(&target) {
                fill(frame, palette.window, x.x, x.y, x.w, x.h, 6.0);
            }
            frame.icon(Icon::Close, Point { x: x.x + 14.0, y: cy }, 14.0, palette.text2);
            self.targets.push((x, target));
            right = x.x - 6.0;
        }
        if !said.is_empty() {
            let said_w = frame.gfx.measure(&said, hint);
            let good_color = if self.dark { Color::hex(0x58C98B, 1.0) } else { Color::hex(0x1E7B45, 1.0) };
            text_centred(frame, &said, hint, if good { good_color } else { ink2 }, right - said_w, cy, said_w + 1.0, Align::Start);
            right -= said_w;
            if good {
                frame.icon(Icon::Check, Point { x: right - 10.0, y: cy }, 15.0, good_color);
                right -= 20.0;
            }
            right -= 12.0;
        }
        // Its name, and its tags under it.
        let words_left = left + 56.0;
        let room = (right - words_left).max(40.0);
        frame.clip(words_left, y, room, DEVICE);
        text_centred(frame, &device.name, Font::new(Family::Segoe, 13.5, 600.0), ink, words_left, cy - 9.0, room, Align::Start);
        let tag_font = Font::new(Family::Segoe, 11.0, 400.0);
        let mut x = words_left;
        for tag in &device.tags {
            let w = frame.gfx.measure(tag, tag_font) + 12.0;
            fill(frame, palette.window, x, cy + 3.0, w, 17.0, 4.0);
            text_centred(frame, tag, tag_font, ink2, x + 6.0, cy + 11.5, w, Align::Start);
            x += w + 6.0;
        }
        frame.unclip();
        if let Some(part) = part {
            frame.clip(rect.x, rect.y, rect.w, rect.h);
            fill(frame, palette.selection, rect.x + 8.0, rect.y + rect.h - 4.0, (rect.w - 16.0) * part, 3.0, 1.5);
            frame.unclip();
        }
        // (After what cancels it: the first under the pointer is pressed, see `hovered`.)
        self.targets.push((rect, target));
        y + DEVICE
    }

    /// The box words are written in to send, at `y`: wrapped, the caret and what is chosen in them shown, how many
    /// characters under them, and what adds them or leaves them. Where it ends.
    fn paint_writer(&mut self, frame: &Frame, palette: &Palette, left: f32, y: f32, width: f32, now: Instant) -> f32 {
        let lang = self.lang;
        let hovered = self.hovered_now.clone();
        let font = WORDS_FONT;
        let boxed = Rect { x: left, y, w: width, h: WRITER + 44.0 };
        fill(frame, palette.control, boxed.x, boxed.y, boxed.w, boxed.h, 8.0);
        Canvas::stroke_rounded(frame, crate::ui::canvas::Fill::Solid(palette.selection), boxed.x + 0.75, boxed.y + 0.75, boxed.w - 1.5, boxed.h - 1.5, 7.25, 1.5);
        let field = Rect { x: left + 10.0, y: y + 10.0, w: width - 20.0, h: WRITER - 14.0 };
        let Some(renaming) = self.renaming.as_mut() else { return y };
        let text = renaming.typing.string();
        let layout = frame.gfx.wrapped(&text, font, field.w);
        let (caret_x, caret_top, caret_tall) = words_caret(&layout, &renaming.typing.text, renaming.typing.caret);
        // Scrolled to show the caret.
        let along = (caret_top + caret_tall - field.h).max(0.0).max(renaming.along.min(caret_top));
        renaming.along = along;
        frame.clip(field.x, field.y, field.w, field.h);
        if text.is_empty() {
            text_centred(frame, pick(lang, "输入要发送的文字", "Write what to send"), font, palette.text3, field.x, field.y + 9.0, field.w, Align::Start);
        }
        let selection = renaming.typing.selection();
        if !selection.is_empty() {
            let mut count = 0;
            let _ = unsafe { layout.HitTestTextRange(selection.start as u32, selection.len() as u32, 0.0, 0.0, None, &mut count) };
            let mut pieces = vec![windows::Win32::Graphics::DirectWrite::DWRITE_HIT_TEST_METRICS::default(); count as usize];
            if unsafe { layout.HitTestTextRange(selection.start as u32, selection.len() as u32, 0.0, 0.0, Some(&mut pieces), &mut count) }.is_ok() {
                for piece in &pieces {
                    frame.fill(palette.selection.alpha(0.3), field.x + piece.left, field.y + piece.top - along, piece.width, piece.height);
                }
            }
        }
        frame.text_wrapped(&text, font, palette.text, field.x, field.y - along, field.w);
        let blinks = now.duration_since(renaming.blink_from).as_millis() / BLINK.as_millis();
        if blinks.is_multiple_of(2) && !renaming.composing {
            frame.fill(palette.text, field.x + caret_x, field.y + caret_top - along, 1.2, caret_tall);
        }
        frame.unclip();
        renaming.field = Some(field);
        renaming.caret = Some((field.x + caret_x, field.y + caret_top - along, caret_tall));
        let composing = renaming.composing;
        let count = text.chars().filter(|c| !c.is_whitespace()).count();
        if composing {
            self.place_ime();
        }
        // How many characters, and what adds them or leaves them.
        let foot = y + WRITER + 18.0;
        let said = match lang {
            Lang::Zh => format!("{count} 字"),
            Lang::En => format!("{count} characters"),
        };
        text_centred(frame, &said, Font::new(Family::Segoe, HINT, 400.0), palette.text2, left + 12.0, foot, width / 2.0, Align::Start);
        let add_left = self.button(frame, palette, pick(lang, "添加", "Add"), left + width - 10.0, foot, Target::SendWordsAdd, Button::Accent, &hovered);
        self.button(frame, palette, pick(lang, "取消", "Cancel"), add_left - 8.0, foot, Target::SendWordsCancel, Button::Plain, &hovered);
        y + boxed.h
    }

    /// What a press in the pane does (see `Target`'s `Send…`).
    pub(super) fn send_press(&mut self, target: Target) {
        match target {
            Target::SendKind(kind @ (0 | 1)) => {
                if self.send.choosing.is_some() {
                    return;
                }
                let (hwnd, lang) = (self.hwnd.0 as isize, self.lang);
                let kinds = if kind == 0 { crate::ui::beside::Kinds::Files } else { crate::ui::beside::Kinds::Folders };
                self.send.choosing = Some(std::thread::spawn(move || unsafe {
                    let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
                    crate::ui::beside::choose(HWND(hwnd as *mut _), kinds, lang)
                }));
            }
            Target::SendKind(2) => {
                if self.writing_words() {
                    return;
                }
                let draft = std::mem::take(&mut self.send.draft);
                self.begin_naming(Named::Words, &draft, WORDS_MOST);
                if let Some(renaming) = self.renaming.as_mut() {
                    let end = renaming.typing.text.len();
                    renaming.typing.go(end, false);
                }
            }
            // Read once, as it is pressed: what may not leave this PC is not taken.
            Target::SendKind(_) => match crate::station::clipboard::read(self.hwnd, crate::station::clipboard::Why::Pasted) {
                Ok(Some(copied)) if copied.stays_here => self.send.say(pick(self.lang, "剪贴板中的内容不允许发送到其他设备", "What is on the clipboard may not leave this PC")),
                Ok(Some(copied)) => self.send.add_given(vec![copied.content], self.lang),
                Ok(None) => self.send.say(pick(self.lang, "剪贴板中没有内容", "Nothing is on the clipboard")),
                Err(_) => self.send.say(pick(self.lang, "剪贴板正被其他程序使用，请稍后再试", "Another program holds the clipboard. Try again in a moment")),
            },
            Target::SendClear => self.send.chosen.clear(),
            Target::SendRemove(id) => self.send.chosen.retain(|chosen| chosen.id != id),
            Target::SendRefresh => {
                crate::nearby::announce();
            }
            Target::SendCancel(job) => {
                crate::nearby::cancel(job);
            }
            Target::SendWordsAdd => self.end_rename(true),
            Target::SendWordsCancel => self.end_rename(false),
            Target::SendTo(fingerprint) => self.send_to(fingerprint),
            _ => {}
        }
        self.next_frame = Instant::now();
    }

    /// All that is chosen sent to the device of `fingerprint` (put together as files on a thread of its own first:
    /// words and pictures written as files, folders walked): unless nothing is chosen, it is a phone paired that does
    /// not receive now, or a sending to it goes on.
    fn send_to(&mut self, fingerprint: String) {
        let shown = crate::nearby::shown();
        let Some(device) = self.send_devices(&shown).into_iter().find(|device| device.fingerprint == fingerprint) else { return };
        if !device.found {
            self.send.say(pick(self.lang, "在手机上打开 Glance，或在 App 中开启“后台接收”", "Open Glance on the phone, or turn on Receive in the background in the app"));
            return;
        }
        if self.send.chosen.is_empty() || self.send.job_to(&fingerprint, &shown).is_some_and(|(_, state)| state.is_none_or(|state| !state.over())) {
            return;
        }
        let job = crate::nearby::job();
        let contents: Vec<Content> = self.send.chosen.iter().map(|chosen| chosen.content.clone()).collect();
        let to = fingerprint.clone();
        std::thread::spawn(move || {
            let files = crate::station::send::outgoing(&contents, &crate::station::send::folder_of(job));
            crate::nearby::send(job, &to, &files);
        });
        self.send.jobs.push(Job { to: fingerprint, job, over_at: None });
    }

    /// Where the pane is, as it was just drawn (none: not drawn), told the station (screen px): what is dragged over it
    /// is caught there (see `station::catch`), and told the window (see `dragged_in`).
    pub(super) fn tell_send_area(&self) {
        let px = |dips: f32| (dips * self.scale).round() as i32;
        let areas = self.send.area.map(|area| {
            let mut corners = [POINT { x: px(area.x), y: px(area.y) }, POINT { x: px(area.x + area.w), y: px(area.y + area.h) }];
            unsafe {
                let _ = windows::Win32::Graphics::Gdi::ClientToScreen(self.hwnd, &mut corners[0]);
                let _ = windows::Win32::Graphics::Gdi::ClientToScreen(self.hwnd, &mut corners[1]);
            }
            (RECT { left: corners[0].x, top: corners[0].y, right: corners[1].x, bottom: corners[1].y }, crate::station::run::Lands::Send)
        });
        crate::station::run::shows_at(self.hwnd, areas.into_iter().collect());
    }

    /// The wheel turned `delta` over the pane: scrolled, where it can be. Whether it was over it.
    pub(super) fn send_wheel(&mut self, delta: i16) -> bool {
        let over = self.page == Page::Nearby && matches!((self.send.area, self.pointer), (Some(area), Some((x, y))) if x >= area.x && x < area.x + area.w && y >= area.y && y < area.y + area.h);
        if over {
            self.send.scroll = (self.send.scroll - delta as f32 / 120.0 * WHEEL).clamp(0.0, self.send.most);
            self.next_frame = Instant::now();
        }
        over
    }

    /// Something dragged over the pane (`told`: 1 over it, 0 gone, 2 let go there: see `SEND_DRAGGED`).
    pub(super) fn send_dragged(&mut self, told: usize) {
        self.send.over = told == 1;
        self.next_frame = Instant::now();
    }
}

/// Where the caret at `at` in `text` laid out as `layout` is drawn: its left, its top, how tall. After a newline that
/// ends the words, at the start of the line under it (where DirectWrite puts it at the end of the newline's line).
fn words_caret(layout: &IDWriteTextLayout, text: &[u16], at: usize) -> (f32, f32, f32) {
    if at > 0 && at == text.len() && text[at - 1] == u16::from(b'\n') {
        let (_, top, tall) = crate::ui::typing::caret_at(layout, at - 1);
        return (0.0, top + tall, tall);
    }
    crate::ui::typing::caret_at(layout, at)
}

/// `text` in `font` and `color` centred across `width` from `x`, and on `cy` (cut short where it is wider).
#[allow(clippy::too_many_arguments)]
fn across(frame: &Frame, text: &str, font: Font, color: Color, x: f32, cy: f32, width: f32) {
    let w = frame.gfx.measure(text, font).min(width);
    text_centred(frame, text, font, color, x + (width - w) / 2.0, cy, w + 1.0, Align::Start);
}

/// For pictures: the pane with `paths` and `words` chosen (folders measured at once), and sendings to the devices of
/// these fingerprints going as said (see `nearby::pretend_sendings`).
#[cfg(all(test, feature = "studio"))]
pub(super) fn pretend(ui: &mut Ui, paths: Vec<std::path::PathBuf>, words: &str, sendings: Vec<(&str, Sending)>) {
    ui.send = Pane::default();
    ui.send.add_paths(paths);
    ui.send.add_words(words.to_string());
    for chosen in &mut ui.send.chosen {
        if let (None, Content::Files(paths)) = (chosen.size, &chosen.content) {
            chosen.size = Some(folder_size(&paths[0]));
        }
    }
    let mut told = std::collections::HashMap::new();
    for (to, state) in sendings {
        let job = crate::nearby::job();
        ui.send.jobs.push(Job { to: to.to_string(), job, over_at: None });
        told.insert(job, state);
    }
    crate::nearby::pretend_sendings(told);
}
