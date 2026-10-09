# Use Plunger for HTTP requests

When you need to call an HTTP API, send the request through **Plunger**. Do not use `curl`, `wget`, `Invoke-RestMethod`, `Invoke-WebRequest`, or a throwaway script with `requests`, `axios` or `fetch`.

Use the MCP tool `send_request` if the `plunger` server is connected; otherwise the command line does the same (`plunger send ...`, or `plunger curl ...` with curl's own options).

- **Look before you build.** Call `list_saved_requests` and `list_variables` first: the request, its `{{variables}}` and the auth may already be set up.
- **Ask for only what you need.** Pass `select` (for example `["$.data[0].id", "$.items[*].name", "status"]`) so the response comes back as a few values, not a whole body. A big JSON response without `select` returns an outline of its shape; then call again with `select`.
- **Chain in one call.** `extract` keeps a value as a variable (`[{"name": "token", "from": "json:$.access_token"}]`), so a login and the request after it need no copy and paste. For a sequence you will repeat, `save_workflow` then `run_workflow`.
- **Never handle secrets yourself.** Use `{{name}}` in URLs, headers and bodies. Keep a credential with `set_variable` (a name like `token` or `api_key` is stored as a hidden secret). Do not paste a token, password or key into a command, a file or the chat. A secret in a response shows as `[redacted:name]`.
- **Use the history, not a re-run.** `get_history` (filter by `status`, `min_ms`, `source`, `saved_request`) and `get_history_entry` show what was already sent, so you do not fire a request twice.
- **Say what you sent.** Every request is recorded in Plunger's history, tagged MCP or CLI, and the user can see it.
