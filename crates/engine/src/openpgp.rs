//! OpenPGP keys: one own key per account, imported from an exported key file with its
//! passphrase and kept sealed next to the offline cache, and the public keys of correspondents.
//!
//! Correspondents' keys come the Autocrypt way (autocrypt.org): a message whose `Autocrypt`
//! header carries a key for its sender's own address teaches that key, once; a later, other key
//! for the same address is not taken over by itself, so a forged header cannot swap a known key.
//! The public keys are kept in `pgp_keys.json`.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use base64::Engine as _;
use mailcraft_mime::{Body, Encryption};
use mailcraft_model::Address;
use mailcraft_pgp::{KeyInfo, PublicKey, SecretKey};
use mailcraft_store::KeyKind;
use serde::{Deserialize, Serialize};

use crate::{Engine, SAMPLE_ACCOUNT, config};

/// Most correspondents' keys kept; beyond it, new addresses are not learned.
const MAX_PEOPLE: usize = 5000;

/// The OpenPGP keys the worker threads need: each account's own, and the correspondents'.
#[derive(Clone, Default)]
pub struct PgpKeys {
    pub own: Arc<RwLock<HashMap<String, Arc<SecretKey>>>>,
    /// By lowercase address.
    pub people: Arc<RwLock<HashMap<String, PublicKey>>>,
}

impl PgpKeys {
    fn people(&self) -> Vec<PublicKey> {
        self.people.read().map(|p| p.values().cloned().collect()).unwrap_or_default()
    }

    /// What the PGP/MIME signature on `raw` says, if it has one.
    pub fn check(&self, raw: &[u8]) -> Option<mailcraft_pgp::Verdict> {
        mailcraft_pgp::check_message(raw, &self.people())
    }

    /// The body of the PGP/MIME encrypted message `raw`, opened with the key of `account_id`;
    /// one that does not open shows why instead of its contents.
    pub fn open_body(&self, account_id: &str, raw: &[u8]) -> Body {
        let key = self.own.read().ok().and_then(|map| map.get(account_id).cloned());
        let people = self.people();
        let opened = match key {
            Some(key) => mailcraft_pgp::decrypt_message(&key, raw, &people).map_err(|e| e.to_string()),
            None => Err("Dit bericht is met OpenPGP versleuteld, maar je hebt voor dit account geen OpenPGP-sleutel. Importeer die onder Bestand om het te lezen.".to_string()),
        };
        match opened {
            Ok((plain, inner)) => {
                let mut body = mailcraft_mime::parse_body(&plain);
                body.encrypted = Some(Encryption::Opened);
                // Signed inside the encryption, or signed as PGP/MIME and then encrypted.
                body.pgp_signature = mailcraft_pgp::check_message(&plain, &people).or(inner);
                body
            }
            Err(reason) => Body { text: format!("🔒 {reason}"), encrypted: Some(Encryption::Locked(reason)), ..Body::default() },
        }
    }
}

/// `pgp_keys.json`: the correspondents' public keys, base64 by address.
#[derive(Default, Serialize, Deserialize)]
struct PeopleFile {
    #[serde(default)]
    keys: std::collections::BTreeMap<String, String>,
}

impl Engine {
    /// The OpenPGP key of `account_id`, if it has one.
    pub fn pgp_key(&self, account_id: &str) -> Option<KeyInfo> {
        self.keys.openpgp.own.read().ok()?.get(account_id).map(|k| k.info().clone())
    }

    /// Imports an exported private key (`gpg --export-secret-keys`, armored or not) as the
    /// OpenPGP key of `account_id`, unlocked with `passphrase`, replacing the one it had. One
    /// of the key's addresses must be the account's.
    pub fn import_pgp_key(&mut self, account_id: &str, bytes: &[u8], passphrase: &str) -> Result<KeyInfo, String> {
        let account = self
            .session
            .accounts
            .iter()
            .find(|a| a.id == account_id && a.id != SAMPLE_ACCOUNT)
            .ok_or_else(|| "Dit account bestaat niet meer.".to_string())?;
        let key = SecretKey::import(bytes, passphrase).map_err(|e| e.to_string())?;
        let email = account.address.email().to_string();
        if !key.info().covers(&email) {
            let issued = key.info().emails.join(", ");
            let issued = if issued.is_empty() { "geen e-mailadres".to_string() } else { issued };
            return Err(format!("Deze sleutel hoort bij {issued}, niet bij {email}."));
        }
        if !self.cache.has_store() {
            self.cache = crate::cache::Cache::open(self.config_path.as_deref(), &*self.secrets);
        }
        self.cache.save_identity(account_id, KeyKind::OpenPgp, &key.to_stored()).map_err(|e| format!("De sleutel kan niet worden bewaard: {e}"))?;
        let info = key.info().clone();
        self.keys.openpgp.own.write().map_err(|_| "De sleutel kan niet worden bewaard.".to_string())?.insert(account_id.to_string(), Arc::new(key));
        Ok(info)
    }

    /// Forgets the OpenPGP key of `account_id`, on disk too.
    pub fn remove_pgp_key(&mut self, account_id: &str) {
        self.cache.remove_identity(account_id, KeyKind::OpenPgp);
        if let Ok(mut map) = self.keys.openpgp.own.write() {
            map.remove(account_id);
        }
    }

    /// The correspondent's key Mailcraft knows for `email`, if any.
    pub fn pgp_key_of(&self, email: &str) -> Option<KeyInfo> {
        self.keys.openpgp.people.read().ok()?.get(&email.trim().to_lowercase()).map(|k| k.info().clone())
    }

    /// Learns the key in the `Autocrypt` header of `raw` (the message `message_id`, as the
    /// server sent it) when it is for the sender's own address `from` and no key is known for
    /// that address yet. Mail in the junk folder teaches nothing.
    pub(crate) fn learn_autocrypt(&mut self, message_id: &str, from: Option<&Address>, raw: &[u8]) {
        let Some(from) = from else { return };
        let Some((addr, key)) = mailcraft_pgp::autocrypt(raw) else { return };
        if addr != from.email().to_lowercase() || self.in_junk(message_id) {
            return;
        }
        let Ok(mut people) = self.keys.openpgp.people.write() else { return };
        if people.contains_key(&addr) || people.len() >= MAX_PEOPLE {
            return;
        }
        people.insert(addr, key);
        let file =
            PeopleFile { keys: people.iter().map(|(k, v)| (k.clone(), base64::engine::general_purpose::STANDARD.encode(v.to_bytes()))).collect() };
        drop(people);
        if let Some(path) = self.config_path.as_deref()
            && let Err(e) = config::save_json(&config::pgp_keys_path(path), &file)
        {
            log::warn!("OpenPGP keys not saved: {e}");
        }
    }

    /// Reads the accounts' own keys from the sealed store and the correspondents' from
    /// `pgp_keys.json`.
    pub(crate) fn load_pgp_keys(&mut self) {
        if let Ok(mut own) = self.keys.openpgp.own.write() {
            for account in &self.session.accounts {
                if let Some(plain) = self.cache.load_identity(&account.id, KeyKind::OpenPgp) {
                    match SecretKey::from_stored(&plain) {
                        Ok(key) => {
                            own.insert(account.id.clone(), Arc::new(key));
                        }
                        Err(e) => log::warn!("{}: OpenPGP key not loaded: {e}", account.id),
                    }
                }
            }
        }
        let Some(path) = self.config_path.as_deref() else { return };
        let file: PeopleFile = config::load_json(&config::pgp_keys_path(path));
        let Ok(mut people) = self.keys.openpgp.people.write() else { return };
        for (email, data) in file.keys {
            if let Some(key) = base64::engine::general_purpose::STANDARD.decode(data).ok().and_then(|b| PublicKey::from_bytes(&b)) {
                people.insert(email.to_lowercase(), key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use mailcraft_model::{Account, Auth, Protocol, Server, Tls};
    use mailcraft_pgp::Verdict;
    use mailcraft_secrets::Secret;

    use super::*;

    const ALAIN: &[u8] = include_bytes!("../../pgp/testdata/alain-secret.asc");
    const JIJ_PUBLIC: &[u8] = include_bytes!("../../pgp/testdata/jij-public.asc");
    const SIGNED: &[u8] = include_bytes!("../../pgp/testdata/signed-gpg.eml");
    const ENCRYPTED: &[u8] = include_bytes!("../../pgp/testdata/encrypted-gpg.eml");

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

    /// `raw` with an Autocrypt header carrying Jij's key for `addr` on top.
    fn with_autocrypt(addr: &str, raw: &[u8]) -> Vec<u8> {
        let key = base64::engine::general_purpose::STANDARD.encode(PublicKey::from_bytes(JIJ_PUBLIC).unwrap().to_bytes());
        let mut out = format!("Autocrypt: addr={addr}; keydata={key}\r\n").into_bytes();
        out.extend_from_slice(raw);
        out
    }

    fn jij() -> Address {
        Address::new(None, "jij@example.org").unwrap()
    }

    #[test]
    fn an_own_key_is_imported_for_the_accounts_address_and_opens_mail() {
        let mut e = engine("alain@voorbeeld.nl");
        assert_eq!(e.import_pgp_key("a1", ALAIN, "fout").unwrap_err(), "De wachtwoordzin klopt niet.");
        let info = e.import_pgp_key("a1", ALAIN, "geheim").unwrap();
        assert_eq!(info.name, "Alain Tester");
        assert_eq!(e.pgp_key("a1"), Some(info));
        let body = crate::certificates::parse_body(&e.keys, "a1", ENCRYPTED);
        assert_eq!(body.encrypted, Some(Encryption::Opened));
        assert!(body.text.contains("Het contract staat klaar."), "{}", body.text);
        assert!(matches!(body.pgp_signature, Some(Verdict::UnknownKey(_))), "Jij's key is not known yet");
        e.remove_pgp_key("a1");
        let locked = crate::certificates::parse_body(&e.keys, "a1", ENCRYPTED);
        assert!(matches!(locked.encrypted, Some(Encryption::Locked(_))) && locked.text.contains("geen OpenPGP-sleutel"));

        let mut other = engine("ander@voorbeeld.nl");
        assert_eq!(
            other.import_pgp_key("a1", ALAIN, "geheim").unwrap_err(),
            "Deze sleutel hoort bij alain@voorbeeld.nl, niet bij ander@voorbeeld.nl."
        );
    }

    #[test]
    fn a_key_is_learned_from_autocrypt_once_and_then_checks_signatures() {
        let mut e = engine("alain@voorbeeld.nl");
        assert!(matches!(crate::certificates::parse_body(&e.keys, "a1", SIGNED).pgp_signature, Some(Verdict::UnknownKey(_))));
        // A header for another address than the sender's teaches nothing.
        e.learn_autocrypt("x", Some(&Address::new(None, "bank@example.org").unwrap()), &with_autocrypt("jij@example.org", SIGNED));
        assert!(e.pgp_key_of("jij@example.org").is_none());
        e.learn_autocrypt("x", Some(&jij()), &with_autocrypt("jij@example.org", SIGNED));
        assert_eq!(e.pgp_key_of("Jij@example.org").map(|k| k.name), Some("Jij Tester".into()));
        match crate::certificates::parse_body(&e.keys, "a1", SIGNED).pgp_signature {
            Some(Verdict::Valid { signer }) => assert!(signer.covers("jij@example.org")),
            other => panic!("{other:?}"),
        }
        e.import_pgp_key("a1", ALAIN, "geheim").unwrap();
        let opened = crate::certificates::parse_body(&e.keys, "a1", ENCRYPTED);
        assert!(matches!(opened.pgp_signature, Some(Verdict::Valid { .. })), "signed inside the encryption");
    }

    #[test]
    fn the_keys_are_kept_and_come_back_after_a_restart() {
        let dir = std::env::temp_dir().join(format!("mailcraft-pgp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("accounts.json");
        let secrets: Arc<dyn mailcraft_secrets::SecretStore> = Arc::new(mailcraft_secrets::MemoryStore::default());
        let mut e = Engine::load(Arc::clone(&secrets), Some(path.clone()));
        e.add_account(account("alain@voorbeeld.nl"), Secret::new("x")).unwrap();
        e.import_pgp_key("a1", ALAIN, "geheim").unwrap();
        e.learn_autocrypt("x", Some(&jij()), &with_autocrypt("jij@example.org", SIGNED));
        let file = std::fs::read(dir.join("cache").join("a1.pgpkey")).unwrap();
        assert!(!file.windows(12).any(|w| w == b"Alain Tester"), "nothing readable on disk");
        drop(e);
        let mut again = Engine::load(Arc::clone(&secrets), Some(path.clone()));
        assert_eq!(again.pgp_key("a1").map(|k| k.name), Some("Alain Tester".into()));
        assert!(again.pgp_key_of("jij@example.org").is_some());
        again.remove_account("a1").unwrap();
        assert!(again.pgp_key("a1").is_none());
        assert!(!dir.join("cache").join("a1.pgpkey").exists(), "removing the account removes the key");
        let _ = std::fs::remove_dir_all(dir);
    }
}
