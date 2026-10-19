//! The configuration files: JSON without secrets, readable by the owner only. `accounts.json`
//! holds the accounts; `calendar.json`, `contacts.json`, `tasks.json` and `rules.json` next to
//! it the local calendars, appointments, contacts, tasks and mail rules.

use std::path::{Path, PathBuf};

use mailcraft_model::Account;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Where the accounts file lives for this user, if the platform has a config directory.
pub fn default_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("nl", "ArtCraft", "Mailcraft").map(|d| d.config_dir().join("accounts.json"))
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

/// The mail rules file that goes with an accounts file.
pub fn rules_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("rules.json")
}

/// The blocked and safe senders file that goes with an accounts file.
pub fn junk_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("junk.json")
}

/// The ignored conversations, next to the accounts file.
pub fn ignored_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("ignored.json")
}

/// The follow-up dates of flagged messages, next to the accounts file.
pub fn follow_ups_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("follow_ups.json")
}

/// The quick steps file that goes with an accounts file.
pub fn quick_steps_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("quick_steps.json")
}

/// The contact groups (distribution lists) file that goes with an accounts file.
pub fn groups_path(accounts_path: &Path) -> PathBuf {
    accounts_path.with_file_name("contact_groups.json")
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
    use mailcraft_model::{Address, Protocol, Server, Tls};

    #[test]
    fn round_trips_without_secrets() {
        let dir = std::env::temp_dir().join(format!("mailcraft-test-{}", std::process::id()));
        let path = dir.join("accounts.json");
        let a = Account {
            id: "a1".into(),
            name: "Test".into(),
            address: Address::new(None, "a@b.nl").unwrap(),
            username: "a@b.nl".into(),
            protocol: Protocol::ImapSmtp,
            auth: mailcraft_model::Auth::Password,
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
}
