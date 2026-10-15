# Design: Bruno `.bru` files and a collection model

Status: proposal, for #203 to #206. Written after reading Bruno's own parser and fixtures (MIT licensed, `packages/bruno-lang`) and measuring Plunger's code.

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

## 6. Decisions needed

1. **Names unique per group** (0d): yes? It is what makes a Bruno folder importable without renaming.
2. **A disabled header**: add an `enabled` flag to headers in Plunger (a real feature, as Bruno and Postman have), or keep disabled headers only in `foreign`?
3. **Environments**: map a Bruno environment to Plunger variables (a set per name, switchable), or only import the active one as variables? Plunger has a single global set today.
4. **Scripts**: kept and reported only (this design), or an explicit "not supported" refusal on import?
