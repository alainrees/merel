//! "Alle accounts": Outlook's folders at the top of the folder pane that show the mail of every
//! account together, one for each special folder (Postvak IN, Concepten, Verzonden items, …).
//!
//! Such a folder is a view, not a place: it holds no messages of its own and the server never
//! hears of it. Its id stands in `selected_folder` like a real one; the list then shows the
//! messages of the folders of that kind in every account, and anything done to a message (reply,
//! move, delete) goes to the account and folder the message really is in.

use merel_model::{Folder, FolderKind};

use crate::Session;

/// What every id of an "Alle accounts" folder starts with. An account id never does, so a
/// real folder (`{account}/{path}`) is never taken for one.
pub const PREFIX: &str = "*alle/";

/// The heading of the group in the folder pane.
pub const HEADING: &str = "Alle accounts";

/// The kinds the group lists, in the folder pane's order.
const KINDS: [(FolderKind, &str); 6] = [
    (FolderKind::Inbox, "inbox"),
    (FolderKind::Drafts, "drafts"),
    (FolderKind::Sent, "sent"),
    (FolderKind::Archive, "archive"),
    (FolderKind::Junk, "junk"),
    (FolderKind::Deleted, "deleted"),
];

/// The id of the "Alle accounts" folder of `kind`; `None` for an ordinary folder.
pub fn id(kind: FolderKind) -> Option<String> {
    KINDS.iter().find(|(k, _)| *k == kind).map(|(_, slug)| format!("{PREFIX}{slug}"))
}

/// The kind an "Alle accounts" folder id stands for; `None` for any other id.
pub fn kind_of(id: &str) -> Option<FolderKind> {
    let slug = id.strip_prefix(PREFIX)?;
    KINDS.iter().find(|(_, s)| *s == slug).map(|(k, _)| *k)
}

/// Whether `id` is an "Alle accounts" folder.
pub fn is_unified(id: &str) -> bool {
    kind_of(id).is_some()
}

impl Session {
    /// The "Alle accounts" folders, with the unread mail of every account added up; none while
    /// there is only one account, where they would only repeat its folders.
    pub fn unified_folders(&self) -> Vec<Folder> {
        if self.accounts.len() < 2 {
            return Vec::new();
        }
        KINDS
            .iter()
            .filter(|(kind, _)| self.folders.iter().any(|f| f.kind == *kind))
            .filter_map(|(kind, _)| self.unified_folder(&id(*kind)?))
            .collect()
    }

    /// The "Alle accounts" folder `id` names, as the folder pane and the list's heading show it.
    pub fn unified_folder(&self, id: &str) -> Option<Folder> {
        let kind = kind_of(id)?;
        let members: Vec<&Folder> = self.folders.iter().filter(|f| f.kind == kind).collect();
        if members.is_empty() {
            return None;
        }
        Some(Folder {
            id: id.to_string(),
            account_id: String::new(),
            name: kind.local_name().unwrap_or_default(),
            path: String::new(),
            kind,
            unread: members.iter().map(|f| f.unread).fold(0, u32::saturating_add),
            parent: None,
        })
    }

    /// The real folders behind `id`: for an "Alle accounts" folder the folder of that kind in
    /// every account, for a real folder just itself.
    pub fn members(&self, id: &str) -> Vec<String> {
        match kind_of(id) {
            Some(kind) => self.folders.iter().filter(|f| f.kind == kind).map(|f| f.id.clone()).collect(),
            None => self.folder(id).map(|f| vec![f.id.clone()]).unwrap_or_default(),
        }
    }

    /// The name of folder `id`, real or "Alle accounts".
    pub fn folder_name(&self, id: &str) -> Option<String> {
        self.folder(id).map(|f| f.name.clone()).or_else(|| self.unified_folder(id).map(|f| f.name))
    }

    /// Whether anything behind folder `id` is being fetched.
    pub fn folder_loading(&self, id: &str) -> bool {
        self.loading.contains(id) || self.members(id).iter().any(|m| self.loading.contains(m))
    }

    /// Where a message dropped on folder `target` goes: the folder itself, or for an "Alle
    /// accounts" folder the folder of that kind in the message's own account, since mail does
    /// not move between accounts.
    pub fn drop_target(&self, target: &str, message_id: &str) -> Option<String> {
        let Some(kind) = kind_of(target) else { return Some(target.to_string()) };
        let account = self.message(message_id).and_then(|m| self.folder(&m.folder_id)).map(|f| f.account_id.clone())?;
        self.folder_of_kind(&account, kind).map(|f| f.id.clone())
    }

    /// When the last account but one goes, the group goes with it: an "Alle accounts" folder
    /// that was open gives way to the same folder of the account that is left.
    pub(crate) fn leave_unified_if_alone(&mut self) {
        if self.accounts.len() >= 2 {
            return;
        }
        let Some(kind) = self.selected_folder.as_deref().and_then(kind_of) else { return };
        self.selected_folder = self.folders.iter().find(|f| f.kind == kind).map(|f| f.id.clone());
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::tests::{account, header};
    use crate::{Event, RemoteFolder, RemoteHeader};

    fn remote(path: &str, kind: FolderKind) -> RemoteFolder {
        RemoteFolder { path: path.into(), name: path.into(), kind, selectable: true, parent: None }
    }

    #[test]
    fn a_change_of_language_renames_the_standard_folders_and_the_local_calendar_only() {
        merel_i18n::set_for_thread(merel_i18n::Language::Dutch);
        let mut s = Session::sample();
        s.calendars =
            vec![merel_model::Calendar::local(), merel_model::Calendar { name: "Sport".into(), id: "c2".into(), ..merel_model::Calendar::local() }];
        let named = |s: &Session, kind| s.folders.iter().find(|f| f.kind == kind).map(|f| f.name.clone());
        assert_eq!(named(&s, FolderKind::Inbox).as_deref(), Some("Postvak IN"));
        merel_i18n::set_for_thread(merel_i18n::Language::English);
        s.relabel();
        assert_eq!(named(&s, FolderKind::Inbox).as_deref(), Some("Inbox"));
        assert_eq!(named(&s, FolderKind::Deleted).as_deref(), Some("Deleted Items"));
        assert_eq!(s.calendars.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["My calendar", "Sport"]);
        merel_i18n::set_for_thread(merel_i18n::Language::Dutch);
        s.relabel();
        assert_eq!(named(&s, FolderKind::Inbox).as_deref(), Some("Postvak IN"));
    }

    fn unread(n: u32) -> RemoteHeader {
        RemoteHeader { seen: false, ..header(n) }
    }

    /// Two accounts, each with an inbox holding one unread message, and a Trash for the first.
    fn two_accounts() -> Session {
        let mut s = Session::with_accounts(vec![account("a"), account("b")]);
        let a = vec![remote("INBOX", FolderKind::Inbox), remote("Trash", FolderKind::Deleted)];
        s.apply(Event::Connected { account_id: "a".into(), folders: a, syncs_items: false });
        s.apply(Event::Connected { account_id: "b".into(), folders: vec![remote("INBOX", FolderKind::Inbox)], syncs_items: false });
        s.apply(Event::Headers { folder_id: "a/INBOX".into(), headers: vec![unread(1)], offset: 0, limit: 200 });
        s.apply(Event::Headers { folder_id: "b/INBOX".into(), headers: vec![unread(2)], offset: 0, limit: 200 });
        s.take_jobs();
        s
    }

    #[test]
    fn ids_name_a_kind_and_never_a_real_folder() {
        let inbox = id(FolderKind::Inbox).expect("inbox");
        assert_eq!(kind_of(&inbox), Some(FolderKind::Inbox));
        assert_eq!(id(FolderKind::Other), None);
        assert!(!is_unified("a/INBOX") && !is_unified("*alle/whatever"));
    }

    #[test]
    fn the_group_adds_up_the_unread_mail_of_every_account() {
        let s = two_accounts();
        let folders = s.unified_folders();
        assert_eq!(folders.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["Postvak IN", "Verwijderde items"]);
        assert_eq!(folders[0].unread, 2);
        let mut one = Session::with_accounts(vec![account("a")]);
        one.apply(Event::Connected { account_id: "a".into(), folders: vec![remote("INBOX", FolderKind::Inbox)], syncs_items: false });
        assert!(one.unified_folders().is_empty(), "one account has no group");
    }

    #[test]
    fn the_unified_inbox_lists_the_mail_of_both_accounts() {
        let mut s = two_accounts();
        s.run("folder.select", &json!({ "id": "*alle/inbox" })).expect("select");
        let mut ids: Vec<&str> = s.visible_messages().iter().map(|m| m.id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(ids, ["a/INBOX#1", "b/INBOX#2"]);
        assert_eq!(s.folder_name("*alle/inbox").as_deref(), Some("Postvak IN"));
        assert!(s.run("folder.select", &json!({ "id": "*alle/sent" })).is_err(), "no account has Sent items");
    }

    #[test]
    fn opening_it_fetches_whatever_is_not_listed_yet() {
        let mut s = two_accounts();
        s.run("folder.select", &json!({ "id": "*alle/deleted" })).expect("select");
        let jobs = s.take_jobs();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].account_id(), "a");
        assert!(s.folder_loading("*alle/deleted"));
        s.run("folder.refresh", &json!({})).expect("refresh");
        assert!(s.take_jobs().is_empty(), "the trash is still being fetched");
        s.run("folder.select", &json!({ "id": "*alle/inbox" })).expect("select");
        s.run("folder.refresh", &json!({})).expect("refresh");
        assert_eq!(s.take_jobs().len(), 2, "both inboxes are asked again");
    }

    #[test]
    fn a_reply_goes_out_from_the_account_the_message_came_to() {
        let mut s = two_accounts();
        s.run("folder.select", &json!({ "id": "*alle/inbox" })).expect("select");
        assert_eq!(s.current_account().map(|a| a.id.as_str()), Some("a"), "nothing selected: the first account");
        s.run("message.select", &json!({ "id": "b/INBOX#2" })).expect("select message");
        assert_eq!(s.current_account().map(|a| a.id.as_str()), Some("b"));
    }

    #[test]
    fn mail_never_goes_out_from_signal() {
        let mut s = two_accounts();
        s.accounts[0].protocol = merel_model::Protocol::Signal;
        assert_eq!(s.mail_accounts().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["b"]);
        assert_eq!(s.current_account().map(|a| a.id.as_str()), Some("b"), "the first mail account");
        s.run("message.select", &json!({ "id": "a/INBOX#1" })).expect("select message");
        assert_eq!(s.current_account().map(|a| a.id.as_str()), Some("b"), "not the Signal account the message is in");
    }

    #[test]
    fn a_message_dropped_on_it_stays_in_its_own_account() {
        let s = two_accounts();
        assert_eq!(s.drop_target("*alle/deleted", "a/INBOX#1").as_deref(), Some("a/Trash"));
        assert_eq!(s.drop_target("*alle/deleted", "b/INBOX#2"), None, "b has no trash");
        assert_eq!(s.drop_target("b/INBOX", "a/INBOX#1").as_deref(), Some("b/INBOX"));
    }

    #[test]
    fn mark_all_read_reaches_every_account() {
        let mut s = two_accounts();
        s.run("folder.select", &json!({ "id": "*alle/inbox" })).expect("select");
        s.run("folder.mark_all_read", &json!({})).expect("mark");
        assert!(s.messages.iter().all(|m| m.flags.seen));
        assert_eq!(s.unified_folder("*alle/inbox").map(|f| f.unread), Some(0));
    }

    #[test]
    fn removing_the_second_account_opens_the_folder_of_the_one_left() {
        let mut s = two_accounts();
        s.run("folder.select", &json!({ "id": "*alle/inbox" })).expect("select");
        s.remove_account("a");
        assert_eq!(s.selected_folder.as_deref(), Some("b/INBOX"));
    }
}
