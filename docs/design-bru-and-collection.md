# Design: Bruno `.bru` files and a collection model

Status: **parked** (decided 2026-10-15). Plunger does not need `.bru` to do its job. If it comes back, build the reader/writer and folder import/export first (an interchange format, no change to how requests are stored) and decide the on-disk format for a git workspace then; an own format that carries `select`, `extract` and workflows is a real alternative. The analysis below stays as the record. For #203 to #206.

## 1. Is the architecture good enough to build this on?

Short answer: **the layering is mostly right, but two things must change first, or `.bru` becomes a sixth ad-hoc conversion of "a request".**

### What is sound

- **Direction of dependencies** is what the architecture notes promise: `domain` is below `store` and `convert`, which are below `engine`, then `agent` and `workflow`, then `headless` and `app`. Measured over the 21,700 lines: `domain` imports nothing from `store`, `engine`, `agent` or `app`.
- **One send path** (`domain::request` builds, `domain::http` sends, `engine::send` records and masks, `agent` shapes) used by the window, the CLI and MCP.
- **The log is the write-ahead log**, the database is derived and rebuildable, and `apply()` is the single writer of the tables.
- **Tests in the right places**: unit tests next to the code, a headless drawing test of every window state, and an MCP scenario against a Jira-shaped fake API that runs in CI on three platforms.

### What is not

| # | Finding | Why it matters here |
|---|---|---|
| 1 | **Five representations of "a request"**: `PersistedState` (the window's form, with session settings mixed in), `SendParams` (an agent's call), `RequestSpec`, `HistoryEntry` / `RequestRow` (the stored row), plus the `Step` of a workflow. Conversions between them are hand-written and lossy. | #181 (`save_request` dropped `select` and `timeout_secs`) was exactly this. A `.bru` file carries more than any of them (enabled flags, path params, docs, scripts). |
| 2 | **`agent` and `workflow` depend on each other.** `agent::SendParams` holds a `workflow::Extract` and calls `workflow::extract`; `workflow` runs steps through `agent`. | The extraction language (JSONPath subset, filters) is a low-level concept used by both. `.bru` post-response vars map onto it. |
| 3 | **`domain::http` imports `ui::json_view`** (to prepare a tree for display). | The core is not pure. Anything that reuses `domain` (a future library crate, the `.bru` code) drags in egui types. |
| 4 | **`History` is a god object** (about 1,260 lines): SQL, row mapping, saved-request options, groups, workflows, variables, the commit path. | A folder-backed collection needs the same operations without SQL. |
| 5 | **`ApiTesterApp` has about 45 fields** and `app/` is 6,700 lines (a third of the code). | Not urgent; sub-structs for sidebar, dialogs and workflow state would help. |
| 6 | **Errors are `String`** all the way down. | Fine for humans; agents and the importer want codes (which file, which line, what was dropped). |
| 7 | **Names are globally unique** for saved requests (database triggers). | A Bruno collection has the same request name in different folders. With groups, uniqueness belongs to (group, name). |

None of these is a rewrite. Items 1 to 3 and 7 are prerequisites; 4 to 6 can follow.

## 2. The collection model (the missing middle)

A single in-memory document for "one item of a collection", independent of SQL, of the window and of any file format:

```rust
// domain/collection.rs
pub struct Item { pub name: String, pub group: Option<String>, pub seq: Option<u32>, pub request: RequestDoc }

pub struct RequestDoc {
    pub method: String,
    pub url: String,
    pub query: Vec<Entry>,          // Entry { name, value, enabled }
    pub path_params: Vec<Entry>,
    pub headers: Vec<Entry>,
    pub body: BodyDoc,              // None | Json | Text | Xml | Form | Multipart | GraphQl { query, vars }
    pub extract: Vec<Extract>,      // from vars:post-response
    pub expect_status: Option<ExpectStatus>,   // from assert { $res.status: 200 }
    pub options: SavedOptions,      // timeout, redirects, tls, select
    pub docs: String,
    pub foreign: Vec<RawBlock>,     // blocks Plunger does not understand: kept verbatim, never dropped
}
```

Conversions live in one module and are the only place the five representations meet:

```
RequestDoc <-> PersistedState (+ SavedOptions + group)   the window and the saved list
RequestDoc <-> SendParams                                 MCP and CLI
RequestDoc <-> RequestRow / HistoryEntry                  the store
RequestDoc <-> .bru text                                  convert/bru
```

The save/show/send round-trip test that already exists (`every_field_of_a_save_is_stored_kept_as_an_option_or_reported`) is extended to this type, so a field cannot be added to one side and silently lost.

## 3. The `.bru` adapter, in three small layers

Bruno's grammar (an Ohm grammar in `bruToJson.js`) is richer than the examples suggest: dictionary blocks, text blocks, lists, `~` for disabled, quoted keys, `@annotation(...)` lines, `'''` multi-line values, and a steady stream of new blocks (`auth:oauth2`, `grpc`, `ws`, `settings`, examples). **The parser must be tolerant and must keep what it does not understand.**

1. **`convert/bru/lex.rs`**: text to a list of blocks (`name`, kind, raw lines, parsed pairs). No meaning attached. Handles `\r\n`, tabs, trailing spaces, a missing final newline, quoted and disabled keys, annotations, `'''`. Never fails on an unknown block name.
2. **`convert/bru/request.rs`**: blocks to `RequestDoc` plus a list of **warnings** (what was imported, what was kept but is not used, what was dropped and why). Unknown blocks go to `foreign`.
3. **`convert/bru/write.rs`**: `RequestDoc` to text, in the order Bruno writes (`meta`, the method block, `params`, `headers`, `auth`, `body`, `vars`, `assert`, `script`, `tests`, `docs`), so a file that Plunger did not change produces no diff in git.

Plus `folder.bru`, `collection.bru` and environments (`vars { }`, `vars:secret [ ]`), which are small variants of the same lexer.

### Mapping

| Bruno | Plunger | Notes |
|---|---|---|
| `meta { name, seq }` | item name, order | `seq` kept; the list is still shown by group then name until ordering exists |
| `get|post|... { url, body, auth }` | method, URL, body mode | `body: json|text|xml|form-urlencoded|multipart-form|graphql` |
| `params:query` | the URL's query / Params rows | `~key` is a disabled row (Plunger has this) |
| `params:path` | `:id` is replaced in the URL | exported as a full URL |
| `headers` | headers | **a disabled header cannot be represented** in Plunger today: kept in `foreign` and reported |
| `auth:bearer`, `auth:basic` | an `Authorization` header | a literal token is **not** imported (it would be credential-blanked on disk): replaced by `{{bearer}}`, with a warning |
| `vars:post-response { t: $res.body.token }` | `extract` (`json:$.token`); `@name` is `scope: run` | |
| `assert { $res.status: 200 }` | `expect_status` | other assertions are kept and reported |
| `vars:pre-request` | kept, reported | a saved request does not carry per-request values (#181) |
| `script:*`, `tests` | **kept verbatim, never run** | Plunger does not execute JavaScript |
| `docs` | kept | a description field is a natural follow-up |
| `settings`, examples, `grpc`, `ws`, `auth:oauth2` | kept, reported | |

### Safety

- Nothing in a file is executed.
- File names written from request names are sanitised (no `/`, `\`, `:`, reserved Windows names; length-capped; collisions get a suffix).
- Reading refuses files over a size cap and does not follow symlinks out of the folder; recursion depth is capped.
- A literal credential in an imported file is reported, never silently written to the database or back to disk.

## 4. Collection stores (workspace mode)

```rust
trait CollectionStore {
    fn list(&self) -> Vec<Item>;  fn get(&self, group: Option<&str>, name: &str) -> Option<Item>;
    fn put(&self, item: &Item) -> Result<(), StoreError>;  fn remove(...);  fn move_to(...);
}
```

- `DbCollection` is what exists today (SQLite).
- `FolderCollection` reads and writes `.bru` files in a folder tree (a folder is a group). The database becomes an **index** of it: saved requests are rebuilt from the folder on open and when files change; the audit log records the changes either way.
- Workflows are our own `*.workflow.json` beside the requests: Bruno has no equivalent of `extract` chaining, `skip_if` or `stop_if`.

## 5. Order of work

| Step | What | Size | Depends on |
|---|---|---|---|
| 0a | Move `workflow::extract` and `Extract` down into `domain` (removes the cycle) | S | done |
| 0b | Take the display preparation out of `domain::http` (removes `domain` to `ui`) | S | done |
| 0c | `domain::collection` with `RequestDoc` and the conversions; extend the round-trip test | M | |
| 0d | Saved-request names unique per group (trigger migration) | S | groups (done) |
| 1 | `convert/bru`: lexer, request mapping, writer, with Bruno's fixtures and our own (#204) | M | 0c |
| 2 | Import and export a collection folder, with a warning report (#205) | M | 1 |
| 3 | `CollectionStore` and workspace mode (#206) | L | 2 |
| 4 | Split `History`; typed errors for the importer | M | |

Each step is a pull request that keeps tests green; 1 can be taken by an outside contributor once 0c exists.

## 6. The git-native workspace

This is what "git native" means concretely. A **workspace** is a folder. Everything a team would review in a pull
request is a file in it; everything personal or large stays out of it.

### 6.1 Layout

```
my-api/                       <- the workspace (a git repository)
  bruno.json                  <- marks the folder as a collection; also what Bruno looks for
  collection.bru              <- collection-wide headers, vars, auth (optional)
  environments/
    local.bru                 <- vars { base: http://localhost:3000 }   vars:secret [ token ]
    staging.bru
  Jira/                       <- a folder is a group
    folder.bru                <- group-wide headers / vars (optional)
    create-issue.bru          <- one request, one file
    Issues/
      get-issue.bru
  workflows/
    close-issue.workflow.json <- our own format: extract, skip_if, stop_if
  .gitignore                  <- written once: .plunger/
  .plunger/                   <- local only: the index database, the history, the audit log
```

What is **in** the folder: requests, groups, environments (names and non-secret values, secret *names* only),
workflows, documentation. What is **not**: history (every request an agent sent), agent variables, secret values,
the audit log, window state. Those stay in the app data folder as today, or in `.plunger/`.

### 6.2 Identity, names and files

- The **path relative to the workspace root** is the identity of a request: `Jira/Issues/get-issue.bru`. The name
  in `meta { name }` is what is shown; the file name is a slug of it.
- Rename = write the new file and delete the old one, which git sees as a rename. Moving to a group = moving the file.
- Slugs: lower-case, spaces to `-`, none of `/ \ : * ? " < > |`, no Windows reserved names, at most 80 characters;
  a collision on disk (including on a case-insensitive file system) gets `-2`. The slug is **stable**: renaming a
  request does not rename the file unless the user asks ("Rename file too"), so a one-word fix to a name is a
  one-line diff.
- Names are unique **per folder**, which is the same rule the file system gives for free (decision 1).

### 6.3 Which side wins

Two modes, chosen when a collection is opened and shown in the window title:

| Mode | Saved requests live in | Used for |
|---|---|---|
| **Personal** (today) | the SQLite database | quick work, no repository |
| **Workspace** | the folder; SQLite is only an index and the history | a team's API, reviewed in git |

In workspace mode the folder is the source of truth, the database can be thrown away and rebuilt from it (the same
property the audit log already gives), and `plunger` finds the workspace the way `git` finds a repository: by
walking up from the current directory to the first `bruno.json` (override with `--workspace` or `PLUNGER_WORKSPACE`).
An agent that runs in the repository therefore sees that repository's requests without any configuration, and
`plunger send "Jira/create-issue"` means the file in front of it.

Moving between modes is explicit: **Open folder as workspace**, **Import folder into my requests** (copy into the
database), **Export my requests to a folder**. Nothing is converted silently.

### 6.4 Reading and writing

- **Read**: scan on open (depth-capped, no symlinks out of the folder, size-capped), parse every `.bru`, fill the
  index. A file that does not parse is shown in the sidebar as a **broken item** with its error and is never
  rewritten or deleted.
- **Write is minimal.** The lexer keeps each block's original text. Saving re-renders only the blocks that changed
  and leaves the rest, the unknown blocks and the line endings alone, so *a request you opened and saved without
  changes produces no diff*. Writes are atomic (temp file, then rename).
- **`seq` is not renumbered.** A new request gets `max + 1`; reordering rewrites only the files that moved. Bruno's
  numbering is the cause of noisy diffs elsewhere; here it is touched as little as possible.
- **Outside changes** (a `git pull`, an editor): the folder is rescanned on a short timer by comparing modification
  times and a content hash, not by a file-watcher dependency. A tab with unsaved edits whose file changed shows a
  **"changed on disk"** bar with *Reload* and *Keep mine*; it never overwrites silently in either direction.
- **The audit log** records `RequestSaved { path, content_hash }` for what Plunger wrote, so `audit verify` can
  say which files changed outside the app.

### 6.5 Secrets

The rule is that a file in the folder can always be committed.

- A literal credential in a header, auth block or body is **never written**: the writer replaces it with
  `{{name}}`, stores the value as a secret variable, and tells the user once. A literal token found when reading is
  reported as a warning and shown masked.
- An environment file holds names and non-secret values; `vars:secret [ token ]` holds only *names*. The values
  come from the credential store, per workspace and environment.
- The existing masking of responses and history is unchanged.

### 6.6 What an agent sees

- `list_saved_requests` and `get_saved_request` read the index, so they show the folder's requests with `path` and
  `group`.
- `save_request` writes a file in workspace mode and says which one (`path`); it never touches files outside the
  workspace root.
- A request an agent saves is a normal file in the working tree: the user reviews it in `git diff` and commits or
  discards it. That is the point of the feature.
- Sending, history, extract, `select` and workflows behave as before.

### 6.7 Failure modes handled up front

| Case | Behaviour |
|---|---|
| File does not parse | Broken item in the list; never rewritten |
| Two files with the same name in one folder (case-insensitive file system) | Both shown with their path; the second is marked a duplicate |
| Folder is read-only or the disk is full | Save fails with the path and the reason; the tab keeps its edits |
| File deleted while its tab is open | Tab stays, marked "deleted on disk"; *Save* recreates it |
| Huge or binary file | Skipped with a warning (size cap) |
| Windows path length / reserved names | Slug rules above; refused with a message rather than a silent rename |
| Line endings | The file's own are kept (CRLF stays CRLF) |
| Merge conflict markers in a `.bru` | Parse fails, so it is a broken item; the user resolves it in git |

## 7. Tests this needs

- **Fixture corpus**: Bruno's own `.bru` files (MIT) plus ours; every file must lex, and every file that parses must
  re-render **byte-identical** when unchanged.
- **Property**: `parse(write(doc)) == doc` for generated `RequestDoc`s, with awkward values (quotes, `}`, `'''`,
  tabs, unicode, empty bodies, disabled rows).
- **Fuzz** the lexer: it must never panic and never loop on arbitrary bytes.
- **Minimal diff**: change one field, write, assert that only that block differs.
- **Round trip across the five representations** (the existing save/show/send test extended to `RequestDoc`).
- **Windows names**, long paths, case collisions, CRLF files.
- **Scenario**: the Jira scenario in CI runs once against a workspace folder instead of the database.

## 8. Rollout

1. `domain::collection::RequestDoc` and its conversions (0c), names per group (0d). No user-visible change.
2. `convert/bru` lexer, reader, minimal-diff writer (#204). Library only.
3. **Import / export folder** (#205): the lowest-risk way for users to try it, and it needs no mode switch.
4. **Workspace mode** (#206), behind an explicit "Open folder as workspace" menu item.
5. `plunger audit export --http` (#207) and `.http` export of a single request, for sharing.

Step 3 ships on its own and is already useful (take a Bruno collection, work in Plunger, export back). Step 4 is
the larger change and is only started once 1 to 3 are in use.

## 9. Decisions needed

1. **Names unique per group** (0d). *Recommended: yes.* It is what the file system gives anyway and what makes a
   Bruno folder importable without renaming.
2. **A disabled header.** *Recommended: add an `enabled` flag to headers in Plunger.* Bruno and Postman both have it,
   it is a real feature, and without it a round trip drops information. The alternative is keeping them only in
   `foreign`, which works but can never be edited in the window.
3. **Environments.** *Recommended: workspace environments become a switchable set of variables* (one active at a
   time, shown in the status bar and in the **Uses** line), with secret values per workspace in the credential
   store. Importing only the active one loses the switch that makes environment files worth having.
4. **Scripts.** *Recommended: keep and report, never run, never refuse.* Refusing makes a Bruno collection
   un-openable for the sake of a few `script:` blocks; keeping them verbatim means the file still round-trips for
   whoever runs them in Bruno.
5. **File name stability** (new, 6.2). *Recommended: the file name does not follow a rename unless asked.* It keeps
   diffs small and links in documents valid.
6. **When a workspace is the source of truth, may an agent create and delete files?** *Recommended: create and
   overwrite inside the workspace yes; delete only with a user confirmation in the window,* since a deleted file
   is the one change that is easy to miss in review.
