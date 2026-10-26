# Examples

Scripts for `plunger run` (and, as the same steps, for `save_workflow` / `run_workflow` over MCP). They show what each part of the step format is for; the full reference is [docs/scripting.md](../docs/scripting.md).

Check any of them without sending anything:

```bash
plunger run examples/github-open-prs.json --validate
```

| File | What it shows | Needs |
|---|---|---|
| [github-open-prs.json](github-open-prs.json) | Pages that follow a `Link` header, `collect`, and a `return` that groups pull requests by author | Nothing: GitHub's API answers without a login for a public repo. Try `plunger run examples/github-open-prs.json --var repo=metamug/plunger --var per_page=2` |
| [jira-sprint-report.json](jira-sprint-report.json) | Offset paging with a `total`, and a `return` that boils a sprint down to counts, points and the unassigned issues | A Jira URL in `base` and a `jira_token` variable |
| [bulk-with-rate-limit.json](bulk-with-rate-limit.json) | `for_each` over a collected list, `retry` with back-off, `delay_ms`, `continue_on_error` | The same Jira setup |
| [slack-channels.json](slack-channels.json) | Cursor paging and an `asserts` on an API that answers 200 even for errors | A `slack_token` variable |
| [wait-for-export.json](wait-for-export.json) | `poll`: start a job, then ask until it says it is done | An API with a job to wait for (change the URLs) |
| [api-checks.json](api-checks.json) | `asserts`, an environment file and a data file, and a JUnit report for CI | `python scripts/shop-api.py` running: `plunger run examples/api-checks.json --env examples/staging.env --data examples/pages.csv --junit report.xml` |

Secrets such as `jira_token` are variables you set in the Plunger window (Tools > Variables) or with `plunger vars`; a script only names them, so the files are safe to commit.

More complete files against the fake shop API in `scripts/shop-api.py`: [../scripts/shop-smoke.json](../scripts/shop-smoke.json) (login, assert, create an order) and [../scripts/shop-report.json](../scripts/shop-report.json) (paging, retry, `for_each` and a `return`).
