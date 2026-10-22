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

use merel_engine::{AccountStatus, Engine};
use merel_model::{Account, Address, Auth, FolderKind, Protocol, Server, Tls};
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
    /// or `outgoing`.
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

    /// The open folder's messages, filtered, searched and sorted as the session says.
    pub fn messages(&self) -> Vec<MessageRow> {
        let engine = self.engine();
        let s = &engine.session;
        s.visible_messages().into_iter().map(|m| row(m, s.selected_message.as_deref())).collect()
    }

    /// The selected message, or `None` when nothing is open.
    pub fn open_message(&self) -> Option<MessageView> {
        let engine = self.engine();
        let s = &engine.session;
        let m = s.selected()?;
        let body = s.selected_body();
        Some(MessageView {
            row: row(m, Some(m.id.as_str())),
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

fn row(m: &merel_model::MessageHeader, selected: Option<&str>) -> MessageRow {
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
        selected: selected == Some(m.id.as_str()),
    }
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
}
