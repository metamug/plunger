#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod curl_import;
mod history;
mod http;
mod json_view;
mod model;
mod theme;

use app::ApiTesterApp;
use eframe::egui;
use model::PersistedState;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([980.0, 680.0])
            .with_min_inner_size([620.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Metamug API Tester",
        options,
        Box::new(|cc| {
            theme::apply_theme(&cc.egui_ctx);

            let state: PersistedState = cc
                .storage
                .and_then(|s| eframe::get_value(s, eframe::APP_KEY))
                .unwrap_or_default();
            Ok(Box::new(ApiTesterApp::from_persisted(state)))
        }),
    )
}
