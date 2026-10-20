//! Mailcraft desktop app.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

use std::sync::Arc;

use mailcraft_engine::{Engine, config};
use mailcraft_secrets::Keychain;
use mailcraft_ui_egui::MailApp;

struct App(MailApp);

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.ui(ui);
    }

    fn on_exit(&mut self) {
        // The offline cache is written a moment after each change; the last changes go now.
        self.0.engine.flush();
    }
}

/// The app icon for the window, the Dock and the taskbar; the installers carry the same picture.
const ICON: &[u8] = include_bytes!("../../../packaging/icon-256.png");

fn main() -> eframe::Result {
    mailcraft_oauth::install_crypto();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Mailcraft")
        .with_app_id("mailcraft")
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([800.0, 500.0]);
    match eframe::icon_data::from_png_bytes(ICON) {
        Ok(icon) => viewport = viewport.with_icon(icon),
        Err(e) => log::warn!("app icon unreadable: {e}"),
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        "Mailcraft",
        options,
        Box::new(|_cc| {
            let engine = Engine::load(Arc::new(Keychain), config::default_path());
            Ok(Box::new(App(MailApp::new(engine))))
        }),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_app_icon_decodes() {
        let icon = eframe::icon_data::from_png_bytes(super::ICON).expect("icon");
        assert_eq!((icon.width, icon.height), (256, 256));
    }
}
