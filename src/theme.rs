use eframe::egui;

/// The accent used for the active tab, focused-field borders, and the Send
/// button — one color, used consistently, rather than egui's default blue.
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(90, 125, 230);
pub const CARD_FILL: egui::Color32 = egui::Color32::from_rgb(32, 35, 42);
pub const INPUT_FILL: egui::Color32 = egui::Color32::from_rgb(21, 23, 28);
pub const BORDER: egui::Color32 = egui::Color32::from_rgb(60, 65, 76);
pub const AMBER: egui::Color32 = egui::Color32::from_rgb(210, 160, 60);

pub fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();

    visuals.override_text_color = Some(egui::Color32::from_gray(225));
    visuals.window_fill = egui::Color32::from_rgb(24, 26, 31);
    visuals.panel_fill = egui::Color32::from_rgb(24, 26, 31);
    visuals.extreme_bg_color = INPUT_FILL; // TextEdit / ScrollArea background
    visuals.faint_bg_color = CARD_FILL;

    visuals.selection.bg_fill = ACCENT.linear_multiply(0.55);
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, ACCENT);

    for widget_visuals in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.noninteractive,
    ] {
        widget_visuals.bg_fill = INPUT_FILL;
        widget_visuals.weak_bg_fill = INPUT_FILL;
        widget_visuals.bg_stroke = egui::Stroke::new(1.0_f32, BORDER);
        widget_visuals.rounding = egui::Rounding::same(5.0);
    }
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.3_f32, ACCENT.linear_multiply(0.85));
    visuals.widgets.hovered.rounding = egui::Rounding::same(5.0);
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.5_f32, ACCENT);
    visuals.widgets.active.rounding = egui::Rounding::same(5.0);
    visuals.widgets.active.bg_fill = ACCENT.linear_multiply(0.25);

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.interact_size.y = 26.0;
    style.spacing.window_margin = egui::Margin::same(14.0);
    for (text_style, font_id) in style.text_styles.iter_mut() {
        match text_style {
            egui::TextStyle::Body | egui::TextStyle::Button => font_id.size = 14.5,
            egui::TextStyle::Monospace => font_id.size = 13.5,
            egui::TextStyle::Small => font_id.size = 12.0,
            _ => {}
        }
    }
    ctx.set_style(style);
}

/// A visually distinct "card" — used to separate sections from the flat
/// window background instead of leaving everything the same shade of dark gray.
pub fn card(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(CARD_FILL)
        .stroke(egui::Stroke::new(1.0_f32, BORDER))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::same(12.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            add_contents(ui);
        });
}

/// A card outlined in a given accent color, e.g. amber for the auth field.
pub fn accented_card(ui: &mut egui::Ui, color: egui::Color32, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .stroke(egui::Stroke::new(1.3_f32, color))
        .rounding(egui::Rounding::same(5.0))
        .inner_margin(egui::Margin::symmetric(10.0, 7.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            add_contents(ui);
        });
}

/// Status code as a colored badge — green (2xx), blue (3xx), orange (4xx),
/// red (5xx) — instead of plain colored text, so it reads at a glance.
pub fn status_badge(ui: &mut egui::Ui, status: u16, status_text: &str) {
    let (bg, fg) = match status {
        200..=299 => (
            egui::Color32::from_rgb(20, 65, 40),
            egui::Color32::from_rgb(140, 235, 180),
        ),
        300..=399 => (
            egui::Color32::from_rgb(25, 40, 85),
            egui::Color32::from_rgb(150, 185, 245),
        ),
        400..=499 => (
            egui::Color32::from_rgb(110, 55, 10),
            egui::Color32::from_rgb(255, 175, 90),
        ),
        _ => (
            egui::Color32::from_rgb(105, 20, 20),
            egui::Color32::from_rgb(255, 130, 120),
        ),
    };
    egui::Frame::none()
        .fill(bg)
        .rounding(egui::Rounding::same(4.0))
        .inner_margin(egui::Margin::symmetric(8.0, 3.0))
        .show(ui, |ui| {
            ui.colored_label(fg, format!("{status} {status_text}"));
        });
}

/// A small colored dot used for compact status indication in the history
/// sidebar, where a full badge would be too wide.
pub fn status_dot_color(status: Option<u16>) -> egui::Color32 {
    match status {
        None => egui::Color32::from_rgb(150, 150, 150),
        Some(200..=299) => egui::Color32::from_rgb(90, 210, 140),
        Some(300..=399) => egui::Color32::from_rgb(120, 160, 235),
        Some(400..=499) => egui::Color32::from_rgb(255, 160, 70),
        Some(_) => egui::Color32::from_rgb(235, 90, 80),
    }
}

/// A small hand-painted copy icon (two overlapping page outlines) rather than
/// a text label or a font glyph — avoids repeating the missing-glyph issue
/// from the header remove button, since this doesn't depend on font coverage
/// at all.
pub fn copy_icon_button(ui: &mut egui::Ui) -> bool {
    let size = egui::vec2(24.0, 22.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let color = visuals.fg_stroke.color;
        let painter = ui.painter();
        let back = egui::Rect::from_min_size(rect.min + egui::vec2(3.0, 4.0), egui::vec2(12.0, 14.0));
        painter.rect_stroke(back, egui::Rounding::same(2.0), egui::Stroke::new(1.3_f32, color));
        let front = egui::Rect::from_min_size(rect.min + egui::vec2(8.0, 1.0), egui::vec2(12.0, 14.0));
        painter.rect_filled(front, egui::Rounding::same(2.0), CARD_FILL);
        painter.rect_stroke(front, egui::Rounding::same(2.0), egui::Stroke::new(1.3_f32, color));
    }
    response.on_hover_text("Copy response body").clicked()
}
