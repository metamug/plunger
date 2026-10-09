//! Picking a value out of a response so a later request can use it: `json:$.data.token`,
//! `header:X-Request-Id` or `status`. The value is read from the response exactly as the server
//! sent it, before any secret is masked, so a token can be passed on without an agent seeing it.

use crate::model::ResponseData;
use serde_json::Value;

/// The value `from` points at in `response`, as text.
pub fn extract(response: &ResponseData, from: &str) -> Result<String, String> {
    let from = from.trim();
    if from == "status" {
        return Ok(response.status.to_string());
    }
    if let Some(name) = from.strip_prefix("header:") {
        let name = name.trim();
        return response
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
            .ok_or_else(|| format!("the response has no `{name}` header"));
    }
    if let Some(path) = from.strip_prefix("json:") {
        let text = response.raw_text.as_deref().unwrap_or(&response.body);
        let value: Value = serde_json::from_str(text).map_err(|_| "the response body is not JSON".to_string())?;
        let found = json_path(&value, path)?;
        return Ok(match found {
            Value::String(s) => s,
            other => other.to_string(),
        });
    }
    Err(format!("`{from}` is not a source: use `json:$.path`, `header:Name` or `status`"))
}

/// Whether `spec` names something that can be read from a response, so a mistake is reported before a
/// request is sent (and has its effect) rather than after. `bare_json` allows `$.a.b` without `json:`.
pub fn check_source(spec: &str, bare_json: bool) -> Result<(), String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err("it is empty: give `json:$.path`, `header:Name` or `status`".into());
    }
    if spec == "status" {
        return Ok(());
    }
    if let Some(name) = spec.strip_prefix("header:") {
        return if name.trim().is_empty() { Err("`header:` needs a header name".into()) } else { Ok(()) };
    }
    if let Some(path) = spec.strip_prefix("json:") {
        return parse_path(path).map(|_| ());
    }
    if bare_json && (spec.starts_with('$') || spec.starts_with('[')) {
        return parse_path(spec).map(|_| ());
    }
    Err(format!("`{spec}` is not a source: use `json:$.path`, `header:Name` or `status`"))
}

/// The value `spec` points at, keeping its JSON type: `$.a.b[0]` or `json:$.a`, `header:Name`, `status`.
pub fn select(response: &ResponseData, spec: &str) -> Result<Value, String> {
    let spec = spec.trim();
    if spec.starts_with("header:") || spec == "status" {
        return extract(response, spec).map(|text| {
            if spec == "status" {
                text.parse::<u16>().map(Value::from).unwrap_or(Value::String(text))
            } else {
                Value::String(text)
            }
        });
    }
    let path = spec.strip_prefix("json:").unwrap_or(spec);
    let text = response.raw_text.as_deref().unwrap_or(&response.body);
    let value: Value = serde_json::from_str(text).map_err(|_| "the response body is not JSON".to_string())?;
    json_path(&value, path)
}

/// One step of a path.
#[derive(Debug, PartialEq)]
enum Segment {
    Key(String),
    Index(usize),
    /// `[*]`: every item of an array.
    Every,
}

fn parse_path(path: &str) -> Result<Vec<Segment>, String> {
    let path = path.trim();
    let rest = path.strip_prefix('$').unwrap_or(path);
    let chars: Vec<char> = rest.chars().collect();
    let mut segments = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '.' => {
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != '.' && chars[i] != '[' {
                    i += 1;
                }
                if start == i {
                    return Err(format!("`{path}` has an empty name"));
                }
                segments.push(Segment::Key(chars[start..i].iter().collect()));
            }
            '[' => {
                let close = chars[i..].iter().position(|c| *c == ']').ok_or_else(|| format!("`{path}` has a [ without a ]"))? + i;
                let inner: String = chars[i + 1..close].iter().collect();
                let inner = inner.trim();
                if inner == "*" {
                    segments.push(Segment::Every);
                } else if let Some(quoted) = inner.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')).or_else(|| inner.strip_prefix('"').and_then(|s| s.strip_suffix('"'))) {
                    segments.push(Segment::Key(quoted.to_string()));
                } else {
                    segments.push(Segment::Index(inner.parse().map_err(|_| format!("`[{inner}]` is not an index or a quoted name"))?));
                }
                i = close + 1;
            }
            _ if segments.is_empty() => {
                // `data.token` without the leading `$.`
                let start = i;
                while i < chars.len() && chars[i] != '.' && chars[i] != '[' {
                    i += 1;
                }
                segments.push(Segment::Key(chars[start..i].iter().collect()));
            }
            c => return Err(format!("`{path}`: unexpected `{c}`")),
        }
    }
    Ok(segments)
}

/// Follows a path like `$.items[0].id`, `$['a b'].c` or `$.items[*].id` through `value`. A `[*]` collects the
/// value from every item into an array (an item that lacks it is skipped).
pub fn json_path(value: &Value, path: &str) -> Result<Value, String> {
    let segments = parse_path(path)?;
    let mut current: Vec<&Value> = vec![value];
    let mut fanned = false;
    for segment in &segments {
        let mut next: Vec<&Value> = Vec::new();
        for node in &current {
            match (segment, node) {
                (Segment::Key(k), Value::Object(map)) => match map.get(k) {
                    Some(v) => next.push(v),
                    None if fanned => {}
                    None => return Err(format!("no `{k}` in the response (at `{path}`)")),
                },
                (Segment::Index(n), Value::Array(items)) => match items.get(*n) {
                    Some(v) => next.push(v),
                    None if fanned => {}
                    None => return Err(format!("no item {n}: the array has {} (at `{path}`)", items.len())),
                },
                (Segment::Every, Value::Array(items)) => {
                    fanned = true;
                    next.extend(items.iter());
                }
                (Segment::Every, _) => return Err(format!("[*] is not inside an array (at `{path}`)")),
                (Segment::Key(k), _) if !fanned => return Err(format!("`{k}` is not inside an object (at `{path}`)")),
                (Segment::Index(n), _) if !fanned => return Err(format!("[{n}] is not inside an array (at `{path}`)")),
                _ => {}
            }
        }
        current = next;
    }
    if fanned {
        Ok(Value::Array(current.into_iter().cloned().collect()))
    } else {
        Ok(current.first().map(|v| (*v).clone()).unwrap_or(Value::Null))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn response(body: &str) -> ResponseData {
        ResponseData {
            status: 201,
            headers: vec![("X-Request-Id".into(), "abc".into())],
            body: body.into(),
            ..Default::default()
        }
    }

    #[test]
    fn paths_walk_objects_arrays_and_quoted_names() {
        let v = json!({"data": {"items": [{"id": 7}, {"id": 8}], "a b": true}});
        assert_eq!(json_path(&v, "$.data.items[1].id").unwrap(), json!(8));
        assert_eq!(json_path(&v, "data.items[0].id").unwrap(), json!(7));
        assert_eq!(json_path(&v, "$.data['a b']").unwrap(), json!(true));
        assert_eq!(json_path(&v, "$").unwrap(), v);
        assert_eq!(json_path(&v, "$.data.items[*].id").unwrap(), json!([7, 8]));
        let loose = json!({"rows": [{"id": 1, "tag": "a"}, {"id": 2}, {"id": 3, "tag": "c"}]});
        assert_eq!(json_path(&loose, "$.rows[*].tag").unwrap(), json!(["a", "c"]), "an item without it is skipped");
        assert_eq!(json_path(&loose, "$.rows[*]").unwrap().as_array().unwrap().len(), 3);
        assert!(json_path(&loose, "$.rows[0][*]").is_err());
    }

    #[test]
    fn a_missing_part_says_where_it_stopped() {
        let v = json!({"a": [1]});
        assert!(json_path(&v, "$.b").unwrap_err().contains("no `b`"));
        assert!(json_path(&v, "$.a[3]").unwrap_err().contains("array has 1"));
        assert!(json_path(&v, "$.a.b").unwrap_err().contains("not inside an object"));
        assert!(json_path(&v, "$[0]").unwrap_err().contains("not inside an array"));
        assert!(json_path(&v, "$.a[x]").is_err());
    }

    #[test]
    fn values_come_from_the_body_headers_or_status() {
        let r = response(r#"{"token": "t0k", "n": 5, "ok": true}"#);
        assert_eq!(extract(&r, "json:$.token").unwrap(), "t0k");
        assert_eq!(extract(&r, "json:$.n").unwrap(), "5");
        assert_eq!(extract(&r, "json:$.ok").unwrap(), "true");
        assert_eq!(extract(&r, "header:x-request-id").unwrap(), "abc");
        assert_eq!(extract(&r, "status").unwrap(), "201");
        assert!(extract(&r, "header:nope").is_err());
        assert!(extract(&r, "cookie:x").unwrap_err().contains("not a source"));
        assert!(extract(&response("plain"), "json:$.a").unwrap_err().contains("not JSON"));
    }

    #[test]
    fn mistakes_in_a_source_are_found_before_anything_is_sent() {
        for good in ["status", "header:Location", "json:$.a.b[0]", "json:$.items[*].id"] {
            assert!(check_source(good, false).is_ok(), "{good}");
        }
        assert!(check_source("$.a.b", true).is_ok());
        assert!(check_source("$.a.b", false).is_err(), "extract needs the json: prefix");
        for bad in ["", "  ", "bogus", "header:", "json:$.items[", "json:$.a[x]", "cookie:x"] {
            assert!(check_source(bad, false).is_err(), "{bad:?}");
        }
        assert!(check_source("", true).unwrap_err().contains("empty"));
    }

    #[test]
    fn select_keeps_the_json_type() {
        let r = response(r#"{"data": [{"id": 7, "tags": ["a"]}], "ok": true}"#);
        assert_eq!(select(&r, "$.data[0].id").unwrap(), serde_json::json!(7));
        assert_eq!(select(&r, "json:$.data[0].tags").unwrap(), serde_json::json!(["a"]));
        assert_eq!(select(&r, "$.ok").unwrap(), serde_json::json!(true));
        assert_eq!(select(&r, "status").unwrap(), serde_json::json!(201));
        assert_eq!(select(&r, "header:X-Request-Id").unwrap(), serde_json::json!("abc"));
        assert!(select(&r, "$.nope").is_err());
    }

    #[test]
    fn the_original_text_wins_over_the_reformatted_copy() {
        let mut r = response("{\n  \"id\": 12345678901234567890\n}");
        r.raw_text = Some("{\"id\":\"keep\"}".into());
        assert_eq!(extract(&r, "json:$.id").unwrap(), "keep");
    }
}
