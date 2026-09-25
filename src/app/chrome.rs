//! The window's frame around the request editor: the menu bar, the row of
//! request tabs, and the status bar.

use super::tab::Tab;
use super::{ApiTesterApp, NOTICE_FOR};
use crate::icons::{self, Icon};
use crate::model::Outcome;
use crate::theme::{self, one_line, palette, ThemeChoice, ACCENT};
use eframe::egui;

const TAB_HEIGHT: f32 = 32.0;
const TAB_TITLE_MAX: f32 = 180.0;

enum TabAction {
    Activate,
    Close,
}

impl ApiTesterApp {
    pub(super) fn render_menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if item(ui, "New tab", "Ctrl+T") {
                        self.new_tab();
                    }
                    if item(ui, "Save request", "Ctrl+S") {
                        self.save_active();
                    }
                    if item(ui, "Close tab", "Ctrl+W") {
                        self.close_tab(self.active);
                    }
                    ui.separator();
                    if item(ui, "Import a curl command\u{2026}", "") {
                        self.open_curl_dialog();
                    }
                    if item(ui, "Import a HAR file\u{2026}", "") {
                        self.open_har_file();
                    }
                    ui.separator();
                    if item(ui, "Clear history", "") {
                        self.clear_history();
                    }
                });
                ui.menu_button("Settings", |ui| {
                    ui.label(egui::RichText::new("Theme").weak().small());
                    let mut choice = self.settings.theme;
                    ui.radio_value(&mut choice, ThemeChoice::Dark, "Dark");
                    ui.radio_value(&mut choice, ThemeChoice::Light, "Light");
                    if choice != self.settings.theme {
                        self.set_theme(ui.ctx(), choice);
                        ui.close_menu();
                    }
                });
            });
        });
    }

    pub(super) fn render_status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar").exact_height(26.0).show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                let p = palette();
                let tab = self.tab();
                if tab.is_loading() {
                    ui.add(egui::Spinner::new().size(12.0));
                    small(ui, format!("Sending {} {}\u{2026}", tab.state.method, tab.state.url), None);
                } else {
                    match &tab.outcome {
                        Outcome::Empty => small(ui, "Ready".to_string(), None),
                        Outcome::Failed(_) => small(ui, "Request failed".to_string(), Some(p.error)),
                        Outcome::Response(r) => {
                            let (_, fg) = p.status[match r.status {
                                200..=299 => 0,
                                300..=399 => 1,
                                400..=499 => 2,
                                _ => 3,
                            }];
                            small(ui, format!("{} {}", r.status, r.status_text), Some(fg));
                            small(ui, format!("{} ms  \u{b7}  {}", r.elapsed_ms, super::response_panel::format_bytes(r.size_bytes)), None);
                        }
                    }
                }
                if let Some((message, at)) = &self.notice {
                    if at.elapsed() < NOTICE_FOR {
                        ui.separator();
                        small(ui, message.clone(), Some(p.accent_text));
                        ctx.request_repaint_after(NOTICE_FOR);
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    small(ui, format!("v{}", env!("CARGO_PKG_VERSION")), None);
                    ui.separator();
                    small(ui, "Ctrl+Enter send  \u{b7}  Ctrl+S save  \u{b7}  Ctrl+T new tab".to_string(), None);
                    if tab.state.insecure_tls {
                        ui.separator();
                        small(ui, "TLS certificate check off".to_string(), Some(p.amber));
                    }
                    if let Some(err) = &self.secrets_error {
                        ui.separator();
                        ui.add(egui::Label::new(egui::RichText::new(err).small().color(p.error)).truncate())
                            .on_hover_text(err);
                    }
                });
            });
        });
    }

    pub(super) fn render_tab_bar(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        let mut new = false;
        let row = ui.horizontal(|ui| {
            egui::ScrollArea::horizontal()
                .max_width(ui.available_width() - icons::SIZE - 12.0)
                .auto_shrink([true, true])
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for (i, tab) in self.tabs.iter().enumerate() {
                            if let Some(a) = tab_chip(ui, tab, i == self.active) {
                                action = Some((i, a));
                            }
                        }
                    });
                });
            new = icons::button(ui, Icon::Plus, "New tab (Ctrl+T)").clicked();
        });
        // The line the tabs sit on.
        let y = row.response.rect.bottom();
        let x = ui.max_rect().x_range();
        ui.painter().hline(x, y, egui::Stroke::new(1.0_f32, palette().border));

        match action {
            Some((i, TabAction::Activate)) => self.activate(i),
            Some((i, TabAction::Close)) => self.close_tab(i),
            None => {}
        }
        if new {
            self.new_tab();
        }
    }
}

/// A menu entry with its keyboard shortcut shown on the right. Closes the menu when clicked.
fn item(ui: &mut egui::Ui, label: &str, shortcut: &str) -> bool {
    let clicked = ui.add(egui::Button::new(label).shortcut_text(shortcut)).clicked();
    if clicked {
        ui.close_menu();
    }
    clicked
}

fn small(ui: &mut egui::Ui, text: String, color: Option<egui::Color32>) {
    let mut rich = egui::RichText::new(text).small();
    rich = match color {
        Some(c) => rich.color(c),
        None => rich.weak(),
    };
    ui.add(egui::Label::new(rich).selectable(false).truncate());
}

/// One request tab: method, title, and a close button (a spinner while the
/// request is in flight). Middle-click also closes.
fn tab_chip(ui: &mut egui::Ui, tab: &Tab, active: bool) -> Option<TabAction> {
    let p = palette();
    let small_font = egui::TextStyle::Small.resolve(ui.style());
    let title_color = if active { p.text } else { ui.visuals().weak_text_color() };
    let method = one_line(ui, &tab.state.method, small_font, theme::method_color(&tab.state.method), 70.0);
    let title = one_line(ui, &tab.title(), egui::FontId::proportional(13.5), title_color, TAB_TITLE_MAX);
    let close_size = 18.0;
    let width = 10.0 + method.size().x + 6.0 + title.size().x + 8.0 + close_size + 6.0;

    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, TAB_HEIGHT), egui::Sense::click());
    let hovered = ui.rect_contains_pointer(rect);
    let rounding = egui::Rounding { nw: 6.0, ne: 6.0, sw: 0.0, se: 0.0 };
    if active {
        ui.painter().rect(rect, rounding, p.card, egui::Stroke::new(1.0_f32, p.border));
        ui.painter().hline(rect.x_range(), rect.top() + 1.0, egui::Stroke::new(2.0_f32, ACCENT));
    } else if hovered {
        ui.painter().rect_filled(rect, rounding, p.hover);
    }

    let cy = rect.center().y;
    let method_pos = egui::pos2(rect.left() + 10.0, cy - method.size().y / 2.0);
    let title_pos = egui::pos2(method_pos.x + method.size().x + 6.0, cy - title.size().y / 2.0);
    ui.painter().galley(method_pos, method, p.text);
    ui.painter().galley(title_pos, title, p.text);

    let close_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 6.0 - close_size / 2.0, cy),
        egui::vec2(close_size, close_size),
    );
    let mut action = None;
    if tab.is_loading() {
        theme::overlay(ui, ("tab-spinner", tab.id), close_rect, egui::Spinner::new().size(12.0));
    } else if active || hovered {
        let close = ui
            .interact(close_rect, ui.id().with(("close-tab", tab.id)), egui::Sense::click())
            .on_hover_text("Close tab (Ctrl+W)");
        if close.hovered() {
            ui.painter().rect_filled(close_rect, egui::Rounding::same(4.0), p.hover);
        }
        let color = if close.hovered() { p.text_hover } else { ui.visuals().weak_text_color() };
        icons::paint(ui.painter(), close_rect.shrink(4.0), Icon::Close, color);
        if close.clicked() {
            action = Some(TabAction::Close);
        }
    }

    let tooltip = if tab.state.url.is_empty() { tab.title() } else { tab.state.url.clone() };
    let response = response.on_hover_text(tooltip);
    if action.is_none() {
        if response.middle_clicked() {
            action = Some(TabAction::Close);
        } else if response.clicked() {
            action = Some(TabAction::Activate);
        }
    }
    action
}
