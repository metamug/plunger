use eframe::egui;

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

    let [color_punct, color_key, color_string, color_number, color_literal, color_default] = crate::theme::palette().json;

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
}
