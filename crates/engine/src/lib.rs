//! The session (accounts, folders, messages, selection) and the command registry.
//!
//! Every user-visible action is a command id run through [`Session::run`]: the ribbon, keyboard
//! shortcuts and, later, scripting all dispatch the same way. That gives one place to validate
//! parameters and one list of what the app can do.

mod commands;
mod sample;

pub use commands::{COMMANDS, CommandError, CommandSpec};

use mailcraft_model::{Account, Folder, FolderKind, MessageHeader};

#[derive(Debug, Default)]
pub struct Session {
    pub accounts: Vec<Account>,
    pub folders: Vec<Folder>,
    pub messages: Vec<MessageHeader>,
    pub selected_folder: Option<String>,
    pub selected_message: Option<String>,
}

impl Session {
    /// A session with built-in sample mail, for first run and tests. No network.
    pub fn sample() -> Self {
        let mut s = sample::session();
        s.selected_folder = s.folders.iter().find(|f| f.kind == FolderKind::Inbox).map(|f| f.id.clone());
        s.recount_unread();
        s
    }

    pub fn folder(&self, id: &str) -> Option<&Folder> {
        self.folders.iter().find(|f| f.id == id)
    }

    pub fn message(&self, id: &str) -> Option<&MessageHeader> {
        self.messages.iter().find(|m| m.id == id)
    }

    /// Messages in the selected folder, newest first.
    pub fn visible_messages(&self) -> Vec<&MessageHeader> {
        let Some(folder) = &self.selected_folder else { return Vec::new() };
        let mut list: Vec<&MessageHeader> = self.messages.iter().filter(|m| &m.folder_id == folder).collect();
        list.sort_by_key(|m| std::cmp::Reverse(m.date));
        list
    }

    pub fn selected(&self) -> Option<&MessageHeader> {
        self.selected_message.as_deref().and_then(|id| self.message(id))
    }

    fn folder_of_kind(&self, account_id: &str, kind: FolderKind) -> Option<&Folder> {
        self.folders.iter().find(|f| f.account_id == account_id && f.kind == kind)
    }

    fn recount_unread(&mut self) {
        for f in &mut self.folders {
            let n = self.messages.iter().filter(|m| m.folder_id == f.id && !m.flags.seen).count();
            f.unread = u32::try_from(n).unwrap_or(u32::MAX);
        }
    }
}
