//! Merel's core for the iPhone and Android apps.
//!
//! The apps draw their own screens (SwiftUI, Jetpack Compose) and leave everything else to the
//! same engine the desktop app runs: accounts, mail protocols, the offline cache, encryption and
//! chat. This crate puts that engine behind a small UniFFI interface. The app calls named
//! commands, exactly as the desktop's buttons do (`run("message.select", {"id": …})`), calls
//! `poll` to take in what the background worker found, and reads plain rows to draw.
//!
//! Secrets stay in the phone's own store: the app hands in a [`SecretVault`] backed by the iOS
//! keychain or the Android Keystore, so this crate never needs `keyring` on a phone.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use merel_engine::calendar::describe_recurrence;
use merel_engine::{AccountStatus, CommandError, Engine, Session};
use merel_model::{Account, Address, Appointment, Auth, FolderKind, Protocol, Response, Server, Tls};
use merel_secrets::{Secret, SecretError, SecretStore};

uniffi::setup_scaffolding!();

/// Why the phone's secret store refused a request.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum VaultError {
    #[error("the secret store refused the request: {reason}")]
    Refused { reason: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for VaultError {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        VaultError::Refused { reason: e.reason }
    }
}

/// The phone's own secret store, written in Swift (keychain, this device only, after first
/// unlock) or Kotlin (Android Keystore). Passwords, tokens and the offline cache's key live there.
#[uniffi::export(with_foreign)]
pub trait SecretVault: Send + Sync {
    fn get(&self, key: String) -> Result<Option<String>, VaultError>;
    fn set(&self, key: String, secret: String) -> Result<(), VaultError>;
    fn delete(&self, key: String) -> Result<(), VaultError>;
}

/// The engine's view of a [`SecretVault`].
struct Vault(Arc<dyn SecretVault>);

impl SecretStore for Vault {
    fn set(&self, key: &str, secret: &Secret) -> Result<(), SecretError> {
        self.0.set(key.to_string(), secret.expose().to_string()).map_err(|e| SecretError::Store(e.to_string()))
    }

    fn get(&self, key: &str) -> Result<Option<Secret>, SecretError> {
        let found = self.0.get(key.to_string()).map_err(|e| SecretError::Store(e.to_string()))?;
        Ok(found.map(Secret::new))
    }

    fn delete(&self, key: &str) -> Result<(), SecretError> {
        self.0.delete(key.to_string()).map_err(|e| SecretError::Store(e.to_string()))
    }
}

/// Why a command did not run.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum CoreError {
    /// The parameters were not JSON.
    #[error("parameters are not valid JSON: {reason}")]
    Params { reason: String },
    /// A field of a form is missing or not valid: `email`, `username`, `password`, `incoming`
    /// or `outgoing` of the account form; `title`, `end`, `calendar_id` or `recurrence` of the
    /// appointment form.
    #[error("invalid {field}")]
    Invalid { field: String },
    /// The engine refused the command; `reason` says why, for the log.
    #[error("{reason}")]
    Command { reason: String },
}

/// Whether an account is reachable, for the dot next to its name.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ConnectionState {
    Offline,
    Connecting,
    Online,
    Failed { reason: String },
}

/// An account as the folder list shows it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AccountRow {
    pub id: String,
    pub name: String,
    pub address: String,
    pub state: ConnectionState,
    /// Signs in with Microsoft or Google in the browser, so "Opnieuw aanmelden" can renew it.
    pub signs_in_with_browser: bool,
}

/// A service whose account signs in in the browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SignInProvider {
    Microsoft365,
    Gmail,
}

/// Where a browser sign-in is.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SignInState {
    /// None started, or the last one was put away.
    Idle,
    /// The browser is open; the provider has not sent the person back yet.
    Waiting,
    /// Signed in: the account is added and connecting.
    Done,
    Failed {
        reason: String,
    },
}

/// Opens a sign-in link in a browser the app shows, written in Swift (Safari in the app) or
/// Kotlin (a Custom Tab). The provider sends the person back to `http://127.0.0.1`, where
/// Merel listens for this one sign-in, as on the computer; the app never sees the password.
#[uniffi::export(with_foreign)]
pub trait SignInBrowser: Send + Sync {
    /// False when no browser could be opened.
    fn show(&self, url: String) -> bool;
}

/// Sets the browser sign-in links open in, once, when the app starts.
#[uniffi::export]
pub fn set_sign_in_browser(browser: Arc<dyn SignInBrowser>) {
    merel_engine::signin::set_browser(Box::new(move |url| if browser.show(url.to_string()) { Ok(()) } else { Err("no browser".into()) }));
}

/// What a folder is for, so the app can pick its icon and its place in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FolderRole {
    Inbox,
    Drafts,
    Sent,
    Archive,
    Junk,
    Deleted,
    Other,
}

impl From<FolderKind> for FolderRole {
    fn from(kind: FolderKind) -> Self {
        match kind {
            FolderKind::Inbox => FolderRole::Inbox,
            FolderKind::Drafts => FolderRole::Drafts,
            FolderKind::Sent => FolderRole::Sent,
            FolderKind::Archive => FolderRole::Archive,
            FolderKind::Junk => FolderRole::Junk,
            FolderKind::Deleted => FolderRole::Deleted,
            FolderKind::Other => FolderRole::Other,
        }
    }
}

/// A folder, with the folder it sits in so the app can nest the tree.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FolderRow {
    pub id: String,
    pub account_id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub role: FolderRole,
    pub unread: u32,
    pub selected: bool,
}

/// One line of the message list.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MessageRow {
    pub id: String,
    pub sender: String,
    pub sender_address: String,
    pub subject: String,
    pub preview: String,
    /// Seconds since the Unix epoch, UTC; the app shows it in the phone's time zone.
    pub date: i64,
    pub unread: bool,
    pub flagged: bool,
    pub has_attachments: bool,
    pub categories: Vec<String>,
    pub selected: bool,
    /// The folder the message is in and its account's address: the list shows them when it
    /// holds more than one folder (a search in all folders, an "Alle accounts" folder).
    pub folder_name: String,
    pub account_address: String,
    /// Found on meaning alone, without the search's words ("Over hetzelfde").
    pub related: bool,
}

/// Where search on meaning is: the small language model on the phone.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum MeaningState {
    /// Fetching the model, `done` of `total` bytes.
    Downloading {
        done: u64,
        total: u64,
    },
    /// Making the downloaded model ready.
    Preparing,
    Ready,
    Failed {
        reason: String,
    },
}

/// What the search settings show while search on meaning is on.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MeaningStatus {
    pub state: MeaningState,
    /// Messages the model has read.
    pub messages: u64,
    /// Whether it is still reading messages.
    pub busy: bool,
    /// Bytes the model takes on the phone.
    pub model_bytes: u64,
}

/// The open message: its envelope and, once it has arrived, its text.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MessageView {
    pub row: MessageRow,
    pub to: Vec<String>,
    /// `None` while the body is still on its way from the server.
    pub text: Option<String>,
    pub attachments: Vec<String>,
}

/// How a connection is secured; there is no plain-text choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ConnectionSecurity {
    /// TLS from the first byte (IMAP 993, SMTP 465).
    Tls,
    /// STARTTLS, required.
    StartTls,
}

impl From<ConnectionSecurity> for Tls {
    fn from(s: ConnectionSecurity) -> Self {
        match s {
            ConnectionSecurity::Tls => Tls::Implicit,
            ConnectionSecurity::StartTls => Tls::StartTls,
        }
    }
}

/// A mail server: host, port and how it is secured.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ServerSettings {
    pub host: String,
    pub port: u16,
    pub security: ConnectionSecurity,
}

/// What the "Account toevoegen" form asks for an IMAP/SMTP account.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ImapAccount {
    pub name: String,
    pub email: String,
    /// Empty: the e-mail address.
    pub username: String,
    pub incoming: ServerSettings,
    pub outgoing: ServerSettings,
}

/// A calendar: its name, the account it belongs to and the colour of its appointments.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CalendarRow {
    pub id: String,
    pub name: String,
    /// The account's address; `None` for a calendar that lives only on this phone.
    pub address: Option<String>,
    /// 0xRRGGBB.
    pub color: u32,
}

/// What someone invited to a meeting answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AttendeeAnswer {
    None,
    Accepted,
    Tentative,
    Declined,
}

impl From<Response> for AttendeeAnswer {
    fn from(r: Response) -> Self {
        match r {
            Response::None => AttendeeAnswer::None,
            Response::Accepted => AttendeeAnswer::Accepted,
            Response::Tentative => AttendeeAnswer::Tentative,
            Response::Declined => AttendeeAnswer::Declined,
        }
    }
}

/// One person of a meeting: the organiser first, then everyone invited.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AttendeeRow {
    pub name: String,
    pub email: String,
    pub answer: AttendeeAnswer,
    pub organizer: bool,
    pub optional: bool,
    /// One of the person's own addresses.
    pub me: bool,
}

/// An appointment, or one occurrence of a recurring one, as the day and the details show it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AppointmentRow {
    /// The series' id for an occurrence of a recurring appointment.
    pub id: String,
    pub calendar_id: String,
    pub title: String,
    /// Seconds since the Unix epoch. An all-day appointment runs from midnight to midnight on
    /// the phone's clock; `end` is exclusive.
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    pub location: String,
    pub notes: String,
    /// The calendar's colour, 0xRRGGBB.
    pub color: u32,
    /// How it repeats, in words ("Elke donderdag"); `None` for a single appointment.
    pub recurrence: Option<String>,
    /// Minutes before the start the reminder comes.
    pub reminder: Option<u32>,
    /// The link to join it online, only ever https.
    pub online_url: Option<String>,
    pub attendees: Vec<AttendeeRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TaskPriority {
    Low,
    Normal,
    High,
}

/// How a finished task comes back, every `interval` of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TaskRepeat {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

/// One task in the list.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TaskRow {
    pub id: String,
    pub title: String,
    pub notes: String,
    /// Local midnight of the due day, Unix seconds; `None` for no deadline.
    pub due: Option<i64>,
    pub priority: TaskPriority,
    pub done: bool,
    /// When it was finished, Unix seconds.
    pub completed: Option<i64>,
    /// The account it lives in and its address; `None` for a task kept on this phone.
    pub account_id: Option<String>,
    pub account_address: Option<String>,
    pub repeat: Option<TaskRepeat>,
    pub repeat_interval: u32,
    /// "Elke week", "Om de 2 weken", …: the repeat in words.
    pub repeat_text: Option<String>,
}

/// Where a new task can be kept: an account that stores tasks.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TaskPlace {
    pub account_id: String,
    pub address: String,
}

/// What the task form gives back.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TaskForm {
    /// `None` for a new task.
    pub id: Option<String>,
    /// For a new task: the account to keep it in, `None` for this phone.
    pub account_id: Option<String>,
    pub title: String,
    pub notes: String,
    pub due: Option<i64>,
    pub priority: TaskPriority,
    /// Only for a task on this phone, and it needs a due day.
    pub repeat: Option<TaskRepeat>,
    pub repeat_interval: u32,
}

/// What a phone number is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PhoneKind {
    Mobile,
    Work,
    Home,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PhoneNumber {
    pub kind: PhoneKind,
    pub number: String,
}

/// A postal address as the lines to show, and whether it is for work or home.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PostalAddress {
    pub work: bool,
    pub home: bool,
    pub lines: Vec<String>,
}

/// One contact, as the list and the contact card show it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactRow {
    pub id: String,
    pub name: String,
    /// The first one is where mail goes.
    pub emails: Vec<String>,
    pub phones: Vec<PhoneNumber>,
    pub company: String,
    pub job_title: String,
    pub department: String,
    pub website: String,
    /// `YYYY-MM-DD`.
    pub birthday: Option<String>,
    pub addresses: Vec<PostalAddress>,
    pub notes: String,
    /// Whether [`MerelCore::contact_photo`] has a picture.
    pub has_photo: bool,
    /// The account it lives in and its address; `None` for a contact kept on this phone.
    pub account_id: Option<String>,
    pub account_address: Option<String>,
}

/// What the contact form gives back. What the form does not show (addresses, birthday,
/// photo, …) stays as it was.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactForm {
    /// `None` for a new contact.
    pub id: Option<String>,
    /// For a new contact: the account to keep it in, `None` for this phone.
    pub account_id: Option<String>,
    pub name: String,
    pub company: String,
    pub job_title: String,
    pub emails: Vec<String>,
    pub phones: Vec<PhoneNumber>,
    pub notes: String,
}

/// What the appointment form gives back.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AppointmentForm {
    /// `None` for a new appointment.
    pub id: Option<String>,
    /// `None`: the appointment's own calendar, or the first one for a new appointment.
    pub calendar_id: Option<String>,
    pub title: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    pub location: String,
    pub notes: String,
    pub reminder: Option<u32>,
}

/// A chat as the chat list shows it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChatRow {
    /// `group:<account>:<group id>` or `person:<account>:<address>`.
    pub id: String,
    pub account_id: String,
    pub account_address: String,
    /// The group's name, else the other person's name or address.
    pub title: String,
    /// The address the avatar is made from: the other person's, or the group's first.
    pub address: String,
    pub people: Vec<String>,
    pub is_group: bool,
    /// When the last message came, in seconds since the Unix epoch.
    pub last: i64,
    pub unread: u32,
    /// The last message's text, `None` while it is still encrypted on the server.
    pub preview: Option<String>,
    pub last_mine: bool,
    /// The first message waits until the others have answered the invitation Merel sent them.
    pub waiting: bool,
}

/// One message in a chat.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChatMessageRow {
    pub id: String,
    /// `None` until it is fetched and decrypted.
    pub text: Option<String>,
    pub mine: bool,
    pub sender_name: String,
    pub sender_address: String,
    pub date: i64,
    /// The invitation link the message holds, to accept it with a tap.
    pub invite: Option<String>,
}

/// What a new message starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ComposeKind {
    New,
    /// To the open message's sender, quoting it.
    Reply,
    /// To the sender and everyone else on the open message but the account itself.
    ReplyAll,
    /// The open message's text under a "Doorgestuurd bericht" header, with no recipient yet.
    Forward,
}

/// A message being written: addresses as typed, separated by commas or semicolons.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ComposeForm {
    pub account_id: String,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub body: String,
    /// The Message-ID this answers, so the reply stays in its conversation.
    pub in_reply_to: Option<String>,
}

/// Where a message handed to [`MerelCore::send_mail`] is.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SendState {
    Sending,
    Sent,
    Failed { reason: String },
}

/// The engine, shared by the app's screens.
#[derive(uniffi::Object)]
pub struct MerelCore {
    engine: Mutex<Engine>,
}

impl MerelCore {
    fn engine(&self) -> MutexGuard<'_, Engine> {
        // A panic elsewhere must not lock the app out of its mail.
        self.engine.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[uniffi::export]
impl MerelCore {
    /// Opens the accounts in `data_dir` (the app's own private folder) and starts connecting
    /// them; with no accounts there, sample mail.
    #[uniffi::constructor]
    pub fn new(data_dir: String, vault: Arc<dyn SecretVault>) -> Arc<Self> {
        let config = PathBuf::from(data_dir).join("accounts.json");
        Arc::new(MerelCore { engine: Mutex::new(Engine::load(Arc::new(Vault(vault)), Some(config))) })
    }

    /// Sample mail only, nothing on disk and no network: for previews and screenshots.
    #[uniffi::constructor]
    pub fn sample() -> Arc<Self> {
        Arc::new(MerelCore { engine: Mutex::new(Engine::sample()) })
    }

    /// Runs the named command with its parameters as JSON (`""` for none), the same commands
    /// the desktop's buttons run.
    pub fn run(&self, command: String, params_json: String) -> Result<(), CoreError> {
        let params = if params_json.trim().is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_str(&params_json).map_err(|e| CoreError::Params { reason: e.to_string() })?
        };
        self.engine().run(&command, &params).map_err(|e| CoreError::Command { reason: e.to_string() })
    }

    /// Adds an IMAP/SMTP account: the password goes to the vault, the account is saved and
    /// starts connecting. Returns the new account's id.
    pub fn add_imap_account(&self, form: ImapAccount, password: String) -> Result<String, CoreError> {
        let account = imap_account(&form, now())?;
        if password.is_empty() {
            return Err(CoreError::Invalid { field: "password".into() });
        }
        let id = account.id.clone();
        self.engine().add_account(account, Secret::new(password)).map_err(|reason| CoreError::Command { reason })?;
        Ok(id)
    }

    /// Starts signing in with Microsoft 365 or Gmail: the browser opens on the provider's page
    /// and [`sign_in_state`](Self::sign_in_state) follows it. An empty client ID takes Merel's
    /// own registration with Microsoft; Google needs the person's own client ID and secret.
    pub fn sign_in(&self, provider: SignInProvider, name: String, email: String, client_id: String, client_secret: String) -> Result<(), CoreError> {
        let protocol = match provider {
            SignInProvider::Microsoft365 => Protocol::Microsoft365,
            SignInProvider::Gmail => Protocol::Gmail,
        };
        if Address::new(None, email.trim()).is_err() {
            return Err(CoreError::Invalid { field: "email".into() });
        }
        if protocol.builtin_client_id().is_none() && client_id.trim().is_empty() {
            return Err(CoreError::Invalid { field: "client_id".into() });
        }
        let client_secret = match client_secret.trim() {
            "" if protocol == Protocol::Gmail => return Err(CoreError::Invalid { field: "client_secret".into() }),
            "" => None,
            secret => Some(Secret::new(secret)),
        };
        let account = merel_engine::signin::account(protocol, &name, &email, &client_id).map_err(|reason| CoreError::Command { reason })?;
        self.engine().sign_in(merel_engine::signin::SignIn { account, client_secret });
        Ok(())
    }

    /// Signs an account in again in the browser, with the provider's current permissions.
    pub fn sign_in_again(&self, account_id: String) {
        self.engine().sign_in_again(&account_id);
    }

    pub fn sign_in_state(&self) -> SignInState {
        match &self.engine().session.sign_in {
            None => SignInState::Idle,
            Some(None) => SignInState::Waiting,
            Some(Some(Ok(()))) => SignInState::Done,
            Some(Some(Err(reason))) => SignInState::Failed { reason: reason.clone() },
        }
    }

    /// Puts a finished sign-in away, or stops waiting for one the person left.
    pub fn clear_sign_in(&self) {
        self.engine().session.sign_in = None;
    }

    /// True while only the sample mail is there, so the app can offer to add an account.
    pub fn is_sample(&self) -> bool {
        self.engine().session.is_sample()
    }

    /// Takes in what the background worker has found; true when the screen should be redrawn.
    pub fn poll(&self) -> bool {
        self.engine().poll()
    }

    /// Writes the offline cache now; for when the app goes to the background.
    pub fn flush(&self) {
        self.engine().flush();
    }

    pub fn accounts(&self) -> Vec<AccountRow> {
        let engine = self.engine();
        let s = &engine.session;
        s.accounts
            .iter()
            .map(|a| AccountRow {
                id: a.id.clone(),
                name: a.name.clone(),
                address: a.address.email().to_string(),
                state: match s.status.get(&a.id) {
                    Some(AccountStatus::Online) => ConnectionState::Online,
                    Some(AccountStatus::Connecting) => ConnectionState::Connecting,
                    Some(AccountStatus::Error(reason)) => ConnectionState::Failed { reason: reason.clone() },
                    Some(AccountStatus::Offline) | None => ConnectionState::Offline,
                },
                signs_in_with_browser: matches!(a.auth, Auth::OAuth2 { .. }),
            })
            .collect()
    }

    pub fn folders(&self) -> Vec<FolderRow> {
        let engine = self.engine();
        let s = &engine.session;
        s.folders
            .iter()
            .map(|f| FolderRow {
                id: f.id.clone(),
                account_id: f.account_id.clone(),
                parent_id: f.parent.clone(),
                name: f.name.clone(),
                role: f.kind.into(),
                unread: f.unread,
                selected: s.selected_folder.as_deref() == Some(f.id.as_str()),
            })
            .collect()
    }

    /// The "Alle accounts" folders, which show the mail of every account together (Postvak IN,
    /// Concepten, …); none while there is only one account. Their `account_id` is empty.
    pub fn all_accounts_folders(&self) -> Vec<FolderRow> {
        let engine = self.engine();
        let s = &engine.session;
        s.unified_folders()
            .into_iter()
            .map(|f| FolderRow {
                selected: s.selected_folder.as_deref() == Some(f.id.as_str()),
                id: f.id,
                account_id: String::new(),
                parent_id: None,
                name: f.name,
                role: f.kind.into(),
                unread: f.unread,
            })
            .collect()
    }

    /// The open folder's messages, filtered, searched and sorted as the session says.
    pub fn messages(&self) -> Vec<MessageRow> {
        let engine = self.engine();
        let s = &engine.session;
        s.visible_messages().into_iter().map(|m| row(s, m)).collect()
    }

    /// Searches as the computer's search box does: the search index, words such as `van:` and
    /// `bijlage:`, the text of attachments. `all_folders` looks in every folder of every
    /// account, else in the open folder (whose server is asked too). An empty `words` ends
    /// the search; [`MerelCore::messages`] lists what was found, best first.
    pub fn search(&self, words: String, all_folders: bool) {
        self.engine().search(words.trim_start(), all_folders);
    }

    /// Whether search on meaning is switched on.
    pub fn search_on_meaning(&self) -> bool {
        self.engine().index_settings().meaning
    }

    /// Switches search on meaning on (the model is downloaded once, see
    /// [`meaning_download_megabytes`]) or off (the model and its vectors are removed).
    pub fn set_search_on_meaning(&self, on: bool) -> Result<(), CoreError> {
        let mut engine = self.engine();
        let settings = merel_engine::indexing::IndexSettings { meaning: on, ..engine.index_settings().clone() };
        engine.set_index_settings(settings).map_err(|reason| CoreError::Command { reason })
    }

    /// Where search on meaning is; `None` while it is off.
    pub fn meaning_status(&self) -> Option<MeaningStatus> {
        use merel_engine::meaning::ModelState;
        let status = self.engine().meaning_status()?;
        let state = match status.model {
            ModelState::Downloading { done, total } => MeaningState::Downloading { done, total },
            ModelState::Preparing | ModelState::Off => MeaningState::Preparing,
            ModelState::Ready => MeaningState::Ready,
            ModelState::Failed(reason) => MeaningState::Failed { reason },
        };
        Some(MeaningStatus { state, messages: status.vectors as u64, busy: status.busy, model_bytes: status.model_size })
    }

    /// Tries the download again after it failed.
    pub fn retry_meaning(&self) {
        self.engine().retry_meaning();
    }

    pub fn calendars(&self) -> Vec<CalendarRow> {
        let engine = self.engine();
        let s = &engine.session;
        s.calendars
            .iter()
            .map(|c| CalendarRow {
                id: c.id.clone(),
                name: c.name.clone(),
                address: c.account_id.as_deref().and_then(|id| s.accounts.iter().find(|a| a.id == id)).map(|a| a.address.email().to_string()),
                color: rgb(c.color),
            })
            .collect()
    }

    /// The appointments touching `[from, to)`, earliest first; a recurring one once per
    /// occurrence.
    pub fn appointments(&self, from: i64, to: i64) -> Vec<AppointmentRow> {
        let engine = self.engine();
        let s = &engine.session;
        s.appointments_between(from, to).iter().map(|a| appointment_row(s, a)).collect()
    }

    /// Saves a new appointment, or changes the one with `form.id`. A recurring appointment is
    /// changed on the computer for now: the phone's form has no series fields.
    pub fn save_appointment(&self, form: AppointmentForm) -> Result<(), CoreError> {
        let mut engine = self.engine();
        let mut params = serde_json::json!({
            "title": form.title,
            "start": form.start,
            "end": form.end,
            "all_day": form.all_day,
            "location": form.location,
            "notes": form.notes,
            "reminder": form.reminder,
        });
        if let Some(id) = &form.id {
            let stored = engine.session.appointment(id).ok_or(CoreError::Invalid { field: "id".into() })?;
            if stored.recurrence.is_some() {
                return Err(CoreError::Invalid { field: "recurrence".into() });
            }
            // What the phone's form does not show stays as it was.
            params["private"] = stored.private.into();
            if stored.notes == form.notes
                && let Some(rich) = &stored.notes_rich
            {
                params["notes_rich"] = serde_json::to_value(rich).unwrap_or_default();
            }
            params["id"] = id.clone().into();
        }
        if let Some(c) = &form.calendar_id {
            params["calendar_id"] = c.clone().into();
        }
        engine.run("appointment.save", &params).map_err(command_error)
    }

    /// Deletes the appointment `id`; with `occurrence` (its start), only that occurrence of a
    /// recurring one.
    pub fn delete_appointment(&self, id: String, occurrence: Option<i64>) -> Result<(), CoreError> {
        let mut engine = self.engine();
        let recurring = engine.session.appointment(&id).is_some_and(|a| a.recurrence.is_some());
        match occurrence {
            Some(start) if recurring => engine.run("appointment.skip", &serde_json::json!({ "id": id, "start": start })),
            _ => engine.run("appointment.delete", &serde_json::json!({ "id": id })),
        }
        .map_err(command_error)
    }

    /// The tasks: open ones first by due day and priority, then (with `include_done`) the
    /// finished ones, newest first.
    pub fn tasks(&self, include_done: bool) -> Vec<TaskRow> {
        use merel_engine::tasks::TaskFilter;
        let engine = self.engine();
        let s = &engine.session;
        s.tasks_filtered(if include_done { TaskFilter::All } else { TaskFilter::Open }).into_iter().map(|t| task_row(s, t)).collect()
    }

    /// The accounts a new task can be kept in, besides this phone.
    pub fn task_places(&self) -> Vec<TaskPlace> {
        let engine = self.engine();
        engine.session.item_accounts().into_iter().map(|a| TaskPlace { account_id: a.id.clone(), address: a.address.email().to_string() }).collect()
    }

    /// Saves a new task, or changes the one with `form.id`.
    pub fn save_task(&self, form: TaskForm) -> Result<(), CoreError> {
        use merel_model::Frequency;
        let mut engine = self.engine();
        let priority = match form.priority {
            TaskPriority::Low => "low",
            TaskPriority::Normal => "normal",
            TaskPriority::High => "high",
        };
        let repeat = form.repeat.map(|r| {
            let frequency = match r {
                TaskRepeat::Daily => Frequency::Daily,
                TaskRepeat::Weekly => Frequency::Weekly,
                TaskRepeat::Monthly => Frequency::Monthly,
                TaskRepeat::Yearly => Frequency::Yearly,
            };
            serde_json::json!({ "frequency": frequency, "interval": form.repeat_interval.max(1) })
        });
        let mut params = serde_json::json!({
            "title": form.title,
            "notes": form.notes,
            "due": form.due,
            "priority": priority,
            "repeat": repeat,
        });
        match &form.id {
            Some(id) => {
                let stored = engine.session.task(id).ok_or(CoreError::Invalid { field: "id".into() })?;
                // Formatting the phone does not show stays while the notes do.
                if stored.notes == form.notes
                    && let Some(rich) = &stored.notes_rich
                {
                    params["notes_rich"] = serde_json::to_value(rich).unwrap_or_default();
                }
                params["id"] = id.clone().into();
            }
            None => {
                if let Some(account) = &form.account_id {
                    params["account_id"] = account.clone().into();
                }
            }
        }
        engine.run("task.save", &params).map_err(command_error)
    }

    /// Marks the task done, or open again.
    pub fn complete_task(&self, id: String, done: bool) -> Result<(), CoreError> {
        self.engine().run("task.complete", &serde_json::json!({ "id": id, "done": done })).map_err(command_error)
    }

    pub fn delete_task(&self, id: String) -> Result<(), CoreError> {
        self.engine().run("task.delete", &serde_json::json!({ "id": id })).map_err(command_error)
    }

    /// The contacts holding every word of `query` (all of them for an empty one), by name.
    pub fn contacts(&self, query: String) -> Vec<ContactRow> {
        let engine = self.engine();
        let s = &engine.session;
        s.contacts_matching(&query).into_iter().map(|c| contact_row(s, c)).collect()
    }

    /// The contact's picture as the image file came (JPEG, PNG, GIF or WebP).
    pub fn contact_photo(&self, id: String) -> Option<Vec<u8>> {
        self.engine().session.contact(&id)?.photo.as_ref().map(|p| p.0.clone())
    }

    /// The accounts a new contact can be kept in, besides this phone.
    pub fn contact_places(&self) -> Vec<TaskPlace> {
        self.task_places()
    }

    /// Saves a new contact, or changes the one with `form.id`.
    pub fn save_contact(&self, form: ContactForm) -> Result<(), CoreError> {
        let mut engine = self.engine();
        let phones: Vec<merel_model::Phone> = form
            .phones
            .iter()
            .filter(|p| !p.number.trim().is_empty())
            .map(|p| {
                let kind = match p.kind {
                    PhoneKind::Mobile => merel_model::PhoneKind::Mobile,
                    PhoneKind::Work => merel_model::PhoneKind::Work,
                    PhoneKind::Home => merel_model::PhoneKind::Home,
                    PhoneKind::Other => merel_model::PhoneKind::Other,
                };
                merel_model::Phone::new(kind, p.number.trim())
            })
            .collect();
        let emails: Vec<String> = form.emails.iter().map(|e| e.trim().to_string()).filter(|e| !e.is_empty()).collect();
        let mut params = serde_json::json!({
            "name": form.name,
            "company": form.company,
            "job_title": form.job_title,
            "emails": emails,
            "phones": phones,
            "notes": form.notes,
        });
        match &form.id {
            Some(id) => {
                let stored = engine.session.contact(id).ok_or(CoreError::Invalid { field: "id".into() })?;
                // Formatting the phone does not show stays while the notes do.
                if stored.notes == form.notes
                    && let Some(rich) = &stored.notes_rich
                {
                    params["notes_rich"] = serde_json::to_value(rich).unwrap_or_default();
                }
                params["id"] = id.clone().into();
            }
            None => {
                if let Some(account) = &form.account_id {
                    params["account_id"] = account.clone().into();
                }
            }
        }
        engine.run("contact.save", &params).map_err(command_error)
    }

    pub fn delete_contact(&self, id: String) -> Result<(), CoreError> {
        self.engine().run("contact.delete", &serde_json::json!({ "id": id })).map_err(command_error)
    }

    /// The selected message, or `None` when nothing is open.
    /// Every chat, the one with the latest message first, and those whose first message waits
    /// for an invitation to be answered.
    pub fn chats(&self) -> Vec<ChatRow> {
        let engine = self.engine();
        let s = &engine.session;
        let address = |account_id: &str| s.accounts.iter().find(|a| a.id == account_id).map(|a| a.address.email().to_string()).unwrap_or_default();
        let mut out: Vec<ChatRow> = s
            .chats()
            .into_iter()
            .map(|c| {
                let last = c.messages.last();
                ChatRow {
                    account_address: address(&c.account_id),
                    address: c.people.first().map(|p| p.email().to_string()).unwrap_or_default(),
                    people: c.people.iter().map(|p| p.display().to_string()).collect(),
                    is_group: c.is_group(),
                    last: c.last,
                    unread: c.unread as u32,
                    preview: last.and_then(|id| s.chat_text(id)).map(|t| t.replace('\n', " ")),
                    last_mine: last.is_some_and(|id| s.chat_is_mine(id)),
                    waiting: false,
                    id: c.id,
                    account_id: c.account_id,
                    title: c.title,
                }
            })
            .collect();
        for w in engine.waiting() {
            let id = w.chat_id();
            if out.iter().any(|c| c.id == id) {
                continue;
            }
            let group = w.draft.chat.as_ref().filter(|c| c.group_id.is_some());
            let names: Vec<String> = w.draft.to.iter().map(|a| a.display().to_string()).collect();
            out.push(ChatRow {
                id,
                account_id: w.account_id.clone(),
                account_address: address(&w.account_id),
                title: group.and_then(|g| g.group_name.clone()).unwrap_or_else(|| names.join(", ")),
                address: w.draft.to.first().map(|a| a.email().to_string()).unwrap_or_default(),
                people: names,
                is_group: group.is_some(),
                last: w.since,
                unread: 0,
                preview: Some(w.draft.body.replace('\n', " ")),
                last_mine: true,
                waiting: true,
            });
        }
        out.sort_by_key(|c| std::cmp::Reverse(c.last));
        out
    }

    /// The messages of the chat `id`, oldest first; the first message of a waiting chat too.
    pub fn chat_messages(&self, id: String) -> Vec<ChatMessageRow> {
        let engine = self.engine();
        let s = &engine.session;
        let Some(chat) = s.chat(&id) else {
            return engine
                .waiting()
                .iter()
                .filter(|w| w.chat_id() == id)
                .map(|w| ChatMessageRow {
                    id: format!("waiting-{}", w.since),
                    text: Some(w.draft.body.clone()),
                    mine: true,
                    sender_name: String::new(),
                    sender_address: String::new(),
                    date: w.since,
                    invite: None,
                })
                .collect();
        };
        chat.messages
            .iter()
            .filter_map(|m| s.message(m))
            .map(|m| {
                let text = s.chat_text(&m.id);
                let invite = text.as_deref().and_then(|t| {
                    let at = t.find(merel_engine::securejoin::INVITE_LINK)?;
                    t[at..].split(char::is_whitespace).next().map(str::to_string)
                });
                ChatMessageRow {
                    id: m.id.clone(),
                    mine: s.chat_is_mine(&m.id),
                    sender_name: m.from.display().to_string(),
                    sender_address: m.from.email().to_string(),
                    date: m.date,
                    invite,
                    text,
                }
            })
            .collect()
    }

    /// Opens the chat `id`: fetches its latest messages and marks them read.
    pub fn open_chat(&self, id: String) {
        self.engine().open_chat(&id);
    }

    /// The accounts that can chat: IMAP and Microsoft 365.
    pub fn chat_places(&self) -> Vec<TaskPlace> {
        let engine = self.engine();
        engine
            .session
            .accounts
            .iter()
            .filter(|a| merel_engine::chat::can_chat(a))
            .map(|a| TaskPlace { account_id: a.id.clone(), address: a.address.email().to_string() })
            .collect()
    }

    /// Sends `text` in the chat `id`, end-to-end encrypted; to someone without a key yet it
    /// waits while an invitation goes out.
    pub fn send_in_chat(&self, id: String, text: String) -> Result<(), CoreError> {
        let mut engine = self.engine();
        let chat = engine.session.chat(&id).ok_or(CoreError::Invalid { field: "id".into() })?;
        let draft = engine.session.chat_reply(&chat, text.trim());
        engine.send_or_invite(&chat.account_id, draft).map(|_| ()).map_err(|reason| CoreError::Command { reason })
    }

    /// Starts a chat from `account_id` with `people` (addresses), a group when there are more
    /// or a `name` is given, and gives its id. The first chat of an account makes its key.
    pub fn start_chat(&self, account_id: String, people: Vec<String>, name: String, text: String) -> Result<String, CoreError> {
        let mut addresses = Vec::new();
        for p in people.iter().map(|p| p.trim()).filter(|p| !p.is_empty()) {
            addresses.push(Address::new(None, p).map_err(|_| CoreError::Invalid { field: "people".into() })?);
        }
        if addresses.is_empty() {
            return Err(CoreError::Invalid { field: "people".into() });
        }
        if text.trim().is_empty() {
            return Err(CoreError::Invalid { field: "text".into() });
        }
        let draft = merel_engine::chat::new_chat(&addresses, &name, text.trim()).map_err(|reason| CoreError::Command { reason })?;
        let mut engine = self.engine();
        if !engine.chat_ready(&account_id) {
            engine.enable_chat(&account_id).map_err(|reason| CoreError::Command { reason })?;
        }
        let id = match draft.chat.as_ref().and_then(|c| c.group_id.as_deref()) {
            Some(group) => merel_engine::chat::group_chat_id(&account_id, group),
            None => merel_engine::chat::person_chat_id(&account_id, draft.to.first().map(Address::email).unwrap_or("")),
        };
        engine.send_or_invite(&account_id, draft).map_err(|reason| CoreError::Command { reason })?;
        Ok(id)
    }

    /// The invitation link of `account_id`, to share or show as a QR code; turns chat on.
    pub fn invite_link(&self, account_id: String) -> Result<String, CoreError> {
        self.engine().invite_link(&account_id).map_err(|reason| CoreError::Command { reason })
    }

    /// Accepts an invitation link (or what its QR code says) on `account_id`; the chat with
    /// the inviter follows once the handshake is done. Gives that chat's id.
    pub fn accept_invite(&self, account_id: String, link: String) -> Result<String, CoreError> {
        let mut engine = self.engine();
        if !engine.chat_ready(&account_id) {
            engine.enable_chat(&account_id).map_err(|reason| CoreError::Command { reason })?;
        }
        engine.accept_invite(&account_id, link.trim()).map_err(|reason| CoreError::Command { reason })
    }

    /// The accounts mail can go from, for the "Van" choice.
    pub fn send_places(&self) -> Vec<TaskPlace> {
        let engine = self.engine();
        engine
            .session
            .accounts
            .iter()
            .filter(|a| a.id != "sample")
            .map(|a| TaskPlace { account_id: a.id.clone(), address: a.address.email().to_string() })
            .collect()
    }

    /// A new message: empty, or an answer to (or forward of) the open message, from the account
    /// it is in, with that account's signature above the quoted text.
    pub fn start_compose(&self, kind: ComposeKind) -> ComposeForm {
        let engine = self.engine();
        let s = &engine.session;
        let original = if kind == ComposeKind::New { None } else { s.selected() };
        let account = original
            .and_then(|m| s.folder(&m.folder_id))
            .and_then(|f| s.accounts.iter().find(|a| a.id == f.account_id))
            .or_else(|| s.current_account())
            .filter(|a| a.id != "sample")
            .or_else(|| s.accounts.iter().find(|a| a.id != "sample"));
        let mut form = ComposeForm {
            account_id: account.map(|a| a.id.clone()).unwrap_or_default(),
            to: String::new(),
            cc: String::new(),
            bcc: String::new(),
            subject: String::new(),
            body: String::new(),
            in_reply_to: None,
        };
        if let Some(m) = original {
            let text = s.selected_body().map(|b| b.text.replace("\r\n", "\n")).unwrap_or_default();
            let me = account.map(|a| a.address.email().to_string()).unwrap_or_default();
            let date = chrono::DateTime::from_timestamp(m.date, 0)
                .map(|d| d.with_timezone(&chrono::Local).format("%d-%m-%Y %H:%M").to_string())
                .unwrap_or_default();
            match kind {
                ComposeKind::New => {}
                ComposeKind::Reply | ComposeKind::ReplyAll => {
                    form.to = m.from.email().to_string();
                    if kind == ComposeKind::ReplyAll {
                        form.cc =
                            m.to.iter()
                                .filter(|a| !a.email().eq_ignore_ascii_case(&me) && !a.email().eq_ignore_ascii_case(m.from.email()))
                                .map(|a| a.email().to_string())
                                .collect::<Vec<_>>()
                                .join(", ");
                    }
                    form.subject = prefixed(&m.subject, "Re:");
                    form.in_reply_to = m.message_id.clone();
                    let header = merel_i18n::tr_args(
                        "reply-header",
                        &[("date", date.as_str().into()), ("name", m.from.display().into()), ("email", m.from.email().into())],
                    );
                    let quoted = text.lines().map(|l| format!("> {l}")).collect::<Vec<_>>().join("\n");
                    form.body = format!("\n\n{header}\n{quoted}");
                }
                ComposeKind::Forward => {
                    form.subject = prefixed(&m.subject, "Fw:");
                    let to: Vec<&str> = m.to.iter().map(|a| a.email()).collect();
                    let header = merel_i18n::tr_args(
                        "forward-header",
                        &[
                            ("name", m.from.display().into()),
                            ("email", m.from.email().into()),
                            ("date", date.as_str().into()),
                            ("subject", m.subject.as_str().into()),
                            ("to", to.join("; ").into()),
                        ],
                    );
                    form.body = format!("\n\n{header}\n\n{text}");
                }
            }
        }
        let signature = account.map(|a| a.signature.trim_matches(['\n', ' ', '\t']).to_string()).unwrap_or_default();
        if !signature.is_empty() {
            form.body = format!("\n\n-- \n{signature}{}", form.body);
        }
        form
    }

    /// Sends the message: the addresses are checked first, then it goes out (signed or
    /// encrypted the way the account's settings say) and lands in Verzonden items. Gives an id
    /// for [`send_state`](Self::send_state).
    pub fn send_mail(&self, form: ComposeForm) -> Result<u64, CoreError> {
        let stored = merel_model::StoredDraft {
            account_id: form.account_id.clone(),
            to: form.to,
            cc: form.cc,
            bcc: form.bcc,
            subject: form.subject,
            body: form.body,
            in_reply_to: form.in_reply_to,
            ..merel_model::StoredDraft::default()
        }
        .cleaned();
        let draft = stored.to_draft().map_err(|reason| CoreError::Command { reason })?;
        let mut engine = self.engine();
        if !engine.session.accounts.iter().any(|a| a.id == form.account_id && a.id != "sample") {
            return Err(CoreError::Invalid { field: "account_id".into() });
        }
        Ok(engine.send(&form.account_id, draft))
    }

    pub fn send_state(&self, id: u64) -> SendState {
        match self.engine().session.outbox.get(&id) {
            Some(merel_engine::SendState::Sent) => SendState::Sent,
            Some(merel_engine::SendState::Failed(reason)) => SendState::Failed { reason: reason.clone() },
            Some(merel_engine::SendState::Sending) | None => SendState::Sending,
        }
    }

    pub fn open_message(&self) -> Option<MessageView> {
        let engine = self.engine();
        let s = &engine.session;
        let m = s.selected()?;
        let body = s.selected_body();
        Some(MessageView {
            row: row(s, m),
            to: m.to.iter().map(|a| a.email().to_string()).collect(),
            text: body.map(|b| b.text.clone()),
            attachments: body.map(|b| b.attachments.iter().map(|a| a.name.clone()).collect()).unwrap_or_default(),
        })
    }
}

/// What the Android app calls once at start, before any connection: hands the platform
/// certificate verifier the app's context, so TLS is checked by Android's own trust store.
#[cfg(target_os = "android")]
mod android {
    use jni::errors::ThrowRuntimeExAndDefault;
    use jni::objects::{JClass, JObject};
    use jni::{EnvUnowned, jni_mangle};

    #[jni_mangle("nl.merel.app.MerelNative")]
    pub fn init<'caller>(mut unowned_env: EnvUnowned<'caller>, _class: JClass<'caller>, context: JObject<'caller>) {
        unowned_env.with_env(|env| rustls_platform_verifier::android::init_with_env(env, context)).resolve::<ThrowRuntimeExAndDefault>();
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The account `form` describes, checked the way the desktop's account form checks it.
fn imap_account(form: &ImapAccount, now: u64) -> Result<Account, CoreError> {
    let invalid = |field: &str| CoreError::Invalid { field: field.into() };
    let name = form.name.trim();
    let address = Address::new(Some(name), form.email.trim()).map_err(|_| invalid("email"))?;
    let username = if form.username.trim().is_empty() { address.email().to_string() } else { form.username.trim().to_string() };
    if username.chars().any(|c| c.is_control() || c == '"' || c == '\\') {
        return Err(invalid("username"));
    }
    let server = |s: &ServerSettings, field: &str| {
        let host = s.host.trim();
        if host.is_empty() || s.port == 0 || host.chars().any(|c| c.is_whitespace() || c.is_control() || c == '/') {
            return Err(invalid(field));
        }
        Ok(Server { host: host.to_string(), port: s.port, tls: s.security.into() })
    };
    let incoming = server(&form.incoming, "incoming")?;
    let outgoing = server(&form.outgoing, "outgoing")?;
    Ok(Account {
        id: format!("{}-{now}", address.email().replace(['@', '.'], "-")),
        name: if name.is_empty() { address.email().to_string() } else { name.to_string() },
        address,
        username,
        protocol: Protocol::ImapSmtp,
        auth: Auth::Password,
        incoming: Some(incoming),
        outgoing: Some(outgoing),
        dav: None,
        signature: String::new(),
    })
}

fn rgb([r, g, b]: [u8; 3]) -> u32 {
    (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
}

/// A field the engine named, for the form; anything else as it is.
fn task_row(s: &Session, t: &merel_model::Task) -> TaskRow {
    use merel_model::{Frequency, Priority};
    TaskRow {
        id: t.id.clone(),
        title: t.title.clone(),
        notes: t.notes.clone(),
        due: t.due,
        priority: match t.priority {
            Priority::Low => TaskPriority::Low,
            Priority::Normal => TaskPriority::Normal,
            Priority::High => TaskPriority::High,
        },
        done: t.done,
        completed: t.completed,
        account_id: t.account_id.clone(),
        account_address: t.account_id.as_deref().and_then(|id| s.accounts.iter().find(|a| a.id == id)).map(|a| a.address.email().to_string()),
        repeat: t.repeat.map(|r| match r.frequency {
            Frequency::Daily => TaskRepeat::Daily,
            Frequency::Weekly => TaskRepeat::Weekly,
            Frequency::Monthly => TaskRepeat::Monthly,
            Frequency::Yearly => TaskRepeat::Yearly,
        }),
        repeat_interval: t.repeat.map(|r| r.interval).unwrap_or(1),
        repeat_text: t.repeat.map(|r| merel_engine::calendar::repeat_text(Some(r.frequency), r.interval)),
    }
}

fn contact_row(s: &Session, c: &merel_model::Contact) -> ContactRow {
    use merel_model::PlaceKind;
    ContactRow {
        id: c.id.clone(),
        name: c.name.clone(),
        emails: c.emails.clone(),
        phones: c
            .phones
            .iter()
            .map(|p| PhoneNumber {
                kind: match p.kind {
                    merel_model::PhoneKind::Mobile => PhoneKind::Mobile,
                    merel_model::PhoneKind::Work => PhoneKind::Work,
                    merel_model::PhoneKind::Home => PhoneKind::Home,
                    merel_model::PhoneKind::Other => PhoneKind::Other,
                },
                number: p.number.clone(),
            })
            .collect(),
        company: c.company.clone(),
        job_title: c.job_title.clone(),
        department: c.department.clone(),
        website: c.website.clone(),
        birthday: c.birthday.clone(),
        addresses: c
            .places
            .iter()
            .map(|p| PostalAddress { work: p.kind == PlaceKind::Work, home: p.kind == PlaceKind::Home, lines: p.lines() })
            .filter(|a| !a.lines.is_empty())
            .collect(),
        notes: c.notes.clone(),
        has_photo: c.photo.is_some(),
        account_id: c.account_id.clone(),
        account_address: c.account_id.as_deref().and_then(|id| s.accounts.iter().find(|a| a.id == id)).map(|a| a.address.email().to_string()),
    }
}

fn command_error(e: CommandError) -> CoreError {
    match e {
        CommandError::Param(field) => CoreError::Invalid { field: field.into() },
        other => CoreError::Command { reason: other.to_string() },
    }
}

fn appointment_row(s: &Session, a: &Appointment) -> AppointmentRow {
    let mine = |email: &str| s.accounts.iter().any(|acc| acc.address.email().eq_ignore_ascii_case(email));
    let mut attendees = Vec::new();
    if let Some(m) = &a.meeting {
        attendees.push(AttendeeRow {
            name: if m.organizer_name.is_empty() { m.organizer.clone() } else { m.organizer_name.clone() },
            email: m.organizer.clone(),
            answer: AttendeeAnswer::Accepted,
            organizer: true,
            optional: false,
            me: mine(&m.organizer),
        });
        attendees.extend(m.attendees.iter().filter(|p| !p.email.eq_ignore_ascii_case(&m.organizer)).map(|p| AttendeeRow {
            name: if p.name.is_empty() { p.email.clone() } else { p.name.clone() },
            email: p.email.clone(),
            answer: p.response.into(),
            organizer: false,
            optional: p.optional,
            me: mine(&p.email),
        }));
    }
    AppointmentRow {
        id: a.id.clone(),
        calendar_id: a.calendar_id.clone(),
        title: a.title.clone(),
        start: a.start,
        end: a.end,
        all_day: a.all_day,
        location: a.location.clone(),
        notes: a.notes.clone(),
        color: s.calendar(&a.calendar_id).map_or(rgb(merel_model::CALENDAR_COLORS[0].1), |c| rgb(c.color)),
        recurrence: a.recurrence.as_ref().map(describe_recurrence),
        reminder: a.reminder,
        online_url: a.meeting_link().map(|l| l.url),
        attendees,
    }
}

fn row(s: &Session, m: &merel_model::MessageHeader) -> MessageRow {
    let folder = s.folder(&m.folder_id);
    let account = folder.and_then(|f| s.accounts.iter().find(|a| a.id == f.account_id));
    MessageRow {
        id: m.id.clone(),
        sender: m.from.display().to_string(),
        sender_address: m.from.email().to_string(),
        subject: m.subject.clone(),
        preview: m.preview.clone(),
        date: m.date,
        unread: !m.flags.seen,
        flagged: m.flags.flagged,
        has_attachments: m.has_attachments,
        categories: m.categories.clone(),
        selected: s.selected_message.as_deref() == Some(m.id.as_str()),
        folder_name: folder.map(|f| f.name.clone()).unwrap_or_default(),
        account_address: account.map(|a| a.address.email().to_string()).unwrap_or_default(),
        related: s.current_index_hits().is_some_and(|h| h.related.contains(&m.id)),
    }
}

/// `subject` with `prefix` ("Re:", "Fw:") in front, unless it already starts with it.
fn prefixed(subject: &str, prefix: &str) -> String {
    let s = subject.trim();
    if s.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase()) { s.to_string() } else { format!("{prefix} {s}") }
}

/// How many megabytes switching search on meaning on downloads, for the switch's explanation.
#[uniffi::export]
pub fn meaning_download_megabytes() -> u64 {
    merel_engine::meaning::download_megabytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct MemoryVault(Mutex<HashMap<String, String>>);

    impl SecretVault for MemoryVault {
        fn get(&self, key: String) -> Result<Option<String>, VaultError> {
            Ok(self.0.lock().unwrap().get(&key).cloned())
        }
        fn set(&self, key: String, secret: String) -> Result<(), VaultError> {
            self.0.lock().unwrap().insert(key, secret);
            Ok(())
        }
        fn delete(&self, key: String) -> Result<(), VaultError> {
            self.0.lock().unwrap().remove(&key);
            Ok(())
        }
    }

    #[test]
    fn the_sample_inbox_lists_and_opens_a_message() {
        let core = MerelCore::sample();
        let inbox = core.folders().into_iter().find(|f| f.role == FolderRole::Inbox).unwrap();
        core.run("folder.select".into(), format!(r#"{{"id":"{}"}}"#, inbox.id)).unwrap();
        let rows = core.messages();
        assert!(!rows.is_empty());
        let first = rows[0].id.clone();
        core.run("message.select".into(), format!(r#"{{"id":"{first}"}}"#)).unwrap();
        let open = core.open_message().unwrap();
        assert_eq!(open.row.id, first);
        assert!(open.row.selected);
        assert!(core.messages().iter().any(|m| m.id == first && m.selected));
    }

    #[test]
    fn search_looks_in_every_folder_or_the_open_one() {
        let core = MerelCore::sample();
        let deleted = core.folders().into_iter().find(|f| f.role == FolderRole::Deleted).unwrap();
        core.run("folder.select".into(), format!(r#"{{"id":"{}"}}"#, deleted.id)).unwrap();
        core.search("planning".into(), false);
        assert!(core.messages().is_empty(), "the open folder has no such mail");
        core.search("planning".into(), true);
        let found = core.messages();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].subject, "Planning voor donderdag");
        assert_eq!(found[0].folder_name, "Postvak IN");
        assert_eq!(found[0].account_address, "jij@example.org");
        core.search("van:sanne".into(), true);
        assert_eq!(core.messages().len(), 1, "the computer's search words work too");
        core.search(String::new(), true);
        assert!(core.messages().is_empty(), "no search: the open folder again");
    }

    #[test]
    fn a_reply_quotes_the_message_and_sending_checks_the_addresses() {
        let core = MerelCore::sample();
        core.run("message.select".into(), r#"{"id":"m2"}"#.into()).unwrap();
        let open = core.open_message().unwrap();
        let reply = core.start_compose(ComposeKind::Reply);
        assert_eq!(reply.subject, "Re: Offerte drukwerk", "Re: is not doubled");
        assert_eq!(reply.to, "ravi@example.org");
        if let Some(text) = open.text.as_deref().and_then(|t| t.lines().next()) {
            assert!(reply.body.contains(&format!("> {text}")), "{}", reply.body);
        }
        let forward = core.start_compose(ComposeKind::Forward);
        assert_eq!(forward.subject, "Fw: Re: Offerte drukwerk");
        assert!(forward.to.is_empty());
        let new = core.start_compose(ComposeKind::New);
        assert!(new.subject.is_empty() && new.body.is_empty() && new.account_id.is_empty(), "the sample sends from nowhere");
        assert!(core.send_places().is_empty());
        let bad = ComposeForm { to: "geen adres".into(), ..new.clone() };
        assert!(matches!(core.send_mail(bad), Err(CoreError::Command { .. })));
        let none = ComposeForm { to: String::new(), ..new.clone() };
        assert!(matches!(core.send_mail(none), Err(CoreError::Command { .. })));
        let sample = ComposeForm { to: "a@b.nl".into(), ..new };
        assert!(matches!(core.send_mail(sample), Err(CoreError::Invalid { field }) if field == "account_id"));
        assert_eq!(core.send_state(99), SendState::Sending);
    }

    #[test]
    fn a_new_chat_checks_its_people_and_text() {
        let core = MerelCore::sample();
        let field = |people: Vec<&str>, text: &str| match core.start_chat(
            "sample".into(),
            people.into_iter().map(String::from).collect(),
            String::new(),
            text.into(),
        ) {
            Err(CoreError::Invalid { field }) => field,
            other => panic!("{other:?}"),
        };
        assert_eq!(field(vec![], "Hoi"), "people");
        assert_eq!(field(vec!["geen adres"], "Hoi"), "people");
        assert_eq!(field(vec!["a@b.nl"], " "), "text");
        // The sample mail cannot chat: no key is made for it, nothing is sent.
        assert!(core.chat_places().is_empty());
        assert!(matches!(core.start_chat("sample".into(), vec!["a@b.nl".into()], String::new(), "Hoi".into()), Err(CoreError::Command { .. })));
        assert!(core.chat_messages("person:sample:a@b.nl".into()).is_empty());
    }

    #[test]
    fn a_browser_sign_in_checks_its_form_first() {
        let core = MerelCore::sample();
        let field =
            |provider, email: &str, id: &str, secret: &str| match core.sign_in(provider, String::new(), email.into(), id.into(), secret.into()) {
                Err(CoreError::Invalid { field }) => field,
                other => panic!("{other:?}"),
            };
        assert_eq!(field(SignInProvider::Microsoft365, "geen adres", "", ""), "email");
        assert_eq!(field(SignInProvider::Gmail, "a@gmail.com", "", "s"), "client_id");
        assert_eq!(field(SignInProvider::Gmail, "a@gmail.com", "id", " "), "client_secret");
        assert_eq!(core.sign_in_state(), SignInState::Idle);
        // The sample has no worker to sign in with, and says so.
        core.sign_in(SignInProvider::Microsoft365, String::new(), "a@outlook.com".into(), String::new(), String::new()).unwrap();
        assert!(matches!(core.sign_in_state(), SignInState::Failed { .. }));
        core.clear_sign_in();
        assert_eq!(core.sign_in_state(), SignInState::Idle);
    }

    #[test]
    fn tasks_are_added_finished_and_deleted() {
        let core = MerelCore::sample();
        let form = |title: &str| TaskForm {
            id: None,
            account_id: None,
            title: title.into(),
            notes: String::new(),
            due: Some(1_792_000_000),
            priority: TaskPriority::High,
            repeat: Some(TaskRepeat::Weekly),
            repeat_interval: 2,
        };
        let before = core.tasks(true).len();
        assert!(matches!(core.save_task(form("")), Err(CoreError::Invalid { field }) if field == "title"));
        core.save_task(form("Weekrapport")).unwrap();
        let t = core.tasks(false).into_iter().find(|t| t.title == "Weekrapport").unwrap();
        assert_eq!((t.priority, t.repeat, t.repeat_interval, t.account_id.clone()), (TaskPriority::High, Some(TaskRepeat::Weekly), 2, None));
        assert!(t.repeat_text.is_some());
        core.save_task(TaskForm { id: Some(t.id.clone()), title: "Weekrapport sturen".into(), ..form("") }).unwrap();
        assert_eq!(core.tasks(true).len(), before + 1, "changed, not added");
        core.complete_task(t.id.clone(), true).unwrap();
        assert!(!core.tasks(false).iter().any(|x| x.id == t.id), "done is out of the open list");
        assert_eq!(core.tasks(true).len(), before + 2, "the next one of the repeat came");
        core.delete_task(t.id.clone()).unwrap();
        assert!(!core.tasks(true).iter().any(|x| x.id == t.id));
    }

    #[test]
    fn contacts_are_added_changed_and_deleted() {
        let core = MerelCore::sample();
        let form = ContactForm {
            id: None,
            account_id: None,
            name: "Eva Jansen".into(),
            company: "Drukkerij Jansen".into(),
            job_title: String::new(),
            emails: vec!["eva@example.org".into(), " ".into()],
            phones: vec![PhoneNumber { kind: PhoneKind::Mobile, number: "06 12345678".into() }],
            notes: String::new(),
        };
        assert!(
            matches!(core.save_contact(ContactForm { name: String::new(), emails: Vec::new(), ..form.clone() }), Err(CoreError::Invalid { field }) if field == "name")
        );
        assert!(
            matches!(core.save_contact(ContactForm { emails: vec!["geen adres".into()], ..form.clone() }), Err(CoreError::Invalid { field }) if field == "emails")
        );
        core.save_contact(form.clone()).unwrap();
        let c = core.contacts("drukkerij".into()).into_iter().next().unwrap();
        assert_eq!((c.name.as_str(), c.emails.clone(), c.phones[0].kind), ("Eva Jansen", vec!["eva@example.org".to_string()], PhoneKind::Mobile));
        assert!(!c.has_photo && core.contact_photo(c.id.clone()).is_none());
        core.save_contact(ContactForm { id: Some(c.id.clone()), job_title: "Eigenaar".into(), ..form }).unwrap();
        assert_eq!(core.contacts("eva".into()).len(), 1, "changed, not added");
        assert_eq!(core.contacts("eva".into())[0].job_title, "Eigenaar");
        core.delete_contact(c.id).unwrap();
        assert!(core.contacts("eva".into()).is_empty());
    }

    #[test]
    fn one_account_has_no_all_accounts_folders() {
        assert!(MerelCore::sample().all_accounts_folders().is_empty());
    }

    #[test]
    fn bad_json_and_unknown_commands_are_errors_not_panics() {
        let core = MerelCore::sample();
        assert!(matches!(core.run("folder.select".into(), "{".into()), Err(CoreError::Params { .. })));
        assert!(matches!(core.run("no.such".into(), String::new()), Err(CoreError::Command { .. })));
    }

    fn form() -> ImapAccount {
        ImapAccount {
            name: "Alain".into(),
            email: "alain@example.org".into(),
            username: String::new(),
            incoming: ServerSettings { host: "mail.example.org".into(), port: 993, security: ConnectionSecurity::Tls },
            outgoing: ServerSettings { host: "mail.example.org".into(), port: 465, security: ConnectionSecurity::Tls },
        }
    }

    #[test]
    fn an_imap_form_becomes_an_account() {
        let a = imap_account(&form(), 7).unwrap();
        assert_eq!(a.id, "alain-example-org-7");
        assert_eq!(a.username, "alain@example.org");
        assert_eq!(a.incoming.unwrap().tls, Tls::Implicit);
        assert_eq!(a.protocol, Protocol::ImapSmtp);
    }

    #[test]
    fn a_bad_form_names_the_field() {
        let field = |f: ImapAccount| match imap_account(&f, 1) {
            Err(CoreError::Invalid { field }) => field,
            other => panic!("{other:?}"),
        };
        assert_eq!(field(ImapAccount { email: "geen adres".into(), ..form() }), "email");
        assert_eq!(field(ImapAccount { username: "a\"b".into(), ..form() }), "username");
        let mut f = form();
        f.incoming.host = "mail example".into();
        assert_eq!(field(f), "incoming");
        let mut f = form();
        f.outgoing.port = 0;
        assert_eq!(field(f), "outgoing");
    }

    #[test]
    fn adding_an_account_keeps_its_password_in_the_vault() {
        let dir = std::env::temp_dir().join(format!("merel-mobile-add-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let vault = Arc::new(MemoryVault::default());
        let core = MerelCore::new(dir.to_string_lossy().into_owned(), vault.clone());
        assert!(core.is_sample());
        assert!(matches!(core.add_imap_account(form(), String::new()), Err(CoreError::Invalid { .. })));
        let id = core.add_imap_account(form(), "geheim".into()).unwrap();
        assert_eq!(vault.get(id.clone()).unwrap().as_deref(), Some("geheim"));
        assert!(core.accounts().iter().any(|a| a.id == id));
        assert!(!core.is_sample());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_vault_keeps_secrets_for_the_engine() {
        let vault = Vault(Arc::new(MemoryVault::default()));
        vault.set("acc", &Secret::new("geheim")).unwrap();
        assert_eq!(vault.get("acc").unwrap().unwrap().expose(), "geheim");
        vault.delete("acc").unwrap();
        assert!(vault.get("acc").unwrap().is_none());
    }

    #[test]
    fn a_fresh_data_folder_starts_with_sample_mail() {
        let dir = std::env::temp_dir().join(format!("merel-mobile-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let core = MerelCore::new(dir.to_string_lossy().into_owned(), Arc::new(MemoryVault::default()));
        assert!(!core.folders().is_empty());
        core.flush();
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn day() -> (i64, i64) {
        let now = now() as i64;
        (now - 3 * 86_400, now + 3 * 86_400)
    }

    #[test]
    fn the_sample_calendar_lists_appointments_with_their_colour() {
        let core = MerelCore::sample();
        let calendars = core.calendars();
        assert!(!calendars.is_empty());
        let (from, to) = day();
        let list = core.appointments(from, to);
        let overleg = list.iter().find(|a| a.title == "Overleg ontwerp").unwrap();
        assert_eq!(overleg.color, calendars.iter().find(|c| c.id == overleg.calendar_id).unwrap().color);
        assert!(list.windows(2).all(|w| w[0].start <= w[1].start));
        assert!(list.iter().any(|a| a.recurrence.is_some()));
    }

    #[test]
    fn an_appointment_is_saved_changed_and_deleted() {
        let core = MerelCore::sample();
        let (from, to) = day();
        let start = from + 86_400;
        let form = AppointmentForm {
            id: None,
            calendar_id: None,
            title: "Tandarts".into(),
            start,
            end: start + 3_600,
            all_day: false,
            location: "Centrum".into(),
            notes: "Inbellen: https://zoom.us/j/42".into(),
            reminder: Some(15),
        };
        core.save_appointment(form.clone()).unwrap();
        let saved = core.appointments(from, to).into_iter().find(|a| a.title == "Tandarts").unwrap();
        assert_eq!(saved.reminder, Some(15));
        assert_eq!(saved.online_url.as_deref(), Some("https://zoom.us/j/42"));
        core.save_appointment(AppointmentForm { id: Some(saved.id.clone()), title: "Tandarts later".into(), ..form.clone() }).unwrap();
        assert!(core.appointments(from, to).iter().any(|a| a.id == saved.id && a.title == "Tandarts later"));
        core.delete_appointment(saved.id.clone(), Some(start)).unwrap();
        assert!(!core.appointments(from, to).iter().any(|a| a.id == saved.id));
        let field = |f: AppointmentForm| match core.save_appointment(f) {
            Err(CoreError::Invalid { field }) => field,
            other => panic!("{other:?}"),
        };
        assert_eq!(field(AppointmentForm { title: " ".into(), ..form.clone() }), "title");
        assert_eq!(field(AppointmentForm { end: start, ..form.clone() }), "end");
        assert_eq!(field(AppointmentForm { calendar_id: Some("nope".into()), ..form }), "calendar_id");
    }

    #[test]
    fn one_occurrence_of_a_series_can_go_and_the_series_is_not_edited_here() {
        let core = MerelCore::sample();
        let (from, to) = day();
        let weekly: Vec<AppointmentRow> = core.appointments(from, to + 14 * 86_400).into_iter().filter(|a| a.recurrence.is_some()).collect();
        let first = weekly[0].clone();
        let form = AppointmentForm {
            id: Some(first.id.clone()),
            calendar_id: None,
            title: "x".into(),
            start: first.start,
            end: first.end,
            all_day: false,
            location: String::new(),
            notes: String::new(),
            reminder: None,
        };
        assert!(matches!(core.save_appointment(form), Err(CoreError::Invalid { field }) if field == "recurrence"));
        core.delete_appointment(first.id.clone(), Some(first.start)).unwrap();
        let after = core.appointments(from, to + 14 * 86_400);
        assert!(!after.iter().any(|a| a.id == first.id && a.start == first.start));
        assert!(after.iter().any(|a| a.id == first.id));
    }

    #[test]
    fn a_meeting_lists_the_organiser_first_and_marks_me() {
        let core = MerelCore::sample();
        let me = core.accounts()[0].address.clone();
        let (from, to) = day();
        let start = from + 86_400;
        core.save_appointment(AppointmentForm {
            id: None,
            calendar_id: None,
            title: "Planning".into(),
            start,
            end: start + 3_600,
            all_day: false,
            location: String::new(),
            notes: String::new(),
            reminder: None,
        })
        .unwrap();
        let id = core.appointments(from, to).into_iter().find(|a| a.title == "Planning").unwrap().id;
        let meeting = serde_json::json!({ "uid": "u1", "organizer": "sanne@example.org", "organizer_name": "Sanne de Vries", "attendees": [
            { "email": "sanne@example.org", "response": "accepted" },
            { "email": me, "name": "Alain" },
            { "email": "ravi@example.org", "name": "Ravi Kumar", "response": "declined", "optional": true },
        ] });
        core.run(
            "appointment.save".into(),
            serde_json::json!({ "id": id, "title": "Planning", "start": start, "end": start + 3_600, "meeting": meeting }).to_string(),
        )
        .unwrap();
        let row = core.appointments(from, to).into_iter().find(|a| a.id == id).unwrap();
        let people: Vec<(&str, AttendeeAnswer, bool, bool)> = row.attendees.iter().map(|p| (p.name.as_str(), p.answer, p.organizer, p.me)).collect();
        assert_eq!(
            people,
            [
                ("Sanne de Vries", AttendeeAnswer::Accepted, true, false),
                ("Alain", AttendeeAnswer::None, false, true),
                ("Ravi Kumar", AttendeeAnswer::Declined, false, false),
            ]
        );
        assert!(row.attendees[2].optional);
    }
}
