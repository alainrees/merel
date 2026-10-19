//! The session (accounts, folders, messages, selection) and the command registry.
//!
//! Every user-visible action is a command id run through [`Session::run`]: the ribbon, keyboard
//! shortcuts and, later, scripting all dispatch the same way. That gives one place to validate
//! parameters and one list of what the app can do.
//!
//! The session itself never touches the network. A command that needs the server leaves a
//! [`Job`] behind; the [`Engine`] hands jobs to a background [`worker`] and feeds the
//! [`Event`]s that come back into [`Session::apply`]. That keeps the session pure and testable.

pub mod cache;
pub mod calendar;
mod commands;
pub mod config;
pub mod contacts;
mod sample;
pub mod signin;
pub mod sync;
pub mod tasks;
pub mod worker;

pub use commands::{COMMANDS, CommandError, CommandSpec};
pub use mailcraft_mime::Body;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use mailcraft_mailbox::{Flag, Item, RemoteFolder, RemoteHeader};
use mailcraft_model::{Account, Appointment, Auth, Calendar, Contact, Draft, Flags, Folder, FolderKind, MessageHeader, Task, clean_text};
use mailcraft_secrets::{Secret, SecretStore};

/// How many of the newest messages a folder shows.
pub const HEADER_LIMIT: u32 = 200;
/// The sample account's id; it has no server behind it.
pub const SAMPLE_ACCOUNT: &str = "sample";

/// Work for the background worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Job {
    Connect {
        account_id: String,
    },
    Headers {
        account_id: String,
        folder_id: String,
        path: String,
    },
    Body {
        account_id: String,
        message_id: String,
        path: String,
        id: String,
    },
    Flag {
        account_id: String,
        path: String,
        id: String,
        flag: Flag,
        on: bool,
    },
    Move {
        account_id: String,
        path: String,
        id: String,
        dest: String,
    },
    /// Send `draft` (over SMTP, or through the store itself) and file a copy in `sent_path`.
    Send {
        account_id: String,
        draft_id: u64,
        draft: Box<Draft>,
        sent_path: Option<String>,
    },
    /// Fetch the appointments in `[from, to)`, the contacts and the tasks of an account.
    Sync {
        account_id: String,
        from: i64,
        to: i64,
    },
    /// Create (empty item id) or replace an item in the account's store; `local_id` is the
    /// session id it has meanwhile.
    SaveItem {
        account_id: String,
        local_id: String,
        item: Item,
    },
    DeleteItem {
        account_id: String,
        id: String,
    },
}

impl Job {
    pub fn account_id(&self) -> &str {
        match self {
            Job::Connect { account_id }
            | Job::Headers { account_id, .. }
            | Job::Body { account_id, .. }
            | Job::Flag { account_id, .. }
            | Job::Move { account_id, .. }
            | Job::Send { account_id, .. }
            | Job::Sync { account_id, .. }
            | Job::SaveItem { account_id, .. }
            | Job::DeleteItem { account_id, .. } => account_id,
        }
    }
}

/// What the worker reports back.
#[derive(Debug)]
pub enum Event {
    Connected {
        account_id: String,
        folders: Vec<RemoteFolder>,
        /// Whether the store also keeps appointments, contacts and tasks.
        syncs_items: bool,
    },
    /// The items of an account's store, replacing what the session had of it.
    Synced {
        account_id: String,
        appointments: Vec<Appointment>,
        contacts: Vec<Contact>,
        tasks: Vec<Task>,
    },
    /// The store named the item the session knew as `local_id`.
    ItemSaved {
        account_id: String,
        local_id: String,
        remote_id: String,
    },
    Headers {
        folder_id: String,
        headers: Vec<RemoteHeader>,
    },
    Body {
        message_id: String,
        body: Box<Body>,
        /// The envelope as the raw message carries it; fills in what the listing left out.
        header: mailcraft_mime::ParsedHeader,
        /// The message as the server sent it, for the offline cache (empty when served from it).
        raw: Vec<u8>,
    },
    Failed {
        account_id: String,
        what: String,
    },
    /// The outcome of a `Send` job; `Err` carries a sentence for the person.
    Sent {
        account_id: String,
        draft_id: u64,
        result: Result<(), String>,
    },
    /// A browser sign-in finished; `Ok` carries the account to add.
    SignedIn {
        result: Result<Box<Account>, String>,
    },
}

/// Where a draft handed to [`Session::send`] is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendState {
    Sending,
    Sent,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountStatus {
    Offline,
    Connecting,
    Online,
    Error(String),
}

#[derive(Debug, Default)]
pub struct Session {
    pub accounts: Vec<Account>,
    pub folders: Vec<Folder>,
    pub messages: Vec<MessageHeader>,
    pub bodies: HashMap<String, Body>,
    pub status: HashMap<String, AccountStatus>,
    /// Folder and message ids with a fetch in flight.
    pub loading: HashSet<String>,
    pub selected_folder: Option<String>,
    pub selected_message: Option<String>,
    /// Drafts handed to `send`, by draft id.
    pub outbox: HashMap<u64, SendState>,
    /// A browser sign-in in progress (`Some(None)`), or how the last one ended.
    pub sign_in: Option<Option<Result<(), String>>>,
    pub calendars: Vec<Calendar>,
    pub appointments: Vec<Appointment>,
    pub selected_appointment: Option<String>,
    pub contacts: Vec<Contact>,
    pub selected_contact: Option<String>,
    pub tasks: Vec<Task>,
    pub selected_task: Option<String>,
    /// Accounts whose store keeps appointments, contacts and tasks, and so syncs them.
    pub item_accounts: HashSet<String>,
    /// Set when a command changed the calendar, the contacts or the tasks; the engine saves
    /// and clears it.
    calendar_dirty: bool,
    contacts_dirty: bool,
    tasks_dirty: bool,
    next_draft_id: u64,
    next_appointment_id: u64,
    next_contact_id: u64,
    next_task_id: u64,
    next_pending_id: u64,
    pending: Vec<Job>,
}

impl Session {
    /// A session with built-in sample mail, for first run and tests. No network.
    pub fn sample() -> Self {
        let mut s = sample::session();
        s.selected_folder = s.folders.iter().find(|f| f.kind == FolderKind::Inbox).map(|f| f.id.clone());
        s.status.insert(SAMPLE_ACCOUNT.into(), AccountStatus::Offline);
        s.recount_unread();
        s
    }

    /// A session for the configured accounts; each connects when its job is run.
    pub fn with_accounts(accounts: Vec<Account>) -> Self {
        let mut s = Session::default();
        for a in accounts {
            s.add_account(a);
        }
        s
    }

    /// Registers an account and queues its connection; an account with the same id (one that
    /// signed in again) is replaced. The sample account goes away.
    pub fn add_account(&mut self, account: Account) {
        if let Some(existing) = self.accounts.iter_mut().find(|a| a.id == account.id) {
            *existing = account.clone();
            self.status.insert(account.id.clone(), AccountStatus::Connecting);
            self.pending.push(Job::Connect { account_id: account.id });
            return;
        }
        if self.accounts.iter().any(|a| a.id == SAMPLE_ACCOUNT) {
            self.accounts.clear();
            self.folders.clear();
            self.messages.clear();
            self.bodies.clear();
            self.status.clear();
            self.selected_folder = None;
            self.selected_message = None;
        }
        self.status.insert(account.id.clone(), AccountStatus::Connecting);
        self.pending.push(Job::Connect { account_id: account.id.clone() });
        self.accounts.push(account);
    }

    pub fn is_sample(&self) -> bool {
        self.accounts.iter().any(|a| a.id == SAMPLE_ACCOUNT)
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

    /// The body of the selected message, once fetched.
    pub fn selected_body(&self) -> Option<&Body> {
        self.selected_message.as_deref().and_then(|id| self.bodies.get(id))
    }

    /// Queues `draft` for sending from `account_id`; the id tracks it in `outbox`.
    pub fn send(&mut self, account_id: &str, draft: Draft) -> u64 {
        self.next_draft_id += 1;
        let draft_id = self.next_draft_id;
        if account_id == SAMPLE_ACCOUNT || !self.accounts.iter().any(|a| a.id == account_id) {
            self.outbox.insert(draft_id, SendState::Failed("Voeg eerst een echt account toe om te kunnen versturen.".into()));
            return draft_id;
        }
        let sent_path = self.folder_of_kind(account_id, FolderKind::Sent).map(|f| f.path.clone());
        self.outbox.insert(draft_id, SendState::Sending);
        self.pending.push(Job::Send { account_id: account_id.to_string(), draft_id, draft: Box::new(draft), sent_path });
        draft_id
    }

    /// The account to send from: the selected folder's, else the first.
    pub fn current_account(&self) -> Option<&Account> {
        let by_folder =
            self.selected_folder.as_deref().and_then(|f| self.folder(f)).and_then(|f| self.accounts.iter().find(|a| a.id == f.account_id));
        by_folder.or_else(|| self.accounts.first())
    }

    /// Jobs left behind by commands since the last call.
    pub fn take_jobs(&mut self) -> Vec<Job> {
        std::mem::take(&mut self.pending)
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty() || !self.loading.is_empty() || self.outbox.values().any(|s| *s == SendState::Sending)
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

    /// Folds a worker event into the session.
    pub fn apply(&mut self, event: Event) {
        match event {
            Event::Connected { account_id, folders, syncs_items } => {
                self.status.insert(account_id.clone(), AccountStatus::Online);
                if syncs_items {
                    self.item_accounts.insert(account_id.clone());
                    self.request_sync(&account_id);
                }
                self.folders.retain(|f| f.account_id != account_id);
                let mut new: Vec<Folder> = folders
                    .into_iter()
                    .filter(|f| f.selectable)
                    .map(|f| Folder {
                        id: format!("{account_id}/{}", f.path),
                        account_id: account_id.clone(),
                        name: f.kind.dutch_name().map(str::to_string).unwrap_or_else(|| clean_text(&f.name)),
                        path: f.path,
                        kind: f.kind,
                        unread: 0,
                    })
                    .collect();
                new.sort_by(|a, b| a.kind.rank().cmp(&b.kind.rank()).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
                self.folders.extend(new);
                // Messages of folders that no longer exist (they came from the cache) go.
                self.messages.retain(|m| !m.folder_id.starts_with(&format!("{account_id}/")) || self.folders.iter().any(|f| f.id == m.folder_id));
                match self.selected_folder.clone().filter(|id| self.folder(id).is_some_and(|f| f.account_id == account_id)) {
                    // The selected folder came from the cache: its list is refreshed.
                    Some(selected) => self.request_headers(&selected),
                    None => {
                        if self.selected_folder.is_none()
                            && let Some(inbox) = self.folder_of_kind(&account_id, FolderKind::Inbox).map(|f| f.id.clone())
                        {
                            self.selected_folder = Some(inbox.clone());
                            self.request_headers(&inbox);
                        }
                    }
                }
            }
            Event::Headers { folder_id, headers } => {
                self.loading.remove(&folder_id);
                if self.folder(&folder_id).is_none() {
                    return;
                }
                self.messages.retain(|m| m.folder_id != folder_id);
                for h in headers {
                    let id = format!("{folder_id}#{}", h.id);
                    let from = h.header.from.clone().or_else(|| mailcraft_model::Address::new(Some("Onbekende afzender"), "unknown@invalid").ok());
                    let Some(from) = from else { continue };
                    self.messages.push(MessageHeader {
                        id,
                        folder_id: folder_id.clone(),
                        remote_id: h.id,
                        from,
                        to: h.header.to.clone(),
                        subject: if h.header.subject.is_empty() { "(Geen onderwerp)".into() } else { h.header.subject.clone() },
                        date: h.date,
                        preview: String::new(),
                        flags: Flags { seen: h.seen, flagged: h.flagged, answered: h.answered },
                        has_attachments: h.has_attachments,
                        message_id: h.header.message_id.clone(),
                    });
                }
                self.recount_unread();
            }
            Event::Body { message_id, body, header, .. } => {
                self.loading.remove(&message_id);
                if let Some(m) = self.messages.iter_mut().find(|m| m.id == message_id) {
                    m.preview = mailcraft_mime::preview(&body.text, 120);
                    if m.to.is_empty() {
                        m.to = header.to;
                    }
                }
                self.bodies.insert(message_id, *body);
            }
            Event::Failed { account_id, what } => {
                self.loading.clear();
                self.status.insert(account_id, AccountStatus::Error(what));
            }
            Event::Synced { account_id, appointments, contacts, tasks } => self.apply_synced(&account_id, appointments, contacts, tasks),
            Event::ItemSaved { account_id, local_id, remote_id } => self.apply_saved(&account_id, &local_id, &remote_id),
            Event::SignedIn { result } => match result {
                Ok(account) => {
                    self.sign_in = Some(Some(Ok(())));
                    self.add_account(*account);
                }
                Err(e) => self.sign_in = Some(Some(Err(e))),
            },
            Event::Sent { account_id, draft_id, result } => {
                let state = match result {
                    Ok(()) => SendState::Sent,
                    Err(e) => SendState::Failed(e),
                };
                if state == SendState::Sent
                    && let Some(sent) = self.folder_of_kind(&account_id, FolderKind::Sent).map(|f| f.id.clone())
                    && self.messages.iter().any(|m| m.folder_id == sent)
                {
                    self.request_headers(&sent);
                }
                self.outbox.insert(draft_id, state);
            }
        }
    }

    fn request_headers(&mut self, folder_id: &str) {
        let Some(f) = self.folder(folder_id).cloned() else { return };
        if f.account_id == SAMPLE_ACCOUNT || self.loading.contains(folder_id) {
            return;
        }
        self.loading.insert(folder_id.to_string());
        self.pending.push(Job::Headers { account_id: f.account_id, folder_id: f.id, path: f.path });
    }

    fn request_body(&mut self, message_id: &str) {
        let Some(m) = self.message(message_id) else { return };
        let Some(f) = self.folder(&m.folder_id) else { return };
        if f.account_id == SAMPLE_ACCOUNT || self.bodies.contains_key(message_id) || self.loading.contains(message_id) {
            return;
        }
        let job = Job::Body { account_id: f.account_id.clone(), message_id: m.id.clone(), path: f.path.clone(), id: m.remote_id.clone() };
        self.loading.insert(message_id.to_string());
        self.pending.push(job);
    }
}

/// The session plus the background worker that serves its jobs.
pub struct Engine {
    pub session: Session,
    worker: Option<worker::Worker>,
    events: Receiver<Event>,
    secrets: Arc<dyn SecretStore>,
    config_path: Option<std::path::PathBuf>,
    cache: cache::Cache,
}

impl Engine {
    /// Sample mail only, no worker: for tests and previews.
    pub fn sample() -> Self {
        let (_tx, rx) = channel();
        Engine {
            session: Session::sample(),
            worker: None,
            events: rx,
            secrets: Arc::new(mailcraft_secrets::MemoryStore::default()),
            config_path: None,
            cache: cache::Cache::none(),
        }
    }

    /// Loads the configured accounts from `config_path` (sample mail when there are none) and
    /// starts connecting them.
    pub fn load(secrets: Arc<dyn SecretStore>, config_path: Option<std::path::PathBuf>) -> Self {
        let accounts = config_path.as_deref().map(config::load).unwrap_or_default();
        let (tx, rx) = channel();
        let worker = worker::Worker::new(tx);
        let calendar = config_path.as_deref().map(|p| config::load_json::<calendar::CalendarFile>(&config::calendar_path(p))).unwrap_or_default();
        let mut engine = Engine { session: Session::default(), worker: Some(worker), events: rx, secrets, config_path, cache: cache::Cache::none() };
        if accounts.is_empty() {
            engine.session = Session::sample();
        } else {
            engine.session = Session::with_accounts(accounts);
            // What the accounts showed last time, until the servers answer.
            engine.cache = cache::Cache::open(engine.config_path.as_deref(), &*engine.secrets);
            engine.cache.restore(&mut engine.session);
            engine.spawn_accounts();
        }
        if !calendar.appointments.is_empty() || !calendar.calendars.is_empty() {
            engine.session.set_calendar_data(calendar);
        }
        let contacts =
            engine.config_path.as_deref().map(|p| config::load_json::<contacts::ContactsFile>(&config::contacts_path(p))).unwrap_or_default();
        if !contacts.contacts.is_empty() {
            engine.session.set_contacts(contacts.contacts);
        }
        let tasks = engine.config_path.as_deref().map(|p| config::load_json::<tasks::TasksFile>(&config::tasks_path(p))).unwrap_or_default();
        if !tasks.tasks.is_empty() {
            engine.session.set_tasks(tasks.tasks);
        }
        engine.dispatch();
        engine
    }

    /// Saves whatever the last command changed.
    fn persist(&mut self) -> Result<(), CommandError> {
        let Some(path) = self.config_path.clone() else {
            self.session.take_calendar_dirty();
            self.session.take_contacts_dirty();
            self.session.take_tasks_dirty();
            return Ok(());
        };
        if self.session.take_calendar_dirty() {
            config::save_json(&config::calendar_path(&path), &self.session.calendar_data()).map_err(|e| CommandError::Store(e.to_string()))?;
        }
        if self.session.take_contacts_dirty() {
            config::save_json(&config::contacts_path(&path), &self.session.contacts_data()).map_err(|e| CommandError::Store(e.to_string()))?;
        }
        if self.session.take_tasks_dirty() {
            config::save_json(&config::tasks_path(&path), &self.session.tasks_data()).map_err(|e| CommandError::Store(e.to_string()))?;
        }
        Ok(())
    }

    fn spawn_accounts(&mut self) {
        let ids: Vec<String> = self.session.accounts.iter().map(|a| a.id.clone()).collect();
        for id in ids {
            self.spawn_account(&id);
        }
    }

    /// Starts the worker thread for one account with the secrets the keychain holds for it.
    fn spawn_account(&mut self, account_id: &str) {
        let Some(account) = self.session.accounts.iter().find(|a| a.id == account_id).cloned() else { return };
        let secret = match self.secrets.get(&account.id) {
            Ok(Some(s)) => s,
            Ok(None) => {
                self.session.status.insert(account.id, AccountStatus::Error("Geen wachtwoord of aanmelding in de sleutelhanger.".into()));
                return;
            }
            Err(e) => {
                self.session.status.insert(account.id, AccountStatus::Error(e.to_string()));
                return;
            }
        };
        let client_secret = self.secrets.get(&signin::client_secret_key(&account.id)).ok().flatten();
        let credentials = signin::Credentials::new(&account, secret, client_secret);
        if let Some(worker) = &mut self.worker {
            worker.start_account(account, credentials);
        }
    }

    /// Starts a browser sign-in on a background thread; the result arrives as an event and
    /// `session.sign_in` follows it.
    pub fn sign_in(&mut self, sign_in: signin::SignIn) {
        let Some(worker) = &self.worker else {
            self.session.sign_in = Some(Some(Err("Aanmelden kan niet in de voorbeeldmodus.".into())));
            return;
        };
        self.session.sign_in = Some(None);
        let secrets = Arc::clone(&self.secrets);
        let events = worker.events();
        let spawned = std::thread::Builder::new().name("oauth-sign-in".into()).spawn(move || {
            let result = signin::run(&secrets, sign_in).map(Box::new);
            let _ = events.send(Event::SignedIn { result });
        });
        if spawned.is_err() {
            self.session.sign_in = Some(Some(Err("Kon geen achtergrondtaak starten.".into())));
        }
    }

    /// Runs the browser sign-in again for an OAuth account, asking the provider's current
    /// scopes and servers, so an account signed in before a scope was added (calendar,
    /// contacts and tasks for Gmail; Microsoft Graph for Microsoft 365) gets a token that
    /// covers it. The result arrives like a first sign-in.
    pub fn sign_in_again(&mut self, account_id: &str) {
        let Some(mut account) = self.session.accounts.iter().find(|a| a.id == account_id).cloned() else { return };
        let preset = account.protocol.preset();
        if let (Auth::OAuth2 { scope, .. }, Some(preset)) = (&mut account.auth, preset) {
            *scope = preset.scope;
            account.incoming = Some(preset.incoming);
            account.outgoing = preset.outgoing;
        }
        let client_secret = self.secrets.get(&signin::client_secret_key(&account.id)).ok().flatten();
        self.sign_in(signin::SignIn { account, client_secret });
    }

    /// Stores the password in the keychain, saves the account and connects it.
    pub fn add_account(&mut self, account: Account, password: Secret) -> Result<(), String> {
        self.secrets.set(&account.id, &password).map_err(|e| e.to_string())?;
        self.session.add_account(account.clone());
        self.save_accounts()?;
        self.spawn_account(&account.id);
        self.dispatch();
        Ok(())
    }

    fn save_accounts(&self) -> Result<(), String> {
        if let Some(path) = &self.config_path {
            config::save(path, &self.session.accounts).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn run(&mut self, id: &str, params: &serde_json::Value) -> Result<(), CommandError> {
        let r = self.session.run(id, params);
        self.cache.note_commands(&self.session);
        self.dispatch();
        self.persist()?;
        self.cache.save(&self.session, false);
        r
    }

    /// Writes the offline cache now; for when the window closes.
    pub fn flush(&mut self) {
        self.cache.save(&self.session, true);
    }

    /// Queues `draft` for sending; see [`Session::send`].
    pub fn send(&mut self, account_id: &str, draft: Draft) -> u64 {
        let id = self.session.send(account_id, draft);
        self.dispatch();
        id
    }

    /// Applies everything the worker has reported. Returns true when something changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(e) = self.events.try_recv() {
            let signed_in = match &e {
                Event::SignedIn { result: Ok(a) } => Some(a.id.clone()),
                _ => None,
            };
            self.cache.note_event(&self.session, &e);
            self.session.apply(e);
            if let Some(id) = signed_in {
                if let Err(e) = self.save_accounts() {
                    self.session.sign_in = Some(Some(Err(e)));
                }
                self.spawn_account(&id);
            }
            changed = true;
        }
        self.dispatch();
        self.cache.save(&self.session, false);
        changed
    }

    fn dispatch(&mut self) {
        let jobs = self.session.take_jobs();
        let jobs = self.cache.serve(&mut self.session, jobs);
        let Some(worker) = &self.worker else { return };
        for j in jobs {
            if j.account_id() != SAMPLE_ACCOUNT {
                worker.submit(j);
            }
        }
    }
}
