//! taggui-rs entry point: loads settings and starts the eframe app.

mod app;
mod extras;
mod panels;

use tg_core::settings::{default_config_path, Settings};

fn main() -> Result<(), eframe::Error> {
    let settings = Settings::load(&default_config_path()).unwrap_or_default();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("TagGUI-rs")
            .with_inner_size([1400.0, 900.0]),
        ..Default::default()
    };
    eframe::run_native(
        "taggui-rs",
        options,
        Box::new(|_creation_context| Ok(Box::new(app::TagGuiApp::new(settings)))),
    )
}
