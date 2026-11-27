//! The interface's calls: what each is named and for, the arguments it takes and what it answers (as JSON Schemas,
//! which the MCP server lists as they are), and how its arguments are read. Its answers are made in `readings` and
//! `files`.

use super::Failure;
use serde_json::{json, Map, Value};

/// How much a call asks of Glance: a light one reads what it holds, a heavy one goes through a drive's names. Each
/// kind waits for one of its own few turns (see `server::Turns`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    Light,
    Heavy,
}

/// A call.
pub struct Tool {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub input: Value,
    pub output: Value,
    pub weight: Weight,
}

/// Every code a failure may be told by.
pub const CODES: [&str; 18] = [
    "glance_not_running",
    "interface_off",
    "version_mismatch",
    "request_too_large",
    "invalid_argument",
    "not_found",
    "not_a_folder",
    "not_local",
    "access_denied",
    "index_listing",
    "no_readings_yet",
    "not_ready",
    "timeout",
    "cancelled",
    "busy",
    "result_too_large",
    "unknown_tool",
    "unknown_error",
];

/// An object of `properties`, `required` among them, nothing else allowed.
fn arguments(properties: Value, required: &[&str]) -> Value {
    let mut schema = json!({ "type": "object", "properties": properties, "additionalProperties": false });
    if !required.is_empty() {
        schema["required"] = json!(required);
    }
    schema
}

/// What a call answers: an object of `properties` (none of them required: a failure, `{error}`, is one too).
fn answer(properties: Value) -> Value {
    let mut properties = properties;
    properties["error"] = json!({
        "type": "object",
        "description": "Present instead of the result when the call failed.",
        "properties": { "code": { "type": "string", "enum": CODES }, "message": { "type": "string" } },
        "required": ["code", "message"]
    });
    json!({ "type": "object", "properties": properties })
}

const PATH: &str = "An absolute path on a lettered drive of this PC, such as C:\\Users or D:\\.";

/// Every call, in the order they are listed.
pub fn all() -> Vec<Tool> {
    let int = |description: &str, min: i64, max: i64, default: Option<i64>| {
        let mut schema = json!({ "type": "integer", "minimum": min, "maximum": max, "description": description });
        if let Some(default) = default {
            schema["default"] = json!(default);
        }
        schema
    };
    let choice = |values: &[&str], default: &str, description: &str| json!({ "type": "string", "enum": values, "default": default, "description": description });
    let path = || json!({ "type": "string", "minLength": 3, "description": PATH });
    let bytes = || json!({ "type": "integer", "minimum": 0 });
    let maybe = |kind: &str| json!({ "type": [kind, "null"] });
    vec![
        Tool {
            name: "glance_status",
            title: "Glance status",
            description: "What Glance is doing: its version, the interval it reads at, where its readings come from (sensors: its elevated sensors process. own: without it, so no board, CPU or memory temperatures. direct: Glance itself elevated), how much history it holds, and how far each drive's file-name index is.",
            input: arguments(json!({}), &[]),
            output: answer(json!({
                "glance": { "type": "string" }, "interface": { "type": "integer" }, "interval_ms": { "type": "integer" },
                "readings": { "type": "string", "enum": ["sensors", "own", "direct"] }, "elevated": { "type": "boolean" },
                "history": { "type": "object" }, "search": { "type": "object" }
            })),
            weight: Weight::Light,
        },
        Tool {
            name: "get_hardware",
            title: "Hardware",
            description: "The hardware as Glance found it: CPU, logical processors, motherboard, memory (total, modules, speed), GPUs (by index, as get_readings and get_history number them), physical drives (by id) and the network adapter.",
            input: arguments(json!({}), &[]),
            output: answer(json!({
                "cpu": { "type": "string" }, "threads": { "type": "integer" }, "board": { "type": "string" }, "memory": { "type": "object" },
                "gpus": { "type": "array" }, "drives": { "type": "array" }, "network_adapter": maybe("string")
            })),
            weight: Weight::Light,
        },
        Tool {
            name: "get_readings",
            title: "Current readings",
            description: "The latest sample Glance read, in parts: cpu (use, clock, temperature, power), gpu (each GPU), memory, network (rates, totals, adapter, addresses, Wi-Fi name, ping), disk (rates, each drive's), volumes (space), sensors (motherboard temperatures and fans), battery, game (the game presenting frames: frame rate, 1% low), devices (wireless devices' charge), wsl, docker and system (counts). A reading not had this time is null. A part Glance does not read on this PC is left out. Programs are in top_processes. Units: percent 0-100, celsius, watts, bytes, bytes_per_s, ms.",
            input: arguments(
                json!({ "parts": {
                    "type": "array", "minItems": 1, "uniqueItems": true,
                    "items": { "type": "string", "enum": super::readings::PARTS },
                    "description": "Only these parts (default: all)."
                } }),
                &[],
            ),
            output: answer(json!({ "t_ms": { "type": "integer" } })),
            weight: Weight::Light,
        },
        Tool {
            name: "get_history",
            title: "Readings over time",
            description: "Readings over the last seconds to hours, for trends. The last few minutes are every sample Glance read. Older points (up to 24 hours, kept while this interface is on) are one-minute averages, said by minutes_until_ms. Series: cpu.percent, cpu.celsius, cpu.watts, cpu.ghz, gpu<N>.percent, gpu<N>.celsius, gpu<N>.watts, gpu<N>.memory_used_bytes, memory.used_bytes, memory.committed_bytes, network.down_bytes_per_s, network.up_bytes_per_s, network.ping_ms, disk.read_bytes_per_s, disk.write_bytes_per_s, disk.active_percent, drive<N>.read_bytes_per_s, drive<N>.write_bytes_per_s, drive<N>.celsius, battery.percent, battery.watts, game.fps, board.<sensor name>.celsius, fan.<fan name>.rpm, program.<exe name>.cpu_percent, program.<exe name>.memory_bytes. A value is null where it was not read (a program null where it was not among the sampled programs). Series with no value in the time asked are listed in unknown.",
            input: arguments(
                json!({
                    "series": { "type": "array", "minItems": 1, "maxItems": 16, "uniqueItems": true, "items": { "type": "string" }, "description": "The series wanted, as named above." },
                    "seconds": int("How far back from the newest sample.", 1, 86_400, Some(60)),
                    "step_ms": int("Average into buckets this wide (not less than the interval). Default: each point as kept.", 1, 3_600_000, None)
                }),
                &["series"],
            ),
            output: answer(json!({
                "interval_ms": { "type": "integer" }, "covered": { "type": "object" }, "minutes_until_ms": maybe("integer"),
                "t_ms": { "type": "array", "items": { "type": "integer" } }, "series": { "type": "object" }, "unknown": { "type": "array", "items": { "type": "string" } }
            })),
            weight: Weight::Light,
        },
        Tool {
            name: "top_processes",
            title: "Busiest programs",
            description: "The busiest programs in the latest sample, by one measure. A program is every process sharing an executable name, added together. Glance samples the top 40 by each measure: a program not listed is not necessarily idle.",
            input: arguments(
                json!({
                    "by": choice(&["cpu", "memory", "io", "gpu"], "cpu", "The measure ranked by."),
                    "limit": int("How many.", 1, 40, Some(10)),
                    "name": { "type": "string", "minLength": 1, "maxLength": 260, "description": "Only programs whose executable name holds this (any case)." }
                }),
                &[],
            ),
            output: answer(json!({ "t_ms": { "type": "integer" }, "by": { "type": "string" }, "processes": { "type": "array" } })),
            weight: Weight::Light,
        },
        Tool {
            name: "list_drives",
            title: "Drives",
            description: "The drives Glance indexes (fixed and removable, by letter) with their space and how far each index is, and the physical drives with their temperature and rates. Network drives are not indexed.",
            input: arguments(json!({}), &[]),
            output: answer(json!({ "volumes": { "type": "array" }, "physical": { "type": "array" } })),
            weight: Weight::Light,
        },
        Tool {
            name: "folder_size",
            title: "Folder size",
            description: "How large a folder is with everything in it, and which of its direct children are largest. Counted from Glance's file index as Explorer counts a folder (file sizes, not through links, without files kept only in the cloud). The index holds what this Windows user could list when Glance last listed it. current: false means the index is as kept at Glance's last run and still being brought up to date. skipped_folders: folders under it whose names could not be listed.",
            input: arguments(json!({ "path": path(), "largest": int("How many of the largest children.", 0, 100, Some(10)) }), &["path"]),
            output: answer(json!({ "path": { "type": "string" }, "bytes": bytes(), "files": bytes(), "largest": { "type": "array" }, "current": { "type": "boolean" }, "skipped_folders": bytes() })),
            weight: Weight::Heavy,
        },
        Tool {
            name: "list_folder",
            title: "Folder contents",
            description: "What is directly in a folder, from Glance's file index, each folder with the size of all it holds and the newest write of anything in it. Faster than listing the disk. Links are listed, not followed.",
            input: arguments(
                json!({
                    "path": path(),
                    "order": choice(&["size", "name", "modified"], "size", "size: largest first. name: A to Z. modified: newest first."),
                    "reversed": { "type": "boolean", "default": false },
                    "limit": int("How many entries.", 1, 200, Some(100)),
                    "offset": int("Entries skipped first.", 0, 1_000_000, Some(0))
                }),
                &["path"],
            ),
            output: answer(json!({ "path": { "type": "string" }, "total": bytes(), "entries": { "type": "array" }, "current": { "type": "boolean" }, "truncated": { "type": "boolean" } })),
            weight: Weight::Heavy,
        },
        Tool {
            name: "largest_files",
            title: "Largest or oldest files",
            description: "The largest (or oldest, or newest) files under a folder or a whole drive, from Glance's file index, filtered by size, age, kind and extension. Kinds are told as Glance's disk organizer tells them: by extension, files in game libraries are game, Windows' own files are system. System files and the Recycle Bin are left out unless asked. matching counts every file that passes, not only those listed. Ages are by last write time.",
            input: arguments(
                json!({
                    "path": path(),
                    "order": choice(&["size", "oldest", "newest"], "size", "size: largest first. oldest / newest: by last write."),
                    "limit": int("How many files.", 1, 200, Some(20)),
                    "offset": int("Files skipped first.", 0, 10_000, Some(0)),
                    "min_bytes": int("Only files at least this large.", 0, i64::MAX, Some(0)),
                    "not_modified_days": int("Only files last written more than this many days ago.", 1, 36_500, None),
                    "kinds": { "type": "array", "uniqueItems": true, "items": { "type": "string", "enum": super::files::KIND_NAMES }, "description": "Only files of these kinds." },
                    "extensions": { "type": "array", "uniqueItems": true, "maxItems": 64, "items": { "type": "string", "minLength": 1, "maxLength": 12 }, "description": "Only files of these extensions, without dots, any case." },
                    "include_system": { "type": "boolean", "default": false },
                    "include_recycle_bin": { "type": "boolean", "default": false, "description": "Files in the Recycle Bin too (always when path is in it)." }
                }),
                &["path"],
            ),
            output: answer(json!({ "path": { "type": "string" }, "files": { "type": "array" }, "matching": { "type": "object" }, "current": { "type": "boolean" }, "skipped_folders": bytes(), "truncated": { "type": "boolean" } })),
            weight: Weight::Heavy,
        },
        Tool {
            name: "disk_breakdown",
            title: "What a folder or drive is made of",
            description: "What the files under a folder or a drive come to: by kind, by extension, and by age of last write (days before now, [from, to)). For a drive root also its total, used and free space, its Recycle Bin, and unaccounted_bytes: used space less what the files come to, an estimate. Positive is space no listed file accounts for (the file system's own, files this user may not list). Negative means files count for more than they take: a hard-linked file is counted once for each name, and compressed or sparse files at their whole size, as Explorer counts them. Files kept only in the cloud are counted apart. The Recycle Bin is left out unless path is in it.",
            input: arguments(json!({ "path": path(), "extensions": int("How many extensions, the largest first.", 1, 40, Some(20)) }), &["path"]),
            output: answer(json!({
                "path": { "type": "string" }, "bytes": bytes(), "files": bytes(), "drive": { "type": "object" }, "by_kind": { "type": "object" },
                "by_extension": { "type": "array" }, "by_age": { "type": "array" }, "cloud_only_bytes": bytes(), "current": { "type": "boolean" }, "skipped_folders": bytes()
            })),
            weight: Weight::Heavy,
        },
        Tool {
            name: "search_files",
            title: "Find files by name",
            description: "Find files and folders by name in Glance's index of every local drive, as its search window does. Query syntax: plain words must all be in the name (any case, an all-letter word also matches Chinese names by pinyin). ext:pdf,docx keeps files of those extensions. A word holding a backslash (src\\docs\\) asks each part to be in the name of some folder the result is in (not an exact path: use under for that). * and ? make a pattern the whole name must meet. complete: false means some drive's names are still being found.",
            input: arguments(
                json!({
                    "query": { "type": "string", "minLength": 1, "maxLength": 512 },
                    "under": { "type": "string", "minLength": 3, "description": "Only results inside this folder (an absolute path)." },
                    "only": { "type": "string", "enum": ["files", "folders"] },
                    "order": choice(&["match", "modified", "name", "size"], "match", "match: best first, the user's own files before Windows', programs' and build folders. modified: newest first. name: A to Z. size: largest first."),
                    "reversed": { "type": "boolean", "default": false },
                    "limit": int("How many results.", 1, 200, Some(20)),
                    "offset": int("Results skipped first (offset + limit at most 1000).", 0, 999, Some(0))
                }),
                &["query"],
            ),
            output: answer(json!({ "total": bytes(), "folders": bytes(), "complete": { "type": "boolean" }, "truncated": { "type": "boolean" }, "hits": { "type": "array" } })),
            weight: Weight::Heavy,
        },
        Tool {
            name: "ai_usage",
            title: "AI tools' token usage",
            description: "What AI coding tools spent on this PC, as Glance's AI component reads it: tokens today by tool and model, tokens on each of the last days by tool (in the order of dates, today last), plan limits (window, percent used, reset time) and account balances. No conversation titles or folders.",
            input: arguments(json!({ "days": int("How many days, today last.", 1, 7, Some(7)) }), &[]),
            output: answer(json!({ "now_ms": { "type": "integer" }, "today": { "type": "array" }, "dates": { "type": "array", "items": { "type": "string" } }, "days": { "type": "array" }, "limits": { "type": "array" }, "failed": { "type": "array" }, "unkeyed": { "type": "array" } })),
            weight: Weight::Light,
        },
    ]
}

/// What a call was given, read as its arguments' schema allows (`tool`'s): no other names, each of its kind and within
/// its bounds.
pub struct Args<'a> {
    given: Map<String, Value>,
    schema: &'a Value,
}

impl<'a> Args<'a> {
    /// The arguments `given` (an object, or none) to `tool`.
    pub fn of(given: &Value, tool: &'a Tool) -> Result<Args<'a>, Failure> {
        let given = match given {
            Value::Null => Map::new(),
            Value::Object(given) => given.clone(),
            _ => return Err(Failure::invalid("The arguments must be an object.")),
        };
        let properties = &tool.input["properties"];
        if let Some(unknown) = given.keys().find(|key| properties.get(key.as_str()).is_none()) {
            let known: Vec<&str> = properties.as_object().map(|known| known.keys().map(String::as_str).collect()).unwrap_or_default();
            return Err(Failure::invalid(if known.is_empty() { format!("{} takes no arguments, not {unknown}.", tool.name) } else { format!("Unknown argument {unknown}. Known: {}.", known.join(", ")) }));
        }
        for required in tool.input["required"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            if given.get(required).is_none_or(Value::is_null) {
                return Err(Failure::invalid(format!("{required} is required.")));
            }
        }
        Ok(Args { given, schema: properties })
    }

    /// What was given as `key`, none where nothing (or null) was.
    fn value(&self, key: &str) -> Option<&Value> {
        self.given.get(key).filter(|value| !value.is_null())
    }

    pub fn string(&self, key: &str) -> Result<Option<&str>, Failure> {
        let Some(value) = self.value(key) else { return Ok(None) };
        let text = value.as_str().ok_or_else(|| Failure::invalid(format!("{key} must be a string.")))?;
        let schema = &self.schema[key];
        let length = text.chars().count() as u64;
        if schema["minLength"].as_u64().is_some_and(|least| length < least) || schema["maxLength"].as_u64().is_some_and(|most| length > most) {
            return Err(Failure::invalid(format!("{key} must be {} to {} characters long.", schema["minLength"].as_u64().unwrap_or(0), schema["maxLength"].as_u64().map_or("any".into(), |most| most.to_string()))));
        }
        if let Some(allowed) = schema["enum"].as_array() {
            if !allowed.iter().any(|allowed| allowed.as_str() == Some(text)) {
                return Err(Failure::invalid(format!("{key} must be one of {}.", names(allowed))));
            }
        }
        Ok(Some(text))
    }

    /// The string given as `key`, or its default.
    pub fn choice(&self, key: &str) -> Result<&str, Failure> {
        Ok(self.string(key)?.or_else(|| self.schema[key]["default"].as_str()).unwrap_or_default())
    }

    pub fn int(&self, key: &str) -> Result<Option<i64>, Failure> {
        let Some(value) = self.value(key) else { return Ok(self.schema[key]["default"].as_i64()) };
        let schema = &self.schema[key];
        let (least, most) = (schema["minimum"].as_i64().unwrap_or(i64::MIN), schema["maximum"].as_i64().unwrap_or(i64::MAX));
        // (A whole number written with a point, 20.0, as some clients send one.)
        let number = value.as_i64().or_else(|| value.as_f64().filter(|number| number.fract() == 0.0 && number.abs() < 9.0e18).map(|number| number as i64));
        match number {
            Some(number) if (least..=most).contains(&number) => Ok(Some(number)),
            _ => Err(Failure::invalid(format!("{key} must be a whole number from {least} to {most}."))),
        }
    }

    pub fn boolean(&self, key: &str) -> Result<bool, Failure> {
        match self.value(key) {
            None => Ok(self.schema[key]["default"].as_bool().unwrap_or(false)),
            Some(value) => value.as_bool().ok_or_else(|| Failure::invalid(format!("{key} must be true or false."))),
        }
    }

    /// The strings given as `key`, each as its schema allows, none twice.
    pub fn strings(&self, key: &str) -> Result<Option<Vec<String>>, Failure> {
        let Some(value) = self.value(key) else { return Ok(None) };
        let schema = &self.schema[key];
        let items = value.as_array().ok_or_else(|| Failure::invalid(format!("{key} must be an array of strings.")))?;
        if schema["minItems"].as_u64().is_some_and(|least| (items.len() as u64) < least) || schema["maxItems"].as_u64().is_some_and(|most| items.len() as u64 > most) {
            return Err(Failure::invalid(format!("{key} must hold {} to {} items.", schema["minItems"].as_u64().unwrap_or(0), schema["maxItems"].as_u64().map_or("any".into(), |most| most.to_string()))));
        }
        let item = &schema["items"];
        let mut read: Vec<String> = Vec::new();
        for value in items {
            let text = value.as_str().ok_or_else(|| Failure::invalid(format!("{key} must be an array of strings.")))?;
            let length = text.chars().count() as u64;
            if item["minLength"].as_u64().is_some_and(|least| length < least) || item["maxLength"].as_u64().is_some_and(|most| length > most) {
                return Err(Failure::invalid(format!("Each of {key} must be {} to {} characters long.", item["minLength"].as_u64().unwrap_or(0), item["maxLength"].as_u64().map_or("any".into(), |most| most.to_string()))));
            }
            if let Some(allowed) = item["enum"].as_array() {
                if !allowed.iter().any(|allowed| allowed.as_str() == Some(text)) {
                    return Err(Failure::invalid(format!("Each of {key} must be one of {}.", names(allowed))));
                }
            }
            if read.iter().any(|had| had == text) {
                return Err(Failure::invalid(format!("{key} names {text} twice.")));
            }
            read.push(text.to_string());
        }
        Ok(Some(read))
    }
}

/// `values` (strings) as a list for a sentence.
fn names(values: &[Value]) -> String {
    values.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")
}

/// The call named `name`.
pub fn named(name: &str) -> Option<Tool> {
    all().into_iter().find(|tool| tool.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Arguments are read as their schema allows: names it does not have, values of another kind or out of bounds,
    /// and a required one missing are refused, saying why; those not given are their defaults.
    #[test]
    fn arguments_are_read_as_their_schema_allows() {
        let tool = named("list_folder").unwrap();
        let read = |given: Value| Args::of(&given, &tool).map(|args| (args.string("path").unwrap().map(String::from), args.choice("order").unwrap().to_string(), args.int("limit").unwrap(), args.boolean("reversed").unwrap()));
        assert_eq!(read(json!({ "path": "C:\\" })), Ok((Some("C:\\".into()), "size".into(), Some(100), false)));
        assert_eq!(read(json!({ "path": "C:\\", "limit": 20.0, "order": "name", "reversed": true })), Ok((Some("C:\\".into()), "name".into(), Some(20), true)));
        assert_eq!(read(json!({})).unwrap_err().message, "path is required.");
        assert!(read(json!({ "path": "C:\\", "depth": 2 })).unwrap_err().message.starts_with("Unknown argument depth."));
        let tool = named("list_folder").unwrap();
        let args = Args::of(&json!({ "path": "C:\\", "limit": 500, "order": "date" }), &tool).unwrap();
        assert_eq!(args.int("limit").unwrap_err().message, "limit must be a whole number from 1 to 200.");
        assert_eq!(args.choice("order").unwrap_err().message, "order must be one of size, name, modified.");
        let tool = named("get_readings").unwrap();
        let args = Args::of(&json!({ "parts": ["cpu", "cpu"] }), &tool).unwrap();
        assert_eq!(args.strings("parts").unwrap_err().message, "parts names cpu twice.");
        let args = Args::of(&json!({ "parts": ["cpu", "fans"] }), &tool).unwrap();
        assert!(args.strings("parts").unwrap_err().message.starts_with("Each of parts must be one of cpu, gpu"));
        assert_eq!(Args::of(&json!({ "x": 1 }), &named("glance_status").unwrap()).err().unwrap().message, "glance_status takes no arguments, not x.");
    }

    /// Every call's arguments and answers are JSON Schemas of objects, as MCP asks, and every name is one an agent may
    /// call it by.
    #[test]
    fn every_call_is_described_as_mcp_asks() {
        let tools = all();
        for tool in &tools {
            assert!(tool.name.len() <= 64 && tool.name.chars().all(|c| c.is_ascii_lowercase() || c == '_'), "{}", tool.name);
            assert_eq!(tool.input["type"], "object", "{}", tool.name);
            assert_eq!(tool.output["type"], "object", "{}", tool.name);
            assert!(tool.output["properties"]["error"].is_object());
            assert!(!tool.description.contains(';'), "{}", tool.name);
        }
        let mut names: Vec<&str> = tools.iter().map(|tool| tool.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), tools.len());
    }
}
