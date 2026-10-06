//! curl for the Windows command prompt: double-quoted arguments and `^` line continuations, and
//! reading what Chrome's "Copy as cURL (cmd)" produces (every special character caret-escaped).

use super::{PartBody, Parts};

/// A double-quoted argument as curl.exe's C runtime reads it: `"` becomes `\"`, and backslashes
/// in front of a quote (or the closing quote) are doubled.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    let mut backslashes = 0;
    for c in s.chars() {
        match c {
            '\\' => {
                backslashes += 1;
                out.push('\\');
            }
            '"' => {
                out.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
                out.push_str("\\\"");
            }
            _ => {
                backslashes = 0;
                out.push(c);
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes));
    out.push('"');
    out
}

/// A body on one line: cmd cannot carry a line break inside an argument. Valid JSON is
/// minified; anything else keeps its text and is reported through `notes`.
fn one_line(body: &str, notes: &mut Vec<String>) -> String {
    if !body.contains(['\n', '\r']) {
        return body.to_string();
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
        return value.to_string();
    }
    notes.push("REM The body has line breaks, which cmd cannot carry in an argument. Use the PowerShell format, or save the body to a file and pass --data-binary @file.".into());
    body.to_string()
}

pub fn export(p: &Parts) -> String {
    let mut notes = Vec::new();
    let mut args: Vec<String> = Vec::new();
    let implied = if p.has_body() { "POST" } else { "GET" };
    if p.method != implied {
        args.push(format!("-X {}", p.method));
    }
    args.push(quote(&p.url));
    for (k, v) in &p.headers {
        args.push(format!("-H {}", quote(&format!("{k}: {v}"))));
    }
    match &p.body {
        PartBody::None => {}
        PartBody::Text(text) | PartBody::Form(text) => args.push(format!("--data-raw {}", quote(&one_line(text, &mut notes)))),
        PartBody::Multipart(fields) => {
            for (name, is_file, value) in fields {
                let spec = if *is_file { format!("{name}=@{value}") } else { format!("{name}={value}") };
                args.push(format!("-F {}", quote(&spec)));
            }
        }
    }
    if p.follow_redirects {
        args.push("-L".into());
    }
    if p.insecure_tls {
        args.push("--insecure".into());
    }
    let mut out = String::new();
    for note in notes {
        out.push_str(&note);
        out.push('\n');
    }
    out.push_str("curl");
    for arg in args {
        out.push_str(" ^\n  ");
        out.push_str(&arg);
    }
    out
}

/// Chrome's cmd format caret-escapes every character cmd would treat specially, and ends a line
/// with a caret to continue it. Removing the carets (`^x` becomes `x`, `^^` becomes `^`, a caret
/// before a line break joins the lines) leaves an ordinary double-quoted curl command.
pub fn normalize(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '^' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\r') => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(' ');
            }
            Some('\n') => out.push(' '),
            Some(escaped) => out.push(escaped),
            None => {}
        }
    }
    out
}

/// A command that uses carets for escaping or for continuing lines.
pub fn looks_like_cmd(input: &str) -> bool {
    input.contains("^\"") || input.lines().any(|line| line.trim_end().ends_with('^'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_follows_the_c_runtime_rules() {
        assert_eq!(quote("plain"), "\"plain\"");
        assert_eq!(quote("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(quote("C:\\dir\\"), "\"C:\\dir\\\\\"");
        assert_eq!(quote("a\\\"b"), "\"a\\\\\\\"b\"");
    }

    #[test]
    fn chromes_copy_as_curl_cmd_is_read_back() {
        let pasted = "curl \"https://example.com/api/items?a=1^&b=2\" ^\r\n  -H \"accept: application/json\" ^\r\n  -H \"content-type: application/json\" ^\r\n  --data-raw \"^{^\\^\"name^\\^\":^\\^\"widget^\\^\"^}\"";
        let r = crate::curl_import::parse_curl(&normalize(pasted)).unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(r.url, "https://example.com/api/items?a=1&b=2");
        assert_eq!(r.headers.len(), 2);
        assert_eq!(r.body.as_deref(), Some("{\"name\":\"widget\"}"));
    }

    #[test]
    fn carets_that_escape_carets_and_line_breaks_are_resolved() {
        assert_eq!(normalize("a ^^ b"), "a ^ b");
        assert_eq!(normalize("a ^\nb"), "a  b");
        assert_eq!(normalize("a ^\r\nb"), "a  b");
        assert_eq!(normalize("100^%"), "100%");
    }

    #[test]
    fn a_body_with_line_breaks_is_minified_when_json_and_flagged_otherwise() {
        let mut notes = Vec::new();
        assert_eq!(one_line("{\n  \"a\": 1\n}", &mut notes), "{\"a\":1}");
        assert!(notes.is_empty());
        assert_eq!(one_line("line one\nline two", &mut notes), "line one\nline two");
        assert_eq!(notes.len(), 1);
    }
}
