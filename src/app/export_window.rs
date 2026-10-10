//! Exporting the active request as a command someone can paste into a shell: curl for bash or
//! the Windows command prompt, or PowerShell. The command is shown highlighted.

use super::ApiTesterApp;
use crate::commands::{self, Dialect};
use crate::{highlight, theme};
use eframe::egui;

/// Tall enough for a long command; beyond this the box scrolls.
const COMMAND_BOX_HEIGHT: f32 = 320.0;

/// The open export dialog and the syntax it shows.
pub(super) struct ExportDialog {
    dialect: Dialect,
}

impl ApiTesterApp {
    pub(super) fn open_export_dialog(&mut self) {
        self.export = Some(ExportDialog { dialect: self.export_dialect });
    }

    /// Puts the active request on the clipboard as a command in `dialect`, and remembers the
    /// choice for Ctrl+Shift+C.
    pub(super) fn copy_request_as(&mut self, ctx: &egui::Context, dialect: Dialect) {
        ctx.copy_text(commands::export(&self.tab().state, dialect));
        self.export_dialect = dialect;
        self.notify(format!("Copied as {}", dialect.label()));
    }

    pub(super) fn render_export_window(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &self.export else { return };
        let mut dialect = dialog.dialect;
        let mut open = true;
        let (mut copy, mut close) = (false, false);
        // Computed from the tab every frame, so an edit made while the dialog is open shows up.
        let text = commands::export(&self.tabs[self.active].state, dialect);
        egui::Window::new("Export request")
            .collapsible(false)
            .resizable(true)
            .default_width(620.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Format");
                    egui::ComboBox::from_id_salt("export-dialect").selected_text(dialect.label()).show_ui(ui, |ui| {
                        for d in Dialect::ALL {
                            ui.selectable_value(&mut dialect, d, d.label());
                        }
                    });
                });
                ui.add_space(4.0);
                let mut shown: &str = &text;
                let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
                    let text = text.as_str();
                    let mut job = highlight::command(text);
                    job.wrap.max_width = wrap_width;
                    ui.ctx().fonts_mut(|fonts| fonts.layout_job(job))
                };
                egui::ScrollArea::vertical().max_height(COMMAND_BOX_HEIGHT).show(ui, |ui| {
                    ui.add(
                        theme::area(&mut shown)
                            .font(egui::TextStyle::Monospace)
                            .desired_rows(8)
                            .desired_width(f32::INFINITY)
                            .layouter(&mut layouter),
                    );
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    copy = ui.button("Copy").on_hover_text("{{variables}} stay as placeholders; secrets are never written out").clicked();
                    close = ui.button("Close").clicked();
                });
            });
        if copy {
            ctx.copy_text(text);
            self.export_dialect = dialect;
            self.notify(format!("Copied as {}", dialect.label()));
        }
        self.export = if close || !open { None } else { Some(ExportDialog { dialect }) };
    }
}
