# Changelog

All notable changes to Plunger. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- `plunger curl [curl options] <url>`: curl's own options on Plunger's engine, so a command written for curl runs unchanged. The response body is printed exactly as the server sent it; `-i -I -s -S -f --fail-with-body -L -k -m -o -w` work, `-d @file` and `-d @-` read the body, clustered flags such as `-sSL` and `-XPOST` are understood, and curl's exit codes are used (22, 6, 7, 28, 60). Undefined `{{variables}}` are still refused, secrets masked and the request recorded in the history. Options Plunger cannot honour (`--proxy`, `--cert`, ...) are refused, not ignored (#100).
- `plunger send https://...` accepts a URL as its first argument (it was read as a saved-request name).

### Changed

- The MCP server's instructions now describe every tool, how variables, secrets, redirects and large bodies behave, and what is not supported yet, so an agent knows the whole surface when it connects. A test fails if a tool is added without being mentioned.
- The Bearer token field has its own **Auth** tab instead of sitting above the headers, and the sentence that explained it is gone (#79, #83).
- The request tabs show what they hold: `Headers (2)`, like `Params (n)` and `Variables (n)` already did, and a bullet on `Body •` and `Auth •` when they are set.

### Fixed

- A JSON response keeps the key order the server sent. It was shown (in the window, the CLI and MCP) with the keys sorted alphabetically.
- Clustered curl flags (`-XPOST`, `-sSL`) in an imported curl command are understood instead of being mistaken for the URL.
- Saving a request no longer erases `{{variable}}` placeholders in credential headers, cookies and query values (`Authorization: Bearer {{token}}`): only literal credentials are blanked. Before this, a saved request that used a token variable could never be re-run, from the window, the CLI or MCP (#92).
- A request sent from the CLI or MCP with a JSON `Content-Type` and a JSON body now opens in the window's JSON body editor instead of Raw, the same as a curl import (#82).
- JSON bodies from the CLI `--json` flag and the MCP `json` field are sent compact instead of pretty-printed.

## [0.4.2] - 2026-10-06

### Fixed

- MCP: `send_request`, `import_curl` and `list_variables` results no longer fail validation in clients that check structured output against the tool's schema ("data must have required property 'redacted'"). The empty `redacted`, `form_fields` and `problems` lists are now optional in the schema; a test checks every tool's output against its schema (#84).

## [0.4.1] - 2026-10-05

### Fixed

- Ctrl+F focuses the response search box, so typing goes into it instead of whichever field had focus (it could silently change the URL) (#67).
- Response search: Prev/Next, and Enter / Shift+Enter in the box, scroll to the match and mark the current one more strongly than the rest (#68).
- Response search highlights matches in formatted XML and HTML responses (#69).
- Response search no longer rebuilds its match list every frame, counts only what is shown, and stops at 5,000 matches (shown as `5000+`); a very common query on a 10 MB body no longer uses over 150 MB extra. Escape closes the search box, and it no longer appears for binary or empty responses (#70).
- Searching a JSON response expands the tree to the matches even after a different query or response was shown, instead of leaving it collapsed.
- Overlapping matches (for example `aa` in `aaa`) no longer repeat text in the highlighted view.
- After Ctrl+S (or a double-click rename) the whole name is selected, so typing replaces it instead of appending to the default name (#71).
- The Variables help text, `plunger vars` and the MCP `list_variables` tool mention `{{$env:NAME}}` (#72).
- `plunger import`, the MCP `import_curl` tool and the import dialog refuse text that isn't a curl command instead of inventing a request from its first word; a pasted `$ ` or `> ` prompt is ignored (#73).

## [0.4.0] - 2026-10-02

### Added

- Ctrl+F searches the response body: case-insensitive match count, Prev/Next, highlighted matches, and JSON tree nodes expand to the results (#18).
- XML and HTML responses are pretty-printed with syntax colouring; HTML void elements such as `<br>` and `<img>` keep indentation correct.

### Fixed

- Pressing Escape while renaming a saved request cancels the rename again; the Escape-to-cancel-request shortcut no longer swallows it.

## [0.3.0] - 2026-09-30

### Added

- `{{$env:NAME}}` reads an OS environment variable at send time, alongside the existing `{{$uuid}}`, `{{$timestamp}}` and `{{$randomInt}}` built-ins (#40).
- Ctrl+L focuses the URL field and selects its contents (#52).
- The response shows the followed redirect chain (status and destination for each hop) when redirects are on (#20).
- The response header and status bar show time to first byte (TTFB) alongside the total time (#21).
- The response header and status bar show the request body size alongside the response size (#37).
- Ctrl+Tab and Ctrl+Shift+Tab cycle through open request tabs, wrapping at
  either end (#39).
- A demo GIF of real Claude Code using Plunger over MCP (`docs/images/mcp-claude-code-demo.gif`), plus a raw-protocol version (`docs/images/mcp-demo.gif`) from a minimal reference MCP client (`scripts/mcp-demo-client.py`) included for anyone building their own client.

### Changed

- Bumped `rusqlite` to 0.40 and `directories` to 6.

### Fixed

- Saving or renaming a request now refuses an exact name already used by another saved request (#38).
- Two saved requests with the exact same name: sending or exporting by name now refuses with a clear error instead of always silently picking the first one (#33).
- Overriding a secret variable's value for one request (MCP/CLI `variables`) now correctly masks the override's value in the response, not the stale stored secret (#49).

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

[Unreleased]: https://github.com/metamug/plunger/compare/v0.4.2...HEAD
[0.4.2]: https://github.com/metamug/plunger/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/metamug/plunger/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/metamug/plunger/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/metamug/plunger/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/metamug/plunger/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/metamug/plunger/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/metamug/plunger/releases/tag/v0.1.0
