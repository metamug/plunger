//! Checks on a step's response: `{"from": "json:$.id", "equals": 7}`. A step passes only when every
//! assert holds; each one is reported on its own, so a run says all that was wrong, not just the first.

use crate::domain::extract::{check_source, select};
use crate::domain::model::ResponseData;
use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One check on a response. Give `from` and at least one operator; every operator given must hold.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Default)]
pub struct Assert {
    /// What to check: `json:$.data.id` (or just `$.data.id`), `header:Name`, `status`, `time` (milliseconds
    /// the response took) or `body` (the whole body as text).
    pub from: String,
    /// A label for the report. Without it the check is described from its parts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The value must equal this (numbers compare as numbers, so 5 equals 5.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equals: Option<Value>,
    /// The value must differ from this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_equals: Option<Value>,
    /// A text value must contain this text; an array must have this item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contains: Option<Value>,
    /// `true`: the value must exist (a null counts as existing); `false`: it must be absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exists: Option<bool>,
    /// The value must be a number greater than this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gt: Option<f64>,
    /// ...at least this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gte: Option<f64>,
    /// ...less than this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lt: Option<f64>,
    /// ...at most this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lte: Option<f64>,
    /// The JSON type: `string`, `number`, `boolean`, `array`, `object` or `null`.
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub value_type: Option<String>,
    /// The number of characters (text), items (array) or members (object) must be this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<usize>,
}

/// What one assert found.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct AssertOutcome {
    pub name: String,
    pub ok: bool,
    /// Why it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl Assert {
    fn operators(&self) -> usize {
        [
            self.equals.is_some(),
            self.not_equals.is_some(),
            self.contains.is_some(),
            self.exists.is_some(),
            self.gt.is_some(),
            self.gte.is_some(),
            self.lt.is_some(),
            self.lte.is_some(),
            self.value_type.is_some(),
            self.length.is_some(),
        ]
        .iter()
        .filter(|set| **set)
        .count()
    }

    /// A mistake in the assert itself, found before anything is sent.
    pub fn validate(&self) -> Result<(), String> {
        let from = self.from.trim();
        if !matches!(from, "time" | "body") {
            check_source(from, true).map_err(|e| format!("`from` {e}"))?;
        }
        if self.operators() == 0 {
            return Err(format!(
                "the assert on `{from}` checks nothing: add equals, not_equals, contains, exists, gt, gte, lt, lte, type or length"
            ));
        }
        if let Some(t) = &self.value_type {
            if !matches!(t.as_str(), "string" | "number" | "boolean" | "array" | "object" | "null") {
                return Err(format!("`type` is `{t}`; use string, number, boolean, array, object or null"));
            }
        }
        Ok(())
    }

    /// How the report names this check.
    pub fn label(&self) -> String {
        if let Some(name) = self.name.as_deref().filter(|n| !n.trim().is_empty()) {
            return name.to_string();
        }
        let from = self.from.trim();
        let mut parts = Vec::new();
        let mut add = |op: &str, value: String| parts.push(format!("{op} {value}"));
        if let Some(v) = &self.equals {
            add("==", show(v));
        }
        if let Some(v) = &self.not_equals {
            add("!=", show(v));
        }
        if let Some(v) = &self.contains {
            add("contains", show(v));
        }
        if let Some(v) = self.exists {
            add("exists:", v.to_string());
        }
        for (op, v) in [(">", self.gt), (">=", self.gte), ("<", self.lt), ("<=", self.lte)] {
            if let Some(v) = v {
                add(op, v.to_string());
            }
        }
        if let Some(v) = &self.value_type {
            add("is", v.clone());
        }
        if let Some(v) = self.length {
            add("length ==", v.to_string());
        }
        format!("{from} {}", parts.join(", "))
    }
}

fn show(value: &Value) -> String {
    match value {
        Value::String(s) => format!("\"{s}\""),
        other => other.to_string(),
    }
}

fn type_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}

/// The value an assert looks at. `Ok(None)` is a value that is not there (for `exists`).
fn value_of(from: &str, response: &ResponseData, elapsed_ms: u64) -> Result<Option<Value>, String> {
    match from.trim() {
        "time" => Ok(Some(Value::from(elapsed_ms))),
        "body" => Ok(Some(Value::String(response.raw_text.clone().unwrap_or_else(|| response.body.clone())))),
        spec => match select(response, spec) {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.contains("not JSON") => Err(e),
            Err(_) => Ok(None),
        },
    }
}

/// Runs one assert against a response.
pub fn evaluate(assert: &Assert, response: &ResponseData, elapsed_ms: u64) -> AssertOutcome {
    let name = assert.label();
    let fail = |message: String| AssertOutcome { name: name.clone(), ok: false, message: Some(message) };
    let found = match value_of(&assert.from, response, elapsed_ms) {
        Ok(found) => found,
        Err(e) => return fail(e),
    };
    if let Some(want) = assert.exists {
        if want != found.is_some() {
            return fail(if want { format!("`{}` is not in the response", assert.from.trim()) } else { format!("`{}` is in the response", assert.from.trim()) });
        }
    }
    let Some(value) = found else {
        return if assert.operators() == 1 && assert.exists == Some(false) {
            AssertOutcome { name, ok: true, message: None }
        } else {
            fail(format!("`{}` is not in the response", assert.from.trim()))
        };
    };
    if let Some(want) = &assert.equals {
        if !same(&value, want) {
            return fail(format!("expected {}, got {}", show(want), show(&value)));
        }
    }
    if let Some(other) = &assert.not_equals {
        if same(&value, other) {
            return fail(format!("expected anything but {}", show(other)));
        }
    }
    if let Some(needle) = &assert.contains {
        let holds = match (&value, needle) {
            (Value::String(hay), Value::String(n)) => hay.contains(n.as_str()),
            (Value::String(hay), other) => hay.contains(&other.to_string()),
            (Value::Array(items), n) => items.iter().any(|i| same(i, n)),
            _ => return fail(format!("`contains` needs text or an array, got {}", type_of(&value))),
        };
        if !holds {
            return fail(format!("{} does not contain {}", short(&value), show(needle)));
        }
    }
    for (op, limit, test) in [
        (">", assert.gt, (|a: f64, b: f64| a > b) as fn(f64, f64) -> bool),
        (">=", assert.gte, |a, b| a >= b),
        ("<", assert.lt, |a, b| a < b),
        ("<=", assert.lte, |a, b| a <= b),
    ] {
        if let Some(limit) = limit {
            let Some(number) = number_of(&value) else {
                return fail(format!("expected a number {op} {limit}, got {} ({})", short(&value), type_of(&value)));
            };
            if !test(number, limit) {
                return fail(format!("expected {op} {limit}, got {number}"));
            }
        }
    }
    if let Some(want) = &assert.value_type {
        if type_of(&value) != want {
            return fail(format!("expected type {want}, got {}", type_of(&value)));
        }
    }
    if let Some(want) = assert.length {
        let got = match &value {
            Value::String(s) => s.chars().count(),
            Value::Array(a) => a.len(),
            Value::Object(o) => o.len(),
            other => return fail(format!("`length` needs text, an array or an object, got {}", type_of(other))),
        };
        if got != want {
            return fail(format!("expected length {want}, got {got}"));
        }
    }
    AssertOutcome { name, ok: true, message: None }
}

/// A number, or text that is one (a header value, a captured id).
fn number_of(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn short(value: &Value) -> String {
    let text = show(value);
    if text.chars().count() > 80 {
        format!("{}...", text.chars().take(80).collect::<String>())
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn response() -> ResponseData {
        ResponseData {
            status: 201,
            headers: vec![("Content-Type".into(), "application/json".into()), ("X-Count".into(), "12".into())],
            body: r#"{"id": 7, "name": "Ada", "tags": ["a", "b"], "owner": null, "price": 9.5}"#.into(),
            ..Default::default()
        }
    }

    fn check(a: Assert) -> AssertOutcome {
        evaluate(&a, &response(), 40)
    }

    fn on(from: &str) -> Assert {
        Assert { from: from.into(), ..Default::default() }
    }

    #[test]
    fn equality_compares_numbers_as_numbers() {
        assert!(check(Assert { equals: Some(json!(7)), ..on("json:$.id") }).ok);
        assert!(check(Assert { equals: Some(json!(7.0)), ..on("$.id") }).ok);
        assert!(check(Assert { equals: Some(json!("Ada")), ..on("$.name") }).ok);
        let bad = check(Assert { equals: Some(json!(8)), ..on("$.id") });
        assert!(!bad.ok);
        assert_eq!(bad.message.as_deref(), Some("expected 8, got 7"));
        assert!(check(Assert { equals: Some(json!(201)), ..on("status") }).ok);
        assert!(check(Assert { equals: Some(json!("application/json")), ..on("header:content-type") }).ok);
        assert!(check(Assert { not_equals: Some(json!(0)), ..on("$.id") }).ok);
        assert!(!check(Assert { not_equals: Some(json!(7)), ..on("$.id") }).ok);
    }

    #[test]
    fn contains_works_on_text_arrays_and_the_body() {
        assert!(check(Assert { contains: Some(json!("da")), ..on("$.name") }).ok);
        assert!(check(Assert { contains: Some(json!("b")), ..on("$.tags") }).ok);
        assert!(!check(Assert { contains: Some(json!("z")), ..on("$.tags") }).ok);
        assert!(check(Assert { contains: Some(json!("\"Ada\"")), ..on("body") }).ok);
        assert!(!check(Assert { contains: Some(json!(1)), ..on("$.id") }).ok, "a number holds nothing");
    }

    #[test]
    fn exists_tells_absent_from_null() {
        assert!(check(Assert { exists: Some(true), ..on("$.owner") }).ok, "null is there");
        assert!(!check(Assert { exists: Some(true), ..on("$.nope") }).ok);
        assert!(check(Assert { exists: Some(false), ..on("$.nope") }).ok);
        assert!(!check(Assert { exists: Some(false), ..on("$.id") }).ok);
        assert!(check(Assert { exists: Some(true), ..on("header:X-Count") }).ok);
        assert!(check(Assert { exists: Some(false), ..on("header:X-Nope") }).ok);
    }

    #[test]
    fn numbers_compare_with_limits_and_time_is_a_source() {
        assert!(check(Assert { gt: Some(6.0), lte: Some(7.0), ..on("$.id") }).ok);
        assert!(!check(Assert { lt: Some(7.0), ..on("$.id") }).ok);
        assert!(check(Assert { gte: Some(10.0), ..on("header:X-Count") }).ok, "a header holding a number counts");
        assert!(check(Assert { lt: Some(100.0), ..on("time") }).ok);
        let slow = check(Assert { lt: Some(10.0), ..on("time") });
        assert_eq!(slow.message.as_deref(), Some("expected < 10, got 40"));
        assert!(!check(Assert { gt: Some(1.0), ..on("$.name") }).ok);
    }

    #[test]
    fn type_and_length() {
        assert!(check(Assert { value_type: Some("array".into()), length: Some(2), ..on("$.tags") }).ok);
        assert!(check(Assert { value_type: Some("null".into()), ..on("$.owner") }).ok);
        assert!(!check(Assert { value_type: Some("string".into()), ..on("$.id") }).ok);
        assert!(check(Assert { length: Some(3), ..on("$.name") }).ok);
        assert!(!check(Assert { length: Some(1), ..on("$.price") }).ok);
    }

    #[test]
    fn a_missing_value_and_a_non_json_body_say_so() {
        let missing = check(Assert { equals: Some(json!(1)), ..on("$.nope") });
        assert_eq!(missing.message.as_deref(), Some("`$.nope` is not in the response"));
        let plain = ResponseData { body: "hello".into(), ..Default::default() };
        let out = evaluate(&Assert { equals: Some(json!(1)), ..on("$.a") }, &plain, 1);
        assert!(out.message.unwrap().contains("not JSON"));
    }

    #[test]
    fn mistakes_in_an_assert_are_found_before_sending() {
        assert!(Assert { equals: Some(json!(1)), ..on("$.a") }.validate().is_ok());
        assert!(Assert { lt: Some(1.0), ..on("time") }.validate().is_ok());
        assert!(on("$.a").validate().unwrap_err().contains("checks nothing"));
        assert!(Assert { equals: Some(json!(1)), ..on("bogus") }.validate().is_err());
        assert!(Assert { value_type: Some("int".into()), ..on("$.a") }.validate().is_err());
        assert!(Assert { equals: Some(json!(1)), ..on("") }.validate().is_err());
    }

    #[test]
    fn a_name_overrides_the_description() {
        assert_eq!(Assert { equals: Some(json!(7)), ..on("$.id") }.label(), "$.id == 7");
        assert_eq!(Assert { name: Some("has an id".into()), equals: Some(json!(7)), ..on("$.id") }.label(), "has an id");
    }
}
