//! The AI component's lanes (see `crate::ai`): what every source spent today, as a figure and how it mounted through
//! the day; then each source's lane, its maker's mark and its plan, each window of its limit as a bar (how much of it
//! is used, how much of the window has gone by, when it starts again), and, opened by its head, what it spent today
//! by model and by kind, and its sessions.

use std::sync::atomic::{AtomicU8, Ordering};

use super::*;
use crate::ui::marks::Mark;

/// A source of the AI component: a tool whose records or limits it reads.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Source {
    /// Claude Code.
    Claude,
    /// Codex, whose limits are ChatGPT's.
    Codex,
}

impl Source {
    /// The source lane `id` is of.
    pub fn of(id: &str) -> Option<Source> {
        match id {
            "ai:claude" => Some(Source::Claude),
            "ai:codex" => Some(Source::Codex),
            _ => None,
        }
    }

    fn mark(self) -> Mark {
        match self {
            Source::Claude => Mark::Claude,
            Source::Codex => Mark::OpenAi,
        }
    }

    /// The name it goes by: its maker's (Codex's limits are ChatGPT's plan's).
    fn name(self) -> &'static str {
        match self {
            Source::Claude => "Claude",
            Source::Codex => "ChatGPT",
        }
    }

    /// The tool its records are of.
    fn tool_name(self) -> &'static str {
        match self {
            Source::Claude => "Claude Code",
            Source::Codex => "Codex",
        }
    }

    fn bit(self) -> u8 {
        match self {
            Source::Claude => 1,
            Source::Codex => 2,
        }
    }
}

/// The sources whose lanes are opened (one bit each: see `Source::bit`), in every window alike.
static OPEN: AtomicU8 = AtomicU8::new(0);

/// `source`'s lane opened, or closed again.
pub fn toggle(source: Source) {
    OPEN.fetch_xor(source.bit(), Ordering::Relaxed);
}

fn opened(source: Source) -> bool {
    OPEN.load(Ordering::Relaxed) & source.bit() != 0
}

/// What an AI lane draws, under its head.
#[derive(Clone, PartialEq, Debug)]
pub enum AiBlock {
    /// Today's tokens, as a figure and its unit, and how they mounted through the day: what was spent by the end of
    /// each quarter hour so far, from 0 to 1 of the whole.
    Today { figure: String, unit: String, mounted: Vec<f32> },
    /// The windows of a limit, two side by side: each one's name, how much of it is used (0 to 1), how much of the
    /// window has gone by (where it is told when it starts again), and when it does.
    Windows(Vec<Window>),
    /// What each model spent today, the most first: its name, its share, and how much.
    Models(Vec<(String, f32, String)>),
    /// Facts two to a row: what, and how much.
    Pairs(Vec<(String, String)>),
    /// The sessions that spent today, the latest first: each one's title, its model and when it last spent, how
    /// much it spent today, and whether it is at work now.
    Sessions(Vec<(String, String, String, bool)>),
}

#[derive(Clone, PartialEq, Debug)]
pub struct Window {
    pub name: String,
    pub used: f32,
    pub gone: Option<f32>,
    pub resets: String,
}

impl Hash for AiBlock {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            AiBlock::Today { figure, unit, mounted } => (figure, unit, mounted.iter().map(|f| f.to_bits()).collect::<Vec<_>>()).hash(state),
            AiBlock::Windows(windows) => windows.iter().for_each(|w| (&w.name, w.used.to_bits(), w.gone.map(f32::to_bits), &w.resets).hash(state)),
            AiBlock::Models(rows) => rows.iter().for_each(|(name, share, value)| (name, share.to_bits(), value).hash(state)),
            AiBlock::Pairs(rows) => rows.hash(state),
            AiBlock::Sessions(rows) => rows.hash(state),
        }
    }
}

/// The room above each block (DIPs).
const GAP: f32 = 10.0;
/// A window's: its name and how much is used, its bar, when it starts again.
const WINDOW: f32 = LINE + 8.0 + LINE;
/// Between two windows side by side, and two rows of them.
const WINDOW_GAP: f32 = 16.0;
/// The models' bar, and the room under it.
const MODELS_BAR: f32 = 6.0;
const MODELS_BAR_GAP: f32 = 8.0;
/// A session's row: its title over what it is.
const SESSION: f32 = 36.0;
/// The most sessions told of.
const SESSIONS: usize = 4;
/// The most models told apart (the rest as one).
const MODELS: usize = 3;
/// A session is at work while it spent this lately (ms).
const AT_WORK_MS: u64 = 3 * 60 * 1000;
/// A window used this much is drawn in the signal colour.
const HOT: f32 = 0.9;

impl AiBlock {
    pub(super) fn height(&self) -> f32 {
        match self {
            AiBlock::Today { .. } => PLOT,
            AiBlock::Windows(windows) => {
                let rows = windows.len().div_ceil(2) as f32;
                GAP + rows * WINDOW + (rows - 1.0).max(0.0) * WINDOW_GAP
            }
            AiBlock::Models(rows) => GAP + MODELS_BAR + MODELS_BAR_GAP + rows.len() as f32 * (LINE + FACT_GAP) - FACT_GAP,
            AiBlock::Pairs(pairs) => GAP + pairs.len().div_ceil(2) as f32 * (LINE + FACT_GAP) - FACT_GAP,
            AiBlock::Sessions(rows) => GAP + rows.len() as f32 * SESSION,
        }
    }
}

/// Tokens as a figure and its unit: in 万 and 亿 in Chinese, else in thousands, millions and billions, three figures
/// at most past ten.
pub fn tokens(count: u64, lang: Lang) -> (String, String) {
    let n = count as f64;
    let steps: &[(f64, &str)] = match lang {
        Lang::Zh => &[(1e8, "亿 tokens"), (1e4, "万 tokens")],
        Lang::En => &[(1e9, "B tokens"), (1e6, "M tokens"), (1e3, "K tokens")],
    };
    match steps.iter().find(|(step, _)| n >= *step) {
        Some((step, unit)) => (figure(n / step), (*unit).into()),
        None => (grouped(count), "tokens".into()),
    }
}

/// Tokens in a few characters, as a list shows them ("903 万", "9.0M").
fn short(count: u64, lang: Lang) -> String {
    let (figure, unit) = tokens(count, lang);
    match unit.strip_suffix("tokens").map(str::trim_end) {
        Some("") | None => figure,
        Some(unit) if lang == Lang::Zh => format!("{figure} {unit}"),
        Some(unit) => format!("{figure}{unit}"),
    }
}

/// A number of a unit: one decimal under ten, else whole, its thousands grouped.
fn figure(value: f64) -> String {
    if value < 10.0 { format!("{value:.1}") } else { grouped(value.round() as u64) }
}

fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// A model as its maker names it: Claude's by family and version ("claude-opus-4-6" as "Opus 4.6", the date of a
/// snapshot left off); others as they are.
pub fn model_name(id: &str) -> String {
    let Some(rest) = id.strip_prefix("claude-") else { return id.to_string() };
    let (mut family, mut version) = (Vec::new(), Vec::new());
    for part in rest.split('-') {
        if part.chars().all(|c| c.is_ascii_digit()) {
            // (A snapshot's date, "20250929", is no part of its name.)
            if part.len() <= 2 {
                version.push(part);
            }
        } else {
            let mut chars = part.chars();
            family.push(chars.next().map_or(String::new(), |first| first.to_uppercase().chain(chars).collect()));
        }
    }
    match (family.is_empty(), version.is_empty()) {
        (true, _) => id.to_string(),
        (false, true) => family.join(" "),
        (false, false) => format!("{} {}", family.join(" "), version.join(".")),
    }
}

/// A window by its length: five hours, a day, a week, else so many hours or days.
fn window_name(minutes: u32, lang: Lang) -> String {
    match minutes {
        1440 => lang.pick("每天", "Daily").into(),
        10080 => lang.pick("每周", "Weekly").into(),
        m if m % 1440 == 0 => match lang {
            Lang::Zh => format!("{} 天", m / 1440),
            Lang::En => format!("{} days", m / 1440),
        },
        m if m % 60 == 0 => match lang {
            Lang::Zh => format!("{} 小时", m / 60),
            Lang::En => format!("{} hours", m / 60),
        },
        m => match lang {
            Lang::Zh => format!("{m} 分钟"),
            Lang::En => format!("{m} min"),
        },
    }
}

/// How long till a window starts again, `minutes` from now: in days and hours, hours and minutes, or minutes, a
/// part of nothing left unsaid ("2 小时", not "2 小时 0 分").
fn left_in(minutes: u64, lang: Lang) -> String {
    let (days, hours, minutes) = (minutes / 1440, minutes % 1440 / 60, minutes % 60);
    let (big, small, units) = match (days, hours) {
        (0, 0) => (minutes, 0, lang.pick("分钟|", "min|")),
        (0, _) => (hours, minutes, lang.pick("小时|分", "h|min")),
        _ => (days, hours, lang.pick("天|小时", "d|h")),
    };
    let (big_unit, small_unit) = units.split_once('|').unwrap();
    if small == 0 { format!("{big} {big_unit}") } else { format!("{big} {big_unit} {small} {small_unit}") }
}

/// A plan as it is sold ("pro" as "Pro").
fn plan_name(plan: &str) -> String {
    let mut chars = plan.chars();
    chars.next().map_or(String::new(), |first| first.to_uppercase().chain(chars).collect())
}

#[cfg(windows)]
fn tool_of(source: Source) -> crate::ai::Tool {
    match source {
        Source::Claude => crate::ai::Tool::Claude,
        Source::Codex => crate::ai::Tool::Codex,
    }
}

/// The lane of AI module `id` as `prefs` show it: what every source spent today (`ai:today`), or one source's; none
/// for an id of none.
#[cfg(windows)]
pub(super) fn ai_lane(id: &str, prefs: &Prefs, lang: Lang) -> Option<Vec<Block>> {
    lane_of(id, prefs, lang, crate::ai::usage().as_deref(), crate::station::now_ms(), Source::of(id).is_some_and(opened))
}

/// As `ai_lane`, of `usage` (none: not read yet) at `now` (ms since 1970), a source's lane `open` or not.
#[cfg(windows)]
pub(super) fn lane_of(id: &str, prefs: &Prefs, lang: Lang, usage: Option<&crate::ai::Usage>, now: u64, open: bool) -> Option<Vec<Block>> {
    let reading = || Block::Facts { rows: vec![(lang.pick("今日用量", "Today").into(), lang.pick("正在读取", "Reading").into(), false)], gap: GAP };
    if id == "ai:today" {
        let head = head(lang.pick("AI 用量", "AI usage"), "", lang.pick("今日", "Today"), false);
        let Some(usage) = usage else { return Some(vec![head, reading()]) };
        let total: u64 = usage.spent_today.iter().map(|(_, _, tokens)| tokens.total()).sum();
        let (figure, unit) = tokens(total, lang);
        // (Every tool's quarters added up, then how far the day had come by each.)
        let quarters = usage.quarters.iter().map(|(_, spent)| spent.len()).max().unwrap_or(0);
        let mut mounted = Vec::with_capacity(quarters);
        let mut sum = 0;
        for at in 0..quarters {
            sum += usage.quarters.iter().map(|(_, spent)| spent.get(at).copied().unwrap_or(0)).sum::<u64>();
            mounted.push(if total == 0 { 0.0 } else { sum as f32 / total as f32 });
        }
        let mut blocks = vec![head, Block::Ai(AiBlock::Today { figure, unit, mounted })];
        // Each tool's share, where there are more than one.
        let tools: Vec<(crate::ai::Tool, u64)> = usage.days.iter().map(|(tool, days)| (*tool, days[6])).filter(|(_, spent)| *spent > 0).collect();
        if tools.len() > 1 {
            let named = |tool| if tool == crate::ai::Tool::Claude { Source::Claude.tool_name() } else { Source::Codex.tool_name() };
            blocks.push(Block::Facts { rows: tools.into_iter().map(|(tool, spent)| (named(tool).into(), short(spent, lang), false)).collect(), gap: GAP });
        }
        return Some(blocks);
    }
    let source = Source::of(id)?;
    let tool = tool_of(source);
    let Some(usage) = usage else { return Some(vec![head(source.name(), source.tool_name(), "", false), reading()]) };
    // Its limit as it was last told: the tool's own (Codex's "codex"), else the first told.
    let limits: Vec<&crate::ai::Limit> = usage.limits.iter().filter(|limit| limit.tool == tool).collect();
    let limit = limits.iter().find(|limit| limit.id == "codex").or_else(|| limits.first()).copied();
    let sessions: Vec<&crate::ai::Session> = usage.sessions.iter().filter(|session| session.tool == tool && session.spent_between(usage.today, usage.now) > 0).collect();
    // When it was last heard of: its limit told, else its last session's spending.
    let heard = limit.map(|limit| limit.told).into_iter().chain(sessions.first().map(|session| session.last)).max();
    let aside = heard.map_or(String::new(), |heard| text::since(heard, now, lang));
    let device = limit.and_then(|limit| limit.plan.as_deref()).map_or(source.tool_name().to_string(), plan_name);
    let mut blocks = vec![head(source.name(), device, aside, false)];
    let windows: Vec<Window> = limit.map_or(Vec::new(), |limit| {
        limit
            .windows
            .iter()
            .map(|window| {
                let length = window.minutes as u64 * 60_000;
                // (Started again since it was told: none of it used.)
                let over = window.resets.is_some_and(|resets| resets <= now);
                let resets = match window.resets {
                    Some(_) if over => lang.pick("已重置", "Reset").into(),
                    Some(resets) => {
                        let left = left_in((resets - now).div_ceil(60_000), lang);
                        match lang {
                            Lang::Zh => format!("{left}后重置"),
                            Lang::En => format!("Resets in {left}"),
                        }
                    }
                    None => String::new(),
                };
                Window {
                    name: window_name(window.minutes, lang),
                    used: if over { 0.0 } else { (window.used / 100.0).clamp(0.0, 1.0) },
                    gone: window.resets.filter(|_| !over && length > 0).map(|resets| (1.0 - (resets - now) as f32 / length as f32).clamp(0.0, 1.0)),
                    resets,
                }
            })
            .collect()
    });
    let spent: Vec<&(crate::ai::Tool, String, crate::ai::Tokens)> = usage.spent_today.iter().filter(|(of, _, _)| *of == tool).collect();
    let mut total = crate::ai::Tokens::default();
    for (_, _, tokens) in &spent {
        total.add(*tokens);
    }
    if !windows.is_empty() {
        blocks.push(Block::Ai(AiBlock::Windows(windows)));
    }
    // Opened (where it spent today): what it spent, by model and by kind, and its sessions; else, with no limit to
    // show, what it spent today.
    if !open || total.total() == 0 {
        if limit.is_none() {
            blocks.push(Block::Facts { rows: vec![(lang.pick("今日用量", "Today").into(), short(total.total(), lang), false)], gap: GAP });
        }
        return Some(blocks);
    }
    let whole = total.total().max(1) as f32;
    let mut models: Vec<(String, f32, String)> = spent.iter().take(MODELS).map(|(_, model, tokens)| (model_name(model), tokens.total() as f32 / whole, short(tokens.total(), lang))).collect();
    let rest: u64 = spent.iter().skip(MODELS).map(|(_, _, tokens)| tokens.total()).sum();
    if rest > 0 {
        models.push((lang.pick("其他", "Other").into(), rest as f32 / whole, short(rest, lang)));
    }
    if prefs.shows(id, "models") {
        blocks.push(Block::Ai(AiBlock::Models(models)));
    }
    let read_from = total.input + total.cache_read + total.cache_write;
    let hit = if read_from == 0 { 0.0 } else { total.cache_read as f32 / read_from as f32 };
    let mut pairs = vec![
        (lang.pick("输入", "Input").to_string(), short(total.input, lang)),
        (lang.pick("输出", "Output").to_string(), short(total.output, lang)),
        (lang.pick("缓存读取", "Cache read").to_string(), short(total.cache_read, lang)),
    ];
    // (Codex tells no cache written, and tells its reasoning apart.)
    if total.cache_write > 0 || total.reasoning == 0 {
        pairs.push((lang.pick("缓存写入", "Cache write").into(), short(total.cache_write, lang)));
    } else {
        pairs.push((lang.pick("推理", "Reasoning").into(), short(total.reasoning, lang)));
    }
    pairs.push((lang.pick("缓存命中", "Cache hits").into(), format!("{:.0}%", hit * 100.0)));
    pairs.push((lang.pick("会话", "Sessions").into(), sessions.len().to_string()));
    blocks.push(Block::Ai(AiBlock::Pairs(pairs)));
    if prefs.shows(id, "sessions") && !sessions.is_empty() {
        let rows = sessions
            .iter()
            .take(SESSIONS)
            .map(|session| {
                let title = session.title.clone().unwrap_or_else(|| lang.pick("未命名会话", "Untitled session").into());
                let model = session.model.as_deref().map(model_name).unwrap_or_default();
                let when = text::since(session.last, now, lang);
                let about = if model.is_empty() { when } else { format!("{model} · {when}") };
                (title, about, short(session.spent_between(usage.today, usage.now), lang), now.saturating_sub(session.last) < AT_WORK_MS)
            })
            .collect();
        blocks.push(Block::Ai(AiBlock::Sessions(rows)));
    }
    Some(blocks)
}

/// A source's head: its maker's mark before its name, its plan after it, and at its end when it was last heard of
/// and whether it opens; the whole of it opens the lane, where it spent today.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_head(frame: &dyn Canvas, scene: &Scene, (left, y, width): (f32, f32, f32), source: Source, title: &str, device: &str, aside: &str, hits: &mut Vec<HitBox>, name: &dyn Fn(&str)) {
    let theme = scene.theme;
    const MARK: f32 = 16.0;
    let row = y + (HEAD - theme.title.size) / 2.0 - 1.0;
    name("mark");
    frame.mark(source.mark(), Point { x: left + MARK / 2.0, y: y + HEAD / 2.0 }, MARK, theme.text, 1.0);
    let title_left = left + MARK + 7.0;
    let title_width = frame.measure(title, theme.title);
    name("label");
    frame.text(title, theme.title, theme.text, title_left, row, width - (title_left - left), Align::Start);
    // (It opens only where there is something to open: see `ai_lane`.)
    let opens = scene_opens(scene, source);
    let chevron = if opens { 16.0 + 4.0 } else { 0.0 };
    let aside_width = if aside.is_empty() { 0.0 } else { frame.measure(aside, theme.small) };
    let device_left = title_left + title_width + 8.0;
    name("device");
    frame.text(device, theme.small, theme.text3, device_left, row + 1.0, (left + width - chevron - aside_width - 8.0 - device_left).max(0.0), Align::Start);
    name("detail");
    frame.text(aside, theme.small, theme.text3, left, row + 1.0, width - chevron, Align::End);
    if opens {
        let hit = Hit::Ai(source);
        let lit = scene.hover == Some(hit);
        let icon = if opened(source) { Icon::ChevronUp } else { Icon::ChevronDown };
        frame.icon(icon, Point { x: left + width - 8.0, y: y + HEAD / 2.0 }, 14.0, if lit { theme.text } else { theme.text3 });
        hits.push((left, y, width, HEAD, hit));
    }
}

/// Whether `source`'s lane has anything to open: it spent today.
fn scene_opens(_scene: &Scene, source: Source) -> bool {
    #[cfg(windows)]
    {
        crate::ai::usage().is_some_and(|usage| usage.spent_today.iter().any(|(tool, _, tokens)| *tool == tool_of(source) && tokens.total() > 0))
    }
    #[cfg(not(windows))]
    {
        let _ = source;
        false
    }
}

/// Draws `block` in its lane's ink, from `y` across `width`.
pub(super) fn paint(frame: &dyn Canvas, scene: &Scene, ink: Ink, block: &AiBlock, (left, y, width): (f32, f32, f32), part: &dyn Fn(&str)) {
    let theme = scene.theme;
    match block {
        AiBlock::Today { figure, unit, mounted } => {
            let bottom = y + PLOT;
            let figure_width = frame.measure(figure, theme.figure);
            let unit_width = frame.measure(unit, theme.unit);
            part("value");
            text_on_line(frame, figure, theme.figure, theme.text, left, bottom, theme.figure.size * theme.figure_line, width);
            part("unit");
            text_on_line(frame, unit, theme.unit, theme.text2, left + figure_width + 3.0, bottom, theme.unit.size, (width - figure_width - 3.0).max(30.0));
            // How it mounted, in the room after the figure (the chart's room at least), over the quarters so far.
            let spark_left = (left + LABEL + LABEL_GAP).max(left + figure_width + 3.0 + unit_width + 16.0);
            let spark_width = left + width - spark_left;
            if mounted.len() < 2 || spark_width < 40.0 {
                return;
            }
            part("chart");
            let (top, foot) = (y + 6.0, bottom - 2.0);
            let points: Vec<Point> = mounted
                .iter()
                .enumerate()
                .map(|(at, share)| Point { x: spark_left + at as f32 / (mounted.len() - 1) as f32 * spark_width, y: foot - share.clamp(0.0, 1.0) * (foot - top) })
                .collect();
            let mut area = points.clone();
            area.push(Point { x: spark_left + spark_width, y: foot });
            area.push(Point { x: spark_left, y: foot });
            let wash = if ink.wash == ink.wash_end { Fill::Solid(ink.wash) } else { Fill::Down { top, from: ink.wash, bottom: foot, to: ink.wash_end } };
            frame.fill_shape(&area, wash);
            frame.stroke(&points, ink.trace, 1.6);
            if let Some(last) = points.last() {
                frame.fill_circle(ink.trace, *last, 2.6);
            }
        }
        AiBlock::Windows(windows) => {
            let column = (width - WINDOW_GAP) / 2.0;
            for (at, window) in windows.iter().enumerate() {
                let (x, top) = (left + (at % 2) as f32 * (column + WINDOW_GAP), y + GAP + (at / 2) as f32 * (WINDOW + WINDOW_GAP));
                // (One alone across the lane.)
                let wide = if windows.len() == 1 { width } else { column };
                let hot = window.used >= HOT;
                part(&format!("{at}.label"));
                frame.text(&window.name, theme.small, theme.text2, x, top, wide, Align::Start);
                part(&format!("{at}.value"));
                frame.text(&format!("{:.0}%", window.used * 100.0), theme.small, if hot { theme.signal } else { theme.text }, x, top, wide, Align::End);
                let bar_y = top + LINE + (8.0 - theme.meter_height) / 2.0;
                part(&format!("{at}.bar"));
                meter(frame, theme, (x, bar_y, wide), window.used, if hot { theme.signal } else { ink.trace });
                // How much of the window has gone by, a mark across the bar: used ahead of it, the limit is spent
                // faster than the window goes.
                if let Some(gone) = window.gone {
                    let mark_x = x + gone * wide;
                    frame.fill(theme.text2, (mark_x - 0.75).clamp(x, x + wide - 1.5), bar_y - 2.0, 1.5, theme.meter_height + 4.0);
                }
                part(&format!("{at}.resets"));
                frame.text(&window.resets, theme.small, theme.text3, x, top + LINE + 8.0, wide, Align::Start);
            }
        }
        AiBlock::Models(rows) => {
            let bar_y = y + GAP;
            part("bar");
            let (bx, by, bw, bh) = frame.on_pixels(left, bar_y, width, MODELS_BAR);
            frame.fill_rounded(theme.track, bx, by, bw, bh, theme.meter_radius.min(MODELS_BAR / 2.0));
            let mut x = bx;
            for (at, (_, share, _)) in rows.iter().enumerate() {
                let w = share * bw;
                frame.fill(ink.trace.alpha(strength(at)), x, by, w, bh);
                x += w;
            }
            for (at, (model, _, value)) in rows.iter().enumerate() {
                let row_y = bar_y + MODELS_BAR + MODELS_BAR_GAP + at as f32 * (LINE + FACT_GAP);
                part(&at.to_string());
                frame.fill_rounded(ink.trace.alpha(strength(at)), left, row_y + (LINE - 8.0) / 2.0, 8.0, 8.0, 2.0);
                frame.text(model, theme.small, theme.text2, left + 14.0, row_y, width - 14.0, Align::Start);
                frame.text(value, theme.small, theme.text, left, row_y, width, Align::End);
            }
        }
        AiBlock::Pairs(pairs) => {
            let column = (width - WINDOW_GAP) / 2.0;
            for (at, (label, value)) in pairs.iter().enumerate() {
                let (x, row_y) = (left + (at % 2) as f32 * (column + WINDOW_GAP), y + GAP + (at / 2) as f32 * (LINE + FACT_GAP));
                part(&at.to_string());
                frame.text(label, theme.small, theme.text2, x, row_y, column, Align::Start);
                frame.text(value, theme.small, theme.text, x, row_y, column, Align::End);
            }
        }
        AiBlock::Sessions(rows) => {
            const DOT: f32 = 3.5;
            for (at, (title, about, value, at_work)) in rows.iter().enumerate() {
                let row_y = y + GAP + at as f32 * SESSION;
                part(&at.to_string());
                // At work: a dot in its ink; else a ring.
                let centre = Point { x: left + DOT, y: row_y + LINE / 2.0 };
                if *at_work {
                    frame.fill_circle(ink.trace, centre, DOT);
                } else {
                    frame.arc(centre, DOT - 0.5, 0.0, 360.0, theme.text3, 1.0);
                }
                let words = left + 2.0 * DOT + 8.0;
                let value_width = frame.measure(value, theme.small);
                frame.text(title, theme.body, theme.text, words, row_y, left + width - value_width - 12.0 - words, Align::Start);
                frame.text(about, theme.small, theme.text3, words, row_y + LINE + 1.0, left + width - words, Align::Start);
                frame.text(value, theme.small, theme.text2, left, row_y, width, Align::End);
            }
        }
    }
}

/// How strong the `at`th model's colour is (the most spending, the strongest).
fn strength(at: usize) -> f32 {
    1.0 - at.min(3) as f32 * 0.25
}

/// A bar `fraction` filled in `color`, as the readings' meters are drawn.
fn meter(frame: &dyn Canvas, theme: &Theme, (x, y, width): (f32, f32, f32), fraction: f32, color: Color) {
    let whole = frame.on_pixels(x, y, width, theme.meter_height);
    let (bx, by, bw, bh) = whole;
    frame.fill_rounded(theme.track, bx, by, bw, bh, theme.meter_radius);
    if filled_part(frame, whole, fraction, false).2 > 0.0 {
        frame.clip_part(x, y, width, theme.meter_height, fraction, false);
        frame.fill_rounded(color, bx, by, bw, bh, theme.meter_radius);
        frame.unclip();
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// Tokens are told in the units of the language they are read in.
    #[test]
    fn tells_tokens_in_each_languages_units() {
        assert_eq!(tokens(11_020_000, Lang::Zh), ("1,102".to_string(), "万 tokens".to_string()));
        assert_eq!(tokens(96_000, Lang::Zh), ("9.6".to_string(), "万 tokens".to_string()));
        assert_eq!(tokens(250_000_000, Lang::Zh), ("2.5".to_string(), "亿 tokens".to_string()));
        assert_eq!(tokens(9_999, Lang::Zh), ("9,999".to_string(), "tokens".to_string()));
        assert_eq!(tokens(11_020_000, Lang::En), ("11".to_string(), "M tokens".to_string()));
        assert_eq!(tokens(950, Lang::En), ("950".to_string(), "tokens".to_string()));
        assert_eq!(short(9_030_000, Lang::Zh), "903 万");
        assert_eq!(short(9_030_000, Lang::En), "9.0M");
        assert_eq!(short(42, Lang::En), "42");
    }

    /// Claude's models are named by family and version, a snapshot's date left off; others as they are.
    #[test]
    fn names_models_as_their_makers_do() {
        assert_eq!(model_name("claude-opus-4-6"), "Opus 4.6");
        assert_eq!(model_name("claude-sonnet-4-5-20250929"), "Sonnet 4.5");
        assert_eq!(model_name("claude-3-5-haiku-20241022"), "Haiku 3.5");
        assert_eq!(model_name("claude-opus-4"), "Opus 4");
        assert_eq!(model_name("gpt-5.5-codex"), "gpt-5.5-codex");
    }

    /// The lanes as they are built of what was read (Windows reads it).
    #[cfg(windows)]
    pub(in crate::ui::view) mod lanes {
        use super::*;

        const HOUR: u64 = 3_600_000;
        /// Noon, 2026-11-18 (UTC), and its day's start as these tests take it.
        pub(in crate::ui::view) const NOON: u64 = 1_795_003_200_000;
        const TODAY: u64 = NOON - 12 * HOUR;

        fn session(tool: crate::ai::Tool, id: &str, title: Option<&str>, last: u64, spent: u64) -> crate::ai::Session {
            let mut session: crate::ai::Session = serde_json::from_value(serde_json::json!({
                "tool": tool, "id": id, "title": title, "folder": null, "model": "gpt-5.5-codex", "last": last, "spent": {}
            }))
            .unwrap();
            session.spent.insert(crate::ai::quarter(last), spent);
            session
        }

        fn spent(input: u64, output: u64, cache_read: u64, cache_write: u64, reasoning: u64) -> crate::ai::Tokens {
            crate::ai::Tokens { input, output, cache_read, cache_write, reasoning }
        }

        /// A day: Codex told its limits an hour ago (its five hours to start again in two, its week used past nine
        /// tenths, a window started again since), and spent on two models, in a session at work and one done; Claude
        /// Code spent, with no limit told.
        pub(in crate::ui::view) fn day() -> crate::ai::Usage {
            use crate::ai::{Limit, Tool, Window};
            crate::ai::Usage {
                now: NOON,
                today: TODAY,
                spent_today: vec![
                    (Tool::Claude, "claude-opus-4-6".into(), spent(1_000, 2_000, 90_000, 7_000, 0)),
                    (Tool::Codex, "gpt-5.5-codex".into(), spent(30_000, 9_000, 60_000, 0, 4_000)),
                    (Tool::Codex, "gpt-5.5".into(), spent(1_000, 0, 0, 0, 0)),
                ],
                days: vec![(Tool::Claude, [0, 0, 0, 0, 0, 0, 100_000]), (Tool::Codex, [0, 0, 0, 0, 0, 0, 100_000])],
                quarters: vec![(Tool::Claude, vec![0, 50_000, 50_000]), (Tool::Codex, vec![100_000])],
                sessions: vec![
                    session(Tool::Codex, "a", Some("Review the release"), NOON - 60_000, 70_000),
                    session(Tool::Claude, "c", None, NOON - HOUR, 100_000),
                    session(Tool::Codex, "b", None, NOON - 2 * HOUR, 30_000),
                ],
                limits: vec![Limit {
                    tool: Tool::Codex,
                    id: "codex".into(),
                    name: None,
                    plan: Some("pro".into()),
                    windows: vec![
                        Window { minutes: 300, used: 62.0, resets: Some(NOON + 2 * HOUR) },
                        Window { minutes: 10080, used: 95.0, resets: Some(NOON + 4 * 24 * HOUR) },
                        Window { minutes: 1440, used: 50.0, resets: Some(NOON - HOUR) },
                    ],
                    told: NOON - HOUR,
                }],
            }
        }

        fn lane(id: &str, usage: Option<&crate::ai::Usage>) -> Vec<Block> {
            lane_of(id, &Prefs::default(), Lang::Zh, usage, NOON, false).unwrap()
        }

        /// A source's lane, closed: its head (its name, its plan, when it was last heard of) and each window of its
        /// limit, how much is used, how much of the window has gone by and when it starts again (one started again since
        /// it was told, none of it used); a source with no limit told, what it spent today.
        #[test]
        fn a_source_shows_its_limits_windows() {
            let usage = day();
            let blocks = lane("ai:codex", Some(&usage));
            let [Block::Head { title, device, aside, .. }, Block::Ai(AiBlock::Windows(windows))] = blocks.as_slice() else { panic!("{} blocks", blocks.len()) };
            assert_eq!((title.as_str(), device.as_str()), ("ChatGPT", "Pro"));
            // (Heard of last as its session spent, a minute ago.)
            assert_eq!(aside, &text::since(NOON - 60_000, NOON, Lang::Zh));
            let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
            assert!(windows[0].name == "5 小时" && near(windows[0].used, 0.62) && windows[0].gone.is_some_and(|gone| near(gone, 0.6)), "{:?}", windows[0]);
            assert_eq!((windows[0].resets.as_str(), windows[1].resets.as_str()), ("2 小时后重置", "4 天后重置"));
            assert!(windows[1].used >= HOT && windows[1].name == "每周");
            assert_eq!(windows[2], Window { name: "每天".into(), used: 0.0, gone: None, resets: "已重置".into() });
            let blocks = lane("ai:claude", Some(&usage));
            let [Block::Head { device, .. }, Block::Facts { rows, .. }] = blocks.as_slice() else { panic!("{} blocks", blocks.len()) };
            assert_eq!((device.as_str(), rows[0].1.as_str()), ("Claude Code", "10 万"));
            // (Not read yet: it says so.)
            let blocks = lane("ai:claude", None);
            assert!(matches!(&blocks[1], Block::Facts { rows, .. } if rows[0].1 == "正在读取"));
        }

        /// Opened, a source's lane tells what it spent today by model (the most first, each one's share), by kind
        /// (Codex's reasoning in place of a cache written, which it does not tell), and its sessions of today, the one at
        /// work marked.
        #[test]
        fn an_opened_source_tells_its_models_and_sessions() {
            let usage = day();
            let blocks = lane_of("ai:codex", &Prefs::default(), Lang::Zh, Some(&usage), NOON, true).unwrap();
            let models = blocks.iter().find_map(|block| if let Block::Ai(AiBlock::Models(rows)) = block { Some(rows.clone()) } else { None }).unwrap();
            assert_eq!(models.iter().map(|(name, _, value)| (name.as_str(), value.as_str())).collect::<Vec<_>>(), [("gpt-5.5-codex", "9.9 万"), ("gpt-5.5", "1,000")]);
            assert!((models[0].1 - 0.99).abs() < 1e-3);
            let pairs = blocks.iter().find_map(|block| if let Block::Ai(AiBlock::Pairs(pairs)) = block { Some(pairs.clone()) } else { None }).unwrap();
            assert!(pairs.contains(&("推理".into(), "4,000".into())) && !pairs.iter().any(|(label, _)| label == "缓存写入"));
            assert!(pairs.contains(&("缓存命中".into(), "66%".into())) && pairs.contains(&("会话".into(), "2".into())));
            let sessions = blocks.iter().find_map(|block| if let Block::Ai(AiBlock::Sessions(rows)) = block { Some(rows.clone()) } else { None }).unwrap();
            assert_eq!(sessions.iter().map(|(title, _, value, at_work)| (title.as_str(), value.as_str(), *at_work)).collect::<Vec<_>>(), [("Review the release", "7.0 万", true), ("未命名会话", "3.0 万", false)]);
            // (Closed: its head and its windows alone.)
            assert_eq!(lane("ai:codex", Some(&usage)).len(), 2);
        }

        /// Today's lane: every source's tokens together, how they mounted by each quarter hour so far, and each tool's
        /// share where there are more than one.
        #[test]
        fn today_adds_every_sources_tokens() {
            let usage = day();
            let blocks = lane("ai:today", Some(&usage));
            let [_, Block::Ai(AiBlock::Today { figure, unit, mounted }), Block::Facts { rows, .. }] = blocks.as_slice() else { panic!("{} blocks", blocks.len()) };
            assert_eq!((figure.as_str(), unit.as_str()), ("20", "万 tokens"));
            assert_eq!(mounted, &vec![0.5, 0.75, 1.0]);
            assert_eq!(rows.iter().map(|(name, value, _)| (name.as_str(), value.as_str())).collect::<Vec<_>>(), [("Claude Code", "10 万"), ("Codex", "10 万")]);
        }
    }

    /// What is left of a window is told without a part of nothing.
    #[test]
    fn tells_what_is_left_without_a_part_of_nothing() {
        assert_eq!(left_in(134, Lang::Zh), "2 小时 14 分");
        assert_eq!(left_in(120, Lang::Zh), "2 小时");
        assert_eq!(left_in(4 * 1440 + 360, Lang::Zh), "4 天 6 小时");
        assert_eq!(left_in(4 * 1440 + 59, Lang::Zh), "4 天");
        assert_eq!(left_in(14, Lang::En), "14 min");
        assert_eq!(left_in(61, Lang::En), "1 h 1 min");
    }

    #[test]
    fn names_windows_by_their_length() {
        assert_eq!(window_name(300, Lang::Zh), "5 小时");
        assert_eq!(window_name(10080, Lang::Zh), "每周");
        assert_eq!(window_name(10080, Lang::En), "Weekly");
        assert_eq!(window_name(2880, Lang::En), "2 days");
    }
}
