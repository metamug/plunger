# Changelog

All notable changes to Plunger. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- A filter box in the sidebar searches the whole history and Saved list by URL, method, name or status.
- `plunger history --search TEXT` and a `search` option on the MCP `get_history` tool.
- Right-click a value in the response tree to copy its path (`$.items[0].name`) or its value.
- A Linux build (`plunger-linux-x86_64.tar.gz`) is attached to releases. The command line and MCP server are tested on Linux in CI; the window builds but has not been tried on a desktop yet.

### Changed

- A binary response is shown as binary (size, and Save writes the raw bytes) instead of as garbled text; the command line and MCP output carry `"binary": true` and no body.

### Fixed

- curl import: `-u`, `-b`, `-A`, `-e`, `--json` and `--data-urlencode` are understood instead of their values being taken as the URL; repeated `-d` are joined, `-G` and `-I` are honoured (#10, #11).
- A header value containing a line break is refused instead of becoming a second header (#12).
- An invalid HTTP method is refused before sending and no longer recorded in history (#13).
- `plunger history --limit` rejects 0 and negative numbers (#14).
- `--form` values are percent-encoded, and a repeated key is an error instead of silently dropped (#15).
- The text selection highlight uses conventional blue instead of an odd cyan (#16).
- Body boxes grow with their content and scroll, with no blank rows below short bodies (#17).
- Saved and history rows in the sidebar draw URLs in the same colour (#8).

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

[Unreleased]: https://github.com/metamug/plunger/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/metamug/plunger/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/metamug/plunger/releases/tag/v0.1.0
