# Scripting with Plunger

Plunger runs a job that takes many requests as one saved workflow, or a file you run with `plunger run`. You write it as data, not code: each step says what to send and what to do with the answer. This is the guide an agent reads (the MCP server serves it as `plunger://scripting`) and the page to start from if you were about to write a Python script that calls an API.

**Use a workflow instead of a script when** the job is "call an API, follow its pages, repeat for each item, wait for something, boil it down". You get Plunger's secret masking, its history and the same behaviour from the window, the command line and MCP. **Write real code when** the job needs files, a database, other programs or logic the steps below cannot say; that code can still send its requests with `plunger send`.

## The idea in one example

Pull every issue of a sprint, however many pages, and answer with ten lines instead of two hundred issues:

```json
[
  {"label": "issues in the sprint",
   "url": "{{base}}/rest/api/3/search?jql=sprint={{sprint}}&startAt={{page.offset}}&maxResults=100",
   "headers": {"Authorization": "Bearer {{jira_token}}"},
   "paginate": {"items": "json:$.issues", "total": "json:$.total"},
   "collect": "issues"},
  {"label": "summary",
   "return": "{issues: (.issues|length), points: ([.issues[].fields.storypoints // 0] | add), unassigned: [.issues[] | select(.fields.assignee == null) | .key]}"}
]
```

Over MCP: `save_workflow` with these steps, then `run_workflow` with `variables` `{"base": "https://example.atlassian.net", "sprint": "12"}`. The result has each step's status, and `output` holds the answer. From a terminal, put the steps in a file and run `plunger run sprint.json --var sprint=12`.

## How a run works

Steps run in order and the first failing step ends the run. A step does, in this order:

1. `skip_if` / `stop_if` are checked; a skipped step counts as passed.
2. The request is sent. With `for_each` once per element; with `paginate` once per page; with `poll` until a condition holds; with `retry` again after a rate limit or a server error.
3. The status is checked (any 2xx, or `expect_status`).
4. `asserts` run (all of them, and every failure is reported).
5. `extract` reads values from the response; `collect` keeps what the step fetched.
6. `return` runs a jq program on everything the run has collected.

Values live in `{{name}}` variables. `extract` with `"scope": "run"` and `collect` keep a value for this run only, nothing is stored; `extract` without a scope stores a variable that stays afterwards (a name like `token` or `password` goes to the credential store and is never shown).

## Step fields

| Field | What it does |
|---|---|
| `url`, `method`, `headers`, `json`, `body`, `form`, `saved_request` | The request, as in `send_request`. |
| `expect_status` | A status, or a list (`[200, 404]`). Default: any 2xx. |
| `asserts` | `[{"from": "json:$.id", "equals": 7}]`. `from` is `json:$.path` (or `$.path`), `header:Name`, `status`, `time` (ms) or `body`. Checks: `equals`, `not_equals`, `contains`, `exists`, `gt`, `gte`, `lt`, `lte`, `type`, `length`. |
| `extract` | `[{"name": "token", "from": "json:$.token", "scope": "run"}]`. |
| `paginate` | Next page until done. `style`: `offset` (default), `page`, `cursor` (needs `next`), `link`. Also `items`, `total`, `next`, `page_size`, `max_pages`. |
| `for_each` / `as` | Once per element of a collected list (`"issues"`) or a written one (`[1, 2, 3]`). |
| `collect` | Keep what the step fetched as `{{name}}`: the items of all pages, the body, or a list with one entry per element. |
| `poll` | `{"until": "{{state}} == DONE", "every_ms": 2000, "max_attempts": 30}`. |
| `retry` | `{"attempts": 3, "backoff_ms": 500, "on_status": [429, 503]}`. Waits for `Retry-After` when sent. |
| `continue_on_error`, `delay_ms` | With `for_each`: keep going past a failed element. Wait between requests. |
| `skip_if`, `stop_if` | `{{a}} == b`, `{{a}} != b`, or a value that is true unless empty, `false`, `0` or `null`. `{{previous_status}}` is the last status. |
| `return` | A jq program. A step may have only this. The last one's result is the run's `output`. |

What a request can use inside a step:

- `paginate`: `{{page.number}}` (1, 2, ...), `{{page.offset}}` (items fetched so far) and `{{page.cursor}}` (write `{{page.cursor?}}` so the first page leaves it out).
- `for_each`: `{{item}}` (or the name in `as`); for an object `{{item.key}}`, `{{item.fields.name}}`; `{{loop.index}}` (from 0) and `{{loop.number}}` (from 1).
- Any variable, any value a step `extract`ed or `collect`ed (collected values over 100 KB are kept out of requests; they remain for `for_each` and `return`), and the built-ins `{{$uuid}}`, `{{$timestamp}}`, `{{$timestampMs}}`, `{{$isoTimestamp}}`, `{{$randomInt}}`, `{{$randomHex}}`, `{{$randomString}}`, `{{$env:NAME}}`.
- A variable is text. In a `json` body `"qty": "{{n}}"` sends a string; to send a number write the body as text: `"body": "{\"qty\": {{n}}}"` with `"Content-Type": "application/json"`.

## Recipes

### Pages of an offset API (Jira)

`paginate` with `items` and `total` stops exactly at the end. Without `total` it stops at a short page (set `page_size` to the size you ask for) or an empty one.

### Pages that follow a Link header (GitHub)

```json
[
  {"label": "open pull requests",
   "url": "https://api.github.com/repos/{{repo}}/pulls?state=open&per_page=100",
   "headers": {"Accept": "application/vnd.github+json"},
   "paginate": {"style": "link"}, "collect": "prs"},
  {"return": "{open: (.prs|length), by_author: (.prs | group_by(.user.login) | map({(.[0].user.login): length}) | add)}"}
]
```

A relative `Link` (`</items?page=2>`) is joined to the server the step asked.

### Pages that give a cursor (Slack)

```json
{"url": "https://slack.com/api/conversations.list?limit=200&cursor={{page.cursor?}}",
 "headers": {"Authorization": "Bearer {{slack_token}}"},
 "paginate": {"style": "cursor", "items": "json:$.channels", "next": "json:$.response_metadata.next_cursor"},
 "collect": "channels"}
```

An empty or missing cursor ends the paging.

### One request per item (worklogs of every issue)

```json
[
  {"url": "{{base}}/search?jql=sprint={{sprint}}&startAt={{page.offset}}", "paginate": {"items": "json:$.issues", "total": "json:$.total"}, "collect": "issues"},
  {"for_each": "issues", "as": "issue", "url": "{{base}}/issue/{{issue.key}}/worklog",
   "retry": {"attempts": 3}, "delay_ms": 100, "continue_on_error": true, "collect": "worklogs"},
  {"return": "{hours: ([.worklogs[] | select(. != null) | .worklogs[].timeSpentSeconds] | add / 3600)}"}
]
```

`collect` on a `for_each` is a list with one entry per element, `null` for one that failed with `continue_on_error`. The step's `warning` says how many failed.

### Wait for a job

```json
{"url": "{{base}}/exports/{{export_id}}", "poll": {"until": "{{state}} == done", "every_ms": 2000, "max_attempts": 60},
 "extract": [{"name": "state", "from": "json:$.state", "scope": "run"}]}
```

`poll` fails with "gave up after N attempts" if the condition never holds.

### Ride out a rate limit

`"retry": {"attempts": 5, "backoff_ms": 500}` retries a 429 or 5xx (and a dropped connection), waiting for `Retry-After` when the server sends it, otherwise 500 ms, 1 s, 2 s and so on (never more than 60 s).

### Check an API in CI

```
plunger run checks.json --env staging.env --data users.csv --junit report.xml
```

Exit code 0 when everything passed, 4 when a step failed. See [examples/](../examples/) for complete files.

## Writing the `return` program

The input is one object: every value the run has (`collect`ed lists as arrays, `scope: run` extracts as text, the variables passed in), without secrets. The program is [jq](https://jqlang.org/manual/):

- Numbers: `[.items[].points // 0] | add`, `length`, `max_by(.price)`, `min`, `map(.a) | unique`.
- Grouping: `group_by(.team) | map({(.[0].team): length}) | add`.
- Filtering: `[.items[] | select(.status == "open") | .key]`, `.items[:10]`, `sort_by(.created) | reverse`.
- A missing field is `null`; use `// "none"` to default it. Values extracted with `extract` are text: `(.count | tonumber)`.
- Several outputs become an array; none is `null`. A failing program fails the step and says why.
- It cannot read files, the network or the environment. A program that runs over 10 seconds is stopped. The result is cut off with an error past 500,000 characters, so return a summary, not the data.

Prefer a `return` over fetching everything and reading it yourself: you then read only the answer, and the data never needs to pass through the conversation.

## Limits and safety

- At most 500 pages and 100,000 items per step, 1,000 `for_each` elements, 50 steps per workflow.
- Secrets: values stored as secrets, the Bearer token and credential-named variables are masked in everything a run returns, kept out of the `return` input, and a collected response is scrubbed of the secrets Plunger knows. A token that arrives in a response and has not been extracted as a secret is not known yet; extract it first.
- A misspelt step field (`paginat`) is refused with a suggestion. `plunger run script.json --validate` checks a file and sends nothing; `save_workflow` checks the same things.
- A write (`POST`, `PUT`, `DELETE`) in a `for_each`, `retry` or `poll` is sent every time it is told to be. `retry` does not know whether a failed `POST` took effect, so use it on reads or idempotent calls.
