//! The left sidebar: saved requests and the history of sent requests.
//! Clicking a row opens it in a tab; double-clicking names it in place, which
//! saves it.

use super::{ApiTesterApp, Rename};
use crate::history::{HistoryEntry, Source};
use crate::icons::{self, Icon};
use crate::theme::{self, one_line, palette, status_dot_color, ACCENT};
use eframe::egui;

const ONE_LINE: f32 = 30.0;
const TWO_LINES: f32 = 46.0;

enum RowAction {
    Open,
    StartRename,
    Commit(String),
    CancelRename,
    Unsave,
}

/// "2026-09-24T11:33:44.5515843Z" -> "2026-09-24 11:33:44 UTC". Anything that
/// doesn't look like an RFC 3339 timestamp is shown as-is.
fn friendly_timestamp(ts: &str) -> String {
    match (ts.get(..10), ts.get(11..19)) {
        (Some(date), Some(time)) if ts.as_bytes().get(10) == Some(&b'T') => format!("{date} {time} UTC"),
        _ => ts.to_string(),
    }
}

impl ApiTesterApp {
    pub(super) fn render_sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("sidebar")
            .resizable(true)
            .default_width(260.0)
            .width_range(180.0..=420.0)
            .show(ctx, |ui| {
                if self.history.is_none() {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("History and saved requests are unavailable (couldn't open the local database).")
                            .weak()
                            .small(),
                    );
                    return;
                }
                let mut actions: Vec<(HistoryEntry, &'static str, RowAction)> = Vec::new();
                let mut clear = false;
                // Each list highlights its own link: the saved request being
                // edited, and the history row this tab last came from or sent.
                let saved_selected = self.tab().saved_id;
                let history_selected = self.tab().history_id;

                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.add_space(6.0);
                    ui.spacing_mut().item_spacing.y = 2.0;

                    section_header(ui, "SAVED", self.saved_entries.len(), &mut self.saved_open, |_| {});
                    if self.saved_open {
                        for entry in &self.saved_entries {
                            let rename = self.renaming.as_mut().filter(|r| r.id == entry.id && r.list == "saved");
                            let is_selected = saved_selected == Some(entry.id);
                            if let Some(action) = entry_row(ui, "saved", entry, is_selected, rename, true) {
                                actions.push((entry.clone(), "saved", action));
                            }
                        }
                        if self.saved_entries.is_empty() {
                            hint(ui, "Double-click a request in History, or press Ctrl+S, to save it here.");
                        }
                    }

                    ui.add_space(8.0);
                    section_header(ui, "HISTORY", self.history_entries.len(), &mut self.history_open, |ui| {
                        let enabled = !self.history_entries.is_empty();
                        let button = ui.add_enabled_ui(enabled, |ui| {
                            icons::button(ui, Icon::Trash, "Clear history (saved requests are kept)")
                        });
                        clear = button.inner.clicked();
                    });
                    if self.history_open {
                        for entry in &self.history_entries {
                            // A row in both lists is renamed where it was double-clicked.
                            let rename = self.renaming.as_mut().filter(|r| r.id == entry.id && r.list == "history");
                            let is_selected = history_selected == Some(entry.id);
                            if let Some(action) = entry_row(ui, "history", entry, is_selected, rename, false) {
                                actions.push((entry.clone(), "history", action));
                            }
                        }
                        if self.history_entries.is_empty() {
                            hint(ui, "Requests you send show up here.");
                        }
                    }
                });

                if clear || !actions.is_empty() {
                    // Row heights change (a name line appears), so lay out again now.
                    ui.ctx().request_repaint();
                }
                if clear {
                    self.clear_history();
                }
                for (entry, list, action) in actions {
                    match action {
                        RowAction::Open => self.open_entry(&entry),
                        RowAction::StartRename => self.start_rename(&entry, list),
                        RowAction::Commit(name) => {
                            self.renaming = None;
                            self.commit_rename(entry.id, &name);
                        }
                        RowAction::CancelRename => self.renaming = None,
                        RowAction::Unsave => self.unsave(entry.id),
                    }
                }
            });
    }
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.add_space(2.0);
    ui.add(egui::Label::new(egui::RichText::new(text).weak().small()).wrap());
    ui.add_space(2.0);
}

/// A collapsible section title with a count, plus optional controls on the right.
fn section_header(ui: &mut egui::Ui, title: &str, count: usize, open: &mut bool, right: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        let label = if count > 0 { format!("{title}  {count}") } else { title.to_string() };
        let text = egui::RichText::new(label).small().strong().color(ui.visuals().weak_text_color());
        let response = ui
            .add(egui::Label::new(text).selectable(false).sense(egui::Sense::click()))
            .on_hover_text(if *open { "Collapse" } else { "Expand" });
        // A small caret after the title, painted so it can't be a missing glyph.
        let caret = egui::Rect::from_center_size(
            egui::pos2(response.rect.right() + 8.0, response.rect.center().y),
            egui::vec2(8.0, 8.0),
        );
        let color = ui.visuals().weak_text_color();
        let points = if *open {
            vec![caret.left_top(), caret.right_top(), caret.center_bottom()]
        } else {
            vec![caret.left_top(), caret.right_center(), caret.left_bottom()]
        };
        ui.painter().add(egui::Shape::convex_polygon(points, color, egui::Stroke::NONE));
        if response.clicked() {
            *open = !*open;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
    });
}

/// One request in the sidebar: an optional name line, then status dot,
/// method and URL. The whole row is the click target; the name field (while
/// renaming) and the remove icon sit on top of it.
fn entry_row(
    ui: &mut egui::Ui,
    list: &str,
    entry: &HistoryEntry,
    selected: bool,
    rename: Option<&mut Rename>,
    removable: bool,
) -> Option<RowAction> {
    let two_lines = entry.name.is_some() || rename.is_some();
    let height = if two_lines { TWO_LINES } else { ONE_LINE };
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    let p = palette();
    let hovered = ui.rect_contains_pointer(rect);

    let fill = if selected {
        ACCENT.linear_multiply(0.22)
    } else if hovered {
        p.hover
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, egui::Rounding::same(5.0), fill);

    let trash_room = if removable { icons::SIZE } else { 0.0 };
    let inner = rect.shrink2(egui::vec2(8.0, 5.0));
    let text_width = inner.width() - trash_room;
    let small = egui::TextStyle::Small.resolve(ui.style());
    let body = egui::FontId::proportional(13.5);
    let weak = ui.visuals().weak_text_color();

    // The request line: status dot, method, URL.
    let line_y = if two_lines { inner.bottom() - 8.0 } else { inner.center().y };
    let dot = status_dot_color(entry.status.map(|s| s as u16));
    ui.painter().circle_filled(egui::pos2(inner.left() + 4.0, line_y), 4.0, dot);
    let method_galley = one_line(ui, &entry.method, small.clone(), theme::method_color(&entry.method), 60.0);
    let method_pos = egui::pos2(inner.left() + 14.0, line_y - method_galley.size().y / 2.0);
    let method_width = method_galley.size().x;
    ui.painter().galley(method_pos, method_galley, weak);
    let mut url_left = method_pos.x + method_width + 6.0;
    // Requests an agent sent get a small tag, so they stand out from your own.
    if entry.source != Source::Gui {
        let tag = one_line(ui, &entry.source.as_str().to_ascii_uppercase(), egui::FontId::proportional(9.5), p.accent_text, 40.0);
        let tag_rect = egui::Rect::from_min_size(
            egui::pos2(url_left, line_y - tag.size().y / 2.0 - 1.0),
            tag.size() + egui::vec2(6.0, 2.0),
        );
        ui.painter().rect_filled(tag_rect, egui::Rounding::same(3.0), ACCENT.linear_multiply(0.25));
        ui.painter().galley(tag_rect.min + egui::vec2(3.0, 1.0), tag, p.accent_text);
        url_left = tag_rect.right() + 6.0;
    }
    let url_color = p.text;
    let url_galley = one_line(ui, &entry.url, small, url_color, inner.left() + text_width - url_left);
    ui.painter().galley(egui::pos2(url_left, line_y - url_galley.size().y / 2.0), url_galley, weak);

    let mut action = None;

    // The name line, or the rename field in its place.
    let name_rect = egui::Rect::from_min_size(inner.min, egui::vec2(text_width, 20.0));
    if let Some(r) = rename {
        let edit = theme::overlay(
            ui,
            (list, "rename-field", entry.id),
            name_rect.expand2(egui::vec2(4.0, 1.0)),
            egui::TextEdit::singleline(&mut r.text)
                .id_salt((list, "rename", entry.id))
                .margin(egui::vec2(4.0, 1.0))
                .hint_text("Name this request"),
        );
        if r.focus {
            edit.request_focus();
            r.focus = false;
        }
        if edit.lost_focus() {
            let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
            action = Some(if escaped { RowAction::CancelRename } else { RowAction::Commit(r.text.clone()) });
        }
    } else if let Some(name) = &entry.name {
        let galley = one_line(ui, name, body, p.text, text_width);
        ui.painter().galley(name_rect.min, galley, p.text);
    }

    if removable && hovered {
        let trash = egui::Rect::from_center_size(
            egui::pos2(rect.right() - icons::SIZE / 2.0 - 2.0, rect.center().y),
            egui::vec2(icons::SIZE, icons::SIZE),
        );
        let clicked = theme::overlay(ui, (list, "unsave", entry.id), trash, |ui: &mut egui::Ui| {
            icons::button(ui, Icon::Trash, "Remove from Saved")
        })
        .clicked();
        if clicked {
            return Some(RowAction::Unsave);
        }
    }

    let verb = if entry.name.is_some() { "rename" } else { "name and save" };
    let when = friendly_timestamp(&entry.created_at);
    let timing = entry.elapsed_ms.map(|ms| format!("{ms} ms")).unwrap_or_else(|| "no response".to_string());
    let sender = match entry.source {
        Source::Gui => String::new(),
        other => format!("  \u{b7}  sent by an agent ({})", other.as_str()),
    };
    let response = response.on_hover_text(format!("{}\n{when}  \u{b7}  {timing}{sender}\nDouble-click to {verb}", entry.url));
    if action.is_none() {
        if response.double_clicked() {
            action = Some(RowAction::StartRename);
        } else if response.clicked() {
            action = Some(RowAction::Open);
        }
    }
    action
}

#[cfg(test)]
mod tests {
    use super::friendly_timestamp;

    #[test]
    fn rfc3339_timestamps_are_shortened() {
        assert_eq!(friendly_timestamp("2026-09-24T11:33:44.5515843Z"), "2026-09-24 11:33:44 UTC");
        assert_eq!(friendly_timestamp("2026-09-24T11:33:44Z"), "2026-09-24 11:33:44 UTC");
    }

    #[test]
    fn anything_else_is_left_alone() {
        assert_eq!(friendly_timestamp("t"), "t");
        assert_eq!(friendly_timestamp("2026-09-24 11:33:44"), "2026-09-24 11:33:44");
    }
}
