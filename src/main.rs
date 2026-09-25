#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod curl_import;
mod history;
mod http;
mod json_view;
mod model;
mod redact;
mod theme;
mod vars;

use app::ApiTesterApp;
use eframe::egui;
use model::PersistedState;
use std::io::Write;

/// Release builds are `panic = "abort"` with no console, so a crash would
/// otherwise vanish without a trace. Append it to a log the user can send.
fn install_crash_log() {
    let path = history::app_data_dir().join("crash.log");
    std::panic::set_hook(Box::new(move |info| {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let now = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();
        let entry = format!("[{now}] v{}\n{info}\n\n", env!("CARGO_PKG_VERSION"));
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = f.write_all(entry.as_bytes());
        }
    }));
}

fn main() -> eframe::Result<()> {
    install_crash_log();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([980.0, 680.0])
            .with_min_inner_size([620.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        &format!("Metamug API Tester {}", env!("CARGO_PKG_VERSION")),
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
