//! The session (accounts, folders, messages, selection) and the command registry.
//!
//! Every user-visible action is a command id run through [`Session::run`]: the ribbon, keyboard
//! shortcuts and, later, scripting all dispatch the same way. That gives one place to validate
//! parameters and one list of what the app can do.
//!
//! The session itself never touches the network. A command that needs the server leaves a
//! [`Job`] behind; the [`Engine`] hands jobs to a background [`worker`] and feeds the
//! [`Event`]s that come back into [`Session::apply`]. That keeps the session pure and testable.

mod commands;
pub mod config;
mod sample;
pub mod worker;

pub use commands::{COMMANDS, CommandError, CommandSpec};
pub use mailcraft_mime::Body;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use mailcraft_imap::{RemoteFolder, RemoteHeader};
use mailcraft_model::{Account, Flags, Folder, FolderKind, MessageHeader, clean_text};
use mailcraft_secrets::{Secret, SecretStore};

/// How many of the newest messages a folder shows.
pub const HEADER_LIMIT: u32 = 200;
/// The sample account's id; it has no server behind it.
pub const SAMPLE_ACCOUNT: &str = "sample";

/// Work for the background worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Job {
    Connect { account_id: String },
    Headers { account_id: String, folder_id: String, path: String },
    Body { account_id: String, message_id: String, path: String, uid: u32 },
    Flag { account_id: String, path: String, uid: u32, flag: &'static str, on: bool },
    Move { account_id: String, path: String, uid: u32, dest: String },
}

impl Job {
    pub fn account_id(&self) -> &str {
        match self {
            Job::Connect { account_id }
            | Job::Headers { account_id, .. }
            | Job::Body { account_id, .. }
            | Job::Flag { account_id, .. }
            | Job::Move { account_id, .. } => account_id,
        }
    }
}

/// What the worker reports back.
#[derive(Debug)]
pub enum Event {
    Connected { account_id: String, folders: Vec<RemoteFolder> },
    Headers { folder_id: String, headers: Vec<RemoteHeader> },
    Body { message_id: String, body: Body },
    Failed { account_id: String, what: String },
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

    /// Registers an account and queues its connection. The sample account goes away.
    pub fn add_account(&mut self, account: Account) {
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

    /// Jobs left behind by commands since the last call.
    pub fn take_jobs(&mut self) -> Vec<Job> {
        std::mem::take(&mut self.pending)
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty() || !self.loading.is_empty()
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
            Event::Connected { account_id, folders } => {
                self.status.insert(account_id.clone(), AccountStatus::Online);
                self.folders.retain(|f| f.account_id != account_id);
                let mut new: Vec<Folder> = folders
                    .into_iter()
                    .filter(|f| f.selectable)
                    .map(|f| Folder {
                        id: format!("{account_id}/{}", f.name),
                        account_id: account_id.clone(),
                        name: f
                            .kind
                            .dutch_name()
                            .map(str::to_string)
                            .unwrap_or_else(|| clean_text(f.name.rsplit(['/', '.']).next().unwrap_or(&f.name))),
                        path: f.name,
                        kind: f.kind,
                        unread: 0,
                    })
                    .collect();
                new.sort_by(|a, b| a.kind.rank().cmp(&b.kind.rank()).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
                self.folders.extend(new);
                if self.selected_folder.is_none()
                    && let Some(inbox) = self.folder_of_kind(&account_id, FolderKind::Inbox).map(|f| f.id.clone())
                {
                    self.selected_folder = Some(inbox.clone());
                    self.request_headers(&inbox);
                }
            }
            Event::Headers { folder_id, headers } => {
                self.loading.remove(&folder_id);
                if self.folder(&folder_id).is_none() {
                    return;
                }
                self.messages.retain(|m| m.folder_id != folder_id);
                for h in headers {
                    let id = format!("{folder_id}#{}", h.uid);
                    let from = h.header.from.clone().or_else(|| mailcraft_model::Address::new(Some("Onbekende afzender"), "unknown@invalid").ok());
                    let Some(from) = from else { continue };
                    self.messages.push(MessageHeader {
                        id,
                        folder_id: folder_id.clone(),
                        uid: h.uid,
                        from,
                        to: h.header.to.clone(),
                        subject: if h.header.subject.is_empty() { "(Geen onderwerp)".into() } else { h.header.subject.clone() },
                        date: h.date,
                        preview: String::new(),
                        flags: Flags { seen: h.seen, flagged: h.flagged, answered: h.answered },
                        has_attachments: h.has_attachments,
                    });
                }
                self.recount_unread();
            }
            Event::Body { message_id, body } => {
                self.loading.remove(&message_id);
                if let Some(m) = self.messages.iter_mut().find(|m| m.id == message_id) {
                    m.preview = mailcraft_mime::preview(&body.text, 120);
                }
                self.bodies.insert(message_id, body);
            }
            Event::Failed { account_id, what } => {
                self.loading.clear();
                self.status.insert(account_id, AccountStatus::Error(what));
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
        let job = Job::Body { account_id: f.account_id.clone(), message_id: m.id.clone(), path: f.path.clone(), uid: m.uid };
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
        }
    }

    /// Loads the configured accounts from `config_path` (sample mail when there are none) and
    /// starts connecting them.
    pub fn load(secrets: Arc<dyn SecretStore>, config_path: Option<std::path::PathBuf>) -> Self {
        let accounts = config_path.as_deref().map(config::load).unwrap_or_default();
        let (tx, rx) = channel();
        let worker = worker::Worker::new(tx);
        let mut engine = Engine { session: Session::default(), worker: Some(worker), events: rx, secrets, config_path };
        if accounts.is_empty() {
            engine.session = Session::sample();
        } else {
            engine.session = Session::with_accounts(accounts);
            engine.spawn_accounts();
        }
        engine.dispatch();
        engine
    }

    fn spawn_accounts(&mut self) {
        let Some(worker) = &mut self.worker else { return };
        for a in &self.session.accounts {
            match self.secrets.get(&a.id) {
                Ok(Some(password)) => worker.start_account(a.clone(), password),
                Ok(None) => self
                    .session
                    .status
                    .insert(a.id.clone(), AccountStatus::Error("Geen wachtwoord in de sleutelhanger.".into()))
                    .map(|_| ())
                    .unwrap_or(()),
                Err(e) => self.session.status.insert(a.id.clone(), AccountStatus::Error(e.to_string())).map(|_| ()).unwrap_or(()),
            }
        }
    }

    /// Stores the password in the keychain, saves the account and connects it.
    pub fn add_account(&mut self, account: Account, password: Secret) -> Result<(), String> {
        self.secrets.set(&account.id, &password).map_err(|e| e.to_string())?;
        self.session.add_account(account.clone());
        if let Some(path) = &self.config_path {
            config::save(path, &self.session.accounts).map_err(|e| e.to_string())?;
        }
        if let Some(worker) = &mut self.worker {
            worker.start_account(account, password);
        }
        self.dispatch();
        Ok(())
    }

    pub fn run(&mut self, id: &str, params: &serde_json::Value) -> Result<(), CommandError> {
        let r = self.session.run(id, params);
        self.dispatch();
        r
    }

    /// Applies everything the worker has reported. Returns true when something changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(e) = self.events.try_recv() {
            self.session.apply(e);
            changed = true;
        }
        self.dispatch();
        changed
    }

    fn dispatch(&mut self) {
        let jobs = self.session.take_jobs();
        let Some(worker) = &self.worker else { return };
        for j in jobs {
            if j.account_id() != SAMPLE_ACCOUNT {
                worker.submit(j);
            }
        }
    }
}
