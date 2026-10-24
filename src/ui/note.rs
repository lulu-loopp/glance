//! A note written: its words in a box the text services write (see
//! `editor`: the keys, the input method, selection, undo and the clipboard theirs), dressed as their Markdown reads,
//! kept in its file as they are typed (a moment after the last key), its to-dos ticked and its links followed where they
//! show. Written on its page beside the panel (see `beside`), or where it is stuck on the desktop, on the widget it is
//! there (see `widget`): each draws it as the other does, its face here.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VIRTUAL_KEY, VK_0, VK_1, VK_2, VK_3, VK_A, VK_B, VK_BACK, VK_C, VK_CONTROL, VK_DELETE, VK_DOWN, VK_E, VK_END, VK_APPS, VK_ESCAPE, VK_F10, VK_HOME, VK_I, VK_INSERT, VK_K, VK_LEFT, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_TAB, VK_UP, VK_V,
    VK_X, VK_Y, VK_Z,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use super::canvas::{Align, Canvas, Color, Family, Font, Inks, Point};
use super::editor::{Editor, Palette};
use super::icons::Icon;
use super::text::Lang;
use super::theme::{Skin, Theme};

mod link;

/// How high a note's words are written (DIPs).
pub const WORDS: f32 = 15.0;
/// The room a note's page leaves at its sides, round its words, its head's and its foot's, and how tall its head and its foot
/// are (DIPs).
pub const PAD: f32 = 20.0;
pub const HEAD: f32 = 52.0;
pub const FOOT: f32 = 48.0;
/// A button in a note's head (its menu, its page's close): how large, how far down, and how far in from its right edge
/// the first is (the next further in by as much again and a gap).
pub const BUTTON: f32 = 28.0;
const BUTTON_TOP: f32 = 12.0;
const BUTTON_IN: f32 = 16.0;
/// How long after the last key a note is kept.
pub const KEEP_AFTER_MS: u32 = 600;

/// `color` as Windows' GDI writes one.
fn colorref(color: Color) -> COLORREF {
    COLORREF(((color.b * 255.0).round() as u32) << 16 | ((color.g * 255.0).round() as u32) << 8 | (color.r * 255.0).round() as u32)
}

/// What a note's words are written in (`pixels` high), its colour `colour`: as its card's on the panel are (glass's in
/// the ink of its colour), and as opaque colours (the words' box takes none other: a faint ink half see through came out
/// as solid as the words, and Markdown's faint marks no fainter), as they show over its page.
pub(super) fn palette_for(theme: &Theme, colour: crate::notes::Colour, pixels: f32) -> Palette {
    let inks = word_inks(theme, colour);
    Palette { text: colorref(inks.words), faint: colorref(inks.faint), accent: colorref(inks.accent), points: pixels * 0.75, face: "Microsoft YaHei UI".into(), code_face: super::canvas::CODE_FACE.into() }
}

/// The inks a note of colour `colour` is written in on its page (see `palette_for`), opaque.
pub(super) fn word_inks(theme: &Theme, colour: crate::notes::Colour) -> Inks {
    // (The skin's own, as its head and foot are written: a card's own ink, glass's, is for its card's light pane, and
    // on a page's, lighter over a page's breadth, it was dark on dark in the dark. Its colour is the page's tint.)
    let _ = colour;
    let (text, faint) = (theme.text, theme.text3);
    let ground = page_under(theme);
    // (Each channel a whole step of 255, as the box takes its colours: else a note shown wrote its words a step apart.)
    let whole = |channel: f32| (channel * 255.0).round() / 255.0;
    let opaque = |ink: Color| {
        let ink = super::view::over(Color { a: 1.0, ..ground }, ink);
        Color { r: whole(ink.r), g: whole(ink.g), b: whole(ink.b), a: 1.0 }
    };
    let accent = match theme.skin {
        Skin::Paper => theme.text,
        _ => super::theme::accent(theme.dark),
    };
    Inks { words: opaque(text), faint: opaque(faint), accent: opaque(accent) }
}

/// What a note's page is reckoned to lie over, its inks made opaque over (the box takes colours with no see-through):
/// the skin's sheet.
pub(super) fn page_under(theme: &Theme) -> Color {
    match theme.skin {
        Skin::Paper => theme.paper,
        _ if theme.dark => Color::hex(0x26282E, 1.0),
        _ => Color::hex(0xF5F6F8, 1.0),
    }
}

/// What is said at the head of a note's page of when it was written (`when`, how long ago, or none: it is new).
fn said(when: Option<&str>, lang: Lang) -> String {
    match (when, lang) {
        (None, Lang::Zh) => "新便签".to_string(),
        (None, Lang::En) => "New note".to_string(),
        (Some(when), Lang::Zh) => format!("{when} 修改"),
        (Some(when), Lang::En) => format!("Written {when}"),
    }
}

/// What is said at the foot of a note's page of its words `text`: how many characters, kept as it is written.
fn counted(text: &str, lang: Lang) -> String {
    let count = text.chars().filter(|c| !c.is_whitespace()).count();
    match lang {
        Lang::Zh => format!("{count} 字 · 自动保存"),
        Lang::En => format!("{count} characters · kept as you write"),
    }
}

/// A note's page's face, `width` × `height` from (`x`, `y`), but for what is pressed on it and its words: its ground of
/// its colour (see `view::note_page`), at its head its colour's dot and what `said` says of when it was written, at its
/// foot, ruled off, what `counted` says of its words and its file's `name` in the room left (none: the foot left for
/// what was said of keeping it). Its words are written between (see `words_in`).
#[allow(clippy::too_many_arguments)]
fn face(frame: &dyn Canvas, theme: &Theme, colour: crate::notes::Colour, said: &str, counted: Option<&str>, name: Option<&str>, (x, y, width, height): (f32, f32, f32, f32)) {
    super::view::note_page(frame, theme, colour, (x, y, width, height));
    frame.fill_circle(super::view::note_colour(colour), Point { x: x + PAD + 5.0, y: y + 26.0 }, 5.0);
    frame.text(said, theme.small, theme.text2, x + PAD + 18.0, y + 18.0, width - 2.0 * PAD - 80.0, Align::Start);
    let bottom = y + height - FOOT;
    frame.fill(theme.rule, x + PAD, bottom, width - 2.0 * PAD, 1.0);
    let Some(counted) = counted else { return };
    frame.text(counted, theme.small, theme.text3, x + PAD, bottom + (FOOT - 16.0) / 2.0, width - 2.0 * PAD, Align::Start);
    let room = width - 2.0 * PAD - frame.measure(counted, theme.small) - 16.0;
    // (Where it fits: shortened past a few letters, a name says nothing.)
    if let Some(name) = name.filter(|_| room >= 96.0) {
        frame.text(name, theme.small, theme.text3, x + width - PAD - room, bottom + (FOOT - 16.0) / 2.0, room, Align::End);
    }
}

/// Where a note's words are written on its page `width` × `height` (DIPs, from its corner): under its head, over its
/// foot (as `words_box` places its box).
pub fn words_in(width: f32, height: f32) -> (f32, f32, f32, f32) {
    (PAD, HEAD + 4.0, width - 2.0 * PAD, height - HEAD - 4.0 - FOOT)
}

/// Where a note's words are written in a window (physical px), its page `wide` × `tall` from (`left`, `top`) in it: under
/// its head, over its foot.
pub fn words_box(left: i32, top: i32, wide: i32, tall: i32, scale: f32) -> RECT {
    RECT { left: left + (PAD * scale) as i32, top: top + ((HEAD + 4.0) * scale) as i32, right: left + wide - (PAD * scale) as i32, bottom: top + tall - (FOOT * scale) as i32 }
}

/// Where the `nth` button of a note's head is (its right edge's first) on its page `width` wide from (`x`, `y`): its left,
/// its top, its width and its height.
pub fn button_at(nth: usize, (x, y, width): (f32, f32, f32)) -> (f32, f32, f32, f32) {
    (x + width - BUTTON_IN - BUTTON - nth as f32 * (BUTTON + 4.0), y + BUTTON_TOP, BUTTON, BUTTON)
}

/// A button of a note's head at `place` (see `button_at`): its mark, on a plate where the pointer is on it (`lit`).
pub fn paint_button(frame: &dyn Canvas, theme: &Theme, place: (f32, f32, f32, f32), icon: Icon, lit: bool) {
    if lit {
        frame.fill_rounded(theme.hover, place.0, place.1, place.2, place.3, theme.control_radius.min(14.0));
    }
    frame.icon(icon, Point { x: place.0 + place.2 / 2.0, y: place.1 + place.3 / 2.0 }, 16.0, theme.text2);
}

/// A note stuck on the desktop, `width` × `height` from (`x`, `y`), not written in: its page as it is written in (see
/// `Writing::paint`) but for what is pressed there and the caret, its words as the page writes them: its words `text`, its
/// colour `colour`, written `when` (how long ago), kept in the file `name`, scrolled `scrolled` down (DIPs: see
/// `stuck_scrolls`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_stuck(frame: &dyn Canvas, theme: &Theme, lang: Lang, text: &str, colour: crate::notes::Colour, when: &str, name: &str, (x, y, width, height): (f32, f32, f32, f32), scrolled: f32) {
    face(frame, theme, colour, &said(Some(when), lang), Some(&counted(text, lang)), Some(name), (x, y, width, height));
    let (left, top, wide, tall) = words_in(width, height);
    frame.markdown(text, Font::new(Family::YaHei, WORDS, 400.0), word_inks(theme, colour), x + left, y + top, wide, tall, scrolled);
}

/// How far the words `text` of a note stuck on the desktop, on a card `width` × `height`, scroll (DIPs): as far as they
/// reach past the foot of their box (none, where they fit in it).
pub(crate) fn stuck_scrolls(frame: &dyn Canvas, text: &str, (width, height): (f32, f32)) -> f32 {
    let (_, _, wide, tall) = words_in(width, height);
    (frame.markdown_tall(text, Font::new(Family::YaHei, WORDS, 400.0), wide) - tall).max(0.0)
}

/// Where a to-do's box is drawn, in the room drawn for it from `left`, by its line's `baseline` (in the window: see
/// `Editor::character`, `Editor::baseline`), its words `pixels` high: its left, its top and its side.
pub(super) fn box_in(left: i32, baseline: i32, pixels: f32) -> (f32, f32, f32) {
    use crate::notes::markdown::{BOX_IN, BOX_RISE, BOX_SIDE};
    let side = BOX_SIDE * pixels;
    (left as f32 + BOX_IN * pixels, baseline as f32 - BOX_RISE * pixels - side / 2.0, side)
}

/// What a note's menu does: its colour chosen, it stuck on the desktop, or its card there taken off it (the note kept),
/// it taken to the Recycle Bin. Or its writing menu's (see `writing_menu`): something done to its words.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Choice {
    Colour(crate::notes::Colour),
    Stick,
    Unstick,
    TakeAway,
    /// The notes' Markdown shown as written, or as it reads (see `crate::notes::source`).
    Source,
    Write(Write),
}

/// The notes' Markdown shown as written where it was shown as it reads, or the other way (chosen in a note's menu): at
/// once, and kept so in the settings. The notes shown drawn again.
pub fn turn_source() {
    let source = !crate::notes::source();
    crate::notes::show_source(source);
    std::thread::spawn(move || crate::app().change(|settings| settings.notes_source = source));
    crate::notes::read_again();
}

/// What a note's writing menu does to its words (see `Writing::write`): what is chosen cut or copied, the clipboard's
/// text pasted, a block inserted (see `notes::edit::insert`), a link written (see `link`: the one there edited), where
/// the link there goes copied, or it taken off its words, the lines chosen made headings of a level (0: words again),
/// the table whose cell is written in changed there (see `notes::table::changed`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Write {
    Cut,
    Copy,
    Paste,
    Insert(crate::notes::edit::Block),
    Link,
    CopyLink,
    Unlink,
    Heading(u8),
    Table(crate::notes::table::Change),
}

/// Where a note's writing menu is asked for: in a table's cell (or its words), on a link (see `notes::edit::link_at`:
/// whether it can be taken off its words, see `notes::edit::unlink`), or on none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Spot {
    pub in_cell: bool,
    pub link: Option<bool>,
}

/// A note's writing menu, shown on its words pressed with the right button (or the menu key, Shift+F10) while it is
/// written in, where `spot` says: cut, copy and paste, the blocks inserted and a link, the headings and words again. In
/// a table's cell, cut, copy and paste, a link, and its table's rows and columns added and taken away, or it. On a link,
/// in place of a link inserted, it edited, where it goes copied, and (but a web address written out) it taken off its
/// words. Its rows, and what each does.
pub fn writing_menu(lang: Lang, spot: Spot) -> (Vec<super::menu::Item<'static>>, Vec<Choice>) {
    use super::menu::{Item, Mark};
    use crate::notes::edit::Block;
    use crate::notes::table::Change;
    let mut rows = vec![
        (Write::Cut, lang.pick("剪切", "Cut"), Icon::Scissors, false),
        (Write::Copy, lang.pick("复制", "Copy"), Icon::Copy, false),
        (Write::Paste, lang.pick("粘贴", "Paste"), Icon::ClipboardPaste, false),
    ];
    if !spot.in_cell {
        rows.extend([
            (Write::Insert(Block::Todo), lang.pick("插入待办事项", "Insert a to-do"), Icon::SquareCheck, true),
            (Write::Insert(Block::Table), lang.pick("插入表格", "Insert a table"), Icon::Table, false),
            (Write::Insert(Block::Code), lang.pick("插入代码块", "Insert a code block"), Icon::SquareCode, false),
            (Write::Insert(Block::Rule), lang.pick("插入分隔线", "Insert a divider"), Icon::Minus, false),
        ]);
    }
    // (Its link's rows: in a cell, a group of their own.)
    let first = spot.in_cell;
    match spot.link {
        None => rows.push((Write::Link, lang.pick("插入链接", "Insert a link"), Icon::Link, first)),
        Some(removable) => {
            rows.push((Write::Link, lang.pick("编辑链接", "Edit the link"), Icon::SquarePen, first));
            rows.push((Write::CopyLink, lang.pick("复制链接地址", "Copy the link address"), Icon::Link, false));
            if removable {
                rows.push((Write::Unlink, lang.pick("移除链接", "Remove the link"), Icon::Unlink, false));
            }
        }
    }
    if spot.in_cell {
        rows.extend([
            (Write::Table(Change::RowAbove), lang.pick("在上方插入行", "Insert a row above"), Icon::ArrowUpToLine, true),
            (Write::Table(Change::RowBelow), lang.pick("在下方插入行", "Insert a row below"), Icon::ArrowDownToLine, false),
            (Write::Table(Change::ColumnLeft), lang.pick("在左侧插入列", "Insert a column to the left"), Icon::ArrowLeftToLine, false),
            (Write::Table(Change::ColumnRight), lang.pick("在右侧插入列", "Insert a column to the right"), Icon::ArrowRightToLine, false),
            (Write::Table(Change::TakeRow), lang.pick("删除行", "Delete the row"), Icon::Trash, true),
            (Write::Table(Change::TakeColumn), lang.pick("删除列", "Delete the column"), Icon::Trash, false),
            (Write::Table(Change::TakeTable), lang.pick("删除表格", "Delete the table"), Icon::Trash, false),
        ]);
    } else {
        rows.extend([
            (Write::Heading(1), lang.pick("一级标题", "Heading 1"), Icon::Heading1, true),
            (Write::Heading(2), lang.pick("二级标题", "Heading 2"), Icon::Heading2, false),
            (Write::Heading(3), lang.pick("三级标题", "Heading 3"), Icon::Heading3, false),
            (Write::Heading(0), lang.pick("正文", "Body text"), Icon::Pilcrow, false),
        ]);
    }
    let items = rows.iter().map(|(_, label, icon, rule_before)| Item { label, mark: Some(Mark::Icon(*icon)), checked: false, rule_before: *rule_before }).collect();
    (items, rows.into_iter().map(|(write, ..)| Choice::Write(write)).collect())
}

/// A note's menu, of colour `colour`: its colours, each a dot of it, the notes' Markdown shown as written (ticked while
/// it is), and kept in a file (`named`), it stuck on the desktop or (`stuck` there) taken off it, and it taken away.
/// Its rows, and what each does.
pub fn menu(lang: Lang, colour: crate::notes::Colour, named: bool, stuck: bool) -> (Vec<super::menu::Item<'static>>, Vec<Choice>) {
    use crate::notes::Colour;
    use super::menu::{Item, Mark};
    let names = [(Colour::Blue, lang.pick("蓝色", "Blue")), (Colour::Yellow, lang.pick("黄色", "Yellow")), (Colour::Green, lang.pick("绿色", "Green")), (Colour::Pink, lang.pick("粉色", "Pink")), (Colour::Purple, lang.pick("紫色", "Purple")), (Colour::Grey, lang.pick("灰色", "Grey"))];
    let mut rows: Vec<(Choice, &'static str, Mark, bool, bool)> = names.iter().map(|(each, name)| (Choice::Colour(*each), *name, Mark::Colour(super::view::note_colour(*each)), colour == *each, false)).collect();
    rows.push((Choice::Source, lang.pick("显示 Markdown 源码", "Show Markdown source"), Mark::Icon(Icon::Code), crate::notes::source(), true));
    if named {
        match stuck {
            true => rows.push((Choice::Unstick, lang.pick("从桌面取下", "Take off the desktop"), Mark::Icon(Icon::PinOff), false, true)),
            false => rows.push((Choice::Stick, lang.pick("贴到桌面", "Stick on the desktop"), Mark::Icon(Icon::Pin), false, true)),
        }
        rows.push((Choice::TakeAway, lang.pick("删除（移到回收站）", "Delete (to the Recycle Bin)"), Mark::Icon(Icon::Trash), false, false));
    }
    let items = rows.iter().map(|(_, label, mark, checked, rule_before)| Item { label, mark: Some(*mark), checked: *checked, rule_before: *rule_before }).collect();
    (items, rows.into_iter().map(|(choice, ..)| choice).collect())
}

/// How the note written shows, for its words' box's messages (see `Writing::answer`), which come whatever its window is
/// in the middle of: as it was last dressed, and the folder it is in.
struct Reads {
    shown: crate::notes::markdown::Shown,
    folder: std::path::PathBuf,
    laid: Laid,
    dressed: Rc<[u16]>,
}

/// A note's tables drawn laid out (see `notes::markdown::Shown::tables`), each laid out in its words' box (pixels: see
/// `Gfx::table_laid`).
type Laid = Vec<(crate::notes::table::Table, super::gfx::TableLaid)>;

/// A table's cell written in: its words in a box of their own (the text services', as the note's are) over where they
/// are drawn, the table drawn round them, laid out again as they are written. Which cell: its table (the note's
/// `nth`, as they are written), its row and its column.
struct InCell {
    editor: Rc<Editor>,
    table: usize,
    row: usize,
    column: usize,
}

/// Where the caret goes in a cell as it is written in: its start, its end, its words all chosen, or where a point
/// (in the window) is over its words, or below or above it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Placed {
    Start,
    End,
    All,
    At(POINT),
}

/// What a note's words' box asks of it as it is written in, done as it is next seen to (see `Writing::written`): a
/// table's cell written in (`table`, row, column), the caret placed in it so. Or the cell written in left: the caret
/// after the end of the line before its table (`before`), or at the start of the one after it (none there: one
/// begun), or (neither: pressed off it) where it is. Or a row more under the table whose cell is written in, written
/// in at its `column`. Or that table changed at the cell (chosen in the writing menu: see `table::changed`). Or a link
/// written where the caret is (see `link`: Ctrl+K, the format bar's button).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Asked {
    Cell(usize, usize, usize, Placed),
    Leave(Option<bool>),
    RowAfter(usize),
    Table(crate::notes::table::Change),
    Link,
}

/// The table cell `point` (in the window) is over, of those drawn laid out in `editor` (`laid`, as its words were
/// dressed: `dressed`): its table, row and column. None off them.
fn cell_at(editor: &Editor, laid: &Laid, dressed: &[u16], point: POINT) -> Option<(usize, usize, usize)> {
    let left = editor.bounds().left;
    let now: Vec<u16> = editor.text().encode_utf16().collect();
    let moved = Moved::between(dressed, &now);
    laid.iter().enumerate().find_map(|(nth, (table, laid))| {
        let top = line_top(editor, &now, moved.at(table.span.start)?)?;
        let (row, column) = laid.cell_at((point.x - left) as f32, (point.y - top) as f32)?;
        Some((nth, row, column))
    })
}

/// Where the cell `row`, `column` of the table `laid` (its first line's top `top`, in `editor`'s window) has its words
/// written in (see `InCell`): in the window, in whole pixels (as wide as all the pixels they may go into).
fn cell_box(editor: &Editor, laid: &super::gfx::TableLaid, top: i32, row: usize, column: usize) -> Option<RECT> {
    let left = editor.bounds().left;
    let (along, down, wide, tall) = laid.words_at(row, column)?;
    let (x, y) = (left + along.round() as i32, top + down.round() as i32);
    Some(RECT { left: x, top: y, right: x + wide.ceil().max(1.0) as i32, bottom: y + tall.round().max(1.0) as i32 })
}

/// Whether the cell `row`, `column` of `table` (as dressed, `moved` to where its words are now) is in what is chosen
/// of them, `chosen` (from, to: UTF-16 units now): what is chosen takes in some of its room, or, its room empty, the
/// place of it.
fn cell_chosen(table: &crate::notes::table::Table, moved: &Moved, (from, to): (i32, i32), row: usize, column: usize) -> bool {
    let Some(cell) = table.rows.get(row).and_then(|cells| cells.get(column)) else { return false };
    let (Some(start), Some(end)) = (moved.at(cell.room.start), moved.at(cell.room.end)) else { return false };
    let (from, to) = (from.max(0) as usize, to.max(0) as usize);
    from < end.max(start + 1) && to > start
}

/// Words pasted into a table's cell (`text`, the clipboard's): on one line (its line breaks spaces), its pipes written
/// as they read in a cell (`\|`).
fn into_cell(text: &str) -> String {
    let one_line = text.replace("\r\n", " ").replace(['\r', '\n'], " ");
    let mut written = String::with_capacity(one_line.len());
    let mut escaped = false;
    for c in one_line.chars() {
        if c == '|' && !escaped {
            written.push('\\');
        }
        escaped = c == '\\' && !escaped;
        written.push(c);
    }
    written
}

/// What a table's cell written in shares with its note's words' box as it answers its window's messages (see
/// `answer_cell`): what it asks of the note (`Writing::asked`), the character the key last down makes to be eaten
/// (`Writing::eaten`), where the writing menu is asked for (`Writing::menu_asked`), whether a press was begun in the
/// cell's box (`Writing::pressing_cell`).
struct Asking {
    asked: Rc<Cell<Option<Asked>>>,
    eaten: Rc<Cell<bool>>,
    menu_asked: Rc<Cell<Option<POINT>>>,
    pressing: Rc<Cell<bool>>,
}

/// A window message for a note's words' box while a table's cell is written in (`inside`: its table's cell `row`,
/// `column`, of a table `size` rows and columns), answered in the cell's box: the keys (a line break, a tab and a pipe
/// as a cell has them), the input method's, the focus, a press begun on its words, the pointer over them. A key that
/// goes past its words (to another cell, or out of the table) asks for it (see `Asked`). None: the note's words' (the
/// pointer off the cell's box, the wheel. Its timers the cell's too).
fn answer_cell(hwnd: HWND, (message, wparam, lparam): (u32, WPARAM, LPARAM), inside: &Editor, (nth, row, column): (usize, usize, usize), (rows, columns): (usize, usize), asking: &Asking) -> Option<Option<LRESULT>> {
    let point = POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 };
    let sent = || Some(Some(inside.send(message, wparam, lparam).unwrap_or(LRESULT(0))));
    let done = Some(Some(LRESULT(0)));
    let ask = |asked: Asked| {
        asking.asked.set(Some(asked));
        done
    };
    // (The cell before it and after it, across its rows: none before its first, none after its last.)
    let before = if column > 0 { Some((row, column - 1)) } else { row.checked_sub(1).map(|row| (row, columns - 1)) };
    let after = if column + 1 < columns { Some((row, column + 1)) } else { (row + 1 < rows).then_some((row + 1, 0)) };
    match message {
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK if inside.holds(point) => {
            asking.pressing.set(true);
            sent()
        }
        // (Let go, or moved with the button up: a press whose letting go went elsewhere is over too.)
        WM_LBUTTONUP | WM_MOUSEMOVE if asking.pressing.get() => {
            asking.pressing.set(message == WM_MOUSEMOVE && wparam.0 & 0x0001 != 0);
            sent()
        }
        // (Pressed off what is chosen, the caret there: the menu, as it is let go, does its work on it.)
        WM_RBUTTONDOWN if inside.holds(point) => {
            let (from, to) = inside.selection();
            let at = inside.at(point) as i32;
            if !inside.composing() && (at < from.min(to) || at > from.max(to)) {
                inside.select((at, at));
            }
            done
        }
        WM_SETCURSOR => {
            let mut at = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut at);
                let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut at);
            }
            inside.holds(at).then(|| {
                inside.set_cursor(at);
                Some(LRESULT(1))
            })
        }
        WM_TIMER => {
            let _ = inside.send(message, wparam, lparam);
            None
        }
        WM_SETFOCUS | WM_KILLFOCUS | WM_KEYUP | WM_SYSCHAR | WM_UNICHAR => sent(),
        // (The input method's, unanswered by the box, are the system's as well.)
        WM_IME_STARTCOMPOSITION | WM_IME_COMPOSITION | WM_IME_ENDCOMPOSITION | WM_IME_NOTIFY | WM_IME_SETCONTEXT | WM_IME_CHAR | WM_IME_REQUEST | WM_IME_COMPOSITIONFULL | WM_IME_SELECT => Some(inside.send(message, wparam, lparam)),
        WM_CHAR => match char::from_u32(wparam.0 as u32) {
            // (A pipe written in a cell is written as one reads there: escaped.)
            Some('|') => {
                let (from, to) = inside.selection();
                let (from, to) = (from.min(to).max(0) as usize, from.max(to).max(0) as usize);
                inside.apply(&crate::notes::edit::Edit { range: from..to, with: "\\|".into(), chosen: (from + 2, from + 2) });
                done
            }
            Some('\t' | '\r' | '\n') => done,
            _ => sent(),
        },
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            asking.eaten.set(false);
            if inside.composing() {
                return sent();
            }
            let held = |key: VIRTUAL_KEY| unsafe { GetKeyState(i32::from(key.0)) } < 0;
            let (ctrl, shift, alt) = (held(VK_CONTROL), held(VK_SHIFT), held(VK_MENU));
            let key = VIRTUAL_KEY(wparam.0 as u16);
            let (from, to) = inside.selection();
            let length = inside.words().encode_utf16().count() as i32;
            let top = |at: i32| inside.character_anywhere(at.max(0) as usize).map(|drawn| drawn.top);
            // (Where the caret is across: where a cell above or below it is written in from.)
            let across = inside.caret().map_or(point.x, |(x, ..)| x);
            let eaten = |answered| {
                asking.eaten.set(true);
                answered
            };
            match key {
                // The writing menu, at the caret (its window's to show).
                VK_APPS | VK_F10 if key == VK_APPS || shift && !ctrl && !alt => {
                    let bounds = inside.bounds();
                    let mut at = inside.caret().map_or(POINT { x: bounds.left, y: bounds.top }, |(x, y, _, tall)| POINT { x, y: y + tall });
                    let _ = unsafe { windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut at) };
                    asking.menu_asked.set(Some(at));
                    eaten(done)
                }
                // (Alt+Up and Alt+Down move the note's lines: not a cell's.)
                _ if message == WM_SYSKEYDOWN => done,
                VK_TAB if shift => eaten(before.map_or(done, |(row, column)| ask(Asked::Cell(nth, row, column, Placed::All)))),
                VK_TAB => eaten(after.map_or_else(|| ask(Asked::RowAfter(0)), |(row, column)| ask(Asked::Cell(nth, row, column, Placed::All)))),
                VK_RETURN if row + 1 < rows => eaten(ask(Asked::Cell(nth, row + 1, column, Placed::End))),
                VK_RETURN => eaten(ask(Asked::RowAfter(column))),
                VK_UP if !shift && !ctrl && top(to) == top(0) => match row {
                    0 => ask(Asked::Leave(Some(true))),
                    _ => ask(Asked::Cell(nth, row - 1, column, Placed::At(POINT { x: across, y: i32::MAX }))),
                },
                VK_DOWN if !shift && !ctrl && top(to) == top(length) => match row + 1 < rows {
                    true => ask(Asked::Cell(nth, row + 1, column, Placed::At(POINT { x: across, y: i32::MIN }))),
                    false => ask(Asked::Leave(Some(false))),
                },
                VK_LEFT if !shift && from == to && to == 0 => before.map_or_else(|| ask(Asked::Leave(Some(true))), |(row, column)| ask(Asked::Cell(nth, row, column, Placed::End))),
                VK_RIGHT if !shift && from == to && to == length => after.map_or_else(|| ask(Asked::Leave(Some(false))), |(row, column)| ask(Asked::Cell(nth, row, column, Placed::Start))),
                // (Nothing before its words to take away, nor after them: the cells round it are not joined to it.)
                VK_BACK if from == to && to == 0 => eaten(done),
                VK_DELETE if from == to && to == length => done,
                // Pasted: on one line, its pipes escaped (see `into_cell`).
                VK_V if ctrl && !alt => eaten(paste_in_cell(hwnd, inside)),
                VK_INSERT if shift && !ctrl && !alt => eaten(paste_in_cell(hwnd, inside)),
                // (Of the keys held with Ctrl, choosing all, copying, cutting, undoing and doing again are a cell's, and a
                // link written: the rest write Markdown, a cell's words written as they are.)
                // (Ctrl with Alt is AltGr, writing a character on many keyboards: not a shortcut.)
                VK_A | VK_C | VK_X | VK_Z | VK_Y if ctrl && !alt => sent(),
                // (Ctrl+K: a link written there, as in the note's words.)
                VK_K if ctrl && !shift && !alt => eaten(ask(Asked::Link)),
                _ if ctrl && !alt && (0x30..=0x5A).contains(&key.0) => eaten(done),
                _ => sent(),
            }
        }
        _ => None,
    }
}

/// The clipboard's words written in a table's cell written in (`inside`), in place of what is chosen (see `into_cell`):
/// nothing where there are none.
fn paste_in_cell(hwnd: HWND, inside: &Editor) -> Option<Option<LRESULT>> {
    if let Some(text) = clipboard_text(hwnd) {
        let (from, to) = inside.selection();
        let (from, to) = (from.min(to).max(0) as usize, from.max(to).max(0) as usize);
        let with = into_cell(&text);
        let end = from + with.encode_utf16().count();
        inside.apply(&crate::notes::edit::Edit { range: from..to, with, chosen: (end, end) });
    }
    Some(Some(LRESULT(0)))
}

/// Where an arrow `key` takes the caret of `editor`'s words (nothing chosen) into a table drawn laid out (`laid`), the
/// cell it is written in at: Down on the last row of the line before one, the cell of its head under the caret, Up on
/// the first row of the line after one, the cell of its last row under it. Right at the end of the line before one,
/// its first cell, Left at the start of the line after one, its last. None where it goes elsewhere.
fn into_table(editor: &Editor, laid: &Laid, key: VIRTUAL_KEY) -> Option<Asked> {
    let (from, to) = editor.selection();
    if from != to {
        return None;
    }
    let units: Vec<u16> = editor.text().encode_utf16().collect();
    let at = (from.max(0) as usize).min(units.len());
    let start = units[..at].iter().rposition(|&unit| unit == u16::from(b'\n')).map_or(0, |at| at + 1);
    let end = (at..units.len()).find(|&unit| units[unit] == u16::from(b'\n'));
    let before = end.and_then(|end| laid.iter().position(|(table, _)| table.span.start == end + 1));
    let after = laid.iter().position(|(table, _)| start > 0 && table.span.end + 1 == start);
    let drawn = editor.character_anywhere(at)?;
    let row_of = |unit: usize| editor.character_anywhere(unit).map(|drawn| drawn.top);
    let column = |nth: usize| laid[nth].1.column_at((drawn.left - editor.bounds().left) as f32);
    let last = |nth: usize| laid[nth].0.rows.len() - 1;
    match key {
        VK_DOWN => {
            let nth = before?;
            (row_of(end?) == Some(drawn.top)).then(|| Asked::Cell(nth, 0, column(nth), Placed::At(POINT { x: drawn.left, y: i32::MIN })))
        }
        VK_UP => {
            let nth = after?;
            (row_of(start) == Some(drawn.top)).then(|| Asked::Cell(nth, last(nth), column(nth), Placed::At(POINT { x: drawn.left, y: i32::MAX })))
        }
        VK_RIGHT => (Some(at) == end).then_some(Asked::Cell(before?, 0, 0, Placed::Start)),
        VK_LEFT => (at == start).then(|| after.map(|nth| Asked::Cell(nth, last(nth), laid[nth].0.aligns.len() - 1, Placed::End))).flatten(),
        _ => None,
    }
}

/// How what was dressed has moved in a note's words as they are now (the input method's words composed in them, not
/// yet dressed: see `Writing::written`): the first unit that changed, the first after the change (as it was), and how
/// much longer the words grew.
struct Moved {
    before: usize,
    after: usize,
    by: isize,
}

impl Moved {
    /// From `was` (UTF-16 units, as dressed) to `now`.
    fn between(was: &[u16], now: &[u16]) -> Moved {
        let before = was.iter().zip(now).take_while(|(a, b)| a == b).count();
        let after = was[before..].iter().rev().zip(now[before..].iter().rev()).take_while(|(a, b)| a == b).count();
        Moved { before, after: was.len() - after, by: now.len() as isize - was.len() as isize }
    }

    /// Where the unit `at` of the words as dressed is now: before the change where it was, after it moved on with it,
    /// in what changed nowhere.
    fn at(&self, at: usize) -> Option<usize> {
        if at < self.before {
            Some(at)
        } else if at >= self.after {
            Some((at as isize + self.by) as usize)
        } else {
            None
        }
    }
}

/// The face a note's tables' words are written in, as its words are (see `palette_for`, `paint_stuck`).
const TABLE_FAMILY: Family = Family::YaHei;

/// The top of the line the character at `at` (UTF-16 units of `units`, its words) is on in `editor` (in the window),
/// found by its line's break (the line's words may be put away, and nowhere drawn: its break is drawn), where it is
/// drawn or would be (scrolled out of the box).
fn line_top(editor: &Editor, units: &[u16], at: usize) -> Option<i32> {
    let end = (at..units.len()).find(|&unit| units[unit] == u16::from(b'\n'))?;
    editor.character_anywhere(end).map(|drawn| drawn.top)
}

/// The foot of the line the character at `at` (UTF-16 units of `units`, its words) is on in `editor` (in the window), of
/// all its rows (found by its line's break, or the words' end), where it is drawn or would be.
fn line_foot(editor: &Editor, units: &[u16], at: usize) -> Option<i32> {
    let end = (at..units.len()).find(|&unit| units[unit] == u16::from(b'\n')).unwrap_or(units.len());
    editor.character_anywhere(end).map(|drawn| drawn.bottom)
}

/// The caret in `editor`, moved `way`, put where words are (see `notes::markdown::Shown::settle`), its words read as
/// `shown` (as they were when dressed: `dressed`, UTF-16 units; changed since, not yet dressed, left where it is), on a
/// line begun after the last where that is where it is to rest (`below`: else left). Not where something is chosen,
/// nor while the input method composes.
fn settle(editor: &Editor, shown: &crate::notes::markdown::Shown, dressed: &[u16], way: crate::notes::markdown::Way, below: bool) {
    use crate::notes::markdown::Settled;
    let (from, to) = editor.selection();
    if from != to || editor.composing() {
        return;
    }
    let now: Vec<u16> = editor.text().encode_utf16().collect();
    if now != dressed {
        return;
    }
    match shown.settle(&now, from.max(0) as usize, way) {
        Some(Settled::At(at)) => editor.select((at as i32, at as i32)),
        Some(Settled::Below) if below => {
            let end = now.len();
            editor.apply(&crate::notes::edit::Edit { range: end..end, with: "\n".into(), chosen: (end + 1, end + 1) });
        }
        _ => {}
    }
}

/// What a note's words' box answers its window's messages with (see `Writing::answer`).
pub use super::keys::Answer;

/// The keys held as another goes down.
#[derive(Clone, Copy, Default)]
struct Held {
    ctrl: bool,
    shift: bool,
    alt: bool,
}

/// Whether `key` writes a character (a letter, a digit, a mark): with Ctrl held, one of the text services' own keys
/// that dress the words (bold, centred, spaced) where it is not one a note's (see `pressed`).
fn writes(key: VIRTUAL_KEY) -> bool {
    matches!(key.0, 0x30..=0x39 | 0x41..=0x5A | 0x60..=0x6F | 0xBA..=0xC0 | 0xDB..=0xDF | 0xE2)
}

/// What `key` going down does to the words as Markdown is written (see `notes::edit`), `text` written and `chosen`
/// chosen, `held` held with it (`system`: told as a system key, Alt held): an edit made (`Some(Some)`), or nothing done
/// (`Some(None)`: the key and the character it makes eaten), or none: the key the text services' as always.
///
/// Enter carries a list on (Shift+Enter a line break), Ctrl+Enter ticks a to-do, Tab and Shift+Tab move a list's items in
/// and out, Backspace takes an item's mark off (and, the note shown as it reads, marks put away one by one, a rule
/// before it: see `edit::backspace`), Delete a rule after it (see `edit::delete`), Alt+Up and Alt+Down move lines. Ctrl+B, Ctrl+I, Ctrl+Shift+X and Ctrl+E
/// put on and take off bold, slanted, struck through and code, Ctrl+1 to 3 headings and Ctrl+0 none. Ctrl+V and
/// Shift+Insert paste what is on the clipboard as Markdown, words among others or blocks (`clipboard` asked for it then
/// only: see `clipboard_pasted`, `edit::paste`: the text services' own paste would bring what a note's file does not
/// keep). Ctrl with any other key that writes is eaten but for choosing all, copying, cutting, undoing and redoing: the
/// text services' own would dress the words in ways a note's file does not keep. (Ctrl+K, a link written, is its words'
/// box's: see `Writing::answer`.)
fn pressed(text: &str, chosen: (usize, usize), key: VIRTUAL_KEY, held: Held, system: bool, clipboard: impl FnOnce() -> Option<crate::notes::edit::Pasted>) -> Option<Option<crate::notes::edit::Edit>> {
    use crate::notes::edit::{self, Marked};
    let caret = (chosen.0 == chosen.1).then_some(chosen.0);
    if system || held.alt {
        return match key {
            VK_UP | VK_DOWN if held.alt && !held.ctrl && !held.shift => Some(edit::move_lines(text, chosen, key == VK_UP)),
            _ => None,
        };
    }
    let marked = |how| Some(Some(edit::toggle(text, chosen, how)));
    match (key, held.ctrl, held.shift) {
        (VK_RETURN, false, false) => caret.and_then(|caret| edit::enter(text, caret)).map(Some),
        (VK_RETURN, true, false) => Some(edit::toggle_task(text, chosen.0)),
        (VK_TAB, false, back) => crate::notes::table::tab(text, chosen, back).or_else(|| edit::indent(text, chosen, back)),
        (VK_BACK, false, false) => caret.and_then(|caret| edit::backspace(text, caret, !crate::notes::source())).map(Some),
        (VK_DELETE, false, false) => caret.and_then(|caret| edit::delete(text, caret, !crate::notes::source())).map(Some),
        (VK_INSERT, false, true) => Some(clipboard().map(|pasted| edit::paste(text, chosen, &pasted))),
        (_, false, _) => None,
        (VK_B, true, false) => marked(Marked::Bold),
        (VK_I, true, false) => marked(Marked::Italic),
        (VK_E, true, false) => marked(Marked::Code),
        (VK_X, true, true) => marked(Marked::Struck),
        (VK_0 | VK_1 | VK_2 | VK_3, true, false) => Some(Some(edit::heading(text, chosen, (key.0 - VK_0.0) as u8))),
        (VK_V, true, false) => Some(clipboard().map(|pasted| edit::paste(text, chosen, &pasted))),
        (VK_A | VK_C | VK_X | VK_Z | VK_Y, true, false) | (VK_Z, true, true) => None,
        (key, true, _) if writes(key) => Some(None),
        _ => None,
    }
}

/// What a button of the format bar puts on the words chosen: marks, or a link.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Format {
    Marked(crate::notes::edit::Marked),
    Link,
}

/// The format bar's buttons: bold, slanted, struck through, code, link.
const BAR: [(Icon, Format); 5] = {
    use crate::notes::edit::Marked;
    [(Icon::Bold, Format::Marked(Marked::Bold)), (Icon::Italic, Format::Marked(Marked::Italic)), (Icon::Strikethrough, Format::Marked(Marked::Struck)), (Icon::Code, Format::Marked(Marked::Code)), (Icon::Link, Format::Link)]
};
/// How large each of its buttons is, how far in from its edge they are, and how far it is from the words chosen (DIPs,
/// for words `WORDS` high: larger as they are).
const BAR_BUTTON: f32 = 28.0;
const BAR_INSET: f32 = 4.0;
const BAR_GAP: f32 = 6.0;

/// Where the format bar shows (window pixels), a note's words written `pixels` high in `editor`: over what is chosen
/// (its first character), under it where the words' box has no room above, within the box. While something is chosen,
/// not while the pointer is choosing it (`pressing`), nor while the input method composes. None: it does not show.
fn bar_at(editor: &Editor, pixels: f32, pressing: bool) -> Option<RECT> {
    let (from, to) = editor.selection();
    if from == to || pressing || editor.composing() {
        return None;
    }
    let per = pixels / WORDS;
    let (wide, tall) = (((BAR.len() as f32 * BAR_BUTTON + 2.0 * BAR_INSET) * per).round() as i32, ((BAR_BUTTON + 2.0 * BAR_INSET) * per).round() as i32);
    let gap = (BAR_GAP * per).round() as i32;
    let bounds = editor.bounds();
    let (from, to) = (from.min(to) as usize, from.max(to) as usize);
    let first = editor.character(from)?;
    let top = match first.top - gap - tall {
        above if above >= bounds.top => above,
        _ => editor.character(to - 1).map_or(first.bottom, |last| last.bottom) + gap,
    };
    let left = first.left.min(bounds.right - wide).max(bounds.left);
    let top = top.min(bounds.bottom - tall).max(bounds.top);
    Some(RECT { left, top, right: left + wide, bottom: top + tall })
}

/// Which of the format bar's buttons (at `bar`, for words `pixels` high) `point` is on: none between them, at its edge.
fn bar_button(bar: &RECT, point: POINT, pixels: f32) -> Option<usize> {
    let per = pixels / WORDS;
    let (x, y) = (point.x as f32 - bar.left as f32 - BAR_INSET * per, point.y as f32 - bar.top as f32 - BAR_INSET * per);
    let nth = (x / (BAR_BUTTON * per)).floor();
    (x >= 0.0 && y >= 0.0 && y < BAR_BUTTON * per && nth < BAR.len() as f32).then_some(nth as usize)
}

/// The clipboard's `format`, if it holds it: its data as far as the zero that ends it (a unit `unit` bytes long, 1 for
/// bytes, 2 for UTF-16).
fn clipboard_data(owner: HWND, format: u32, unit: usize) -> Option<Vec<u8>> {
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::{GetClipboardData, IsClipboardFormatAvailable};
    use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    if unsafe { IsClipboardFormatAvailable(format) }.is_err() {
        return None;
    }
    let mut taken = Vec::new();
    let _opened = crate::clipboard::open(owner)?;
    unsafe {
        if let Ok(held) = GetClipboardData(format) {
            let memory = HGLOBAL(held.0);
            let size = GlobalSize(memory);
            let bytes = GlobalLock(memory) as *const u8;
            if !bytes.is_null() {
                taken = std::slice::from_raw_parts(bytes, size).to_vec();
                let _ = GlobalUnlock(memory);
            }
        }
    }
    // (As far as the zero that ends it: what is held past it is the memory's rounding.)
    let end = taken.chunks(unit).position(|chunk| chunk.iter().all(|byte| *byte == 0)).map_or(taken.len(), |at| at * unit);
    taken.truncate(end);
    (!taken.is_empty()).then_some(taken)
}

/// The text on the clipboard (its line breaks `\n`), if there is any.
fn clipboard_text(owner: HWND) -> Option<String> {
    // (CF_UNICODETEXT.)
    const TEXT: u32 = 13;
    let bytes = clipboard_data(owner, TEXT, 2)?;
    let units: Vec<u16> = bytes.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
    Some(String::from_utf16_lossy(&units).replace("\r\n", "\n").replace('\r', "\n"))
}

/// What is on the clipboard, to be pasted into a note: its HTML (a web page's, Word's) as Markdown (see `notes::html`),
/// else its text. A web address alone as its text is pasted as written (over words chosen, a link of them: see
/// `edit::paste`), not as its HTML's link to it.
fn clipboard_pasted(owner: HWND) -> Option<crate::notes::edit::Pasted> {
    use crate::notes::edit::{web_address, Pasted};
    use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
    let text = clipboard_text(owner);
    let plain = |text: String| Pasted { text, blocks: false };
    if text.as_deref().is_some_and(|text| web_address(text).is_some()) {
        return text.map(plain);
    }
    let html = unsafe { RegisterClipboardFormatW(windows::core::w!("HTML Format")) };
    let written = clipboard_data(owner, html, 1).and_then(|bytes| crate::notes::html::pasted(&bytes));
    written.filter(|written| !written.text.is_empty()).or_else(|| text.map(plain))
}

/// A note as it is written: its file's name (none till it is first kept), its file as last read or written here, its
/// colour, its words' box, whether what is written is not yet kept, and what was last said of keeping it.
pub struct Writing {
    /// The folder it is in (kept to: the folder chosen in the settings changed meanwhile, it is kept where it was read).
    pub folder: std::path::PathBuf,
    pub name: Option<String>,
    stamp: Option<crate::notes::store::Stamp>,
    pub colour: crate::notes::Colour,
    written_ms: u64,
    pub editor: Rc<Editor>,
    /// What its Markdown is dressed in (see `Editor::dress`).
    palette: Palette,
    pub unkept: bool,
    pub said: Option<String>,
    /// How its Markdown shows, as it was last dressed (see `notes::markdown::Shown`), the same for its box's messages
    /// (see `Reads`).
    shown: crate::notes::markdown::Shown,
    /// Whether its Markdown was shown as written as it was last dressed (see `crate::notes::source`).
    source: bool,
    /// Its tables drawn laid out, as it was last dressed, and its words then (UTF-16 units).
    laid: Laid,
    dressed: Rc<[u16]>,
    reads: Rc<RefCell<Reads>>,
    /// The table's cell written in, if one is (see `InCell`), and what its box asked of it, to be done as it is next
    /// seen to (see `Asked`), and whether a press on the words was begun in the cell's box (followed there till let go).
    cell: Rc<RefCell<Option<InCell>>>,
    asked: Rc<Cell<Option<Asked>>>,
    pressing_cell: Rc<Cell<bool>>,
    /// Its box was given a key, or the input method's words, since this was last asked (see `touched`), and how deep its
    /// box's messages are in it now (one comes inside another: see `answering`).
    keyed: Rc<Cell<bool>>,
    depth: Rc<Cell<u32>>,
    /// The character the key last down makes is to be eaten: the key did what it does to the words as Markdown is
    /// written (see `pressed`), or nothing.
    eaten: Rc<Cell<bool>>,
    /// How high its words are written (pixels), whether the pointer is pressed on them (choosing), and which of the
    /// format bar's buttons it is over (see `bar_at`).
    pixels: Rc<Cell<f32>>,
    pressing: Rc<Cell<bool>>,
    over: Rc<Cell<Option<usize>>>,
    /// Where its writing menu was asked for (on the screen), not yet shown: see `take_menu_asked`.
    menu_asked: Rc<Cell<Option<POINT>>>,
    /// The link being written in it, if one is (see `link`).
    link: link::Held,
}

impl Writing {
    /// The note `name` in `folder` (none: a new one, taking the next colour round), its words in a box at `rect` in the
    /// window `hwnd`, `pixels` high, in `theme`'s inks, the caret at its start. None where the text services cannot be had.
    pub fn open(hwnd: HWND, rect: RECT, folder: &crate::notes::store::Folder, name: Option<String>, theme: &Theme, pixels: f32) -> Option<Writing> {
        let read = name.as_ref().and_then(|name| folder.read(name));
        let shown = crate::notes::shown();
        let colour = match &name {
            Some(name) => shown.iter().find(|note| &note.name == name).map(|note| note.colour).unwrap_or_default(),
            // (A new note takes the next colour round.)
            None => crate::notes::Colour::ALL[shown.len() % crate::notes::Colour::ALL.len()],
        };
        let palette = palette_for(theme, colour, pixels);
        let editor = Editor::new(hwnd, rect, &palette.face, pixels, palette.text)?;
        if let Some((text, _)) = &read {
            editor.set_text(text);
        }
        let editor = Rc::new(editor);
        let mut writing = Writing {
            folder: folder.path.clone(),
            name,
            written_ms: read.as_ref().map_or_else(crate::station::now_ms, |(_, stamp)| stamp.written_ms),
            stamp: read.map(|(_, stamp)| stamp),
            colour,
            editor,
            palette,
            unkept: false,
            said: None,
            shown: Default::default(),
            source: false,
            laid: Vec::new(),
            dressed: Rc::from([]),
            reads: Rc::new(RefCell::new(Reads { shown: Default::default(), folder: folder.path.clone(), laid: Vec::new(), dressed: Rc::from([]) })),
            cell: Rc::new(RefCell::new(None)),
            asked: Rc::new(Cell::new(None)),
            pressing_cell: Rc::new(Cell::new(false)),
            keyed: Rc::new(Cell::new(false)),
            depth: Rc::new(Cell::new(0)),
            eaten: Rc::new(Cell::new(false)),
            pixels: Rc::new(Cell::new(pixels)),
            pressing: Rc::new(Cell::new(false)),
            over: Rc::new(Cell::new(None)),
            menu_asked: Rc::new(Cell::new(None)),
            link: Rc::new(RefCell::new(None)),
        };
        writing.dress();
        // (The caret where words are, not among what is put away.)
        settle(&writing.editor, &writing.shown, &writing.dressed, crate::notes::markdown::Way::Near, false);
        Some(writing)
    }

    /// Its words dressed as their Markdown reads (or as written, its source asked for: see `crate::notes::source`), its
    /// marks put away, its tables laid out in its words' box (as a note shown lays them out, see `Frame::markdown`),
    /// their lines as tall as they take. A table's cell written in is dressed again with them, and put where it is drawn
    /// now.
    pub fn dress(&mut self) {
        let text = self.editor.text();
        self.source = crate::notes::source();
        let shown = crate::notes::shown_as(&text);
        let (hidden, rooms) = shown.put_away();
        // (Its words whole pixels high, as a note shown makes them, at its window's scale.)
        let (pixels, per) = (self.pixels().round(), self.pixels() / WORDS);
        let bounds = self.editor.bounds();
        // (The device asked for only where there is a table to lay out: made on a thread, it is that thread's to let go.)
        let tables: Vec<_> = shown.tables.iter().collect();
        self.laid = match (!tables.is_empty()).then(super::gfx::current) {
            Some(Ok(gfx)) => tables.into_iter().map(|table| (table.clone(), gfx.table_laid(table, TABLE_FAMILY, pixels, per, (bounds.right - bounds.left) as f32))).collect(),
            _ => Vec::new(),
        };
        let drawn = shown.drawn_lines(pixels, self.laid.iter().map(|(table, laid)| (table.span.clone(), laid.laid.height())));
        self.editor.dress(&crate::notes::markdown::looks(&text), &hidden, &rooms, &drawn, &shown.leads, &self.palette);
        self.dressed = text.encode_utf16().collect();
        if let Ok(mut reads) = self.reads.try_borrow_mut() {
            *reads = Reads { shown: shown.clone(), folder: self.folder.clone(), laid: self.laid.clone(), dressed: self.dressed.clone() };
        }
        self.shown = shown;
        self.dress_cell();
    }

    /// Its words' box put at `rect` (in its window): as wide as it was, as it is, else dressed again (its tables laid out
    /// in the room it has now).
    pub fn place(&mut self, rect: RECT) {
        let was = self.editor.bounds();
        if was == rect {
            return;
        }
        self.editor.place(rect);
        if rect.right - rect.left != was.right - was.left {
            self.dress();
        } else {
            self.place_cell();
        }
    }

    /// Whether a table's cell is written in (its writing menu is the cell's: see `writing_menu`).
    pub fn in_cell(&self) -> bool {
        self.cell.borrow().is_some()
    }

    /// Its words told they have the keys (`on`), or no longer (what the input method composes written): a table's cell
    /// written in, where one is, the field of a link being written that has them. A link being written is left as they
    /// go (nothing written).
    pub fn focus(&self, on: bool) {
        if !on {
            link::end(&self.link, link::Ending::Left);
        }
        let linking = self.link.borrow().clone();
        let focused = match linking {
            Some(linking) => linking.field(linking.focused()).clone(),
            None => self.target(),
        };
        let _ = focused.send(if on { WM_SETFOCUS } else { WM_KILLFOCUS }, WPARAM(0), LPARAM(0));
    }

    /// The box its words are written in now: a table's cell written in, where one is, else its own.
    fn target(&self) -> Rc<Editor> {
        self.cell_now().map_or_else(|| self.editor.clone(), |(inside, ..)| inside)
    }

    /// The link what is chosen where it is written in is on (see `notes::edit::link_at`), in the box it is written in,
    /// with that box's words.
    fn link_here(&self) -> Option<(Rc<Editor>, String, crate::notes::edit::Found)> {
        let target = self.target();
        let (from, to) = target.selection();
        let text = target.words();
        let found = crate::notes::edit::link_at(&text, (from.max(0) as usize, to.max(0) as usize))?;
        Some((target, text, found))
    }

    /// Where its writing menu is asked for now (see `Spot`).
    pub fn spot(&self) -> Spot {
        Spot { in_cell: self.in_cell(), link: self.link_here().map(|(_, text, found)| crate::notes::edit::unlink(&text, &found).is_some()) }
    }

    /// A link written where it is written in (see `link`): the one what is chosen is on edited, else one put in place
    /// of what is chosen (on one line: its words those, else none, put after it), going to the web address on the
    /// clipboard where there is one. Not while one is.
    fn open_link(&self) {
        if self.link.borrow().is_some() {
            return;
        }
        let target = self.target();
        let opened = match self.link_here() {
            Some((_, _, found)) => link::Linking::open(target, (found.span.start, found.span.end), self.editor.clone(), (&found.shown, &found.to), &self.palette.face, self.pixels(), self.palette.text),
            None => {
                let (from, to) = target.selection();
                let (from, to) = (from.min(to).max(0) as usize, from.max(to).max(0) as usize);
                let units: Vec<u16> = target.words().encode_utf16().collect();
                let chosen = String::from_utf16_lossy(&units[from.min(units.len())..to.min(units.len())]);
                let (range, words) = match chosen.contains('\n') {
                    false => ((from, to), crate::notes::edit::brackets_unescaped(&chosen)),
                    true => ((to, to), String::new()),
                };
                let on_clipboard = clipboard_text(self.editor.window());
                let to = on_clipboard.as_deref().and_then(crate::notes::edit::web_address).unwrap_or_default();
                link::Linking::open(target, range, self.editor.clone(), (&words, to), &self.palette.face, self.pixels(), self.palette.text)
            }
        };
        *self.link.borrow_mut() = opened.map(Rc::new);
    }

    /// Whether `point` (in its window) is its: on its words, or on the link being written.
    pub fn holds(&self, point: POINT) -> bool {
        self.editor.holds(point) || self.link.borrow().as_ref().is_some_and(|linking| linking.holds(point))
    }

    /// Pressed elsewhere in its window (off what is its: see `holds`): the link being written left.
    pub fn pressed_off(&self) {
        link::end(&self.link, link::Ending::Left);
    }

    /// What is chosen where it is written in (see `Editor::selection`): in a table's cell written in, where one is.
    pub fn chosen(&self) -> (i32, i32) {
        self.cell_now().map_or_else(|| self.editor.selection(), |(inside, ..)| inside.selection())
    }

    /// The table's cell written in (see `InCell`), its box and where it is: the note's table, row and column.
    fn cell_now(&self) -> Option<(Rc<Editor>, usize, usize, usize)> {
        self.cell.borrow().as_ref().map(|cell| (cell.editor.clone(), cell.table, cell.row, cell.column))
    }

    /// The table's cell written in dressed as its table draws its words (its marks put away, its head's words bold, set
    /// as its column is), and put over where they are drawn. Its table, or its cell, gone: left.
    fn dress_cell(&mut self) {
        let Some((editor, nth, row, column)) = self.cell_now() else { return };
        let Some((table, _)) = self.laid.get(nth) else {
            self.close_cell();
            return;
        };
        let Some(cell) = table.rows.get(row).and_then(|cells| cells.get(column)).filter(|cell| cell.written) else {
            self.close_cell();
            return;
        };
        let start = cell.words.start;
        let marks: Vec<_> = cell.marks().into_iter().map(|mark| mark.start - start..mark.end - start).collect();
        let looks = if row == 0 { vec![(0..cell.words.len(), crate::notes::markdown::Look::Bold)] } else { Vec::new() };
        // (Put where its words are first: dressed, they are laid out again in its box as wide as they are now.)
        self.place_cell();
        editor.dress(&looks, &marks, &[], &[], &[], &self.palette);
        editor.align(table.aligns[column]);
    }

    /// The table's cell written in put over where its words are drawn now (its table laid out again, the note's words
    /// scrolled).
    fn place_cell(&self) {
        let Some((editor, nth, row, column)) = self.cell_now() else { return };
        let Some((table, laid)) = self.laid.get(nth) else { return };
        let now: Vec<u16> = self.editor.text().encode_utf16().collect();
        let Some(top) = line_top(&self.editor, &now, table.span.start) else { return };
        if let Some(rect) = cell_box(&self.editor, laid, top, row, column) {
            editor.place(rect);
        }
    }

    /// The cell `row`, `column` of the note's `nth` table written in (one its row leaves out written first, with those
    /// before it), the caret in it as `placed` says: its words in a box of their own over where they are drawn (see
    /// `InCell`), given the keys. The caret of the note's words where the cell's are (put away there, not drawn).
    fn open_cell(&mut self, nth: usize, row: usize, column: usize, placed: Placed) {
        self.sync_cell();
        let Some(table) = self.laid.get(nth).map(|(table, _)| table.clone()) else { return };
        let Some(cell) = table.rows.get(row).and_then(|cells| cells.get(column)).cloned() else { return };
        if !cell.written {
            // (Tab from its row's last written cell writes the rest of its row.)
            let Some(last) = table.rows[row].iter().rposition(|cell| cell.written) else { return };
            let end = table.rows[row][last].room.end;
            if let Some(Some(edit)) = crate::notes::table::tab(&self.editor.words(), (end, end), false) {
                self.editor.apply(&edit);
                self.dress();
                if self.laid.get(nth).and_then(|(table, _)| table.rows.get(row)?.get(column)).is_some_and(|cell| cell.written) {
                    self.open_cell(nth, row, column, placed);
                }
            }
            return;
        }
        let units: Vec<u16> = self.editor.text().encode_utf16().collect();
        let words = String::from_utf16_lossy(&units[cell.words.clone()]);
        let editor = match self.cell_now() {
            Some((editor, ..)) => editor,
            None => {
                let Some(editor) = Editor::new(self.editor.window(), RECT::default(), &self.palette.face, self.pixels(), self.palette.text) else { return };
                let _ = self.editor.send(WM_KILLFOCUS, Default::default(), Default::default());
                let editor = Rc::new(editor);
                let _ = editor.send(WM_SETFOCUS, Default::default(), Default::default());
                editor
            }
        };
        editor.set_text(&words);
        *self.cell.borrow_mut() = Some(InCell { editor: editor.clone(), table: nth, row, column });
        self.editor.select((cell.words.start as i32, cell.words.start as i32));
        self.dress_cell();
        let length = editor.words().encode_utf16().count() as i32;
        let (from, to) = match placed {
            Placed::Start => (0, 0),
            Placed::End => (length, length),
            Placed::All => (0, length),
            Placed::At(point) => {
                // (Above its words, their first line, below them, their last.)
                let bounds = editor.bounds();
                let at = editor.at(POINT { x: point.x, y: point.y.clamp(bounds.top, (bounds.bottom - 1).max(bounds.top)) }) as i32;
                (at, at)
            }
        };
        editor.select((from, to));
    }

    /// What is written in the table's cell written in written into the note's words, in its cell's place (and the table
    /// laid out again with it), what is chosen in the note's words kept where it was: not while the input method
    /// composes in it.
    fn sync_cell(&mut self) {
        let Some((editor, nth, row, column)) = self.cell_now() else { return };
        if editor.composing() || !editor.take_changed() {
            return;
        }
        let Some(cell) = self.laid.get(nth).and_then(|(table, _)| table.rows.get(row)?.get(column)).cloned() else { return };
        let with = into_cell(&editor.words());
        let (range, grown) = (cell.words.clone(), with.encode_utf16().count() as isize - cell.words.len() as isize);
        let kept = |at: i32| {
            let at = at.max(0) as usize;
            if at >= range.end { (at as isize + grown) as usize } else { at.min(range.start) }
        };
        let (from, to) = self.editor.selection();
        self.editor.apply(&crate::notes::edit::Edit { range: cell.words, with, chosen: (kept(from), kept(to)) });
        self.dress();
    }

    /// The table's cell written in no longer written in (its words not written back: see `leave_cell`), the note's words
    /// given the keys again.
    fn close_cell(&mut self) {
        let Some(cell) = self.cell.borrow_mut().take() else { return };
        let _ = cell.editor.send(WM_KILLFOCUS, Default::default(), Default::default());
        let _ = self.editor.send(WM_SETFOCUS, Default::default(), Default::default());
    }

    /// The table's cell written in left: its words written into the note's, the note's words given the keys again, the
    /// caret after the end of the line before its table (`Some(true)`: one begun where there is none), at the start of
    /// the line after it (`Some(false)`: one begun where there is none), or where it is (none).
    fn leave_cell(&mut self, to: Option<bool>) {
        self.sync_cell();
        let Some((_, nth, ..)) = self.cell_now() else { return };
        self.close_cell();
        let Some(span) = self.laid.get(nth).map(|(table, _)| table.span.clone()) else { return };
        let units: Vec<u16> = self.editor.text().encode_utf16().collect();
        let begun = |at: usize, caret: usize| crate::notes::edit::Edit { range: at..at, with: "\n".into(), chosen: (caret, caret) };
        match to {
            Some(true) if span.start == 0 => self.editor.apply(&begun(0, 0)),
            Some(true) => {
                let at = (span.start - 1) as i32;
                self.editor.select((at, at));
            }
            Some(false) if units.get(span.end) == Some(&u16::from(b'\n')) && span.end + 1 < units.len() => {
                let at = (span.end + 1) as i32;
                self.editor.select((at, at));
            }
            Some(false) => self.editor.apply(&begun(span.end, span.end + 1)),
            None => {}
        }
    }

    /// What its box asked of it (see `Asked`), done.
    fn do_asked(&mut self) {
        let Some(asked) = self.asked.take() else { return };
        match asked {
            Asked::Cell(nth, row, column, placed) => self.open_cell(nth, row, column, placed),
            Asked::Leave(to) => self.leave_cell(to),
            Asked::RowAfter(column) => {
                self.sync_cell();
                let Some((_, nth, ..)) = self.cell_now() else { return };
                let Some((table, _)) = self.laid.get(nth) else { return };
                let edit = crate::notes::table::row_after(table);
                self.editor.apply(&edit);
                self.dress();
                if let Some(row) = self.laid.get(nth).map(|(table, _)| table.rows.len() - 1) {
                    self.open_cell(nth, row, column, Placed::Start);
                }
            }
            Asked::Link => self.open_link(),
            Asked::Table(change) => {
                self.sync_cell();
                let Some((_, nth, row, column)) = self.cell_now() else { return };
                let Some((table, _)) = self.laid.get(nth) else { return };
                let units: Vec<u16> = self.editor.text().encode_utf16().collect();
                let Some((edit, to)) = crate::notes::table::changed(&units, table, (row, column), change) else { return };
                self.editor.apply(&edit);
                self.dress();
                match to {
                    Some((row, column)) => self.open_cell(nth, row, column, Placed::End),
                    None => self.close_cell(),
                }
            }
        }
    }

    /// Where the caret of the note's words is in a table drawn laid out (put there by what was written, or moved there
    /// past what the keys would take it to), the cell it is in written in, or its first. Not where something is chosen,
    /// nor while a cell is written in.
    fn caret_into_cell(&mut self) {
        if self.in_cell() || crate::notes::source() {
            return;
        }
        let (from, to) = self.editor.selection();
        if from != to {
            return;
        }
        let at = from.max(0) as usize;
        let found = self.laid.iter().enumerate().find(|(_, (table, _))| table.span.start <= at && at < table.span.end).map(|(nth, (table, _))| (nth, crate::notes::table::cell_of(table, at)));
        if let Some((nth, cell)) = found {
            let (row, column) = cell.unwrap_or((0, 0));
            self.open_cell(nth, row, column, Placed::End);
        }
    }

    /// Its words written `pixels` high in `theme`'s inks (its window's scale changed, or its skin), dressed again so.
    pub fn restyle(&mut self, theme: &Theme, pixels: f32) {
        self.palette = palette_for(theme, self.colour, pixels);
        self.pixels.set(pixels);
        self.dress();
    }

    /// Of colour `colour` (chosen in its menu: its words in the ink of its colour, where its card's are), kept so in
    /// its folder where it is in a file.
    pub fn recolour(&mut self, theme: &Theme, colour: crate::notes::Colour) {
        self.colour = colour;
        self.palette = palette_for(theme, colour, self.palette.points / 0.75);
        self.dress();
        if let Some(name) = &self.name {
            let _ = crate::notes::folder_at(self.folder.clone()).colour(name, colour);
        }
        crate::notes::read_again();
    }

    /// How high its words are written (pixels).
    pub fn pixels(&self) -> f32 {
        self.palette.points / 0.75
    }

    /// Where its writing menu was asked for (see `writing_menu`), on the screen, taken: its words pressed with the
    /// right button there, or the menu key or Shift+F10 pressed (at the caret). Its window's to show.
    pub fn take_menu_asked(&self) -> Option<POINT> {
        self.menu_asked.take()
    }

    /// `how` done to its words, chosen in its writing menu (see `Write`): each change one undo.
    pub fn write(&self, how: Write) {
        use crate::notes::edit;
        // (A link, where it is written in: its words' or a table's cell's.)
        match how {
            Write::Link => self.open_link(),
            Write::CopyLink => {
                if let Some((_, _, found)) = self.link_here() {
                    crate::diagnostics::copy(self.editor.window(), &found.to);
                }
            }
            Write::Unlink => {
                if let Some((target, text, found)) = self.link_here() {
                    if let Some(edit) = edit::unlink(&text, &found) {
                        target.apply(&edit);
                    }
                }
            }
            _ => {}
        }
        if matches!(how, Write::Link | Write::CopyLink | Write::Unlink) {
            self.keyed.set(true);
            return;
        }
        // (A table's cell written in: cut, copied and pasted there, its table changed as it is next seen to.)
        if let Some((inside, ..)) = self.cell_now() {
            match how {
                Write::Cut => drop(inside.send(WM_CUT, Default::default(), Default::default())),
                Write::Copy => drop(inside.send(WM_COPY, Default::default(), Default::default())),
                Write::Paste => drop(paste_in_cell(self.editor.window(), &inside)),
                Write::Table(change) => self.asked.set(Some(Asked::Table(change))),
                _ => {}
            }
            self.keyed.set(true);
            return;
        }
        let (from, to) = self.editor.selection();
        let (text, chosen) = (self.editor.words(), (from.max(0) as usize, to.max(0) as usize));
        let owner = self.editor.window();
        let made = match how {
            Write::Cut | Write::Copy => {
                let _ = self.editor.send(if how == Write::Cut { WM_CUT } else { WM_COPY }, Default::default(), Default::default());
                None
            }
            Write::Paste => clipboard_pasted(owner).map(|pasted| edit::paste(&text, chosen, &pasted)),
            Write::Insert(block) => Some(edit::insert(&text, chosen, block)),
            Write::Heading(level) => Some(edit::heading(&text, chosen, level)),
            // (Only in a table's cell's menu. A link's done above.)
            Write::Table(_) | Write::Link | Write::CopyLink | Write::Unlink => None,
        };
        if let Some(edit) = made {
            self.editor.apply(&edit);
        }
        self.keyed.set(true);
    }

    /// What its words' box answers its window's messages with, given it whatever the window is in the middle of: an input
    /// method answers inside the message that asked it (the box placing its windows as it is drawn, say), and a
    /// composition the box ends (a press on its words) comes back inside that press. (Given only while the window was
    /// free, those went past the box: a composition it ended never ended in it, its words left in it as they were shown,
    /// the syllables' marks with them.) See `Answer`: Esc is the window's, but while a link is being written (see
    /// `link`), whose plate answers first.
    ///
    /// A to-do's box pressed is ticked, or no longer (as if typed: undo takes it back): drawn as a box (off the lines
    /// written on), pressed anywhere in its room, written out, on its `[ ]`. A link's words, where they show as it reads,
    /// pressed, are followed (a web address, or a file of the PC's). Over them the pointer is the hand.
    pub fn answer(&self) -> Answer {
        let (editor, reads, keyed, depth, eaten) = (self.editor.clone(), self.reads.clone(), self.keyed.clone(), self.depth.clone(), self.eaten.clone());
        let (pixels, pressing, over, menu_asked) = (self.pixels.clone(), self.pressing.clone(), self.over.clone(), self.menu_asked.clone());
        let cell = self.cell.clone();
        let asking = Asking { asked: self.asked.clone(), eaten: self.eaten.clone(), menu_asked: self.menu_asked.clone(), pressing: self.pressing_cell.clone() };
        let asked = self.asked.clone();
        let link = self.link.clone();
        Rc::new(move |hwnd, message, wparam, lparam| {
            let point = POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 };
            let pointer = || {
                let mut at = POINT::default();
                unsafe {
                    let _ = GetCursorPos(&mut at);
                    let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut at);
                }
                at
            };
            // A link being written: what is its answered there (see `link::Linking::answer`), Esc among it. As it ends,
            // its link written where it was to go, the keys given back there. (A press off it ends it, and is the note's.)
            let linking = link.try_borrow().ok().and_then(|held| held.clone());
            let was_linking = linking.is_some();
            if let Some(linking) = linking {
                depth.set(depth.get() + 1);
                let (answered, ending) = linking.answer(hwnd, (message, wparam, lparam));
                if let Some(ending) = ending {
                    link::end(&link, ending);
                    keyed.set(true);
                    // (Ended by a key, the character it makes is none of the words'.)
                    eaten.set(matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN));
                }
                depth.set(depth.get() - 1);
                if answered.is_some() {
                    if matches!(message, WM_KEYDOWN | WM_CHAR | WM_IME_COMPOSITION) {
                        keyed.set(true);
                    }
                    return answered;
                }
            }
            let its = match message {
                // (Esc is the window's: it leaves the words.)
                WM_KEYDOWN => wparam.0 != usize::from(VK_ESCAPE.0),
                // (Of the system's keys, Alt+Up and Alt+Down: lines moved. Shift+F10: the writing menu.)
                WM_SYSKEYDOWN => match VIRTUAL_KEY(wparam.0 as u16) {
                    VK_UP | VK_DOWN => true,
                    VK_F10 => (unsafe { GetKeyState(i32::from(VK_SHIFT.0)) }) < 0,
                    _ => false,
                },
                WM_KEYUP | WM_CHAR | WM_SYSCHAR | WM_UNICHAR | WM_SETFOCUS | WM_KILLFOCUS => true,
                WM_IME_STARTCOMPOSITION | WM_IME_COMPOSITION | WM_IME_ENDCOMPOSITION | WM_IME_NOTIFY | WM_IME_SETCONTEXT | WM_IME_CHAR | WM_IME_REQUEST | WM_IME_COMPOSITIONFULL | WM_IME_SELECT => true,
                WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_RBUTTONUP => editor.holds(point),
                // (A press begun in it is followed till let go, wherever the pointer goes.)
                WM_LBUTTONUP | WM_MOUSEMOVE => editor.holds(point) || unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetCapture() } == hwnd,
                WM_SETCURSOR => (lparam.0 & 0xFFFF) as u32 == HTCLIENT && editor.holds(pointer()),
                WM_MOUSEWHEEL | WM_TIMER => true,
                _ => false,
            };
            if !its {
                return None;
            }
            if matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN | WM_CHAR | WM_IME_COMPOSITION) {
                keyed.set(true);
            }
            // A key that writes Markdown (see `pressed`), decided as it goes down (with what is held with it), the
            // character it makes eaten after it: not while the input method composes (its keys are its).
            if message == WM_CHAR && eaten.replace(false) {
                return Some(Some(LRESULT(0)));
            }
            // A table's cell written in: what is its box's answered there (see `answer_cell`).
            let open = cell.try_borrow().ok().and_then(|cell| cell.as_ref().map(|cell| (cell.editor.clone(), cell.table, cell.row, cell.column)));
            let in_cell = open.is_some();
            if let Some((inside, nth, row, column)) = open {
                let size = reads.try_borrow().ok().and_then(|reads| reads.laid.get(nth).map(|(table, _)| (table.rows.len(), table.aligns.len())));
                if let Some(size) = size {
                    depth.set(depth.get() + 1);
                    let answered = answer_cell(hwnd, (message, wparam, lparam), &inside, (nth, row, column), size, &asking);
                    depth.set(depth.get() - 1);
                    if answered.is_some() {
                        return answered;
                    }
                }
            }
            if matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN) {
                eaten.set(false);
                let held = |key: VIRTUAL_KEY| unsafe { GetKeyState(i32::from(key.0)) } < 0;
                let held = Held { ctrl: held(VK_CONTROL), shift: held(VK_SHIFT), alt: held(VK_MENU) };
                let key = VIRTUAL_KEY(wparam.0 as u16);
                // An arrow that takes the caret into a table drawn laid out: its cell written in (see `into_table`).
                if message == WM_KEYDOWN && !held.ctrl && !held.shift && !held.alt && !editor.composing() {
                    let into = reads.try_borrow().ok().and_then(|reads| into_table(&editor, &reads.laid, key));
                    if let Some(into) = into {
                        asked.set(Some(into));
                        return Some(Some(LRESULT(0)));
                    }
                }
                // The writing menu asked for by its key, or Shift+F10: at the caret (its window's to show, see
                // `take_menu_asked`). Not while the input method composes.
                if (key == VK_APPS || key == VK_F10 && held.shift && !held.ctrl && !held.alt) && !editor.composing() {
                    let bounds = editor.bounds();
                    let mut at = editor.caret().map_or(POINT { x: bounds.left, y: bounds.top }, |(x, y, _, tall)| POINT { x, y: y + tall });
                    let _ = unsafe { windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut at) };
                    menu_asked.set(Some(at));
                    eaten.set(true);
                    return Some(Some(LRESULT(0)));
                }
                // Ctrl+K: a link written where the caret is (the one there edited), as it is next seen to.
                if message == WM_KEYDOWN && key == VK_K && held.ctrl && !held.shift && !editor.composing() {
                    asked.set(Some(Asked::Link));
                    eaten.set(true);
                    return Some(Some(LRESULT(0)));
                }
                let (from, to) = editor.selection();
                let made = (!editor.composing()).then(|| pressed(&editor.words(), (from.max(0) as usize, to.max(0) as usize), VIRTUAL_KEY(wparam.0 as u16), held, message == WM_SYSKEYDOWN, || clipboard_pasted(hwnd))).flatten();
                if let Some(made) = made {
                    eaten.set(true);
                    depth.set(depth.get() + 1);
                    if let Some(edit) = made {
                        editor.apply(&edit);
                    }
                    depth.set(depth.get() - 1);
                    return Some(Some(LRESULT(0)));
                }
            }
            // The format bar's, where it shows and the pointer is on it: a button pressed puts what it puts on the words
            // chosen (what is chosen chosen still), the pointer over it lights it, an arrow. Else what is pressed on the
            // words chooses (the bar not shown till it is let go).
            // (Not shown while a link was being written: what was pressed is the words'.)
            let at_bar = |point: POINT| bar_at(&editor, pixels.get(), pressing.get() || was_linking).filter(|bar| point.x >= bar.left && point.x < bar.right && point.y >= bar.top && point.y < bar.bottom);
            let on_bar = match message {
                WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_MOUSEMOVE => at_bar(point).map(|bar| bar_button(&bar, point, pixels.get())),
                WM_SETCURSOR => at_bar(pointer()).map(|_| None),
                _ => None,
            };
            if message == WM_MOUSEMOVE {
                over.set(on_bar.flatten());
            }
            if let Some(button) = on_bar {
                match (message, button) {
                    (WM_LBUTTONDOWN | WM_LBUTTONDBLCLK, Some(button)) => {
                        let (from, to) = editor.selection();
                        let (text, chosen) = (editor.words(), (from.max(0) as usize, to.max(0) as usize));
                        let made = match BAR[button].1 {
                            Format::Marked(how) => Some(crate::notes::edit::toggle(&text, chosen, how)),
                            // (A link written of them, as it is next seen to: see `link`.)
                            Format::Link => {
                                asked.set(Some(Asked::Link));
                                None
                            }
                        };
                        depth.set(depth.get() + 1);
                        if let Some(edit) = made {
                            editor.apply(&edit);
                        }
                        depth.set(depth.get() - 1);
                    }
                    (WM_SETCURSOR, _) => unsafe {
                        SetCursor(LoadCursorW(None, IDC_ARROW).ok());
                        return Some(Some(LRESULT(1)));
                    },
                    _ => {}
                }
                return Some(Some(LRESULT(0)));
            }
            // A table drawn laid out pressed on a cell (with either button): that cell written in, the caret where it was
            // pressed. Pressed off the cell written in, on none: it is left, the press the note's words'.
            if matches!(message, WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN) && !editor.composing() {
                match reads.try_borrow().ok().and_then(|reads| cell_at(&editor, &reads.laid, &reads.dressed, point)) {
                    Some((nth, row, column)) => {
                        asked.set(Some(Asked::Cell(nth, row, column, Placed::At(point))));
                        return Some(Some(LRESULT(0)));
                    }
                    None if in_cell => asked.set(Some(Asked::Leave(None))),
                    None => {}
                }
            }
            // The right button on the words: pressed off what is chosen, the caret there (as pressed on, it is what the
            // menu does its work on), let go, the writing menu asked for there.
            match message {
                WM_RBUTTONDOWN => {
                    let (from, to) = editor.selection();
                    let at = editor.at(point) as i32;
                    if !editor.composing() && (at < from.min(to) || at > from.max(to)) {
                        editor.select((at, at));
                    }
                    return Some(Some(LRESULT(0)));
                }
                WM_RBUTTONUP => {
                    let mut at = point;
                    let _ = unsafe { windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut at) };
                    menu_asked.set(Some(at));
                    return Some(Some(LRESULT(0)));
                }
                _ => {}
            }
            match message {
                WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => pressing.set(true),
                // (Let go, or moved with the button up: a press whose letting go went elsewhere is over too.)
                WM_LBUTTONUP => pressing.set(false),
                WM_MOUSEMOVE => pressing.set(pressing.get() && wparam.0 & 0x0001 != 0),
                _ => {}
            }
            depth.set(depth.get() + 1);
            let ticked = (message == WM_LBUTTONDOWN && !editor.composing())
                .then(|| {
                    let drawn = reads.try_borrow().ok().and_then(|reads| {
                        reads.shown.boxes.iter().find_map(|boxed| {
                            let room = editor.character(boxed.room)?;
                            (point.x >= room.left && point.x < room.right && point.y >= room.top && point.y < room.bottom).then(|| (boxed.marks.end - 3..boxed.marks.end, boxed.done))
                        })
                    });
                    drawn.or_else(|| {
                        let at = editor.at(point);
                        crate::notes::markdown::tasks(&editor.text()).into_iter().find(|(range, _)| range.start <= at && at < range.end)
                    })
                })
                .flatten();
            let link_under = |point: POINT| {
                let at = editor.at(point);
                let on = editor.character(at).is_some_and(|drawn| point.x >= drawn.left && point.x < drawn.right && point.y >= drawn.top && point.y < drawn.bottom);
                reads.try_borrow().ok().filter(|_| on).and_then(|reads| reads.shown.link_at(at).map(|link| (link.to.clone(), reads.folder.clone())))
            };
            let followed = match (ticked.is_none(), message) {
                (true, WM_LBUTTONDOWN) => link_under(point),
                _ => None,
            };
            let answered = match (ticked, followed, message) {
                (Some((range, done)), _, _) => {
                    editor.replace(range.start, range.end, if done { "[ ]" } else { "[x]" });
                    Some(LRESULT(0))
                }
                (None, Some((to, folder)), _) => {
                    crate::notes::follow_link(&to, &folder);
                    Some(LRESULT(0))
                }
                // (Its shape asked of the text services as their host asks it: see `Editor::set_cursor`. Over a link that
                // would be followed, the hand.)
                (None, None, WM_SETCURSOR) => {
                    match link_under(pointer()) {
                        Some(_) => unsafe {
                            SetCursor(LoadCursorW(None, IDC_HAND).ok());
                        },
                        None => editor.set_cursor(pointer()),
                    }
                    Some(LRESULT(1))
                }
                (None, None, _) => {
                    let answered = editor.send(message, wparam, lparam);
                    // The caret moved by a key, or put where the words were pressed: where words are, the way it went.
                    use crate::notes::markdown::Way;
                    let way = match (message, VIRTUAL_KEY(wparam.0 as u16)) {
                        (WM_KEYDOWN, VK_LEFT) => Some(Way::Left),
                        (WM_KEYDOWN, VK_RIGHT) => Some(Way::Right),
                        (WM_KEYDOWN, VK_UP | VK_PRIOR) => Some(Way::Up),
                        (WM_KEYDOWN, VK_DOWN | VK_NEXT) => Some(Way::Down),
                        (WM_KEYDOWN, VK_HOME | VK_END) | (WM_LBUTTONDOWN | WM_LBUTTONUP, _) => Some(Way::Near),
                        _ => None,
                    };
                    if let (Some(way), Ok(reads)) = (way, reads.try_borrow()) {
                        settle(&editor, &reads.shown, &reads.dressed, way, true);
                    }
                    answered
                }
            };
            depth.set(depth.get() - 1);
            // (The input method's, unanswered by the box, are the system's as well.)
            Some(match (answered, message) {
                (None, WM_IME_STARTCOMPOSITION | WM_IME_COMPOSITION | WM_IME_ENDCOMPOSITION | WM_IME_NOTIFY | WM_IME_SETCONTEXT | WM_IME_CHAR | WM_IME_REQUEST) => None,
                (answered, _) => Some(answered.unwrap_or(LRESULT(0))),
            })
        })
    }

    /// Whether its box is in the middle of a message (see `answer`): it is not to go now.
    pub fn answering(&self) -> bool {
        self.depth.get() > 0
    }

    /// Its own part of what was written in its box: its words dressed as their Markdown reads (not while the input
    /// method composes: its words are its till they are written), what is written marked as not kept. What its box asked
    /// of it done first (see `Asked`), what is written in a table's cell written into its words. The caret come into a
    /// table drawn laid out (by what was written, as a table is put in): the cell it is in written in. Whether it changed
    /// (to be kept a moment after: see `KEEP_AFTER_MS`).
    pub fn written(&mut self) -> bool {
        self.do_asked();
        self.sync_cell();
        let composed = self.editor.take_composed();
        let changed = self.editor.take_changed();
        let keyed = self.keyed.replace(false);
        let composing = self.editor.composing();
        // (Dressed as it is now already, by what was asked or written in a cell: not again.)
        let fresh = || self.editor.text().encode_utf16().eq(self.dressed.iter().copied());
        if composed || (!composing && ((changed && !fresh()) || self.source != crate::notes::source())) {
            self.dress();
            // (Written in: the caret where words are, a line begun after one put away whole, as a rule just written.)
            if keyed && changed && !self.in_cell() {
                settle(&self.editor, &self.shown, &self.dressed, crate::notes::markdown::Way::Near, true);
            }
        }
        if !composing {
            self.caret_into_cell();
        }
        if changed {
            self.unkept = true;
            self.said = None;
        }
        changed
    }

    /// Kept in its file, where what is written is not kept yet (see `notes::Folder::write`): a new one named by its title
    /// as it is first kept (an empty one never), one changed outside meanwhile kept apart (said so). Whether all that was
    /// written is kept now (a failure leaves it not kept, saying so).
    pub fn keep(&mut self, lang: Lang) -> bool {
        if !self.unkept {
            return true;
        }
        let text = self.editor.text();
        let folder = crate::notes::folder_at(self.folder.clone());
        let name = match &self.name {
            Some(name) => name.clone(),
            None if text.trim().is_empty() => {
                self.unkept = false;
                return true;
            }
            None => {
                let title = crate::notes::Note { name: String::new(), text: text.clone(), colour: self.colour, stamp: Default::default() }.title();
                let today = super::text::Local::at(crate::station::now_ms());
                let named = folder.name_for(&title, &match lang {
                    Lang::Zh => format!("便签 {}月{}日", today.month, today.day),
                    Lang::En => format!("Note {}-{}", today.month, today.day),
                });
                let _ = folder.colour(&named, self.colour);
                named
            }
        };
        match folder.write(&name, &text, self.stamp) {
            Ok(crate::notes::Written::Done(stamp)) => {
                self.name = Some(name);
                self.stamp = Some(stamp);
                self.written_ms = stamp.written_ms;
            }
            Ok(crate::notes::Written::Apart(apart, stamp)) => {
                self.said = Some(match lang {
                    Lang::Zh => format!("这条便签在别处被改过或删除，你写的已另存为“{apart}”"),
                    Lang::En => format!("Changed or deleted elsewhere meanwhile: what you wrote is kept as \"{apart}\""),
                });
                // (Its stamp of the bytes written here: read again, another's change meanwhile would be taken for them.)
                self.stamp = Some(stamp);
                self.name = Some(apart);
            }
            Err(_) => {
                self.said = Some(lang.pick("未能保存，请检查便签文件夹，稍后将再次尝试", "Could not be saved. Check the notes folder, it will be tried again").to_string());
                return false;
            }
        }
        self.unkept = false;
        crate::notes::read_again();
        true
    }

    /// Glance going (or the note's window), with what is written not kept and not to be: put beside Glance's settings,
    /// in "unsaved notes", not lost with it (and said so in the journal).
    pub fn put_aside(&self) {
        let stem = self.name.as_deref().and_then(|name| std::path::Path::new(name).file_stem()).map_or_else(|| "note".to_string(), |stem| stem.to_string_lossy().into_owned());
        let dir = crate::settings::config_dir().join("unsaved notes");
        let path = dir.join(format!("{stem} {}.md", crate::station::now_ms()));
        let put = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, self.editor.text()));
        crate::journal::note(match put {
            Ok(()) => format!("note: what was written could not be kept; it is in {}", path.display()),
            Err(error) => format!("note: what was written could not be kept, nor put in {} ({error})", path.display()),
        });
    }

    /// As its file is now, where that was changed outside and nothing written here waits to be kept (or is being
    /// composed): read again, what was chosen kept as far as the words go. (With something waiting, it is kept apart as it
    /// is kept: see `keep`. Deleted outside: left as it is, kept apart likewise.)
    pub fn read_outside(&mut self) {
        let Some((folder, name)) = self.to_read() else { return };
        let Some((text, stamp)) = crate::notes::folder_at(folder).read(&name) else { return };
        self.take_outside(&text, stamp);
    }

    /// Where its file is to be read for a change made outside (its folder and its name), while it may be read again:
    /// not over what is written here and not yet taken in, in its words or in a table's cell, nor as the input method
    /// composes in either (taken in, it is kept, and that is read again then). None for a note in no file yet.
    pub fn to_read(&self) -> Option<(std::path::PathBuf, String)> {
        let cell_busy = self.cell.borrow().as_ref().is_some_and(|cell| cell.editor.composing() || cell.editor.changed());
        let busy = self.unkept || self.editor.composing() || self.editor.changed() || cell_busy;
        Some((self.folder.clone(), self.name.clone()?)).filter(|_| !busy)
    }

    /// Its file as read (`text`, as `stamp` says it was) taken in, where it changed since it was last read or kept, and
    /// it may still be read again (read elsewhere meanwhile: see `to_read`).
    pub fn take_outside(&mut self, text: &str, stamp: crate::notes::store::Stamp) {
        if self.to_read().is_none() || self.stamp.is_some_and(|was| was.same(&stamp)) {
            return;
        }
        // (A link being written there left: what it was for is written anew.)
        link::end(&self.link, link::Ending::Left);
        // (A cell written in left as it was: its words, as they were, are not the note's now. Opened again on the
        // words read, where the caret still is in its table: see `written`.)
        self.close_cell();
        let chosen = self.editor.selection();
        self.editor.set_text(text);
        self.editor.select(chosen);
        self.stamp = Some(stamp);
        self.written_ms = stamp.written_ms;
        self.dress();
    }

    /// Whether the notes as last read show it as it is written here (kept, and read again since): drawn from them, it
    /// shows as it does here.
    pub fn read_as_written(&self) -> bool {
        let Some(name) = &self.name else { return !self.unkept };
        !self.unkept && crate::notes::shown().iter().find(|note| &note.name == name).is_some_and(|note| note.text.replace("\r\n", "\n").trim_end() == self.editor.text().trim_end())
    }

    /// Its page `width` × `height` from (`x`, `y`) on `frame` (DIPs), its window's corner at `window` on it and `scale`
    /// pixels a DIP: its face (as a note stuck on the desktop is, see `paint_stuck`: of its colour as its card on the
    /// panel is, its dot by when it was written, at its foot how many characters, kept as it is written, and where, or in
    /// place of them what was said of keeping it, in the foot's whole width, on two lines where it needs them), its words
    /// as the text services draw them, its to-dos' boxes in the rooms made for them (off the lines written on), and the
    /// caret where they say it is (`caret_on`: in its blink, shown).
    #[allow(clippy::too_many_arguments)]
    pub fn paint(&self, frame: &super::gfx::Frame, theme: &Theme, lang: Lang, (x, y, width, height): (f32, f32, f32, f32), window: (f32, f32), scale: f32, caret_on: bool) {
        let when = super::text::since(self.written_ms, crate::station::now_ms(), lang);
        let said_now = said(self.name.as_ref().map(|_| when.as_str()), lang);
        let counted = counted(&self.editor.text(), lang);
        face(frame, theme, self.colour, &said_now, self.said.is_none().then_some(counted.as_str()), self.name.as_deref(), (x, y, width, height));
        if let Some(said) = &self.said {
            let bottom = y + height - FOOT;
            let tall = frame.gfx.wrapped_height(said, theme.small, width - 2.0 * PAD).min(FOOT - 8.0);
            frame.text_wrapped(said, theme.small, theme.signal, x + PAD, bottom + (FOOT - tall) / 2.0, width - 2.0 * PAD);
        }
        // (Its box placed in its window: drawn from the window's corner.)
        let in_window = |px: f32, py: f32| (window.0 + px / scale, window.1 + py / scale);
        let pixels = self.pixels();
        let inks = word_inks(theme, self.colour);
        let bounds = self.editor.bounds();
        let (left, top) = in_window(bounds.left as f32, bounds.top as f32);
        let (right, bottom) = in_window(bounds.right as f32, bounds.bottom as f32);
        // (Where what was dressed is now: the input method's words may be in them, not yet dressed.)
        let now: Vec<u16> = self.editor.text().encode_utf16().collect();
        let moved = Moved::between(&self.dressed, &now);
        // Its blocks of code's boxes, under its words, reaching out past their sides (as a note shown draws them: see
        // `Frame::markdown`, its words whole pixels high).
        let out = crate::notes::markdown::CODE_OUT * pixels.round() / scale;
        frame.clip(left - out - 1.0, top, right - left + 2.0 * out + 2.0, bottom - top);
        for code in &self.shown.codes {
            let first = moved.at(code.span.start).and_then(|start| line_top(&self.editor, &now, start));
            let last = moved.at(code.span.end).and_then(|end| line_foot(&self.editor, &now, end));
            if let (Some(first), Some(last)) = (first, last) {
                let (x, y) = in_window(bounds.left as f32, first as f32);
                super::canvas::code_box(frame, inks, x - out, y, right - left + 2.0 * out, (last - first) as f32 / scale, pixels.round() / scale);
            }
        }
        frame.unclip();
        let mut was = windows_numerics::Matrix3x2::default();
        unsafe {
            frame.dc.GetTransform(&mut was);
            frame.dc.SetTransform(&(windows_numerics::Matrix3x2::translation(window.0, window.1) * was));
        }
        frame.in_pixels(|target| self.editor.draw(target));
        unsafe { frame.dc.SetTransform(&was) };
        frame.clip(left, top, right - left, bottom - top);
        // (A cell written in: its words its box's, drawn over where the table leaves them out, where they are now.)
        let open = self.cell_now();
        self.place_cell();
        // (What is chosen of its words, where no cell is written in: the cells it takes in shown chosen, as its words
        // are. The table's words are put away under it: the text services showed only a sliver of them chosen.)
        let (from, to) = self.editor.selection();
        let chosen_ink = super::editor::chosen_ink();
        for (nth, (table, laid)) in self.laid.iter().enumerate() {
            let Some(line) = moved.at(table.span.start).and_then(|start| line_top(&self.editor, &now, start)) else { continue };
            let (x, y) = in_window(bounds.left as f32, line as f32);
            let written_in = open.as_ref().filter(|(_, table, ..)| *table == nth).map(|(_, _, row, column)| (*row, *column));
            let chosen = |row: usize, column: usize| cell_chosen(table, &moved, (from, to), row, column);
            frame.table(laid, inks, x, y, (top, bottom), written_in, (open.is_none() && from < to).then_some((chosen_ink, &chosen as &dyn Fn(usize, usize) -> bool)));
        }
        if let Some((inside, ..)) = &open {
            unsafe { frame.dc.SetTransform(&(windows_numerics::Matrix3x2::translation(window.0, window.1) * was)) };
            frame.in_pixels(|target| inside.draw(target));
            unsafe { frame.dc.SetTransform(&was) };
        }
        // (Each where it is drawn or would be: one only partly in the box drawn as far as it is, as a note shown draws it.)
        for rule in &self.shown.rules {
            let Some(line) = moved.at(rule.end).and_then(|end| self.editor.character_anywhere(end)) else { continue };
            let (x, y) = in_window(bounds.left as f32, line.top as f32);
            frame.rule(inks, x, y, (bounds.right - bounds.left) as f32 / scale, (line.bottom - line.top) as f32);
        }
        for boxed in &self.shown.boxes {
            let Some(room) = moved.at(boxed.room) else { continue };
            let (Some(room), Some(baseline)) = (self.editor.character_anywhere(room), self.editor.baseline_anywhere(room + 1)) else { continue };
            let (bx, by, side) = box_in(room.left, baseline, pixels.round());
            let (bx, by) = in_window(bx, by);
            super::canvas::task_box(frame, inks, bx, by, side / scale, boxed.done);
        }
        // Each list's dot and quote's bars in its room (the bars all its line's rows down, the next line's joining them).
        for roomed in &self.shown.roomed {
            let Some(at) = moved.at(roomed.room) else { continue };
            let Some(room) = self.editor.character_anywhere(at) else { continue };
            let (x, y) = in_window(room.left as f32, room.top as f32);
            match roomed.drawn {
                crate::notes::markdown::Drawn::Bullet => {
                    let Some(baseline) = self.editor.baseline_anywhere(at + 1) else { continue };
                    super::canvas::bullet(frame, inks, x, in_window(0.0, baseline as f32).1, pixels.round() / scale);
                }
                crate::notes::markdown::Drawn::Bars(deep) => {
                    let Some(foot) = line_foot(&self.editor, &now, at) else { continue };
                    super::canvas::quote_bars(frame, inks, (x, y, (foot - room.top) as f32 / scale), deep, pixels.round() / scale, scale);
                }
            }
        }
        frame.unclip();
        let linking = self.link.borrow().clone();
        if let Some(bar) = bar_at(&self.editor, self.pixels.get(), self.pressing.get()).filter(|_| linking.is_none()) {
            let per = self.pixels.get() / WORDS / scale;
            let (left, top) = in_window(bar.left as f32, bar.top as f32);
            let (right, bottom) = in_window(bar.right as f32, bar.bottom as f32);
            let plate = super::menu::plate(frame, super::menu::Look::of(theme.skin, theme.dark), (left, top, right - left, bottom - top));
            for (nth, (icon, _)) in BAR.iter().enumerate() {
                let (x, y, side) = (left + (BAR_INSET + nth as f32 * BAR_BUTTON) * per, top + BAR_INSET * per, BAR_BUTTON * per);
                if self.over.get() == Some(nth) {
                    frame.fill_rounded(plate.highlight, x, y, side, side, (plate.radius - BAR_INSET * per).max(2.0));
                }
                frame.icon(*icon, Point { x: x + side / 2.0, y: y + side / 2.0 }, 16.0 * per, plate.icon);
            }
        }
        let caret = match &open {
            Some((inside, ..)) => inside.caret(),
            None => self.editor.caret(),
        };
        if let (Some((cx, cy, cw, ch)), true, None) = (caret, caret_on, &linking) {
            let (cx, cy) = in_window(cx as f32, cy as f32);
            frame.fill(theme.text, cx, cy, (cw as f32 / scale).max(1.2), ch as f32 / scale);
        }
        // A link being written: over all of it, its caret in the field that has the keys.
        if let Some(linking) = linking {
            linking.paint(frame, theme, lang, window, scale, caret_on);
        }
    }
}

#[cfg(all(test, feature = "studio"))]
mod pictures {
    use super::*;

    /// How many pixels of a note's words' room, written `text`, differ drawn written in and not (stuck on the desktop), at
    /// each of the screen's usual scales. Each drawing is written to `target/beside/alike-<name>-<scale>-<page|face>.png`.
    fn drawn_apart(text: &str, name: &str) -> Vec<(f32, usize)> {
        drawn_apart_in(text, name, &[])
    }

    /// As `drawn_apart`, a table's cell written in as its words are drawn (each of `cells` in turn: its table, row and
    /// column, the caret not shown).
    fn drawn_apart_in(text: &str, name: &str, cells: &[(usize, usize, usize)]) -> Vec<(f32, usize)> {
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_POPUP};
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        let gfx = super::super::gfx::current().unwrap();
        let out = std::path::PathBuf::from(format!("{}/target/beside", env!("CARGO_MANIFEST_DIR")));
        std::fs::create_dir_all(&out).unwrap();
        let folder = std::env::temp_dir().join(format!("glance-note-alike-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("a.md"), text).unwrap();
        let (w, h) = super::super::view::STUCK_NOTE;
        let hwnd = unsafe { CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WS_POPUP, 0, 0, (w * 2.0) as i32, (h * 2.0) as i32, None, None, None, None) }.unwrap();
        let mut apart = Vec::new();
        for scale in [1.0f32, 1.25, 1.5, 2.0] {
            let size = ((w * scale) as u32, (h * scale) as u32);
            let theme = Theme::new(Skin::Fluent, true);
            let mut writing = Writing::open(hwnd, words_box(0, 0, size.0 as i32, size.1 as i32, scale), &crate::notes::store::Folder { path: folder.clone(), apart: " (conflicted copy)" }, Some("a.md".into()), &theme, WORDS * scale).unwrap();
            let not = gfx.draw_offscreen(size, scale, |frame| paint_stuck(frame, &theme, Lang::Zh, text, writing.colour, "x", "a.md", (0.0, 0.0, w, h), 0.0)).unwrap();
            crate::studio::save_png(&out.join(format!("alike-{name}-{scale}-face.png")), size.0, size.1, &not).unwrap();
            let opened: Vec<Option<(usize, usize, usize)>> = if cells.is_empty() { vec![None] } else { cells.iter().copied().map(Some).collect() };
            for cell in opened {
                if let Some((nth, row, column)) = cell {
                    writing.open_cell(nth, row, column, Placed::End);
                    assert_eq!(writing.cell_now().map(|(_, nth, row, column)| (nth, row, column)), cell);
                }
                let written = gfx.draw_offscreen(size, scale, |frame| writing.paint(frame, &theme, Lang::Zh, (0.0, 0.0, w, h), (0.0, 0.0), scale, false)).unwrap();
                let which = cell.map_or(String::new(), |(nth, row, column)| format!("-cell{nth}.{row}.{column}"));
                crate::studio::save_png(&out.join(format!("alike-{name}{which}-{scale}-page.png")), size.0, size.1, &written).unwrap();
                // Over its words' room: how many of its pixels differ.
                let (left, top, wide, tall) = words_in(w, h);
                let differ = ((top * scale) as u32..((top + tall) * scale) as u32)
                    .flat_map(|y| ((left * scale) as u32..((left + wide) * scale) as u32).map(move |x| ((y * size.0 + x) * 4) as usize))
                    .filter(|&at| written[at..at + 4] != not[at..at + 4])
                    .count();
                apart.push((scale, differ));
            }
        }
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        let _ = std::fs::remove_dir_all(&folder);
        // (Its device let go of before the thread ends: else it hung there.)
        drop(gfx);
        super::super::gfx::let_go();
        apart
    }

    /// A note's words drawn not written in (stuck on the desktop) as they are drawn written in, pixel for pixel, at each
    /// of the screen's usual scales: as the text services draw them (else, written in, they came out sharper).
    #[test]
    #[ignore = "makes a window of this session's"]
    fn its_words_are_drawn_alike_written_in_or_not() {
        let apart = drawn_apart("天下\n你是我的我是你的谁哦\nThe quick brown fox, 1% low", "words");
        assert!(apart.iter().all(|(_, differ)| *differ == 0), "pixels apart at each scale: {apart:?}");
    }

    /// A note's tables and rules drawn not written in as they are drawn written in, pixel for pixel, at each of the
    /// screen's usual scales: a table laid out in the room its lines leave, the words under it where they are, a rule
    /// through its line's middle. A table in a quote is left as written.
    #[test]
    #[ignore = "makes a window of this session's"]
    fn tables_and_rules_are_drawn_alike_written_in_or_not() {
        let text = "上面一行\n\n| 名称 | 数量 | 说明 |\n| :-- | :-: | --: |\n| 苹果 | 3 | 红的，很甜，放在厨房的篮子里面 |\n| pear | 12 |  |\n\n---\n下面一行\n> | 引 | 用 |\n> | - | - |";
        let apart = drawn_apart(text, "table");
        assert!(apart.iter().all(|(_, differ)| *differ == 0), "pixels apart at each scale: {apart:?}");
    }

    /// A table's cell written in drawn as the table draws its words, pixel for pixel, at each of the screen's usual
    /// scales: a head's (bold), one set in the middle, one set right, one whose words wrap, one empty, one with marks put
    /// away (emphasis, code, an escaped pipe).
    #[test]
    #[ignore = "makes a window of this session's"]
    fn a_table_s_cell_written_in_is_drawn_alike() {
        let text = "| 名称 | 数量 | 说明 |\n| :-- | :-: | --: |\n| 苹果 | 3 | 红的，很甜，放在厨房的篮子里面 |\n| **pear** | `12` | a\\|b |\n|  | 1 | 2 |";
        let apart = drawn_apart_in(text, "cell", &[(0, 0, 1), (0, 1, 1), (0, 1, 2), (0, 2, 0), (0, 2, 1), (0, 2, 2), (0, 3, 0)]);
        assert!(apart.iter().all(|(_, differ)| *differ == 0), "pixels apart at each scale and cell: {apart:?}");
    }

    /// A note's lists' dots, quotes' bars, to-dos' boxes and blocks of code's boxes drawn not written in as they are drawn
    /// written in, pixel for pixel, at each of the screen's usual scales: each in its room, a list's item's and a quote's
    /// words wrapped under each other, a block of code's fences set low round its words, its box reaching out past their
    /// sides. A heading's and bold words' lines set by the plain words' baseline (as the box sets them), a done to-do's
    /// faint words in the ink the box has.
    #[test]
    #[ignore = "makes a window of this session's"]
    fn lists_quotes_and_code_are_drawn_alike_written_in_or_not() {
        let lists = "## 清单\n- 苹果\n  - 红的\n- [ ] 买牛奶\n1. 第一\n\n> 引用的话，很长很长很长很长很长很长很长很长很长很长很长很长\n> > 再引\n> 又一行";
        let code = "上面一行\n```rust\nlet a = 1;\n\nlet b = \"一段很长很长很长很长很长很长很长很长的字\";\n```\n下面一行\n\n    缩进的代码";
        let one_by_one = [("## 清单\n正文", "heading"), ("清单\n---\n正文", "underlined"), ("**粗体**\n正文", "bold"), ("- [x] 完成\n正文", "done"), ("> 引用\n\n正文", "quote"), ("```c\n#include 你好\n```", "code-last")];
        for (text, name) in one_by_one.into_iter().chain([(lists, "lists"), (code, "code")]) {
            let apart = drawn_apart(text, name);
            assert!(apart.iter().all(|(_, differ)| *differ == 0), "{name}: pixels apart at each scale: {apart:?}");
        }
    }

    /// Chinese typed into a block of code (its face has none) is drawn in the face the system falls back on, written in as
    /// not, pixel for pixel, not as boxes. Written to `target/beside/typed-code-<page|face>.png`.
    #[test]
    #[ignore = "makes a window of this session's"]
    fn chinese_typed_into_code_is_drawn() {
        use windows::core::w;
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WM_CHAR, WS_POPUP};
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        let gfx = super::super::gfx::current().unwrap();
        let out = std::path::PathBuf::from(format!("{}/target/beside", env!("CARGO_MANIFEST_DIR")));
        std::fs::create_dir_all(&out).unwrap();
        let folder = std::env::temp_dir().join(format!("glance-note-typed-code-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("a.md"), "```c\n#include \n```\n").unwrap();
        let (w, h) = super::super::view::STUCK_NOTE;
        let hwnd = unsafe { CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WS_POPUP, 0, 0, w as i32, h as i32, None, None, None, None) }.unwrap();
        let theme = Theme::new(Skin::Fluent, true);
        let mut writing = Writing::open(hwnd, words_box(0, 0, w as i32, h as i32, 1.0), &crate::notes::store::Folder { path: folder.clone(), apart: " (conflicted copy)" }, Some("a.md".into()), &theme, WORDS).unwrap();
        writing.editor.select((14, 14));
        let answer = writing.answer();
        for character in "你好".encode_utf16() {
            answer(hwnd, WM_CHAR, WPARAM(usize::from(character)), LPARAM(1));
        }
        writing.written();
        let text = writing.editor.text();
        assert_eq!(text.trim_end_matches('\n'), "```c\n#include 你好\n```");
        let size = (w as u32, h as u32);
        let written = gfx.draw_offscreen(size, 1.0, |frame| writing.paint(frame, &theme, Lang::Zh, (0.0, 0.0, w, h), (0.0, 0.0), 1.0, false)).unwrap();
        let not = gfx.draw_offscreen(size, 1.0, |frame| paint_stuck(frame, &theme, Lang::Zh, text.strip_suffix('\n').unwrap_or(&text), writing.colour, "x", "a.md", (0.0, 0.0, w, h), 0.0)).unwrap();
        crate::studio::save_png(&out.join("typed-code-page.png"), size.0, size.1, &written).unwrap();
        crate::studio::save_png(&out.join("typed-code-face.png"), size.0, size.1, &not).unwrap();
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        let _ = std::fs::remove_dir_all(&folder);
        // Over its words' room: how many of its pixels differ.
        let (left, top, wide, tall) = words_in(w, h);
        let differ = (top as u32..(top + tall) as u32).flat_map(|y| (left as u32..(left + wide) as u32).map(move |x| ((y * size.0 + x) * 4) as usize)).filter(|&at| written[at..at + 4] != not[at..at + 4]).count();
        drop(gfx);
        super::super::gfx::let_go();
        assert_eq!(differ, 0);
    }

    /// A note too long for its card, not written in, scrolls as far as the box it is written in scrolls it, and shows
    /// there what that box shows, pixel for pixel: its lines, a list, a quote, a block of code and a table scrolled
    /// past its top among them, at each scale. Short, it does not scroll.
    #[test]
    #[ignore = "makes a window of this session's"]
    fn a_long_note_scrolls_as_its_box_does() {
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_POPUP};
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        let gfx = super::super::gfx::current().unwrap();
        let text = format!(
            "## 很长的便签\n{}- 苹果\n  - 红的\n- [ ] 买牛奶\n> 引用的话，很长很长很长很长很长很长很长很长很长很长很长很长\n| 名称 | 说明 |\n| - | - |\n| 苹果 | 红的，很甜 |\n```rust\nlet a = 1;\n```\n{}最后一行",
            "第一段很长很长很长很长很长很长很长很长很长很长很长很长很长的字\n".repeat(6),
            "又一行\n".repeat(12)
        );
        let folder = std::env::temp_dir().join(format!("glance-note-long-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("a.md"), &text).unwrap();
        let theme = Theme::new(Skin::Fluent, true);
        let (mut differs, mut reach) = (Vec::new(), Vec::new());
        for scale in [1.0f32, 1.5] {
            let (w, h) = super::super::view::STUCK_NOTE;
            let size = ((w * scale) as u32, (h * scale) as u32);
            let hwnd = unsafe { CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WS_POPUP, 0, 0, size.0 as i32, size.1 as i32, None, None, None, None) }.unwrap();
            let writing = Writing::open(hwnd, words_box(0, 0, size.0 as i32, size.1 as i32, scale), &crate::notes::store::Folder { path: folder.clone(), apart: " (conflicted copy)" }, Some("a.md".into()), &theme, WORDS * scale).unwrap();
            let words = writing.editor.text();
            let words = words.strip_suffix('\n').unwrap_or(&words);
            // As far as it scrolls: as far as its box does.
            let most = std::cell::Cell::new(0.0);
            gfx.draw_offscreen(size, scale, |frame| most.set(stuck_scrolls(frame, words, (w, h)))).unwrap();
            let most = most.get();
            // Its words reach as far down as the box sets its last line's foot.
            let (_, top, _, tall) = words_in(w, h);
            let foot = writing.editor.character_anywhere(words.encode_utf16().count() - 1).map_or(0.0, |drawn| drawn.bottom as f32 / scale - top);
            reach.push((scale, most + tall, foot));
            // Part way, and to its foot: the same pixels.
            for down in [((most * 0.45) * scale).round() as i32, (most * scale).round() as i32] {
                writing.editor.scroll_to(down);
                let scrolled = writing.editor.scrolled() as f32 / scale;
                let written = gfx.draw_offscreen(size, scale, |frame| writing.paint(frame, &theme, Lang::Zh, (0.0, 0.0, w, h), (0.0, 0.0), scale, false)).unwrap();
                let not = gfx.draw_offscreen(size, scale, |frame| paint_stuck(frame, &theme, Lang::Zh, words, writing.colour, "x", "a.md", (0.0, 0.0, w, h), scrolled)).unwrap();
                let (left, top, wide, tall) = words_in(w, h);
                let (left, top, wide, tall) = ((left * scale) as u32, (top * scale).ceil() as u32, (wide * scale) as u32, (tall * scale) as u32);
                let differ = (top..top + tall).flat_map(|y| (left..left + wide).map(move |x| ((y * size.0 + x) * 4) as usize)).filter(|&at| written[at..at + 4] != not[at..at + 4]).count();
                if differ > 0 {
                    let out = std::path::PathBuf::from(format!("{}/target/beside", env!("CARGO_MANIFEST_DIR")));
                    std::fs::create_dir_all(&out).unwrap();
                    crate::studio::save_png(&out.join(format!("long-{scale}-{down}-page.png")), size.0, size.1, &written).unwrap();
                    crate::studio::save_png(&out.join(format!("long-{scale}-{down}-face.png")), size.0, size.1, &not).unwrap();
                }
                differs.push((scale, down, differ));
            }
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
        let short = std::cell::Cell::new(-1.0);
        gfx.draw_offscreen((10, 10), 1.0, |frame| short.set(stuck_scrolls(frame, "短的", super::super::view::STUCK_NOTE))).unwrap();
        let short = short.get();
        let _ = std::fs::remove_dir_all(&folder);
        drop(gfx);
        super::super::gfx::let_go();
        assert_eq!(short, 0.0);
        assert!(reach.iter().all(|(scale, reach, foot)| *reach > 400.0 && (reach - foot).abs() <= 1.0 / scale + 0.01), "how far down its words reach, and its box' last line' foot: {reach:?}");
        assert!(differs.iter().all(|(.., differ)| *differ == 0), "pixels apart: {differs:?}");
    }

    /// A table below its box's room is found where it would be (below the box, not at its top: the text services'
    /// `S_FALSE`, its point unset, taken for none), what is not drawn is not found drawn, and the box made narrower lays
    /// the table out again in the room it has.
    #[test]
    #[ignore = "makes a window of this session's"]
    fn a_table_out_of_the_box_and_the_box_made_narrower() {
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_POPUP};
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        let text = format!("{}| 名称 | 说明 |\n| - | - |\n| 苹果 | 红的，很甜，放在厨房的篮子里面，还有很多别的水果 |\n", "行\n".repeat(60));
        let folder = std::env::temp_dir().join(format!("glance-note-out-of-box-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("a.md"), &text).unwrap();
        let (w, h) = super::super::view::STUCK_NOTE;
        let hwnd = unsafe { CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WS_POPUP, 0, 0, w as i32, h as i32, None, None, None, None) }.unwrap();
        let theme = Theme::new(Skin::Fluent, true);
        let mut writing = Writing::open(hwnd, words_box(0, 0, w as i32, h as i32, 1.0), &crate::notes::store::Folder { path: folder.clone(), apart: " (conflicted copy)" }, Some("a.md".into()), &theme, WORDS).unwrap();
        let units: Vec<u16> = text.encode_utf16().collect();
        let start = writing.laid[0].0.span.start;
        let bounds = writing.editor.bounds();
        assert!(line_top(&writing.editor, &units, start).unwrap() >= bounds.bottom);
        assert!(writing.editor.character(start).is_none());
        let wide = writing.laid[0].1.laid.width();
        let half = (bounds.right - bounds.left) / 2;
        writing.place(RECT { right: bounds.left + half, ..bounds });
        let narrower = writing.laid[0].1.laid.width();
        assert!(narrower < wide && narrower <= half as f32, "{wide} to {narrower} in {half}");
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        let _ = std::fs::remove_dir_all(&folder);
        super::super::gfx::let_go();
    }

    /// A link being written on its plate, in each skin, light and dark, in Chinese and English: under the words chosen,
    /// its words filled in, where it goes empty (its button faint), then filled in. (`cargo test --features studio
    /// --lib -- --ignored a_link_s_plate` writes `target/beside/link-<skin>-<light|dark>-<zh|en>-<empty|filled>.png`.)
    #[test]
    #[ignore = "writes pictures to look at"]
    fn a_link_s_plate() {
        let _device = crate::ui::gfx::held().unwrap();
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_POPUP};
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        let gfx = super::super::gfx::current().unwrap();
        let out = std::path::PathBuf::from(format!("{}/target/beside", env!("CARGO_MANIFEST_DIR")));
        std::fs::create_dir_all(&out).unwrap();
        let folder = std::env::temp_dir().join(format!("glance-note-link-pictures-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let text = "# 本周要做\n看一下这份文档，再回复\n- [ ] 买牛奶";
        std::fs::write(folder.join("a.md"), text).unwrap();
        let (w, h) = super::super::view::STUCK_NOTE;
        let scale = 1.5;
        let size = ((w * scale) as u32, (h * scale) as u32);
        let hwnd = unsafe { CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WS_POPUP, 0, 0, size.0 as i32, size.1 as i32, None, None, None, None) }.unwrap();
        for (skin, name) in [(Skin::Glass, "glass"), (Skin::Fluent, "fluent"), (Skin::Paper, "paper")] {
            for dark in [false, true] {
                for lang in [Lang::Zh, Lang::En] {
                    let theme = Theme::new(skin, dark);
                    let writing = Writing::open(hwnd, words_box(0, 0, size.0 as i32, size.1 as i32, scale), &crate::notes::store::Folder { path: folder.clone(), apart: " (conflicted copy)" }, Some("a.md".into()), &theme, WORDS * scale).unwrap();
                    let from = text.find("文档").map(|at| text[..at].encode_utf16().count()).unwrap() as i32;
                    writing.editor.select((from, from + 2));
                    writing.write(Write::Link);
                    {
                        let held = writing.link.borrow();
                        let linking = held.as_ref().unwrap();
                        linking.to.set_text("");
                    }
                    let dark_name = if dark { "dark" } else { "light" };
                    let lang_name = if lang == Lang::Zh { "zh" } else { "en" };
                    let drawn = gfx.draw_offscreen(size, scale, |frame| writing.paint(frame, &theme, lang, (0.0, 0.0, w, h), (0.0, 0.0), scale, true)).unwrap();
                    crate::studio::save_png(&out.join(format!("link-{name}-{dark_name}-{lang_name}-empty.png")), size.0, size.1, &drawn).unwrap();
                    {
                        let held = writing.link.borrow();
                        let linking = held.as_ref().unwrap();
                        linking.to.set_text("https://docs.example.com/weekly/report-2026-10");
                        let _ = linking.to.send(WM_KEYDOWN, WPARAM(usize::from(VK_END.0)), LPARAM(0));
                    }
                    let drawn = gfx.draw_offscreen(size, scale, |frame| writing.paint(frame, &theme, lang, (0.0, 0.0, w, h), (0.0, 0.0), scale, true)).unwrap();
                    crate::studio::save_png(&out.join(format!("link-{name}-{dark_name}-{lang_name}-filled.png")), size.0, size.1, &drawn).unwrap();
                    drop(writing);
                }
            }
        }
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        let _ = std::fs::remove_dir_all(&folder);
        drop(gfx);
        super::super::gfx::let_go();
    }

    /// What was dressed found where it is now, the input method's words composed before it, in it or after it.
    #[test]
    fn what_was_dressed_where_it_is_now() {
        let units = |text: &str| text.encode_utf16().collect::<Vec<u16>>();
        let moved = super::Moved::between(&units("a\n\n| h |"), &units("你好a\n\n| h |"));
        assert_eq!((moved.at(0), moved.at(3)), (Some(2), Some(5)));
        let moved = super::Moved::between(&units("ab\ncd"), &units("aXYb\ncd"));
        assert_eq!((moved.at(0), moved.at(1), moved.at(3)), (Some(0), Some(3), Some(5)));
        let moved = super::Moved::between(&units("abcd"), &units("aXd"));
        assert_eq!((moved.at(0), moved.at(1), moved.at(2), moved.at(3)), (Some(0), None, None, Some(2)));
    }

    /// A note written, and as it is drawn not written in (stuck on the desktop), in each skin: each line's words
    /// beginning where they do on the other (written in where it is, its words do not move along their lines), its marks
    /// put away (a heading's `#`, emphasis's, code's, a link's), a to-do a box, and once its to-do's line is written on,
    /// that line as written. (Down the page, the text services set lines a little further apart than DirectWrite, as
    /// they always have: within a few pixels after a few lines.) (`cargo test --features studio --lib -- --ignored
    /// written_and_not` writes `target/beside/note-<skin>-<face|page|written>.png`, the note in a folder of the test's own.)
    #[test]
    #[ignore = "writes pictures to look at"]
    fn written_and_not() {
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_POPUP};
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        let gfx = super::super::gfx::current().unwrap();
        let out = std::path::PathBuf::from(format!("{}/target/beside", env!("CARGO_MANIFEST_DIR")));
        std::fs::create_dir_all(&out).unwrap();
        let folder = std::env::temp_dir().join(format!("glance-note-pictures-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let text = "# 本周要做\n- [ ] 买**牛奶**和面包\n- [x] 交 `周报`\n看 [文档](https://a.b) 或 https://x.y/z\n> 引一句话\n- 列表一项";
        std::fs::write(folder.join("a.md"), text).unwrap();
        let (w, h) = super::super::view::STUCK_NOTE;
        let scale = 1.5;
        let size = ((w * scale) as u32, (h * scale) as u32);
        let hwnd = unsafe { CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WS_POPUP, 0, 0, size.0 as i32, size.1 as i32, None, None, None, None) }.unwrap();
        let mut unlike = Vec::new();
        for (skin, name) in [(Skin::Glass, "glass"), (Skin::Fluent, "fluent"), (Skin::Paper, "paper")] {
            let theme = Theme::new(skin, false);
            let mut writing = Writing::open(hwnd, words_box(0, 0, size.0 as i32, size.1 as i32, scale), &crate::notes::store::Folder { path: folder.clone(), apart: " (conflicted copy)" }, Some("a.md".into()), &theme, WORDS * scale).unwrap();
            let now = crate::station::now_ms();
            let when = super::super::text::since(writing.written_ms, now, Lang::Zh);
            let face = gfx.draw_offscreen(size, scale, |frame| paint_stuck(frame, &theme, Lang::Zh, text, writing.colour, &when, "a.md", (0.0, 0.0, w, h), 0.0)).unwrap();
            crate::studio::save_png(&out.join(format!("note-{name}-face.png")), size.0, size.1, &face).unwrap();
            let page = gfx.draw_offscreen(size, scale, |frame| writing.paint(frame, &theme, Lang::Zh, (0.0, 0.0, w, h), (0.0, 0.0), scale, false)).unwrap();
            crate::studio::save_png(&out.join(format!("note-{name}-page.png")), size.0, size.1, &page).unwrap();
            // Each line's words (rows with ink, over white) beginning where they do on the other.
            let (left, top, wide, tall) = words_in(w, h);
            let lines = |pixels: &[u8]| {
                let inked = |x: u32, y: u32| {
                    let at = ((y * size.0 + x) * 4) as usize;
                    u32::from(pixels[at + 1]) + 255 - u32::from(pixels[at + 3]) < 140
                };
                let (xs, ys) = ((left * scale) as u32..((left + wide) * scale) as u32, (top * scale) as u32..((top + tall) * scale) as u32);
                let mut found: Vec<u32> = Vec::new();
                let mut in_line = false;
                for y in ys {
                    let first = xs.clone().find(|&x| inked(x, y));
                    match (first, in_line) {
                        (Some(x), false) => {
                            found.push(x);
                            in_line = true;
                        }
                        (Some(x), true) => {
                            let last = found.last_mut().unwrap();
                            *last = (*last).min(x);
                        }
                        (None, _) => in_line = false,
                    }
                }
                found
            };
            let (on_face, on_page) = (lines(&face), lines(&page));
            let apart = on_face.len() != on_page.len() || on_face.iter().zip(&on_page).any(|(a, b)| a.abs_diff(*b) > 2);
            unlike.push((name, apart, on_face, on_page));
            // Written at: the caret at its to-do's box, the box as it is written.
            let at = text.encode_utf16().position(|c| c == u16::from(b'-')).unwrap() as i32 + 3;
            writing.editor.select((at, at));
            writing.dress();
            let written = gfx.draw_offscreen(size, scale, |frame| writing.paint(frame, &theme, Lang::Zh, (0.0, 0.0, w, h), (0.0, 0.0), scale, false)).unwrap();
            crate::studio::save_png(&out.join(format!("note-{name}-written.png")), size.0, size.1, &written).unwrap();
            // Words chosen (the quote's): the format bar over them.
            let quote = text.encode_utf16().position(|c| c == u16::from(b'>')).unwrap() as i32 + 2;
            writing.editor.select((quote, quote + 3));
            writing.dress();
            let chosen = gfx.draw_offscreen(size, scale, |frame| writing.paint(frame, &theme, Lang::Zh, (0.0, 0.0, w, h), (0.0, 0.0), scale, false)).unwrap();
            crate::studio::save_png(&out.join(format!("note-{name}-bar.png")), size.0, size.1, &chosen).unwrap();
        }
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        let _ = std::fs::remove_dir_all(&folder);
        drop(gfx);
        super::super::gfx::let_go();
        println!("where each line's words begin, not written in and written: {unlike:?}");
        assert!(unlike.iter().all(|(_, apart, ..)| !apart), "{unlike:?}");
    }
}

#[cfg(test)]
mod tests {
    /// What is chosen of a note written in takes in a table's cells as it takes in their words: all of them chosen across
    /// the table, those reached chosen from its middle, none before it; words written before it since it was dressed
    /// moving them on.
    #[test]
    fn a_table_s_cells_are_chosen_with_its_words() {
        let text = "a\n| x | y |\n| --- | --- |\n| 1 | 2 |\nb";
        let units = |text: &str| text.encode_utf16().collect::<Vec<u16>>();
        let table = &crate::notes::table::tables(text)[0];
        let moved = super::Moved::between(&units(text), &units(text));
        let chosen = |from: usize, to: usize| -> Vec<(usize, usize)> {
            (0..2).flat_map(|row| (0..2).map(move |column| (row, column))).filter(|(row, column)| super::cell_chosen(table, &moved, (from as i32, to as i32), *row, *column)).collect()
        };
        let at = |words: &str| text.find(words).unwrap();
        assert_eq!(chosen(0, text.len()), vec![(0, 0), (0, 1), (1, 0), (1, 1)]);
        assert_eq!(chosen(0, at("x") + 1), vec![(0, 0)]);
        assert_eq!(chosen(at("2"), text.len()), vec![(1, 1)]);
        assert_eq!(chosen(0, 1), vec![]);
        let grown = format!("aaa{}", &text[1..]);
        let moved = super::Moved::between(&units(text), &units(&grown));
        assert!(!super::cell_chosen(table, &moved, (0, at("x") as i32 + 1), 0, 0));
        assert!(super::cell_chosen(table, &moved, (0, at("x") as i32 + 3), 0, 0));
    }

    /// A note's words, in every skin, light and dark: its faint marks (Markdown's) fainter than its words, not as solid
    /// (the box takes colours with no see-through: a faint ink given as one came out as the words' own).
    #[test]
    fn a_note_s_faint_marks_are_fainter() {
        use super::super::theme::{Skin, Theme};
        for skin in [Skin::Glass, Skin::Fluent, Skin::Paper] {
            for dark in [false, true] {
                let palette = super::palette_for(&Theme::new(skin, dark), crate::notes::Colour::Blue, 15.0);
                assert_ne!(palette.text.0, palette.faint.0, "{skin:?} {dark}");
            }
        }
    }

    /// A note's words on its page, in every skin, light and dark, every colour: as easily read as text is asked to be
    /// (4.5 to 1 against what is under them, the page drawn over its sheet). On glass in the dark the card's own dark
    /// ink, on a page's lighter pane, read at about 2 to 1.
    #[test]
    #[cfg(feature = "studio")]
    fn a_note_s_words_read_on_its_page() {
        use super::super::canvas::{Canvas, Color};
        use super::super::theme::{Skin, Theme};
        let luminance = |c: Color| {
            let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
            0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
        };
        let gfx = super::super::gfx::Gfx::new().expect("graphics");
        for skin in [Skin::Glass, Skin::Fluent, Skin::Paper] {
            for dark in [false, true] {
                let theme = Theme::new(skin, dark);
                for colour in crate::notes::Colour::ALL {
                    let pixels = gfx
                        .draw_offscreen((40, 40), 1.0, |frame| {
                            frame.fill(super::page_under(&theme), 0.0, 0.0, 40.0, 40.0);
                            super::super::view::note_page(frame, &theme, colour, (0.0, 0.0, 40.0, 40.0));
                        })
                        .unwrap();
                    let at = (20 * 40 + 20) * 4;
                    let ground = Color { r: pixels[at + 2] as f32 / 255.0, g: pixels[at + 1] as f32 / 255.0, b: pixels[at] as f32 / 255.0, a: 1.0 };
                    let words = super::word_inks(&theme, colour).words;
                    let (a, b) = (luminance(ground), luminance(words));
                    let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                    assert!(contrast >= 4.5, "{skin:?} dark {dark} {colour:?}: words {words:?} on {ground:?}, {contrast:.2} to 1");
                }
            }
        }
    }

    /// Which keys write Markdown: Enter in a list (not Shift+Enter, not with words chosen), Tab, Backspace at an item's
    /// words, the format keys. Ctrl with the text services' own keys (centring, line spacing) eaten, choosing all,
    /// copying, pasting, undoing theirs still, as are the keys that write nothing (Ctrl+arrows). The clipboard asked only
    /// for a paste. (Ctrl+K, a link, is its box's: eaten here.)
    #[test]
    fn which_keys_write_markdown() {
        use super::{pressed, Held, VIRTUAL_KEY};
        use windows::Win32::UI::Input::KeyboardAndMouse::*;
        let none = Held::default();
        let ctrl = Held { ctrl: true, ..none };
        let shift = Held { shift: true, ..none };
        let alt = Held { alt: true, ..none };
        let asked = std::cell::Cell::new(false);
        let plain = |text: &str| crate::notes::edit::Pasted { text: text.into(), blocks: false };
        let clipboard = || {
            asked.set(true);
            Some(plain("https://a.b"))
        };
        let made = |text: &str, chosen: (usize, usize), key: VIRTUAL_KEY, held: Held| pressed(text, chosen, key, held, held.alt, || None).map(|edit| edit.map(|edit| edit.with));
        assert_eq!(made("- a", (3, 3), VK_RETURN, none), Some(Some("\n- ".into())));
        assert_eq!(made("- a", (3, 3), VK_RETURN, shift), None);
        assert_eq!(made("- a", (2, 3), VK_RETURN, none), None);
        assert_eq!(made("words", (5, 5), VK_RETURN, none), None);
        assert_eq!(made("- [ ] a", (7, 7), VK_RETURN, ctrl), Some(Some("x".into())));
        assert_eq!(made("- a\n- b", (7, 7), VK_TAB, none), Some(Some("  ".into())));
        assert_eq!(made("- a", (3, 3), VK_TAB, none), Some(None));
        assert_eq!(made("words", (5, 5), VK_TAB, none), None);
        // In a table, Tab and Shift+Tab go from cell to cell (choosing its words, nothing written), past the last a row more.
        let chosen = |text: &str, at: usize, held: Held| pressed(text, (at, at), VK_TAB, held, false, || None).flatten().map(|edit| (edit.with, edit.chosen));
        assert_eq!(chosen("| a | b |\n| - | - |", 2, none), Some((String::new(), (6, 7))));
        assert_eq!(chosen("| a | b |\n| - | - |", 6, shift), Some((String::new(), (2, 3))));
        assert_eq!(chosen("| a | b |\n| - | - |\n| c | d |", 26, none), Some(("\n|  |  |".into(), (32, 32))));
        assert_eq!(made("- a", (2, 2), VK_BACK, none), Some(Some(String::new())));
        assert_eq!(made("- a", (3, 3), VK_BACK, none), None);
        assert_eq!(made("a\nb", (2, 2), VK_UP, alt), Some(Some("b\na".into())));
        assert_eq!(made("a\nb", (2, 2), VK_F4, alt), None);
        assert_eq!(made("ab", (0, 2), VK_B, ctrl), Some(Some("**ab**".into())));
        assert_eq!(made("ab", (0, 2), VK_X, Held { ctrl: true, shift: true, alt: false }), Some(Some("~~ab~~".into())));
        assert_eq!(made("ab", (0, 2), VK_2, ctrl), Some(Some("## ".into())));
        for key in [VK_L, VK_R, VK_J, VK_5, VK_OEM_PLUS, VK_H] {
            assert_eq!(made("ab", (0, 2), key, ctrl), Some(None), "Ctrl+{:#x}", key.0);
        }
        for key in [VK_A, VK_C, VK_X, VK_Z, VK_Y, VK_LEFT, VK_BACK, VK_DELETE, VK_HOME] {
            assert_eq!(made("ab", (0, 2), key, ctrl), None, "Ctrl+{:#x}", key.0);
        }
        assert_eq!(pressed("ab", (0, 2), VK_B, ctrl, false, clipboard).map(|edit| edit.map(|edit| edit.with)), Some(Some("**ab**".into())));
        assert!(!asked.get());
        assert_eq!(pressed("ab", (0, 2), VK_K, ctrl, false, clipboard).map(|edit| edit.map(|edit| edit.with)), Some(None));
        assert!(!asked.get());
        let _ = pressed("ab", (1, 1), VK_V, ctrl, false, clipboard);
        assert!(asked.get());
        // Pasting: the clipboard's text, its line breaks `\n`, a web address over words a link, nothing with no text.
        let pasted = |chosen: (usize, usize), key: VIRTUAL_KEY, held: Held, on: Option<&str>| pressed("ab", chosen, key, held, false, || on.map(plain)).map(|edit| edit.map(|edit| edit.with));
        assert_eq!(pasted((1, 1), VK_V, ctrl, Some("x\r\ny")), Some(Some("x\ny".into())));
        assert_eq!(pasted((1, 1), VK_INSERT, shift, Some("x")), Some(Some("x".into())));
        assert_eq!(pasted((0, 2), VK_V, ctrl, Some("https://a.b")), Some(Some("[ab](https://a.b)".into())));
        assert_eq!(pasted((1, 1), VK_V, ctrl, None), Some(None));
        assert_eq!(pasted((1, 1), VK_INSERT, none, Some("x")), None);
    }

    /// The writing menu: cut, copy, paste, the blocks and a link, the headings and words again, each row's icon its own.
    /// On a link: it edited, where it goes copied, it taken off its words (but a web address written out).
    #[test]
    fn the_writing_menu_s_rows() {
        use super::{writing_menu, Choice, Spot, Write};
        use crate::notes::edit::Block;
        let (items, choices) = writing_menu(super::Lang::Zh, Spot::default());
        assert_eq!(items.len(), choices.len());
        assert_eq!(items.iter().map(|item| item.label).collect::<Vec<_>>(), ["剪切", "复制", "粘贴", "插入待办事项", "插入表格", "插入代码块", "插入分隔线", "插入链接", "一级标题", "二级标题", "三级标题", "正文"]);
        assert_eq!(choices[4], Choice::Write(Write::Insert(Block::Table)));
        assert_eq!(choices[11], Choice::Write(Write::Heading(0)));
        assert_eq!(items.iter().filter(|item| item.rule_before).count(), 2);
        let labels = |spot: Spot| writing_menu(super::Lang::Zh, spot).0.iter().map(|item| item.label).collect::<Vec<_>>();
        assert_eq!(labels(Spot { in_cell: false, link: Some(true) })[7..10], ["编辑链接", "复制链接地址", "移除链接"]);
        assert_eq!(labels(Spot { in_cell: false, link: Some(false) })[7..9], ["编辑链接", "复制链接地址"]);
        assert_eq!(labels(Spot { in_cell: false, link: Some(false) })[9], "一级标题");
        // (In a table's cell: cut, copy, paste, a link, and what is done to its rows, its columns and itself.)
        let (items, choices) = writing_menu(super::Lang::Zh, Spot { in_cell: true, link: None });
        assert_eq!(items.len(), choices.len());
        assert_eq!(items.iter().map(|item| item.label).collect::<Vec<_>>(), ["剪切", "复制", "粘贴", "插入链接", "在上方插入行", "在下方插入行", "在左侧插入列", "在右侧插入列", "删除行", "删除列", "删除表格"]);
        assert_eq!(choices[10], Choice::Write(Write::Table(crate::notes::table::Change::TakeTable)));
        assert_eq!(items.iter().filter(|item| item.rule_before).count(), 3);
        assert_eq!(labels(Spot { in_cell: true, link: Some(true) })[3..6], ["编辑链接", "复制链接地址", "移除链接"]);
    }

    /// The format bar's buttons side by side in it, none at its edge or past its last, larger as the words are.
    #[test]
    fn the_format_bar_s_buttons() {
        use super::{bar_button, BAR_BUTTON, BAR_INSET, WORDS};
        use windows::Win32::Foundation::{POINT, RECT};
        let bar = RECT { left: 100, top: 50, right: 100 + (5.0 * BAR_BUTTON + 2.0 * BAR_INSET) as i32, bottom: 50 + (BAR_BUTTON + 2.0 * BAR_INSET) as i32 };
        let at = |x: f32, y: f32| bar_button(&bar, POINT { x: 100 + x as i32, y: 50 + y as i32 }, WORDS);
        assert_eq!(at(BAR_INSET + 1.0, BAR_INSET + 1.0), Some(0));
        assert_eq!(at(BAR_INSET + BAR_BUTTON * 4.5, 20.0), Some(4));
        assert_eq!(at(1.0, 20.0), None);
        assert_eq!(at(BAR_INSET + BAR_BUTTON * 5.0 + 1.0, 20.0), None);
        assert_eq!(bar_button(&bar, POINT { x: 100 + (BAR_INSET * 2.0 + BAR_BUTTON * 1.5 * 2.0) as i32, y: 70 }, WORDS * 2.0), Some(1));
    }

    /// Words chosen in a note's box: the format bar shows over them, its bold pressed makes them bold (chosen still),
    /// pressed again plain. Pressed on the words (choosing), it shows no more till the pointer is let go.
    #[test]
    fn the_format_bar_marks_the_words_chosen() {
        use windows::core::w;
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WS_POPUP};
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        let folder = std::env::temp_dir().join(format!("glance-note-bar-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("b.md"), "第一行\n第二行\n第三行\n一 粗体 字").unwrap();
        let hwnd = unsafe { CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WS_POPUP, 0, 0, 400, 300, None, None, None, None) }.unwrap();
        let theme = super::Theme::new(super::Skin::Paper, false);
        let mut writing = super::Writing::open(hwnd, super::words_box(0, 0, 400, 300, 1.0), &crate::notes::store::Folder { path: folder.clone(), apart: " (conflicted copy)" }, Some("b.md".into()), &theme, super::WORDS).unwrap();
        let answer = writing.answer();
        let at = |x: i32, y: i32| LPARAM(((y as isize) << 16) | (x as isize & 0xFFFF));
        let written = |writing: &super::Writing| writing.editor.text().strip_suffix('\n').unwrap().to_string();
        writing.editor.select((14, 16));
        writing.written();
        let bar = super::bar_at(&writing.editor, super::WORDS, false).expect("the bar over the words chosen");
        let chosen = writing.editor.character(14).unwrap();
        assert!(bar.bottom <= chosen.top, "{bar:?} over {chosen:?}");
        let bold = (bar.left + (super::BAR_INSET + super::BAR_BUTTON / 2.0) as i32, (bar.top + bar.bottom) / 2);
        answer(hwnd, WM_LBUTTONDOWN, WPARAM(1), at(bold.0, bold.1));
        answer(hwnd, WM_LBUTTONUP, WPARAM(0), at(bold.0, bold.1));
        writing.written();
        assert_eq!(written(&writing), "第一行\n第二行\n第三行\n一 **粗体** 字");
        assert_eq!(writing.editor.selection(), (16, 18));
        let bar = super::bar_at(&writing.editor, super::WORDS, false).unwrap();
        let bold = (bar.left + (super::BAR_INSET + super::BAR_BUTTON / 2.0) as i32, (bar.top + bar.bottom) / 2);
        answer(hwnd, WM_LBUTTONDOWN, WPARAM(1), at(bold.0, bold.1));
        writing.written();
        assert_eq!(written(&writing), "第一行\n第二行\n第三行\n一 粗体 字");
        assert_eq!(writing.editor.selection(), (14, 16));
        // Pressed on the words: choosing, no bar till let go.
        let words = writing.editor.character(15).unwrap();
        answer(hwnd, WM_LBUTTONDOWN, WPARAM(1), at(words.left + 1, words.top + 2));
        answer(hwnd, WM_MOUSEMOVE, WPARAM(1), at(words.right + 4, words.top + 2));
        assert!(writing.pressing.get());
        answer(hwnd, WM_LBUTTONUP, WPARAM(0), at(words.right + 4, words.top + 2));
        assert!(!writing.pressing.get());
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A table drawn laid out written in a cell at a time: pressed on a cell, its words written there (a pipe escaped),
    /// Tab to the next (past the last, a row more), Shift+Tab back, a row put above from the menu. Down off its last
    /// row: out of it, on the line after it. Up from there: back in its last row. Right at the end of the line before
    /// it: its first cell.
    #[test]
    fn a_table_written_in_a_cell_at_a_time() {
        let _device = crate::ui::gfx::held().unwrap();
        use windows::core::w;
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::Input::KeyboardAndMouse::{VIRTUAL_KEY, VK_DOWN, VK_END, VK_RIGHT, VK_SHIFT, VK_TAB, VK_UP};
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WM_CHAR, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WS_POPUP};
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        let folder = std::env::temp_dir().join(format!("glance-note-cells-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("t.md"), "前\n\n| 甲 | 乙 |\n| - | - |\n| x | y |\n\n后").unwrap();
        let hwnd = unsafe { CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WS_POPUP, 0, 0, 400, 300, None, None, None, None) }.unwrap();
        let theme = super::Theme::new(super::Skin::Paper, false);
        let mut writing = super::Writing::open(hwnd, super::words_box(0, 0, 400, 300, 1.0), &crate::notes::store::Folder { path: folder.clone(), apart: " (conflicted copy)" }, Some("t.md".into()), &theme, super::WORDS).unwrap();
        let answer = writing.answer();
        let at = |x: i32, y: i32| LPARAM(((y as isize) << 16) | (x as isize & 0xFFFF));
        let written = |writing: &super::Writing| writing.editor.words();
        let open = |writing: &super::Writing| writing.cell.borrow().as_ref().map(|cell| (cell.row, cell.column));
        // (Each key as the window gets it: Tab's character after it, as translated.)
        let key = |writing: &mut super::Writing, key: VIRTUAL_KEY| {
            answer(hwnd, WM_KEYDOWN, WPARAM(usize::from(key.0)), LPARAM(0));
            if key == VK_TAB {
                answer(hwnd, WM_CHAR, WPARAM(usize::from(b'\t')), LPARAM(0));
            }
            writing.written();
        };
        let typed = |writing: &mut super::Writing, text: &str| {
            for c in text.encode_utf16() {
                answer(hwnd, WM_CHAR, WPARAM(usize::from(c)), LPARAM(0));
                writing.written();
            }
        };
        // Pressed on the cell of y: written in there.
        let units: Vec<u16> = writing.editor.text().encode_utf16().collect();
        let top = super::line_top(&writing.editor, &units, writing.laid[0].0.span.start).unwrap();
        let cell = super::cell_box(&writing.editor, &writing.laid[0].1, top, 1, 1).unwrap();
        answer(hwnd, WM_LBUTTONDOWN, WPARAM(1), at((cell.left + cell.right) / 2, (cell.top + cell.bottom) / 2));
        writing.written();
        assert_eq!(open(&writing), Some((1, 1)));
        answer(hwnd, WM_LBUTTONUP, WPARAM(0), at((cell.left + cell.right) / 2, (cell.top + cell.bottom) / 2));
        key(&mut writing, VK_END);
        assert!(!writing.unkept);
        typed(&mut writing, "z|");
        assert_eq!(written(&writing), "前\n\n| 甲 | 乙 |\n| - | - |\n| x | yz\\| |\n\n后");
        // (Written in a cell alone, the note is to be kept: it was dressed again as the cell's words came into it.)
        assert!(writing.unkept);
        writing.unkept = false;
        // Tab past the last cell: a row more, written in at its first.
        key(&mut writing, VK_TAB);
        assert_eq!(open(&writing), Some((2, 0)));
        typed(&mut writing, "新");
        assert_eq!(written(&writing), "前\n\n| 甲 | 乙 |\n| - | - |\n| x | yz\\| |\n| 新 |  |\n\n后");
        // Shift+Tab back (its words all chosen), a row put above it from the menu.
        let shift = |down: bool| unsafe {
            let mut keys = [0u8; 256];
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardState(&mut keys);
            keys[usize::from(VK_SHIFT.0)] = if down { 0x80 } else { 0 };
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetKeyboardState(&keys);
        };
        shift(true);
        key(&mut writing, VK_TAB);
        shift(false);
        assert_eq!(open(&writing), Some((1, 1)));
        assert_eq!(writing.chosen(), (0, 4));
        writing.unkept = false;
        writing.write(super::Write::Table(crate::notes::table::Change::RowAbove));
        writing.written();
        assert!(writing.unkept);
        assert_eq!(open(&writing), Some((1, 1)));
        assert_eq!(written(&writing), "前\n\n| 甲 | 乙 |\n|---|---|\n|  |  |\n| x | yz\\| |\n| 新 |  |\n\n后");
        // Down off its last row: on the line after it. Up: back in its last row.
        key(&mut writing, VK_DOWN);
        key(&mut writing, VK_DOWN);
        key(&mut writing, VK_DOWN);
        assert_eq!(open(&writing), None);
        let after = written(&writing).encode_utf16().count() as i32 - 2;
        assert_eq!(writing.editor.selection(), (after, after));
        key(&mut writing, VK_UP);
        assert_eq!(open(&writing), Some((3, 0)));
        // Up off its head: at the end of the line before it. Right there: its first cell.
        for _ in 0..4 {
            key(&mut writing, VK_UP);
        }
        assert_eq!(open(&writing), None);
        assert_eq!(writing.editor.selection(), (2, 2));
        key(&mut writing, VK_RIGHT);
        assert_eq!(open(&writing), Some((0, 0)));
        // Its file written over outside, all here kept: read again, the cell written in shows its words as read, and
        // what is written in it next goes into them, not into its words as they were.
        writing.unkept = false;
        let store = crate::notes::store::Folder { path: folder.clone(), apart: " (conflicted copy)" };
        let read_outside = |writing: &mut super::Writing| {
            if writing.to_read().is_some() {
                let (text, stamp) = store.read("t.md").unwrap();
                writing.take_outside(&text, stamp);
            }
        };
        std::fs::write(folder.join("t.md"), "前\n\n| 丙 | 乙 |\n| - | - |\n| x | y |\n\n后").unwrap();
        read_outside(&mut writing);
        writing.written();
        assert_eq!(open(&writing), Some((0, 0)));
        assert_eq!(writing.cell.borrow().as_ref().unwrap().editor.words(), "丙");
        typed(&mut writing, "丁");
        assert_eq!(written(&writing), "前\n\n| 丙丁 | 乙 |\n| - | - |\n| x | y |\n\n后");
        // Not read over what is written in a cell and not yet taken in.
        std::fs::write(folder.join("t.md"), "前\n\n| 戊 | 乙 |\n| - | - |\n| x | y |\n\n后").unwrap();
        writing.unkept = false;
        answer(hwnd, WM_CHAR, WPARAM(usize::from(b'!')), LPARAM(0));
        read_outside(&mut writing);
        writing.written();
        assert_eq!(written(&writing), "前\n\n| 丙丁! | 乙 |\n| - | - |\n| x | y |\n\n后");
        // AltGr (Ctrl with Alt) writes its character in a cell, as on a German keyboard AltGr+Q writes @.
        let ctrl_alt = |down: bool| unsafe {
            let mut keys = [0u8; 256];
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardState(&mut keys);
            for key in [windows::Win32::UI::Input::KeyboardAndMouse::VK_CONTROL, windows::Win32::UI::Input::KeyboardAndMouse::VK_MENU] {
                keys[usize::from(key.0)] = if down { 0x80 } else { 0 };
            }
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetKeyboardState(&keys);
        };
        ctrl_alt(true);
        answer(hwnd, WM_KEYDOWN, WPARAM(usize::from(b'Q')), LPARAM(0));
        answer(hwnd, WM_CHAR, WPARAM(usize::from(b'@')), LPARAM(0));
        ctrl_alt(false);
        writing.written();
        assert_eq!(written(&writing), "前\n\n| 丙丁!@ | 乙 |\n| - | - |\n| x | y |\n\n后");
        // (Its boxes' text services shut down while their window is there.)
        drop(answer);
        drop(writing);
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A link written on its plate, keys given as Windows gives them: words chosen and Ctrl+K, the plate opens with them
    /// as its words, the keys in its field for where it goes, Enter writes the link (as one undo), an address with a
    /// space written so it reads back. On the link, Ctrl+K opens it filled from it: its words cleared, Enter writes
    /// nothing, words with a bracket written escaped. Esc, and a press off it, leave it with nothing written, the keys
    /// the note's again (what is typed then is its words'). Taken off its words from the menu. In a table's cell, one
    /// written there.
    #[test]
    fn a_link_is_written_on_its_plate() {
        let _device = crate::ui::gfx::held().unwrap();
        use windows::core::w;
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::Input::KeyboardAndMouse::{VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_END, VK_ESCAPE, VK_K, VK_RETURN, VK_TAB};
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WM_CHAR, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WS_POPUP};
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        let folder = std::env::temp_dir().join(format!("glance-note-link-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("l.md"), "看文档吧\n\n| 甲 | 乙 |\n| - | - |\n| x | y |\n\n后").unwrap();
        let hwnd = unsafe { CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WS_POPUP, 0, 0, 400, 400, None, None, None, None) }.unwrap();
        let theme = super::Theme::new(super::Skin::Paper, false);
        let mut writing = super::Writing::open(hwnd, super::words_box(0, 0, 400, 400, 1.0), &crate::notes::store::Folder { path: folder.clone(), apart: " (conflicted copy)" }, Some("l.md".into()), &theme, super::WORDS).unwrap();
        let answer = writing.answer();
        let at = |x: i32, y: i32| LPARAM(((y as isize) << 16) | (x as isize & 0xFFFF));
        let first = |writing: &super::Writing| writing.editor.words().split('\n').next().unwrap().to_string();
        let fields = |writing: &super::Writing| writing.link.borrow().as_ref().map(|linking| (linking.words.words(), linking.to.words(), linking.focused()));
        // (Each key as Windows gives it: down, the character it makes, up. The note seen to after each, as its window
        // does.)
        let key = |writing: &mut super::Writing, key: VIRTUAL_KEY, character: Option<u16>| {
            answer(hwnd, WM_KEYDOWN, WPARAM(usize::from(key.0)), LPARAM(1));
            writing.written();
            if let Some(character) = character {
                answer(hwnd, WM_CHAR, WPARAM(usize::from(character)), LPARAM(1));
            }
            answer(hwnd, WM_KEYUP, WPARAM(usize::from(key.0)), LPARAM(1));
            writing.written();
        };
        let typed = |writing: &mut super::Writing, text: &str| {
            for c in text.encode_utf16() {
                answer(hwnd, WM_CHAR, WPARAM(usize::from(c)), LPARAM(1));
                writing.written();
            }
        };
        let ctrl = |down: bool| unsafe {
            let mut keys = [0u8; 256];
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardState(&mut keys);
            keys[usize::from(VK_CONTROL.0)] = if down { 0x80 } else { 0 };
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetKeyboardState(&keys);
        };
        let ctrl_k = |writing: &mut super::Writing| {
            ctrl(true);
            key(writing, VK_K, Some(0x0B));
            ctrl(false);
        };
        // Words chosen, Ctrl+K: the plate, its words those, the keys where it goes (all of it chosen, to be typed over).
        writing.editor.select((1, 3));
        ctrl_k(&mut writing);
        let (words, _, focused) = fields(&writing).expect("the plate up");
        assert_eq!((words.as_str(), focused), ("文档", super::link::Field::To));
        typed(&mut writing, "https://a.b/x y");
        key(&mut writing, VK_RETURN, Some(0x0D));
        assert!(fields(&writing).is_none());
        assert_eq!(first(&writing), "看[文档](<https://a.b/x y>)吧");
        let end = "看[文档](<https://a.b/x y>)".encode_utf16().count() as i32;
        assert_eq!(writing.editor.selection(), (end, end));
        // On it, Ctrl+K: filled from it. Its words cleared: Enter writes nothing. Written anew, with a bracket.
        writing.editor.select((2, 2));
        ctrl_k(&mut writing);
        assert_eq!(fields(&writing).map(|(words, to, _)| (words, to)), Some(("文档".into(), "https://a.b/x y".into())));
        key(&mut writing, VK_TAB, Some(0x09));
        assert_eq!(fields(&writing).map(|(.., focused)| focused), Some(super::link::Field::Words));
        key(&mut writing, VK_BACK, Some(0x08));
        key(&mut writing, VK_RETURN, Some(0x0D));
        assert_eq!(fields(&writing).map(|(words, ..)| words), Some(String::new()));
        typed(&mut writing, "说明]");
        key(&mut writing, VK_RETURN, Some(0x0D));
        assert!(fields(&writing).is_none());
        assert_eq!(first(&writing), "看[说明\\]](<https://a.b/x y>)吧");
        // (EM_UNDO: the link as it was, in one step.)
        writing.editor.send(0x00C7, WPARAM(0), LPARAM(0));
        writing.written();
        assert_eq!(first(&writing), "看[文档](<https://a.b/x y>)吧");
        // Esc: left, nothing written, the keys the note's again.
        writing.editor.select((2, 2));
        ctrl_k(&mut writing);
        typed(&mut writing, "别处");
        key(&mut writing, VK_ESCAPE, Some(0x1B));
        assert!(fields(&writing).is_none());
        assert_eq!(writing.editor.selection(), (2, 2));
        typed(&mut writing, "好");
        assert_eq!(first(&writing), "看[好文档](<https://a.b/x y>)吧");
        // A press off it: left, the press the note's.
        ctrl_k(&mut writing);
        let plate = writing.link.borrow().as_ref().unwrap().laid().plate;
        let off = writing.editor.character(0).unwrap();
        assert!(off.bottom <= plate.top, "{plate:?} under the words");
        answer(hwnd, WM_LBUTTONDOWN, WPARAM(1), at(off.left + 1, off.top + 2));
        answer(hwnd, WM_LBUTTONUP, WPARAM(0), at(off.left + 1, off.top + 2));
        writing.written();
        assert!(fields(&writing).is_none());
        assert_eq!(first(&writing), "看[好文档](<https://a.b/x y>)吧");
        // Taken off its words from the menu: they stay, chosen.
        writing.editor.select((3, 3));
        assert_eq!(writing.spot(), super::Spot { in_cell: false, link: Some(true) });
        writing.write(super::Write::Unlink);
        writing.written();
        assert_eq!(first(&writing), "看好文档吧");
        assert_eq!(writing.editor.selection(), (1, 4));
        // In a table's cell: one written there, after its words.
        let units: Vec<u16> = writing.editor.text().encode_utf16().collect();
        let top = super::line_top(&writing.editor, &units, writing.laid[0].0.span.start).unwrap();
        let cell = super::cell_box(&writing.editor, &writing.laid[0].1, top, 1, 1).unwrap();
        answer(hwnd, WM_LBUTTONDOWN, WPARAM(1), at((cell.left + cell.right) / 2, (cell.top + cell.bottom) / 2));
        writing.written();
        answer(hwnd, WM_LBUTTONUP, WPARAM(0), at((cell.left + cell.right) / 2, (cell.top + cell.bottom) / 2));
        key(&mut writing, VK_END, None);
        ctrl_k(&mut writing);
        assert_eq!(fields(&writing).map(|(.., focused)| focused), Some(super::link::Field::Words));
        typed(&mut writing, "网|页");
        key(&mut writing, VK_TAB, Some(0x09));
        typed(&mut writing, "https://c.d");
        key(&mut writing, VK_RETURN, Some(0x0D));
        assert!(fields(&writing).is_none());
        assert!(writing.in_cell());
        assert!(writing.editor.words().contains("| x | y[网\\|页](https://c.d) |"), "{}", writing.editor.words());
        // (Its boxes' text services shut down while their window is there.)
        drop(answer);
        drop(writing);
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// The writing menu asked for on a note's words: the right button pressed off what is chosen puts the caret there,
    /// let go asks for it there (on the screen), as the menu key does at the caret. What it does is done as one undo.
    #[test]
    fn the_writing_menu_is_asked_for_and_does_its_work() {
        use windows::core::w;
        use windows::Win32::Foundation::{LPARAM, POINT, WPARAM};
        use windows::Win32::UI::Input::KeyboardAndMouse::VK_APPS;
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WM_KEYDOWN, WM_KEYUP, WM_RBUTTONDOWN, WM_RBUTTONUP, WS_POPUP};
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        let folder = std::env::temp_dir().join(format!("glance-note-menu-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("m.md"), "一行字\n二行字").unwrap();
        let hwnd = unsafe { CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WS_POPUP, 50, 60, 400, 300, None, None, None, None) }.unwrap();
        let theme = super::Theme::new(super::Skin::Paper, false);
        let mut writing = super::Writing::open(hwnd, super::words_box(0, 0, 400, 300, 1.0), &crate::notes::store::Folder { path: folder.clone(), apart: " (conflicted copy)" }, Some("m.md".into()), &theme, super::WORDS).unwrap();
        let answer = writing.answer();
        let at = |x: i32, y: i32| LPARAM(((y as isize) << 16) | (x as isize & 0xFFFF));
        let written = |writing: &super::Writing| writing.editor.text().strip_suffix('\n').unwrap().to_string();
        writing.editor.select((0, 2));
        let second = writing.editor.character(5).unwrap();
        let (x, y) = (second.left + 1, second.top + 2);
        answer(hwnd, WM_RBUTTONDOWN, WPARAM(2), at(x, y));
        assert_eq!(writing.editor.selection(), (5, 5));
        assert_eq!(writing.take_menu_asked(), None);
        answer(hwnd, WM_RBUTTONUP, WPARAM(0), at(x, y));
        assert_eq!(writing.take_menu_asked(), Some(POINT { x: 50 + x, y: 60 + y }));
        // Pressed on what is chosen, it stays chosen.
        writing.editor.select((4, 7));
        answer(hwnd, WM_RBUTTONDOWN, WPARAM(2), at(x, y));
        assert_eq!(writing.editor.selection(), (4, 7));
        answer(hwnd, WM_KEYDOWN, WPARAM(usize::from(VK_APPS.0)), LPARAM(1));
        answer(hwnd, WM_KEYUP, WPARAM(usize::from(VK_APPS.0)), LPARAM(1));
        assert!(writing.take_menu_asked().is_some());
        writing.editor.select((5, 5));
        writing.write(super::Write::Heading(2));
        writing.written();
        assert_eq!(written(&writing), "一行字\n## 二行字");
        writing.write(super::Write::Insert(crate::notes::edit::Block::Rule));
        writing.written();
        // (After the last line, before the mark the text services end the words with: a line after it to go on, the
        // caret there.)
        assert_eq!(written(&writing), "一行字\n## 二行字\n\n---\n");
        assert_eq!(writing.editor.selection(), (16, 16));
        // (EM_UNDO.)
        writing.editor.send(0x00C7, WPARAM(0), LPARAM(0));
        assert_eq!(written(&writing), "一行字\n## 二行字");
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// What is chosen in a note copied with Ctrl+C on the panel's thread (in the process's multithreaded apartment): its
    /// words as they show, there still once the note is written in no longer (its box gone), and as rich text. Ctrl+X
    /// takes them out as well, undo brings them back.
    #[test]
    #[ignore = "puts words on this session's clipboard"]
    fn what_is_chosen_is_copied_and_stays_copied() {
        use windows::core::w;
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::System::DataExchange::{IsClipboardFormatAvailable, RegisterClipboardFormatW};
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardState, SetKeyboardState, VIRTUAL_KEY, VK_C, VK_CONTROL, VK_X};
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WM_CHAR, WM_KEYDOWN, WM_KEYUP, WS_POPUP};
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let folder = std::env::temp_dir().join(format!("glance-note-copy-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("c.md"), "plain **bold** words").unwrap();
        let hwnd = unsafe { CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WS_POPUP, 0, 0, 400, 300, None, None, None, None) }.unwrap();
        let theme = super::Theme::new(super::Skin::Paper, false);
        let writing = super::Writing::open(hwnd, super::words_box(0, 0, 400, 300, 1.0), &crate::notes::store::Folder { path: folder.clone(), apart: " (conflicted copy)" }, Some("c.md".into()), &theme, super::WORDS).unwrap();
        writing.focus(true);
        let answer = writing.answer();
        // (Ctrl held as the keys go: the thread's keyboard as read.)
        let with_ctrl = |key: VIRTUAL_KEY, character: usize| {
            let mut state = [0u8; 256];
            unsafe {
                let _ = GetKeyboardState(&mut state);
                state[usize::from(VK_CONTROL.0)] = 0x80;
                let _ = SetKeyboardState(&state);
            }
            let answered = [answer(hwnd, WM_KEYDOWN, WPARAM(usize::from(key.0)), LPARAM(1)), answer(hwnd, WM_CHAR, WPARAM(character), LPARAM(1)), answer(hwnd, WM_KEYUP, WPARAM(usize::from(key.0)), LPARAM(1))];
            unsafe {
                state[usize::from(VK_CONTROL.0)] = 0;
                let _ = SetKeyboardState(&state);
            }
            answered
        };
        let written = writing.editor.words();
        let every = written.encode_utf16().count() as i32;
        writing.editor.select((0, every));
        let copied = with_ctrl(VK_C, 0x03);
        let after_copy = writing.editor.words();
        // Ctrl+X on the last word: gone, on the clipboard, back with undo.
        writing.editor.select((every - 5, every));
        let _ = with_ctrl(VK_X, 0x18);
        let after_cut = (writing.editor.words(), super::clipboard_text(hwnd));
        // (EM_UNDO.)
        writing.editor.send(0x00C7, WPARAM(0), LPARAM(0));
        let undone = writing.editor.words();
        writing.editor.select((0, every));
        let _ = with_ctrl(VK_C, 0x03);
        drop(answer);
        drop(writing);
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        let owner = unsafe { CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WS_POPUP, 0, 0, 1, 1, None, None, None, None) }.unwrap();
        let stays = super::clipboard_text(owner);
        let rich = {
            let _opened = crate::clipboard::open(owner);
            unsafe { IsClipboardFormatAvailable(RegisterClipboardFormatW(w!("Rich Text Format"))) }.is_ok()
        };
        unsafe {
            let _ = DestroyWindow(owner);
        }
        let _ = std::fs::remove_dir_all(&folder);
        assert!(copied.iter().all(|answered| answered.is_some()), "{copied:?}");
        assert_eq!(after_copy, written);
        assert_eq!(after_cut, ("plain **bold** ".into(), Some("words".into())));
        assert_eq!(undone, written);
        assert_eq!(stays.as_deref(), Some("plain bold words"));
        assert!(rich);
    }

    /// Keys given to a note's words' box as Windows gives them (the key down, the character it makes, the key up): Enter
    /// in a list carries it on, the caret on the new item, Tab moves that item in, its character eaten (no tab written
    /// after it), and one undo takes back each.
    #[test]
    fn keys_given_to_the_box_write_markdown() {
        use windows::core::w;
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::Input::KeyboardAndMouse::{VIRTUAL_KEY, VK_RETURN, VK_TAB};
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WM_CHAR, WM_KEYDOWN, WM_KEYUP, WS_POPUP};
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        let folder = std::env::temp_dir().join(format!("glance-note-keys-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("k.md"), "1. a\n2. b").unwrap();
        let hwnd = unsafe { CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WS_POPUP, 0, 0, 400, 300, None, None, None, None) }.unwrap();
        let theme = super::Theme::new(super::Skin::Paper, false);
        let writing = super::Writing::open(hwnd, super::words_box(0, 0, 400, 300, 1.0), &crate::notes::store::Folder { path: folder.clone(), apart: " (conflicted copy)" }, Some("k.md".into()), &theme, super::WORDS).unwrap();
        let answer = writing.answer();
        let key = |key: VIRTUAL_KEY, character: usize| {
            answer(hwnd, WM_KEYDOWN, WPARAM(usize::from(key.0)), LPARAM(1));
            answer(hwnd, WM_CHAR, WPARAM(character), LPARAM(1));
            answer(hwnd, WM_KEYUP, WPARAM(usize::from(key.0)), LPARAM(1));
        };
        let written = || writing.editor.text().strip_suffix('\n').unwrap().to_string();
        let undo = || {
            // (EM_UNDO.)
            writing.editor.send(0x00C7, WPARAM(0), LPARAM(0));
        };
        writing.editor.select((4, 4));
        key(VK_RETURN, 0x0D);
        assert_eq!(written(), "1. a\n2. \n3. b");
        assert_eq!(writing.editor.selection(), (8, 8));
        key(VK_TAB, 0x09);
        assert_eq!(written(), "1. a\n   1. \n2. b");
        assert_eq!(writing.editor.selection(), (11, 11));
        undo();
        assert_eq!(written(), "1. a\n2. \n3. b");
        undo();
        assert_eq!(written(), "1. a\n2. b");
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        let _ = std::fs::remove_dir_all(&folder);
    }
}
