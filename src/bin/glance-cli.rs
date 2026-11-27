//! `glance-cli.exe`: what Glance knows of this PC, read from a terminal, or by AI tools as an MCP server (see
//! `agent`). A console program: a terminal waits for it and shows what it writes.

fn main() {
    glance_lib::run_cli()
}
