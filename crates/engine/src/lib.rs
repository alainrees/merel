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
pub mod calls;
pub use calls::{Call, CallTask};
pub mod categories;
pub mod certificates;
pub mod chat;
pub mod cleanup;
mod commands;
pub mod config;
pub mod contacts;
pub mod drafts;
pub mod folders;
pub mod followup;
pub mod groups;
pub mod ignore;
pub mod indexing;
pub mod junk;
pub mod meaning;
pub mod meetings;
pub mod openpgp;
pub mod quicksteps;
pub mod reminders;
pub mod rules;
mod sample;
pub mod search;
pub mod securejoin;
pub mod signin;
pub mod snooze;
pub mod suggest;
pub mod sync;
pub mod tasks;
pub mod templates;
pub mod unified;
pub mod view;
pub mod worker;

pub use commands::{COMMANDS, CommandError, CommandSpec};
pub use merel_mime::Body;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use merel_mailbox::{Flag, Item};
pub use merel_mailbox::{RemoteFolder, RemoteHeader};
use merel_model::{
    Account, Appointment, Auth, Calendar, Category, Contact, ContactGroup, Draft, Flags, Folder, FolderKind, MessageHeader, QuickStep, Rule,
    StoredDraft, Task, Template, clean_text,
};
use merel_secrets::{Secret, SecretStore};
use serde::{Deserialize, Serialize};

/// How many of the newest messages a folder shows.
pub const HEADER_LIMIT: u32 = 200;
/// The most a refresh of one folder fetches at once, however many pages were loaded.
pub const MAX_REFETCH: u32 = 2_000;
/// How often open folders are refreshed, and the calendar, contacts and tasks.
pub const REFRESH_EVERY: Duration = Duration::from_secs(120);
pub const ITEMS_EVERY: Duration = Duration::from_secs(300);

fn default_limit() -> u32 {
    HEADER_LIMIT
}
/// The sample account's id; it has no server behind it.
pub const SAMPLE_ACCOUNT: &str = "sample";

/// Work for the background worker. A job that changes the server (`is_change`) is kept when
/// there is no connection and done when it is back, across restarts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Job {
    Connect {
        account_id: String,
    },
    Headers {
        account_id: String,
        folder_id: String,
        path: String,
        /// How many of the newest messages to skip: 0 for the top of the folder.
        #[serde(default)]
        offset: u32,
        /// How many to fetch: a refresh asks for as many as are listed, so loaded pages stay.
        #[serde(default = "default_limit")]
        limit: u32,
    },
    Body {
        account_id: String,
        message_id: String,
        path: String,
        id: String,
    },
    /// A page of a folder's headers for the search index, which the session does not list.
    IndexHeaders {
        account_id: String,
        folder_id: String,
        path: String,
        offset: u32,
        limit: u32,
    },
    /// The text of a few messages of one folder for the search index: each message's session
    /// id and the id the server knows it by. Nothing of it reaches the session.
    IndexTexts {
        account_id: String,
        folder_id: String,
        path: String,
        ids: Vec<(String, String)>,
    },
    /// Ask the server which messages of a folder match `query`.
    Search {
        account_id: String,
        folder_id: String,
        path: String,
        query: String,
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
    /// Remove one message for good (it was in the Deleted folder already).
    Delete {
        account_id: String,
        path: String,
        id: String,
    },
    /// Remove every message in a folder for good.
    Empty {
        account_id: String,
        path: String,
    },
    /// Give a message its new set of categories; `removed` are the ones it loses.
    Categories {
        account_id: String,
        path: String,
        id: String,
        categories: Vec<String>,
        removed: Vec<String>,
    },
    /// Tell the store the person called a message junk (or not junk); queued before the move,
    /// while the message is still where `path` and `id` say.
    Junk {
        account_id: String,
        path: String,
        id: String,
        junk: bool,
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
        item: Box<Item>,
    },
    DeleteItem {
        account_id: String,
        id: String,
    },
    /// Make a folder under `parent` (a folder path; `None` for the top of the mailbox), then
    /// list the folders again.
    CreateFolder {
        account_id: String,
        parent: Option<String>,
        name: String,
    },
    RenameFolder {
        account_id: String,
        path: String,
        name: String,
    },
    DeleteFolder {
        account_id: String,
        path: String,
    },
    /// Ask whether `email` is free now and when they work.
    Availability {
        account_id: String,
        email: String,
    },
}

impl Job {
    pub fn account_id(&self) -> &str {
        match self {
            Job::Connect { account_id }
            | Job::Headers { account_id, .. }
            | Job::Body { account_id, .. }
            | Job::Search { account_id, .. }
            | Job::IndexHeaders { account_id, .. }
            | Job::IndexTexts { account_id, .. }
            | Job::Flag { account_id, .. }
            | Job::Move { account_id, .. }
            | Job::Delete { account_id, .. }
            | Job::Empty { account_id, .. }
            | Job::Categories { account_id, .. }
            | Job::Junk { account_id, .. }
            | Job::Send { account_id, .. }
            | Job::Sync { account_id, .. }
            | Job::SaveItem { account_id, .. }
            | Job::DeleteItem { account_id, .. }
            | Job::CreateFolder { account_id, .. }
            | Job::RenameFolder { account_id, .. }
            | Job::DeleteFolder { account_id, .. }
            | Job::Availability { account_id, .. } => account_id,
        }
    }

    /// What the session marks as loading while this job runs: the folder or the message.
    pub fn loading_key(&self) -> Option<String> {
        match self {
            Job::Headers { folder_id, .. } => Some(folder_id.clone()),
            Job::Body { message_id, .. } => Some(message_id.clone()),
            _ => None,
        }
    }

    /// Whether the job changes something on the server, so it must not be lost offline. A
    /// folder change is not kept: the person sees the folder list, so it needs the server now.
    pub fn is_change(&self) -> bool {
        matches!(
            self,
            Job::Flag { .. }
                | Job::Move { .. }
                | Job::Delete { .. }
                | Job::Empty { .. }
                | Job::Categories { .. }
                | Job::Junk { .. }
                | Job::SaveItem { .. }
                | Job::DeleteItem { .. }
        )
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
    /// The folders of an account, listed again after one was made, renamed or removed.
    Folders { account_id: String, folders: Vec<RemoteFolder> },
    /// The items of an account's store, replacing what the session had of it.
    Synced { account_id: String, appointments: Vec<Appointment>, contacts: Vec<Contact>, tasks: Vec<Task> },
    /// The store named the item the session knew as `local_id`.
    ItemSaved { account_id: String, local_id: String, remote_id: String },
    Headers {
        folder_id: String,
        headers: Vec<RemoteHeader>,
        /// The offset the job asked for; 0 replaces the folder's list, more adds older mail.
        offset: u32,
        /// The limit the job asked for; fewer headers than this means the folder's start.
        limit: u32,
    },
    /// A page of headers for the search index, or why it did not come.
    IndexHeaders { account_id: String, folder_id: String, offset: u32, limit: u32, result: Result<Vec<RemoteHeader>, String> },
    /// The text of messages for the search index. `failed` says why the rest did not come,
    /// with the id of the message that failed.
    IndexTexts { account_id: String, texts: Vec<indexing::IndexedText>, failed: Option<(String, String)> },
    /// What the server found for a search in a folder, or why it could not search.
    Found { folder_id: String, query: String, result: Result<Vec<RemoteHeader>, String> },
    Body {
        message_id: String,
        body: Box<Body>,
        /// The envelope as the raw message carries it; fills in what the listing left out.
        header: Box<merel_mime::ParsedHeader>,
        /// The message as the server sent it, for the offline cache (empty when served from it).
        raw: Vec<u8>,
        /// The keys an OpenPGP-encrypted message carried inside (not when served from the cache).
        carried: Option<Box<openpgp::Carried>>,
    },
    /// The account could not connect; `what` is a sentence for the person.
    Failed { account_id: String, what: String },
    /// One job went wrong while the account itself works: it stays connected and the person
    /// is told once. `loading` is what the session was waiting for (a folder or message id).
    JobFailed { account_id: String, loading: Option<String>, what: String },
    /// The server no longer has the folder or message a job was for: it was moved or deleted
    /// elsewhere. Nothing is wrong; the session lets it go.
    Gone { account_id: String, folder_id: Option<String>, message_id: Option<String> },
    /// The account's mail connected, but the calendar server next to it did not; `what` is a
    /// sentence for the person.
    ItemsFailed { account_id: String, what: String },
    /// A change could not reach the server because there is no connection; it waits in the
    /// offline queue until the account connects again.
    Deferred { account_id: String, job: Box<Job> },
    /// The outcome of a `Send` job; `Err` carries a sentence for the person.
    Sent { account_id: String, draft_id: u64, result: Result<(), String> },
    /// Whether someone is free, as their calendar says; `None` when the server would not tell.
    Availability { email: String, availability: Option<merel_model::Availability> },
    /// A browser sign-in finished; `Ok` carries the account to add.
    SignedIn { result: Result<Box<Account>, String> },
}

/// Where a draft handed to [`Session::send`] is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendState {
    Sending,
    Sent,
    Failed(String),
}

/// A listed header as the session shows it; `None` when it has no usable sender.
/// Puts the header from inside an opened encrypted message in the place of the outer one: the
/// real subject, date and sender's name, and the chat tag Delta Chat only writes inside.
fn open_sealed(m: &mut MessageHeader, inside: &merel_mime::ParsedHeader) {
    m.sealed = merel_model::Sealed::Opened;
    if let Some(from) = inside.from.as_ref().filter(|f| f.email().eq_ignore_ascii_case(m.from.email())) {
        m.from = from.clone();
    }
    if !inside.to.is_empty() {
        m.to = inside.to.clone();
    }
    if !inside.subject.is_empty() {
        m.subject = inside.subject.clone();
    }
    if let Some(date) = inside.date {
        m.date = date;
    }
    if inside.chat.is_some() {
        m.chat = inside.chat.clone();
    }
}

fn message_of(folder_id: &str, h: RemoteHeader) -> Option<MessageHeader> {
    let from = h.header.from.clone().or_else(|| merel_model::Address::new(Some("Onbekende afzender"), "unknown@invalid").ok())?;
    Some(MessageHeader {
        id: format!("{folder_id}#{}", h.id),
        folder_id: folder_id.to_string(),
        remote_id: h.id,
        from,
        to: h.header.to.clone(),
        subject: if h.header.subject.is_empty() { merel_i18n::tr("no-subject-capital") } else { h.header.subject.clone() },
        date: h.date,
        preview: String::new(),
        flags: Flags { seen: h.seen, flagged: h.flagged, answered: h.answered },
        has_attachments: h.has_attachments,
        message_id: h.header.message_id.clone(),
        categories: h.categories,
        importance: h.header.importance,
        chat: h.header.chat,
        sealed: if h.header.encrypted { merel_model::Sealed::Closed } else { merel_model::Sealed::No },
        size: u64::from(h.size),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountStatus {
    Offline,
    Connecting,
    Online,
    Error(String),
}

/// The outcome of a search run on the server; see [`Session::server_hits`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerHits {
    pub folder_id: String,
    pub query: String,
    pub ids: HashSet<String>,
}

/// How many messages a server search lists at most.
pub const SEARCH_LIMIT: u32 = 200;

#[derive(Debug, Default)]
pub struct Session {
    pub accounts: Vec<Account>,
    pub folders: Vec<Folder>,
    pub messages: Vec<MessageHeader>,
    pub bodies: HashMap<String, Body>,
    pub status: HashMap<String, AccountStatus>,
    /// Folder and message ids with a fetch in flight.
    pub loading: HashSet<String>,
    /// Folders whose oldest message has been reached, so there is nothing older to load.
    pub complete_folders: HashSet<String>,
    /// Unread messages a refresh found that were not listed before; the UI takes and shows it.
    arrivals: Vec<String>,
    pub selected_folder: Option<String>,
    pub selected_message: Option<String>,
    /// Messages selected alongside `selected_message` (Ctrl- or Shift-click in the list); the
    /// message commands act on these as well. Never holds `selected_message` or a message that
    /// is gone: [`Session::run`] prunes it after every command.
    pub marked: Vec<String>,
    /// What the person typed in the search box; empty shows the selected folder as it is.
    pub search: String,
    /// Whether a search covers every folder of every account, or the selected folder only.
    pub search_all: bool,
    /// Which messages of the folder the list shows, and in what order ("Beeld" tab).
    pub filter: view::Filter,
    pub sort: view::Sort,
    /// Only messages with this category are listed, when set ("Beeld" tab).
    pub category_filter: Option<String>,
    /// What the server found for the last search asked of it: the folder, the words, and the
    /// ids of the messages (now listed) it named. Shown next to the local matches while the
    /// search box still holds those words and that folder is selected.
    pub server_hits: Option<ServerHits>,
    /// Why the last server search gave nothing, when it failed.
    pub search_error: Option<String>,
    /// What the search index found for the search box; see [`indexing::Indexer::search`].
    pub index_hits: Option<indexing::IndexHits>,
    /// Whether search results stand newest first rather than best match first.
    pub search_newest: bool,
    /// Messages that left their place since the engine last looked (deleted, moved), for the
    /// search index to forget.
    pub(crate) left: Vec<String>,
    /// Drafts handed to `send`, by draft id.
    pub outbox: HashMap<u64, SendState>,
    /// Messages being written that were put aside, every account's together.
    pub drafts: Vec<StoredDraft>,
    /// Scheduled messages on their way: the outbox id and the stored draft it came from, which
    /// stays until the server took the message.
    pub scheduled_sends: HashMap<u64, String>,
    scheduled_failures: Vec<String>,
    /// What went wrong with single jobs since the last call to `take_job_failures`.
    job_failures: Vec<String>,
    /// The id `draft.save` gave or kept, for the window that saved it.
    pub last_draft: Option<String>,
    next_stored_draft: u64,
    /// A browser sign-in in progress (`Some(None)`), or how the last one ended.
    pub sign_in: Option<Option<Result<(), String>>>,
    pub calendars: Vec<Calendar>,
    pub appointments: Vec<Appointment>,
    pub selected_appointment: Option<String>,
    /// Colours picked for account calendars, by calendar id; saved with the local calendar.
    pub calendar_colors: std::collections::BTreeMap<String, [u8; 3]>,
    /// The id `calendar.add` gave, for the window that added it.
    pub last_calendar: Option<String>,
    /// Reminders closed for good, by occurrence; saved with the local calendar.
    pub dismissed_reminders: Vec<reminders::ReminderKey>,
    /// Reminders put off, by occurrence, until the given moment.
    pub snoozed_reminders: reminders::Snoozed,
    pub contacts: Vec<Contact>,
    pub selected_contact: Option<String>,
    pub tasks: Vec<Task>,
    pub selected_task: Option<String>,
    /// Accounts whose store keeps appointments, contacts and tasks, and so syncs them.
    pub item_accounts: HashSet<String>,
    /// Why an account's calendar and address book could not be reached while its mail could.
    pub item_errors: HashMap<String, String>,
    /// What Microsoft 365 said about people's calendars, by lower-case address: when it was
    /// asked, and the answer once it came.
    pub availability: HashMap<String, (i64, Option<merel_model::Availability>)>,
    /// Rules for incoming mail, every account's together, in the order they run.
    pub rules: Vec<Rule>,
    rules_dirty: bool,
    /// Blocked and safe senders, for every account; `junk.json`.
    pub junk: merel_model::JunkLists,
    junk_dirty: bool,
    /// Conversations whose mail goes to Deleted Items on arrival; see [`ignore`].
    pub ignored: Vec<merel_model::IgnoredConversation>,
    ignored_dirty: bool,
    /// The due days of flagged messages, kept in `follow_ups.json`; see [`followup`].
    pub follow_ups: Vec<merel_model::FollowUp>,
    follow_ups_dirty: bool,
    /// Follow-up reminders put off until a moment, by the follow-up's key; in memory only.
    pub snoozed_follow_ups: HashMap<String, i64>,
    /// Task reminders closed for a due day, kept in `tasks.json`; see [`tasks`].
    pub dismissed_task_reminders: Vec<tasks::TaskReminderKey>,
    /// Task reminders put off until a moment, by task id; in memory only.
    pub snoozed_task_reminders: HashMap<String, i64>,
    /// Messages put off with "Sluimeren", kept in `snoozed.json`; see [`snooze`].
    pub snoozes: Vec<merel_model::Snooze>,
    snoozes_dirty: bool,
    /// The categories (coloured labels) the person knows, kept in `categories.json`.
    pub categories: Vec<Category>,
    categories_dirty: bool,
    /// Quick steps, every account's together, in the order they are shown; `quick_steps.json`.
    pub quick_steps: Vec<QuickStep>,
    quick_steps_dirty: bool,
    next_quick_step_id: u64,
    /// Contact groups (distribution lists), kept on this computer in `contact_groups.json`.
    pub groups: Vec<ContactGroup>,
    groups_dirty: bool,
    next_group_id: u64,
    /// Text templates ("Mijn sjablonen"), kept on this computer in `templates.json`.
    pub templates: Vec<Template>,
    templates_dirty: bool,
    next_template_id: u64,
    /// Messages the rules changed since the UI last asked.
    rule_hits: usize,
    next_rule_id: u64,
    /// Changes waiting for a connection, per account.
    pub queued: HashMap<String, usize>,
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
    /// Gives the standard folders and the local calendar their names in the app's language,
    /// after a change of language; a name the user chose stays.
    pub fn relabel(&mut self) {
        for f in &mut self.folders {
            if let Some(name) = f.kind.local_name()
                && f.kind.is_standard_name(&f.name)
            {
                f.name = name;
            }
        }
        let standard = |name: &str| merel_i18n::Language::ALL.iter().any(|&l| merel_i18n::plain(l, "calendar-mine").as_deref() == Some(name));
        for c in &mut self.calendars {
            if c.id == "local" && standard(&c.name) {
                c.name = merel_i18n::tr("calendar-mine");
            }
        }
    }

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
        let mut s = Session { categories: Category::defaults(), ..Session::default() };
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
            // Whatever the old connection was fetching is forgotten with it.
            let prefix = format!("{}/", account.id);
            self.loading.retain(|id| !id.starts_with(&prefix) && *id != format!("sync/{}", account.id));
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

    /// Forgets an account and everything the session holds of it: folders, messages, bodies,
    /// status, the calendar and items its store synced, and anything queued for it. Local
    /// appointments, contacts and tasks stay. When it was the last account, the sample mail
    /// comes back so the window is not empty.
    pub fn remove_account(&mut self, account_id: &str) {
        if !self.accounts.iter().any(|a| a.id == account_id) {
            return;
        }
        let prefix = format!("{account_id}/");
        self.accounts.retain(|a| a.id != account_id);
        self.folders.retain(|f| f.account_id != account_id);
        self.messages.retain(|m| !m.folder_id.starts_with(&prefix));
        self.bodies.retain(|id, _| !id.starts_with(&prefix));
        self.loading.retain(|id| !id.starts_with(&prefix) && *id != format!("sync/{account_id}"));
        self.complete_folders.retain(|id| !id.starts_with(&prefix));
        self.drafts.retain(|d| d.account_id != account_id);
        if self.server_hits.as_ref().is_some_and(|h| h.folder_id.starts_with(&prefix)) {
            self.server_hits = None;
        }
        self.status.remove(account_id);
        self.queued.remove(account_id);
        self.item_accounts.remove(account_id);
        self.item_errors.remove(account_id);
        self.pending.retain(|j| j.account_id() != account_id);
        if self.selected_folder.as_deref().is_some_and(|f| f.starts_with(&prefix)) {
            self.selected_folder = self.folders.iter().find(|f| f.kind == FolderKind::Inbox).map(|f| f.id.clone());
            self.selected_message = None;
        }
        self.leave_unified_if_alone();
        self.calendars.retain(|c| c.account_id.as_deref() != Some(account_id));
        if self.calendar_colors.remove(&Self::account_calendar_id(account_id)).is_some() {
            self.calendar_dirty = true;
        }
        if self.rules.iter().any(|r| r.account_id == account_id) {
            self.rules.retain(|r| r.account_id != account_id);
            self.rules_dirty = true;
        }
        if self.quick_steps.iter().any(|q| q.account_id == account_id) {
            self.quick_steps.retain(|q| q.account_id != account_id);
            self.quick_steps_dirty = true;
        }
        if self.ignored.iter().any(|i| i.account_id == account_id) {
            self.ignored.retain(|i| i.account_id != account_id);
            self.ignored_dirty = true;
        }
        self.appointments.retain(|a| !a.id.starts_with(&prefix));
        self.dismissed_reminders.retain(|k| !k.id.starts_with(&prefix));
        self.snoozed_reminders.retain(|k, _| !k.id.starts_with(&prefix));
        if self.selected_appointment.as_deref().is_some_and(|id| id.starts_with(&prefix)) {
            self.selected_appointment = None;
        }
        self.contacts.retain(|c| c.account_id.as_deref() != Some(account_id));
        if self.selected_contact.as_deref().is_some_and(|id| id.starts_with(&prefix)) {
            self.selected_contact = None;
        }
        self.tasks.retain(|t| t.account_id.as_deref() != Some(account_id));
        if self.selected_task.as_deref().is_some_and(|id| id.starts_with(&prefix)) {
            self.selected_task = None;
        }
        if self.accounts.is_empty() {
            let (calendar, contacts, tasks) = (self.calendar_data(), self.contacts_data(), self.tasks_data());
            let mut fresh = Session::sample();
            fresh.set_calendar_data(calendar);
            fresh.set_contacts(contacts.contacts);
            fresh.set_tasks_data(tasks);
            // The lists that are not an account's stay, and what the removal changed still
            // reaches its file.
            fresh.junk = std::mem::take(&mut self.junk);
            fresh.categories = std::mem::take(&mut self.categories);
            fresh.follow_ups = std::mem::take(&mut self.follow_ups);
            fresh.snoozes = std::mem::take(&mut self.snoozes);
            fresh.snoozes_dirty = self.snoozes_dirty;
            fresh.junk_dirty = self.junk_dirty;
            fresh.categories_dirty = self.categories_dirty;
            fresh.follow_ups_dirty = self.follow_ups_dirty;
            fresh.rules_dirty = self.rules_dirty;
            fresh.quick_steps_dirty = self.quick_steps_dirty;
            fresh.groups = std::mem::take(&mut self.groups);
            fresh.groups_dirty = self.groups_dirty;
            fresh.templates = std::mem::take(&mut self.templates);
            fresh.templates_dirty = self.templates_dirty;
            fresh.ignored_dirty = self.ignored_dirty;
            *self = fresh;
        }
    }

    /// Whether `event` is about something the session still has; a worker that was told to
    /// stop can report once more, and that report must not bring its account back.
    pub fn concerns_current(&self, event: &Event) -> bool {
        match event {
            Event::Connected { account_id, .. }
            | Event::Folders { account_id, .. }
            | Event::Synced { account_id, .. }
            | Event::ItemSaved { account_id, .. }
            | Event::Failed { account_id, .. }
            | Event::JobFailed { account_id, .. }
            | Event::Gone { account_id, .. }
            | Event::ItemsFailed { account_id, .. }
            | Event::Deferred { account_id, .. }
            | Event::Sent { account_id, .. }
            | Event::IndexHeaders { account_id, .. }
            | Event::IndexTexts { account_id, .. } => self.accounts.iter().any(|a| &a.id == account_id),
            Event::Headers { folder_id, .. } | Event::Found { folder_id, .. } => self.folder(folder_id).is_some(),
            Event::Body { message_id, .. } => self.message(message_id).is_some(),
            Event::SignedIn { .. } | Event::Availability { .. } => true,
        }
    }

    pub fn folder(&self, id: &str) -> Option<&Folder> {
        self.folders.iter().find(|f| f.id == id)
    }

    pub fn message(&self, id: &str) -> Option<&MessageHeader> {
        self.messages.iter().find(|m| m.id == id)
    }

    /// Messages in the selected folder, newest first; while a search is on, the messages that
    /// match it, in the selected folder or (`search_all`) everywhere the session has mail.
    /// A search looks at what has been fetched: the newest headers of each opened folder and
    /// what the offline cache holds, not the whole server.
    pub fn visible_messages(&self) -> Vec<&MessageHeader> {
        let query = search::Query::parse(&self.search);
        let now = snooze::now();
        let hits = self.current_hits().map(|h| &h.ids);
        let found = self.current_index_hits();
        // `map:` and `account:` narrow the search to some folders.
        let places = self.search_folders(&query);
        // The selected folder, or each account's folder behind an "Alle accounts" one.
        let members = self.selected_folder.as_deref().map(|id| self.members(id)).unwrap_or_default();
        let mut list: Vec<&MessageHeader> = self
            .messages
            .iter()
            .filter(|m| {
                let in_folder = members.contains(&m.folder_id);
                if query.is_empty() {
                    in_folder
                } else {
                    // The server searched the plain words only; its hits still owe the operators.
                    ((in_folder || self.search_all || places.is_some()) && query.matches(m))
                        || ((hits.is_some_and(|ids| ids.contains(&m.id)) || found.is_some_and(|f| f.scores.contains_key(&m.id))) && query.keeps(m))
                }
            })
            .filter(|m| places.as_ref().is_none_or(|p| p.contains(&m.folder_id)))
            .filter(|m| self.filter.keeps(m))
            // Snoozed mail is out of sight until its moment, and the only thing the "Gesluimerd"
            // filter shows.
            .filter(|m| (self.filter == view::Filter::Snoozed) == self.snoozed_until(m, now).is_some())
            .filter(|m| self.category_filter.as_deref().is_none_or(|c| m.categories.iter().any(|x| x.eq_ignore_ascii_case(c))))
            // Chat messages are read in the chat module, as Gmail keeps its chat apart.
            .filter(|m| m.chat.is_none())
            .collect();
        match found.filter(|_| !self.search_newest) {
            // Best match first, as the index ranked them; what only matched here comes after.
            Some(f) => {
                let score = |m: &MessageHeader| f.scores.get(&m.id).copied().unwrap_or(0.0);
                list.sort_by(|a, b| score(b).total_cmp(&score(a)).then_with(|| self.sort.order(a, b)));
            }
            None => list.sort_by(|a, b| self.sort.order(a, b)),
        }
        list
    }

    /// The folders a search's `map:` and `account:` words leave: each word must occur in the
    /// folder's name or path, or in the account's name or address. `None` when it has neither.
    pub fn search_folders(&self, q: &search::Query) -> Option<HashSet<String>> {
        if q.folders.is_empty() && q.accounts.is_empty() {
            return None;
        }
        let accounts: HashSet<&str> = self
            .accounts
            .iter()
            .filter(|a| {
                let hay = format!("{}\n{}", a.name, a.address.email()).to_lowercase();
                q.accounts.iter().all(|w| hay.contains(w.as_str()))
            })
            .map(|a| a.id.as_str())
            .collect();
        Some(
            self.folders
                .iter()
                .filter(|f| q.accounts.is_empty() || accounts.contains(f.account_id.as_str()))
                .filter(|f| {
                    let hay = format!("{}\n{}", f.name, f.path).to_lowercase();
                    q.folders.iter().all(|w| hay.contains(w.as_str()))
                })
                .map(|f| f.id.clone())
                .collect(),
        )
    }

    /// The search index's answer for the search box as it stands now.
    pub fn current_index_hits(&self) -> Option<&indexing::IndexHits> {
        self.index_hits
            .as_ref()
            .filter(|h| h.query == self.search && h.all == self.search_all && h.folder == self.selected_folder && self.is_searching())
    }

    /// Whether the message list shows search results rather than a folder.
    pub fn is_searching(&self) -> bool {
        !self.search.trim().is_empty()
    }

    /// The server's answer for the search as it stands now, if it has been asked.
    pub fn current_hits(&self) -> Option<&ServerHits> {
        let plain = search::Query::parse(&self.search).plain();
        self.server_hits.as_ref().filter(|h| Some(h.folder_id.as_str()) == self.selected_folder.as_deref() && h.query == plain)
    }

    /// Whether the server is being asked about the selected folder right now.
    pub fn searching_server(&self) -> bool {
        self.selected_folder.as_deref().is_some_and(|f| self.loading.contains(&format!("search/{f}")))
    }

    /// Whether a server search makes sense now: plain words in the box (operators such as
    /// `van:` are this computer's to apply) and a real, connected folder.
    pub fn can_search_server(&self) -> bool {
        let Some(f) = self.selected_folder.as_deref().and_then(|id| self.folder(id)) else { return false };
        !search::Query::parse(&self.search).plain().is_empty()
            && f.account_id != SAMPLE_ACCOUNT
            && self.status.get(&f.account_id) == Some(&AccountStatus::Online)
    }

    /// Asks the server of the selected folder which of its messages match the search box.
    pub fn request_server_search(&mut self) {
        if !self.can_search_server() || self.searching_server() {
            return;
        }
        let Some(f) = self.selected_folder.as_deref().and_then(|id| self.folder(id)).cloned() else { return };
        let query = search::Query::parse(&self.search).plain();
        self.search_error = None;
        self.loading.insert(format!("search/{}", f.id));
        self.pending.push(Job::Search { account_id: f.account_id, folder_id: f.id, path: f.path, query });
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
            self.outbox.insert(draft_id, SendState::Failed(merel_i18n::tr("send-needs-account")));
            return draft_id;
        }
        let sent_path = self.folder_of_kind(account_id, FolderKind::Sent).map(|f| f.path.clone());
        self.outbox.insert(draft_id, SendState::Sending);
        self.pending.push(Job::Send { account_id: account_id.to_string(), draft_id, draft: Box::new(draft), sent_path });
        draft_id
    }

    /// The account to send from: the selected folder's, else (in an "Alle accounts" folder)
    /// the selected message's, else the first.
    pub fn current_account(&self) -> Option<&Account> {
        let folder = match self.selected_folder.as_deref() {
            Some(id) if unified::is_unified(id) => self.selected().map(|m| m.folder_id.as_str()),
            id => id,
        };
        let by_folder = folder.and_then(|f| self.folder(f)).and_then(|f| self.accounts.iter().find(|a| a.id == f.account_id));
        by_folder.or_else(|| self.accounts.first())
    }

    /// Jobs left behind by commands since the last call.
    pub fn take_jobs(&mut self) -> Vec<Job> {
        std::mem::take(&mut self.pending)
    }

    /// Puts `jobs` ahead of whatever is pending: the offline queue goes first on connect.
    pub fn prepend_jobs(&mut self, mut jobs: Vec<Job>) {
        jobs.append(&mut self.pending);
        self.pending = jobs;
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty() || !self.loading.is_empty() || self.outbox.values().any(|s| *s == SendState::Sending)
    }

    pub(crate) fn folder_of_kind(&self, account_id: &str, kind: FolderKind) -> Option<&Folder> {
        self.folders.iter().find(|f| f.account_id == account_id && f.kind == kind)
    }

    /// Unread mail in the inbox of every account together, for the window title.
    pub fn inbox_unread(&self) -> u32 {
        self.folders.iter().filter(|f| f.kind == FolderKind::Inbox).map(|f| f.unread).fold(0, u32::saturating_add)
    }

    /// The selected message followed by the ones marked with it, for a command that acts on
    /// the whole selection.
    pub fn selection(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.selected_message.iter().cloned().collect();
        ids.extend(self.marked.iter().filter(|id| self.selected_message.as_ref() != Some(id)).cloned());
        ids
    }

    /// Drops marked messages that are gone or became the selected one.
    pub(crate) fn prune_marked(&mut self) {
        if self.selected_message.is_none() {
            self.marked.clear();
            return;
        }
        self.marked.retain(|id| self.selected_message.as_ref() != Some(id) && self.messages.iter().any(|m| &m.id == id));
    }

    pub(crate) fn recount_unread(&mut self) {
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
                self.queued.remove(&account_id);
                self.item_errors.remove(&account_id);
                if syncs_items {
                    self.item_accounts.insert(account_id.clone());
                    self.request_sync(&account_id);
                }
                self.set_folders(&account_id, folders);
                match self.selected_folder.clone().filter(|id| self.folder(id).is_some_and(|f| f.account_id == account_id)) {
                    // The selected folder came from the cache: its list is refreshed.
                    Some(selected) => self.request_headers(&selected),
                    // "Alle accounts" is open: this account's folder of that kind joins it.
                    None if self.selected_folder.as_deref().is_some_and(unified::is_unified) => {
                        let kind = self.selected_folder.as_deref().and_then(unified::kind_of);
                        if let Some(id) = kind.and_then(|k| self.folder_of_kind(&account_id, k)).map(|f| f.id.clone()) {
                            self.request_headers(&id);
                        }
                    }
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
            Event::Folders { account_id, folders } => {
                let selected = self.selected_folder.clone().filter(|id| id.starts_with(&format!("{account_id}/")));
                self.set_folders(&account_id, folders);
                // The selected folder was renamed or removed: the inbox takes its place.
                if let Some(selected) = selected
                    && self.folder(&selected).is_none()
                {
                    self.selected_folder = self.folder_of_kind(&account_id, FolderKind::Inbox).map(|f| f.id.clone());
                    self.selected_message = None;
                    if let Some(inbox) = self.selected_folder.clone() {
                        self.request_headers(&inbox);
                    }
                }
            }
            Event::Headers { folder_id, headers, offset, limit } => {
                self.loading.remove(&folder_id);
                let Some(folder) = self.folder(&folder_id) else { return };
                let is_inbox = folder.kind == FolderKind::Inbox;
                if (headers.len() as u32) < limit.max(1) {
                    self.complete_folders.insert(folder_id.clone());
                } else if offset == 0 {
                    self.complete_folders.remove(&folder_id);
                }
                // What was listed before a refresh, to tell new arrivals from a first load.
                let known: HashSet<String> = self.messages.iter().filter(|m| m.folder_id == folder_id).map(|m| m.id.clone()).collect();
                // Opened encrypted messages keep the header from inside, which the listing lacks.
                let opened: HashMap<String, MessageHeader> = self
                    .messages
                    .iter()
                    .filter(|m| m.folder_id == folder_id && m.sealed == merel_model::Sealed::Opened)
                    .map(|m| (m.id.clone(), m.clone()))
                    .collect();
                if offset == 0 {
                    self.messages.retain(|m| m.folder_id != folder_id);
                }
                let mut fresh = Vec::new();
                for h in headers {
                    let id = format!("{folder_id}#{}", h.id);
                    if offset > 0 && known.contains(&id) {
                        continue;
                    }
                    if offset == 0 && is_inbox && !known.is_empty() && !known.contains(&id) {
                        if !h.seen {
                            self.arrivals.push(id.clone());
                        }
                        fresh.push(id);
                    }
                    if let Some(mut m) = message_of(&folder_id, h) {
                        if let Some(o) = opened.get(&m.id).filter(|_| m.sealed == merel_model::Sealed::Closed) {
                            (m.from, m.to, m.subject, m.date, m.chat, m.sealed) =
                                (o.from.clone(), o.to.clone(), o.subject.clone(), o.date, o.chat.clone(), o.sealed);
                        }
                        self.messages.push(m);
                    }
                }
                // Mail that arrived since the last listing goes past the blocked senders, then
                // the rules.
                for id in fresh {
                    if !self.apply_junk(&id) && !self.apply_ignored(&id) {
                        self.apply_rules(&id, false);
                    }
                }
                self.recount_unread();
                self.fetch_answers(&folder_id);
            }
            Event::Found { folder_id, query, result } => {
                self.loading.remove(&format!("search/{folder_id}"));
                if self.folder(&folder_id).is_none() {
                    return;
                }
                match result {
                    Ok(headers) => {
                        let mut ids = HashSet::new();
                        for h in headers {
                            let id = format!("{folder_id}#{}", h.id);
                            // A hit the listing does not hold yet (older mail) joins it.
                            if !self.messages.iter().any(|m| m.id == id)
                                && let Some(m) = message_of(&folder_id, h)
                            {
                                self.messages.push(m);
                            }
                            ids.insert(id);
                        }
                        self.recount_unread();
                        self.server_hits = Some(ServerHits { folder_id, query, ids });
                    }
                    Err(what) => self.search_error = Some(what),
                }
            }
            Event::Body { message_id, body, header, .. } => {
                self.loading.remove(&message_id);
                if let Some(m) = self.messages.iter_mut().find(|m| m.id == message_id) {
                    m.preview = merel_mime::preview(&body.text, 120);
                    if m.sealed != merel_model::Sealed::No && body.encrypted == Some(merel_mime::Encryption::Opened) {
                        open_sealed(m, &header);
                    } else if m.to.is_empty() {
                        m.to = header.to;
                    }
                }
                // An answer to a meeting the person organises goes into the appointment.
                self.apply_answer_in(&message_id, &body.attachments);
                self.bodies.insert(message_id, *body);
            }
            Event::Failed { account_id, what } => {
                self.loading.clear();
                self.status.insert(account_id, AccountStatus::Error(what));
            }
            Event::ItemsFailed { account_id, what } => {
                self.item_errors.insert(account_id, what);
            }
            Event::JobFailed { loading, what, .. } => {
                if let Some(key) = loading {
                    self.loading.remove(&key);
                }
                // A refresh that fails each time is said once, not every two minutes.
                if !self.job_failures.contains(&what) {
                    self.job_failures.push(what);
                }
            }
            Event::Gone { folder_id, message_id, .. } => {
                if let Some(folder_id) = folder_id {
                    self.loading.remove(&folder_id);
                }
                if let Some(id) = message_id {
                    self.loading.remove(&id);
                    self.messages.retain(|m| m.id != id);
                    self.bodies.remove(&id);
                    if self.selected_message.as_deref() == Some(id.as_str()) {
                        self.selected_message = None;
                    }
                    self.prune_marked();
                    self.recount_unread();
                }
            }
            Event::Deferred { account_id, .. } => {
                *self.queued.entry(account_id.clone()).or_default() += 1;
                self.status.insert(account_id, AccountStatus::Offline);
            }
            Event::Synced { account_id, appointments, contacts, tasks } => self.apply_synced(&account_id, appointments, contacts, tasks),
            Event::ItemSaved { account_id, local_id, remote_id } => self.apply_saved(&account_id, &local_id, &remote_id),
            Event::Availability { email, availability } => {
                let asked = self.availability.get(&email).map(|(at, _)| *at).unwrap_or_else(drafts::now);
                // A question that got no answer keeps what was known.
                let known = self.availability.get(&email).and_then(|(_, a)| a.clone());
                self.availability.insert(email, (asked, availability.or(known)));
            }
            // The engine's indexer takes these; the session lists none of it.
            Event::IndexHeaders { .. } | Event::IndexTexts { .. } => {}
            Event::SignedIn { result } => match result {
                Ok(account) => {
                    self.sign_in = Some(Some(Ok(())));
                    self.add_account(*account);
                }
                Err(e) => self.sign_in = Some(Some(Err(e))),
            },
            Event::Sent { account_id, draft_id, result } => {
                self.settle_scheduled(draft_id, result.clone());
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

    /// Replaces the folders of an account with what the server lists, sorted the Outlook way;
    /// messages of folders that are gone go with them.
    fn set_folders(&mut self, account_id: &str, folders: Vec<RemoteFolder>) {
        self.folders.retain(|f| f.account_id != account_id);
        let folders: Vec<RemoteFolder> = folders.into_iter().filter(|f| f.selectable).collect();
        // A folder nests under its parent when that is listed too; else it stands at the top.
        let listed: HashSet<String> = folders.iter().map(|f| f.path.clone()).collect();
        let mut new: Vec<Folder> = folders
            .into_iter()
            .map(|f| Folder {
                id: format!("{account_id}/{}", f.path),
                account_id: account_id.to_string(),
                name: f.kind.local_name().unwrap_or_else(|| clean_text(&f.name)),
                parent: f.parent.filter(|p| listed.contains(p) && *p != f.path).map(|p| format!("{account_id}/{p}")),
                path: f.path,
                kind: f.kind,
                unread: 0,
            })
            .collect();
        new.sort_by(|a, b| a.kind.rank().cmp(&b.kind.rank()).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        self.folders.extend(new);
        self.messages.retain(|m| !m.folder_id.starts_with(&format!("{account_id}/")) || self.folders.iter().any(|f| f.id == m.folder_id));
        self.recount_unread();
    }

    fn request_headers(&mut self, folder_id: &str) {
        let Some(f) = self.folder(folder_id).cloned() else { return };
        if f.account_id == SAMPLE_ACCOUNT || self.loading.contains(folder_id) {
            return;
        }
        // A refresh fetches as much as is listed, so pages loaded with "older messages" stay.
        let listed = self.messages.iter().filter(|m| m.folder_id == folder_id).count() as u32;
        let limit = listed.clamp(HEADER_LIMIT, MAX_REFETCH);
        self.loading.insert(folder_id.to_string());
        self.pending.push(Job::Headers { account_id: f.account_id, folder_id: f.id, path: f.path, offset: 0, limit });
    }

    /// Refreshes the inbox of every connected account and the selected folder, for new mail.
    pub fn refresh_open_folders(&mut self) {
        let online: Vec<String> =
            self.accounts.iter().filter(|a| self.status.get(&a.id) == Some(&AccountStatus::Online)).map(|a| a.id.clone()).collect();
        let mut folders: Vec<String> = online.iter().filter_map(|a| self.folder_of_kind(a, FolderKind::Inbox)).map(|f| f.id.clone()).collect();
        for selected in self.selected_folder.as_deref().map(|id| self.members(id)).unwrap_or_default() {
            if let Some(f) = self.folder(&selected)
                && online.contains(&f.account_id)
                && !folders.contains(&f.id)
            {
                folders.push(f.id.clone());
            }
        }
        for id in folders {
            self.request_headers(&id);
        }
    }

    /// Asks every connected account that keeps them for its appointments, contacts and tasks.
    pub fn refresh_items(&mut self) {
        let online: Vec<String> = self.item_accounts.iter().filter(|a| self.status.get(*a) == Some(&AccountStatus::Online)).cloned().collect();
        for id in online {
            self.request_sync(&id);
        }
    }

    /// Sentences about single jobs that went wrong since the last call, for the message bar.
    pub fn take_job_failures(&mut self) -> Vec<String> {
        std::mem::take(&mut self.job_failures)
    }

    /// New unread messages found by refreshes since the last call, by id.
    pub fn take_arrivals(&mut self) -> Vec<String> {
        std::mem::take(&mut self.arrivals)
    }

    /// Whether `folder_id` may have mail older than what is listed, and nothing is being
    /// fetched for it right now.
    pub fn can_load_older(&self, folder_id: &str) -> bool {
        if unified::is_unified(folder_id) {
            return self.members(folder_id).iter().any(|m| self.can_load_older(m));
        }
        let Some(f) = self.folder(folder_id) else { return false };
        f.account_id != SAMPLE_ACCOUNT
            && !self.complete_folders.contains(folder_id)
            && !self.loading.contains(folder_id)
            && self.messages.iter().filter(|m| m.folder_id == folder_id).count() as u32 >= HEADER_LIMIT
    }

    /// Asks for the page of mail below what is listed for `folder_id`.
    pub(crate) fn request_older(&mut self, folder_id: &str) {
        let Some(f) = self.folder(folder_id).cloned() else { return };
        if !self.can_load_older(folder_id) {
            return;
        }
        let offset = self.messages.iter().filter(|m| m.folder_id == folder_id).count() as u32;
        self.loading.insert(folder_id.to_string());
        self.pending.push(Job::Headers { account_id: f.account_id, folder_id: f.id, path: f.path, offset, limit: HEADER_LIMIT });
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
    /// The S/MIME certificates, the accounts' own and the correspondents', shared with the
    /// worker threads that sign, encrypt and decrypt.
    keys: certificates::Keys,
    /// The invitations: own tokens, those under way and the keys they verified.
    invites: securejoin::Invites,
    /// The meetings in Merel's own room and what waits for the media side.
    calls: calls::Calls,
    last_refresh: Instant,
    last_item_refresh: Instant,
    /// The search index over all mail.
    indexer: indexing::Indexer,
}

impl Engine {
    /// Where the accounts file is, and next to it the other local files; `None` in tests.
    pub fn config_path(&self) -> Option<&std::path::Path> {
        self.config_path.as_deref()
    }

    /// The source of an opened message, headers and all, as the server sent it: what
    /// "Doorsturen als bijlage" attaches. `None` for sample mail and for a message the cache
    /// does not hold.
    pub fn raw_message(&self, message_id: &str) -> Option<&[u8]> {
        let m = self.session.message(message_id)?;
        let account = &self.session.folder(&m.folder_id)?.account_id;
        self.cache.raw(account, message_id)
    }

    /// The secret store the accounts' passwords and tokens are in.
    pub fn secrets(&self) -> &Arc<dyn SecretStore> {
        &self.secrets
    }

    /// The name of every secret Merel may have filed: the cache key, and per account its
    /// password or token and its own client secret. What an update hands to the next version.
    pub fn secret_names(&self) -> Vec<String> {
        let mut names = vec![merel_store::KEY_NAME.to_string()];
        for account in &self.session.accounts {
            names.push(account.id.clone());
            names.push(signin::client_secret_key(&account.id));
        }
        names
    }

    /// Sample mail only, no worker: for tests and previews.
    pub fn sample() -> Self {
        let (_tx, rx) = channel();
        Engine {
            session: Session::sample(),
            worker: None,
            events: rx,
            secrets: Arc::new(merel_secrets::MemoryStore::default()),
            config_path: None,
            cache: cache::Cache::none(),
            keys: certificates::Keys::default(),
            invites: securejoin::Invites::default(),
            calls: calls::Calls::default(),
            last_refresh: Instant::now(),
            last_item_refresh: Instant::now(),
            indexer: indexing::Indexer::none(),
        }
    }

    /// Loads the configured accounts from `config_path` (sample mail when there are none) and
    /// starts connecting them.
    pub fn load(secrets: Arc<dyn SecretStore>, config_path: Option<std::path::PathBuf>) -> Self {
        let accounts = config_path.as_deref().map(config::load).unwrap_or_default();
        let (tx, rx) = channel();
        let keys = certificates::Keys::default();
        let worker = worker::Worker::new(tx, keys.clone());
        let calendar = config_path.as_deref().map(|p| config::load_json::<calendar::CalendarFile>(&config::calendar_path(p))).unwrap_or_default();
        let mut engine = Engine {
            session: Session::default(),
            worker: Some(worker),
            events: rx,
            secrets,
            config_path,
            cache: cache::Cache::none(),
            keys,
            invites: securejoin::Invites::default(),
            calls: calls::Calls::default(),
            last_refresh: Instant::now(),
            last_item_refresh: Instant::now(),
            indexer: indexing::Indexer::none(),
        };
        if accounts.is_empty() {
            engine.session = Session::sample();
        } else {
            engine.session = Session::with_accounts(accounts);
            // What the accounts showed last time, until the servers answer.
            engine.cache = cache::Cache::open(engine.config_path.as_deref(), &*engine.secrets);
            engine.cache.restore(&mut engine.session);
            engine.indexer = indexing::Indexer::open(engine.config_path.as_deref(), engine.cache.store());
            engine.load_identities();
            engine.load_people();
            engine.load_pgp_keys();
            engine.load_invites();
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
            engine.session.set_tasks_data(tasks);
        }
        let rules = engine.config_path.as_deref().map(|p| config::load_json::<rules::RulesFile>(&config::rules_path(p))).unwrap_or_default();
        engine.session.set_rules(rules.rules);
        let junk = engine.config_path.as_deref().map(|p| config::load_json::<merel_model::JunkLists>(&config::junk_path(p))).unwrap_or_default();
        engine.session.set_junk(junk);
        let ignored = engine.config_path.as_deref().map(|p| config::load_json::<ignore::IgnoredFile>(&config::ignored_path(p))).unwrap_or_default();
        engine.session.set_ignored(ignored.ignored);
        let follow_ups =
            engine.config_path.as_deref().map(|p| config::load_json::<followup::FollowUpFile>(&config::follow_ups_path(p))).unwrap_or_default();
        engine.session.set_follow_ups(follow_ups.follow_ups);
        let snoozes = engine.config_path.as_deref().map(|p| config::load_json::<snooze::SnoozeFile>(&config::snoozes_path(p))).unwrap_or_default();
        engine.session.set_snoozes(snoozes.snoozes);
        let categories =
            engine.config_path.as_deref().map(|p| config::load_json::<categories::CategoriesFile>(&config::categories_path(p))).unwrap_or_default();
        engine.session.set_categories(categories.categories);
        let steps =
            engine.config_path.as_deref().map(|p| config::load_json::<quicksteps::QuickStepsFile>(&config::quick_steps_path(p))).unwrap_or_default();
        engine.session.set_quick_steps(steps.quick_steps);
        let groups = engine.config_path.as_deref().map(|p| config::load_json::<groups::GroupsFile>(&config::groups_path(p))).unwrap_or_default();
        engine.session.set_groups(groups.groups);
        let templates =
            engine.config_path.as_deref().map(|p| config::load_json::<templates::TemplatesFile>(&config::templates_path(p))).unwrap_or_default();
        engine.session.set_templates(templates.templates);
        engine.dispatch();
        engine
    }

    /// Saves whatever the last command changed.
    fn persist(&mut self) -> Result<(), CommandError> {
        let Some(path) = self.config_path.clone() else {
            self.session.take_calendar_dirty();
            self.session.take_contacts_dirty();
            self.session.take_tasks_dirty();
            self.session.take_rules_dirty();
            self.session.take_junk_dirty();
            self.session.take_ignored_dirty();
            self.session.take_follow_ups_dirty();
            self.session.take_snoozes_dirty();
            self.session.take_categories_dirty();
            self.session.take_quick_steps_dirty();
            self.session.take_groups_dirty();
            self.session.take_templates_dirty();
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
        if self.session.take_rules_dirty() {
            config::save_json(&config::rules_path(&path), &self.session.rules_data()).map_err(|e| CommandError::Store(e.to_string()))?;
        }
        if self.session.take_junk_dirty() {
            config::save_json(&config::junk_path(&path), &self.session.junk_data()).map_err(|e| CommandError::Store(e.to_string()))?;
        }
        if self.session.take_ignored_dirty() {
            config::save_json(&config::ignored_path(&path), &self.session.ignored_data()).map_err(|e| CommandError::Store(e.to_string()))?;
        }
        if self.session.take_follow_ups_dirty() {
            config::save_json(&config::follow_ups_path(&path), &self.session.follow_ups_data()).map_err(|e| CommandError::Store(e.to_string()))?;
        }
        if self.session.take_snoozes_dirty() {
            config::save_json(&config::snoozes_path(&path), &self.session.snoozes_data()).map_err(|e| CommandError::Store(e.to_string()))?;
        }
        if self.session.take_categories_dirty() {
            config::save_json(&config::categories_path(&path), &self.session.categories_data()).map_err(|e| CommandError::Store(e.to_string()))?;
        }
        if self.session.take_quick_steps_dirty() {
            config::save_json(&config::quick_steps_path(&path), &self.session.quick_steps_data()).map_err(|e| CommandError::Store(e.to_string()))?;
        }
        if self.session.take_groups_dirty() {
            config::save_json(&config::groups_path(&path), &self.session.groups_data()).map_err(|e| CommandError::Store(e.to_string()))?;
        }
        if self.session.take_templates_dirty() {
            config::save_json(&config::templates_path(&path), &self.session.templates_data()).map_err(|e| CommandError::Store(e.to_string()))?;
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
                self.session.status.insert(account.id, AccountStatus::Error(merel_i18n::tr("no-credentials")));
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
            self.session.sign_in = Some(Some(Err(merel_i18n::tr("signin-sample"))));
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
            self.session.sign_in = Some(Some(Err(merel_i18n::tr("conn-no-thread"))));
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

    /// Replaces the name, address, user name and servers of a password account, and its
    /// password when a new one is given, then connects it again. The id, the signature and the
    /// kind of sign-in stay as they were, so its folders, messages and offline cache stay with it.
    pub fn update_account(&mut self, mut account: Account, password: Option<Secret>) -> Result<(), String> {
        let existing =
            self.session.accounts.iter().find(|a| a.id == account.id && a.id != SAMPLE_ACCOUNT).ok_or_else(|| merel_i18n::tr("account-gone"))?;
        if !matches!(existing.auth, Auth::Password) {
            return Err(merel_i18n::tr("account-is-oauth"));
        }
        account.signature = existing.signature.clone();
        account.auth = Auth::Password;
        if let Some(password) = password {
            self.secrets.set(&account.id, &password).map_err(|e| e.to_string())?;
        }
        if let Some(worker) = &mut self.worker {
            worker.stop_account(&account.id);
        }
        self.session.add_account(account.clone());
        self.save_accounts()?;
        self.spawn_account(&account.id);
        self.dispatch();
        Ok(())
    }

    /// Removes an account for good: its worker stops, its password or sign-in and its
    /// offline cache are deleted, and the accounts file is rewritten. Mail on the server is
    /// not touched.
    pub fn remove_account(&mut self, account_id: &str) -> Result<(), String> {
        if !self.session.accounts.iter().any(|a| a.id == account_id) || account_id == SAMPLE_ACCOUNT {
            return Err(merel_i18n::tr("account-gone"));
        }
        if let Some(worker) = &mut self.worker {
            worker.stop_account(account_id);
        }
        let mut problems = Vec::new();
        if let Err(e) = self.secrets.delete(account_id) {
            problems.push(merel_i18n::tr_args("remove-keychain-password", &[("error", e.to_string().into())]));
        }
        if let Err(e) = self.secrets.delete(&signin::client_secret_key(account_id)) {
            problems.push(merel_i18n::tr_args("remove-keychain-app", &[("error", e.to_string().into())]));
        }
        self.cache.remove(account_id);
        self.indexer.remove_account(account_id);
        self.remove_certificate(account_id);
        self.remove_pgp_key(account_id);
        self.session.remove_account(account_id);
        self.save_accounts()?;
        self.persist().map_err(|e| e.to_string())?;
        if problems.is_empty() { Ok(()) } else { Err(merel_i18n::tr_args("remove-keychain-failed", &[("problems", problems.join("; ").into())])) }
    }

    /// Stores the signature of `account_id` and rewrites the accounts file.
    pub fn set_signature(&mut self, account_id: &str, signature: &str) -> Result<(), String> {
        let account = self.session.accounts.iter_mut().find(|a| a.id == account_id).ok_or_else(|| merel_i18n::tr("account-gone"))?;
        account.signature = merel_model::clean_signature(signature);
        self.save_accounts()
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
        self.index(false);
        self.dispatch();
        self.persist()?;
        self.cache.save(&self.session, false);
        r
    }

    /// Writes the offline cache now; for when the window closes.
    pub fn flush(&mut self) {
        self.cache.save(&self.session, true);
        self.indexer.flush(&self.session);
    }

    /// Keeps the search index in step: messages that left their place leave it, it writes
    /// what changed, its next jobs go out, and the search box gets its answer.
    fn index(&mut self, force: bool) {
        let left = std::mem::take(&mut self.session.left);
        self.indexer.forget(&left);
        let jobs = self.indexer.tick(&self.session, force);
        if let Some(worker) = &self.worker {
            for j in jobs {
                worker.submit(j);
            }
        }
        self.indexer.search(&mut self.session);
    }

    /// Searches for `words` at once, in every folder of every account or in the selected folder
    /// only (whose server is asked too), as a phone's search field does: the index answers
    /// before this returns, [`Session::visible_messages`] lists the result.
    pub fn search(&mut self, words: &str, all: bool) {
        self.session.search = words.to_string();
        self.session.search_all = all;
        if !all {
            self.session.request_server_search();
            self.dispatch();
        }
        self.indexer.search(&mut self.session);
    }

    /// How far the search index is.
    pub fn index_progress(&self) -> indexing::Progress {
        self.indexer.progress()
    }

    pub fn index_settings(&self) -> &indexing::IndexSettings {
        self.indexer.settings()
    }

    /// Where search on meaning is; `None` while it is off.
    pub fn meaning_status(&self) -> Option<meaning::MeaningStatus> {
        self.indexer.meaning_status()
    }

    pub fn retry_meaning(&mut self) {
        self.indexer.retry_meaning();
    }

    /// Whether there is a search index to ask (it is on and its key was there).
    pub fn index_on(&self) -> bool {
        self.indexer.is_on()
    }

    pub fn set_index_settings(&mut self, settings: indexing::IndexSettings) -> Result<(), String> {
        self.indexer.set_settings(settings)?;
        self.index(false);
        Ok(())
    }

    /// Empties the search index; it fills again from the mail.
    pub fn clear_index(&mut self) -> Result<(), String> {
        self.indexer.clear()?;
        self.index(false);
        Ok(())
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
        // Messages whose moment has come go out now.
        let due = self.session.due_scheduled(drafts::now());
        if !due.is_empty() {
            for id in due {
                self.session.send_scheduled(&id);
            }
            self.cache.note_commands(&self.session);
            changed = true;
        }
        // New mail comes in on its own: open folders every two minutes, items every five.
        if self.last_refresh.elapsed() >= REFRESH_EVERY {
            self.last_refresh = Instant::now();
            self.session.refresh_open_folders();
            self.refresh_chat_folders();
        }
        if self.last_item_refresh.elapsed() >= ITEMS_EVERY {
            self.last_item_refresh = Instant::now();
            self.session.refresh_items();
        }
        while let Ok(mut e) = self.events.try_recv() {
            let signed_in = match &e {
                Event::SignedIn { result: Ok(a) } => Some(a.id.clone()),
                _ => None,
            };
            if !self.session.concerns_current(&e) {
                continue;
            }
            self.cache.note_event(&self.session, &e);
            self.indexer.note_event(&self.session, &e);
            self.take_in(&mut e);
            // Changes made offline reach the server before anything else does.
            let replay = match &e {
                Event::Connected { account_id, .. } => self.cache.take_pending(account_id),
                _ => Vec::new(),
            };
            let sent = matches!(e, Event::Sent { .. });
            let listed = match &e {
                Event::Headers { folder_id, .. } => Some(folder_id.clone()),
                _ => None,
            };
            self.session.apply(e);
            if let Some(folder_id) = listed {
                self.open_sealed_in(&folder_id);
            }
            if sent {
                // A scheduled message that went out, or came back as a draft, changes the drafts.
                self.cache.note_commands(&self.session);
            }
            self.session.prepend_jobs(replay);
            if let Some(id) = signed_in {
                if let Err(e) = self.save_accounts() {
                    self.session.sign_in = Some(Some(Err(e)));
                }
                self.spawn_account(&id);
            }
            changed = true;
        }
        self.index(false);
        self.dispatch();
        self.cache.save(&self.session, false);
        changed
    }

    /// What an opened message teaches before the session shows it: the keys and certificates
    /// it carries, and an invitation's handshake step to answer.
    pub(crate) fn take_in(&mut self, e: &mut Event) {
        let Event::Body { message_id, body, header, raw, carried } = e else { return };
        self.learn_certificate(message_id, header.from.as_ref(), body);
        let carried = carried.take();
        match &carried {
            Some(carried) => self.learn_carried(message_id, header.from.as_ref(), body, carried),
            None => self.learn_autocrypt(message_id, header.from.as_ref(), raw),
        }
        self.handle_handshake(message_id, header, body, carried.as_deref());
        self.handle_call(message_id, header, body);
        self.send_waiting();
    }

    fn dispatch(&mut self) {
        let jobs = self.session.take_jobs();
        let jobs = self.cache.serve(&mut self.session, &self.keys, jobs);
        let Some(worker) = &self.worker else { return };
        for j in jobs {
            if j.account_id() != SAMPLE_ACCOUNT {
                worker.submit(j);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use merel_model::{Address, Protocol, Server, Tls};

    use super::*;

    pub(crate) fn account(id: &str) -> Account {
        Account {
            id: id.into(),
            name: id.into(),
            address: Address::new(None, &format!("{id}@voorbeeld.nl")).unwrap(),
            username: id.into(),
            protocol: Protocol::ImapSmtp,
            auth: Auth::Password,
            incoming: Some(Server { host: "127.0.0.1".into(), port: 1, tls: Tls::Implicit }),
            outgoing: None,
            dav: None,
            signature: String::new(),
        }
    }

    pub(crate) fn header(n: u32) -> RemoteHeader {
        RemoteHeader {
            id: n.to_string(),
            seen: true,
            flagged: false,
            answered: false,
            size: 1,
            has_attachments: false,
            header: merel_mime::parse_header(format!("From: x@y.nl\r\nSubject: Bericht {n}\r\n\r\n").as_bytes()),
            date: i64::from(n),
            categories: Vec::new(),
        }
    }

    #[test]
    fn a_subfolder_nests_under_its_parent_when_the_server_lists_that_too() {
        let mut s = Session::with_accounts(vec![account("a")]);
        let f = |path: &str, kind, parent: Option<&str>, selectable| RemoteFolder {
            path: path.into(),
            name: path.rsplit('/').next().unwrap_or(path).into(),
            kind,
            selectable,
            parent: parent.map(str::to_string),
        };
        let folders = vec![
            f("INBOX", FolderKind::Inbox, None, true),
            f("INBOX/HUIS", FolderKind::Other, Some("INBOX"), true),
            f("[Gmail]", FolderKind::Other, None, false),
            f("[Gmail]/Belangrijk", FolderKind::Other, Some("[Gmail]"), true),
        ];
        s.apply(Event::Connected { account_id: "a".into(), folders, syncs_items: false });
        assert_eq!(s.folder("a/INBOX/HUIS").and_then(|f| f.parent.as_deref()), Some("a/INBOX"));
        assert_eq!(s.folder("a/INBOX/HUIS").map(|f| f.name.as_str()), Some("HUIS"));
        assert_eq!(s.folder("a/[Gmail]/Belangrijk").and_then(|f| f.parent.as_deref()), None, "its parent cannot be opened: it stands at the top");
    }

    #[test]
    fn the_inbox_unread_count_sums_every_account_and_follows_the_flags() {
        let mut s = Session::sample();
        let before = s.inbox_unread();
        assert!(before > 0, "the sample inbox has unread mail");
        assert_eq!(before, s.folder("inbox").map(|f| f.unread).unwrap_or(0));
        let other: Vec<String> = s.folders.iter().filter(|f| f.kind != FolderKind::Inbox).map(|f| f.id.clone()).collect();
        for id in &other {
            s.run("folder.mark_all_read", &serde_json::json!({ "id": id })).unwrap();
        }
        assert_eq!(s.inbox_unread(), before, "other folders do not count");
        s.run("folder.mark_all_read", &serde_json::json!({ "id": "inbox" })).unwrap();
        assert_eq!(s.inbox_unread(), 0);
    }

    #[test]
    fn a_server_search_lists_what_the_server_found_next_to_local_matches() {
        let mut s = Session::with_accounts(vec![account("a")]);
        s.apply(Event::Connected {
            account_id: "a".into(),
            folders: vec![RemoteFolder { path: "INBOX".into(), name: "Inbox".into(), kind: FolderKind::Inbox, selectable: true, parent: None }],
            syncs_items: false,
        });
        s.apply(Event::Headers { folder_id: "a/INBOX".into(), headers: vec![header(1), header(2)], offset: 0, limit: HEADER_LIMIT });
        s.selected_folder = Some("a/INBOX".into());
        s.take_jobs();
        assert!(!s.can_search_server(), "nothing to search for yet");
        s.search = "is:ongelezen".into();
        assert!(!s.can_search_server(), "operators alone are this computer's to apply");
        s.search = "factuur van:y.nl ".into();
        assert!(s.can_search_server());
        s.request_server_search();
        assert!(s.searching_server());
        let jobs = s.take_jobs();
        assert_eq!(jobs, vec![Job::Search { account_id: "a".into(), folder_id: "a/INBOX".into(), path: "INBOX".into(), query: "factuur".into() }]);
        s.request_server_search();
        assert!(s.take_jobs().is_empty(), "one search at a time");
        assert!(s.visible_messages().is_empty(), "no local match for that word");

        // The server names a listed message and one the listing did not hold.
        s.apply(Event::Found { folder_id: "a/INBOX".into(), query: "factuur".into(), result: Ok(vec![header(2), header(9)]) });
        assert!(!s.searching_server());
        let mut ids: Vec<&str> = s.visible_messages().iter().map(|m| m.id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(ids, ["a/INBOX#2", "a/INBOX#9"], "the server searched 'factuur' alone; van:y.nl holds for its hits");
        s.search = "factuur is:ongelezen".into();
        assert!(s.visible_messages().is_empty(), "the same server hits, now held to a flag they lack");
        s.search = "factuur van:y.nl ".into();
        assert_eq!(s.messages.len(), 3, "the older hit joined the listing once");
        s.search = "factuur 2026".into();
        assert!(s.current_hits().is_none() && s.visible_messages().is_empty(), "other words, other answer");
        s.search = "factuur".into();
        assert_eq!(s.visible_messages().len(), 2);
        s.search.clear();
        assert_eq!(s.visible_messages().len(), 3, "the folder shows everything listed, hits included");

        s.search = "x".into();
        s.apply(Event::Found { folder_id: "a/INBOX".into(), query: "x".into(), result: Err("De server gaf een fout: nee".into()) });
        assert_eq!(s.search_error.as_deref(), Some("De server gaf een fout: nee"));
        assert!(s.current_hits().is_none());
        s.remove_account("a");
        assert!(s.server_hits.is_none());
    }

    #[test]
    fn older_mail_is_fetched_page_by_page_until_the_folder_starts() {
        let mut s = Session::with_accounts(vec![account("a")]);
        s.apply(Event::Connected {
            account_id: "a".into(),
            folders: vec![RemoteFolder { path: "INBOX".into(), name: "Inbox".into(), kind: FolderKind::Inbox, selectable: true, parent: None }],
            syncs_items: false,
        });
        s.take_jobs();
        let folder = "a/INBOX";
        s.apply(Event::Headers { folder_id: folder.into(), headers: (1..=HEADER_LIMIT).map(header).collect(), offset: 0, limit: HEADER_LIMIT });
        assert!(s.can_load_older(folder), "a full first page may have more below it");
        s.run("folder.older", &serde_json::Value::Null).unwrap();
        assert!(!s.can_load_older(folder), "not while the page is on its way");
        let jobs = s.take_jobs();
        assert!(matches!(&jobs[..], [Job::Headers { folder_id, offset, .. }] if folder_id == folder && *offset == HEADER_LIMIT), "{jobs:?}");
        // The page below: one overlaps (the server shifted), two are new, and it is short.
        s.apply(Event::Headers {
            folder_id: folder.into(),
            headers: vec![header(1), header(0), header(999)],
            offset: HEADER_LIMIT,
            limit: HEADER_LIMIT,
        });
        assert_eq!(s.messages.len() as u32, HEADER_LIMIT + 2);
        assert!(!s.can_load_older(folder), "a short page means the folder's start was reached");
        s.run("folder.older", &serde_json::Value::Null).unwrap();
        assert!(s.take_jobs().is_empty());
        // A refresh from the top with a full page opens the folder up again.
        s.apply(Event::Headers { folder_id: folder.into(), headers: (1..=HEADER_LIMIT).map(header).collect(), offset: 0, limit: HEADER_LIMIT });
        assert_eq!(s.messages.len() as u32, HEADER_LIMIT);
        assert!(s.can_load_older(folder));
    }

    #[test]
    fn a_refresh_keeps_loaded_pages_and_counts_new_unread_mail() {
        let mut s = Session::with_accounts(vec![account("a")]);
        s.apply(Event::Connected {
            account_id: "a".into(),
            folders: vec![RemoteFolder { path: "INBOX".into(), name: "Inbox".into(), kind: FolderKind::Inbox, selectable: true, parent: None }],
            syncs_items: false,
        });
        s.take_jobs();
        let folder = "a/INBOX";
        // The first load lists unread mail without counting it as new.
        let mut first: Vec<RemoteHeader> = (1..=HEADER_LIMIT).map(header).collect();
        first[0].seen = false;
        s.apply(Event::Headers { folder_id: folder.into(), headers: first, offset: 0, limit: HEADER_LIMIT });
        assert!(s.take_arrivals().is_empty());
        s.run("folder.older", &serde_json::Value::Null).unwrap();
        s.take_jobs();
        s.apply(Event::Headers { folder_id: folder.into(), headers: (201..=400).map(header).collect(), offset: HEADER_LIMIT, limit: HEADER_LIMIT });
        assert_eq!(s.messages.len(), 400);
        // A refresh asks for everything listed, so the second page is not lost.
        s.refresh_open_folders();
        let jobs = s.take_jobs();
        assert!(matches!(&jobs[..], [Job::Headers { folder_id, offset: 0, limit: 400, .. }] if folder_id == folder), "{jobs:?}");
        let mut again: Vec<RemoteHeader> = (1..=400).map(header).collect();
        let mut fresh = header(401);
        fresh.seen = false;
        again.insert(0, fresh);
        s.apply(Event::Headers { folder_id: folder.into(), headers: again, offset: 0, limit: 400 });
        assert_eq!(s.messages.len(), 401);
        assert_eq!(s.take_arrivals().len(), 1, "one unread message that was not listed before");
        assert!(s.take_arrivals().is_empty());
        // Only connected accounts are refreshed.
        s.status.insert("a".into(), AccountStatus::Offline);
        s.refresh_open_folders();
        assert!(s.take_jobs().is_empty());
    }

    #[test]
    fn a_search_filters_what_has_been_fetched() {
        let mut s = Session::sample();
        let all = s.visible_messages().len();
        assert!(all > 1);
        s.search = "  DONDERDAG ".into();
        let found = s.visible_messages();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].subject, "Planning voor donderdag");
        assert!(s.is_searching());
        s.search = "donderdag planning".into();
        assert_eq!(s.visible_messages().len(), 1, "every word must match, in any order");
        s.search = "donderdag zzzz".into();
        assert!(s.visible_messages().is_empty());
        // Sender names count too.
        let from = s.messages[0].from.display().to_string();
        s.search = from.clone();
        assert!(s.visible_messages().iter().all(|m| m.from.display() == from) && !s.visible_messages().is_empty());
        // Across folders, every matching message of every account is listed.
        s.search = "e".into();
        let in_folder = s.visible_messages().len();
        s.search_all = true;
        let everywhere = s.visible_messages().len();
        assert!(everywhere >= in_folder && everywhere == s.messages.iter().filter(|m| search::Query::parse("e").matches(m)).count());
        s.search.clear();
        assert_eq!(s.visible_messages().len(), all, "an empty search shows the folder again");

        // Outlook's search words narrow by one field or flag.
        s.search_all = true;
        s.search = "is:ongelezen".into();
        let unread = s.visible_messages();
        assert!(!unread.is_empty() && unread.iter().all(|m| !m.flags.seen));
        assert_eq!(unread.len(), s.messages.iter().filter(|m| !m.flags.seen && s.snoozed_until(m, snooze::now()).is_none()).count());
        s.search = "heeft:bijlage".into();
        assert!(s.visible_messages().iter().all(|m| m.has_attachments));
        let sender = s.messages[0].from.email().to_string();
        s.search = format!("van:{sender}");
        let from = s.visible_messages();
        assert!(!from.is_empty() && from.iter().all(|m| m.from.email() == sender));
        s.search = format!("onderwerp:{sender}");
        assert!(s.visible_messages().is_empty(), "an address is not a subject");
    }

    #[test]
    fn removing_an_account_clears_its_traces_and_keeps_local_items() {
        let mut engine = Engine::sample();
        engine.add_account(account("a"), Secret::new("pw")).unwrap();
        engine.add_account(account("b"), Secret::new("pw")).unwrap();
        engine.session.run("appointment.save", &serde_json::json!({ "title": "Lokaal", "start": 10, "end": 20 })).unwrap();
        let s = &mut engine.session;
        s.apply(Event::Connected {
            account_id: "a".into(),
            folders: vec![RemoteFolder { path: "INBOX".into(), name: "Inbox".into(), kind: FolderKind::Inbox, selectable: true, parent: None }],
            syncs_items: true,
        });
        s.apply(Event::Synced {
            account_id: "a".into(),
            appointments: vec![Appointment {
                id: "x".into(),
                calendar_id: String::new(),
                title: "Van a".into(),
                start: 0,
                end: 1,
                all_day: false,
                location: String::new(),
                notes: String::new(),
                notes_rich: None,
                recurrence: None,
                reminder: None,
                private: false,
                show_as: merel_model::ShowAs::Busy,
                categories: Vec::new(),
                meeting: None,
                online: None,
                make_online: false,
            }],
            contacts: Vec::new(),
            tasks: Vec::new(),
        });
        s.selected_folder = Some("a/INBOX".into());
        assert!(s.appointments.iter().any(|a| a.title == "Van a"));
        assert!(s.calendars.iter().any(|c| c.account_id.as_deref() == Some("a")));

        engine.remove_account("a").unwrap();
        let s = &engine.session;
        assert_eq!(s.accounts.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["b"]);
        assert!(s.folders.is_empty() && s.selected_folder.is_none());
        assert!(s.appointments.iter().all(|a| a.title != "Van a") && s.appointments.iter().any(|a| a.title == "Lokaal"));
        assert!(!s.calendars.iter().any(|c| c.account_id.as_deref() == Some("a")));
        assert!(!s.item_accounts.contains("a") && !s.status.contains_key("a"));
        assert!(engine.secrets.get("a").unwrap().is_none(), "the password leaves the keychain");
        assert!(engine.secrets.get("b").unwrap().is_some());
        // A late report from the stopped worker changes nothing.
        let late = Event::Connected { account_id: "a".into(), folders: Vec::new(), syncs_items: false };
        assert!(!engine.session.concerns_current(&late));

        engine.remove_account("b").unwrap();
        assert!(engine.session.is_sample(), "the last account gone brings the sample mail back");
        assert!(engine.session.appointments.iter().any(|a| a.title == "Lokaal"), "local items survive");
        assert!(engine.remove_account("b").is_err());
    }

    #[test]
    fn secret_names_cover_the_cache_key_and_every_account() {
        let engine = Engine::sample();
        let names = engine.secret_names();
        assert_eq!(names[0], merel_store::KEY_NAME);
        for account in &engine.session.accounts {
            assert!(names.contains(&account.id));
            assert!(names.contains(&signin::client_secret_key(&account.id)));
        }
    }

    #[test]
    fn editing_an_account_keeps_its_id_signature_and_mail_and_connects_again() {
        let mut engine = Engine::sample();
        engine.add_account(account("a"), Secret::new("pw")).unwrap();
        engine.set_signature("a", "Groet").unwrap();
        let s = &mut engine.session;
        s.apply(Event::Connected {
            account_id: "a".into(),
            folders: vec![RemoteFolder { path: "INBOX".into(), name: "Inbox".into(), kind: FolderKind::Inbox, selectable: true, parent: None }],
            syncs_items: false,
        });
        s.apply(Event::Headers { folder_id: "a/INBOX".into(), headers: vec![header(1)], offset: 0, limit: HEADER_LIMIT });
        s.loading.insert("a/INBOX".into());
        s.take_jobs();

        let mut changed = account("a");
        changed.name = "Alain".into();
        changed.username = "alain".into();
        changed.incoming = Some(Server { host: "imap.voorbeeld.nl".into(), port: 143, tls: Tls::StartTls });
        changed.signature = "weg".into();
        engine.update_account(changed, None).unwrap();
        let s = &engine.session;
        let a = &s.accounts[0];
        assert_eq!((a.id.as_str(), a.name.as_str(), a.username.as_str(), a.signature.as_str()), ("a", "Alain", "alain", "Groet"));
        assert_eq!(a.incoming.as_ref().map(|i| i.port), Some(143));
        assert_eq!(s.messages.len(), 1, "fetched mail stays with the account");
        assert!(!s.loading.contains("a/INBOX"), "what the old connection was fetching is forgotten");
        assert_eq!(s.status.get("a"), Some(&AccountStatus::Connecting));
        assert_eq!(engine.secrets.get("a").unwrap().map(|p| p.expose().to_string()), Some("pw".into()), "no new password keeps the old one");

        engine.update_account(account("a"), Some(Secret::new("new"))).unwrap();
        assert_eq!(engine.secrets.get("a").unwrap().map(|p| p.expose().to_string()), Some("new".into()));
        assert!(engine.update_account(account("nope"), None).is_err());
    }
    #[test]
    fn a_calendar_server_that_fails_leaves_the_mail_online() {
        let mut s = Session::default();
        s.add_account(account("a"));
        let connected = || Event::Connected { account_id: "a".into(), folders: Vec::new(), syncs_items: false };
        s.apply(connected());
        s.apply(Event::ItemsFailed { account_id: "a".into(), what: "Agenda en adresboek: weg.".into() });
        assert_eq!(s.status.get("a"), Some(&AccountStatus::Online));
        assert_eq!(s.item_errors.get("a").map(String::as_str), Some("Agenda en adresboek: weg."));
        // The next connection tries the calendar again; until it says otherwise, all is well.
        s.apply(connected());
        assert!(s.item_errors.is_empty());
        s.apply(Event::ItemsFailed { account_id: "a".into(), what: "x".into() });
        s.remove_account("a");
        assert!(s.item_errors.is_empty());
    }
    #[test]
    fn a_job_that_fails_is_said_once_and_leaves_the_account_online() {
        let mut s = Session::default();
        s.add_account(account("a"));
        s.apply(Event::Connected { account_id: "a".into(), folders: Vec::new(), syncs_items: false });
        s.loading.insert("a/Oud".into());
        let failed = || Event::JobFailed { account_id: "a".into(), loading: Some("a/Oud".into()), what: "Bij a@b.nl lukte iets niet.".into() };
        s.apply(failed());
        s.apply(failed());
        assert_eq!(s.status.get("a"), Some(&AccountStatus::Online), "the account keeps refreshing");
        assert!(!s.loading.contains("a/Oud"));
        assert_eq!(s.take_job_failures(), vec!["Bij a@b.nl lukte iets niet.".to_string()]);
        assert!(s.take_job_failures().is_empty());
    }

    #[test]
    fn a_message_that_is_gone_leaves_the_list() {
        let mut s = Session::sample();
        let id = s.messages[0].id.clone();
        s.selected_message = Some(id.clone());
        s.loading.insert(id.clone());
        s.apply(Event::Gone { account_id: SAMPLE_ACCOUNT.into(), folder_id: None, message_id: Some(id.clone()) });
        assert!(s.message(&id).is_none() && s.selected_message.is_none() && !s.loading.contains(&id));
    }
}
