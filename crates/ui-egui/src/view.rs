//! The "Beeld" tab: which messages the list shows and in what order, where the reading pane
//! sits, how tall a row is, and the theme. The choices are kept in `view.json` next to the
//! accounts file, so the window opens as it was left.

use std::path::Path;
use std::time::{Duration, Instant};

use egui::RichText;
use mailcraft_engine::view::{Filter, Sort};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{MailApp, Module, Tokens};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadingPane {
    #[default]
    Right,
    Bottom,
    /// No reading pane: the list takes the width and a message opens in its own window.
    Off,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    const ALL: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark];

    fn dutch_name(self) -> &'static str {
        match self {
            ThemeChoice::System => "Systeem",
            ThemeChoice::Light => "Licht",
            ThemeChoice::Dark => "Donker",
        }
    }

    pub fn preference(self) -> egui::ThemePreference {
        match self {
            ThemeChoice::System => egui::ThemePreference::System,
            ThemeChoice::Light => egui::ThemePreference::Light,
            ThemeChoice::Dark => egui::ThemePreference::Dark,
        }
    }
}

/// When a message opened in the reading pane is marked read, as Outlook's reading-pane options:
/// at once, after a few seconds, or not by itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkRead {
    #[default]
    Now,
    AfterSeconds,
    Never,
}

impl MarkRead {
    pub const ALL: [MarkRead; 3] = [MarkRead::Now, MarkRead::AfterSeconds, MarkRead::Never];
    /// How long a message is open before `AfterSeconds` marks it read.
    pub const DELAY: Duration = Duration::from_secs(5);

    pub fn dutch_name(self) -> &'static str {
        match self {
            MarkRead::Now => "Meteen",
            MarkRead::AfterSeconds => "Na 5 seconden",
            MarkRead::Never => "Niet vanzelf",
        }
    }
}

/// The menu in the Leesvenster group that holds the [`MarkRead`] choice.
pub const MARK_READ_MENU: &str = "◉ ▾";

/// What the person chose on the "Beeld" tab.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewPrefs {
    pub reading_pane: ReadingPane,
    /// When a message opened in the reading pane counts as read.
    pub mark_read: MarkRead,
    pub theme: ThemeChoice,
    /// Rows without the preview line, so more messages fit.
    pub compact: bool,
    /// Messages on one subject folded behind the one that stands first in the list.
    pub conversations: bool,
    /// Headings "Vandaag", "Gisteren", "Deze week", … over the list when it is sorted by date.
    pub groups: bool,
    pub sort: Sort,
    /// The ISO week number left of each week in the calendar's month and week views, as
    /// Outlook's "Weeknummers weergeven".
    pub week_numbers: bool,
    /// The hours outside the working day (before 8:00, from 17:00, and the weekend) shaded in
    /// the calendar's day and week views, as Outlook shades them. On unless switched off.
    pub work_hours: bool,
    /// The folders pinned under "Favorieten" at the top of the folder pane, by id, in order.
    pub favorites: Vec<String>,
    /// The saved searches listed under "Zoekmappen" in the folder pane.
    pub search_folders: Vec<crate::search_folders::SearchFolder>,
    /// What was last typed in the mail search box, newest first, as Outlook's "Recente
    /// zoekopdrachten"; at most [`RECENT_SEARCHES`].
    pub recent_searches: Vec<String>,
    /// The calendars whose tick box under "Mijn agenda's" is off, by id: their appointments stay
    /// out of the views, the search list and the printed page, as in Outlook when a calendar is
    /// unticked. Reminders still come.
    pub hidden_calendars: Vec<String>,
}

/// How many recent searches the menu next to the search box keeps.
pub const RECENT_SEARCHES: usize = 10;
/// The menu's button and its hover text.
pub const RECENT_MENU: &str = "🕒";
pub const RECENT_HINT: &str = "Recente zoekopdrachten";
pub const RECENT_CLEAR: &str = "Lijst wissen";

impl Default for ViewPrefs {
    fn default() -> Self {
        ViewPrefs {
            reading_pane: ReadingPane::default(),
            mark_read: MarkRead::default(),
            theme: ThemeChoice::default(),
            compact: false,
            conversations: false,
            groups: true,
            sort: Sort::default(),
            week_numbers: false,
            work_hours: true,
            favorites: Vec::new(),
            search_folders: Vec::new(),
            recent_searches: Vec::new(),
            hidden_calendars: Vec::new(),
        }
    }
}

impl ViewPrefs {
    /// Whether the calendar's tick box is on, so its appointments show.
    pub fn calendar_shown(&self, calendar_id: &str) -> bool {
        !self.hidden_calendars.iter().any(|c| c == calendar_id)
    }

    /// Ticks or unticks the calendar.
    pub fn set_calendar_shown(&mut self, calendar_id: &str, shown: bool) {
        self.hidden_calendars.retain(|c| c != calendar_id);
        if !shown {
            self.hidden_calendars.push(calendar_id.to_string());
        }
    }

    /// Puts `query` at the top of the recent searches, once, dropping the oldest beyond the
    /// limit. Blank text is not kept.
    pub fn remember_search(&mut self, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            return;
        }
        self.recent_searches.retain(|q| !q.eq_ignore_ascii_case(query));
        self.recent_searches.insert(0, query.to_string());
        self.recent_searches.truncate(RECENT_SEARCHES);
    }

    /// The saved choices, or the defaults when there is no file or it does not parse.
    pub fn load(path: Option<&Path>) -> Self {
        path.map(|p| mailcraft_engine::config::load_json(&mailcraft_engine::config::view_path(p))).unwrap_or_default()
    }

    fn save(&self, path: Option<&Path>) {
        if let Some(p) = path
            && let Err(e) = mailcraft_engine::config::save_json(&mailcraft_engine::config::view_path(p), self)
        {
            log::warn!("view preferences not saved: {e}");
        }
    }
}

impl MailApp {
    /// Writes the choices to disk and applies the theme.
    pub fn save_view(&mut self, ctx: &egui::Context) {
        self.view.save(self.engine.config_path());
        ctx.set_theme(self.view.theme.preference());
    }

    /// Whether the folder is pinned under "Favorieten".
    pub fn is_favorite(&self, folder_id: &str) -> bool {
        self.view.favorites.iter().any(|f| f == folder_id)
    }

    /// Selects the message the way "Gelezen markeren" says: marked read at once, after
    /// [`MarkRead::DELAY`] while it stays open, or not by itself.
    pub fn select_message(&mut self, id: &str) {
        self.run("message.select", json!({ "id": id, "mark_read": self.view.mark_read == MarkRead::Now }));
        self.read_timer = (self.view.mark_read == MarkRead::AfterSeconds).then(|| (id.to_string(), Instant::now()));
    }

    /// Marks the open message read once it has been open for the delay; called every frame.
    pub fn tick_read_timer(&mut self, ctx: &egui::Context) {
        let Some((id, since)) = self.read_timer.clone() else { return };
        if self.engine.session.selected_message.as_deref() != Some(id.as_str()) {
            self.read_timer = None;
            return;
        }
        let left = MarkRead::DELAY.saturating_sub(since.elapsed());
        if left.is_zero() {
            self.read_timer = None;
            self.run("message.mark_read_one", json!({ "id": id }));
        } else {
            ctx.request_repaint_after(left);
        }
    }

    /// Pins the folder under "Favorieten", or takes it off again, and keeps the choice.
    pub fn toggle_favorite(&mut self, folder_id: &str, ctx: &egui::Context) {
        if self.is_favorite(folder_id) {
            self.view.favorites.retain(|f| f != folder_id);
        } else {
            self.view.favorites.push(folder_id.to_string());
        }
        self.save_view(ctx);
    }
}

/// The "Beeld" tab: list filter, category and order, reading pane, row height and theme.
pub fn ribbon(app: &mut MailApp, ui: &mut egui::Ui, t: &Tokens) {
    let mut changed = false;
    if app.module == Module::Mail {
        group(ui, t, "Weergeven", |ui| {
            for f in Filter::ALL {
                if ui.selectable_label(app.engine.session.filter == f, f.dutch_name()).clicked() {
                    app.run("view.filter", json!({ "filter": f }));
                }
            }
        });
        ui.separator();
        group(ui, t, "Categorie", |ui| {
            let current = app.engine.session.category_filter.clone();
            let names: Vec<String> = app.engine.session.categories.iter().map(|c| c.name.clone()).collect();
            let mut pick: Option<Option<String>> = None;
            egui::ComboBox::from_id_salt("view-category").selected_text(current.clone().unwrap_or_else(|| "Alle".to_string())).width(100.0).show_ui(
                ui,
                |ui| {
                    if ui.selectable_label(current.is_none(), "Alle").clicked() {
                        pick = Some(None);
                    }
                    for n in names {
                        if ui.selectable_label(current.as_deref() == Some(n.as_str()), &n).clicked() {
                            pick = Some(Some(n));
                        }
                    }
                },
            );
            if let Some(name) = pick {
                app.run("view.category", json!({ "name": name }));
            }
        });
        ui.separator();
        group(ui, t, "Sorteren", |ui| {
            let mut sort = app.view.sort;
            egui::ComboBox::from_id_salt("view-sort").selected_text(sort.dutch_name()).width(92.0).show_ui(ui, |ui| {
                for s in Sort::ALL {
                    ui.selectable_value(&mut sort, s, s.dutch_name());
                }
            });
            if sort != app.view.sort {
                app.view.sort = sort;
                app.run("view.sort", json!({ "sort": sort }));
                changed = true;
            }
        });
        ui.separator();
        group(ui, t, "Leesvenster", |ui| {
            for (pane, name) in [(ReadingPane::Right, "Rechts"), (ReadingPane::Bottom, "Onder"), (ReadingPane::Off, "Uit")] {
                let hint =
                    if pane == ReadingPane::Off { "Geen leesvenster; Enter of een dubbelklik opent het bericht in een eigen venster" } else { "" };
                if ui.selectable_label(app.view.reading_pane == pane, name).on_hover_text(hint).clicked() {
                    app.view.reading_pane = pane;
                    changed = true;
                }
            }
            // When an opened message counts as read, as Outlook's reading-pane options.
            let mut choice = app.view.mark_read;
            ui.menu_button(MARK_READ_MENU, |ui| {
                for m in MarkRead::ALL {
                    if ui.selectable_label(choice == m, m.dutch_name()).clicked() {
                        choice = m;
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text(format!("Gelezen markeren: wanneer een geopend bericht als gelezen telt. Nu: {}.", choice.dutch_name().to_lowercase()));
            if choice != app.view.mark_read {
                app.view.mark_read = choice;
                changed = true;
            }
        });
        ui.separator();
        group(ui, t, "Lijst", |ui| {
            if ui.checkbox(&mut app.view.compact, "Compact").on_hover_text("Rijen zonder voorbeeldregel, zodat er meer berichten passen").changed() {
                changed = true;
            }
            if ui
                .checkbox(&mut app.view.conversations, "Gesprekken")
                .on_hover_text("Berichten over hetzelfde onderwerp als één gesprek achter het nieuwste bericht; het pijltje ervoor toont de rest.")
                .changed()
            {
                changed = true;
            }
            if ui
                .checkbox(&mut app.view.groups, "Groepen")
                .on_hover_text("Kopjes Vandaag, Gisteren, Deze week, Vorige week, Deze maand… boven de berichten, zoals in Outlook; alleen bij sorteren op datum.")
                .changed()
            {
                changed = true;
            }
        });
        ui.separator();
    }
    if app.module == Module::Calendar {
        group(ui, t, "Agenda", |ui| {
            if ui
                .checkbox(&mut app.view.week_numbers, "Weeknummers")
                .on_hover_text("Het weeknummer links van elke week in de maand- en weekweergave, zoals in Outlook.")
                .changed()
            {
                changed = true;
            }
            if ui
                .checkbox(&mut app.view.work_hours, crate::calendar::WORK_HOURS)
                .on_hover_text("De uren buiten 8:00-17:00 en het weekend grijs in de dag- en weekweergave, zoals in Outlook.")
                .changed()
            {
                changed = true;
            }
        });
        ui.separator();
    }
    group(ui, t, "Thema", |ui| {
        for choice in ThemeChoice::ALL {
            if ui.selectable_label(app.view.theme == choice, choice.dutch_name()).clicked() {
                app.view.theme = choice;
                changed = true;
            }
        }
    });
    if changed {
        let ctx = ui.ctx().clone();
        app.save_view(&ctx);
    }
}

/// A ribbon group: its controls in a row, its name in small print underneath, like Outlook.
fn group(ui: &mut egui::Ui, t: &Tokens, name: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        ui.horizontal(add);
        ui.add_space(2.0);
        ui.label(RichText::new(name).size(11.0).color(t.text_dim));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_searches_keep_the_newest_once_and_drop_the_oldest() {
        let mut v = ViewPrefs::default();
        v.remember_search("  ");
        assert!(v.recent_searches.is_empty(), "blank text is not kept");
        for q in ["offerte", "factuur", "Offerte "] {
            v.remember_search(q);
        }
        assert_eq!(v.recent_searches, ["Offerte", "factuur"], "the same words once, newest first, case aside");
        for i in 0..RECENT_SEARCHES {
            v.remember_search(&format!("zoek{i}"));
        }
        assert_eq!(v.recent_searches.len(), RECENT_SEARCHES);
        assert_eq!(v.recent_searches[0], format!("zoek{}", RECENT_SEARCHES - 1));
    }
}
