//! Making, renaming, removing, emptying and marking folders read, and pinning a folder under
//! "Favorieten": the "Map" tab in the ribbon, the right-click menu on a folder row, and one small window for the name or the question
//! before something goes for good (a folder, its contents, or a message in the Deleted folder).

use crate::i18n::tr;
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
            self.notice = Some(crate::Notice::info(special()));
            return;
        }
        self.folder_edit = Some(FolderEdit::new(Action::Rename { id }, f.name));
    }

    /// Opens the window that asks before an ordinary folder is removed.
    pub fn delete_folder(&mut self, id: String) {
        let Some(f) = self.engine.session.folder(&id).cloned() else { return };
        if f.kind != FolderKind::Other {
            self.notice = Some(crate::Notice::info(special()));
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

fn special() -> String {
    tr("folder-special")
}

/// The error of a folder command, as a sentence.
pub fn describe(e: &CommandError) -> String {
    match e {
        CommandError::Param("name") => tr("folder-name-rule").to_string(),
        CommandError::Param("id") => special(),
        CommandError::NoSelection => tr("folder-add-account-first").to_string(),
        CommandError::NotFound => tr("folder-gone").to_string(),
        e => e.to_string(),
    }
}

/// The "Map" tab: new folder, rename and delete for the selected folder.
pub fn ribbon(app: &mut MailApp, ui: &mut egui::Ui, t: &Tokens) {
    let selected = app.engine.session.selected_folder.clone();
    let ordinary = selected.as_deref().and_then(|id| app.engine.session.folder(id)).is_some_and(|f| f.kind == FolderKind::Other);
    if ribbon_action_hint(ui, t, "📁", &tr("folder-new"), !app.engine.session.accounts.is_empty(), Some(&tr("folder-new-hint"))) {
        app.new_folder(None);
    }
    crate::ribbon_separator(ui);
    if ribbon_action_hint(ui, t, "✎", &tr("folder-rename"), ordinary, Some(&tr("folder-rename-hint")))
        && let Some(id) = selected.clone()
    {
        app.rename_folder(id);
    }
    if ribbon_action_hint(ui, t, "🗑", &tr("folder-delete"), ordinary, Some(&tr("folder-delete-hint")))
        && let Some(id) = selected.clone()
    {
        app.delete_folder(id);
    }
    crate::ribbon_separator(ui);
    let (has_mail, has_unread) = selected
        .as_deref()
        .map(|id| (app.engine.session.messages.iter().any(|m| m.folder_id == id), app.engine.session.folder(id).is_some_and(|f| f.unread > 0)))
        .unwrap_or((false, false));
    if ribbon_action_hint(ui, t, "🗑", &tr("folder-empty"), has_mail, Some(&tr("folder-empty-hint")))
        && let Some(id) = selected.clone()
    {
        app.empty_folder(id);
    }
    if ribbon_action_hint(ui, t, "◉", &tr("folder-all-read"), has_unread, Some(&tr("folder-all-read-hint")))
        && let Some(id) = selected.clone()
    {
        app.run("folder.mark_all_read", json!({ "id": id }));
    }
    let deleted = selected.as_deref().and_then(|id| app.engine.session.folder(id)).is_some_and(|f| f.kind == FolderKind::Deleted);
    if ribbon_action_hint(ui, t, "♻", &tr("folder-clean-up"), has_mail && !deleted, Some(&tr("folder-clean-up-hint")))
        && let Some(id) = selected.clone()
    {
        app.ask_cleanup_folder(&id);
    }
    crate::ribbon_separator(ui);
    let favorite = selected.as_deref().is_some_and(|id| app.is_favorite(id));
    let hint = tr(if favorite { "folder-favorite-off-hint" } else { "folder-favorite-on-hint" });
    if ribbon_action_hint(ui, t, if favorite { "★" } else { "☆" }, &tr("folder-favorite"), selected.is_some(), Some(&hint))
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
        if ui.button(if pinned { tr("folder-unpin") } else { tr("folder-pin") }).clicked() {
            favorite = true;
            ui.close();
        }
        ui.separator();
        if ui.button(tr("folder-new-menu")).clicked() {
            action = Some(Action::Create { account_id: String::new(), parent: Some(id.to_string()) });
            ui.close();
        }
        if kind == FolderKind::Other {
            if ui.button(tr("folder-rename-menu")).clicked() {
                action = Some(Action::Rename { id: id.to_string() });
                ui.close();
            }
            if ui.button(tr("folder-delete-menu")).clicked() {
                action = Some(Action::Delete { id: id.to_string() });
                ui.close();
            }
        }
        ui.separator();
        if ui.button(tr("folders-mark-all-read")).clicked() {
            mark_read = true;
            ui.close();
        }
        if kind != FolderKind::Deleted && ui.button(tr("folder-clean-up-menu")).clicked() {
            cleanup = true;
            ui.close();
        }
        if ui.button(tr("folder-empty-menu")).clicked() {
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
        Action::Create { .. } => tr("folder-new"),
        Action::Rename { .. } => tr("folder-rename-title"),
        Action::Delete { .. } => tr("folder-delete"),
        Action::Empty { .. } => tr("folder-empty"),
        Action::DeleteForever { .. } => tr("delete-forever"),
    };
    let place = match &edit.action {
        Action::Create { account_id, parent } => match parent.as_deref().and_then(|p| app.engine.session.folder(p)) {
            Some(f) => crate::i18n::tr_args("folder-in", &[("folder", f.name.as_str().into())]),
            None => {
                let email = app.engine.session.accounts.iter().find(|a| &a.id == account_id).map(|a| a.address.email().to_string());
                crate::i18n::tr_args("folder-top-of-account", &[("account", email.unwrap_or_default().into())])
            }
        },
        _ => String::new(),
    };
    let mut submit = false;
    let mut close = false;
    if crate::helder::popup(ctx, t, "folder-edit", &title, 300.0, |ui| {
        match &edit.action {
            Action::Delete { .. } => {
                ui.label(RichText::new(crate::i18n::tr_args("folder-delete-confirm", &[("folder", edit.name.as_str().into())])).color(t.text));
            }
            Action::Empty { .. } => {
                ui.label(RichText::new(crate::i18n::tr_args("folder-empty-confirm", &[("folder", edit.name.as_str().into())])).color(t.text));
            }
            Action::DeleteForever { .. } => {
                ui.label(RichText::new(crate::i18n::tr_args("delete-forever-confirm", &[("name", edit.name.as_str().into())])).color(t.text));
            }
            _ => {
                if !place.is_empty() {
                    ui.label(RichText::new(place.as_str()).size(12.0).color(t.text_dim));
                }
                let field = ui.add(egui::TextEdit::singleline(&mut edit.name).desired_width(280.0).hint_text(tr("folder-name")));
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
                Action::Create { .. } => tr("folder-create"),
                Action::Rename { .. } => tr("save"),
                Action::Delete { .. } | Action::DeleteForever { .. } => tr("yes-delete"),
                Action::Empty { .. } => tr("yes-empty"),
            };
            if ui.add(crate::helder::primary_text(t, &ok)).clicked() {
                submit = true;
            }
            if ui.button(tr("cancel")).clicked() {
                close = true;
            }
        });
    }) {
        close = true;
    }
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
