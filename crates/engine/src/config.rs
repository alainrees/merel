//! The accounts file: JSON without secrets, readable by the owner only.

use std::path::{Path, PathBuf};

use mailcraft_model::Account;

/// Where the accounts file lives for this user, if the platform has a config directory.
pub fn default_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("nl", "ArtCraft", "Mailcraft").map(|d| d.config_dir().join("accounts.json"))
}

pub fn load(path: &Path) -> Vec<Account> {
    let Ok(bytes) = std::fs::read(path) else { return Vec::new() };
    match serde_json::from_slice::<Vec<Account>>(&bytes) {
        Ok(a) => a,
        Err(e) => {
            log::warn!("accounts file unreadable: {e}");
            Vec::new()
        }
    }
}

pub fn save(path: &Path, accounts: &[Account]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_vec_pretty(accounts).map_err(std::io::Error::other)?;
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
        };
        save(&path, std::slice::from_ref(&a)).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("password"));
        assert_eq!(load(&path), vec![a]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
