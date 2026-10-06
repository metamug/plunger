//! `plunger curl`: curl's command line on top of Plunger's engine, so a command written for curl
//! runs unchanged and still gets what Plunger adds (an undefined `{{variable}}` is refused,
//! secrets are filled in and masked, the request lands in the shared history).
//!
//! Output follows curl: the body goes to stdout, `-i` adds the status line and headers, `-o FILE`
//! writes the body to a file, `-w` prints a template afterwards, `-f` fails on 400 and above.
//! Exit codes are curl's where they exist (6 DNS, 7 refused, 22 HTTP error, 28 timeout).
//! Plunger-only extras: `--var name=value`, `--use-saved-bearer`, `--plunger-json`.

use crate::curl_import::{parse_curl_args, CurlOptions};
use crate::engine::{self, AgentResponse, Scrubber, SendError, Session, DEFAULT_MAX_BODY_CHARS};
use crate::history::{History, Source};
use crate::model::ParsedRequest;
use crate::redact::is_sensitive_header;
use std::io::Write;

pub const USAGE: &str = "\
plunger curl [curl options] <url>

Takes curl's own options and sends through Plunger: undefined {{variables}} are refused,
secrets are filled in and masked, and the request is recorded in the shared history.

Supported: -X -H -d -u -F -G -A -b -e --json --data-urlencode --url, -d @file and -d @-,
-L -k -s -S -i -I -f --fail-with-body -m/--max-time -o -w, and clustered flags such as -sSL.
Like curl, redirects are followed only with -L.

--retry N, --retry-delay S, --retry-max-time S, --retry-all-errors: transient failures (timeouts,
refused connections, 408, 429, 500, 502, 503, 504) are tried again, waiting for a Retry-After header
when the server sends one.

Not supported (refused, never ignored): -x/--proxy, --cert, --key, --cacert, -T, -K, --resolve,
--interface, -c/--cookie-jar.

Plunger extras: --var name=value (repeatable), --use-saved-bearer, --plunger-json (print
Plunger's structured result instead of curl's output).

Exit codes: 0 ok, 22 -f and status >= 400, 6 DNS, 7 refused, 28 timeout, 2 not sent, 1 other.
";

const EXIT_OTHER: i32 = 1;
const EXIT_NOT_SENT: i32 = 2;
const EXIT_DNS: i32 = 6;
const EXIT_REFUSED: i32 = 7;
const EXIT_HTTP_ERROR: i32 = 22;
const EXIT_TIMEOUT: i32 = 28;
const EXIT_TLS: i32 = 60;

/// Runs `plunger curl` with the arguments after the word `curl`. Returns the exit code.
pub fn run(args: Vec<String>) -> i32 {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return 0;
    }
    match execute(args) {
        Ok(code) => code,
        Err(failure) => {
            if !failure.quiet {
                eprintln!("plunger curl: {}", failure.message);
            }
            failure.code
        }
    }
}

struct Failure {
    code: i32,
    message: String,
    /// `-s` without `-S`: curl prints no error text.
    quiet: bool,
}

fn fail(code: i32, message: impl Into<String>) -> Failure {
    Failure { code, message: message.into(), quiet: false }
}

/// The options Plunger adds to curl's, taken out before curl's own are parsed.
#[derive(Default)]
struct Extras {
    variables: Vec<(String, String)>,
    use_saved_bearer: bool,
    plunger_json: bool,
}

fn split_extras(args: Vec<String>) -> Result<(Vec<String>, Extras), Failure> {
    let (mut rest, mut extras) = (Vec::new(), Extras::default());
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--var" => {
                let spec = iter.next().ok_or_else(|| fail(EXIT_OTHER, "--var needs name=value"))?;
                let (name, value) = spec.split_once('=').ok_or_else(|| fail(EXIT_OTHER, format!("--var needs name=value, not `{spec}`")))?;
                extras.variables.push((name.trim().to_string(), value.to_string()));
            }
            "--use-saved-bearer" => extras.use_saved_bearer = true,
            "--plunger-json" => extras.plunger_json = true,
            _ => rest.push(arg),
        }
    }
    Ok((rest, extras))
}

fn read_source(spec: &str) -> Result<String, String> {
    if spec == "-" {
        std::io::read_to_string(std::io::stdin()).map_err(|e| format!("can't read standard input: {e}"))
    } else {
        std::fs::read_to_string(spec).map_err(|e| format!("can't read {spec}: {e}"))
    }
}

/// curl sends `Content-Type: application/x-www-form-urlencoded` with `-d` unless told otherwise.
fn with_curl_default_content_type(request: &mut ParsedRequest) {
    let has_type = request.headers.iter().any(|(name, _)| name.eq_ignore_ascii_case("content-type"));
    if request.body.is_some() && !has_type {
        request.headers.push(("Content-Type".into(), "application/x-www-form-urlencoded".into()));
    }
}

fn execute(args: Vec<String>) -> Result<i32, Failure> {
    let (curl_args, extras) = split_extras(args)?;
    let (mut request, opts) = parse_curl_args(&curl_args, &mut read_source).map_err(|e| fail(EXIT_OTHER, e))?;
    if !opts.unsupported.is_empty() {
        return Err(fail(
            EXIT_OTHER,
            format!("not supported by plunger curl: {} (run `plunger curl --help`)", opts.unsupported.join(", ")),
        ));
    }
    with_curl_default_content_type(&mut request);
    let quiet = opts.silent && !opts.show_error;
    let session = Session::load();
    let history = History::open().map_err(|e| fail(EXIT_NOT_SENT, format!("Couldn't open Plunger's history database: {e}")))?;
    let mut state = request.into_state().with_session_from(&session.state);
    // curl follows redirects only with -L, unlike Plunger's window.
    let timeout = opts.max_time.map(|t| t.ceil().max(1.0) as u64);
    engine::apply_overrides(&mut state, &extras.variables, timeout, Some(opts.follow), opts.insecure.then_some(true));
    let scrubber = Scrubber::new(&state, &session.bearer);
    let bearer = if extras.use_saved_bearer { session.bearer.as_str() } else { "" };

    let started = std::time::Instant::now();
    let mut attempt = 0u32;
    let outcome = loop {
        let result = engine::send(state.clone(), bearer, Some(&history), Source::Cli);
        let wait = match &result {
            Ok(sent) => retry_wait(&opts, attempt, started.elapsed(), RetryReason::Status(sent.response.status, retry_after(&sent.response.headers))),
            Err(SendError::Failed { message, .. }) => retry_wait(&opts, attempt, started.elapsed(), RetryReason::Transport(message)),
            Err(SendError::Refused(_)) => None,
        };
        let Some(wait) = wait else { break result };
        attempt += 1;
        if !quiet {
            eprintln!("plunger curl: retrying in {:.1}s (attempt {attempt} of {})", wait.as_secs_f64(), opts.retry);
        }
        std::thread::sleep(wait);
    };

    match outcome {
        Ok(sent) => {
            if extras.plunger_json {
                let response = AgentResponse::from_sent(&sent, &scrubber, DEFAULT_MAX_BODY_CHARS);
                println!("{}", serde_json::to_string_pretty(&response).unwrap_or_default());
                return Ok(if opts.fail && response.status >= 400 { EXIT_HTTP_ERROR } else { 0 });
            }
            render(&sent.response, &sent.state.url, &opts, &scrubber, quiet)
        }
        Err(SendError::Refused(message)) => {
            // Plunger's own refusals (an undefined {{variable}}) are never silenced by -s.
            let hint = if message.starts_with("Undefined variable") { " Pass a value with --var name=value." } else { "" };
            Err(fail(EXIT_NOT_SENT, format!("{}{hint}", scrubber.text(&message))))
        }
        Err(SendError::Failed { message, .. }) => {
            Err(Failure { code: exit_code_for(&message), message: scrubber.text(&message), quiet })
        }
    }
}

/// Why a try failed, for deciding whether to try again.
enum RetryReason<'a> {
    Status(u16, Option<std::time::Duration>),
    Transport(&'a str),
}

/// The seconds a `Retry-After` header asks for (the HTTP-date form is not interpreted).
fn retry_after(headers: &[(String, String)]) -> Option<std::time::Duration> {
    let value = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("retry-after"))?.1.trim();
    value.parse::<f64>().ok().filter(|s| *s >= 0.0).map(std::time::Duration::from_secs_f64)
}

/// How long to wait before the next try, or None when this failure is final: out of tries, out of
/// time, or not a transient problem. curl's schedule: the server's `Retry-After`, else `--retry-delay`,
/// else 1 s doubling each time (at most 10 minutes).
fn retry_wait(opts: &CurlOptions, attempt: u32, elapsed: std::time::Duration, reason: RetryReason) -> Option<std::time::Duration> {
    if attempt >= opts.retry {
        return None;
    }
    let (transient, server_wait) = match reason {
        RetryReason::Status(status, wait) => (matches!(status, 408 | 429 | 500 | 502 | 503 | 504) || (opts.retry_all_errors && status >= 400), wait),
        RetryReason::Transport(message) => {
            let m = message.to_ascii_lowercase();
            (m.contains("timed out") || m.contains("refused") || m.contains("reset") || opts.retry_all_errors, None)
        }
    };
    if !transient {
        return None;
    }
    let backoff = std::time::Duration::from_secs_f64(1.0 * 2f64.powi(attempt as i32)).min(std::time::Duration::from_secs(600));
    let wait = server_wait.or(opts.retry_delay.map(std::time::Duration::from_secs_f64)).unwrap_or(backoff);
    if let Some(limit) = opts.retry_max_time {
        if elapsed + wait > std::time::Duration::from_secs_f64(limit) {
            return None;
        }
    }
    Some(wait)
}

/// curl's exit code for a transport failure, from the text of Plunger's error.
fn exit_code_for(message: &str) -> i32 {
    let m = message.to_ascii_lowercase();
    if m.contains("dns error") || m.contains("no such host") || m.contains("failed to lookup") || m.contains("name or service not known") {
        EXIT_DNS
    } else if m.contains("timed out") {
        EXIT_TIMEOUT
    } else if m.contains("refused") || m.contains("connect") && !m.contains("certificate") {
        EXIT_REFUSED
    } else if m.contains("certificate") || m.contains("tls") || m.contains("ssl") {
        EXIT_TLS
    } else {
        EXIT_OTHER
    }
}

/// The status line and headers the way `curl -i` prints them; credentials in headers are masked.
fn head_text(status: u16, status_text: &str, headers: &[(String, String)], scrubber: &Scrubber) -> String {
    let mut out = format!("HTTP/1.1 {status} {status_text}\r\n");
    for (name, value) in headers {
        let shown = if is_sensitive_header(name) { "[redacted]".to_string() } else { scrubber.text(value) };
        out.push_str(&format!("{name}: {shown}\r\n"));
    }
    out.push_str("\r\n");
    out
}

fn render(
    r: &crate::model::ResponseData,
    url: &str,
    opts: &CurlOptions,
    scrubber: &Scrubber,
    quiet: bool,
) -> Result<i32, Failure> {
    let http_failed = r.status >= 400;
    let mut out: Vec<u8> = Vec::new();
    if http_failed && opts.fail && !opts.fail_with_body {
        // curl -f prints no body, only an error.
    } else {
        if opts.include_headers || opts.head_only {
            out.extend(head_text(r.status, &r.status_text, &r.headers, scrubber).into_bytes());
        }
        if !opts.head_only {
            match &r.binary {
                Some(bytes) => out.extend(bytes),
                None => out.extend(scrubber.text(r.raw_text.as_deref().unwrap_or(&r.body)).into_bytes()),
            }
        }
    }
    match opts.output.as_deref() {
        None | Some("-") => write_stdout(&out),
        Some(path) if path.eq_ignore_ascii_case("nul") || path == "/dev/null" => {}
        Some(path) => std::fs::write(path, &out).map_err(|e| fail(23, format!("can't write {path}: {e}")))?,
    }
    if let Some(template) = &opts.write_out {
        write_stdout(render_write_out(template, &WriteOut::new(r, url)).as_bytes());
    }
    if http_failed && (opts.fail || opts.fail_with_body) {
        if !quiet {
            eprintln!("plunger curl: (22) The requested URL returned error: {}", r.status);
        }
        return Ok(EXIT_HTTP_ERROR);
    }
    Ok(0)
}

/// Output goes to a pipe an agent may close early; a write error there is not a failure.
fn write_stdout(bytes: &[u8]) {
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(bytes);
    let _ = stdout.flush();
}

/// The values `-w` can print, the common subset of curl's.
struct WriteOut {
    http_code: u16,
    size_download: usize,
    time_total: f64,
    time_starttransfer: f64,
    content_type: String,
    num_redirects: usize,
    url_effective: String,
    redirect_url: String,
}

impl WriteOut {
    fn new(r: &crate::model::ResponseData, sent_url: &str) -> Self {
        let header = |name: &str| r.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone()).unwrap_or_default();
        Self {
            http_code: r.status,
            size_download: r.size_bytes,
            time_total: r.elapsed_ms as f64 / 1000.0,
            time_starttransfer: r.ttfb_ms as f64 / 1000.0,
            content_type: header("content-type"),
            num_redirects: r.redirect_chain.len(),
            url_effective: r.redirect_chain.last().map(|(_, to)| to.clone()).unwrap_or_else(|| sent_url.to_string()),
            redirect_url: if (300..400).contains(&r.status) { header("location") } else { String::new() },
        }
    }
}

/// curl's `-w` template: `%{name}` variables, `%%`, and the escapes `\n`, `\r`, `\t`, `\\`.
fn render_write_out(template: &str, v: &WriteOut) -> String {
    let mut out = String::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            },
            '%' if chars.peek() == Some(&'%') => {
                chars.next();
                out.push('%');
            }
            '%' if chars.peek() == Some(&'{') => {
                chars.next();
                let name: String = chars.by_ref().take_while(|c| *c != '}').collect();
                match name.as_str() {
                    "http_code" | "response_code" => out.push_str(&format!("{:03}", v.http_code)),
                    "size_download" => out.push_str(&v.size_download.to_string()),
                    "time_total" => out.push_str(&format!("{:.6}", v.time_total)),
                    "time_starttransfer" => out.push_str(&format!("{:.6}", v.time_starttransfer)),
                    "content_type" => out.push_str(&v.content_type),
                    "num_redirects" => out.push_str(&v.num_redirects.to_string()),
                    "url_effective" => out.push_str(&v.url_effective),
                    "redirect_url" => out.push_str(&v.redirect_url),
                    _ => {}
                }
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> WriteOut {
        WriteOut {
            http_code: 201,
            size_download: 54,
            time_total: 0.0125,
            time_starttransfer: 0.004,
            content_type: "application/json".into(),
            num_redirects: 1,
            url_effective: "http://h/final".into(),
            redirect_url: String::new(),
        }
    }

    #[test]
    fn write_out_fills_variables_and_escapes() {
        assert_eq!(render_write_out("%{http_code}\\n", &vars()), "201\n");
        assert_eq!(render_write_out("%{http_code} %{size_download} %{time_total}", &vars()), "201 54 0.012500");
        assert_eq!(render_write_out("100%% %{url_effective}|%{content_type}", &vars()), "100% http://h/final|application/json");
        assert_eq!(render_write_out("[%{unknown}]", &vars()), "[]");
        assert_eq!(render_write_out("a\\tb\\\\c", &vars()), "a\tb\\c");
    }

    #[test]
    fn transport_errors_map_to_curls_exit_codes() {
        assert_eq!(exit_code_for("error sending request: dns error: No such host is known."), EXIT_DNS);
        assert_eq!(exit_code_for("error sending request: operation timed out"), EXIT_TIMEOUT);
        assert_eq!(exit_code_for("client error (Connect): tcp connect error: actively refused it"), EXIT_REFUSED);
        assert_eq!(exit_code_for("invalid peer certificate: UnknownIssuer"), EXIT_TLS);
        assert_eq!(exit_code_for("something else"), EXIT_OTHER);
    }

    #[test]
    fn a_body_without_a_content_type_gets_curls_form_default() {
        let request = |headers: Vec<(String, String)>, body: Option<&str>| ParsedRequest {
            method: "POST".into(),
            url: "http://h".into(),
            headers,
            body: body.map(String::from),
            form_fields: vec![],
        };
        let mut r = request(vec![], Some("a=1"));
        with_curl_default_content_type(&mut r);
        assert_eq!(r.headers, vec![("Content-Type".to_string(), "application/x-www-form-urlencoded".to_string())]);

        let mut typed = request(vec![("content-type".into(), "application/json".into())], Some("{}"));
        with_curl_default_content_type(&mut typed);
        assert_eq!(typed.headers.len(), 1);

        let mut none = request(vec![], None);
        with_curl_default_content_type(&mut none);
        assert!(none.headers.is_empty());
    }

    #[test]
    fn retrying_follows_curls_rules_and_honours_retry_after() {
        use std::time::Duration;
        let opts = |retry| CurlOptions { retry, ..Default::default() };
        let secs = Duration::from_secs;
        // only transient statuses, only while tries remain
        assert_eq!(retry_wait(&opts(3), 0, secs(0), RetryReason::Status(503, None)), Some(secs(1)));
        assert_eq!(retry_wait(&opts(3), 2, secs(0), RetryReason::Status(503, None)), Some(secs(4)), "the wait doubles");
        assert_eq!(retry_wait(&opts(3), 3, secs(0), RetryReason::Status(503, None)), None, "out of tries");
        assert_eq!(retry_wait(&opts(3), 0, secs(0), RetryReason::Status(404, None)), None, "404 is not transient");
        assert_eq!(retry_wait(&opts(0), 0, secs(0), RetryReason::Status(503, None)), None, "no --retry, no retries");
        // the server's Retry-After wins over the schedule, then --retry-delay
        assert_eq!(retry_wait(&opts(3), 0, secs(0), RetryReason::Status(429, Some(secs(7)))), Some(secs(7)));
        let delayed = CurlOptions { retry: 3, retry_delay: Some(2.0), ..Default::default() };
        assert_eq!(retry_wait(&delayed, 1, secs(0), RetryReason::Status(500, None)), Some(secs(2)));
        // --retry-all-errors and the total time limit
        let all = CurlOptions { retry: 3, retry_all_errors: true, ..Default::default() };
        assert_eq!(retry_wait(&all, 0, secs(0), RetryReason::Status(404, None)), Some(secs(1)));
        let limited = CurlOptions { retry: 5, retry_max_time: Some(5.0), ..Default::default() };
        assert_eq!(retry_wait(&limited, 0, secs(5), RetryReason::Status(503, None)), None, "the next wait would pass the limit");
        // connection trouble
        assert!(retry_wait(&opts(2), 0, secs(0), RetryReason::Transport("operation timed out")).is_some());
        assert!(retry_wait(&opts(2), 0, secs(0), RetryReason::Transport("tcp connect error: connection refused")).is_some());
        assert!(retry_wait(&opts(2), 0, secs(0), RetryReason::Transport("invalid peer certificate")).is_none());
    }

    #[test]
    fn a_retry_after_header_is_read_in_seconds() {
        let headers = vec![("Retry-After".to_string(), " 3 ".to_string())];
        assert_eq!(retry_after(&headers), Some(std::time::Duration::from_secs(3)));
        assert_eq!(retry_after(&[("retry-after".into(), "Wed, 21 Oct 2026 07:28:00 GMT".into())]), None);
        assert_eq!(retry_after(&[]), None);
    }

    #[test]
    fn plunger_extras_are_taken_out_before_curl_parses() {
        let args: Vec<String> = ["-s", "--var", "a=1", "http://h/x", "--plunger-json", "--use-saved-bearer", "--var", "b=2=3"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (rest, extras) = split_extras(args).ok().unwrap();
        assert_eq!(rest, vec!["-s", "http://h/x"]);
        assert_eq!(extras.variables, vec![("a".to_string(), "1".to_string()), ("b".to_string(), "2=3".to_string())]);
        assert!(extras.plunger_json && extras.use_saved_bearer);
        assert!(split_extras(vec!["--var".into()]).is_err());
    }
}
