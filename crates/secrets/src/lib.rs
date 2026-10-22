//! Secrets: passwords and tokens, kept in the system credential store and never in a file.
//!
//! macOS Keychain, Windows Credential Manager and the Secret Service on Linux, through the
//! `keyring` crate. A [`MemoryStore`] stands in for tests and for first-run previews. Secrets
//! are wiped from memory when dropped and never appear in `Debug` output or logs.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use zeroize::{Zeroize, ZeroizeOnDrop};

pub mod handover;

/// The keychain service name every Merel entry is filed under. It keeps the name from the time
/// the app was called Mailcraft, so passwords saved then are still found.
pub const SERVICE: &str = "nl.artcraft.mailcraft";

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("the system credential store is not available: {0}")]
    Unavailable(String),
    #[error("the credential store refused the request: {0}")]
    Store(String),
}

/// A secret string that is zeroed when dropped and hidden from `Debug`.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Self {
        Secret(s.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(…)")
    }
}

/// Where secrets live. `key` is the account id.
pub trait SecretStore: Send + Sync {
    fn set(&self, key: &str, secret: &Secret) -> Result<(), SecretError>;
    fn get(&self, key: &str) -> Result<Option<Secret>, SecretError>;
    fn delete(&self, key: &str) -> Result<(), SecretError>;
}

/// The system credential store. Not on phones, where the app passes in the keychain or the
/// Android Keystore itself.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
#[derive(Default)]
pub struct Keychain;

#[cfg(not(any(target_os = "ios", target_os = "android")))]
impl Keychain {
    fn entry(key: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(SERVICE, key).map_err(|e| SecretError::Unavailable(e.to_string()))
    }
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
impl SecretStore for Keychain {
    fn set(&self, key: &str, secret: &Secret) -> Result<(), SecretError> {
        Self::entry(key)?.set_password(secret.expose()).map_err(|e| SecretError::Store(e.to_string()))
    }

    fn get(&self, key: &str) -> Result<Option<Secret>, SecretError> {
        match Self::entry(key)?.get_password() {
            Ok(mut p) => {
                let s = Secret::new(p.as_str());
                p.zeroize();
                Ok(Some(s))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(SecretError::Store(e.to_string())),
        }
    }

    fn delete(&self, key: &str) -> Result<(), SecretError> {
        match Self::entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(SecretError::Store(e.to_string())),
        }
    }
}

/// An in-memory store for tests. Nothing is persisted.
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, Secret>>);

impl SecretStore for MemoryStore {
    fn set(&self, key: &str, secret: &Secret) -> Result<(), SecretError> {
        self.0.lock().map_err(|_| SecretError::Store("poisoned".into()))?.insert(key.to_string(), secret.clone());
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<Secret>, SecretError> {
        Ok(self.0.lock().map_err(|_| SecretError::Store("poisoned".into()))?.get(key).cloned())
    }

    fn delete(&self, key: &str) -> Result<(), SecretError> {
        self.0.lock().map_err(|_| SecretError::Store("poisoned".into()))?.remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_never_prints() {
        assert_eq!(format!("{:?}", Secret::new("hunter2")), "Secret(…)");
    }

    #[test]
    fn memory_store_round_trips() {
        let s = MemoryStore::default();
        assert!(s.get("a").unwrap().is_none());
        s.set("a", &Secret::new("x")).unwrap();
        assert_eq!(s.get("a").unwrap().unwrap().expose(), "x");
        s.delete("a").unwrap();
        assert!(s.get("a").unwrap().is_none());
    }
}
