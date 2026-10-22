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
use merel_model::FolderKind;
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
