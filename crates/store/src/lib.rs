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
use merel_model::{Appointment, Calendar, Contact, Folder, MessageHeader, StoredDraft, Task};
use merel_secrets::{Secret, SecretStore};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

/// Keychain entry holding the cache key.
pub const KEY_NAME: &str = "cache-key";
/// Most opened messages kept per account.
pub const MAX_BODIES: usize = 300;
/// Most bytes of opened messages kept per account.
pub const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;
/// A single message larger than this is not cached.
pub const MAX_ONE_BODY: usize = 4 * 1024 * 1024;

const MAGIC: &[u8; 4] = b"MCC1";
const IDENTITY_MAGIC: &[u8; 4] = b"MCI1";
const KEY_LEN: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{}", merel_i18n::tr_args("store-keychain", &[("detail", _0.clone().into())]))]
    Keychain(String),
    #[error("{}", merel_i18n::tr_args("store-io", &[("detail", _0.to_string().into())]))]
    Io(#[from] std::io::Error),
    #[error("{}", merel_i18n::tr("store-crypto"))]
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
            Some(s) => decode_key(s.expose()).ok_or_else(|| StoreError::Keychain(merel_i18n::tr("store-bad-key")))?,
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

    fn open_bytes(&self, account_id: &str, bytes: Vec<u8>) -> Option<AccountCache> {
        let plain = self.unseal(MAGIC, account_id.as_bytes(), bytes)?;
        serde_json::from_slice(&plain).ok()
    }

    /// The plain text of a file written by [`Store::seal`] with the same tag and associated
    /// data; `None` for anything else.
    fn unseal(&self, magic: &[u8; 4], aad: &[u8], mut bytes: Vec<u8>) -> Option<Zeroizing<Vec<u8>>> {
        if bytes.len() < magic.len() + NONCE_LEN || !bytes.starts_with(magic) {
            return None;
        }
        let nonce = Nonce::try_assume_unique_for_key(bytes.get(magic.len()..magic.len() + NONCE_LEN)?).ok()?;
        let body = bytes.get_mut(magic.len() + NONCE_LEN..)?;
        let plain = Zeroizing::new(self.key.open_in_place(nonce, Aad::from(aad), body).ok()?.to_vec());
        bytes.zeroize();
        Some(plain)
    }

    fn seal(&self, magic: &[u8; 4], aad: &[u8], mut body: Vec<u8>) -> Result<Vec<u8>, StoreError> {
        let mut out = magic.to_vec();
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|_| StoreError::Crypto)?;
        out.extend_from_slice(&nonce);
        let nonce = Nonce::try_assume_unique_for_key(&nonce).map_err(|_| StoreError::Crypto)?;
        self.key.seal_in_place_append_tag(nonce, Aad::from(aad), &mut body).map_err(|_| StoreError::Crypto)?;
        out.extend_from_slice(&body);
        Ok(out)
    }

    /// Writes the cache of `cache.account_id` through a temporary file, readable by the
    /// owner only.
    pub fn save(&self, cache: &AccountCache) -> Result<(), StoreError> {
        std::fs::create_dir_all(&self.dir)?;
        let body = serde_json::to_vec(cache).map_err(|_| StoreError::Crypto)?;
        let out = self.seal(MAGIC, cache.account_id.as_bytes(), body)?;
        let path = self.path(&cache.account_id);
        write_private(&path, &out)
    }

    /// Forgets an account, its S/MIME certificate and OpenPGP key included.
    pub fn remove(&self, account_id: &str) {
        let _ = std::fs::remove_file(self.path(account_id));
        self.remove_identity(account_id, KeyKind::Certificate);
        self.remove_identity(account_id, KeyKind::OpenPgp);
    }

    fn identity_path(&self, account_id: &str, kind: KeyKind) -> PathBuf {
        self.path(account_id).with_extension(kind.extension())
    }

    /// Keeps a private key of `account_id` (its S/MIME certificate, or its OpenPGP key),
    /// sealed like the cache and bound to the account and the kind, in a file of its own: the
    /// cache is rewritten all the time, the key only when it changes.
    pub fn save_identity(&self, account_id: &str, kind: KeyKind, plain: &[u8]) -> Result<(), StoreError> {
        std::fs::create_dir_all(&self.dir)?;
        let out = self.seal(IDENTITY_MAGIC, &identity_aad(account_id, kind), plain.to_vec())?;
        write_private(&self.identity_path(account_id, kind), &out)
    }

    /// What [`Store::save_identity`] kept for `account_id`; `None` when there is nothing, or
    /// the file does not open.
    pub fn load_identity(&self, account_id: &str, kind: KeyKind) -> Option<Zeroizing<Vec<u8>>> {
        let path = self.identity_path(account_id, kind);
        let bytes = std::fs::read(&path).ok()?;
        let plain = self.unseal(IDENTITY_MAGIC, &identity_aad(account_id, kind), bytes);
        if plain.is_none() {
            log::warn!("{}: key unreadable, ignored", path.display());
        }
        plain
    }

    pub fn remove_identity(&self, account_id: &str, kind: KeyKind) {
        let _ = std::fs::remove_file(self.identity_path(account_id, kind));
    }
}

/// Which private key of an account a sealed file holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyKind {
    /// The S/MIME certificate with its key.
    Certificate,
    /// The OpenPGP key.
    OpenPgp,
}

impl KeyKind {
    fn extension(self) -> &'static str {
        match self {
            KeyKind::Certificate => "identity",
            KeyKind::OpenPgp => "pgpkey",
        }
    }
}

/// Associated data of a key file: not the bare account id, so a cache file renamed to a key
/// file, or one kind of key file to the other, does not open.
fn identity_aad(account_id: &str, kind: KeyKind) -> Vec<u8> {
    match kind {
        KeyKind::Certificate => format!("identity\0{account_id}").into_bytes(),
        KeyKind::OpenPgp => format!("openpgp\0{account_id}").into_bytes(),
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
    use merel_model::{Address, Flags, FolderKind};
    use merel_secrets::MemoryStore;

    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("merel-store-{name}-{}", std::process::id()));
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
            parent: None,
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
            categories: Vec::new(),
            importance: Default::default(),
            chat: None,
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
    fn an_identity_is_sealed_bound_to_its_account_and_removed_with_it() {
        let secrets = MemoryStore::default();
        let d = dir("identity");
        let store = Store::open(d.clone(), &secrets).unwrap();
        assert!(store.load_identity("a1", KeyKind::Certificate).is_none());
        store.save_identity("a1", KeyKind::Certificate, b"PRIVATE KEY").unwrap();
        assert_eq!(store.load_identity("a1", KeyKind::Certificate).as_deref().map(Vec::as_slice), Some(&b"PRIVATE KEY"[..]));
        let bytes = std::fs::read(store.identity_path("a1", KeyKind::Certificate)).unwrap();
        assert!(!bytes.windows(7).any(|w| w == b"PRIVATE"), "nothing readable on disk");
        // Under another account's name, or as a cache file, it does not open.
        std::fs::write(store.identity_path("a2", KeyKind::Certificate), &bytes).unwrap();
        assert!(store.load_identity("a2", KeyKind::Certificate).is_none());
        std::fs::write(store.path("a1"), &bytes).unwrap();
        assert!(store.load("a1").is_none());
        store.save(&cache()).unwrap();
        std::fs::write(store.identity_path("a1", KeyKind::Certificate), std::fs::read(store.path("a1")).unwrap()).unwrap();
        assert!(store.load_identity("a1", KeyKind::Certificate).is_none());
        store.save_identity("a1", KeyKind::Certificate, b"PRIVATE KEY").unwrap();
        // The OpenPGP key has a file of its own, and one kind does not open as the other.
        store.save_identity("a1", KeyKind::OpenPgp, b"PGP KEY").unwrap();
        assert_eq!(store.load_identity("a1", KeyKind::OpenPgp).as_deref().map(Vec::as_slice), Some(&b"PGP KEY"[..]));
        std::fs::copy(store.identity_path("a1", KeyKind::OpenPgp), store.identity_path("a1", KeyKind::Certificate)).unwrap();
        assert!(store.load_identity("a1", KeyKind::Certificate).is_none());
        store.remove("a1");
        assert!(store.load_identity("a1", KeyKind::OpenPgp).is_none() && store.load("a1").is_none());
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
