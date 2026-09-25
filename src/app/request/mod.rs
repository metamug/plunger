//! The request editor tabs: Params, Headers, Body, Variables, Options.

mod body;
mod headers;
mod options;
mod params;
mod rows;

use eframe::egui;

const ERROR_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 100, 90);
const OK_COLOR: egui::Color32 = egui::Color32::from_rgb(90, 200, 140);
