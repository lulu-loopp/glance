# AI tools reading Glance (MCP and CLI)

Agreed with the owner on 2026-11-25 and 2026-11-27. AI tools on this PC (Claude Code, Codex, Cursor and any other MCP
client), and a terminal, can read what Glance already knows: its readings and their history, the busiest programs, the
drives, and the files in its index of every local drive. Nothing is changed through it. It is off until the user turns
it on.

## Turning it on

Settings, General, 「AI 工具」: the switch 「允许 AI 工具读取本机信息」 (Let AI tools read this PC's information). Its note
names what can be read (hardware readings, program and device names, disk usage, file names and paths, network
addresses and AI Token usage) and says that an AI tool may send what it reads to its model provider.

While it is on, the row 「MCP 配置」 copies the configuration an AI tool's MCP settings take:

```json
{
  "mcpServers": {
    "glance": {
      "command": "C:\\Program Files\\Glance\\glance-cli.exe",
      "args": ["mcp"]
    }
  }
}
```

The path is the installed folder's. Glance puts nothing on PATH. In Claude Code the same is
`claude mcp add glance -- "C:\Program Files\Glance\glance-cli.exe" mcp`.

## The programs

- `glance-cli.exe mcp`: an MCP server on standard input and output. It speaks MCP 2026-07-28 (each request says its
  version in `_meta`, `server/discover`, no handshake) and the versions before it with an `initialize` handshake
  (2025-11-25, 2025-06-18, 2025-03-26). Standard output carries nothing but MCP. It ends when standard input ends.
- `glance-cli.exe query <command> [path or words] [--argument value ...]`: one call, its answer written as JSON.
  Commands: `status`, `hardware`, `readings`, `history`, `processes`, `drives`, `folder-size`, `list`,
  `largest-files`, `breakdown`, `search`, `ai-usage` (or a tool's own name). Arguments are the tool's, `_` written
  `-`. A list is `a,b`. A switch stands alone (`--reversed`). `glance-cli.exe --help` lists them.
  Exit codes: 0 done, 1 Glance not running, 2 arguments not read, 3 interface off, 4 the call failed,
  5 another version of Glance.
- `glance.exe mcp` serves MCP too, for a client set up with Glance itself.

Neither keeps anything: each call is asked of the running Glance on a connection of its own, so a client started
before Glance, or before the switch was turned on, finds it once it is.

## Tools

All are read-only (`readOnlyHint`). Each has an input schema (no other arguments taken) and an output schema. A failure
is a result with `isError` and `{ error: { code, message } }`.

| Tool | What it answers |
| --- | --- |
| `glance_status` | Version, interval, where readings come from (sensors, own, direct), history held, each drive's index |
| `get_hardware` | CPU, board, memory, GPUs, physical drives, network adapter |
| `get_readings` | The latest sample, by parts (cpu, gpu, memory, network, disk, volumes, sensors, battery, game, devices, wsl, docker, system) |
| `get_history` | Series over up to 24 hours: every sample for the last minutes, one-minute averages before (kept only while the interface is on) |
| `top_processes` | The busiest programs by CPU, memory, I/O or GPU |
| `list_drives` | Indexed drives with space and index state, physical drives with temperature and rates |
| `folder_size` | A folder's size with everything in it, and its largest children |
| `list_folder` | A folder's entries, each folder with its whole size and newest write |
| `largest_files` | The largest, oldest or newest files under a folder, filtered by size, age, kind, extension |
| `disk_breakdown` | What a folder or drive is made of: by kind, extension, age. For a root also free space, Recycle Bin and unaccounted space |
| `search_files` | Files and folders by name, with the search window's syntax, optionally under a folder |
| `ai_usage` | Tokens AI tools spent today and on the last days, plan limits, balances |

Units: bytes, `bytes_per_s`, percent 0 to 100, celsius, watts, ms since 1970 for sample times, ISO 8601 UTC for
file times. A value not read is null.

Error codes: `glance_not_running`, `interface_off`, `version_mismatch`, `request_too_large`, `invalid_argument`,
`not_found`, `not_a_folder`, `not_local`, `access_denied`, `index_listing`, `no_readings_yet`, `not_ready`, `timeout`,
`cancelled`, `busy`, `result_too_large`, `unknown_tool`, `unknown_error`.

## Files

Every file tool reads Glance's index of the drive (see `search`), never the disk itself, so a whole drive is answered in
about a second. The index holds what this Windows user could list when Glance last listed it.

- A path must be absolute on a lettered drive. Network paths are refused (`not_local`).
- Before the index is read, the folder is opened as the client (impersonated, `FILE_LIST_DIRECTORY`), so Windows
  checks that the client's user may list it, and links on the way are resolved to where the folder really is.
- While Glance runs as administrator its index holds names an ordinary process may not list: a client that is not
  elevated is then refused every file tool (`access_denied`). The index on disk records whether it was listed elevated
  (store version 6), and an index listed with other rights is listed again rather than searched as it was.
- `current: false` says the index is as kept at Glance's last run and is still being brought up to date.
  `index_listing` says a drive's names are still being found, with how many so far.
- Sizes are counted as the disk organizer and Explorer count them (a hard-linked file once for each name, compressed
  files at their whole size): not through links, files kept only in the cloud apart, the
  Recycle Bin apart unless the folder asked about is in it.

## The pipe

`\\.\pipe\dev.weiyi.glance.agent.<user SID>` (a debug build adds `.debug`). Glance serves it while the switch is on.

- Its DACL gives the system and this user read and write (what a duplex pipe's further instances take), and nothing to
  anyone else. Its label keeps processes below medium integrity from writing to it. Remote clients are refused.
- The first instance is made with `FILE_FLAG_FIRST_PIPE_INSTANCE`: if another program holds the name, the interface
  does not turn on, and the switch says it could not.
- The client checks that the process serving the pipe runs as its own user before it sends anything.
- Glance checks each client's token, after its first line, and serves only this user's processes.
- One instance is always there to be found while it is served: up to 4 clients at once, a fifth waits.

Lines of JSON, at most 256 KiB asked and 2 MiB answered:

1. The client: `{"hello": 1, "glance": "<version>", "client": "glance-cli <version>"}`. Glance answers
   `{"hello": 1, "glance": "<version>"}`, or `version_mismatch` and closes: the CLI must be the one installed with it.
2. Then `{"id", "tool", "args"}`, answered by `{"id", "result"}` or `{"id", "error"}`, one call at a time.

A call has 10 seconds. Two light calls run at once and one heavy (file) call. A call waits for its turn within its
time. A client closing its connection stops its call: the MCP server does that when a call is cancelled
(`notifications/cancelled`) or its standard input ends.

## History

While the switch is on, Glance keeps one average per series per minute for 24 hours, in memory only, next to the few
minutes of samples it always keeps. Series names are kept once each, and a minute holds only the series it had. Turned
off, the minutes are let go.

## AI usage

`ai_usage` reads what Glance's AI component reads. Asked while no AI component is shown, Glance starts reading it and
keeps it current for 15 minutes after the last asking. The first answer may then be `not_ready` for a moment.
