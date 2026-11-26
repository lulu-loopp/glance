//! The folders the storage component tells the sizes of (see `crate::folders`): a row each under the drives, with its
//! name and size (and, of those that bear the same name, the folder it is in), pressed to open it; and the card beside
//! what they are drawn in, of the one the pointer rests on (see `ui::hover_card`): where it is, how many files it
//! holds, how its size changed lately and what in it is largest.

use super::super::canvas::{Align, Canvas, Color, Fill, Font, Point};
use super::super::icons::Icon;
use super::super::text::{self, Lang};
use super::super::theme::{Ink, Skin, Theme};
use super::{fitted, name_shown, plate_round, rounded_outline, text_on_line, Block, Hit, HitBox, Lane, Rect, Scene, FACT_GAP, GLASS_ROW_ROUND, HEAD, LINE, PLOT};
use crate::folders::{Change, Folder};
use crate::todo::Day;

/// A row's height.
const ROW: f32 = 26.0;
/// Over the rows, between the drives and them: the line across, the room over and under it.
const DIVIDER: (f32, f32) = (9.0, 5.0);
/// How far a row's plate reaches past the lane's words, at each end.
const REACH: f32 = 6.0;
/// A row's folder, and the room before its name.
const PICTURE: f32 = 15.0;
const PICTURE_GAP: f32 = 8.0;

/// A folder's row: the number a press on it names it by, its name, the folder it is in where another bears its name,
/// and how large what it holds is (said, where that is not known).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(super) struct Row {
    number: u64,
    name: String,
    within: Option<String>,
    value: String,
    known: bool,
    /// The card tells of it (see `card`): its name whole there.
    told: bool,
}

/// The rows, as the folders are told now.
pub(super) fn rows(lang: Lang) -> Vec<Row> {
    let folders = crate::folders::shown();
    folders
        .iter()
        .map(|folder| {
            let same: Vec<&str> = folders.iter().filter(|other| other.name == folder.name).map(|other| other.path.as_str()).collect();
            // (At a drive's top, the drive it is on.)
            let within = if same.len() > 1 { crate::station::apps::apart_by_folder(&folder.path, &same).or_else(|| folder.path.split_once('\\').map(|(drive, _)| drive.to_string())) } else { None };
            let (value, known) = match (&folder.held, folder.away) {
                (_, true) => (lang.pick("不可用", "Unavailable").into(), false),
                (Some(held), false) => (text::bytes(held.bytes as f64), true),
                (None, false) => (lang.pick("正在统计", "Counting").into(), false),
            };
            Row { number: crate::folders::number_of(&folder.path), name: folder.name.clone(), within, value, known, told: folder.held.is_some() || folder.away }
        })
        .collect()
}

/// How tall the rows are, `divided` from the drives over them.
pub(super) fn height(rows: &[Row], divided: bool) -> f32 {
    if rows.is_empty() {
        return 0.0;
    }
    rows.len() as f32 * ROW + if divided { DIVIDER.0 + 1.0 + DIVIDER.1 } else { 0.0 }
}

/// What the lane's head says at its end while a folder is dragged over it.
pub(super) fn dropping(lang: Lang) -> String {
    if crate::folders::shown().len() < crate::folders::MOST {
        lang.pick("松开以添加文件夹", "Release to add").into()
    } else {
        match lang {
            Lang::Zh => format!("最多添加 {} 个文件夹", crate::folders::MOST),
            Lang::En => format!("{} folders at most", crate::folders::MOST),
        }
    }
}

/// A folder's picture's colour: amber, as Windows' own are, but chart paper's ink.
fn picture_ink(theme: &Theme) -> Color {
    match (theme.skin, theme.dark) {
        (Skin::Paper, _) => theme.text2,
        (_, true) => Color::hex(0xFFD77A, 1.0),
        (_, false) => Color::hex(0xC98A0C, 1.0),
    }
}

/// The rows from (`left`, `top`), `width` across, in a lane all of `area` (where a folder may be let go, outlined while
/// one is dragged `over` it).
pub(super) fn paint(frame: &dyn Canvas, scene: &Scene, (left, top, width): (f32, f32, f32), area: Rect, rows: &[Row], (divided, over): (bool, bool), hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    let mut y = top;
    if divided && !rows.is_empty() {
        frame.key("storage.folders.rule");
        frame.fill(theme.rule, left, y + DIVIDER.0, width, 1.0);
        y += DIVIDER.0 + 1.0 + DIVIDER.1;
    }
    for row in rows {
        let hit = Hit::Folder(row.number);
        let lit = scene.hover == Some(hit);
        frame.key(&format!("storage.folder.{}", row.number));
        if lit {
            frame.fill_rounded(theme.hover, left - REACH, y, width + 2.0 * REACH, ROW, plate_round(theme, GLASS_ROW_ROUND, 0.0));
        }
        let middle = y + ROW / 2.0;
        frame.icon(Icon::Folder, Point { x: left + PICTURE / 2.0, y: middle }, PICTURE, picture_ink(theme));
        let figures = Font { weight: 600.0, ..theme.small };
        let font = if row.known { figures } else { theme.small };
        let value_width = frame.measure(&row.value, font);
        frame.text(&row.value, font, if row.known { theme.text } else { theme.text3 }, left, frame.line_in(font, y, ROW), width, Align::End);
        // Its name, and after it the folder it is in, each shortened where there is no room for both.
        let name_left = left + PICTURE + PICTURE_GAP;
        let room = left + width - value_width - 10.0 - name_left;
        let line = frame.line_in(theme.small, y, ROW);
        let within = row.within.as_deref().map(|within| (within, frame.measure(within, theme.small)));
        let name_room = within.map_or(room, |(_, wide)| (room - 6.0 - wide.min(room * 0.4)).max(0.0));
        let name = fitted(frame, &row.name, theme.small, name_room);
        frame.text(&name, theme.small, if lit { theme.text } else { theme.text2 }, name_left, line, name_room, Align::Start);
        if let Some((within, _)) = within {
            let from = frame.measure(&name, theme.small) + 6.0;
            let shown = fitted(frame, within, theme.small, room - from);
            frame.text(&shown, theme.small, theme.text3, name_left + from, line, room - from, Align::Start);
        }
        hits.push((left - REACH, y, width + 2.0 * REACH, ROW, hit));
        if !row.told {
            name_shown(hits, hit, &row.name, &name);
        }
        y += ROW;
    }
    if over {
        frame.key("storage.folders.over");
        let accent = if theme.skin == Skin::Paper { theme.text } else { super::super::theme::accent(theme.dark) };
        let (inset, round) = (3.0, (theme.radius - 3.0).max(2.0));
        let outline = rounded_outline(area.x + inset, area.y + inset, area.w - 2.0 * inset, area.h - 2.0 * inset, round);
        frame.stroke_dashed(&outline, accent, 1.5);
    }
    hits.push((area.x, area.y, area.w, area.h, Hit::FoldersArea));
}

/// What the card of a folder tells: its name and path, and what it holds, or why it tells none of that.
#[derive(Clone, PartialEq, Debug)]
pub(super) struct Card {
    name: String,
    path: String,
    told: Result<Told, String>,
}

/// What a folder holds as its card tells it: how large (its figure, its unit), how many files, how that changed, its
/// size on the days told of (each where along the time and how far up of them, 0–1), and what in it is largest.
#[derive(Clone, PartialEq, Debug)]
struct Told {
    figure: String,
    unit: String,
    files: String,
    change: String,
    days: Vec<(f32, f32)>,
    largest: Vec<(String, String)>,
}

impl std::hash::Hash for Card {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (&self.name, &self.path).hash(state);
        match &self.told {
            Ok(told) => {
                (&told.figure, &told.unit, &told.files, &told.change, &told.largest).hash(state);
                told.days.iter().for_each(|(x, y)| (x.to_bits(), y.to_bits()).hash(state));
            }
            Err(said) => said.hash(state),
        }
    }
}

/// The card of the folder `number` stands for: none where it is told of no more, or not counted yet.
pub fn card(scene: &Scene, number: u64) -> Option<Lane> {
    let folder = crate::folders::numbered(number)?;
    let card = card_of(&folder, crate::folders::today(), scene.lang)?;
    Some(Lane { id: "storage".into(), blocks: vec![Block::Folder(card)], ink: scene.theme.ink("storage") })
}

fn card_of(folder: &Folder, today: i64, lang: Lang) -> Option<Card> {
    let told = if folder.away {
        Err(lang.pick("找不到此文件夹，或此位置不支持统计。", "Can't find this folder, or sizes can't be measured here.").to_string())
    } else {
        let held = folder.held.as_ref()?;
        let whole = text::bytes(held.bytes as f64);
        let (figure, unit) = whole.split_once(' ').unwrap_or((&whole, ""));
        let files = super::ai::grouped(held.files);
        Ok(Told {
            figure: figure.into(),
            unit: unit.into(),
            files: match lang {
                Lang::Zh => format!("{files} 个文件"),
                Lang::En => format!("{files} {}", if held.files == 1 { "file" } else { "files" }),
            },
            change: changed(folder.change(today), lang),
            days: along(folder, today),
            largest: held.largest.iter().map(|(name, bytes)| (name.clone(), text::bytes(*bytes as f64))).collect(),
        })
    };
    Some(Card { name: folder.name.clone(), path: folder.path.clone(), told })
}

/// How a folder's size changed, said.
fn changed(change: Change, lang: Lang) -> String {
    let by = |by: i64| {
        let size = text::bytes(by.unsigned_abs() as f64);
        match (by.signum(), lang) {
            (0, Lang::Zh) => "没有变化".to_string(),
            (0, Lang::En) => "No change".to_string(),
            (1, Lang::Zh) => format!("增加 {size}"),
            (1, Lang::En) => format!("Up {size}"),
            (_, Lang::Zh) => format!("减少 {size}"),
            (_, Lang::En) => format!("Down {size}"),
        }
    };
    match (change, lang) {
        (Change::Begun, _) => lang.pick("从今天开始记录", "Recording from today").into(),
        (Change::Week(to), Lang::Zh) => format!("近 7 天{}", by(to)),
        (Change::Week(to), Lang::En) => format!("{} in 7 days", by(to)),
        (Change::Since(day, to), Lang::Zh) => {
            let day = Day::from_number(day);
            format!("自 {} 月 {} 日起{}", day.month, day.day, by(to))
        }
        (Change::Since(day, to), Lang::En) => {
            const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
            let day = Day::from_number(day);
            format!("{} since {} {}", by(to), MONTHS[day.month as usize - 1], day.day)
        }
    }
}

/// A folder's size on the days told of and now, each where it is along that time and how far up of the least to the
/// most of them (in the middle where it was always the same): none on the day it was added.
fn along(folder: &Folder, today: i64) -> Vec<(f32, f32)> {
    let Some(held) = folder.held.as_ref().filter(|_| folder.change(today) != Change::Begun) else { return Vec::new() };
    // (The last day before the week stands at its start; now, at its end.)
    let first = folder.days.first().map_or(today, |(day, _)| (*day).max(today - 7));
    let sizes: Vec<(i64, u64)> = folder.days.iter().map(|&(day, bytes)| (day.max(first), bytes)).chain([(today + 1, held.bytes)]).collect();
    let (least, most) = sizes.iter().fold((u64::MAX, 0), |(least, most), (_, bytes)| (least.min(*bytes), most.max(*bytes)));
    let up = |bytes: u64| if most > least { (bytes - least) as f32 / (most - least) as f32 } else { 0.5 };
    sizes.iter().map(|&(day, bytes)| ((day - first) as f32 / (today + 1 - first) as f32, up(bytes))).collect()
}

/// Over what the card tells, under its path; how tall its curve is; over what is largest.
const PART: f32 = 8.0;
const CURVE: f32 = 44.0;
const LARGEST: f32 = 10.0;

impl Card {
    pub(super) fn height(&self) -> f32 {
        let head = HEAD + 2.0 + LINE;
        match &self.told {
            Err(_) => head + PART + LINE,
            Ok(told) => {
                let curve = if told.days.is_empty() { 0.0 } else { PART + CURVE };
                let largest = if told.largest.is_empty() { 0.0 } else { LARGEST + LINE + told.largest.len() as f32 * (LINE + FACT_GAP) };
                head + PART + PLOT + curve + 6.0 + LINE + largest
            }
        }
    }

    pub(super) fn paint(&self, frame: &dyn Canvas, scene: &Scene, ink: Ink, (left, top, width): (f32, f32, f32)) {
        let theme = scene.theme;
        frame.key("storage.card.name");
        frame.text(&fitted(frame, &self.name, theme.title, width), theme.title, theme.text, left, top + (HEAD - theme.title.size) / 2.0 - 1.0, width, Align::Start);
        let mut y = top + HEAD + 2.0;
        frame.key("storage.card.path");
        frame.text(&fitted(frame, &self.path, theme.small, width), theme.small, theme.text3, left, y, width, Align::Start);
        y += LINE + PART;
        let told = match &self.told {
            Ok(told) => told,
            Err(said) => {
                frame.key("storage.card.said");
                return frame.text(said, theme.small, theme.text2, left, y, width, Align::Start);
            }
        };
        // How large it is, its unit and its files on the figure's line.
        frame.key("storage.card.value");
        let bottom = y + PLOT;
        let figure_width = frame.measure(&told.figure, theme.figure);
        text_on_line(frame, &told.figure, theme.figure, theme.text, left, bottom, theme.figure.size * theme.figure_line, width);
        let unit_left = left + figure_width + 3.0;
        frame.key("storage.card.unit");
        text_on_line(frame, &told.unit, theme.unit, theme.text2, unit_left, bottom, theme.unit.size, width);
        let files_left = unit_left + frame.measure(&told.unit, theme.unit) + 12.0;
        frame.key("storage.card.files");
        text_on_line(frame, &told.files, theme.small, theme.text2, files_left, bottom, theme.small.size, left + width - files_left);
        y = bottom;
        if !told.days.is_empty() {
            frame.key("storage.card.curve");
            let (high, foot) = (y + PART + 5.0, y + PART + CURVE - 5.0);
            let points: Vec<Point> = told.days.iter().map(|&(x, up)| Point { x: left + x * (width - 4.0), y: foot - up * (foot - high) }).collect();
            let ground = y + PART + CURVE;
            let mut area = points.clone();
            area.push(Point { x: left + width - 4.0, y: ground });
            area.push(Point { x: left, y: ground });
            let wash = if ink.wash == ink.wash_end { Fill::Solid(ink.wash) } else { Fill::Down { top: high, from: ink.wash, bottom: ground, to: ink.wash_end } };
            frame.fill_shape(&area, wash);
            frame.stroke(&points, ink.trace, 1.6);
            if let Some(last) = points.last() {
                frame.fill_circle(ink.trace, *last, 2.6);
            }
            y = ground;
        }
        y += 6.0;
        frame.key("storage.card.change");
        frame.text(&told.change, theme.small, theme.text2, left, y, width, Align::Start);
        y += LINE;
        if told.largest.is_empty() {
            return;
        }
        y += LARGEST;
        frame.key("storage.card.largest");
        frame.text(scene.lang.pick("其中最大", "Largest inside"), theme.small, theme.text3, left, y, width, Align::Start);
        y += LINE;
        for (at, (name, size)) in told.largest.iter().enumerate() {
            let row = y + FACT_GAP + at as f32 * (LINE + FACT_GAP);
            frame.key(&format!("storage.card.{at}"));
            let room = width - frame.measure(size, theme.small) - 12.0;
            frame.text(&fitted(frame, name, theme.small, room), theme.small, theme.text, left, row, room, Align::Start);
            frame.text(size, theme.small, theme.text2, left, row, width, Align::End);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::folders::Held;

    const GB: u64 = 1 << 30;

    fn folder(bytes: u64, days: &[(i64, u64)]) -> Folder {
        Folder { path: r"D:\a".into(), name: "a".into(), held: Some(Held { bytes, files: 2914, largest: vec![("b".into(), GB)] }), away: false, days: days.to_vec() }
    }

    /// A folder's change is said: up or down by how much in the week (not changed: so), since when where that is less
    /// than a week back, and from today on where it was added today.
    #[test]
    fn a_folder_s_change_is_said() {
        let said = |change: Change| (changed(change, Lang::Zh), changed(change, Lang::En));
        assert_eq!(said(Change::Begun).0, "从今天开始记录");
        assert_eq!(said(Change::Week(GB as i64 * 2)), ("近 7 天增加 2.0 GB".into(), "Up 2.0 GB in 7 days".into()));
        assert_eq!(said(Change::Week(0)), ("近 7 天没有变化".into(), "No change in 7 days".into()));
        let day = Day { year: 2026, month: 11, day: 20 }.number();
        assert_eq!(said(Change::Since(day, -(GB as i64))), ("自 11 月 20 日起减少 1.0 GB".into(), "Down 1.0 GB since Nov 20".into()));
    }

    /// The curve goes from the first day told of (the last before the week, at its start) to now, from the least to the
    /// most, and there is none on the day it was added.
    #[test]
    fn a_folder_s_curve_goes_to_now() {
        assert!(along(&folder(5, &[(100, 5)]), 100).is_empty());
        let curve = along(&folder(30, &[(60, 10), (97, 20), (100, 30)]), 100);
        assert_eq!(curve, [(0.0, 0.0), (4.0 / 8.0, 0.5), (7.0 / 8.0, 1.0), (1.0, 1.0)]);
        assert_eq!(along(&folder(10, &[(98, 10)]), 100), [(0.0, 0.5), (1.0, 0.5)]);
    }

    /// A folder not counted yet has no card, and one not there says so.
    #[test]
    fn a_folder_not_there_says_so() {
        let counting = Folder { held: None, ..folder(0, &[]) };
        assert!(card_of(&counting, 100, Lang::Zh).is_none());
        let away = Folder { away: true, ..folder(5, &[(90, 5)]) };
        assert!(card_of(&away, 100, Lang::Zh).unwrap().told.is_err());
        let told = card_of(&folder(5 * GB, &[(100, GB)]), 100, Lang::Zh).unwrap().told.unwrap();
        assert_eq!((told.figure.as_str(), told.unit.as_str(), told.files.as_str()), ("5.0", "GB", "2,914 个文件"));
    }
}
