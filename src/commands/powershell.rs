//! PowerShell: `Invoke-RestMethod` / `Invoke-WebRequest` commands, written out and read back,
//! including what Chrome's "Copy as PowerShell" produces (backtick line continuations, a
//! `-Headers @{...}` table, and a `$session` that carries the user agent and cookies).

use super::{PartBody, Parts};
use crate::model::{FieldKind, FormField, ParsedRequest};

/// A single-quoted PowerShell string: nothing inside is expanded, `'` is doubled.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn method_name(method: &str) -> String {
    match method {
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS" => {
            let lower = method.to_ascii_lowercase();
            let mut chars = lower.chars();
            chars.next().map(|c| c.to_ascii_uppercase().to_string() + chars.as_str()).unwrap_or_default()
        }
        other => quote(other),
    }
}

pub fn export(p: &Parts, cmdlet: &str) -> String {
    let mut notes: Vec<&str> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    let basic = if cmdlet == "Invoke-WebRequest" { " -UseBasicParsing" } else { "" };
    lines.push(format!("{cmdlet}{basic} -Uri {}", quote(&p.url)));
    let implied = if p.has_body() { "POST" } else { "GET" };
    if p.method != implied {
        lines.push(format!("-Method {}", method_name(&p.method)));
    }

    let mut content_type = None;
    let mut user_agent = None;
    let mut table: Vec<String> = Vec::new();
    for (name, value) in &p.headers {
        if name.eq_ignore_ascii_case("content-type") {
            content_type = Some(value.clone());
        } else if name.eq_ignore_ascii_case("user-agent") {
            user_agent = Some(value.clone());
        } else {
            table.push(format!("{} = {}", quote(name), quote(value)));
        }
    }
    if !table.is_empty() {
        lines.push(format!("-Headers @{{ {} }}", table.join("; ")));
    }
    if let Some(value) = &user_agent {
        lines.push(format!("-UserAgent {}", quote(value)));
    }
    if let Some(value) = &content_type {
        if !matches!(p.body, PartBody::Multipart(_)) {
            lines.push(format!("-ContentType {}", quote(value)));
        }
    }
    match &p.body {
        PartBody::None => {}
        PartBody::Text(text) | PartBody::Form(text) => lines.push(format!("-Body {}", quote(text))),
        PartBody::Multipart(fields) => {
            let entries: Vec<String> = fields
                .iter()
                .map(|(name, is_file, value)| {
                    if *is_file {
                        format!("{} = Get-Item {}", quote(name), quote(value))
                    } else {
                        format!("{} = {}", quote(name), quote(value))
                    }
                })
                .collect();
            lines.push(format!("-Form @{{ {} }}", entries.join("; ")));
            notes.push("# -Form needs PowerShell 7 or later.");
        }
    }
    if !p.follow_redirects {
        lines.push("-MaximumRedirection 0".into());
    }
    if p.insecure_tls {
        lines.push("-SkipCertificateCheck".into());
        notes.push("# -SkipCertificateCheck needs PowerShell 7 or later.");
    }

    let mut out = lines.join(" `\n  ");
    for note in notes {
        out.push('\n');
        out.push_str(note);
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    /// `-Name`
    Param(String),
    Str(String),
    Bare(String),
    HashOpen,
    HashClose,
    Eq,
    Semi,
    Newline,
}

/// Splits PowerShell source into tokens: quoted strings with their escapes resolved, `-Param`
/// names, `@{ }` tables and bare words. A backtick before a line break joins the lines.
fn tokenize(src: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '\r' => i += 1,
            '\n' => {
                toks.push(Tok::Newline);
                i += 1;
            }
            '`' if matches!(chars.get(i + 1), Some('\n') | Some('\r')) => {
                i += 1;
                while matches!(chars.get(i), Some('\n') | Some('\r')) {
                    i += 1;
                }
            }
            '#' => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '\'' => {
                i += 1;
                let mut s = String::new();
                loop {
                    match chars.get(i) {
                        None => return Err("A quoted string is not closed.".into()),
                        Some('\'') if chars.get(i + 1) == Some(&'\'') => {
                            s.push('\'');
                            i += 2;
                        }
                        Some('\'') => {
                            i += 1;
                            break;
                        }
                        Some(&ch) => {
                            s.push(ch);
                            i += 1;
                        }
                    }
                }
                toks.push(Tok::Str(s));
            }
            '"' => {
                i += 1;
                let mut s = String::new();
                loop {
                    match chars.get(i) {
                        None => return Err("A quoted string is not closed.".into()),
                        Some('`') => {
                            let escaped = chars.get(i + 1).copied().ok_or("A quoted string is not closed.")?;
                            s.push(match escaped {
                                'n' => '\n',
                                'r' => '\r',
                                't' => '\t',
                                '0' => '\0',
                                other => other,
                            });
                            i += 2;
                        }
                        Some('"') if chars.get(i + 1) == Some(&'"') => {
                            s.push('"');
                            i += 2;
                        }
                        Some('"') => {
                            i += 1;
                            break;
                        }
                        Some(&ch) => {
                            s.push(ch);
                            i += 1;
                        }
                    }
                }
                toks.push(Tok::Str(s));
            }
            '@' if chars.get(i + 1) == Some(&'{') => {
                toks.push(Tok::HashOpen);
                i += 2;
            }
            '}' => {
                toks.push(Tok::HashClose);
                i += 1;
            }
            '=' => {
                toks.push(Tok::Eq);
                i += 1;
            }
            ';' => {
                toks.push(Tok::Semi);
                i += 1;
            }
            '-' if chars.get(i + 1).is_some_and(|n| n.is_ascii_alphabetic()) => {
                let start = i + 1;
                i = start;
                while i < chars.len() && chars[i].is_ascii_alphanumeric() {
                    i += 1;
                }
                toks.push(Tok::Param(chars[start..i].iter().collect()));
            }
            _ => {
                let start = i;
                while i < chars.len() && !matches!(chars[i], ' ' | '\t' | '\r' | '\n' | ';' | '=' | '}') {
                    i += 1;
                }
                toks.push(Tok::Bare(chars[start..i].iter().collect()));
            }
        }
    }
    Ok(toks)
}

/// Parameters that take a value (resolved by exact name or a unique prefix, as PowerShell does).
const VALUE_PARAMS: &[&str] = &[
    "uri", "method", "headers", "body", "contenttype", "useragent", "form", "websession", "maximumredirection", "timeoutsec",
    "outfile", "infile", "credential", "certificate", "certificatethumbprint", "proxy", "proxycredential", "authentication",
    "token", "transferencoding", "sslprotocol", "retrycount", "retryintervalsec", "httpversion", "responseheadersvariable",
    "statuscodevariable", "sessionvariable",
];
/// Parameters Plunger cannot reproduce.
const UNSUPPORTED: &[&str] = &["credential", "certificate", "certificatethumbprint", "proxy", "proxycredential", "infile", "authentication", "token"];

fn resolve_param(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    if let Some(exact) = VALUE_PARAMS.iter().find(|p| **p == lower) {
        return Some(exact);
    }
    let mut matches = VALUE_PARAMS.iter().filter(|p| p.starts_with(&lower));
    match (matches.next(), matches.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
}

/// One `key = value` entry of a table; the flag is true when the value is a file from `Get-Item`.
type TableEntry = (String, String, bool);

/// `@{ key = value; ... }` after the opening brace; returns the entries and where parsing stopped.
fn parse_table(toks: &[Tok], mut i: usize) -> Result<(Vec<TableEntry>, usize), String> {
    let mut entries = Vec::new();
    loop {
        match toks.get(i) {
            None => return Err("A @{ } table is not closed.".into()),
            Some(Tok::Newline) | Some(Tok::Semi) => i += 1,
            Some(Tok::HashClose) => return Ok((entries, i + 1)),
            Some(Tok::Str(key)) | Some(Tok::Bare(key)) => {
                if toks.get(i + 1) != Some(&Tok::Eq) {
                    return Err(format!("Expected `=` after `{key}` in a @{{ }} table."));
                }
                i += 2;
                match toks.get(i) {
                    Some(Tok::Bare(word)) if word.eq_ignore_ascii_case("get-item") => {
                        let Some(Tok::Str(path)) = toks.get(i + 1) else {
                            return Err("Get-Item needs a quoted path.".into());
                        };
                        entries.push((key.clone(), path.clone(), true));
                        i += 2;
                    }
                    Some(Tok::Str(value)) | Some(Tok::Bare(value)) => {
                        entries.push((key.clone(), value.clone(), false));
                        i += 1;
                    }
                    _ => return Err(format!("Expected a value for `{key}` in a @{{ }} table.")),
                }
            }
            Some(other) => return Err(format!("Unexpected {other:?} in a @{{ }} table.")),
        }
    }
}

/// The user agent and cookies Chrome sets on `$session` before the request.
fn session_preamble(src: &str) -> (Option<String>, Vec<(String, String)>) {
    let (mut agent, mut cookies) = (None, Vec::new());
    for line in src.lines() {
        if let Some(rest) = line.split_once(".UserAgent").map(|(_, r)| r) {
            if let Ok(toks) = tokenize(rest) {
                agent = toks.into_iter().find_map(|t| if let Tok::Str(s) = t { Some(s) } else { None });
            }
        }
        if let Some((_, rest)) = line.split_once("Net.Cookie(") {
            if let Ok(toks) = tokenize(rest) {
                let strings: Vec<String> = toks.into_iter().filter_map(|t| if let Tok::Str(s) = t { Some(s) } else { None }).collect();
                if let [name, value, ..] = strings.as_slice() {
                    cookies.push((name.clone(), value.clone()));
                }
            }
        }
    }
    (agent, cookies)
}

pub fn parse(src: &str) -> Result<ParsedRequest, String> {
    let (session_agent, session_cookies) = session_preamble(src);
    let toks = tokenize(src)?;
    let command = toks
        .iter()
        .position(|t| matches!(t, Tok::Bare(w) if ["invoke-webrequest", "invoke-restmethod", "iwr", "irm"].contains(&w.to_ascii_lowercase().as_str())))
        .ok_or("Could not find Invoke-WebRequest or Invoke-RestMethod in that text.")?;

    let mut url: Option<String> = None;
    let mut method: Option<String> = None;
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut body: Option<String> = None;
    let mut form_fields: Vec<FormField> = Vec::new();
    let mut unsupported: Vec<String> = Vec::new();

    let mut i = command + 1;
    while i < toks.len() {
        match &toks[i] {
            Tok::Newline => break,
            Tok::Param(name) => {
                i += 1;
                let Some(param) = resolve_param(name) else {
                    // A switch (-UseBasicParsing, -SkipCertificateCheck, ...) takes no value.
                    continue;
                };
                if UNSUPPORTED.contains(&param) {
                    unsupported.push(format!("-{name}"));
                }
                // A table value, or one word / string.
                let table = if toks.get(i) == Some(&Tok::HashOpen) {
                    let (entries, next) = parse_table(&toks, i + 1)?;
                    i = next;
                    Some(entries)
                } else {
                    None
                };
                let value = if table.is_none() {
                    match toks.get(i) {
                        Some(Tok::Str(s)) | Some(Tok::Bare(s)) => {
                            i += 1;
                            Some(s.clone())
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                match param {
                    "uri" => url = value,
                    "method" => method = value,
                    "headers" => headers.extend(table.into_iter().flatten().map(|(k, v, _)| (k, v))),
                    "contenttype" => headers.extend(value.map(|v| ("Content-Type".to_string(), v))),
                    "useragent" => headers.extend(value.map(|v| ("User-Agent".to_string(), v))),
                    "body" => {
                        body = match (value, table) {
                            (Some(text), _) => Some(text),
                            (None, Some(entries)) => Some(
                                entries.iter().map(|(k, v, _)| format!("{}={}", crate::query::encode_value(k), crate::query::encode_value(v))).collect::<Vec<_>>().join("&"),
                            ),
                            _ => None,
                        }
                    }
                    "form" => {
                        for (key, value, is_file) in table.into_iter().flatten() {
                            form_fields.push(FormField {
                                key,
                                kind: if is_file { FieldKind::File } else { FieldKind::Text },
                                value,
                                enabled: true,
                            });
                        }
                    }
                    _ => {}
                }
            }
            Tok::Str(s) | Tok::Bare(s) => {
                // The first bare value is the URL (`-Uri` is positional).
                if url.is_none() {
                    url = Some(s.clone());
                }
                i += 1;
            }
            _ => i += 1,
        }
    }

    if !unsupported.is_empty() {
        return Err(format!("Plunger cannot import {} (credentials and certificates are not carried in a command).", unsupported.join(", ")));
    }
    let url = url.ok_or("Could not find a URL (-Uri) in that command.")?;

    if let Some(agent) = session_agent {
        if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("user-agent")) {
            headers.push(("User-Agent".into(), agent));
        }
    }
    if !session_cookies.is_empty() && !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("cookie")) {
        let cookie = session_cookies.iter().map(|(n, v)| format!("{n}={v}")).collect::<Vec<_>>().join("; ");
        headers.push(("Cookie".into(), cookie));
    }

    let has_payload = body.is_some() || !form_fields.is_empty();
    let method = method.map(|m| m.to_ascii_uppercase()).unwrap_or_else(|| if has_payload { "POST".into() } else { "GET".into() });
    Ok(ParsedRequest { method, url, headers, body, form_fields })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PersistedState;

    #[test]
    fn chromes_copy_as_powershell_is_read_back() {
        let pasted = "$session = New-Object Microsoft.PowerShell.Commands.WebRequestSession\r\n\
$session.UserAgent = \"Mozilla/5.0 (Windows NT 10.0)\"\r\n\
$session.Cookies.Add((New-Object System.Net.Cookie(\"sid\", \"abc123\", \"/\", \"example.com\")))\r\n\
Invoke-WebRequest -UseBasicParsing -Uri \"https://example.com/api/items?x=1\" `\r\n\
-Method \"POST\" `\r\n\
-WebSession $session `\r\n\
-Headers @{\r\n\
\"accept\"=\"application/json\"\r\n\
\"authorization\"=\"Bearer tok\"\r\n\
} `\r\n\
-ContentType \"application/json\" `\r\n\
-Body \"{`\"name`\":`\"widget`\"}\"";
        let r = parse(pasted).unwrap();
        assert_eq!((r.method.as_str(), r.url.as_str()), ("POST", "https://example.com/api/items?x=1"));
        assert_eq!(r.body.as_deref(), Some("{\"name\":\"widget\"}"));
        let header = |name: &str| r.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str());
        assert_eq!(header("accept"), Some("application/json"));
        assert_eq!(header("authorization"), Some("Bearer tok"));
        assert_eq!(header("content-type"), Some("application/json"));
        assert_eq!(header("user-agent"), Some("Mozilla/5.0 (Windows NT 10.0)"));
        assert_eq!(header("cookie"), Some("sid=abc123"));
    }

    #[test]
    fn a_hand_written_command_with_abbreviated_parameters() {
        let r = parse("irm https://h/items -Meth Put -Head @{ 'X-A' = 'it''s'; 'X-B' = 2 } -Body 'a=1'").unwrap();
        assert_eq!((r.method.as_str(), r.url.as_str(), r.body.as_deref()), ("PUT", "https://h/items", Some("a=1")));
        assert_eq!(r.headers, vec![("X-A".to_string(), "it's".to_string()), ("X-B".to_string(), "2".to_string())]);
        // a body table becomes a url-encoded form; the method follows from having a body
        let r = parse("Invoke-RestMethod -Uri 'https://h/f' -Body @{ a = '1 2'; b = 'x&y' }").unwrap();
        assert_eq!((r.method.as_str(), r.body.as_deref()), ("POST", Some("a=1%202&b=x%26y")));
    }

    #[test]
    fn a_multipart_table_becomes_form_fields() {
        let r = parse("Invoke-RestMethod -Uri 'https://h/u' -Method Post -Form @{ title = 'my doc'; file = Get-Item 'C:/a.txt' }").unwrap();
        assert_eq!(r.form_fields.len(), 2);
        assert_eq!((r.form_fields[0].key.as_str(), r.form_fields[0].kind), ("title", FieldKind::Text));
        assert_eq!((r.form_fields[1].value.as_str(), r.form_fields[1].kind), ("C:/a.txt", FieldKind::File));
    }

    #[test]
    fn what_cannot_be_carried_is_reported() {
        let e = parse("Invoke-RestMethod -Uri 'https://h' -Credential $cred -Proxy 'http://p'").unwrap_err();
        assert!(e.contains("-Credential") && e.contains("-Proxy"), "{e}");
        assert!(parse("Get-Process").is_err());
        assert!(parse("Invoke-RestMethod -Method Get").unwrap_err().contains("URL"));
        assert!(parse("Invoke-RestMethod -Uri 'unterminated").is_err());
    }

    #[test]
    fn exports_are_readable_powershell() {
        let state = PersistedState {
            method: "POST".into(),
            url: "https://h/orders".into(),
            headers_text: "Authorization: Bearer {{token}}\nContent-Type: application/json\nUser-Agent: agent/1".into(),
            body_mode: crate::model::BodyMode::Json,
            json_body: "{\"a\":\"it's\"}".into(),
            ..Default::default()
        };
        let text = super::super::export(&state, super::super::Dialect::PowerShellRest);
        assert!(text.starts_with("Invoke-RestMethod -Uri 'https://h/orders'"), "{text}");
        assert!(text.contains("-Headers @{ 'Authorization' = 'Bearer {{token}}' }"), "{text}");
        assert!(text.contains("-UserAgent 'agent/1'") && text.contains("-ContentType 'application/json'"), "{text}");
        assert!(text.contains("-Body '{\"a\":\"it''s\"}'"), "{text}");
        assert!(!text.contains("-MaximumRedirection"), "redirects are followed by default: {text}");
        let no_follow = PersistedState { follow_redirects: false, insecure_tls: true, ..state };
        let text = super::super::export(&no_follow, super::super::Dialect::PowerShellWeb);
        assert!(text.starts_with("Invoke-WebRequest -UseBasicParsing -Uri"), "{text}");
        assert!(text.contains("-MaximumRedirection 0") && text.contains("-SkipCertificateCheck"), "{text}");
    }
}
