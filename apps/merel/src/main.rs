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
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if self.app.title_bar_inset > 0.0 {
            merel_window_buttons::place(frame, f64::from(merel_ui_egui::TITLE_BAR_HEIGHT));
        }
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

/// The app icon for the window, the Dock and the taskbar; the installers carry the same picture.
const ICON: &[u8] = include_bytes!("../../../packaging/icon-256.png");

/// Writes warnings and errors to the terminal, so a window that fails to open says why.
struct Stderr;

impl log::Log for Stderr {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("[{} {}] {}", record.level(), record.target(), record.args());
        }
    }

    fn flush(&self) {}
}

fn main() -> eframe::Result {
    if log::set_logger(&Stderr).is_ok() {
        log::set_max_level(log::LevelFilter::Warn);
    }
    merel_oauth::install_crypto();
    merel_touch_bar::guard();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Merel")
        .with_app_id("merel")
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([800.0, 500.0]);
    // On macOS the window's content runs up under a see-through title bar, as Outlook's does:
    // the search box sits in the tall title bar, the window buttons in its middle beside it.
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
        Box::new(|cc| {
            let engine = Engine::load(Arc::new(Keychain), config::default_path());
            // MEREL_EMBEDDED_WINDOWS=1 opens items inside the main window, should windows
            // of their own not work on a system.
            if std::env::var_os("MEREL_EMBEDDED_WINDOWS").is_some() {
                cc.egui_ctx.set_embed_viewports(true);
            }
            let mut app = MailApp::new(engine);
            if cfg!(target_os = "macos") {
                app.title_bar_inset = merel_window_buttons::WIDTH;
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
