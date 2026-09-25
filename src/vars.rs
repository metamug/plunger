//! `{{name}}` variable substitution, plus a few dynamic built-ins
//! (`{{$uuid}}`, `{{$timestamp}}`, `{{$randomInt}}`).

use crate::model::Variable;
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
            let name = after[..end].trim();
            match self.lookup(name) {
                Some(value) => out.push_str(&encode(&value)),
                None => {
                    if is_valid_name(name) {
                        self.undefined.insert(name.to_string());
                    }
                    // Undefined or not a variable at all (e.g. `{{"a":1}}`): leave untouched.
                    out.push_str(&rest[start..start + token_len]);
                }
            }
            rest = &rest[start + token_len..];
        }
        out.push_str(rest);
        out
    }

    fn lookup(&self, name: &str) -> Option<String> {
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
            "Undefined variable{}: {}. Define {} in the Variables tab.",
            if names.len() == 1 { "" } else { "s" },
            names.join(", "),
            if names.len() == 1 { "it" } else { "them" },
        ))
    }
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
    fn unknown_dynamic_variable_is_undefined() {
        let (_, res) = resolve("{{$nope}}");
        assert!(res.unwrap_err().contains("{{$nope}}"));
    }
}
