//! A run as JUnit XML, the format CI systems (GitHub Actions, Jenkins, GitLab) turn into a test report:
//! one test suite per run, one test case per step.

use super::WorkflowResult;

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 has no place for most control characters
            c if (c as u32) < 0x20 && !matches!(c, '\n' | '\r' | '\t') => {}
            c => out.push(c),
        }
    }
    out
}

/// One `<testsuite>` for a run.
fn suite(run: &WorkflowResult, name: &str) -> String {
    let failures = run.steps.iter().filter(|s| !s.ok).count();
    let skipped = run.steps.iter().filter(|s| s.skipped).count();
    let seconds: f64 = run.steps.iter().map(|s| s.elapsed_ms.unwrap_or(0) as f64 / 1000.0).sum();
    let mut out = format!(
        "  <testsuite name=\"{}\" tests=\"{}\" failures=\"{failures}\" skipped=\"{skipped}\" time=\"{seconds:.3}\">\n",
        escape(name),
        run.steps.len()
    );
    for step in &run.steps {
        let case = format!("{}. {}", step.step, step.label);
        let time = step.elapsed_ms.unwrap_or(0) as f64 / 1000.0;
        out.push_str(&format!("    <testcase classname=\"{}\" name=\"{}\" time=\"{time:.3}\"", escape(name), escape(&case)));
        if step.skipped {
            out.push_str(">\n      <skipped/>\n    </testcase>\n");
        } else if !step.ok {
            let message = step.error.as_deref().unwrap_or("failed");
            let mut detail = String::new();
            for a in step.asserts.iter().filter(|a| !a.ok) {
                detail.push_str(&format!("{}: {}\n", a.name, a.message.as_deref().unwrap_or("failed")));
            }
            if detail.is_empty() {
                detail = message.to_string();
            }
            out.push_str(&format!(
                ">\n      <failure message=\"{}\">{}</failure>\n    </testcase>\n",
                escape(message),
                escape(detail.trim_end())
            ));
        } else {
            out.push_str("/>\n");
        }
    }
    out.push_str("  </testsuite>\n");
    out
}

/// The runs (a name for each: the script, or the script plus the data row) as a `<testsuites>` document.
pub fn to_junit(runs: &[(String, &WorkflowResult)]) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites>\n");
    for (name, run) in runs {
        out.push_str(&suite(run, name));
    }
    out.push_str("</testsuites>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::{AssertOutcome, StepResult};

    fn step(n: usize, label: &str, ok: bool) -> StepResult {
        StepResult { step: n, label: label.into(), ok, skipped: false, status: Some(200), elapsed_ms: Some(250), set: vec![], asserts: vec![], error: None }
    }

    #[test]
    fn a_run_becomes_cases_with_failures_and_skips() {
        let mut bad = step(2, "get <me>", false);
        bad.error = Some("1 of 1 asserts failed: $.id == 8: expected 8, got 7".into());
        bad.asserts = vec![AssertOutcome { name: "$.id == 8".into(), ok: false, message: Some("expected 8, got 7".into()) }];
        let mut skipped = step(3, "cleanup", true);
        skipped.skipped = true;
        let run = WorkflowResult {
            workflow: "w".into(),
            ok: false,
            cancelled: false,
            cleaned_up: vec![],
            stopped_early: false,
            steps_run: 3,
            steps: vec![step(1, "log in", true), bad, skipped],
            last_response: None,
        };
        let xml = to_junit(&[("w".to_string(), &run)]);
        assert!(xml.starts_with("<?xml"), "{xml}");
        assert!(xml.contains("tests=\"3\" failures=\"1\" skipped=\"1\" time=\"0.750\""), "{xml}");
        assert!(xml.contains("<testcase classname=\"w\" name=\"1. log in\" time=\"0.250\"/>"), "{xml}");
        assert!(xml.contains("name=\"2. get &lt;me&gt;\""), "{xml}");
        assert!(xml.contains("<failure message=\"1 of 1 asserts failed"), "{xml}");
        assert!(xml.contains(">$.id == 8: expected 8, got 7</failure>"), "{xml}");
        assert!(xml.contains("<skipped/>"), "{xml}");
    }

    #[test]
    fn markup_and_control_characters_are_escaped() {
        assert_eq!(escape("a&b<c>\"d\"'e'\u{1}\n"), "a&amp;b&lt;c&gt;&quot;d&quot;&apos;e&apos;\n");
    }
}
