//! S/MIME certificates: one per account, imported from a PKCS#12 file and kept sealed next to
//! the offline cache. Outgoing mail of an account with a certificate is signed, unless the
//! message says otherwise.
//!
//! The worker threads share the certificates with the engine, so a certificate imported or
//! removed takes effect with the next message, without reconnecting.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use mailcraft_model::Account;
use mailcraft_smime::{CertInfo, Identity};

use crate::{Engine, SAMPLE_ACCOUNT};

/// The certificate of each account that has one, by account id.
pub type Identities = Arc<RwLock<HashMap<String, Arc<Identity>>>>;

/// Most bytes a PKCS#12 file may have; real ones are a few kilobytes.
pub const MAX_PKCS12: usize = 256 * 1024;

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// `raw` signed with the certificate of `account` when `sign` asks for it, or when it is
/// `None` and the account has a certificate; `raw` itself otherwise. A message that should be
/// signed and cannot be is not sent unsigned.
pub fn sign_if_asked(identities: &Identities, account: &Account, sign: Option<bool>, raw: Vec<u8>) -> Result<Vec<u8>, String> {
    let identity = identities.read().ok().and_then(|map| map.get(&account.id).cloned());
    match (sign, identity) {
        (Some(false), _) | (None, None) => Ok(raw),
        (Some(true), None) => {
            Err("Dit account heeft geen certificaat om mee te ondertekenen. Importeer er een onder Bestand, of zet Ondertekenen uit.".into())
        }
        (_, Some(identity)) => {
            identity.check_usable(now()).map_err(|e| format!("{e} Importeer een nieuw certificaat onder Bestand, of zet Ondertekenen uit."))?;
            mailcraft_smime::sign_message(&identity, &raw, now()).map_err(|e| e.to_string())
        }
    }
}

impl Engine {
    /// Reads the certificate each account kept from the sealed store.
    pub(crate) fn load_identities(&mut self) {
        let Ok(mut map) = self.identities.write() else { return };
        for account in &self.session.accounts {
            if let Some(plain) = self.cache.load_identity(&account.id) {
                match Identity::from_stored(&plain) {
                    Ok(identity) => {
                        map.insert(account.id.clone(), Arc::new(identity));
                    }
                    Err(e) => log::warn!("{}: certificate not loaded: {e}", account.id),
                }
            }
        }
    }

    /// The certificate `account_id` signs with, if any.
    pub fn certificate(&self, account_id: &str) -> Option<CertInfo> {
        self.identities.read().ok()?.get(account_id).map(|i| i.info().clone())
    }

    /// Imports the PKCS#12 file `bytes` (.p12, .pfx) as the certificate of `account_id`,
    /// replacing the one it had. The certificate must be issued for the account's address.
    pub fn import_certificate(&mut self, account_id: &str, bytes: &[u8], password: &str) -> Result<CertInfo, String> {
        let account = self
            .session
            .accounts
            .iter()
            .find(|a| a.id == account_id && a.id != SAMPLE_ACCOUNT)
            .ok_or_else(|| "Dit account bestaat niet meer.".to_string())?;
        if bytes.len() > MAX_PKCS12 {
            return Err("Dit bestand is te groot voor een certificaat.".into());
        }
        let identity = Identity::from_pkcs12(bytes, password, now()).map_err(|e| e.to_string())?;
        let email = account.address.email().to_string();
        if !identity.info().covers(&email) {
            let issued = identity.info().emails.join(", ");
            let issued = if issued.is_empty() { "geen e-mailadres".to_string() } else { issued };
            return Err(format!("Dit certificaat is uitgegeven voor {issued}, niet voor {email}."));
        }
        // An account added in this run may not have its cache directory yet.
        if !self.cache.has_store() {
            self.cache = crate::cache::Cache::open(self.config_path.as_deref(), &*self.secrets);
        }
        self.cache.save_identity(account_id, &identity.to_stored()).map_err(|e| format!("Het certificaat kan niet worden bewaard: {e}"))?;
        let info = identity.info().clone();
        self.identities
            .write()
            .map_err(|_| "Het certificaat kan niet worden bewaard.".to_string())?
            .insert(account_id.to_string(), Arc::new(identity));
        Ok(info)
    }

    /// Forgets the certificate of `account_id`, on disk too.
    pub fn remove_certificate(&mut self, account_id: &str) {
        self.cache.remove_identity(account_id);
        if let Ok(mut map) = self.identities.write() {
            map.remove(account_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use mailcraft_model::{Address, Auth, Protocol, Server, Tls};
    use mailcraft_secrets::Secret;

    use super::*;

    const P12: &[u8] = include_bytes!("../../smime/testdata/alain.p12");

    fn account(email: &str) -> Account {
        Account {
            id: "a1".into(),
            name: "Alain".into(),
            address: Address::new(Some("Alain"), email).unwrap(),
            username: email.into(),
            protocol: Protocol::ImapSmtp,
            auth: Auth::Password,
            incoming: Some(Server { host: "127.0.0.1".into(), port: 1, tls: Tls::Implicit }),
            outgoing: None,
            dav: None,
            signature: String::new(),
        }
    }

    fn engine(email: &str) -> Engine {
        let mut engine = Engine::sample();
        engine.add_account(account(email), Secret::new("x")).unwrap();
        engine
    }

    #[test]
    fn a_certificate_is_imported_for_its_own_address_only() {
        let mut e = engine("alain@voorbeeld.nl");
        assert!(e.certificate("a1").is_none());
        assert_eq!(e.import_certificate("a1", P12, "fout").unwrap_err(), "Het wachtwoord klopt niet, of dit is geen PKCS#12-bestand (.p12 of .pfx).");
        let info = e.import_certificate("a1", P12, "geheim").unwrap();
        assert_eq!(info.name, "Alain Tester");
        assert_eq!(e.certificate("a1"), Some(info));
        e.remove_certificate("a1");
        assert!(e.certificate("a1").is_none());
        assert!(e.import_certificate("sample", P12, "geheim").is_err());

        let mut other = engine("ander@voorbeeld.nl");
        assert_eq!(
            other.import_certificate("a1", P12, "geheim").unwrap_err(),
            "Dit certificaat is uitgegeven voor alain@voorbeeld.nl, niet voor ander@voorbeeld.nl."
        );
        assert!(other.certificate("a1").is_none());
    }

    #[test]
    fn mail_is_signed_when_the_account_has_a_certificate_unless_the_message_says_no() {
        let mut e = engine("alain@voorbeeld.nl");
        let a = account("alain@voorbeeld.nl");
        let raw = b"From: alain@voorbeeld.nl\r\nTo: sanne@voorbeeld.nl\r\nSubject: Hoi\r\nContent-Type: text/plain\r\n\r\nHallo\r\n".to_vec();
        assert_eq!(sign_if_asked(&e.identities, &a, None, raw.clone()).unwrap(), raw, "no certificate: as it was");
        assert!(sign_if_asked(&e.identities, &a, Some(true), raw.clone()).unwrap_err().contains("geen certificaat"));
        e.import_certificate("a1", P12, "geheim").unwrap();
        let signed = String::from_utf8(sign_if_asked(&e.identities, &a, None, raw.clone()).unwrap()).unwrap();
        assert!(signed.contains("multipart/signed") && signed.contains("smime.p7s"));
        assert!(String::from_utf8(sign_if_asked(&e.identities, &a, Some(true), raw.clone()).unwrap()).unwrap().contains("multipart/signed"));
        assert_eq!(sign_if_asked(&e.identities, &a, Some(false), raw.clone()).unwrap(), raw);
    }

    #[test]
    fn the_certificate_is_kept_sealed_and_comes_back_after_a_restart() {
        let dir = std::env::temp_dir().join(format!("mailcraft-certs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("accounts.json");
        let secrets: Arc<dyn mailcraft_secrets::SecretStore> = Arc::new(mailcraft_secrets::MemoryStore::default());
        let mut e = Engine::load(Arc::clone(&secrets), Some(path.clone()));
        e.add_account(account("alain@voorbeeld.nl"), Secret::new("x")).unwrap();
        e.import_certificate("a1", P12, "geheim").unwrap();
        let file = std::fs::read(dir.join("cache").join("a1.identity")).unwrap();
        assert!(!file.windows(12).any(|w| w == b"Alain Tester"), "nothing readable on disk");
        drop(e);
        let mut again = Engine::load(Arc::clone(&secrets), Some(path.clone()));
        assert_eq!(again.certificate("a1").map(|c| c.name), Some("Alain Tester".into()));
        again.remove_account("a1").unwrap();
        assert!(again.certificate("a1").is_none());
        assert!(!dir.join("cache").join("a1.identity").exists(), "removing the account removes the certificate");
        let _ = std::fs::remove_dir_all(dir);
    }
}
