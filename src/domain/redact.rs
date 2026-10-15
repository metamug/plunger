//! Keeps credentials out of anything written to disk (history database and
//! the persisted form state). Values are blanked rather than dropped so the
//! header/parameter still shows up as "something was here".

const SENSITIVE_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "api-key",
    "x-csrf-token",
    "x-xsrf-token",
];
const SENSITIVE_FRAGMENTS: &[&str] = &["token", "secret", "password", "passwd", "apikey", "api-key", "api_key"];
const SENSITIVE_PARAMS: &[&str] = &["key", "sig", "signature", "auth", "pwd", "code"];

pub fn is_sensitive_header(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    SENSITIVE_HEADERS.contains(&n.as_str()) || SENSITIVE_FRAGMENTS.iter().any(|f| n.contains(f))
}

/// An environment variable whose value should be masked in results: its name looks like a
/// credential (`API_TOKEN`, `STRIPE_KEY`, `DB_PASSWORD`).
pub fn is_secret_env_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["token", "secret", "password", "passwd", "pwd", "key", "credential", "auth", "bearer"].iter().any(|f| n.contains(f))
}

pub fn is_sensitive_param(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    SENSITIVE_PARAMS.contains(&n.as_str()) || SENSITIVE_FRAGMENTS.iter().any(|f| n.contains(f))
}

/// True for a value that is only `{{variable}}` placeholders, optionally behind an auth scheme
/// word (`Bearer {{token}}`) or a `name=` key (`sid={{sid}}`). It holds no secret, so blanking it
/// would only erase the reference and break the saved request.
pub fn is_placeholder_only(value: &str) -> bool {
    let mut rest = value;
    let mut placeholders = 0;
    let mut literal = String::new();
    while let Some(start) = rest.find("{{") {
        let Some(len) = rest[start..].find("}}") else { return false };
        literal.push_str(&rest[..start]);
        literal.push(' ');
        rest = &rest[start + len + 2..];
        placeholders += 1;
    }
    literal.push_str(rest);
    placeholders > 0
        && literal.split(|c: char| c.is_whitespace() || c == ';' || c == ',').filter(|t| !t.is_empty()).all(|token| {
            token.ends_with('=') || ["bearer", "basic", "token"].contains(&token.to_ascii_lowercase().as_str())
        })
}

/// Whether a key in a request *body* (a JSON member, a form field, an XML element) names a credential.
/// Deliberately narrower than headers and query parameters: a body is full of ordinary names, and only
/// string values are ever blanked, so `"tokens_used": 5` is never touched.
pub fn is_sensitive_body_key(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    const FRAGMENTS: &[&str] = &["credential", "passphrase", "private_key", "privatekey", "access_key", "secret_key"];
    const EXACT: &[&str] = &["pwd", "auth", "authorization", "cookie", "otp", "cvv", "cvc"];
    EXACT.contains(&n.as_str()) || SENSITIVE_FRAGMENTS.iter().chain(FRAGMENTS).any(|f| n.contains(f))
}

fn blankable(value: &str) -> bool {
    !value.trim().is_empty() && !is_placeholder_only(value)
}

/// The end (exclusive) of the JSON string literal that starts at `start` (a `"`), or the end of the text.
fn json_string_end(s: &str, start: usize) -> usize {
    let bytes = s.as_bytes();
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    s.len()
}

/// JSON (or JSON with `{{placeholders}}` where a value goes, which is not valid JSON) with the string
/// values of credential-looking members blanked. Works on the text, so the layout, the key order and an
/// unquoted `{{placeholder}}` are all kept exactly as written.
pub fn redact_json_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut depth: i32 = 0;
    // while inside the value of a credential-looking member that is an object or array: the depth it opened at
    let mut inside_sensitive: Option<i32> = None;
    let bytes = text.as_bytes();
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let end = json_string_end(text, i);
                let literal = &text[i..end];
                let after = text[end..].trim_start();
                if after.starts_with(':') {
                    // a key
                    out.push_str(literal);
                    let inner = literal.trim_matches('"');
                    if is_sensitive_body_key(inner) {
                        let colon = end + (text[end..].len() - after.len());
                        let value_start = colon + 1;
                        let rest = &text[value_start..];
                        let trimmed = rest.trim_start();
                        let gap = rest.len() - trimmed.len();
                        out.push_str(&text[end..value_start + gap]);
                        i = value_start + gap;
                        match trimmed.as_bytes().first() {
                            Some(b'"') => {
                                let vend = json_string_end(text, i);
                                let inner_value = &text[i + 1..vend.saturating_sub(1).max(i + 1)];
                                if blankable(inner_value) {
                                    out.push_str("\"\"");
                                } else {
                                    out.push_str(&text[i..vend]);
                                }
                                i = vend;
                            }
                            Some(b'{') | Some(b'[') => inside_sensitive = Some(depth),
                            _ => {}
                        }
                        continue;
                    }
                    i = end;
                } else if inside_sensitive.is_some() && blankable(literal.trim_matches('"')) {
                    out.push_str("\"\"");
                    i = end;
                } else {
                    out.push_str(literal);
                    i = end;
                }
            }
            b'{' | b'[' => {
                depth += 1;
                out.push(bytes[i] as char);
                i += 1;
            }
            b'}' | b']' => {
                depth -= 1;
                if inside_sensitive == Some(depth) {
                    inside_sensitive = None;
                }
                out.push(bytes[i] as char);
                i += 1;
            }
            _ => {
                // copy one whole character (the text may hold multi-byte UTF-8)
                let ch = text[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    out
}

/// `name=value` pairs separated by `&` or line breaks, with credential-looking values blanked.
pub fn redact_form_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut segment = String::new();
    let flush = |segment: &mut String, out: &mut String| {
        match segment.split_once('=') {
            Some((key, value)) if is_sensitive_body_key(key) && blankable(value) => {
                out.push_str(key);
                out.push('=');
            }
            _ => out.push_str(segment),
        }
        segment.clear();
    };
    for c in text.chars() {
        if c == '&' || c == '\n' {
            flush(&mut segment, &mut out);
            out.push(c);
        } else {
            segment.push(c);
        }
    }
    flush(&mut segment, &mut out);
    out
}

/// XML or HTML with the text of credential-looking leaf elements blanked (`<password>x</password>`).
pub fn redact_xml_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let tag_and_after = &rest[open..];
        let Some(close) = tag_and_after.find('>') else {
            out.push_str(tag_and_after);
            return out;
        };
        let tag = &tag_and_after[..=close];
        out.push_str(tag);
        rest = &tag_and_after[close + 1..];
        let inner = &tag[1..tag.len() - 1];
        let name: String = inner.chars().take_while(|c| !c.is_whitespace() && *c != '/').collect();
        let is_open = !inner.starts_with('/') && !inner.starts_with('!') && !inner.starts_with('?') && !inner.ends_with('/');
        if is_open && is_sensitive_body_key(&name) {
            let closing = format!("</{name}");
            if let Some(end) = rest.find(&closing) {
                let content = &rest[..end];
                if !content.contains('<') && blankable(content) {
                    rest = &rest[end..];
                }
            }
        }
    }
    out.push_str(rest);
    out
}

/// A raw body of unknown kind: told apart by what it looks like, as the highlighter does.
pub fn redact_raw_body(text: &str) -> String {
    let t = text.trim_start();
    if t.starts_with('{') || t.starts_with('[') {
        redact_json_text(text)
    } else if t.starts_with('<') {
        redact_xml_text(text)
    } else if text.contains('=') {
        redact_form_text(text)
    } else {
        text.to_string()
    }
}

/// `Name: Value` lines with credential-bearing values blanked out.
pub fn redact_headers_text(text: &str) -> String {
    text.lines()
        .map(|line| match line.split_once(':') {
            Some((name, value)) if is_sensitive_header(name) && !is_placeholder_only(value) => format!("{}:", name.trim_end()),
            _ => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Blanks credential-looking query parameters and any `user:password@`
/// userinfo in a URL.
pub fn redact_url(url: &str) -> String {
    let (main, fragment) = match url.split_once('#') {
        Some((m, f)) => (m, Some(f)),
        None => (url, None),
    };
    let (base, query) = match main.split_once('?') {
        Some((b, q)) => (b, Some(q)),
        None => (main, None),
    };

    let mut out = redact_userinfo(base);
    if let Some(q) = query {
        let redacted: Vec<String> = q
            .split('&')
            .map(|pair| match pair.split_once('=') {
                Some((k, v)) if is_sensitive_param(k) && !is_placeholder_only(v) => format!("{k}="),
                _ => pair.to_string(),
            })
            .collect();
        out.push('?');
        out.push_str(&redacted.join("&"));
    }
    if let Some(f) = fragment {
        out.push('#');
        out.push_str(f);
    }
    out
}

/// A message (an error from the HTTP client, say) with every URL in it passed through [`redact_url`], so a
/// credential in a query string or in `user:password@` does not reach the disk through the error text.
pub fn redact_urls_in_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        // the scheme starts at the letters just before "://"
        let scheme_start = rest[..at].rfind(|c: char| !c.is_ascii_alphanumeric() && c != '+' && c != '-' && c != '.').map_or(0, |i| i + 1);
        out.push_str(&rest[..scheme_start]);
        let after = &rest[at + 3..];
        let end = after.find(|c: char| c.is_whitespace() || matches!(c, ')' | '"' | '\'' | '<' | '>' | ',')).unwrap_or(after.len());
        let token = &after[..end];
        // trailing punctuation belongs to the sentence, not the URL
        let trimmed = token.trim_end_matches(['.', ':', ';', '!', '?']);
        let url = format!("{}://{}", &rest[scheme_start..at], trimmed);
        out.push_str(&redact_url(&url));
        out.push_str(&token[trimmed.len()..]);
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

fn redact_userinfo(base: &str) -> String {
    let (scheme, rest) = match base.split_once("://") {
        Some((s, r)) => (Some(s), r),
        None => (None, base),
    };
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let (authority, path) = rest.split_at(authority_end);
    let authority = match authority.rsplit_once('@') {
        Some((userinfo, host)) => match userinfo.split_once(':') {
            Some((user, password)) if !is_placeholder_only(password) => format!("{user}@{host}"),
            Some(_) => authority.to_string(),
            None => authority.to_string(),
        },
        None => authority.to_string(),
    };
    match scheme {
        Some(s) => format!("{s}://{authority}{path}"),
        None => format!("{authority}{path}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_inside_a_message_are_redacted_and_the_words_around_them_are_not() {
        assert_eq!(
            redact_urls_in_text("error sending request for url (http://h/x?api_key=SECRET&page=2): client error (Connect): refused."),
            "error sending request for url (http://h/x?api_key=&page=2): client error (Connect): refused."
        );
        assert_eq!(redact_urls_in_text("failed: https://bob:hunter2@h/p?token=abc."), "failed: https://bob@h/p?token=.");
        assert_eq!(redact_urls_in_text("no urls here"), "no urls here");
        assert_eq!(redact_urls_in_text("a http://h/ok?x=1 and b https://h2/y?password=p"), "a http://h/ok?x=1 and b https://h2/y?password=");
        // a placeholder is a reference, not a secret
        assert_eq!(redact_urls_in_text("url (http://h/x?token={{t}})"), "url (http://h/x?token={{t}})");
    }

    #[test]
    fn credential_members_of_a_json_body_are_blanked_and_the_rest_is_untouched() {
        let body = r#"{"user": "ann", "password": "hunter2", "profile": {"api_key": "sk_live_1", "n": 3}, "tokens_used": 5, "note": "ok"}"#;
        assert_eq!(
            redact_json_text(body),
            r#"{"user": "ann", "password": "", "profile": {"api_key": "", "n": 3}, "tokens_used": 5, "note": "ok"}"#
        );
        // an object under a credential-looking key: every string inside goes, structure stays
        assert_eq!(
            redact_json_text(r#"{"credentials": {"user": "a", "pass": "b"}, "x": "keep"}"#),
            r#"{"credentials": {"user": "", "pass": ""}, "x": "keep"}"#
        );
        // placeholders are references, not secrets; an unquoted placeholder is not valid JSON and still works
        assert_eq!(
            redact_json_text(r#"{"password": "{{pw}}", "labels": {{labels}}, "token": "abc"}"#),
            r#"{"password": "{{pw}}", "labels": {{labels}}, "token": ""}"#
        );
        // escapes, unicode and multi-line layout survive
        let nasty = "{\n  \"note\": \"quote \\\" and \u{e9}\",\n  \"secret\": \"a\\\"b\"\n}";
        assert_eq!(redact_json_text(nasty), "{\n  \"note\": \"quote \\\" and \u{e9}\",\n  \"secret\": \"\"\n}");
        // a key that merely contains a quote-colon in a value is not a key
        assert_eq!(redact_json_text(r#"{"a": "x\": \"password\": \"y"}"#), r#"{"a": "x\": \"password\": \"y"}"#);
        assert_eq!(redact_json_text(""), "");
    }

    #[test]
    fn credential_fields_of_form_and_xml_bodies_are_blanked() {
        assert_eq!(redact_form_text("user=ann&password=hunter2&page=2"), "user=ann&password=&page=2");
        assert_eq!(redact_form_text("a=1\nclient_secret=zzz\nb={{x}}\npassword={{pw}}"), "a=1\nclient_secret=\nb={{x}}\npassword={{pw}}");
        assert_eq!(
            redact_xml_text("<login><user>ann</user><password>hunter2</password><note>x</note></login>"),
            "<login><user>ann</user><password></password><note>x</note></login>"
        );
        assert_eq!(redact_raw_body(r#"{"password":"p"}"#), r#"{"password":""}"#);
        assert_eq!(redact_raw_body("<a><token>t</token></a>"), "<a><token></token></a>");
        assert_eq!(redact_raw_body("grant_type=password&password=p"), "grant_type=password&password=");
        assert_eq!(redact_raw_body("plain words, no secrets"), "plain words, no secrets");
    }

    #[test]
    fn redacting_twice_changes_nothing_more() {
        let body = r#"{"password": "a", "nested": {"token": "b"}}"#;
        let once = redact_json_text(body);
        assert_eq!(redact_json_text(&once), once);
    }

    #[test]
    fn secret_looking_environment_names_are_recognised() {
        for n in ["API_TOKEN", "github_token", "STRIPE_KEY", "DB_PASSWORD", "AWS_SECRET_ACCESS_KEY", "OAUTH_CREDENTIALS"] {
            assert!(is_secret_env_name(n), "{n}");
        }
        for n in ["HOME", "PATH", "BASE_URL", "REGION", "USERNAME"] {
            assert!(!is_secret_env_name(n), "{n}");
        }
    }

    #[test]
    fn classifies_sensitive_headers() {
        for h in ["Authorization", "cookie", "X-Api-Key", "X-Auth-Token", "x-refresh-token", "Client-Secret"] {
            assert!(is_sensitive_header(h), "{h}");
        }
        for h in ["Accept", "Content-Type", "X-Request-Id", "User-Agent", "Cache-Control"] {
            assert!(!is_sensitive_header(h), "{h}");
        }
    }

    #[test]
    fn redacts_only_sensitive_header_values() {
        let out = redact_headers_text("Accept: */*\nAuthorization: Bearer abc\nX-Api-Key : k123\nX-Trace: 1");
        assert_eq!(out, "Accept: */*\nAuthorization:\nX-Api-Key:\nX-Trace: 1");
    }

    #[test]
    fn variable_placeholders_are_not_secrets_and_survive() {
        let out = redact_headers_text(
            "Authorization: Bearer {{token}}
Cookie: sid={{sid}}
X-Api-Key: {{key}}
Authorization: Bearer abc{{x}}
Authorization: Bearer REAL",
        );
        assert_eq!(
            out,
            "Authorization: Bearer {{token}}
Cookie: sid={{sid}}
X-Api-Key: {{key}}
Authorization:
Authorization:"
        );
        assert_eq!(redact_url("https://a.com/x?token={{t}}&api_key=LIT&page=1"), "https://a.com/x?token={{t}}&api_key=&page=1");
        assert_eq!(redact_url("https://bob:{{pw}}@a.com/x"), "https://bob:{{pw}}@a.com/x");
        assert_eq!(redact_url("https://bob:real@a.com/x"), "https://bob@a.com/x");
    }

    #[test]
    fn only_pure_placeholders_count_as_placeholders() {
        for ok in ["{{t}}", "Bearer {{t}}", "bearer  {{t}} ", "sid={{s}}", "a={{x}}; b={{y}}"] {
            assert!(is_placeholder_only(ok), "{ok:?}");
        }
        for bad in ["", "abc", "Bearer abc", "Bearer abc{{t}}", "{{t}}secret", "{{unterminated", "sid=1{{s}}"] {
            assert!(!is_placeholder_only(bad), "{bad:?}");
        }
    }

    #[test]
    fn redacts_credential_query_params_and_keeps_the_rest() {
        assert_eq!(
            redact_url("https://a.com/x?page=2&api_key=SECRET&q=hi&access_token=t#frag"),
            "https://a.com/x?page=2&api_key=&q=hi&access_token=#frag"
        );
        assert_eq!(redact_url("https://a.com/x?key=AIza"), "https://a.com/x?key=");
    }

    #[test]
    fn redacts_url_password_but_keeps_username() {
        assert_eq!(redact_url("https://bob:hunter2@a.com/p?x=1"), "https://bob@a.com/p?x=1");
        assert_eq!(redact_url("https://bob@a.com/p"), "https://bob@a.com/p");
        assert_eq!(redact_url("localhost:3000/a?token=z"), "localhost:3000/a?token=");
    }

    #[test]
    fn leaves_plain_urls_untouched() {
        for u in ["https://a.com", "https://a.com/x?a=1&b=2", "http://localhost:8080/users/1", ""] {
            assert_eq!(redact_url(u), u);
        }
    }
}
