//! Shared behaviour for the "one editable line per item, plus a spare blank
//! line at the end" lists (headers, params, variables, form-data fields).

use crate::icons::{self, Icon};
use eframe::egui;

/// Removes the row at `remove` (if any), then makes sure the list ends with
/// exactly one blank row to type into.
pub(super) fn tidy_rows<T>(
    rows: &mut Vec<T>,
    remove: Option<usize>,
    blank: impl Fn() -> T,
    is_blank: impl Fn(&T) -> bool,
) {
    if let Some(i) = remove {
        if i < rows.len() {
            rows.remove(i);
        }
    }
    if rows.last().is_none_or(|last| !is_blank(last)) {
        rows.push(blank());
    }
}

/// Draws each row with `row_ui` (which returns true when that row's remove
/// button was clicked), then tidies the list.
pub(super) fn edit_rows<T>(
    ui: &mut egui::Ui,
    rows: &mut Vec<T>,
    blank: impl Fn() -> T,
    is_blank: impl Fn(&T) -> bool,
    mut row_ui: impl FnMut(&mut egui::Ui, &mut T) -> bool,
) {
    let mut remove = None;
    for (i, row) in rows.iter_mut().enumerate() {
        if row_ui(ui, row) {
            remove = Some(i);
        }
    }
    tidy_rows(rows, remove, blank, is_blank);
}

pub(super) fn remove_button(ui: &mut egui::Ui, what: &str) -> bool {
    icons::button(ui, Icon::Trash, &format!("Remove this {what}")).clicked()
}

/// The "include this row" tick box that starts param and form-field rows.
pub(super) fn enabled_checkbox(ui: &mut egui::Ui, enabled: &mut bool) {
    ui.checkbox(enabled, "")
        .on_hover_text(if *enabled { "Included — untick to leave it out" } else { "Left out — tick to include" });
}

#[cfg(test)]
mod tests {
    use super::tidy_rows;

    fn tidy(rows: &mut Vec<String>, remove: Option<usize>) {
        tidy_rows(rows, remove, String::new, |s| s.is_empty());
    }

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn an_empty_list_gets_one_blank_row() {
        let mut rows = v(&[]);
        tidy(&mut rows, None);
        assert_eq!(rows, v(&[""]));
    }

    #[test]
    fn a_filled_last_row_gets_a_fresh_blank_after_it() {
        let mut rows = v(&["a", "b"]);
        tidy(&mut rows, None);
        assert_eq!(rows, v(&["a", "b", ""]));
    }

    #[test]
    fn an_existing_trailing_blank_is_not_duplicated() {
        let mut rows = v(&["a", ""]);
        tidy(&mut rows, None);
        tidy(&mut rows, None);
        assert_eq!(rows, v(&["a", ""]));
    }

    #[test]
    fn removing_a_row_keeps_the_rest_in_order() {
        let mut rows = v(&["a", "b", "c", ""]);
        tidy(&mut rows, Some(1));
        assert_eq!(rows, v(&["a", "c", ""]));
    }

    #[test]
    fn removing_the_only_row_leaves_one_blank() {
        let mut rows = v(&["a"]);
        tidy(&mut rows, Some(0));
        assert_eq!(rows, v(&[""]));
    }

    #[test]
    fn removing_the_trailing_blank_just_recreates_it() {
        let mut rows = v(&["a", ""]);
        tidy(&mut rows, Some(1));
        assert_eq!(rows, v(&["a", ""]));
    }

    #[test]
    fn an_out_of_range_removal_is_ignored() {
        let mut rows = v(&["a", ""]);
        tidy(&mut rows, Some(9));
        assert_eq!(rows, v(&["a", ""]));
    }
}
