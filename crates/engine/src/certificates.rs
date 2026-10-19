//! S/MIME certificates: one per account, imported from a PKCS#12 file and kept sealed next to
//! the offline cache. Outgoing mail of an account with a certificate is signed, unless the
//! message says otherwise, and encrypted when the message asks for it.
//!
//! The certificates of the people who write to us come with their signed mail, as Outlook
//! learns them: a valid signature that matches the sender keeps the certificate (public, so
//! in `certificates.json`), and mail to that address can then be encrypted.
//!
//! The worker threads share the certificates with the engine, so a certificate imported or
//! removed takes effect with the next message, without reconnecting.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use base64::Engine as _;
use mailcraft_mime::{Body, Encryption};
use mailcraft_model::{Account, Address, Draft};
use mailcraft_smime::{CertInfo, Identity, Signature};
use serde::{Deserialize, Serialize};

use crate::{Engine, SAMPLE_ACCOUNT, config};

/// The certificate of each account that has one, by account id.
pub type Identities = Arc<RwLock<HashMap<String, Arc<Identity>>>>;

/// The certificates of correspondents, DER, by lowercase address.
pub type People = Arc<RwLock<HashMap<String, Vec<u8>>>>;

/// Most correspondents' certificates kept; the oldest learned go first beyond it.
const MAX_PEOPLE: usize = 5000;

/// Everything the worker threads need to sign, encrypt and decrypt.
#[derive(Clone, Default)]
pub struct Keys {
    pub identities: Identities,
    pub people: People,
}

/// `certificates.json`: the correspondents' certificates, base64 DER by address.
#[derive(Default, Serialize, Deserialize)]
struct PeopleFile {
    #[serde(default)]
    certificates: std::collections::BTreeMap<String, String>,
}

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

/// `raw` as it goes out for `draft` from `account`: signed when asked (see [`sign_if_asked`]),
/// then encrypted when the draft asks for it. A message that should be encrypted and cannot
/// be is not sent in the clear.
pub fn outgoing(keys: &Keys, account: &Account, draft: &Draft, raw: Vec<u8>) -> Result<Vec<u8>, String> {
    let signed = sign_if_asked(&keys.identities, account, draft.sign, raw)?;
    if !draft.encrypt {
        return Ok(signed);
    }
    let own = keys.identities.read().ok().and_then(|map| map.get(&account.id).map(|i| i.certificate_der().to_vec()));
    let Some(own) = own else {
        return Err("Dit account heeft geen certificaat. Importeer er een onder Bestand om versleuteld te mailen, of zet Versleutelen uit.".into());
    };
    let recipients: Vec<&Address> = draft.to.iter().chain(&draft.cc).collect();
    let missing = missing(&keys.people, recipients.iter().copied());
    if !missing.is_empty() {
        return Err(format!("{} Zet Versleutelen uit om toch te verzenden.", missing_text(&missing)));
    }
    let people = keys.people.read().map_err(|_| "De certificaten kunnen niet worden gelezen.".to_string())?;
    // The sender's own copy too, so Verzonden items stays readable.
    let mut certificates = vec![own];
    certificates.extend(recipients.iter().filter_map(|a| people.get(&a.email().to_lowercase()).cloned()));
    mailcraft_smime::encrypt_message(&signed, &certificates).map_err(|e| e.to_string())
}

/// The addresses among `addresses` without a known certificate, lowercase, each once.
fn missing<'a>(people: &People, addresses: impl Iterator<Item = &'a Address>) -> Vec<String> {
    let Ok(people) = people.read() else { return addresses.map(|a| a.email().to_lowercase()).collect() };
    let mut out: Vec<String> = Vec::new();
    for email in addresses.map(|a| a.email().to_lowercase()) {
        if !people.contains_key(&email) && !out.contains(&email) {
            out.push(email);
        }
    }
    out
}

/// Says which recipients cannot get encrypted mail yet.
pub fn missing_text(missing: &[String]) -> String {
    format!(
        "Van {} is nog geen certificaat bekend, dus het bericht kan niet versleuteld naar {} worden gestuurd. Vraag om eerst een ondertekend bericht te sturen.",
        missing.join(", "),
        if missing.len() == 1 { "die ontvanger" } else { "deze ontvangers" }
    )
}

/// The body of `raw` for `account_id`: an encrypted message is opened with the account's
/// certificate first; one that does not open shows why instead of its contents.
pub fn parse_body(identities: &Identities, account_id: &str, raw: &[u8]) -> Body {
    if !mailcraft_smime::is_encrypted(raw) {
        return mailcraft_mime::parse_body(raw);
    }
    let identity = identities.read().ok().and_then(|map| map.get(account_id).cloned());
    let opened = match identity {
        Some(identity) => mailcraft_smime::decrypt_message(&identity, raw).map_err(|e| e.to_string()),
        None => Err("Dit bericht is versleuteld, maar je hebt voor dit account geen certificaat met privésleutel. Importeer het onder Bestand om het te lezen.".to_string()),
    };
    match opened {
        Ok(plain) => {
            let mut body = mailcraft_mime::parse_body(&plain);
            body.encrypted = Some(Encryption::Opened);
            body
        }
        Err(reason) => Body { text: format!("🔒 {reason}"), encrypted: Some(Encryption::Locked(reason)), ..Body::default() },
    }
}

impl Engine {
    /// The addresses among `addresses` mail cannot be encrypted to yet, for want of a
    /// certificate.
    pub fn missing_certificates(&self, addresses: &[Address]) -> Vec<String> {
        missing(&self.keys.people, addresses.iter())
    }

    /// Keeps the certificate of a valid signature on `message_id` that belongs to its sender
    /// `from`. Mail in the junk folder teaches nothing.
    pub(crate) fn learn_certificate(&mut self, message_id: &str, from: Option<&Address>, body: &Body) {
        let (Some(from), Some(Signature::Valid { signer, certificate })) = (from, &body.signature) else { return };
        let junk = self
            .session
            .message(message_id)
            .and_then(|m| self.session.folder(&m.folder_id))
            .is_some_and(|f| f.kind == mailcraft_model::FolderKind::Junk);
        if junk {
            return;
        }
        let email = from.email().to_lowercase();
        if !signer.covers(&email) || !mailcraft_smime::can_encrypt_to(certificate) {
            return;
        }
        let Ok(mut people) = self.keys.people.write() else { return };
        if people.get(&email) == Some(certificate) {
            return;
        }
        if people.len() >= MAX_PEOPLE && !people.contains_key(&email) {
            return;
        }
        people.insert(email, certificate.clone());
        let file =
            PeopleFile { certificates: people.iter().map(|(k, v)| (k.clone(), base64::engine::general_purpose::STANDARD.encode(v))).collect() };
        drop(people);
        if let Some(path) = self.config_path.as_deref()
            && let Err(e) = config::save_json(&config::certificates_path(path), &file)
        {
            log::warn!("certificates not saved: {e}");
        }
    }

    /// Reads the correspondents' certificates kept in `certificates.json`.
    pub(crate) fn load_people(&mut self) {
        let Some(path) = self.config_path.as_deref() else { return };
        let file: PeopleFile = config::load_json(&config::certificates_path(path));
        let Ok(mut people) = self.keys.people.write() else { return };
        for (email, der) in file.certificates {
            if let Ok(der) = base64::engine::general_purpose::STANDARD.decode(der)
                && mailcraft_smime::can_encrypt_to(&der)
            {
                people.insert(email.to_lowercase(), der);
            }
        }
    }

    /// Reads the certificate each account kept from the sealed store.
    pub(crate) fn load_identities(&mut self) {
        let Ok(mut map) = self.keys.identities.write() else { return };
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
        self.keys.identities.read().ok()?.get(account_id).map(|i| i.info().clone())
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
        self.keys
            .identities
            .write()
            .map_err(|_| "Het certificaat kan niet worden bewaard.".to_string())?
            .insert(account_id.to_string(), Arc::new(identity));
        Ok(info)
    }

    /// Forgets the certificate of `account_id`, on disk too.
    pub fn remove_certificate(&mut self, account_id: &str) {
        self.cache.remove_identity(account_id);
        if let Ok(mut map) = self.keys.identities.write() {
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
    const JIJ: &[u8] = include_bytes!("../../smime/testdata/jij.p12");
    /// Signed by "Jij Tester" <jij@example.org>.
    const SIGNED: &[u8] = include_bytes!("../../smime/testdata/signed-openssl.eml");

    fn jij() -> Address {
        Address::new(None, "Jij@Example.org").unwrap()
    }

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
        assert_eq!(sign_if_asked(&e.keys.identities, &a, None, raw.clone()).unwrap(), raw, "no certificate: as it was");
        assert!(sign_if_asked(&e.keys.identities, &a, Some(true), raw.clone()).unwrap_err().contains("geen certificaat"));
        e.import_certificate("a1", P12, "geheim").unwrap();
        let signed = String::from_utf8(sign_if_asked(&e.keys.identities, &a, None, raw.clone()).unwrap()).unwrap();
        assert!(signed.contains("multipart/signed") && signed.contains("smime.p7s"));
        assert!(String::from_utf8(sign_if_asked(&e.keys.identities, &a, Some(true), raw.clone()).unwrap()).unwrap().contains("multipart/signed"));
        assert_eq!(sign_if_asked(&e.keys.identities, &a, Some(false), raw.clone()).unwrap(), raw);
    }

    #[test]
    fn mail_is_encrypted_only_to_people_whose_certificate_came_with_signed_mail() {
        let mut e = engine("alain@voorbeeld.nl");
        let a = account("alain@voorbeeld.nl");
        e.import_certificate("a1", P12, "geheim").unwrap();
        let draft = Draft { to: vec![jij()], subject: "Contract".into(), body: "Hallo".into(), encrypt: true, ..Draft::default() };
        let raw = b"From: alain@voorbeeld.nl\r\nTo: jij@example.org\r\nSubject: Contract\r\nContent-Type: text/plain\r\n\r\nHallo\r\n".to_vec();
        let refused = outgoing(&e.keys, &a, &draft, raw.clone()).unwrap_err();
        assert!(refused.contains("Van jij@example.org is nog geen certificaat bekend"), "{refused}");
        assert_eq!(e.missing_certificates(&[jij()]), vec!["jij@example.org".to_string()]);

        // A valid signature teaches the certificate, but only for the address it was issued to.
        let body = mailcraft_mime::parse_body(SIGNED);
        e.learn_certificate("x", Some(&Address::new(None, "bank@example.org").unwrap()), &body);
        assert!(!e.missing_certificates(&[jij()]).is_empty());
        e.learn_certificate("x", Some(&jij()), &body);
        assert!(e.missing_certificates(&[jij()]).is_empty());

        let sent = outgoing(&e.keys, &a, &draft, raw.clone()).unwrap();
        assert!(mailcraft_smime::is_encrypted(&sent) && !sent.windows(5).any(|w| w == b"Hallo"));
        // The recipient opens it, and so does the sender, signature and all.
        let theirs = Identity::from_pkcs12(JIJ, "geheim", now()).unwrap();
        assert!(String::from_utf8_lossy(&mailcraft_smime::decrypt_message(&theirs, &sent).unwrap()).contains("multipart/signed"));
        let mine = parse_body(&e.keys.identities, "a1", &sent);
        assert_eq!(mine.encrypted, Some(Encryption::Opened));
        assert!(mine.text.contains("Hallo"), "{}", mine.text);
        assert!(matches!(mine.signature, Some(Signature::Valid { .. })));
        let locked = parse_body(&Identities::default(), "a1", &sent);
        assert!(matches!(locked.encrypted, Some(Encryption::Locked(_))) && locked.text.contains("geen certificaat met privésleutel"));

        let plain = Draft { encrypt: false, ..draft };
        assert!(!mailcraft_smime::is_encrypted(&outgoing(&e.keys, &a, &plain, raw).unwrap()));
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
        e.learn_certificate("x", Some(&jij()), &mailcraft_mime::parse_body(SIGNED));
        let file = std::fs::read(dir.join("cache").join("a1.identity")).unwrap();
        assert!(!file.windows(12).any(|w| w == b"Alain Tester"), "nothing readable on disk");
        drop(e);
        let mut again = Engine::load(Arc::clone(&secrets), Some(path.clone()));
        assert_eq!(again.certificate("a1").map(|c| c.name), Some("Alain Tester".into()));
        assert!(again.missing_certificates(&[jij()]).is_empty(), "the correspondents' certificates are kept too");
        again.remove_account("a1").unwrap();
        assert!(again.certificate("a1").is_none());
        assert!(!dir.join("cache").join("a1.identity").exists(), "removing the account removes the certificate");
        let _ = std::fs::remove_dir_all(dir);
    }
}
