//! Drawing helpers shared by the window: theme and palette, icons, syntax highlighting, the JSON tree and the
//! fallback fonts for scripts the bundled ones lack.

pub mod theme;
pub mod icons;
pub mod highlight;
pub mod json_view;
pub mod fallback_fonts;

/// A dialog never grows past the window it opens in: on a laptop screen it stops short of the edges and its
/// contents scroll, so a button is always within reach.
pub fn fit_window<'a>(ctx: &egui::Context, window: egui::Window<'a>) -> egui::Window<'a> {
    let screen = ctx.content_rect();
    window
        .max_width((screen.width() - 24.0).max(240.0))
        .max_height((screen.height() - 72.0).max(180.0))
        .vscroll(true)
}
