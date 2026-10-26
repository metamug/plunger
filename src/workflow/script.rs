//! What `plunger run` reads: a script file of steps, an environment file, and a data file with one row per
//! iteration.

use super::Step;
use serde::Deserialize;
use std::collections::BTreeMap;

/// A script: the steps, and optionally a name and variables to run them with. The file is either a JSON
/// array of steps (the same as a saved workflow) or `{"name": ..., "variables": {...}, "steps": [...]}`.
#[derive(Debug)]
pub struct Script {
    pub name: Option<String>,
    pub variables: BTreeMap<String, String>,
    pub steps: Vec<Step>,
}

#[derive(Deserialize)]
struct Document {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    variables: BTreeMap<String, serde_json::Value>,
    steps: Vec<Step>,
}

/// A value as a variable's text: a string as it is, anything else as its JSON.
fn text_of(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The fields a step can have.
fn step_fields() -> Vec<String> {
    let schema = serde_json::to_value(rmcp::schemars::schema_for!(Step)).unwrap_or_default();
    schema.get("properties").and_then(|p| p.as_object()).map(|p| p.keys().cloned().collect()).unwrap_or_default()
}

/// How many single-character edits turn `a` into `b`.
fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { prev } else { prev.min(row[j]).min(row[j + 1]) + 1 };
            prev = row[j + 1];
            row[j + 1] = cost;
        }
    }
    row[b.len()]
}

/// A step field that does not exist is usually a typo (`paginat`, `foreach`), and would otherwise be ignored
/// without a word, so the step would quietly do less than it says.
pub fn unknown_step_fields(steps: &[serde_json::Value]) -> Result<(), String> {
    let known = step_fields();
    if known.is_empty() {
        return Ok(());
    }
    for (i, step) in steps.iter().enumerate() {
        let Some(object) = step.as_object() else { return Err(format!("Step {} is not an object", i + 1)) };
        if let Some(key) = object.keys().find(|k| !known.contains(k)) {
            let near = known.iter().min_by_key(|k| distance(key, k)).filter(|k| distance(key, k) <= 3);
            return Err(match near {
                Some(near) => format!("Step {}: unknown field `{key}`. Did you mean `{near}`?", i + 1),
                None => format!("Step {}: unknown field `{key}`.", i + 1),
            });
        }
    }
    Ok(())
}

pub fn parse_script(text: &str) -> Result<Script, String> {
    let value: serde_json::Value =
        serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("The script isn't valid JSON: {e}"))?;
    let listed = match &value {
        serde_json::Value::Array(steps) => Some(steps.as_slice()),
        other => other.get("steps").and_then(|s| s.as_array()).map(Vec::as_slice),
    };
    if let Some(steps) = listed {
        unknown_step_fields(steps)?;
    }
    if value.is_array() {
        let steps = serde_json::from_value(value).map_err(|e| format!("The steps aren't valid: {e}"))?;
        return Ok(Script { name: None, variables: BTreeMap::new(), steps });
    }
    let doc: Document =
        serde_json::from_value(value).map_err(|e| format!("The script needs a `steps` list (or be a list of steps): {e}"))?;
    Ok(Script { name: doc.name, variables: doc.variables.iter().map(|(k, v)| (k.clone(), text_of(v))).collect(), steps: doc.steps })
}

/// An environment file: a JSON object, or `NAME=value` lines (`#` starts a comment; a value may be in quotes).
pub fn parse_env(text: &str) -> Result<BTreeMap<String, String>, String> {
    let trimmed = text.trim_start_matches('\u{feff}').trim();
    if trimmed.starts_with('{') {
        let map: BTreeMap<String, serde_json::Value> =
            serde_json::from_str(trimmed).map_err(|e| format!("The environment file isn't valid JSON: {e}"))?;
        return Ok(map.iter().map(|(k, v)| (k.clone(), text_of(v))).collect());
    }
    let mut out = BTreeMap::new();
    for (n, line) in trimmed.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let (name, value) = line.split_once('=').ok_or_else(|| format!("Line {} of the environment file isn't NAME=value", n + 1))?;
        let value = value.trim();
        let value = [('"', '"'), ('\'', '\'')]
            .iter()
            .find_map(|(open, close)| value.strip_prefix(*open).and_then(|v| v.strip_suffix(*close)))
            .unwrap_or(value);
        out.insert(name.trim().to_string(), value.to_string());
    }
    Ok(out)
}

/// A data file: a JSON array of objects, or CSV with a header row. Each row is one run of the script, its
/// columns the variables of that run.
pub fn parse_data(text: &str) -> Result<Vec<BTreeMap<String, String>>, String> {
    let trimmed = text.trim_start_matches('\u{feff}').trim();
    if trimmed.starts_with('[') {
        let rows: Vec<BTreeMap<String, serde_json::Value>> =
            serde_json::from_str(trimmed).map_err(|e| format!("The data file should be a JSON array of objects: {e}"))?;
        return Ok(rows.iter().map(|row| row.iter().map(|(k, v)| (k.clone(), text_of(v))).collect()).collect());
    }
    let mut records = csv_records(trimmed).into_iter();
    let header = records.next().ok_or("The data file is empty")?;
    let mut rows = Vec::new();
    for (n, record) in records.enumerate() {
        if record.len() != header.len() {
            return Err(format!("Row {} of the data file has {} columns; the header has {}", n + 1, record.len(), header.len()));
        }
        rows.push(header.iter().cloned().zip(record).collect());
    }
    Ok(rows)
}

/// CSV records, with "quoted, fields" and "" for a quote inside one. Blank lines are skipped.
fn csv_records(text: &str) -> Vec<Vec<String>> {
    let mut records = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            (true, '"') => quoted = false,
            (true, c) => field.push(c),
            (false, '"') if field.is_empty() => quoted = true,
            (false, ',') => record.push(std::mem::take(&mut field).trim().to_string()),
            (false, '\r') => {}
            (false, '\n') => {
                record.push(std::mem::take(&mut field).trim().to_string());
                if record.iter().any(|f| !f.is_empty()) {
                    records.push(std::mem::take(&mut record));
                } else {
                    record.clear();
                }
            }
            (false, c) => field.push(c),
        }
    }
    record.push(field.trim().to_string());
    if record.iter().any(|f| !f.is_empty()) {
        records.push(record);
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_is_a_list_of_steps_or_a_document() {
        let list = parse_script(r#"[{"url": "http://h/a"}]"#).unwrap();
        assert_eq!(list.steps.len(), 1);
        assert!(list.name.is_none());
        let doc = parse_script(r#"{"name": "smoke", "variables": {"base": "http://h", "n": 3}, "steps": [{"url": "{{base}}/a"}]}"#).unwrap();
        assert_eq!(doc.name.as_deref(), Some("smoke"));
        assert_eq!(doc.variables["n"], "3");
        assert!(parse_script("{}").unwrap_err().contains("steps"));
        assert!(parse_script("nope").unwrap_err().contains("JSON"));
    }

    #[test]
    fn a_misspelt_field_is_named_instead_of_ignored() {
        let err = parse_script(r#"[{"url": "http://h/a", "paginat": {}}]"#).unwrap_err();
        assert_eq!(err, "Step 1: unknown field `paginat`. Did you mean `paginate`?");
        let err = parse_script(r#"{"steps": [{"url": "http://h/a"}, {"url": "http://h/b", "foreach": "x"}]}"#).unwrap_err();
        assert!(err.contains("Step 2") && err.contains("for_each"), "{err}");
        assert!(parse_script(r#"[{"url": "http://h/a", "method": "POST", "json": {}, "asserts": [], "return": "."}]"#).is_ok());
        assert!(parse_script(r#"[{"url": "http://h/a", "paginate": {"stile": "page"}}]"#).unwrap_err().contains("stile"));
        assert!(parse_script(r#"[{"url": "http://h/a", "asserts": [{"from": "status", "equal": 1}]}]"#).unwrap_err().contains("equal"));
    }

    #[test]
    fn an_environment_file_is_json_or_name_value_lines() {
        let env = parse_env("# staging\nBASE=http://h\nexport KEY=\"a b\"\n\nQ='x'\n").unwrap();
        assert_eq!(env["BASE"], "http://h");
        assert_eq!(env["KEY"], "a b");
        assert_eq!(env["Q"], "x");
        assert_eq!(parse_env(r#"{"base": "http://h", "n": 1}"#).unwrap()["n"], "1");
        assert!(parse_env("oops").unwrap_err().contains("Line 1"));
    }

    #[test]
    fn data_rows_come_from_csv_or_json() {
        let rows = parse_data("id,name\n1,Ada\n2,\"Lovelace, A\"\n\n").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1]["name"], "Lovelace, A");
        let rows = parse_data(r#"[{"id": 1}, {"id": "two"}]"#).unwrap();
        assert_eq!(rows[1]["id"], "two");
        assert!(parse_data("a,b\n1\n").unwrap_err().contains("1 columns"));
        assert!(parse_data("").is_err());
    }
}
