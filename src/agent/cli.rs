//! `glance-cli.exe`: the interface for a terminal (`glance-cli query …`), and its MCP server (`glance-cli mcp`).
//!
//! A query names a call by a short name (`status`, `folder-size`, …, or its own name), its path or words given as they
//! are, each other argument as `--name value` (`--step-ms 5000`; a list as `a,b`; a switch alone, `--reversed`), as
//! the call's arguments are named (see `tools`). Its answer is written as JSON.

use super::pipe::Event;
use super::Failure;
use serde_json::{json, Map, Value};

/// The short names of the calls, and the call each names.
const COMMANDS: [(&str, &str); 12] = [
    ("status", "glance_status"),
    ("hardware", "get_hardware"),
    ("readings", "get_readings"),
    ("history", "get_history"),
    ("processes", "top_processes"),
    ("drives", "list_drives"),
    ("folder-size", "folder_size"),
    ("list", "list_folder"),
    ("largest-files", "largest_files"),
    ("breakdown", "disk_breakdown"),
    ("search", "search_files"),
    ("ai-usage", "ai_usage"),
];

/// What a run ends with: done, Glance not running, the arguments not read, the interface off, the call failed,
/// Glance of another version.
pub const DONE: i32 = 0;
pub const NOT_RUNNING: i32 = 1;
pub const BAD_ARGUMENTS: i32 = 2;
pub const OFF: i32 = 3;
pub const FAILED: i32 = 4;
pub const MISMATCH: i32 = 5;

fn usage() -> String {
    let mut usage = format!(
        "glance-cli {} reads what Glance knows of this PC, while Glance runs and \"Let AI tools read this PC's information\" is on.\n\n\
         glance-cli mcp                 an MCP server on standard input and output, for AI tools\n\
         glance-cli query <command> [path or words] [--argument value ...] [--json]\n\nCommands:\n",
        super::VERSION
    );
    for (command, name) in COMMANDS {
        let tool = super::tools::named(name).expect("a call");
        let arguments: Vec<String> = tool.input["properties"].as_object().map(|properties| properties.keys().map(|key| format!("--{}", key.replace('_', "-"))).collect()).unwrap_or_default();
        usage.push_str(&format!("  {command:<14} {}{}\n", tool.title, if arguments.is_empty() { String::new() } else { format!(" ({})", arguments.join(" ")) }));
    }
    usage.push_str("\nExamples:\n  glance-cli query readings --parts cpu,gpu\n  glance-cli query folder-size \"D:\\Documents\" --largest 20\n  glance-cli query search \"ext:iso ubuntu\" --under D:\\\n");
    usage
}

/// The call `words` (what follows `query`) ask for, and its arguments, read as its schema names and types them.
pub fn read(words: &[String]) -> Result<(String, Value), String> {
    let command = words.first().ok_or_else(|| "Name a command.".to_string())?;
    let name = COMMANDS.iter().find(|(short, _)| short == command).map(|(_, name)| *name).or_else(|| super::tools::named(command).map(|tool| tool.name)).ok_or_else(|| format!("{command} is not a command."))?;
    let tool = super::tools::named(name).expect("a call");
    let properties = tool.input["properties"].as_object().cloned().unwrap_or_default();
    // (What is given alone is the call's path, or its words.)
    let alone = ["path", "query"].into_iter().find(|key| properties.contains_key(*key));
    let mut args = Map::new();
    let mut rest = words[1..].iter();
    while let Some(word) = rest.next() {
        if word == "--json" {
            continue;
        }
        let Some(flag) = word.strip_prefix("--") else {
            let key = alone.ok_or_else(|| format!("{command} takes no path or words: {word}"))?;
            if args.contains_key(key) {
                return Err(format!("{command} takes one {key}: {word} is another."));
            }
            args.insert(key.into(), json!(word));
            continue;
        };
        let key = flag.replace('-', "_");
        let schema = properties.get(&key).ok_or_else(|| format!("{command} has no --{flag}."))?;
        let kind = schema["type"].as_str().unwrap_or("string");
        let value = if kind == "boolean" {
            json!(true)
        } else {
            let given = rest.next().ok_or_else(|| format!("--{flag} needs a value."))?;
            match kind {
                "integer" => json!(given.parse::<i64>().map_err(|_| format!("--{flag} must be a whole number."))?),
                "array" => json!(given.split(',').map(str::trim).filter(|item| !item.is_empty()).collect::<Vec<_>>()),
                _ => json!(given),
            }
        };
        args.insert(key, value);
    }
    Ok((name.to_string(), Value::Object(args)))
}

/// What a run ends with, where Glance answered `failure`.
fn ended_by(failure: &Failure) -> i32 {
    match failure.code {
        "glance_not_running" => NOT_RUNNING,
        "interface_off" => OFF,
        "version_mismatch" => MISMATCH,
        _ => FAILED,
    }
}

/// `glance-cli.exe` run with `arguments` (its own name left out): what it ends with.
pub fn run(arguments: &[String]) -> i32 {
    match arguments.first().map(String::as_str) {
        Some("mcp") => {
            super::mcp::serve();
            DONE
        }
        Some("query") => {
            let (tool, args) = match read(&arguments[1..]) {
                Ok(read) => read,
                Err(why) => {
                    eprintln!("{why}\n\n{}", usage());
                    return BAD_ARGUMENTS;
                }
            };
            let stop = Event::new();
            match super::client::call(&tool, &args, &stop) {
                Ok(result) => {
                    println!("{}", serde_json::to_string_pretty(&result).unwrap_or_default());
                    DONE
                }
                Err(failure) => {
                    println!("{}", serde_json::to_string_pretty(&json!({ "error": failure.told() })).unwrap_or_default());
                    eprintln!("{}", failure.message);
                    ended_by(&failure)
                }
            }
        }
        Some("--version" | "-V") => {
            println!("glance-cli {}", super::VERSION);
            DONE
        }
        Some("--help" | "-h" | "help") | None => {
            print!("{}", usage());
            DONE
        }
        Some(other) => {
            eprintln!("{other} is not a command.\n\n{}", usage());
            BAD_ARGUMENTS
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &[&str]) -> Vec<String> {
        line.iter().map(|word| word.to_string()).collect()
    }

    /// A query's words are read as the call's arguments are named and typed: its path given alone, each other as
    /// --name value, a list split at its commas, a switch alone.
    #[test]
    fn a_query_is_read_as_its_call_names_its_arguments() {
        assert_eq!(read(&words(&["status", "--json"])), Ok(("glance_status".into(), json!({}))));
        assert_eq!(
            read(&words(&["largest-files", "C:\\", "--order", "oldest", "--min-bytes", "1000", "--kinds", "video, archive", "--include-system"])),
            Ok(("largest_files".into(), json!({ "path": "C:\\", "order": "oldest", "min_bytes": 1000, "kinds": ["video", "archive"], "include_system": true })))
        );
        assert_eq!(read(&words(&["search", "ext:iso ubuntu", "--under", "D:\\"])), Ok(("search_files".into(), json!({ "query": "ext:iso ubuntu", "under": "D:\\" }))));
        assert_eq!(read(&words(&["get_history", "--series", "cpu.percent", "--step-ms", "5000"])), Ok(("get_history".into(), json!({ "series": ["cpu.percent"], "step_ms": 5000 }))));
        assert_eq!(read(&words(&["status", "C:\\"])), Err("status takes no path or words: C:\\".into()));
        assert_eq!(read(&words(&["list", "C:\\", "--depth", "2"])), Err("list has no --depth.".into()));
        assert_eq!(read(&words(&["list", "C:\\", "--limit", "many"])), Err("--limit must be a whole number.".into()));
        assert_eq!(read(&words(&["remove", "C:\\"])), Err("remove is not a command.".into()));
    }
}
