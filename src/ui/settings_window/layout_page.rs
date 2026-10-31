//! The layout page: the components, to be dragged onto the edges of the screens, one to an edge; the screens as
//! Windows arranges them, every edge with what it opens (all of them small, or one screen large, its desktop and the
//! real panel slid out from the edge last pressed); a component on an edge dragged to another (the two change places)
//! or off it.

use std::time::Instant;

use windows::Win32::Foundation::RECT;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};

use super::*;
use crate::settings::{Part, Places, PARTS};

/// The library: a tile for each component, side by side, how tall each is and the room between them; the picture of
/// the component at its top, how tall, and the room round it.
/// (The picture, its name and word under it, and its padding above and below.)
const TILE: f32 = TILE_PAD + TILE_PICTURE + 8.0 + 40.0 + TILE_PAD;
const TILE_GAP: f32 = 12.0;
const TILE_PICTURE: f32 = 88.0;
const TILE_PAD: f32 = 12.0;
/// What plays, drawn as its strip at the least a music widget is wide, made smaller into its picture; the room round
/// each component in its picture.
const TILE_MUSIC: (f32, f32) = (240.0, 72.0);
const TILE_ROOM: f32 = 8.0;
/// A row of the editor's head (the screens to show, every screen alike), and the room round the screens under it.
const EDITOR_ROW: f32 = 46.0;
const EDITOR_ROOM: f32 = 20.0;
/// A line of the words under the screens saying how each kind of edge opens.
const LEGEND_ROW: f32 = 20.0;
/// How far into a screen its edges take a drop: drawn small, and one screen large.
const ZONE_SMALL: f32 = 30.0;
const ZONE_LARGE: f32 = 112.0;
const ZONE_TOP_LARGE: f32 = 48.0;
/// A component on an edge: its pill's height, drawn small (its icon in a disc) and large (its icon and name, and a
/// button that takes it off); how thick the edge's line is.
const CHIP_SMALL: f32 = 22.0;
const CHIP: f32 = 28.0;
const EDGE_LINE: f32 = 3.0;
/// While a component is carried: how far into a screen each edge's glow reaches (drawn small, and one screen large),
/// and how strong it is at the edge (every edge, and the one under the pointer); how thick that edge's line grows.
const GLOW_SMALL: f32 = 32.0;
const GLOW_LARGE: f32 = 96.0;
const GLOW: (f32, f32) = (0.25, 0.75);
const EDGE_LINE_OVER: f32 = 5.0;
/// How far the pointer moves before a press on a component carries it.
const CARRY_FROM: f32 = 4.0;

/// A component carried in the editor: from the library, or off an edge (its screen and edge); where it was taken
/// (window DIPs), and whether it has gone far enough from there to be carried.
pub(super) struct Carry {
    pub part: Part,
    pub from: Option<(String, Edge)>,
    pub start: (f32, f32),
    pub moved: bool,
}

/// An edge of a screen in the editor as last drawn: where it takes a drop.
pub(super) struct Zone {
    pub screen: String,
    pub edge: Edge,
    pub rect: Rect,
}

pub(super) fn part_name(part: Part, lang: Lang) -> &'static str {
    match part {
        Part::Monitor => pick(lang, "监控", "Monitor"),
        Part::Workbench => pick(lang, "工作台", "Workbench"),
        Part::Music => pick(lang, "音乐", "Music"),
    }
}

/// What a component shows, in a few words (under its name in its tile).
fn part_word(part: Part, lang: Lang) -> &'static str {
    match part {
        Part::Monitor => pick(lang, "实时硬件数据", "Live readings"),
        Part::Workbench => pick(lang, "时间与工具", "Time and tools"),
        Part::Music => pick(lang, "歌曲与歌词", "Songs and lyrics"),
    }
}

pub(super) fn part_icon(part: Part) -> Icon {
    match part {
        Part::Monitor => Icon::Gauge,
        Part::Workbench => Icon::PencilRuler,
        Part::Music => Icon::Music,
    }
}

impl Ui {
    /// The library row's height: its words, then the components' tiles side by side.
    pub(super) fn library_height(&self) -> f32 {
        let (_, word) = self.library_words();
        let room = self.cards() - 2.0 * ROW_SIDE;
        22.0 + self.gfx.wrapped_height(word, Font::new(Family::Segoe, HINT, 400.0), room) + 14.0 + TILE + ROW_SIDE
    }

    fn library_words(&self) -> (&'static str, &'static str) {
        (
            pick(self.lang, "组件", "Components"),
            pick(
                self.lang,
                "将组件拖到右侧屏幕的边缘，即可从该边缘呼出。每条边缘放置一个组件，同一组件可放在多条边缘。拖到已有组件的边缘将替换原组件，在两条边缘之间拖动则互换位置",
                "Drag a component onto a screen's edge on the right to open it from there. Each edge holds one, and a component can be on several. Dropped on an edge that holds one, it replaces it. Dragged between two edges, the two change places",
            ),
        )
    }

    /// The library: a tile for each component, side by side, to be dragged onto an edge: at its top the component
    /// as it looks now, small (the readings' load and its chart, the workbench's time and what plays, what plays as its
    /// strip shows it), its name and what it shows under that.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn paint_library(&mut self, frame: &Frame, palette: &Palette, left: f32, y: f32, width: f32, height: f32, hovered: &Option<Target>) {
        card(frame, palette, left, y, width, height, palette.card);
        self.library_at = Some(Rect { x: left, y, w: width, h: height });
        let (label, hint) = (Font::new(Family::Segoe, 14.0, 400.0), Font::new(Family::Segoe, HINT, 400.0));
        let (name, word) = self.library_words();
        let room = width - 2.0 * ROW_SIDE;
        let head = 22.0 + self.gfx.wrapped_height(word, hint, room) + 14.0;
        field_label(frame, palette, name, Some(word), label, hint, left + ROW_SIDE, y + head / 2.0, room);
        let wide = (room - TILE_GAP * (PARTS.len() - 1) as f32) / PARTS.len() as f32;
        for (index, part) in PARTS.into_iter().enumerate() {
            let r = Rect { x: left + ROW_SIDE + index as f32 * (wide + TILE_GAP), y: y + head, w: wide, h: TILE };
            let target = Target::Library(part);
            let carried = self.carry.as_ref().is_some_and(|carry| carry.moved && carry.part == part && carry.from.is_none());
            fill(frame, if hovered.as_ref() == Some(&target) || carried { palette.control_on } else { palette.control }, r.x, r.y, r.w, r.h, 8.0);
            stroke_inside(frame, r, 8.0, palette.card_stroke);
            // Its picture: the component as it looks now, as a widget of it would on the desktop, small (each the
            // same way: the real thing over the wallpaper).
            let picture = Rect { x: r.x + TILE_PAD, y: r.y + TILE_PAD, w: r.w - 2.0 * TILE_PAD, h: TILE_PICTURE };
            let (sw, sh) = self.stage.size;
            let cover = (picture.w / sw).max(picture.h / sh);
            // (The desktop as held already, whatever the panel's size it was made for: drawn to the picture's size, it
            // is not made again for each.)
            let held = self.stage.bitmap.as_ref().map_or(1.0, |(key, _)| f32::from_bits(*key));
            self.stage_picture(frame, picture, (picture.x + (picture.w - sw * cover) / 2.0, picture.y + (picture.h - sh * cover) / 2.0), cover, held);
            unsafe { frame.dc.PushAxisAlignedClip(&rect(picture.x, picture.y, picture.w, picture.h), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
            let inside = Rect { x: picture.x + TILE_ROOM, y: picture.y + TILE_ROOM, w: picture.w - 2.0 * TILE_ROOM, h: picture.h - 2.0 * TILE_ROOM };
            match part {
                Part::Monitor => self.component_picture(frame, Part::Monitor, inside, 0),
                Part::Workbench => self.component_picture(frame, Part::Workbench, inside, 1),
                Part::Music => {
                    let mut shows = super::super::music::Shows::from(&self.settings.view);
                    // As a strip: one line, the cover and what plays beside it, made smaller into the picture (on a
                    // ground: with none, its words would be lost on the picture's desktop).
                    if shows.ground == MusicGround::Clear {
                        shows.ground = MusicGround::Cover;
                    }
                    let k = (inside.w / TILE_MUSIC.0).min(inside.h / TILE_MUSIC.1).min(1.0);
                    let (w, h) = (TILE_MUSIC.0 * k, TILE_MUSIC.1 * k);
                    let local = Matrix3x2::scale(k, k) * Matrix3x2::translation(inside.x + (inside.w - w) / 2.0, inside.y + (inside.h - h) / 2.0);
                    // (Light or dark as the readings' pictures are, on the skin's ground as theirs: see `component_picture`.)
                    let middle = self.stage.middle();
                    let (theme, frost) = self.stage.look(Skin::named(&self.settings.skin), self.prefs.theme, middle);
                    let ground = super::super::music::Ground::of(&shows, theme.dark);
                    if let super::super::music::Ground::Skin { .. } = ground {
                        frame.origin(0.0, 0.0);
                        self.stage.music_ground(frame, &mut self.library_music_ground, &theme, frost, TILE_MUSIC, None, local, self.scale * k);
                    }
                    frame.place(local);
                    let (family, now) = (theme.body.family, Instant::now());
                    let _ = if super::super::music::heard() {
                        self.library_music.paint(frame, TILE_MUSIC, &shows, ground, family, self.scale * k, None, now)
                    } else {
                        self.library_music.paint_example(frame, TILE_MUSIC, &shows, ground, family, self.scale * k, None, now, self.example_from.elapsed().as_secs_f64())
                    };
                    frame.origin(0.0, 0.0);
                }
            }
            unsafe { frame.dc.PopAxisAlignedClip() };
            let below = picture.y + picture.h + 8.0;
            text_centred(frame, part_name(part, self.lang), Font::new(Family::Segoe, 13.5, 600.0), palette.text, r.x + TILE_PAD + 2.0, below + 10.0, r.w - 2.0 * TILE_PAD, Align::Start);
            text_centred(frame, part_word(part, self.lang), hint, palette.text2, r.x + TILE_PAD + 2.0, below + 30.0, r.w - 2.0 * TILE_PAD, Align::Start);
            self.targets.push((r, target));
        }
    }

    /// The readings (as their corner shows them: the processor's last minutes, the chips' rings, the network) or the
    /// workbench (as its panel does), drawn as the skin draws them, as large as `area` holds, in its middle; in the
    /// layers `layers` (one each: see `library_layers`).
    fn component_picture(&mut self, frame: &Frame, part: Part, area: Rect, layers: usize) {
        let lang = self.lang;
        let app = crate::app();
        let skin = Skin::named(&self.settings.skin);
        let prefs = self.prefs.clone().holding(part);
        let history = app.controller.history.read().unwrap();
        let samples = crate::panel::contiguous(&history);
        if samples.is_empty() {
            return;
        }
        let wall = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64() * 1000.0;
        let pen = wall - self.settings.interval_ms as f64 - PEN_LAG_MS;
        let seen = app.controller.seen.lock().unwrap().clone();
        let (overlay, newer) = (app.controller.overlay_wanted(), update::available().is_some());
        // Light or dark as the desktop is under the picture (where the skin follows it).
        let (sw, sh) = self.stage.size;
        let middle = self.stage.middle();
        let (theme, frost) = self.stage.look(skin, prefs.theme, middle);
        let scene = Scene { music_apart: false, hint: None, thumbs: &crate::ui::view::RESTING, info: &app.info, prefs: &prefs, theme: &theme, lang, history: samples, seen: &seen, pen_ms: pen, scrolls: crate::ui::view::Scrolls::default(), tab: crate::ui::view::Tab::Shelf, hover: None, pinned: false, overlay, buttons: false, newer, settings_lit: None };
        let heights = view::heights(&scene);
        let lanes: Vec<(&str, arrange::Heights)> = heights.iter().map(|(id, height)| (id.as_str(), *height)).collect();
        let readings = super::super::forms::count(&scene);
        let want = match part {
            Part::Monitor => arrange::Want::Kept(arrange::Choice { form: arrange::Form::Small(super::super::forms::Small::Corner), scale: 1.0, extra: (0.0, 0.0) }),
            _ => arrange::Want::Itself,
        };
        let opening = arrange::Opening::new(&theme, Edge::Right, &lanes, readings, (sw, sh), None, 1.0, want, seen.clone());
        let drawn = (opening.size.0 * opening.zoom, opening.size.1 * opening.zoom);
        let k = (area.w / drawn.0).min(area.h / drawn.1).min(1.0);
        let at = (area.x + (area.w - drawn.0 * k) / 2.0, area.y + (area.h - drawn.1 * k) / 2.0);
        let detail = opening.detail();
        let shown = view::lanes_at(&scene, |_| detail);
        let picture = render::Picture { scene: &scene, lanes: &shown, layout: &opening.layout, small: opening.small().map(|small| (small, opening.size)), size: opening.size, at: (0.0, 0.0), edge: None, backdrop: None, frost, rough: false };
        let zoom = opening.zoom * k;
        let drew = self.library_layers[layers].draw(frame, &picture, Matrix3x2::scale(zoom, zoom) * Matrix3x2::translation(at.0, at.1), self.scale * zoom);
        if let Err(error) = drew {
            frame.fail(error);
        }
    }

    /// The screen shown large in the editor, by its name: the one chosen, while it is there; every screen small, else
    /// (but the one screen there is, shown large).
    pub(super) fn editor_screen(&self) -> Option<String> {
        if let [only] = self.screens.as_slice() {
            return Some(only.id.clone());
        }
        self.view.clone().filter(|id| self.screens.iter().any(|screen| screen.id == *id))
    }

    /// The editor, in the preview's room: its head (the screens to show, every screen alike), the screens, and the
    /// words under them. Returns how soon to draw again (see `paint`), while the real panel shows.
    pub(super) fn paint_layout(&mut self, frame: &Frame, palette: &Palette, area: Rect, now: Instant) -> Option<f32> {
        let hovered = self.hovered_now.clone();
        let lang = self.lang;
        self.zones.clear();
        self.editor_at = Some(area);
        let places = self.settings.places().clone();
        // The head: a button for every screen together and one for each, and every screen alike. They follow one
        // another in rows as wide as the editor, a row more where one does not hold the next (the switch at the end
        // of the last, or on a row of its own).
        let width = area.w - 2.0 * EDITOR_ROOM;
        let shown = self.editor_screen();
        let mut head = 6.0;
        if self.screens.len() > 1 {
            let mut views = vec![(pick(lang, "全部屏幕", "All screens").to_string(), None)];
            views.extend(self.screens.iter().enumerate().map(|(i, screen)| (if lang == Lang::Zh { format!("屏幕 {}", i + 1) } else { format!("Screen {}", i + 1) }, Some(screen.id.clone()))));
            let font = Font::new(Family::Segoe, 14.0, 400.0);
            let (mut x, mut cy) = (0.0, area.y + head + EDITOR_ROW / 2.0);
            for (text, view) in views {
                let w = frame.gfx.measure(&text, font) + 32.0;
                if x > 0.0 && x + w > width {
                    (x, cy) = (0.0, cy + EDITOR_ROW);
                }
                let kind = if view == shown { Button::Accent } else { Button::Plain };
                self.button(frame, palette, &text, area.x + EDITOR_ROOM + x + w, cy, Target::View(view), kind, &hovered);
                x += w + 8.0;
            }
            let label = pick(lang, "所有屏幕同样布局", "Every screen alike");
            let label_w = frame.gfx.measure(label, font);
            let switch = label_w + 12.0 + 40.0;
            if x + 16.0 + switch > width {
                cy += EDITOR_ROW;
            }
            let toggle_x = area.x + area.w - EDITOR_ROOM - 40.0;
            text_centred(frame, label, font, palette.text, toggle_x - 12.0 - label_w, cy, label_w + 4.0, Align::Start);
            let pressed = self.pressed == Some(Target::Same);
            self.toggle(frame, palette, "layout:same".into(), places.same, true, toggle_x, cy, now, pressed);
            self.targets.push((Rect { x: toggle_x - 12.0 - label_w, y: cy - 16.0, w: label_w + 52.0, h: 32.0 }, Target::Same));
            head = cy + EDITOR_ROW / 2.0 - area.y;
        }
        // The words under the screens: how each kind of edge opens (a line each, side by side where they fit), and
        // under that what a press does, in as many lines as it takes. Measured first: the screens have the rest.
        let small = Font::new(Family::Segoe, HINT, 400.0);
        let legends = [(false, pick(lang, "外边缘：向外推动呼出", "Outer edge: push past it")), (true, pick(lang, "屏幕相接处：停留片刻呼出", "Where screens meet: rest on it"))];
        let mark = 18.0 + 6.0;
        let mut placed = Vec::new();
        let (mut x, mut row) = (0.0, 0usize);
        for (dashed, text) in legends {
            let w = mark + frame.gfx.measure(text, small);
            if x > 0.0 && x + w > width {
                (x, row) = (0.0, row + 1);
            }
            placed.push((dashed, text, x, row));
            x += w + 20.0;
        }
        let rest = if shown.is_some() {
            pick(lang, "点击边缘可预览其面板，再次点击收起。底边为任务栏，不放置组件", "Click an edge to preview its panel, and again to close it. The bottom edge is the taskbar's")
        } else {
            pick(lang, "点击屏幕可放大查看。底边为任务栏，不放置组件", "Click a screen to enlarge it. The bottom edge is the taskbar's")
        };
        let rest_h = frame.gfx.wrapped_height(rest, small, width);
        let foot_h = 12.0 + (row + 1) as f32 * LEGEND_ROW + 4.0 + rest_h + 14.0;
        let canvas = Rect { x: area.x + EDITOR_ROOM, y: area.y + head + 8.0, w: width, h: area.h - head - 8.0 - foot_h };
        if canvas.w <= 40.0 || canvas.h <= 40.0 {
            return None;
        }
        let creep = match &shown {
            Some(screen) => self.paint_one_screen(frame, palette, &places, screen, canvas, &hovered),
            None => {
                self.paint_all_screens(frame, palette, &places, canvas, &hovered);
                None
            }
        };
        let top = area.y + area.h - foot_h + 12.0;
        for (dashed, text, x, row) in placed {
            let (x, cy) = (area.x + EDITOR_ROOM + x, top + row as f32 * LEGEND_ROW + LEGEND_ROW / 2.0);
            let h = EDGE_LINE;
            if dashed {
                for i in 0..3 {
                    fill(frame, palette.selection, x + i as f32 * 7.0, cy - h / 2.0, 4.0, h, h / 2.0);
                }
            } else {
                fill(frame, palette.selection, x, cy - h / 2.0, 18.0, h, h / 2.0);
            }
            let tw = frame.gfx.measure(text, small);
            text_centred(frame, text, small, palette.text2, x + mark, cy, tw + 4.0, Align::Start);
        }
        frame.text_wrapped(rest, small, palette.text3, area.x + EDITOR_ROOM, top + (row + 1) as f32 * LEGEND_ROW + 4.0, width);
        // What is carried, at the pointer.
        if let (Some(carry), Some((px, py))) = (self.carry.as_ref().filter(|carry| carry.moved), self.hand()) {
            let part = carry.part;
            self.chip(frame, palette, part, Rect { x: px + 10.0, y: py + 6.0, w: self.chip_width(frame, part) - CHIP + 8.0, h: CHIP }, true, 0.92, None);
        }
        creep
    }

    /// Every screen, small, as Windows arranges them: each its desktop and its number, its edges with what they open.
    fn paint_all_screens(&mut self, frame: &Frame, palette: &Palette, places: &Places, canvas: Rect, hovered: &Option<Target>) {
        let screens = self.screens.clone();
        let bound = |pick: fn(&RECT) -> i32, most: bool| {
            let all = screens.iter().map(|screen| pick(&screen.monitor));
            if most { all.max() } else { all.min() }.unwrap_or(0)
        };
        let (left, top, right, bottom) = (bound(|m| m.left, false), bound(|m| m.top, false), bound(|m| m.right, true), bound(|m| m.bottom, true));
        let (across, down) = ((right - left).max(1) as f32, (bottom - top).max(1) as f32);
        let k = (canvas.w / across).min(canvas.h / down);
        let (ox, oy) = (canvas.x + (canvas.w - across * k) / 2.0, canvas.y + (canvas.h - down * k) / 2.0);
        let number = Font::new(Family::Segoe, 13.0, 600.0);
        let monitors: Vec<RECT> = screens.iter().map(|screen| screen.monitor).collect();
        for (index, screen) in screens.iter().enumerate() {
            let m = screen.monitor;
            let r = Rect { x: ox + (m.left - left) as f32 * k + 2.0, y: oy + (m.top - top) as f32 * k + 2.0, w: (m.right - m.left) as f32 * k - 4.0, h: (m.bottom - m.top) as f32 * k - 4.0 };
            let target = Target::View(Some(screen.id.clone()));
            let pointed = hovered.as_ref() == Some(&target);
            let pixels = ((r.w * self.scale).round().max(1.0) as u32, (r.h * self.scale).round().max(1.0) as u32);
            match self.map_picture(frame, &screen.id, m, pixels) {
                Some(picture) => picture_rounded(frame, &picture, r, MAP_RADIUS * 2.0, if pointed { 1.0 } else { 0.85 }),
                None => fill(frame, palette.switch_off, r.x, r.y, r.w, r.h, MAP_RADIUS * 2.0),
            }
            stroke_inside(frame, r, MAP_RADIUS * 2.0, palette.card_stroke);
            let digit = (index + 1).to_string();
            // (At its foot, the taskbar's edge, which holds nothing: clear of the pills on its other edges. On a screen
            // drawn too small for them, all of its marks smaller, apart.)
            let size = marks_size(r);
            let number = Font { size: number.size * size, ..number };
            let digit_w = frame.gfx.measure(&digit, number);
            let round = number_at(r, size);
            fill(frame, palette.window.alpha(0.92), round.x, round.y, round.w, round.h, round.h / 2.0);
            text_centred(frame, &digit, number, palette.text, round.x + (round.w - digit_w) / 2.0, round.y + round.h / 2.0, digit_w + 1.0, Align::Start);
            let along = |px: i32, upright: bool| if upright { r.y + (px - m.top) as f32 * r.h / (m.bottom - m.top).max(1) as f32 } else { r.x + (px - m.left) as f32 * r.w / (m.right - m.left).max(1) as f32 };
            self.paint_zones(frame, palette, places, &screen.id, &monitors, index, r, &along, false, hovered);
            // The screen takes a press where its edges do not (they are found first).
            self.targets.push((r, target));
        }
    }

    /// One screen, large: its desktop, the real panel slid out from the edge last pressed (where that edge opens one),
    /// and its edges with what they open.
    fn paint_one_screen(&mut self, frame: &Frame, palette: &Palette, places: &Places, screen: &str, canvas: Rect, hovered: &Option<Target>) -> Option<f32> {
        let index = self.screens.iter().position(|known| known.id == screen)?;
        // While one is carried over an edge, what it would open there; else what the edge last pressed opens. (The
        // screen is drawn whole wherever a panel is: where it is is known before it is drawn.)
        let r = self.whole_stage(canvas);
        let carried = self.carry.as_ref().filter(|carry| carry.moved).and_then(|carry| {
            let (x, y) = self.hand()?;
            let edge = EDGES.into_iter().find(|&edge| {
                let zone = zone_of(r, edge, ZONE_LARGE, ZONE_TOP_LARGE);
                x >= zone.x && x < zone.x + zone.w && y >= zone.y && y < zone.y + zone.h
            })?;
            Some((edge, carry.part))
        });
        let held = carried.or_else(|| self.shown_edge.and_then(|edge| Some((edge, places.part(screen, edge)?))));
        let (creep, r) = match held {
            Some((edge, Part::Music)) => self.music_on_stage(frame, canvas, edge, 1.0),
            Some((edge, part)) => {
                let prefs = self.prefs.clone().holding(part);
                self.panel_on_stage(frame, canvas, Some(edge), Some(&prefs), true, 1.0)
            }
            None => self.panel_on_stage(frame, canvas, None, None, true, 1.0),
        };
        let work = self.stage.work;
        let monitors: Vec<RECT> = self.screens.iter().map(|known| known.monitor).collect();
        let along = |px: i32, upright: bool| if upright { r.y + (px - work.top) as f32 * r.h / (work.bottom - work.top).max(1) as f32 } else { r.x + (px - work.left) as f32 * r.w / (work.right - work.left).max(1) as f32 };
        self.paint_zones(frame, palette, places, screen, &monitors, index, r, &along, true, hovered);
        creep
    }

    /// The stage's whole screen in `area`, and what plays slid out from its `edge` as the panel shows it there (as
    /// large as on the screen, the screen made smaller with it), as far as `out` (1 all the way, 0 just past the edge).
    /// Returns how soon to draw again, and where the screen was drawn.
    pub(super) fn music_on_stage(&mut self, frame: &Frame, area: Rect, edge: Edge, out: f32) -> (Option<f32>, Rect) {
        let (_, r) = self.panel_on_stage(frame, area, None, None, true, 1.0);
        let k = r.w / self.stage.size.0;
        let shows = super::super::music::Shows::from(&self.settings.view);
        let room = self.settings.music_rooms.at(edge == Edge::Top);
        // (As large as it rests at: what plays is not made larger with the panel's size, as the readings are.)
        let size = super::super::music::rests_at(room, &shows);
        let (w, h) = size;
        let (sw, sh) = self.stage.size;
        let inset = 8.0;
        let away = 1.0 - out;
        let (x, y) = match edge {
            Edge::Left => (inset - away * (w + inset + GAP), (sh - h) / 2.0),
            Edge::Right => (sw - inset - w + away * (w + inset + GAP), (sh - h) / 2.0),
            Edge::Top => ((sw - w) / 2.0, inset - away * (h + inset + GAP)),
        };
        unsafe { frame.dc.PushAxisAlignedClip(&rect(r.x, r.y, r.w, r.h), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
        let local = Matrix3x2::scale(k, k) * Matrix3x2::translation(r.x + x * k, r.y + y * k);
        let (scale, now) = (self.scale * k, Instant::now());
        // In the theme the desktop under it gives, and on the skin, the skin's ground under it (the stage's desktop
        // behind it, made a pixel a DIP: see `panel_on_stage`).
        let behind = RECT { left: x as i32, top: y as i32, right: (x + w) as i32, bottom: (y + h) as i32 };
        let (theme, frost) = self.stage.look(Skin::named(&self.settings.skin), self.prefs.theme, behind);
        let ground = super::super::music::Ground::of(&shows, theme.dark);
        if let super::super::music::Ground::Skin { .. } = ground {
            frame.origin(0.0, 0.0);
            self.stage.music_ground(frame, &mut self.music_ground, &theme, frost, size, Some(((x, y), 1.0)), local, scale);
        }
        frame.place(local);
        let family = theme.body.family;
        let painted = if super::super::music::heard() {
            self.music.paint(frame, size, &shows, ground, family, scale, None, now)
        } else {
            self.music.paint_example(frame, size, &shows, ground, family, scale, None, now, self.example_from.elapsed().as_secs_f64())
        };
        frame.origin(0.0, 0.0);
        unsafe { frame.dc.PopAxisAlignedClip() };
        ((painted.busy || !super::super::music::heard()).then_some(MUSIC_PREVIEW_PACE), r)
    }

    /// The edges of the screen `screen` (`monitors[index]`), drawn at `r` (`along` placing a physical pixel along an
    /// edge on it): each edge's line (solid where it is pushed into, dashed where another screen lies against it; in
    /// the accent where it opens anything), and the component it holds in a pill at its middle; each the zone it takes
    /// a drop in.
    #[allow(clippy::too_many_arguments)]
    fn paint_zones(&mut self, frame: &Frame, palette: &Palette, places: &Places, screen: &str, monitors: &[RECT], index: usize, r: Rect, along: &dyn Fn(i32, bool) -> f32, large: bool, hovered: &Option<Target>) {
        let eased = crate::screens::eased();
        let deep = if large { ZONE_LARGE } else { ZONE_SMALL.min(r.w / 4.0) };
        let deep_top = if large { ZONE_TOP_LARGE } else { ZONE_SMALL.min(r.h / 4.0) };
        let carrying = self.carry.as_ref().is_some_and(|carry| carry.moved);
        let pale = Color::hex(0xFFFFFF, 0.55);
        let now = Instant::now();
        for edge in EDGES {
            let upright = edge != Edge::Top;
            let zone = zone_of(r, edge, deep, deep_top);
            let part = places.part(screen, edge);
            let over = carrying && self.hand().is_some_and(|(x, y)| x >= zone.x && x < zone.x + zone.w && y >= zone.y && y < zone.y + zone.h);
            // While a component is carried, every edge glows where it takes it, the one under the pointer more, each
            // easing to how it is; and the screen pointed at, large, shows what its edge would open lighter.
            let glow = self.animate(format!("glow:{screen}:{edge:?}:{large}"), if over { GLOW.1 } else if carrying { GLOW.0 } else { 0.0 }, SLIDE, now, false);
            if glow > 0.0 {
                edge_glow(frame, r, edge, if large { GLOW_LARGE } else { GLOW_SMALL }.min(if upright { r.w } else { r.h } / 3.0), palette.selection, glow);
            } else if large && hovered.as_ref() == Some(&Target::Zone(screen.to_string(), edge)) {
                edge_glow(frame, r, edge, GLOW_LARGE.min(if upright { r.w } else { r.h } / 3.0), Color::hex(0xFFFFFF, 1.0), 0.22);
            }
            // The line along the edge, stretch by stretch: in the accent while one is carried (thicker where it would
            // go), else where the edge opens anything.
            let colour = if part.is_some() || carrying { palette.selection } else { pale };
            let line = EDGE_LINE + (EDGE_LINE_OVER - EDGE_LINE) * ((glow - GLOW.0) / (GLOW.1 - GLOW.0)).clamp(0.0, 1.0);
            // (Each kept clear of the corners, and of the line along the top: they meet none.)
            let corner = 2.0 + EDGE_LINE + MAP_RADIUS * 2.0;
            let (from, to) = if upright { (r.y + corner, r.y + r.h - corner) } else { (r.x + corner, r.x + r.w - corner) };
            for (a, b, seam) in crate::screens::stretches(monitors, index, edge, eased) {
                let (a, b) = (along(a, upright).max(from), along(b, upright).min(to));
                if b <= a {
                    continue;
                }
                let bar = |a: f32, b: f32| match edge {
                    Edge::Left => fill(frame, colour, r.x + 2.0, a, line, b - a, line / 2.0),
                    Edge::Right => fill(frame, colour, r.x + r.w - 2.0 - line, a, line, b - a, line / 2.0),
                    Edge::Top => fill(frame, colour, a, r.y + 2.0, b - a, line, line / 2.0),
                };
                if seam {
                    let count = ((b - a + MAP_GAP) / (MAP_DASH + MAP_GAP)).floor().max(1.0);
                    let dash = ((b - a) - (count - 1.0) * MAP_GAP) / count;
                    for i in 0..count as usize {
                        let at = a + i as f32 * (dash + MAP_GAP);
                        bar(at, at + dash);
                    }
                } else {
                    bar(a, b);
                }
            }
            // What it holds, in a pill at its middle; none while its panel shows there, slid out over where it would
            // be (pressed again, the edge puts it away). While one carried is over it, what would take its place:
            // that one's pill, or on one screen large its panel (see `paint_one_screen`), not a pill.
            let showing = large && !carrying && self.shown_edge == Some(edge);
            let carried = self.carry.as_ref().filter(|carry| carry.moved).map(|carry| carry.part);
            let held = if over { carried.filter(|_| !large) } else { part.filter(|_| !showing) };
            if let Some(part) = held {
                let rect = if large { pill_at(r, zone, edge, (self.chip_width(frame, part), CHIP), PILL_IN) } else { small_pill_at(r, zone, edge, marks_size(r)) };
                let lifted = self.carry.as_ref().is_some_and(|carry| carry.moved && carry.from.as_ref().is_some_and(|(s, e)| s == screen && *e == edge));
                let pointed = hovered.as_ref().is_some_and(|target| matches!(target, Target::Chip(s, e, _) | Target::Unplace(s, e) if s == screen && *e == edge));
                self.chip(frame, palette, part, rect, large, if lifted && !over { 0.35 } else { 1.0 }, large.then_some(pointed && !carrying));
                // What takes a press: the pill's button first, then the pill.
                if large && !over {
                    self.targets.push((Rect { x: rect.x + rect.w - CHIP, y: rect.y, w: CHIP, h: rect.h }, Target::Unplace(screen.to_string(), edge)));
                }
                self.targets.push((rect, Target::Chip(screen.to_string(), edge, part)));
            }
            if large {
                self.targets.push((zone, Target::Zone(screen.to_string(), edge)));
            }
            self.zones.push(Zone { screen: screen.to_string(), edge, rect: zone });
        }
    }

    /// How wide a component's pill is drawn large: its icon and name, and the button that takes it off.
    fn chip_width(&self, frame: &Frame, part: Part) -> f32 {
        8.0 + 16.0 + 6.0 + frame.gfx.measure(part_name(part, self.lang), Font::new(Family::Segoe, 13.0, 600.0)) + 8.0 + CHIP - 6.0
    }

    /// A component's pill at `r`, as opaque as `opacity`: frosted as the screens' numbers are, its icon in the
    /// window's ink (and, large, its name, and where `button` says, the button that takes it off, lit while the
    /// pointer is on the pill). (Which edges hold one, their lines say, in the accent.)
    #[allow(clippy::too_many_arguments)]
    fn chip(&self, frame: &Frame, palette: &Palette, part: Part, r: Rect, large: bool, opacity: f32, button: Option<bool>) {
        let ink = palette.text.alpha(opacity);
        fill(frame, Color::hex(0x000000, 0.14 * opacity), r.x, r.y + 1.0, r.w, r.h, r.h / 2.0);
        fill(frame, palette.window.alpha(0.92 * opacity), r.x, r.y, r.w, r.h, r.h / 2.0);
        if !large {
            frame.icon(part_icon(part), Point { x: r.x + r.w / 2.0, y: r.y + r.h / 2.0 }, 12.0 * r.h / CHIP_SMALL, ink);
            return;
        }
        frame.icon(part_icon(part), Point { x: r.x + 8.0 + 8.0, y: r.y + r.h / 2.0 }, 14.0, ink);
        let font = Font::new(Family::Segoe, 13.0, 600.0);
        let name = part_name(part, self.lang);
        text_centred(frame, name, font, ink, r.x + 30.0, r.y + r.h / 2.0, frame.gfx.measure(name, font) + 4.0, Align::Start);
        let Some(pointed) = button else { return };
        let (cx, cy) = (r.x + r.w - CHIP / 2.0, r.y + r.h / 2.0);
        fill(frame, palette.text.alpha(if pointed { 0.16 } else { 0.08 } * opacity), cx - 8.0, cy - 8.0, 16.0, 16.0, 8.0);
        frame.icon(Icon::Close, Point { x: cx, y: cy }, 10.0, ink);
    }

    /// A press on a component (in the library, or on an edge): carried from there once the pointer moves.
    pub(super) fn take_up(&mut self, target: &Target, x: f32, y: f32) -> bool {
        let (part, from) = match target {
            Target::Library(part) => (*part, None),
            Target::Chip(screen, edge, part) => (*part, Some((screen.clone(), *edge))),
            _ => return false,
        };
        self.carry = Some(Carry { part, from, start: (x, y), moved: false });
        unsafe { SetCapture(self.hwnd) };
        true
    }

    /// The pointer moved while a component is in hand.
    pub(super) fn carry_to(&mut self, x: f32, y: f32) {
        if let Some(carry) = &mut self.carry {
            if !carry.moved && ((x - carry.start.0).powi(2) + (y - carry.start.1).powi(2)).sqrt() >= CARRY_FROM {
                carry.moved = true;
            }
        }
    }

    /// The component in hand let go: on an edge, put there (taken off the one it came from); off every edge, taken off
    /// the one it came from. Not carried (a press and no more): a pill pressed shows the panel its edge opens.
    pub(super) fn put_down(&mut self) {
        let Some(carry) = self.carry.take() else { return };
        unsafe {
            let _ = ReleaseCapture();
        }
        self.pressed = None;
        if !carry.moved {
            if let Some((screen, edge)) = carry.from {
                self.try_edge(screen, edge);
            }
            return;
        }
        if self.put(carry, self.pointer) {
            self.save();
            self.guide_saw_layout();
        }
    }

    /// `carry` let go at `at` (window DIPs): on an edge, put there (taken off the one it came from), the panel slid
    /// out from it; off every edge, taken off the one it came from. Whether the edges changed (saved by the caller).
    pub(super) fn put(&mut self, carry: Carry, at: Option<(f32, f32)>) -> bool {
        let zone = at.and_then(|(x, y)| self.zones.iter().find(|zone| x >= zone.rect.x && x < zone.rect.x + zone.rect.w && y >= zone.rect.y && y < zone.rect.y + zone.rect.h));
        let dropped = zone.map(|zone| (zone.screen.clone(), zone.edge));
        let places = self.settings.places_mut();
        match (dropped, &carry.from) {
            (Some((screen, edge)), from) => {
                places.put(&screen, edge, carry.part, from.as_ref().map(|(s, e)| (s.as_str(), *e)));
                self.shown_edge = Some(edge);
            }
            (None, Some((screen, edge))) => places.take(screen, *edge),
            (None, None) => return false,
        }
        self.next_frame = Instant::now();
        true
    }

    /// Where what is carried is: at the pointer, or, while the guide plays its step here, at the pointer it moves.
    pub(super) fn hand(&self) -> Option<(f32, f32)> {
        self.played_hand().or(self.pointer)
    }

    /// The editor shows screen `screen` large, the real panel slid out from its `edge`.
    fn try_edge(&mut self, screen: String, edge: Edge) {
        self.view = Some(screen);
        self.shown_edge = Some(edge);
        self.next_frame = Instant::now();
    }

    /// Does what a press on the editor does; whether it was one of its.
    pub(super) fn press_editor(&mut self, target: &Target) -> bool {
        match target {
            Target::View(view) => {
                self.view = view.clone();
                self.shown_edge = None;
            }
            Target::Same => {
                let screen = self.editor_screen().or_else(|| self.stage.screen.clone()).unwrap_or_default();
                let same = !self.settings.places().same;
                self.settings.places_mut().set_same(same, &screen);
                self.save();
            }
            // (Pressed again, its panel is put away.)
            Target::Zone(_, edge) if self.shown_edge == Some(*edge) => self.shown_edge = None,
            Target::Zone(screen, edge) => self.try_edge(screen.clone(), *edge),
            Target::Unplace(screen, edge) => {
                self.settings.places_mut().take(screen, *edge);
                self.save();
            }
            _ => return false,
        }
        self.next_frame = Instant::now();
        true
    }

    /// The editor's focus stops: its head's buttons, every screen alike, and each component on an edge (its button).
    pub(super) fn editor_stops(&self) -> Vec<Target> {
        let mut stops = Vec::new();
        if self.screens.len() > 1 {
            stops.push(Target::View(None));
            stops.extend(self.screens.iter().map(|screen| Target::View(Some(screen.id.clone()))));
            stops.push(Target::Same);
        }
        if let Some(screen) = self.editor_screen() {
            let places = self.settings.places();
            for edge in EDGES {
                stops.push(Target::Zone(screen.clone(), edge));
                if places.part(&screen, edge).is_some() {
                    stops.push(Target::Unplace(screen.clone(), edge));
                }
            }
        }
        stops
    }
}

/// How far in from its edge a pill is drawn on a screen, and how large a small screen's number is.
const PILL_IN: f32 = 10.0;
const NUMBER: f32 = 26.0;
const NUMBER_IN: f32 = 6.0;

/// A pill `size` (wide, high) on edge `edge` of a screen drawn at `r`, `inside` in from it, at the middle of the edge's
/// zone `zone`.
fn pill_at(r: Rect, zone: Rect, edge: Edge, (w, h): (f32, f32), inside: f32) -> Rect {
    match edge {
        Edge::Left => Rect { x: r.x + inside, y: zone.y + (zone.h - h) / 2.0, w, h },
        Edge::Right => Rect { x: r.x + r.w - inside - w, y: zone.y + (zone.h - h) / 2.0, w, h },
        Edge::Top => Rect { x: zone.x + (zone.w - w) / 2.0, y: r.y + inside, w, h },
    }
}

/// A small screen's pill on edge `edge` (see `pill_at`), its marks drawn `size` as large as they are.
fn small_pill_at(r: Rect, zone: Rect, edge: Edge, size: f32) -> Rect {
    pill_at(r, zone, edge, (CHIP_SMALL * size, CHIP_SMALL * size), PILL_IN * size)
}

/// Where a small screen's number is drawn, its marks `size` as large as they are: at its foot, in its middle.
fn number_at(r: Rect, size: f32) -> Rect {
    let side = NUMBER * size;
    Rect { x: r.x + (r.w - side) / 2.0, y: r.y + r.h - NUMBER_IN * size - side, w: side, h: side }
}

/// How large a small screen drawn at `r` draws its marks (a pill on each edge that holds one, and its number): as large
/// as they are, or as much smaller as keeps them all apart (a pill on each edge counted, so that they keep their size
/// as one is put on an edge or taken off), down to half.
fn marks_size(r: Rect) -> f32 {
    let deep = ZONE_SMALL.min(r.w / 4.0);
    let deep_top = ZONE_SMALL.min(r.h / 4.0);
    let apart = |size: f32| {
        let mut marks: Vec<Rect> = EDGES.iter().map(|&edge| small_pill_at(r, zone_of(r, edge, deep, deep_top), edge, size)).collect();
        marks.push(number_at(r, size));
        let gap = 2.0 * size;
        let meet = |a: &Rect, b: &Rect| a.x < b.x + b.w + gap && b.x < a.x + a.w + gap && a.y < b.y + b.h + gap && b.y < a.y + a.h + gap;
        marks.iter().enumerate().all(|(i, a)| marks[i + 1..].iter().all(|b| !meet(a, b)))
    };
    (0..=10).map(|step| 1.0 - step as f32 * 0.05).find(|&size| apart(size)).unwrap_or(0.5)
}

/// Where edge `edge` of a screen drawn at `r` takes a drop: `deep` into it from a side, `deep_top` from its top
/// (the top's between the sides'), each at most a third of the screen (one drawn narrow, standing up, keeps a top
/// between its sides).
fn zone_of(r: Rect, edge: Edge, deep: f32, deep_top: f32) -> Rect {
    let (deep, deep_top) = (deep.min(r.w / 3.0), deep_top.min(r.h / 3.0));
    match edge {
        Edge::Left => Rect { x: r.x, y: r.y, w: deep, h: r.h },
        Edge::Right => Rect { x: r.x + r.w - deep, y: r.y, w: deep, h: r.h },
        Edge::Top => Rect { x: r.x + deep, y: r.y, w: r.w - 2.0 * deep, h: deep_top },
    }
}

/// A glow along edge `edge` of a screen drawn at `r`, `color` at `strength` on the edge fading to nothing `reach`
/// into the screen, inside the screen's rounded corners.
fn edge_glow(frame: &Frame, r: Rect, edge: Edge, reach: f32, color: Color, strength: f32) {
    let dc = &frame.dc;
    let (start, end) = match edge {
        Edge::Left => ((r.x, r.y), (r.x + reach, r.y)),
        Edge::Right => ((r.x + r.w, r.y), (r.x + r.w - reach, r.y)),
        Edge::Top => ((r.x, r.y), (r.x, r.y + reach)),
    };
    let band = match edge {
        Edge::Left => Rect { x: r.x, y: r.y, w: reach, h: r.h },
        Edge::Right => Rect { x: r.x + r.w - reach, y: r.y, w: reach, h: r.h },
        Edge::Top => Rect { x: r.x, y: r.y, w: r.w, h: reach },
    };
    // (Eased off, not straight: brightest along the edge itself.)
    let stops = [
        D2D1_GRADIENT_STOP { position: 0.0, color: color.alpha(color.a * strength).d2d() },
        D2D1_GRADIENT_STOP { position: 0.35, color: color.alpha(color.a * strength * 0.45).d2d() },
        D2D1_GRADIENT_STOP { position: 1.0, color: color.alpha(0.0).d2d() },
    ];
    unsafe {
        let Ok(stops) = ID2D1RenderTarget::CreateGradientStopCollection(dc, &stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) else { return };
        let line = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: Vector2 { X: start.0, Y: start.1 }, endPoint: Vector2 { X: end.0, Y: end.1 } };
        let Ok(brush) = dc.CreateLinearGradientBrush(&line, None, &stops) else { return };
        let Ok(mask) = frame.gfx.factory.CreateRoundedRectangleGeometry(&rounded(r, MAP_RADIUS * 2.0)) else { return };
        let layer = D2D1_LAYER_PARAMETERS1 {
            contentBounds: rect(band.x, band.y, band.w, band.h),
            geometricMask: std::mem::ManuallyDrop::new(windows::core::Interface::cast(&mask).ok()),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: Matrix3x2::identity(),
            opacity: 1.0,
            ..Default::default()
        };
        dc.PushLayer(&layer, None);
        dc.FillRectangle(&rect(band.x, band.y, band.w, band.h), &brush);
        dc.PopLayer();
        drop(std::mem::ManuallyDrop::into_inner(layer.geometricMask));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small screen's marks, a pill on each edge and its number, never meet, however small the screen is drawn: on one
    /// drawn large enough they keep their size, on a smaller one they are drawn smaller.
    #[test]
    fn a_small_screen_s_marks_never_meet() {
        for r in [Rect { x: 0.0, y: 0.0, w: 300.0, h: 170.0 }, Rect { x: 5.0, y: 8.0, w: 120.0, h: 68.0 }, Rect { x: 0.0, y: 0.0, w: 90.0, h: 50.0 }, Rect { x: 0.0, y: 0.0, w: 60.0, h: 107.0 }] {
            let size = marks_size(r);
            let (deep, deep_top) = (ZONE_SMALL.min(r.w / 4.0), ZONE_SMALL.min(r.h / 4.0));
            let mut marks: Vec<Rect> = EDGES.iter().map(|&edge| small_pill_at(r, zone_of(r, edge, deep, deep_top), edge, size)).collect();
            marks.push(number_at(r, size));
            for (at, a) in marks.iter().enumerate() {
                assert!(a.x >= r.x && a.y >= r.y && a.x + a.w <= r.x + r.w && a.y + a.h <= r.y + r.h, "{a:?} out of {r:?}");
                for b in &marks[at + 1..] {
                    assert!(a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y, "{a:?} meets {b:?} on {r:?} at {size}");
                }
            }
        }
        assert_eq!(marks_size(Rect { x: 0.0, y: 0.0, w: 300.0, h: 170.0 }), 1.0);
        assert!(marks_size(Rect { x: 0.0, y: 0.0, w: 90.0, h: 50.0 }) < 1.0);
    }

    /// Each edge of a screen drawn takes a drop in a zone of its own, however narrow the screen is drawn: a screen
    /// standing up keeps a top between its sides, none of them meeting.
    #[test]
    fn every_edge_takes_a_drop_however_narrow() {
        for r in [Rect { x: 0.0, y: 0.0, w: 400.0, h: 225.0 }, Rect { x: 10.0, y: 20.0, w: 150.0, h: 266.0 }, Rect { x: 0.0, y: 0.0, w: 60.0, h: 100.0 }] {
            let zones = EDGES.map(|edge| zone_of(r, edge, ZONE_LARGE, ZONE_TOP_LARGE));
            for (edge, zone) in EDGES.iter().zip(&zones) {
                assert!(zone.w > 0.0 && zone.h > 0.0, "{edge:?} of {r:?}: {zone:?}");
            }
            for (at, a) in zones.iter().enumerate() {
                for b in &zones[at + 1..] {
                    assert!(a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y, "{a:?} meets {b:?}");
                }
            }
        }
    }
}
