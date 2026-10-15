//! Conditions on a workflow step (`skip_if`, `stop_if`) once their variables are filled in.

/// Whether a condition, with its variables already filled in, is true: `a == b`, `a != b`, or one value
/// that is true unless it is empty, `false`, `0` or `null`. Quotes around either side are ignored.
pub fn condition_holds(text: &str) -> bool {
    fn side(s: &str) -> &str {
        let s = s.trim();
        s.strip_prefix('\'')
            .and_then(|r| r.strip_suffix('\''))
            .or_else(|| s.strip_prefix('"').and_then(|r| r.strip_suffix('"')))
            .unwrap_or(s)
    }
    if let Some((left, right)) = text.split_once("!=") {
        return side(left) != side(right);
    }
    if let Some((left, right)) = text.split_once("==") {
        return side(left) == side(right);
    }
    let value = side(text);
    !(value.is_empty() || value.eq_ignore_ascii_case("false") || value == "0" || value.eq_ignore_ascii_case("null"))
}
