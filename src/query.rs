//! Keeps the Params table and the URL's query string as two views of the same
//! data. The table shows decoded values; the URL holds them percent-encoded
//! (only as much as needed — `{{variables}}`, `/`, `:` and non-ASCII stay
//! readable, and are encoded properly when the request is sent).

use crate::model::KeyValue;

/// Splits a URL into (everything before `?`, the query, the `#fragment`).
pub fn split_url(url: &str) -> (&str, Option<&str>, Option<&str>) {
    let (main, fragment) = match url.split_once('#') {
        Some((m, f)) => (m, Some(f)),
        None => (url, None),
    };
    match main.split_once('?') {
        Some((base, query)) => (base, Some(query), fragment),
        None => (main, None, fragment),
    }
}

/// The URL's query as decoded (key, value) pairs, in order.
pub fn parse_query(url: &str) -> Vec<(String, String)> {
    let (_, query, _) = split_url(url);
    query
        .unwrap_or("")
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (decode(k), decode(v)),
            None => (decode(pair), String::new()),
        })
        .collect()
}

/// `url` with its query replaced by the enabled, named rows of `rows`.
pub fn url_with_params(url: &str, rows: &[KeyValue]) -> String {
    let (base, _, fragment) = split_url(url);
    let pairs: Vec<String> = rows
        .iter()
        .filter(|p| in_url(p))
        .map(|p| {
            let key = encode(&p.key, true);
            if p.value.is_empty() {
                key
            } else {
                format!("{key}={}", encode(&p.value, false))
            }
        })
        .collect();
    let mut out = base.to_string();
    if !pairs.is_empty() {
        out.push('?');
        out.push_str(&pairs.join("&"));
    }
    if let Some(f) = fragment {
        out.push('#');
        out.push_str(f);
    }
    out
}

/// The table after the URL was edited: rows that live in the URL are replaced
/// by what the URL now says; rows that don't (unticked, or with no name yet)
/// keep their place so nothing the user typed disappears.
pub fn params_from_url(url: &str, old: &[KeyValue]) -> Vec<KeyValue> {
    let mut parsed = parse_query(url).into_iter();
    let mut rows = Vec::new();
    for row in old {
        if in_url(row) {
            if let Some((key, value)) = parsed.next() {
                rows.push(KeyValue { key, value, enabled: true });
            }
        } else if !row.is_blank() {
            rows.push(row.clone());
        }
    }
    rows.extend(parsed.map(|(key, value)| KeyValue { key, value, enabled: true }));
    rows
}

/// Brings state saved before the table and URL were synced up to date. Back
/// then enabled params lived only in the table and were appended at send
/// time, so fold them into the URL. A state that is already in sync is left
/// exactly as it is.
pub fn reconcile(url: &mut String, params: &mut Vec<KeyValue>) {
    let in_table: Vec<(String, String)> = params
        .iter()
        .filter(|p| in_url(p))
        .map(|p| (p.key.clone(), p.value.clone()))
        .collect();
    if in_table.is_empty() || in_table == parse_query(url) {
        *params = params_from_url(url, params);
        return;
    }
    let mut rows: Vec<KeyValue> = parse_query(url)
        .into_iter()
        .map(|(key, value)| KeyValue { key, value, enabled: true })
        .collect();
    rows.extend(params.iter().filter(|p| !p.is_blank()).cloned());
    *url = url_with_params(url, &rows);
    *params = rows;
}

/// Encodes a variable's value for use inside a query string at send time,
/// so a value containing `&`, `#` or `+` can't split or corrupt the query.
pub fn encode_value(value: &str) -> String {
    encode(value, false)
}

fn in_url(p: &KeyValue) -> bool {
    p.enabled && !p.key.trim().is_empty()
}

/// Percent-encodes only what would change the query's meaning (plus spaces
/// and control characters). `{{` / `}}` are left alone so variables stay
/// recognisable in the URL field.
fn encode(s: &str, is_key: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '%' | '&' | '#' | '+' | ' ' | '"' | '<' | '>' | '`' => push_escaped(&mut out, c),
            '=' if is_key => push_escaped(&mut out, c),
            c if c.is_control() => push_escaped(&mut out, c),
            c => out.push(c),
        }
    }
    out
}

fn push_escaped(out: &mut String, c: char) {
    let mut buf = [0u8; 4];
    for b in c.encode_utf8(&mut buf).bytes() {
        out.push_str(&format!("%{b:02X}"));
    }
}

/// Form-style decoding (`+` is a space). A `%` not followed by two hex digits
/// is kept literally rather than rejected.
fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() && hex(bytes[i + 1]).is_some() && hex(bytes[i + 2]).is_some() => {
                out.push(hex(bytes[i + 1]).unwrap() * 16 + hex(bytes[i + 2]).unwrap());
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    (b as char).to_digit(16).map(|d| d as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kv(key: &str, value: &str, enabled: bool) -> KeyValue {
        KeyValue { key: key.into(), value: value.into(), enabled }
    }

    fn pairs(url: &str) -> Vec<(String, String)> {
        parse_query(url)
    }

    #[test]
    fn the_query_is_parsed_and_decoded() {
        assert_eq!(
            pairs("https://h/x?include=events.player&q=a%20b+c&flag&e=%C3%A9#top"),
            vec![
                ("include".into(), "events.player".into()),
                ("q".into(), "a b c".into()),
                ("flag".into(), String::new()),
                ("e".into(), "é".into()),
            ]
        );
        assert!(pairs("https://h/x").is_empty());
        assert!(pairs("https://h/x?").is_empty());
    }

    #[test]
    fn a_stray_percent_is_kept() {
        assert_eq!(pairs("h?d=100%&e=%zz&f=%4"), vec![
            ("d".into(), "100%".into()),
            ("e".into(), "%zz".into()),
            ("f".into(), "%4".into()),
        ]);
    }

    #[test]
    fn rows_are_written_into_the_url_keeping_path_and_fragment() {
        let rows = [kv("q", "a b&c", true), kv("off", "x", false), kv("", "nameless", true), kv("tok", "{{token}}", true), kv("flag", "", true)];
        assert_eq!(url_with_params("https://h/x?old=1#top", &rows), "https://h/x?q=a%20b%26c&tok={{token}}&flag#top");
        assert_eq!(url_with_params("https://h/x?old=1", &[]), "https://h/x");
    }

    #[test]
    fn encoding_then_parsing_gives_back_the_same_values() {
        for value in ["a b", "x&y=z", "50%", "1+1", "#hash", "é/ü:{{v}}", "tab\there", "\"q\""] {
            let url = url_with_params("h", &[kv("k=ey", value, true)]);
            assert_eq!(pairs(&url), vec![("k=ey".to_string(), value.to_string())], "{url}");
        }
    }

    #[test]
    fn editing_the_url_updates_named_rows_and_keeps_the_others_in_place() {
        let old = [kv("a", "1", true), kv("off", "x", false), kv("b", "2", true), kv("", "typing", true), kv("", "", true)];
        let rows = params_from_url("h?a=9&c=3&d=4", &old);
        assert_eq!(rows, vec![kv("a", "9", true), kv("off", "x", false), kv("c", "3", true), kv("", "typing", true), kv("d", "4", true)]);
    }

    #[test]
    fn removing_the_query_from_the_url_empties_the_named_rows() {
        let rows = params_from_url("h", &[kv("a", "1", true), kv("off", "x", false)]);
        assert_eq!(rows, vec![kv("off", "x", false)]);
    }

    #[test]
    fn old_style_state_folds_its_params_into_the_url() {
        let mut url = "https://h/x?a=1".to_string();
        let mut params = vec![kv("page", "2", true), kv("off", "x", false), kv("", "", true)];
        reconcile(&mut url, &mut params);
        assert_eq!(url, "https://h/x?a=1&page=2");
        assert_eq!(params, vec![kv("a", "1", true), kv("page", "2", true), kv("off", "x", false)]);
    }

    #[test]
    fn state_that_is_already_in_sync_is_not_duplicated() {
        let mut url = "https://h/x?a=1&page=2".to_string();
        let mut params = vec![kv("a", "1", true), kv("page", "2", true)];
        reconcile(&mut url, &mut params);
        assert_eq!(url, "https://h/x?a=1&page=2");
        assert_eq!(params, vec![kv("a", "1", true), kv("page", "2", true)]);
    }

    #[test]
    fn a_url_with_a_query_but_no_rows_fills_the_table() {
        let mut url = "https://h/x?include=events&api_token=".to_string();
        let mut params = vec![];
        reconcile(&mut url, &mut params);
        assert_eq!(url, "https://h/x?include=events&api_token=");
        assert_eq!(params, vec![kv("include", "events", true), kv("api_token", "", true)]);
    }
}
