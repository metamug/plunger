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

## MCP tools

| Tool | What it does |
|---|---|
| `send_request` | Sends a request and returns `status`, `ok`, `elapsed_ms`, `size_bytes`, `headers`, and the body (`json` when it parses, otherwise `body`). Send a saved request by name with `saved_request`, or describe one with `method`, `url`, `headers`, and one of `json`, `body` or `form`. `variables` adds or overrides `{{variables}}` for this request only. |
| `import_curl` | Parses a curl command into a request without sending it. With `save_as`, adds it to the Saved list. |
| `list_saved_requests` | The user's saved requests: name, method, URL, headers, and the `{{variables}}` each one needs. |
| `get_history` | Recent requests, newest first, with who sent each (`gui`, `cli` or `mcp`). Optional `search` narrows it to a URL, method, name or status (for example `orders` or `500`). Credentials are blanked. |
| `list_variables` | Variable names; values only for non-secret variables. Also says whether a saved Bearer token is available. |
| `export_curl` | A saved request or history entry as a curl command, with `{{variables}}` left as placeholders. |

The read-only tools are marked as such, so a client can run them without asking.

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
plunger export "Get user"
plunger --help
```

Everything prints JSON, errors included: `{"error": "...", "kind": "not_sent"}`. `export` prints the curl command as plain text.

| Exit code | Meaning |
|---|---|
| 0 | Done (any HTTP status, unless `--fail`) |
| 1 | Bad command line |
| 2 | Not sent, e.g. an undefined `{{variable}}`, or another error |
| 3 | Sent, but no response (refused, DNS, timeout) |
| 4 | `--fail` was given and the status was 400 or higher |

`plunger.exe` is a Windows GUI program. Agents run it with piped output and that works normally. Typed by hand in a terminal, the output still appears, but the prompt may come back before it does.

## What an agent can and can't do

- **Variables come from the window.** The agent sees the variables configured in Plunger. The window saves its state when it closes and about every 30 seconds, so a variable edited a moment ago may take that long to reach an agent.
- **The saved Bearer token is off by default.** It's only attached when a request sets `use_saved_bearer` (`--use-saved-bearer` on the command line). An agent chooses its URLs, and the token should only go where you intend.
- **`{{secret}}` can still go anywhere.** The agent never sees a secret's value, but it can put `{{token}}` into a request to any host. Only give an agent Plunger access in projects where you trust where its requests go.
- **No file uploads over MCP.** `send_request` over MCP can't upload files, so a prompt-injected agent can't send files from your disk. The command line and the window can.

## Trying it without touching your data

Set `PLUNGER_DATA_DIR` to an empty folder and Plunger (window, CLI and MCP alike) uses that folder for its history and settings instead of `%APPDATA%\Plunger`.
