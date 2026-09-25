//! Clickable suggestion chips under a text field: header names, content types,
//! and `{{variables}}`. Picking one fills the field.

use crate::model::Variable;
use eframe::egui::{self, text::{CCursor, CCursorRange}};

const MAX_CHIPS: usize = 6;

/// Variables that are always available, offered after the ones the user defined.
const BUILT_INS: [&str; 3] = ["{{$uuid}}", "{{$timestamp}}", "{{$randomInt}}"];

/// Candidates that start with what has been typed (ignoring case), leaving out
/// an exact match since there is nothing left to complete.
pub(super) fn matching<'a>(typed: &str, candidates: &[&'a str]) -> Vec<&'a str> {
    let typed = typed.to_lowercase();
    candidates
        .iter()
        .filter(|c| {
            let c = c.to_lowercase();
            c.starts_with(&typed) && c != typed
        })
        .take(MAX_CHIPS)
        .copied()
        .collect()
}

/// `{{name}}` for every named variable, then the built-ins.
pub(super) fn variable_candidates(vars: &[Variable]) -> Vec<String> {
    vars.iter()
        .filter(|v| !v.name.trim().is_empty())
        .map(|v| format!("{{{{{}}}}}", v.name.trim()))
        .chain(BUILT_INS.iter().map(|b| b.to_string()))
        .collect()
}

/// Chips for `{{variables}}`, offered once the value starts with `{{` so they
/// stay out of the way of ordinary values.
pub(super) fn variable_chips(ui: &mut egui::Ui, field: &egui::Response, target: &mut String, vars: &[Variable]) {
    if !target.starts_with("{{") {
        return;
    }
    let owned = variable_candidates(vars);
    let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
    chips(ui, field, target, &refs);
}

/// A row of small buttons for the candidates matching `target`, shown while
/// `field` has focus. Clicking one replaces the text, puts the cursor at the end
/// and gives focus back to the field.
///
/// Pressing a button takes focus off the field, so the chips would vanish before
/// the click completes. To avoid that they stay for as long as a press that
/// started on them is still down.
pub(super) fn chips(ui: &mut egui::Ui, field: &egui::Response, target: &mut String, candidates: &[&str]) {
    let memory_id = field.id.with("suggestion-chips");
    let last_rect: Option<egui::Rect> = ui.data(|d| d.get_temp(memory_id));
    let held = last_rect.is_some_and(|rect| {
        ui.input(|i| {
            (i.pointer.primary_down() || i.pointer.primary_released())
                && i.pointer.interact_pos().is_some_and(|p| rect.contains(p))
        })
    });

    let matches = matching(target, candidates);
    if matches.is_empty() || !(field.has_focus() || held) {
        ui.data_mut(|d| d.remove::<egui::Rect>(memory_id));
        return;
    }

    let mut picked = None;
    let row = ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new("suggestions:").weak().small());
        for m in matches {
            if ui.small_button(m).clicked() {
                picked = Some(m);
            }
        }
    });
    ui.data_mut(|d| d.insert_temp(memory_id, row.response.rect));

    if let Some(text) = picked {
        *target = text.to_string();
        let ctx = ui.ctx().clone();
        ctx.memory_mut(|m| m.request_focus(field.id));
        let mut state = egui::text_edit::TextEditState::load(&ctx, field.id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(text.chars().count()))));
        state.store(&ctx, field.id);
        ui.data_mut(|d| d.remove::<egui::Rect>(memory_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADERS: &[&str] = &["Accept", "Accept-Encoding", "Content-Type", "Cookie"];

    #[test]
    fn suggestions_are_a_case_insensitive_prefix_match() {
        assert_eq!(matching("acc", HEADERS), vec!["Accept", "Accept-Encoding"]);
        assert_eq!(matching("CO", HEADERS), vec!["Content-Type", "Cookie"]);
    }

    #[test]
    fn an_empty_field_offers_everything_up_to_the_limit() {
        let many: Vec<&str> = vec!["a", "b", "c", "d", "e", "f", "g", "h"];
        assert_eq!(matching("", &many).len(), MAX_CHIPS);
    }

    #[test]
    fn a_finished_word_is_not_suggested_back() {
        assert_eq!(matching("accept", HEADERS), vec!["Accept-Encoding"]);
        assert!(matching("cookie", HEADERS).is_empty());
    }

    #[test]
    fn text_that_matches_nothing_gets_no_suggestions() {
        assert!(matching("zzz", HEADERS).is_empty());
    }

    #[test]
    fn variable_suggestions_list_defined_names_first_then_the_built_ins() {
        let vars = vec![
            Variable { name: "base".into(), ..Default::default() },
            Variable { name: "  ".into(), ..Default::default() },
            Variable { name: "token".into(), ..Default::default() },
        ];
        assert_eq!(
            variable_candidates(&vars),
            vec!["{{base}}", "{{token}}", "{{$uuid}}", "{{$timestamp}}", "{{$randomInt}}"]
        );
    }

    #[test]
    fn typing_the_start_of_a_variable_narrows_the_choices() {
        let owned = variable_candidates(&[Variable { name: "base".into(), ..Default::default() }]);
        let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
        assert_eq!(matching("{{", &refs).len(), 4);
        assert_eq!(matching("{{$t", &refs), vec!["{{$timestamp}}"]);
        assert_eq!(matching("{{b", &refs), vec!["{{base}}"]);
    }
}
