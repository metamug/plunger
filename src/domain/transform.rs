//! Reshaping data with a jq program (the jaq implementation): how a script turns the pages and
//! responses it collected into the small answer it returns. jq is a pure language: it has no files, no
//! network and no processes, and the functions that read the environment or the process input are left
//! out here, so a program can only compute on the data it is given.

use jaq_core::load::{Arena, File, Loader};
use jaq_core::{data, unwrap_valr, Compiler, Ctx, Vars};
use jaq_json::{read, Val};
use serde_json::Value;
use std::sync::mpsc;
use std::time::Duration;

/// Longest a program may run. A program that loops forever is abandoned after this.
const TIME_LIMIT: Duration = Duration::from_secs(10);
/// Most values one program may produce.
const MAX_OUTPUTS: usize = 10_000;
/// Names left out of the standard library because they read the process environment or its input. (`debug`
/// and `stderr` stay: they only write to standard error, and the library's own definitions build on them.)
const DENIED: &[&str] = &["env", "input", "inputs", "input_line_number", "get_search_list"];

fn compile_and_run(program: &str, input: Option<&Value>) -> Result<Vec<Value>, String> {
    let defs = jaq_core::defs().chain(jaq_std::defs()).chain(jaq_json::defs());
    let funs = jaq_core::funs().chain(jaq_std::funs()).chain(jaq_json::funs()).filter(|(name, _, _)| !DENIED.contains(name));
    let loader = Loader::new(defs);
    let arena = Arena::default();
    let modules = loader
        .load(&arena, File { code: program, path: () })
        .map_err(|e| format!("could not read the jq program: {}", describe(&format!("{e:?}"))))?;
    let filter = Compiler::default()
        .with_funs(funs)
        .compile(modules)
        .map_err(|e| format!("could not compile the jq program: {}", describe(&format!("{e:?}"))))?;
    let Some(input) = input else { return Ok(Vec::new()) };
    let text = serde_json::to_string(input).map_err(|e| e.to_string())?;
    let val = read::parse_single(text.as_bytes()).map_err(|e| format!("could not pass the data to jq: {e}"))?;
    let ctx = Ctx::<data::JustLut<Val>>::new(&filter.lut, Vars::new([]));
    let mut out = Vec::new();
    for result in filter.id.run((ctx, val)).map(unwrap_valr) {
        let val = result.map_err(|e| format!("the jq program failed: {e}"))?;
        let json = val.to_string();
        out.push(serde_json::from_str(&json).unwrap_or(Value::String(json)));
        if out.len() >= MAX_OUTPUTS {
            return Err(format!("the jq program produced more than {MAX_OUTPUTS} values; collect them into an array"));
        }
    }
    Ok(out)
}

/// The parser's report is a debug dump; keep it to one readable line.
fn describe(debug: &str) -> String {
    let flat: String = debug.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 300 {
        format!("{}...", flat.chars().take(300).collect::<String>())
    } else {
        flat
    }
}

/// Whether `program` is a valid jq program, so a mistake is found before anything is sent.
pub fn check(program: &str) -> Result<(), String> {
    compile_and_run(program, None).map(|_| ())
}

/// Runs `program` on `input`. One output is returned as it is; several are returned as an array.
pub fn run(program: &str, input: &Value) -> Result<Value, String> {
    let (tx, rx) = mpsc::channel();
    let program = program.to_string();
    let input = input.clone();
    std::thread::spawn(move || {
        let _ = tx.send(compile_and_run(&program, Some(&input)));
    });
    let mut outputs = match rx.recv_timeout(TIME_LIMIT) {
        Ok(result) => result?,
        Err(_) => return Err(format!("the jq program ran for more than {} seconds and was stopped", TIME_LIMIT.as_secs())),
    };
    Ok(match outputs.len() {
        0 => Value::Null,
        1 => outputs.remove(0),
        _ => Value::Array(outputs),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_program_reshapes_the_data() {
        let data = json!({"issues": [
            {"key": "A-1", "points": 3, "who": "ada"},
            {"key": "A-2", "points": 5, "who": "bob"},
            {"key": "A-3", "points": 2, "who": "ada"}
        ]});
        let out = run(
            "{count: (.issues|length), points: ([.issues[].points]|add), by_who: (.issues | group_by(.who) | map({(.[0].who): length}) | add), keys: [.issues[].key]}",
            &data,
        )
        .unwrap();
        assert_eq!(out, json!({"count": 3, "points": 10, "by_who": {"ada": 2, "bob": 1}, "keys": ["A-1", "A-2", "A-3"]}));
    }

    #[test]
    fn several_outputs_become_an_array_and_none_is_null() {
        assert_eq!(run(".[]", &json!([1, 2])).unwrap(), json!([1, 2]));
        assert_eq!(run("empty", &json!(1)).unwrap(), Value::Null);
        assert_eq!(run(".a // \"none\"", &json!({})).unwrap(), json!("none"));
    }

    #[test]
    fn a_bad_program_is_found_before_running() {
        assert!(check(".a | map(").unwrap_err().contains("jq program"));
        assert!(check("nosuchfunction(1)").is_err());
        assert!(check(".a | length").is_ok());
    }

    #[test]
    fn a_failing_program_says_so() {
        let err = run(".a | error(\"boom\")", &json!({"a": 1})).unwrap_err();
        assert!(err.contains("failed") && err.contains("boom"), "{err}");
    }

    #[test]
    fn the_environment_and_the_terminal_are_out_of_reach() {
        for program in ["env", "$ENV", "input", "inputs"] {
            assert!(run(program, &json!(1)).is_err(), "{program} should not run");
        }
    }

    #[test]
    fn strings_numbers_and_nulls_keep_their_types() {
        let out = run("{s: .s, n: .n, f: .f, z: .z, t: (.s|test(\"a.c\"))}", &json!({"s": "abc", "n": 5, "f": 1.5, "z": null})).unwrap();
        assert_eq!(out, json!({"s": "abc", "n": 5, "f": 1.5, "z": null, "t": true}));
    }
}
