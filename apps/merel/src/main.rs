//! Merel desktop app.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

use std::sync::Arc;

use merel_engine::{Engine, config};
use merel_secrets::Keychain;
use merel_ui_egui::MailApp;

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
                    app_menu::Action::About => self.app.help = Some(merel_ui_egui::help::HelpWindow::About),
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

/// The width the close, minimise and zoom buttons take at the left of a macOS title bar.
const MACOS_WINDOW_BUTTONS: f32 = 72.0;

/// The app icon for the window, the Dock and the taskbar; the installers carry the same picture.
const ICON: &[u8] = include_bytes!("../../../packaging/icon-256.png");

fn main() -> eframe::Result {
    merel_oauth::install_crypto();
    merel_touch_bar::guard();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Merel")
        .with_app_id("merel")
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([800.0, 500.0]);
    // On macOS the window's content runs up under a see-through title bar, as Outlook's does:
    // the search box sits in the title bar beside the window buttons.
    #[cfg(target_os = "macos")]
    {
        viewport = viewport.with_fullsize_content_view(true).with_titlebar_shown(false).with_title_shown(false);
    }
    match eframe::icon_data::from_png_bytes(ICON) {
        Ok(icon) => viewport = viewport.with_icon(icon),
        Err(e) => log::warn!("app icon unreadable: {e}"),
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        "Merel",
        options,
        Box::new(|#[cfg_attr(not(target_os = "macos"), allow(unused_variables))] cc| {
            let engine = Engine::load(Arc::new(Keychain), config::default_path());
            let mut app = MailApp::new(engine);
            if cfg!(target_os = "macos") {
                app.title_bar_inset = MACOS_WINDOW_BUTTONS;
            }
            Ok(Box::new(App {
                app,
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
