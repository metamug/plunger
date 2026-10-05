//! Response search: which parts of the displayed body match the query.
//!
//! Matching is ASCII case-insensitive, like the JSON tree's own highlighting. Lists stop at
//! `MAX_MATCHES` so a query like `x` on a huge body stays cheap in time and memory.

use serde_json::Value;

pub const MAX_MATCHES: usize = 5_000;

pub type Range = (usize, usize);

/// The match list for the response on screen; rebuilt only when the response or query changes,
/// not on every frame.
#[derive(Default)]
pub struct Cache {
    key: Option<(usize, usize, u128, String)>,
    /// The (response, query) the JSON tree's open/closed state was last reset for.
    pub expand_key: Option<(usize, usize, u128, Option<String>)>,
    pub text: Vec<Range>,
    pub json: Vec<String>,
}

impl Cache {
    pub fn refresh(&mut self, key: (usize, usize, u128, String), compute: impl FnOnce() -> (Vec<Range>, Vec<String>)) {
        if self.key.as_ref() != Some(&key) {
            (self.text, self.json) = compute();
            self.key = Some(key);
        }
    }
}

/// Non-overlapping byte ranges of `query` in `text`, in order.
pub fn text_matches(text: &str, query: &str) -> Vec<Range> {
    let (hay, needle) = (text.as_bytes(), query.as_bytes());
    let mut out = Vec::new();
    if needle.is_empty() || needle.len() > hay.len() {
        return out;
    }
    let mut i = 0;
    while i + needle.len() <= hay.len() && out.len() < MAX_MATCHES {
        let end = i + needle.len();
        if text.is_char_boundary(i) && text.is_char_boundary(end) && hay[i..end].eq_ignore_ascii_case(needle) {
            out.push((i, end));
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// JSON pointers of the nodes the tree highlights for `query`, in document order: an object
/// key that contains it, or a string, number, bool or null value that does.
pub fn json_matches(value: &Value, query: &str) -> Vec<String> {
    let mut out = Vec::new();
    if !query.is_empty() {
        walk(value, &query.to_ascii_lowercase(), &mut String::new(), &mut out);
    }
    out
}

fn note(out: &mut Vec<String>, path: &str) {
    if out.last().map(String::as_str) != Some(path) {
        out.push(path.to_owned());
    }
}

fn walk(value: &Value, needle: &str, path: &mut String, out: &mut Vec<String>) {
    if out.len() >= MAX_MATCHES {
        return;
    }
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let len = path.len();
                path.push('/');
                path.push_str(&key.replace('~', "~0").replace('/', "~1"));
                if key.to_ascii_lowercase().contains(needle) {
                    note(out, path);
                }
                walk(child, needle, path, out);
                path.truncate(len);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                let len = path.len();
                path.push_str(&format!("/{index}"));
                walk(child, needle, path, out);
                path.truncate(len);
            }
        }
        base => {
            let shown = match base {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            if shown.to_ascii_lowercase().contains(needle) {
                note(out, path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn text_matches_ignore_ascii_case_and_never_overlap() {
        assert_eq!(text_matches("Item item ITEM", "item"), vec![(0, 4), (5, 9), (10, 14)]);
        assert_eq!(text_matches("aaa", "aa"), vec![(0, 2)]);
        assert!(text_matches("anything", "").is_empty());
        assert!(text_matches("short", "much longer query").is_empty());
    }

    #[test]
    fn text_matches_stay_on_char_boundaries() {
        assert_eq!(text_matches("İx TEST", "test"), vec![(4, 8)]);
        assert_eq!(text_matches("café", "é"), vec![(3, 5)]);
        assert!(text_matches("café", "CAFÉ").is_empty());
    }

    #[test]
    fn text_matches_are_capped() {
        let body = "x".repeat(MAX_MATCHES * 3);
        assert_eq!(text_matches(&body, "x").len(), MAX_MATCHES);
    }

    #[test]
    fn json_matches_cover_keys_and_values_in_document_order() {
        let doc = json!({"items": [{"name": "Grace", "id": 2}, {"name": "ada", "grace": null}], "odd/key~": "grace"});
        assert_eq!(
            json_matches(&doc, "grace"),
            vec!["/items/0/name", "/items/1/grace", "/odd~1key~0"]
        );
    }

    #[test]
    fn json_matches_count_a_node_once_when_key_and_value_both_match() {
        assert_eq!(json_matches(&json!({"name": "name"}), "name"), vec!["/name"]);
    }

    #[test]
    fn json_matches_are_capped() {
        let doc = Value::Array((0..MAX_MATCHES * 2).map(|_| json!("x")).collect());
        assert_eq!(json_matches(&doc, "x").len(), MAX_MATCHES);
    }
}
