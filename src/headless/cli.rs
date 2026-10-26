//! Command-line mode: `plunger <command>` runs headless and prints JSON, for
//! scripts and agents. With no arguments `plunger` opens the window instead.
//!
//! Exit codes: 0 done (whatever the HTTP status, unless --fail), 1 bad
//! command line, 2 not sent (e.g. an undefined {{variable}}) or another
//! error, 3 sent but no response, 4 --fail and the status was 400 or higher.

use crate::agent::{self, SendFailure, SendParams};
use crate::store::history::Source;
use crate::headless::mcp;
use crate::workflow;
use serde::Serialize;
use std::collections::BTreeMap;

pub const USAGE: &str = "\
Plunger: send HTTP requests from the window, the command line, or an AI agent.

USAGE
  plunger                               Open the window
  plunger send <saved name> [options]   Send a saved request
  plunger send --url <url> [options]    Send a request described on the command line
  plunger curl [curl options] <url>     Run a curl command through Plunger (see `plunger curl --help`)
  plunger import \"<curl command>\"       Parse a curl command (prints it as a request)
      --save <name>                     ...and add it to the Saved list
      --send [options]                  ...or send it
  plunger saved                         List saved requests
  plunger saved show <name>             One saved request in full (headers, body, variables it needs)
  plunger saved delete <name>           Remove a saved request
  plunger save <name> [send options]    Save a request without sending it (--overwrite replaces)
  plunger workflow list|show|delete|run|save ...
                                        Ordered requests where one response feeds the next
  plunger run <script.json> [--env FILE] [--data FILE] [--var k=v] [--junit FILE]
                                        Run a script file of requests, asserts and captures (for CI too)
  plunger install [agent ...] [--scope project|user] [--via exe|uvx] [--dry-run]
                                        Add Plunger to Claude Code, Cursor, Kiro, Codex, Windsurf,
                                        VS Code or Gemini CLI, with steering that says to use it
                                        instead of curl (plunger install --list shows the tools)
  plunger history [--limit N] [--search TEXT]
                                        Recent requests, newest first (default 20);
                                        --search matches URL, method, name or status
  plunger audit verify|show|head|rebuild
                                        The audit log: every change, in order, hash-chained.
                                        verify finds an edited, removed or reordered line
                                        (exit 4); show lists it; rebuild recreates the
                                        database from it
  plunger where                         The version, this program, and the data folder it uses (history,
                                        audit log), with how many requests are in it: for finding out
                                        whether the window and an agent share one history
  plunger vars                          List variables (secret values are never shown)
  plunger vars set <name> <value>       Set a variable for later requests ({{name}}); --secret keeps it
                                        in the credential store (use `-` as the value to read stdin)
  plunger vars unset <name>             Remove a variable an agent set
  plunger vars clear                    Remove every variable agents set
  plunger export <saved name>           A saved request as a curl command
  plunger export --id <history id>      A history entry as a curl command
  plunger mcp                           Run as an MCP server on stdin/stdout
  plunger --version

SEND OPTIONS
  -X, --method <METHOD>       GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS
  -H, --header \"Name: value\"  Add a header (repeatable)
  --json <JSON>               JSON body (sent as application/json)
  -d, --data <TEXT>           Raw text body
  --form <name=value>         Form field, sent as x-www-form-urlencoded (repeatable)
  --var <name=value>          Set a {{variable}} for this request (repeatable)
  --use-saved-bearer          Attach the Bearer token saved in Plunger
  --timeout <SECONDS>         Default: the window's setting (20)
  --insecure                  Skip TLS certificate checks
  --no-follow                 Don't follow redirects
  --max-body <CHARS>          Longer JSON gives an outline, longer text is cut (default 50000)
  --fail                      Exit with 4 when the status is 400 or higher

Run `plunger <command> --help` for the options of one command. Environment variables work too:
{{$env:NAME}} is read when the request is sent (a name like API_TOKEN is masked in results).

Output is JSON on stdout; errors are JSON too: {\"error\": \"...\", \"kind\": \"...\"}.
Requests are recorded in the same history the window shows. Secret values never
appear in the output.

EXIT CODES
  0 ok   1 bad command line   2 not sent / error   3 no response   4 --fail and status >= 400
";

pub const EXIT_OK: i32 = 0;
pub const EXIT_USAGE: i32 = 1;
pub const EXIT_NOT_SENT: i32 = 2;
pub const EXIT_NO_RESPONSE: i32 = 3;
pub const EXIT_HTTP_ERROR: i32 = 4;

/// Runs a command and returns the process exit code.
pub fn run(args: Vec<String>) -> i32 {
    let command = args.first().cloned().unwrap_or_default();
    match dispatch(args) {
        Ok(code) => code,
        Err(Exit { code, kind, message }) => {
            // A usage error points at the help of the command that was used, when it has some.
            let message = if kind == "usage" && command_help(&command).is_some() {
                message.replace("run `plunger --help`", &format!("run `plunger {command} --help`"))
            } else {
                message
            };
            print_json(&serde_json::json!({ "error": message, "kind": kind }));
            code
        }
    }
}

struct Exit {
    code: i32,
    kind: &'static str,
    message: String,
}

fn usage_error(message: impl Into<String>) -> Exit {
    Exit { code: EXIT_USAGE, kind: "usage", message: format!("{} (run `plunger --help`)", message.into()) }
}

fn error(message: impl Into<String>) -> Exit {
    Exit { code: EXIT_NOT_SENT, kind: "error", message: message.into() }
}

fn print_json(value: &impl Serialize) {
    match serde_json::to_string_pretty(value) {
        Ok(text) => println!("{text}"),
        Err(e) => println!("{{\"error\": \"couldn't format the output: {e}\", \"kind\": \"error\"}}"),
    }
}

/// The options of one command, shown by `plunger <command> --help`.
fn command_help(command: &str) -> Option<&'static str> {
    Some(match command {
        "send" => "\
plunger send <saved name> [options]      send a saved request
plunger send <url> | --url <url> [options]

  -X, --method M   -H \"Name: value\" (repeatable)   --json <JSON|@file|@->   -d <TEXT|@file|@->
  --form k=v (repeatable)   --var name=value (repeatable)   --use-saved-bearer
  --timeout SECONDS   --insecure   --no-follow   --max-body CHARS   --fail
  --extract name=FROM (repeatable)   keep a value from the response as {{name}} for later requests;
                    FROM is json:$.path, header:Name or status (a name like token stays secret)
  --select PATH (repeatable)         print only these values (\"$.data[0].id\", header:Location, status)
                    instead of the whole body

{{variables}} in the URL, headers and body are filled in; {{$env:NAME}} reads an environment variable.
Output is JSON: status, timing, headers, and `json` (parsed) or `body` (text).
",
        "vars" | "variables" => "\
plunger vars                         list variables (names, whether secret, who set them; secret values never shown)
plunger vars set <name> <value>      set a variable for later requests, used as {{name}}
plunger vars set <name>=<value>
    --secret   keep the value in the system credential store and mask it in results (a name like
               token, password or api_key is always secret)
    --plain    force a plain value
    a value of `-` reads standard input, so a secret does not appear in the command line
plunger vars unset <name>            remove a variable an agent set
plunger vars clear                   remove every variable agents set

Variables you define in the Plunger window cannot be changed or removed from here.
Environment variables need no setup: use {{$env:NAME}} in a request.
",
        "saved" => "\
plunger saved                       list saved requests (name, method, URL, the variables each needs)
plunger saved show <name>           one saved request in full, including its body
plunger saved delete <name>         remove a saved request (history stays)
plunger save <name> ...             save a request, see `plunger save --help`
plunger send <name> [options]       send a saved request; any option overrides that part of it
",
        "workflow" | "workflows" => "\
plunger workflow                        list workflows
plunger workflow show <name>            a workflow's steps
plunger workflow run <name> [--var name=value ...] [--cleanup]
                                        send the steps in order; stops at the first failure.
                                        --cleanup removes the variables the run stored when it is done
plunger workflow save <name> <steps> [--overwrite] [--group G]
                                        <steps> is JSON, or @file.json, or @- for standard input
plunger workflow list [--group G]       the workflows (only those in a group, or under it)
plunger workflow group <name> <group>   file a workflow under a group (an empty group takes it out)
plunger workflow delete <name>

A workflow is a JSON array of steps. A step is a request (saved_request, or method / url / headers /
json / body / form) plus:
  extract        [{\"name\": \"token\", \"from\": \"json:$.access_token\"}]  keep a value as {{name}} for the next steps
                 `from` is json:$.path, header:Name or status; a name like token or password stays secret
  expect_status  the status the step must return, or a list [200, 404] (default: any 2xx)
  skip_if        a condition ({{a}} == b, {{a}} != b, or a value that is true unless empty, false, 0, null):
                 the step is not sent and counts as passed. {{previous_status}} is the step before's status
  stop_if        the same condition: the run ends here, successfully
  extract scope  \"scope\": \"run\" keeps the value for this run's later steps only, never stored
  asserts        checks on the response, e.g. [{\"from\": \"json:$.id\", \"equals\": 7}] (see `plunger run --help`)

Example: [{\"method\":\"POST\",\"url\":\"{{base}}/login\",\"json\":{\"user\":\"{{username}}\",\"password\":\"{{password}}\"},
          \"extract\":[{\"name\":\"token\",\"from\":\"json:$.token\"}]},
         {\"url\":\"{{base}}/me\",\"headers\":{\"Authorization\":\"Bearer {{token}}\"}}]
",
        "run" => "\
plunger run <script.json> [options]     run a script file: ordered requests, each with asserts and captures
plunger run @-                          ...read from standard input

  --env FILE      variables from a file: NAME=value lines, or a JSON object
  --var name=value (repeatable)           a variable; wins over --env and the script's own
  --data FILE     run the whole script once per row: a CSV with a header row, or a JSON array of objects;
                  the columns are that run's variables
  --junit FILE    also write the results as JUnit XML (GitHub Actions, Jenkins and GitLab show it as a test report)
  --validate      only check the script (fields, paths, jq programs) and send nothing
  --quiet         no progress lines on standard error

The script is a JSON array of steps (same as `plunger workflow save`), or
{\"name\": \"smoke\", \"variables\": {\"base\": \"http://localhost:3000\"}, \"steps\": [...]}.
A step is a request (method / url / headers / json / body / form, or saved_request) plus:
  expect_status  the status to expect, or a list [200, 404] (default: any 2xx)
  asserts        [{\"from\": \"json:$.id\", \"equals\": 7}, {\"from\": \"time\", \"lt\": 500}]
                 from: json:$.path (or $.path), header:Name, status, time (ms), body
                 checks: equals, not_equals, contains, exists, gt, gte, lt, lte, type, length; `name` labels one
  extract        [{\"name\": \"token\", \"from\": \"json:$.token\"}]  keep a value as {{token}} for the next steps
  skip_if / stop_if                       conditions, see `plunger workflow --help`

Loops, paging and waiting (what you would otherwise write a script for):
  paginate       {\"items\": \"json:$.issues\", \"total\": \"json:$.total\"}  send again for the next page, gather the items
                 style: offset (default), page, cursor (with \"next\": \"json:$.next\") or link (Link: rel=next header)
                 the request uses {{page.offset}} {{page.number}} {{page.cursor?}}; page_size, max_pages (default 20)
  for_each       \"issues\" (a collected list) or [1,2,3]: send once per element as {{item}} (name it with \"as\"),
                 its fields are {{item.key}} {{item.fields.name}}; {{loop.index}} counts from 0
  collect        \"issues\": keep what the step fetched as {{issues}} for the later steps (a list for paginate / for_each)
  poll           {\"until\": \"{{state}} == DONE\", \"every_ms\": 2000, \"max_attempts\": 30}  repeat until it holds;
                 read the value with an extract that has \"scope\": \"run\"
  retry          {\"attempts\": 3, \"backoff_ms\": 500, \"on_status\": [429, 503]}  try again, honouring Retry-After
  continue_on_error / delay_ms            keep going past a failed element / wait between requests
  return         a jq program that turns the collected data into the answer, e.g.
                 {count: (.issues|length), points: ([.issues[].fields.points]|add)}
                 A step may hold only a `return`. The answer is `output` in the result. jq here cannot read
                 files, the network or the environment, and secrets are masked in what it returns.
Built-in variables: {{$uuid}} {{$timestamp}} {{$timestampMs}} {{$isoTimestamp}} {{$randomInt}} {{$randomHex}}
{{$randomString}} and {{$env:NAME}}.

Progress goes to standard error, the full result to standard output as JSON. The first failing step ends a run;
with --data the next row still runs. Exit code 0 when everything passed, 4 when a step failed.

Example: {\"steps\": [{\"method\": \"POST\", \"url\": \"{{base}}/login\", \"json\": {\"user\": \"{{user}}\"},
           \"asserts\": [{\"from\": \"json:$.token\", \"type\": \"string\"}],
           \"extract\": [{\"name\": \"token\", \"from\": \"json:$.token\"}]},
          {\"url\": \"{{base}}/me\", \"headers\": {\"Authorization\": \"Bearer {{token}}\"},
           \"asserts\": [{\"from\": \"json:$.user\", \"equals\": \"{{user}}\"}, {\"from\": \"time\", \"lt\": 800}]}]}
",
        "install" => "\
plunger install [agent ...] [options]
plunger install --list                 the tools it knows, and which look installed

agent: claude-code, cursor, kiro, codex, windsurf, vscode, gemini, or all (default: the ones found)
  --scope project|user   a project folder (default) or every project of this user
  --dir PATH             the project folder (default: the current folder)
  --via exe|uvx          start the server from this program (default) or with `uvx plunger-cli mcp`
  --no-mcp               only write the steering          --no-steering   only register the server
  --dry-run              show what would change, write nothing

Registers `plunger mcp` in each tool's own config and writes always-on steering that tells the agent to
send HTTP requests through Plunger, not curl or Invoke-RestMethod. Existing settings are kept, a copy is left
beside anything changed, and running it twice changes nothing.
",
        "save" => "\
plunger save <name> [--url] <url> [send options] [--overwrite]

Saves the request without sending it. {{placeholders}} (including in Authorization headers) are
kept as written. An existing name is refused unless --overwrite is given, which replaces it.
Options are those of `plunger send`: -X, -H, --json, -d, --form, --timeout, --insecure, --no-follow.
",
        "where" => "\
plunger where                         version, program path, data folder, history database and audit log, and the number of saved and history requests
",
        "audit" => "\
plunger audit verify                the log's chain: ok with the number of entries and the head hash, or the first bad entry (exit 4)
plunger audit show [--source gui|cli|mcp] [--action send|request_saved|variable_set|workflow_run|...] [--since 1h|30m|2d] [--limit N]
                                    the changes, newest last, as one line each (no bodies, no secret values)
plunger audit head                  the hash of the newest entry, to write down somewhere else
plunger audit rebuild               recreate the history database from the log (a log that does not verify is refused)
The log is audit.log beside the database. No agent tool can read or write it.
",
        "history" => "\
plunger history [--limit N] [--search TEXT]   recent requests, newest first (default 20)
  --status 401|4xx|5xx|ok|fail|error   --min-ms N (the slow ones)   --source gui|cli|mcp   --saved <name>
plunger history show <id>           one entry in full, with the request as sent
",
        "import" => "\
plunger import \"<curl command>\" [--save <name>] [--send [send options]]

Reads a curl command (bash, Windows cmd or PowerShell) into a request.
",
        "export" => "\
plunger export <saved name>        a saved request as a curl command
plunger export --id <history id>   a history entry as a curl command
",
        "mcp" => "\
plunger mcp   run as an MCP server on stdin/stdout (see docs/agents.md)
",
        _ => return None,
    })
}

fn dispatch(args: Vec<String>) -> Result<i32, Exit> {
    if args.len() >= 2 && matches!(args[args.len() - 1].as_str(), "-h" | "--help") && args[0] != "curl" {
        if let Some(text) = command_help(&args[0]) {
            print!("{text}");
            return Ok(EXIT_OK);
        }
    }
    let mut args = Args::new(args);
    let command = args.next().unwrap_or_default();
    match command.as_str() {
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            Ok(EXIT_OK)
        }
        "-V" | "--version" | "version" => {
            println!("plunger {}", env!("CARGO_PKG_VERSION"));
            Ok(EXIT_OK)
        }
        "mcp" => {
            args.finish()?;
            mcp::run().map_err(error)?;
            Ok(EXIT_OK)
        }
        "send" => {
            let mut opts = SendOptions::default();
            while let Some(arg) = args.next() {
                if !opts.take(&arg, &mut args)? {
                    if arg.starts_with('-') || opts.params.saved_request.is_some() || opts.params.url.is_some() {
                        return Err(usage_error(format!("Unexpected argument `{arg}`")));
                    }
                    if arg.contains("://") {
                        opts.params.url = Some(arg);
                    } else {
                        opts.params.saved_request = Some(arg);
                    }
                }
            }
            if opts.params.saved_request.is_none() && opts.params.url.is_none() {
                return Err(usage_error("Give a saved request name, or --url"));
            }
            match opts.out.clone() {
                Some(file) => finish_send(send_to_file(&opts.params, &file), opts.fail),
                None => finish_send(agent::send_request(&opts.params, Source::Cli), opts.fail),
            }
        }
        "curl" => Ok(crate::headless::curl_cli::run(args.rest())),
        "import" => {
            let curl = args.next().ok_or_else(|| usage_error("Give the curl command in quotes"))?;
            let (mut save, mut send, mut opts) = (None, false, SendOptions::default());
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--save" => save = Some(args.value("--save")?),
                    "--send" => send = true,
                    _ if send && opts.take(&arg, &mut args)? => {}
                    _ => return Err(usage_error(format!("Unexpected argument `{arg}`"))),
                }
            }
            if send {
                if save.is_some() {
                    return Err(usage_error("Use either --save or --send"));
                }
                return finish_send(agent::send_curl(&curl, &opts.params, Source::Cli), opts.fail);
            }
            print_json(&agent::import_curl(&curl, save.as_deref(), Source::Cli).map_err(error)?);
            Ok(EXIT_OK)
        }
        "saved" => {
            match args.next().as_deref() {
                None => print_json(&agent::list_saved_requests(None).map_err(error)?),
                Some("--group") => print_json(&agent::list_saved_requests(Some(&args.value("saved --group")?)).map_err(error)?),
                Some("group") => {
                    let name = args.value("saved group")?;
                    let group = args.value("saved group")?;
                    args.finish()?;
                    print_json(&agent::set_group("request", &name, &group).map_err(error)?);
                }
                Some("show") => print_json(&agent::show_saved_request(&args.value("saved show")?).map_err(error)?),
                Some("delete") => {
                    let name = args.value("saved delete")?;
                    args.finish()?;
                    let removed = agent::delete_saved_request(&name).map_err(error)?;
                    print_json(&serde_json::json!({ "deleted": name, "was": removed }));
                }
                Some(other) => return Err(usage_error(format!("Unexpected argument `{other}`"))),
            }
            Ok(EXIT_OK)
        }
        "save" => {
            let (mut name, mut overwrite, mut opts) = (None, false, SendOptions::default());
            let mut group: Option<String> = None;
            while let Some(arg) = args.next() {
                if arg == "--overwrite" {
                    overwrite = true;
                } else if arg == "--group" {
                    group = Some(args.value(&arg)?);
                } else if !opts.take(&arg, &mut args)? {
                    if arg.starts_with('-') {
                        return Err(usage_error(format!("Unexpected argument `{arg}`")));
                    }
                    if name.is_none() {
                        name = Some(arg);
                    } else if opts.params.url.is_none() && arg.contains("://") {
                        opts.params.url = Some(arg);
                    } else {
                        return Err(usage_error(format!("Unexpected argument `{arg}`")));
                    }
                }
            }
            let name = name.ok_or_else(|| usage_error("Give the request a name"))?;
            print_json(&agent::save_request(&name, overwrite, &opts.params, group.as_deref(), Source::Cli).map_err(error)?);
            Ok(EXIT_OK)
        }
        "install" => {
            use crate::headless::install::{self, Agent, Options, Scope, Via};
            let (mut names, mut scope, mut dir, mut via) = (Vec::<String>::new(), Scope::Project, None::<String>, None::<Via>);
            let (mut mcp, mut steering, mut dry_run, mut list) = (true, true, false, false);
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--scope" => {
                        scope = match args.value(&arg)?.as_str() {
                            "project" => Scope::Project,
                            "user" => Scope::User,
                            other => return Err(usage_error(format!("--scope is `project` or `user`, not `{other}`"))),
                        }
                    }
                    "--dir" => dir = Some(args.value(&arg)?),
                    "--via" => {
                        via = Some(match args.value(&arg)?.as_str() {
                            "exe" => install::default_via_exe(),
                            "uvx" => Via::Uvx,
                            other => return Err(usage_error(format!("--via is `exe` or `uvx`, not `{other}`"))),
                        })
                    }
                    "--no-mcp" => mcp = false,
                    "--no-steering" => steering = false,
                    "--dry-run" => dry_run = true,
                    "--list" => list = true,
                    other if other.starts_with('-') => return Err(usage_error(format!("Unexpected argument `{other}`"))),
                    name => names.push(name.to_string()),
                }
            }
            let home = install::home_dir().ok_or_else(|| error("Couldn't find your home folder."))?;
            let project = match &dir {
                Some(d) => std::path::PathBuf::from(d),
                None => std::env::current_dir().map_err(|e| error(format!("Couldn't read the current folder: {e}")))?,
            };
            if list {
                let tools: Vec<_> = install::ALL
                    .iter()
                    .map(|a| serde_json::json!({ "agent": a.id(), "name": a.label(), "found": a.detected(&home) }))
                    .collect();
                print_json(&serde_json::json!({ "agents": tools }));
                return Ok(EXIT_OK);
            }
            if !mcp && !steering {
                return Err(usage_error("--no-mcp and --no-steering together leave nothing to do"));
            }
            let agents: Vec<Agent> = if names.is_empty() || names.iter().any(|n| n == "all") {
                let found: Vec<Agent> = install::ALL.iter().copied().filter(|a| a.detected(&home)).collect();
                if names.is_empty() && found.is_empty() {
                    return Err(error("None of the supported tools was found on this computer. Name one (plunger install --list shows them), or use `all`."));
                }
                if names.is_empty() { found } else { install::ALL.to_vec() }
            } else {
                let mut agents = Vec::new();
                for n in &names {
                    agents.push(Agent::parse(n).ok_or_else(|| usage_error(format!("Unknown tool `{n}`. Try: claude-code, cursor, kiro, codex, windsurf, vscode, gemini")))?);
                }
                agents
            };
            let via = via.unwrap_or_else(install::default_via);
            let options = Options { scope, via: via.clone(), mcp, steering, dry_run };
            let changes = install::install(&agents, &home, &project, &options);
            let failed = changes.iter().any(|c| c.action == "skipped" && c.note.as_deref().is_some_and(|n| n.starts_with("could not write")));
            print_json(&serde_json::json!({
                "scope": if scope == Scope::Project { "project" } else { "user" },
                "project_dir": if scope == Scope::Project { Some(project.display().to_string()) } else { None },
                "server": match &via { Via::Exe(p) => p.display().to_string(), Via::Uvx => "uvx plunger-cli mcp".to_string() },
                "dry_run": dry_run,
                "changes": changes,
            }));
            Ok(if failed { EXIT_NOT_SENT } else { EXIT_OK })
        }
        "workflow" | "workflows" => {
            match args.next().as_deref() {
                None | Some("list") => {
                    let mut group: Option<String> = None;
                    while let Some(arg) = args.next() {
                        if arg == "--group" {
                            group = Some(args.value(&arg)?);
                        } else {
                            return Err(usage_error(format!("Unexpected argument `{arg}`")));
                        }
                    }
                    print_json(&workflow::list(group.as_deref()).map_err(error)?);
                }
                Some("group") => {
                    let name = args.value("workflow group")?;
                    let group = args.value("workflow group")?;
                    args.finish()?;
                    print_json(&agent::set_group("workflow", &name, &group).map_err(error)?);
                }
                Some("show") => print_json(&workflow::get(&args.value("workflow show")?).map_err(error)?),
                Some("delete") => {
                    let name = args.value("workflow delete")?;
                    args.finish()?;
                    let removed = workflow::delete(&name).map_err(error)?;
                    print_json(&serde_json::json!({ "deleted": name, "was": removed }));
                }
                Some("save") => {
                    let name = args.value("workflow save")?;
                    let mut steps = None;
                    let mut overwrite = false;
                    let mut group: Option<String> = None;
                    while let Some(arg) = args.next() {
                        if arg == "--overwrite" {
                            overwrite = true;
                        } else if arg == "--group" {
                            group = Some(args.value(&arg)?);
                        } else if steps.is_none() {
                            steps = Some(arg);
                        } else {
                            return Err(usage_error(format!("Unexpected argument `{arg}`")));
                        }
                    }
                    let text = file_or_text(steps.ok_or_else(|| usage_error("Give the steps as JSON, @file.json or @-"))?)?;
                    if let Ok(serde_json::Value::Array(listed)) = serde_json::from_str::<serde_json::Value>(&text) {
                        workflow::script::unknown_step_fields(&listed).map_err(usage_error)?;
                    }
                    let steps: Vec<workflow::Step> =
                        serde_json::from_str(&text).map_err(|e| usage_error(format!("The steps aren't valid: {e}")))?;
                    print_json(&workflow::save(&name, steps, overwrite, group.as_deref()).map_err(error)?);
                }
                Some("run") => {
                    let name = args.value("workflow run")?;
                    let mut variables = BTreeMap::new();
                    let mut cleanup = false;
                    while let Some(arg) = args.next() {
                        if arg == "--var" {
                            let (k, v) = args.pair(&arg)?;
                            variables.insert(k, v);
                        } else if arg == "--cleanup" {
                            cleanup = true;
                        } else {
                            return Err(usage_error(format!("Unexpected argument `{arg}`")));
                        }
                    }
                    let result = workflow::run(&name, &variables, Source::Cli, cleanup).map_err(error)?;
                    print_json(&result);
                    return Ok(if result.ok { EXIT_OK } else { EXIT_HTTP_ERROR });
                }
                Some(other) => return Err(usage_error(format!("Unexpected argument `{other}`"))),
            }
            Ok(EXIT_OK)
        }
        "run" => run_script(&mut args),
        "where" => {
            args.finish()?;
            let dir = crate::store::history::app_data_dir();
            let history = crate::store::history::History::open().map_err(|e| error(e.to_string()))?;
            let (saved, in_history) = history.counts().map_err(|e| error(e.to_string()))?;
            print_json(&serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "program": std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default(),
                "data_dir": dir.display().to_string(),
                "data_dir_from_env": std::env::var(crate::store::history::DATA_DIR_ENV).is_ok(),
                "database": dir.join("history.sqlite3").display().to_string(),
                "audit_log": history.log_path().map(|p| p.display().to_string()),
                "saved_requests": saved,
                "history_requests": in_history,
            }));
            Ok(EXIT_OK)
        }
        "audit" => {
            use crate::store::log;
            let history = crate::store::history::History::open().map_err(|e| error(e.to_string()))?;
            let path = history.log_path().map(std::path::Path::to_path_buf).ok_or_else(|| error("This database has no audit log"))?;
            match args.next().as_deref() {
                Some("verify") => {
                    args.finish()?;
                    match log::verify(&path) {
                        Ok((entries, head)) => {
                            print_json(&serde_json::json!({ "ok": true, "entries": entries, "head": head, "log": path.display().to_string() }));
                            Ok(EXIT_OK)
                        }
                        Err(broken) => {
                            print_json(&serde_json::json!({ "ok": false, "broken_at": broken.seq, "reason": broken.reason, "log": path.display().to_string() }));
                            Ok(EXIT_HTTP_ERROR)
                        }
                    }
                }
                Some("head") => {
                    args.finish()?;
                    match log::verify(&path) {
                        Ok((entries, head)) => {
                            print_json(&serde_json::json!({ "entries": entries, "head": head }));
                            Ok(EXIT_OK)
                        }
                        Err(broken) => Err(error(format!("The log is broken at entry {}: {}", broken.seq, broken.reason))),
                    }
                }
                Some("rebuild") => {
                    args.finish()?;
                    log::verify(&path).map_err(|b| error(format!("The log is broken at entry {} ({}), so nothing was rebuilt.", b.seq, b.reason)))?;
                    let applied = history.rebuild_from_log().map_err(|e| error(e.to_string()))?;
                    print_json(&serde_json::json!({ "rebuilt": true, "entries_applied": applied }));
                    Ok(EXIT_OK)
                }
                Some("show") | None => {
                    let (mut source, mut action, mut since, mut limit) = (None, None, None, 50usize);
                    while let Some(arg) = args.next() {
                        match arg.as_str() {
                            "--source" => source = Some(args.value(&arg)?),
                            "--action" => action = Some(args.value(&arg)?),
                            "--since" => since = Some(args.value(&arg)?),
                            "--limit" => limit = args.number(&arg)?,
                            other => return Err(usage_error(format!("Unexpected argument `{other}`"))),
                        }
                    }
                    let cutoff = match since {
                        Some(text) => Some(since_cutoff(&text).ok_or_else(|| usage_error("--since is like 30m, 2h or 7d"))?),
                        None => None,
                    };
                    let entries = log::read_from(&path, 0).map_err(|e| error(e.to_string()))?;
                    let mut rows = Vec::new();
                    for (entry, _) in entries {
                        if source.as_deref().is_some_and(|s| entry.event.source() != Some(s)) {
                            continue;
                        }
                        if action.as_deref().is_some_and(|a| entry.event.action() != a) {
                            continue;
                        }
                        if let Some(cutoff) = cutoff {
                            let when = time::OffsetDateTime::parse(&entry.time, &time::format_description::well_known::Rfc3339).ok();
                            if when.is_some_and(|w| w < cutoff) {
                                continue;
                            }
                        }
                        rows.push(serde_json::json!({
                            "seq": entry.seq,
                            "time": entry.time,
                            "action": entry.event.action(),
                            "source": entry.event.source(),
                            "what": entry.event.summary(),
                        }));
                    }
                    let skip = rows.len().saturating_sub(limit);
                    print_json(&serde_json::json!({ "entries": rows.into_iter().skip(skip).collect::<Vec<_>>() }));
                    Ok(EXIT_OK)
                }
                Some(other) => Err(usage_error(format!("Unexpected argument `{other}`"))),
            }
        }
        "history" => {
            if args.peek_is("show") {
                args.next();
                let id: i64 = args.number("history show")?;
                args.finish()?;
                print_json(&agent::show_history_entry(id).map_err(error)?);
                return Ok(EXIT_OK);
            }
            let mut query = agent::HistoryQuery::default();
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--search" | "-s" => query.search = Some(args.value(&arg)?),
                    "--status" => query.status = Some(args.value(&arg)?),
                    "--min-ms" => query.min_ms = Some(args.number(&arg)?),
                    "--source" => query.source = Some(args.value(&arg)?),
                    "--saved" => query.saved_request = Some(args.value(&arg)?),
                    "--limit" | "-n" => {
                        let n = args.number::<i64>(&arg)?;
                        if n < 1 {
                            return Err(usage_error(format!("{arg} needs a number of 1 or more, not `{n}`")));
                        }
                        query.limit = Some(n);
                    }
                    _ => return Err(usage_error(format!("Unexpected argument `{arg}`"))),
                }
            }
            print_json(&agent::query_history(&query).map_err(error)?);
            Ok(EXIT_OK)
        }
        "vars" | "variables" => {
            let Some(sub) = args.next() else {
                print_json(&agent::list_variables());
                return Ok(EXIT_OK);
            };
            match sub.as_str() {
                "set" => {
                    let (mut name, mut value, mut secret) = (None::<String>, None::<String>, None);
                    while let Some(arg) = args.next() {
                        match arg.as_str() {
                            "--secret" => secret = Some(true),
                            "--plain" => secret = Some(false),
                            _ if arg.starts_with('-') && arg != "-" => return Err(usage_error(format!("Unexpected argument `{arg}`"))),
                            _ if name.is_none() => match arg.split_once('=') {
                                Some((n, v)) if !n.is_empty() => {
                                    name = Some(n.to_string());
                                    value = Some(v.to_string());
                                }
                                _ => name = Some(arg),
                            },
                            _ if value.is_none() => value = Some(arg),
                            _ => return Err(usage_error(format!("Unexpected argument `{arg}`"))),
                        }
                    }
                    let name = name.ok_or_else(|| usage_error("Give a variable name"))?;
                    let mut value = value.ok_or_else(|| usage_error(format!("Give a value for `{name}`")))?;
                    if value == "-" {
                        value = std::io::read_to_string(std::io::stdin()).map_err(|e| error(format!("Couldn't read standard input: {e}")))?;
                        value = value.trim_end_matches(['\r', '\n']).to_string();
                    }
                    print_json(&agent::set_variable(&name, &value, secret, Source::Cli).map_err(error)?);
                }
                "unset" | "delete" | "rm" => {
                    let name = args.value("vars unset")?;
                    args.finish()?;
                    agent::delete_variable(&name).map_err(error)?;
                    print_json(&serde_json::json!({ "deleted": name }));
                }
                "clear" => {
                    args.finish()?;
                    print_json(&serde_json::json!({ "deleted": agent::clear_variables().map_err(error)? }));
                }
                other => return Err(usage_error(format!("Unexpected argument `{other}`"))),
            }
            Ok(EXIT_OK)
        }
        "export" => {
            let (mut name, mut id) = (None, None);
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--id" => id = Some(args.number::<i64>("--id")?),
                    _ if !arg.starts_with('-') && name.is_none() => name = Some(arg),
                    _ => return Err(usage_error(format!("Unexpected argument `{arg}`"))),
                }
            }
            // Plain text: a curl command is meant to be pasted, not parsed.
            println!("{}", agent::export_curl(name.as_deref(), id).map_err(error)?);
            Ok(EXIT_OK)
        }
        other => Err(usage_error(format!("Unknown command `{other}`"))),
    }
}

/// Sends the request and writes the whole body to `file`: raw bytes for a binary body, the (masked) text
/// otherwise. The result prints without the body and says where it went (`body_file`).
fn send_to_file(params: &SendParams, file: &str) -> Result<crate::engine::AgentResponse, SendFailure> {
    let mut params = params.clone();
    // the body is going to a file, so the size limit for what is printed does not apply
    params.max_body_chars = Some(20_000_000);
    let (mut shaped, raw) = agent::send_request_full(&params, Source::Cli)?;
    let bytes: Vec<u8> = match (&raw.binary, &shaped.json, &shaped.body) {
        (Some(bytes), _, _) => bytes.clone(),
        (_, Some(json), _) => serde_json::to_string_pretty(json).unwrap_or_default().into_bytes(),
        (_, _, Some(text)) => text.clone().into_bytes(),
        _ => Vec::new(),
    };
    std::fs::write(file, &bytes).map_err(|e| SendFailure::Failed(format!("Could not write {file}: {e}")))?;
    shaped.json = None;
    shaped.body = None;
    shaped.outline = None;
    shaped.hint = None;
    shaped.body_cut_from_chars = None;
    shaped.body_file = Some(file.to_string());
    Ok(shaped)
}

fn finish_send(result: Result<crate::engine::AgentResponse, SendFailure>, fail: bool) -> Result<i32, Exit> {
    match result {
        Ok(response) => {
            print_json(&response);
            Ok(if fail && response.status >= 400 { EXIT_HTTP_ERROR } else { EXIT_OK })
        }
        Err(SendFailure::NotSent(message)) => Err(Exit { code: EXIT_NOT_SENT, kind: "not_sent", message }),
        Err(SendFailure::Failed(message)) => Err(Exit { code: EXIT_NO_RESPONSE, kind: "no_response", message }),
    }
}

#[derive(Default)]
struct SendOptions {
    params: SendParams,
    fail: bool,
    /// `--out FILE`: write the whole body to this file.
    out: Option<String>,
}

impl SendOptions {
    /// Consumes one send option (and its value). False if `arg` isn't one.
    fn take(&mut self, arg: &str, args: &mut Args) -> Result<bool, Exit> {
        let p = &mut self.params;
        match arg {
            "--url" => p.url = Some(args.value(arg)?),
            "-X" | "--method" => p.method = Some(args.value(arg)?),
            "-H" | "--header" => {
                let raw = args.value(arg)?;
                let (name, value) = raw.split_once(':').ok_or_else(|| usage_error(format!("Header `{raw}` needs a colon: \"Name: value\"")))?;
                p.headers.get_or_insert_with(BTreeMap::new).insert(name.trim().to_string(), value.trim().to_string());
            }
            "--json" => {
                let raw = file_or_text(args.value(arg)?)?;
                match serde_json::from_str(&raw) {
                    Ok(value) => p.json = Some(value),
                    // not JSON as typed, but it has placeholders (`"labels": {{labels}}`): send it as a JSON body
                    // and let the variables make it JSON
                    Err(_) if raw.contains("{{") => {
                        p.body = Some(raw);
                        p.headers.get_or_insert_with(BTreeMap::new).entry("Content-Type".to_string()).or_insert_with(|| "application/json".to_string());
                    }
                    Err(e) => return Err(usage_error(format!("--json isn't valid JSON: {e}"))),
                }
            }
            "-d" | "--data" => p.body = Some(file_or_text(args.value(arg)?)?),
            "--form" => {
                let (k, v) = args.pair(arg)?;
                if p.form.get_or_insert_with(BTreeMap::new).insert(k.clone(), v).is_some() {
                    return Err(usage_error(format!("--form `{k}` was given twice; repeated form fields aren't supported")));
                }
            }
            "--var" => {
                let (k, v) = args.pair(arg)?;
                p.variables.get_or_insert_with(BTreeMap::new).insert(k, v);
            }
            "--extract" => {
                let (name, from) = args.pair(arg)?;
                p.extract.push(crate::workflow::Extract { name, from, secret: None, scope: None });
            }
            "--select" => p.select.get_or_insert_with(Vec::new).push(args.value(arg)?),
            "--use-saved-bearer" => p.use_saved_bearer = Some(true),
            "--timeout" => p.timeout_secs = Some(args.number(arg)?),
            "--insecure" | "-k" => p.insecure_tls = Some(true),
            "--no-follow" => p.follow_redirects = Some(false),
            "--max-body" => p.max_body_chars = Some(args.number(arg)?),
            "--include-headers" => p.include_headers = Some(args.value(arg)?),
            "--out" => self.out = Some(args.value(arg)?),
            "--fail" => self.fail = true,
            _ => return Ok(false),
        }
        Ok(true)
    }
}

/// A body option's value: `@path` reads that file, `@-` reads standard input, anything else is the text.
/// This is how a JSON body gets past shell quoting (PowerShell mangles quotes inside arguments).
fn file_or_text(value: String) -> Result<String, Exit> {
    let Some(source) = value.strip_prefix('@') else { return Ok(value) };
    let read = if source == "-" { std::io::read_to_string(std::io::stdin()) } else { std::fs::read_to_string(source) };
    read.map_err(|e| usage_error(format!("Couldn't read {}: {e}", if source == "-" { "standard input" } else { source })))
}

/// `plunger run`: a script file's steps, once or once per data row, with a JUnit report if asked.
fn run_script(args: &mut Args) -> Result<i32, Exit> {
    use crate::workflow::{junit, script};
    let mut file = None;
    let mut env_file = None;
    let mut data_file = None;
    let mut junit_file = None;
    let mut quiet = false;
    let mut validate_only = false;
    let mut cli_vars = BTreeMap::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--validate" => validate_only = true,
            "--env" => env_file = Some(args.value(&arg)?),
            "--data" => data_file = Some(args.value(&arg)?),
            "--junit" => junit_file = Some(args.value(&arg)?),
            "--quiet" => quiet = true,
            "--var" => {
                let (k, v) = args.pair(&arg)?;
                cli_vars.insert(k, v);
            }
            _ if file.is_none() => file = Some(arg),
            _ => return Err(usage_error(format!("Unexpected argument `{arg}`"))),
        }
    }
    let source = file.ok_or_else(|| usage_error("Give the script file: plunger run script.json"))?;
    let default_name = std::path::Path::new(&source)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|_| source != "@-")
        .unwrap_or("script")
        .to_string();
    let text = file_or_text(if source.starts_with('@') { source } else { format!("@{source}") })?;
    let script = script::parse_script(&text).map_err(usage_error)?;
    let name = script.name.clone().unwrap_or(default_name);
    if validate_only {
        workflow::validate(&script.steps).map_err(usage_error)?;
        print_json(&serde_json::json!({ "valid": true, "script": name, "steps": script.steps.len() }));
        return Ok(EXIT_OK);
    }

    // the script's own variables, then the environment file, then --var
    let mut base = script.variables.clone();
    if let Some(path) = env_file {
        let text = file_or_text(format!("@{path}"))?;
        base.extend(script::parse_env(&text).map_err(usage_error)?);
    }
    base.extend(cli_vars);
    let rows = match data_file {
        Some(path) => {
            let rows = script::parse_data(&file_or_text(format!("@{path}"))?).map_err(usage_error)?;
            if rows.is_empty() {
                return Err(usage_error("The data file has no rows"));
            }
            Some(rows)
        }
        None => None,
    };

    let iterations: Vec<Option<BTreeMap<String, String>>> = match rows {
        Some(rows) => rows.into_iter().map(Some).collect(),
        None => vec![None],
    };
    let many = iterations.len() > 1;
    let mut outcomes = Vec::new();
    for (n, row) in iterations.into_iter().enumerate() {
        let mut variables = base.clone();
        if let Some(row) = &row {
            variables.extend(row.clone());
        }
        let label = if many { format!("{name} [row {}]", n + 1) } else { name.clone() };
        if many && !quiet {
            eprintln!("{label}");
        }
        let result = workflow::run_inline(&name, &script.steps, &variables, Source::Cli).map_err(error)?;
        if !quiet {
            for step in &result.steps {
                let verdict = if step.skipped { "SKIP" } else if step.ok { "PASS" } else { "FAIL" };
                let status = step.status.map(|s| format!(" {s}")).unwrap_or_default();
                let took = step.elapsed_ms.map(|ms| format!(" {ms} ms")).unwrap_or_default();
                let sent = if step.requests > 1 { format!(" ({} requests)", step.requests) } else { String::new() };
                eprintln!("{verdict} {}. {}{status}{took}{sent}", step.step, step.label);
                if let Some(w) = &step.warning {
                    eprintln!("       warning: {w}");
                }
                for a in step.asserts.iter().filter(|a| !a.ok) {
                    eprintln!("       {}: {}", a.name, a.message.as_deref().unwrap_or("failed"));
                }
                if step.asserts.iter().all(|a| a.ok) {
                    if let Some(e) = &step.error {
                        eprintln!("       {e}");
                    }
                }
            }
        }
        outcomes.push((label, row, result));
    }

    if let Some(path) = junit_file {
        let runs: Vec<(String, &workflow::WorkflowResult)> = outcomes.iter().map(|(label, _, r)| (label.clone(), r)).collect();
        std::fs::write(&path, junit::to_junit(&runs)).map_err(|e| error(format!("Couldn't write {path}: {e}")))?;
    }
    let ok = outcomes.iter().all(|(_, _, r)| r.ok);
    if !quiet {
        let passed = outcomes.iter().filter(|(_, _, r)| r.ok).count();
        eprintln!("{} of {} run{} passed", passed, outcomes.len(), if outcomes.len() == 1 { "" } else { "s" });
    }
    if many {
        let runs: Vec<_> = outcomes.iter().map(|(label, row, result)| serde_json::json!({ "run": label, "data": row, "result": result })).collect();
        print_json(&serde_json::json!({ "script": name, "ok": ok, "runs": runs }));
    } else if let Some((_, _, result)) = outcomes.first() {
        print_json(result);
    }
    Ok(if ok { EXIT_OK } else { EXIT_HTTP_ERROR })
}

/// A cursor over the arguments with small helpers for option values.
struct Args {
    items: std::vec::IntoIter<String>,
}

impl Args {
    fn new(args: Vec<String>) -> Self {
        Self { items: args.into_iter() }
    }

    fn next(&mut self) -> Option<String> {
        self.items.next()
    }

    /// Whether the next argument is `word`, without taking it.
    fn peek_is(&self, word: &str) -> bool {
        self.items.as_slice().first().is_some_and(|a| a == word)
    }

    /// Everything not consumed yet.
    fn rest(&mut self) -> Vec<String> {
        self.items.by_ref().collect()
    }

    fn value(&mut self, option: &str) -> Result<String, Exit> {
        self.items.next().ok_or_else(|| usage_error(format!("{option} needs a value")))
    }

    fn number<T: std::str::FromStr>(&mut self, option: &str) -> Result<T, Exit> {
        let raw = self.value(option)?;
        raw.parse().map_err(|_| usage_error(format!("{option} needs a number, not `{raw}`")))
    }

    fn pair(&mut self, option: &str) -> Result<(String, String), Exit> {
        let raw = self.value(option)?;
        raw.split_once('=')
            .map(|(k, v)| (k.trim().to_string(), v.to_string()))
            .ok_or_else(|| usage_error(format!("{option} needs name=value, not `{raw}`")))
    }

    fn finish(&mut self) -> Result<(), Exit> {
        match self.items.next() {
            Some(extra) => Err(usage_error(format!("Unexpected argument `{extra}`"))),
            None => Ok(()),
        }
    }
}

/// A release build is a Windows GUI program, which starts without a console.
/// When run from a terminal (and not redirected), attach to the terminal's
/// console so output shows up. Piped or redirected output (how agents run
/// it) already works and is left alone.
#[cfg(windows)]
pub fn attach_parent_console() {
    use std::ffi::c_void;
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(which: u32) -> *mut c_void;
        fn AttachConsole(process_id: u32) -> i32;
    }
    // SAFETY: plain Win32 calls with constant arguments; no pointers are dereferenced.
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        if handle.is_null() || handle as isize == -1 {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

#[cfg(not(windows))]
pub fn attach_parent_console() {}

/// "30m", "2h" or "7d" as the time that long ago.
fn since_cutoff(text: &str) -> Option<time::OffsetDateTime> {
    let text = text.trim();
    let (number, unit) = text.split_at(text.len().checked_sub(1)?);
    let n: i64 = number.parse().ok()?;
    let seconds = match unit {
        "m" => n * 60,
        "h" => n * 3600,
        "d" => n * 86_400,
        _ => return None,
    };
    Some(time::OffsetDateTime::now_utc() - time::Duration::seconds(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn send_opts(list: &[&str]) -> Result<SendOptions, Exit> {
        let mut a = Args::new(args(list));
        let mut opts = SendOptions::default();
        while let Some(arg) = a.next() {
            assert!(opts.take(&arg, &mut a)?, "not an option: {arg}");
        }
        Ok(opts)
    }

    #[test]
    fn a_body_option_can_come_from_a_file() {
        let path = std::env::temp_dir().join(format!("plunger-body-{}.json", std::process::id()));
        std::fs::write(&path, "{\"a\": 1}").unwrap();
        let spec = format!("@{}", path.display());
        assert_eq!(file_or_text(spec.clone()).ok().unwrap(), "{\"a\": 1}");
        let opts = send_opts(&["--json", &spec]).ok().unwrap();
        assert_eq!(opts.params.json, Some(serde_json::json!({"a": 1})));
        let opts = send_opts(&["-d", &spec]).ok().unwrap();
        assert_eq!(opts.params.body.as_deref(), Some("{\"a\": 1}"));
        assert_eq!(file_or_text("plain text".into()).ok().unwrap(), "plain text");
        assert!(file_or_text("@/no/such/file.json".into()).is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn every_command_in_the_usage_has_its_own_help() {
        for command in ["send", "vars", "saved", "save", "workflow", "run", "install", "history", "import", "export", "mcp", "where"] {
            assert!(command_help(command).is_some(), "{command}");
        }
        assert!(command_help("nonsense").is_none());
    }

    #[test]
    fn send_options_fill_the_same_params_the_mcp_tool_takes() {
        let o = send_opts(&[
            "--url", "{{base}}/x", "-X", "post", "-H", "Accept: application/json", "--json", "{\"a\":1}",
            "--var", "base=http://h", "--timeout", "5", "--insecure", "--no-follow", "--fail", "--max-body", "100",
        ])
        .ok()
        .unwrap();
        let p = &o.params;
        assert_eq!(p.url.as_deref(), Some("{{base}}/x"));
        assert_eq!(p.method.as_deref(), Some("post"));
        assert_eq!(p.headers.as_ref().unwrap()["Accept"], "application/json");
        assert_eq!(p.json.as_ref().unwrap()["a"], 1);
        assert_eq!(p.variables.as_ref().unwrap()["base"], "http://h");
        assert_eq!((p.timeout_secs, p.insecure_tls, p.follow_redirects, p.max_body_chars), (Some(5), Some(true), Some(false), Some(100)));
        assert!(o.fail);
    }

    #[test]
    fn bad_command_lines_exit_with_1_and_say_why() {
        for bad in [
            vec!["frobnicate"],
            vec!["send"],
            vec!["send", "--timeout", "soon"],
            vec!["send", "--json", "{nope"],
            vec!["send", "-H", "NoColon"],
            vec!["send", "a", "b"],
            vec!["history", "--limit"],
            vec!["history", "--limit", "0"],
            vec!["history", "--limit", "-1"],
            vec!["send", "--url", "http://h", "--form", "a=1", "--form", "a=2"],
            vec!["saved", "extra"],
            vec!["import"],
            vec!["import", "curl x", "--save", "n", "--send"],
        ] {
            let Err(e) = dispatch(args(&bad)) else { panic!("{bad:?} should fail") };
            assert_eq!((e.code, e.kind), (EXIT_USAGE, "usage"), "{bad:?}: {}", e.message);
        }
    }

    #[test]
    fn since_is_minutes_hours_or_days_ago() {
        use super::since_cutoff;
        let now = time::OffsetDateTime::now_utc();
        let hour = since_cutoff("1h").unwrap();
        assert!((3599..=3601).contains(&(now - hour).whole_seconds()));
        assert!((29..=30).contains(&(now - since_cutoff("30m").unwrap()).whole_minutes()));
        assert!((47..=48).contains(&(now - since_cutoff("2d").unwrap()).whole_hours()));
        for bad in ["", "h", "1", "1w", "x1h"] {
            assert!(since_cutoff(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn help_and_version_succeed() {
        assert_eq!(dispatch(args(&["--help"])).ok(), Some(EXIT_OK));
        assert_eq!(dispatch(args(&["--version"])).ok(), Some(EXIT_OK));
    }

    #[test]
    fn send_failures_map_to_distinct_exit_codes() {
        let not_sent = finish_send(Err(SendFailure::NotSent("x".into())), false).err().unwrap();
        let no_response = finish_send(Err(SendFailure::Failed("x".into())), false).err().unwrap();
        assert_eq!((not_sent.code, not_sent.kind), (EXIT_NOT_SENT, "not_sent"));
        assert_eq!((no_response.code, no_response.kind), (EXIT_NO_RESPONSE, "no_response"));
    }
}
