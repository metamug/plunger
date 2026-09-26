//! Command-line mode: `plunger <command>` runs headless and prints JSON, for
//! scripts and agents. With no arguments `plunger` opens the window instead.
//!
//! Exit codes: 0 done (whatever the HTTP status, unless --fail), 1 bad
//! command line, 2 not sent (e.g. an undefined {{variable}}) or another
//! error, 3 sent but no response, 4 --fail and the status was 400 or higher.

use crate::agent::{self, SendFailure, SendParams};
use crate::history::Source;
use crate::mcp;
use serde::Serialize;
use std::collections::BTreeMap;

pub const USAGE: &str = "\
Plunger: send HTTP requests from the window, the command line, or an AI agent.

USAGE
  plunger                               Open the window
  plunger send <saved name> [options]   Send a saved request
  plunger send --url <url> [options]    Send a request described on the command line
  plunger import \"<curl command>\"       Parse a curl command (prints it as a request)
      --save <name>                     ...and add it to the Saved list
      --send [options]                  ...or send it
  plunger saved                         List saved requests
  plunger history [--limit N] [--search TEXT]
                                        Recent requests, newest first (default 20);
                                        --search matches URL, method, name or status
  plunger vars                          List variables (secret values are never shown)
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
  --max-body <CHARS>          Cut longer bodies in the output (default 50000)
  --fail                      Exit with 4 when the status is 400 or higher

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
    match dispatch(args) {
        Ok(code) => code,
        Err(Exit { code, kind, message }) => {
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

fn dispatch(args: Vec<String>) -> Result<i32, Exit> {
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
                    if arg.starts_with('-') || opts.params.saved_request.is_some() {
                        return Err(usage_error(format!("Unexpected argument `{arg}`")));
                    }
                    opts.params.saved_request = Some(arg);
                }
            }
            if opts.params.saved_request.is_none() && opts.params.url.is_none() {
                return Err(usage_error("Give a saved request name, or --url"));
            }
            finish_send(agent::send_request(&opts.params, Source::Cli), opts.fail)
        }
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
            args.finish()?;
            print_json(&agent::list_saved_requests().map_err(error)?);
            Ok(EXIT_OK)
        }
        "history" => {
            let mut limit = None;
            let mut search = None;
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--search" | "-s" => search = Some(args.value(&arg)?),
                    "--limit" | "-n" => {
                        let n = args.number::<i64>(&arg)?;
                        if n < 1 {
                            return Err(usage_error(format!("{arg} needs a number of 1 or more, not `{n}`")));
                        }
                        limit = Some(n);
                    }
                    _ => return Err(usage_error(format!("Unexpected argument `{arg}`"))),
                }
            }
            print_json(&agent::get_history(limit, search.as_deref()).map_err(error)?);
            Ok(EXIT_OK)
        }
        "vars" | "variables" => {
            args.finish()?;
            print_json(&agent::list_variables());
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
                let raw = args.value(arg)?;
                p.json = Some(serde_json::from_str(&raw).map_err(|e| usage_error(format!("--json isn't valid JSON: {e}")))?);
            }
            "-d" | "--data" => p.body = Some(args.value(arg)?),
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
            "--use-saved-bearer" => p.use_saved_bearer = Some(true),
            "--timeout" => p.timeout_secs = Some(args.number(arg)?),
            "--insecure" | "-k" => p.insecure_tls = Some(true),
            "--no-follow" => p.follow_redirects = Some(false),
            "--max-body" => p.max_body_chars = Some(args.number(arg)?),
            "--fail" => self.fail = true,
            _ => return Ok(false),
        }
        Ok(true)
    }
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
