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
}

fn main() -> eframe::Result {
    mailcraft_oauth::install_crypto();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Mailcraft").with_inner_size([1280.0, 800.0]).with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Mailcraft",
        options,
        Box::new(|_cc| {
            let engine = Engine::load(Arc::new(Keychain), config::default_path());
            Ok(Box::new(App(MailApp::new(engine))))
        }),
    )
}
