//! Signed releases. The release workflow lists the SHA-256 of every installer in
//! `SHA256SUMS.txt` and signs that list with an Ed25519 key kept as a GitHub secret
//! (`SHA256SUMS.txt.sig`). The app carries the public key: before it installs an update it
//! checks the signature on the list and the downloaded file against its line in it, so a
//! file swapped on the way or on the release page is never opened.
//!
//! Keys and signatures are plain base64: the secret key as a PKCS#8 document, the public key
//! as its 32 bytes, the signature as its 64 bytes.

use aws_lc_rs::digest;
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{ED25519, Ed25519KeyPair, KeyPair as _, UnparsedPublicKey};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use zeroize::Zeroizing;

/// The checksum list and its signature, as the release names them.
pub const SUMS: &str = "SHA256SUMS.txt";
pub const SIGNATURE: &str = "SHA256SUMS.txt.sig";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SignError {
    #[error("the key is not a valid Ed25519 key")]
    BadKey,
    #[error("the signature is not readable")]
    BadSignature,
    #[error("the signature does not match the checksum list")]
    Forged,
    #[error("the checksum list has no line for {0}")]
    Unlisted(String),
    #[error("the file does not match its checksum")]
    Changed,
    #[error("making a key failed")]
    NoRandom,
}

/// A new key pair: the secret key (PKCS#8, base64) and the public key (base64).
pub fn keygen() -> Result<(Zeroizing<String>, String), SignError> {
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).map_err(|_| SignError::NoRandom)?;
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).map_err(|_| SignError::BadKey)?;
    Ok((Zeroizing::new(STANDARD.encode(pkcs8.as_ref())), STANDARD.encode(pair.public_key().as_ref())))
}

/// The public key that goes with a secret key, to put in the app.
pub fn public_key(secret: &str) -> Result<String, SignError> {
    Ok(STANDARD.encode(pair(secret)?.public_key().as_ref()))
}

fn pair(secret: &str) -> Result<Ed25519KeyPair, SignError> {
    let der = Zeroizing::new(STANDARD.decode(secret.trim()).map_err(|_| SignError::BadKey)?);
    Ed25519KeyPair::from_pkcs8(&der).map_err(|_| SignError::BadKey)
}

/// The signature (base64) of `message` under the secret key.
pub fn sign(secret: &str, message: &[u8]) -> Result<String, SignError> {
    Ok(STANDARD.encode(pair(secret)?.sign(message).as_ref()))
}

/// Whether `signature` is the public key's signature of `message`.
pub fn verify(public: &str, message: &[u8], signature: &str) -> Result<(), SignError> {
    let key = STANDARD.decode(public.trim()).map_err(|_| SignError::BadKey)?;
    if key.len() != 32 {
        return Err(SignError::BadKey);
    }
    let sig = STANDARD.decode(signature.trim()).map_err(|_| SignError::BadSignature)?;
    UnparsedPublicKey::new(&ED25519, key).verify(message, &sig).map_err(|_| SignError::Forged)
}

/// The SHA-256 (lowercase hex) the checksum list gives for `name`; the list is `sha256sum`
/// output, "<hex>  <name>" (or "<hex> *<name>") per line.
pub fn listed_hash(sums: &str, name: &str) -> Result<String, SignError> {
    sums.lines()
        .find_map(|line| {
            let (hash, file) = line.trim_end().split_once(' ')?;
            let file = file.trim_start_matches(' ').trim_start_matches('*');
            (file == name && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then(|| hash.to_ascii_lowercase())
        })
        .ok_or_else(|| SignError::Unlisted(name.to_owned()))
}

/// A running SHA-256, fed while a download comes in.
pub struct Hasher(digest::Context);

impl Default for Hasher {
    fn default() -> Self {
        Self(digest::Context::new(&digest::SHA256))
    }
}

impl Hasher {
    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    /// The hash as lowercase hex.
    pub fn finish(self) -> String {
        self.0.finish().as_ref().iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Checks a whole download: the list's signature, then the file's hash against its line.
pub fn check(public: &str, sums: &[u8], signature: &str, name: &str, file_hash: &str) -> Result<(), SignError> {
    verify(public, sums, signature)?;
    let sums = std::str::from_utf8(sums).map_err(|_| SignError::Forged)?;
    if listed_hash(sums, name)? == file_hash { Ok(()) } else { Err(SignError::Changed) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(bytes: &[u8]) -> String {
        let mut h = Hasher::default();
        h.update(bytes);
        h.finish()
    }

    #[test]
    fn a_signed_list_lets_only_the_listed_file_through() {
        let (secret, public) = keygen().unwrap();
        assert_eq!(public_key(&secret).unwrap(), public);
        let dmg = b"the installer";
        let sums = format!("{}  Merel-0.2.0-macos.dmg\n{}  Merel-0.2.0-windows-setup.exe\n", hash(dmg), hash(b"other"));
        let sig = sign(&secret, sums.as_bytes()).unwrap();
        assert_eq!(check(&public, sums.as_bytes(), &sig, "Merel-0.2.0-macos.dmg", &hash(dmg)), Ok(()));
        assert_eq!(check(&public, sums.as_bytes(), &sig, "Merel-0.2.0-macos.dmg", &hash(b"tampered")), Err(SignError::Changed));
        assert_eq!(
            check(&public, sums.as_bytes(), &sig, "Merel-0.2.0-linux-x86_64.AppImage", &hash(dmg)),
            Err(SignError::Unlisted("Merel-0.2.0-linux-x86_64.AppImage".into()))
        );
        // A list changed after signing, or signed with another key, is refused.
        let forged = sums.replace(&hash(dmg), &hash(b"evil"));
        assert_eq!(check(&public, forged.as_bytes(), &sig, "Merel-0.2.0-macos.dmg", &hash(b"evil")), Err(SignError::Forged));
        let (other, _) = keygen().unwrap();
        let other_sig = sign(&other, sums.as_bytes()).unwrap();
        assert_eq!(check(&public, sums.as_bytes(), &other_sig, "Merel-0.2.0-macos.dmg", &hash(dmg)), Err(SignError::Forged));
        assert_eq!(verify(&public, sums.as_bytes(), "not base64!"), Err(SignError::BadSignature));
        assert_eq!(verify("c2hvcnQ=", sums.as_bytes(), &sig), Err(SignError::BadKey));
    }

    #[test]
    fn the_list_is_read_as_sha256sum_writes_it() {
        let h = "a".repeat(64);
        assert_eq!(listed_hash(&format!("{h}  Merel.dmg\n"), "Merel.dmg").unwrap(), h);
        assert_eq!(listed_hash(&format!("{h} *Merel.dmg\r\n"), "Merel.dmg").unwrap(), h);
        assert!(listed_hash(&format!("{h}  Merel.dmg.evil\n"), "Merel.dmg").is_err());
        assert!(listed_hash("abc  Merel.dmg\n", "Merel.dmg").is_err());
        assert_eq!(hash(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }
}
