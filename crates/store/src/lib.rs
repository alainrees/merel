//! The offline cache: for each account, the folders, message list, opened messages and
//! synced items it showed last time, so the window fills at once on the next start and keeps
//! working without a connection. The server stays the truth: whatever it reports replaces the
//! cache, and nothing here is ever written back to it.
//!
//! One file per account, sealed with ChaCha20-Poly1305 under a random key that lives in the
//! system keychain: a copied file is unreadable without the keychain, and a changed one is
//! rejected rather than shown. The account id is bound in as associated data, so a file
//! renamed to another account does not open either.

use std::path::{Path, PathBuf};

use aws_lc_rs::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use base64::Engine as _;
use mailcraft_model::{Appointment, Calendar, Contact, Folder, MessageHeader, StoredDraft, Task};
use mailcraft_secrets::{Secret, SecretStore};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// Keychain entry holding the cache key.
pub const KEY_NAME: &str = "cache-key";
/// Most opened messages kept per account.
pub const MAX_BODIES: usize = 300;
/// Most bytes of opened messages kept per account.
pub const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;
/// A single message larger than this is not cached.
pub const MAX_ONE_BODY: usize = 4 * 1024 * 1024;

const MAGIC: &[u8; 4] = b"MCC1";
const KEY_LEN: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("de sleutelhanger is niet beschikbaar: {0}")]
    Keychain(String),
    #[error("de cache kan niet worden geschreven: {0}")]
    Io(#[from] std::io::Error),
    #[error("de cache kan niet worden versleuteld")]
    Crypto,
}

/// A message as it came from the server, by the id the session gives it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedBody {
    pub id: String,
    #[serde(with = "raw_bytes")]
    pub raw: Vec<u8>,
}

/// Everything cached for one account.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountCache {
    pub account_id: String,
    /// Seconds since the Unix epoch when the server last answered.
    pub saved_at: i64,
    pub folders: Vec<Folder>,
    pub messages: Vec<MessageHeader>,
    /// Oldest first; the newest opened message is last.
    pub bodies: Vec<CachedBody>,
    /// Changes that found no connection, oldest first, as the engine serialises its jobs.
    #[serde(default)]
    pub pending: Vec<serde_json::Value>,
    /// Whether the account's store keeps appointments, contacts and tasks.
    pub syncs_items: bool,
    pub calendars: Vec<Calendar>,
    pub appointments: Vec<Appointment>,
    pub contacts: Vec<Contact>,
    pub tasks: Vec<Task>,
    /// Messages being written that were put aside.
    #[serde(default)]
    pub drafts: Vec<StoredDraft>,
}

impl AccountCache {
    /// Remembers a message; the oldest ones leave when the bounds are reached.
    pub fn put_body(&mut self, id: &str, raw: &[u8]) {
        if raw.len() > MAX_ONE_BODY {
            return;
        }
        self.bodies.retain(|b| b.id != id);
        self.bodies.push(CachedBody { id: id.to_string(), raw: raw.to_vec() });
        while self.bodies.len() > MAX_BODIES || self.bodies.iter().map(|b| b.raw.len()).sum::<usize>() > MAX_BODY_BYTES {
            self.bodies.remove(0);
        }
    }

    pub fn body(&self, id: &str) -> Option<&[u8]> {
        self.bodies.iter().find(|b| b.id == id).map(|b| b.raw.as_slice())
    }

    /// Drops the messages that are no longer listed in any folder.
    pub fn prune_bodies(&mut self) {
        self.bodies.retain(|b| self.messages.iter().any(|m| m.id == b.id));
    }
}

/// The cache directory and its key.
pub struct Store {
    dir: PathBuf,
    key: LessSafeKey,
}

impl Store {
    /// Opens the cache in `dir`, fetching the key from `secrets` or minting one on first use.
    pub fn open(dir: PathBuf, secrets: &dyn SecretStore) -> Result<Store, StoreError> {
        let key = match secrets.get(KEY_NAME).map_err(|e| StoreError::Keychain(e.to_string()))? {
            Some(s) => decode_key(s.expose()).ok_or_else(|| StoreError::Keychain("de cachesleutel in de sleutelhanger is ongeldig".into()))?,
            None => {
                let mut key = Zeroizing::new([0u8; KEY_LEN]);
                getrandom::fill(key.as_mut_slice()).map_err(|_| StoreError::Crypto)?;
                let encoded = Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(key.as_slice()));
                secrets.set(KEY_NAME, &Secret::new(encoded.as_str())).map_err(|e| StoreError::Keychain(e.to_string()))?;
                key
            }
        };
        let key = LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, key.as_slice()).map_err(|_| StoreError::Crypto)?);
        Ok(Store { dir, key })
    }

    fn path(&self, account_id: &str) -> PathBuf {
        // The file name is derived from the id without trusting it as a path.
        let name: String = account_id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
        self.dir.join(format!("{name}.cache"))
    }

    /// What was cached for `account_id`; `None` when there is nothing, or the file does not
    /// open (which is logged, never shown).
    pub fn load(&self, account_id: &str) -> Option<AccountCache> {
        let path = self.path(account_id);
        let bytes = std::fs::read(&path).ok()?;
        match self.open_bytes(account_id, bytes) {
            Some(cache) if cache.account_id == account_id => Some(cache),
            _ => {
                log::warn!("{}: cache unreadable, ignored", path.display());
                None
            }
        }
    }

    fn open_bytes(&self, account_id: &str, mut bytes: Vec<u8>) -> Option<AccountCache> {
        if bytes.len() < MAGIC.len() + NONCE_LEN || !bytes.starts_with(MAGIC) {
            return None;
        }
        let nonce = Nonce::try_assume_unique_for_key(bytes.get(MAGIC.len()..MAGIC.len() + NONCE_LEN)?).ok()?;
        let body = bytes.get_mut(MAGIC.len() + NONCE_LEN..)?;
        let plain = self.key.open_in_place(nonce, Aad::from(account_id.as_bytes()), body).ok()?;
        serde_json::from_slice(plain).ok()
    }

    /// Writes the cache of `cache.account_id` through a temporary file, readable by the
    /// owner only.
    pub fn save(&self, cache: &AccountCache) -> Result<(), StoreError> {
        std::fs::create_dir_all(&self.dir)?;
        let mut out = MAGIC.to_vec();
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|_| StoreError::Crypto)?;
        out.extend_from_slice(&nonce);
        let mut body = serde_json::to_vec(cache).map_err(|_| StoreError::Crypto)?;
        let nonce = Nonce::try_assume_unique_for_key(&nonce).map_err(|_| StoreError::Crypto)?;
        self.key.seal_in_place_append_tag(nonce, Aad::from(cache.account_id.as_bytes()), &mut body).map_err(|_| StoreError::Crypto)?;
        out.extend_from_slice(&body);
        let path = self.path(&cache.account_id);
        write_private(&path, &out)
    }

    /// Forgets an account.
    pub fn remove(&self, account_id: &str) {
        let _ = std::fs::remove_file(self.path(account_id));
    }
}

fn decode_key(encoded: &str) -> Option<Zeroizing<[u8; KEY_LEN]>> {
    let bytes = Zeroizing::new(base64::engine::general_purpose::STANDARD.decode(encoded.trim()).ok()?);
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    if bytes.len() != KEY_LEN {
        return None;
    }
    key.copy_from_slice(&bytes);
    Some(key)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let tmp = path.with_extension("cache.tmp");
    std::fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(tmp, path)?;
    Ok(())
}

/// Raw messages as base64 strings, which JSON carries far more compactly than byte arrays.
mod raw_bytes {
    use base64::Engine as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        base64::engine::general_purpose::STANDARD.encode(bytes).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        base64::engine::general_purpose::STANDARD.decode(text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use mailcraft_model::{Address, Flags, FolderKind};
    use mailcraft_secrets::MemoryStore;

    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mailcraft-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn cache() -> AccountCache {
        let folder = Folder {
            id: "a1/INBOX".into(),
            account_id: "a1".into(),
            name: "Postvak IN".into(),
            path: "INBOX".into(),
            kind: FolderKind::Inbox,
            unread: 1,
        };
        let message = MessageHeader {
            id: "a1/INBOX#7".into(),
            folder_id: folder.id.clone(),
            remote_id: "7".into(),
            from: Address::new(Some("Sanne"), "sanne@example.org").unwrap(),
            to: vec![],
            subject: "Hoi".into(),
            date: 1_000,
            preview: "Hallo".into(),
            flags: Flags::default(),
            has_attachments: false,
            message_id: None,
        };
        let mut c = AccountCache { account_id: "a1".into(), saved_at: 5, folders: vec![folder], messages: vec![message], ..AccountCache::default() };
        c.put_body("a1/INBOX#7", b"From: sanne@example.org\r\n\r\nHallo\r\n");
        c
    }

    #[test]
    fn round_trips_sealed_and_rejects_tampering() {
        let secrets = MemoryStore::default();
        let d = dir("roundtrip");
        let store = Store::open(d.clone(), &secrets).unwrap();
        assert!(store.load("a1").is_none());
        let c = cache();
        store.save(&c).unwrap();
        assert_eq!(store.load("a1"), Some(c.clone()));
        let path = store.path("a1");
        let bytes = std::fs::read(&path).unwrap();
        assert!(!bytes.windows(5).any(|w| w == b"Hallo"), "nothing readable on disk");
        assert!(!bytes.windows(4).any(|w| w == b"Sann"));

        // A flipped byte, a file under another account's name, and another key all fail closed.
        let mut bad = bytes.clone();
        if let Some(b) = bad.last_mut() {
            *b ^= 1;
        }
        std::fs::write(&path, &bad).unwrap();
        assert!(store.load("a1").is_none());
        std::fs::write(store.path("a2"), &bytes).unwrap();
        assert!(store.load("a2").is_none());
        std::fs::write(&path, &bytes).unwrap();
        let other = Store::open(d.clone(), &MemoryStore::default()).unwrap();
        assert!(other.load("a1").is_none());
        // The same keychain opens it again, and the key stays where it was.
        let again = Store::open(d.clone(), &secrets).unwrap();
        assert_eq!(again.load("a1"), Some(c));
        assert!(secrets.get(KEY_NAME).unwrap().is_some());
        store.remove("a1");
        assert!(store.load("a1").is_none());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn bodies_are_bounded_and_pruned() {
        let mut c = cache();
        for i in 0..MAX_BODIES + 5 {
            c.put_body(&format!("a1/INBOX#{i}"), b"x");
        }
        assert_eq!(c.bodies.len(), MAX_BODIES);
        assert!(c.body("a1/INBOX#0").is_none() && c.body("a1/INBOX#7").is_some());
        c.put_body("big", &vec![0u8; MAX_ONE_BODY + 1]);
        assert!(c.body("big").is_none());
        c.prune_bodies();
        assert_eq!(c.bodies.len(), 1, "only the listed message stays");
        assert_eq!(c.body("a1/INBOX#7"), Some(&b"x"[..]));
    }

    #[test]
    fn a_bad_key_in_the_keychain_is_reported() {
        let secrets = MemoryStore::default();
        secrets.set(KEY_NAME, &Secret::new("not base64!")).unwrap();
        assert!(matches!(Store::open(dir("badkey"), &secrets), Err(StoreError::Keychain(_))));
    }
}
