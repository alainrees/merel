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
use merel_mime::{Body, Encryption, ParsedHeader};
use merel_model::Address;
use merel_pgp::{KeyInfo, PublicKey, SecretKey};
use merel_store::KeyKind;
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
    /// The correspondents' keys and the public halves of the own keys, for the sent copies.
    fn people(&self) -> Vec<PublicKey> {
        let mut keys: Vec<PublicKey> = self.people.read().map(|p| p.values().cloned().collect()).unwrap_or_default();
        if let Ok(own) = self.own.read() {
            keys.extend(own.values().map(|k| k.public()));
        }
        keys
    }

    /// The own key of `account_id`.
    pub fn own_key(&self, account_id: &str) -> Option<Arc<SecretKey>> {
        self.own.read().ok()?.get(account_id).cloned()
    }

    /// The addresses among `addresses` without a known key mail can be encrypted to at `now`,
    /// lowercase, each once.
    pub fn missing<'a>(&self, addresses: impl Iterator<Item = &'a Address>, now: i64) -> Vec<String> {
        let people = self.people.read().ok();
        let mut out: Vec<String> = Vec::new();
        for email in addresses.map(|a| a.email().to_lowercase()) {
            let known = people.as_ref().and_then(|p| p.get(&email)).is_some_and(|k| k.can_encrypt(now));
            if !known && !out.contains(&email) {
                out.push(email);
            }
        }
        out
    }

    /// The known keys of `addresses`.
    pub fn keys_of<'a>(&self, addresses: impl Iterator<Item = &'a Address>) -> Vec<PublicKey> {
        let Ok(people) = self.people.read() else { return Vec::new() };
        addresses.filter_map(|a| people.get(&a.email().to_lowercase()).cloned()).collect()
    }

    /// What the PGP/MIME signature on `raw` says, if it has one.
    pub fn check(&self, raw: &[u8]) -> Option<merel_pgp::Verdict> {
        merel_pgp::check_message(raw, &self.people())
    }

    /// The body of the PGP/MIME encrypted message `raw`, opened with the key of `account_id`;
    /// one that does not open shows why instead of its contents. An opened message also gives
    /// its header as it reads inside the encryption, and the keys it carried there.
    pub fn open(&self, account_id: &str, raw: &[u8]) -> (Body, Option<(ParsedHeader, Carried)>) {
        let key = self.own.read().ok().and_then(|map| map.get(account_id).cloned());
        let people = self.people();
        let opened = match key {
            Some(key) => merel_pgp::open_message(&key, raw, &people).map_err(|e| e.to_string()),
            None => Err("Dit bericht is met OpenPGP versleuteld, maar je hebt voor dit account geen OpenPGP-sleutel. Importeer die onder Bestand om het te lezen.".to_string()),
        };
        match opened {
            Ok(opened) => {
                let mut body = merel_mime::parse_body(&opened.plain);
                let header = merel_mime::parse_header(&opened.plain);
                body.encrypted = Some(Encryption::Opened);
                // Signed inside the encryption, or signed as PGP/MIME and then encrypted.
                let verdict = merel_pgp::check_message(&opened.plain, &people).or(opened.verdict);
                body.pgp_signature = verdict.map(|v| self.weigh(v, header.from.as_ref()));
                (body, Some((header, Carried { autocrypt: opened.autocrypt, gossip: opened.gossip })))
            }
            Err(reason) => (Body { text: format!("🔒 {reason}"), encrypted: Some(Encryption::Locked(reason)), ..Body::default() }, None),
        }
    }

    /// `verdict` as it stands for mail from `from`: a signature that only checks out with the
    /// key the message brought along counts for nothing when another key is known for `from`
    /// (someone else's key under that address).
    fn weigh(&self, verdict: merel_pgp::Verdict, from: Option<&Address>) -> merel_pgp::Verdict {
        let merel_pgp::Verdict::Valid { signer } = &verdict else { return verdict };
        let Ok(people) = self.people.read() else { return verdict };
        let known = people.values().any(|k| k.info().fingerprint == signer.fingerprint)
            || self.own.read().is_ok_and(|own| own.values().any(|k| k.info().fingerprint == signer.fingerprint));
        let other = from.and_then(|f| people.get(&f.email().to_lowercase())).is_some_and(|k| k.info().fingerprint != signer.fingerprint);
        if !known && other { merel_pgp::Verdict::UnknownKey(signer.fingerprint.clone()) } else { verdict }
    }
}

/// The keys an OpenPGP-encrypted message carried inside: the sender's own (`Autocrypt`) and
/// those of the others it went to (`Autocrypt-Gossip`), for the engine to weigh.
#[derive(Debug, Default)]
pub struct Carried {
    pub autocrypt: Option<(String, PublicKey)>,
    pub gossip: Vec<(String, PublicKey)>,
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
        self.keep_pgp_key(account_id, key)
    }

    /// Makes `key` the OpenPGP key of `account_id`, sealed on disk.
    pub(crate) fn keep_pgp_key(&mut self, account_id: &str, key: SecretKey) -> Result<KeyInfo, String> {
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

    /// The correspondent's key Merel knows for `email`, if any.
    pub fn pgp_key_of(&self, email: &str) -> Option<KeyInfo> {
        self.keys.openpgp.people.read().ok()?.get(&email.trim().to_lowercase()).map(|k| k.info().clone())
    }

    /// Learns the key in the `Autocrypt` header of `raw` (the message `message_id`, as the
    /// server sent it) when it is for the sender's own address `from` and no key is known for
    /// that address yet. Mail in the junk folder teaches nothing.
    pub(crate) fn learn_autocrypt(&mut self, message_id: &str, from: Option<&Address>, raw: &[u8]) {
        if let Some((addr, key)) = merel_pgp::autocrypt(raw) {
            self.learn_key(message_id, from, addr, key);
        }
    }

    /// Learns the keys an encrypted message carried inside: the sender's own, as with
    /// [`Self::learn_autocrypt`], and, when the message is signed with a key Merel believes,
    /// those of the others it went to, for addresses without a key yet (Delta Chat's groups).
    pub(crate) fn learn_carried(&mut self, message_id: &str, from: Option<&Address>, body: &Body, carried: Carried) {
        if let Some((addr, key)) = carried.autocrypt {
            self.learn_key(message_id, from, addr, key);
        }
        let Some(from) = from else { return };
        let Some(merel_pgp::Verdict::Valid { signer }) = &body.pgp_signature else { return };
        if self.pgp_key_of(from.email()).is_none_or(|k| k.fingerprint != signer.fingerprint) || self.in_junk(message_id) {
            return;
        }
        let own: Vec<String> = self.session.accounts.iter().map(|a| a.address.email().to_lowercase()).collect();
        for (addr, key) in carried.gossip {
            if !own.contains(&addr) {
                self.remember_key(addr, key);
            }
        }
    }

    /// Keeps `key` for `addr` when it is the sender's own address and no key is known yet.
    fn learn_key(&mut self, message_id: &str, from: Option<&Address>, addr: String, key: PublicKey) {
        let Some(from) = from else { return };
        if addr != from.email().to_lowercase() || self.in_junk(message_id) {
            return;
        }
        self.remember_key(addr, key);
    }

    /// Keeps `key` for `addr`, on disk too, unless a key is known for it already.
    pub(crate) fn remember_key(&mut self, addr: String, key: PublicKey) {
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
    use merel_model::{Account, Auth, Protocol, Server, Tls};
    use merel_pgp::Verdict;
    use merel_secrets::Secret;

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
        let dir = std::env::temp_dir().join(format!("merel-pgp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("accounts.json");
        let secrets: Arc<dyn merel_secrets::SecretStore> = Arc::new(merel_secrets::MemoryStore::default());
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

    #[test]
    fn mail_from_an_account_with_a_key_is_signed_carries_autocrypt_and_encrypts_to_known_keys() {
        use crate::certificates::{outgoing, parse_body};
        use merel_model::Draft;
        let mut e = engine("alain@voorbeeld.nl");
        let a = account("alain@voorbeeld.nl");
        let raw = b"From: alain@voorbeeld.nl\r\nTo: jij@example.org\r\nSubject: Contract\r\nContent-Type: text/plain\r\n\r\nHallo\r\n".to_vec();
        let draft = Draft { to: vec![jij()], subject: "Contract".into(), body: "Hallo".into(), ..Draft::default() };
        assert_eq!(outgoing(&e.keys, &a, &draft, raw.clone()).unwrap(), raw, "no key: as it was");
        e.import_pgp_key("a1", ALAIN, "geheim").unwrap();

        let signed = outgoing(&e.keys, &a, &draft, raw.clone()).unwrap();
        assert!(String::from_utf8_lossy(&signed).contains("application/pgp-signature"));
        let (addr, key) = merel_pgp::autocrypt(&signed).unwrap();
        assert_eq!(addr, "alain@voorbeeld.nl");
        assert_eq!(key.info().name, "Alain Tester");
        let unsigned = outgoing(&e.keys, &a, &Draft { sign: Some(false), ..draft.clone() }, raw.clone()).unwrap();
        assert!(!String::from_utf8_lossy(&unsigned).contains("multipart/signed") && merel_pgp::autocrypt(&unsigned).is_some());

        let secret = Draft { encrypt: true, ..draft.clone() };
        let refused = outgoing(&e.keys, &a, &secret, raw.clone()).unwrap_err();
        assert!(refused.contains("Van jij@example.org is nog geen certificaat of sleutel bekend"), "{refused}");
        assert_eq!(e.missing_certificates("a1", &[jij()]), vec!["jij@example.org".to_string()]);
        e.learn_autocrypt("x", Some(&jij()), &with_autocrypt("jij@example.org", SIGNED));
        assert!(e.missing_certificates("a1", &[jij()]).is_empty());
        let sent = outgoing(&e.keys, &a, &secret, raw).unwrap();
        assert!(merel_pgp::is_encrypted(&sent) && !sent.windows(5).any(|w| w == b"Hallo"));
        let mine = parse_body(&e.keys, "a1", &sent);
        assert_eq!(mine.encrypted, Some(Encryption::Opened));
        assert!(mine.text.contains("Hallo"), "{}", mine.text);
        assert!(matches!(mine.pgp_signature, Some(Verdict::Valid { signer }) if signer.covers("alain@voorbeeld.nl")), "own key");
    }
}
