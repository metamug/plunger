use crate::model::ParsedRequest;
use serde::Deserialize;
use std::path::Path;

/// Parses a pasted curl command (as copied from a browser's "Copy as cURL",
/// or typed by hand) into method/url/headers/body.
///
/// Recognizes `-X`/`--request`, `-H`/`--header`, `-d`/`--data`/`--data-raw`/
/// `--data-binary`, and their `--flag=value` forms, plus the bare URL
/// argument. Everything else (`-u`, `--compressed`, `-k`, cookies, etc.) is
/// silently ignored rather than erroring — most real-world pastes only use
/// the flags above.
pub fn parse_curl(input: &str) -> Result<ParsedRequest, String> {
    let tokens = shell_words::split(input.trim()).map_err(|e| format!("Could not parse that as a shell command: {e}"))?;
    if tokens.is_empty() {
        return Err("Nothing to parse.".to_string());
    }

    let mut iter = tokens.into_iter().peekable();
    if let Some(first) = iter.peek() {
        if first.eq_ignore_ascii_case("curl") || first.eq_ignore_ascii_case("curl.exe") {
            iter.next();
        }
    }

    let mut method: Option<String> = None;
    let mut url: Option<String> = None;
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut body: Option<String> = None;

    while let Some(tok) = iter.next() {
        match tok.as_str() {
            "-X" | "--request" => method = iter.next(),
            "-H" | "--header" => {
                if let Some(h) = iter.next() {
                    if let Some((k, v)) = h.split_once(':') {
                        headers.push((k.trim().to_string(), v.trim().to_string()));
                    }
                }
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" | "--data-ascii" => {
                body = iter.next();
            }
            _ if tok.starts_with("--header=") => {
                let h = &tok["--header=".len()..];
                if let Some((k, v)) = h.split_once(':') {
                    headers.push((k.trim().to_string(), v.trim().to_string()));
                }
            }
            _ if tok.starts_with("--data=")
                || tok.starts_with("--data-raw=")
                || tok.starts_with("--data-binary=") =>
            {
                if let Some(idx) = tok.find('=') {
                    body = Some(tok[idx + 1..].to_string());
                }
            }
            _ if tok.starts_with("--request=") => {
                method = Some(tok["--request=".len()..].to_string());
            }
            _ if tok.starts_with('-') => {
                // Unknown/unsupported flag. Deliberately not consuming the
                // next token for these — guessing wrong would eat the URL.
            }
            _ => {
                if url.is_none() {
                    url = Some(tok);
                }
            }
        }
    }

    let url = url.ok_or_else(|| "Could not find a URL in that curl command.".to_string())?;
    let method = method.unwrap_or_else(|| if body.is_some() { "POST".to_string() } else { "GET".to_string() });

    Ok(ParsedRequest {
        method: method.to_uppercase(),
        url,
        headers,
        body,
    })
}

#[derive(Deserialize)]
struct Har {
    log: HarLog,
}
#[derive(Deserialize)]
struct HarLog {
    entries: Vec<HarEntryWrapper>,
}
#[derive(Deserialize)]
struct HarEntryWrapper {
    request: HarRequest,
}
#[derive(Deserialize)]
struct HarRequest {
    method: String,
    url: String,
    #[serde(default)]
    headers: Vec<HarHeader>,
    #[serde(rename = "postData", default)]
    post_data: Option<HarPostData>,
}
#[derive(Deserialize)]
struct HarHeader {
    name: String,
    value: String,
}
#[derive(Deserialize)]
struct HarPostData {
    #[serde(default)]
    text: Option<String>,
}

/// Parses every request entry out of a HAR file. A HAR captured from a
/// browser's Network tab usually has many entries (every request the page
/// made), so this returns all of them for the caller to present as a pick
/// list rather than guessing which one the user wants.
pub fn parse_har(path: &Path) -> Result<Vec<ParsedRequest>, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("Could not read file: {e}"))?;
    let har: Har = serde_json::from_str(&content).map_err(|e| format!("Not a valid HAR file: {e}"))?;

    Ok(har
        .log
        .entries
        .into_iter()
        .map(|entry| {
            let req = entry.request;
            let headers = req
                .headers
                .into_iter()
                // HTTP/2 pseudo-headers (:authority, :method, :path, :scheme)
                // show up in Chrome's HAR exports but aren't valid to send
                // as regular request headers.
                .filter(|h| !h.name.starts_with(':'))
                .map(|h| (h.name, h.value))
                .collect();
            let body = req.post_data.and_then(|p| p.text);
            ParsedRequest {
                method: req.method.to_uppercase(),
                url: req.url,
                headers,
                body,
            }
        })
        .collect())
}
