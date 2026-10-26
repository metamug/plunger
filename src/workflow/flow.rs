//! The control flow a step can have beyond one request: paging through a list, repeating until a job is done,
//! trying again after a 429, and going over the items of a list. The types are what a workflow file says; the
//! helpers are the small pure parts of running them.

use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const DEFAULT_MAX_PAGES: usize = 20;
pub const MAX_PAGES: usize = 500;
/// Most items one step may collect across its pages.
pub const MAX_ITEMS: usize = 100_000;
/// Most elements a `for_each` may go over.
pub const MAX_ITERATIONS: usize = 1000;
/// Most text one collected value may hold.
pub const MAX_COLLECTED_BYTES: usize = 20_000_000;
/// Longest a step may wait between two requests.
/// Most characters a `return` may produce.
pub const MAX_OUTPUT_CHARS: usize = 500_000;
/// Run values bigger than this are not handed to requests as variables (they stay available to `for_each` and `return`).
pub const MAX_REQUEST_VARIABLE_BYTES: usize = 100_000;
pub const MAX_WAIT_MS: u64 = 60_000;
pub const DEFAULT_POLL_ATTEMPTS: usize = 30;
pub const MAX_POLL_ATTEMPTS: usize = 1000;
pub const MAX_RETRY_ATTEMPTS: usize = 10;

/// Send the step's request again for the next page, until there are no more. Each page must return the
/// expected status. The items of every page are gathered into one list (see `collect`); `asserts` and
/// `extract` see the last page.
///
/// What the request can use: `{{page.number}}` (1, 2, ...), `{{page.offset}}` (items fetched so far) and
/// `{{page.cursor}}` (the cursor from the page before; write `{{page.cursor?}}` so the first page leaves it out).
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Paginate {
    /// How the API pages: `offset` (default: `startAt=0`, `startAt=100`, ...), `page` (`page=1`, `page=2`, ...),
    /// `cursor` (each response says where the next page starts; needs `next`) or `link` (each response has a
    /// `Link: <url>; rel="next"` header, as GitHub does).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// Where the list is in each response: `json:$.issues`. Without it the whole body must be an array.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<String>,
    /// For `offset`: where the total count is (`json:$.total`), so paging stops exactly at the end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<String>,
    /// For `cursor`: where the next cursor is (`json:$.next_cursor`). A missing, null or empty value ends the paging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    /// The page size the request asks for. A page with fewer items than this is the last one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_size: Option<usize>,
    /// Stop after this many pages (default 20, at most 500). The step says so when it stopped here with more to fetch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_pages: Option<usize>,
}

impl Paginate {
    pub fn style(&self) -> &str {
        self.style.as_deref().unwrap_or("offset")
    }

    pub fn max_pages(&self) -> usize {
        self.max_pages.unwrap_or(DEFAULT_MAX_PAGES).clamp(1, MAX_PAGES)
    }

    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.style(), "offset" | "page" | "cursor" | "link") {
            return Err(format!("`style` is `{}`; use offset, page, cursor or link", self.style()));
        }
        if self.style() == "cursor" && self.next.as_deref().is_none_or(|n| n.trim().is_empty()) {
            return Err("a `cursor` paginate needs `next`: where the next cursor is, e.g. json:$.next_cursor".into());
        }
        for (field, spec) in [("items", &self.items), ("total", &self.total), ("next", &self.next)] {
            if let Some(spec) = spec {
                crate::domain::extract::check_source(spec, true).map_err(|m| format!("`{field}` {m}"))?;
            }
        }
        if self.max_pages.is_some_and(|m| m == 0 || m > MAX_PAGES) {
            return Err(format!("`max_pages` must be 1 to {MAX_PAGES}"));
        }
        if self.page_size == Some(0) {
            return Err("`page_size` must be at least 1".into());
        }
        Ok(())
    }
}

/// Send the step's request again and again until a condition holds, for something that finishes later (an
/// export, a deployment, an async job). Use `extract` with `"scope": "run"` to read the status from each response,
/// then name it in `until`. The step fails if the condition never holds.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Poll {
    /// A condition on the values extracted so far, like `skip_if`: `{{state}} == DONE`.
    pub until: String,
    /// Wait this long between requests (default 1000, at most 60000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every_ms: Option<u64>,
    /// Give up after this many requests (default 30, at most 1000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_attempts: Option<usize>,
}

impl Poll {
    pub fn every_ms(&self) -> u64 {
        self.every_ms.unwrap_or(1000).min(MAX_WAIT_MS)
    }

    pub fn max_attempts(&self) -> usize {
        self.max_attempts.unwrap_or(DEFAULT_POLL_ATTEMPTS).clamp(1, MAX_POLL_ATTEMPTS)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.until.trim().is_empty() {
            return Err("`until` is empty".into());
        }
        if self.max_attempts.is_some_and(|m| m == 0 || m > MAX_POLL_ATTEMPTS) {
            return Err(format!("`max_attempts` must be 1 to {MAX_POLL_ATTEMPTS}"));
        }
        if self.every_ms.is_some_and(|m| m > MAX_WAIT_MS) {
            return Err(format!("`every_ms` is at most {MAX_WAIT_MS}"));
        }
        Ok(())
    }
}

/// Send the request again when it fails for a reason that passes: a rate limit, a busy server, a dropped
/// connection. It waits the `Retry-After` the server asks for, or `backoff_ms` doubled each time.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Retry {
    /// The statuses to try again on (default 429, 500, 502, 503, 504). A connection that failed is always tried again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_status: Option<Vec<u16>>,
    /// How many requests in all, the first included (default 3, at most 10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempts: Option<usize>,
    /// The first wait in milliseconds, doubled for each try (default 500, at most 60000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff_ms: Option<u64>,
}

impl Retry {
    pub fn statuses(&self) -> Vec<u16> {
        self.on_status.clone().unwrap_or_else(|| vec![429, 500, 502, 503, 504])
    }

    pub fn attempts(&self) -> usize {
        self.attempts.unwrap_or(3).clamp(1, MAX_RETRY_ATTEMPTS)
    }

    /// The wait before try `attempt + 1`, given the response's `Retry-After` header if it had one.
    pub fn wait_ms(&self, attempt: usize, retry_after: Option<&str>) -> u64 {
        if let Some(seconds) = retry_after.and_then(|v| v.trim().parse::<u64>().ok()) {
            return seconds.saturating_mul(1000).min(MAX_WAIT_MS);
        }
        let base = self.backoff_ms.unwrap_or(500).min(MAX_WAIT_MS);
        base.saturating_mul(1u64 << (attempt.saturating_sub(1)).min(10)).min(MAX_WAIT_MS)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.attempts.is_some_and(|a| a == 0 || a > MAX_RETRY_ATTEMPTS) {
            return Err(format!("`attempts` must be 1 to {MAX_RETRY_ATTEMPTS}"));
        }
        if self.on_status.as_ref().is_some_and(Vec::is_empty) {
            return Err("`on_status` is an empty list".into());
        }
        if self.backoff_ms.is_some_and(|m| m > MAX_WAIT_MS) {
            return Err(format!("`backoff_ms` is at most {MAX_WAIT_MS}"));
        }
        Ok(())
    }
}

/// A name usable for a collected list or a loop item.
pub fn valid_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
}

/// The URL of `rel="next"` in a `Link` header value, if it has one.
pub fn next_link(header: &str) -> Option<String> {
    let mut rest = header;
    while let Some(open) = rest.find('<') {
        let after = &rest[open + 1..];
        let close = after.find('>')?;
        let url = &after[..close];
        let tail = &after[close + 1..];
        let params = &tail[..tail.find('<').unwrap_or(tail.len())];
        let compact: String = params.chars().filter(|c| !c.is_whitespace()).collect();
        if compact.contains("rel=\"next\"") || compact.contains("rel=next") || compact.contains("rel=\"next,") {
            return Some(url.to_string());
        }
        rest = tail;
    }
    None
}

/// The URL to fetch for a `Link` header's `next`: an absolute URL as it is, a path (`/items?page=2`) on the
/// server `request_url` names. `request_url` is the step's URL as written, so a `{{base}}/items` keeps its
/// `{{base}}`, which is filled in when the request is sent.
pub fn resolve_link(next: &str, request_url: &str) -> Result<String, String> {
    if next.starts_with("http://") || next.starts_with("https://") {
        return Ok(next.to_string());
    }
    if next.starts_with('/') {
        let start = request_url.find("://").map_or(0, |i| i + 3);
        let origin_end = request_url[start..].find(['/', '?', '#']).map_or(request_url.len(), |i| start + i);
        return Ok(format!("{}{next}", &request_url[..origin_end]));
    }
    Err(format!("the Link header's next URL `{next}` is neither absolute nor a path"))
}

/// A JSON value as the text a variable holds: a string as it is, null as nothing, the rest as JSON.
pub fn text_of(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Makes `item` visible to a request as `{{name}}` (its text) and, for objects and arrays, as
/// `{{name.field}}`, `{{name.field.sub}}`, `{{name.0}}`. `{{loop.index}}` (from 0) and `{{loop.number}}` (from 1)
/// say where in the list it is.
pub fn bind_item(scope: &mut BTreeMap<String, String>, name: &str, item: &Value, index: usize) {
    const MAX_NAMES: usize = 500;
    const MAX_DEPTH: usize = 6;
    fn walk(scope: &mut BTreeMap<String, String>, path: &str, value: &Value, depth: usize, budget: &mut usize) {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        scope.insert(path.to_string(), text_of(value));
        if depth >= MAX_DEPTH {
            return;
        }
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    walk(scope, &format!("{path}.{key}"), child, depth + 1, budget);
                }
            }
            Value::Array(items) => {
                for (i, child) in items.iter().enumerate() {
                    walk(scope, &format!("{path}.{i}"), child, depth + 1, budget);
                }
            }
            _ => {}
        }
    }
    let mut budget = MAX_NAMES;
    walk(scope, name, item, 0, &mut budget);
    scope.insert("loop.index".into(), index.to_string());
    scope.insert("loop.number".into(), (index + 1).to_string());
}

/// The elements `for_each` goes over: a variable holding a JSON array (`issues`, or `{{issues}}`), or a JSON array
/// written out.
pub fn list_for_each(source: &str, visible: &BTreeMap<String, String>) -> Result<Vec<Value>, String> {
    let source = source.trim();
    let text = if source.starts_with('[') {
        source.to_string()
    } else {
        let name = source.trim_start_matches("{{").trim_end_matches("}}").trim();
        visible
            .get(name)
            .cloned()
            .ok_or_else(|| format!("`for_each` names `{name}`, which has no value yet; collect it in an earlier step"))?
    };
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&text) else {
        return Err(format!("`for_each` `{source}` is not a list: it must be a JSON array"));
    };
    if items.len() > MAX_ITERATIONS {
        return Err(format!("`for_each` has {} elements; the limit is {MAX_ITERATIONS}. Narrow the list first.", items.len()));
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_next_link_is_found_among_the_others() {
        let header = r#"<https://api.x/items?page=1>; rel="prev", <https://api.x/items?page=3&a=1,2>; rel="next", <https://api.x/items?page=9>; rel="last""#;
        assert_eq!(next_link(header).as_deref(), Some("https://api.x/items?page=3&a=1,2"));
        assert_eq!(next_link(r#"<https://a/b?p=2>; rel=next"#).as_deref(), Some("https://a/b?p=2"));
        assert_eq!(next_link(r#"<https://a/b?p=1>; rel="prev""#), None);
        assert_eq!(next_link(""), None);
        assert_eq!(next_link("<https://a/b"), None);
    }

    #[test]
    fn a_relative_next_link_is_joined_to_the_server_asked() {
        assert_eq!(resolve_link("https://a/b?p=2", "http://x/y").unwrap(), "https://a/b?p=2");
        assert_eq!(resolve_link("/items?page=2", "http://127.0.0.1:18095/items?page=1&per_page=5").unwrap(), "http://127.0.0.1:18095/items?page=2");
        assert_eq!(resolve_link("/v2/x", "https://api.example.com").unwrap(), "https://api.example.com/v2/x");
        assert_eq!(resolve_link("/items?page=2", "{{base}}/items?page=1").unwrap(), "{{base}}/items?page=2");
        assert!(resolve_link("items?page=2", "http://x").is_err());
    }

    #[test]
    fn an_item_is_visible_as_text_and_as_fields() {
        let mut scope = BTreeMap::new();
        bind_item(&mut scope, "issue", &json!({"key": "A-1", "fields": {"points": 3, "owner": null}, "labels": ["x", "y"]}), 4);
        assert_eq!(scope["issue.key"], "A-1");
        assert_eq!(scope["issue.fields.points"], "3");
        assert_eq!(scope["issue.fields.owner"], "");
        assert_eq!(scope["issue.labels.1"], "y");
        assert_eq!(scope["issue.fields"], r#"{"points":3,"owner":null}"#);
        assert!(scope["issue"].starts_with('{'));
        assert_eq!((scope["loop.index"].as_str(), scope["loop.number"].as_str()), ("4", "5"));
        let mut plain = BTreeMap::new();
        bind_item(&mut plain, "id", &json!("A-9"), 0);
        assert_eq!(plain["id"], "A-9");
    }

    #[test]
    fn for_each_reads_a_variable_or_a_written_list() {
        let mut vars = BTreeMap::new();
        vars.insert("keys".to_string(), r#"["A-1","A-2"]"#.to_string());
        vars.insert("name".to_string(), "not a list".to_string());
        assert_eq!(list_for_each("keys", &vars).unwrap().len(), 2);
        assert_eq!(list_for_each("{{keys}}", &vars).unwrap().len(), 2);
        assert_eq!(list_for_each("[1, 2, 3]", &vars).unwrap().len(), 3);
        assert!(list_for_each("nope", &vars).unwrap_err().contains("no value yet"));
        assert!(list_for_each("name", &vars).unwrap_err().contains("not a list"));
        let big = format!("[{}]", vec!["1"; MAX_ITERATIONS + 1].join(","));
        assert!(list_for_each(&big, &vars).unwrap_err().contains("limit"));
    }

    #[test]
    fn retries_wait_for_retry_after_or_back_off() {
        let retry = Retry { backoff_ms: Some(100), ..Default::default() };
        assert_eq!(retry.wait_ms(1, None), 100);
        assert_eq!(retry.wait_ms(2, None), 200);
        assert_eq!(retry.wait_ms(3, None), 400);
        assert_eq!(retry.wait_ms(1, Some("7")), 7000);
        assert_eq!(retry.wait_ms(1, Some("100000")), MAX_WAIT_MS, "a server cannot make a run wait for ever");
        assert_eq!(retry.wait_ms(1, Some("Wed, 21 Oct 2026 07:28:00 GMT")), 100, "a date is not understood, so back off");
        assert_eq!(Retry::default().statuses(), [429, 500, 502, 503, 504]);
    }

    #[test]
    fn mistakes_in_the_flow_settings_are_found_before_sending() {
        assert!(Paginate::default().validate().is_ok());
        assert!(Paginate { style: Some("cursor".into()), ..Default::default() }.validate().unwrap_err().contains("needs `next`"));
        assert!(Paginate { style: Some("magic".into()), ..Default::default() }.validate().is_err());
        assert!(Paginate { items: Some("bogus".into()), ..Default::default() }.validate().is_err());
        assert!(Paginate { max_pages: Some(501), ..Default::default() }.validate().is_err());
        assert!(Poll { until: " ".into(), ..Default::default() }.validate().is_err());
        assert!(Poll { until: "{{a}} == b".into(), ..Default::default() }.validate().is_ok());
        assert!(Retry { attempts: Some(0), ..Default::default() }.validate().is_err());
        assert!(valid_name("sprint-issues_2") && !valid_name("a b") && !valid_name(""));
    }
}
