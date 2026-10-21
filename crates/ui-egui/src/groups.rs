//! Contact groups in the contacts module, as Outlook's "Contactgroep": a list under the
//! contacts, a card with the members, and a window to make or change one. A group's name typed
//! in an address field offers its members; see `contacts::complete_addresses`.

use egui::{RichText, Ui, vec2};
use serde_json::json;

use merel_engine::CommandError;
use merel_model::{ContactGroup, mailbox};

use crate::{ComposeKind, MailApp, Tokens};

pub const HEADING: &str = "Contactgroepen";
pub const NEW: &str = "Nieuwe groep";
pub const MAIL: &str = "E-mail aan groep";

/// The window's fields as text.
#[derive(Default)]
pub struct Editor {
    pub id: Option<String>,
    pub name: String,
    /// The members as an address line: `Naam <adres>; adres`, line breaks allowed.
    pub members: String,
    pub error: Option<String>,
}

impl Editor {
    pub fn from_group(g: &ContactGroup) -> Self {
        Editor { id: Some(g.id.clone()), name: g.name.clone(), members: g.address_line(), error: None }
    }
}

/// The label of a group in the list and among the address suggestions.
pub fn label(g: &ContactGroup) -> String {
    format!("👥 {} ({})", g.name, g.members.len())
}

impl MailApp {
    pub fn new_group(&mut self) {
        self.contacts.group_editor = Some(Editor::default());
    }

    pub fn edit_group(&mut self, id: &str) {
        if let Some(g) = self.engine.session.group(id) {
            self.contacts.group_editor = Some(Editor::from_group(g));
        }
    }

    /// A new message to every member of the group.
    pub fn mail_group(&mut self, id: &str) {
        let Some(line) = self.engine.session.group(id).map(ContactGroup::address_line) else { return };
        self.open_compose(ComposeKind::New);
        if let Some(c) = self.compose.as_mut() {
            c.to = format!("{line}; ");
        }
    }
}

fn describe(e: &CommandError) -> String {
    match e {
        CommandError::Param("name") => "Vul een naam in.".into(),
        CommandError::Param("members") => format!("Vul bij Leden ten minste één geldig adres in, hoogstens {}.", merel_model::MAX_GROUP_MEMBERS),
        CommandError::NotFound => "Deze groep bestaat niet meer.".into(),
        _ => "Opslaan is niet gelukt.".into(),
    }
}

/// The groups under the contacts in the list pane; returns the one clicked.
pub fn section(ui: &mut Ui, t: &Tokens, groups: &[ContactGroup], selected: Option<&str>) -> Option<String> {
    if groups.is_empty() {
        return None;
    }
    let mut pick = None;
    ui.add_space(10.0);
    ui.label(RichText::new(HEADING).size(12.0).strong().color(t.accent_text));
    for g in groups {
        if ui.selectable_label(selected == Some(g.id.as_str()), RichText::new(label(g)).color(t.text)).clicked() {
            pick = Some(g.id.clone());
        }
    }
    pick
}

/// The card for the selected group: its members and the buttons to mail, change or remove it.
pub fn card(app: &mut MailApp, ui: &mut Ui, t: &Tokens, id: &str) {
    let Some(g) = app.engine.session.group(id).cloned() else {
        app.contacts.group = None;
        return;
    };
    ui.label(RichText::new(&g.name).size(22.0).strong().color(t.text));
    let n = g.members.len();
    ui.label(RichText::new(format!("Contactgroep met {n} {}", if n == 1 { "lid" } else { "leden" })).color(t.text_dim));
    ui.add_space(14.0);
    for m in &g.members {
        ui.label(RichText::new(mailbox(m)).color(t.text));
    }
    ui.add_space(14.0);
    let (mut mail, mut edit, mut delete) = (false, false, false);
    ui.horizontal(|ui| {
        mail = ui.add(egui::Button::new(RichText::new(MAIL).color(t.on_accent)).fill(t.accent).min_size(vec2(130.0, 26.0))).clicked();
        edit = ui.add(egui::Button::new("Bewerken").min_size(vec2(90.0, 26.0))).clicked();
        delete = ui
            .add(egui::Button::new("Groep verwijderen").min_size(vec2(90.0, 26.0)))
            .on_hover_text("Alleen de groep verdwijnt; de contacten blijven.")
            .clicked();
    });
    if mail {
        app.mail_group(id);
    }
    if edit {
        app.edit_group(id);
    }
    if delete {
        app.run("group.delete", json!({ "id": id }));
        app.contacts.group = None;
    }
}

/// The window to make or change a group.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(editor) = app.contacts.group_editor.as_mut() else { return };
    let mut save = false;
    let mut close = false;
    let title = if editor.id.is_some() { "Contactgroep bewerken" } else { "Nieuwe contactgroep" };
    egui::Window::new(title).collapsible(false).resizable(false).default_width(470.0).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.add_sized([90.0, 22.0], egui::Label::new(RichText::new("Naam").color(t.text)));
            ui.add(egui::TextEdit::singleline(&mut editor.name).desired_width(340.0));
        });
        ui.horizontal(|ui| {
            ui.add_sized([90.0, 22.0], egui::Label::new(RichText::new("Leden").color(t.text)));
            ui.add(egui::TextEdit::multiline(&mut editor.members).desired_width(340.0).desired_rows(5));
        });
        crate::contacts::complete_addresses(ui, t, &app.engine.session, &mut editor.members);
        ui.label(
            RichText::new("Adressen gescheiden door ; of elk op een eigen regel. Typ een naam voor suggesties uit je contacten.")
                .size(11.5)
                .color(t.text_dim),
        );
        if let Some(e) = &editor.error {
            ui.label(RichText::new(e).color(t.flag));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            save = ui.add(egui::Button::new(RichText::new("Opslaan").color(t.on_accent)).fill(t.accent).min_size(vec2(90.0, 26.0))).clicked();
            close = ui.add(egui::Button::new("Annuleren").min_size(vec2(90.0, 26.0))).clicked();
        });
    });
    if save {
        let mut params = json!({ "name": editor.name, "members": editor.members });
        if let Some(id) = &editor.id {
            params["id"] = json!(id);
        }
        let name = editor.name.trim().to_string();
        match app.engine.run("group.save", &params) {
            Ok(()) => {
                // Show the group that was just saved.
                let id = editor.id.clone().or_else(|| app.engine.session.groups.iter().rev().find(|g| g.name == name).map(|g| g.id.clone()));
                app.contacts.group = id;
                app.contacts.editor = None;
                close = true;
            }
            Err(e) => editor.error = Some(describe(&e)),
        }
    }
    if close {
        app.contacts.group_editor = None;
    }
}
