# mcp-install

Register an MCP server in the AI coding tools on a machine, and add the always-on instructions that tell
the agent to use it.

Every tool keeps its MCP servers in a different file, in a different place, in a different shape. This
crate knows them, so your server's own `install` command is a few lines instead of seven special cases.
It is the installer behind [Plunger](https://github.com/metamug/plunger)'s `plunger install`.

```toml
[dependencies]
mcp-install = "0.1"
```

```rust
use mcp_install::{install, Agent, Options, Scope, Server, Steering};

let server = Server::new("acme", "acme-mcp").arg("serve");
let steering = Steering::new(
    "Use Acme for deployments",
    "# Use Acme\n\nFor anything about deployments, call the `acme` MCP tools instead of running the CLI.\n",
);

let home = mcp_install::home_dir().expect("a home folder");
let project = std::env::current_dir().unwrap();
let options = Options { scope: Scope::Project, mcp: true, steering: true, dry_run: true };

for change in install(&Agent::ALL, &home, &project, &server, Some(&steering), &options) {
    println!("{:12} {:9} {:9} {}", change.agent, change.what, change.action, change.file);
}
```

Try it from a checkout without writing a program: `cargo run -p mcp-install --example add -- --name acme
--command acme-mcp --arg serve --dry-run`.

## What it writes

| Tool | Server registered in | Instructions written to |
|---|---|---|
| Claude Code | `.mcp.json` (project), `~/.claude.json` (user) | `CLAUDE.md` block and a skill in `.claude/skills/<name>/` |
| Cursor | `.cursor/mcp.json`, `~/.cursor/mcp.json` | `.cursor/rules/<name>.mdc` with `alwaysApply: true` |
| Kiro | `.kiro/settings/mcp.json`, `~/.kiro/settings/mcp.json` | `.kiro/steering/<name>.md` with `inclusion: always` |
| Codex | `~/.codex/config.toml` (`[mcp_servers.<name>]`) | `AGENTS.md` block (project) or `~/.codex/AGENTS.md` |
| Windsurf | `~/.codeium/windsurf/mcp_config.json` | `.windsurf/rules/<name>.md` (always on) |
| VS Code (Copilot) | `.vscode/mcp.json` (`servers`, with `"type": "stdio"`) | `.github/copilot-instructions.md` block |
| Gemini CLI | `.gemini/settings.json`, `~/.gemini/settings.json` | `GEMINI.md` block |

A tool with no file for a scope (Codex and Windsurf have no project-level MCP file) is reported as
`skipped` with the reason, not guessed at.

## How careful it is

- **Merges, never replaces.** An existing config is parsed and the server added; other servers and
  settings stay. A file that does not parse is left alone and reported.
- **Keeps the original.** The first time a file is changed, a copy is left next to it as
  `<file>.<name>-backup`.
- **Shares files politely.** Instructions in `CLAUDE.md`, `AGENTS.md`, `GEMINI.md` and
  `copilot-instructions.md` go between `<!-- <name>:start -->` and `<!-- <name>:end -->`, and are replaced
  in place next time, so two servers can each have a block.
- **Idempotent.** Running it again reports `unchanged`.
- **Dry run.** `dry_run: true` reports `created` or `updated` for each file and writes nothing.

## What it does not do

- It does not start the tool, check that the server works, or restart anything: tell the user to restart
  the tool.
- It writes stdio servers only (a command and arguments), not remote HTTP servers.
- The file locations are the ones each tool documents at the time of writing; they do change. A path that
  is wrong is a small fix in one `match`, and a pull request is welcome.

## Licence

MIT.
