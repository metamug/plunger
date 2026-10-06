//! Syntax highlighting for the places Plunger shows code: pasted and exported commands, request
//! fields that hold `{{variables}}`, and request bodies. Hand-written and small on purpose (no
//! highlighting engine), each function returns a `LayoutJob` that draws exactly the text it was given.

use crate::model::Variable;
use crate::theme::palette;
use eframe::egui::{self, text::LayoutJob, Color32, FontId, TextFormat};
use std::sync::Arc;

fn append(job: &mut LayoutJob, text: &str, font: &FontId, color: Color32) {
    if !text.is_empty() {
        job.append(text, 0.0, TextFormat { font_id: font.clone(), color, ..Default::default() });
    }
}

/// Adds `text` in `color`, with each `{{placeholder}}` in the placeholder colour.
fn append_with_placeholders(job: &mut LayoutJob, text: &str, font: &FontId, color: Color32, placeholder: Color32) {
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let Some(len) = rest[start..].find("}}") else { break };
        append(job, &rest[..start], font, color);
        append(job, &rest[start..start + len + 2], font, placeholder);
        rest = &rest[start + len + 2..];
    }
    append(job, rest, font, color);
}

const COMMAND_WORDS: &[&str] = &["curl", "curl.exe", "invoke-restmethod", "invoke-webrequest", "irm", "iwr", "plunger", "new-object"];

/// A shell command in any of the supported dialects: bash curl, Windows cmd curl, PowerShell. The
/// command name, `-flags`, quoted strings, `{{placeholders}}`, `$variables`, numbers and the
/// punctuation that continues or structures a command each get their own colour.
pub fn command(text: &str) -> LayoutJob {
    let [punct, flag, string, number, variable, default] = palette().json;
    let (accent, placeholder) = (palette().accent_text, palette().amber);
    let comment = default.gamma_multiply(0.6);
    let font = FontId::monospace(13.0);
    let mut job = LayoutJob::default();

    let mut i = 0;
    let at = |i: usize| text[i..].chars().next();
    let is_word_start = |i: usize| i == 0 || text[..i].chars().next_back().is_some_and(|c| c.is_whitespace());
    while let Some(c) = at(i) {
        let rest = &text[i..];
        let next_start = if c.is_whitespace() {
            let end = rest.find(|ch: char| !ch.is_whitespace()).map_or(text.len(), |n| i + n);
            append(&mut job, &text[i..end], &font, default);
            end
        } else if c == '#' && is_word_start(i) {
            let end = rest.find('\n').map_or(text.len(), |n| i + n);
            append(&mut job, &text[i..end], &font, comment);
            end
        } else if c == '\'' || c == '"' {
            // To the closing quote, skipping an escaped one (\" in bash and cmd, `" in PowerShell).
            let mut end = text.len();
            let mut chars = text[i + 1..].char_indices();
            while let Some((n, ch)) = chars.next() {
                if (ch == '\\' && c == '"') || (ch == '`' && c == '"') {
                    chars.next();
                } else if ch == c {
                    end = i + 1 + n + ch.len_utf8();
                    break;
                }
            }
            append_with_placeholders(&mut job, &text[i..end], &font, string, placeholder);
            end
        } else if c == '-' && at(i + 1).is_some_and(|n| n.is_ascii_alphabetic() || n == '-') && is_word_start(i) {
            let end = rest[1..].find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')).map_or(text.len(), |n| i + 1 + n);
            append(&mut job, &text[i..end], &font, flag);
            end
        } else if c == '$' {
            let end = rest[1..].find(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | ':' | '.'))).map_or(text.len(), |n| i + 1 + n);
            append(&mut job, &text[i..end], &font, variable);
            end
        } else if rest.starts_with("{{") && rest.contains("}}") {
            let end = i + rest.find("}}").unwrap_or(0) + 2;
            append(&mut job, &text[i..end], &font, placeholder);
            end
        } else if matches!(c, '@' | '{' | '}' | '(' | ')' | ';' | '=' | '^' | '`' | '\\' | '|' | '&' | ',') {
            append(&mut job, &text[i..i + c.len_utf8()], &font, punct);
            i + c.len_utf8()
        } else {
            // A bare word: a URL, a command name, a number, a method.
            let end = rest.find(|ch: char| ch.is_whitespace() || matches!(ch, '\'' | '"' | ';' | '{' | '}' | '(' | ')' | '^' | '`' | '|' | ',')).map_or(text.len(), |n| i + n);
            let end = if end == i { i + c.len_utf8() } else { end };
            let word = &text[i..end];
            let lower = word.to_ascii_lowercase();
            if COMMAND_WORDS.contains(&lower.as_str()) {
                append(&mut job, word, &font, accent);
            } else if lower.starts_with("http://") || lower.starts_with("https://") {
                append_with_placeholders(&mut job, word, &font, string, placeholder);
            } else if word.chars().all(|ch| ch.is_ascii_digit() || ch == '.') {
                append(&mut job, word, &font, number);
            } else {
                append(&mut job, word, &font, default);
            }
            end
        };
        i = next_start;
    }
    job
}

/// The names of variables agents set. They are defined for every request, so wherever
/// `{{variables}}` are coloured they count as defined even though the tab does not hold them.
static AGENT_VARIABLES: std::sync::RwLock<Vec<String>> = std::sync::RwLock::new(Vec::new());

pub fn set_agent_variable_names(names: Vec<String>) {
    if let Ok(mut guard) = AGENT_VARIABLES.write() {
        *guard = names;
    }
}

/// Whether `{{name}}` would be filled in when the request is sent.
pub fn variable_is_defined(name: &str, variables: &[Variable]) -> bool {
    let name = name.trim();
    if AGENT_VARIABLES.read().is_ok_and(|names| names.iter().any(|n| n == name)) {
        return true;
    }
    if let Some(var) = name.strip_prefix("$env:") {
        return std::env::var_os(var).is_some();
    }
    matches!(name, "$uuid" | "$timestamp" | "$randomInt") || variables.iter().any(|v| v.name.trim() == name && !name.is_empty())
}

/// Text with each `{{variable}}` coloured by whether it is defined (accent) or not (red, the
/// request would be refused).
pub fn variables(text: &str, base: Color32, font: &FontId, variables: &[Variable]) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let Some(len) = rest[start..].find("}}") else { break };
        let token = &rest[start..start + len + 2];
        append(&mut job, &rest[..start], font, base);
        let color = if variable_is_defined(&token[2..token.len() - 2], variables) { palette().accent_text } else { palette().error };
        append(&mut job, token, font, color);
        rest = &rest[start + len + 2..];
    }
    append(&mut job, rest, font, base);
    job
}

/// A layouter for a single-line field that holds `{{variables}}`:
/// `ui.add(theme::field(text).layouter(&mut highlight::variable_layouter(&vars)))`.
pub fn variable_layouter(vars: &[Variable]) -> impl FnMut(&egui::Ui, &str, f32) -> Arc<egui::Galley> + '_ {
    move |ui, text, wrap_width| {
        let font = egui::TextStyle::Body.resolve(ui.style());
        let mut job = variables(text, ui.visuals().text_color(), &font, vars);
        job.wrap.max_width = wrap_width;
        ui.fonts(|f| f.layout_job(job))
    }
}

/// `Name: value` lines: names in the key colour, `{{variables}}` as in [`variables`].
pub fn header_lines(text: &str, vars: &[Variable]) -> LayoutJob {
    let [punct, key, _, _, _, default] = palette().json;
    let font = FontId::monospace(13.0);
    let mut job = LayoutJob::default();
    for (n, line) in text.split('\n').enumerate() {
        if n > 0 {
            append(&mut job, "\n", &font, default);
        }
        match line.split_once(':') {
            Some((name, value)) => {
                append(&mut job, name, &font, key);
                append(&mut job, ":", &font, punct);
                let colored = variables(value, default, &font, vars);
                for s in &colored.sections {
                    append(&mut job, &colored.text[s.byte_range.clone()], &font, s.format.color);
                }
            }
            None => append(&mut job, line, &font, default),
        }
    }
    job
}

/// `key=value` lines or an `a=1&b=2` string: keys, `=` and `&` apart from values.
pub fn form_body(text: &str) -> LayoutJob {
    let [punct, key, string, _, _, default] = palette().json;
    let placeholder = palette().amber;
    let font = FontId::monospace(13.0);
    let mut job = LayoutJob::default();
    let mut rest = text;
    while !rest.is_empty() {
        let end = rest.find(['\n', '&']).unwrap_or(rest.len());
        let (pair, tail) = rest.split_at(end);
        match pair.split_once('=') {
            Some((k, v)) => {
                append(&mut job, k, &font, key);
                append(&mut job, "=", &font, punct);
                append_with_placeholders(&mut job, v, &font, string, placeholder);
            }
            None => append(&mut job, pair, &font, default),
        }
        let sep_len = tail.chars().next().map_or(0, char::len_utf8);
        append(&mut job, &tail[..sep_len], &font, punct);
        rest = &tail[sep_len..];
    }
    job
}

/// Colour for each stretch of a pretty-printed XML/HTML document: tags apart from text. The
/// stretches cover the whole text, even an unterminated tag at the end.
pub fn markup_segments(text: &str) -> Vec<(usize, usize, egui::Color32)> {
    let colors = palette().json;
    let mut segments = Vec::new();
    let mut pos = 0;
    while let Some(rel) = text[pos..].find('<') {
        let start = pos + rel;
        if start > pos {
            segments.push((pos, start, colors[2]));
        }
        let Some(end_rel) = text[start..].find('>') else {
            segments.push((start, text.len(), colors[2]));
            return segments;
        };
        let end = start + end_rel + 1;
        segments.push((start, end, colors[1]));
        pos = end;
    }
    if pos < text.len() {
        segments.push((pos, text.len(), colors[2]));
    }
    segments
}

/// XML or HTML, tags apart from text.
pub fn markup(text: &str) -> LayoutJob {
    let font = FontId::monospace(13.0);
    let mut job = LayoutJob::default();
    for (start, end, color) in markup_segments(text) {
        append(&mut job, &text[start..end], &font, color);
    }
    job
}

/// A body of unknown type: JSON when it looks like JSON, markup when it looks like XML or HTML,
/// a form when it is `key=value`, otherwise plain text.
pub fn body(text: &str) -> LayoutJob {
    let trimmed = text.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        crate::json_view::highlight_json(text)
    } else if trimmed.starts_with('<') {
        markup(text)
    } else if !trimmed.contains('\n') && trimmed.contains('=') && !trimmed.contains(' ') {
        form_body(text)
    } else {
        let mut job = LayoutJob::default();
        append(&mut job, text, &FontId::monospace(13.0), palette().json[5]);
        job
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pieces(job: &LayoutJob) -> Vec<(String, Color32)> {
        job.sections.iter().map(|s| (job.text[s.byte_range.clone()].to_string(), s.format.color)).collect()
    }

    fn color_of(job: &LayoutJob, piece: &str) -> Color32 {
        pieces(job).into_iter().find(|(t, _)| t == piece).unwrap_or_else(|| panic!("no piece {piece:?} in {:?}", pieces(job))).1
    }

    const SAMPLES: &[&str] = &[
        "",
        "curl 'https://h/x?a=1' -H 'A: it'\\''s' -d '{\"a\":1}' -L",
        "curl \"https://h\" ^\r\n  -H \"accept: x\" ^\r\n  --data-raw \"^{^\\^\"a^\\^\":1^}\"",
        "Invoke-RestMethod -Uri 'https://h/it''s' `\n  -Headers @{ 'A' = 'b'; 'C' = {{token}} } `\n  -Body \"x`\"y\" # note",
        "$session.UserAgent = \"ü ünï ✓ 日本語\"\nirm https://h/{{id}}?a=1&b=2 -Method Put",
        "unterminated 'string and \"another",
        "-1 --x -- - $ $$ @ {{ }} {{a}",
        "tab\tseparated\n\n\n  trailing  ",
    ];

    #[test]
    fn highlighting_never_changes_the_text_it_draws() {
        for sample in SAMPLES {
            for job in [command(sample), form_body(sample), body(sample), header_lines(sample, &[])] {
                assert_eq!(job.text, *sample);
                let joined: String = job.sections.iter().map(|s| &job.text[s.byte_range.clone()]).collect();
                assert_eq!(joined, *sample, "sections do not cover {sample:?}");
            }
            let font = FontId::monospace(13.0);
            let job = variables(sample, Color32::WHITE, &font, &[]);
            let joined: String = job.sections.iter().map(|s| &job.text[s.byte_range.clone()]).collect();
            assert_eq!(joined, *sample);
        }
    }

    #[test]
    fn a_command_is_coloured_by_what_each_part_is() {
        let [_, flag, string, number, variable, _] = palette().json;
        let job = command("curl -X POST 'https://h/{{id}}' --max-time 5 $name # done");
        assert_eq!(color_of(&job, "curl"), palette().accent_text);
        assert_eq!(color_of(&job, "-X"), flag);
        assert_eq!(color_of(&job, "--max-time"), flag);
        assert_eq!(color_of(&job, "'https://h/"), string);
        assert_eq!(color_of(&job, "{{id}}"), palette().amber);
        assert_eq!(color_of(&job, "5"), number);
        assert_eq!(color_of(&job, "$name"), variable);
        assert_ne!(color_of(&job, "# done"), palette().json[5]);
    }

    #[test]
    fn variables_show_whether_they_will_be_filled_in() {
        let vars = vec![Variable { name: "base".into(), value: String::new(), secret: false, remember: false }];
        let font = FontId::monospace(13.0);
        let job = variables("{{base}}/x/{{missing}}/{{$uuid}}/{{ base }}", Color32::WHITE, &font, &vars);
        let pieces = pieces(&job);
        let color = |t: &str| pieces.iter().find(|(p, _)| p == t).unwrap().1;
        assert_eq!(color("{{base}}"), palette().accent_text);
        assert_eq!(color("{{missing}}"), palette().error);
        assert_eq!(color("{{$uuid}}"), palette().accent_text);
        assert_eq!(color("{{ base }}"), palette().accent_text);
        assert!(!variable_is_defined("", &vars));
    }

    #[test]
    fn a_body_is_highlighted_by_what_it_looks_like() {
        let json_color = |t: &str| pieces(&body(t)).iter().map(|(_, c)| *c).collect::<std::collections::HashSet<_>>().len();
        assert!(json_color("{\"a\": [1, true]}") > 3, "JSON uses several colours");
        assert!(json_color("a=1&b=2") >= 3, "a form shows keys, separators and values");
        assert_eq!(json_color("just some words"), 1, "plain text is one colour");
    }
}
