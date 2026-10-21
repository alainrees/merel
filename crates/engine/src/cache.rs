//! Keeping the offline cache (`merel_store`) in step with the session.
//!
//! On start the cache fills the session with what each account showed last time, so the
//! window is usable before the first server answer, and stays usable without one. Every
//! server answer and every command marks the account dirty; the engine writes the dirty
//! accounts a couple of seconds later and when the window closes. An opened message is served
//! from the cache when it is there, so reading mail needs no connection. Changes that found no
//! connection wait in the cache too (the offline queue) and are replayed when the account
//! connects, in this run or a later one.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use merel_secrets::SecretStore;
use merel_store::{AccountCache, KeyKind, Store};

use crate::{Event, Job, SAMPLE_ACCOUNT, Session};

/// How long after a change the cache is written, at most.
const SAVE_AFTER: Duration = Duration::from_secs(2);

pub struct Cache {
    store: Option<Store>,
    accounts: HashMap<String, AccountCache>,
    dirty: HashSet<String>,
    last_save: Instant,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl Cache {
    /// No cache at all: sample mail, and tests.
    pub fn none() -> Cache {
        Cache { store: None, accounts: HashMap::new(), dirty: HashSet::new(), last_save: Instant::now() }
    }

    /// The cache next to the accounts file. When the keychain or the directory refuses, the
    /// app runs without a cache, and says so in the log.
    pub fn open(accounts_path: Option<&Path>, secrets: &dyn SecretStore) -> Cache {
        let mut cache = Cache::none();
        let Some(dir) = accounts_path.and_then(Path::parent) else { return cache };
        match Store::open(dir.join("cache"), secrets) {
            Ok(store) => cache.store = Some(store),
            Err(e) => log::warn!("offline cache disabled: {e}"),
        }
        cache
    }

    /// Whether there is a cache directory with its key; without one nothing is kept.
    pub fn has_store(&self) -> bool {
        self.store.is_some()
    }

    /// Keeps a private key of `account_id` (S/MIME certificate or OpenPGP key) sealed on disk.
    pub fn save_identity(&self, account_id: &str, kind: KeyKind, plain: &[u8]) -> Result<(), String> {
        match &self.store {
            Some(store) => store.save_identity(account_id, kind, plain).map_err(|e| e.to_string()),
            None => Ok(()),
        }
    }

    pub fn load_identity(&self, account_id: &str, kind: KeyKind) -> Option<zeroize::Zeroizing<Vec<u8>>> {
        self.store.as_ref()?.load_identity(account_id, kind)
    }

    pub fn remove_identity(&self, account_id: &str, kind: KeyKind) {
        if let Some(store) = &self.store {
            store.remove_identity(account_id, kind);
        }
    }

    /// Fills `session` with the cached state of each of its accounts.
    pub fn restore(&mut self, session: &mut Session) {
        let Some(store) = &self.store else { return };
        let ids: Vec<String> = session.accounts.iter().map(|a| a.id.clone()).filter(|id| id != SAMPLE_ACCOUNT).collect();
        for id in ids {
            if let Some(cached) = store.load(&id) {
                session.restore(&cached);
                if !cached.pending.is_empty() {
                    session.queued.insert(id.clone(), cached.pending.len());
                }
                self.accounts.insert(id, cached);
            }
        }
    }

    /// Forgets `account_id`, in memory and on disk.
    pub fn remove(&mut self, account_id: &str) {
        self.accounts.remove(account_id);
        self.dirty.remove(account_id);
        if let Some(store) = &self.store {
            store.remove(account_id);
        }
    }

    /// Takes note of a server answer before the session applies it.
    pub fn note_event(&mut self, session: &Session, event: &Event) {
        let account = match event {
            Event::Connected { account_id, .. }
            | Event::Folders { account_id, .. }
            | Event::Synced { account_id, .. }
            | Event::ItemSaved { account_id, .. } => Some(account_id.clone()),
            Event::Headers { folder_id, .. } | Event::Found { folder_id, .. } => session.folder(folder_id).map(|f| f.account_id.clone()),
            Event::Body { message_id, raw, .. } => {
                let account = session.message(message_id).and_then(|m| session.folder(&m.folder_id)).map(|f| f.account_id.clone());
                if let Some(account) = &account {
                    self.accounts.entry(account.clone()).or_default().put_body(message_id, raw);
                }
                account
            }
            Event::Deferred { account_id, job } => {
                match serde_json::to_value(job) {
                    Ok(value) => self.accounts.entry(account_id.clone()).or_default().pending.push(value),
                    Err(e) => log::warn!("{account_id}: change not queued: {e}"),
                }
                Some(account_id.clone())
            }
            Event::Failed { .. } | Event::ItemsFailed { .. } | Event::Sent { .. } | Event::SignedIn { .. } => None,
        };
        if let Some(account) = account.filter(|a| a != SAMPLE_ACCOUNT) {
            self.dirty.insert(account);
        }
    }

    /// The message as the server sent it, while it is still in the cache; `None` for sample
    /// mail, for a message never opened and for one too large to keep.
    pub fn raw(&self, account_id: &str, message_id: &str) -> Option<&[u8]> {
        self.accounts.get(account_id).and_then(|c| c.body(message_id))
    }

    /// The changes waiting for `account_id`, oldest first; the queue is emptied. A change that
    /// fails again comes back through `Event::Deferred`.
    pub fn take_pending(&mut self, account_id: &str) -> Vec<Job> {
        let Some(cached) = self.accounts.get_mut(account_id) else { return Vec::new() };
        if cached.pending.is_empty() {
            return Vec::new();
        }
        self.dirty.insert(account_id.to_string());
        std::mem::take(&mut cached.pending).into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect()
    }

    /// Takes note that commands ran: flags, moves and items may have changed anywhere.
    pub fn note_commands(&mut self, session: &Session) {
        self.dirty.extend(session.accounts.iter().map(|a| a.id.clone()).filter(|id| id != SAMPLE_ACCOUNT));
    }

    /// Serves what the cache has from `jobs` (opened messages) and returns the rest.
    pub fn serve(&mut self, session: &mut Session, keys: &crate::certificates::Keys, jobs: Vec<Job>) -> Vec<Job> {
        let mut rest = Vec::with_capacity(jobs.len());
        for job in jobs {
            if let Job::Body { account_id, message_id, .. } = &job
                && let Some(raw) = self.accounts.get(account_id).and_then(|c| c.body(message_id))
            {
                let body = Box::new(crate::certificates::parse_body(keys, account_id, raw));
                let header = merel_mime::parse_header(raw);
                session.apply(Event::Body { message_id: message_id.clone(), body, header, raw: Vec::new() });
                continue;
            }
            rest.push(job);
        }
        rest
    }

    /// Writes the dirty accounts, once enough time has passed since the last write, or at
    /// once when `force` (the window is closing).
    pub fn save(&mut self, session: &Session, force: bool) {
        let Some(store) = &self.store else { return };
        if self.dirty.is_empty() || (!force && self.last_save.elapsed() < SAVE_AFTER) {
            return;
        }
        for id in std::mem::take(&mut self.dirty) {
            if !session.accounts.iter().any(|a| a.id == id) {
                continue;
            }
            let mut cached = session.snapshot(&id);
            if let Some(old) = self.accounts.get_mut(&id) {
                cached.bodies = std::mem::take(&mut old.bodies);
                cached.pending = std::mem::take(&mut old.pending);
            }
            cached.prune_bodies();
            if let Err(e) = store.save(&cached) {
                log::warn!("{id}: cache not written: {e}");
            }
            self.accounts.insert(id, cached);
        }
        self.last_save = Instant::now();
    }
}

impl Session {
    /// What the cache keeps of `account_id`: the session's view of it, minus opened messages.
    pub fn snapshot(&self, account_id: &str) -> AccountCache {
        let folders: Vec<_> = self.folders.iter().filter(|f| f.account_id == account_id).cloned().collect();
        let messages = self.messages.iter().filter(|m| folders.iter().any(|f| f.id == m.folder_id)).cloned().collect();
        let calendars: Vec<_> = self.calendars.iter().filter(|c| c.account_id.as_deref() == Some(account_id)).cloned().collect();
        let appointments = self.appointments.iter().filter(|a| calendars.iter().any(|c| c.id == a.calendar_id)).cloned().collect();
        let contacts = self.contacts.iter().filter(|c| c.account_id.as_deref() == Some(account_id)).cloned().collect();
        let tasks = self.tasks.iter().filter(|t| t.account_id.as_deref() == Some(account_id)).cloned().collect();
        let drafts = self.drafts.iter().filter(|d| d.account_id == account_id).cloned().collect();
        AccountCache {
            account_id: account_id.to_string(),
            saved_at: now(),
            folders,
            messages,
            bodies: Vec::new(),
            pending: Vec::new(),
            syncs_items: self.item_accounts.contains(account_id),
            calendars,
            appointments,
            contacts,
            tasks,
            drafts,
        }
    }

    /// Puts the cached state of an account in place of whatever the session had of it. The
    /// server's answers replace it again as they come.
    pub fn restore(&mut self, cached: &AccountCache) {
        let id = cached.account_id.as_str();
        if !self.accounts.iter().any(|a| a.id == id) {
            return;
        }
        self.folders.retain(|f| f.account_id != id);
        self.messages.retain(|m| !m.folder_id.starts_with(&format!("{id}/")));
        self.folders.extend(cached.folders.iter().cloned());
        self.messages.extend(cached.messages.iter().cloned());
        let calendar_ids: Vec<String> = cached.calendars.iter().map(|c| c.id.clone()).collect();
        self.calendars.retain(|c| c.account_id.as_deref() != Some(id));
        self.appointments.retain(|a| !calendar_ids.contains(&a.calendar_id));
        self.calendars.extend(cached.calendars.iter().cloned());
        self.appointments.extend(cached.appointments.iter().cloned());
        self.contacts.retain(|c| c.account_id.as_deref() != Some(id));
        self.contacts.extend(cached.contacts.iter().cloned());
        self.tasks.retain(|t| t.account_id.as_deref() != Some(id));
        self.tasks.extend(cached.tasks.iter().cloned());
        self.drafts.retain(|d| d.account_id != id);
        self.drafts.extend(cached.drafts.iter().cloned());
        if cached.syncs_items {
            self.item_accounts.insert(id.to_string());
        }
        self.recount_unread();
        if self.selected_folder.is_none() {
            self.selected_folder = self.folder_of_kind(id, merel_model::FolderKind::Inbox).map(|f| f.id.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use merel_mailbox::{RemoteFolder, RemoteHeader};
    use merel_model::{Account, Address, Auth, FolderKind, Protocol, Server, Tls};
    use merel_secrets::MemoryStore;
    use serde_json::json;

    use super::*;

    fn account() -> Account {
        Account {
            id: "a1".into(),
            name: "Test".into(),
            address: Address::new(None, "a@b.nl").unwrap(),
            username: "a@b.nl".into(),
            protocol: Protocol::ImapSmtp,
            auth: Auth::Password,
            incoming: Some(Server { host: "imap.b.nl".into(), port: 993, tls: Tls::Implicit }),
            outgoing: None,
            dav: None,
            signature: String::new(),
        }
    }

    const RAW: &[u8] = b"From: Sanne <sanne@example.org>\r\nSubject: Hoi\r\n\r\nHallo daar\r\n";

    fn connected_session() -> Session {
        let mut s = Session::with_accounts(vec![account()]);
        s.take_jobs();
        let folders = vec![
            RemoteFolder { path: "INBOX".into(), name: "INBOX".into(), kind: FolderKind::Inbox, selectable: true, parent: None },
            RemoteFolder { path: "Archief".into(), name: "Archief".into(), kind: FolderKind::Archive, selectable: true, parent: None },
        ];
        s.apply(Event::Connected { account_id: "a1".into(), folders, syncs_items: true });
        let header = RemoteHeader {
            id: "7".into(),
            seen: false,
            flagged: false,
            answered: false,
            size: 10,
            has_attachments: false,
            categories: Vec::new(),
            header: merel_mime::parse_header(RAW),
            date: 1_000,
        };
        s.apply(Event::Headers { folder_id: "a1/INBOX".into(), headers: vec![header], offset: 0, limit: crate::HEADER_LIMIT });
        s.apply(Event::Synced { account_id: "a1".into(), appointments: vec![], contacts: vec![], tasks: vec![] });
        s
    }

    #[test]
    fn the_cache_fills_a_new_session_and_serves_opened_messages() {
        let dir = std::env::temp_dir().join(format!("merel-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("accounts.json");
        let secrets = MemoryStore::default();

        // First run: the server answers, a message is opened, the cache is written.
        let mut cache = Cache::open(Some(&path), &secrets);
        assert!(cache.store.is_some());
        let mut s = connected_session();
        for event in [
            Event::Connected { account_id: "a1".into(), folders: vec![], syncs_items: false },
            Event::Headers { folder_id: "a1/INBOX".into(), headers: vec![], offset: 0, limit: crate::HEADER_LIMIT },
        ] {
            cache.note_event(&s, &event);
        }
        let body = Event::Body {
            message_id: "a1/INBOX#7".into(),
            body: Box::new(merel_mime::parse_body(RAW)),
            header: merel_mime::parse_header(RAW),
            raw: RAW.to_vec(),
        };
        cache.note_event(&s, &body);
        s.apply(body);
        // A flag set while the connection is gone waits in the queue.
        let flag = Job::Flag { account_id: "a1".into(), path: "INBOX".into(), id: "7".into(), flag: merel_mailbox::Flag::Seen, on: true };
        let deferred = Event::Deferred { account_id: "a1".into(), job: Box::new(flag.clone()) };
        cache.note_event(&s, &deferred);
        s.apply(deferred);
        assert_eq!((s.queued.get("a1"), s.status.get("a1")), (Some(&1), Some(&crate::AccountStatus::Offline)));
        cache.save(&s, false);
        assert!(!cache.dirty.is_empty(), "a write right after a change waits");
        cache.save(&s, true);
        assert!(cache.dirty.is_empty());

        // Second run: the window fills before any server answer.
        let mut cache = Cache::open(Some(&path), &secrets);
        let mut s = Session::with_accounts(vec![account()]);
        s.take_jobs();
        cache.restore(&mut s);
        assert_eq!(s.folders.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), vec!["Postvak IN", "Archief"]);
        assert_eq!(s.selected_folder.as_deref(), Some("a1/INBOX"));
        assert_eq!(s.messages.len(), 1);
        assert_eq!(s.folder("a1/INBOX").map(|f| f.unread), Some(1));
        assert!(s.item_accounts.contains("a1"));
        assert!(s.calendar("a1/calendar").is_some());
        assert_eq!(s.queued.get("a1"), Some(&1), "the queued change survived the restart");
        assert_eq!(cache.take_pending("a1"), vec![flag]);
        assert!(cache.take_pending("a1").is_empty());

        // Opening the message needs no server.
        s.run("message.select", &json!({ "id": "a1/INBOX#7" })).unwrap();
        let pending = s.take_jobs();
        let jobs = cache.serve(&mut s, &Default::default(), pending);
        assert!(!jobs.iter().any(|j| matches!(j, Job::Body { .. })), "{jobs:?}");
        assert_eq!(s.bodies.get("a1/INBOX#7").map(|b| b.text.trim()), Some("Hallo daar"));
        assert!(!s.loading.contains("a1/INBOX#7"));

        // The server's answer replaces the cached folders and refreshes the selected one.
        let folders = vec![RemoteFolder { path: "INBOX".into(), name: "INBOX".into(), kind: FolderKind::Inbox, selectable: true, parent: None }];
        s.apply(Event::Connected { account_id: "a1".into(), folders, syncs_items: true });
        assert_eq!(s.folders.len(), 1);
        assert!(s.queued.is_empty());
        let jobs = s.take_jobs();
        assert!(jobs.iter().any(|j| matches!(j, Job::Headers { folder_id, .. } if folder_id == "a1/INBOX")), "{jobs:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn without_a_store_nothing_happens() {
        let mut cache = Cache::none();
        let mut s = connected_session();
        cache.note_commands(&s);
        cache.restore(&mut s);
        cache.save(&s, true);
        let snap = s.snapshot("a1");
        assert_eq!((snap.folders.len(), snap.messages.len(), snap.syncs_items), (2, 1, true));
        assert!(s.snapshot("nope").folders.is_empty());
    }
}
