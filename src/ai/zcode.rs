//! ZCode's (Z.ai's) records: what each request to the model spent, a row of its database's `model_usage`
//! (`~/.zcode/cli/db/db.sqlite`), with its session's folder from `session`. Read as tokscale (MIT, Copyright (c) 2025
//! Junho Yeo) reads them: where a row tells its total, it tells whether its input counts what was read from and
//! written to the cache among it and its output its reasoning (its total then theirs alone); a row of the database's
//! first kind, which tells no total, always counts them so; one of the later kind that does not tell it, never.

use super::sqlite::{Database, Value};
use super::{key, Told, Tokens};
use std::path::Path;

/// The columns asked of each row: its id, session, model, when it started and ended and how long it took, its tokens
/// by kind (and their total, where the database tells it), and its session's folder.
const COLUMNS: &str = "mu.id, mu.session_id, mu.model_id, mu.started_at, mu.completed_at, mu.duration_ms, mu.input_tokens, mu.output_tokens, mu.reasoning_tokens, mu.cache_read_input_tokens, mu.cache_creation_input_tokens";

/// What a row of `model_usage` tells.
#[derive(Default)]
struct Row {
    id: Option<String>,
    session: Option<String>,
    model: Option<String>,
    started: Option<i64>,
    completed: Option<i64>,
    took: Option<i64>,
    input: i64,
    output: i64,
    reasoning: i64,
    cache_read: i64,
    cache_write: i64,
    total: Option<i64>,
    folder: Option<String>,
}

/// What the database at `path` (written to at `modified`) tells was spent from `from` (ms since 1970) on.
pub(super) fn told_in(path: &Path, from: u64, modified: u64) -> Vec<Told> {
    let Some(database) = Database::read(path) else { return Vec::new() };
    // (The database's first kind tells no total.)
    let first_kind = database.rows("SELECT computed_total_tokens FROM model_usage LIMIT 1").is_none();
    let total = if first_kind { "NULL" } else { "mu.computed_total_tokens" };
    let ask = |folder: &str| {
        format!("SELECT {COLUMNS}, {total}, {folder} FROM model_usage mu {} WHERE COALESCE(mu.completed_at, mu.started_at, 0) >= {from}", if folder == "NULL" { "" } else { "LEFT JOIN session s ON s.id = mu.session_id" })
    };
    // (Its sessions' folders where it keeps them.)
    let Some(rows) = database.rows(&ask("COALESCE(NULLIF(s.directory, ''), NULLIF(s.path, ''))")).or_else(|| database.rows(&ask("NULL"))) else { return Vec::new() };
    let mut told = Vec::new();
    for values in rows {
        let row = row_of(&values);
        told.extend(told_of(row, first_kind, modified));
    }
    told
}

fn row_of(values: &[Value]) -> Row {
    let at = |index: usize| values.get(index).cloned().unwrap_or(Value::Null);
    let count = |index: usize| at(index).int().unwrap_or(0).max(0);
    Row {
        id: at(0).text(),
        session: at(1).text(),
        model: at(2).text(),
        started: at(3).int(),
        completed: at(4).int(),
        took: at(5).int(),
        input: count(6),
        output: count(7),
        reasoning: count(8),
        cache_read: count(9),
        cache_write: count(10),
        total: at(11).int().map(|total| total.max(0)),
        folder: at(12).text(),
    }
}

/// What `row` spent (of the database's `first_kind`, written to at `modified`), and its session's folder.
fn told_of(row: Row, first_kind: bool, modified: u64) -> Vec<Told> {
    let Some(id) = row.id.clone() else { return Vec::new() };
    let tokens = tokens_of(&row, first_kind);
    if tokens.total() == 0 {
        return Vec::new();
    }
    // When it started: as told, else from when it ended, less how long it took.
    let at = match (row.started.filter(|at| *at > 0), row.completed) {
        (Some(started), _) => started,
        (None, Some(completed)) => row.took.filter(|took| *took > 0).map(|took| completed - took).filter(|at| *at > 0).unwrap_or(completed),
        (None, None) => modified as i64,
    };
    let session = row.session.unwrap_or_else(|| "unknown".into());
    let mut told = Vec::new();
    if let Some(folder) = row.folder {
        told.push(Told::Folder { session: session.clone(), folder });
    }
    let model = row.model.map_or_else(|| "unknown".into(), |model| model.to_lowercase());
    told.push(Told::Spent { key: key(&["zcode", &id]), at: at.max(0) as u64, session, model, tokens });
    told
}

/// What `row` spent, its input less what came from or went to the cache, its output with its reasoning among it: as
/// its total tells it counts them, else as the database's kind does.
fn tokens_of(row: &Row, first_kind: bool) -> Tokens {
    let cached = row.cache_read + row.cache_write;
    let among = match row.total {
        Some(total) => (cached > 0 || row.reasoning > 0) && total == row.input + row.output && total != row.input + row.output + cached + row.reasoning,
        None => first_kind,
    };
    let (input, output) = if among { (row.input - cached.min(row.input), row.output) } else { (row.input, row.output + row.reasoning) };
    let reasoning = if among { row.reasoning.min(row.output) } else { row.reasoning };
    Tokens { input: input as u64, output: output as u64, cache_read: row.cache_read as u64, cache_write: row.cache_write as u64, reasoning: reasoning as u64 }
}
