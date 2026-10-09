//! The window's keyboard shortcuts.

use super::*;

impl ApiTesterApp {
    pub(super) fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let pressed = |key| {
            ctx.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, key)))
        };
        if pressed(egui::Key::L) {
            let url_id = command_bar::url_field_id(self.tab().id);
            let url_length = self.tab().state.url.chars().count();
            ctx.memory_mut(|memory| memory.request_focus(url_id));
            let mut state = egui::text_edit::TextEditState::load(ctx, url_id).unwrap_or_default();
            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(url_length),
            )));
            state.store(ctx, url_id);
        }
        if pressed(egui::Key::Enter) && !self.tab().is_loading() {
            self.trigger_send(ctx);
        }
        if self.renaming.is_none()
            && self.tab().is_loading()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.tab_mut().cancel();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::C))) {
            self.copy_request_as(ctx, self.export_dialect);
        }
        if pressed(egui::Key::F) {
            let tab = self.tab_mut();
            if tab.response_tab == ResponseTab::Body && matches!(tab.outcome, Outcome::Response(_)) {
                tab.response_search_open = true;
                tab.response_search_focus = true;
            }
        }
        if pressed(egui::Key::T) {
            self.new_tab();
        }
        if pressed(egui::Key::W) {
            self.close_tab(self.active);
        }
        let previous_tab = ctx.input_mut(|i| {
            i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::Tab,
            ))
        });
        if previous_tab {
            let previous = (self.active + self.tabs.len() - 1) % self.tabs.len();
            self.activate(previous);
        } else if pressed(egui::Key::Tab) {
            let next = (self.active + 1) % self.tabs.len();
            self.activate(next);
        }
        if pressed(egui::Key::S) {
            self.save_active();
        }
    }
}
