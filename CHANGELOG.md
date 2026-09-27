# Changelog

All notable changes to Plunger. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- A demo GIF of real Claude Code using Plunger over MCP (`docs/images/mcp-claude-code-demo.gif`), plus a raw-protocol version (`docs/images/mcp-demo.gif`) from a minimal reference MCP client (`scripts/mcp-demo-client.py`) included for anyone building their own client.

### Fixed

- Saving or renaming a request now refuses an exact name already used by another saved request (#38).
- Two saved requests with the exact same name: sending or exporting by name now refuses with a clear error instead of always silently picking the first one (#33).

## [0.2.1] - 2026-09-27

### Added

- A filter box in the sidebar searches the whole history and Saved list by URL, method, name or status.
- `plunger history --search TEXT` and a `search` option on the MCP `get_history` tool.
- Right-click a value in the response tree to copy its path (`$.items[0].name`) or its value.
- A Linux build (`plunger-linux-x86_64.tar.gz`) is attached to releases. The command line and MCP server are tested on Linux in CI; the window builds but has not been tried on a desktop yet.

### Changed

- A binary response is shown as binary (size, and Save writes the raw bytes) instead of as garbled text; the command line and MCP output carry `"binary": true` and no body.

### Fixed

- A large JSON response no longer freezes the window or uses gigabytes of memory: the tree shows the start of each long list, with a note; Copy and Save keep everything (#25).
- An 11 MB text response shows a 256 KB preview instead of using over 1 GB (#26).
- Text declared as ISO-8859-1 or windows-1252 is decoded correctly instead of showing replacement characters (#27).
- The method dropdown showed only five of its seven methods; HEAD and OPTIONS were hidden below the fold (#29).
- An empty response body says so instead of showing a blank box (#30).
- Text in scripts the bundled fonts lack (Chinese, Japanese, Korean, Hindi, Arabic, symbols such as a check mark) shows instead of empty boxes: a matching system font is loaded the first time such text appears (#28). Right-to-left text is drawn without reordering.
- curl import: `-u`, `-b`, `-A`, `-e`, `--json` and `--data-urlencode` are understood instead of their values being taken as the URL; repeated `-d` are joined, `-G` and `-I` are honoured (#10, #11).
- A header value containing a line break is refused instead of becoming a second header (#12).
- An invalid HTTP method is refused before sending and no longer recorded in history (#13).
- `plunger history --limit` rejects 0 and negative numbers (#14).
- `--form` values are percent-encoded, and a repeated key is an error instead of silently dropped (#15).
- The text selection highlight uses conventional blue instead of an odd cyan (#16).
- Body boxes grow with their content and scroll, with no blank rows below short bodies (#17).
- Saved and history rows in the sidebar draw URLs in the same colour (#8).
- On the light theme, the selected sidebar row and the Send button are blue instead of a washed-out cyan (#31).
- The Variables tab's help text wraps instead of running off the panel and pushing the Forget-secrets button out of view (#32).

## [0.2.0] - 2026-09-25

### Added

- **AI-native modes.** The same `plunger.exe` is now a command-line client and an MCP server:
  - `plunger send / import / saved / history / vars / export` print JSON, with distinct exit codes.
  - `plunger mcp` serves six tools over stdio: `send_request`, `import_curl`, `list_saved_requests`, `get_history`, `list_variables`, `export_curl`.
- All modes send through one engine, so an undefined `{{variable}}` is refused everywhere.
- Secret values never reach an agent; a server echoing one back is replaced with `[redacted:name]`.
- Requests record who sent them (window, CLI or MCP). The window shows agent requests as they happen, with a tag.
- `docs/agents.md`, with the `.mcp.json` setup, every tool, and the exit codes.
- Light theme, request tabs, named saved requests, a menu bar and a status bar.
- `PLUNGER_DATA_DIR` to run against a throwaway data folder.

### Changed

- The history database now uses write-ahead logging, so the window and an agent can write at the same time.
- Sending and curl import now share one code path between the window and agents.

## [0.1.0] - 2026-09-25

First public release: send requests with params, headers, JSON, form-urlencoded, raw and multipart bodies; `{{variables}}`; secrets kept in Windows Credential Manager on request; curl and HAR import; local history; TLS-skip for local servers.

[Unreleased]: https://github.com/metamug/plunger/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/metamug/plunger/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/metamug/plunger/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/metamug/plunger/releases/tag/v0.1.0
