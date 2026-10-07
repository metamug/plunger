# Plunger for AI agents

Plunger is one executable with three ways in:

| You run | You get |
|---|---|
| `plunger.exe` | The window. |
| `plunger.exe <command>` | The command line: JSON on stdout, meaningful exit codes. |
| `plunger.exe mcp` | An MCP server on stdin/stdout, for Claude Code, Cursor and other MCP clients. |

All three send requests through the same engine as the window's Ctrl+Enter, and all three write to the same local history. You can open Plunger afterwards and see exactly what an agent sent.

## Why not just let the agent run curl?

- **Undefined variables are refused.** A request with an undefined `{{variable}}` is never sent. curl would send the placeholder text.
- **Secrets stay by name.** Secrets you've chosen to remember (the key icon on a secret variable or the Bearer field) live in Windows Credential Manager, and only those are available to an agent. The agent writes `{{token}}`; Plunger fills it in. No tool output contains a secret value, and if a server echoes one back, it is replaced with `[redacted:token]`.
- **Structured results.** Status, time, size, headers and the parsed JSON body come back as separate fields, and long bodies are cut to a size a model can read.
- **An audit trail.** Every agent request lands in Plunger's history, tagged `MCP` or `CLI`, and the window picks it up live.

## Set it up in Claude Code

Add Plunger to your project's `.mcp.json` (use the path where you unzipped it):

```json
{
  "mcpServers": {
    "plunger": {
      "command": "C:\\Tools\\plunger\\plunger.exe",
      "args": ["mcp"]
    }
  }
}
```

Or from a terminal:

```bash
claude mcp add plunger -- "C:\Tools\plunger\plunger.exe" mcp
```

Any MCP client that can launch a stdio server works the same way: the command is `plunger.exe` and the only argument is `mcp`.

Once it's added, an agent's calls show up in Plunger's own history live, tagged `MCP`. This is real Claude Code, unedited, asked to fetch a joke through Plunger and then asked twice more:

<p align="center">
  <img src="images/mcp-claude-code-demo.gif" alt="Claude Code calling Plunger's send_request tool three times for a random joke; each call shows up live in Plunger's history on the right, tagged MCP, while Claude's replies stream on the left." width="900">
</p>

### Seeing the raw protocol

Claude Code's chat hides the actual bytes on the wire behind "Used plunger: Send an HTTP request." If you're building your own MCP client and want to see the literal JSON-RPC frames, [`scripts/mcp-demo-client.py`](../scripts/mcp-demo-client.py) is a minimal reference client with no SDK — it does the same initialize handshake and one `tools/call`, and prints every frame:

```bash
python scripts/mcp-demo-client.py path\to\plunger.exe
```

<p align="center">
  <img src="images/mcp-demo.gif" alt="The reference script's output: the initialize handshake, then a tools/call frame for send_request and its reply, printed as JSON next to the Plunger window, which picks up the request live." width="900">
</p>

## MCP tools

| Tool | What it does |
|---|---|
| `send_request` | Sends a request and returns `status`, `ok`, `elapsed_ms`, `size_bytes`, `headers`, and the body (`json` when it parses, otherwise `body`). Send a saved request by name with `saved_request`, or describe one with `method`, `url`, `headers`, and one of `json`, `body` or `form`. `variables` adds or overrides `{{variables}}` for this request only. |
| `import_curl` | Parses a curl command into a request without sending it. With `save_as`, adds it to the Saved list. |
| `save_request` | Saves a request without sending it, described like `send_request` plus a `name`. `{{placeholders}}` (even in `Authorization: Bearer {{token}}`) are kept. An existing name is refused unless `overwrite` is true, so a mistake can be fixed. |
| `get_saved_request` | One saved request in full: method, URL, headers, body and the variables it needs. |
| `delete_saved_request` | Removes a saved request (history stays). |
| `list_saved_requests` | The user's saved requests: name, method, URL, headers, and the `{{variables}}` each one needs (an environment variable shows as `$env:NAME`). |
| `get_history` | Recent requests, newest first, with who sent each (`gui`, `cli` or `mcp`). Optional `search` narrows it to a URL, method, name or status (for example `orders` or `500`). Credentials are blanked. |
| `list_variables` | Variable names and where each came from (`window` or `agent`); values only for non-secret variables. Also says whether a saved Bearer token is available. |
| `set_variable` | Keeps a value for later requests as `{{name}}`, for example a token read from a login response. It persists, is shared with the window and the command line, and appears in the window under "Set by agents". A secret, or a name like `token` or `api_key`, is kept in the system credential store and masked in results. |
| `delete_variable` | Removes a variable an agent set. The user's own variables cannot be changed or removed by an agent. |
| `export_curl` | A saved request or history entry as a curl command, with `{{variables}}` left as placeholders. |
| `save_workflow` | Saves a workflow: ordered steps, each described like `send_request`, plus `extract` and `expect_status`. See [Workflows](#workflows). |
| `run_workflow` | Runs a workflow by name, optionally with `variables` for every step. Returns each step's status and the variables it set (secrets by name only), and the last response in full. |
| `list_workflows` / `delete_workflow` | List the saved workflows with their steps, or remove one. |

The read-only tools are marked as such, so a client can run them without asking.

## Workflows

A workflow is a saved list of requests where a later one can use a value from an earlier response, for example log in, then call the API with the token. A step is a request (a saved one by name, or `method` / `url` / `headers` / `json` / `body` / `form`) plus:

- `extract`: `[{"name": "token", "from": "json:$.access_token"}]` keeps a value as `{{token}}` for the steps after it. `from` is `json:$.a.b[0]`, `header:Name` or `status`. A name like `token` or `password` (or `"secret": true`) goes to the system credential store and is never shown, but it is used in the next request. The value is read from the response as the server sent it, before any secret is masked.
- `expect_status`: the status the step must return. Without it any 2xx passes. A failing step stops the run.

```json
[
  {"label": "log in", "method": "POST", "url": "{{base}}/login",
   "json": {"username": "{{username}}", "password": "{{password}}"},
   "extract": [{"name": "token", "from": "json:$.token"}]},
  {"label": "create an order", "method": "POST", "url": "{{base}}/orders",
   "headers": {"Authorization": "Bearer {{token}}"}, "json": {"item_id": 1, "qty": 2},
   "expect_status": 201, "extract": [{"name": "order_url", "from": "header:Location"}]}
]
```

From a terminal: `plunger workflow save shop @steps.json`, `plunger workflow run shop --var base=http://localhost:8080` (exit code 4 when a step fails), `plunger workflow list|show|delete`. Variables a workflow sets stay afterwards and show in the window under "Set by agents".

## Resources and prompts

Besides tools, the MCP server offers resources a client can read without calling a tool: `plunger://guide`, `plunger://variables`, `plunger://saved-requests`, `plunger://workflows`, `plunger://history`, and one for each saved request and workflow (`plunger://saved-requests/{name}`, `plunger://workflows/{name}`). It also offers prompts that start common jobs: `test_endpoint`, `login_workflow`, `debug_failed_request` and `record_workflow`.

## Command line

```bash
plunger send "Get user" --var id=42              # a saved request, by name
plunger send --url "{{base}}/users" -H "Accept: application/json"
plunger send --url https://api.example.com/items -X POST --json '{"name": "widget"}' --fail
plunger import "curl https://api.example.com/items -d 'a=1'" --send
plunger import "curl https://api.example.com/items" --save "List items"
plunger saved
plunger history --limit 5
plunger history --search orders      # URL, method, name or status contains the text
plunger vars
plunger vars set base https://api.example.com        # keep a value for later requests: {{base}}
echo "$TOKEN" | plunger vars set token -            # a secret read from stdin, kept in the credential store
plunger vars unset token
plunger save "create order" --url "{{base}}/orders" -X POST -H "Authorization: Bearer {{token}}" --json @order.json
plunger save "create order" ... --overwrite          # fix a saved request
plunger saved show "create order"                    # headers, body and variables it needs
plunger saved delete "create order"
plunger export "Get user"
plunger send "Get user" --help                       # every command has its own --help
plunger --help
```

A typical agent session sets its variables once and then refers to them by name, so no credential is pasted into a command: `plunger vars set username=demo`, `echo demo | plunger vars set password -`, then `plunger curl -s -X POST {{base}}/login --json '{"username":"{{username}}","password":"{{password}}"}'`.

Everything prints JSON, errors included: `{"error": "...", "kind": "not_sent"}`. `export` prints the curl command as plain text.

| Exit code | Meaning |
|---|---|
| 0 | Done (any HTTP status, unless `--fail`) |
| 1 | Bad command line |
| 2 | Not sent, e.g. an undefined `{{variable}}`, or another error |
| 3 | Sent, but no response (refused, DNS, timeout) |
| 4 | `--fail` was given and the status was 400 or higher |

### `plunger curl`: curl's options, Plunger's safety

An agent that already writes curl commands can swap `curl` for `plunger curl` and change nothing else. The arguments are curl's, the output is curl's (the response body, byte for byte, on stdout), and the request still goes through Plunger: an undefined `{{variable}}` is refused, secrets are filled in and masked, and the request lands in the history the window shows.

```bash
plunger curl -sS https://api.example.com/items
plunger curl -s -X POST https://api.example.com/orders   -H "Authorization: Bearer {{token}}" -H "Content-Type: application/json" -d @order.json --var token=...
plunger curl -sL https://example.com/old -o /dev/null -w "%{http_code} %{url_effective}
"
plunger curl -sf https://api.example.com/health || echo "unhealthy"
plunger curl -s -F file=@report.csv https://api.example.com/upload
```

| Supported | Notes |
|---|---|
| `-X -H -d -u -F -G -A -b -e --json --data-urlencode --url` | `-d @file` and `-d @-` (standard input) read the body like curl does |
| `--retry N --retry-delay S --retry-max-time S --retry-all-errors` | Timeouts, refused connections and 408, 429, 500, 502, 503, 504 are tried again; a `Retry-After` header is waited for. Each try is recorded in the history. |
| `-L -k -s -S -i -I -f --fail-with-body -m/--max-time -o -w` | Like curl, redirects are followed only with `-L`. `-w` knows `%{http_code} %{size_download} %{time_total} %{time_starttransfer} %{content_type} %{num_redirects} %{url_effective} %{redirect_url}` |
| clustered flags | `-sSL`, `-XPOST`, `-ofile` |
| Plunger extras | `--var name=value`, `--use-saved-bearer`, `--plunger-json` (print Plunger's structured result instead) |

These are refused with a message rather than silently ignored: `-x/--proxy`, `--cert`, `--key`, `--cacert`, `-T`, `-K`, `--resolve`, `--interface`, `-c/--cookie-jar`.

Exit codes follow curl: 0 ok, 22 with `-f` and a status of 400 or higher, 6 DNS, 7 connection refused, 28 timeout, 60 TLS, and 2 when Plunger refused to send (for example an undefined variable), which is always reported on stderr even with `-s`.

To hand a request from the window to an agent or a script, **File > Copy as** writes it as curl (bash), curl (Windows cmd) or PowerShell, and `plunger export` prints the curl form.

A URL can also be given straight to `send`: `plunger send https://api.example.com/items`.

`plunger.exe` is a Windows GUI program. Agents run it with piped output and that works normally. Typed by hand in a terminal, the output still appears, but the prompt may come back before it does.

## What an agent can and can't do

- **OS environment variables can enter requests.** `{{$env:NAME}}` reads NAME from Plunger's process environment at send time. Treat those values like other secrets: they can be sent to any request destination even though they are not stored in Plunger's state or history.
- **Two kinds of variables.** The ones you define in the window (an agent sees them, and the window saves its state when it closes and about every 30 seconds, so one edited a moment ago may take that long to reach an agent), and the ones agents set with `set_variable` / `plunger vars set`, which are visible everywhere at once. An agent cannot change or delete yours, and yours win when the names clash.
- **Environment variables cannot be set by an agent.** `{{$env:NAME}}` reads the environment the Plunger process started with (for MCP, whatever the client launched it with, for example `claude mcp add -e NAME=value`). A name that looks like a credential (`API_TOKEN`, `STRIPE_KEY`, `DB_PASSWORD`) is masked in results.
- **The saved Bearer token is off by default.** It's only attached when a request sets `use_saved_bearer` (`--use-saved-bearer` on the command line). An agent chooses its URLs, and the token should only go where you intend.
- **`{{secret}}` can still go anywhere.** The agent never sees a secret's value, but it can put `{{token}}` into a request to any host. Only give an agent Plunger access in projects where you trust where its requests go.
- **No file uploads over MCP.** `send_request` over MCP can't upload files, so a prompt-injected agent can't send files from your disk. The command line and the window can.

## Trying it without touching your data

Set `PLUNGER_DATA_DIR` to an empty folder and Plunger (window, CLI and MCP alike) uses that folder for its history and settings instead of `%APPDATA%\Plunger`.
