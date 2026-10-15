//! The left sidebar: saved requests and the history of sent requests.
//! Clicking a row opens it in a tab; double-clicking names it in place, which
//! saves it.

use super::{ApiTesterApp, Rename};
use crate::store::history::{HistoryEntry, Source};
use crate::ui::icons::{self, Icon};
use crate::ui::theme::{self, one_line, palette, status_dot_color};
use eframe::egui;

const ONE_LINE: f32 = 30.0;
const TWO_LINES: f32 = 46.0;

enum RowAction {
    Open,
    StartRename,
    Commit(String),
    CancelRename,
    Unsave,
    /// File the saved request under a group (None: take it out of its group).
    MoveToGroup(Option<String>),
}

/// The "Move to group" submenu: the groups there are, no group, and a field for a new one. Returns the
/// choice (`Some(None)` is "no group").
pub(super) fn move_to_group_menu(ui: &mut egui::Ui, groups: &[String], current: Option<&str>, input: &mut String) -> Option<Option<String>> {
    let mut chosen = None;
    ui.menu_button("Move to group", |ui| {
        if ui.add_enabled(current.is_some(), egui::Button::new("No group")).clicked() {
            chosen = Some(None);
            ui.close();
        }
        for group in groups {
            if ui.add_enabled(current != Some(group.as_str()), egui::Button::new(group)).clicked() {
                chosen = Some(Some(group.clone()));
                ui.close();
            }
        }
        ui.separator();
        ui.horizontal(|ui| {
            let edit = ui.add(theme::field(input).hint_text("New group").desired_width(140.0));
            let add = ui.button("Add").clicked() || (edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if add {
                match crate::store::history::normalize_group(input) {
                    Ok(Some(name)) => {
                        chosen = Some(Some(name));
                        input.clear();
                        ui.close();
                    }
                    Ok(None) => {}
                    Err(message) => {
                        ui.colored_label(palette().error, message);
                    }
                }
            }
        });
    });
    chosen
}

/// A group's title row in a list: the path, a count, and a caret. True when it was clicked.
pub(super) fn group_header(ui: &mut egui::Ui, path: &str, count: usize, collapsed: bool) -> bool {
    let depth = path.matches('/').count() as f32;
    let weak = ui.visuals().weak_text_color();
    let response = ui
        .horizontal(|ui| {
            ui.add_space(depth * 10.0 + 2.0);
            let caret_room = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover()).0;
            let c = caret_room.center();
            let points = if collapsed {
                vec![c + egui::vec2(-2.5, -4.0), c + egui::vec2(3.0, 0.0), c + egui::vec2(-2.5, 4.0)]
            } else {
                vec![c + egui::vec2(-4.0, -2.5), c + egui::vec2(4.0, -2.5), c + egui::vec2(0.0, 3.0)]
            };
            ui.painter().add(egui::Shape::convex_polygon(points, weak, egui::Stroke::NONE));
            let name = path.rsplit('/').next().unwrap_or(path);
            ui.add(
                egui::Label::new(egui::RichText::new(format!("{name}  {count}")).small().strong().color(weak))
                    .selectable(false)
                    .sense(egui::Sense::click()),
            )
            .on_hover_text(if collapsed { format!("Expand {path}") } else { format!("Collapse {path}") })
        })
        .inner;
    response.clicked()
}

/// Whether `path` or a group above it is folded away.
pub(super) fn group_is_hidden(path: &str, collapsed: &std::collections::HashSet<String>) -> bool {
    let mut prefix = String::new();
    for part in path.split('/') {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(part);
        if collapsed.contains(&prefix) {
            return true;
        }
    }
    false
}

/// A timestamp for a tooltip: in the user's time zone when the system says what it is, else UTC.
fn friendly_timestamp(ts: &str) -> String {
    let local = crate::domain::timefmt::full(ts);
    if local != ts {
        return local;
    }
    utc_timestamp(ts)
}

/// "2026-09-24T11:33:44.5515843Z" -> "2026-09-24 11:33:44 UTC". Anything that
/// doesn't look like an RFC 3339 timestamp is shown as-is.
fn utc_timestamp(ts: &str) -> String {
    match (ts.get(..10), ts.get(11..19)) {
        (Some(date), Some(time)) if ts.as_bytes().get(10) == Some(&b'T') => format!("{date} {time} UTC"),
        _ => ts.to_string(),
    }
}

impl ApiTesterApp {
    pub(super) fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        if !self.settings.show_sidebar {
            return;
        }
        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(260.0)
            .size_range(180.0..=420.0)
            .show(ui, |ui| {
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
                let mut workflow_action: Option<super::workflows::ListAction> = None;
                let mut toggled_group: Option<String> = None;
                let workflow_selected = self.workflow_view.as_ref().map(|v| v.name.clone());
                // Each list highlights its own link: the saved request being
                // edited, and the history row this tab last came from or sent.
                let saved_selected = self.tab().saved_id;
                let history_selected = self.tab().history_id;

                ui.add_space(8.0);
                let filter = ui.add(
                    theme::field(&mut self.sidebar_filter)
                        .hint_text("Filter requests")
                        .desired_width(ui.available_width() - theme::FIELD_MARGIN_X),
                );
                let filtering = !self.sidebar_filter.trim().is_empty();
                let filter_changed = filter.changed();

                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.add_space(6.0);
                    ui.spacing_mut().item_spacing.y = 2.0;

                    section_header(ui, "SAVED", self.saved_entries.len(), &mut self.saved_open, |_| {});
                    if self.saved_open {
                        // the list is ordered by group, so each run of one group is one block under its header
                        let mut groups: Vec<String> = self.saved_entries.iter().filter_map(|e| e.group.clone()).collect();
                        groups.dedup();
                        let mut run_start = 0;
                        while run_start < self.saved_entries.len() {
                            let group = self.saved_entries[run_start].group.clone();
                            let run_end = self.saved_entries[run_start..]
                                .iter()
                                .position(|e| e.group != group)
                                .map_or(self.saved_entries.len(), |n| run_start + n);
                            let run = &self.saved_entries[run_start..run_end];
                            run_start = run_end;
                            let hidden = !filtering && group.as_deref().is_some_and(|g| group_is_hidden(g, &self.collapsed_groups));
                            if let Some(path) = &group {
                                let collapsed = self.collapsed_groups.contains(path);
                                if group_header(ui, path, run.len(), collapsed && !filtering) {
                                    toggled_group = Some(path.clone());
                                }
                            }
                            if hidden {
                                continue;
                            }
                            let mut rows = |ui: &mut egui::Ui| {
                                for entry in run {
                                    let rename = self.renaming.as_mut().filter(|r| r.id == entry.id && r.list == "saved");
                                    let is_selected = saved_selected == Some(entry.id);
                                    if let Some(action) = entry_row(ui, "saved", entry, is_selected, rename, true, &groups, &mut self.group_input) {
                                        actions.push((entry.clone(), "saved", action));
                                    }
                                }
                            };
                            if group.is_some() {
                                ui.indent(("group-rows", run_end), |ui| rows(ui));
                            } else {
                                rows(ui);
                            }
                        }
                        if self.saved_entries.is_empty() && filtering {
                            hint(ui, "No saved request matches.");
                        } else if self.saved_entries.is_empty() {
                            hint(ui, "Double-click a request in History, or press Ctrl+S, to save it here.");
                        }
                    }

                    if !filtering {
                        workflow_action = super::workflows::sidebar_section(
                            ui,
                            &self.workflows,
                            &mut self.workflows_open,
                            workflow_selected.as_deref(),
                            &self.collapsed_groups,
                            &mut self.group_input,
                        );
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
                            if let Some(action) = entry_row(ui, "history", entry, is_selected, rename, false, &[], &mut self.group_input) {
                                actions.push((entry.clone(), "history", action));
                            }
                        }
                        if self.history_entries.is_empty() && filtering {
                            hint(ui, "No request in the history matches.");
                        } else if self.history_entries.is_empty() {
                            hint(ui, "Requests you send show up here.");
                        }
                    }
                });

                if filter_changed {
                    self.refresh_lists();
                }
                if clear || !actions.is_empty() {
                    // Row heights change (a name line appears), so lay out again now.
                    ui.ctx().request_repaint();
                }
                if clear {
                    self.clear_history();
                }
                if let Some(path) = toggled_group {
                    if !self.collapsed_groups.remove(&path) {
                        self.collapsed_groups.insert(path);
                    }
                }
                if let Some(action) = workflow_action {
                    self.apply_workflow_action(&ui.ctx().clone(), action);
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
                        RowAction::MoveToGroup(group) => {
                            if let Some(h) = &self.history {
                                let _ = h.set_group(entry.id, group.as_deref());
                            }
                            self.refresh_lists();
                        }
                    }
                }
            });
    }
}

pub(super) fn hint(ui: &mut egui::Ui, text: &str) {
    ui.add_space(2.0);
    ui.add(egui::Label::new(egui::RichText::new(text).weak().small()).wrap());
    ui.add_space(2.0);
}

/// A collapsible section title with a count, plus optional controls on the right.
pub(super) fn section_header(ui: &mut egui::Ui, title: &str, count: usize, open: &mut bool, right: impl FnOnce(&mut egui::Ui)) {
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
#[allow(clippy::too_many_arguments)]
fn entry_row(
    ui: &mut egui::Ui,
    list: &str,
    entry: &HistoryEntry,
    selected: bool,
    rename: Option<&mut Rename>,
    removable: bool,
    groups: &[String],
    group_input: &mut String,
) -> Option<RowAction> {
    let two_lines = entry.name.is_some() || rename.is_some();
    let height = if two_lines { TWO_LINES } else { ONE_LINE };
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    // The row keeps its height so the list scrolls correctly, but a row outside the visible area
    // is not laid out or painted: with thousands of saved requests that was most of every frame.
    if rename.is_none() && !ui.is_rect_visible(rect) {
        return None;
    }
    let p = palette();
    let hovered = ui.rect_contains_pointer(rect);

    let fill = if selected {
        p.accent_soft
    } else if hovered {
        p.hover
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, egui::CornerRadius::same(5), fill);

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
        let tag = one_line(ui, &entry.source.as_str().to_ascii_uppercase(), egui::FontId::proportional(8.0), p.accent_text, 30.0);
        let tag_rect = egui::Rect::from_min_size(
            egui::pos2(url_left, line_y - tag.size().y / 2.0 - 1.0),
            tag.size() + egui::vec2(4.0, 2.0),
        );
        ui.painter().rect_filled(tag_rect, egui::CornerRadius::same(3), p.accent_soft);
        ui.painter().galley(tag_rect.min + egui::vec2(2.0, 1.0), tag, p.accent_text);
        url_left = tag_rect.right() + 4.0;
    }
    // The host is usually the same on every row, so show what tells them
    // apart; the full URL is in the tooltip.
    let url_color = p.text;
    let url_galley = one_line(ui, url_path(&entry.url), small, url_color, inner.left() + text_width - url_left);
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
            // Select the whole name so typing replaces it instead of appending to it.
            let mut state = egui::text_edit::TextEditState::load(ui.ctx(), edit.id).unwrap_or_default();
            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(r.text.chars().count()),
            )));
            state.store(ui.ctx(), edit.id);
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
    if removable {
        egui::Popup::context_menu(&response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            if ui.button("Open").clicked() {
                action = Some(RowAction::Open);
                ui.close();
            }
            if ui.button("Rename").clicked() {
                action = Some(RowAction::StartRename);
                ui.close();
            }
            if let Some(chosen) = move_to_group_menu(ui, groups, entry.group.as_deref(), group_input) {
                action = Some(RowAction::MoveToGroup(chosen));
                ui.close();
            }
            ui.separator();
            if ui.button("Remove from Saved").clicked() {
                action = Some(RowAction::Unsave);
                ui.close();
            }
        });
    }
    if action.is_none() {
        if response.double_clicked() {
            action = Some(RowAction::StartRename);
        } else if response.clicked() {
            action = Some(RowAction::Open);
        }
    }
    action
}

/// The path and query of `url`, without the scheme and host. Falls back to
/// the whole string when it has no scheme (say, an unresolved `{{base}}/x`).
fn url_path(url: &str) -> &str {
    let Some((_, rest)) = url.split_once("://") else { return url };
    match rest.find(['/', '?', '#']) {
        Some(i) => &rest[i..],
        None => rest,
    }
}

#[cfg(test)]
mod tests {
    use super::{friendly_timestamp, url_path, utc_timestamp};

    #[test]
    fn rfc3339_timestamps_are_shortened() {
        assert_eq!(utc_timestamp("2026-09-24T11:33:44.5515843Z"), "2026-09-24 11:33:44 UTC");
        assert_eq!(utc_timestamp("2026-09-24T11:33:44Z"), "2026-09-24 11:33:44 UTC");
    }

    #[test]
    fn anything_else_is_left_alone() {
        assert_eq!(friendly_timestamp("t"), "t");
        assert_eq!(friendly_timestamp("2026-09-24 11:33:44"), "2026-09-24 11:33:44");
    }

    #[test]
    fn a_folded_group_hides_what_is_under_it() {
        use super::group_is_hidden;
        use std::collections::HashSet;
        let collapsed: HashSet<String> = ["Jira".to_string()].into();
        assert!(group_is_hidden("Jira", &collapsed));
        assert!(group_is_hidden("Jira/Issues", &collapsed));
        assert!(!group_is_hidden("GitLab", &collapsed));
        assert!(!group_is_hidden("Jiraffe", &collapsed), "a name that only starts the same is another group");
        let only_child: HashSet<String> = ["Jira/Issues".to_string()].into();
        assert!(!group_is_hidden("Jira", &only_child));
    }

    #[test]
    fn url_path_drops_scheme_and_host() {
        assert_eq!(url_path("http://127.0.0.1:18090/echo?x=1"), "/echo?x=1");
        assert_eq!(url_path("https://api.example.com?q=2"), "?q=2");
        assert_eq!(url_path("https://api.example.com"), "api.example.com");
        assert_eq!(url_path("{{base}}/users"), "{{base}}/users");
    }
}
