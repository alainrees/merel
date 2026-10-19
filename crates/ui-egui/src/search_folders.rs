//! "Zoekmappen", Outlook's search folders: a search the person saved under a name, listed in
//! the folder pane like a folder. One holds the words, whether it looks in every folder and the
//! list filter; choosing it runs that search again over the mail Mailcraft has fetched. The list
//! lives in `view.json` with the other choices of the Beeld tab.

use egui::RichText;
use mailcraft_engine::view::Filter;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{MailApp, Tokens};

/// A saved search.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchFolder {
    pub name: String,
    pub query: String,
    #[serde(default)]
    pub all_folders: bool,
    #[serde(default)]
    pub filter: Filter,
}

/// The heading over the saved searches in the folder pane.
pub const HEADING: &str = "🔍 Zoekmappen";
/// Longest name kept.
const MAX_NAME: usize = 60;

/// The form for a new search folder: the name, over the search that is open now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Form {
    pub name: String,
}

/// What the search folder looks for, for the hover text and the form.
pub fn describe(f: &SearchFolder) -> String {
    let where_ = if f.all_folders { "alle mappen" } else { "de huidige map" };
    let only = match f.filter {
        Filter::All => "",
        Filter::Unread => ", alleen ongelezen",
        Filter::Flagged => ", alleen gevlagd",
        Filter::Attachments => ", alleen met bijlagen",
        Filter::Snoozed => ", alleen gesluimerd",
    };
    format!("Zoekt “{}” in {where_}{only}", f.query)
}

impl MailApp {
    /// The search that is open now, as a search folder would keep it; `None` without words.
    fn current_search(&self) -> Option<SearchFolder> {
        let s = &self.engine.session;
        let query = s.search.trim();
        (!query.is_empty()).then(|| SearchFolder { name: String::new(), query: query.to_string(), all_folders: s.search_all, filter: s.filter })
    }

    /// Opens the form to save the open search, its name prefilled with the words.
    pub fn ask_search_folder(&mut self) {
        if let Some(f) = self.current_search() {
            self.search_folder_form = Some(Form { name: f.query });
        }
    }

    /// Whether the search folder is what the list shows now.
    pub fn search_folder_open(&self, f: &SearchFolder) -> bool {
        self.current_search().is_some_and(|c| c.query == f.query && c.all_folders == f.all_folders && c.filter == f.filter)
    }

    /// Runs the saved search: the words into the search box, the scope and the filter with it.
    pub fn open_search_folder(&mut self, f: &SearchFolder) {
        self.drafts_for = None;
        self.engine.session.search = f.query.clone();
        self.engine.session.search_all = f.all_folders;
        self.run("view.filter", json!({ "filter": f.filter }));
    }

    /// Saves the open search under `name`; a name that is taken replaces that search folder.
    fn save_search_folder(&mut self, name: &str, ctx: &egui::Context) {
        let name: String = name.trim().chars().take(MAX_NAME).collect();
        let Some(mut f) = self.current_search() else { return };
        if name.is_empty() {
            return;
        }
        f.name = name;
        self.view.search_folders.retain(|s| s.name != f.name);
        self.view.search_folders.push(f);
        self.save_view(ctx);
    }

    /// Forgets the search folder; the mail it found is untouched.
    pub fn delete_search_folder(&mut self, name: &str, ctx: &egui::Context) {
        self.view.search_folders.retain(|s| s.name != name);
        self.save_view(ctx);
    }
}

/// The section in the folder pane: one row per saved search, with its menu.
pub fn section(app: &mut MailApp, ui: &mut egui::Ui, t: &Tokens) {
    if app.view.search_folders.is_empty() {
        return;
    }
    ui.label(RichText::new(HEADING).strong().color(t.text));
    ui.add_space(4.0);
    let mut open = None;
    let mut delete = None;
    for f in app.view.search_folders.clone() {
        let on = app.search_folder_open(&f);
        let r = ui.add_sized(
            [ui.available_width(), 28.0],
            egui::Button::new("").fill(if on { t.checked } else { egui::Color32::TRANSPARENT }).frame_when_inactive(on),
        );
        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &f.name));
        let r = r.on_hover_text(describe(&f));
        let rect = r.rect.shrink2(egui::vec2(8.0, 0.0));
        ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, format!("🔍  {}", f.name), egui::FontId::proportional(13.5), t.text);
        if r.clicked() {
            open = Some(f.clone());
        }
        r.context_menu(|ui| {
            if ui.button("Zoekmap verwijderen").clicked() {
                delete = Some(f.name.clone());
                ui.close();
            }
        });
    }
    ui.add_space(12.0);
    if let Some(f) = open {
        app.open_search_folder(&f);
    }
    if let Some(name) = delete {
        app.delete_search_folder(&name, ui.ctx());
    }
}

/// The form, while it is open.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(mut form) = app.search_folder_form.clone() else { return };
    let Some(current) = app.current_search() else {
        app.search_folder_form = None;
        return;
    };
    let mut save = false;
    let mut close = false;
    egui::Window::new("Zoekmap opslaan").collapsible(false).resizable(false).default_width(320.0).show(ctx, |ui| {
        ui.label(
            RichText::new(format!("{}. De zoekmap komt in het mappenvenster onder Zoekmappen.", describe(&current))).size(12.0).color(t.text_dim),
        );
        ui.add_space(6.0);
        let field = ui.add(egui::TextEdit::singleline(&mut form.name).hint_text("Naam van de zoekmap").desired_width(300.0));
        if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            save = true;
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(!form.name.trim().is_empty(), egui::Button::new("Opslaan")).clicked() {
                save = true;
            }
            if ui.button("Annuleren").clicked() {
                close = true;
            }
        });
    });
    if save {
        app.save_search_folder(&form.name, ctx);
    }
    app.search_folder_form = if save || close { None } else { Some(form) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_search_folder_describes_itself() {
        let f = SearchFolder { name: "Offertes".into(), query: "offerte".into(), all_folders: true, filter: Filter::Unread };
        assert_eq!(describe(&f), "Zoekt “offerte” in alle mappen, alleen ongelezen");
        let f = SearchFolder { all_folders: false, filter: Filter::All, ..f };
        assert_eq!(describe(&f), "Zoekt “offerte” in de huidige map");
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(serde_json::from_str::<SearchFolder>(&json).unwrap(), f);
    }
}
