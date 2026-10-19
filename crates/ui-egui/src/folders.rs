//! Making, renaming and removing folders: the "Map" tab in the ribbon, the right-click menu on
//! a folder row, and one small window for the name or the confirmation.

use egui::RichText;
use mailcraft_engine::CommandError;
use mailcraft_model::FolderKind;
use serde_json::json;

use crate::{MailApp, Tokens, ribbon_action_hint};

/// What the window is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// A new folder under `parent`, or at the top of the account when there is none.
    Create {
        account_id: String,
        parent: Option<String>,
    },
    Rename {
        id: String,
    },
    Delete {
        id: String,
    },
}

/// The open window: what it does, the name being typed and the last error.
#[derive(Clone, Debug)]
pub struct FolderEdit {
    pub action: Action,
    pub name: String,
    pub error: Option<String>,
    /// Whether the window has yet to put the cursor in the name field.
    fresh: bool,
}

impl FolderEdit {
    fn new(action: Action, name: String) -> Self {
        FolderEdit { action, name, error: None, fresh: true }
    }
}

impl MailApp {
    /// Opens the window for a new folder under `parent`, else under the selected folder, else
    /// at the top of the current account.
    pub fn new_folder(&mut self, parent: Option<String>) {
        let parent = parent.or_else(|| self.engine.session.selected_folder.clone());
        let account_id = match parent.as_deref().and_then(|p| self.engine.session.folder(p)) {
            Some(f) => f.account_id.clone(),
            None => match self.engine.session.current_account() {
                Some(a) => a.id.clone(),
                None => return,
            },
        };
        self.folder_edit = Some(FolderEdit::new(Action::Create { account_id, parent }, String::new()));
    }

    /// Opens the window to rename an ordinary folder; a special folder gives a notice instead.
    pub fn rename_folder(&mut self, id: String) {
        let Some(f) = self.engine.session.folder(&id).cloned() else { return };
        if f.kind != FolderKind::Other {
            self.notice = Some(SPECIAL.to_string());
            return;
        }
        self.folder_edit = Some(FolderEdit::new(Action::Rename { id }, f.name));
    }

    /// Opens the window that asks before an ordinary folder is removed.
    pub fn delete_folder(&mut self, id: String) {
        let Some(f) = self.engine.session.folder(&id).cloned() else { return };
        if f.kind != FolderKind::Other {
            self.notice = Some(SPECIAL.to_string());
            return;
        }
        self.folder_edit = Some(FolderEdit::new(Action::Delete { id }, f.name));
    }
}

const SPECIAL: &str = "Deze map hoort bij het account en kan niet worden hernoemd of verwijderd.";

/// The error of a folder command, as a sentence.
pub fn describe(e: &CommandError) -> String {
    match e {
        CommandError::Param("name") => "Geef de map een naam van hoogstens 80 tekens, zonder / of \\, die nog niet bestaat.".to_string(),
        CommandError::Param("id") => SPECIAL.to_string(),
        CommandError::NoSelection => "Voeg eerst een account toe.".to_string(),
        CommandError::NotFound => "Die map bestaat niet meer.".to_string(),
        e => e.to_string(),
    }
}

/// The "Map" tab: new folder, rename and delete for the selected folder.
pub fn ribbon(app: &mut MailApp, ui: &mut egui::Ui, t: &Tokens) {
    let selected = app.engine.session.selected_folder.clone();
    let ordinary = selected.as_deref().and_then(|id| app.engine.session.folder(id)).is_some_and(|f| f.kind == FolderKind::Other);
    if ribbon_action_hint(ui, t, "📁", "Nieuwe map", !app.engine.session.accounts.is_empty(), Some("Een map onder de geselecteerde map")) {
        app.new_folder(None);
    }
    ui.separator();
    if ribbon_action_hint(ui, t, "✎", "Naam wijzigen", ordinary, Some("De geselecteerde map een andere naam geven"))
        && let Some(id) = selected.clone()
    {
        app.rename_folder(id);
    }
    if ribbon_action_hint(ui, t, "🗑", "Map verwijderen", ordinary, Some("De geselecteerde map met alles erin verwijderen"))
        && let Some(id) = selected
    {
        app.delete_folder(id);
    }
}

/// The right-click menu of a folder row.
pub fn context_menu(app: &mut MailApp, r: &egui::Response, id: &str, kind: FolderKind) {
    let mut action: Option<Action> = None;
    r.context_menu(|ui| {
        if ui.button("Nieuwe map…").clicked() {
            action = Some(Action::Create { account_id: String::new(), parent: Some(id.to_string()) });
            ui.close();
        }
        if kind == FolderKind::Other {
            if ui.button("Naam wijzigen…").clicked() {
                action = Some(Action::Rename { id: id.to_string() });
                ui.close();
            }
            if ui.button("Map verwijderen…").clicked() {
                action = Some(Action::Delete { id: id.to_string() });
                ui.close();
            }
        }
    });
    match action {
        Some(Action::Create { parent, .. }) => app.new_folder(parent),
        Some(Action::Rename { id }) => app.rename_folder(id),
        Some(Action::Delete { id }) => app.delete_folder(id),
        None => {}
    }
}

/// The window for the open action, if any.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(mut edit) = app.folder_edit.clone() else { return };
    let title = match &edit.action {
        Action::Create { .. } => "Nieuwe map",
        Action::Rename { .. } => "Naam van map wijzigen",
        Action::Delete { .. } => "Map verwijderen",
    };
    let place = match &edit.action {
        Action::Create { account_id, parent } => match parent.as_deref().and_then(|p| app.engine.session.folder(p)) {
            Some(f) => format!("In de map {}", f.name),
            None => {
                let email = app.engine.session.accounts.iter().find(|a| &a.id == account_id).map(|a| a.address.email().to_string());
                format!("Bovenin het account {}", email.unwrap_or_default())
            }
        },
        _ => String::new(),
    };
    let mut submit = false;
    let mut close = false;
    egui::Window::new(title).collapsible(false).resizable(false).default_width(300.0).show(ctx, |ui| {
        match &edit.action {
            Action::Delete { .. } => {
                ui.label(RichText::new(format!("De map “{}” en alle berichten erin worden verwijderd.", edit.name)).color(t.text));
            }
            _ => {
                if !place.is_empty() {
                    ui.label(RichText::new(place).size(12.0).color(t.text_dim));
                }
                let field = ui.add(egui::TextEdit::singleline(&mut edit.name).desired_width(280.0).hint_text("Naam van de map"));
                if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submit = true;
                }
                if edit.fresh {
                    field.request_focus();
                    edit.fresh = false;
                }
            }
        }
        if let Some(e) = &edit.error {
            ui.label(RichText::new(e).color(t.flag));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let ok = match &edit.action {
                Action::Create { .. } => "Map aanmaken",
                Action::Rename { .. } => "Opslaan",
                Action::Delete { .. } => "Ja, verwijderen",
            };
            if ui.button(ok).clicked() {
                submit = true;
            }
            if ui.button("Annuleren").clicked() {
                close = true;
            }
        });
    });
    if submit {
        let params = match &edit.action {
            Action::Create { account_id, parent } => json!({ "name": edit.name, "parent": parent, "account_id": account_id }),
            Action::Rename { id } => json!({ "id": id, "name": edit.name }),
            Action::Delete { id } => json!({ "id": id }),
        };
        let command = match &edit.action {
            Action::Create { .. } => "folder.create",
            Action::Rename { .. } => "folder.rename",
            Action::Delete { .. } => "folder.delete",
        };
        match app.engine.run(command, &params) {
            Ok(()) => close = true,
            Err(e) => edit.error = Some(describe(&e)),
        }
    }
    app.folder_edit = if close { None } else { Some(edit) };
}
