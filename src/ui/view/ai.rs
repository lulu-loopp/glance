//! The AI component (see `crate::ai`), one lane on one plate: what every source spent today, as a figure and how it
//! mounted through the day; then each source, its maker's mark, its plan and what it spent today, each window of its
//! limit as a bar (how much of it is used, how much of the window has gone by, when it starts again), and, opened by
//! its head, what it spent today by model and by kind, and its sessions; and the sessions at work. Small, the dock
//! (see `paint_dock`): each source as a ring round its mark, down the screen's side or across its top, its lane beside
//! it as the pointer rests on it (see `ui::hover_card`). A widget too small for all of it shows the dock's strip across or
//! down, else as near a square as it is, today's figure over each source's ring (see `paint_shaped`).

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
pub(super) const SOURCES: [Source; 10] = [
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

/// What of the dock the pointer rests on, the card beside the dock showing it: what every source spent today, a
/// source's ring, or the sessions at work.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Spot {
    Today,
    Ring(Source),
    Sessions,
}

impl Spot {
    /// The id of the lane its card shows (its ink's).
    pub fn id(self) -> &'static str {
        match self {
            Spot::Today => "ai:today",
            Spot::Ring(source) => source.id(),
            Spot::Sessions => "ai:sessions",
        }
    }
}

/// The sources whose lanes are folded (one bit each: see `Source::bit`), in every window alike: as the settings keep
/// them (see `Settings::ai_folded`), the others opened.
static FOLDED: AtomicU16 = AtomicU16::new(0);

/// The sources folded as the settings keep them (`folded`, their lanes' ids).
pub fn follow(folded: &[String]) {
    let bits = SOURCES.into_iter().filter(|source| folded.iter().any(|id| id == source.id())).fold(0, |bits, source| bits | source.bit());
    FOLDED.store(bits, Ordering::Relaxed);
}

/// `source`'s lane folded, or opened again: shown so at once, and kept in the settings (off this thread, which they tell
/// to restyle). Each such change kept as what it changes, whatever order they come into the settings in.
#[cfg(windows)]
pub fn toggle(source: Source) {
    FOLDED.fetch_xor(source.bit(), Ordering::Relaxed);
    std::thread::spawn(move || crate::app().change(|settings| folded_again(&mut settings.ai_folded, source)));
}

/// `source` folded where the settings keep it opened (`folded`, the lanes folded), opened where they keep it folded.
fn folded_again(folded: &mut Vec<String>, source: Source) {
    match folded.iter().position(|id| id == source.id()) {
        Some(at) => {
            folded.remove(at);
        }
        None => folded.push(source.id().to_string()),
    }
}

fn opened(source: Source) -> bool {
    FOLDED.load(Ordering::Relaxed) & source.bit() == 0
}

/// What an AI lane draws, under its head.
#[derive(Clone, PartialEq, Debug)]
pub enum AiBlock {
    /// A source's head: its maker's mark before its name, its plan after it (else the tool it reads), when it was last
    /// heard of at its end, and whether its lane is opened: none where it does not open (it has nothing more to tell,
    /// or it tells all it has, as beside the dock).
    Head { source: Source, title: String, device: String, aside: String, opened: Option<bool>, flag: Option<Flag> },
    /// Today's tokens, as a figure and its unit, and how they mounted through the day: what was spent by the end of
    /// each quarter hour so far, from 0 to 1 of the whole.
    Today { figure: String, unit: String, mounted: Vec<f32> },
    /// The windows of a limit, two side by side: each one's name, how much of it is used (0 to 1), how much of the
    /// window has gone by (where it is told when it starts again), and when it does.
    Windows(Vec<Window>),
    /// What each model spent today, the most first: its name, its share, and how much.
    Models(Vec<(String, f32, String)>),
    /// What each source spent today, the most first: the source, its name, its share, and how much.
    Sources(Vec<(Source, String, f32, String)>),
    /// Facts two to a row: what, and how much.
    Pairs(Vec<(String, String)>),
    /// The sessions that spent today, the latest first: each one's title, its model and when it last spent, how
    /// much it spent today, whether it is at work now, and the source it is of where sessions of several are told.
    Sessions(Vec<(String, String, String, bool, Option<Source>)>),
    /// The sessions of the sources shown: how many are at work, and how many spent today.
    Working { at_work: usize, today: usize },
    /// A source's windows all side by side in one row, without when each starts again (a source's part in a widget too
    /// small for all of them: see `paint_paged`).
    WindowsRow(Vec<Window>),
    /// The whole component in a widget too small for all of it, in `shape`: what the dock shows, how today's spending
    /// mounted, and what it tells in a widget roomy enough for more (see `paint_shaped`).
    Shaped { shape: Shape, dock: Dock, mounted: Vec<f32>, rich: Box<Rich> },
}

#[derive(Clone, PartialEq, Debug)]
pub struct Window {
    pub name: String,
    pub used: f32,
    pub gone: Option<f32>,
    pub resets: String,
    /// Told by how much of it is left (see `Prefs::ai_left`), not used.
    pub left: bool,
    /// Its time to start again gone by, and nothing told of it since: what it tells may be out of date (drawn faint).
    pub stale: bool,
}

/// What a source's mark tells beside its ring, its figure and its name, where compact forms tell no more (its card says
/// it): what was told of a limit may be out of date (a window's time to start again gone by, nothing told since), a
/// hollow grey ring, not a problem; or reading it failed (its key refused, Codex's sign-in expired, the provider not
/// reached), a red dot.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Flag {
    Stale,
    Problem,
}

/// `tool`'s flag, of `usage` (see `Flag`), its `windows` as they are told: a problem before anything out of date.
#[cfg(windows)]
fn flag_of(usage: &crate::ai::Usage, tool: crate::ai::Tool, windows: &[Window]) -> Option<Flag> {
    if usage.failed.iter().any(|(of, _)| *of == tool) {
        Some(Flag::Problem)
    } else if windows.iter().any(|window| window.stale) {
        Some(Flag::Stale)
    } else {
        None
    }
}

/// Draws `flag` round `centre`, `side` across: a red dot, or a hollow grey ring.
fn paint_flag(frame: &dyn Canvas, theme: &Theme, flag: Flag, centre: Point, side: f32) {
    match flag {
        Flag::Problem => frame.fill_circle(theme.signal, centre, side / 2.0),
        Flag::Stale => frame.arc(centre, side / 2.0 - 0.9, 0.0, std::f32::consts::TAU, theme.text2, 1.8),
    }
}

/// How a slot's figure is written: hot where its window is nearly spent, faint where what it tells may be out of date.
fn figure_ink(theme: &Theme, slot: &Slot) -> Color {
    if slot.faint {
        theme.text.alpha(theme.text.a * 0.45)
    } else if slot.used.is_some_and(|used| used >= HOT) {
        theme.signal
    } else {
        theme.text
    }
}

impl Window {
    /// The share of it its bar is drawn to: what is used, or what is left.
    pub fn shown(&self) -> f32 {
        if self.left { 1.0 - self.used } else { self.used }
    }

    /// Where the mark of how much of it has gone by is drawn across its bar: that share, or, told by what is left, how
    /// much of it is to come (so a bar short of its mark tells, either way, that it is spent faster than it goes by).
    pub fn mark(&self) -> Option<f32> {
        self.gone.map(|gone| if self.left { 1.0 - gone } else { gone })
    }
}

/// The share `shown` told as a figure, what is left said with it where `left` and the words fit (`long`).
fn share_told(shown: f32, left: bool, long: bool, lang: Lang) -> String {
    let figure = format!("{:.0}%", shown * 100.0);
    match (left && long, lang) {
        (false, _) => figure,
        (true, Lang::Zh) => format!("剩余 {figure}"),
        (true, Lang::En) => format!("{figure} left"),
    }
}

/// What a figure of `window` (its name, where it has several) is of, told by what is left.
fn left_of(window: &str, lang: Lang) -> String {
    match (window.is_empty(), lang) {
        (true, Lang::Zh) => "剩余".into(),
        (true, Lang::En) => "Left".into(),
        (false, Lang::Zh) => format!("{window}剩余"),
        (false, Lang::En) => format!("{window} left"),
    }
}

/// What the component tells in a widget roomy enough for more than its dock tells (see `paint_shaped`), each part as
/// its lane laid out across tells it (see `across`): what every source spent today, and each source's head and limits
/// with what it spent by model; the sessions at work and of today; and the sessions of the sources docked.
#[derive(Clone, PartialEq, Debug, Default, Hash)]
pub struct Rich {
    pub parts: Vec<Part>,
    pub working: Option<AiBlock>,
    pub sessions: Vec<SessionRow>,
}

/// A session as a row (see `AiBlock::Sessions`).
pub type SessionRow = (String, String, String, bool, Option<Source>);

/// A part of what the component tells in a roomy widget (see `Rich`): its module, what it shows, and (a source's) what it
/// spent by model, told where there is room for it.
#[derive(Clone, PartialEq, Debug, Hash)]
pub struct Part {
    pub id: String,
    pub shows: Vec<Told>,
    pub models: Option<AiBlock>,
}

/// A block a part shows (see `Part`), kept as what it tells, compared and copied with the rest of the widget's shape: a
/// head, one of the component's own, or facts.
#[derive(Clone, PartialEq, Debug, Hash)]
pub enum Told {
    Head { title: String, aside: String },
    Ai(AiBlock),
    Facts(Vec<(String, String, bool)>),
}

impl Told {
    /// What `block` tells, where a part shows it.
    fn of(block: Block) -> Option<Told> {
        match block {
            Block::Head { title, aside, .. } => Some(Told::Head { title, aside }),
            Block::Ai(block) => Some(Told::Ai(block)),
            Block::Facts { rows, .. } => Some(Told::Facts(rows)),
            _ => None,
        }
    }

    /// It as a block of a lane, as it was.
    fn block(&self) -> Block {
        match self {
            Told::Head { title, aside } => head(title.clone(), "", aside.clone(), false),
            Told::Ai(block) => Block::Ai(block.clone()),
            Told::Facts(rows) => Block::Facts { rows: rows.clone(), gap: GAP },
        }
    }
}

impl Hash for AiBlock {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            AiBlock::Head { source, title, device, aside, opened, flag } => (source, title, device, aside, opened, flag).hash(state),
            AiBlock::Today { figure, unit, mounted } => (figure, unit, mounted.iter().map(|f| f.to_bits()).collect::<Vec<_>>()).hash(state),
            AiBlock::Windows(windows) | AiBlock::WindowsRow(windows) => windows.iter().for_each(|w| (&w.name, w.used.to_bits(), w.gone.map(f32::to_bits), &w.resets, w.left, w.stale).hash(state)),
            AiBlock::Models(rows) => rows.iter().for_each(|(name, share, value)| (name, share.to_bits(), value).hash(state)),
            AiBlock::Sources(rows) => rows.iter().for_each(|(source, name, share, value)| (source, name, share.to_bits(), value).hash(state)),
            AiBlock::Pairs(rows) => rows.hash(state),
            AiBlock::Sessions(rows) => rows.hash(state),
            AiBlock::Working { at_work, today } => (at_work, today).hash(state),
            AiBlock::Shaped { shape, dock, mounted, rich } => {
                (shape, &dock.today, &dock.at_work, dock.working).hash(state);
                dock.slots.iter().for_each(|slot| {
                    (slot.source, slot.used.map(f32::to_bits), &slot.figure, &slot.label, &slot.resets, slot.left, slot.flag, slot.faint).hash(state);
                    slot.windows.iter().for_each(|(name, used)| (name, used.to_bits()).hash(state));
                });
                mounted.iter().for_each(|share| share.to_bits().hash(state));
                rich.hash(state);
            }
        }
    }
}

/// The room above each block (DIPs): what one tells set apart from what the one before it does.
const GAP: f32 = 14.0;
/// Between what every source spent and each source, one source and the next, and the last and the sessions at work:
/// each source read as one, apart from the next.
const SOURCE_GAP: f32 = 22.0;
/// Between two rows of a block (its models, its facts two to a row): each row read as one, apart from the next.
const ROW_GAP: f32 = 6.0;
/// A window's bar, the room it is drawn in between its name and when it starts again (its middle there): clear of both.
const BAR_BAND: f32 = 14.0;
/// A window's: its name and how much is used, its bar, when it starts again (see `window_tall`).
const WINDOW: f32 = LINE + BAR_BAND + LINE;

/// How tall each of `windows` is: when it starts again under its bar only where any tells it.
fn window_tall(windows: &[Window]) -> f32 {
    if windows.iter().any(|window| !window.resets.is_empty()) { WINDOW } else { LINE + BAR_BAND }
}
/// Between two windows side by side, and two rows of them.
const WINDOW_GAP: f32 = 16.0;
/// The models' bar, and the room under it.
const MODELS_BAR: f32 = 6.0;
const MODELS_BAR_GAP: f32 = 10.0;
/// A session's: its title over what it is (`SESSION_LINES` apart), and the room between it and the next, more than
/// that: each session's two lines read as one.
const SESSION_LINES: f32 = 2.0;
const SESSION_GAP: f32 = 10.0;
const SESSION: f32 = LINE + SESSION_LINES + LINE + SESSION_GAP;
/// The most sessions told of in a source's lane.
const SESSIONS: usize = 4;
/// The sessions the card beside the dock's 会话 tells of: as many as there are at work, up to the most, and at least
/// so many with the others of today.
const SESSIONS_AT_WORK: usize = 8;
const SESSIONS_TOLD: usize = 6;
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
                GAP + rows * window_tall(windows) + (rows - 1.0).max(0.0) * WINDOW_GAP
            }
            AiBlock::WindowsRow(_) => GAP + LINE + BAR_BAND,
            AiBlock::Models(rows) => GAP + MODELS_BAR + MODELS_BAR_GAP + rows.len() as f32 * (LINE + ROW_GAP) - ROW_GAP,
            AiBlock::Sources(rows) => GAP + MODELS_BAR + MODELS_BAR_GAP + rows.len() as f32 * (LINE + ROW_GAP) - ROW_GAP,
            AiBlock::Pairs(pairs) => GAP + pairs.len().div_ceil(2) as f32 * (LINE + ROW_GAP) - ROW_GAP,
            AiBlock::Sessions(rows) => GAP + rows.len() as f32 * SESSION - SESSION_GAP,
            AiBlock::Working { .. } => SOURCE_GAP + LINE,
            // (As large as the widget it fills: see `paint_shaped`.)
            AiBlock::Shaped { .. } => BENCH,
        }
    }

    /// What it is, as its parts are named (see `paint_lane`).
    pub(super) fn kind(&self) -> &'static str {
        match self {
            AiBlock::Head { .. } => "head",
            AiBlock::Today { .. } => "today",
            AiBlock::Windows(_) | AiBlock::WindowsRow(_) => "windows",
            AiBlock::Models(_) => "models",
            AiBlock::Sources(_) => "sources",
            AiBlock::Pairs(_) => "pairs",
            AiBlock::Sessions(_) => "sessions",
            AiBlock::Working { .. } => "working",
            AiBlock::Shaped { .. } => "shaped",
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

pub(super) fn grouped(value: u64) -> String {
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
fn failed_words(failed: crate::ai::Failed, keyed: bool, lang: Lang) -> &'static str {
    match failed {
        // (Codex is asked with the sign-in it keeps, not a key.)
        crate::ai::Failed::Key if !keyed => lang.pick("登录已过期", "Sign-in expired"),
        crate::ai::Failed::Key => lang.pick("密钥无效", "Key refused"),
        crate::ai::Failed::Limited => lang.pick("请求过于频繁", "Asked too often"),
        crate::ai::Failed::Unavailable => lang.pick("暂时无法读取", "Unreachable"),
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
        Some((_, failed)) => Some(fact(lang.pick("状态", "State"), failed_words(*failed, true, lang), true)),
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

/// The AI component's lane as `prefs` show it, at `detail` (see `together`); none where it shows nothing.
#[cfg(windows)]
pub(super) fn ai_lane(prefs: &Prefs, lang: Lang, detail: Detail, theme: &Theme) -> Option<Vec<Block>> {
    together(prefs, lang, crate::ai::usage().as_deref(), crate::station::now_ms(), detail, opened, theme)
}

/// The AI component's lane of `usage` (none: not read yet) at `now`, one plate for it all: what every source spent
/// today (where `ai:today` is on), then each source switched on, in their order, `opened` as it is toggled, and the
/// sessions of those at work; laid out across where `prefs` say so (see `across`); in a widget too small for it all,
/// its shape (see `paint_shaped`). None where it shows none of them.
#[cfg(windows)]
pub(super) fn together(prefs: &Prefs, lang: Lang, usage: Option<&crate::ai::Usage>, now: u64, detail: Detail, opened: impl Fn(Source) -> bool, theme: &Theme) -> Option<Vec<Block>> {
    let ids: Vec<&str> = prefs.modules.iter().filter(|entry| shows_module(entry) && entry.id.starts_with("ai:")).map(|entry| entry.id.as_str()).collect();
    if ids.is_empty() {
        return None;
    }
    if let Detail::Shape(shape) = detail {
        let mounted = usage.map_or(Vec::new(), mounted);
        let rich = Box::new(rich(prefs, &ids, lang, usage, now));
        return Some(vec![Block::Ai(AiBlock::Shaped { shape, dock: dock_of(prefs, lang, usage, now), mounted, rich })]);
    }
    if prefs.across {
        return Some(vec![Block::Across(across(prefs, &ids, lang, usage, now, &opened, theme))]);
    }
    let mut blocks = Vec::new();
    for id in ids {
        if !blocks.is_empty() {
            blocks.push(Block::Room(SOURCE_GAP));
        }
        if id == "ai:today" {
            blocks.extend(spent_today(lang, usage));
            continue;
        }
        let Some(source) = Source::of(id) else { continue };
        blocks.extend(lane_of(id, prefs, lang, usage, now, Some(opened(source))).into_iter().flatten());
    }
    blocks.extend(working(prefs, usage, now));
    Some(blocks)
}

/// The most parts the component laid out across sets side by side (see `across`): what every source spent today and
/// three sources, the rest in the rows under them.
const ACROSS_MOST: usize = 4;

/// The component laid out across, as a panel opened from a top edge shows it, low and wide (see `Block::Across`): what
/// every source spent today, with the sessions at work under it, beside each source's head and limits, at most
/// `ACROSS_MOST` side by side, the rest in rows under them; and under them, for each source opened, what it spent by
/// kind, by model and its sessions, side by side in a row of its own, each under the source's mark and what it tells.
/// The parts of `ids` (the modules shown, in their order) as `together` tells them.
#[cfg(windows)]
fn across(prefs: &Prefs, ids: &[&str], lang: Lang, usage: Option<&crate::ai::Usage>, now: u64, opened: &impl Fn(Source) -> bool, theme: &Theme) -> Vec<Vec<Lane>> {
    let part = |id: String, blocks: Vec<Block>| Lane { ink: theme.ink(&id), id, blocks };
    let mut parts: Vec<Lane> = Vec::new();
    let mut told: Vec<Vec<Lane>> = Vec::new();
    for id in ids {
        if *id == "ai:today" {
            parts.push(part(id.to_string(), spent_today(lang, usage)));
            continue;
        }
        let Some(source) = Source::of(id) else { continue };
        let Some(blocks) = lane_of(id, prefs, lang, usage, now, Some(opened(source))) else { continue };
        // What opening it tells goes in a row of its own under the heads, the rest stays under its head.
        let (opens, stays): (Vec<Block>, Vec<Block>) = blocks.into_iter().partition(|block| matches!(block, Block::Ai(AiBlock::Models(_) | AiBlock::Pairs(_) | AiBlock::Sessions(_))));
        parts.push(part(id.to_string(), stays));
        let mut opens: Vec<(usize, Block)> = opens.into_iter().map(|block| (opened_order(&block), block)).collect();
        opens.sort_by_key(|(order, _)| *order);
        let row: Vec<Lane> = opens
            .into_iter()
            .map(|(order, block)| {
                let (kind, title) = [("pairs", lang.pick("今日构成", "Today by kind")), ("models", lang.pick("模型", "Models")), ("sessions", lang.pick("会话", "Sessions"))][order];
                let caption = Block::Ai(AiBlock::Head { source, title: title.into(), device: String::new(), aside: String::new(), opened: None, flag: None });
                part(format!("{id}.{kind}"), vec![caption, block])
            })
            .collect();
        if !row.is_empty() {
            told.push(row);
        }
    }
    // The sessions at work under the first part (what every source spent, where it shows), else as one of their own.
    if let Some(working) = working(prefs, usage, now) {
        match parts.first_mut() {
            Some(first) => first.blocks.push(working),
            None => parts.push(part("ai:sessions".into(), vec![working])),
        }
    }
    let mut rows = Vec::new();
    for row in std::iter::once(parts).chain(told) {
        let mut row = row.into_iter().peekable();
        while row.peek().is_some() {
            rows.push(row.by_ref().take(ACROSS_MOST).collect());
        }
    }
    rows
}

/// Where what opening a source tells goes in its row laid out across (see `across`): what it spent by kind, by model,
/// then its sessions.
fn opened_order(block: &Block) -> usize {
    match block {
        Block::Ai(AiBlock::Pairs(_)) => 0,
        Block::Ai(AiBlock::Models(_)) => 1,
        _ => 2,
    }
}

/// What every source spent today, under its head: the figure and how it mounted (or that it is being read).
fn spent_today(lang: Lang, usage: Option<&crate::ai::Usage>) -> Vec<Block> {
    vec![head(lang.pick("AI 用量", "AI usage"), "", lang.pick("今日", "Today"), false), usage.map_or_else(|| reading(lang), |usage| today_block(usage, lang))]
}

/// What the component tells in a widget roomy enough for more than its dock tells (see `Rich`), of `ids` (the modules
/// shown, in their order): each source as its lane closed shows it, its head opening nothing, and what it spent by
/// model, as opened.
#[cfg(windows)]
fn rich(prefs: &Prefs, ids: &[&str], lang: Lang, usage: Option<&crate::ai::Usage>, now: u64) -> Rich {
    let mut parts = Vec::new();
    for id in ids {
        if *id == "ai:today" {
            parts.push(Part { id: id.to_string(), shows: spent_today(lang, usage).into_iter().filter_map(Told::of).collect(), models: None });
            continue;
        }
        let Some(closed) = lane_of(id, prefs, lang, usage, now, Some(false)) else { continue };
        let shows = closed
            .into_iter()
            .filter_map(|block| match block {
                Block::Ai(AiBlock::Head { source, title, device, aside, flag, .. }) => Some(Told::Ai(AiBlock::Head { source, title, device, aside, opened: None, flag })),
                block => Told::of(block),
            })
            .collect();
        let models = lane_of(id, prefs, lang, usage, now, Some(true)).into_iter().flatten().find_map(|block| match block {
            Block::Ai(models @ AiBlock::Models(_)) => Some(models),
            _ => None,
        });
        parts.push(Part { id: id.to_string(), shows, models });
    }
    let working = working(prefs, usage, now).and_then(|block| match block {
        Block::Ai(working) => Some(working),
        _ => None,
    });
    let sessions = usage.map_or(Vec::new(), |usage| docked_sessions(prefs, lang, usage, now).0);
    Rich { parts, working, sessions }
}

/// The sessions of the sources shown (see `docked`): how many are at work, and of today; none where there are none.
fn working(prefs: &Prefs, usage: Option<&crate::ai::Usage>, now: u64) -> Option<Block> {
    let tools: Vec<crate::ai::Tool> = docked(prefs).into_iter().map(tool_of).collect();
    let usage = usage.filter(|_| !tools.is_empty())?;
    let today = usage.sessions.iter().filter(|session| tools.contains(&session.tool) && session.spent_between(usage.today, usage.now) > 0).count();
    let at_work = usage.sessions.iter().filter(|session| tools.contains(&session.tool) && now.saturating_sub(session.last) < AT_WORK_MS).count();
    (today > 0 || at_work > 0).then_some(Block::Ai(AiBlock::Working { at_work, today: today.max(at_work) }))
}

/// The card beside the dock for `spot`: a source's lane, all it tells, its head opening nothing; what every source spent
/// today (see `today_card`); or the sessions of the sources docked (see `sessions_card`).
#[cfg(windows)]
pub fn card(scene: &Scene, spot: Spot) -> Option<Lane> {
    let (usage, now) = (crate::ai::usage(), crate::station::now_ms());
    let blocks = match spot {
        Spot::Ring(source) => lane_of(source.id(), scene.prefs, scene.lang, usage.as_deref(), now, None)?,
        Spot::Today => today_card(scene.lang, usage.as_deref()),
        Spot::Sessions => sessions_card(scene.prefs, scene.lang, usage.as_deref(), now),
    };
    Some(Lane { id: spot.id().into(), blocks, ink: scene.theme.ink(spot.id()) })
}

/// What every source spent today, as the card beside the dock's 今日 tells it: the figure and how it mounted through the
/// day, each source's share (the most first), and the whole by kind.
pub(super) fn today_card(lang: Lang, usage: Option<&crate::ai::Usage>) -> Vec<Block> {
    let head = head(lang.pick("今日用量", "Today's usage"), "", "", false);
    let Some(usage) = usage else { return vec![head, reading(lang)] };
    let mut blocks = vec![head, today_block(usage, lang)];
    let mut by: Vec<(Source, u64)> = Vec::new();
    let mut whole = crate::ai::Tokens::default();
    for (tool, _, tokens) in &usage.spent_today {
        whole.add(*tokens);
        let Some(source) = SOURCES.into_iter().find(|source| tool_of(*source) == *tool) else { continue };
        match by.iter_mut().find(|(of, _)| *of == source) {
            Some((_, spent)) => *spent += tokens.total(),
            None => by.push((source, tokens.total())),
        }
    }
    by.retain(|(_, spent)| *spent > 0);
    by.sort_by(|a, b| b.1.cmp(&a.1));
    let told: u64 = by.iter().map(|(_, spent)| spent).sum();
    if told > 0 {
        let name = |source: Source| if source.keyed() { source.name(lang).to_string() } else { source.tool_name().to_string() };
        blocks.push(Block::Ai(AiBlock::Sources(by.iter().map(|&(source, spent)| (source, name(source), spent as f32 / told as f32, short(spent, lang))).collect())));
    }
    let sessions = usage.sessions.iter().filter(|session| session.spent_between(usage.today, usage.now) > 0).count();
    blocks.push(Block::Ai(AiBlock::Pairs(kinds(whole, sessions, lang))));
    blocks
}

/// The sessions of the sources docked, as the card beside the dock's 会话 tells them: those at work (how many in its
/// head), the latest first, then the others of today, each with its source's mark.
pub(super) fn sessions_card(prefs: &Prefs, lang: Lang, usage: Option<&crate::ai::Usage>, now: u64) -> Vec<Block> {
    let Some(usage) = usage else { return vec![head(lang.pick("会话", "Sessions"), "", "", false), reading(lang)] };
    let (sessions, working) = docked_sessions(prefs, lang, usage, now);
    let aside = match lang {
        Lang::Zh => format!("{working} 个进行中"),
        Lang::En => format!("{working} at work"),
    };
    let mut blocks = vec![head(lang.pick("会话", "Sessions"), "", aside, false)];
    if sessions.is_empty() {
        blocks.push(Block::Facts { rows: vec![(lang.pick("今日会话", "Today").into(), lang.pick("暂无", "None").into(), false)], gap: GAP });
        return blocks;
    }
    let shown = working.clamp(SESSIONS_TOLD, SESSIONS_AT_WORK);
    blocks.push(Block::Ai(AiBlock::Sessions(sessions.into_iter().take(shown).collect())));
    blocks
}

/// The sessions of the sources docked, as rows: those at work, the latest first, then the others of today, each with
/// its source's mark; and how many are at work.
fn docked_sessions(prefs: &Prefs, lang: Lang, usage: &crate::ai::Usage, now: u64) -> (Vec<SessionRow>, usize) {
    let docked = docked(prefs);
    let at_work = |session: &crate::ai::Session| now.saturating_sub(session.last) < AT_WORK_MS;
    let mut sessions: Vec<(Source, &crate::ai::Session)> = usage
        .sessions
        .iter()
        .filter(|session| at_work(session) || session.spent_between(usage.today, usage.now) > 0)
        .filter_map(|session| Some((docked.iter().copied().find(|source| tool_of(*source) == session.tool)?, session)))
        .collect();
    sessions.sort_by(|(_, a), (_, b)| at_work(b).cmp(&at_work(a)).then(b.last.cmp(&a.last)));
    let working = sessions.iter().filter(|(_, session)| at_work(session)).count();
    (sessions.into_iter().map(|(source, session)| session_row(session, usage, now, lang, Some(source))).collect(), working)
}

/// Today's figure and how it mounted through the day, every tool's quarters added up.
fn today_block(usage: &crate::ai::Usage, lang: Lang) -> Block {
    let total: u64 = usage.spent_today.iter().map(|(_, _, tokens)| tokens.total()).sum();
    let (figure, unit) = tokens(total, lang);
    Block::Ai(AiBlock::Today { figure, unit, mounted: mounted(usage) })
}

/// How today's spending mounted: what every tool spent by the end of each quarter hour so far, from 0 to 1 of the whole.
fn mounted(usage: &crate::ai::Usage) -> Vec<f32> {
    let total: u64 = usage.spent_today.iter().map(|(_, _, tokens)| tokens.total()).sum();
    // (Every tool's quarters added up, then how far the day had come by each.)
    let quarters = usage.quarters.iter().map(|(_, spent)| spent.len()).max().unwrap_or(0);
    let mut mounted = Vec::with_capacity(quarters);
    let mut sum = 0;
    for at in 0..quarters {
        sum += usage.quarters.iter().map(|(_, spent)| spent.get(at).copied().unwrap_or(0)).sum::<u64>();
        mounted.push(if total == 0 { 0.0 } else { sum as f32 / total as f32 });
    }
    mounted
}

/// What `total` was spent on by kind, and how many `sessions` spent it.
fn kinds(total: crate::ai::Tokens, sessions: usize, lang: Lang) -> Vec<(String, String)> {
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
    pairs.push((lang.pick("会话", "Sessions").into(), sessions.to_string()));
    pairs
}

/// A session as a row: its title (else its folder's name), its model and when it last spent, what it spent today,
/// whether it is at work, and `source` where sessions of several are told.
fn session_row(session: &crate::ai::Session, usage: &crate::ai::Usage, now: u64, lang: Lang, source: Option<Source>) -> (String, String, String, bool, Option<Source>) {
    let title = session.title.clone().or_else(|| session.folder.as_deref().and_then(folder_name)).unwrap_or_else(|| lang.pick("未命名会话", "Untitled session").into());
    let model = session.model.as_deref().map(model_name).unwrap_or_default();
    let when = text::since(session.last, now, lang);
    let about = if model.is_empty() { when } else { format!("{model} · {when}") };
    (title, about, short(session.spent_between(usage.today, usage.now), lang), now.saturating_sub(session.last) < AT_WORK_MS, source)
}

/// A lane's row while it is being read.
fn reading(lang: Lang) -> Block {
    Block::Facts { rows: vec![(lang.pick("今日用量", "Today").into(), lang.pick("正在读取", "Reading").into(), false)], gap: GAP }
}

/// The limit `tool` last told: its own (Codex's "codex"), else the first told.
#[cfg(windows)]
fn limit_of(usage: &crate::ai::Usage, tool: crate::ai::Tool) -> Option<&crate::ai::Limit> {
    let limits: Vec<&crate::ai::Limit> = usage.limits.iter().filter(|limit| limit.tool == tool).collect();
    limits.iter().find(|limit| limit.id == "codex").or_else(|| limits.first()).copied()
}

/// The windows of `limit` as they stand at `now`: one started again since it was told, none of it used.
#[cfg(windows)]
fn windows_of(limit: Option<&crate::ai::Limit>, now: u64, lang: Lang, told_left: bool) -> Vec<Window> {
    limit.map_or(Vec::new(), |limit| {
        limit
            .windows
            .iter()
            .map(|window| {
                let over = window.resets.is_some_and(|resets| resets <= now);
                // (How long till it starts again, and so how much of it has gone, as of the minute: as it is told.)
                let left = window.resets.filter(|_| !over).map(|resets| (resets - now).div_ceil(60_000));
                let resets = match left {
                    _ if over => lang.pick("已过重置时间，尚未取得新数据", "Past its reset, not read again yet").into(),
                    Some(left) => {
                        let left = left_in(left, lang);
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
                    gone: left.filter(|_| window.minutes > 0).map(|left| (1.0 - left as f32 / window.minutes as f32).clamp(0.0, 1.0)),
                    resets,
                    left: told_left,
                    stale: over,
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
    let reading = || reading(lang);
    if id == "ai:today" {
        let head = head(lang.pick("AI 用量", "AI usage"), "", lang.pick("今日", "Today"), false);
        let Some(usage) = usage else { return Some(vec![head, reading()]) };
        let mut blocks = vec![head, today_block(usage, lang)];
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
    let head = |device: String, aside: String, opened: Option<bool>, flag: Option<Flag>| Block::Ai(AiBlock::Head { source, title: source.name(lang).into(), device, aside, opened, flag });
    let Some(usage) = usage else { return Some(vec![head(source.tool_name().into(), String::new(), None, None), reading()]) };
    let limit = limit_of(usage, tool);
    let sessions: Vec<&crate::ai::Session> = usage.sessions.iter().filter(|session| session.tool == tool && session.spent_between(usage.today, usage.now) > 0).collect();
    // When it was last heard of: its limit told, else its last session's spending.
    let heard = limit.map(|limit| limit.told).into_iter().chain(sessions.first().map(|session| session.last)).max();
    let device = limit.and_then(|limit| limit.plan.as_deref()).map_or(source.tool_name().to_string(), plan_name);
    let windows = windows_of(limit, now, lang, prefs.ai_left);
    let (spent, total) = spent_by(usage, tool);
    // Its head tells what it spent today, in the component's lane, where it spent anything (beside the dock, what it
    // spent is told below); else when it was last heard of.
    let aside = match open {
        Some(_) if total.total() > 0 => short(total.total(), lang),
        _ => heard.map_or(String::new(), |heard| text::since(heard, now, lang)),
    };
    // (It opens where it spent today: there is more to tell.)
    let mut blocks = vec![head(device, aside, open.filter(|_| total.total() > 0), flag_of(usage, tool, &windows))];
    // A provider's account: why it is not shown as it stands, where it is not. Codex's account, where asking it
    // failed: why, its limits as its logs here last told them.
    if source.keyed() {
        blocks.extend(account_state(tool, usage, limit, total.total() > 0, lang));
    } else if let Some((_, failed)) = usage.failed.iter().find(|(of, _)| *of == tool) {
        let said = match lang {
            Lang::Zh => format!("{}，额度来自本机记录", failed_words(*failed, false, lang)),
            Lang::En => format!("{}, limits from this PC's records", failed_words(*failed, false, lang)),
        };
        blocks.push(Block::Facts { rows: vec![(lang.pick("状态", "State").into(), said, true)], gap: GAP });
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
        // (What it spent is told by its head in the component's lane.)
        let today = open.is_none() && if source.keyed() { total.total() > 0 && !shown_limit } else { limit.is_none() };
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
    blocks.push(Block::Ai(AiBlock::Pairs(kinds(total, sessions.len(), lang))));
    if prefs.shows(id, "sessions") && !sessions.is_empty() {
        blocks.push(Block::Ai(AiBlock::Sessions(sessions.iter().take(SESSIONS).map(|session| session_row(session, usage, now, lang, None)).collect())));
    }
    Some(blocks)
}

/// A source's mark at the start of its head, and how far its name is from the head's start (DIPs).
const HEAD_MARK_SIDE: f32 = 16.0;
const HEAD_MARK: f32 = HEAD_MARK_SIDE + 7.0;

/// Draws `block` in its lane's ink, from `y` across `width`; where a press opens or closes it, there.
pub(super) fn paint(frame: &dyn Canvas, scene: &Scene, ink: Ink, block: &AiBlock, (left, y, width): (f32, f32, f32), part: &dyn Fn(&str), hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    match block {
        AiBlock::Head { source, title, device, aside, opened, flag } => {
            let row = y + (HEAD - theme.title.size) / 2.0 - 1.0;
            part("mark");
            frame.mark(source.mark(), Point { x: left + HEAD_MARK_SIDE / 2.0, y: y + HEAD / 2.0 }, HEAD_MARK_SIDE, theme.text, 1.0);
            let title_left = left + HEAD_MARK;
            let title_width = frame.measure(title, theme.title);
            part("label");
            frame.text(title, theme.title, theme.text, title_left, row, width - (title_left - left), Align::Start);
            // (Its mark after its name: what its card says.)
            let title_width = match flag {
                Some(flag) => {
                    part("flag");
                    paint_flag(frame, theme, *flag, Point { x: title_left + title_width + 7.0, y: y + HEAD / 2.0 }, 7.0);
                    title_width + 12.0
                }
                None => title_width,
            };
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
        AiBlock::Windows(windows) | AiBlock::WindowsRow(windows) => {
            // (Two to a row, or all in one.)
            let across = if matches!(block, AiBlock::WindowsRow(_)) { windows.len().max(1) } else { 2 };
            let column = (width - (across - 1) as f32 * WINDOW_GAP) / across as f32;
            let tall = window_tall(windows);
            for (at, window) in windows.iter().enumerate() {
                let (x, top) = (left + (at % across) as f32 * (column + WINDOW_GAP), y + GAP + (at / across) as f32 * (tall + WINDOW_GAP));
                // (One alone across the lane.)
                let wide = if windows.len() == 1 { width } else { column };
                let hot = window.used >= HOT;
                part(&format!("{at}.label"));
                frame.text(&window.name, theme.small, theme.text2, x, top, wide, Align::Start);
                part(&format!("{at}.value"));
                // (What is left said with it where it fits beside the window's name.)
                let long = share_told(window.shown(), window.left, true, scene.lang);
                let value = if frame.measure(&window.name, theme.small) + 8.0 + frame.measure(&long, theme.small) <= wide { long } else { share_told(window.shown(), window.left, false, scene.lang) };
                let faint = |color: Color| if window.stale { color.alpha(color.a * 0.45) } else { color };
                frame.text(&value, theme.small, faint(if hot { theme.signal } else { theme.text }), x, top, wide, Align::End);
                let bar_y = top + LINE + (BAR_BAND - theme.meter_height) / 2.0;
                part(&format!("{at}.bar"));
                meter(frame, theme, (x, bar_y, wide), window.shown(), faint(if hot { theme.signal } else { ink.trace }));
                // How much of the window has gone by (or is to come, told by what is left), a mark across the bar:
                // used ahead of it (left short of it), the limit is spent faster than the window goes.
                if let Some(gone) = window.mark() {
                    let mark_x = x + gone * wide;
                    // (Kept within the bar, at its start where the bar is shorter than the mark is wide.)
                    frame.fill(theme.text2, (mark_x - 0.75).min(x + wide - 1.5).max(x), bar_y - 2.0, 1.5, theme.meter_height + 4.0);
                }
                part(&format!("{at}.resets"));
                frame.text(&window.resets, theme.small, if window.stale { theme.text2 } else { theme.text3 }, x, top + LINE + BAR_BAND, wide, Align::Start);
            }
        }
        AiBlock::Models(rows) => {
            let bar_y = y + GAP;
            part("bar");
            let parts: Vec<_> = rows.iter().enumerate().map(|(at, (_, share, _))| (*share, ink.trace.alpha(strength(at)))).collect();
            shares(frame, theme, (left, bar_y + (MODELS_BAR - theme.meter_height) / 2.0, width), &parts);
            for (at, (model, _, value)) in rows.iter().enumerate() {
                let row_y = bar_y + MODELS_BAR + MODELS_BAR_GAP + at as f32 * (LINE + ROW_GAP);
                part(&at.to_string());
                frame.fill_rounded(ink.trace.alpha(strength(at)), left, row_y + (LINE - 8.0) / 2.0, 8.0, 8.0, 2.0);
                frame.text(model, theme.small, theme.text2, left + 14.0, row_y, width - 14.0, Align::Start);
                frame.text(value, theme.small, theme.text, left, row_y, width, Align::End);
            }
        }
        AiBlock::Sources(rows) => {
            const MARK: f32 = 12.0;
            let bar_y = y + GAP;
            part("bar");
            let parts: Vec<_> = rows.iter().map(|(source, _, share, _)| (*share, theme.ink(source.id()).trace)).collect();
            shares(frame, theme, (left, bar_y + (MODELS_BAR - theme.meter_height) / 2.0, width), &parts);
            for (at, (source, name, _, value)) in rows.iter().enumerate() {
                let row_y = bar_y + MODELS_BAR + MODELS_BAR_GAP + at as f32 * (LINE + ROW_GAP);
                part(&at.to_string());
                frame.mark(source.mark(), Point { x: left + MARK / 2.0, y: row_y + LINE / 2.0 }, MARK, theme.text2, 1.0);
                frame.text(name, theme.small, theme.text2, left + MARK + 6.0, row_y, width - MARK - 6.0, Align::Start);
                frame.text(value, theme.small, theme.text, left, row_y, width, Align::End);
            }
        }
        AiBlock::Pairs(pairs) => {
            let column = (width - WINDOW_GAP) / 2.0;
            for (at, (label, value)) in pairs.iter().enumerate() {
                let (x, row_y) = (left + (at % 2) as f32 * (column + WINDOW_GAP), y + GAP + (at / 2) as f32 * (LINE + ROW_GAP));
                part(&at.to_string());
                frame.text(label, theme.small, theme.text2, x, row_y, column, Align::Start);
                frame.text(value, theme.small, theme.text, x, row_y, column, Align::End);
            }
        }
        AiBlock::Sessions(rows) => {
            const DOT: f32 = 3.5;
            const MARK: f32 = 13.0;
            for (at, (title, about, value, at_work, source)) in rows.iter().enumerate() {
                let row_y = y + GAP + at as f32 * SESSION;
                part(&at.to_string());
                let lead = match source {
                    // Its source's mark: whole at work, else faint.
                    Some(source) => {
                        frame.mark(source.mark(), Point { x: left + MARK / 2.0, y: row_y + LINE / 2.0 }, MARK, theme.text, if *at_work { 1.0 } else { 0.4 });
                        MARK
                    }
                    // At work: a dot in its ink; else a ring.
                    None => {
                        let centre = Point { x: left + DOT, y: row_y + LINE / 2.0 };
                        if *at_work {
                            frame.fill_circle(ink.trace, centre, DOT);
                        } else {
                            frame.arc(centre, DOT - 0.5, 0.0, 360.0, theme.text3, 1.0);
                        }
                        2.0 * DOT
                    }
                };
                let words = left + lead + 8.0;
                let value_width = frame.measure(value, theme.small);
                frame.text(title, theme.body, theme.text, words, row_y, left + width - value_width - 12.0 - words, Align::Start);
                frame.text(about, theme.small, theme.text3, words, row_y + LINE + SESSION_LINES, left + width - words, Align::Start);
                frame.text(value, theme.small, theme.text2, left, row_y, width, Align::End);
            }
        }
        AiBlock::Working { at_work, today } => {
            const DOT: f32 = 3.5;
            let row_y = y + SOURCE_GAP;
            let lang = scene.lang;
            part("dot");
            let centre = Point { x: left + DOT, y: row_y + LINE / 2.0 };
            if *at_work > 0 {
                frame.fill_circle(ink.trace, centre, DOT);
            } else {
                frame.arc(centre, DOT - 0.5, 0.0, 360.0, theme.text3, 1.0);
            }
            let said = match lang {
                Lang::Zh => format!("{at_work} 个会话进行中"),
                Lang::En => format!("{at_work} {} at work", if *at_work == 1 { "session" } else { "sessions" }),
            };
            part("at_work");
            frame.text(&said, theme.small, theme.text2, left + 2.0 * DOT + 8.0, row_y, width - 2.0 * DOT - 8.0, Align::Start);
            let all = match lang {
                Lang::Zh => format!("今日共 {today} 个"),
                Lang::En => format!("{today} today"),
            };
            part("today");
            frame.text(&all, theme.small, theme.text3, left, row_y, width, Align::End);
        }
        // (Drawn by `paint_shaped`, in all the room the widget gives it.)
        AiBlock::Shaped { .. } => {}
    }
}

/// The whole component in a widget too small for all of it, filling `room` (DIPs) on the widget's plate, in whichever
/// of the forms it may take there tells the most (see `Form`): what each source spent by model first (the owner's
/// choice), then the most it says. Each form tells more the larger its room, so the larger a widget is made, the more
/// it tells, never less. (Each form was picked by how wide the widget was for how tall: a little narrower than the
/// dock's strip, a widget wide and low showed today's figure alone, and as near a square it showed less than the strip
/// it was a little lower than, as the owner saw.) Where two tell as much, the one nearer the widget's shape. On the
/// dock's forms and as near a square, where the pointer rests to show a spot's card beside it.
pub(super) fn paint_shaped(frame: &dyn Canvas, scene: &Scene, ink: Ink, told: (&Dock, &[f32], &Rich), room: (f32, f32, f32, f32), hits: &mut Vec<HitBox>) {
    let (_, _, w, h) = room;
    let squares = (0..LEAST_PLANS).flat_map(|plan| [Form::Square { plan, listing: false }, Form::Square { plan, listing: true }]);
    let forms: Vec<Form> = if w >= h {
        [Form::Across, Form::Board].into_iter().chain(squares).chain([Form::Column, Form::Down, Form::Today]).collect()
    } else {
        [Form::Down, Form::Column].into_iter().chain(squares).chain([Form::Board, Form::Across, Form::Today]).collect()
    };
    let tally = Tally::new(frame);
    let mut best: Option<(Form, (bool, usize))> = None;
    for form in forms {
        tally.said.set(0);
        let Some(models) = paint_form(form, &tally, scene, ink, told, room, &mut Vec::new()) else { continue };
        let says = (models, tally.said.get());
        if best.is_none_or(|(_, most)| says > most) {
            best = Some((form, says));
        }
    }
    // No form holding every source (one figure alone fits): as many sources as fit whole, the rest turned to by
    // the wheel (see `paint_paged`). (A widget only a little smaller than one holding them all showed today's figure
    // alone, as the owner saw.)
    if best.is_none_or(|(form, _)| form == Form::Today) && paint_paged(frame, scene, told, room, hits) {
        return;
    }
    if let Some((form, _)) = best {
        paint_form(form, frame, scene, ink, told, room, hits);
    }
}

/// Laid out a page at a time (see `paint_paged`): how wide a source's part is at least, what every source spent beside
/// the parts and the room between them, and the room the dots that tell the page take at the right (DIPs).
const PAGED_PART: f32 = 200.0;
const PAGED_TODAY: f32 = 92.0;
const PAGED_GAP: f32 = 14.0;
const PAGER: f32 = 12.0;

/// How much a source's part tells laid out a page at a time, as there is room down: its head and its limits (or what
/// it has and spent), then when each window starts again, then what it spent by model.
const PAGED_LEVELS: usize = 3;

/// The least a window is wide, its windows side by side in one row (see `AiBlock::WindowsRow`).
const ROW_WINDOW_LEAST: f32 = 64.0;

/// `part` (a source's) as a lane `width` wide telling as much as `level` (see `PAGED_LEVELS`): telling least, its
/// windows in one row where they fit so, two to a row where they do not.
fn paged_lane(part: &Part, theme: &Theme, width: f32, level: usize) -> Lane {
    let mut lane = part_lane(part, theme, level >= 2);
    if level == 0 {
        for block in &mut lane.blocks {
            if let Block::Ai(AiBlock::Windows(windows)) = block {
                windows.iter_mut().for_each(|window| window.resets.clear());
                let across = windows.len() as f32;
                if windows.len() > 2 && (width - (across - 1.0) * WINDOW_GAP) / across >= ROW_WINDOW_LEAST {
                    *block = Block::Ai(AiBlock::WindowsRow(std::mem::take(windows)));
                }
            }
        }
    }
    lane
}

/// The id of the page what every source spent today is, first of the pages the smallest form turns round (see
/// `paint_one` and `view::keep_source`).
pub const TODAY_PAGE: &str = "ai:today";

/// The pages `sources` (by their ids, in their order) are turned round as, what every source spent today before them
/// where that is one, as last drawn (`hits`); not drawn yet, where `shown` was kept on it. None where they are not
/// turned round: all of them shown, the one kept to be shown first is left as it is.
pub fn pages(hits: &[HitBox], shown: &super::SourceShown, sources: Vec<String>) -> Option<Vec<String>> {
    let turned = hits.iter().find_map(|hit| if let Hit::AiPages(_, today_first) = hit.4 { Some(today_first) } else { None });
    let today_first = match turned {
        Some(today_first) => today_first,
        None if hits.is_empty() => shown.id() == Some(TODAY_PAGE),
        None => return None,
    };
    (!sources.is_empty()).then(|| today_first.then(|| TODAY_PAGE.to_string()).into_iter().chain(sources).collect())
}

/// The component in a widget too small for any form that holds every source (see `paint_shaped`), filling `room`: what
/// every source spent today and the sessions at work at the left where there is room beside a source's part, then as
/// many sources' parts side by side as fit whole (at least `PAGED_PART` wide), each telling as much as it has room for
/// down (see `PAGED_LEVELS`); the rest turned round to by the wheel, a source a notch (`List::AiSources`: past the
/// last, the first again), one under another a part alone, else side by side, and dots at the right telling which are
/// shown of how many, a press on one turning to it. A part's head shows the source's card beside the widget as the
/// pointer rests on it, as the dock's spot does (a part tells no more than its limits). Whether it fits (else nothing
/// drawn): a source's part, as little as it tells.
pub(super) fn paint_paged(frame: &dyn Canvas, scene: &Scene, (dock, _, rich): (&Dock, &[f32], &Rich), (x, y, w, h): (f32, f32, f32, f32), hits: &mut Vec<HitBox>) -> bool {
    let (theme, lang) = (scene.theme, scene.lang);
    let parts: Vec<(&Slot, &Part)> = dock.slots.iter().filter_map(|slot| rich.parts.iter().find(|part| part.id == slot.source.id()).map(|part| (slot, part))).collect();
    let count = parts.len();
    // (Inset as the least forms are: see `paint_least`.)
    let (inset_x, inset_y) = (14.0_f32.min(w / 6.0), 14.0_f32.min(h / 6.0));
    let (left, top, width, height) = (x + inset_x, y + inset_y, (w - 2.0 * inset_x).max(0.0), (h - 2.0 * inset_y).max(0.0));
    if count == 0 || width < PAGED_PART {
        return false;
    }
    let tall = |lane: &Lane| lane.blocks.iter().map(Block::height).sum::<f32>();
    // Laid out with `per` parts side by side, what every source spent beside them or not: the room the parts take (the
    // dots' taken where some are turned to), each part's width, and each part as much as it has room for down; none
    // where a part is narrower than `PAGED_PART` or fits not even as little as it tells.
    let laid = |per: usize, today: bool| -> Option<(f32, f32, Vec<Lane>)> {
        let room = width - if today { PAGED_TODAY + PAGED_GAP } else { 0.0 } - if per < count { PAGER } else { 0.0 };
        let each = (room - (per as f32 - 1.0) * PAGED_GAP) / per as f32;
        if each < PAGED_PART {
            return None;
        }
        let lanes = parts.iter().map(|(_, part)| (0..PAGED_LEVELS).rev().map(|level| paged_lane(part, theme, each, level)).find(|lane| tall(lane) <= height)).collect::<Option<Vec<Lane>>>()?;
        Some((room, each, lanes))
    };
    // As many side by side as fit, then what every source spent beside them where it fits too. (So, made wider, it
    // shows no fewer: what fits narrower fits wider.)
    let today_shown = dock.today.is_some();
    let Some((per, today, (room, each, lanes))) = (1..=count).rev().flat_map(|per| [(per, true), (per, false)]).filter(|(_, today)| today_shown || !today).find_map(|(per, today)| Some((per, today, laid(per, today)?))) else { return false };
    let paged = per < count;
    let parts_left = left + if today { PAGED_TODAY + PAGED_GAP } else { 0.0 };
    // What every source spent today, and the sessions at work under it, in the middle down.
    if today {
        let label = Font { size: 11.0, weight: 500.0, ..theme.body };
        let figure = Font { size: 20.0, weight: 650.0, ..theme.figure };
        let working = dock.working > 0;
        let column = label.size + 4.0 + figure.size + if working { 6.0 + LINE } else { 0.0 };
        let mut row = top + (height - column) / 2.0;
        if scene.hover == Some(Hit::AiSlot(Spot::Today)) {
            frame.fill_rounded(theme.hover, left - 6.0, row - 4.0, PAGED_TODAY + 12.0, label.size + 4.0 + figure.size + 8.0, 10.0);
        }
        hits.push((left, row, PAGED_TODAY, label.size + 4.0 + figure.size, Hit::AiSlot(Spot::Today)));
        frame.key("ai:today.label");
        frame.text(lang.pick("今日", "Today"), label, theme.text3, left, row, PAGED_TODAY, Align::Start);
        row += label.size + 4.0;
        frame.key("ai:today.value");
        frame.text(dock.today.as_deref().unwrap_or_default(), figure, theme.text, left, row, PAGED_TODAY, Align::Start);
        row += figure.size + 6.0;
        if working {
            frame.key("ai:dock.at_work");
            frame.fill_circle(theme.ink(AI_LANE).trace, Point { x: left + 3.5, y: row + LINE / 2.0 }, 3.5);
            let said = match lang {
                Lang::Zh => format!("{} 个会话", dock.working),
                Lang::En => format!("{} at work", dock.working),
            };
            frame.text(&said, theme.small, theme.text2, left + 13.0, row, PAGED_TODAY - 13.0, Align::Start);
        }
    }
    // The parts, turned round to where the wheel turned them: a part alone one under another, else side by side.
    let at = if paged { scene.scrolls.at(List::AiSources) / PAGE } else { 0.0 };
    let step = if per == 1 { height } else { each + PAGED_GAP };
    let reach = if per == 1 { height } else { room };
    let round = count as f32;
    frame.clip(parts_left, top, room, height);
    for (n, (lane, (slot, _))) in lanes.iter().zip(&parts).enumerate() {
        // (Each where it comes round to: past the last, the first again, a turn on.)
        let turns = ((at - n as f32 - 1.0) / round).ceil() as i32..=((at - n as f32 + per as f32) / round).floor() as i32;
        for turn in turns {
            let off = (n as f32 + turn as f32 * round - at) * step;
            // (Not drawn where less than half a DIP of it would show: the wheel's slide settles within a twentieth of one.)
            if off <= 0.5 - step || off >= reach - 0.5 {
                continue;
            }
            let lane_tall = tall(lane);
            let (px, py) = if per == 1 { (parts_left, top + off + (height - lane_tall) / 2.0) } else { (parts_left + off, top + (height - lane_tall) / 2.0) };
            // Its head (the source's mark and name): its card beside the widget as the pointer rests there.
            if let Some(Block::Ai(AiBlock::Head { title, .. })) = lane.blocks.first() {
                let spot = Spot::Ring(slot.source);
                let named = (HEAD_MARK + frame.measure(title, theme.title)).min(each);
                if scene.hover == Some(Hit::AiSlot(spot)) {
                    frame.fill_rounded(theme.hover, px - 5.0, py - 3.0, named + 10.0, HEAD + 6.0, 6.0);
                }
                hits.push((px, py, named, HEAD, Hit::AiSlot(spot)));
            }
            let area = Rect { x: px - theme.pad_x, y: py - theme.pad_top, w: each + 2.0 * theme.pad_x, h: lane_tall + theme.pad_top + theme.pad_bottom };
            paint_lane(frame, scene, lane, area, 0.0, Pass::Content, hits);
        }
    }
    frame.unclip();
    if paged {
        // Which are shown, of how many.
        pager_dots(frame, scene, (left + width - PAGER / 2.0, top, height), count, (round_page(at, count), per), hits);
        hits.push((x, y, w, h, Hit::AiPages(count as u32, false)));
    }
    true
}

/// The page shown first of `count` turned round, turned to `at` pages (see `Scrolls::fold`: within a turn or not).
fn round_page(at: f32, count: usize) -> usize {
    (at.round() as i64).rem_euclid(count.max(1) as i64) as usize
}

/// A form the component may take in a widget too small for all of it (see `paint_shaped`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Form {
    /// The dock's strip across, its slots narrower than designed where it is a little narrower, wider and telling more
    /// where it is wider.
    Across,
    /// The dock's strip down, each slot telling each of its windows where it is taller.
    Down,
    /// As near a square: today's figure over each source's ring, roomier (`plan`, see `LEAST_PLANS`) with the head, how
    /// today's spending mounted, what each figure is of and when it starts again, and the sessions at work; `listing`,
    /// what each source spent by model under its ring, then its sessions.
    Square { plan: usize, listing: bool },
    /// Its parts side by side, as from a top edge, then what each spent by model, then its sessions.
    Board,
    /// Its parts one over another, as its lane is, then what each spent by model, then its sessions.
    Column,
    /// One figure alone: today's, where it is shown, else a source's, the rest turned to by the wheel (see `paint_one`).
    Today,
}

/// Draws the component in `form` in `room`: whether it tells what any source spent by model; none where it does not
/// fit (nothing drawn).
fn paint_form(form: Form, frame: &dyn Canvas, scene: &Scene, ink: Ink, (dock, mounted, rich): (&Dock, &[f32], &Rich), room: (f32, f32, f32, f32), hits: &mut Vec<HitBox>) -> Option<bool> {
    let (theme, lang) = (scene.theme, scene.lang);
    let (_, _, w, h) = room;
    fn models<'a>(mut blocks: impl Iterator<Item = &'a Block>) -> bool {
        blocks.any(|block| matches!(block, Block::Ai(AiBlock::Models(_))))
    }
    match form {
        Form::Across => {
            (w >= strip_least(frame, theme, dock) && h >= BAR_HEIGHT).then(|| hits.extend(draw_dock(frame, scene, dock, true, room)))?;
            Some(false)
        }
        Form::Down => {
            let (dw, dh) = dock_size(false, dock.slots.len(), dock.today.is_some());
            (w >= dw * 0.8 && h >= dh).then(|| hits.extend(draw_dock(frame, scene, dock, false, room)))?;
            Some(false)
        }
        Form::Square { plan, listing } => paint_least(frame, scene, ink, (dock, mounted, rich), room, (plan, listing), hits),
        Form::Board => {
            let rows = board_of(rich, theme, lang, (w, h))?;
            let told = models(rows.iter().flatten().flat_map(|lane| &lane.blocks));
            draw_board(frame, scene, &rows, room, hits);
            Some(told)
        }
        Form::Column => {
            (w - 2.0 * theme.pad_x >= BOARD_COLUMN_LEAST).then_some(())?;
            let lane = column_of(rich, theme, lang, (w, h))?;
            let told = models(lane.blocks.iter());
            draw_column(frame, scene, ink, lane, room, hits);
            Some(told)
        }
        Form::Today => {
            paint_one(frame, scene, dock, room, hits);
            Some(false)
        }
    }
}

/// A canvas that draws nothing and counts the texts it is asked to draw (none empty), measuring as `frame` does, each
/// text once: for how much a form tells (see `paint_shaped`).
struct Tally<'a> {
    frame: &'a dyn Canvas,
    said: std::cell::Cell<usize>,
    measured: std::cell::RefCell<std::collections::HashMap<(String, crate::ui::canvas::FontKey), f32>>,
}

impl<'a> Tally<'a> {
    fn new(frame: &'a dyn Canvas) -> Self {
        Tally { frame, said: std::cell::Cell::new(0), measured: Default::default() }
    }
}

impl Canvas for Tally<'_> {
    fn text(&self, text: &str, _: Font, _: Color, _: f32, _: f32, _: f32, _: Align) {
        if !text.trim().is_empty() {
            self.said.set(self.said.get() + 1);
        }
    }
    fn markdown(&self, _: &str, _: Font, _: crate::ui::canvas::Inks, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn markdown_tall(&self, text: &str, font: Font, width: f32) -> f32 {
        self.frame.markdown_tall(text, font, width)
    }
    fn measure(&self, text: &str, font: Font) -> f32 {
        *self.measured.borrow_mut().entry((text.to_string(), font.key())).or_insert_with(|| self.frame.measure(text, font))
    }
    fn baseline(&self, font: Font) -> (f32, f32) {
        self.frame.baseline(font)
    }
    fn ink(&self, text: &str, font: Font) -> (f32, f32) {
        self.frame.ink(text, font)
    }
    fn clip(&self, _: f32, _: f32, _: f32, _: f32) {}
    fn unclip(&self) {}
    fn fill(&self, _: Color, _: f32, _: f32, _: f32, _: f32) {}
    fn fill_rounded(&self, _: Color, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn fill_circle(&self, _: Color, _: Point, _: f32) {}
    fn stroke(&self, _: &[Point], _: Color, _: f32) {}
    fn stroke_dashed(&self, _: &[Point], _: Color, _: f32) {}
    fn fill_shape(&self, _: &[Point], _: Fill) {}
    fn stroke_rounded(&self, _: Fill, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn shadow(&self, _: Color, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn frosted(&self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32, _: &dyn Fn(&dyn Canvas)) {}
    fn arc(&self, _: Point, _: f32, _: f32, _: f32, _: Color, _: f32) {}
    fn icon(&self, _: crate::ui::icons::Icon, _: Point, _: f32, _: Color) {}
    fn mark(&self, _: crate::ui::marks::Mark, _: Point, _: f32, _: Color, _: f32) {}
    fn picture_within(&self, _: &crate::ui::canvas::Picture, _: (f32, f32, f32, f32), _: (f32, f32, f32, f32, f32), _: f32) {}
    fn pixels_per_dip(&self) -> f32 {
        self.frame.pixels_per_dip()
    }
}

/// How wide each part side by side is at least to be given a row of fewer, and at least (DIPs, inside its padding).
const BOARD_COLUMN: f32 = 220.0;
const BOARD_COLUMN_LEAST: f32 = 160.0;

/// `part` as a lane: what it shows, and what it spent by model where `models`.
fn part_lane(part: &Part, theme: &Theme, models: bool) -> Lane {
    let mut blocks: Vec<Block> = part.shows.iter().map(Told::block).collect();
    if models {
        blocks.extend(part.models.clone().map(Block::Ai));
    }
    Lane { id: part.id.clone(), blocks, ink: theme.ink(&part.id) }
}

/// `sessions` in `columns` side by side, as many in each, the first under the head, each with its source's mark.
fn sessions_lanes(sessions: &[SessionRow], columns: usize, theme: &Theme, lang: Lang) -> Vec<Lane> {
    let titled = || head(lang.pick("会话", "Sessions"), "", "", false);
    let above = titled().height();
    let each = sessions.len().div_ceil(columns.max(1)).max(1);
    sessions
        .chunks(each)
        .enumerate()
        .map(|(at, rows)| Lane { id: format!("ai:sessions.{at}"), blocks: vec![if at == 0 { titled() } else { Block::Room(above) }, Block::Ai(AiBlock::Sessions(rows.to_vec()))], ink: theme.ink("ai:sessions") })
        .collect()
}

/// `rich`'s parts in rows of `per`, the sessions at work under the first, each source with what it spent by model where
/// `models`, and the first `sessions` of its sessions in a row under them, in as many columns.
fn board_rows(rich: &Rich, theme: &Theme, lang: Lang, per: usize, models: bool, sessions: usize) -> Vec<Vec<Lane>> {
    let mut parts: Vec<Lane> = rich.parts.iter().map(|part| part_lane(part, theme, models)).collect();
    if let Some(working) = &rich.working {
        match parts.first_mut() {
            Some(first) => first.blocks.push(Block::Ai(working.clone())),
            None => parts.push(Lane { id: "ai:sessions".into(), blocks: vec![Block::Ai(working.clone())], ink: theme.ink("ai:sessions") }),
        }
    }
    let mut rows = Vec::new();
    let mut parts = parts.into_iter().peekable();
    while parts.peek().is_some() {
        rows.push(parts.by_ref().take(per).collect());
    }
    if sessions > 0 {
        rows.push(sessions_lanes(&rich.sessions[..sessions], per, theme, lang));
    }
    rows
}

/// How tall `rows` of parts side by side are (see `paint_across`).
fn rows_tall(rows: &[Vec<Lane>]) -> f32 {
    rows.iter().map(|row| across_tall(row)).sum::<f32>() + rows.len().saturating_sub(1) as f32 * ACROSS_ROW_GAP
}

/// `rich` laid out side by side in a widget `room` large (DIPs): its parts in rows of as many as leave each the least
/// room, as far as they fit what each source spent by model, and after that its sessions, a row of them at a time; in
/// as many to a row as tell the most so (each count to a row it may be given as it is made wider: the fewest to a row
/// that left each its room took over from all in one, and might tell less), else as few rows as leave each its room,
/// else as few as fit. None where its parts do not fit in any, or it has none (drawn empty).
pub(super) fn board_of(rich: &Rich, theme: &Theme, lang: Lang, (w, h): (f32, f32)) -> Option<Vec<Vec<Lane>>> {
    if rich.parts.is_empty() {
        return None;
    }
    let count = rich.parts.len();
    let (width, room) = (w - 2.0 * theme.pad_x, h - theme.pad_top - theme.pad_bottom);
    let column = |per: usize| (width - (per as f32 - 1.0) * ACROSS_GAP) / per as f32;
    let fits = |per: usize, models: bool, sessions: usize| rows_tall(&board_rows(rich, theme, lang, per, models, sessions)) <= room;
    let told = |per: usize| -> Option<(bool, usize)> {
        fits(per, false, 0).then_some(())?;
        if !fits(per, true, 0) {
            return Some((false, 0));
        }
        let mut sessions = 0;
        while sessions < rich.sessions.len() {
            let more = ((sessions / per + 1) * per).min(rich.sessions.len());
            if !fits(per, true, more) {
                break;
            }
            sessions = more;
        }
        Some((true, sessions))
    };
    let (per, (models, sessions)) = (1..=count)
        .filter(|&per| column(per) >= BOARD_COLUMN_LEAST)
        .filter_map(|per| Some((per, told(per)?)))
        .max_by_key(|&(per, told)| (told, column(per) >= BOARD_COLUMN, per))?;
    Some(board_rows(rich, theme, lang, per, models, sessions))
}

/// Draws `rows` of parts side by side (see `board_of`) in the middle of `room`.
fn draw_board(frame: &dyn Canvas, scene: &Scene, rows: &[Vec<Lane>], (x, y, w, h): (f32, f32, f32, f32), hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    let room = h - theme.pad_top - theme.pad_bottom;
    paint_across(frame, scene, rows, (x + theme.pad_x, y + theme.pad_top + (room - rows_tall(rows)) / 2.0, w - 2.0 * theme.pad_x), Pass::Content, hits);
}

/// `rich`'s parts one over another, each source with what it spent by model where `models`, the sessions at work, and the
/// first `sessions` of its sessions.
fn column_lane(rich: &Rich, theme: &Theme, lang: Lang, models: bool, sessions: usize) -> Lane {
    let mut blocks = Vec::new();
    for part in &rich.parts {
        if !blocks.is_empty() {
            blocks.push(Block::Room(SOURCE_GAP));
        }
        blocks.extend(part_lane(part, theme, models).blocks);
    }
    blocks.extend(rich.working.clone().map(Block::Ai));
    if sessions > 0 {
        blocks.push(Block::Room(SOURCE_GAP));
        blocks.extend(sessions_lanes(&rich.sessions[..sessions], 1, theme, lang).into_iter().flat_map(|lane| lane.blocks));
    }
    Lane { id: AI_LANE.into(), blocks, ink: theme.ink(AI_LANE) }
}

/// `rich` one over another in a widget `room` large (DIPs): its parts, then, as far as they fit, what each source spent by
/// model, and after that its sessions. None where its parts do not fit, or it has none (drawn empty).
pub(super) fn column_of(rich: &Rich, theme: &Theme, lang: Lang, (_, h): (f32, f32)) -> Option<Lane> {
    if rich.parts.is_empty() {
        return None;
    }
    let fits = |lane: &Lane| lane.height(theme) <= h;
    if !fits(&column_lane(rich, theme, lang, false, 0)) {
        return None;
    }
    if !fits(&column_lane(rich, theme, lang, true, 0)) {
        return Some(column_lane(rich, theme, lang, false, 0));
    }
    let sessions = (1..=rich.sessions.len()).take_while(|&sessions| fits(&column_lane(rich, theme, lang, true, sessions))).last().unwrap_or(0);
    Some(column_lane(rich, theme, lang, true, sessions))
}

/// Draws `lane`, parts one over another (see `column_of`), in the middle of `room`, in `ink`.
fn draw_column(frame: &dyn Canvas, scene: &Scene, ink: Ink, mut lane: Lane, (x, y, w, h): (f32, f32, f32, f32), hits: &mut Vec<HitBox>) {
    lane.ink = ink;
    let tall = lane.height(scene.theme);
    paint_lane(frame, scene, &lane, Rect { x, y: y + (h - tall) / 2.0, w, h: tall }, 0.0, Pass::Content, hits);
}

/// Between the rings as near a square and what is listed under them, and between the lists (DIPs).
const LIST_ABOVE: f32 = 18.0;

/// The least a ring is drawn as near a square (DIPs).
const LEAST_RING: f32 = 26.0;

/// How many ways the component may lay out as near a square (see `paint_least`): roomier with when each window starts
/// again and the sessions at work at its foot; with the sessions at work; roomier alone; and least, today's figure over
/// the rings.
const LEAST_PLANS: usize = 4;

/// The component as near a square as its widget is, in `room`, laid out as `plan` (see `LEAST_PLANS`): today's figure;
/// each source's ring round its mark and its figure under it, side by side; and, roomier, the head, how today's
/// spending mounted, what each figure is of, when its window starts again, and the sessions at work. `listing`, its
/// rings in a row as large as they are drawn, and under each, as there is room, what its source spent by model, and
/// after that as many of its sessions as there is room for, in two columns where it is wide (the sessions at work at its
/// foot giving way to them): what was spent by model before the sessions, as in every form. Whether it tells what any
/// source spent by model; none where it does not fit (nothing drawn): its rings, or the room `plan` is for, or what it
/// is to tell (where there is none, laid out as another plan is).
pub(super) fn paint_least(frame: &dyn Canvas, scene: &Scene, ink: Ink, (dock, mounted, rich): (&Dock, &[f32], &Rich), (x, y, w, h): (f32, f32, f32, f32), (plan, listing): (usize, bool), hits: &mut Vec<HitBox>) -> Option<bool> {
    let (theme, lang) = (scene.theme, scene.lang);
    let label = Font { size: 11.0, weight: 500.0, ..theme.body };
    let count = dock.slots.len();
    let (roomy, resets, foot) = [(true, true, true), (true, false, true), (true, false, false), (false, false, false)][plan];
    let told_resets = dock.slots.iter().any(|slot| !slot.resets.is_empty());
    if count == 0 || (roomy && (w < LEAST_ROOMY.0 || h < LEAST_ROOMY.1)) || (resets && !told_resets) || (foot && dock.working == 0) || (listing && !roomy) {
        return None;
    }
    // (Its inset across by its width, down by its height: down by its width, it had less room down the wider it was.)
    let (inset_x, inset_y) = if roomy { (theme.pad_x.min(24.0), theme.pad_x.min(24.0)) } else { (14.0_f32.min(w / 6.0), 14.0_f32.min(h / 6.0)) };
    let (left, top, width, height) = (x + inset_x, y + inset_y, (w - 2.0 * inset_x).max(0.0), (h - 2.0 * inset_y).max(0.0));
    // Today's figure as large as its room across lets it be, in the room it has as designed (made smaller where it is
    // narrower, it gave the rings room they did not have where it was wider).
    let figure_font = Font { size: if roomy { 30.0 } else { (height * 0.2).clamp(15.0, 24.0) }, weight: 650.0, ..theme.figure };
    let today_tall = if dock.today.is_some() { label.size + 4.0 + figure_font.size + 12.0 } else { 0.0 };
    let natural = dock.today.as_deref().map_or(0.0, |today| frame.measure(today, figure_font));
    let figure_font = if natural > width && natural > 0.0 { Font { size: (figure_font.size * width / natural).max(11.0), ..figure_font } } else { figure_font };
    let rings_top = top + if roomy { HEAD + 4.0 } else { 0.0 } + today_tall;
    let under = if roomy { 36.0 + if resets { 14.0 } else { 0.0 } } else { 18.0 };
    let foot = if foot { LINE + 10.0 } else { 0.0 };
    let most = if roomy { 56.0 } else { 44.0 };
    let below = top + height - foot - rings_top;
    // In as many columns as leave the rings the largest (the most of them, as large in more), each with its figure under
    // it; listing, in a row, as large as they are drawn (so what is listed under them has as much room the wider it is).
    let (columns, cell, ring_room) = if listing {
        let cell = (width - (count as f32 - 1.0) * 8.0) / count as f32;
        (count, cell, if cell >= most && below >= most + under { most } else { 0.0 })
    } else {
        (1..=count)
            .map(|columns| {
                let rows = count.div_ceil(columns) as f32;
                let cell = (width - (columns as f32 - 1.0) * 8.0) / columns as f32;
                (columns, cell, ((below - (rows - 1.0) * 8.0) / rows - under).min(cell).min(most))
            })
            .fold((1, 0.0, f32::MIN), |best, this| if this.2 >= best.2 { this } else { best })
    };
    if ring_room < LEAST_RING {
        return None;
    }
    // Each spot the pointer rests on to show its card, as on the dock.
    let lit = |spot: Spot| scene.hover == Some(Hit::AiSlot(spot));
    let mut spot = |spot: Spot, (sx, sy, sw, sh): (f32, f32, f32, f32)| {
        if lit(spot) {
            frame.fill_rounded(theme.hover, sx - 4.0, sy - 4.0, sw + 8.0, sh + 8.0, 12.0);
        }
        hits.push((sx, sy, sw, sh, Hit::AiSlot(spot)));
    };
    // What today's figure is of, in as many words as fit.
    let of = [lang.pick("今日 tokens", "Tokens today"), lang.pick("今日", "Today")].into_iter().find(|of| frame.measure(of, label) <= width).unwrap_or(lang.pick("今日", "Today"));
    let rows = count.div_ceil(columns) as f32;
    let rings_tall = rows * (ring_room + under) + (rows - 1.0) * 8.0;
    // Listing, what each source spent by model, under its ring, where there is room for all of it; the sessions after
    // it, where there is room for them.
    let models: Vec<Option<Lane>> = dock
        .slots
        .iter()
        .map(|slot| {
            let part = rich.parts.iter().find(|part| part.id == slot.source.id())?;
            Some(Lane { id: part.id.clone(), blocks: vec![Block::Ai(part.models.clone()?)], ink: theme.ink(&part.id) })
        })
        .collect();
    let models_tall = models.iter().flatten().map(|lane| lane.height(theme)).fold(0.0, f32::max);
    let any_models = models.iter().any(Option::is_some);
    let models_shown = listing && any_models && models_tall <= below - rings_tall - LIST_ABOVE;
    let list_columns = if width >= 2.0 * BOARD_COLUMN { 2 } else { 1 };
    let list_room = below + foot - rings_tall - LIST_ABOVE - if models_shown { models_tall + LIST_ABOVE } else { 0.0 };
    let sessions = &rich.sessions;
    let listed = if listing && (models_shown || !any_models) {
        (1..=sessions.len()).take_while(|&shown| across_tall(&sessions_lanes(&sessions[..shown], list_columns, theme, lang)) <= list_room).last().unwrap_or(0)
    } else {
        0
    };
    if listing && !models_shown && listed == 0 {
        return None;
    }
    // (In the middle of the room under the figure, where nothing is under them.)
    let ring_from = if listing { rings_top } else { rings_top + ((below - rings_tall) / 2.0).max(0.0) };
    let mut row = top;
    if roomy {
        let row_y = row + (HEAD - theme.title.size) / 2.0 - 1.0;
        frame.key("ai:head");
        frame.text(lang.pick("AI 用量", "AI usage"), theme.title, theme.text, left, row_y, width, Align::Start);
        if dock.today.is_some() {
            frame.key("ai:today.aside");
            frame.text(lang.pick("今日", "Today"), theme.small, theme.text3, left, row_y + 1.0, width, Align::End);
        }
        row += HEAD + 4.0;
    } else if dock.today.is_some() {
        frame.key("ai:today.label");
        // (Over one column of rings, in the middle as they are.)
        if columns == 1 {
            centred(frame, of, label, theme.text3, left + width / 2.0, row, width);
        } else {
            frame.text(of, label, theme.text3, left, row, width, Align::Start);
        }
        row += label.size + 4.0;
    }
    // Today's figure, and how it mounted after it, where it is shown.
    let figure_width = if let Some(today) = &dock.today {
        spot(Spot::Today, (left, top, width, rings_top - 12.0 - top));
        frame.key("ai:today.value");
        let figure_top = if roomy { row + label.size + 4.0 - figure_font.size * 0.25 } else { row };
        if columns == 1 && !roomy {
            centred(frame, today, figure_font, theme.text, left + width / 2.0, figure_top, width);
        } else {
            frame.text(today, figure_font, theme.text, left, figure_top, width, Align::Start);
        }
        Some(frame.measure(today, figure_font))
    } else {
        None
    };
    let spark_left = left + figure_width.unwrap_or(0.0) + 16.0;
    if figure_width.is_some() && roomy && mounted.len() >= 2 && left + width - spark_left >= 40.0 {
        let (spark_top, spark_foot) = (row + 6.0, rings_top - 16.0);
        let spark_width = left + width - spark_left;
        let points: Vec<Point> = mounted
            .iter()
            .enumerate()
            .map(|(at, share)| Point { x: spark_left + at as f32 / (mounted.len() - 1) as f32 * spark_width, y: spark_foot - share.clamp(0.0, 1.0) * (spark_foot - spark_top) })
            .collect();
        let mut area = points.clone();
        area.push(Point { x: spark_left + spark_width, y: spark_foot });
        area.push(Point { x: spark_left, y: spark_foot });
        frame.key("ai:today.chart");
        frame.fill_shape(&area, if ink.wash == ink.wash_end { Fill::Solid(ink.wash) } else { Fill::Down { top: spark_top, from: ink.wash, bottom: spark_foot, to: ink.wash_end } });
        frame.stroke(&points, ink.trace, 1.6);
        if let Some(last) = points.last() {
            frame.fill_circle(ink.trace, *last, 2.4);
        }
    }
    // Each source's ring in its cell, its figure (and, roomier, what it is of and when it starts again) under it.
    let figure = Font { size: if roomy { 15.0 } else { 12.0 }, weight: 650.0, ..theme.figure };
    let under_font = Font { size: 10.5, weight: 500.0, ..theme.body };
    let radius = ring_room / 2.0 - 2.0;
    for (at, slot) in dock.slots.iter().enumerate() {
        let (cell_left, cell_top) = (left + (at % columns) as f32 * (cell + 8.0), ring_from + (at / columns) as f32 * (ring_room + under + 8.0));
        let centre = Point { x: cell_left + cell / 2.0, y: cell_top + ring_room / 2.0 };
        spot(Spot::Ring(slot.source), (cell_left, cell_top, cell, ring_room + under));
        ring(frame, theme, slot, centre, radius, if roomy { 4.0 } else { 3.0 }, radius * 0.9);
        frame.key(&format!("{}.value", slot.source.id()));
        centred(frame, &slot.figure, figure, figure_ink(theme, slot), centre.x, cell_top + ring_room + 4.0, cell);
        if roomy {
            let label_top = cell_top + ring_room + 4.0 + figure.size + 3.0;
            frame.key(&format!("{}.label", slot.source.id()));
            centred(frame, &slot.label, under_font, theme.text3, centre.x, label_top, cell);
            if resets {
                frame.key(&format!("{}.resets", slot.source.id()));
                centred(frame, &slot.resets, under_font, theme.text3, centre.x, label_top + under_font.size + 3.0, cell);
            }
        }
    }
    // The sessions at work, at its foot, where no sessions are listed.
    if foot > 0.0 && listed == 0 {
        let row_y = top + height - LINE;
        spot(Spot::Sessions, (left, row_y, width, LINE));
        frame.key("ai:dock.at_work");
        frame.fill_circle(ink.trace, Point { x: left + 3.5, y: row_y + LINE / 2.0 }, 3.5);
        let said = match lang {
            Lang::Zh => format!("{} 个会话进行中", dock.working),
            Lang::En => format!("{} at work", dock.working),
        };
        frame.text(&said, theme.small, theme.text2, left + 15.0, row_y, width - 15.0, Align::Start);
    }
    let mut list_top = ring_from + rings_tall + LIST_ABOVE;
    if models_shown {
        for (at, lane) in models.iter().enumerate() {
            let Some(lane) = lane else { continue };
            let cell_left = left + (at % columns) as f32 * (cell + 8.0);
            paint_lane(frame, scene, lane, Rect { x: cell_left + 4.0, y: list_top, w: cell - 8.0, h: models_tall }, 0.0, Pass::Content, hits);
        }
        list_top += models_tall + LIST_ABOVE;
    }
    if listed > 0 {
        paint_across(frame, scene, &[sessions_lanes(&sessions[..listed], list_columns, theme, lang)], (left, list_top, width), Pass::Content, hits);
    }
    Some(models_shown)
}

/// One figure alone in the middle of `room`, too small for anything more, and what it is of over it where it is tall
/// enough: what every source spent today first, where it is shown, then each source's (its tightest window's share
/// used, or what it has), turned round to by the wheel, a figure a notch (`List::AiSources`, as in `paint_paged`: past
/// the last, the first again), with dots at the right telling which is shown of how many where there is room for them
/// beside it, a press on one turning to it.
fn paint_one(frame: &dyn Canvas, scene: &Scene, dock: &Dock, (x, y, w, h): (f32, f32, f32, f32), hits: &mut Vec<HitBox>) {
    let (theme, lang) = (scene.theme, scene.lang);
    let label = Font { size: 11.0, weight: 500.0, ..theme.body };
    let inset = 14.0_f32.min(w / 6.0);
    let today_first = dock.today.is_some();
    let count = dock.slots.len() + usize::from(today_first);
    let paged = count > 1;
    // (The dots' room at the right, where there is any beside the figure.)
    let pager = if paged && w - 2.0 * inset >= 4.0 * PAGER && pager_fits(count, 1, (h - 2.0 * inset).max(0.0)) { PAGER } else { 0.0 };
    let (left, width) = (x + inset, (w - 2.0 * inset - pager).max(0.0));
    let font = Font { size: (w * 0.26).clamp(11.0, 22.0), weight: 650.0, ..theme.figure };
    let tall = (label.size + 4.0 + font.size).min((h - 2.0 * inset).max(0.0));
    let middle = y + h / 2.0;
    let first = if paged { round_page(scene.scrolls.at(List::AiSources) / PAGE, count) } else { 0 };
    // (Today's the first page, where it is shown: a source's place one on.)
    let slot = if today_first { first.checked_sub(1) } else { Some(first) }.map(|at| dock.slots.get(at));
    let (spot, of, figure, color, flag): (Spot, String, &str, Color, Option<Flag>) = match (slot, &dock.today) {
        (None, Some(today)) => (Spot::Today, lang.pick("今日", "Today").into(), today, theme.text, None),
        (Some(Some(slot)), _) => {
            let name = slot.source.name(lang);
            let of = if slot.label.is_empty() { name.to_string() } else { format!("{name} · {}", slot.label) };
            (Spot::Ring(slot.source), of, &slot.figure, figure_ink(theme, slot), slot.flag)
        }
        _ => return,
    };
    if scene.hover == Some(Hit::AiSlot(spot)) {
        frame.fill_rounded(theme.hover, left - 4.0, middle - tall / 2.0 - 4.0, width + 8.0, tall + 8.0, 12.0);
    }
    hits.push((left, middle - tall / 2.0, width, tall, Hit::AiSlot(spot)));
    let key = match spot {
        Spot::Ring(source) => source.id(),
        _ => "ai:today",
    };
    if h >= 60.0 {
        frame.key(&format!("{key}.label"));
        centred(frame, &of, label, theme.text3, left + width / 2.0, middle - tall / 2.0, width);
    }
    frame.key(&format!("{key}.value"));
    let figure_top = if h >= 60.0 { middle - tall / 2.0 + label.size + 4.0 } else { middle - font.size * 0.6 };
    centred(frame, figure, font, color, left + width / 2.0, figure_top, width);
    // (Its mark just after the figure, at its top, where there is room for it.)
    if let Some(flag) = flag {
        let side = (font.size * 0.42).clamp(6.0, 9.0);
        let after = left + width / 2.0 + frame.measure(figure, font).min(width) / 2.0 + 2.0 + side / 2.0;
        if after + side / 2.0 <= left + width {
            frame.key(&format!("{key}.flag"));
            paint_flag(frame, theme, flag, Point { x: after, y: figure_top + font.size * 0.25 }, side);
        }
    }
    if paged {
        if pager > 0.0 {
            pager_dots(frame, scene, (left + width + pager / 2.0, y + inset, (h - 2.0 * inset).max(0.0)), count, (first, 1), hits);
        }
        hits.push((x, y, w, h, Hit::AiPages(count as u32, today_first)));
    }
}

/// The dots that tell which of `count` turned round are shown (`shown`: the first of them and how many, past the last
/// the first again), down a column at `x` (their middle), in the middle of the room from `top` `tall`: a dot each,
/// those shown drawn long; closer together and shorter where the column would be taller than the room (see
/// `pager_fits`), none where they would be too small to tell apart. A press on one turns to its page (see
/// `Hit::AiPage`): the one under the pointer drawn as those shown are, and its room to be pressed `PAGER` wide.
fn pager_dots(frame: &dyn Canvas, scene: &Scene, (x, top, tall): (f32, f32, f32), count: usize, (first, per): (usize, usize), hits: &mut Vec<HitBox>) {
    let theme = scene.theme;
    let Some(k) = pager_scale(count, per, tall) else { return };
    let (dot, long, gap) = (PAGER_DOT, PAGER_LONG * k, PAGER_DOT * k);
    let column = (count - per) as f32 * (dot + gap) + per as f32 * (long + gap) - gap;
    let mut dot_y = top + (tall - column) / 2.0;
    frame.key("ai:pages");
    for n in 0..count {
        let on = (n + count - first) % count < per;
        let length = if on { long } else { dot };
        let hit = Hit::AiPage(n as u32);
        let lit = on || scene.hover == Some(hit);
        frame.fill_rounded(if lit { theme.text } else { theme.text3 }, x - dot / 2.0, dot_y, dot, length, dot / 2.0);
        hits.push((x - PAGER / 2.0, dot_y - gap / 2.0, PAGER, length + gap, hit));
        dot_y += length + gap;
    }
}

/// A page dot's width (and an unlit one's length), a lit one's length, and the least the room between them comes to.
const PAGER_DOT: f32 = 4.0;
const PAGER_LONG: f32 = 12.0;
const PAGER_GAP_LEAST: f32 = 1.5;

/// How much shorter the page dots' lit lengths and the room between them are drawn for `count` of them, `per` lit, to
/// be no taller than `tall` (1: as designed); none where that would leave less than `PAGER_GAP_LEAST` between them.
fn pager_scale(count: usize, per: usize, tall: f32) -> Option<f32> {
    let dots = (count - per) as f32 * PAGER_DOT;
    let scaled = per as f32 * PAGER_LONG + (count as f32 - 1.0) * PAGER_DOT;
    let k = ((tall - dots) / scaled).min(1.0);
    (k * PAGER_DOT >= PAGER_GAP_LEAST && k * PAGER_LONG >= PAGER_DOT).then_some(k)
}

/// Whether the page dots for `count`, `per` lit, fit in a column `tall`.
fn pager_fits(count: usize, per: usize, tall: f32) -> bool {
    pager_scale(count, per, tall).is_some()
}

/// The least a widget is to show the head, how today's spending mounted and what each ring's figure is of (DIPs).
const LEAST_ROOMY: (f32, f32) = (240.0, 200.0);

/// A source as the dock shows it: how much of its tightest window is used (none, with no limit told: its ring whole and
/// faint), and its figure under its ring (that share, else what it spent today) and what the figure is of (that window,
/// where it has several; else today).
#[derive(Clone, PartialEq, Debug)]
pub struct Slot {
    /// Its mark beside its ring and figure (see `Flag`), and whether its figure is drawn faint (the window it tells is
    /// out of date).
    pub flag: Option<Flag>,
    pub faint: bool,
    /// Its ring and figure told by how much of its tightest window is left (see `Prefs::ai_left`), not used.
    pub left: bool,
    pub source: Source,
    pub used: Option<f32>,
    pub figure: String,
    pub label: String,
    /// When its tightest window starts again, else what it spent today beside its balance (else none): told under what
    /// its figure is of where its slot has room.
    pub resets: String,
    /// Each window of its limit, by name, and how much of it is used: told under its ring down a tall widget.
    pub windows: Vec<(String, f32)>,
}

/// What the dock shows: what every source spent today, each source docked, those with a session at work, and how many
/// sessions are at work.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Dock {
    /// What every source spent today, where "Today" is shown (see `shows_today`).
    pub today: Option<String>,
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
pub(super) const BAR_HEIGHT: f32 = 60.0;
const BAR_END: f32 = 76.0;
const BAR_SLOT: f32 = 120.0;

/// The sources the dock shows, as `prefs` show the AI component: those switched on, in their order.
/// Whether the component shows what every source spent today (its module "Today" on): drawn nowhere where it is not,
/// in its lane, its dock or a widget.
pub fn shows_today(prefs: &Prefs) -> bool {
    prefs.modules.iter().any(|entry| entry.id == "ai:today" && shows_module(entry))
}

pub fn docked(prefs: &Prefs) -> Vec<Source> {
    prefs.modules.iter().filter(|entry| shows_module(entry)).filter_map(|entry| Source::of(&entry.id)).collect()
}

/// The ids of the sources shown, in their order: those a widget too small for all of them turns through (see
/// `paint_paged`).
pub fn sources_shown(prefs: &Prefs) -> Vec<String> {
    docked(prefs).into_iter().map(|source| source.id().to_string()).collect()
}

/// The dock's size as designed for `count` sources, with what every source spent today or not (`today`): down a side,
/// or `across` a top edge (DIPs).
pub fn dock_size(across: bool, count: usize, today: bool) -> (f32, f32) {
    let (count, today) = (count as f32, if today { 1.0 } else { 0.0 });
    if across {
        (2.0 * DOCK_PAD + (1.0 + today) * BAR_END + count * BAR_SLOT, BAR_HEIGHT)
    } else {
        (DOCK_WIDTH, 2.0 * DOCK_PAD + today * DOCK_TODAY + count * DOCK_SLOT + DOCK_FOOT)
    }
}

/// What the dock shows of `usage` (none: not read yet) at `now`, as `prefs` show the component.
#[cfg(windows)]
pub(super) fn dock_of(prefs: &Prefs, lang: Lang, usage: Option<&crate::ai::Usage>, now: u64) -> Dock {
    let sources = docked(prefs);
    let Some(usage) = usage else {
        let slots = sources.into_iter().map(|source| Slot { flag: None, faint: false, left: false, source, used: None, figure: "—".into(), label: lang.pick("正在读取", "Reading").into(), resets: String::new(), windows: Vec::new() }).collect();
        return Dock { today: shows_today(prefs).then(|| "—".into()), slots, at_work: Vec::new(), working: 0 };
    };
    let today = shows_today(prefs).then(|| short(usage.spent_today.iter().map(|(_, _, tokens)| tokens.total()).sum(), lang));
    let slots = sources
        .iter()
        .map(|&source| {
            let tool = tool_of(source);
            let limit = limit_of(usage, tool);
            let told_left = prefs.ai_left;
            let windows = windows_of(limit, now, lang, told_left);
            let flag = flag_of(usage, tool, &windows);
            let failed = usage.failed.iter().find(|(of, _)| *of == tool).map(|(_, failed)| *failed);
            let balance = limit.and_then(|limit| limit.balance.as_ref()).and_then(|balance| Some((balance.amount?, balance.currency.as_str())));
            let none = |label: &str| Slot { flag, faint: false, left: false, source, used: None, figure: "—".into(), label: label.into(), resets: String::new(), windows: Vec::new() };
            let spent = spent_by(usage, tool).1.total();
            // (A provider whose records on this PC tell it spent today shows that where its key is not set.)
            if source.keyed() && usage.unkeyed.contains(&tool) && spent == 0 {
                return none(lang.pick("未设置密钥", "No key"));
            }
            // (What was read before is not shown as it stands now: why, its card says, its red dot here.)
            if failed.is_some() && source.keyed() {
                return none("");
            }
            match windows.iter().max_by(|a, b| a.used.total_cmp(&b.used)) {
                Some(tightest) => Slot {
                    flag,
                    faint: tightest.stale,
                    left: told_left,
                    source,
                    used: Some(tightest.used),
                    figure: share_told(tightest.shown(), told_left, false, lang),
                    label: match (windows.len() > 1, told_left) {
                        (true, false) => tightest.name.clone(),
                        (false, false) => String::new(),
                        (several, true) => left_of(if several { &tightest.name } else { "" }, lang),
                    },
                    // (Out of date, said in its card alone.)
                    resets: if tightest.stale { String::new() } else { tightest.resets.clone() },
                    windows: windows.iter().map(|window| (if told_left { left_of(&window.name, lang) } else { window.name.clone() }, window.shown())).collect(),
                },
                None => match balance {
                    Some((amount, currency)) => Slot {
                        flag,
                        faint: false,
                        left: false,
                        source,
                        used: None,
                        figure: money_short(amount, currency),
                        label: lang.pick("余额", "Balance").into(),
                        resets: if spent > 0 { format!("{} {}", lang.pick("今日", "Today"), short(spent, lang)) } else { String::new() },
                        windows: Vec::new(),
                    },
                    None if source.keyed() && spent == 0 && !usage.unkeyed.contains(&tool) => none(lang.pick("正在读取", "Reading")),
                    None => Slot { flag, faint: false, left: false, source, used: None, figure: short(spent, lang), label: lang.pick("今日", "Today").into(), resets: String::new(), windows: Vec::new() },
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
pub fn paint_dock(frame: &dyn Canvas, scene: &Scene, across: bool, (w, h): (f32, f32)) -> Vec<HitBox> {
    #[cfg(windows)]
    let dock = dock_of(scene.prefs, scene.lang, crate::ai::usage().as_deref(), crate::station::now_ms());
    #[cfg(not(windows))]
    let dock = Dock { slots: docked(scene.prefs).into_iter().map(|source| Slot { flag: None, faint: false, left: false, source, used: None, figure: String::new(), label: String::new(), resets: String::new(), windows: Vec::new() }).collect(), ..Dock::default() };
    draw_dock(frame, scene, &dock, across, (0.0, 0.0, w, h))
}

/// `text` in `font`, its middle at `centre` across, from `top`.
fn centred(frame: &dyn Canvas, text: &str, font: Font, color: Color, centre: f32, top: f32, room: f32) {
    let width = frame.measure(text, font).min(room);
    frame.text(text, font, color, centre - width / 2.0, top, width + 2.0, Align::Start);
}

/// A source's ring, `radius` round `centre` and `width` wide, in its colour (the signal's from nine tenths used): as
/// much of it as is used, else (no limit told) whole and faint; its maker's mark in it.
fn ring(frame: &dyn Canvas, theme: &Theme, slot: &Slot, centre: Point, radius: f32, width: f32, mark: f32) {
    let hot = slot.used.is_some_and(|used| used >= HOT);
    let color = if hot { theme.signal } else { theme.ink(slot.source.id()).trace };
    frame.key(&format!("{}.ring", slot.source.id()));
    frame.arc(centre, radius, 0.0, std::f32::consts::TAU, theme.track, width);
    match slot.used.map(|used| if slot.left { 1.0 - used } else { used }) {
        Some(shown) if shown > 0.0 => frame.arc(centre, radius, 0.0, std::f32::consts::TAU * shown.min(1.0), color, width),
        Some(_) => {}
        None => frame.arc(centre, radius, 0.0, std::f32::consts::TAU, color.alpha(color.a * 0.55), width),
    }
    frame.key(&format!("{}.mark", slot.source.id()));
    frame.mark(slot.source.mark(), centre, mark, theme.text, 1.0);
    if let Some(flag) = slot.flag {
        frame.key(&format!("{}.flag", slot.source.id()));
        let off = (radius + width / 2.0) * std::f32::consts::FRAC_1_SQRT_2;
        paint_flag(frame, theme, flag, Point { x: centre.x + off, y: centre.y - off }, (radius * 0.42).clamp(6.0, 9.0));
    }
}

/// Draws `dock` filling the box `(x0, y0, w, h)`, down a side or `across` a top edge (see `paint_dock`). Drawn larger than
/// designed (a widget's), its parts kept together, so far apart at most, the rest of its room round them all: across,
/// each slot wider, telling its source's name and when its window starts again; down, each slot taller, telling each of
/// its windows, where there is room for them. Each spot as tall across as designed, not as the widget is.
pub(super) fn draw_dock(frame: &dyn Canvas, scene: &Scene, dock: &Dock, across: bool, (x0, y0, w, h): (f32, f32, f32, f32)) -> Vec<HitBox> {
    let (theme, lang) = (scene.theme, scene.lang);
    let hits = std::cell::RefCell::new(Vec::new());
    let label = Font { size: 11.0, weight: 500.0, ..theme.body };
    let big = Font { size: 14.0, weight: 650.0, ..theme.figure };
    let (figure, under) = slot_fonts(theme);
    let count = dock.slots.len();
    let parts = count as f32 + if dock.today.is_some() { 2.0 } else { 1.0 };
    let natural = dock_size(across, count, dock.today.is_some());
    let figure_of = |slot: &Slot| figure_ink(theme, slot);
    let lit = |spot: Spot| scene.hover == Some(Hit::AiSlot(spot));
    // Each spot the pointer rests on to show its card beside the dock: where it is, and its plate lit under the pointer.
    let spot = |frame: &dyn Canvas, spot: Spot, (x, y, w, h): (f32, f32, f32, f32), (px, py, pw, ph, round): (f32, f32, f32, f32, f32)| {
        if lit(spot) {
            frame.fill_rounded(theme.hover, px, py, pw, ph, round);
        }
        hits.borrow_mut().push((x, y, w, h, Hit::AiSlot(spot)));
    };
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
        let middle = y0 + h / 2.0;
        let extra = (w - natural.0).max(0.0);
        let grow = (extra / count.max(1) as f32).min(STRIP_GROW);
        let gap = ((extra - grow * count as f32) / parts).min(STRIP_GAP);
        let round_it = extra - grow * count as f32 - gap * parts;
        // (Narrower than designed, each slot narrower, as far as `strip_least` lets it be.)
        let slot_w = BAR_SLOT + grow - (natural.0 - w).max(0.0) / count.max(1) as f32;
        // (A spot and its plate as tall as designed, in the middle.)
        let band = h.min(BAR_HEIGHT);
        let pill = band - 10.0;
        let at = |x: f32, wide: f32| ((x, middle - band / 2.0, wide, band), (x, middle - pill / 2.0, wide, pill, pill / 2.0));
        let named = slot_w >= STRIP_NAMED;
        let mut x = x0 + DOCK_PAD + gap / 2.0 + round_it / 2.0;
        if let Some(today) = &dock.today {
            let (hit, plate) = at(x, BAR_END);
            spot(frame, Spot::Today, hit, plate);
            frame.key("ai:today.label");
            centred(frame, lang.pick("今日", "Today"), label, theme.text3, x + BAR_END / 2.0, middle - 17.0, BAR_END);
            frame.key("ai:today.value");
            centred(frame, today, big, theme.text, x + BAR_END / 2.0, middle - 2.0, BAR_END);
            x += BAR_END + gap;
        }
        for slot in &dock.slots {
            let (hit, plate) = at(x, slot_w);
            spot(frame, Spot::Ring(slot.source), hit, plate);
            ring(frame, theme, slot, Point { x: x + 8.0 + 17.0, y: middle }, 16.0, 2.6, 15.0);
            let words = x + SLOT_WORDS;
            let room = x + slot_w - 8.0 - words;
            // (Wider, its source named before what its figure is of, and when its window starts again under them.)
            let name = slot.source.name(lang);
            let told = match (named, slot.label.is_empty()) {
                (false, _) => slot.label.clone(),
                (true, true) => name.to_string(),
                (true, false) => format!("{name} · {}", slot.label),
            };
            let resets = (named && !slot.resets.is_empty()).then_some(slot.resets.as_str());
            let (figure_top, told_top) = match (resets, told.is_empty()) {
                (Some(_), _) => (middle - 24.0, middle - 7.0),
                (None, true) => (middle - 9.0, middle + 1.0),
                (None, false) => (middle - 17.0, middle + 1.0),
            };
            frame.key(&format!("{}.value", slot.source.id()));
            frame.text(&slot.figure, figure, figure_of(slot), words, figure_top, room, Align::Start);
            frame.key(&format!("{}.label", slot.source.id()));
            frame.text(&told, under, theme.text3, words, told_top, room, Align::Start);
            if let Some(resets) = resets {
                frame.key(&format!("{}.resets", slot.source.id()));
                frame.text(resets, under, theme.text3, words, middle + 8.0, room, Align::Start);
            }
            x += slot_w + gap;
        }
        let (hit, plate) = at(x, BAR_END);
        spot(frame, Spot::Sessions, hit, plate);
        frame.key("ai:dock.sessions");
        centred(frame, sessions.0, label, theme.text3, x + BAR_END / 2.0, middle - 17.0, BAR_END);
        let count_width = frame.measure(&sessions.1, big);
        let row = if dock.at_work.is_empty() { 0.0 } else { dock.at_work.len() as f32 * 15.0 + 2.0 };
        let start = x + BAR_END / 2.0 - (count_width + row) / 2.0;
        frame.key("ai:dock.at_work");
        frame.text(&sessions.1, big, theme.text, start, middle - 2.0, count_width + 2.0, Align::Start);
        marks(start + count_width + 4.0 + (row - 4.0) / 2.0, middle + 7.0);
        return hits.into_inner();
    }
    let centre = x0 + w / 2.0;
    // What each slot tells under its figure where the dock is tall enough for all of it: each of its windows by name and
    // how much is used, else what its figure is of and when it starts again.
    let lines_of = |slot: &Slot| -> Vec<String> {
        if slot.windows.is_empty() {
            [&slot.label, &slot.resets].into_iter().filter(|line| !line.is_empty()).cloned().collect()
        } else {
            slot.windows.iter().take(DOWN_LINES).map(|(name, used)| format!("{name} {:.0}%", used * 100.0)).collect()
        }
    };
    let most = dock.slots.iter().map(|slot| lines_of(slot).len()).max().unwrap_or(0);
    let more = most.saturating_sub(1) as f32 * DOWN_LINE;
    let lined = more > 0.0 && h >= natural.1 + count as f32 * more;
    let slot_h = DOCK_SLOT + if lined { more } else { 0.0 };
    let tall = natural.1 + count as f32 * (slot_h - DOCK_SLOT);
    let gap = ((h - tall) / parts).clamp(0.0, DOWN_GAP);
    let round_it = (h - tall - gap * parts).max(0.0);
    let mut y = y0 + DOCK_PAD + gap / 2.0 + round_it / 2.0;
    let round = 16.0_f32.min((w - 10.0) / 2.0);
    if let Some(today) = &dock.today {
        spot(frame, Spot::Today, (x0, y, w, DOCK_TODAY), (x0 + 5.0, y - 4.0, w - 10.0, DOCK_TODAY, round));
        frame.key("ai:today.label");
        centred(frame, lang.pick("今日", "Today"), label, theme.text3, centre, y + 2.0, w - 10.0);
        frame.key("ai:today.value");
        centred(frame, today, big, theme.text, centre, y + 18.0, w - 10.0);
        y += DOCK_TODAY + gap;
    }
    for slot in &dock.slots {
        spot(frame, Spot::Ring(slot.source), (x0, y, w, slot_h), (x0 + 5.0, y, w - 10.0, slot_h - 4.0, round));
        ring(frame, theme, slot, Point { x: centre, y: y + 8.0 + 22.0 }, 21.5, 3.0, 20.0);
        frame.key(&format!("{}.value", slot.source.id()));
        centred(frame, &slot.figure, figure, figure_of(slot), centre, y + 56.0, w - 10.0);
        if lined {
            for (at, line) in lines_of(slot).iter().enumerate() {
                frame.key(&if at == 0 { format!("{}.label", slot.source.id()) } else { format!("{}.line{at}", slot.source.id()) });
                centred(frame, line, under, theme.text3, centre, y + 74.0 + at as f32 * DOWN_LINE, w - 10.0);
            }
        } else {
            frame.key(&format!("{}.label", slot.source.id()));
            centred(frame, &slot.label, under, theme.text3, centre, y + 74.0, w - 10.0);
        }
        y += slot_h + gap;
    }
    spot(frame, Spot::Sessions, (x0, y, w, DOCK_FOOT), (x0 + 5.0, y, w - 10.0, DOCK_FOOT + 4.0, round));
    frame.key("ai:dock.sessions");
    centred(frame, sessions.0, label, theme.text3, centre, y + 4.0, w - 10.0);
    frame.key("ai:dock.at_work");
    centred(frame, &sessions.1, big, theme.text, centre, y + 19.0, w - 10.0);
    marks(centre, y + 44.0);
    hits.into_inner()
}

/// The fonts of a slot's figure and of what it is of, on the dock.
fn slot_fonts(theme: &Theme) -> (Font, Font) {
    (Font { size: 13.0, weight: 650.0, ..theme.figure }, Font { size: 10.5, weight: 500.0, ..theme.body })
}

/// How narrow the dock's strip across is drawn at least (DIPs): each slot as wide as its ring and the wider of its
/// figure and what it is of take (none wider than designed), the rest as designed.
pub(super) fn strip_least(frame: &dyn Canvas, theme: &Theme, dock: &Dock) -> f32 {
    let (figure, under) = slot_fonts(theme);
    let slot = dock.slots.iter().map(|slot| SLOT_WORDS + frame.measure(&slot.figure, figure).max(frame.measure(&slot.label, under)) + 8.0).fold(0.0, f32::max).min(BAR_SLOT);
    2.0 * DOCK_PAD + (if dock.today.is_some() { 2.0 } else { 1.0 }) * BAR_END + dock.slots.len() as f32 * slot
}

/// Where a slot's words start across its strip (DIPs): its ring's room.
const SLOT_WORDS: f32 = 8.0 + 34.0 + 9.0;

/// The dock's strip drawn wider than designed: how much wider each slot is at most, how far apart its parts are at most,
/// and how wide a slot is to name its source and tell when its window starts again (DIPs).
pub(super) const STRIP_GROW: f32 = 80.0;
pub(super) const STRIP_GAP: f32 = 32.0;
const STRIP_NAMED: f32 = 150.0;
/// Drawn taller down a side: how far apart its parts are at most, the lines each slot tells under its figure at most,
/// and a line's (DIPs).
pub(super) const DOWN_GAP: f32 = 28.0;
const DOWN_LINES: usize = 3;
const DOWN_LINE: f32 = 14.0;

/// The name of folder `folder` (its last part, as written on Windows or elsewhere): none for a drive's root.
fn folder_name(folder: &str) -> Option<String> {
    let name = folder.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next()?;
    (!name.is_empty() && !name.ends_with(':')).then(|| name.to_string())
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

/// Shares of one whole along a bar, each in its colour, from the left: the bar as a meter is (as tall, its ends as
/// round), each share cut out of that one rounded bar, so the bar's round ends are the first's and the last's and where
/// two meet is straight. (Where each ends on whole pixels as a meter's filling does: one after another, no seam.)
fn shares(frame: &dyn Canvas, theme: &Theme, (x, y, width): (f32, f32, f32), parts: &[(f32, Color)]) {
    let whole = frame.on_pixels(x, y, width, theme.meter_height);
    let (bx, by, bw, bh) = whole;
    frame.fill_rounded(theme.track, bx, by, bw, bh, theme.meter_radius);
    let (mut gone, mut from) = (0.0, bx);
    for &(share, colour) in parts {
        gone += share;
        let (_, _, filled, _) = filled_part(frame, whole, gone, false);
        let to = bx + filled;
        if to > from {
            frame.clip(from, by, to - from, bh);
            frame.fill_rounded(colour, bx, by, bw, bh, theme.meter_radius);
            frame.unclip();
            from = to;
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    #[test]
    fn every_source_opens_but_those_folded_by_hand() {
        follow(&[]);
        assert!(SOURCES.into_iter().all(opened));
        follow(&["ai:codex".to_string(), "ai:glm".to_string()]);
        assert!(!opened(Source::Codex) && !opened(Source::Glm) && opened(Source::Claude));
        follow(&[]);
    }

    #[test]
    fn sources_folded_one_after_another_are_kept_whatever_order_their_changes_come_in() {
        // Claude folded, then Codex, then Claude opened again: their changes into the settings in each order.
        let changes = [Source::Claude, Source::Codex, Source::Claude];
        for order in [[0, 1, 2], [2, 1, 0], [1, 0, 2], [1, 2, 0]] {
            let mut kept = Vec::new();
            for n in order {
                folded_again(&mut kept, changes[n]);
            }
            assert_eq!(kept, vec!["ai:codex".to_string()], "{order:?}");
        }
    }

    type Box4 = (f32, f32, f32, f32);
    /// A fill: its colour, its box, how round (none, a plain rectangle), and the clip it was drawn in.
    type Drawn = (Color, Box4, Option<f32>, Option<Box4>);

    /// What a bar's drawing came to: each fill, the box it was asked for, how round, and the clip it was drawn in.
    #[derive(Default)]
    struct Bars {
        clips: std::cell::RefCell<Vec<Box4>>,
        fills: std::cell::RefCell<Vec<Drawn>>,
    }

    impl Canvas for Bars {
        fn text(&self, _: &str, _: Font, _: Color, _: f32, _: f32, _: f32, _: Align) {}
        fn markdown(&self, _: &str, _: Font, _: crate::ui::canvas::Inks, _: f32, _: f32, _: f32, _: f32, _: f32) {}
        fn markdown_tall(&self, _: &str, _: Font, _: f32) -> f32 {
            0.0
        }
        fn measure(&self, _: &str, _: Font) -> f32 {
            0.0
        }
        fn baseline(&self, font: Font) -> (f32, f32) {
            (font.size, 0.0)
        }
        fn ink(&self, _: &str, _: Font) -> (f32, f32) {
            (0.0, 0.0)
        }
        fn clip(&self, x: f32, y: f32, w: f32, h: f32) {
            self.clips.borrow_mut().push((x, y, w, h));
        }
        fn unclip(&self) {
            self.clips.borrow_mut().pop();
        }
        fn fill(&self, color: Color, x: f32, y: f32, w: f32, h: f32) {
            self.fills.borrow_mut().push((color, (x, y, w, h), None, self.clips.borrow().last().copied()));
        }
        fn fill_rounded(&self, color: Color, x: f32, y: f32, w: f32, h: f32, radius: f32) {
            self.fills.borrow_mut().push((color, (x, y, w, h), Some(radius), self.clips.borrow().last().copied()));
        }
        fn fill_circle(&self, _: Color, _: Point, _: f32) {}
        fn stroke(&self, _: &[Point], _: Color, _: f32) {}
        fn stroke_dashed(&self, _: &[Point], _: Color, _: f32) {}
        fn fill_shape(&self, _: &[Point], _: Fill) {}
        fn stroke_rounded(&self, _: Fill, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
        fn shadow(&self, _: Color, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
        fn frosted(&self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32, _: &dyn Fn(&dyn Canvas)) {}
        fn arc(&self, _: Point, _: f32, _: f32, _: f32, _: Color, _: f32) {}
        fn icon(&self, _: Icon, _: Point, _: f32, _: Color) {}
        fn mark(&self, _: Mark, _: Point, _: f32, _: Color, _: f32) {}
        fn picture_within(&self, _: &crate::ui::canvas::Picture, _: (f32, f32, f32, f32), _: (f32, f32, f32, f32, f32), _: f32) {}
        fn pixels_per_dip(&self) -> f32 {
            1.0
        }
    }

    /// The models' shares along their bar are drawn as a meter is, in every skin: the bar as tall and its ends as round
    /// as a limit's meter, each share cut out of that one rounded bar, so the first's left end and the last's right end
    /// are the bar's round ends, where two shares meet is straight, and the shares cover the bar edge to edge.
    #[test]
    fn the_models_shares_are_cut_from_one_bar_as_round_as_a_meter() {
        for skin in [Skin::Glass, Skin::Fluent, Skin::Paper] {
            let theme = Theme::new(skin, false);
            let (meter, bar) = (Bars::default(), Bars::default());
            super::meter(&meter, &theme, (10.0, 20.0, 300.0), 0.5, Color::hex(0x123456, 1.0));
            let parts = [(0.6, Color::hex(0xff0000, 1.0)), (0.3, Color::hex(0x00ff00, 1.0)), (0.1, Color::hex(0x0000ff, 1.0))];
            shares(&bar, &theme, (10.0, 20.0, 300.0), &parts);
            let track = meter.fills.borrow()[0];
            let fills = bar.fills.borrow();
            assert_eq!(fills[0], track, "{skin:?}: the track as a meter's");
            let mut from = track.1 .0;
            for (&(_, colour), &(drawn, at, round, clip)) in parts.iter().zip(&fills[1..]) {
                assert_eq!((drawn, at, round), (colour, track.1, track.2), "{skin:?}: a share drawn as the whole bar, as round");
                let clip = clip.expect("each share clipped to its part");
                assert_eq!((clip.0, clip.1, clip.3), (from, track.1 .1, track.1 .3), "{skin:?}: shares meet");
                from = clip.0 + clip.2;
            }
            assert_eq!(fills.len(), 1 + parts.len());
            assert_eq!(from, track.1 .0 + track.1 .2, "{skin:?}: the last share ends where the bar does");
        }
    }

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

        /// How much of a window has gone by is told as of the minute, as when it starts again is: the same through the
        /// minute (the card beside the dock, which shows it, is not drawn again for each of its frames), the next minute
        /// a minute more.
        #[test]
        fn a_windows_time_gone_is_told_as_of_the_minute() {
            let usage = day();
            let windows = |now: u64| match lane_of("ai:codex", &Prefs::default(), Lang::Zh, Some(&usage), now, Some(false)).unwrap().as_slice() {
                [_, Block::Ai(AiBlock::Windows(windows))] => windows.clone(),
                blocks => panic!("{} blocks", blocks.len()),
            };
            assert_eq!(windows(NOON + 1_000), windows(NOON + 59_000));
            let (minute, next) = (windows(NOON + 1_000)[0].gone.unwrap(), windows(NOON + 61_000)[0].gone.unwrap());
            assert!((next - minute - 1.0 / 300.0).abs() < 1e-5, "{minute} then {next}");
        }

        /// A source in the component's lane, closed: its head (its name, its plan, what it spent today) and each window
        /// of its limit, how much is used, how much of the window has gone by and when it starts again (one started
        /// again since it was told, none of it used); a source with no limit told, its head alone.
        #[test]
        fn a_source_shows_its_limits_windows() {
            let usage = day();
            let blocks = lane("ai:codex", Some(&usage));
            let [Block::Ai(AiBlock::Head { title, device, aside, opened, .. }), Block::Ai(AiBlock::Windows(windows))] = blocks.as_slice() else { panic!("{} blocks", blocks.len()) };
            assert_eq!((title.as_str(), device.as_str(), *opened), ("ChatGPT", "Pro", Some(false)));
            assert_eq!(aside, "10 万");
            let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
            assert!(windows[0].name == "5 小时" && near(windows[0].used, 0.62) && windows[0].gone.is_some_and(|gone| near(gone, 0.6)), "{:?}", windows[0]);
            assert_eq!((windows[0].resets.as_str(), windows[1].resets.as_str()), ("2 小时后重置", "4 天后重置"));
            assert!(windows[1].used >= HOT && windows[1].name == "每周");
            // (Started again since it was told, nothing told of it since: told as out of date, see
            // `a_source_out_of_date_or_failing_is_marked`.)
            assert_eq!(windows[2], Window { name: "每天".into(), used: 0.0, gone: None, resets: "已过重置时间，尚未取得新数据".into(), left: false, stale: true });
            let blocks = lane("ai:claude", Some(&usage));
            let [Block::Ai(AiBlock::Head { device, aside, .. })] = blocks.as_slice() else { panic!("{} blocks", blocks.len()) };
            assert_eq!((device.as_str(), aside.as_str()), ("Claude Code", "10 万"));
            // (Beside the dock, when it was last heard of, as its session spent a minute ago: what it spent is told below.)
            let card = lane_of("ai:codex", &Prefs::default(), Lang::Zh, Some(&usage), NOON, None).unwrap();
            assert!(matches!(&card[0], Block::Ai(AiBlock::Head { aside, .. }) if *aside == text::since(NOON - 60_000, NOON, Lang::Zh)));
            // (Not read yet: it says so, and opens nothing.)
            let blocks = lane("ai:claude", None);
            assert!(matches!(&blocks[0], Block::Ai(AiBlock::Head { opened: None, .. })));
            assert!(matches!(&blocks[1], Block::Facts { rows, .. } if rows[0].1 == "正在读取"));
        }

        /// The card beside the dock's 今日: the figure and how it mounted, each source's share of it (the most first, a
        /// source of records by its tool's name), and the whole by kind with every session of today.
        #[test]
        fn the_today_card_tells_each_sources_share_and_the_whole_by_kind() {
            let mut usage = day();
            // (Codex spent half again as much as Claude Code, read after it.)
            usage.spent_today.push((crate::ai::Tool::Codex, "gpt-5.5-mini".into(), spent(50_000, 0, 0, 0, 0)));
            let blocks = today_card(Lang::Zh, Some(&usage));
            assert!(matches!(&blocks[0], Block::Head { title, .. } if title == "今日用量"));
            assert!(matches!(&blocks[1], Block::Ai(AiBlock::Today { figure, unit, .. }) if figure == "25" && unit == "万 tokens"));
            let Block::Ai(AiBlock::Sources(sources)) = &blocks[2] else { panic!("no sources") };
            assert_eq!(sources.iter().map(|(source, name, _, value)| (*source, name.as_str(), value.as_str())).collect::<Vec<_>>(), [(Source::Codex, "Codex", "15 万"), (Source::Claude, "Claude Code", "10 万")]);
            assert!((sources[0].2 - 0.6).abs() < 1e-4 && (sources.iter().map(|(_, _, share, _)| share).sum::<f32>() - 1.0).abs() < 1e-5);
            let Block::Ai(AiBlock::Pairs(pairs)) = &blocks[3] else { panic!("no pairs") };
            assert_eq!(pairs[0], ("输入".to_string(), "8.2 万".to_string()));
            assert!(pairs.contains(&("缓存写入".into(), "7,000".into())) && pairs.contains(&("会话".into(), "3".into())));
            // (Not read yet: it says so.)
            assert!(matches!(&today_card(Lang::Zh, None)[1], Block::Facts { rows, .. } if rows[0].1 == "正在读取"));
        }

        /// The card beside the dock's 会话: the sessions of the sources docked, those at work first (how many in its head),
        /// then the others of today, the latest first, each with its source.
        #[test]
        fn the_sessions_card_tells_every_sources_sessions_at_work_first() {
            let mut usage = day();
            let prefs = Prefs::resolve(&serde_json::Value::Null, &[]).holding(crate::settings::Part::Ai);
            let told = |usage: &crate::ai::Usage| {
                let blocks = sessions_card(&prefs, Lang::Zh, Some(usage), NOON);
                let Block::Head { aside, .. } = &blocks[0] else { panic!() };
                let Some(Block::Ai(AiBlock::Sessions(rows))) = blocks.get(1) else { return (aside.clone(), Vec::new()) };
                (aside.clone(), rows.iter().map(|(title, _, value, at_work, source)| (title.clone(), value.clone(), *at_work, source.unwrap())).collect::<Vec<_>>())
            };
            assert_eq!(
                told(&usage),
                ("1 个进行中".to_string(), vec![
                    ("Review the release".to_string(), "7.0 万".to_string(), true, Source::Codex),
                    ("未命名会话".to_string(), "10 万".to_string(), false, Source::Claude),
                    ("未命名会话".to_string(), "3.0 万".to_string(), false, Source::Codex),
                ])
            );
            // (Claude's at work now: before the one that was at work first, being the later.)
            usage.sessions[1].last = NOON - 30_000;
            assert_eq!(told(&usage).0, "2 个进行中");
            assert_eq!(told(&usage).1.iter().map(|row| row.3).collect::<Vec<_>>(), [Source::Claude, Source::Codex, Source::Codex]);
            // (None of today: it says so.)
            usage.sessions.clear();
            let blocks = sessions_card(&prefs, Lang::Zh, Some(&usage), NOON);
            assert!(matches!(&blocks[1], Block::Facts { rows, .. } if rows[0].1 == "暂无"));
        }

        /// A session nothing names (one whose every prompt another program sent it) is told by the folder it was worked
        /// in, the folder's own name: as untitled only with neither, or a folder that is a drive's root.
        #[test]
        fn a_session_nothing_names_is_told_by_its_folder() {
            let told = |folder: Option<&str>| {
                let mut usage = day();
                usage.sessions.iter_mut().find(|session| session.id == "b").unwrap().folder = folder.map(String::from);
                let blocks = lane_of("ai:codex", &Prefs::default(), Lang::Zh, Some(&usage), NOON, Some(true)).unwrap();
                let sessions = blocks.iter().find_map(|block| if let Block::Ai(AiBlock::Sessions(rows)) = block { Some(rows.clone()) } else { None }).unwrap();
                sessions.iter().map(|(title, ..)| title.clone()).collect::<Vec<_>>()
            };
            assert_eq!(told(Some(r"E:\Developer\Glance")), ["Review the release", "Glance"]);
            assert_eq!(told(Some(r"E:\Developer\Glance\")), ["Review the release", "Glance"]);
            assert_eq!(told(Some("/Users/someone/projects/glance-port")), ["Review the release", "glance-port"]);
            assert_eq!(told(Some(r"D:\")), ["Review the release", "未命名会话"]);
            assert_eq!(told(None), ["Review the release", "未命名会话"]);
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
            assert_eq!(sessions.iter().map(|(title, _, value, at_work, source)| (title.as_str(), value.as_str(), *at_work, *source)).collect::<Vec<_>>(), [("Review the release", "7.0 万", true, None), ("未命名会话", "3.0 万", false, None)]);
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
            assert_eq!(dock.today.as_deref(), Some("20 万"));
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

        /// A source whose limit may be out of date (a window's time to start again gone by, nothing told since) is marked
        /// by a hollow grey ring at its ring, its figure faint where the window it tells is that one, nothing said under
        /// it; its card says it under that window, faint. One whose reading failed is marked by a red dot: Codex's
        /// sign-in expired, its limits from its logs still shown, its card saying so; a provider's key refused, its
        /// figure "—" and nothing said under it (it said "密钥无效" there), its card saying why. (Nothing told either:
        /// what was out of date was shown as it was told, and the red words crowded a ring's cell.)
        #[test]
        fn a_source_out_of_date_or_failing_is_marked() {
            use crate::ai::{Failed, Tool};
            let prefs = Prefs::resolve(&serde_json::Value::Null, &[]).holding(crate::settings::Part::Ai);
            let codex = |usage: &crate::ai::Usage| dock_of(&prefs, Lang::Zh, Some(usage), NOON).slots.into_iter().find(|slot| slot.source == Source::Codex).unwrap();
            let head_flag = |blocks: &[Block]| match &blocks[0] {
                Block::Ai(AiBlock::Head { flag, .. }) => *flag,
                _ => panic!("no head"),
            };
            // A day's: one of Codex's windows started again an hour ago, nothing told since.
            let usage = day();
            let slot = codex(&usage);
            assert_eq!((slot.flag, slot.faint, slot.figure.as_str()), (Some(Flag::Stale), false, "95%"));
            let blocks = lane("ai:codex", Some(&usage));
            assert_eq!(head_flag(&blocks), Some(Flag::Stale));
            let windows = blocks.iter().find_map(|block| if let Block::Ai(AiBlock::Windows(windows)) = block { Some(windows) } else { None }).unwrap();
            assert_eq!(windows.iter().map(|window| (window.stale, window.resets.as_str() == "已过重置时间，尚未取得新数据")).collect::<Vec<_>>(), [(false, false), (false, false), (true, true)]);
            // Every window started again since: the figure faint, nothing under it.
            let mut usage = day();
            usage.limits[0].windows.iter_mut().for_each(|window| window.resets = Some(NOON - 60_000));
            let slot = codex(&usage);
            assert_eq!((slot.flag, slot.faint, slot.resets.as_str()), (Some(Flag::Stale), true, ""));
            // Told again: no mark.
            let mut usage = day();
            usage.limits[0].windows.truncate(2);
            assert_eq!(codex(&usage).flag, None);
            assert_eq!(head_flag(&lane("ai:codex", Some(&usage))), None);
            // Codex's sign-in expired: a red dot, its logs' limits still shown, its card saying why.
            usage.failed.push((Tool::Codex, Failed::Key));
            let slot = codex(&usage);
            assert_eq!((slot.flag, slot.figure.as_str()), (Some(Flag::Problem), "95%"));
            let blocks = lane("ai:codex", Some(&usage));
            assert_eq!(head_flag(&blocks), Some(Flag::Problem));
            assert!(blocks.iter().any(|block| matches!(block, Block::Facts { rows, .. } if rows[0] == ("状态".into(), "登录已过期，额度来自本机记录".into(), true))));
            // A provider's key refused: "—", nothing under it, a red dot.
            let usage = accounts();
            let prefs = Prefs::resolve(&serde_json::json!({ "ai": [{ "id": "ai:kimi", "on": true }] }), &[]).holding(crate::settings::Part::Ai);
            let kimi = dock_of(&prefs, Lang::Zh, Some(&usage), NOON).slots.into_iter().find(|slot| slot.source == Source::Kimi).expect("Kimi docked");
            assert_eq!((kimi.flag, kimi.figure.as_str(), kimi.label.as_str()), (Some(Flag::Problem), "—", ""));
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
            assert!(matches!(blocks.as_slice(), [Block::Ai(AiBlock::Head { opened: Some(false), device, aside, .. })] if device == "Kimi Code" && aside == "3.1 万"));
            let stored = serde_json::json!({ "ai": [{ "id": "ai:kimi", "on": true }] });
            let dock = dock_of(&Prefs::resolve(&stored, &[]).holding(crate::settings::Part::Ai), Lang::Zh, Some(&usage), NOON);
            assert!(dock.slots.iter().any(|slot| slot.source == Source::Kimi && slot.figure == "3.1 万" && slot.label == "今日"));
        }

        /// The dock: a provider by its tightest window, else by its balance; one whose key is not set, by that; one whose
        /// key was refused, by "—" and its red dot (why said in its card: see `a_source_out_of_date_or_failing_is_marked`),
        /// nothing read before shown as it stands now.
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
                    (Source::Kimi, None, "—", ""),
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
