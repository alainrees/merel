//! "Opruimen", Outlook's Clean Up, for one conversation or a whole folder: the question first,
//! with how many messages would go to Verwijderde items and what stays, then the command. See
//! `merel_engine::cleanup` for what counts as redundant.

use crate::i18n::tr;
use egui::RichText;
use merel_model::{FolderKind, conversation_key};
use serde_json::json;

use crate::{MailApp, Tokens};

/// What is being cleaned up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    Conversation { message_id: String, subject: String },
    Folder { id: String, name: String },
}

/// The open question: the scope and how many messages would move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirm {
    pub scope: Scope,
    pub count: usize,
    /// The folder is Verwijderde items, which is never cleaned up.
    pub in_deleted: bool,
}

/// What the window says stays put.
pub fn keeps() -> String {
    tr("cleanup-keeps")
}

impl MailApp {
    /// Asks before cleaning up the conversation of a message.
    pub fn ask_cleanup_conversation(&mut self, message_id: &str) {
        let Some(m) = self.engine.session.message(message_id) else { return };
        let key = conversation_key(&m.subject);
        let in_deleted = self.engine.session.folder(&m.folder_id).is_some_and(|f| f.kind == FolderKind::Deleted);
        let count = if key.is_empty() || in_deleted { 0 } else { self.engine.session.redundant_messages(&m.folder_id, Some(&key)).len() };
        let scope = Scope::Conversation { message_id: message_id.to_string(), subject: m.subject.clone() };
        self.cleanup = Some(Confirm { scope, count, in_deleted });
    }

    /// Asks before cleaning up every conversation in a folder.
    pub fn ask_cleanup_folder(&mut self, id: &str) {
        let Some(f) = self.engine.session.folder(id) else { return };
        let in_deleted = f.kind == FolderKind::Deleted;
        let count = if in_deleted { 0 } else { self.engine.session.redundant_messages(id, None).len() };
        self.cleanup = Some(Confirm { scope: Scope::Folder { id: id.to_string(), name: f.name.clone() }, count, in_deleted });
    }
}

/// The question, while it is open.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(c) = app.cleanup.clone() else { return };
    let (title, what) = match &c.scope {
        Scope::Conversation { subject, .. } => {
            (tr("cleanup-conversation"), crate::i18n::tr_args("cleanup-the-conversation", &[("subject", subject.as_str().into())]))
        }
        Scope::Folder { name, .. } => (tr("cleanup-folder"), crate::i18n::tr_args("cleanup-the-folder", &[("folder", name.as_str().into())])),
    };
    let text = if c.in_deleted {
        tr("cleanup-in-deleted")
    } else {
        match c.count {
            0 => crate::i18n::tr_args("cleanup-nothing", &[("what", what.into())]),
            n => format!("{} {}", crate::i18n::tr_args("cleanup-count", &[("count", n.into()), ("what", what.into())]), keeps()),
        }
    };
    let mut go = false;
    let mut close = false;
    if crate::helder::popup(ctx, t, "cleanup", &title, 340.0, |ui| {
        ui.label(RichText::new(text).color(t.text));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if c.count > 0 && ui.add(crate::helder::primary_text(t, &tr("cleanup-yes"))).clicked() {
                go = true;
            }
            if ui.button(tr(if c.count > 0 { "cancel" } else { "close" })).clicked() {
                close = true;
            }
        });
    }) {
        close = true;
    }
    if go {
        match &c.scope {
            Scope::Conversation { message_id, .. } => app.run("conversation.cleanup", json!({ "id": message_id })),
            Scope::Folder { id, .. } => app.run("folder.cleanup", json!({ "id": id })),
        }
    }
    if go || close {
        app.cleanup = None;
    }
}
