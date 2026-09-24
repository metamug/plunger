use super::ApiTesterApp;
use crate::theme::{self, status_dot_color};
use eframe::egui;

const URL_LABEL_MAX_CHARS: usize = 34;

/// Shortens `s` to at most `max` characters, ending in "...". Counts chars,
/// not bytes, so non-ASCII text can't be cut mid-character.
fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let kept: String = s.chars().take(max.saturating_sub(3)).collect();
    format!("{kept}...")
}

impl ApiTesterApp {
    pub(super) fn render_history_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("history_panel")
            .resizable(true)
            .default_width(230.0)
            .width_range(160.0..=400.0)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("HISTORY").weak().small());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Clear").clicked() {
                            if let Some(h) = &self.history {
                                let _ = h.clear();
                            }
                            self.refresh_history();
                        }
                    });
                });
                ui.add_space(4.0);
                ui.separator();

                if self.history.is_none() {
                    ui.label(
                        egui::RichText::new("History unavailable (couldn't open local database).")
                            .weak()
                            .small(),
                    );
                    return;
                }

                let mut pick: Option<usize> = None;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, entry) in self.history_entries.iter().enumerate() {
                        let dot = status_dot_color(entry.status.map(|s| s as u16));
                        let row = ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                            ui.painter().circle_filled(rect.center(), 4.0, dot);
                            ui.add_space(2.0);
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new(&entry.method).small().strong());
                                ui.label(
                                    egui::RichText::new(ellipsize(&entry.url, URL_LABEL_MAX_CHARS))
                                        .small()
                                        .weak(),
                                );
                            });
                        });
                        let rect = row.response.rect;
                        let row_response = ui
                            .interact(rect, ui.id().with(("history-row", entry.id)), egui::Sense::click())
                            .on_hover_text(format!(
                                "{}\n{}\n{}",
                                entry.url,
                                entry.created_at,
                                entry
                                    .elapsed_ms
                                    .map(|ms| format!("{ms} ms"))
                                    .unwrap_or_else(|| "no response".to_string()),
                            ));
                        if row_response.clicked() {
                            pick = Some(i);
                        }
                        if row_response.hovered() {
                            ui.painter()
                                .rect_filled(rect, egui::Rounding::same(4.0), theme::CARD_FILL);
                        }
                        ui.add_space(3.0);
                    }
                    if self.history_entries.is_empty() {
                        ui.label(egui::RichText::new("No requests yet.").weak().small());
                    }
                });

                if let Some(i) = pick {
                    let entry = self.history_entries[i].clone();
                    self.load_history_entry(&entry);
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::ellipsize;

    #[test]
    fn short_strings_are_untouched() {
        assert_eq!(ellipsize("abc", 5), "abc");
        assert_eq!(ellipsize("abcde", 5), "abcde");
    }

    #[test]
    fn long_strings_are_cut_with_ellipsis() {
        assert_eq!(ellipsize("abcdefghij", 8), "abcde...");
    }

    #[test]
    fn multibyte_text_is_not_cut_mid_character() {
        // Byte-index truncation at 31 would panic inside the 'é'/emoji here.
        let url = "https://example.com/caf\u{e9}/\u{1F680}\u{1F680}\u{1F680}/\u{4E16}\u{754C}/aaaaaaaaaaaa";
        let out = ellipsize(url, 34);
        assert!(out.chars().count() <= 34);
        assert!(out.ends_with("..."));
    }
}
