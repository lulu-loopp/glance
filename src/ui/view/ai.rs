//! The AI component's lanes (see `crate::ai`): what every source spent today, as a figure and how it mounted through
//! the day; then each source's lane, its maker's mark and its plan, each window of its limit as a bar (how much of it
//! is used, how much of the window has gone by, when it starts again), and, opened by its head, what it spent today
//! by model and by kind, and its sessions. Small, the dock (see `paint_dock`): each source as a ring round its mark,
//! down the screen's side or across its top, its lane beside it as the pointer rests on it (see `ui::ai_card`).

use std::sync::atomic::{AtomicU16, Ordering};

use super::*;
use crate::ui::marks::Mark;

/// A source of the AI component: a tool whose records or limits it reads, or an AI provider whose account it reads by
/// the user's key (see `ai::providers`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Source {
    /// Claude Code.
    Claude,
    /// Codex, whose limits are ChatGPT's.
    Codex,
    /// Qwen Code.
    Qwen,
    DeepSeek,
    /// Kimi Code's plan.
    Kimi,
    /// Moonshot AI's open platform.
    Moonshot,
    /// Zhipu's GLM Coding Plan.
    Glm,
    MiniMax,
    /// Volcengine Ark's Coding Plan.
    Volcengine,
    OpenRouter,
}

/// Every source, as the AI component lists them.
const SOURCES: [Source; 10] = [
    Source::Claude,
    Source::Codex,
    Source::Qwen,
    Source::DeepSeek,
    Source::Kimi,
    Source::Moonshot,
    Source::Glm,
    Source::MiniMax,
    Source::Volcengine,
    Source::OpenRouter,
];

impl Source {
    /// The source lane `id` is of.
    pub fn of(id: &str) -> Option<Source> {
        SOURCES.into_iter().find(|source| source.id() == id)
    }

    fn mark(self) -> Mark {
        match self {
            Source::Claude => Mark::Claude,
            Source::Codex => Mark::OpenAi,
            Source::Qwen => Mark::Qwen,
            Source::DeepSeek => Mark::DeepSeek,
            Source::Kimi => Mark::Kimi,
            Source::Moonshot => Mark::Moonshot,
            Source::Glm => Mark::Zai,
            Source::MiniMax => Mark::MiniMax,
            Source::Volcengine => Mark::Volcengine,
            Source::OpenRouter => Mark::OpenRouter,
        }
    }

    /// Its lane's module.
    pub fn id(self) -> &'static str {
        match self {
            Source::Claude => "ai:claude",
            Source::Codex => "ai:codex",
            Source::Qwen => "ai:qwen",
            Source::DeepSeek => "ai:deepseek",
            Source::Kimi => "ai:kimi",
            Source::Moonshot => "ai:moonshot",
            Source::Glm => "ai:glm",
            Source::MiniMax => "ai:minimax",
            Source::Volcengine => "ai:volcengine",
            Source::OpenRouter => "ai:openrouter",
        }
    }

    /// The name it goes by: its maker's (Codex's limits are ChatGPT's plan's), or its product's.
    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            Source::Claude => "Claude",
            Source::Codex => "ChatGPT",
            Source::Qwen => "Qwen",
            Source::DeepSeek => "DeepSeek",
            Source::Kimi => "Kimi",
            Source::Moonshot => "Moonshot",
            Source::Glm => "GLM",
            Source::MiniMax => "MiniMax",
            Source::Volcengine => lang.pick("火山方舟", "Volcengine Ark"),
            Source::OpenRouter => "OpenRouter",
        }
    }

    /// The tool its records are of, or what of the provider's it reads (shown where no plan is told).
    fn tool_name(self) -> &'static str {
        match self {
            Source::Claude => "Claude Code",
            Source::Codex => "Codex",
            Source::Qwen => "Qwen Code",
            Source::Kimi => "Kimi Code",
            Source::Glm | Source::Volcengine => "Coding Plan",
            Source::DeepSeek | Source::Moonshot | Source::MiniMax | Source::OpenRouter => "API",
        }
    }

    /// Whether it is a provider whose account is read by the user's key.
    pub fn keyed(self) -> bool {
        !matches!(self, Source::Claude | Source::Codex | Source::Qwen)
    }

    fn bit(self) -> u16 {
        1 << SOURCES.iter().position(|source| *source == self).unwrap_or(0)
    }
}

/// The sources whose lanes are opened (one bit each: see `Source::bit`), in every window alike.
static OPEN: AtomicU16 = AtomicU16::new(0);

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
    /// A source's head: its maker's mark before its name, its plan after it (else the tool it reads), when it was last
    /// heard of at its end, and whether its lane is opened: none where it does not open (it has nothing more to tell,
    /// or it tells all it has, as beside the dock).
    Head { source: Source, title: String, device: String, aside: String, opened: Option<bool> },
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
            AiBlock::Head { source, title, device, aside, opened } => (source, title, device, aside, opened).hash(state),
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
            AiBlock::Head { .. } => HEAD + HEAD_GAP,
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
        // (A limit that never starts again.)
        0 => lang.pick("额度", "Limit").into(),
        1440 => lang.pick("每天", "Daily").into(),
        10080 => lang.pick("每周", "Weekly").into(),
        43200 => lang.pick("每月", "Monthly").into(),
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
    use crate::ai::Tool;
    match source {
        Source::Claude => Tool::Claude,
        Source::Codex => Tool::Codex,
        Source::Qwen => Tool::Qwen,
        Source::DeepSeek => Tool::DeepSeek,
        Source::Kimi => Tool::Kimi,
        Source::Moonshot => Tool::Moonshot,
        Source::Glm => Tool::Glm,
        Source::MiniMax => Tool::MiniMax,
        Source::Volcengine => Tool::Volcengine,
        Source::OpenRouter => Tool::OpenRouter,
    }
}

/// The provider whose lane module `id` is, where it is one's.
#[cfg(windows)]
pub fn provider_of(id: &str) -> Option<&'static crate::ai::providers::Provider> {
    Source::of(id).filter(|source| source.keyed()).and_then(|source| crate::ai::providers::of(tool_of(source)))
}

/// The lane of the AI source `tool` is, where there is one.
#[cfg(windows)]
pub fn module_of(tool: crate::ai::Tool) -> Option<&'static str> {
    SOURCES.into_iter().find(|source| tool_of(*source) == tool).map(Source::id)
}

/// Money as an account's balance shows it: by its currency's sign (else its code after it), to the cent below a
/// thousand, else whole and grouped.
fn money(amount: f64, currency: &str) -> String {
    let figure = if amount.abs() < 1000.0 { format!("{amount:.2}") } else { grouped(amount.round().max(0.0) as u64) };
    match currency {
        "CNY" => format!("¥{figure}"),
        "USD" => format!("${figure}"),
        other => format!("{figure} {other}"),
    }
}

/// Money in a few characters, as the dock shows it: to a tenth below a hundred, else whole.
fn money_short(amount: f64, currency: &str) -> String {
    let figure = if amount.abs() < 100.0 { format!("{amount:.1}") } else { grouped(amount.round().max(0.0) as u64) };
    match currency {
        "CNY" => format!("¥{figure}"),
        "USD" => format!("${figure}"),
        other => format!("{figure} {other}"),
    }
}

/// Why a provider's account could not be read, in a few words.
fn failed_words(failed: crate::ai::Failed, lang: Lang) -> &'static str {
    match failed {
        crate::ai::Failed::Key => lang.pick("密钥无效", "Key refused"),
        crate::ai::Failed::Limited => lang.pick("请求过于频繁", "Asked too often"),
        crate::ai::Failed::Unavailable => lang.pick("暂时无法读取", "Not reached"),
    }
}

/// Why provider `tool`'s account (its last reading `limit`) is not shown as it stands: its key not set (said only
/// where its records on this PC tell nothing it spent today: `spent`), refused, the provider not reached, or not read
/// yet.
#[cfg(windows)]
fn account_state(tool: crate::ai::Tool, usage: &crate::ai::Usage, limit: Option<&crate::ai::Limit>, spent: bool, lang: Lang) -> Option<Block> {
    let fact = |what: &str, value: &str, hot: bool| Block::Facts { rows: vec![(what.into(), value.into(), hot)], gap: GAP };
    if usage.unkeyed.contains(&tool) {
        return (!spent).then(|| fact(lang.pick("API 密钥", "API key"), lang.pick("未设置", "Not set"), false));
    }
    match usage.failed.iter().find(|(of, _)| *of == tool) {
        Some((_, failed)) => Some(fact(lang.pick("状态", "State"), failed_words(*failed, lang), *failed == crate::ai::Failed::Key)),
        None if limit.is_none() => Some(fact(lang.pick("状态", "State"), lang.pick("正在读取", "Reading"), false)),
        None => None,
    }
}

/// An account's `balance`, and what it spent today (from when that was first read, where that was after the day,
/// `today`, began).
#[cfg(windows)]
fn balance_pairs(balance: &crate::ai::Balance, today: u64, lang: Lang) -> Option<Block> {
    let mut pairs = Vec::new();
    if let Some(amount) = balance.amount {
        pairs.push((lang.pick("余额", "Balance").to_string(), money(amount, &balance.currency)));
    }
    if let Some(spent) = balance.today {
        let what = match balance.since {
            Some(since) => {
                let minutes = since.saturating_sub(today) / 60_000;
                match lang {
                    Lang::Zh => format!("{:02}:{:02} 起消费", minutes / 60, minutes % 60),
                    Lang::En => format!("Spent since {:02}:{:02}", minutes / 60, minutes % 60),
                }
            }
            None => lang.pick("今日消费", "Spent today").into(),
        };
        pairs.push((what, money(spent, &balance.currency)));
    }
    (!pairs.is_empty()).then_some(Block::Ai(AiBlock::Pairs(pairs)))
}

/// The lane of AI module `id` as `prefs` show it: what every source spent today (`ai:today`), or one source's; none
/// for an id of none.
#[cfg(windows)]
pub(super) fn ai_lane(id: &str, prefs: &Prefs, lang: Lang) -> Option<Vec<Block>> {
    lane_of(id, prefs, lang, crate::ai::usage().as_deref(), crate::station::now_ms(), Some(Source::of(id).is_some_and(opened)))
}

/// `source`'s lane as the card beside the dock shows it: all it tells, its head opening nothing.
#[cfg(windows)]
pub fn card(scene: &Scene, source: Source) -> Option<Lane> {
    let blocks = lane_of(source.id(), scene.prefs, scene.lang, crate::ai::usage().as_deref(), crate::station::now_ms(), None)?;
    Some(Lane { id: source.id().into(), blocks, ink: scene.theme.ink(source.id()) })
}

/// The limit `tool` last told: its own (Codex's "codex"), else the first told.
#[cfg(windows)]
fn limit_of(usage: &crate::ai::Usage, tool: crate::ai::Tool) -> Option<&crate::ai::Limit> {
    let limits: Vec<&crate::ai::Limit> = usage.limits.iter().filter(|limit| limit.tool == tool).collect();
    limits.iter().find(|limit| limit.id == "codex").or_else(|| limits.first()).copied()
}

/// The windows of `limit` as they stand at `now`: one started again since it was told, none of it used.
#[cfg(windows)]
fn windows_of(limit: Option<&crate::ai::Limit>, now: u64, lang: Lang) -> Vec<Window> {
    limit.map_or(Vec::new(), |limit| {
        limit
            .windows
            .iter()
            .map(|window| {
                let length = window.minutes as u64 * 60_000;
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
    })
}

/// What `tool` spent today, by model, the most first, and all of it together.
#[cfg(windows)]
fn spent_by(usage: &crate::ai::Usage, tool: crate::ai::Tool) -> (Vec<&(crate::ai::Tool, String, crate::ai::Tokens)>, crate::ai::Tokens) {
    let spent: Vec<&(crate::ai::Tool, String, crate::ai::Tokens)> = usage.spent_today.iter().filter(|(of, _, _)| *of == tool).collect();
    let mut total = crate::ai::Tokens::default();
    for (_, _, tokens) in &spent {
        total.add(*tokens);
    }
    (spent, total)
}

/// As `ai_lane`, of `usage` (none: not read yet) at `now` (ms since 1970): a source's lane `open` or not as it is
/// toggled, or (none) showing all it tells, opening nothing.
#[cfg(windows)]
pub(super) fn lane_of(id: &str, prefs: &Prefs, lang: Lang, usage: Option<&crate::ai::Usage>, now: u64, open: Option<bool>) -> Option<Vec<Block>> {
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
            let named = |tool| SOURCES.into_iter().find(|source| tool_of(*source) == tool).map_or("", Source::tool_name);
            blocks.push(Block::Facts { rows: tools.into_iter().map(|(tool, spent)| (named(tool).into(), short(spent, lang), false)).collect(), gap: GAP });
        }
        return Some(blocks);
    }
    let source = Source::of(id)?;
    let tool = tool_of(source);
    let head = |device: String, aside: String, opened: Option<bool>| Block::Ai(AiBlock::Head { source, title: source.name(lang).into(), device, aside, opened });
    let Some(usage) = usage else { return Some(vec![head(source.tool_name().into(), String::new(), None), reading()]) };
    let limit = limit_of(usage, tool);
    let sessions: Vec<&crate::ai::Session> = usage.sessions.iter().filter(|session| session.tool == tool && session.spent_between(usage.today, usage.now) > 0).collect();
    // When it was last heard of: its limit told, else its last session's spending.
    let heard = limit.map(|limit| limit.told).into_iter().chain(sessions.first().map(|session| session.last)).max();
    let aside = heard.map_or(String::new(), |heard| text::since(heard, now, lang));
    let device = limit.and_then(|limit| limit.plan.as_deref()).map_or(source.tool_name().to_string(), plan_name);
    let windows = windows_of(limit, now, lang);
    let (spent, total) = spent_by(usage, tool);
    // (It opens where it spent today: there is more to tell.)
    let mut blocks = vec![head(device, aside, open.filter(|_| total.total() > 0))];
    // A provider's account: why it is not shown as it stands, where it is not.
    if source.keyed() {
        blocks.extend(account_state(tool, usage, limit, total.total() > 0, lang));
    }
    let shown_limit = !windows.is_empty();
    if shown_limit {
        blocks.push(Block::Ai(AiBlock::Windows(windows)));
    }
    if let Some(balance) = limit.and_then(|limit| limit.balance.as_ref()).and_then(|balance| balance_pairs(balance, usage.today, lang)) {
        blocks.push(balance);
    }
    // Opened, or showing all (where it spent today): what it spent, by model and by kind, and its sessions; else, with
    // no limit to show, what it spent today (a provider's, where its records on this PC tell it spent anything).
    if !open.unwrap_or(true) || total.total() == 0 {
        let today = if source.keyed() { total.total() > 0 && !shown_limit } else { limit.is_none() };
        if today {
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

/// Draws `block` in its lane's ink, from `y` across `width`; where a press opens or closes it, there.
pub(super) fn paint(frame: &dyn Canvas, scene: &Scene, ink: Ink, block: &AiBlock, (left, y, width): (f32, f32, f32), part: &dyn Fn(&str), hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    match block {
        AiBlock::Head { source, title, device, aside, opened } => {
            const MARK: f32 = 16.0;
            let row = y + (HEAD - theme.title.size) / 2.0 - 1.0;
            part("mark");
            frame.mark(source.mark(), Point { x: left + MARK / 2.0, y: y + HEAD / 2.0 }, MARK, theme.text, 1.0);
            let title_left = left + MARK + 7.0;
            let title_width = frame.measure(title, theme.title);
            part("label");
            frame.text(title, theme.title, theme.text, title_left, row, width - (title_left - left), Align::Start);
            let chevron = if opened.is_some() { 16.0 + 4.0 } else { 0.0 };
            let aside_width = if aside.is_empty() { 0.0 } else { frame.measure(aside, theme.small) };
            let device_left = title_left + title_width + 8.0;
            part("device");
            frame.text(device, theme.small, theme.text3, device_left, row + 1.0, (left + width - chevron - aside_width - 8.0 - device_left).max(0.0), Align::Start);
            part("detail");
            frame.text(aside, theme.small, theme.text3, left, row + 1.0, width - chevron, Align::End);
            // The whole head opens or closes the lane, where it opens.
            if let Some(opened) = opened {
                let hit = Hit::Ai(*source);
                let lit = scene.hover == Some(hit);
                let icon = if *opened { Icon::ChevronUp } else { Icon::ChevronDown };
                part("chevron");
                frame.icon(icon, Point { x: left + width - 8.0, y: y + HEAD / 2.0 }, 14.0, if lit { theme.text } else { theme.text3 });
                hits.push((left, y, width, HEAD, hit));
            }
        }
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

/// A source as the dock shows it: how much of its tightest window is used (none, with no limit told: its ring whole and
/// faint), and its figure under its ring (that share, else what it spent today) and what the figure is of (that window,
/// where it has several; else today).
#[derive(Clone, PartialEq, Debug)]
pub struct Slot {
    pub source: Source,
    pub used: Option<f32>,
    pub figure: String,
    pub label: String,
}

/// What the dock shows: what every source spent today, each source docked, those with a session at work, and how many
/// sessions are at work.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Dock {
    pub today: String,
    pub slots: Vec<Slot>,
    pub at_work: Vec<Source>,
    pub working: usize,
}

/// The dock down a side: how wide it is, the room at its top and foot, today's figure, a source's slot (its ring, its
/// figure and what that is of) and the sessions at work (DIPs).
const DOCK_WIDTH: f32 = 84.0;
const DOCK_PAD: f32 = 12.0;
const DOCK_TODAY: f32 = 42.0;
const DOCK_SLOT: f32 = 92.0;
const DOCK_FOOT: f32 = 50.0;
/// Across a top edge: how tall it is, the room of today's figure and of the sessions at its ends, and a source's.
const BAR_HEIGHT: f32 = 60.0;
const BAR_END: f32 = 76.0;
const BAR_SLOT: f32 = 120.0;

/// The sources the dock shows, as `prefs` show the AI component: those switched on, in their order.
pub fn docked(prefs: &Prefs) -> Vec<Source> {
    prefs.modules.iter().filter(|entry| shows_module(entry)).filter_map(|entry| Source::of(&entry.id)).collect()
}

/// The dock's size as designed for `count` sources: down a side, or `across` a top edge (DIPs).
pub fn dock_size(across: bool, count: usize) -> (f32, f32) {
    let count = count as f32;
    if across {
        (2.0 * DOCK_PAD + 2.0 * BAR_END + count * BAR_SLOT, BAR_HEIGHT)
    } else {
        (DOCK_WIDTH, 2.0 * DOCK_PAD + DOCK_TODAY + count * DOCK_SLOT + DOCK_FOOT)
    }
}

/// What the dock shows of `usage` (none: not read yet) at `now`, as `prefs` show the component.
#[cfg(windows)]
pub(super) fn dock_of(prefs: &Prefs, lang: Lang, usage: Option<&crate::ai::Usage>, now: u64) -> Dock {
    let sources = docked(prefs);
    let Some(usage) = usage else {
        let slots = sources.into_iter().map(|source| Slot { source, used: None, figure: "—".into(), label: lang.pick("正在读取", "Reading").into() }).collect();
        return Dock { today: "—".into(), slots, at_work: Vec::new(), working: 0 };
    };
    let today = short(usage.spent_today.iter().map(|(_, _, tokens)| tokens.total()).sum(), lang);
    let slots = sources
        .iter()
        .map(|&source| {
            let tool = tool_of(source);
            let limit = limit_of(usage, tool);
            let windows = windows_of(limit, now, lang);
            let failed = usage.failed.iter().find(|(of, _)| *of == tool).map(|(_, failed)| *failed);
            let balance = limit.and_then(|limit| limit.balance.as_ref()).and_then(|balance| Some((balance.amount?, balance.currency.as_str())));
            let none = |label: &str| Slot { source, used: None, figure: "—".into(), label: label.into() };
            let spent = spent_by(usage, tool).1.total();
            // (A provider whose records on this PC tell it spent today shows that where its key is not set.)
            if source.keyed() && usage.unkeyed.contains(&tool) && spent == 0 {
                return none(lang.pick("未设置密钥", "No key"));
            }
            // (What was read before is not shown as it stands now.)
            if let Some(failed) = failed.filter(|_| source.keyed()) {
                return none(failed_words(failed, lang));
            }
            match windows.iter().max_by(|a, b| a.used.total_cmp(&b.used)) {
                Some(tightest) => Slot {
                    source,
                    used: Some(tightest.used),
                    figure: format!("{:.0}%", tightest.used * 100.0),
                    label: if windows.len() > 1 { tightest.name.clone() } else { String::new() },
                },
                None => match balance {
                    Some((amount, currency)) => Slot { source, used: None, figure: money_short(amount, currency), label: lang.pick("余额", "Balance").into() },
                    None if source.keyed() && spent == 0 && !usage.unkeyed.contains(&tool) => none(lang.pick("正在读取", "Reading")),
                    None => Slot { source, used: None, figure: short(spent, lang), label: lang.pick("今日", "Today").into() },
                },
            }
        })
        .collect();
    let working: Vec<&crate::ai::Session> = usage.sessions.iter().filter(|session| now.saturating_sub(session.last) < AT_WORK_MS && sources.iter().any(|&source| tool_of(source) == session.tool)).collect();
    let at_work = sources.into_iter().filter(|&source| working.iter().any(|session| session.tool == tool_of(source))).collect();
    Dock { today, slots, at_work, working: working.len() }
}

/// Draws the dock filling `size` (down a side, or `across` a top edge), its corner at the frame's origin: what every
/// source spent today at its start, each source's slot, the sessions at work at its end; where each source is, to
/// show its lane beside it as the pointer rests there.
pub fn paint_dock(frame: &dyn Canvas, scene: &Scene, across: bool, size: (f32, f32)) -> Vec<HitBox> {
    #[cfg(windows)]
    let dock = dock_of(scene.prefs, scene.lang, crate::ai::usage().as_deref(), crate::station::now_ms());
    #[cfg(not(windows))]
    let dock = Dock { slots: docked(scene.prefs).into_iter().map(|source| Slot { source, used: None, figure: String::new(), label: String::new() }).collect(), ..Dock::default() };
    draw_dock(frame, scene, &dock, across, size)
}

/// `text` in `font`, its middle at `centre` across, from `top`.
fn centred(frame: &dyn Canvas, text: &str, font: Font, color: Color, centre: f32, top: f32) {
    let width = frame.measure(text, font);
    frame.text(text, font, color, centre - width / 2.0, top, width + 2.0, Align::Start);
}

/// A source's ring, `radius` round `centre` and `width` wide, in its colour (the signal's from nine tenths used): as
/// much of it as is used, else (no limit told) whole and faint; its maker's mark in it.
fn ring(frame: &dyn Canvas, theme: &Theme, slot: &Slot, centre: Point, radius: f32, width: f32, mark: f32) {
    let hot = slot.used.is_some_and(|used| used >= HOT);
    let color = if hot { theme.signal } else { theme.ink(slot.source.id()).trace };
    frame.key(&format!("{}.ring", slot.source.id()));
    frame.arc(centre, radius, 0.0, std::f32::consts::TAU, theme.track, width);
    match slot.used {
        Some(used) if used > 0.0 => frame.arc(centre, radius, 0.0, std::f32::consts::TAU * used.min(1.0), color, width),
        Some(_) => {}
        None => frame.arc(centre, radius, 0.0, std::f32::consts::TAU, color.alpha(color.a * 0.55), width),
    }
    frame.key(&format!("{}.mark", slot.source.id()));
    frame.mark(slot.source.mark(), centre, mark, theme.text, 1.0);
}

pub(super) fn draw_dock(frame: &dyn Canvas, scene: &Scene, dock: &Dock, across: bool, (w, h): (f32, f32)) -> Vec<HitBox> {
    let (theme, lang) = (scene.theme, scene.lang);
    let mut hits = Vec::new();
    let label = Font { size: 11.0, weight: 500.0, ..theme.body };
    let big = Font { size: 14.0, weight: 650.0, ..theme.figure };
    let figure = Font { size: 13.0, weight: 650.0, ..theme.figure };
    let under = Font { size: 10.5, weight: 500.0, ..theme.body };
    let count = dock.slots.len();
    let natural = dock_size(across, count);
    // (Drawn larger than designed: the room left over shared out round its parts.)
    let spread = if across { (w - natural.0) / (count as f32 + 2.0) } else { (h - natural.1) / (count as f32 + 2.0) }.max(0.0);
    let figure_of = |slot: &Slot| if slot.used.is_some_and(|used| used >= HOT) { theme.signal } else { theme.text };
    let lit = |source: Source| scene.hover == Some(Hit::AiSlot(source));
    let sessions = (lang.pick("会话", "Sessions"), dock.working.to_string());
    // The sessions at work: their sources' marks in a row, its middle at `centre`.
    let marks = |centre: f32, y: f32| {
        const MARK: f32 = 12.0;
        let row = dock.at_work.len() as f32 * (MARK + 3.0) - 3.0;
        for (at, source) in dock.at_work.iter().enumerate() {
            frame.key(&format!("ai:dock.at_work.{at}"));
            frame.mark(source.mark(), Point { x: centre - row / 2.0 + at as f32 * (MARK + 3.0) + MARK / 2.0, y }, MARK, theme.text2, 1.0);
        }
    };
    if across {
        let middle = h / 2.0;
        let mut x = DOCK_PAD + spread / 2.0;
        frame.key("ai:today.label");
        centred(frame, lang.pick("今日", "Today"), label, theme.text3, x + BAR_END / 2.0, middle - 17.0);
        frame.key("ai:today.value");
        centred(frame, &dock.today, big, theme.text, x + BAR_END / 2.0, middle - 2.0);
        x += BAR_END + spread;
        for slot in &dock.slots {
            let hit = Hit::AiSlot(slot.source);
            if lit(slot.source) {
                frame.fill_rounded(theme.hover, x, 5.0, BAR_SLOT, h - 10.0, (h - 10.0) / 2.0);
            }
            hits.push((x, 0.0, BAR_SLOT, h, hit));
            ring(frame, theme, slot, Point { x: x + 8.0 + 17.0, y: middle }, 16.0, 2.6, 15.0);
            let words = x + 8.0 + 34.0 + 9.0;
            let (top, room) = (if slot.label.is_empty() { middle - 9.0 } else { middle - 17.0 }, x + BAR_SLOT - 8.0 - words);
            frame.key(&format!("{}.value", slot.source.id()));
            frame.text(&slot.figure, figure, figure_of(slot), words, top, room, Align::Start);
            frame.key(&format!("{}.label", slot.source.id()));
            frame.text(&slot.label, under, theme.text3, words, middle + 1.0, room, Align::Start);
            x += BAR_SLOT + spread;
        }
        frame.key("ai:dock.sessions");
        centred(frame, sessions.0, label, theme.text3, x + BAR_END / 2.0, middle - 17.0);
        let count_width = frame.measure(&sessions.1, big);
        let row = if dock.at_work.is_empty() { 0.0 } else { dock.at_work.len() as f32 * 15.0 + 2.0 };
        let start = x + BAR_END / 2.0 - (count_width + row) / 2.0;
        frame.key("ai:dock.at_work");
        frame.text(&sessions.1, big, theme.text, start, middle - 2.0, count_width + 2.0, Align::Start);
        marks(start + count_width + 4.0 + (row - 4.0) / 2.0, middle + 7.0);
        return hits;
    }
    let centre = w / 2.0;
    let mut y = DOCK_PAD + spread / 2.0;
    frame.key("ai:today.label");
    centred(frame, lang.pick("今日", "Today"), label, theme.text3, centre, y + 2.0);
    frame.key("ai:today.value");
    centred(frame, &dock.today, big, theme.text, centre, y + 18.0);
    y += DOCK_TODAY + spread;
    for slot in &dock.slots {
        let hit = Hit::AiSlot(slot.source);
        if lit(slot.source) {
            frame.fill_rounded(theme.hover, 5.0, y, w - 10.0, DOCK_SLOT - 4.0, 16.0_f32.min((w - 10.0) / 2.0));
        }
        hits.push((0.0, y, w, DOCK_SLOT, hit));
        ring(frame, theme, slot, Point { x: centre, y: y + 8.0 + 22.0 }, 21.5, 3.0, 20.0);
        frame.key(&format!("{}.value", slot.source.id()));
        centred(frame, &slot.figure, figure, figure_of(slot), centre, y + 56.0);
        frame.key(&format!("{}.label", slot.source.id()));
        centred(frame, &slot.label, under, theme.text3, centre, y + 74.0);
        y += DOCK_SLOT + spread;
    }
    frame.key("ai:dock.sessions");
    centred(frame, sessions.0, label, theme.text3, centre, y + 4.0);
    frame.key("ai:dock.at_work");
    centred(frame, &sessions.1, big, theme.text, centre, y + 19.0);
    marks(centre, y + 44.0);
    hits
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
                failed: Vec::new(),
                unkeyed: Vec::new(),
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
                    balance: None,
                    told: NOON - HOUR,
                }],
            }
        }

        fn lane(id: &str, usage: Option<&crate::ai::Usage>) -> Vec<Block> {
            lane_of(id, &Prefs::default(), Lang::Zh, usage, NOON, Some(false)).unwrap()
        }

        /// A source's lane, closed: its head (its name, its plan, when it was last heard of) and each window of its
        /// limit, how much is used, how much of the window has gone by and when it starts again (one started again since
        /// it was told, none of it used); a source with no limit told, what it spent today.
        #[test]
        fn a_source_shows_its_limits_windows() {
            let usage = day();
            let blocks = lane("ai:codex", Some(&usage));
            let [Block::Ai(AiBlock::Head { title, device, aside, opened, .. }), Block::Ai(AiBlock::Windows(windows))] = blocks.as_slice() else { panic!("{} blocks", blocks.len()) };
            assert_eq!((title.as_str(), device.as_str(), *opened), ("ChatGPT", "Pro", Some(false)));
            // (Heard of last as its session spent, a minute ago.)
            assert_eq!(aside, &text::since(NOON - 60_000, NOON, Lang::Zh));
            let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
            assert!(windows[0].name == "5 小时" && near(windows[0].used, 0.62) && windows[0].gone.is_some_and(|gone| near(gone, 0.6)), "{:?}", windows[0]);
            assert_eq!((windows[0].resets.as_str(), windows[1].resets.as_str()), ("2 小时后重置", "4 天后重置"));
            assert!(windows[1].used >= HOT && windows[1].name == "每周");
            assert_eq!(windows[2], Window { name: "每天".into(), used: 0.0, gone: None, resets: "已重置".into() });
            let blocks = lane("ai:claude", Some(&usage));
            let [Block::Ai(AiBlock::Head { device, .. }), Block::Facts { rows, .. }] = blocks.as_slice() else { panic!("{} blocks", blocks.len()) };
            assert_eq!((device.as_str(), rows[0].1.as_str()), ("Claude Code", "10 万"));
            // (Not read yet: it says so, and opens nothing.)
            let blocks = lane("ai:claude", None);
            assert!(matches!(&blocks[0], Block::Ai(AiBlock::Head { opened: None, .. })));
            assert!(matches!(&blocks[1], Block::Facts { rows, .. } if rows[0].1 == "正在读取"));
        }

        /// Opened, a source's lane tells what it spent today by model (the most first, each one's share), by kind
        /// (Codex's reasoning in place of a cache written, which it does not tell), and its sessions of today, the one at
        /// work marked.
        #[test]
        fn an_opened_source_tells_its_models_and_sessions() {
            let usage = day();
            let blocks = lane_of("ai:codex", &Prefs::default(), Lang::Zh, Some(&usage), NOON, Some(true)).unwrap();
            assert!(matches!(&blocks[0], Block::Ai(AiBlock::Head { opened: Some(true), .. })));
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
            // Beside the dock: all of it, its head opening nothing.
            let whole = lane_of("ai:codex", &Prefs::default(), Lang::Zh, Some(&usage), NOON, None).unwrap();
            assert!(matches!(&whole[0], Block::Ai(AiBlock::Head { opened: None, .. })));
            let drawn = |blocks: &[Block]| {
                let mut hasher = std::hash::DefaultHasher::new();
                blocks.hash(&mut hasher);
                hasher.finish()
            };
            assert_eq!(drawn(&whole[1..]), drawn(&blocks[1..]));
        }

        /// Nothing spent today, a source's lane opens nothing, as it is toggled or beside the dock.
        #[test]
        fn a_source_that_spent_nothing_opens_nothing() {
            let mut usage = day();
            usage.spent_today.retain(|(tool, _, _)| *tool != crate::ai::Tool::Codex);
            for open in [Some(true), Some(false), None] {
                let blocks = lane_of("ai:codex", &Prefs::default(), Lang::Zh, Some(&usage), NOON, open).unwrap();
                assert!(matches!(&blocks[0], Block::Ai(AiBlock::Head { opened: None, .. })), "{open:?}");
                assert_eq!(blocks.len(), 2, "{open:?}");
            }
        }

        /// The dock: every source's tokens together; each source by its tightest window (how much of it is used, and
        /// which, where it has several: a window started again since counts as unused), or with no limit told by what
        /// it spent today; the sources with a session at work, and how many sessions are.
        #[test]
        fn the_dock_shows_each_source_by_its_tightest_window() {
            let usage = day();
            let dock = dock_of(&Prefs::resolve(&serde_json::Value::Null, &[]).holding(crate::settings::Part::Ai), Lang::Zh, Some(&usage), NOON);
            assert_eq!(dock.today, "20 万");
            let slots: Vec<(Source, Option<f32>, &str, &str)> = dock.slots.iter().map(|slot| (slot.source, slot.used, slot.figure.as_str(), slot.label.as_str())).collect();
            assert_eq!(slots, [(Source::Claude, None, "10 万", "今日"), (Source::Codex, Some(0.95), "95%", "每周")]);
            assert_eq!((dock.at_work.as_slice(), dock.working), ([Source::Codex].as_slice(), 1));
            // Two sessions of one source at work: two sessions, one source's mark.
            let mut usage = usage;
            let mut second = usage.sessions.iter().find(|session| NOON.saturating_sub(session.last) < AT_WORK_MS).unwrap().clone();
            second.id = "second".into();
            usage.sessions.push(second);
            let dock = dock_of(&Prefs::resolve(&serde_json::Value::Null, &[]).holding(crate::settings::Part::Ai), Lang::Zh, Some(&usage), NOON);
            assert_eq!((dock.at_work.as_slice(), dock.working), ([Source::Codex].as_slice(), 2));
            // (Not read yet: each source says so.)
            let dock = dock_of(&Prefs::resolve(&serde_json::Value::Null, &[]).holding(crate::settings::Part::Ai), Lang::Zh, None, NOON);
            assert!(dock.slots.len() == 2 && dock.slots.iter().all(|slot| slot.used.is_none() && slot.label == "正在读取"));
        }

        /// A provider's account as it was read: DeepSeek's balance and what it spent today (from 09:30, its first
        /// reading of the day), GLM's plan's windows (a monthly one among them), Kimi's key refused, MiniMax's key not set.
        pub(in crate::ui::view) fn accounts() -> crate::ai::Usage {
            use crate::ai::{Balance, Failed, Limit, Tool, Window};
            let mut usage = day();
            let limit = |tool, plan: Option<&str>, windows, balance| Limit { tool, id: String::new(), name: None, plan: plan.map(String::from), windows, balance, told: NOON - 60_000 };
            usage.limits.push(limit(
                Tool::DeepSeek,
                None,
                Vec::new(),
                Some(Balance { currency: "CNY".into(), amount: Some(1_234.5), today: Some(3.2), since: Some(TODAY + 9 * HOUR + 30 * 60_000), spent: None }),
            ));
            usage.limits.push(limit(
                Tool::Glm,
                Some("pro"),
                vec![Window { minutes: 300, used: 20.0, resets: Some(NOON + HOUR) }, Window { minutes: 43200, used: 91.0, resets: Some(NOON + 9 * 24 * HOUR) }],
                None,
            ));
            usage.limits.push(limit(Tool::Kimi, Some("Andante"), vec![Window { minutes: 10080, used: 40.0, resets: None }], None));
            usage.failed.push((Tool::Kimi, Failed::Key));
            usage.unkeyed.push(Tool::MiniMax);
            // (Qwen Code spent too, in a session of its own.)
            usage.spent_today.push((Tool::Qwen, "qwen3-coder-plus".into(), spent(20_000, 3_000, 40_000, 0, 1_000)));
            usage.sessions.push(session(Tool::Qwen, "q", Some("Add the lunar date"), NOON - 30 * 60_000, 63_000));
            usage
        }

        /// A provider's lane: its plan (else what of the provider it reads) and when it was read; its balance and what
        /// it spent today, from when that was first read where it was not read since the day began; its plan's windows;
        /// why it could not be read, or that its key is not set.
        #[test]
        fn a_provider_shows_its_account() {
            let usage = accounts();
            let blocks = lane("ai:deepseek", Some(&usage));
            let [Block::Ai(AiBlock::Head { title, device, aside, opened, .. }), Block::Ai(AiBlock::Pairs(pairs))] = blocks.as_slice() else { panic!("{} blocks", blocks.len()) };
            assert_eq!((title.as_str(), device.as_str(), *opened), ("DeepSeek", "API", None));
            assert_eq!(aside, &text::since(NOON - 60_000, NOON, Lang::Zh));
            assert_eq!(pairs, &vec![("余额".to_string(), "¥1,235".to_string()), ("09:30 起消费".to_string(), "¥3.20".to_string())]);
            let blocks = lane("ai:glm", Some(&usage));
            let [Block::Ai(AiBlock::Head { device, .. }), Block::Ai(AiBlock::Windows(windows))] = blocks.as_slice() else { panic!("{} blocks", blocks.len()) };
            assert_eq!((device.as_str(), windows[1].name.as_str()), ("Pro", "每月"));
            assert!(windows[1].used >= HOT);
            // (Refused: said so, what was read before under it.)
            let blocks = lane("ai:kimi", Some(&usage));
            assert!(matches!(&blocks[1], Block::Facts { rows, .. } if rows[0] == ("状态".into(), "密钥无效".into(), true)));
            let blocks = lane("ai:minimax", Some(&usage));
            assert!(matches!(blocks.as_slice(), [_, Block::Facts { rows, .. }] if rows[0].1 == "未设置"));
            // (Not read yet.)
            let blocks = lane("ai:openrouter", Some(&usage));
            assert!(matches!(blocks.as_slice(), [Block::Ai(AiBlock::Head { device, .. }), Block::Facts { rows, .. }] if device == "API" && rows[0].1 == "正在读取"));
            assert_eq!(money(12.0, "USD"), "$12.00");
            assert_eq!(money_short(1_234.5, "CNY"), "¥1,235");
            // Kimi Code spent today on this PC, its key not set: what it spent, opening as a recorded source's does, its
            // key not asked for, and so in the dock.
            let mut usage = accounts();
            usage.limits.retain(|limit| limit.tool != crate::ai::Tool::Kimi);
            usage.failed.clear();
            usage.unkeyed.push(crate::ai::Tool::Kimi);
            usage.spent_today.push((crate::ai::Tool::Kimi, "kimi-for-coding".into(), spent(1_000, 200, 30_000, 0, 0)));
            let blocks = lane("ai:kimi", Some(&usage));
            assert!(matches!(blocks.as_slice(), [Block::Ai(AiBlock::Head { opened: Some(false), device, .. }), Block::Facts { rows, .. }] if device == "Kimi Code" && rows[0].1 == "3.1 万"));
            let stored = serde_json::json!({ "ai": [{ "id": "ai:kimi", "on": true }] });
            let dock = dock_of(&Prefs::resolve(&stored, &[]).holding(crate::settings::Part::Ai), Lang::Zh, Some(&usage), NOON);
            assert!(dock.slots.iter().any(|slot| slot.source == Source::Kimi && slot.figure == "3.1 万" && slot.label == "今日"));
        }

        /// The dock: a provider by its tightest window, else by its balance; one whose key was refused or is not set,
        /// by why, nothing read before shown as it stands now.
        #[test]
        fn the_dock_shows_a_provider_by_its_window_or_balance() {
            let usage = accounts();
            let on = ["ai:today", "ai:deepseek", "ai:glm", "ai:kimi", "ai:minimax"].map(|id| serde_json::json!({ "id": id, "on": true }));
            let stored = serde_json::json!({ "ai": on });
            let prefs = Prefs::resolve(&stored, &[]).holding(crate::settings::Part::Ai);
            let dock = dock_of(&prefs, Lang::Zh, Some(&usage), NOON);
            let slots: Vec<(Source, Option<f32>, &str, &str)> = dock.slots.iter().map(|slot| (slot.source, slot.used, slot.figure.as_str(), slot.label.as_str())).collect();
            assert_eq!(
                slots,
                [
                    (Source::Claude, None, "10 万", "今日"),
                    (Source::Codex, Some(0.95), "95%", "每周"),
                    (Source::DeepSeek, None, "¥1,235", "余额"),
                    (Source::Glm, Some(0.91), "91%", "每月"),
                    (Source::Kimi, None, "—", "密钥无效"),
                    (Source::MiniMax, None, "—", "未设置密钥"),
                ]
            );
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
        assert_eq!(window_name(43200, Lang::Zh), "每月");
        assert_eq!(window_name(0, Lang::En), "Limit");
    }

    /// A provider's lane is off till it is turned on (as its key is given); Claude's and ChatGPT's are on.
    #[test]
    fn a_providers_lane_is_off_till_turned_on() {
        let prefs = Prefs::resolve(&serde_json::Value::Null, &[]);
        let on: Vec<&str> = prefs.ai.iter().filter(|entry| entry.on).map(|entry| entry.id.as_str()).collect();
        assert_eq!(on, ["ai:today", "ai:claude", "ai:codex"]);
        assert_eq!(prefs.ai.len(), crate::ui::prefs::AI.len());
        assert!(!crate::ui::prefs::module_on_in(&serde_json::Value::Null, "ai", "ai:deepseek"));
        assert!(crate::ui::prefs::module_on_in(&serde_json::Value::Null, "ai", "ai:claude"));
        // (Each source its own bit, its own id.)
        for (at, source) in SOURCES.iter().enumerate() {
            assert_eq!((source.bit(), Source::of(source.id())), (1 << at, Some(*source)));
        }
    }
}
