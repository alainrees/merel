//! Quick steps, as Outlook has them: the person's own buttons on the "Start" tab that do several
//! things to the selected message at once, also on Ctrl+Shift+1 to 9, and the window to make
//! and change them.

use crate::i18n::tr;
use egui::{RichText, Ui, vec2};
use serde_json::json;

use merel_engine::CommandError;
use merel_model::QuickStep;

use crate::{MailApp, Tokens, key_hint};

/// What the window remembers between frames.
#[derive(Default)]
pub struct State {
    pub form: Option<Form>,
}

/// The quick step being made or changed, as the person edits it.
#[derive(Clone, Debug, Default)]
pub struct Form {
    pub id: Option<String>,
    pub name: String,
    pub account_id: String,
    pub move_to: Option<String>,
    pub mark_read: bool,
    pub flag: bool,
    pub categories: Vec<String>,
    pub error: Option<String>,
}

impl Form {
    pub fn new(account_id: &str) -> Self {
        Form { account_id: account_id.to_string(), ..Form::default() }
    }

    pub fn from_step(q: &QuickStep) -> Self {
        Form {
            id: Some(q.id.clone()),
            name: q.name.clone(),
            account_id: q.account_id.clone(),
            move_to: q.move_to.clone(),
            mark_read: q.mark_read,
            flag: q.flag,
            categories: q.categories.clone(),
            error: None,
        }
    }

    /// The `quickstep.save` parameters.
    pub fn params(&self) -> serde_json::Value {
        let mut p = json!({
            "name": self.name,
            "account_id": self.account_id,
            "move_to": self.move_to,
            "mark_read": self.mark_read,
            "flag": self.flag,
            "categories": self.categories,
        });
        if let Some(id) = &self.id {
            p["id"] = json!(id);
        }
        p
    }
}

/// One line that says what a step does: "naar Archief, markeren als gelezen".
pub fn describe(q: &QuickStep, folder_name: Option<&str>) -> String {
    let mut actions = Vec::new();
    if q.move_to.is_some() {
        actions.push(format!("naar {}", folder_name.unwrap_or("een map die er niet meer is")));
    }
    if q.mark_read {
        actions.push("markeren als gelezen".to_string());
    }
    if q.flag {
        actions.push("vlag zetten".to_string());
    }
    if !q.categories.is_empty() {
        actions.push(format!("categorie {}", q.categories.join(", ")));
    }
    actions.join(", ")
}

impl MailApp {
    pub fn open_quick_steps(&mut self) {
        self.quick_steps = Some(State::default());
    }

    /// The account the selected message belongs to, else the current account.
    fn quick_step_account(&self) -> Option<String> {
        let s = &self.engine.session;
        s.selected().and_then(|m| s.folder(&m.folder_id)).map(|f| f.account_id.clone()).or_else(|| s.current_account().map(|a| a.id.clone()))
    }

    /// Runs the `n`-th (from 0) quick step of the selected message's account, as Ctrl+Shift+1
    /// to 9 do; nothing happens without a message or a step.
    pub fn run_quick_step(&mut self, n: usize) {
        if self.engine.session.selected_message.is_none() {
            return;
        }
        let Some(account) = self.quick_step_account() else { return };
        let Some(id) = self.engine.session.quick_steps_of(&account).get(n).map(|q| q.id.clone()) else { return };
        self.run_quick_step_id(&id);
    }

    /// Runs one quick step on the selected message; what went wrong becomes a notice.
    pub fn run_quick_step_id(&mut self, id: &str) {
        self.notice = match self.engine.run("quickstep.run", &json!({ "id": id })) {
            Ok(()) => None,
            Err(e @ (CommandError::NoSelection | CommandError::Param(_))) => Some(crate::Notice::warning(run_error(&e))),
            Err(e) => Some(crate::Notice::error(run_error(&e))),
        };
    }
}

fn run_error(e: &CommandError) -> String {
    match e {
        CommandError::NoSelection => "Selecteer eerst een bericht.".into(),
        CommandError::Param("move_to") => "De map van deze snelle stap bestaat niet meer; de rest is wel gedaan.".into(),
        CommandError::Param("id") => "Deze snelle stap hoort bij een ander account.".into(),
        e => e.to_string(),
    }
}

fn save_error(e: &CommandError) -> String {
    match e {
        CommandError::Param("name") => "Geef de snelle stap een naam van hoogstens 40 tekens.".into(),
        CommandError::Param("move_to") => {
            "Kies wat de snelle stap moet doen: verplaatsen, markeren als gelezen, een vlag zetten of een categorie geven.".into()
        }
        CommandError::Param("categories") => "Kies categorieën uit de lijst.".into(),
        CommandError::Param("account_id") => "Kies een account.".into(),
        CommandError::Store(m) => format!("Opslaan is mislukt: {m}"),
        e => e.to_string(),
    }
}

/// The "Snelle stappen" group in the ribbon: a small button per step of the current account
/// (the first nine also answer to Ctrl+Shift+1 to 9) and one to manage them.
pub fn ribbon(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let steps: Vec<(String, String)> = app
        .quick_step_account()
        .map(|a| app.engine.session.quick_steps_of(&a).iter().map(|q| (q.id.clone(), q.name.clone())).collect())
        .unwrap_or_default();
    let has = app.engine.session.selected_message.is_some();
    let mut run: Option<String> = None;
    let mut manage = false;
    crate::view::section(ui, t, &tr("quick-steps"), |ui| {
        for (n, (id, name)) in steps.iter().enumerate() {
            let hint = (n < 9).then(|| key_hint(&format!("Shift+{}", n + 1)));
            if crate::ribbon_action_hint(ui, t, "⚡", name, has, hint.as_deref()) {
                run = Some(id.clone());
            }
        }
        if crate::ribbon_action_hint(ui, t, "⚙", &tr("quick-steps-manage"), true, Some(&tr("quick-steps-manage-hint"))) {
            manage = true;
        }
    });
    if let Some(id) = run {
        app.run_quick_step_id(&id);
    }
    if manage {
        app.open_quick_steps();
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
        ui.add(egui::TextEdit::singleline(&mut form.name).desired_width(280.0).hint_text("Bijvoorbeeld: Klaar"));
    });
    if accounts.len() > 1 {
        let current = accounts.iter().find(|(id, _)| *id == form.account_id).map(|(_, e)| e.as_str()).unwrap_or("");
        ui.horizontal(|ui| {
            label(ui, "Account");
            egui::ComboBox::from_id_salt("quickstep-account").selected_text(current).width(280.0).show_ui(ui, |ui| {
                for (id, email) in &accounts {
                    if ui.selectable_value(&mut form.account_id, id.clone(), email).changed() {
                        form.move_to = None;
                    }
                }
            });
        });
    }
    let folders: Vec<(String, String)> =
        app.engine.session.folders.iter().filter(|f| f.account_id == form.account_id).map(|f| (f.id.clone(), f.name.clone())).collect();
    ui.horizontal(|ui| {
        label(ui, "Doet");
        let mut moving = form.move_to.is_some();
        if ui.checkbox(&mut moving, "verplaatsen naar").changed() {
            form.move_to = if moving { folders.first().map(|(id, _)| id.clone()) } else { None };
        }
        let current = form.move_to.as_ref().and_then(|id| folders.iter().find(|(f, _)| f == id)).map(|(_, n)| n.as_str()).unwrap_or("map");
        ui.add_enabled_ui(moving, |ui| {
            egui::ComboBox::from_id_salt("quickstep-folder").selected_text(current).width(160.0).show_ui(ui, |ui| {
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
        save =
            ui.add(egui::Button::new(RichText::new("Snelle stap opslaan").color(t.on_accent)).fill(t.accent).min_size(vec2(130.0, 26.0))).clicked();
        cancel = ui.add(egui::Button::new("Annuleren").min_size(vec2(90.0, 26.0))).clicked();
    });
    (save, cancel)
}

enum Action {
    Edit(String),
    Delete(String),
    New,
}

/// The quick steps window, when it is open.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    if app.quick_steps.is_none() {
        return;
    }
    let steps: Vec<QuickStep> = app.engine.session.quick_steps.clone();
    let folder_name = |id: &str| app.engine.session.folder(id).map(|f| f.name.clone());
    let account_name = |id: &str| app.engine.session.accounts.iter().find(|a| a.id == id).map(|a| a.address.email().to_string());
    let several = app.engine.session.accounts.len() > 1;
    let mut action: Option<Action> = None;
    let mut save = false;
    let mut cancel = false;
    let mut close = false;
    let mut form_state = app.quick_steps.as_mut().and_then(|s| s.form.take());
    egui::Window::new("Snelle stappen").collapsible(false).resizable(false).default_width(460.0).show(ctx, |ui| {
        ui.label(
            RichText::new(format!(
                "Een snelle stap is een eigen knop op het tabblad Start die in één keer meerdere dingen met het geselecteerde bericht doet; de eerste negen ook met {} tot 9.",
                key_hint("Shift+1")
            ))
            .size(12.0)
            .color(t.text_dim),
        );
        ui.add_space(6.0);
        if let Some(form) = form_state.as_mut() {
            ui.label(RichText::new(if form.id.is_some() { "Snelle stap bewerken" } else { "Nieuwe snelle stap" }).strong().color(t.text));
            let (s, c) = form_ui(app, ui, t, form);
            save = s;
            cancel = c;
            return;
        }
        if ui.button("Nieuwe snelle stap").clicked() {
            action = Some(Action::New);
        }
        ui.add_space(6.0);
        if steps.is_empty() {
            ui.label(RichText::new("Nog geen snelle stappen.").color(t.text_dim));
        }
        for q in &steps {
            egui::Frame::new().fill(t.list).corner_radius(6).inner_margin(8).show(ui, |ui| {
                ui.set_width(440.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&q.name).strong().color(t.text));
                    if several && let Some(a) = account_name(&q.account_id) {
                        ui.label(RichText::new(a).size(12.0).color(t.text_dim));
                    }
                });
                ui.label(RichText::new(describe(q, q.move_to.as_deref().and_then(folder_name).as_deref())).size(12.0).color(t.text_dim));
                ui.horizontal(|ui| {
                    if ui.button("Bewerken").clicked() {
                        action = Some(Action::Edit(q.id.clone()));
                    }
                    if ui.button("Snelle stap verwijderen").clicked() {
                        action = Some(Action::Delete(q.id.clone()));
                    }
                });
            });
            ui.add_space(4.0);
        }
        ui.add_space(6.0);
        if ui.button("Sluiten").clicked() {
            close = true;
        }
    });
    if save && let Some(form) = form_state.as_mut() {
        match app.engine.run("quickstep.save", &form.params()) {
            Ok(()) => form_state = None,
            Err(e) => form.error = Some(save_error(&e)),
        }
    }
    if cancel {
        form_state = None;
    }
    match action {
        Some(Action::New) => {
            let account = app.quick_step_account().unwrap_or_default();
            form_state = Some(Form::new(&account));
        }
        Some(Action::Edit(id)) => form_state = app.engine.session.quick_step(&id).map(Form::from_step),
        Some(Action::Delete(id)) => app.run("quickstep.delete", json!({ "id": id })),
        None => {}
    }
    if close {
        app.quick_steps = None;
    } else if let Some(state) = app.quick_steps.as_mut() {
        state.form = form_state;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quick_step_reads_as_one_line() {
        let q = QuickStep {
            id: "q".into(),
            name: "Klaar".into(),
            account_id: "a".into(),
            move_to: Some("a/Archief".into()),
            mark_read: true,
            flag: false,
            categories: vec!["Rood".into()],
        };
        assert_eq!(describe(&q, Some("Archief")), "naar Archief, markeren als gelezen, categorie Rood");
        let f = Form::from_step(&q);
        assert_eq!(f.params()["move_to"], "a/Archief");
        assert_eq!(f.params()["categories"], json!(["Rood"]));
    }
}
