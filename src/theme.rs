//! Colors, spacing and the small visual building blocks (cards, badges, text
//! fields) shared by every panel. There is one palette per theme; UI code
//! asks for `palette()` instead of hard-coding colors, so switching between
//! dark and light re-colors everything at once.

use eframe::egui;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

/// The accent used for the active tab, focused-field borders, and the Send
/// button — one color, used consistently, in both themes.
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(90, 125, 230);

/// Breathing room inside text fields; egui's default (4×2) makes them cramped.
pub const FIELD_MARGIN: egui::Margin = egui::Margin::symmetric(8.0, 6.0);
/// TextEdit adds its margin *outside* `desired_width`, so subtract this when
/// sizing a field to fill the remaining space.
pub const FIELD_MARGIN_X: f32 = 16.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeChoice {
    #[default]
    Dark,
    Light,
}

pub struct Palette {
    pub panel: egui::Color32,
    pub card: egui::Color32,
    pub input: egui::Color32,
    pub button: egui::Color32,
    pub checkbox: egui::Color32,
    pub hover: egui::Color32,
    pub border: egui::Color32,
    pub text: egui::Color32,
    pub text_widget: egui::Color32,
    pub text_hover: egui::Color32,
    pub text_active: egui::Color32,
    /// Text on an accent-tinted background (selected tab, toggles that are on).
    pub accent_text: egui::Color32,
    pub amber: egui::Color32,
    pub error: egui::Color32,
    pub ok: egui::Color32,
    /// (background, text) for 2xx / 3xx / 4xx / 5xx status badges.
    pub status: [(egui::Color32, egui::Color32); 4],
    /// GET, POST, PUT, PATCH, DELETE, anything else.
    pub methods: [egui::Color32; 6],
    /// punctuation, key, string, number, literal, other — for the JSON editor.
    pub json: [egui::Color32; 6],
}

const fn rgb(r: u8, g: u8, b: u8) -> egui::Color32 {
    egui::Color32::from_rgb(r, g, b)
}

const DARK: Palette = Palette {
    panel: rgb(22, 24, 29),
    card: rgb(30, 33, 40),
    input: rgb(19, 21, 26),
    button: rgb(40, 44, 53),
    checkbox: rgb(66, 72, 87),
    hover: rgb(46, 51, 62),
    border: rgb(48, 53, 63),
    text: rgb(222, 222, 222),
    text_widget: rgb(205, 205, 205),
    text_hover: rgb(240, 240, 240),
    text_active: rgb(255, 255, 255),
    accent_text: rgb(170, 195, 255),
    amber: rgb(210, 160, 60),
    error: rgb(230, 100, 90),
    ok: rgb(90, 210, 140),
    status: [
        (rgb(20, 65, 40), rgb(140, 235, 180)),
        (rgb(25, 40, 85), rgb(150, 185, 245)),
        (rgb(110, 55, 10), rgb(255, 175, 90)),
        (rgb(105, 20, 20), rgb(255, 130, 120)),
    ],
    methods: [
        rgb(90, 210, 140),
        rgb(235, 185, 80),
        rgb(110, 160, 240),
        rgb(180, 140, 235),
        rgb(235, 105, 95),
        rgb(170, 170, 170),
    ],
    json: [
        rgb(150, 150, 150),
        rgb(220, 120, 160),
        rgb(120, 200, 140),
        rgb(110, 170, 230),
        rgb(220, 160, 90),
        rgb(210, 210, 210),
    ],
};

const LIGHT: Palette = Palette {
    panel: rgb(243, 244, 247),
    card: rgb(255, 255, 255),
    input: rgb(250, 251, 252),
    button: rgb(234, 236, 241),
    checkbox: rgb(255, 255, 255),
    hover: rgb(225, 229, 236),
    border: rgb(203, 208, 217),
    text: rgb(33, 36, 43),
    text_widget: rgb(48, 52, 60),
    text_hover: rgb(15, 17, 22),
    text_active: rgb(0, 0, 0),
    accent_text: rgb(35, 70, 185),
    amber: rgb(175, 115, 15),
    error: rgb(195, 50, 40),
    ok: rgb(25, 140, 80),
    status: [
        (rgb(218, 244, 228), rgb(20, 110, 60)),
        (rgb(224, 233, 255), rgb(40, 80, 190)),
        (rgb(255, 234, 212), rgb(165, 85, 10)),
        (rgb(255, 224, 220), rgb(175, 35, 25)),
    ],
    methods: [
        rgb(20, 140, 80),
        rgb(185, 120, 0),
        rgb(40, 100, 210),
        rgb(125, 75, 200),
        rgb(200, 55, 45),
        rgb(100, 100, 100),
    ],
    json: [
        rgb(120, 120, 120),
        rgb(170, 40, 110),
        rgb(20, 130, 60),
        rgb(30, 100, 190),
        rgb(170, 95, 0),
        rgb(40, 40, 40),
    ],
};

static LIGHT_MODE: AtomicBool = AtomicBool::new(false);

/// The palette of the theme currently applied.
pub fn palette() -> &'static Palette {
    if LIGHT_MODE.load(Ordering::Relaxed) {
        &LIGHT
    } else {
        &DARK
    }
}

pub fn method_color(method: &str) -> egui::Color32 {
    let m = &palette().methods;
    match method {
        "GET" => m[0],
        "POST" => m[1],
        "PUT" => m[2],
        "PATCH" => m[3],
        "DELETE" => m[4],
        _ => m[5],
    }
}

/// True when `ctx` still shows our styling for `choice`. egui keeps one style
/// per theme and can swap to its stock one (e.g. on an OS theme change), so
/// compare a color we set rather than just dark vs light.
pub fn is_applied(ctx: &egui::Context, choice: ThemeChoice) -> bool {
    let expected = match choice {
        ThemeChoice::Dark => &DARK,
        ThemeChoice::Light => &LIGHT,
    };
    ctx.style().visuals.panel_fill == expected.panel
}

pub fn apply_theme(ctx: &egui::Context, choice: ThemeChoice) {
    LIGHT_MODE.store(choice == ThemeChoice::Light, Ordering::Relaxed);
    let p = palette();
    let mut visuals = match choice {
        ThemeChoice::Dark => egui::Visuals::dark(),
        ThemeChoice::Light => egui::Visuals::light(),
    };

    // Text colors go on the widget states rather than `override_text_color`:
    // the override is baked into every galley, including TextEdit hint text,
    // which made placeholders look exactly like typed values.
    visuals.widgets.noninteractive.fg_stroke.color = p.text;
    visuals.widgets.inactive.fg_stroke.color = p.text_widget;
    visuals.widgets.hovered.fg_stroke.color = p.text_hover;
    visuals.widgets.active.fg_stroke.color = p.text_active;
    visuals.widgets.open.fg_stroke.color = p.text_hover;

    visuals.window_fill = p.panel;
    visuals.panel_fill = p.panel;
    visuals.extreme_bg_color = p.input; // TextEdit / ScrollArea background
    visuals.faint_bg_color = p.card;
    visuals.window_stroke = egui::Stroke::new(1.0_f32, p.border);

    // selection.stroke is also the text color of the selected tab, so it has
    // to stay readable on top of selection.bg_fill.
    // Opaque, conventional selection colours (VS Code dark / Chrome light). Scaling
    // the accent's alpha instead blended into a washed-out cyan.
    visuals.selection.bg_fill = match choice {
        ThemeChoice::Dark => egui::Color32::from_rgb(38, 79, 120),
        ThemeChoice::Light => egui::Color32::from_rgb(173, 214, 255),
    };
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, p.accent_text);

    let rounding = egui::Rounding::same(5.0);
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, p.border);
    visuals.widgets.noninteractive.rounding = rounding;

    // bg_fill: checkbox/radio boxes (text fields use extreme_bg_color);
    // weak_bg_fill: buttons and combo boxes.
    visuals.widgets.inactive.bg_fill = p.checkbox;
    visuals.widgets.inactive.weak_bg_fill = p.button;
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, p.border);
    visuals.widgets.inactive.rounding = rounding;

    visuals.widgets.hovered.bg_fill = p.checkbox;
    visuals.widgets.hovered.weak_bg_fill = p.hover;
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, ACCENT.linear_multiply(0.7));
    visuals.widgets.hovered.rounding = rounding;
    visuals.widgets.hovered.expansion = 0.0;

    visuals.widgets.active.bg_fill = ACCENT.linear_multiply(0.25);
    visuals.widgets.active.weak_bg_fill = ACCENT.linear_multiply(0.25);
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.active.rounding = rounding;
    visuals.widgets.active.expansion = 0.0;

    visuals.widgets.open.weak_bg_fill = p.hover;
    visuals.widgets.open.rounding = rounding;

    let theme = match choice {
        ThemeChoice::Dark => egui::Theme::Dark,
        ThemeChoice::Light => egui::Theme::Light,
    };
    let mut style = (*ctx.style_of(theme)).clone();
    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.interact_size.y = 30.0;
    style.spacing.window_margin = egui::Margin::same(14.0);
    for (text_style, font_id) in style.text_styles.iter_mut() {
        match text_style {
            egui::TextStyle::Body | egui::TextStyle::Button => font_id.size = 14.5,
            egui::TextStyle::Monospace => font_id.size = 13.5,
            egui::TextStyle::Small => font_id.size = 12.0,
            _ => {}
        }
    }
    ctx.set_style_of(theme, style);
    // Pin the theme so the OS light/dark setting doesn't override the choice.
    ctx.set_theme(theme);
}

/// A single-line text field with the app's padding.
pub fn field(text: &mut dyn egui::TextBuffer) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(text).margin(FIELD_MARGIN)
}

/// A multi-line text area with the app's padding.
pub fn area(text: &mut dyn egui::TextBuffer) -> egui::TextEdit<'_> {
    egui::TextEdit::multiline(text).margin(FIELD_MARGIN)
}

/// Lays `text` out on one line, cut with an ellipsis to `max_width`.
pub fn one_line(ui: &egui::Ui, text: &str, font: egui::FontId, color: egui::Color32, max_width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), egui::TextFormat::simple(font, color));
    job.wrap = egui::text::TextWrapping {
        max_width: max_width.max(10.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('\u{2026}'),
    };
    ui.fonts(|f| f.layout_job(job))
}

/// Adds `widget` exactly over `rect` without taking any space in the
/// surrounding layout (unlike `Ui::put`), so hover-only controls don't make
/// the rows around them jump.
pub fn overlay(ui: &mut egui::Ui, salt: impl std::hash::Hash, rect: egui::Rect, widget: impl egui::Widget) -> egui::Response {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(salt)
            .max_rect(rect)
            .layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight)),
    );
    child.add(widget)
}

/// A visually distinct "card" — used to separate sections from the flat
/// window background.
pub fn card(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    let p = palette();
    egui::Frame::none()
        .fill(p.card)
        .stroke(egui::Stroke::new(1.0_f32, p.border))
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
    let (bg, fg) = palette().status[match status {
        200..=299 => 0,
        300..=399 => 1,
        400..=499 => 2,
        _ => 3,
    }];
    egui::Frame::none()
        .fill(bg)
        .rounding(egui::Rounding::same(4.0))
        .inner_margin(egui::Margin::symmetric(8.0, 3.0))
        .show(ui, |ui| {
            ui.colored_label(fg, format!("{status} {status_text}"));
        });
}

/// A small colored dot used for compact status indication in the history
/// sidebar, where a full badge would be too wide. Mid-tones, readable on both
/// themes.
pub fn status_dot_color(status: Option<u16>) -> egui::Color32 {
    match status {
        None => rgb(150, 150, 150),
        Some(200..=299) => rgb(60, 190, 120),
        Some(300..=399) => rgb(100, 145, 230),
        Some(400..=499) => rgb(240, 150, 50),
        Some(_) => rgb(225, 80, 70),
    }
}
