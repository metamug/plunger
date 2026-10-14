//! Variables are shared by every request, the command line and agents, so they have a window of
//! their own (Tools > Variables) instead of a tab in each request. Each request shows only the
//! ones it uses, with the value each stands for.

use super::*;
use crate::ui::theme::palette;

/// How many placeholders the line under the URL names before it says "and N more".
const USES_SHOWN: usize = 8;

impl ApiTesterApp {
    /// The Variables window: the user's variables (editable), and those agents set.
    pub(super) fn render_variables_window(&mut self, ctx: &egui::Context) {
        if !self.variables_open {
            return;
        }
        let mut open = true;
        let mut forget = false;
        let mut delete_agent = None;
        crate::ui::fit_window(ctx, egui::Window::new("Variables"))
            .id(egui::Id::new("variables-window"))
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .default_height(420.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Shared by every request and agent").weak().small());
                    crate::ui::icons::help(
                        ui,
                        "Use {{name}} in a URL, a header or a body.\n\
                         Built-ins: {{$uuid}}, {{$timestamp}}, {{$randomInt}} and {{$env:NAME}}.\n\
                         {{name|default}} has a fallback; {{name?}} may be missing (left out of a JSON body).\n\n\
                         Secret values (lock on, or a name like token or password) are never written to a file; \
                         turn on the key to keep one in the system credential store.",
                    );
                });
                let secrets_error = self.secrets_error.clone();
                let tab = &mut self.tabs[self.active];
                forget = tab.render_variables_tab(ui, secrets_error.as_deref(), &self.agent_variables, &mut delete_agent);
            });
        if !open {
            self.variables_open = false;
        }
        if forget {
            self.forget_secrets();
        }
        if let Some(name) = delete_agent {
            self.delete_agent_variable(&name);
        }
    }

    /// One line under the URL: the placeholders this request uses and what each stands for now. A secret
    /// shows as hidden, a placeholder with nothing behind it in red. A tab opened from the history shows
    /// the values that request was actually sent with.
    pub(super) fn render_uses_line(&mut self, ui: &mut egui::Ui) {
        let tab = &self.tabs[self.active];
        let names = crate::domain::vars::placeholders_used(&tab.state);
        if names.is_empty() {
            return;
        }
        let sent = &tab.sent_values;
        let p = palette();
        let mut open_window = false;
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(if sent.is_empty() { "Uses" } else { "Sent with" }).weak().small());
            for used in names.iter().take(USES_SHOWN) {
                let name = &used.name;
                let (mut text, mut color) = self.describe_value(name, sent);
                // a placeholder that may be missing is not a problem when it is
                if text == "undefined" {
                    if let Some(default) = &used.default {
                        text = format!("default {}", one_line_value(default));
                        color = p.text;
                    } else if used.optional {
                        text = "optional, left out".to_string();
                        color = ui.visuals().weak_text_color();
                    }
                }
                let label = egui::Label::new(egui::RichText::new(format!("{name} = {text}")).monospace().small().color(color)).sense(egui::Sense::click());
                if ui.add(label).on_hover_text("Open the variables").clicked() {
                    open_window = true;
                }
            }
            if names.len() > USES_SHOWN {
                ui.label(egui::RichText::new(format!("and {} more", names.len() - USES_SHOWN)).weak().small());
            }
        });
        if open_window {
            self.variables_open = true;
        }
    }

    /// What `{{name}}` stands for, as text and a colour. `sent` holds values a history entry was sent with.
    fn describe_value(&self, name: &str, sent: &std::collections::BTreeMap<String, String>) -> (String, egui::Color32) {
        let p = palette();
        let tab = &self.tabs[self.active];
        if let Some(value) = sent.get(name) {
            return (one_line_value(value), p.text);
        }
        if let Some(env) = name.strip_prefix("$env:") {
            return match std::env::var(env) {
                Ok(_) if crate::domain::redact::is_secret_env_name(env) => ("[secret]".to_string(), p.text),
                Ok(value) => (one_line_value(&value), p.text),
                Err(_) => ("not set".to_string(), p.error),
            };
        }
        let own = tab.state.variables.iter().find(|v| v.name.trim() == name);
        if let Some(v) = own {
            return if v.is_secret() { ("[secret]".to_string(), p.text) } else { (one_line_value(&v.value), p.text) };
        }
        if let Some(v) = self.agent_variables.iter().find(|v| v.name == name) {
            return if v.secret { ("[secret]".to_string(), p.text) } else { (one_line_value(&v.value), p.text) };
        }
        ("undefined".to_string(), p.error)
    }
}

/// A value on one line, cut so a long one cannot fill the screen.
fn one_line_value(value: &str) -> String {
    let flat: String = value.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let flat = flat.trim();
    if flat.is_empty() {
        return "(empty)".to_string();
    }
    if flat.chars().count() > 40 {
        let cut: String = flat.chars().take(40).collect();
        format!("{cut}\u{2026}")
    } else {
        flat.to_string()
    }
}
