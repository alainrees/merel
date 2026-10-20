//! The rules window: the rules for incoming mail per account, a form to make or change one,
//! and a button to run them over the open folder right away.

use egui::{RichText, Ui, vec2};
use serde_json::json;

use mailcraft_model::{FolderKind, Rule, RuleField};

use crate::{MailApp, Tokens};

/// What the window remembers between frames.
#[derive(Default)]
pub struct State {
    pub form: Option<Form>,
}

/// The rule being made or changed, as the person edits it.
#[derive(Clone, Debug, Default)]
pub struct Form {
    pub id: Option<String>,
    pub name: String,
    pub account_id: String,
    pub field: RuleField,
    pub contains: String,
    pub move_to: Option<String>,
    pub mark_read: bool,
    pub flag: bool,
    pub categories: Vec<String>,
    pub enabled: bool,
    pub error: Option<String>,
}

impl Form {
    pub fn new(account_id: &str) -> Self {
        Form { account_id: account_id.to_string(), enabled: true, ..Form::default() }
    }

    pub fn from_rule(r: &Rule) -> Self {
        Form {
            id: Some(r.id.clone()),
            name: r.name.clone(),
            account_id: r.account_id.clone(),
            field: r.field,
            contains: r.contains.clone(),
            move_to: r.move_to.clone(),
            mark_read: r.mark_read,
            flag: r.flag,
            categories: r.categories.clone(),
            enabled: r.enabled,
            error: None,
        }
    }

    /// The `rule.save` parameters.
    pub fn params(&self) -> serde_json::Value {
        let mut p = json!({
            "name": self.name,
            "account_id": self.account_id,
            "field": self.field,
            "contains": self.contains,
            "move_to": self.move_to,
            "mark_read": self.mark_read,
            "flag": self.flag,
            "categories": self.categories,
            "enabled": self.enabled,
        });
        if let Some(id) = &self.id {
            p["id"] = json!(id);
        }
        p
    }
}

/// One line that says what a rule does: "Van bevat sanne@ → naar Facturen, gelezen".
pub fn describe(r: &Rule, folder_name: Option<&str>) -> String {
    let mut actions = Vec::new();
    if r.move_to.is_some() {
        actions.push(format!("naar {}", folder_name.unwrap_or("een map die er niet meer is")));
    }
    if r.mark_read {
        actions.push("markeren als gelezen".to_string());
    }
    if r.flag {
        actions.push("vlag zetten".to_string());
    }
    if !r.categories.is_empty() {
        actions.push(format!("categorie {}", r.categories.join(", ")));
    }
    format!("{} bevat “{}” → {}", r.field.dutch_name(), r.contains, actions.join(", "))
}

impl MailApp {
    /// Opens the window, with a new rule for the selected message's sender when `from_message`.
    pub fn open_rules(&mut self, from_message: bool) {
        let mut state = State::default();
        if from_message && let Some(m) = self.engine.session.selected().cloned() {
            let account = self.engine.session.folder(&m.folder_id).map(|f| f.account_id.clone()).unwrap_or_default();
            let mut form = Form::new(&account);
            form.field = RuleField::From;
            form.contains = m.from.email().to_string();
            form.name = format!("Mail van {}", m.from.display());
            state.form = Some(form);
        }
        self.rules = Some(state);
    }
}

fn save_error(e: &mailcraft_engine::CommandError) -> String {
    use mailcraft_engine::CommandError;
    match e {
        CommandError::Param("contains") => "Vul in waar de regel op moet letten.".into(),
        CommandError::Param("move_to") => {
            "Kies wat de regel moet doen: verplaatsen, markeren als gelezen, een vlag zetten of een categorie geven.".into()
        }
        CommandError::Param("categories") => "Kies categorieën uit de lijst.".into(),
        CommandError::Param("account_id") => "Kies een account.".into(),
        CommandError::Store(m) => format!("Opslaan is mislukt: {m}"),
        e => e.to_string(),
    }
}

fn form_ui(app: &MailApp, ui: &mut Ui, t: &Tokens, form: &mut Form) -> (bool, bool) {
    let accounts: Vec<(String, String)> = app.engine.session.accounts.iter().map(|a| (a.id.clone(), a.address.email().to_string())).collect();
    if form.account_id.is_empty() {
        form.account_id = accounts.first().map(|(id, _)| id.clone()).unwrap_or_default();
    }
    let label = |ui: &mut Ui, text: &str| {
        ui.add_sized([110.0, 22.0], egui::Label::new(RichText::new(text).color(t.text)));
    };
    ui.horizontal(|ui| {
        label(ui, "Naam");
        ui.add(egui::TextEdit::singleline(&mut form.name).desired_width(280.0).hint_text("Bijvoorbeeld: Facturen"));
    });
    if accounts.len() > 1 {
        let current = accounts.iter().find(|(id, _)| *id == form.account_id).map(|(_, e)| e.as_str()).unwrap_or("");
        ui.horizontal(|ui| {
            label(ui, "Account");
            egui::ComboBox::from_id_salt("rule-account").selected_text(current).width(280.0).show_ui(ui, |ui| {
                for (id, email) in &accounts {
                    if ui.selectable_value(&mut form.account_id, id.clone(), email).changed() {
                        form.move_to = None;
                    }
                }
            });
        });
    }
    ui.horizontal(|ui| {
        label(ui, "Als");
        egui::ComboBox::from_id_salt("rule-field").selected_text(form.field.dutch_name()).width(110.0).show_ui(ui, |ui| {
            for f in RuleField::ALL {
                ui.selectable_value(&mut form.field, f, f.dutch_name());
            }
        });
        ui.label(RichText::new("bevat").color(t.text));
        ui.add(egui::TextEdit::singleline(&mut form.contains).desired_width(160.0).hint_text("tekst"));
    });
    let folders: Vec<(String, String)> = app
        .engine
        .session
        .folders
        .iter()
        .filter(|f| f.account_id == form.account_id && f.kind != FolderKind::Inbox)
        .map(|f| (f.id.clone(), f.name.clone()))
        .collect();
    ui.horizontal(|ui| {
        label(ui, "Dan");
        let mut moving = form.move_to.is_some();
        if ui.checkbox(&mut moving, "verplaatsen naar").changed() {
            form.move_to = if moving { folders.first().map(|(id, _)| id.clone()) } else { None };
        }
        let current = form.move_to.as_ref().and_then(|id| folders.iter().find(|(f, _)| f == id)).map(|(_, n)| n.as_str()).unwrap_or("map");
        ui.add_enabled_ui(moving, |ui| {
            egui::ComboBox::from_id_salt("rule-folder").selected_text(current).width(160.0).show_ui(ui, |ui| {
                for (id, name) in &folders {
                    ui.selectable_value(&mut form.move_to, Some(id.clone()), name);
                }
            });
        });
    });
    ui.horizontal(|ui| {
        ui.add_space(114.0);
        ui.checkbox(&mut form.mark_read, "markeren als gelezen");
        ui.checkbox(&mut form.flag, "vlag zetten");
    });
    let known: Vec<String> = app.engine.session.categories.iter().map(|c| c.name.clone()).collect();
    if !known.is_empty() {
        ui.horizontal(|ui| {
            label(ui, "Categorie");
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(300.0);
                for name in &known {
                    let mut on = form.categories.iter().any(|c| c.eq_ignore_ascii_case(name));
                    if ui.checkbox(&mut on, name).changed() {
                        if on {
                            form.categories.push(name.clone());
                        } else {
                            form.categories.retain(|c| !c.eq_ignore_ascii_case(name));
                        }
                    }
                }
            });
        });
    }
    if let Some(e) = &form.error {
        ui.label(RichText::new(e).color(t.flag));
    }
    ui.add_space(4.0);
    let mut save = false;
    let mut cancel = false;
    ui.horizontal(|ui| {
        save = ui.add(egui::Button::new(RichText::new("Regel opslaan").color(t.on_accent)).fill(t.accent).min_size(vec2(110.0, 26.0))).clicked();
        cancel = ui.add(egui::Button::new("Annuleren").min_size(vec2(90.0, 26.0))).clicked();
    });
    (save, cancel)
}

enum Action {
    Edit(String),
    Delete(String),
    Toggle(String, bool),
    New,
    FromMessage,
    Run,
}

/// The rules window, when it is open.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    if app.rules.is_none() {
        return;
    }
    let rules: Vec<Rule> = app.engine.session.rules.clone();
    let folder_name = |id: &str| app.engine.session.folder(id).map(|f| f.name.clone());
    let account_name = |id: &str| app.engine.session.accounts.iter().find(|a| a.id == id).map(|a| a.address.email().to_string());
    let several = app.engine.session.accounts.len() > 1;
    let has_message = app.engine.session.selected_message.is_some();
    let open_folder = app.engine.session.selected_folder.clone();
    let mut action: Option<Action> = None;
    let mut save = false;
    let mut cancel = false;
    let mut close = false;
    let mut form_state = app.rules.as_mut().and_then(|s| s.form.take());
    egui::Window::new("Regels").collapsible(false).resizable(false).default_width(460.0).show(ctx, |ui| {
        ui.label(
            RichText::new("Regels kijken naar nieuwe mail in het Postvak IN van een account en verplaatsen of markeren wat erop past.")
                .size(12.0)
                .color(t.text_dim),
        );
        ui.add_space(6.0);
        if let Some(form) = form_state.as_mut() {
            ui.label(RichText::new(if form.id.is_some() { "Regel bewerken" } else { "Nieuwe regel" }).strong().color(t.text));
            let (s, c) = form_ui(app, ui, t, form);
            save = s;
            cancel = c;
            return;
        }
        ui.horizontal(|ui| {
            if ui.button("Nieuwe regel").clicked() {
                action = Some(Action::New);
            }
            if ui
                .add_enabled(has_message, egui::Button::new("Regel voor deze afzender"))
                .on_hover_text("Een regel voor de afzender van het geselecteerde bericht")
                .clicked()
            {
                action = Some(Action::FromMessage);
            }
        });
        ui.add_space(6.0);
        if rules.is_empty() {
            ui.label(RichText::new("Nog geen regels.").color(t.text_dim));
        }
        for r in &rules {
            egui::Frame::new().fill(t.list).corner_radius(6).inner_margin(8).show(ui, |ui| {
                ui.set_width(440.0);
                ui.horizontal(|ui| {
                    let mut on = r.enabled;
                    if ui.checkbox(&mut on, "").on_hover_text(if on { "Regel staat aan" } else { "Regel staat uit" }).changed() {
                        action = Some(Action::Toggle(r.id.clone(), on));
                    }
                    ui.label(RichText::new(&r.name).strong().color(if r.enabled { t.text } else { t.text_dim }));
                    if several && let Some(a) = account_name(&r.account_id) {
                        ui.label(RichText::new(a).size(12.0).color(t.text_dim));
                    }
                });
                ui.label(RichText::new(describe(r, r.move_to.as_deref().and_then(folder_name).as_deref())).size(12.0).color(t.text_dim));
                ui.horizontal(|ui| {
                    if ui.button("Bewerken").clicked() {
                        action = Some(Action::Edit(r.id.clone()));
                    }
                    if ui.button("Regel verwijderen").clicked() {
                        action = Some(Action::Delete(r.id.clone()));
                    }
                });
            });
            ui.add_space(4.0);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(open_folder.is_some() && !rules.is_empty(), egui::Button::new("Regels nu uitvoeren op deze map")).clicked() {
                action = Some(Action::Run);
            }
            if ui.button("Sluiten").clicked() {
                close = true;
            }
        });
    });
    if save && let Some(form) = form_state.as_mut() {
        match app.engine.run("rule.save", &form.params()) {
            Ok(()) => form_state = None,
            Err(e) => form.error = Some(save_error(&e)),
        }
    }
    if cancel {
        form_state = None;
    }
    match action {
        Some(Action::New) => {
            let account = app.engine.session.current_account().map(|a| a.id.clone()).unwrap_or_default();
            form_state = Some(Form::new(&account));
        }
        Some(Action::FromMessage) => {
            app.open_rules(true);
            return;
        }
        Some(Action::Edit(id)) => form_state = app.engine.session.rule(&id).map(Form::from_rule),
        Some(Action::Delete(id)) => app.run("rule.delete", json!({ "id": id })),
        Some(Action::Toggle(id, on)) => {
            if let Some(r) = app.engine.session.rule(&id).cloned() {
                let mut form = Form::from_rule(&r);
                form.enabled = on;
                app.run("rule.save", form.params());
            }
        }
        Some(Action::Run) => {
            app.run("rule.run", serde_json::Value::Null);
            let n = app.engine.session.take_rule_hits();
            app.notice = Some(if n == 0 {
                crate::Notice::info("Geen berichten in deze map waar een regel op past.")
            } else {
                crate::Notice::success(rule_notice(n))
            });
        }
        None => {}
    }
    if close {
        app.rules = None;
    } else if let Some(state) = app.rules.as_mut() {
        state.form = form_state;
    }
}

/// "Regels toegepast op 3 berichten."
pub fn rule_notice(n: usize) -> String {
    if n == 1 { "Regels toegepast op 1 bericht.".to_string() } else { format!("Regels toegepast op {n} berichten.") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rule_reads_as_one_line() {
        let r = Rule {
            id: "r".into(),
            name: "Facturen".into(),
            account_id: "a".into(),
            field: RuleField::Subject,
            contains: "factuur".into(),
            move_to: Some("a/Facturen".into()),
            mark_read: true,
            flag: false,
            categories: Vec::new(),
            enabled: true,
        };
        assert_eq!(describe(&r, Some("Facturen")), "Onderwerp bevat “factuur” → naar Facturen, markeren als gelezen");
        let labelled = Rule { categories: vec!["Rood".into(), "Werk".into()], ..r.clone() };
        assert_eq!(describe(&labelled, Some("Facturen")), "Onderwerp bevat “factuur” → naar Facturen, markeren als gelezen, categorie Rood, Werk");
        let f = Form::from_rule(&r);
        assert_eq!(f.params()["move_to"], "a/Facturen");
        assert_eq!(f.params()["field"], "subject");
        assert_eq!(rule_notice(1), "Regels toegepast op 1 bericht.");
    }
}
