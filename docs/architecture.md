# Architecture: a map for contributors

This is the short version. [design.md](../design.md) has the reasons behind each choice.

## One program, three faces

`plunger` is a single executable. `main.rs` looks at the arguments and starts one of:

- **the window** (no arguments): egui, in `src/app/`;
- **the command line** (`plunger send`, `plunger curl`, `plunger workflow`, ...): `headless/cli.rs`, `headless/curl_cli.rs`;
- **an MCP server** (`plunger mcp`): `headless/mcp.rs`.

All three send requests through the same path and write to the same SQLite database, so what an agent does shows up in the window.

## Source layout

| Folder | What is in it | Knows about |
|---|---|---|
| `domain/` | The request itself: form state (`model`), building (`request`) and sending (`http`), `{{variables}}`, the query string, redaction, outlines, file names, time formatting | nothing else in the program |
| `store/` | The SQLite history (`history/`: requests and saved requests, with agent variables and workflows plus their runs in their own files) and the opt-in secret store | `domain` |
| `convert/` | curl, cmd and PowerShell commands and HAR, in and out | `domain` |
| `engine/`, `agent/`, `workflow/` | What an agent gets: sending with secrets masked, the tool logic, ordered steps with `extract` | `domain`, `store` |
| `headless/` | The ways in without a window: the CLI, `plunger curl`, the MCP server, `plunger install` | everything above |
| `ui/` | Drawing helpers: theme, icons, highlighting, the JSON tree, fallback fonts | `domain` |
| `app/` | The window: tabs, panels, menus and actions, sidebar, workflows window | everything above |

## The path of a request

```
form or agent input
      |
 domain/request.rs   prepare_to_send: {{variables}} filled in, body built, undefined names refused
      |
  domain/http.rs     blocking reqwest, returns ResponseData
      |
 store/history.rs   one row per send (credentials blanked, placeholders kept)
      |
 engine/      for agents only: secrets masked (Scrubber), body cut or outlined, select / extract applied
```

## Where things live

| You want to change | Look in |
|---|---|
| How a request is built or sent | `domain/request.rs`, `domain/http.rs`, `domain/vars.rs`, `domain/query.rs` |
| Something in the window | `src/app/`; each editor tab is a file in `src/app/request/` |
| A menu entry or a keyboard shortcut | `app/actions.rs`: one table of actions drives the menus, the keyboard and Help > Keyboard shortcuts |
| Layout of request and response | `app/request/mod.rs` (heights), `app/mod.rs` (`render_divider`), `app/tab.rs` (`Pane`) |
| What an agent gets back | `engine/response.rs` (`AgentResponse`, `Scrubber`), `domain/outline.rs`, `workflow/extract.rs` (`select`) |
| An MCP tool | `headless/mcp.rs`; its logic is in `agent/` |
| A CLI command | `headless/cli.rs` (help text is in `command_help`) |
| curl, PowerShell or cmd import and export | `convert/curl_import.rs`, `convert/curl_export.rs`, `convert/commands/` |
| Secrets | `store/secrets.rs` (OS credential store), `domain/redact.rs`, `engine/response.rs` (`Scrubber`) |
| Setting up AI tools | `crates/mcp-install` (the tools, their files and the safe writing, a reusable crate), `headless/install.rs` (Plunger's server and instructions), `app/agents_window.rs` |

## Rules that keep it safe

- **A secret never reaches a file, the history, or agent output.** Variables named like credentials are held in the OS credential store; history stores `{{placeholders}}`; `Scrubber` masks any known secret in a response before an agent sees it.
- **Dependencies point downward.** The window and the agent layer use the core (`model`, `request`, `http`); the core knows nothing about either.
- **Do the same work in the window and for agents.** If a fix is in one path, check the other: the window's Ctrl+Enter and `agent::send_request` both end in `engine::send` / `request::prepare_to_send`.

## Tests

`cargo test` runs everything, including headless drawing of every window state (`src/app/tests.rs`), a local echo server for HTTP tests (`src/test_server.rs`), and tests that fail if a tool is added to the MCP server without being described. `scripts/shop-api.py` is a fake API for trying the window, CLI and MCP by hand.
