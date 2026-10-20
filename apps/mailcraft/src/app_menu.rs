//! The app menu on macOS (the menu named Mailcraft next to the Apple menu). The one the window
//! library makes on its own names the program file ("About mailcraft", "Quit mailcraft") and
//! its About shows an empty panel. This one is in Dutch, like the rest of Mailcraft: "Over
//! Mailcraft" opens Mailcraft's own window with the version and build, "Zoek naar updates…"
//! asks GitHub at once, and hiding and quitting work as in every Mac app. No Edit menu: its
//! copy and paste items would take Cmd+C and Cmd+V away from the text fields.

use std::sync::mpsc::{Receiver, channel};

use muda::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};

/// What a click in the menu asks the app to do.
pub enum Action {
    About,
    Updates,
}

pub struct AppMenu {
    /// The menu lives as long as this does.
    _menu: Menu,
    about: MenuId,
    updates: MenuId,
    events: Receiver<MenuEvent>,
}

impl AppMenu {
    /// Puts the menu in place of the default one; `None` (and the default stays) if that fails.
    pub fn install(ctx: &egui::Context) -> Option<AppMenu> {
        let about = MenuItem::new("Over Mailcraft", true, None);
        let updates = MenuItem::new("Zoek naar updates…", true, None);
        let app = Submenu::with_items(
            "Mailcraft",
            true,
            &[
                &about,
                &updates,
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::services(Some("Voorzieningen")),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::hide(Some("Verberg Mailcraft")),
                &PredefinedMenuItem::hide_others(Some("Verberg andere")),
                &PredefinedMenuItem::show_all(Some("Toon alles")),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::quit(Some("Stop Mailcraft")),
            ],
        )
        .map_err(|e| log::warn!("app menu not built: {e}"))
        .ok()?;
        let menu = Menu::with_items(&[&app]).map_err(|e| log::warn!("app menu not built: {e}")).ok()?;
        menu.init_for_nsapp();
        let (tx, events) = channel();
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let _ = tx.send(event);
            ctx.request_repaint();
        }));
        Some(AppMenu { _menu: menu, about: about.id().clone(), updates: updates.id().clone(), events })
    }

    /// The clicks since the last frame.
    pub fn actions(&self) -> Vec<Action> {
        self.events
            .try_iter()
            .filter_map(|e| {
                if e.id == self.about {
                    Some(Action::About)
                } else if e.id == self.updates {
                    Some(Action::Updates)
                } else {
                    None
                }
            })
            .collect()
    }
}
