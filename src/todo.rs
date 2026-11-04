//! To-dos: one note of the notes' folder (`待办.md`, or `To-do.md` where Glance began in English), plain Markdown
//! that reads anywhere: an item a line, `- [ ] words`, ticked `- [x] words`, its time after it as ` @2026-11-04`
//! or ` @2026-11-04 18:00`. What else the note holds (a title, a line of words) stays as it is written.
//!
//! The items not yet done come first, in the order they were put down (a new one at the top); those done after
//! them, the last done first. Ticked, an item goes to the top of those done; unticked, back where it was.
//!
//! The to-do note is not among the notes listed (see `notes::listed`): notes are notes, to-dos are to-dos.

use std::fmt::Write as _;

/// The names the to-do note goes by, in each language: the first of them in the folder is the one.
pub const NAMES: [&str; 2] = ["待办.md", "To-do.md"];

/// A day of the calendar.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Day {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl Day {
    /// The days from 1970-01-01 to it (before it, less than none): as the proleptic Gregorian calendar counts them.
    pub fn number(self) -> i64 {
        let (month, day) = (self.month as i64, self.day as i64);
        let year = self.year as i64 - i64::from(month <= 2);
        let era = year.div_euclid(400);
        let of_era = year - era * 400;
        let of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
        let of_era_days = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
        era * 146_097 + of_era_days - 719_468
    }

    /// The day `number` days from 1970-01-01 (see `number`).
    pub fn from_number(number: i64) -> Day {
        let z = number + 719_468;
        let era = z.div_euclid(146_097);
        let of_era = z - era * 146_097;
        let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
        let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let mp = (5 * of_year + 2) / 153;
        let day = (of_year - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        Day { year: (year_of_era + era * 400 + i64::from(month <= 2)) as i32, month, day }
    }

    /// The day `days` after it (before it, less than none).
    pub fn plus(self, days: i64) -> Day {
        Day::from_number(self.number() + days)
    }

    /// Its day of the week: Monday 0 to Sunday 6.
    pub fn weekday(self) -> u32 {
        // (1970-01-01 was a Thursday.)
        (self.number() + 3).rem_euclid(7) as u32
    }

    /// How many days its month has.
    pub fn month_days(year: i32, month: u32) -> u32 {
        match month {
            2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        }
    }

    /// The day `year`-`month`-`day`, where there is one.
    pub fn of(year: i32, month: u32, day: u32) -> Option<Day> {
        ((1..=12).contains(&month) && (1..=Day::month_days(year, month)).contains(&day)).then_some(Day { year, month, day })
    }

    /// Today, as this PC's clock and time zone have it.
    #[cfg(windows)]
    pub fn today() -> Day {
        let now = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
        Day { year: now.wYear as i32, month: now.wMonth as u32, day: now.wDay as u32 }
    }
}

/// When an item is due: a day, and a time of it (none: the whole day).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Due {
    pub day: Day,
    pub time: Option<(u32, u32)>,
}

impl Due {
    /// As the note writes it: `2026-11-04` or `2026-11-04 18:00`.
    pub fn written(self) -> String {
        let Day { year, month, day } = self.day;
        match self.time {
            Some((hour, minute)) => format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}"),
            None => format!("{year:04}-{month:02}-{day:02}"),
        }
    }

    /// Read as the note writes it (see `written`).
    pub fn read(text: &str) -> Option<Due> {
        let (date, time) = match text.split_once(' ') {
            Some((date, time)) => (date, Some(time)),
            None => (text, None),
        };
        let mut parts = date.split('-');
        let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() || year.len() != 4 || month.len() != 2 || day.len() != 2 {
            return None;
        }
        let day = Day::of(year.parse().ok()?, month.parse().ok()?, day.parse().ok()?)?;
        let time = match time {
            Some(time) => Some(clock(time).filter(|_| time.len() == 5)?),
            None => None,
        };
        Some(Due { day, time })
    }

    /// Whether it is due by `today` (today, or past).
    pub fn by(self, today: Day) -> bool {
        self.day <= today
    }

    /// As its label says it (`today`, as days go): today, tomorrow, the day after, a weekday within the week, else
    /// the month and day (and the year, another year's); and its time.
    pub fn label(self, today: Day, zh: bool) -> String {
        const WEEK_ZH: [&str; 7] = ["一", "二", "三", "四", "五", "六", "日"];
        const WEEK_EN: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        const MONTHS_EN: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        let days = self.day.number() - today.number();
        let Day { year, month, day } = self.day;
        let mut label = match (days, zh) {
            (0, true) => "今天".to_string(),
            (1, true) => "明天".to_string(),
            (2, true) => "后天".to_string(),
            (0, false) => "Today".to_string(),
            (1, false) => "Tomorrow".to_string(),
            (3..=6, true) => format!("周{}", WEEK_ZH[self.day.weekday() as usize]),
            (2..=6, false) => WEEK_EN[self.day.weekday() as usize].to_string(),
            (_, true) if year == today.year => format!("{month}月{day}日"),
            (_, true) => format!("{year}年{month}月{day}日"),
            (_, false) if year == today.year => format!("{} {day}", MONTHS_EN[month as usize - 1]),
            (_, false) => format!("{} {day}, {year}", MONTHS_EN[month as usize - 1]),
        };
        if let Some((hour, minute)) = self.time {
            let _ = write!(label, " {hour:02}:{minute:02}");
        }
        label
    }
}

/// A time of day written `18:00` or `7:30` (a full-width colon too): its hour and minute.
pub fn clock(text: &str) -> Option<(u32, u32)> {
    let (hour, minute) = text.split_once([':', '：'])?;
    let all_digits = |part: &str| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit());
    if !all_digits(hour) || hour.len() > 2 || !all_digits(minute) || minute.len() != 2 {
        return None;
    }
    let (hour, minute) = (hour.parse().ok()?, minute.parse().ok()?);
    (hour < 24 && minute < 60).then_some((hour, minute))
}

/// A to-do: its words, whether it is done, and when it is due.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Item {
    pub text: String,
    pub done: bool,
    pub due: Option<Due>,
}

/// A line of the note: an item (and what its line began with: its indent and mark), or anything else, as written.
#[derive(Clone, PartialEq, Debug)]
enum Line {
    Item { lead: String, item: Item },
    Other(String),
}

/// The to-do note's lines, read.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct List {
    lines: Vec<Line>,
    /// Whether the note ended its last line with a line break.
    ended: bool,
}

impl List {
    /// The note's `text`, read.
    pub fn read(text: &str) -> List {
        let ended = text.ends_with('\n');
        let body = text.strip_suffix('\n').unwrap_or(text);
        let lines = if text.is_empty() { Vec::new() } else { body.split('\n').map(line).collect() };
        List { lines, ended }
    }

    /// The note's text, as it is to be written.
    pub fn written(&self) -> String {
        let mut text = String::new();
        for (at, line) in self.lines.iter().enumerate() {
            if at > 0 {
                text.push('\n');
            }
            match line {
                Line::Other(other) => text.push_str(other),
                Line::Item { lead, item } => {
                    let _ = write!(text, "{lead}[{}] {}", if item.done { 'x' } else { ' ' }, item.text);
                    if let Some(due) = item.due {
                        let _ = write!(text, " @{}", due.written());
                    }
                }
            }
        }
        if self.ended || !self.lines.is_empty() {
            text.push('\n');
        }
        text
    }

    /// The items, in the note's order.
    pub fn items(&self) -> Vec<&Item> {
        self.lines.iter().filter_map(|line| if let Line::Item { item, .. } = line { Some(item) } else { None }).collect()
    }

    /// Where the `n`th item's line is.
    fn line_of(&self, n: usize) -> Option<usize> {
        self.lines.iter().enumerate().filter(|(_, line)| matches!(line, Line::Item { .. })).nth(n).map(|(at, _)| at)
    }

    fn item_mut(&mut self, n: usize) -> Option<&mut Item> {
        let at = self.line_of(n)?;
        match &mut self.lines[at] {
            Line::Item { item, .. } => Some(item),
            Line::Other(_) => None,
        }
    }

    /// The mark items are written with: as the first item has it (none yet: `- `).
    fn lead(&self) -> String {
        self.lines.iter().find_map(|line| if let Line::Item { lead, .. } = line { Some(lead.clone()) } else { None }).unwrap_or_else(|| "- ".into())
    }

    /// Where the line of an item put among those not done, `n`th of them, goes: before the one there now; after the last
    /// of them, `n` past them all (and none: after the note's last item, or at its end, where it has none).
    fn place_among_undone(&self, n: usize) -> usize {
        let undone: Vec<usize> = self.lines.iter().enumerate().filter(|(_, line)| matches!(line, Line::Item { item, .. } if !item.done)).map(|(at, _)| at).collect();
        if let Some(at) = undone.get(n) {
            return *at;
        }
        match undone.last() {
            Some(last) => last + 1,
            None => self.first_done().unwrap_or_else(|| self.after_items()),
        }
    }

    /// Where the first line of a done item is.
    fn first_done(&self) -> Option<usize> {
        self.lines.iter().position(|line| matches!(line, Line::Item { item, .. } if item.done))
    }

    /// Just after the last item's line; the note's end, where it has none (before blank lines ending it).
    fn after_items(&self) -> usize {
        match self.lines.iter().rposition(|line| matches!(line, Line::Item { .. })) {
            Some(last) => last + 1,
            None => self.lines.iter().rposition(|line| !matches!(line, Line::Other(other) if other.trim().is_empty())).map_or(0, |last| last + 1),
        }
    }

    /// `item` put first among those not done.
    pub fn add(&mut self, item: Item) {
        let lead = self.lead();
        let at = self.place_among_undone(0);
        self.lines.insert(at, Line::Item { lead, item: Item { done: false, ..item } });
    }

    /// The `n`th item ticked: done, first among those done. Where it was among those not done (to be put back there,
    /// unticked: see `untick`); none, where it was done already or is not there.
    pub fn tick(&mut self, n: usize) -> Option<usize> {
        let at = self.line_of(n)?;
        let Line::Item { item, .. } = &self.lines[at] else { return None };
        if item.done {
            return None;
        }
        let was = self.lines[..at].iter().filter(|line| matches!(line, Line::Item { item, .. } if !item.done)).count();
        let Line::Item { lead, mut item } = self.lines.remove(at) else { unreachable!() };
        item.done = true;
        let to = self.first_done().unwrap_or_else(|| self.after_items());
        self.lines.insert(to, Line::Item { lead, item });
        Some(was)
    }

    /// The `n`th item unticked: not done, back `was`th among those not done (none: last of them).
    pub fn untick(&mut self, n: usize, was: Option<usize>) {
        let Some(at) = self.line_of(n) else { return };
        let Line::Item { item, .. } = &self.lines[at] else { return };
        if !item.done {
            return;
        }
        let Line::Item { lead, mut item } = self.lines.remove(at) else { unreachable!() };
        item.done = false;
        let to = self.place_among_undone(was.unwrap_or(usize::MAX));
        self.lines.insert(to, Line::Item { lead, item });
    }

    /// The `n`th item's time.
    pub fn set_due(&mut self, n: usize, due: Option<Due>) {
        if let Some(item) = self.item_mut(n) {
            item.due = due;
        }
    }

    /// The `n`th item taken out.
    pub fn remove(&mut self, n: usize) {
        if let Some(at) = self.line_of(n) {
            self.lines.remove(at);
        }
    }

    /// Every item done taken out; how many.
    pub fn clear_done(&mut self) -> usize {
        let before = self.lines.len();
        self.lines.retain(|line| !matches!(line, Line::Item { item, .. } if item.done));
        before - self.lines.len()
    }
}

/// A line of the note, read: an item where it is a task list's (`- [ ] `, `* [x] `, `+ [X] `, indented or not), its
/// time taken from its end.
fn line(text: &str) -> Line {
    let trimmed = text.trim_start();
    let indent = &text[..text.len() - trimmed.len()];
    let mut chars = trimmed.chars();
    let item = match (chars.next(), chars.next(), chars.next(), chars.next(), chars.next(), chars.next()) {
        (Some(mark @ ('-' | '*' | '+')), Some(' '), Some('['), Some(state @ (' ' | 'x' | 'X')), Some(']'), Some(' ') | None) => Some((mark, state != ' ')),
        _ => None,
    };
    let Some((mark, done)) = item else { return Line::Other(text.to_string()) };
    let words = trimmed.get(6..).unwrap_or("").trim_end();
    let (words, due) = match words.rsplit_once(" @").and_then(|(words, due)| Some((words, Due::read(due)?))) {
        Some((words, due)) => (words.trim_end(), Some(due)),
        None => (words, None),
    };
    Line::Item { lead: format!("{indent}{mark} "), item: Item { text: words.to_string(), done, due } }
}

/// Words written to add an item (`today` as days go): the time they name taken from them, where they name one —
/// 今天, 明天, 后天, 周五 (星期五), 11月4日, and a time (18:00), in Chinese; today, tomorrow, a weekday (fri,
/// friday), and a time, in English. A weekday is the next of it (today's: a week on); a month and day this year's,
/// or next year's where it has gone; a time alone, today's. What is left, the item's words; all of them, where that
/// leaves nothing.
pub fn words(text: &str, today: Day) -> (String, Option<Due>) {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut kept: Vec<String> = Vec::new();
    let (mut day, mut time) = (None, None);
    for token in tokens {
        let (rest, found_day, found_time) = token_time(token, today);
        if found_day.is_some() && day.is_none() {
            day = found_day;
        } else if found_day.is_some() {
            kept.push(token.to_string());
            continue;
        }
        if found_time.is_some() && time.is_none() {
            time = found_time;
        }
        if !rest.is_empty() {
            kept.push(rest);
        }
    }
    let words = kept.join(" ");
    if words.is_empty() {
        return (text.trim().to_string(), None);
    }
    let due = match (day, time) {
        (Some(day), time) => Some(Due { day, time }),
        (None, Some(time)) => Some(Due { day: today, time: Some(time) }),
        (None, None) => None,
    };
    (words, due)
}

/// The day and the time a word written names (Chinese ones may run on into the words: `明天交报表`), and what of it
/// is left.
fn token_time(token: &str, today: Day) -> (String, Option<Day>, Option<(u32, u32)>) {
    let lower = token.to_lowercase();
    if let Some(time) = clock(token) {
        return (String::new(), None, Some(time));
    }
    const EN: [(&str, &str); 7] = [("mon", "monday"), ("tue", "tuesday"), ("wed", "wednesday"), ("thu", "thursday"), ("fri", "friday"), ("sat", "saturday"), ("sun", "sunday")];
    let english = match lower.as_str() {
        "today" => Some(today),
        "tomorrow" => Some(today.plus(1)),
        _ => EN.iter().position(|(short, long)| lower == *short || lower == *long).map(|weekday| next_weekday(today, weekday as u32)),
    };
    if english.is_some() {
        return (String::new(), english, None);
    }
    // Chinese: a day word anywhere in it, then a time after it (`明天18:00`).
    let (mut rest, mut day) = (token.to_string(), None);
    for (word, days) in [("今天", 0), ("明天", 1), ("后天", 2)] {
        if let Some(at) = rest.find(word) {
            day = Some(today.plus(days));
            rest.replace_range(at..at + word.len(), " ");
            break;
        }
    }
    if day.is_none() {
        for lead in ["星期", "周"] {
            let found = rest.find(lead).and_then(|at| {
                let after = &rest[at + lead.len()..];
                let c = after.chars().next()?;
                let weekday = "一二三四五六日".chars().position(|d| d == c).or((c == '天').then_some(6))?;
                Some((at, at + lead.len() + c.len_utf8(), weekday as u32))
            });
            if let Some((from, to, weekday)) = found {
                day = Some(next_weekday(today, weekday));
                rest.replace_range(from..to, " ");
                break;
            }
        }
    }
    if day.is_none() {
        day = month_day(&mut rest, today);
    }
    let mut time = None;
    if let Some((from, to, found)) = time_in(&rest) {
        time = Some(found);
        rest.replace_range(from..to, " ");
    }
    if day.is_none() && time.is_none() {
        return (token.to_string(), None, None);
    }
    (rest.split_whitespace().collect::<Vec<_>>().join(" "), day, time)
}

/// The next `weekday` (Monday 0) after `today`: a week on, today's.
fn next_weekday(today: Day, weekday: u32) -> Day {
    let ahead = (weekday as i64 - today.weekday() as i64).rem_euclid(7);
    today.plus(if ahead == 0 { 7 } else { ahead })
}

/// A month and day written in `rest` (`11月4日`, `11月4号`), taken out of it: this year's, or next year's where it
/// has gone.
fn month_day(rest: &mut String, today: Day) -> Option<Day> {
    let month_at = rest.find('月')?;
    let digits_before = rest[..month_at].chars().rev().take_while(char::is_ascii_digit).count();
    let from = month_at - digits_before;
    let after = &rest[month_at + '月'.len_utf8()..];
    let digits_after = after.chars().take_while(char::is_ascii_digit).count();
    let mark = after[digits_after..].chars().next().filter(|c| *c == '日' || *c == '号')?;
    if digits_before == 0 || digits_before > 2 || digits_after == 0 || digits_after > 2 {
        return None;
    }
    let (month, day): (u32, u32) = (rest[from..month_at].parse().ok()?, after[..digits_after].parse().ok()?);
    let this_year = Day::of(today.year, month, day);
    let found = match this_year {
        Some(this) if this >= today => this,
        _ => Day::of(today.year + 1, month, day)?,
    };
    let to = month_at + '月'.len_utf8() + digits_after + mark.len_utf8();
    rest.replace_range(from..to, " ");
    Some(found)
}

/// A time of day written in `text` (`18:00`, `7：30`): where it is, and the time.
fn time_in(text: &str) -> Option<(usize, usize, (u32, u32))> {
    let colon = text.find([':', '：'])?;
    let before = text[..colon].chars().rev().take_while(char::is_ascii_digit).count();
    let colon_len = text[colon..].chars().next()?.len_utf8();
    let after = text[colon + colon_len..].chars().take_while(char::is_ascii_digit).count();
    let (from, to) = (colon - before, colon + colon_len + after);
    Some((from, to, clock(&text[from..to])?))
}

/// The to-do note among `notes`: the first of its names there (see `NAMES`).
pub fn note_of(notes: &[crate::notes::Note]) -> Option<&crate::notes::Note> {
    NAMES.iter().find_map(|name| notes.iter().find(|note| note.name.eq_ignore_ascii_case(name)))
}

/// The items as they are now, and the note as Glance last read or wrote it: a change made here shows at once, and is
/// written on a thread of its own (see `change`); one made outside (another editor, a sync) is taken as the notes are
/// read again, where nothing made here waits to be written.
struct State {
    /// The note's name in the folder (none: there is none yet).
    name: Option<String>,
    list: List,
    /// Its file as last read or written here (none: never, or it was gone).
    stamp: Option<crate::notes::store::Stamp>,
    /// The note as the notes read last had it, as it was last looked at (none: not there): what they read is taken only
    /// as it changes (written here, they still have it as it was, till they are read again).
    seen: Option<(String, crate::notes::store::Stamp)>,
    /// How many changes were made here, and how many of them have been written.
    made: u64,
    written: u64,
    /// Where each item ticked here was among those not done, to go back there unticked (see `List::tick`).
    was: Vec<(Item, usize)>,
}

static STATE: std::sync::Mutex<State> = std::sync::Mutex::new(State { name: None, list: List { lines: Vec::new(), ended: false }, stamp: None, seen: None, made: 0, written: 0, was: Vec::new() });
static WRITE: std::sync::Condvar = std::sync::Condvar::new();

impl State {
    /// As the folder was last read: the note there taken, where the notes read changed since last looked at, it is not
    /// as last read or written here, and nothing made here waits to be written. Gone from the folder: no items.
    fn follow(&mut self) {
        if self.made != self.written {
            return;
        }
        let notes = crate::notes::shown();
        let note = note_of(&notes);
        let now = note.map(|note| (note.name.clone(), note.stamp));
        if now == self.seen {
            return;
        }
        self.seen = now;
        match note {
            Some(note) if self.name.as_deref() == Some(note.name.as_str()) && self.stamp.is_some_and(|stamp| stamp.same(&note.stamp)) => {}
            Some(note) => {
                self.list = List::read(&note.text);
                self.stamp = Some(note.stamp);
                self.name = Some(note.name.clone());
            }
            None => {
                (self.list, self.stamp, self.name) = (List::default(), None, None);
            }
        }
    }
}

/// The to-do note's items as they are now.
pub fn list() -> List {
    let mut state = STATE.lock().unwrap();
    state.follow();
    state.list.clone()
}

/// `text` added as an item, first among those not done, its time the one its words name (see `words`), or `due`
/// where one is set for it: its words as kept. Nothing, with no words left.
pub fn add(text: &str, due: Option<Due>) -> Option<String> {
    let (words, named) = words(text, Day::today());
    if words.trim().is_empty() {
        return None;
    }
    change(|list| list.add(Item { text: words.clone(), done: false, due: due.or(named) }));
    Some(words)
}

/// The items changed here by `change`: shown so at once, and written (see `start`). What it gives back.
pub fn change<R>(change: impl FnOnce(&mut List) -> R) -> R {
    let mut state = STATE.lock().unwrap();
    state.follow();
    let given = change(&mut state.list);
    state.made += 1;
    WRITE.notify_one();
    given
}

/// The `n`th item, as `item`, ticked or unticked (`done`): unticked, back where it was among those not done, where it
/// was ticked here. Nothing, where the `n`th is no longer that item (changed outside meanwhile).
pub fn tick(n: usize, item: &Item, done: bool) {
    let mut state = STATE.lock().unwrap();
    state.follow();
    if state.list.items().get(n).copied() != Some(item) {
        return;
    }
    if done {
        if let Some(was) = state.list.tick(n) {
            state.was.retain(|(kept, _)| kept != item);
            state.was.push((Item { done: true, ..item.clone() }, was));
        }
    } else {
        let at = state.was.iter().position(|(kept, _)| kept == item);
        let was = at.map(|at| state.was.remove(at).1);
        state.list.untick(n, was);
    }
    state.made += 1;
    WRITE.notify_one();
}

/// The `n`th item, as `item`, changed by `change` (its time set, taken out): nothing, where it is no longer that item.
pub fn change_item(n: usize, item: &Item, change: impl FnOnce(&mut List, usize)) {
    let mut state = STATE.lock().unwrap();
    state.follow();
    if state.list.items().get(n).copied() != Some(item) {
        return;
    }
    change(&mut state.list, n);
    state.made += 1;
    WRITE.notify_one();
}

/// Writes what is changed here (see `change`), on a thread of its own: the last of the changes made by the time it
/// writes, in the note's place, unless it was changed outside since it was read (then kept beside it as a conflict's
/// copy, as a note is: see `notes::store`); `told` after each write.
pub fn start(told: impl Fn() + Send + 'static) {
    let _ = std::thread::Builder::new().name("glance: to-do".into()).spawn(move || loop {
        let (made, name, text, was) = {
            let mut state = WRITE.wait_while(STATE.lock().unwrap(), |state| state.made == state.written).unwrap();
            let name = state.name.get_or_insert_with(|| crate::update::language().pick(NAMES[0], NAMES[1]).to_string()).clone();
            (state.made, name, state.list.written(), state.stamp)
        };
        let folder = crate::notes::folder();
        let written = folder.write(&name, &text, was);
        let mut state = STATE.lock().unwrap();
        match written {
            Ok(crate::notes::Written::Done(stamp)) => state.stamp = Some(stamp),
            // Changed outside meanwhile: what was written here is beside it; it is taken as it is there now.
            Ok(crate::notes::Written::Apart(..)) => state.stamp = None,
            Err(error) => {
                crate::journal::note(format!("to-do: not written ({error})"));
                state.stamp = None;
            }
        }
        state.written = made;
        drop(state);
        crate::notes::read_again();
        told();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(year: i32, month: u32, d: u32) -> Day {
        Day { year, month, day: d }
    }

    #[test]
    fn days_count_as_the_calendar_does() {
        assert_eq!(day(1970, 1, 1).number(), 0);
        assert_eq!(day(2000, 3, 1).number() - day(2000, 2, 28).number(), 2);
        assert_eq!(day(1900, 3, 1).number() - day(1900, 2, 28).number(), 1);
        for number in [-800_000, -1, 0, 59, 10_957, 20_760, 800_000] {
            assert_eq!(Day::from_number(number).number(), number);
        }
        // 2026-11-03 is a Tuesday.
        assert_eq!(day(2026, 11, 3).weekday(), 1);
        assert_eq!(day(2026, 12, 31).plus(1), day(2027, 1, 1));
        assert_eq!(Day::of(2026, 2, 29), None);
        assert_eq!(Day::of(2028, 2, 29), Some(day(2028, 2, 29)));
    }

    #[test]
    fn reads_and_writes_the_note_as_it_was() {
        let text = "# 待办\n\n- [ ] 交季度报表 @2026-11-04 18:00\n  * [x] 续订网盘 @2026-11-10\n- [ ] 写 @ 符号的说明\n普通的一行\n";
        let list = List::read(text);
        assert_eq!(list.written(), text);
        let items = list.items();
        assert_eq!(items[0], &Item { text: "交季度报表".into(), done: false, due: Some(Due { day: day(2026, 11, 4), time: Some((18, 0)) }) });
        assert_eq!(items[1], &Item { text: "续订网盘".into(), done: true, due: Some(Due { day: day(2026, 11, 10), time: None }) });
        // (An @ that is not followed by a time is words.)
        assert_eq!(items[2].text, "写 @ 符号的说明");
        assert_eq!(items[2].due, None);
        assert_eq!(List::read("").written(), "");
        assert_eq!(List::read("- [ ] a").written(), "- [ ] a\n");
    }

    #[test]
    fn ticked_goes_first_among_the_done_and_unticked_back_where_it_was() {
        let mut list = List::read("# 待办\n- [ ] a\n- [ ] b\n- [ ] c\n- [x] old\n");
        let was = list.tick(1);
        assert_eq!(was, Some(1));
        assert_eq!(list.written(), "# 待办\n- [ ] a\n- [ ] c\n- [x] b\n- [x] old\n");
        // (The 3rd item now: b.)
        list.untick(2, was);
        assert_eq!(list.written(), "# 待办\n- [ ] a\n- [ ] b\n- [ ] c\n- [x] old\n");
        // Unticked with no place known: last of those not done.
        list.untick(3, None);
        assert_eq!(list.written(), "# 待办\n- [ ] a\n- [ ] b\n- [ ] c\n- [ ] old\n");
        // All done, and one unticked: first, before those done.
        let mut all = List::read("- [x] a\n- [x] b\n");
        all.untick(1, Some(0));
        assert_eq!(all.written(), "- [ ] b\n- [x] a\n");
        assert_eq!(all.tick(1), None);
    }

    #[test]
    fn added_first_and_cleared() {
        let mut list = List::read("# 待办\n\n* [ ] a\n* [x] b\n");
        list.add(Item { text: "new".into(), done: true, due: Some(Due { day: day(2026, 11, 5), time: None }) });
        assert_eq!(list.written(), "# 待办\n\n* [ ] new @2026-11-05\n* [ ] a\n* [x] b\n");
        assert_eq!(list.clear_done(), 1);
        list.set_due(1, Some(Due { day: day(2026, 11, 6), time: Some((9, 30)) }));
        list.remove(0);
        assert_eq!(list.written(), "# 待办\n\n* [ ] a @2026-11-06 09:30\n");
        // A note of words alone: the first item after them, before the blank lines ending it.
        let mut words = List::read("# 待办\n\n");
        words.add(Item { text: "x".into(), done: false, due: None });
        assert_eq!(words.written(), "# 待办\n- [ ] x\n\n");
        let mut empty = List::default();
        empty.add(Item { text: "x".into(), done: false, due: None });
        assert_eq!(empty.written(), "- [ ] x\n");
    }

    #[test]
    fn time_words_are_taken_from_the_words() {
        // A Tuesday.
        let today = day(2026, 11, 3);
        let due = |d: Day, time: Option<(u32, u32)>| Some(Due { day: d, time });
        assert_eq!(words("买牛奶 明天", today), ("买牛奶".into(), due(day(2026, 11, 4), None)));
        assert_eq!(words("周五 18:00 交报表", today), ("交报表".into(), due(day(2026, 11, 6), Some((18, 0)))));
        assert_eq!(words("明天18:00交报表", today), ("交报表".into(), due(day(2026, 11, 4), Some((18, 0)))));
        assert_eq!(words("周二开会", today), ("开会".into(), due(day(2026, 11, 10), None)));
        assert_eq!(words("星期天 爬山", today), ("爬山".into(), due(day(2026, 11, 8), None)));
        assert_eq!(words("1月2日 续费", today), ("续费".into(), due(day(2027, 1, 2), None)));
        assert_eq!(words("11月30号续费", today), ("续费".into(), due(day(2026, 11, 30), None)));
        assert_eq!(words("call mom tomorrow 7:30", today), ("call mom".into(), due(day(2026, 11, 4), Some((7, 30)))));
        assert_eq!(words("pay rent fri", today), ("pay rent".into(), due(day(2026, 11, 6), None)));
        assert_eq!(words("standup 9:15", today), ("standup".into(), due(today, Some((9, 15)))));
        assert_eq!(words("整理桌面截图", today), ("整理桌面截图".into(), None));
        // Only a time word: the words as written, no time taken.
        assert_eq!(words("明天", today), ("明天".into(), None));
        // (Not a time: 25:00, 1:5.)
        assert_eq!(words("25:00 x", today), ("25:00 x".into(), None));
    }

    #[test]
    fn labels_say_when() {
        let today = day(2026, 11, 3);
        let label = |d: Day, time| Due { day: d, time }.label(today, true);
        assert_eq!(label(today, Some((17, 0))), "今天 17:00");
        assert_eq!(label(day(2026, 11, 4), None), "明天");
        assert_eq!(label(day(2026, 11, 5), None), "后天");
        assert_eq!(label(day(2026, 11, 6), None), "周五");
        assert_eq!(label(day(2026, 11, 10), None), "11月10日");
        assert_eq!(label(day(2026, 11, 1), None), "11月1日");
        assert_eq!(label(day(2027, 1, 2), None), "2027年1月2日");
        assert_eq!(Due { day: day(2026, 11, 5), time: None }.label(today, false), "Thu");
        assert_eq!(Due { day: day(2026, 12, 25), time: Some((9, 0)) }.label(today, false), "Dec 25 09:00");
        assert!(Due { day: day(2026, 11, 1), time: None }.by(today) && !Due { day: day(2026, 11, 4), time: None }.by(today));
    }
}
