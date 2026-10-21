//! Mail that goes out with OpenPGP (RFC 3156): signed as `multipart/signed` with a detached
//! signature, or signed and encrypted together (the combined method) as `multipart/encrypted`
//! for the recipients and the sender, and the sender's key in an `Autocrypt` header so the
//! other side can answer encrypted.

use base64::Engine as _;
use merel_smime::raw::split_message;
use pgp::composed::{ArmorOptions, DetachedSignature, MessageBuilder, SignedPublicKey};
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::sym::SymmetricKeyAlgorithm;
use pgp::packet::Signature;
use pgp::types::{KeyDetails, Password, SigningKey};
use rand::rngs::OsRng;

use crate::{PgpError, PublicKey, SecretKey};

/// Whether the key's latest binding signature in `signatures` allows `wanted`, and the key
/// (created at `created`) has not run out at `now`.
fn usable(signatures: &[Signature], created: u32, now: i64, wanted: impl Fn(&Signature) -> bool) -> bool {
    let Some(sig) = signatures.iter().max_by_key(|s| s.created().map(|t| t.as_secs()).unwrap_or(0)) else { return false };
    let alive = match sig.key_expiration_time().map(|d| d.as_secs()) {
        None | Some(0) => true,
        Some(secs) => now < i64::from(created) + i64::from(secs),
    };
    alive && wanted(sig)
}

/// The subkeys of `key` mail can be encrypted to at `now`.
fn encryption_keys(key: &SignedPublicKey, now: i64) -> Vec<&pgp::composed::SignedPublicSubKey> {
    key.public_subkeys
        .iter()
        .filter(|s| {
            s.key.algorithm().can_encrypt()
                && usable(&s.signatures, s.key.created_at().as_secs(), now, |sig| {
                    let flags = sig.key_flags();
                    flags.encrypt_comms() || flags.encrypt_storage()
                })
        })
        .collect()
}

impl PublicKey {
    /// Whether mail can be encrypted to this key at `now` (seconds since the Unix epoch).
    pub fn can_encrypt(&self, now: i64) -> bool {
        !encryption_keys(&self.key, now).is_empty()
    }
}

impl SecretKey {
    /// The part of the key that signs: a signing subkey, else the primary key.
    fn signing_key(&self, now: i64) -> &dyn SigningKey {
        let sub = self
            .key
            .secret_subkeys
            .iter()
            .find(|s| s.key.algorithm().can_sign() && usable(&s.signatures, s.key.created_at().as_secs(), now, |sig| sig.key_flags().sign()));
        match sub {
            Some(s) => &s.key,
            None => &self.key.primary_key,
        }
    }

    /// The `Autocrypt` header field for mail from `addr`, folded, with its final line break.
    pub fn autocrypt_header(&self, addr: &str) -> String {
        let keydata = base64::engine::general_purpose::STANDARD.encode(self.public().to_bytes());
        let mut out = format!("Autocrypt: addr={}; keydata=", addr.trim().to_lowercase());
        for line in keydata.as_bytes().chunks(76) {
            out.push_str("\r\n ");
            out.push_str(&String::from_utf8_lossy(line));
        }
        out.push_str("\r\n");
        out
    }
}

fn boundary() -> String {
    use rand::RngCore as _;
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    format!("----=_pgp_{}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// `raw` signed with `key` as PGP/MIME: the message's content inside `multipart/signed`, with
/// the signature as the second part.
pub fn sign_message(key: &SecretKey, raw: &[u8], now: i64) -> Result<Vec<u8>, PgpError> {
    let (outer, inner) = split_message(raw);
    // The part ends with a line break that belongs to the boundary after it.
    let signed = &inner[..inner.len() - 2];
    let signature = DetachedSignature::sign_binary_data(OsRng, &Box::new(key.signing_key(now)), &Password::empty(), HashAlgorithm::Sha256, signed)
        .map_err(|_| PgpError::Sign)?;
    let armored = signature.to_armored_bytes(ArmorOptions::default()).map_err(|_| PgpError::Sign)?;

    let boundary = boundary();
    let mut out = outer;
    out.extend_from_slice(b"MIME-Version: 1.0\r\n");
    out.extend_from_slice(
        format!("Content-Type: multipart/signed; protocol=\"application/pgp-signature\"; micalg=pgp-sha256;\r\n\tboundary=\"{boundary}\"\r\n\r\n")
            .as_bytes(),
    );
    out.extend_from_slice(b"This is an OpenPGP/MIME signed message (RFC 4880 and 3156).\r\n\r\n");
    out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    out.extend_from_slice(&inner);
    out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    out.extend_from_slice(b"Content-Type: application/pgp-signature; name=\"signature.asc\"\r\n");
    out.extend_from_slice(b"Content-Description: OpenPGP digital signature\r\n");
    out.extend_from_slice(b"Content-Disposition: attachment; filename=\"signature.asc\"\r\n\r\n");
    out.extend_from_slice(&merel_smime::raw::crlf(&armored));
    if !out.ends_with(b"\r\n") {
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Ok(out)
}

/// `raw` encrypted as PGP/MIME for `recipients` and the sender's own `key` (so the sent copy
/// stays readable), and signed inside with `key` when `sign` is set. Every recipient needs a
/// key that can be encrypted to.
pub fn encrypt_message(key: &SecretKey, raw: &[u8], recipients: &[PublicKey], sign: bool, now: i64) -> Result<Vec<u8>, PgpError> {
    let (outer, inner) = split_message(raw);
    let own = key.public();
    let mut targets = Vec::new();
    for person in std::iter::once(&own).chain(recipients) {
        let subkeys = encryption_keys(&person.key, now);
        if subkeys.is_empty() {
            return Err(PgpError::NoEncryptionKey);
        }
        targets.extend(subkeys);
    }
    let mut builder = MessageBuilder::from_bytes("", inner).seipd_v1(OsRng, SymmetricKeyAlgorithm::AES256);
    for target in targets {
        builder.encrypt_to_key(OsRng, target).map_err(|_| PgpError::Encrypt)?;
    }
    if sign {
        builder.sign(key.signing_key(now), Password::empty(), HashAlgorithm::Sha256);
    }
    let armored = builder.to_armored_string(OsRng, ArmorOptions::default()).map_err(|_| PgpError::Encrypt)?;

    let boundary = boundary();
    let mut out = outer;
    out.extend_from_slice(b"MIME-Version: 1.0\r\n");
    out.extend_from_slice(
        format!("Content-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\";\r\n\tboundary=\"{boundary}\"\r\n\r\n").as_bytes(),
    );
    out.extend_from_slice(b"This is an OpenPGP/MIME encrypted message (RFC 4880 and 3156).\r\n\r\n");
    out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    out.extend_from_slice(b"Content-Type: application/pgp-encrypted\r\n");
    out.extend_from_slice(b"Content-Description: PGP/MIME version identification\r\n\r\n");
    out.extend_from_slice(b"Version: 1\r\n\r\n");
    out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    out.extend_from_slice(b"Content-Type: application/octet-stream; name=\"encrypted.asc\"\r\n");
    out.extend_from_slice(b"Content-Description: OpenPGP encrypted message\r\n");
    out.extend_from_slice(b"Content-Disposition: inline; filename=\"encrypted.asc\"\r\n\r\n");
    out.extend_from_slice(&merel_smime::raw::crlf(armored.as_bytes()));
    if !out.ends_with(b"\r\n") {
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Ok(out)
}

/// `raw` with `field` (a whole header field with its line break) added to its header.
pub fn with_header(raw: &[u8], field: &str) -> Vec<u8> {
    let raw = merel_smime::raw::crlf(raw);
    let mut out = field.as_bytes().to_vec();
    out.extend_from_slice(&raw);
    out
}
