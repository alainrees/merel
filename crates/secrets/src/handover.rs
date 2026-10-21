//! Handing the secrets to the next version of Merel during an update, so macOS does not ask for
//! the keychain password afterwards.
//!
//! macOS files each keychain item with the build that made it. Merel is not signed by a paid
//! Apple developer account, so every new build counts as a stranger and has to ask, and
//! "Sta altijd toe" only ever covers the build that asked. A build that makes an item itself
//! is never asked about it.
//!
//! So at "Opnieuw starten" the old version, which may read the items, [`export`]s them: it
//! seals them into a file readable by this user only, with a fresh key that never touches the
//! disk, and removes the items. The new version gets that key in its environment and
//! [`import`]s the secrets before anything reads the keychain, making each item again as its
//! own. The file is removed as soon as it has been read, and is useless without the key.
//! Should anything go wrong the items are put back, or at worst the new version asks for the
//! passwords again, as before.

use std::io::Write;
use std::path::Path;

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use base64::Engine as _;
use zeroize::{Zeroize, Zeroizing};

use crate::{Secret, SecretError, SecretStore};

/// The environment variable that carries the key to the new version.
pub const ENV: &str = "MEREL_HANDOVER";

const KEY_LEN: usize = 32;
/// Binds the file to its purpose and format.
const AAD: &[u8] = b"merel-handover-1";
/// More than a person ever has; a bigger file is not ours.
const MAX_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum HandoverError {
    #[error("the handover file could not be written or read: {0}")]
    Io(#[from] std::io::Error),
    #[error("the handover could not be sealed or opened")]
    Crypto,
    #[error(transparent)]
    Store(#[from] SecretError),
}

/// The key to an exported handover, base64; hidden from `Debug` and wiped when dropped.
pub struct HandoverKey(Zeroizing<String>);

impl HandoverKey {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for HandoverKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HandoverKey(…)")
    }
}

/// Seals the secrets named `names` that exist into `path`, then removes them from `store`.
/// Returns `None` when there was nothing to hand over. Should a removal fail, every secret is
/// put back and the file removed, so the store is as it was.
pub fn export(store: &dyn SecretStore, names: &[String], path: &Path) -> Result<Option<HandoverKey>, HandoverError> {
    let mut found = Vec::new();
    for name in names {
        if found.iter().any(|(n, _): &(String, Secret)| n == name) {
            continue;
        }
        if let Some(secret) = store.get(name)? {
            found.push((name.clone(), secret));
        }
    }
    if found.is_empty() {
        return Ok(None);
    }
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    getrandom::fill(key.as_mut_slice()).map_err(|_| HandoverError::Crypto)?;
    let sealed = seal(key.as_slice(), &encode(&found))?;
    write_private(path, &sealed)?;
    for (i, (name, _)) in found.iter().enumerate() {
        if let Err(e) = store.delete(name) {
            for (name, secret) in &found[..i] {
                if let Err(e) = store.set(name, secret) {
                    log::warn!("handover: a secret could not be put back: {e}");
                }
            }
            let _ = std::fs::remove_file(path);
            return Err(e.into());
        }
    }
    Ok(Some(HandoverKey(Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(key.as_slice())))))
}

/// Puts the secrets sealed in `path` into `store` and removes the file, also when it does not
/// open. Returns how many secrets came over; 0 when there is no file.
pub fn import(store: &dyn SecretStore, key: &str, path: &Path) -> Result<usize, HandoverError> {
    let read = read_limited(path);
    let _ = std::fs::remove_file(path);
    let Some(sealed) = read? else { return Ok(0) };
    let key = Zeroizing::new(base64::engine::general_purpose::STANDARD.decode(key.trim()).map_err(|_| HandoverError::Crypto)?);
    let plain = open(&key, sealed)?;
    let secrets = decode(&plain).ok_or(HandoverError::Crypto)?;
    let mut failed = None;
    for (name, secret) in &secrets {
        if let Err(e) = store.set(name, secret) {
            failed = Some(e);
        }
    }
    match failed {
        Some(e) => Err(e.into()),
        None => Ok(secrets.len()),
    }
}

fn read_limited(path: &Path) -> Result<Option<Vec<u8>>, std::io::Error> {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
        Ok(m) if m.len() > MAX_FILE_BYTES => return Err(std::io::Error::other("too big")),
        Ok(_) => {}
    }
    std::fs::read(path).map(Some)
}

/// Writes `bytes` to a new file only this user may read; one left behind is replaced.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _ = std::fs::remove_file(path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn seal(key: &[u8], plain: &[u8]) -> Result<Vec<u8>, HandoverError> {
    let key = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).map_err(|_| HandoverError::Crypto)?);
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce).map_err(|_| HandoverError::Crypto)?;
    let mut sealed = Zeroizing::new(plain.to_vec());
    key.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(AAD), &mut *sealed).map_err(|_| HandoverError::Crypto)?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&sealed);
    Ok(out)
}

fn open(key: &[u8], mut sealed: Vec<u8>) -> Result<Zeroizing<Vec<u8>>, HandoverError> {
    if sealed.len() < NONCE_LEN {
        return Err(HandoverError::Crypto);
    }
    let key = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).map_err(|_| HandoverError::Crypto)?);
    let mut body = Zeroizing::new(sealed.split_off(NONCE_LEN));
    let nonce = Nonce::try_assume_unique_for_key(&sealed).map_err(|_| HandoverError::Crypto)?;
    let len = key.open_in_place(nonce, Aad::from(AAD), &mut body).map_err(|_| HandoverError::Crypto)?.len();
    body.truncate(len);
    Ok(body)
}

/// Name and secret after each other, each as a 4-byte length and its UTF-8.
fn encode(secrets: &[(String, Secret)]) -> Zeroizing<Vec<u8>> {
    let mut out = Zeroizing::new(Vec::new());
    for (name, secret) in secrets {
        for part in [name.as_str(), secret.expose()] {
            out.extend_from_slice(&(part.len() as u32).to_be_bytes());
            out.extend_from_slice(part.as_bytes());
        }
    }
    out
}

fn decode(mut bytes: &[u8]) -> Option<Vec<(String, Secret)>> {
    fn part(bytes: &mut &[u8]) -> Option<String> {
        let len = u32::from_be_bytes(bytes.get(..4)?.try_into().ok()?) as usize;
        let text = std::str::from_utf8(bytes.get(4..4 + len)?).ok()?.to_string();
        *bytes = &bytes[4 + len..];
        Some(text)
    }
    let mut out = Vec::new();
    while !bytes.is_empty() {
        let name = part(&mut bytes)?;
        let mut value = part(&mut bytes)?;
        out.push((name, Secret::new(value.as_str())));
        value.zeroize();
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryStore;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn secrets_move_from_one_store_to_the_next() {
        let dir = std::env::temp_dir().join(format!("merel-handover-{}", std::process::id()));
        let path = dir.join("move");
        let old = MemoryStore::default();
        old.set("cache-key", &Secret::new("k")).unwrap();
        old.set("acct", &Secret::new("pässwörd")).unwrap();
        let key = export(&old, &names(&["cache-key", "acct", "acct#client_secret", "acct"]), &path).unwrap().unwrap();
        assert!(old.get("acct").unwrap().is_none() && old.get("cache-key").unwrap().is_none(), "the old version's items are gone");
        let bytes = std::fs::read(&path).unwrap();
        assert!(!bytes.windows(4).any(|w| w == b"p\xc3\xa4s"), "the file holds no secret in the clear");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }

        let new = MemoryStore::default();
        assert_eq!(import(&new, key.expose(), &path).unwrap(), 2);
        assert_eq!(new.get("acct").unwrap().unwrap().expose(), "pässwörd");
        assert_eq!(new.get("cache-key").unwrap().unwrap().expose(), "k");
        assert!(new.get("acct#client_secret").unwrap().is_none());
        assert!(!path.exists(), "the file is removed once read");
        assert_eq!(import(&new, key.expose(), &path).unwrap(), 0, "no file, nothing to do");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_wrong_key_opens_nothing_and_still_removes_the_file() {
        let dir = std::env::temp_dir().join(format!("merel-handover-wrong-{}", std::process::id()));
        let path = dir.join("wrong");
        let old = MemoryStore::default();
        old.set("acct", &Secret::new("pw")).unwrap();
        export(&old, &names(&["acct"]), &path).unwrap().unwrap();
        let other = base64::engine::general_purpose::STANDARD.encode([7u8; KEY_LEN]);
        let new = MemoryStore::default();
        assert!(matches!(import(&new, &other, &path), Err(HandoverError::Crypto)));
        assert!(new.get("acct").unwrap().is_none());
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_to_hand_over_writes_no_file() {
        let path = std::env::temp_dir().join(format!("merel-handover-none-{}", std::process::id()));
        assert!(export(&MemoryStore::default(), &names(&["acct"]), &path).unwrap().is_none());
        assert!(!path.exists());
    }

    #[test]
    fn a_failed_removal_puts_everything_back() {
        struct Sticky(MemoryStore);
        impl SecretStore for Sticky {
            fn set(&self, key: &str, secret: &Secret) -> Result<(), SecretError> {
                self.0.set(key, secret)
            }
            fn get(&self, key: &str) -> Result<Option<Secret>, SecretError> {
                self.0.get(key)
            }
            fn delete(&self, key: &str) -> Result<(), SecretError> {
                if key == "b" { Err(SecretError::Store("no".into())) } else { self.0.delete(key) }
            }
        }
        let path = std::env::temp_dir().join(format!("merel-handover-sticky-{}", std::process::id()));
        let store = Sticky(MemoryStore::default());
        store.set("a", &Secret::new("1")).unwrap();
        store.set("b", &Secret::new("2")).unwrap();
        assert!(export(&store, &names(&["a", "b"]), &path).is_err());
        assert_eq!(store.get("a").unwrap().unwrap().expose(), "1");
        assert_eq!(store.get("b").unwrap().unwrap().expose(), "2");
        assert!(!path.exists());
    }

    #[test]
    fn the_key_never_prints() {
        assert_eq!(format!("{:?}", HandoverKey(Zeroizing::new("abc".into()))), "HandoverKey(…)");
    }
}
