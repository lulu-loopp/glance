//! The AI providers' accounts, read by the user's keys (see `keys`), each from its provider alone: their plans'
//! windows (how much of each is used, when it starts again) and their balances (what is left, what was spent today).
//!
//! Which of each provider's endpoints tell what, how their answers are read, and how Volcengine's requests are signed
//! follow token-monitor (https://github.com/Javis603/token-monitor, MIT License, Copyright (c) 2026 Javis), its
//! readers ported here; its licence is in `licenses/`. Moonshot's balance is read as its own documentation gives.
//! Nothing is read that would spend from an account (token-monitor reads Volcengine Ark's request limits by asking
//! its models for an answer: not here).

use serde_json::Value;

use super::{Balance, Failed, Limit, Tool, Window};

/// One of what a provider's account is read by: its key's name among the keys given (see `keys`), and the
/// environment variables its own tools read it from, the first of them first.
pub struct Field {
    pub id: &'static str,
    pub env: &'static [&'static str],
}

/// A provider whose account is read by the user's keys: which tool it is, what it is read by, and how.
pub struct Provider {
    pub tool: Tool,
    pub fields: &'static [Field],
    read: fn(&[String], u64) -> Result<Limit, Failed>,
}

impl Provider {
    /// Its account read with `keys` (one for each of its fields) at `now`.
    pub fn read(&self, keys: &[String], now: u64) -> Result<Limit, Failed> {
        (self.read)(keys, now)
    }

    /// Its keys, where each of its fields has one (from the environment, or as given).
    pub fn keys(&self) -> Option<Vec<String>> {
        self.fields.iter().map(|field| super::keys::key(field.id, field.env)).collect()
    }
}

/// The providers, as the AI component lists them.
pub const ALL: [Provider; 7] = [
    Provider { tool: Tool::DeepSeek, fields: &[Field { id: "deepseek", env: &["DEEPSEEK_API_KEY", "DEEPSEEK_KEY"] }], read: deepseek },
    Provider { tool: Tool::Kimi, fields: &[Field { id: "kimi", env: &["KIMI_CODE_API_KEY"] }], read: kimi },
    Provider { tool: Tool::Moonshot, fields: &[Field { id: "moonshot", env: &["MOONSHOT_API_KEY"] }], read: moonshot },
    Provider { tool: Tool::Glm, fields: &[Field { id: "glm", env: &["ZAI_API_KEY", "Z_AI_API_KEY", "GLM_API_KEY", "ZHIPU_API_KEY"] }], read: glm },
    Provider { tool: Tool::MiniMax, fields: &[Field { id: "minimax", env: &["MINIMAX_CODING_API_KEY", "MINIMAX_API_KEY"] }], read: minimax },
    Provider {
        tool: Tool::Volcengine,
        fields: &[
            Field { id: "volcengine-access", env: &["VOLCENGINE_ACCESS_KEY_ID", "VOLCENGINE_ACCESS_KEY", "VOLC_ACCESSKEY"] },
            Field { id: "volcengine-secret", env: &["VOLCENGINE_SECRET_ACCESS_KEY", "VOLCENGINE_SECRET_KEY", "VOLC_SECRETKEY"] },
        ],
        read: volcengine,
    },
    Provider { tool: Tool::OpenRouter, fields: &[Field { id: "openrouter", env: &["OPENROUTER_API_KEY"] }], read: openrouter },
];

/// The provider `tool` is, where it is one.
pub fn of(tool: Tool) -> Option<&'static Provider> {
    ALL.iter().find(|provider| provider.tool == tool)
}

/// What `url` answered to the request with `headers` (`sent` POSTed): its status, and its body as JSON (null where it
/// is not that). Not reached: unavailable.
fn ask(url: &str, headers: &[(&str, &str)], sent: Option<&[u8]>) -> Result<(u32, Value), Failed> {
    let (status, body) = crate::update::answer_keyed(url, headers, sent).ok_or(Failed::Unavailable)?;
    Ok((status, serde_json::from_slice(&body).unwrap_or(Value::Null)))
}

/// A status answered: through where it is one of success, else what failed (`forbidden` too a key refused).
fn passed(status: u32, forbidden: bool) -> Result<(), Failed> {
    match status {
        200..=299 => Ok(()),
        401 => Err(Failed::Key),
        403 if forbidden => Err(Failed::Key),
        429 => Err(Failed::Limited),
        _ => Err(Failed::Unavailable),
    }
}

fn bearer(key: &str) -> String {
    format!("Bearer {key}")
}

/// A number, written as one or as a string of one.
fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }.filter(|number: &f64| number.is_finite())
}

/// A moment (ms since 1970): an ISO 8601 time, or a count, of seconds where it is below `seconds_below`, else of ms.
fn moment(value: &Value, seconds_below: f64) -> Option<u64> {
    if let Some(text) = value.as_str().filter(|text| text.contains('T')) {
        return super::moment(text);
    }
    let count = number(value).filter(|count| *count > 0.0)?;
    Some(if count < seconds_below { count * 1000.0 } else { count } as u64)
}

/// A share used, from 0 to 100.
fn share(used: f64) -> f32 {
    used.clamp(0.0, 100.0) as f32
}

/// An account's limit as it was read at `now`.
fn limit(tool: Tool, plan: Option<String>, windows: Vec<Window>, balance: Option<Balance>, now: u64) -> Limit {
    let id = of(tool).map_or("", |provider| provider.fields[0].id);
    Limit { tool, id: id.into(), name: None, plan, windows, balance, told: now }
}

// DeepSeek: what is left of its balance (as its API tells it), today's spending counted from one reading to the next.

fn deepseek(keys: &[String], now: u64) -> Result<Limit, Failed> {
    let (status, body) = ask("https://api.deepseek.com/user/balance", &[("Authorization", &bearer(&keys[0]))], None)?;
    passed(status, false)?;
    Ok(limit(Tool::DeepSeek, None, Vec::new(), Some(deepseek_balance(&body)?), now))
}

/// The balance of the answer `body`: of its currencies, the one with the most left (the dollar first among equals),
/// else with none left the dollar's, else the first.
fn deepseek_balance(body: &Value) -> Result<Balance, Failed> {
    let rows: Vec<(String, f64)> = body["balance_infos"]
        .as_array()
        .ok_or(Failed::Unavailable)?
        .iter()
        .filter_map(|row| Some((row["currency"].as_str()?.trim().to_uppercase(), number(&row["total_balance"])?)))
        .filter(|(currency, _)| !currency.is_empty())
        .collect();
    let funded = rows.iter().filter(|(_, amount)| *amount > 0.0).max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| (a.0 == "USD").cmp(&(b.0 == "USD"))));
    let (currency, amount) = funded.or_else(|| rows.iter().find(|(currency, _)| currency == "USD")).or(rows.first()).ok_or(Failed::Unavailable)?;
    Ok(Balance { currency: currency.clone(), amount: Some(*amount), today: None, since: None, spent: None })
}

// Moonshot AI's open platform: its balance, on the platform in China (in yuan) or abroad (in dollars), whichever
// knows the key.

fn moonshot(keys: &[String], now: u64) -> Result<Limit, Failed> {
    let auth = bearer(&keys[0]);
    let mut failed = Failed::Unavailable;
    for (host, currency) in [("https://api.moonshot.cn", "CNY"), ("https://api.moonshot.ai", "USD")] {
        let (status, body) = ask(&format!("{host}/v1/users/me/balance"), &[("Authorization", &auth)], None)?;
        match passed(status, true) {
            Ok(()) => return Ok(limit(Tool::Moonshot, None, Vec::new(), Some(moonshot_balance(&body, currency)?), now)),
            // (A key of the other platform's.)
            Err(Failed::Key) => failed = Failed::Key,
            Err(other) => return Err(other),
        }
    }
    Err(failed)
}

fn moonshot_balance(body: &Value, currency: &str) -> Result<Balance, Failed> {
    let amount = number(&body["data"]["available_balance"]).ok_or(Failed::Unavailable)?;
    Ok(Balance { currency: currency.into(), amount: Some(amount), today: None, since: None, spent: None })
}

// OpenRouter: the account's credits (what was paid in, less what was spent), and the key's own: what it spent today
// and its limit.

fn openrouter(keys: &[String], now: u64) -> Result<Limit, Failed> {
    let auth = bearer(&keys[0]);
    let headers = [("Authorization", auth.as_str()), ("HTTP-Referer", "https://glancepc.com"), ("X-Title", "Glance")];
    let key = ask("https://openrouter.ai/api/v1/key", &headers, None).and_then(|(status, body)| passed(status, true).map(|()| body));
    let credits = ask("https://openrouter.ai/api/v1/credits", &headers, None).and_then(|(status, body)| passed(status, true).map(|()| body));
    match (&key, &credits) {
        (Err(a), Err(b)) => Err(if *a == Failed::Key && *b == Failed::Key {
            Failed::Key
        } else if [a, b].contains(&&Failed::Limited) {
            Failed::Limited
        } else {
            Failed::Unavailable
        }),
        _ => {
            let (windows, plan, today) = key.as_ref().map_or((Vec::new(), None, None), openrouter_key);
            let amount = credits.as_ref().ok().and_then(|body| Some((number(&body["data"]["total_credits"])? - number(&body["data"]["total_usage"])?).max(0.0)));
            Ok(limit(Tool::OpenRouter, plan, windows, Some(Balance { currency: "USD".into(), amount, today, since: None, spent: None }), now))
        }
    }
}

/// The key's limit as a window (how long it lasts, none told: a limit that never starts again, of no length), its
/// plan, and what it spent today (as OpenRouter's day is).
fn openrouter_key(body: &Value) -> (Vec<Window>, Option<String>, Option<f64>) {
    let data = &body["data"];
    let (minutes, used_in) = match data["limit_reset"].as_str() {
        Some("daily") => (1440, "usage_daily"),
        Some("weekly") => (10080, "usage_weekly"),
        Some("monthly") => (43200, "usage_monthly"),
        _ => (0, "usage"),
    };
    let window = number(&data["limit"]).filter(|limit| *limit > 0.0).and_then(|limit| {
        let used = match number(&data["limit_remaining"]) {
            Some(remaining) => limit - remaining,
            None => number(&data[used_in])? + if data["include_byok_in_limit"] == true { number(&data[format!("byok_{used_in}")]).unwrap_or(0.0) } else { 0.0 },
        };
        Some(Window { minutes, used: share(used / limit * 100.0), resets: None })
    });
    let plan = match (data["is_management_key"].as_bool(), data["is_free_tier"].as_bool()) {
        (Some(true), _) => Some("Management".into()),
        (_, Some(true)) => Some("Free".into()),
        _ => None,
    };
    (window.into_iter().collect(), plan, number(&data["usage_daily"]).map(|spent| spent.max(0.0)))
}

// Kimi Code: its plan's windows.

fn kimi(keys: &[String], now: u64) -> Result<Limit, Failed> {
    let (status, body) = ask("https://api.kimi.com/coding/v1/usages", &[("Authorization", &bearer(&keys[0]))], None)?;
    passed(status, true)?;
    let windows = kimi_windows(&body);
    if windows.is_empty() {
        return Err(Failed::Unavailable);
    }
    Ok(limit(Tool::Kimi, None, windows, None, now))
}

/// The windows of the answer `body` (or its `data`): the week's (`usage`), the rate limits' (`limits`, each by its
/// length), the five hours' and the month's (`usages`), each length once, the first told of it kept.
fn kimi_windows(body: &Value) -> Vec<Window> {
    let body = if body["data"].is_object() { &body["data"] } else { body };
    let mut windows: Vec<Window> = Vec::new();
    let mut add = |window: Option<Window>| {
        if let Some(window) = window.filter(|window| !windows.iter().any(|had| had.minutes == window.minutes)) {
            windows.push(window);
        }
    };
    // A detail's share used: of what it is limited to, what was used, else what is not left.
    let detail = |detail: &Value, minutes: u32| -> Option<Window> {
        let limit = number(&detail["limit"]).filter(|limit| *limit > 0.0)?;
        let used = number(&detail["used"]).or_else(|| Some(limit - number(&detail["remaining"])?))?;
        let resets = ["resetTime", "reset_time", "resetAt", "reset_at"].iter().find_map(|key| moment(&detail[*key], 2e10));
        Some(Window { minutes, used: share(used / limit * 100.0), resets })
    };
    if body["usage"].is_object() {
        let hours = body["usage"]["name"].as_str().is_some_and(|name| ["hour", "小时"].iter().any(|word| name.to_lowercase().contains(word)));
        add(detail(&body["usage"], if hours { 300 } else { 10080 }));
    }
    for entry in body["limits"].as_array().into_iter().flatten() {
        let window = &entry["window"];
        let unit = window["timeUnit"].as_str().unwrap_or_default().to_uppercase();
        let by = [("MIN", 1.0), ("HOUR", 60.0), ("DAY", 1440.0), ("WEEK", 10080.0), ("MONTH", 43200.0)].iter().find(|(name, _)| unit.contains(name)).map(|(_, by)| *by);
        let minutes = number(&window["duration"]).zip(by).map(|(duration, by)| (duration * by) as u32).filter(|minutes| *minutes > 0);
        if let Some(minutes) = minutes {
            add(detail(&entry["detail"], minutes));
        }
    }
    let ratio = |pool: &Value, minutes: u32| -> Option<Window> {
        let ratio = number(&pool["used_ratio"]).filter(|ratio| *ratio >= 0.0)?;
        Some(Window { minutes, used: share(ratio * 100.0), resets: moment(&pool["reset_time"], 2e10) })
    };
    add(ratio(&body["usages"]["limit_5h"], 300));
    add(ratio(&body["usages"]["limit_month_total"], 43200));
    windows.sort_by_key(|window| window.minutes);
    windows
}

// Zhipu's GLM Coding Plan: its windows and its plan, and the account's balance, on bigmodel.cn (in yuan) or Z.ai's
// abroad (in dollars), whichever knows the key.

/// Which platform last knew the key (0: bigmodel.cn, 1: Z.ai), asked first.
static GLM_AT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

const GLM: [(&str, &str); 2] = [("https://open.bigmodel.cn", "CNY"), ("https://api.z.ai", "USD")];

fn glm(keys: &[String], now: u64) -> Result<Limit, Failed> {
    use std::sync::atomic::Ordering;
    let auth = bearer(&keys[0]);
    let first = GLM_AT.load(Ordering::Relaxed);
    let mut failed = Failed::Unavailable;
    for at in [first, 1 - first] {
        let (base, currency) = GLM[at];
        let asked = |path: &str| ask(&format!("{base}{path}"), &[("Authorization", &auth)], None).and_then(|(status, body)| {
            passed(status, true)?;
            // (Its gateway answers a key refused with a code of its own.)
            match body["code"].as_i64() {
                Some(401 | 403) => Err(Failed::Key),
                _ => Ok(body),
            }
        });
        match asked("/api/monitor/usage/quota/limit") {
            Ok(quota) => {
                GLM_AT.store(at, Ordering::Relaxed);
                let windows = glm_windows(&quota);
                if windows.is_empty() {
                    return Err(Failed::Unavailable);
                }
                let plan = asked("/api/biz/subscription/list").ok().and_then(|body| glm_plan(&body));
                let amount = asked("/api/biz/account/query-customer-account-report").ok().and_then(|body| number(&body["data"]["availableBalance"]));
                let balance = amount.map(|amount| Balance { currency: currency.into(), amount: Some(amount.max(0.0)), today: None, since: None, spent: None });
                return Ok(limit(Tool::Glm, plan, windows, balance, now));
            }
            Err(Failed::Key) => failed = Failed::Key,
            Err(other) => return Err(other),
        }
    }
    Err(failed)
}

/// The plan's windows in the answer `body`: each of its token limits, by its length (its unit: 5 minutes, 3 hours,
/// 1 days, 6 weeks), how much of it is used (`usage` is what it is limited to), and when it starts again.
fn glm_windows(body: &Value) -> Vec<Window> {
    let mut windows: Vec<Window> = Vec::new();
    for item in body["data"]["limits"].as_array().into_iter().flatten() {
        if item["type"].as_str().is_some_and(|kind| kind != "TOKENS_LIMIT") {
            continue;
        }
        let by = match item["unit"].as_i64() {
            Some(5) => 1.0,
            Some(3) => 60.0,
            Some(1) => 1440.0,
            Some(6) => 10080.0,
            _ => continue,
        };
        let Some(minutes) = number(&item["number"]).map(|number| (number * by) as u32).filter(|minutes| *minutes > 0) else { continue };
        let current = number(&item["currentValue"]);
        let used = match number(&item["usage"]).filter(|limit| *limit > 0.0) {
            Some(limit) => {
                let used = match number(&item["remaining"]) {
                    Some(remaining) => (limit - remaining).max(current.unwrap_or(0.0)),
                    None => match current {
                        Some(current) => current,
                        None => continue,
                    },
                };
                used.clamp(0.0, limit) / limit * 100.0
            }
            None => match number(&item["percentage"]) {
                Some(percentage) => percentage,
                None => continue,
            },
        };
        if !windows.iter().any(|had| had.minutes == minutes) {
            windows.push(Window { minutes, used: share(used), resets: moment(&item["nextResetTime"], 2e10) });
        }
    }
    windows.sort_by_key(|window| window.minutes);
    windows
}

/// The plan's name in the answer of its subscriptions: the one of this period in force, else of this period, else
/// in force, else the first.
fn glm_plan(body: &Value) -> Option<String> {
    let rows = body["data"].as_array()?;
    let current = |row: &&Value| row["inCurrentPeriod"] == true;
    let valid = |row: &&Value| row["status"] == "VALID";
    let row = rows.iter().find(|row| current(row) && valid(row)).or_else(|| rows.iter().find(current)).or_else(|| rows.iter().find(valid)).or(rows.first())?;
    ["productName", "product_name", "planName", "plan_name"].iter().find_map(|key| row[*key].as_str()).map(|name| name.trim().to_string()).filter(|name| !name.is_empty())
}

// MiniMax: its Token Plan's windows, else (no plan) the open platform's balance and what it spent today, on its
// platform in China (in yuan) or abroad (in dollars), whichever knows the key.

/// Which platform last knew the key (0: China's, 1: abroad), asked first.
static MINIMAX_AT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

const MINIMAX: [(&str, &str); 2] = [("https://api.minimaxi.com", "CNY"), ("https://api.minimax.io", "USD")];

fn minimax(keys: &[String], now: u64) -> Result<Limit, Failed> {
    use std::sync::atomic::Ordering;
    let auth = bearer(&keys[0]);
    let first = MINIMAX_AT.load(Ordering::Relaxed);
    let mut failed = Failed::Unavailable;
    for at in [first, 1 - first] {
        let (base, currency) = MINIMAX[at];
        let asked = |path: &str| -> Result<Value, Failed> {
            let (status, body) = ask(&format!("{base}{path}"), &[("Authorization", &auth)], None)?;
            // (A key of the other platform's is told by a code of its own, under any status.)
            match body["base_resp"]["status_code"].as_i64() {
                Some(1004) => return Err(Failed::Key),
                Some(0) | None => {}
                Some(_) => return Err(Failed::Unavailable),
            }
            passed(status, true).map(|()| body)
        };
        // The plan's, as it is asked now, else as it was.
        let plan = asked("/v1/token_plan/remains").map(|body| minimax_windows(&body)).and_then(|windows| match windows.is_empty() {
            true => asked("/v1/api/openplatform/coding_plan/remains").map(|body| minimax_windows(&body)),
            false => Ok(windows),
        });
        match plan {
            Ok(windows) if !windows.is_empty() => {
                MINIMAX_AT.store(at, Ordering::Relaxed);
                return Ok(limit(Tool::MiniMax, Some("Token Plan".into()), windows, None, now));
            }
            Err(Failed::Key) => {
                failed = Failed::Key;
                continue;
            }
            Err(Failed::Limited) => return Err(Failed::Limited),
            // No plan: the balance.
            _ => {}
        }
        match asked("/account/query_balance") {
            Ok(body) => {
                MINIMAX_AT.store(at, Ordering::Relaxed);
                let amount = number(&body["available_amount"]).ok_or(Failed::Unavailable)?;
                let day = super::time::local_date(now);
                let date = format!("{:04}-{:02}-{:02}", day.year, day.month, day.day);
                let today = asked(&format!("/account/amount?page=1&limit=10&aggregate=true&start_date={date}&end_date={date}")).ok().and_then(|body| number(&body["consume_cash_sum"]));
                return Ok(limit(Tool::MiniMax, None, Vec::new(), Some(Balance { currency: currency.into(), amount: Some(amount), today, since: None, spent: None }), now));
            }
            Err(Failed::Key) => failed = Failed::Key,
            Err(other) => return Err(other),
        }
    }
    Err(failed)
}

/// The plan's windows in the answer `body`: of its general models (not its video's or voice's), the interval's and
/// the week's, each from how much of it is left, from when to when.
fn minimax_windows(body: &Value) -> Vec<Window> {
    let rows = body["data"]["model_remains"].as_array().or_else(|| body["model_remains"].as_array());
    let Some(row) = rows.into_iter().flatten().find(|row| row["model_name"] == "general") else { return Vec::new() };
    let window = |left: &str, start: &str, end: &str, status: &str, minutes: u32| -> Option<Window> {
        let left = number(&row[left])?;
        // (A tier not held reads as all of it left.)
        if row[status].as_i64() == Some(3) && left >= 100.0 {
            return None;
        }
        let (start, end) = (moment(&row[start], 1e12), moment(&row[end], 1e12));
        let minutes = start.zip(end).filter(|(start, end)| end > start).map_or(minutes, |(start, end)| ((end - start) / 60_000) as u32);
        Some(Window { minutes, used: share(100.0 - left), resets: end })
    };
    [
        window("current_interval_remaining_percent", "start_time", "end_time", "current_interval_status", 300),
        window("current_weekly_remaining_percent", "weekly_start_time", "weekly_end_time", "current_weekly_status", 10080),
    ]
    .into_iter()
    .flatten()
    .collect()
}

// Volcengine Ark's Coding Plan: its windows and its plan, asked of Volcengine's OpenAPI by the account's access key,
// each request signed (HMAC-SHA256, as Volcengine signs: see `signed`).

const VOLCENGINE_HOST: &str = "open.volcengineapi.com";
const VOLCENGINE_QUERY: &str = "Action=GetCodingPlanUsage&Version=2024-01-01";

fn volcengine(keys: &[String], now: u64) -> Result<Limit, Failed> {
    let region = ["VOLCENGINE_REGION", "VOLC_REGION"].iter().find_map(|name| std::env::var(name).ok().filter(|region| !region.trim().is_empty())).unwrap_or_else(|| "cn-beijing".into());
    let signed = signed(&keys[0], &keys[1], region.trim(), now);
    let headers: Vec<(&str, &str)> = signed.iter().map(|(name, value)| (*name, value.as_str())).collect();
    let (status, body) = ask(&format!("https://{VOLCENGINE_HOST}/?{VOLCENGINE_QUERY}"), &headers, Some(b""))?;
    passed(status, true)?;
    let (windows, plan) = volcengine_plan(&body);
    if windows.is_empty() {
        // (Its key refused, as it tells it in a body of success.)
        let code = body["ResponseMetadata"]["Error"]["Code"].as_str().unwrap_or_default();
        return Err(if ["AccessKey", "Signature", "Credential", "Unauthorized"].iter().any(|word| code.contains(word)) { Failed::Key } else { Failed::Unavailable });
    }
    Ok(limit(Tool::Volcengine, plan, windows, None, now))
}

/// The plan's windows and its name in the answer `body`: each of its quotas by its level (five hours, a week, a
/// month), how much of it is used, and when it starts again.
fn volcengine_plan(body: &Value) -> (Vec<Window>, Option<String>) {
    let result = if body["Result"].is_object() { &body["Result"] } else { &body["result"] };
    let mut windows: Vec<Window> = Vec::new();
    for quota in result["QuotaUsage"].as_array().or_else(|| result["quotaUsage"].as_array()).into_iter().flatten() {
        let level = quota["Level"].as_str().or_else(|| quota["level"].as_str()).unwrap_or_default().to_lowercase();
        let minutes = match level.as_str() {
            "session" | "5-hour" | "five_hour" | "5h" => 300,
            "weekly" | "week" => 10080,
            "monthly" | "month" => 43200,
            _ => continue,
        };
        let Some(used) = number(&quota["Percent"]).or_else(|| number(&quota["percent"])) else { continue };
        let resets = moment(&quota["ResetTimestamp"], 2e10).or_else(|| moment(&quota["resetTimestamp"], 2e10));
        if !windows.iter().any(|had| had.minutes == minutes) {
            windows.push(Window { minutes, used: share(used), resets });
        }
    }
    windows.sort_by_key(|window| window.minutes);
    let plan = ["PlanName", "PlanTier", "ProductName"].iter().find_map(|key| result[*key].as_str()).map(|name| {
        let name = name.trim_start_matches("PLAN_TIER_");
        // (Its tiers written in capitals: as a name is.)
        if name.chars().all(|c| c.is_ascii_uppercase() || c == '_') { title_case(&name.replace('_', " ")) } else { name.to_string() }
    });
    (windows, plan.filter(|name| !name.is_empty()))
}

fn title_case(words: &str) -> String {
    words
        .split(' ')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or(String::new(), |first| first.to_uppercase().chain(chars.flat_map(char::to_lowercase)).collect())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The headers of the Coding Plan's request, as asked at `now` by access key `access` (its secret `secret`) of
/// `region`: signed as Volcengine signs (its V4, HMAC-SHA256), its body empty and its host signed, not sent.
fn signed(access: &str, secret: &str, region: &str, now: u64) -> Vec<(&'static str, String)> {
    const CONTENT: &str = "application/x-www-form-urlencoded; charset=utf-8";
    let day = crate::todo::Day::from_number((now / super::DAY_MS) as i64);
    let seconds = now / 1000 % 86_400;
    let date = format!("{:04}{:02}{:02}", day.year, day.month, day.day);
    let at = format!("{date}T{:02}{:02}{:02}Z", seconds / 3600, seconds / 60 % 60, seconds % 60);
    let body = hex(&sha256(b""));
    let request = format!("POST\n/\n{VOLCENGINE_QUERY}\ncontent-type:{CONTENT}\nhost:{VOLCENGINE_HOST}\nx-content-sha256:{body}\nx-date:{at}\n\ncontent-type;host;x-content-sha256;x-date\n{body}");
    let scope = format!("{date}/{region}/ark/request");
    let to_sign = format!("HMAC-SHA256\n{at}\n{scope}\n{}", hex(&sha256(request.as_bytes())));
    let key = [region.as_bytes(), b"ark", b"request"].iter().fold(hmac(secret.as_bytes(), date.as_bytes()), |key, part| hmac(&key, part));
    let signature = hex(&hmac(&key, to_sign.as_bytes()));
    vec![
        ("Content-Type", CONTENT.into()),
        ("X-Date", at),
        ("X-Content-Sha256", body),
        ("Authorization", format!("HMAC-SHA256 Credential={access}/{scope}, SignedHeaders=content-type;host;x-content-sha256;x-date, Signature={signature}")),
    ]
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    use windows::Win32::Security::Cryptography::{BCryptHash, BCRYPT_SHA256_ALG_HANDLE};
    let mut digest = [0u8; 32];
    let _ = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, bytes, &mut digest) };
    digest
}

fn hmac(key: &[u8], bytes: &[u8]) -> [u8; 32] {
    use windows::Win32::Security::Cryptography::{BCryptHash, BCRYPT_HMAC_SHA256_ALG_HANDLE};
    let mut digest = [0u8; 32];
    let _ = unsafe { BCryptHash(BCRYPT_HMAC_SHA256_ALG_HANDLE, Some(key), bytes, &mut digest) };
    digest
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Volcengine's request signed as token-monitor's test of it has it (its access key `ak`, its secret `sk`).
    #[test]
    fn signs_volcengines_request_as_volcengine_does() {
        // 2026-07-06T00:00:00Z.
        let at = crate::todo::Day { year: 2026, month: 7, day: 6 }.number() as u64 * super::super::DAY_MS;
        let headers = signed("ak", "sk", "cn-beijing", at);
        let auth = &headers.iter().find(|(name, _)| *name == "Authorization").unwrap().1;
        assert!(auth.starts_with("HMAC-SHA256 Credential=ak/20260706/cn-beijing/ark/request, "), "{auth}");
        assert!(auth.ends_with("Signature=68ffeb3bf30b0dc6b2bf535890a9dea4390245346b12a8035b940b83043cf7c0"), "{auth}");
        assert_eq!(headers.iter().find(|(name, _)| *name == "X-Date").unwrap().1, "20260706T000000Z");
    }

    /// DeepSeek's balance: the currency with the most left, else the dollar's.
    #[test]
    fn reads_deepseeks_balance() {
        let body = json!({"is_available": true, "balance_infos": [
            {"currency": "CNY", "total_balance": "110.00", "granted_balance": "10.00", "topped_up_balance": "100.00"},
            {"currency": "USD", "total_balance": "5.00", "granted_balance": "0.00", "topped_up_balance": "5.00"}]});
        let balance = deepseek_balance(&body).unwrap();
        assert_eq!((balance.currency.as_str(), balance.amount), ("CNY", Some(110.0)));
        let none_left = json!({"balance_infos": [{"currency": "CNY", "total_balance": "0"}, {"currency": "USD", "total_balance": "0"}]});
        assert_eq!(deepseek_balance(&none_left).unwrap().currency, "USD");
        assert_eq!(deepseek_balance(&json!({"error": {}})), Err(Failed::Unavailable));
    }

    #[test]
    fn reads_moonshots_balance() {
        let body = json!({"code": 0, "data": {"available_balance": 49.58894, "voucher_balance": 46.58893, "cash_balance": 3.00001}, "scode": "0x0", "status": true});
        assert_eq!(moonshot_balance(&body, "CNY").unwrap().amount, Some(49.58894));
    }

    /// OpenRouter's key: its limit's share used (from what is left of it, else from what it spent in the limit's
    /// period), its plan, and what it spent today.
    #[test]
    fn reads_openrouters_key() {
        let body = json!({"data": {"usage": 12.5, "usage_daily": 0.75, "usage_weekly": 3.0, "usage_monthly": 9.0, "limit": 20, "limit_remaining": 15, "limit_reset": "weekly", "is_free_tier": false}});
        let (windows, plan, today) = openrouter_key(&body);
        assert_eq!((windows, plan, today), (vec![Window { minutes: 10080, used: 25.0, resets: None }], None, Some(0.75)));
        let body = json!({"data": {"usage": 4.0, "usage_daily": 0, "limit": 8, "limit_reset": null, "is_free_tier": true}});
        assert_eq!(openrouter_key(&body), (vec![Window { minutes: 0, used: 50.0, resets: None }], Some("Free".into()), Some(0.0)));
        assert!(openrouter_key(&json!({"data": {"usage": 4.0, "limit": null}})).0.is_empty());
    }

    /// Kimi Code's windows: the week's, the rate limit's by its length, the five hours' and the month's by their
    /// shares (0 to 1), each length once.
    #[test]
    fn reads_kimis_windows() {
        let body = json!({
            "usage": {"limit": "2048", "used": "214", "remaining": "1834", "resetTime": "2026-07-14T00:00:00Z"},
            "limits": [{"window": {"duration": 300, "timeUnit": "TIME_UNIT_MINUTE"}, "detail": {"limit": "200", "remaining": "50", "resetTime": "2026-07-08T05:00:00Z"}}],
            "usages": {"limit_5h": {"used_ratio": 0.9, "reset_time": "2026-07-08T05:00:00Z"}, "limit_month_total": {"used_ratio": 0.0338, "reset_time": "2026-08-01T00:00:00Z"}}});
        let windows = kimi_windows(&body);
        let shown: Vec<(u32, i32)> = windows.iter().map(|window| (window.minutes, (window.used * 100.0).round() as i32)).collect();
        assert_eq!(shown, [(300, 7500), (10080, 1045), (43200, 338)]);
        assert_eq!(windows[1].resets, super::super::moment("2026-07-14T00:00:00Z"));
        let wrapped = json!({"data": {"usages": {"limit_5h": {"used_ratio": 1.4}}}});
        assert_eq!(kimi_windows(&wrapped), [Window { minutes: 300, used: 100.0, resets: None }]);
    }

    /// GLM's windows: each token limit by its unit and number, `usage` what it is limited to; its tools' calls not
    /// among them. Its plan, of this period's subscription in force.
    #[test]
    fn reads_glms_windows_and_plan() {
        let body = json!({"code": 200, "data": {"limits": [
            {"type": "TOKENS_LIMIT", "unit": 3, "number": 5, "usage": 800, "remaining": 200, "currentValue": 600, "nextResetTime": 1_780_297_200_000u64},
            {"type": "TOKENS_LIMIT", "unit": 6, "number": 1, "usage": 0, "percentage": 12},
            {"type": "TIME_LIMIT", "unit": 5, "number": 1, "usage": 100, "currentValue": 3}]}});
        assert_eq!(glm_windows(&body), [Window { minutes: 300, used: 75.0, resets: Some(1_780_297_200_000) }, Window { minutes: 10080, used: 12.0, resets: None }]);
        let plans = json!({"data": [{"productName": "GLM Coding Lite", "status": "EXPIRED", "inCurrentPeriod": false}, {"productName": "GLM Coding Pro", "status": "VALID", "inCurrentPeriod": true}]});
        assert_eq!(glm_plan(&plans).as_deref(), Some("GLM Coding Pro"));
    }

    /// MiniMax's windows: its general models' interval and week, from how much is left, the interval's length from
    /// when to when; a tier not held left out; its video's not read.
    #[test]
    fn reads_minimaxs_windows() {
        let body = json!({"base_resp": {"status_code": 0}, "data": {"model_remains": [
            {"model_name": "video", "current_interval_remaining_percent": "10"},
            {"model_name": "general", "current_interval_remaining_percent": "96", "start_time": 1_780_279_200_000u64, "end_time": 1_780_297_200_000u64,
             "current_weekly_remaining_percent": "100", "current_weekly_status": 3, "weekly_start_time": 1_780_243_200_000u64, "weekly_end_time": 1_780_848_000_000u64}]}});
        assert_eq!(minimax_windows(&body), [Window { minutes: 300, used: 4.0, resets: Some(1_780_297_200_000) }]);
        assert!(minimax_windows(&json!({"data": {}})).is_empty());
    }

    /// Volcengine's plan: each quota by its level, its reset in seconds; its tier as a name.
    #[test]
    fn reads_volcengines_plan() {
        let body = json!({"ResponseMetadata": {}, "Result": {"Status": "ACTIVE", "PlanTier": "PLAN_TIER_PRO", "QuotaUsage": [
            {"Level": "weekly", "Percent": 41.5, "ResetTimestamp": 1_780_848_000},
            {"Level": "session", "Percent": 3, "ResetTimestamp": 1_780_297_200},
            {"Level": "daily", "Percent": 9}]}});
        let (windows, plan) = volcengine_plan(&body);
        assert_eq!(windows, [Window { minutes: 300, used: 3.0, resets: Some(1_780_297_200_000) }, Window { minutes: 10080, used: 41.5, resets: Some(1_780_848_000_000) }]);
        assert_eq!(plan.as_deref(), Some("Pro"));
    }
}
