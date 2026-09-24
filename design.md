# Metamug API Tester — design spec

Scope for this round of changes, written up before continuing the implementation so it can be reviewed/redirected first.

## 1. UI fixes (small, in progress)

- **Method + URL merged visually.** Currently the method dropdown and the URL field sit side by side with a gap and slightly different heights. They should read as one control — no gap between them, and the URL field's height matched exactly to the dropdown's, the way Postman (and most API clients) join them.
- **Header input mode: checkbox, not tabs.** The "Table | Text" tab-style toggle becomes a single checkbox — e.g. "Edit as raw text" — unchecked (default) shows the Table (key/value rows), checked shows the free-text textarea. Removes a full tab row for something that's really a binary switch.
- **Status colors: 4xx orange, 5xx red, more saturated.** The current badge colors for 4xx/5xx are a muted amber-brown; bumping both to clearly-orange and clearly-red respectively so they're unambiguous at a glance.
- **Authorization field:** already moved into the Headers tab with an amber border (shipped last round) — no change here, just noting it stays.

## 2. Code refactor — module split

`main.rs` has grown to ~940 lines covering theme, state, networking, JSON highlighting, and all UI rendering in one file. Splitting into:

```
src/
  main.rs        entry point only — NativeOptions, window setup, hands off to App
  theme.rs        colors, apply_theme, card/accented_card, status_badge, status_dot_color, copy_icon_button
  model.rs        BodyMode, RequestTab, ResponseTab, PersistedState, ResponseData, ParsedRequest
  http.rs         parse_headers, format_bytes, send_request (networking, unchanged logic)
  json_view.rs    pretty_json_if_possible, highlight_json (the hand-rolled request-body highlighter)
  history.rs      SQLite: schema, open/init, insert, list_recent
  curl_import.rs  curl command parser + HAR file parser -> ParsedRequest
  app.rs          ApiTesterApp struct, eframe::App impl, trigger_send, all UI rendering
```

`theme.rs` and `model.rs` are already extracted (mechanical moves, no logic changes). Continuing with `http.rs`, `json_view.rs`, `history.rs`, `curl_import.rs`, `app.rs` next.

## 3. Request history — SQLite sidebar

**Storage:** `rusqlite` with the `bundled` feature (compiles SQLite from source via the C toolchain already set up — no new system dependency). DB file lives next to the existing eframe persistence data, via the `directories` crate: `%APPDATA%\Metamug API Tester\history.sqlite3` on Windows.

**Schema:**
```sql
CREATE TABLE IF NOT EXISTS requests (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at  TEXT NOT NULL,      -- RFC3339
    method      TEXT NOT NULL,
    url         TEXT NOT NULL,
    headers_text TEXT NOT NULL,
    body_mode   TEXT NOT NULL,      -- "None" | "Json" | "UrlEncoded" | "Raw"
    json_body   TEXT NOT NULL,
    urlencoded_body TEXT NOT NULL,
    raw_body    TEXT NOT NULL,
    status      INTEGER,            -- NULL if the request failed at the network level
    elapsed_ms  INTEGER
);
```

**Behavior:**
- Every send (success or failure) inserts one row after the response/error arrives.
- A left `SidePanel` lists the most recent ~50 requests, newest first: method, truncated URL, a small colored status dot (green/blue/orange/red/gray-for-failed).
- Clicking a row loads that request's method/URL/headers/body back into the form. It does **not** auto-send — loading and sending stay separate actions.
- The bearer token is never stored in history, same as it's never stored in the regular persisted state — it's a credential, not request shape.
- No row limit enforcement beyond what's shown (no pruning yet) — fine at this scale (a local single-user tool), flagged here in case it ever needs a cap.

## 4. Import — curl and HAR

One "Import" entry point (button near the command bar) opening a small window with two paths:

**Paste a curl command.** Tokenized with the `shell-words` crate (handles single/double-quoted and escaped arguments the way a real shell would, rather than a hand-rolled splitter). Recognized flags:
- `-X` / `--request` → method
- `-H` / `--header` → one header per flag, `"Name: Value"`
- `-d` / `--data` / `--data-raw` / `--data-binary` → body (implies POST if no explicit `-X`, matching real curl behavior)
- the first bare (non-flag) argument → URL

Anything else (`-u`, cookies, `--compressed`, `-k`, etc.) is silently ignored rather than erroring — the common case is a "Copy as cURL" paste from a browser's Network tab, which mostly uses the flags above.

**Import a HAR file.** Native file picker via `rfd` (sync API, no async/tokio pulled in — kept minimal). Parses `log.entries[].request` (method, url, headers, postData.text). Since a HAR from a browser session usually has many entries, the dialog shows a scrollable pick-list (method + URL) rather than guessing which one you want; clicking an entry populates the form the same way a curl import does.

## 5. Retry logic — dropped

Discussed and cut. Rationale: this is an interactive tool — a human triggers every request and is watching the result — which is a different problem from unattended/production retry-with-backoff. A manual retry is just clicking Send again, which already works; auto-retry would add a background backoff loop for a failure mode (transient flakiness during *unattended* use) that doesn't really apply here. Not worth the complexity against the "lightweight, quick" goal.

## Open items / things worth flagging

- HAR import's entry list has no search/filter — fine for typical HAR sizes from a single page load, could get unwieldy for a huge capture. Not solving that now.
- History has no delete/clear action yet — only additive. Worth adding a "Clear history" button at minimum.
- No dedup — sending the same request twice creates two history rows. Matches how Postman's history works too (not a collection), so treating this as correct rather than a gap.
