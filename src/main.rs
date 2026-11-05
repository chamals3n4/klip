//! Klip — a fast, focused video editor for Reels, Shorts and TikToks.

mod app;
mod audio;
mod dev;
mod export;
mod fx;
mod media;
mod model;
mod theme;
mod ui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Klip")
            .with_app_id("klip")
            .with_inner_size([1480.0, 920.0])
            .with_min_inner_size([960.0, 600.0])
            .with_maximized(true)
            .with_icon(std::sync::Arc::new(theme::app_icon())),
        ..Default::default()
    };
    eframe::run_native("Klip", options, Box::new(|cc| Ok(Box::new(app::KlipApp::new(cc)))))
}
