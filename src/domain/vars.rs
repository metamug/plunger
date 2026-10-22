//! `{{name}}` variable substitution, plus a few dynamic built-ins
//! (`{{$uuid}}`, `{{$timestamp}}`, `{{$randomInt}}`, and `{{$env:NAME}}`).

use crate::domain::model::{BodyMode, PersistedState, Variable};
use std::collections::BTreeSet;
use std::hash::{BuildHasher, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

/// Substitutes variables across many fields while remembering which names
/// couldn't be resolved, so the caller can refuse to send instead of
/// transmitting a literal `{{token}}`.
pub struct Resolver<'a> {
    vars: &'a [Variable],
    undefined: BTreeSet<String>,
}

impl<'a> Resolver<'a> {
    pub fn new(vars: &'a [Variable]) -> Self {
        Self {
            vars,
            undefined: BTreeSet::new(),
        }
    }

    pub fn apply(&mut self, text: &str) -> String {
        self.apply_with(text, str::to_string)
    }

    /// Like `apply`, but each substituted value is passed through `encode`
    /// first — e.g. to percent-encode values that land in a query string.
    pub fn apply_with(&mut self, text: &str, encode: impl Fn(&str) -> String) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find("{{") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let Some(end) = after.find("}}") else {
                // Unterminated: not a variable, keep the text as-is.
                out.push_str(&rest[start..]);
                return out;
            };
            let token_len = 2 + end + 2;
            match self.resolve(&after[..end]) {
                Resolved::Value(value) => out.push_str(&encode(&value)),
                // `{{name?}}` with no value: nothing, outside a JSON body
                Resolved::Omit => {}
                // Undefined or not a variable at all (e.g. `{{"a":1}}`): leave untouched.
                Resolved::Undefined => out.push_str(&rest[start..start + token_len]),
            }
            rest = &rest[start + token_len..];
        }
        out.push_str(rest);
        out
    }

    /// Substitutes into a JSON document. A value that lands inside a JSON string is escaped for it
    /// (quotes, backslashes, newlines and other control characters), so a description with a line
    /// break or a quote can't break the body. A value outside a string (`{"n": {{count}}}`,
    /// `{"labels": {{labels}}}`) is inserted as it is, so it can be a number, `true` or a whole
    /// JSON array or object.
    ///
    /// An optional placeholder with no value, `{{name?}}`, takes its whole member out of the
    /// document (`"assignee": "{{who?}}"` or `"labels": {{labels?}}`), or its item out of an array.
    pub fn apply_json(&mut self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        let mut in_string = false;
        let mut omitted = false;
        while let Some(start) = rest.find("{{") {
            let before = &rest[..start];
            in_string = advance_json_string_state(before, in_string);
            out.push_str(before);
            let after = &rest[start + 2..];
            let Some(end) = after.find("}}") else {
                out.push_str(&rest[start..]);
                return finish_json(out, omitted);
            };
            let token_len = 2 + end + 2;
            match self.resolve(&after[..end]) {
                Resolved::Value(value) if in_string => out.push_str(&json_escape(&value)),
                Resolved::Value(value) => out.push_str(&value),
                Resolved::Omit => {
                    // the whole string, `"{{x?}}"`, or a value outside a string: the member goes
                    let whole_string = in_string && opens_a_string(&out) && rest[start + token_len..].starts_with('"');
                    if whole_string {
                        out.push_str(OMIT_INSIDE_STRING);
                        omitted = true;
                    } else if !in_string {
                        out.push_str(OMIT_AS_STRING);
                        omitted = true;
                    }
                    // inside a longer string it is just empty
                }
                Resolved::Undefined => out.push_str(&rest[start..start + token_len]),
            }
            rest = &rest[start + token_len..];
        }
        out.push_str(rest);
        finish_json(out, omitted)
    }

    /// What `{{inner}}` stands for: its value, nothing (an optional one with no value), or unknown
    /// (which is remembered, so the send can be refused).
    fn resolve(&mut self, inner: &str) -> Resolved {
        let placeholder = parse_placeholder(inner);
        let name = placeholder.name;
        let found = self.lookup(name);
        let usable = found.filter(|v| !v.is_empty() || (!placeholder.optional && placeholder.default.is_none()));
        if let Some(value) = usable {
            return Resolved::Value(value);
        }
        if let Some(default) = placeholder.default {
            return Resolved::Value(default.to_string());
        }
        if placeholder.optional {
            return Resolved::Omit;
        }
        if is_valid_name(name) || environment_name(name).is_some() {
            self.undefined.insert(name.to_string());
        }
        Resolved::Undefined
    }

    fn lookup(&self, name: &str) -> Option<String> {
        if let Some(env_name) = environment_name(name) {
            return std::env::var(env_name).ok();
        }
        if !is_valid_name(name) {
            return None;
        }
        if name.starts_with('$') {
            return dynamic_value(name);
        }
        self.vars.iter().find(|v| v.name.trim() == name).map(|v| v.value.clone())
    }

    pub fn finish(self) -> Result<(), String> {
        if self.undefined.is_empty() {
            return Ok(());
        }
        let names: Vec<String> = self.undefined.iter().map(|n| format!("{{{{{n}}}}}")).collect();
        Err(format!(
            "Undefined variable{}: {}. Define {} in Tools > Variables.",
            if names.len() == 1 { "" } else { "s" },
            names.join(", "),
            if names.len() == 1 { "it" } else { "them" },
        ))
    }
}

/// `{{names}}` referenced anywhere in the request, in order of first use. The built-ins that fill
/// themselves in (`$uuid`, ...) are left out; an environment variable is listed as `$env:NAME`
/// because it must be set when the request is sent.
pub fn variables_used(state: &PersistedState) -> Vec<String> {
    placeholders_used(state).into_iter().map(|p| p.name).collect()
}

/// A `{{placeholder}}` a request uses, with what its marks say.
#[derive(Debug, Clone, PartialEq)]
pub struct UsedPlaceholder {
    pub name: String,
    /// `{{name|default}}`
    pub default: Option<String>,
    /// `{{name?}}`
    pub optional: bool,
}

/// The placeholders of a request, in order of first use (see [`variables_used`]).
pub fn placeholders_used(state: &PersistedState) -> Vec<UsedPlaceholder> {
    let mut texts = vec![state.url.as_str(), state.headers_text.as_str()];
    match state.body_mode {
        BodyMode::Json => texts.push(&state.json_body),
        BodyMode::Raw | BodyMode::Xml => texts.push(&state.raw_body),
        BodyMode::UrlEncoded => texts.push(&state.urlencoded_body),
        BodyMode::Multipart | BodyMode::None => {}
    }
    let mut names: Vec<UsedPlaceholder> = Vec::new();
    for field in &state.multipart_fields {
        collect_names(&field.value, &mut names);
    }
    for text in texts {
        collect_names(text, &mut names);
    }
    names
}

fn collect_names(text: &str, names: &mut Vec<UsedPlaceholder>) {
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { return };
        let placeholder = parse_placeholder(&after[..end]);
        let name = placeholder.name;
        let is_env = name.strip_prefix("$env:").is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
        let valid = is_env
            || (!name.is_empty()
                && !name.starts_with('$')
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'));
        if valid && !names.iter().any(|n| n.name == name) {
            names.push(UsedPlaceholder {
                name: name.to_string(),
                default: placeholder.default.map(str::to_string),
                optional: placeholder.optional,
            });
        }
        rest = &after[end + 2..];
    }
}

enum Resolved {
    Value(String),
    Omit,
    Undefined,
}

/// What is written between the braces: `name`, `name|default` (used when there is no value) or
/// `name?` (optional: left out of a JSON body, empty elsewhere, when there is no value).
#[derive(Debug, PartialEq)]
pub struct Placeholder<'a> {
    pub name: &'a str,
    pub default: Option<&'a str>,
    pub optional: bool,
}

pub fn parse_placeholder(inner: &str) -> Placeholder<'_> {
    let (name, default) = match inner.split_once('|') {
        Some((name, default)) => (name, Some(default.trim())),
        None => (inner, None),
    };
    let name = name.trim();
    match name.strip_suffix('?') {
        Some(stripped) => Placeholder { name: stripped.trim_end(), default, optional: true },
        None => Placeholder { name, default, optional: false },
    }
}

/// A string no request would carry, standing for "take this member out" until the body is parsed.
const OMIT_INSIDE_STRING: &str = "\\u0000plunger-omit";
const OMIT_AS_STRING: &str = "\"\\u0000plunger-omit\"";
const OMIT_VALUE: &str = "\u{0}plunger-omit";

/// Whether the last character written is a quote that opens (or closes) a string, not an escaped one.
fn opens_a_string(out: &str) -> bool {
    let Some(before) = out.strip_suffix('"') else { return false };
    (before.len() - before.trim_end_matches('\\').len()) % 2 == 0
}

/// Takes the members and items that stand for an omitted placeholder out of the document.
fn finish_json(text: String, omitted: bool) -> String {
    if !omitted {
        return text;
    }
    /// Removes the omitted members and items. A container that this left empty is removed too (an
    /// `"assignee": {"name": "{{who?}}"}` with no `who` has no assignee), while one that was empty in the
    /// template stays. Returns whether `value` is such an emptied container.
    fn prune(value: &mut serde_json::Value) -> bool {
        match value {
            serde_json::Value::Object(map) => {
                let before = map.len();
                map.retain(|k, v| k != OMIT_VALUE && v.as_str() != Some(OMIT_VALUE));
                let mut dropped = map.len() != before;
                let emptied: Vec<String> = map.iter_mut().filter_map(|(k, v)| prune(v).then(|| k.clone())).collect();
                if !emptied.is_empty() {
                    map.retain(|k, _| !emptied.contains(k));
                    dropped = true;
                }
                dropped && map.is_empty()
            }
            serde_json::Value::Array(items) => {
                let before = items.len();
                items.retain(|v| v.as_str() != Some(OMIT_VALUE));
                let mut dropped = items.len() != before;
                items.retain_mut(|v| {
                    let gone = prune(v);
                    dropped |= gone;
                    !gone
                });
                dropped && items.is_empty()
            }
            _ => false,
        }
    }
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(mut value) => {
            let _ = prune(&mut value);
            serde_json::to_string(&value).unwrap_or(text)
        }
        // not JSON as written; say null rather than send the marker
        Err(_) => text.replace(OMIT_AS_STRING, "null").replace(OMIT_INSIDE_STRING, ""),
    }
}

/// Whether the end of `text` is inside a JSON string, given whether its start was.
fn advance_json_string_state(text: &str, mut in_string: bool) -> bool {
    let mut escaped = false;
    for c in text.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
        }
    }
    in_string
}

/// `value` as the inside of a JSON string (no surrounding quotes).
pub fn json_escape(value: &str) -> String {
    let quoted = serde_json::to_string(value).unwrap_or_default();
    quoted.strip_prefix('"').and_then(|q| q.strip_suffix('"')).unwrap_or(&quoted).to_string()
}

fn environment_name(name: &str) -> Option<&str> {
    let env_name = name.strip_prefix("$env:")?;
    (!env_name.is_empty()
        && env_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')))
    .then_some(env_name)
}

fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '$'))
}

fn random_u64() -> u64 {
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    h.finish()
}

fn dynamic_value(name: &str) -> Option<String> {
    match name {
        "$uuid" => Some(uuid_v4()),
        "$timestamp" => Some(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()).to_string()),
        "$randomInt" => Some((random_u64() % 1000).to_string()),
        _ => None,
    }
}

fn uuid_v4() -> String {
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&random_u64().to_le_bytes());
    b[8..].copy_from_slice(&random_u64().to_le_bytes());
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> Vec<Variable> {
        vec![
            Variable { name: "host".into(), value: "localhost:3000".into(), secret: false, remember: false },
            Variable { name: "token".into(), value: "abc".into(), secret: true, remember: false },
            Variable { name: "empty".into(), value: String::new(), secret: false, remember: false },
        ]
    }

    fn resolve(text: &str) -> (String, Result<(), String>) {
        let v = vars();
        let mut r = Resolver::new(&v);
        let out = r.apply(text);
        (out, r.finish())
    }

    #[test]
    fn substitutes_defined_variables_including_repeats_and_whitespace() {
        let (out, res) = resolve("http://{{host}}/x?t={{ token }}&again={{host}}");
        assert_eq!(out, "http://localhost:3000/x?t=abc&again=localhost:3000");
        assert!(res.is_ok());
    }

    #[test]
    fn an_empty_value_still_counts_as_defined() {
        let (out, res) = resolve("a{{empty}}b");
        assert_eq!(out, "ab");
        assert!(res.is_ok());
    }

    #[test]
    fn undefined_variables_are_reported_once_each_and_left_in_place() {
        let (out, res) = resolve("{{nope}} {{host}} {{nope}} {{other}}");
        assert_eq!(out, "{{nope}} localhost:3000 {{nope}} {{other}}");
        let err = res.unwrap_err();
        assert!(err.contains("{{nope}}") && err.contains("{{other}}") && err.contains("variables"), "{err}");
        assert_eq!(err.matches("{{nope}}").count(), 1);
    }

    #[test]
    fn braces_that_are_not_variables_are_left_alone() {
        for text in ["{\"a\":{\"b\":1}}", "{{\"a\":1}}", "{{ not a var }}", "{{", "a }} b {{", "{{}}"] {
            let (out, res) = resolve(text);
            assert_eq!(out, text, "{text}");
            assert!(res.is_ok(), "{text}");
        }
    }

    #[test]
    fn dynamic_variables_generate_fresh_values() {
        let (out, res) = resolve("{{$uuid}}|{{$uuid}}|{{$timestamp}}|{{$randomInt}}");
        assert!(res.is_ok());
        let parts: Vec<&str> = out.split('|').collect();
        assert_ne!(parts[0], parts[1]);
        let u = parts[0];
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
        assert!(parts[2].parse::<u64>().unwrap() > 1_700_000_000);
        assert!(parts[3].parse::<u32>().unwrap() < 1000);
    }

    #[test]
    fn environment_variable_is_resolved_at_apply_time() {
        let expected = std::env::var("PATH").expect("test process should have PATH");
        let (out, res) = resolve("{{$env:PATH}}");
        assert_eq!(out, expected);
        assert!(res.is_ok());
    }

    #[test]
    fn missing_environment_variable_is_undefined() {
        let (_, res) = resolve("{{$env:PLUNGER_TEST_VARIABLE_THAT_DOES_NOT_EXIST_8F2A}}");
        assert!(res.unwrap_err().contains("{{$env:PLUNGER_TEST_VARIABLE_THAT_DOES_NOT_EXIST_8F2A}}"));
    }

    #[test]
    fn unknown_dynamic_variable_is_undefined() {
        let (_, res) = resolve("{{$nope}}");
        assert!(res.unwrap_err().contains("{{$nope}}"));
    }
}
