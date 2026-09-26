//! The plunger, embossed into the empty response pane: a soft raised shape in
//! the pane's own colour, lit from the top left. It is only a hint of the
//! artwork, so it stays quiet next to the text under it.

use crate::theme::palette;
use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions};

/// White-on-transparent silhouette, 144x204 raw RGBA, made by
/// `packaging/icons/make-icons.ps1`.
const MASK: &[u8] = include_bytes!("../../packaging/icons/plunger-mask.rgba");
const MASK_W: usize = 144;
const MASK_H: usize = 204;
/// The mask is drawn at this fraction of its pixel size, so it stays sharp on high-DPI screens.
const SCALE: f32 = 0.44;
/// How far the highlight and the shadow are pushed apart, in points.
const RELIEF: f32 = 1.5;
/// Height of the drawn plunger in points, so callers can centre it.
pub(super) const HEIGHT: f32 = MASK_H as f32 * SCALE;

fn texture(ctx: &egui::Context) -> TextureHandle {
    let id = egui::Id::new("plunger-emboss-mask");
    if let Some(tex) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return tex;
    }
    let image = ColorImage::from_rgba_unmultiplied([MASK_W, MASK_H], MASK);
    let tex = ctx.load_texture("plunger-emboss", image, TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

/// Moves `c` toward white (`amount` > 0) or black (`amount` < 0) by that many levels.
fn shift(c: Color32, amount: i16) -> Color32 {
    let f = |v: u8| (v as i16 + amount).clamp(0, 255) as u8;
    Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
}

/// Draws the embossed plunger, centred in the current layout.
pub(super) fn plunger(ui: &mut egui::Ui) {
    let size = egui::vec2(MASK_W as f32, MASK_H as f32) * SCALE;
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let tex = texture(ui.ctx());
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    let dark = ui.visuals().dark_mode;
    // Highlight on the lit (top-left) edge, shadow on the far (bottom-right) edge.
    let (light, shade, body) = if dark {
        (
            Color32::from_rgba_unmultiplied(255, 255, 255, 40),
            Color32::from_rgba_unmultiplied(0, 0, 0, 200),
            shift(palette().panel, 7),
        )
    } else {
        (
            Color32::from_rgba_unmultiplied(255, 255, 255, 255),
            Color32::from_rgba_unmultiplied(0, 0, 0, 70),
            shift(palette().panel, -8),
        )
    };
    let offset = egui::vec2(RELIEF, RELIEF);
    let painter = ui.painter();
    painter.image(tex.id(), rect.translate(offset), uv, shade);
    painter.image(tex.id(), rect.translate(-offset), uv, light);
    painter.image(tex.id(), rect, uv, body);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mask_is_exactly_one_rgba_image() {
        assert_eq!(MASK.len(), MASK_W * MASK_H * 4);
    }

    #[test]
    fn the_mask_has_a_shape_and_leaves_the_corners_clear() {
        let alpha = |x: usize, y: usize| MASK[(y * MASK_W + x) * 4 + 3];
        assert_eq!(alpha(0, 0), 0);
        assert_eq!(alpha(MASK_W - 1, MASK_H - 1), 0);
        // the middle of the handle and the middle of the cup are solid
        assert_eq!(alpha(MASK_W / 2, 40), 255);
        assert_eq!(alpha(MASK_W / 2, MASK_H - 45), 255);
    }

    #[test]
    fn shifting_colours_stays_in_range() {
        assert_eq!(shift(Color32::from_rgb(250, 3, 128), 10), Color32::from_rgb(255, 13, 138));
        assert_eq!(shift(Color32::from_rgb(250, 3, 128), -10), Color32::from_rgb(240, 0, 118));
    }
}
