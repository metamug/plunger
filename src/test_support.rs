//! Helpers for the headless egui tests.

use eframe::egui;

/// Runs one egui pass with no window behind it and discards the output. egui insists that the
/// texture changes of a pass are applied or cleared before it is dropped.
pub fn pass(ctx: &egui::Context, input: egui::RawInput, add_contents: impl FnMut(&mut egui::Ui)) {
    ctx.run_ui(input, add_contents).textures_delta.clear();
}
