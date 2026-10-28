//! The configuration files: JSON without secrets, readable by the owner only. `accounts.json`
//! holds the accounts; `calendar.json`, `contacts.json`, `tasks.json` and `rules.json` next to
//! it the local calendars, appointments, contacts, tasks and mail rules.

use std::path::{Path, PathBuf};

use merel_model::Account;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Where the accounts file lives for this user, if the platform has a config directory.
pub fn default_path() -> Option<PathBuf> {
    let dir = directories::ProjectDirs::from("nl", "", "Merel")?.config_dir().to_path_buf();
    let old = directories::ProjectDirs::from("nl", "ArtCraft", "Mailcraft").map(|d| d.config_dir().to_path_buf());
    Some(settle(&dir, old.as_deref()).join("accounts.json"))
}

/// The config directory to use: `dir`, after moving the folder from the time the app was
/// called Mailcraft (`old`) there on the first start; `old` itself when that move fails, so
/// no account, calendar or cached mail goes missing.
fn settle(dir: &Path, old: Option<&Path>) -> PathBuf {
    match old {
        Some(old) if !dir.exists() && old.is_dir() => {
            if let Some(parent) = dir.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::rename(old, dir).is_ok() { dir.to_path_buf() } else { old.to_path_buf() }
        }
        _ => dir.to_path_buf(),
    }
}

/// The calendar file that goes with an accounts file.
pub fn calendar_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("calendar.json")
}

/// The contacts file that goes with an accounts file.
pub fn contacts_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("contacts.json")
}

/// The tasks file that goes with an accounts file.
pub fn tasks_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("tasks.json")
}

/// The search index settings that go with an accounts file.
pub fn search_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("search.json")
}

/// The directory of the sealed search index that goes with an accounts file.
pub fn index_dir(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("search-index")
}

/// Where the Signal accounts keep their encrypted stores, one directory per account.
pub fn signal_dir(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("signal")
}

/// Where the model for search on meaning is kept, once downloaded.
pub fn model_dir(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("search-model")
}

/// The mail rules file that goes with an accounts file.
pub fn rules_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("rules.json")
}

/// The blocked and safe senders file that goes with an accounts file.
pub fn junk_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("junk.json")
}

/// What the learning junk filter learned, next to the accounts file.
pub fn learned_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("junk-learned.json")
}

/// The correspondents' S/MIME certificates, next to the accounts file.
pub fn certificates_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("certificates.json")
}

/// The correspondents' OpenPGP keys, next to the accounts file.
pub fn pgp_keys_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("pgp_keys.json")
}

/// The invitation tokens and verified chat keys, next to the accounts file.
pub fn invites_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("invites.json")
}

/// The ignored conversations, next to the accounts file.
pub fn ignored_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("ignored.json")
}

/// The follow-up dates of flagged messages, next to the accounts file.
pub fn follow_ups_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("follow_ups.json")
}

/// The snoozed-mail file that goes with an accounts file.
pub fn snoozes_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("snoozed.json")
}

/// The quick steps file that goes with an accounts file.
pub fn quick_steps_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("quick_steps.json")
}

/// The contact groups (distribution lists) file that goes with an accounts file.
pub fn groups_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("contact_groups.json")
}

/// The text templates file that goes with an accounts file.
pub fn templates_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("templates.json")
}

/// The categories (coloured labels) file that goes with an accounts file.
pub fn categories_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("categories.json")
}

/// The window's view choices ("Beeld" tab) that go with an accounts file.
pub fn view_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("view.json")
}

pub fn load(path: &Path) -> Vec<Account> {
    load_json(path)
}

pub fn save(path: &Path, accounts: &[Account]) -> std::io::Result<()> {
    save_json(path, &accounts)
}

/// The file's contents, or the default when it is missing or unreadable (which is logged).
pub fn load_json<T: Default + DeserializeOwned>(path: &Path) -> T {
    let Ok(bytes) = std::fs::read(path) else { return T::default() };
    match serde_json::from_slice::<T>(&bytes) {
        Ok(a) => a,
        Err(e) => {
            log::warn!("{} unreadable: {e}", path.display());
            T::default()
        }
    }
}

/// Writes `value` through a temporary file, so a crash never leaves a half-written one.
pub fn save_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use merel_model::{Address, Protocol, Server, Tls};

    #[test]
    fn round_trips_without_secrets() {
        let dir = std::env::temp_dir().join(format!("merel-test-{}", std::process::id()));
        let path = dir.join("accounts.json");
        let a = Account {
            id: "a1".into(),
            name: "Test".into(),
            address: Address::new(None, "a@b.nl").unwrap(),
            username: "a@b.nl".into(),
            protocol: Protocol::ImapSmtp,
            auth: merel_model::Auth::Password,
            incoming: Some(Server { host: "imap.b.nl".into(), port: 993, tls: Tls::Implicit }),
            outgoing: None,
            dav: None,
            signature: String::new(),
        };
        save(&path, std::slice::from_ref(&a)).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("password"));
        assert_eq!(load(&path), vec![a]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_mailcraft_folder_moves_to_the_new_name_once() {
        let root = std::env::temp_dir().join(format!("merel-settle-{}", std::process::id()));
        let (old, dir) = (root.join("mailcraft"), root.join("config").join("merel"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("accounts.json"), "[]").unwrap();
        assert_eq!(settle(&dir, Some(&old)), dir);
        assert!(dir.join("accounts.json").exists() && !old.exists(), "moved, not copied");
        std::fs::create_dir_all(&old).unwrap();
        assert_eq!(settle(&dir, Some(&old)), dir, "an existing new folder wins");
        assert_eq!(settle(&root.join("fresh"), Some(&root.join("absent"))), root.join("fresh"), "nothing to move");
        std::fs::remove_dir_all(root).unwrap();
    }
}
