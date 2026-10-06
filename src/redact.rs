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
