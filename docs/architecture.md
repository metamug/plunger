# Architecture: a map for contributors

This is the short version. [design.md](../design.md) has the reasons behind each choice.

## One program, three faces

`plunger` is a single executable. `main.rs` looks at the arguments and starts one of:

- **the window** (no arguments): egui, in `src/app/`;
- **the command line** (`plunger send`, `plunger curl`, `plunger workflow`, ...): `src/cli.rs`, `src/curl_cli.rs`;
- **an MCP server** (`plunger mcp`): `src/mcp.rs`.

All three send requests through the same path and write to the same SQLite database, so what an agent does shows up in the window.

## The path of a request

```
form or agent input
      |
 request.rs   prepare_to_send: {{variables}} filled in, body built, undefined names refused
      |
  http.rs     blocking reqwest, returns ResponseData
      |
 history.rs   one row per send (credentials blanked, placeholders kept)
      |
 engine/      for agents only: secrets masked (Scrubber), body cut or outlined, select / extract applied
```

## Where things live

| You want to change | Look in |
|---|---|
| How a request is built or sent | `request.rs`, `http.rs`, `vars.rs`, `query.rs` |
| Something in the window | `src/app/`; each editor tab is a file in `src/app/request/` |
| A menu entry or a keyboard shortcut | `app/actions.rs`: one table of actions drives the menus, the keyboard and Help > Keyboard shortcuts |
| Layout of request and response | `app/request/mod.rs` (heights), `app/mod.rs` (`render_divider`), `app/tab.rs` (`Pane`) |
| What an agent gets back | `engine/response.rs` (`AgentResponse`, `Scrubber`), `outline.rs`, `workflow/extract.rs` (`select`) |
| An MCP tool | `mcp.rs`; its logic is in `agent/` |
| A CLI command | `cli.rs` (help text is in `command_help`) |
| curl, PowerShell or cmd import and export | `curl_import.rs`, `curl_export.rs`, `commands/` |
| Secrets | `secrets.rs` (OS credential store), `redact.rs`, `engine/response.rs` (`Scrubber`) |
| Setting up AI tools | `install.rs` (data table of tools and file formats), `app/agents_window.rs` |

## Rules that keep it safe

- **A secret never reaches a file, the history, or agent output.** Variables named like credentials are held in the OS credential store; history stores `{{placeholders}}`; `Scrubber` masks any known secret in a response before an agent sees it.
- **Dependencies point downward.** The window and the agent layer use the core (`model`, `request`, `http`); the core knows nothing about either.
- **Do the same work in the window and for agents.** If a fix is in one path, check the other: the window's Ctrl+Enter and `agent::send_request` both end in `engine::send` / `request::prepare_to_send`.

## Tests

`cargo test` runs everything, including headless drawing of every window state (`src/app/tests.rs`), a local echo server for HTTP tests (`src/test_server.rs`), and tests that fail if a tool is added to the MCP server without being described. `scripts/shop-api.py` is a fake API for trying the window, CLI and MCP by hand.
