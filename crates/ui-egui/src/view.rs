//! The "Beeld" tab: which messages the list shows and in what order, where the reading pane
//! sits, how tall a row is, and the theme. The choices are kept in `view.json` next to the
//! accounts file, so the window opens as it was left.

use std::path::Path;
use std::time::{Duration, Instant};

use merel_engine::view::Sort;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::helder::{self, Icon};
use crate::i18n::{LanguageChoice, tr};
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
    pub(crate) const ALL: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark];

    pub(crate) fn name(self) -> String {
        tr(match self {
            ThemeChoice::System => "theme-system",
            ThemeChoice::Light => "theme-light",
            ThemeChoice::Dark => "theme-dark",
        })
    }

    pub(crate) fn hint(self) -> String {
        tr(match self {
            ThemeChoice::System => "theme-system-hint",
            ThemeChoice::Light => "theme-light-hint",
            ThemeChoice::Dark => "theme-dark-hint",
        })
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

    pub fn name(self) -> String {
        tr(match self {
            MarkRead::Now => "mark-read-now",
            MarkRead::AfterSeconds => "mark-read-later",
            MarkRead::Never => "mark-read-never",
        })
    }
}

/// The name of a sort order in the current language.
pub fn sort_name(sort: Sort) -> String {
    tr(match sort {
        Sort::Newest => "sort-newest",
        Sort::Oldest => "sort-oldest",
        Sort::Sender => "sort-sender",
        Sort::Subject => "sort-subject",
    })
}

/// The menu in the Leesvenster group that holds the [`MarkRead`] choice.
pub fn mark_read_menu() -> String {
    tr("view-mark-read-menu")
}
/// The Categorie menu's choice that shows every message.
pub fn all_categories() -> String {
    tr("view-all-categories")
}

/// What the person chose on the "Beeld" tab.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewPrefs {
    pub reading_pane: ReadingPane,
    /// When a message opened in the reading pane counts as read.
    pub mark_read: MarkRead,
    pub theme: ThemeChoice,
    /// The language of the words on screen: the computer's, or one picked under Beeld > Taal.
    pub language: LanguageChoice,
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
    /// Outlook's "Naast elkaar weergeven": with more than one calendar ticked, the day and week
    /// views give each calendar its own days next to the others instead of one merged grid.
    pub calendars_side_by_side: bool,
    /// The folders pinned under "Favorieten" at the top of the folder pane, by id, in order.
    pub favorites: Vec<String>,
    /// The saved searches listed under "Zoekmappen" in the folder pane.
    pub search_folders: Vec<crate::search_folders::SearchFolder>,
    /// What was last typed in the mail search box, newest first, as Outlook's "Recente
    /// zoekopdrachten"; at most [`RECENT_SEARCHES`].
    pub recent_searches: Vec<String>,
    /// The calendars whose tick box in the side pane is off, by id: their appointments stay
    /// out of the views, the search list and the printed page, as in Outlook when a calendar is
    /// unticked. Reminders still come.
    pub hidden_calendars: Vec<String>,
    /// Whether Merel asks GitHub once a day for a newer release: `None` until the person
    /// has chosen, which counts as no (see [`crate::updates`]).
    pub update_check: Option<bool>,
    /// The release the person chose to skip ("Deze versie overslaan"); a newer one is offered.
    pub skipped_version: Option<String>,
    /// Whether the welcome of the first start has been closed (see [`crate::welcome`]).
    pub welcomed: bool,
    /// The count on the app icon and the computer's notifications (Bestand > Meldingen).
    pub notifications: crate::notifications::Prefs,
    /// Whether the "Alle accounts" group at the top of the folder pane is folded shut.
    pub all_accounts_closed: bool,
    /// The accounts whose folders are folded shut in the folder pane, by id.
    pub closed_accounts: Vec<String>,
    /// The folders whose subfolders are shown, by id; a folder with subfolders starts folded
    /// shut, as in Outlook.
    pub open_folders: Vec<String>,
}

/// How many recent searches the menu next to the search box keeps.
pub const RECENT_SEARCHES: usize = 10;
/// The menu's button, which a screen reader and the hover text name.
pub fn recent_menu() -> String {
    tr("search-recent")
}
pub fn recent_clear() -> String {
    tr("search-recent-clear")
}

impl Default for ViewPrefs {
    fn default() -> Self {
        ViewPrefs {
            reading_pane: ReadingPane::default(),
            mark_read: MarkRead::default(),
            theme: ThemeChoice::default(),
            language: LanguageChoice::default(),
            compact: false,
            conversations: false,
            groups: true,
            sort: Sort::default(),
            week_numbers: false,
            work_hours: true,
            calendars_side_by_side: false,
            favorites: Vec::new(),
            search_folders: Vec::new(),
            recent_searches: Vec::new(),
            hidden_calendars: Vec::new(),
            update_check: None,
            skipped_version: None,
            welcomed: false,
            notifications: Default::default(),
            all_accounts_closed: false,
            closed_accounts: Vec::new(),
            open_folders: Vec::new(),
        }
    }
}

impl ViewPrefs {
    /// Folds `id` open when it is in `list` of the shut ones, or shut when it is not; or the
    /// other way round for a list of open ones.
    pub fn toggle(list: &mut Vec<String>, id: &str) {
        if list.iter().any(|x| x == id) {
            list.retain(|x| x != id);
        } else {
            list.push(id.to_string());
        }
    }

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
        path.map(|p| merel_engine::config::load_json(&merel_engine::config::view_path(p))).unwrap_or_default()
    }

    fn save(&self, path: Option<&Path>) {
        if let Some(p) = path
            && let Err(e) = merel_engine::config::save_json(&merel_engine::config::view_path(p), self)
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
        crate::i18n::set(self.view.language.language());
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
        // The list's filters are chips over the list itself; the category stays here, as a menu
        // like "Categoriseren" on the Start tab.
        group(ui, t, &tr("view-category"), |ui| {
            let current = app.engine.session.category_filter.clone();
            let names: Vec<String> = app.engine.session.categories.iter().map(|c| c.name.clone()).collect();
            let mut pick: Option<Option<String>> = None;
            let shown = current.clone().unwrap_or_else(|| tr("view-all-categories"));
            if crate::menubar::capturing() {
                let mut options = vec![(tr("view-all-categories"), current.is_none())];
                options.extend(names.iter().map(|n| (n.clone(), current.as_deref() == Some(n.as_str()))));
                if let Some(i) = crate::menubar::choice(&tr("view-category"), &options) {
                    pick = Some(if i == 0 { None } else { Some(names[i - 1].clone()) });
                }
            } else {
                helder::menu_button(ui, t, Some(Icon::Tag), shown, |ui| {
                    ui.set_min_width(180.0);
                    if ui.selectable_label(current.is_none(), tr("view-all-categories")).clicked() {
                        pick = Some(None);
                        ui.close();
                    }
                    for n in names {
                        if ui.selectable_label(current.as_deref() == Some(n.as_str()), &n).clicked() {
                            pick = Some(Some(n));
                            ui.close();
                        }
                    }
                })
                .on_hover_text(tr("view-category-hint"));
            }
            if let Some(name) = pick {
                app.run("view.category", json!({ "name": name }));
            }
        });
        group(ui, t, &tr("view-sort"), |ui| {
            let mut sort = app.view.sort;
            if crate::menubar::capturing() {
                let options: Vec<(String, bool)> = Sort::ALL.iter().map(|s| (sort_name(*s), *s == sort)).collect();
                if let Some(i) = crate::menubar::choice(&tr("view-sort"), &options) {
                    sort = Sort::ALL[i];
                }
            } else {
                helder::menu_button(ui, t, Some(Icon::ArrowDownUp), sort_name(sort), |ui| {
                    ui.set_min_width(160.0);
                    for s in Sort::ALL {
                        if ui.selectable_value(&mut sort, s, sort_name(s)).clicked() {
                            ui.close();
                        }
                    }
                })
                .on_hover_text(tr("view-sort-hint"));
            }
            if sort != app.view.sort {
                app.view.sort = sort;
                app.run("view.sort", json!({ "sort": sort }));
                changed = true;
            }
        });
        helder::bar_separator(ui, t);
        section(ui, t, &tr("view-reading-pane"), |ui| {
            for (pane, name, hint) in [
                (ReadingPane::Right, "view-pane-right", "view-pane-right-hint"),
                (ReadingPane::Bottom, "view-pane-bottom", "view-pane-bottom-hint"),
                (ReadingPane::Off, "view-pane-off", "view-pane-off-hint"),
            ] {
                if crate::ribbon_toggle_hint(ui, t, &tr(name), app.view.reading_pane == pane, &tr(hint)) {
                    app.view.reading_pane = pane;
                    changed = true;
                }
            }
            // When an opened message counts as read, as Outlook's reading-pane options.
            let mut choice = app.view.mark_read;
            if crate::menubar::capturing() {
                let options: Vec<(String, bool)> = MarkRead::ALL.iter().map(|m| (m.name(), *m == choice)).collect();
                if let Some(i) = crate::menubar::choice(&tr("view-mark-read"), &options) {
                    choice = MarkRead::ALL[i];
                }
            } else {
                let button = egui::Button::image(Icon::MailCheck.image(helder::ICON, t.accent_text))
                    .right_text(Icon::ChevronDown.image(14.0, t.text_dim))
                    .frame_when_inactive(false)
                    .corner_radius(helder::CONTROL_RADIUS)
                    .min_size(egui::vec2(0.0, helder::BUTTON_HEIGHT));
                let r = egui::containers::menu::MenuButton::from_button(button)
                    .ui(ui, |ui| {
                        ui.label(egui::RichText::new(tr("view-mark-read")).size(12.0).color(t.text_dim));
                        for m in MarkRead::ALL {
                            if ui.selectable_label(choice == m, m.name()).clicked() {
                                choice = m;
                                ui.close();
                            }
                        }
                    })
                    .0;
                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tr("view-mark-read-menu")));
                r.on_hover_text(crate::i18n::tr_args(
                    "view-mark-read-hint",
                    &[("menu", tr("view-mark-read-menu").into()), ("now", choice.name().to_lowercase().into())],
                ));
            }
            if choice != app.view.mark_read {
                app.view.mark_read = choice;
                changed = true;
            }
        });
        helder::bar_separator(ui, t);
        section(ui, t, &tr("view-list"), |ui| {
            for (on, label, hint) in [
                (&mut app.view.compact, "view-compact", "view-compact-hint"),
                (&mut app.view.conversations, "view-conversations", "view-conversations-hint"),
                (&mut app.view.groups, "view-groups", "view-groups-hint"),
            ] {
                if switch(ui, t, on, &tr(label), &tr(hint)) {
                    changed = true;
                }
            }
        });
        helder::bar_separator(ui, t);
    }
    if app.module == Module::Calendar {
        section(ui, t, &tr("view-calendar"), |ui| {
            if switch(ui, t, &mut app.view.week_numbers, &tr("view-week-numbers"), &tr("view-week-numbers-hint")) {
                changed = true;
            }
            if switch(ui, t, &mut app.view.work_hours, &tr("view-work-hours"), &tr("view-work-hours-hint")) {
                changed = true;
            }
        });
        helder::bar_separator(ui, t);
    }
    section(ui, t, &tr("theme-group"), |ui| {
        for choice in ThemeChoice::ALL {
            if crate::ribbon_toggle_hint(ui, t, &choice.name(), app.view.theme == choice, &choice.hint()) {
                app.view.theme = choice;
                changed = true;
            }
        }
    });
    helder::bar_separator(ui, t);
    // One menu, not a button per language: the ribbon is full, and more languages may come.
    group(ui, t, &tr("language-group"), |ui| {
        let mut language = app.view.language;
        if crate::menubar::capturing() {
            let options: Vec<(String, bool)> = LanguageChoice::ALL.iter().map(|c| (c.name(), *c == language)).collect();
            if let Some(i) = crate::menubar::choice(&tr("language-group"), &options) {
                language = LanguageChoice::ALL[i];
            }
        } else {
            helder::menu_button(ui, t, Some(Icon::Globe), language.name(), |ui| {
                ui.set_min_width(160.0);
                for choice in LanguageChoice::ALL {
                    if ui.selectable_value(&mut language, choice, choice.name()).on_hover_text(choice.hint()).clicked() {
                        ui.close();
                    }
                }
            })
            .on_hover_text(tr("language-menu-hint"));
        }
        if language != app.view.language {
            app.view.language = language;
            changed = true;
        }
    });
    if changed {
        let ctx = ui.ctx().clone();
        app.save_view(&ctx);
    }
}

/// An on/off choice on the ribbon: a button with its icon that stays lit while on, as the
/// view buttons of the new Outlook; a screen reader hears a checkbox.
fn switch(ui: &mut egui::Ui, t: &Tokens, on: &mut bool, label: &str, hint: &str) -> bool {
    let clicked = crate::ribbon_toggle_hint(ui, t, label, *on, hint);
    if clicked {
        *on = !*on;
    }
    clicked
}

/// A ribbon [`group`] that is a submenu of its own in the menu bar, such as "Thema".
pub(crate) fn section(ui: &mut egui::Ui, t: &Tokens, name: &str, add: impl FnOnce(&mut egui::Ui)) {
    if crate::menubar::capturing() {
        crate::menubar::submenu(name, || add(ui));
    } else {
        group(ui, t, name, add);
    }
}

/// A ribbon group: its controls in a row, its name in small print underneath, like Outlook.
/// A ribbon group: its buttons in a row with the group's name under them, as Outlook and Word
/// caption their ribbon groups.
pub(crate) fn group(ui: &mut egui::Ui, _t: &Tokens, name: &str, add: impl FnOnce(&mut egui::Ui)) {
    // The Helder ribbon is one line, so the name is not drawn; a screen reader still hears it.
    let (_, r) = ui.allocate_exact_size(egui::Vec2::ZERO, egui::Sense::hover());
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, name));
    // The ribbon row is already horizontal; a nested row would sit lower than its neighbours.
    add(ui);
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
