//! Mailcraft desktop app.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

use std::sync::Arc;

use mailcraft_engine::{Engine, config};
use mailcraft_secrets::Keychain;
use mailcraft_ui_egui::MailApp;

#[cfg(target_os = "macos")]
mod app_menu;

struct App {
    app: MailApp,
    #[cfg(target_os = "macos")]
    menu: Option<app_menu::AppMenu>,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        if let Some(menu) = &self.menu {
            for action in menu.actions() {
                match action {
                    app_menu::Action::About => self.app.help = Some(mailcraft_ui_egui::help::HelpWindow::About),
                    app_menu::Action::Updates => self.app.check_for_updates(ui.ctx(), true),
                }
            }
        }
        self.app.ui(ui);
    }

    fn on_exit(&mut self) {
        // The offline cache is written a moment after each change; the last changes go now.
        self.app.engine.flush();
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
        Box::new(|#[cfg_attr(not(target_os = "macos"), allow(unused_variables))] cc| {
            let engine = Engine::load(Arc::new(Keychain), config::default_path());
            Ok(Box::new(App {
                app: MailApp::new(engine),
                #[cfg(target_os = "macos")]
                menu: app_menu::AppMenu::install(&cc.egui_ctx),
            }))
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
