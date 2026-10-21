//! Making, renaming, removing, emptying and marking folders read, and pinning a folder under
//! "Favorieten": the "Map" tab in the ribbon, the right-click menu on a folder row, and one small window for the name or the question
//! before something goes for good (a folder, its contents, or a message in the Deleted folder).

use egui::RichText;
use merel_engine::CommandError;
use merel_model::FolderKind;
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
    /// Every message in the folder goes for good.
    Empty {
        id: String,
    },
    /// A message in the Deleted folder goes for good.
    DeleteForever {
        message_id: String,
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
        // An "Alle accounts" folder is no place for a new one.
        let parent = parent.or_else(|| self.engine.session.selected_folder.clone()).filter(|p| self.engine.session.folder(p).is_some());
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
            self.notice = Some(crate::Notice::info(SPECIAL));
            return;
        }
        self.folder_edit = Some(FolderEdit::new(Action::Rename { id }, f.name));
    }

    /// Opens the window that asks before an ordinary folder is removed.
    pub fn delete_folder(&mut self, id: String) {
        let Some(f) = self.engine.session.folder(&id).cloned() else { return };
        if f.kind != FolderKind::Other {
            self.notice = Some(crate::Notice::info(SPECIAL));
            return;
        }
        self.folder_edit = Some(FolderEdit::new(Action::Delete { id }, f.name));
    }
}

impl MailApp {
    /// Opens the window that asks before every message in a folder is removed.
    pub fn empty_folder(&mut self, id: String) {
        let Some(f) = self.engine.session.folder(&id).cloned() else { return };
        self.folder_edit = Some(FolderEdit::new(Action::Empty { id }, f.name));
    }

    /// Opens the window that asks before the selected message, already in the Deleted folder,
    /// is removed for good.
    pub fn delete_forever(&mut self) {
        let Some(m) = self.engine.session.selected().cloned() else { return };
        self.folder_edit = Some(FolderEdit::new(Action::DeleteForever { message_id: m.id }, m.subject));
    }

    /// Whether the selected message sits in the Deleted folder, so deleting it is for good.
    pub fn selected_is_deleted(&self) -> bool {
        self.engine.session.selected().and_then(|m| self.engine.session.folder(&m.folder_id)).is_some_and(|f| f.kind == FolderKind::Deleted)
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
    crate::ribbon_separator(ui);
    if ribbon_action_hint(ui, t, "✎", "Naam wijzigen", ordinary, Some("De geselecteerde map een andere naam geven"))
        && let Some(id) = selected.clone()
    {
        app.rename_folder(id);
    }
    if ribbon_action_hint(ui, t, "🗑", "Map verwijderen", ordinary, Some("De geselecteerde map met alles erin verwijderen"))
        && let Some(id) = selected.clone()
    {
        app.delete_folder(id);
    }
    crate::ribbon_separator(ui);
    let (has_mail, has_unread) = selected
        .as_deref()
        .map(|id| (app.engine.session.messages.iter().any(|m| m.folder_id == id), app.engine.session.folder(id).is_some_and(|f| f.unread > 0)))
        .unwrap_or((false, false));
    if ribbon_action_hint(ui, t, "🗑", "Map leegmaken", has_mail, Some("Alle berichten in de geselecteerde map definitief verwijderen"))
        && let Some(id) = selected.clone()
    {
        app.empty_folder(id);
    }
    if ribbon_action_hint(ui, t, "◉", "Alles gelezen", has_unread, Some("Alle berichten in de geselecteerde map als gelezen markeren"))
        && let Some(id) = selected.clone()
    {
        app.run("folder.mark_all_read", json!({ "id": id }));
    }
    let deleted = selected.as_deref().and_then(|id| app.engine.session.folder(id)).is_some_and(|f| f.kind == FolderKind::Deleted);
    if ribbon_action_hint(
        ui,
        t,
        "♻",
        "Opruimen",
        has_mail && !deleted,
        Some("Oudere berichten van elk gesprek in de geselecteerde map naar Verwijderde items"),
    ) && let Some(id) = selected.clone()
    {
        app.ask_cleanup_folder(&id);
    }
    crate::ribbon_separator(ui);
    let favorite = selected.as_deref().is_some_and(|id| app.is_favorite(id));
    let hint =
        if favorite { "De geselecteerde map uit Favorieten halen" } else { "De geselecteerde map bovenin het mappenvenster onder Favorieten zetten" };
    if ribbon_action_hint(ui, t, if favorite { "★" } else { "☆" }, "Favoriet", selected.is_some(), Some(hint))
        && let Some(id) = selected
    {
        app.toggle_favorite(&id, ui.ctx());
    }
}

/// The right-click menu of a folder row.
pub fn context_menu(app: &mut MailApp, r: &egui::Response, id: &str, kind: FolderKind) {
    let mut action: Option<Action> = None;
    let mut mark_read = false;
    let mut cleanup = false;
    let mut favorite = false;
    let pinned = app.is_favorite(id);
    r.context_menu(|ui| {
        if ui.button(if pinned { "Uit Favorieten verwijderen" } else { "Toevoegen aan Favorieten" }).clicked() {
            favorite = true;
            ui.close();
        }
        ui.separator();
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
        ui.separator();
        if ui.button("Alles als gelezen markeren").clicked() {
            mark_read = true;
            ui.close();
        }
        if kind != FolderKind::Deleted && ui.button("Map opruimen…").clicked() {
            cleanup = true;
            ui.close();
        }
        if ui.button("Map leegmaken…").clicked() {
            action = Some(Action::Empty { id: id.to_string() });
            ui.close();
        }
    });
    if cleanup {
        app.ask_cleanup_folder(id);
    }
    if favorite {
        app.toggle_favorite(id, &r.ctx);
    }
    if mark_read {
        app.run("folder.mark_all_read", json!({ "id": id }));
    }
    match action {
        Some(Action::Create { parent, .. }) => app.new_folder(parent),
        Some(Action::Rename { id }) => app.rename_folder(id),
        Some(Action::Delete { id }) => app.delete_folder(id),
        Some(Action::Empty { id }) => app.empty_folder(id),
        Some(Action::DeleteForever { .. }) | None => {}
    }
}

/// The window for the open action, if any.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(mut edit) = app.folder_edit.clone() else { return };
    let title = match &edit.action {
        Action::Create { .. } => "Nieuwe map",
        Action::Rename { .. } => "Naam van map wijzigen",
        Action::Delete { .. } => "Map verwijderen",
        Action::Empty { .. } => "Map leegmaken",
        Action::DeleteForever { .. } => "Definitief verwijderen",
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
            Action::Empty { .. } => {
                ui.label(
                    RichText::new(format!("Alle berichten in “{}” worden definitief verwijderd, ook wat nog niet in de lijst staat.", edit.name))
                        .color(t.text),
                );
            }
            Action::DeleteForever { .. } => {
                ui.label(RichText::new(format!("“{}” wordt definitief verwijderd en is dan niet meer terug te halen.", edit.name)).color(t.text));
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
                Action::Delete { .. } | Action::DeleteForever { .. } => "Ja, verwijderen",
                Action::Empty { .. } => "Ja, leegmaken",
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
            Action::Delete { id } | Action::Empty { id } => json!({ "id": id }),
            Action::DeleteForever { message_id } => json!({ "id": message_id }),
        };
        let command = match &edit.action {
            Action::Create { .. } => "folder.create",
            Action::Rename { .. } => "folder.rename",
            Action::Delete { .. } => "folder.delete",
            Action::Empty { .. } => "folder.empty",
            Action::DeleteForever { .. } => "message.delete",
        };
        match app.engine.run(command, &params) {
            Ok(()) => {
                close = true;
                // A new subfolder shows: its parent folds open, as in Outlook.
                if let Action::Create { parent: Some(parent), .. } = &edit.action
                    && !app.view.open_folders.contains(parent)
                {
                    app.view.open_folders.push(parent.clone());
                    app.save_view(ctx);
                }
            }
            Err(e) => edit.error = Some(describe(&e)),
        }
    }
    app.folder_edit = if close { None } else { Some(edit) };
}
