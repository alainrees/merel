//! OpenPGP for mail (RFC 9580, PGP/MIME as in RFC 3156): the person's own key, imported from
//! an armored file with its passphrase, signatures on mail that comes in, encrypted mail
//! opened with the own key, and the keys correspondents send along in an `Autocrypt` header.
//!
//! The cryptography is rPGP (`pgp`), pure Rust. The own key is kept unlocked in a
//! [`SecretKey`], which only leaves this crate as [`SecretKey::to_stored`], for the engine to
//! seal on disk; it is never printed.

use std::fmt;
use std::io::Cursor;

use base64::Engine as _;
use merel_smime::raw::{crlf, fields, header, parameter, split_entity, split_message, two_parts};
use pgp::composed::{DecryptionOptions, Deserializable, DetachedSignature, Message, SignedPublicKey, SignedSecretKey, TheRing};
use pgp::ser::Serialize as _;
use pgp::types::{KeyDetails, Password};
use zeroize::Zeroizing;

mod send;
pub use send::{encrypt_message, sign_message, with_header};

/// Most bytes a key file or an `Autocrypt` key may have.
pub const MAX_KEY: usize = 256 * 1024;
/// Most bytes an encrypted message may hold once opened.
const MAX_CONTENT: usize = 64 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum PgpError {
    #[error("{}", merel_i18n::tr("pgp-not-a-key"))]
    NotAKey,
    #[error("{}", merel_i18n::tr("pgp-wrong-passphrase"))]
    Passphrase,
    #[error("{}", merel_i18n::tr("pgp-revoked"))]
    Revoked,
    #[error("{}", merel_i18n::tr("pgp-no-decrypt-key"))]
    NoEncryptionKey,
    #[error("{}", merel_i18n::tr("pgp-not-for-me"))]
    NotForMe,
    #[error("{}", merel_i18n::tr("pgp-decrypt-failed"))]
    Decrypt,
    #[error("{}", merel_i18n::tr("pgp-sign-failed"))]
    Sign,
    #[error("{}", merel_i18n::tr("pgp-encrypt-failed"))]
    Encrypt,
    #[error("{}", merel_i18n::tr("pgp-generate-failed"))]
    Generate,
}

/// What a key says about its holder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyInfo {
    /// Upper-case hex, as mail programs show it.
    pub fingerprint: String,
    /// The name of the first user id, or its address.
    pub name: String,
    /// The addresses in the user ids, lower case.
    pub emails: Vec<String>,
    /// Seconds since the Unix epoch.
    pub created: i64,
}

impl KeyInfo {
    /// Whether `email` is one of the key's addresses.
    pub fn covers(&self, email: &str) -> bool {
        let email = email.trim().to_lowercase();
        self.emails.contains(&email)
    }

    /// The fingerprint in groups of four, the way people compare it on the phone.
    pub fn grouped(&self) -> String {
        self.fingerprint.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join(" ")
    }
}

fn info_of(primary: &impl KeyDetails, users: &[pgp::types::SignedUser]) -> KeyInfo {
    let mut emails = Vec::new();
    let mut name = String::new();
    for user in users {
        let Some(id) = user.id.as_str() else { continue };
        let id: String = id.chars().filter(|c| !c.is_control()).collect();
        let (who, email) = match (id.rfind('<'), id.rfind('>')) {
            (Some(a), Some(b)) if a < b => (id[..a].trim().to_string(), id[a + 1..b].trim().to_lowercase()),
            _ if id.contains('@') => (String::new(), id.trim().to_lowercase()),
            _ => (id.trim().to_string(), String::new()),
        };
        if name.is_empty() {
            name = if who.is_empty() { email.clone() } else { who };
        }
        if !email.is_empty() && !emails.contains(&email) {
            emails.push(email);
        }
    }
    KeyInfo { fingerprint: format!("{:X}", primary.fingerprint()), name, emails, created: i64::from(primary.created_at().as_secs()) }
}

/// The person's own key, unlocked.
pub struct SecretKey {
    key: SignedSecretKey,
    info: KeyInfo,
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretKey").field("info", &self.info).field("key", &"…").finish()
    }
}

impl SecretKey {
    /// Reads an exported private key (armored or binary, as `gpg --export-secret-keys` and
    /// other programs write it) and unlocks it with `passphrase`.
    pub fn import(data: &[u8], passphrase: &str) -> Result<SecretKey, PgpError> {
        if data.len() > MAX_KEY {
            return Err(PgpError::NotAKey);
        }
        let (mut key, _) = SignedSecretKey::from_reader_single(Cursor::new(data)).map_err(|_| PgpError::NotAKey)?;
        key.verify_bindings().map_err(|_| PgpError::NotAKey)?;
        if !key.details.revocation_signatures.is_empty() {
            return Err(PgpError::Revoked);
        }
        let password = Password::from(passphrase);
        key.primary_key.remove_password(&password).map_err(|_| PgpError::Passphrase)?;
        for sub in &mut key.secret_subkeys {
            sub.key.remove_password(&password).map_err(|_| PgpError::Passphrase)?;
        }
        SecretKey::assemble(key)
    }

    fn assemble(key: SignedSecretKey) -> Result<SecretKey, PgpError> {
        let can_open = key.secret_subkeys.iter().any(|s| s.key.algorithm().can_encrypt()) || key.primary_key.algorithm().can_encrypt();
        if !can_open {
            return Err(PgpError::NoEncryptionKey);
        }
        let info = info_of(&key.primary_key, &key.details.users);
        Ok(SecretKey { key, info })
    }

    pub fn info(&self) -> &KeyInfo {
        &self.info
    }

    /// The public half, to hand to others.
    pub fn public(&self) -> PublicKey {
        PublicKey { key: self.key.to_public_key(), info: self.info.clone() }
    }

    /// The unlocked key in binary form, for the engine to keep sealed. It holds the private key
    /// in the clear and is wiped when dropped.
    pub fn to_stored(&self) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(self.key.to_bytes().unwrap_or_default())
    }

    /// A new key for `email`, made the way Delta Chat makes one so its users can write back:
    /// an Ed25519 key that signs and certifies, with a Curve25519 subkey that encrypts, no
    /// passphrase (Merel seals it on disk itself) and no expiry.
    pub fn generate(name: &str, email: &str) -> Result<SecretKey, PgpError> {
        use pgp::composed::{EncryptionCaps, KeyType, SecretKeyParamsBuilder, SubkeyParamsBuilder};
        use pgp::crypto::ecc_curve::ECCCurve;
        use pgp::crypto::hash::HashAlgorithm;
        use pgp::crypto::sym::SymmetricKeyAlgorithm;
        let name: String = name.chars().filter(|c| !c.is_control() && !matches!(c, '<' | '>')).collect();
        let user = if name.trim().is_empty() { format!("<{email}>") } else { format!("{} <{email}>", name.trim()) };
        let sub = SubkeyParamsBuilder::default()
            .key_type(KeyType::ECDH(ECCCurve::Curve25519Legacy))
            .can_encrypt(EncryptionCaps::All)
            .build()
            .map_err(|_| PgpError::Generate)?;
        let params = SecretKeyParamsBuilder::default()
            .key_type(KeyType::Ed25519Legacy)
            .can_certify(true)
            .can_sign(true)
            .primary_user_id(user)
            .preferred_symmetric_algorithms([SymmetricKeyAlgorithm::AES256, SymmetricKeyAlgorithm::AES128].into_iter().collect())
            .preferred_hash_algorithms([HashAlgorithm::Sha256, HashAlgorithm::Sha512].into_iter().collect())
            .subkey(sub)
            .build()
            .map_err(|_| PgpError::Generate)?;
        let key = params.generate(rand::rngs::OsRng).map_err(|_| PgpError::Generate)?;
        SecretKey::assemble(key)
    }

    /// The key [`SecretKey::to_stored`] wrote.
    pub fn from_stored(bytes: &[u8]) -> Result<SecretKey, PgpError> {
        let key = SignedSecretKey::from_bytes(Cursor::new(bytes)).map_err(|_| PgpError::NotAKey)?;
        SecretKey::assemble(key)
    }
}

/// Someone's public key.
#[derive(Clone, Debug)]
pub struct PublicKey {
    key: SignedPublicKey,
    info: KeyInfo,
}

impl PartialEq for PublicKey {
    fn eq(&self, other: &Self) -> bool {
        self.info.fingerprint == other.info.fingerprint
    }
}

impl PublicKey {
    /// Reads a public key, armored or binary; its self-signatures must hold.
    pub fn from_bytes(data: &[u8]) -> Option<PublicKey> {
        if data.len() > MAX_KEY {
            return None;
        }
        let (key, _) = SignedPublicKey::from_reader_single(Cursor::new(data)).ok()?;
        key.verify_bindings().ok()?;
        if !key.details.revocation_signatures.is_empty() {
            return None;
        }
        let info = info_of(&key.primary_key, &key.details.users);
        Some(PublicKey { key, info })
    }

    /// The key in binary form, to keep.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.key.to_bytes().unwrap_or_default()
    }

    pub fn info(&self) -> &KeyInfo {
        &self.info
    }

    /// Whether the signature `sig` over `content` was made by this key or one of its subkeys.
    fn made(&self, sig: &DetachedSignature, content: &[u8]) -> bool {
        sig.verify(&self.key, content).is_ok() || self.key.public_subkeys.iter().any(|s| sig.verify(s, content).is_ok())
    }

    /// Whether `fingerprints` or `key_ids` (hex, upper case) name this key or a subkey.
    fn named(&self, fingerprints: &[String], key_ids: &[String]) -> bool {
        let mut own = vec![(format!("{:X}", self.key.primary_key.fingerprint()), hex(self.key.primary_key.legacy_key_id()))];
        own.extend(self.key.public_subkeys.iter().map(|s| (format!("{:X}", s.key.fingerprint()), hex(s.key.legacy_key_id()))));
        own.iter().any(|(fp, id)| fingerprints.contains(fp) || key_ids.contains(id))
    }
}

/// What an OpenPGP signature on a message says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Made by the key of `signer`, one Merel knows; the message is as it was signed.
    Valid { signer: KeyInfo },
    /// Made by a known key, but the message changed after it was signed, or the signature is
    /// false.
    Broken,
    /// Signed with a key Merel does not have yet; its id, in hex.
    UnknownKey(String),
    /// Signed in a way Merel cannot check; the reason, in Dutch.
    Unchecked(String),
}

/// A key id in upper-case hex.
fn hex(id: impl AsRef<[u8]>) -> String {
    id.as_ref().iter().map(|b| format!("{b:02X}")).collect()
}

fn unchecked(key: &str) -> Verdict {
    Verdict::Unchecked(merel_i18n::tr(key))
}

/// Checks the detached signature `signature` (armored or binary) over `content` against
/// `keys`.
pub fn verify_detached(content: &[u8], signature: &[u8], keys: &[PublicKey]) -> Verdict {
    let Ok((sig, _)) = DetachedSignature::from_reader_single(Cursor::new(signature)) else {
        return unchecked("sig-unreadable");
    };
    let fingerprints: Vec<String> = sig.signature.issuer_fingerprint().iter().map(|f| format!("{f:X}")).collect();
    let key_ids: Vec<String> = sig.signature.issuer_key_id().iter().map(hex).collect();
    let named: Vec<&PublicKey> = keys.iter().filter(|k| k.named(&fingerprints, &key_ids)).collect();
    if named.is_empty() {
        return Verdict::UnknownKey(fingerprints.first().or(key_ids.first()).cloned().unwrap_or_default());
    }
    match named.iter().find(|k| k.made(&sig, content)) {
        Some(k) => Verdict::Valid { signer: k.info.clone() },
        None => Verdict::Broken,
    }
}

/// The signature on `raw`, when it is signed the PGP/MIME way (`multipart/signed` with
/// `application/pgp-signature` at the top); `None` for a message that is not.
pub fn check_message(raw: &[u8], keys: &[PublicKey]) -> Option<Verdict> {
    let raw = crlf(raw);
    let (head, body) = split_entity(&raw);
    let kind = header(head, b"content-type")?;
    let lower = kind.to_ascii_lowercase();
    if !lower.starts_with("multipart/signed") || !lower.contains("application/pgp-signature") {
        return None;
    }
    let broken = || Some(unchecked("sig-malformed"));
    let Some(boundary) = parameter(&kind, "boundary") else { return broken() };
    let Some((content, second)) = two_parts(body, boundary.as_bytes()) else { return broken() };
    let (_, signature) = split_entity(second);
    Some(verify_detached(content, signature, keys))
}

/// Whether `raw` is a PGP/MIME encrypted message (`multipart/encrypted` with
/// `application/pgp-encrypted` at the top).
pub fn is_encrypted(raw: &[u8]) -> bool {
    let raw = crlf(raw);
    let (head, _) = split_entity(&raw);
    header(head, b"content-type")
        .map(|k| k.to_ascii_lowercase())
        .is_some_and(|k| k.starts_with("multipart/encrypted") && k.contains("application/pgp-encrypted"))
}

/// An encrypted message, opened: see [`open_message`].
#[derive(Debug)]
pub struct Opened {
    /// The message as it was before it was encrypted: the outer header fields, with the ones
    /// the encrypted part repeats in its own header (protected headers, RFC 9788, as Delta
    /// Chat sends every field) taken from inside, then the decrypted entity.
    pub plain: Zeroizing<Vec<u8>>,
    /// What the signature inside says, when it was signed too.
    pub verdict: Option<Verdict>,
    /// The key in the `Autocrypt` header inside, when it is for the sender's address.
    pub autocrypt: Option<(String, PublicKey)>,
    /// The keys in the `Autocrypt-Gossip` headers inside: those of the other people the
    /// message went to, as the sender knows them.
    pub gossip: Vec<(String, PublicKey)>,
}

/// Opens the PGP/MIME message `raw` with `key`. The result is the message as it was before it
/// was encrypted (the outer headers with the decrypted entity under them), and what the
/// signature inside says, if it was signed too, checked against `keys`.
pub fn decrypt_message(key: &SecretKey, raw: &[u8], keys: &[PublicKey]) -> Result<(Zeroizing<Vec<u8>>, Option<Verdict>), PgpError> {
    open_message(key, raw, keys).map(|o| (o.plain, o.verdict))
}

/// Opens the PGP/MIME message `raw` with `key`, as [`decrypt_message`], with the keys it
/// carries inside. A signature is checked against `keys` and against the key in the
/// `Autocrypt` header inside, so the first message from someone checks out with the key it
/// brings; whether that key is to be believed is the caller's to say.
pub fn open_message(key: &SecretKey, raw: &[u8], keys: &[PublicKey]) -> Result<Opened, PgpError> {
    let raw = crlf(raw);
    let (head, body) = split_entity(&raw);
    let kind = header(head, b"content-type").ok_or(PgpError::Decrypt)?;
    let boundary = parameter(&kind, "boundary").ok_or(PgpError::Decrypt)?;
    let (_, second) = two_parts(body, boundary.as_bytes()).ok_or(PgpError::Decrypt)?;
    let (_, armored) = split_entity(second);
    let (message, _) = Message::from_armor(Cursor::new(armored.to_vec())).map_err(|_| PgpError::Decrypt)?;
    let for_me = message_is_for(&message, key);
    let password = Password::empty();
    // GnuPG 2.4 encrypts with its own AEAD packet (OCB) for keys that announce it, which is
    // the default for its new keys; it has to be allowed by name.
    let ring = TheRing {
        secret_keys: vec![&key.key],
        key_passwords: vec![&password],
        decrypt_options: DecryptionOptions::new().enable_gnupg_aead(),
        ..TheRing::default()
    };
    let mut message = match message.decrypt_the_ring(ring, true) {
        Ok((m, _)) => m,
        Err(_) if !for_me => return Err(PgpError::NotForMe),
        Err(_) => return Err(PgpError::Decrypt),
    };
    if message.is_compressed() {
        message = message.decompress().map_err(|_| PgpError::Decrypt)?;
    }
    let mut data = Zeroizing::new(Vec::new());
    {
        use std::io::Read as _;
        let mut limited = (&mut message).take(MAX_CONTENT as u64 + 1);
        limited.read_to_end(&mut data).map_err(|_| PgpError::Decrypt)?;
    }
    if data.len() > MAX_CONTENT {
        return Err(PgpError::Decrypt);
    }
    let data = Zeroizing::new(crlf(&data));
    let (outer, _) = split_message(&raw);
    let plain = protected(&outer, &data);
    let (inner_head, _) = split_entity(&plain);
    let from = header(inner_head, b"from").map(|f| address_of(&f)).unwrap_or_default();
    let autocrypt = header(inner_head, b"autocrypt").and_then(|v| parse_autocrypt(&v)).filter(|(addr, _)| *addr == from);
    let gossip = all_headers(inner_head, b"autocrypt-gossip").iter().filter_map(|v| parse_autocrypt(v)).collect();
    let verdict = message.is_signed().then(|| {
        for k in keys.iter().chain(autocrypt.as_ref().map(|(_, k)| k)) {
            if message.verify(&k.key).is_ok() || k.key.public_subkeys.iter().any(|s| message.verify(s).is_ok()) {
                return Verdict::Valid { signer: k.info.clone() };
            }
        }
        Verdict::UnknownKey(String::new())
    });
    Ok(Opened { plain, verdict, autocrypt, gossip })
}

/// The outer header fields `outer` and the decrypted entity `data` as one message: an outer
/// field the entity repeats gives way to the one inside, except a `From` with another
/// address, where the outer one stays (a message passed on under someone else's name).
fn protected(outer: &[u8], data: &[u8]) -> Zeroizing<Vec<u8>> {
    let (inner_head, inner_body) = split_entity(data);
    let name = |f: &[u8]| f.split(|b| *b == b':').next().unwrap_or_default().trim_ascii().to_ascii_lowercase();
    let outer_from = header(outer, b"from").map(|f| address_of(&f));
    let inner_from = header(inner_head, b"from").map(|f| address_of(&f));
    let keep_outer_from = outer_from.is_some() && inner_from.is_some() && outer_from != inner_from;
    let inner_names: Vec<Vec<u8>> = fields(inner_head).iter().map(|f| name(f)).collect();
    let mut out = Zeroizing::new(Vec::with_capacity(outer.len() + data.len() + 32));
    for f in fields(outer) {
        let n = name(f);
        if !inner_names.contains(&n) || (n == b"from" && keep_outer_from) {
            out.extend_from_slice(f);
        }
    }
    out.extend_from_slice(b"MIME-Version: 1.0\r\n");
    for f in fields(inner_head) {
        let n = name(f);
        if !(n == b"mime-version" || n == b"hp-outer" || (n == b"from" && keep_outer_from)) {
            out.extend_from_slice(f);
        }
    }
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(inner_body);
    out
}

/// The address in a `From` value, lower case: the part in angle brackets, else all of it.
fn address_of(value: &str) -> String {
    let addr = match (value.rfind('<'), value.rfind('>')) {
        (Some(open), Some(close)) if open < close => &value[open + 1..close],
        _ => value,
    };
    addr.trim().to_lowercase()
}

/// The unfolded values of every header field called `name` (lower case).
fn all_headers(head: &[u8], name: &[u8]) -> Vec<String> {
    fields(head)
        .into_iter()
        .filter_map(|f| {
            let colon = f.iter().position(|b| *b == b':')?;
            f[..colon]
                .trim_ascii()
                .eq_ignore_ascii_case(name)
                .then(|| String::from_utf8_lossy(&f[colon + 1..]).replace("\r\n", "").trim().to_string())
        })
        .collect()
}

/// Whether one of the message's encrypted session keys names `key` (or a subkey of it).
fn message_is_for(message: &Message<'_>, key: &SecretKey) -> bool {
    let Message::Encrypted { esk, .. } = message else { return false };
    let mut ids = vec![hex(key.key.primary_key.legacy_key_id())];
    ids.extend(key.key.secret_subkeys.iter().map(|s| hex(s.key.legacy_key_id())));
    esk.iter().any(|e| match e {
        pgp::composed::Esk::PublicKeyEncryptedSessionKey(p) => p.id().map(|id| ids.contains(&hex(id))).unwrap_or(true),
        _ => false,
    })
}

/// The key a message carries in its `Autocrypt` header (autocrypt.org, level 1): the address
/// it is for, lower case, and the key. The engine decides whether to keep it.
pub fn autocrypt(raw: &[u8]) -> Option<(String, PublicKey)> {
    let raw = crlf(raw);
    let (head, _) = split_entity(&raw);
    parse_autocrypt(&header(head, b"autocrypt")?)
}

/// The address and key of an `Autocrypt` (or `Autocrypt-Gossip`) header value; `None` when
/// the key is not for that address.
fn parse_autocrypt(value: &str) -> Option<(String, PublicKey)> {
    let mut addr = None;
    let mut keydata = None;
    for part in value.split(';') {
        let (k, v) = part.split_once('=')?;
        match k.trim().to_ascii_lowercase().as_str() {
            "addr" => addr = Some(v.trim().to_lowercase()),
            "keydata" => keydata = Some(v.split_whitespace().collect::<String>()),
            "prefer-encrypt" => {}
            // Unknown critical attributes make the header void.
            other if !other.starts_with('_') => return None,
            _ => {}
        }
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(keydata?).ok()?;
    let key = PublicKey::from_bytes(&bytes)?;
    let addr = addr?;
    key.info.covers(&addr).then_some((addr, key))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALAIN: &[u8] = include_bytes!("../testdata/alain-secret.asc");
    const JIJ_PUBLIC: &[u8] = include_bytes!("../testdata/jij-public.asc");

    fn alain() -> SecretKey {
        SecretKey::import(ALAIN, "geheim").unwrap()
    }

    fn jij() -> PublicKey {
        PublicKey::from_bytes(JIJ_PUBLIC).unwrap()
    }

    #[test]
    fn imports_a_key_with_its_passphrase_and_keeps_it() {
        assert_eq!(SecretKey::import(ALAIN, "fout").unwrap_err(), PgpError::Passphrase);
        assert_eq!(SecretKey::import(b"geen sleutel", "geheim").unwrap_err(), PgpError::NotAKey);
        let key = alain();
        assert_eq!(key.info().name, "Alain Tester");
        assert_eq!(key.info().emails, vec!["alain@voorbeeld.nl".to_string()]);
        assert_eq!(key.info().fingerprint.len(), 40);
        assert!(!format!("{key:?}").contains("secret_params"), "the key never prints");
        let again = SecretKey::from_stored(&key.to_stored()).unwrap();
        assert_eq!(again.info(), key.info());
        // Stored unlocked: it opens mail without the passphrase.
        assert!(decrypt_message(&again, include_bytes!("../testdata/encrypted-gpg.eml"), &[]).is_ok());
        let rsa = SecretKey::import(include_bytes!("../testdata/rsa-secret.asc"), "geheim").unwrap();
        assert_eq!(rsa.info().emails, vec!["rsa@example.org".to_string()]);
    }

    #[test]
    fn a_new_key_is_made_that_signs_encrypts_and_is_kept() {
        let key = SecretKey::generate("Alain <Rees>", "alain@voorbeeld.nl").unwrap();
        assert_eq!(key.info().name, "Alain Rees");
        assert_eq!(key.info().emails, vec!["alain@voorbeeld.nl".to_string()]);
        assert!(key.public().can_encrypt(0x7000_0000), "no expiry");
        let again = SecretKey::from_stored(&key.to_stored()).unwrap();
        assert_eq!(again.info(), key.info());
        let other = SecretKey::generate("", "jij@example.org").unwrap();
        assert_eq!(other.info().emails, vec!["jij@example.org".to_string()]);
        assert_ne!(other.info().fingerprint, key.info().fingerprint);

        // A message encrypted to the new key opens with it, and its signature checks out.
        let raw = b"From: jij@example.org\r\nTo: alain@voorbeeld.nl\r\nSubject: Hoi\r\nContent-Type: text/plain\r\n\r\nTot straks\r\n";
        let sent = encrypt_message(&other, raw, &[key.public()], true, 1_800_000_000).unwrap();
        let (plain, verdict) = decrypt_message(&key, &sent, &[other.public()]).unwrap();
        assert!(plain.windows(10).any(|w| w == b"Tot straks"));
        assert!(matches!(verdict, Some(Verdict::Valid { .. })), "{verdict:?}");
    }

    #[test]
    fn checks_a_pgp_mime_signature_made_by_gnupg() {
        let signed = include_bytes!("../testdata/signed-gpg.eml");
        match check_message(signed, &[jij()]) {
            Some(Verdict::Valid { signer }) => assert_eq!(signer.emails, vec!["jij@example.org".to_string()]),
            other => panic!("{other:?}"),
        }
        assert!(matches!(check_message(signed, &[]), Some(Verdict::UnknownKey(id)) if !id.is_empty()));
        assert_eq!(check_message(include_bytes!("../testdata/signed-gpg-tampered.eml"), &[jij()]), Some(Verdict::Broken));
        let rsa = PublicKey::from_bytes(include_bytes!("../testdata/rsa-public.asc")).unwrap();
        assert!(matches!(check_message(include_bytes!("../testdata/signed-rsa.eml"), &[jij(), rsa]), Some(Verdict::Valid { .. })));
        assert_eq!(check_message(b"From: a@b.nl\r\nContent-Type: text/plain\r\n\r\nHoi\r\n", &[]), None);
    }

    #[test]
    fn opens_mail_gnupg_encrypted_and_signed() {
        let raw = include_bytes!("../testdata/encrypted-gpg.eml");
        assert!(is_encrypted(raw));
        assert!(!is_encrypted(include_bytes!("../testdata/signed-gpg.eml")));
        let (plain, verdict) = decrypt_message(&alain(), raw, &[jij()]).unwrap();
        let text = String::from_utf8(plain.to_vec()).unwrap();
        assert!(text.starts_with("From: Jij Tester <jij@example.org>\r\n"), "{text}");
        assert!(text.contains("Het contract staat klaar."), "{text}");
        assert!(!text.contains("multipart/encrypted"));
        assert!(matches!(verdict, Some(Verdict::Valid { signer }) if signer.covers("jij@example.org")));
        assert!(matches!(decrypt_message(&alain(), raw, &[]).unwrap().1, Some(Verdict::UnknownKey(_))));
        let rsa = SecretKey::import(include_bytes!("../testdata/rsa-secret.asc"), "geheim").unwrap();
        assert_eq!(decrypt_message(&rsa, raw, &[]).unwrap_err(), PgpError::NotForMe);
        let (plain, verdict) = decrypt_message(&rsa, include_bytes!("../testdata/encrypted-rsa.eml"), &[]).unwrap();
        assert!(String::from_utf8_lossy(&plain).contains("Het contract staat klaar.") && verdict.is_none());
    }

    #[test]
    fn reads_the_key_in_an_autocrypt_header() {
        let key = base64::engine::general_purpose::STANDARD.encode(jij().to_bytes());
        let folded: String = key.as_bytes().chunks(70).map(|c| format!(" {}\r\n", String::from_utf8_lossy(c))).collect();
        let raw = format!(
            "From: jij@example.org\r\nAutocrypt: addr=Jij@example.org; prefer-encrypt=mutual; keydata=\r\n{folded}Subject: Hoi\r\n\r\nHallo\r\n"
        );
        let (addr, found) = autocrypt(raw.as_bytes()).unwrap();
        assert_eq!(addr, "jij@example.org");
        assert_eq!(found, jij());
        // For another address than the key's, or with an unknown attribute, the header is void.
        assert!(autocrypt(raw.replace("addr=Jij@", "addr=bank@").as_bytes()).is_none());
        assert!(autocrypt(raw.replace("prefer-encrypt", "critical").as_bytes()).is_none());
        assert!(autocrypt(b"From: a@b.nl\r\n\r\nHoi").is_none());
    }

    #[test]
    fn signs_and_encrypts_mail_that_reads_back() {
        const NOW: i64 = 1_798_761_600;
        let raw = b"From: Alain Tester <alain@voorbeeld.nl>\r\nTo: jij@example.org\r\nSubject: Contract\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHet contract staat klaar.\r\n";
        let me = alain();
        let signed = sign_message(&me, raw, NOW).unwrap();
        let text = String::from_utf8_lossy(&signed);
        assert!(text.starts_with("From: Alain Tester"), "{text}");
        assert!(text.contains("multipart/signed; protocol=\"application/pgp-signature\"; micalg=pgp-sha256"));
        assert!(matches!(check_message(&signed, &[me.public()]), Some(Verdict::Valid { signer }) if signer.covers("alain@voorbeeld.nl")));
        let changed = text.replace("staat klaar", "staat niet klaar");
        assert_eq!(check_message(changed.as_bytes(), &[me.public()]), Some(Verdict::Broken));

        assert!(jij().can_encrypt(NOW));
        let encrypted = encrypt_message(&me, raw, &[jij()], true, NOW).unwrap();
        assert!(is_encrypted(&encrypted));
        assert!(!String::from_utf8_lossy(&encrypted).contains("contract staat"));
        // The sender's own copy opens, and so does the recipient's.
        let (plain, verdict) = decrypt_message(&me, &encrypted, &[me.public()]).unwrap();
        assert!(String::from_utf8_lossy(&plain).contains("Het contract staat klaar."));
        assert!(matches!(verdict, Some(Verdict::Valid { .. })));
        let jij_secret = SecretKey::import(include_bytes!("../testdata/jij-secret.asc"), "geheim").unwrap();
        assert!(decrypt_message(&jij_secret, &encrypted, &[]).is_ok());
        let unsigned = encrypt_message(&me, raw, &[jij()], false, NOW).unwrap();
        assert!(decrypt_message(&me, &unsigned, &[]).unwrap().1.is_none());

        let header = me.autocrypt_header("Alain@voorbeeld.nl");
        let with = with_header(raw, &header);
        let (addr, key) = autocrypt(&with).unwrap();
        assert_eq!((addr.as_str(), key), ("alain@voorbeeld.nl", me.public()));
    }

    #[test]
    fn encrypted_mail_carries_its_header_inside_and_the_secret_fields_only_there() {
        const NOW: i64 = 1_798_761_600;
        let me = alain();
        let gossip = jij().to_bytes();
        let gossip = base64::engine::general_purpose::STANDARD.encode(gossip);
        let raw = format!(
            "From: Alain Tester <alain@voorbeeld.nl>\r\nTo: jij@example.org\r\nSubject: Chatbericht\r\nChat-Version: 1.0\r\n\
             Secure-Join: vc-request-with-auth\r\nSecure-Join-Auth: geheimtoken\r\nAutocrypt-Gossip: addr=jij@example.org; keydata={gossip}\r\n\
             Content-Type: text/plain; charset=utf-8\r\n\r\nHoi\r\n"
        );
        let raw = with_header(raw.as_bytes(), &me.autocrypt_header("alain@voorbeeld.nl"));
        let encrypted = encrypt_message(&me, &raw, &[jij()], true, NOW).unwrap();
        let outside = String::from_utf8_lossy(&encrypted);
        assert!(!outside.contains("geheimtoken") && !outside.contains("Secure-Join") && !outside.contains("Autocrypt-Gossip"), "{outside}");
        assert!(outside.contains("Chat-Version: 1.0") && outside.contains("Autocrypt: addr=alain@voorbeeld.nl"));

        // The first message from someone checks out with the key it brings along.
        let jij_secret = SecretKey::import(include_bytes!("../testdata/jij-secret.asc"), "geheim").unwrap();
        let opened = open_message(&jij_secret, &encrypted, &[]).unwrap();
        assert!(matches!(&opened.verdict, Some(Verdict::Valid { signer }) if signer.covers("alain@voorbeeld.nl")));
        assert_eq!(opened.autocrypt.as_ref().map(|(a, k)| (a.as_str(), k)), Some(("alain@voorbeeld.nl", &me.public())));
        assert_eq!(opened.gossip, vec![("jij@example.org".to_string(), jij())]);
        let plain = String::from_utf8_lossy(&opened.plain);
        assert!(plain.contains("Secure-Join-Auth: geheimtoken") && plain.contains("protected-headers=\"v1\""), "{plain}");
        assert_eq!(plain.matches("Subject:").count(), 1, "{plain}");
    }

    #[test]
    fn the_protected_header_inside_wins_but_not_a_from_with_another_address() {
        let outer = b"From: alain@voorbeeld.nl\r\nSubject: [...]\r\nDate: Mon, 1 Jan 2024 00:00:00 +0000\r\n";
        let data = b"From: Alain <alain@voorbeeld.nl>\r\nSubject: Echt\r\nHP-Outer: Subject: [...]\r\nContent-Type: text/plain\r\n\r\nHoi\r\n";
        let plain = String::from_utf8_lossy(&protected(outer, data)).into_owned();
        assert!(plain.contains("From: Alain <alain@voorbeeld.nl>") && plain.contains("Subject: Echt") && plain.contains("Date: Mon"), "{plain}");
        assert!(!plain.contains("[...]") && plain.matches("From:").count() == 1, "{plain}");
        let forged = b"From: Bank <bank@example.org>\r\nContent-Type: text/plain\r\n\r\nHoi\r\n";
        let plain = String::from_utf8_lossy(&protected(outer, forged)).into_owned();
        assert!(plain.contains("From: alain@voorbeeld.nl") && !plain.contains("bank@"), "{plain}");
    }
}
