//! The "Beeld" tab: which messages the list shows and in what order, where the reading pane
//! sits, how tall a row is, and the theme. The choices are kept in `view.json` next to the
//! accounts file, so the window opens as it was left.

use std::path::Path;

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

/// What the person chose on the "Beeld" tab.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewPrefs {
    pub reading_pane: ReadingPane,
    pub theme: ThemeChoice,
    /// Rows without the preview line, so more messages fit.
    pub compact: bool,
    pub sort: Sort,
}

impl ViewPrefs {
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
            egui::ComboBox::from_id_salt("view-category").selected_text(current.clone().unwrap_or_else(|| "Alle".to_string())).width(150.0).show_ui(
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
            egui::ComboBox::from_id_salt("view-sort").selected_text(sort.dutch_name()).show_ui(ui, |ui| {
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
            for (pane, name) in [(ReadingPane::Right, "Rechts"), (ReadingPane::Bottom, "Onder")] {
                if ui.selectable_label(app.view.reading_pane == pane, name).clicked() {
                    app.view.reading_pane = pane;
                    changed = true;
                }
            }
        });
        ui.separator();
        group(ui, t, "Lijst", |ui| {
            if ui.checkbox(&mut app.view.compact, "Compact").on_hover_text("Rijen zonder voorbeeldregel, zodat er meer berichten passen").changed() {
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
