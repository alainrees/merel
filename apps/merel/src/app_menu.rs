//! The macOS menu bar. The app menu (the menu named Merel next to the Apple menu): the one the
//! window library makes on its own names the program file ("About merel", "Quit merel") and
//! its About shows an empty panel. This one is in Dutch, like the rest of Merel: "Over
//! Merel" opens Merel's own window with the version and build, "Zoek naar updates…"
//! asks GitHub at once, and hiding and quitting work as in every Mac app. No Edit menu: its
//! copy and paste items would take Cmd+C and Cmd+V away from the text fields.
//!
//! After it come the menus the app notes each frame from its ribbon tabs (Bestand, Bericht,
//! Map, Weergeven, Help; see `merel_ui_egui::menubar`), and the usual Venster menu, as Outlook
//! for Mac has them.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};

use merel_ui_egui::menubar::{self, Entry};
use muda::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};

/// What a click in the menu asks the app to do.
pub enum Action {
    About,
    Updates,
    /// An item of the app's own menus: the menu's place and the labels down to the item.
    Menu(usize, menubar::Path),
}

/// An item of the app's own menus, kept to update whether it can be used and is on.
enum Item {
    Plain(MenuItem, bool),
    Check(CheckMenuItem, bool, bool),
}

pub struct AppMenu {
    /// The menu lives as long as this does.
    menu: Menu,
    about: MenuId,
    updates: MenuId,
    events: Receiver<MenuEvent>,
    /// The app's own menus as last put in the bar, and their menus in it.
    shown: Vec<menubar::Menu>,
    tops: Vec<Submenu>,
    /// The items in the order the menus list them, and where each one leads.
    items: Vec<Item>,
    paths: HashMap<MenuId, (usize, menubar::Path)>,
}

impl AppMenu {
    /// Puts the menu in place of the default one; `None` (and the default stays) if that fails.
    pub fn install(ctx: &egui::Context) -> Option<AppMenu> {
        let about = MenuItem::new("Over Merel", true, None);
        let updates = MenuItem::new("Zoek naar updates…", true, None);
        let app = Submenu::with_items(
            "Merel",
            true,
            &[
                &about,
                &updates,
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::services(Some("Voorzieningen")),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::hide(Some("Verberg Merel")),
                &PredefinedMenuItem::hide_others(Some("Verberg andere")),
                &PredefinedMenuItem::show_all(Some("Toon alles")),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::quit(Some("Stop Merel")),
            ],
        )
        .map_err(|e| log::warn!("app menu not built: {e}"))
        .ok()?;
        let window = Submenu::with_items(
            "Venster",
            true,
            &[
                &PredefinedMenuItem::minimize(Some("Minimaliseer")),
                &PredefinedMenuItem::maximize(Some("Zoom")),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::fullscreen(Some("Schermvullende weergave")),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::bring_all_to_front(Some("Alles op voorgrond")),
            ],
        )
        .map_err(|e| log::warn!("window menu not built: {e}"))
        .ok()?;
        let menu = Menu::with_items(&[&app, &window]).map_err(|e| log::warn!("app menu not built: {e}")).ok()?;
        window.set_as_windows_menu_for_nsapp();
        menu.init_for_nsapp();
        let (tx, events) = channel();
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let _ = tx.send(event);
            ctx.request_repaint();
        }));
        Some(AppMenu {
            menu,
            about: about.id().clone(),
            updates: updates.id().clone(),
            events,
            shown: Vec::new(),
            tops: Vec::new(),
            items: Vec::new(),
            paths: HashMap::new(),
        })
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
                    self.paths.get(&e.id).map(|(menu, path)| Action::Menu(*menu, path.clone()))
                }
            })
            .collect()
    }

    /// Brings the bar in line with `menus`: the menus are built again when their lines
    /// changed, else only the items that changed whether they can be used or are on.
    pub fn sync(&mut self, menus: &[menubar::Menu]) {
        let same = menus.len() == self.shown.len() && menus.iter().zip(&self.shown).all(|(a, b)| a.same_shape(b));
        if !same {
            self.rebuild(menus);
            return;
        }
        let mut items = self.items.iter_mut();
        for m in menus {
            update(&m.entries, &mut items);
        }
        self.shown = menus.to_vec();
    }

    fn rebuild(&mut self, menus: &[menubar::Menu]) {
        for top in self.tops.drain(..) {
            let _ = self.menu.remove(&top);
        }
        self.items.clear();
        self.paths.clear();
        // After the app menu; Help goes last, after the Venster menu, as on every Mac.
        let mut place = 1;
        for (index, m) in menus.iter().enumerate() {
            let top = Submenu::new(m.title, true);
            fill(&top, &m.entries, index, &mut Vec::new(), &mut self.items, &mut self.paths);
            let added = if m.title == "Help" {
                self.menu.append(&top)
            } else {
                place += 1;
                self.menu.insert(&top, place - 1)
            };
            if let Err(e) = added {
                log::warn!("menu {} not added: {e}", m.title);
            }
            if m.title == "Help" {
                top.set_as_help_menu_for_nsapp();
            }
            self.tops.push(top);
        }
        self.shown = menus.to_vec();
    }
}

/// Puts `entries` in `menu`, noting each item and the path to it.
fn fill(
    menu: &Submenu,
    entries: &[Entry],
    index: usize,
    path: &mut Vec<String>,
    items: &mut Vec<Item>,
    paths: &mut HashMap<MenuId, (usize, menubar::Path)>,
) {
    for e in entries {
        let added = match e {
            Entry::Separator => menu.append(&PredefinedMenuItem::separator()),
            Entry::Item { label, enabled, checked } => {
                let mut to = path.clone();
                to.push(label.clone());
                match checked {
                    Some(on) => {
                        let item = CheckMenuItem::new(label, *enabled, *on, None);
                        paths.insert(item.id().clone(), (index, to));
                        let r = menu.append(&item);
                        items.push(Item::Check(item, *enabled, *on));
                        r
                    }
                    None => {
                        let item = MenuItem::new(label, *enabled, None);
                        paths.insert(item.id().clone(), (index, to));
                        let r = menu.append(&item);
                        items.push(Item::Plain(item, *enabled));
                        r
                    }
                }
            }
            Entry::Submenu { label, entries } => {
                let sub = Submenu::new(label, true);
                path.push(label.clone());
                fill(&sub, entries, index, path, items, paths);
                path.pop();
                menu.append(&sub)
            }
        };
        if let Err(e) = added {
            log::warn!("menu item not added: {e}");
        }
    }
}

/// Sets whether each item can be used and is on, taking the items in the order of `entries`.
fn update<'a>(entries: &[Entry], items: &mut impl Iterator<Item = &'a mut Item>) {
    for e in entries {
        match e {
            Entry::Separator => {}
            Entry::Submenu { entries, .. } => update(entries, items),
            Entry::Item { enabled, checked, .. } => match (items.next(), checked) {
                (Some(Item::Plain(item, was)), None) if *was != *enabled => {
                    item.set_enabled(*enabled);
                    *was = *enabled;
                }
                (Some(Item::Check(item, was, was_on)), Some(on)) => {
                    if *was != *enabled {
                        item.set_enabled(*enabled);
                        *was = *enabled;
                    }
                    // A click ticks the item by itself; the app's state decides.
                    if *was_on != *on || item.is_checked() != *on {
                        item.set_checked(*on);
                        *was_on = *on;
                    }
                }
                _ => {}
            },
        }
    }
}
