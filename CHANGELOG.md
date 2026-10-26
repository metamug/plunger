# Changelog

All notable changes to Plunger. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.8.0] - 2026-10-26

Plunger can run scripts: files of requests that check their responses, page through lists, loop, wait, retry and boil the result down to an answer, with no scripting language to run.

### Added
- An **XML** body type next to JSON: the editor is highlighted, the request is sent as `application/xml` unless you set a Content-Type yourself, and `{{variables}}` are filled in. A curl command or an agent's text body that says XML (`application/xml`, `text/xml`, `+xml`) opens in it, and the curl, PowerShell, Python and JavaScript exports carry the right Content-Type. Thanks @tayfuryldz (#58, #9); the change is ported onto the current code.
- **Scripts, without code:** `plunger run script.json` runs a file of requests and exits non-zero when one fails, so it works in CI in place of Newman or `bru run`. Each step can have `asserts` (`{"from": "json:$.id", "equals": 7}`; `from` is a JSON path, `header:Name`, `status`, `time` or `body`; checks are `equals`, `not_equals`, `contains`, `exists`, `gt`, `gte`, `lt`, `lte`, `type`, `length`), and every assert is reported, not just the first failure. `--env FILE` (NAME=value lines or JSON), `--var`, `--data FILE` (CSV or JSON, one run per row) and `--junit FILE` (JUnit XML for GitHub Actions, Jenkins, GitLab). Saved workflows and `save_workflow` take `asserts` too.
- **Paging, loops, waiting and an answer, in the steps themselves:** `paginate` (offset, page, cursor or `Link` header; `{{page.offset}}`, `{{page.number}}`, `{{page.cursor}}`), `for_each` (one request per element of a collected list, with `{{item.field}}`), `collect`, `poll` (repeat until a condition holds), `retry` (after a 429, a 5xx or a dropped connection, honouring `Retry-After`), `continue_on_error`, `delay_ms`, and `return`: a jq program (the `jaq` engine, with no access to files, the network or the environment) that turns what was collected into the run's `output`. Secrets are kept out of the program's input and masked in its result. A step may hold only a `return`.
- `plunger run --validate` checks a script without sending anything, and a misspelt step field (`paginat`) is refused with a suggestion instead of being ignored.
- More built-in variables: `{{$timestampMs}}`, `{{$isoTimestamp}}`, `{{$randomHex}}` and `{{$randomString}}`.

## [0.7.4] - 2026-10-21

Secrets stay out of what is stored, and two TLS advisories are fixed.

### Added
- CI checks every dependency for RustSec advisories, licences and sources (`cargo deny`, configured in `deny.toml`) (#148).
- A headless test draws every dialog at laptop and small-screen sizes in both themes and fails if one reaches past the screen (#147).

### Security
- Updated `rustls` (0.23.46), `rustls-webpki` (0.103.16, RUSTSEC-2026-0382: certificate verification could accept distrusted intermediates) and `webpki-root-certs` (1.0.10, RUSTSEC-2026-0383: bundled roots allowed distrusted emSign intermediates). CI now fails on a new advisory.
- Credentials in a request **body** are no longer written to disk. In a JSON, form or XML body, and in a raw body that looks like one, the string value of a member or field named like a credential (`password`, `token`, `secret`, `api_key`, ...) is blanked before the request is stored in the history, a saved request or the audit log. The request that is sent is not changed, a `{{placeholder}}` is kept, and `"tokens_used": 5` is left alone. The history database is scrubbed of earlier rows when it is opened. A value that an earlier version already wrote to a hash-chained `audit.log` stays there; delete the file if that matters (a new one starts). See `docs/privacy.md`.

### Fixed
- A request an agent sent that got no response (connection refused, DNS failure, timeout) now keeps the reason: opening it from the history shows the error text in the response pane instead of "no response". A secret variable's value, and a credential in the URL's query string or `user:password@`, is masked in that text before it is stored, in the database and in the audit log. Thanks @jay23606 (#211, #118).

## [0.7.3] - 2026-10-15

### Changed
- The request tab strip scrolls to the tab you open or switch to, and the mouse wheel scrolls it sideways, so a tab is never cut off at the edge.
- A new JSON body starts empty with a hint instead of a pre-filled `{"key": "value"}` that would be sent if you forgot to edit it. The Raw editor says what it is and where the Content-Type goes.
- Tabs that would read the same (same method and title) are told apart on the strip by host, then query, then a count: `items (2)`.
- The method menu shows each method in its colour.
- Selected controls (request tabs, selected options) are a calm slate with an amber label instead of saturated blue, and the divider between request and response lights up in a thin amber line instead of a thick bright one.

### Fixed
- Move to group: the "New group" field and the list of groups in a saved request's right-click menu no longer vanish when you click them (#208).
- The JSON and raw body editors do not wrap a long line in the middle of a value, before or after Format: the line stays whole and the box scrolls sideways (#209).
- Agent requests you opened from the history only to look at them (not saved, not sent again from the window) are not brought back as tabs the next time Plunger starts (#210).

## [0.7.2] - 2026-10-14

### Added
- `plunger where` prints the version, the program, the data folder, the history database and audit log in use, and how many saved and history requests are in it, so you can tell whether the window and an agent share one history (they do not when they run with different data folders, for example from a sandboxed app).

### Fixed
- `$.items[*].owner.n` gives one entry per item: an item that lacks the value, or whose parent is null, gives `null` instead of being left out, so the list lines up with `$.items[*].id`.
- A step's own `variables` win over the run's: a workflow-level value no longer replaces a step's override.
- A `=` in a form value is percent-encoded, so a variable's value can never become another `key=value` pair.
- When some `select` paths cannot be read, the ones that can are returned and `problems` names the rest; the whole response is returned only when every path fails.
- `max_body_chars` is described as it behaves: a long JSON body returns an outline, a long text body is cut.

## [0.7.1] - 2026-10-14

### Fixed
- History stopped saving new requests when the table held a row the audit log's counter had not seen (an older build wrote it after the log started): the next id collided and the insert was lost without a word. Ids now continue after the highest row in the table. If a history write still fails, the window says so ("History was not saved: ...").
- Dialogs (Variables, Workflow, Import, Export, Set up AI agents, Shortcuts, About) no longer grow past the window on a laptop screen; they stop short of the edges and scroll.

### Changed
- A calmer look: neutral borders, flat menus, a quiet shadow under dialogs, amber kept for the active tab, Send and the selected row. Send is as tall as the URL box, and Save and Import are in the File menu only, not next to Send.
- What an agent is told matches what Plunger does: the server instructions and the installed skill (`plunger install`) now cover groups, `select` filters, optional and default placeholders, JSON escaping, conditional workflow steps, `include_headers`, redirect and binary fields. A test fails if a tool is missing from the instructions, if a send option has no description in the tool schema, or if the skill loses a key feature. Re-run `plunger install` to refresh an installed skill.

## [0.7.0] - 2026-10-14

Agents get answers they can use and cannot fail on quietly, requests can be organised in groups, and every change is written to a log the database is derived from.

### Changed
- A workflow's steps are drawn as a flow: a card per step on a rail of connected dots, the dots coloured by the last or running run (a diamond is a step that can be skipped, a square one that can end the run), the values a step hands to later ones written on the connector, and the values the workflow needs from outside listed on top.
- The Variables window explains itself with a help icon instead of a paragraph.
- Variables are no longer a tab in every request, because they are shared by all requests, the command line and agents. **Tools > Variables** opens them in a window, and each request shows only the ones it uses under the URL (`Uses base = https://..., token = [secret]`). A request opened from the history shows the values it was sent with (non-secret ones; a secret is never kept) (#191).
- The Auth tab is gone: Authorization is a header (`Authorization: Bearer {{token}}`). A Bearer token saved in the old tab becomes the secret variable `bearer` and is no longer added to every request behind your back. Ctrl+1 to Ctrl+4 now open Params, Headers, Body and Options (#192).

### Added
- A response says how fast it started (`ttfb_ms`) and where a redirect went: `final_url` and `redirects` (each hop's status and destination), absent when there were none (#94).
- A binary response gives its `sha256` and a sniffed `detected_type` (`png`, `pdf`, `zip`, ...), so an agent can check a download without receiving the bytes (#99).
- `include_headers` (`all`, `none` or `relevant`) on `send_request` and `--include-headers` on `plunger send`; the relevant ones are location, link, retry-after, content type and disposition, etag, rate-limit headers and the names of cookies (#104).
- `plunger send --out FILE` writes the whole body to a file (raw bytes for a binary one) and prints the result with `body_file` (#139).
- Export a request as Python (`requests`) and as HTTPie, from File > Copy as and File > Export request (#136, #137).
- The audit log is moved aside as `audit.<seq>.log` when it passes 20 MB, and the new file starts with a snapshot that names the hash it continues, so `plunger audit verify` still passes on each file.
- A tab without a name is titled by the last two path segments (`users/42`, not `42`) (#127), and a tab restored after a restart still says how its history request went (#164).
- An audit log: every change to the history, saved requests, workflows and agent variables is first written as a line of `audit.log` (who, when, what; no secrets, no response bodies), each line hash-chained to the one before, and applied to the database from it. `plunger audit verify` finds an edited, removed or reordered line (exit 4), `audit show` lists the changes, `audit head` prints the newest hash, and `audit rebuild` recreates the database from the log. An existing database starts its log with a snapshot (#199, #200).
- Groups for saved requests and workflows, like Postman's folders: `Jira`, or `Jira/Issues` for one level under it. The sidebar shows a collapsible header per group, and a row's right-click menu has **Move to group** (existing groups, no group, or a new name). Over MCP, `save_request` and `save_workflow` take `group`, `list_saved_requests` and `list_workflows` take a `group` filter (a group includes what is under it), and a new `set_group` tool moves one without re-sending it. On the command line: `--group` on `save` and `workflow save`, `saved --group G`, `workflow list --group G`, `saved group <name> <group>` and `workflow group <name> <group>` (#194).
- Placeholders can have a default, `{{priority|Medium}}`, and can be optional, `{{epic?}}`: with no value, an optional one is left out of a JSON body (the member or array item goes) and is empty elsewhere. One request can create an issue with an optional assignee, labels and epic (#184).
- Workflow steps can accept several statuses (`"expect_status": [200, 404]`) and can be conditional: `skip_if` leaves a step out and `stop_if` ends the run successfully, with `{{a}} == b`, `{{a}} != b` or a truthy value, and `{{previous_status}}` for the status before (#188).
- A workflow `extract` can be `"scope": "run"`, kept for that run only, and `run_workflow` takes `cleanup: true` (`--cleanup` on the command line) to remove the variables a run stored, so values from old runs do not pile up (#189).
- `select` filters also take `!=`, `[?(@.status!='Done')]` (#183).
- File > Copy as and File > Export request can write a request as a JavaScript `fetch` call that runs in Node.js 18+. A JSON body becomes `JSON.stringify({...})`, and `{{variables}}` stay as placeholders.
- `select` and `extract` understand a filter, `[?(@.name=='Done')]` (equality only), so an item can be found by name in one call (#183). With `select`, the response headers are left out (#182), a path through a `null` is `null` instead of a problem (#187), and when a path cannot be read the whole response is returned with the reason instead of nothing (#186).

### Fixed
- An agent's common mistakes are understood or refused by name instead of failing oddly or being ignored: `headers` as a list of `{name, value}` objects or `"Name: value"` strings (#89); `json` given as a string that holds a JSON object; an unknown field such as `retries` is refused with the list of real ones; a `form` value that is a file path (`@C:/x.txt`) is refused with how to upload (#108); a `timeout_secs` outside 1 to 600 and a non-standard method are noted in `problems`; the undefined-variable message tells an agent what it can do rather than pointing at the window.
- Under the URL, a placeholder that is optional (`{{who?}}`) or has a default (`{{x|dflt}}`) is no longer shown as undefined.
- A value could change the shape of a request outside a JSON body. In a header, a line break started another header; in a form body, `&` or `=` added fields; in a URL path, `?` or `#` began a query or fragment. A header value with a line break is now refused, form values are percent-encoded and path values keep `?`, `#`, spaces and control characters from changing the URL (#190).
- A raw body sent with a JSON content type (how an agent sends `"labels": {{labels}}`, which is not a JSON value it could pass as `json`) is filled in as JSON: values are escaped and optional members can be left out. `--json` on the command line accepts a body with such placeholders (#190).
- An object that an optional placeholder leaves empty is removed with it (`"assignee": {"name": "{{who?}}"}`).
- A `{{variable}}` inside a JSON body string is now JSON-escaped. A value with a newline, a quote or a control character made the server refuse the body, and a backslash could silently turn into a different character (`C:\temp` arrived as a tab and `emp`). A value outside a string is inserted as JSON, so a variable can be a number or a whole array or object. A value you pre-escaped as a workaround is now escaped again: pass the real text (#180).
- `save_request` kept neither `select` nor `timeout_secs` (nor `follow_redirects`, `insecure_tls`), so the next send of the saved request used the defaults. They are saved, shown by `get_saved_request` and used when it is sent by name. Fields that cannot be kept (`variables`, `use_saved_bearer`, `extract`, `max_body_chars`) are listed in `not_saved`, and a `select` that could not work is refused when saving (#181).
- Help > Documentation and Help > Report an issue did nothing since 0.6.0 (the toolkit upgrade dropped the feature that opens the browser). They open the browser again.

## [0.6.0] - 2026-10-10

Workflows get a window of their own, the toolkit is current (egui 0.36), and the source is organised by what it does.

### Changed
- Dependencies brought up to date: egui and eframe 0.36 (from 0.29), egui_json_tree 0.17, reqwest 0.13, rfd 0.17, ron 0.12, keyring 4 and rmcp 3.5. No change in behaviour. TLS uses ring instead of the aws-lc-rs that reqwest 0.13 selects by default, and keyring stays on 3, which keeps the Windows executable about 1.5 MB smaller (10.0 MB; egui 0.36's new text engine accounts for most of the rest of the growth over 0.5.3's 7.1 MB).
- The source is grouped by what it does: `domain/`, `store/`, `convert/`, `headless/` and `ui/` next to `app/`, `engine/`, `agent/` and `workflow/` (see docs/architecture.md).
- The code that registers an MCP server in Claude Code, Cursor, Kiro, Codex, Windsurf, VS Code and Gemini CLI is now its own crate, `mcp-install` (`crates/mcp-install`), usable by any MCP server, with its own tests, README and an example. Plunger uses it for `plunger install`; nothing changes for users. The repository is now a Cargo workspace and CI tests all of it.

### Added
- Right-click a workflow in the sidebar for Open, Run, Copy as JSON and Delete; deleting asks first and also removes the workflow's recorded runs. The workflow window has a Delete button too.
- Workflows in the window: a **WORKFLOWS** section in the sidebar with each workflow's step count and last run (a dot and "3 min ago"); click one to see its steps, **Run** it with live per-step progress (and Cancel), and see every recorded run, including the ones an agent made, with each step's status, time and the variables it set. Every run is now recorded (who, when, each step's result; never values or response bodies). **Tools > Workflows** and **Ctrl+Shift+R** (Run the open workflow) are in the menus (#162).
- Homebrew: `brew install --cask metamug/tap/plunger` installs `Plunger.app` and a `plunger` command (the tap is metamug/homebrew-tap, tested on a macOS runner on every push).

## [0.5.3] - 2026-10-10

### Added

- A macOS app and disk image: each release now has `Plunger-<version>-macos.dmg`, one `Plunger.app` for Apple Silicon and Intel, with an icon, ad-hoc signed, built and checked by the release workflow (it is universal, the signature verifies, the window starts, the image mounts). A Homebrew cask is ready in `packaging/homebrew/`. See `docs/packaging-macos.md`.
- Ctrl+1 to Ctrl+6 (Cmd on a Mac) open the Params, Auth, Headers, Body, Variables and Options tabs of the request.
- A scripted, captioned demo video: Actions > Demo video records the real window on a macOS runner with real keyboard and mouse input and makes an MP4 and a GIF (`scripts/demo/`).
- A manual Screenshots workflow that runs the window on macOS and uploads a screenshot, so the window can be checked without a Mac.

### Changed

- The menu bar is now File, Edit, View, Request, Tools and Help, built from one table of actions that also drives the keyboard shortcuts, so a shortcut is always in a menu and shown next to its item (#102, #128, #129):
  - **Edit:** Copy URL, Copy response body, Find in response (Ctrl+F), Go to URL (Ctrl+L).
  - **View:** Theme (moved from Settings), Zoom in, out and reset (Ctrl+=, Ctrl+-, Ctrl+0, remembered), and Hide or show the sidebar (Ctrl+B).
  - **Request:** Send, Cancel, the Params, Auth, Headers, Body, Variables and Options tabs, Expand the request, Expand the response, Show both, Duplicate tab, Next and Previous tab.
  - **Tools:** Set up AI agents.
  - **Help:** Keyboard shortcuts (F1), Documentation, Report an issue (opens a GitHub issue with the version and system filled in), About Plunger.
- The status bar no longer carries the row of shortcuts; it says "F1 shortcuts" and the Help window lists them all. On a Mac they show as Cmd.
- The body type (a drop-down: No body, JSON, form-data, urlencoded, Raw), a Valid / Invalid chip with the line and column (hover for the message) and a Prettify button now sit at the right end of the request tab strip, as in Bruno. The row of body-type pills and the row under the editor are gone, so the editor gets the room. Prettify is greyed out while the JSON does not parse.
- The divider between the request and the response has a visible grip, so it is clear it can be pulled.
- The response's Headers tab shows how many headers there are.
- Request and response panels: the request panel is as tall as its content (the body editor no longer scrolls inside it) and grows until the response would be squeezed; it folds away when you send, so the response fills the window, and clicking any request tab brings it back. A two-arrow icon on each panel's header expands it, and shows arrows pointing in on the panel that is expanded. Dragging the divider still sets your own split (double-click resets), and a split you dragged is kept when you send.
- A body editor is as tall as its text plus a spare line (at least four lines) instead of a fixed block of blank lines the cursor could not enter; an expanded request gives the editor the whole surface.
- The regular CI now runs the tests, lint and smoke test on macOS too, not only a compile check.

### Fixed

- MCP: `list_saved_requests` and `get_history` advertised an array as their output schema, which is not valid in MCP 2025-06-18 and 2025-11-25 (strict clients reject it; the 2026-07-28 spec relaxed this). They now return an object, which every version accepts: `{"requests": [...]}` and `{"history": [...]}`. A test checks that every tool's output schema is an object (#97). The command line still prints plain arrays.
- `extract` and `select` mistakes (an empty or unknown source, a bad variable name, a malformed path) are reported before the request is sent, so a POST is not fired and then fails to extract; `plunger workflow save` checks the same. `plunger history --status banana` is an error, not an empty list.
- The macOS release builds failed their smoke test because `scripts/smoke-cli.sh` used GNU-only `date +%s%N` and `timeout`; it now works on macOS. The macOS tarballs for 0.5.2 were attached afterwards (the release workflow can attach them to an existing release).

## [0.5.2] - 2026-10-09

### Added
- The request editor and the response share the window more sensibly: before there is a response the request gets most of the height, and the JSON body editor shows more lines with a single scroll bar; once there is a response, the response gets the room. Drag the line between them to set your own split (double-click to reset). The expand chevron now makes the request fill the window and folds it back when you send, so the response is what you see (#158, #125).
- A request opened from the history says when it was sent, by whom, and how it went (status and time) where the response would be, since the response itself is not stored (#159).
- A raw body that is JSON, sent with a JSON `Content-Type`, opens in the JSON tab when a request is opened from the history or sent by an agent.
- `docs/architecture.md`: a map of the code for contributors (#146).

### Changed
- The window code is split into smaller files: tabs, lists, shortcuts and the tests are no longer in `app/mod.rs`.

## [0.5.1] - 2026-10-09

### Added

- `plunger install` and **File > Set up AI agents...**: register the MCP server in Claude Code, Cursor, Kiro, Codex, Windsurf, VS Code (Copilot) or Gemini CLI, and write the always-on steering (a `CLAUDE.md`/`AGENTS.md`/`GEMINI.md` block, a Claude skill, a Cursor rule, Kiro steering, a Windsurf rule) that tells the agent to send HTTP requests through Plunger instead of curl or Invoke-RestMethod. Existing config is merged, not replaced; a backup is kept; running it twice changes nothing; `--dry-run` shows the plan.
- A JSON response over `max_body_chars` comes back as an outline (keys, types, array lengths, an example each) with a hint to use `select`, instead of cut-off text.
- `[*]` in `select` and `extract` paths collects a field from every item of an array (`$.items[*].id`).
- `get_history` filters: `status` (`401`, `4xx`, `5xx`, `ok`, `fail`, `error`), `min_ms`, `source` and `saved_request`; `plunger history --status 5xx --min-ms 1000`. The new `get_history_entry` tool (and `plunger history show ID`) returns one entry in full, with the request as sent.
- `extract` on `send_request` (and `plunger send --extract name=json:$.path`): keep a value from the response as a variable in the same call, so a login and its token take one request instead of two. A credential-like name is a hidden secret, and the response that carried it is masked too. `variables_set` lists what was set.
- `select` on `send_request` (and `plunger send --select PATH`): return only the values you ask for (`$.data[0].id`, `header:Location`, `status`) under `selected`, instead of the whole body, so a big response costs a few tokens. A path that is not in the response is reported under `problems`.
- The time a request was sent: `sent_at` on every response an agent gets, and in the window the clock time on the response row, with the full date and the timing breakdown on hover (in your own time zone when the system tells us). The sidebar tooltip uses local time too (#159).
- Hide the request editor to give the response the whole height, or hide the response to give the editor the whole height: two chevrons at the right end of the request tabs (#158).
- `plunger-cli` on PyPI: the compiled program in a platform wheel (Windows, Linux, macOS), so `pipx install plunger-cli` or `uvx plunger-cli mcp` works with nothing else to download. Built with maturin and published from CI with PyPI trusted publishing.
- macOS builds (Apple Silicon and Intel) attached to each GitHub release as `plunger-macos-arm64.tar.gz` and `plunger-macos-x86_64.tar.gz`. They are not signed or notarized yet.

### Fixed

- Saving a response body suggests a real file name: the server's `Content-Disposition` name, then the last part of the URL (`401.jpg`), then `response` with an extension that fits the `Content-Type` (`.png`, `.pdf`, ...), instead of always `response.bin` (#154).
- A URL with no host, such as `https:///AphiaRecordsByAphiaIDs`, is refused with a clear error. It used to be read as host `AphiaRecordsByAphiaIDs` and sent to the wrong server (#155).
- The Variables tab counts the variables agents set, so they are not missed when you have none of your own (#157).

## [0.5.0] - 2026-10-07

### Added

- Workflows: `save_workflow`, `run_workflow`, `list_workflows` and `delete_workflow` over MCP, and `plunger workflow list|show|run|save|delete` on the command line. A step can `extract` a value from a response (`json:$.path`, `header:Name`, `status`) into a variable for the next steps; credential-like names stay secret and are never shown. A failing step stops the run.
- MCP resources (`plunger://guide`, variables, saved requests, workflows, history, and one per saved request and workflow) and prompts (`test_endpoint`, `login_workflow`, `debug_failed_request`, `record_workflow`).
- Agents can set variables. `plunger vars set NAME VALUE` (`--secret`, or `-` to read the value from standard input), `plunger vars unset`, `plunger vars clear`, and the MCP tools `set_variable` / `delete_variable` keep a value for later requests as `{{name}}`. It persists, is shared by the window, the CLI and MCP at once, and shows in the window under "Set by agents" (usable from the window too, with a delete button). A secret or a credential-looking name is kept in the system credential store only and masked in results. An agent cannot change or delete variables the user defined, and the user's win on a clash (#36).
- Agents can manage saved requests: `plunger save NAME ...` and the MCP `save_request` save a request without sending it, keep `{{placeholders}}`, and `--overwrite` / `overwrite: true` fixes an existing one; `plunger saved show|delete` and the MCP `get_saved_request` / `delete_saved_request` read one in full or remove it.
- `plunger curl` supports `--retry`, `--retry-delay`, `--retry-max-time` and `--retry-all-errors`: transient failures (timeouts, refused connections, 408, 429, 500, 502, 503, 504) are tried again, waiting for the server's `Retry-After`.
- Every command has its own help (`plunger vars --help`, ...), and a usage error points at the help of the command that was used.
- `plunger send --json @file` and `-d @file` (and `@-` for standard input) read the body from a file, which gets a JSON body past PowerShell's quote handling (#138).
- Export a request as a command: **File > Copy as** and **File > Export request...** write curl (bash), curl (Windows cmd), PowerShell `Invoke-RestMethod` or PowerShell `Invoke-WebRequest`, shown with syntax highlighting; **Ctrl+Shift+C** copies in the last format used. `{{variables}}` stay as placeholders, so an exported command never carries a secret (#81).
- Import reads those same syntaxes: curl for bash, curl for the Windows command prompt (including Chrome's caret-escaped "Copy as cURL (cmd)"), and PowerShell (including Chrome's "Copy as PowerShell" with its `$session` cookies and user agent). The dialog detects the syntax and highlights what you paste.
- Syntax highlighting where code is shown: in the import and export dialogs, in the raw and form body editors (JSON, XML/HTML or form detected from the text), in the raw headers editor and in plain-text responses; `{{variables}}` in the URL, params, headers and form-data fields are blue when defined and red when undefined.
- `plunger curl [curl options] <url>`: curl's own options on Plunger's engine, so a command written for curl runs unchanged. The response body is printed exactly as the server sent it; `-i -I -s -S -f --fail-with-body -L -k -m -o -w` work, `-d @file` and `-d @-` read the body, clustered flags such as `-sSL` and `-XPOST` are understood, and curl's exit codes are used (22, 6, 7, 28, 60). Undefined `{{variables}}` are still refused, secrets masked and the request recorded in the history. Options Plunger cannot honour (`--proxy`, `--cert`, ...) are refused, not ignored (#100).
- `plunger send https://...` accepts a URL as its first argument (it was read as a saved-request name).

### Changed

- The code is organised into smaller modules (`engine` and `agent` are folders), with the new `workflow` and `mcp_content` modules.
- Compact layout: the help paragraphs under the Params and Variables tabs are now hover tips on the tabs. The response's Body/Headers tabs, status, time and size share one row; hovering the time shows TTFB, download and sizes. The status bar no longer repeats the response numbers.
- The saved-requests list draws only the rows in view (fast with thousands of saved requests).
- The MCP server's instructions now describe every tool, how variables, secrets, redirects and large bodies behave, and what is not supported yet, so an agent knows the whole surface when it connects. A test fails if a tool is added without being mentioned.
- The Bearer token field has its own **Auth** tab instead of sitting above the headers, and the sentence that explained it is gone (#79, #83).
- The request tabs show what they hold: `Headers (2)`, like `Params (n)` and `Variables (n)` already did, and a bullet on `Body •` and `Auth •` when they are set.
- Less wasted space above the editors: the Headers raw-text switch moved into the tab strip, and the spacing and padding around the request editor are tighter, so the response gets the room.
- The sidebar shows a request's path and query instead of its host, with a smaller CLI/MCP tag, so rows that hit the same host can be told apart; the full URL is in the tooltip (#87, #91).

### Fixed

- Pasting a multi-megabyte body into the request editor no longer keeps a CPU core busy: above 128 KB the editors draw plain text and skip colouring and validation (the body editor says so), instead of tokenising and parsing it on every frame.
- A large XML or HTML response no longer uses over a gigabyte of memory and a full CPU core: the formatted view is built once per response from the first 256 KB (with the same "showing the first 256 KB" notice plain text has), instead of formatting and colouring the whole body on every frame. A 4 MB XML response now takes about 200 MB.
- An environment variable whose name looks like a credential (`API_TOKEN`, `STRIPE_KEY`, ...) used as `{{$env:NAME}}` is masked in results like a secret variable, and a saved request lists the environment variables it needs (`$env:NAME`).
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

[Unreleased]: https://github.com/metamug/plunger/compare/v0.7.4...HEAD
[0.7.4]: https://github.com/metamug/plunger/compare/v0.7.3...v0.7.4
[0.7.3]: https://github.com/metamug/plunger/compare/v0.7.2...v0.7.3
[0.7.2]: https://github.com/metamug/plunger/compare/v0.7.1...v0.7.2
[0.7.1]: https://github.com/metamug/plunger/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/metamug/plunger/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/metamug/plunger/compare/v0.5.3...v0.6.0
[0.5.3]: https://github.com/metamug/plunger/compare/v0.5.2...v0.5.3
[0.5.2]: https://github.com/metamug/plunger/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/metamug/plunger/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/metamug/plunger/compare/v0.4.2...v0.5.0
[0.4.2]: https://github.com/metamug/plunger/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/metamug/plunger/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/metamug/plunger/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/metamug/plunger/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/metamug/plunger/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/metamug/plunger/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/metamug/plunger/releases/tag/v0.1.0
