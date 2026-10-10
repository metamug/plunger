//! The shape of a big JSON response, in a few hundred tokens: keys and types, array lengths, and a
//! short example of each scalar. An agent reads this instead of 50,000 characters of cut-off text,
//! then asks for exactly the values it needs with `select`.

use serde_json::{json, Map, Value};

const MAX_DEPTH: usize = 5;
const MAX_KEYS: usize = 30;
const MAX_EXAMPLE_CHARS: usize = 24;

/// The outline of `value`: objects keep their keys, an array becomes `{"array": length, "items": <outline of
/// the first item>}`, and a scalar becomes its type with a short example.
pub fn outline(value: &Value) -> Value {
    walk(value, 0)
}

fn walk(value: &Value, depth: usize) -> Value {
    match value {
        Value::Null => json!("null"),
        Value::Bool(b) => json!(format!("boolean, e.g. {b}")),
        Value::Number(n) => json!(format!("number, e.g. {n}")),
        Value::String(s) => {
            let shown: String = s.chars().take(MAX_EXAMPLE_CHARS).collect();
            let cut = if s.chars().count() > MAX_EXAMPLE_CHARS { "…" } else { "" };
            json!(format!("string, e.g. {shown:?}{cut}"))
        }
        Value::Array(items) => {
            let mut out = Map::new();
            out.insert("array".into(), json!(items.len()));
            if let Some(first) = items.first() {
                out.insert("items".into(), if depth >= MAX_DEPTH { json!("…") } else { walk(first, depth + 1) });
            }
            Value::Object(out)
        }
        Value::Object(map) => {
            if depth >= MAX_DEPTH {
                return json!(format!("object with {} keys", map.len()));
            }
            let mut out = Map::new();
            for (key, child) in map.iter().take(MAX_KEYS) {
                out.insert(key.clone(), walk(child, depth + 1));
            }
            if map.len() > MAX_KEYS {
                out.insert("…".into(), json!(format!("{} more keys", map.len() - MAX_KEYS)));
            }
            Value::Object(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_outline_shows_keys_types_and_array_lengths() {
        let v = json!({"data": [{"id": 7, "name": "Ann", "tags": ["a", "b"]}, {"id": 8}], "total": 2, "next": null, "ok": true});
        let o = outline(&v);
        assert_eq!(o["data"]["array"], 2);
        assert_eq!(o["data"]["items"]["id"], "number, e.g. 7");
        assert_eq!(o["data"]["items"]["name"], "string, e.g. \"Ann\"");
        assert_eq!(o["data"]["items"]["tags"]["array"], 2);
        assert_eq!(o["next"], "null");
        assert_eq!(o["ok"], "boolean, e.g. true");
    }

    #[test]
    fn a_huge_response_stays_small() {
        let rows: Vec<Value> = (0..5000).map(|i| json!({"id": i, "name": "x".repeat(500), "meta": {"a": {"b": {"c": {"d": {"e": {"f": 1}}}}}}})).collect();
        let big = json!({"rows": rows, "wide": (0..200).map(|i| (format!("k{i}"), json!(i))).collect::<Map<String, Value>>()});
        let text = outline(&big).to_string();
        assert!(text.len() < 3000, "{} chars", text.len());
        assert!(text.contains("\"array\":5000") && text.contains("170 more keys"), "{text}");
        assert!(text.contains("…") && !text.contains(&"x".repeat(100)), "long strings are cut");
    }
}
