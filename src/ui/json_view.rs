use eframe::egui;

/// The tree is not virtualised: every visible value is laid out every frame, so a
/// 60,000-object array cost gigabytes and froze the window. Documents with more values
/// than this are shown as a trimmed copy (see `limit_for_display`).
pub const TREE_NODE_BUDGET: usize = 4000;
const TREE_MAX_CHILDREN: usize = 200;

fn count_nodes(v: &serde_json::Value) -> usize {
    match v {
        serde_json::Value::Array(items) => 1 + items.iter().map(count_nodes).sum::<usize>(),
        serde_json::Value::Object(map) => 1 + map.values().map(count_nodes).sum::<usize>(),
        _ => 1,
    }
}

/// `None` when the whole document fits the tree budget. Otherwise a trimmed copy for the
/// tree (long lists keep their first items, followed by an "… N more" entry), and the
/// document's real number of values.
pub fn limit_for_display(v: &serde_json::Value) -> Option<(serde_json::Value, usize)> {
    let total = count_nodes(v);
    if total <= TREE_NODE_BUDGET {
        return None;
    }
    let mut budget = TREE_NODE_BUDGET;
    Some((trim(v, &mut budget), total))
}

fn trim(v: &serde_json::Value, budget: &mut usize) -> serde_json::Value {
    use serde_json::Value;
    *budget = budget.saturating_sub(1);
    match v {
        Value::Array(items) => {
            let mut out = Vec::new();
            for (i, item) in items.iter().enumerate() {
                if i >= TREE_MAX_CHILDREN || *budget == 0 {
                    out.push(Value::String(format!("… {} more items", items.len() - i)));
                    break;
                }
                out.push(trim(item, budget));
            }
            Value::Array(out)
        }
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (i, (k, item)) in map.iter().enumerate() {
                if i >= TREE_MAX_CHILDREN || *budget == 0 {
                    out.insert("…".to_string(), Value::String(format!("{} more properties", map.len() - i)));
                    break;
                }
                out.insert(k.clone(), trim(item, budget));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

/// The path to a node as a developer would type it (`$.items[0].name`), from its
/// RFC 6901 pointer. Walking the document tells an array index from an object key
/// that happens to be a number.
pub fn json_path(root: &serde_json::Value, pointer: &str) -> String {
    let mut path = String::from("$");
    let mut node = Some(root);
    for raw in pointer.split('/').skip(1) {
        let segment = raw.replace("~1", "/").replace("~0", "~");
        match node {
            Some(serde_json::Value::Array(items)) => {
                path.push_str(&format!("[{segment}]"));
                node = segment.parse::<usize>().ok().and_then(|i| items.get(i));
            }
            other => {
                let plain = !segment.is_empty()
                    && !segment.starts_with(|c: char| c.is_ascii_digit())
                    && segment.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                if plain {
                    path.push('.');
                    path.push_str(&segment);
                } else {
                    path.push_str(&format!("[{}]", serde_json::Value::String(segment.clone())));
                }
                node = match other {
                    Some(serde_json::Value::Object(map)) => map.get(&segment),
                    _ => None,
                };
            }
        }
    }
    path
}

pub fn pretty_json_if_possible(text: &str) -> (String, Option<serde_json::Value>) {
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(value) => {
            let pretty = serde_json::to_string_pretty(&value).unwrap_or_else(|_| text.to_string());
            (pretty, Some(value))
        }
        Err(_) => (text.to_string(), None),
    }
}

/// A small hand-written JSON highlighter (not a general syntax-highlighting
/// engine like syntect) — keeps the binary lightweight, and JSON's grammar
/// is simple enough that a general engine would be overkill. Used for the
/// request body editor; the response body uses egui_json_tree instead, which
/// gives real per-node folding.
pub fn highlight_json(text: &str) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let font_id = egui::FontId::monospace(13.0);

    let [color_punct, color_key, color_string, color_number, color_literal, color_default] = crate::ui::theme::palette().json;

    let append = |job: &mut egui::text::LayoutJob, s: &str, color: egui::Color32| {
        job.append(
            s,
            0.0,
            egui::TextFormat {
                font_id: font_id.clone(),
                color,
                ..Default::default()
            },
        );
    };

    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let n = chars.len();
    let mut idx = 0;

    while idx < n {
        let (byte_pos, ch) = chars[idx];

        if ch.is_whitespace() {
            let start = byte_pos;
            let mut end = byte_pos + ch.len_utf8();
            idx += 1;
            while idx < n && chars[idx].1.is_whitespace() {
                end = chars[idx].0 + chars[idx].1.len_utf8();
                idx += 1;
            }
            append(&mut job, &text[start..end], color_default);
            continue;
        }

        if ch == '"' {
            let start = byte_pos;
            let mut end = byte_pos + 1;
            idx += 1;
            let mut escaped = false;
            while idx < n {
                let (bp, c) = chars[idx];
                end = bp + c.len_utf8();
                idx += 1;
                if escaped {
                    escaped = false;
                    continue;
                }
                if c == '\\' {
                    escaped = true;
                    continue;
                }
                if c == '"' {
                    break;
                }
            }
            let mut lookahead = idx;
            while lookahead < n && chars[lookahead].1.is_whitespace() {
                lookahead += 1;
            }
            let is_key = lookahead < n && chars[lookahead].1 == ':';
            append(
                &mut job,
                &text[start..end],
                if is_key { color_key } else { color_string },
            );
            continue;
        }

        if matches!(ch, '{' | '}' | '[' | ']' | ',' | ':') {
            let start = byte_pos;
            let end = byte_pos + ch.len_utf8();
            idx += 1;
            append(&mut job, &text[start..end], color_punct);
            continue;
        }

        if ch == '-' || ch.is_ascii_digit() {
            let start = byte_pos;
            let mut end = byte_pos + ch.len_utf8();
            idx += 1;
            while idx < n {
                let (bp, c) = chars[idx];
                if c.is_ascii_digit() || c == '.' || c == 'e' || c == 'E' || c == '+' || c == '-' {
                    end = bp + c.len_utf8();
                    idx += 1;
                } else {
                    break;
                }
            }
            append(&mut job, &text[start..end], color_number);
            continue;
        }

        if ch.is_alphabetic() {
            let start = byte_pos;
            let mut end = byte_pos + ch.len_utf8();
            idx += 1;
            while idx < n && chars[idx].1.is_alphanumeric() {
                end = chars[idx].0 + chars[idx].1.len_utf8();
                idx += 1;
            }
            let word = &text[start..end];
            let color = if word == "true" || word == "false" || word == "null" {
                color_literal
            } else {
                color_default
            };
            append(&mut job, word, color);
            continue;
        }

        let start = byte_pos;
        let end = byte_pos + ch.len_utf8();
        idx += 1;
        append(&mut job, &text[start..end], color_default);
    }

    job
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_response_keeps_the_key_order_the_server_sent() {
        let (pretty, value) = pretty_json_if_possible(r#"{"zebra":1,"apple":{"y":2,"x":[3,{"b":1,"a":2}]}}"#);
        let order = |key: &str| pretty.find(&format!("\"{key}\"")).unwrap();
        assert!(order("zebra") < order("apple") && order("y") < order("x") && order("b") < order("a"), "{pretty}");
        assert_eq!(value.unwrap().as_object().unwrap().keys().collect::<Vec<_>>(), ["zebra", "apple"]);
    }

    use super::*;

    #[test]
    fn pretty_prints_valid_json_and_passes_through_invalid() {
        let (pretty, value) = pretty_json_if_possible("{\"a\":[1,2]}");
        assert!(pretty.contains("\n  \"a\""));
        assert!(value.is_some());
        let (raw, none) = pretty_json_if_possible("not json");
        assert_eq!(raw, "not json");
        assert!(none.is_none());
    }

    #[test]
    fn highlighter_never_drops_or_reorders_text() {
        for text in [
            "",
            "{\"a\": [1, -2.5e+3, true, null, \"x\\\"y\"]}",
            "{\"unterminated\": \"abc",
            "{\"emoji\": \"héllo 🚀 世界\"}",
            "garbage ~!@ {{ ]] 12abc",
        ] {
            assert_eq!(highlight_json(text).text, text, "{text:?}");
        }
    }

    #[test]
    fn paths_read_like_code() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"items":[{"name":"a","odd key":1,"7":true}],"a/b":2,"x~y":3}"#,
        )
        .unwrap();
        assert_eq!(json_path(&v, ""), "$");
        assert_eq!(json_path(&v, "/items"), "$.items");
        assert_eq!(json_path(&v, "/items/0/name"), "$.items[0].name");
        assert_eq!(json_path(&v, "/items/0/odd key"), "$.items[0][\"odd key\"]");
        // A numeric key on an object is a key, not an index.
        assert_eq!(json_path(&v, "/items/0/7"), "$.items[0][\"7\"]");
        assert_eq!(json_path(&v, "/a~1b"), "$[\"a/b\"]");
        assert_eq!(json_path(&v, "/x~0y"), "$[\"x~y\"]");
    }

    #[test]
    fn a_small_document_is_shown_whole() {
        let v: serde_json::Value = serde_json::from_str(r#"{"a":[1,2,3],"b":{"c":true}}"#).unwrap();
        assert!(limit_for_display(&v).is_none());
    }

    #[test]
    fn a_huge_list_is_trimmed_to_its_start_with_a_count_of_the_rest() {
        let rows: Vec<serde_json::Value> = (0..60_000).map(|i| serde_json::json!({"id": i, "name": "x"})).collect();
        let v = serde_json::Value::Array(rows);
        let (shown, total) = limit_for_display(&v).unwrap();
        assert_eq!(total, 1 + 60_000 * 3);
        let items = shown.as_array().unwrap();
        assert_eq!(items.len(), TREE_MAX_CHILDREN + 1);
        assert_eq!(items[0]["id"], 0);
        assert_eq!(items[TREE_MAX_CHILDREN], serde_json::json!("… 59800 more items"));
        assert!(count_nodes(&shown) <= TREE_NODE_BUDGET);
    }

    #[test]
    fn a_deep_wide_document_stays_within_the_budget() {
        let inner: Vec<serde_json::Value> = (0..150).map(|i| serde_json::json!({"a": i, "b": [1, 2, 3]})).collect();
        let outer: Vec<serde_json::Value> = (0..150).map(|_| serde_json::Value::Array(inner.clone())).collect();
        let (shown, _) = limit_for_display(&serde_json::Value::Array(outer)).unwrap();
        assert!(count_nodes(&shown) <= TREE_NODE_BUDGET + TREE_MAX_CHILDREN, "{}", count_nodes(&shown));
    }

    #[test]
    fn a_huge_object_keeps_its_first_properties() {
        let map: serde_json::Map<String, serde_json::Value> = (0..6000).map(|i| (format!("k{i:04}"), serde_json::json!(i))).collect();
        let (shown, total) = limit_for_display(&serde_json::Value::Object(map)).unwrap();
        assert_eq!(total, 6001);
        let obj = shown.as_object().unwrap();
        assert_eq!(obj.len(), TREE_MAX_CHILDREN + 1);
        assert_eq!(obj["…"], serde_json::json!("5800 more properties"));
    }
}
