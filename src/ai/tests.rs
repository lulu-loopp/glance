use super::*;
use std::io::Write as _;

/// A folder of this test's own (made empty).
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glance-ai-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn append(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap().write_all(text.as_bytes()).unwrap();
}

/// The moment `ms` (since 1970) as the tools write it ("2026-11-18T12:00:00.000Z").
pub(super) fn stamp(ms: u64) -> String {
    let (days, of_day) = ((ms / DAY_MS) as i64, ms % DAY_MS);
    let date = crate::todo::Day::from_number(days);
    let (hour, minute, second, milli) = (of_day / 3_600_000, of_day / 60_000 % 60, of_day / 1000 % 60, of_day % 1000);
    format!("{:04}-{:02}-{:02}T{hour:02}:{minute:02}:{second:02}.{milli:03}Z", date.year, date.month, date.day)
}

/// 2026-11-18 12:00 UTC.
const NOON: u64 = 1_795_003_200_000;

/// A line of Claude Code's: the model's response `id` to request `request` in `session` at `time`, as it spent.
fn claude(session: &str, id: &str, request: &str, time: &str, model: &str, input: u64, output: u64, read: u64, write: u64) -> String {
    format!(
        r#"{{"type":"assistant","sessionId":"{session}","requestId":"{request}","timestamp":"{time}","cwd":"E:\\work","message":{{"id":"{id}","model":"{model}","usage":{{"input_tokens":{input},"output_tokens":{output},"cache_creation_input_tokens":{write},"cache_read_input_tokens":{read},"cache_creation":{{"ephemeral_5m_input_tokens":0}}}}}}}}"#
    ) + "\n"
}

fn spent(ledger: &Ledger) -> Vec<(Tool, String, Tokens)> {
    ledger.spent_between(0, u64::MAX / 2)
}

#[test]
fn counts_each_claude_response_once_however_many_lines_and_files_hold_it() {
    let dir = folder("claude-once");
    let a = dir.join("projects").join("E--work").join("one.jsonl");
    // (A response written in two lines, each with its whole usage, and another in a subagent's file.)
    append(&a, &claude("s1", "msg_1", "req_1", "2026-11-18T11:00:00.000Z", "claude-opus-5-5", 10, 20, 300, 40));
    append(&a, &claude("s1", "msg_1", "req_1", "2026-11-18T11:00:01.000Z", "claude-opus-5-5", 10, 20, 300, 40));
    append(&a, &claude("s1", "msg_2", "req_2", "2026-11-18T11:20:00.000Z", "claude-opus-5-5", 1, 2, 3, 4));
    // (Claude Code's own error, shown as a response: it spent nothing.)
    append(&a, &claude("s1", "msg_3", "req_3", "2026-11-18T11:21:00.000Z", "<synthetic>", 0, 0, 0, 0));
    // (A resumed session: what came before copied into its own file.)
    let b = dir.join("projects").join("E--work").join("two.jsonl");
    append(&b, &claude("s1", "msg_1", "req_1", "2026-11-18T11:00:00.000Z", "claude-opus-5-5", 10, 20, 300, 40));
    append(&b, &claude("s2", "msg_4", "req_4", "2026-11-18T11:40:00.000Z", "claude-sonnet-5-5", 5, 6, 7, 8));
    let mut ledger = Ledger::default();
    assert!(ledger.refresh(&[(Tool::Claude, dir.join("projects"))], NOON));
    assert_eq!(
        spent(&ledger),
        vec![
            (Tool::Claude, "claude-opus-5-5".to_string(), Tokens { input: 11, output: 22, cache_read: 303, cache_write: 44, reasoning: 0 }),
            (Tool::Claude, "claude-sonnet-5-5".to_string(), Tokens { input: 5, output: 6, cache_read: 7, cache_write: 8, reasoning: 0 }),
        ]
    );
    let sessions = ledger.sessions_since(0);
    assert_eq!(sessions.iter().map(|session| (session.id.as_str(), session.folder.as_deref(), session.spent_between(0, NOON))).collect::<Vec<_>>(), [("s2", Some("E:\\work"), 26), ("s1", Some("E:\\work"), 380)]);
    // (Read again unchanged: nothing new.)
    assert!(!ledger.refresh(&[(Tool::Claude, dir.join("projects"))], NOON));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reads_only_whole_lines_and_what_was_added_since() {
    let dir = folder("claude-grows");
    let file = dir.join("p").join("s.jsonl");
    let line = claude("s1", "msg_1", "req_1", "2026-11-18T11:00:00Z", "claude-opus-5-5", 1, 1, 1, 1);
    let (head, tail) = line.split_at(40);
    append(&file, head);
    let roots = [(Tool::Claude, dir.clone())];
    let mut ledger = Ledger::default();
    ledger.refresh(&roots, NOON);
    assert!(spent(&ledger).is_empty(), "a line half written is not read");
    append(&file, tail);
    assert!(ledger.refresh(&roots, NOON));
    assert_eq!(spent(&ledger)[0].2.total(), 4);
    append(&file, &claude("s1", "msg_2", "req_2", "2026-11-18T11:05:00Z", "claude-opus-5-5", 2, 2, 2, 2));
    assert!(ledger.refresh(&roots, NOON));
    assert_eq!(spent(&ledger)[0].2.total(), 12);
    // (Written anew, shorter: read from its start, what was counted not counted again.)
    std::fs::write(&file, claude("s1", "msg_2", "req_2", "2026-11-18T11:05:00Z", "claude-opus-5-5", 2, 2, 2, 2)).unwrap();
    ledger.refresh(&roots, NOON);
    assert_eq!(spent(&ledger)[0].2.total(), 12);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn names_a_claude_session_by_hand_before_by_summary_before_by_what_was_asked() {
    let dir = folder("claude-titles");
    let file = dir.join("p").join("s.jsonl");
    // (A summary, written before any line names the session: the file's.)
    append(&file, "{\"type\":\"summary\",\"summary\":\"Sensor fixes\",\"leafUuid\":\"x\"}\n");
    append(&file, "{\"type\":\"last-prompt\",\"sessionId\":\"s1\",\"lastPrompt\":\"\\n  fix the fan curve\\nplease\"}\n");
    append(&file, &claude("s1", "msg_1", "req_1", "2026-11-18T11:00:00Z", "claude-opus-5-5", 1, 1, 1, 1));
    let roots = [(Tool::Claude, dir.clone())];
    let mut ledger = Ledger::default();
    ledger.refresh(&roots, NOON);
    assert_eq!(ledger.sessions_since(0)[0].title.as_deref(), Some("Sensor fixes"));
    append(&file, "{\"type\":\"custom-title\",\"sessionId\":\"s1\",\"customTitle\":\"Fans\"}\n");
    append(&file, "{\"type\":\"last-prompt\",\"sessionId\":\"s1\",\"lastPrompt\":\"and the pump\"}\n");
    ledger.refresh(&roots, NOON);
    assert_eq!(ledger.sessions_since(0)[0].title.as_deref(), Some("Fans"));
    let _ = std::fs::remove_dir_all(dir);
}

/// Codex's lines: a session's start, a turn's model, and what a response spent with the limits as they stood.
fn codex_meta(session: &str) -> String {
    format!(r#"{{"timestamp":"2026-11-18T10:00:00.000Z","type":"session_meta","payload":{{"id":"{session}","cwd":"D:\\code","cli_version":"0.99.0"}}}}"#) + "\n"
}

fn codex_turn(model: &str) -> String {
    format!(r#"{{"timestamp":"2026-11-18T10:00:01.000Z","type":"turn_context","payload":{{"model":"{model}","cwd":"D:\\code","effort":"high"}}}}"#) + "\n"
}

fn codex_count(time: &str, total: u64, input: u64, cached: u64, output: u64, reasoning: u64, used: f32, resets: u64) -> String {
    format!(
        r#"{{"timestamp":"{time}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":0,"cached_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0,"total_tokens":{total}}},"last_token_usage":{{"input_tokens":{input},"cached_input_tokens":{cached},"cache_write_input_tokens":0,"output_tokens":{output},"reasoning_output_tokens":{reasoning},"total_tokens":{}}},"model_context_window":258400}},"rate_limits":{{"limit_id":"codex","limit_name":null,"primary":{{"used_percent":{used},"window_minutes":300,"resets_at":{resets}}},"secondary":{{"used_percent":40.0,"window_minutes":10080,"resets_at":1796000000}},"plan_type":"plus","credits":null}}}}}}"#,
        input + output
    ) + "\n"
}

#[test]
fn counts_codex_responses_by_their_last_usage_once_with_the_latest_limits() {
    let dir = folder("codex");
    let file = dir.join("sessions").join("2026").join("11").join("18").join("rollout-a.jsonl");
    append(&file, &codex_meta("t1"));
    append(&file, &codex_turn("gpt-5.5-codex"));
    append(&file, &codex_count("2026-11-18T10:01:00.000Z", 1100, 1000, 800, 100, 30, 12.5, 1795530000));
    // (Told again unchanged: spent nothing new.)
    append(&file, &codex_count("2026-11-18T10:01:02.000Z", 1100, 1000, 800, 100, 30, 12.5, 1795530000));
    append(&file, &codex_count("2026-11-18T10:30:00.000Z", 1650, 500, 500, 50, 0, 20.0, 1795530000));
    // (A session forked from it starts with a copy of what it spent.)
    let fork = dir.join("sessions").join("2026").join("11").join("18").join("rollout-b.jsonl");
    append(&fork, &codex_meta("t2"));
    append(&fork, &codex_turn("gpt-5.5-codex"));
    append(&fork, &codex_count("2026-11-18T10:01:00.000Z", 1100, 1000, 800, 100, 30, 12.5, 1795530000));
    append(&fork, &codex_count("2026-11-18T11:00:00.000Z", 1300, 200, 0, 0, 0, 25.0, 1795530000));
    // (The names of the sessions, renamed later.)
    let index = dir.join("session_index.jsonl");
    append(&index, "{\"id\":\"t1\",\"thread_name\":\"Port the sensors\",\"updated_at\":\"2026-11-18T10:00:00Z\"}\n");
    append(&index, "{\"id\":\"t1\",\"thread_name\":\"Port the sensors to macOS\",\"updated_at\":\"2026-11-18T10:40:00Z\"}\n");
    let mut ledger = Ledger::default();
    ledger.refresh(&[(Tool::Codex, dir.join("sessions")), (Tool::Codex, index)], NOON);
    assert_eq!(spent(&ledger), vec![(Tool::Codex, "gpt-5.5-codex".to_string(), Tokens { input: 200 + 0 + 200, output: 150, cache_read: 1300, cache_write: 0, reasoning: 30 })]);
    let sessions = ledger.sessions_since(0);
    assert_eq!(sessions.iter().map(|session| (session.id.as_str(), session.title.as_deref(), session.spent_between(0, NOON))).collect::<Vec<_>>(), [("t2", None, 200), ("t1", Some("Port the sensors to macOS"), 1650)]);
    assert_eq!(
        ledger.limits(),
        [Limit {
            tool: Tool::Codex,
            id: "codex".to_string(),
            name: None,
            plan: Some("plus".to_string()),
            windows: vec![Window { minutes: 300, used: 25.0, resets: Some(1_795_530_000_000) }, Window { minutes: 10080, used: 40.0, resets: Some(1_796_000_000_000) }],
            balance: None,
            told: moment("2026-11-18T11:00:00Z").unwrap(),
        }]
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn keeps_what_was_read_from_one_run_to_the_next() {
    let dir = folder("kept");
    let records = dir.join("records");
    let file = records.join("p").join("s.jsonl");
    append(&file, &claude("s1", "msg_1", "req_1", "2026-11-18T11:00:00Z", "claude-opus-5-5", 1, 2, 3, 4));
    append(&file, "{\"type\":\"custom-title\",\"sessionId\":\"s1\",\"customTitle\":\"Fans\"}\n");
    let roots = [(Tool::Claude, records.clone())];
    let mut ledger = Ledger::default();
    ledger.refresh(&roots, NOON);
    ledger.limits.push(Limit { tool: Tool::Codex, id: "codex".to_string(), name: None, plan: None, windows: vec![Window { minutes: 300, used: 1.5, resets: None }], balance: None, told: NOON });
    ledger.save(&dir).unwrap();
    let mut again = Ledger::load(&dir);
    assert_eq!(spent(&again), spent(&ledger));
    assert_eq!(again.sessions_since(0), ledger.sessions_since(0));
    assert_eq!(again.limits(), ledger.limits());
    assert!(!again.refresh(&roots, NOON), "nothing read again");
    // (The same response in another file, read after: counted already.)
    append(&records.join("p").join("t.jsonl"), &claude("s1", "msg_1", "req_1", "2026-11-18T11:00:00Z", "claude-opus-5-5", 1, 2, 3, 4));
    again.refresh(&roots, NOON);
    assert_eq!(spent(&again)[0].2.total(), 10);
    // (A name given by hand is not taken over by what was asked later.)
    append(&file, "{\"type\":\"last-prompt\",\"sessionId\":\"s1\",\"lastPrompt\":\"later\"}\n");
    again.refresh(&roots, NOON);
    assert_eq!(again.sessions_since(0)[0].title.as_deref(), Some("Fans"));
    // (Kept unreadably: read afresh.)
    std::fs::write(dir.join(FILE), b"{not json\n").unwrap();
    assert!(spent(&Ledger::load(&dir)).is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn lets_go_of_what_was_spent_before_the_days_kept() {
    let dir = folder("forget");
    let file = dir.join("p").join("s.jsonl");
    append(&file, &claude("s1", "msg_1", "req_1", "2026-10-01T11:00:00Z", "claude-opus-5-5", 1, 0, 0, 0));
    append(&file, &claude("s2", "msg_2", "req_2", "2026-11-17T11:00:00Z", "claude-opus-5-5", 2, 0, 0, 0));
    std::fs::File::options().write(true).open(&file).unwrap().set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_millis(NOON)).unwrap();
    let roots = [(Tool::Claude, dir.clone())];
    let mut ledger = Ledger::default();
    ledger.refresh(&roots, NOON);
    assert_eq!(spent(&ledger)[0].2.total(), 2, "spent before the days kept is not counted");
    assert_eq!(ledger.sessions_since(0).len(), 1);
    // (A month on, with the file not written to since: let go of whole.)
    assert!(ledger.refresh(&roots, NOON + 40 * DAY_MS));
    assert!(spent(&ledger).is_empty());
    assert!(ledger.sessions_since(0).is_empty());
    assert!(ledger.files.is_empty() && ledger.counted.is_empty());
    ledger.limits.push(Limit { tool: Tool::Codex, id: "codex".to_string(), name: None, plan: None, windows: vec![Window { minutes: 300, used: 1.5, resets: None }], balance: None, told: NOON });
    assert!(ledger.refresh(&roots, NOON + 40 * DAY_MS));
    assert!(ledger.limits().is_empty(), "a limit told before the days kept is let go of");
    let codex = dir.join("codex").join("rollout.jsonl");
    append(&codex, &codex_meta("t1"));
    append(&codex, &codex_turn("gpt-5.5-codex"));
    append(&codex, &codex_count("2026-11-18T10:01:00.000Z", 1100, 1000, 800, 100, 30, 12.5, 1795530000));
    // (Written to within the days kept, as a session's log is that went on: read.)
    std::fs::File::options().write(true).open(&codex).unwrap().set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_millis(NOON + 40 * DAY_MS)).unwrap();
    ledger.refresh(&[(Tool::Codex, dir.join("codex"))], NOON + 40 * DAY_MS);
    assert_eq!(ledger.files.len(), 1);
    assert!(ledger.limits().is_empty(), "nor taken in as it is read");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn tells_what_was_spent_by_quarter_hour() {
    let dir = folder("quarters");
    let file = dir.join("p").join("s.jsonl");
    append(&file, &claude("s1", "msg_1", "req_1", "2026-11-18T11:00:00Z", "claude-opus-5-5", 1, 0, 0, 0));
    append(&file, &claude("s1", "msg_2", "req_2", "2026-11-18T11:14:59Z", "claude-opus-5-5", 2, 0, 0, 0));
    append(&file, &claude("s1", "msg_3", "req_3", "2026-11-18T11:45:00Z", "claude-opus-5-5", 4, 0, 0, 0));
    let mut ledger = Ledger::default();
    ledger.refresh(&[(Tool::Claude, dir.clone())], NOON);
    let eleven = moment("2026-11-18T11:00:00Z").unwrap();
    assert_eq!(ledger.by_quarter(None, eleven, NOON), [3, 0, 0, 4]);
    assert_eq!(ledger.by_quarter(Some(Tool::Codex), eleven, NOON), [0, 0, 0, 0]);
    assert_eq!(ledger.spent_between(eleven + QUARTER_MS, NOON)[0].2.total(), 4);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reads_the_times_records_are_written_in() {
    assert_eq!(moment("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(moment("2026-11-18T12:00:00.000Z"), Some(NOON));
    assert_eq!(moment("2026-11-18T12:00:00.1234567Z"), Some(NOON + 123));
    assert_eq!(moment("2026-11-18T20:00:00+08:00"), Some(NOON));
    assert_eq!(moment("2026-11-18T07:30:00.5-04:30"), Some(NOON + 500));
    assert_eq!(moment("2026-11-18"), None);
    assert_eq!(moment("2026-11-18T12:00:00 UTC"), None);
    for ms in [0, NOON + 1, NOON + 59 * 60_000 + 999, 951_782_400_000] {
        assert_eq!(moment(&stamp(ms)), Some(ms));
    }
    assert_eq!(moment("2024-03-01T00:00:00Z").unwrap() - moment("2024-02-28T00:00:00Z").unwrap(), 2 * DAY_MS);
}

#[test]
fn begins_the_day_at_this_pcs_midnight() {
    let start = today_start(NOON);
    assert!(start <= NOON && NOON - start < DAY_MS);
    assert_eq!(today_start(start), start);
    assert_eq!(today_start(start + DAY_MS - 1), start);
    // (Summer time begins or ends on some days: the next day begins within an hour of a day after.)
    let next = today_start(start + DAY_MS + 2 * 60 * 60 * 1000);
    assert!(next.abs_diff(start + DAY_MS) <= 60 * 60 * 1000);
}

/// This PC's own records read (into a folder of the test's own), as Glance would as it starts and again a minute on:
/// how long each took, and what was spent today and this week by tool and model, with the limits, for comparing with
/// what the tools show. Nothing a session says (its title, its folder) is shown.
#[test]
#[ignore = "reads this PC's records: run by hand"]
fn reads_this_pcs_records() {
    let dir = folder("this-pc");
    let roots = roots();
    let now = crate::station::now_ms();
    let mut ledger = Ledger::default();
    let first = std::time::Instant::now();
    ledger.refresh(&roots, now);
    let first = first.elapsed();
    ledger.save(&dir).unwrap();
    let kept = std::fs::metadata(dir.join(FILE)).unwrap().len();
    let again = std::time::Instant::now();
    let mut loaded = Ledger::load(&dir);
    loaded.refresh(&roots, crate::station::now_ms());
    let again = again.elapsed();
    let usage = ledger.usage(now);
    println!("usage: {} today by model, days {:?}, {} quarters today, {} sessions", usage.spent_today.len(), usage.days, usage.quarters.first().map_or(0, |(_, quarters)| quarters.len()), usage.sessions.len());
    println!("read whole {first:?}, kept {kept} bytes, loaded and read again {again:?}, {} files", ledger.files.len());
    let today = today_start(now);
    for (name, from) in [("today", today), ("week", today - 6 * DAY_MS)] {
        for (tool, model, tokens) in ledger.spent_between(from, now) {
            println!("{name} {tool:?} {model}: {tokens:?} total {}", tokens.total());
        }
        let sessions = ledger.sessions_since(from);
        for tool in [Tool::Claude, Tool::Codex] {
            let of: Vec<_> = sessions.iter().filter(|session| session.tool == tool).collect();
            println!("{name} {tool:?} sessions {}, titled {}, with a folder {}", of.len(), of.iter().filter(|session| session.title.is_some()).count(), of.iter().filter(|session| session.folder.is_some()).count());
        }
    }
    for limit in ledger.limits() {
        println!("limit {} plan {:?} windows {:?} told {} min ago", limit.id, limit.plan, limit.windows.iter().map(|window| (window.minutes, window.used)).collect::<Vec<_>>(), (now - limit.told) / 60_000);
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// What a provider's account spent today, counted from one reading to the next where it does not tell it: what its
/// balance fell by (a rise is money paid in, counted on from), from the day's first reading, then from the day's
/// start; what it tells of today kept as it is; a reading failed keeping what was read, told as failed; its key taken
/// away, all of it let go of.
#[test]
fn counts_what_an_account_spent_today_from_its_balance() {
    const HOUR: u64 = 3_600_000;
    let read = |amount: f64, today: Option<f64>, at: u64| -> Result<Limit, Failed> {
        let balance = Balance { currency: "CNY".into(), amount: Some(amount), today, since: None, spent: None };
        Ok(Limit { tool: Tool::DeepSeek, id: "deepseek".into(), name: None, plan: None, windows: Vec::new(), balance: Some(balance), told: at })
    };
    let shown = |ledger: &Ledger| ledger.limits[0].balance.as_ref().map(|balance| (balance.today, balance.since)).unwrap();
    let mut ledger = Ledger::default();
    assert!(ledger.read_of(Tool::DeepSeek, read(100.0, None, NOON), NOON));
    assert_eq!(shown(&ledger), (Some(0.0), Some(NOON)));
    ledger.read_of(Tool::DeepSeek, read(97.5, None, NOON + HOUR), NOON + HOUR);
    ledger.read_of(Tool::DeepSeek, read(147.5, None, NOON + 2 * HOUR), NOON + 2 * HOUR);
    ledger.read_of(Tool::DeepSeek, read(146.0, None, NOON + 3 * HOUR), NOON + 3 * HOUR);
    assert_eq!(shown(&ledger), (Some(4.0), Some(NOON)));
    // Failed: what was read kept, and told as failed till it is read again.
    assert!(ledger.read_of(Tool::DeepSeek, Err(Failed::Key), NOON + 4 * HOUR));
    assert!(!ledger.read_of(Tool::DeepSeek, Err(Failed::Key), NOON + 4 * HOUR));
    assert_eq!((shown(&ledger), ledger.usage(NOON + 4 * HOUR).failed), ((Some(4.0), Some(NOON)), vec![(Tool::DeepSeek, Failed::Key)]));
    // The next day, counted from its start.
    ledger.read_of(Tool::DeepSeek, read(145.0, None, NOON + DAY_MS), NOON + DAY_MS);
    assert_eq!((shown(&ledger), ledger.usage(NOON + DAY_MS).failed), ((Some(1.0), None), vec![]));
    // Kept from one run to the next.
    let again = Ledger::from_bytes(&ledger.to_bytes()).unwrap();
    assert_eq!(again.paid, ledger.paid);
    // Told by the provider: as it is told.
    ledger.read_of(Tool::DeepSeek, read(140.0, Some(7.25), NOON + DAY_MS + HOUR), NOON + DAY_MS + HOUR);
    assert_eq!(shown(&ledger), (Some(7.25), None));
    // Its key taken away: what was read forgotten, and told as unkeyed (once) till it is read again.
    assert!(ledger.unkeyed(Tool::DeepSeek) && ledger.limits.is_empty() && ledger.paid.is_empty());
    assert!(!ledger.unkeyed(Tool::DeepSeek));
    assert_eq!(ledger.usage(NOON + DAY_MS).unkeyed, vec![Tool::DeepSeek]);
    assert!(ledger.read_of(Tool::DeepSeek, Err(Failed::Unavailable), NOON + DAY_MS + 2 * HOUR));
    assert!(ledger.usage(NOON + DAY_MS).unkeyed.is_empty());
}

/// A line of Kimi Code's: what agent `agent` of a session spent at `at` (ms), as its record writes it.
fn kimi(agent: &str, at: u64, scope: &str, input: u64, output: u64, read: u64, write: u64) -> String {
    format!(r#"{{"type":"usage.record","agentId":"{agent}","model":"kimi-for-coding","usage":{{"inputOther":{input},"output":{output},"inputCacheRead":{read},"inputCacheCreation":{write}}},"usageScope":"{scope}","time":{at}}}"#) + "\n"
}

/// Kimi Code's sessions: each request's spending counted once (its main agent's and its subagents', each in a record of
/// its own), the session known by its folder, its folder by its agent's binding, its title by what the user asked
/// last (not by a subagent's task).
#[test]
fn reads_kimi_codes_sessions_by_their_agents_records() {
    let dir = folder("kimi");
    let session = dir.join("sessions").join("wd_glance_1a2b").join("session_k1");
    let main = session.join("agents").join("main").join("wire.jsonl");
    append(&main, "{\"type\":\"metadata\",\"protocol_version\":\"2\",\"created_at\":1}\n");
    append(&main, "{\"type\":\"profile.bind\",\"agentId\":\"main\",\"environmentDisclosure\":{\"cwd\":\"E:\\\\work\"},\"time\":1}\n");
    append(&main, "{\"type\":\"turn.prompt\",\"agentId\":\"main\",\"input\":[{\"type\":\"text\",\"text\":\"tidy the sensors\"}],\"origin\":{\"kind\":\"user\"},\"time\":2}\n");
    append(&main, &kimi("main", NOON - 3_600_000, "turn", 100, 20, 3_000, 400));
    append(&main, &kimi("main", NOON - 3_000_000, "session", 5, 1, 0, 0));
    let sub = session.join("agents").join("agent-0").join("wire.jsonl");
    append(&sub, "{\"type\":\"turn.prompt\",\"agentId\":\"agent-0\",\"input\":[{\"type\":\"text\",\"text\":\"read the fan module\"}],\"origin\":{\"kind\":\"system_trigger\"},\"time\":3}\n");
    append(&sub, &kimi("agent-0", NOON - 3_500_000, "turn", 10, 2, 300, 0));
    let roots = [(Tool::Kimi, dir.join("sessions"))];
    let mut ledger = Ledger::default();
    assert!(ledger.refresh(&roots, NOON));
    assert_eq!(spent(&ledger), vec![(Tool::Kimi, "kimi-for-coding".to_string(), Tokens { input: 115, output: 23, cache_read: 3_300, cache_write: 400, reasoning: 0 })]);
    let sessions = ledger.sessions_since(0);
    assert_eq!(
        sessions.iter().map(|session| (session.tool, session.id.as_str(), session.folder.as_deref(), session.title.as_deref())).collect::<Vec<_>>(),
        [(Tool::Kimi, "k1", Some("E:\\work"), Some("tidy the sensors"))]
    );
    // (The same request read again, from a copy of the record: not counted again.)
    append(&session.join("agents").join("main").join("copy.jsonl"), &kimi("main", NOON - 3_600_000, "turn", 100, 20, 3_000, 400));
    ledger.refresh(&roots, NOON);
    assert_eq!(spent(&ledger)[0].2.total(), 115 + 23 + 3_300 + 400);
    assert_eq!(kimi::session_of(&main).as_deref(), Some("k1"));
    let _ = std::fs::remove_dir_all(dir);
}

/// A line of Qwen Code's: its answer `uuid` in `session` at `time`, as it spent (its thoughts' count, and the total it
/// tells, where it tells one).
fn qwen(uuid: &str, time: &str, prompt: u64, answer: u64, thoughts: u64, cached: u64, total: Option<u64>) -> String {
    let total = total.map_or(String::new(), |total| format!(r#","totalTokenCount":{total}"#));
    format!(
        r#"{{"uuid":"{uuid}","parentUuid":null,"sessionId":"q1","timestamp":"{time}","type":"assistant","cwd":"D:\\app","model":"qwen3-coder-plus","usageMetadata":{{"promptTokenCount":{prompt},"candidatesTokenCount":{answer},"thoughtsTokenCount":{thoughts},"cachedContentTokenCount":{cached}{total}}}}}"#
    ) + "\n"
}

/// Qwen Code's sessions: each answer counted once by its id, its prompt less what came from the cache, its thoughts
/// among its answer where the total it tells is the prompt's and the answer's alone (else beside it); the session's
/// folder, and its title from what the user asked.
#[test]
fn reads_qwen_codes_answers_with_their_thoughts() {
    let dir = folder("qwen");
    let file = dir.join("projects").join("D--app").join("chats").join("q1.jsonl");
    append(&file, "{\"uuid\":\"u0\",\"sessionId\":\"q1\",\"timestamp\":\"2026-11-18T10:00:00.000Z\",\"type\":\"user\",\"cwd\":\"D:\\\\app\",\"message\":{\"role\":\"user\",\"parts\":[{\"text\":\"add the lunar date\"}]}}\n");
    // (Thoughts among the answer: the total is the prompt's and the answer's.)
    append(&file, &qwen("a1", "2026-11-18T10:01:00.000Z", 1_000, 300, 100, 400, Some(1_300)));
    // (Thoughts beside it.)
    append(&file, &qwen("a2", "2026-11-18T10:02:00.000Z", 500, 50, 20, 0, Some(570)));
    append(&file, &qwen("a2", "2026-11-18T10:02:00.000Z", 500, 50, 20, 0, Some(570)));
    let mut ledger = Ledger::default();
    assert!(ledger.refresh(&[(Tool::Qwen, dir.join("projects"))], NOON));
    assert_eq!(spent(&ledger), vec![(Tool::Qwen, "qwen3-coder-plus".to_string(), Tokens { input: 600 + 500, output: 300 + 70, cache_read: 400, cache_write: 0, reasoning: 120 })]);
    let sessions = ledger.sessions_since(0);
    assert_eq!(
        sessions.iter().map(|session| (session.id.as_str(), session.folder.as_deref(), session.title.as_deref())).collect::<Vec<_>>(),
        [("q1", Some("D:\\app"), Some("add the lunar date"))]
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// ZCode's database: each row counted once, however often it is asked again as the database is written to; where a
/// row tells its total, its input and output read as that total counts them (the cache's and the reasoning among
/// them, or beside them); a row of the later kind telling none as it is; a database of the first kind (no total
/// column) with them always among; the session's folder from its table.
#[test]
fn reads_zcodes_database_as_its_totals_count() {
    let dir = folder("zcode");
    let db = dir.join("cli").join("db");
    std::fs::create_dir_all(&db).unwrap();
    let later = sqlite::Database::made(&db.join("db.sqlite")).expect("Windows' SQLite");
    let hour = 3_600_000;
    for sql in [
        "CREATE TABLE session (id TEXT, directory TEXT, path TEXT)".to_string(),
        "CREATE TABLE model_usage (id TEXT, session_id TEXT, turn_id TEXT, model_id TEXT, started_at INTEGER, completed_at INTEGER, duration_ms INTEGER, input_tokens INTEGER, output_tokens INTEGER, reasoning_tokens INTEGER, cache_read_input_tokens INTEGER, cache_creation_input_tokens INTEGER, computed_total_tokens INTEGER, agent TEXT, mode TEXT)".into(),
        "INSERT INTO session VALUES ('z1', 'E:\\work', '')".into(),
        // (Among: its total is its input's and output's.)
        format!("INSERT INTO model_usage VALUES ('r1', 'z1', 't1', 'GLM-5.2', {}, NULL, NULL, 1000, 300, 100, 600, 200, 1300, 'main', NULL)", NOON - 2 * hour),
        // (Beside: its total is every count's.)
        format!("INSERT INTO model_usage VALUES ('r2', 'z1', 't1', 'glm-5.2', NULL, {}, 60000, 100, 30, 10, 50, 0, 190, 'main', NULL)", NOON - hour),
        // (No total: as it is.)
        format!("INSERT INTO model_usage VALUES ('r3', 'z1', 't2', 'glm-5.2', {}, NULL, NULL, 10, 5, 2, 3, 0, NULL, 'main', NULL)", NOON - hour),
    ] {
        later.rows(&sql).expect(&sql);
    }
    drop(later);
    let roots = [(Tool::Glm, db.clone())];
    let mut ledger = Ledger::default();
    assert!(ledger.refresh(&roots, NOON));
    let among = Tokens { input: 200, output: 300, cache_read: 600, cache_write: 200, reasoning: 100 };
    let beside = Tokens { input: 100, output: 40, cache_read: 50, cache_write: 0, reasoning: 10 };
    let plain = Tokens { input: 10, output: 7, cache_read: 3, cache_write: 0, reasoning: 2 };
    let mut all = among;
    all.add(beside);
    all.add(plain);
    assert_eq!(spent(&ledger), vec![(Tool::Glm, "glm-5.2".to_string(), all)]);
    let sessions = ledger.sessions_since(0);
    assert_eq!(sessions.iter().map(|session| (session.id.as_str(), session.folder.as_deref(), session.last)).collect::<Vec<_>>(), [("z1", Some("E:\\work"), NOON - hour)]);
    // (Written to again: asked again, what was counted not counted again.)
    let later = sqlite::Database::made(&db.join("db.sqlite")).unwrap();
    later.rows(&format!("INSERT INTO model_usage VALUES ('r4', 'z1', 't3', 'glm-5.2', {}, NULL, NULL, 1, 1, 0, 0, 0, 2, 'main', NULL)", NOON - 60_000)).unwrap();
    drop(later);
    std::thread::sleep(std::time::Duration::from_millis(20));
    // (Told by the name of what is written ahead of it, gone by now.)
    ledger.read_paths(Tool::Glm, &[db.join("db.sqlite-wal")], NOON);
    assert_eq!(spent(&ledger)[0].2.total(), all.total() + 2);
    // A database of the first kind: no total told, the cache's and the reasoning always among.
    let first = dir.join("first").join("db.sqlite");
    std::fs::create_dir_all(first.parent().unwrap()).unwrap();
    let made = sqlite::Database::made(&first).unwrap();
    for sql in [
        "CREATE TABLE model_usage (id TEXT, session_id TEXT, turn_id TEXT, model_id TEXT, started_at INTEGER, completed_at INTEGER, duration_ms INTEGER, input_tokens INTEGER, output_tokens INTEGER, reasoning_tokens INTEGER, cache_read_input_tokens INTEGER, cache_creation_input_tokens INTEGER, agent TEXT, mode TEXT)".to_string(),
        format!("INSERT INTO model_usage VALUES ('f1', 'z2', 't1', 'glm-4.6', {}, NULL, NULL, 1000, 300, 100, 600, 200, 'main', NULL)", NOON - hour),
    ] {
        made.rows(&sql).expect(&sql);
    }
    drop(made);
    let mut ledger = Ledger::default();
    ledger.refresh(&[(Tool::Glm, first.parent().unwrap().to_path_buf())], NOON);
    assert_eq!(spent(&ledger), vec![(Tool::Glm, "glm-4.6".to_string(), among)]);
    let _ = std::fs::remove_dir_all(dir);
}
