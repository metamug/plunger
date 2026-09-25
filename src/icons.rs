//! The app's icon set and the two controls built on it: a plain icon button
//! and an on/off icon toggle. Every action in the UI uses these, so icons look
//! and behave the same everywhere, and each one carries hover text saying
//! what it does.
//!
//! Icons are painted with lines rather than font glyphs, so they can't turn
//! into missing-glyph boxes and they follow the widget's hover/active colors.

use crate::theme::{palette, ACCENT};
use eframe::egui;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    /// Copy to clipboard.
    Copy,
    /// Shown briefly after a copy.
    Check,
    /// Save to a file.
    Download,
    /// Import a request (curl / HAR).
    Import,
    /// Pick a file from disk.
    Folder,
    /// Remove / clear / forget. The one icon for every destructive action.
    Trash,
    /// Remember in the system credential store.
    Key,
    /// Treat a value as secret (masked, never written to a file).
    Lock,
    /// Reformat (e.g. prettify JSON).
    Format,
    /// Switch to editing as raw text.
    Code,
    /// Save the request.
    Save,
    /// Open a new tab.
    Plus,
    /// Close a tab (not a delete — that is always the trash can).
    Close,
}

pub const SIZE: f32 = 28.0;

/// Width to leave at the end of a row for `n` icon buttons, including the
/// spacing before each and the padding a text field adds outside its width.
pub fn trailing_room(ui: &egui::Ui, n: usize) -> f32 {
    n as f32 * (SIZE + ui.spacing().item_spacing.x) + crate::theme::FIELD_MARGIN_X + 2.0
}

/// A square, borderless icon button that lights up on hover.
pub fn button(ui: &mut egui::Ui, icon: Icon, tooltip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(SIZE, SIZE), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        if response.hovered() || response.has_focus() {
            ui.painter().rect_filled(rect, egui::Rounding::same(5.0), palette().hover);
        }
        let color = if icon == Icon::Check { palette().ok } else { visuals.fg_stroke.color };
        paint(ui.painter(), rect.shrink(7.0), icon, color);
    }
    // Disabled widgets only show *disabled* hover text, so set both.
    response.on_hover_text(tooltip).on_disabled_hover_text(tooltip)
}

/// An icon that flips `on` when clicked. When on it is tinted with the accent
/// color; the hover text describes the current state and what a click does.
pub fn toggle(ui: &mut egui::Ui, on: &mut bool, icon: Icon, tooltip_on: &str, tooltip_off: &str) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(SIZE, SIZE), egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let rounding = egui::Rounding::same(5.0);
        if *on {
            ui.painter().rect_filled(rect, rounding, ACCENT.linear_multiply(0.3));
        } else if response.hovered() || response.has_focus() {
            ui.painter().rect_filled(rect, rounding, palette().hover);
        }
        let color = if *on {
            palette().accent_text
        } else {
            visuals.fg_stroke.color.linear_multiply(0.7)
        };
        paint(ui.painter(), rect.shrink(7.0), icon, color);
    }
    let tooltip = if *on { tooltip_on } else { tooltip_off };
    response.on_hover_text(tooltip).on_disabled_hover_text(tooltip)
}

/// Draws `icon` inside `r`, designed on a 14×14 grid.
pub fn paint(painter: &egui::Painter, r: egui::Rect, icon: Icon, color: egui::Color32) {
    let s = r.width() / 14.0;
    let p = |x: f32, y: f32| r.min + egui::vec2(x * s, y * s);
    let stroke = egui::Stroke::new(1.4_f32, color);
    let line = |pts: Vec<egui::Pos2>| {
        painter.add(egui::Shape::line(pts, stroke));
    };
    let arc = |cx: f32, cy: f32, radius: f32, from_deg: f32, to_deg: f32| {
        let steps = 12;
        (0..=steps)
            .map(|i| {
                let a = (from_deg + (to_deg - from_deg) * i as f32 / steps as f32).to_radians();
                p(cx + radius * a.cos(), cy + radius * a.sin())
            })
            .collect::<Vec<_>>()
    };
    match icon {
        Icon::Copy => {
            // Front page, plus the visible edges of the page behind it.
            painter.rect_stroke(
                egui::Rect::from_min_max(p(4.0, 3.0), p(14.0, 14.0)),
                egui::Rounding::same(1.5),
                stroke,
            );
            line(vec![p(4.0, 11.0), p(0.0, 11.0), p(0.0, 0.0), p(10.0, 0.0), p(10.0, 3.0)]);
        }
        Icon::Check => line(vec![p(1.0, 7.5), p(5.0, 11.5), p(13.0, 2.5)]),
        Icon::Download => {
            line(vec![p(7.0, 0.0), p(7.0, 9.5)]);
            line(vec![p(3.0, 5.5), p(7.0, 9.5), p(11.0, 5.5)]);
            line(vec![p(0.5, 10.0), p(0.5, 13.5), p(13.5, 13.5), p(13.5, 10.0)]);
        }
        Icon::Import => {
            line(vec![p(0.0, 7.0), p(9.0, 7.0)]);
            line(vec![p(5.5, 3.5), p(9.0, 7.0), p(5.5, 10.5)]);
            line(vec![p(8.0, 0.5), p(13.5, 0.5), p(13.5, 13.5), p(8.0, 13.5)]);
        }
        Icon::Folder => {
            line(vec![
                p(0.5, 2.0),
                p(5.0, 2.0),
                p(6.5, 4.0),
                p(13.5, 4.0),
                p(13.5, 12.5),
                p(0.5, 12.5),
                p(0.5, 2.0),
            ]);
        }
        Icon::Trash => {
            line(vec![p(0.5, 3.0), p(13.5, 3.0)]);
            line(vec![p(5.0, 3.0), p(5.0, 0.5), p(9.0, 0.5), p(9.0, 3.0)]);
            line(vec![p(2.0, 3.0), p(3.0, 13.5), p(11.0, 13.5), p(12.0, 3.0)]);
            line(vec![p(5.5, 6.0), p(5.5, 11.0)]);
            line(vec![p(8.5, 6.0), p(8.5, 11.0)]);
        }
        Icon::Key => {
            painter.circle_stroke(p(4.0, 7.0), 3.2 * s, stroke);
            line(vec![p(7.2, 7.0), p(13.5, 7.0)]);
            line(vec![p(11.0, 7.0), p(11.0, 10.0)]);
            line(vec![p(13.5, 7.0), p(13.5, 9.5)]);
        }
        Icon::Lock => {
            painter.rect_stroke(
                egui::Rect::from_min_max(p(1.5, 6.5), p(12.5, 13.5)),
                egui::Rounding::same(1.5),
                stroke,
            );
            let mut shackle = vec![p(3.5, 6.5)];
            shackle.extend(arc(7.0, 4.0, 3.5, 180.0, 360.0));
            shackle.push(p(10.5, 6.5));
            line(shackle);
        }
        Icon::Format => {
            line(vec![p(0.5, 1.5), p(13.5, 1.5)]);
            line(vec![p(4.0, 5.2), p(13.5, 5.2)]);
            line(vec![p(4.0, 8.8), p(13.5, 8.8)]);
            line(vec![p(0.5, 12.5), p(13.5, 12.5)]);
        }
        Icon::Code => {
            line(vec![p(4.0, 3.0), p(0.5, 7.0), p(4.0, 11.0)]);
            line(vec![p(10.0, 3.0), p(13.5, 7.0), p(10.0, 11.0)]);
            line(vec![p(8.5, 1.5), p(5.5, 12.5)]);
        }
        Icon::Save => {
            // A floppy disk: body with a clipped corner, the shutter and the label.
            line(vec![
                p(0.5, 0.5),
                p(10.5, 0.5),
                p(13.5, 3.5),
                p(13.5, 13.5),
                p(0.5, 13.5),
                p(0.5, 0.5),
            ]);
            line(vec![p(3.5, 0.5), p(3.5, 4.5), p(9.5, 4.5), p(9.5, 0.5)]);
            line(vec![p(3.5, 13.5), p(3.5, 8.5), p(10.5, 8.5), p(10.5, 13.5)]);
        }
        Icon::Plus => {
            line(vec![p(7.0, 1.0), p(7.0, 13.0)]);
            line(vec![p(1.0, 7.0), p(13.0, 7.0)]);
        }
        Icon::Close => {
            line(vec![p(2.5, 2.5), p(11.5, 11.5)]);
            line(vec![p(11.5, 2.5), p(2.5, 11.5)]);
        }
    }
}
