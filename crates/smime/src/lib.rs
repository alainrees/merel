//! S/MIME (RFC 8551): the person's own certificate, imported from a PKCS#12 file, and mail
//! signed with it.
//!
//! The CMS structures come from the RustCrypto `cms`, `der` and `x509-cert` crates, which only
//! encode and decode. Every cryptographic operation (hashing, RSA signatures) goes through
//! aws-lc-rs, the same library behind TLS and the offline cache. The private key lives in an
//! [`Identity`], which wipes it when dropped and never shows it in `Debug` output; it only
//! leaves this crate as [`Identity::to_stored`], for the engine to seal on disk.

use std::fmt;

mod envelope;

pub use envelope::{can_encrypt_to, decrypt_message, encrypt_message, is_encrypted};

use aws_lc_rs::digest;
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::KeyPair;
use aws_lc_rs::signature::{self, KeyPair as _, UnparsedPublicKey};
use base64::Engine as _;
use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::content_info::{CmsVersion, ContentInfo};
use cms::signed_data::{CertificateSet, EncapsulatedContentInfo, SignatureValue, SignedData, SignerIdentifier, SignerInfo, SignerInfos};
use der::asn1::{Null, ObjectIdentifier, OctetString, SetOfVec, UtcTime};
use der::{Any, Decode, Encode};
use x509_cert::Certificate;
use x509_cert::attr::{Attribute, AttributeValue};
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::ext::pkix::{ExtendedKeyUsage, KeyUsage, SubjectAltName};
use x509_cert::spki::AlgorithmIdentifierOwned;
use zeroize::Zeroizing;

const ID_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1");
const ID_SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");
const ID_CONTENT_TYPE: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.3");
const ID_MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
const ID_SIGNING_TIME: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.5");
const ID_SMIME_CAPABILITIES: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.15");
const ID_EMAIL_ADDRESS: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.1");
const RSA_ENCRYPTION: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const SHA256_WITH_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.11");
const ID_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
const AES256_CBC: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.1.42");
const AES128_CBC: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.1.2");
const EMAIL_PROTECTION: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.4");
const ANY_EXTENDED_KEY_USAGE: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.29.37.0");

/// Version tag of [`Identity::to_stored`].
const STORED_MAGIC: &[u8; 4] = b"MCS1";
/// Most certificates kept with an identity: the person's own plus the issuers above it.
const MAX_CHAIN: usize = 8;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SmimeError {
    #[error("Het wachtwoord klopt niet, of dit is geen PKCS#12-bestand (.p12 of .pfx).")]
    Pkcs12,
    #[error("Dit bestand bevat geen privésleutel met certificaat.")]
    NoKey,
    #[error("Mailcraft ondersteunt voorlopig alleen certificaten met een RSA-sleutel.")]
    KeyType,
    #[error("De privésleutel hoort niet bij het certificaat.")]
    Mismatch,
    #[error("Dit certificaat is niet bedoeld voor e-mail.")]
    Usage,
    #[error("Dit certificaat is nog niet geldig.")]
    NotYetValid,
    #[error("Dit certificaat is verlopen.")]
    Expired,
    #[error("Het certificaat kan niet worden gelezen.")]
    Certificate,
    #[error("Het bericht kan niet worden ondertekend.")]
    Sign,
    #[error("Het certificaat van een ontvanger is niet geschikt om naar te versleutelen.")]
    RecipientKey,
    #[error("Het bericht kan niet worden versleuteld.")]
    Encrypt,
    #[error("Het bericht is niet voor jouw certificaat versleuteld.")]
    NotForMe,
    #[error("Dit bericht is versleuteld, maar {0}.")]
    Decrypt(String),
}

/// What a person sees of a certificate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertInfo {
    /// The subject's common name, or its first e-mail address.
    pub name: String,
    /// The addresses the certificate is issued for, in lower case.
    pub emails: Vec<String>,
    /// The issuer's common name.
    pub issuer: String,
    /// Seconds since the Unix epoch.
    pub not_before: i64,
    pub not_after: i64,
}

impl CertInfo {
    /// Whether the certificate names `email`.
    pub fn covers(&self, email: &str) -> bool {
        let email = email.trim().to_lowercase();
        self.emails.contains(&email)
    }
}

/// The person's own certificate with its private key, and the issuers' certificates that
/// came with it.
pub struct Identity {
    /// PKCS#8 DER.
    key: Zeroizing<Vec<u8>>,
    /// DER, the person's own first.
    certs: Vec<Vec<u8>>,
    info: CertInfo,
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity").field("info", &self.info).field("key", &"…").finish()
    }
}

impl Identity {
    /// Reads a PKCS#12 file (.p12, .pfx) as browsers, Windows, macOS and certificate
    /// authorities export it, and checks that its certificate may sign mail at `now`
    /// (seconds since the Unix epoch).
    pub fn from_pkcs12(bytes: &[u8], password: &str, now: i64) -> Result<Identity, SmimeError> {
        let store =
            p12_keystore::KeyStore::from_pkcs12(bytes, password, p12_keystore::Pkcs12ImportPolicy::Relaxed).map_err(|_| SmimeError::Pkcs12)?;
        let (_, chain) = store.private_key_chain().ok_or(SmimeError::NoKey)?;
        let key = Zeroizing::new(chain.key().as_der().to_vec());
        let certs: Vec<Vec<u8>> = chain.certs().iter().take(MAX_CHAIN).map(|c| c.as_der().to_vec()).collect();
        if certs.is_empty() {
            return Err(SmimeError::NoKey);
        }
        let identity = Identity::assemble(key, certs)?;
        identity.check_usable(now)?;
        Ok(identity)
    }

    fn assemble(key: Zeroizing<Vec<u8>>, certs: Vec<Vec<u8>>) -> Result<Identity, SmimeError> {
        let own = certs.first().ok_or(SmimeError::NoKey)?;
        let cert = Certificate::from_der(own).map_err(|_| SmimeError::Certificate)?;
        let spki = cert.tbs_certificate().subject_public_key_info();
        if spki.algorithm.oid != RSA_ENCRYPTION {
            return Err(SmimeError::KeyType);
        }
        let pair = KeyPair::from_pkcs8(&key).map_err(|_| SmimeError::KeyType)?;
        if pair.public_key().as_ref() != spki.subject_public_key.raw_bytes() {
            return Err(SmimeError::Mismatch);
        }
        let info = cert_info(&cert)?;
        Ok(Identity { key, certs, info })
    }

    /// Refuses a certificate that is not valid at `now` or not meant for signing mail.
    pub fn check_usable(&self, now: i64) -> Result<(), SmimeError> {
        if now < self.info.not_before {
            return Err(SmimeError::NotYetValid);
        }
        if now > self.info.not_after {
            return Err(SmimeError::Expired);
        }
        let cert = self.certificate()?;
        let tbs = cert.tbs_certificate();
        if let Ok(Some((_, usage))) = tbs.get_extension::<KeyUsage>()
            && !(usage.digital_signature() || usage.non_repudiation())
        {
            return Err(SmimeError::Usage);
        }
        if let Ok(Some((_, eku))) = tbs.get_extension::<ExtendedKeyUsage>()
            && !eku.0.iter().any(|o| *o == EMAIL_PROTECTION || *o == ANY_EXTENDED_KEY_USAGE)
        {
            return Err(SmimeError::Usage);
        }
        Ok(())
    }

    pub fn info(&self) -> &CertInfo {
        &self.info
    }

    /// The person's own certificate, DER: what others encrypt to.
    pub fn certificate_der(&self) -> &[u8] {
        self.certs.first().map(Vec::as_slice).unwrap_or_default()
    }

    fn certificate(&self) -> Result<Certificate, SmimeError> {
        Certificate::from_der(self.certs.first().ok_or(SmimeError::Certificate)?).map_err(|_| SmimeError::Certificate)
    }

    /// The key and certificates in a compact form, for the engine to keep sealed. The result
    /// holds the private key in the clear and is wiped when dropped.
    pub fn to_stored(&self) -> Zeroizing<Vec<u8>> {
        let mut out = Zeroizing::new(STORED_MAGIC.to_vec());
        for part in std::iter::once(self.key.as_slice()).chain(self.certs.iter().map(Vec::as_slice)) {
            out.extend_from_slice(&(part.len() as u32).to_be_bytes());
            out.extend_from_slice(part);
        }
        out
    }

    /// The identity [`Identity::to_stored`] wrote.
    pub fn from_stored(bytes: &[u8]) -> Result<Identity, SmimeError> {
        let mut rest = bytes.strip_prefix(STORED_MAGIC.as_slice()).ok_or(SmimeError::Certificate)?;
        let mut parts = Vec::new();
        while !rest.is_empty() {
            let (len, tail) = rest.split_first_chunk::<4>().ok_or(SmimeError::Certificate)?;
            let len = u32::from_be_bytes(*len) as usize;
            let part = tail.get(..len).ok_or(SmimeError::Certificate)?;
            parts.push(part.to_vec());
            rest = tail.get(len..).ok_or(SmimeError::Certificate)?;
            if parts.len() > MAX_CHAIN + 1 {
                return Err(SmimeError::Certificate);
            }
        }
        if parts.len() < 2 {
            return Err(SmimeError::Certificate);
        }
        let key = Zeroizing::new(parts.remove(0));
        Identity::assemble(key, parts)
    }

    /// A detached CMS SignedData over `content` (RFC 5652), DER-encoded: SHA-256 with RSA,
    /// the certificates enclosed so the reader can check the signature and answer encrypted.
    pub fn sign_detached(&self, content: &[u8], now: i64) -> Result<Vec<u8>, SmimeError> {
        let cert = self.certificate()?;
        let sha256 = AlgorithmIdentifierOwned { oid: ID_SHA256, parameters: None };
        let digest = digest::digest(&digest::SHA256, content);
        let time = UtcTime::from_unix_duration(std::time::Duration::from_secs(u64::try_from(now).map_err(|_| SmimeError::Sign)?))
            .map_err(|_| SmimeError::Sign)?;
        let attrs = vec![
            attribute(ID_CONTENT_TYPE, Any::encode_from(&ID_DATA))?,
            attribute(ID_SIGNING_TIME, Any::encode_from(&time))?,
            attribute(ID_MESSAGE_DIGEST, Any::encode_from(&OctetString::new(digest.as_ref()).map_err(|_| SmimeError::Sign)?))?,
            attribute(ID_SMIME_CAPABILITIES, smime_capabilities())?,
        ];
        let attrs: SetOfVec<Attribute> = SetOfVec::try_from(attrs).map_err(|_| SmimeError::Sign)?;
        // The signature covers the attributes as a DER SET OF, not as the [0] they travel in.
        let signed = attrs.to_der().map_err(|_| SmimeError::Sign)?;
        let pair = KeyPair::from_pkcs8(&self.key).map_err(|_| SmimeError::Sign)?;
        let mut sig = vec![0u8; pair.public_modulus_len()];
        pair.sign(&signature::RSA_PKCS1_SHA256, &SystemRandom::new(), &signed, &mut sig).map_err(|_| SmimeError::Sign)?;

        let tbs = cert.tbs_certificate();
        let signer = SignerInfo {
            version: CmsVersion::V1,
            sid: SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
                issuer: tbs.issuer().clone(),
                serial_number: tbs.serial_number().clone(),
            }),
            digest_alg: sha256.clone(),
            signed_attrs: Some(attrs),
            signature_algorithm: AlgorithmIdentifierOwned {
                oid: RSA_ENCRYPTION,
                parameters: Some(Any::encode_from(&Null).map_err(|_| SmimeError::Sign)?),
            },
            signature: SignatureValue::new(sig).map_err(|_| SmimeError::Sign)?,
            unsigned_attrs: None,
        };
        let mut certificates = Vec::new();
        for der in &self.certs {
            certificates.push(CertificateChoices::Certificate(Certificate::from_der(der).map_err(|_| SmimeError::Certificate)?));
        }
        let data = SignedData {
            version: CmsVersion::V1,
            digest_algorithms: SetOfVec::try_from(vec![sha256]).map_err(|_| SmimeError::Sign)?,
            encap_content_info: EncapsulatedContentInfo { econtent_type: ID_DATA, econtent: None },
            certificates: Some(CertificateSet::try_from(certificates).map_err(|_| SmimeError::Sign)?),
            crls: None,
            signer_infos: SignerInfos::try_from(vec![signer]).map_err(|_| SmimeError::Sign)?,
        };
        let info = ContentInfo { content_type: ID_SIGNED_DATA, content: Any::encode_from(&data).map_err(|_| SmimeError::Sign)? };
        info.to_der().map_err(|_| SmimeError::Sign)
    }
}

fn attribute(oid: ObjectIdentifier, value: der::Result<Any>) -> Result<Attribute, SmimeError> {
    let value: AttributeValue = value.map_err(|_| SmimeError::Sign)?;
    Ok(Attribute { oid, values: SetOfVec::try_from(vec![value]).map_err(|_| SmimeError::Sign)? })
}

/// SMIMECapabilities (RFC 8551 section 2.5.2): the ciphers a reply may be encrypted with,
/// strongest first.
fn smime_capabilities() -> der::Result<Any> {
    #[derive(der::Sequence)]
    struct Capability {
        oid: ObjectIdentifier,
    }
    let list = vec![Capability { oid: AES256_CBC }, Capability { oid: AES128_CBC }];
    Any::encode_from(&list)
}

/// What a person sees of `cert`.
fn cert_info(cert: &Certificate) -> Result<CertInfo, SmimeError> {
    let tbs = cert.tbs_certificate();
    let mut emails = Vec::new();
    if let Ok(Some((_, san))) = tbs.get_extension::<SubjectAltName>() {
        for name in san.0 {
            if let GeneralName::Rfc822Name(e) = name {
                emails.push(e.to_string().to_lowercase());
            }
        }
    }
    for atv in tbs.subject().iter().filter(|a| a.oid == ID_EMAIL_ADDRESS) {
        if let Ok(s) = der::asn1::Ia5StringRef::try_from(&atv.value) {
            let e = s.to_string().to_lowercase();
            if !emails.contains(&e) {
                emails.push(e);
            }
        }
    }
    let name = common_name(tbs.subject()).or_else(|| emails.first().cloned()).unwrap_or_default();
    let issuer = common_name(tbs.issuer()).unwrap_or_default();
    let validity = tbs.validity();
    let secs = |t: x509_cert::time::Time| i64::try_from(t.to_unix_duration().as_secs()).unwrap_or(i64::MAX);
    Ok(CertInfo { name, emails, issuer, not_before: secs(validity.not_before), not_after: secs(validity.not_after) })
}

fn common_name(name: &x509_cert::name::Name) -> Option<String> {
    use x509_cert::ext::pkix::name::DirectoryString;
    let s = match name.common_name().ok()?? {
        DirectoryString::PrintableString(s) => s.to_string(),
        DirectoryString::TeletexString(s) => s.to_string(),
        DirectoryString::Utf8String(s) => s,
        DirectoryString::BmpString(s) => s.to_string(),
    };
    let s: String = s.chars().filter(|c| !c.is_control()).collect();
    (!s.trim().is_empty()).then(|| s.trim().to_string())
}

/// What a signature on a message says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Signature {
    /// The signature matches: the message is as the holder of `signer` sent it. Whether the
    /// issuer of the certificate is to be trusted is not checked yet.
    Valid {
        signer: CertInfo,
        /// The signer's certificate, DER: what an encrypted answer would need.
        certificate: Vec<u8>,
    },
    /// It does not match: the message changed after it was signed, or the signature is false.
    Broken,
    /// Signed in a way Mailcraft cannot check; the reason, in Dutch.
    Unchecked(String),
}

const ID_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.2");
const ID_SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.3");
const ID_SHA1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.14.3.2.26");
const SHA384_WITH_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.12");
const SHA512_WITH_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.13");
const EC_PUBLIC_KEY: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");
const SECP256R1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.3.1.7");
const SECP384R1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.132.0.34");
const ECDSA_WITH_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");
const ECDSA_WITH_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.3");

fn unchecked(reason: &str) -> Signature {
    Signature::Unchecked(reason.to_string())
}

/// Checks a detached signature `p7s` (CMS SignedData, RFC 5652) over `content`. Only the
/// signature itself is checked, not whether the certificate's issuer is trusted.
pub fn verify_detached(content: &[u8], p7s: &[u8]) -> Signature {
    let Ok(info) = ContentInfo::from_der(p7s) else { return unchecked("de handtekening kan niet worden gelezen") };
    if info.content_type != ID_SIGNED_DATA {
        return unchecked("de handtekening kan niet worden gelezen");
    }
    let Ok(data) = info.content.decode_as::<SignedData>() else { return unchecked("de handtekening kan niet worden gelezen") };
    let certs: Vec<&Certificate> = data
        .certificates
        .as_ref()
        .map(|set| {
            set.0
                .iter()
                .filter_map(|c| match c {
                    CertificateChoices::Certificate(c) => Some(c),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let Some(signer) = data.signer_infos.0.iter().next() else { return unchecked("er staat geen ondertekenaar in") };
    let found = certs.iter().find(|c| match &signer.sid {
        SignerIdentifier::IssuerAndSerialNumber(isn) => {
            c.tbs_certificate().issuer() == &isn.issuer && c.tbs_certificate().serial_number() == &isn.serial_number
        }
        SignerIdentifier::SubjectKeyIdentifier(ski) => {
            matches!(c.tbs_certificate().get_extension::<x509_cert::ext::pkix::SubjectKeyIdentifier>(), Ok(Some((_, own))) if own == *ski)
        }
    });
    let Some(cert) = found else { return unchecked("het certificaat van de afzender zit er niet bij") };
    let (hash, sha) = match signer.digest_alg.oid {
        o if o == ID_SHA256 => (&digest::SHA256, 256),
        o if o == ID_SHA384 => (&digest::SHA384, 384),
        o if o == ID_SHA512 => (&digest::SHA512, 512),
        o if o == ID_SHA1 => return unchecked("de afzender gebruikt het verouderde SHA-1"),
        _ => return unchecked("de afzender gebruikt een onbekend algoritme"),
    };
    let digest = digest::digest(hash, content);
    // With signed attributes the signature covers them, and they carry the content's digest;
    // without, it covers the content itself.
    let signed = match &signer.signed_attrs {
        Some(attrs) => {
            let claimed = attrs
                .iter()
                .find(|a| a.oid == ID_MESSAGE_DIGEST)
                .and_then(|a| a.values.iter().next())
                .and_then(|v| v.decode_as::<OctetString>().ok());
            match claimed {
                Some(c) if c.as_bytes() == digest.as_ref() => {}
                Some(_) => return Signature::Broken,
                None => return unchecked("de handtekening kan niet worden gelezen"),
            }
            match attrs.to_der() {
                Ok(der) => der,
                Err(_) => return unchecked("de handtekening kan niet worden gelezen"),
            }
        }
        None => content.to_vec(),
    };
    let spki = cert.tbs_certificate().subject_public_key_info();
    let key = spki.subject_public_key.raw_bytes();
    let sig_alg = signer.signature_algorithm.oid;
    let ok = if spki.algorithm.oid == RSA_ENCRYPTION {
        let algorithm: &dyn signature::VerificationAlgorithm = match sha {
            256 => &signature::RSA_PKCS1_2048_8192_SHA256,
            384 => &signature::RSA_PKCS1_2048_8192_SHA384,
            _ => &signature::RSA_PKCS1_2048_8192_SHA512,
        };
        if ![RSA_ENCRYPTION, SHA256_WITH_RSA, SHA384_WITH_RSA, SHA512_WITH_RSA].contains(&sig_alg) {
            return unchecked("de afzender gebruikt een onbekend algoritme");
        }
        UnparsedPublicKey::new(algorithm, key).verify(&signed, signer.signature.as_bytes()).is_ok()
    } else if spki.algorithm.oid == EC_PUBLIC_KEY {
        let curve = spki.algorithm.parameters.as_ref().and_then(|p| p.decode_as::<ObjectIdentifier>().ok());
        let algorithm: &dyn signature::VerificationAlgorithm = match (curve, sha) {
            (Some(c), 256) if c == SECP256R1 => &signature::ECDSA_P256_SHA256_ASN1,
            (Some(c), 384) if c == SECP256R1 => &signature::ECDSA_P256_SHA384_ASN1,
            (Some(c), 256) if c == SECP384R1 => &signature::ECDSA_P384_SHA256_ASN1,
            (Some(c), 384) if c == SECP384R1 => &signature::ECDSA_P384_SHA384_ASN1,
            _ => return unchecked("de afzender gebruikt een onbekend algoritme"),
        };
        if ![EC_PUBLIC_KEY, ECDSA_WITH_SHA256, ECDSA_WITH_SHA384].contains(&sig_alg) {
            return unchecked("de afzender gebruikt een onbekend algoritme");
        }
        UnparsedPublicKey::new(algorithm, key).verify(&signed, signer.signature.as_bytes()).is_ok()
    } else {
        return unchecked("de afzender gebruikt een onbekend soort sleutel");
    };
    if !ok {
        return Signature::Broken;
    }
    match (cert_info(cert), cert.to_der()) {
        (Ok(signer), Ok(certificate)) => Signature::Valid { signer, certificate },
        _ => unchecked("het certificaat van de afzender kan niet worden gelezen"),
    }
}

/// The signature on `raw`, when the message is signed the usual way (RFC 8551 section 3.5,
/// `multipart/signed` at the top); `None` for a message that is not.
pub fn check_message(raw: &[u8]) -> Option<Signature> {
    let raw = crlf(raw);
    let (head, body) = split_entity(&raw);
    let kind = header(head, b"content-type")?;
    let lower = kind.to_ascii_lowercase();
    if !lower.starts_with("multipart/signed") {
        return None;
    }
    if !lower.contains("pkcs7-signature") {
        return Some(unchecked("het is geen S/MIME-handtekening"));
    }
    let Some(boundary) = parameter(&kind, "boundary") else { return Some(unchecked("het bericht is niet goed opgebouwd")) };
    let Some((content, p7s)) = signed_parts(body, boundary.as_bytes()) else { return Some(unchecked("het bericht is niet goed opgebouwd")) };
    Some(verify_detached(content, &p7s))
}

/// The header block (with its final CRLF) and the body of a MIME entity in CRLF form.
fn split_entity(raw: &[u8]) -> (&[u8], &[u8]) {
    if raw.starts_with(b"\r\n") {
        return (&[], &raw[2..]);
    }
    match find(raw, b"\r\n\r\n") {
        Some(i) => (&raw[..i + 2], &raw[i + 4..]),
        None => (raw, &[]),
    }
}

/// The unfolded value of the first header field called `name` (lower case).
fn header(head: &[u8], name: &[u8]) -> Option<String> {
    fields(head).into_iter().find_map(|f| {
        let colon = f.iter().position(|b| *b == b':')?;
        if !f[..colon].trim_ascii().eq_ignore_ascii_case(name) {
            return None;
        }
        let value = String::from_utf8_lossy(&f[colon + 1..]).replace("\r\n", "");
        Some(value.trim().to_string())
    })
}

/// A parameter of a header value such as `boundary="abc"`.
fn parameter(value: &str, name: &str) -> Option<String> {
    for part in value.split(';').skip(1) {
        let (key, v) = part.split_once('=')?;
        if key.trim().eq_ignore_ascii_case(name) {
            let v = v.trim();
            let v = v.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(v);
            return (!v.is_empty() && v.len() <= 200).then(|| v.to_string());
        }
    }
    None
}

/// The signed first part, byte for byte, and the decoded signature from the second part of a
/// `multipart/signed` body.
fn signed_parts<'a>(body: &'a [u8], boundary: &[u8]) -> Option<(&'a [u8], Vec<u8>)> {
    let mut delimiter = b"\r\n--".to_vec();
    delimiter.extend_from_slice(boundary);
    // The first delimiter may open the body, without a line break before it.
    let first = if body.starts_with(&delimiter[2..]) { 0 } else { find(body, &delimiter)? + 2 };
    let after = first + delimiter.len() - 2;
    let start = after + find(&body[after..], b"\r\n")? + 2;
    let end = start + find(&body[start..], &delimiter)?;
    let content = &body[start..end];
    let next = end + delimiter.len();
    let second = next + find(&body[next..], b"\r\n")? + 2;
    let close = second + find(&body[second..], &delimiter)?;
    let (head, data) = split_entity(&body[second..close]);
    let kind = header(head, b"content-type")?.to_ascii_lowercase();
    if !kind.contains("pkcs7-signature") {
        return None;
    }
    let encoding = header(head, b"content-transfer-encoding").unwrap_or_default().to_ascii_lowercase();
    let p7s = if encoding == "base64" {
        let text: Vec<u8> = data.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect();
        base64::engine::general_purpose::STANDARD.decode(text).ok()?
    } else {
        data.to_vec()
    };
    Some((content, p7s))
}

/// The header fields of `raw` that stay on the outside of a signed or encrypted message (all
/// but the content headers and MIME-Version), and the MIME entity inside it: the content
/// headers and the body, in CRLF form, ending with a line break.
fn split_message(raw: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let raw = crlf(raw);
    let (head, body) = match find(&raw, b"\r\n\r\n") {
        Some(i) => (&raw[..i + 2], &raw[i + 4..]),
        None => (raw.as_slice(), &b""[..]),
    };
    let (mut outer, mut inner) = (Vec::new(), Vec::new());
    for field in fields(head) {
        let name = field.split(|b| *b == b':').next().unwrap_or_default().to_ascii_lowercase();
        if name.starts_with(b"content-") {
            inner.extend_from_slice(field);
        } else if name != b"mime-version" {
            outer.extend_from_slice(field);
        }
    }
    if inner.is_empty() {
        inner.extend_from_slice(b"Content-Type: text/plain; charset=us-ascii\r\n");
    }
    inner.extend_from_slice(b"\r\n");
    inner.extend_from_slice(body);
    if !inner.ends_with(b"\r\n") {
        inner.extend_from_slice(b"\r\n");
    }
    (outer, inner)
}

/// Turns the RFC 5322 message `raw` into a signed one (RFC 8551 section 3.5,
/// multipart/signed): the content headers and the body become the signed part, the other
/// headers stay on top, and the signature follows as `smime.p7s`.
pub fn sign_message(identity: &Identity, raw: &[u8], now: i64) -> Result<Vec<u8>, SmimeError> {
    let (outer, inner) = split_message(raw);
    // The part ends with a line break that belongs to the boundary after it.
    let signed = &inner[..inner.len() - 2];
    let p7s = identity.sign_detached(signed, now)?;

    let boundary = boundary()?;
    let mut out = outer;
    out.extend_from_slice(b"MIME-Version: 1.0\r\n");
    out.extend_from_slice(
        format!("Content-Type: multipart/signed; protocol=\"application/pkcs7-signature\"; micalg=sha-256;\r\n\tboundary=\"{boundary}\"\r\n\r\n")
            .as_bytes(),
    );
    out.extend_from_slice(b"This is a cryptographically signed message in MIME format.\r\n\r\n");
    out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    out.extend_from_slice(&inner);
    out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    out.extend_from_slice(b"Content-Type: application/pkcs7-signature; name=\"smime.p7s\"\r\n");
    out.extend_from_slice(b"Content-Transfer-Encoding: base64\r\n");
    out.extend_from_slice(b"Content-Disposition: attachment; filename=\"smime.p7s\"\r\n");
    out.extend_from_slice(b"Content-Description: S/MIME Cryptographic Signature\r\n\r\n");
    let encoded = base64::engine::general_purpose::STANDARD.encode(&p7s);
    for line in encoded.as_bytes().chunks(76) {
        out.extend_from_slice(line);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Ok(out)
}

/// Every line ending as CRLF, the canonical form a signature covers.
fn crlf(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + raw.len() / 40);
    let mut prev = 0u8;
    for &b in raw {
        if b == b'\n' && prev != b'\r' {
            out.push(b'\r');
        }
        out.push(b);
        prev = b;
    }
    out
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// The header fields of `head`, each with its folded continuation lines and final CRLF.
fn fields(head: &[u8]) -> Vec<&[u8]> {
    let mut out: Vec<&[u8]> = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < head.len() {
        if head[i] == b'\n' {
            let next = head.get(i + 1).copied();
            if !matches!(next, Some(b' ' | b'\t')) {
                out.push(&head[start..=i]);
                start = i + 1;
            }
        }
        i += 1;
    }
    if start < head.len() {
        out.push(&head[start..]);
    }
    out
}

fn boundary() -> Result<String, SmimeError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| SmimeError::Sign)?;
    Ok(format!("----=_smime_{}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2027-01-01, inside the test certificates' validity.
    const NOW: i64 = 1_798_761_600;
    const P12: &[u8] = include_bytes!("../testdata/jij.p12");

    fn identity() -> Identity {
        Identity::from_pkcs12(P12, "geheim", NOW).unwrap()
    }

    #[test]
    fn imports_a_pkcs12_file_with_its_chain() {
        let id = identity();
        assert_eq!(id.info().name, "Jij Tester");
        assert_eq!(id.info().emails, vec!["jij@example.org".to_string()]);
        assert_eq!(id.info().issuer, "Mailcraft Test CA");
        assert!(id.info().covers(" JIJ@example.org"));
        assert!(!id.info().covers("ander@example.org"));
        assert_eq!(id.certs.len(), 2, "the issuer came along");
        assert!(!format!("{id:?}").contains("key: ["), "the key never prints");
        // Windows and older programs export with Triple DES.
        let legacy = Identity::from_pkcs12(include_bytes!("../testdata/jij-3des.p12"), "geheim", NOW).unwrap();
        assert_eq!(legacy.info(), id.info());
    }

    #[test]
    fn refuses_a_wrong_password_an_expired_or_unsuitable_certificate() {
        assert_eq!(Identity::from_pkcs12(P12, "fout", NOW).unwrap_err(), SmimeError::Pkcs12);
        assert_eq!(Identity::from_pkcs12(b"geen p12", "geheim", NOW).unwrap_err(), SmimeError::Pkcs12);
        let after = identity().info().not_after + 1;
        assert_eq!(Identity::from_pkcs12(P12, "geheim", after).unwrap_err(), SmimeError::Expired);
        assert_eq!(Identity::from_pkcs12(P12, "geheim", 0).unwrap_err(), SmimeError::NotYetValid);
        // A web server's certificate is not for mail.
        assert_eq!(Identity::from_pkcs12(include_bytes!("../testdata/server.p12"), "geheim", NOW).unwrap_err(), SmimeError::Usage);
    }

    #[test]
    fn the_stored_form_round_trips_and_rejects_damage() {
        let id = identity();
        let stored = id.to_stored();
        let again = Identity::from_stored(&stored).unwrap();
        assert_eq!(again.info(), id.info());
        assert_eq!(again.certs, id.certs);
        assert!(Identity::from_stored(&stored[..stored.len() - 3]).is_err());
        assert!(Identity::from_stored(b"MCS1").is_err());
        assert!(Identity::from_stored(b"").is_err());
    }

    #[test]
    fn a_detached_signature_verifies_and_catches_changes() {
        let id = identity();
        let content = b"Content-Type: text/plain\r\n\r\nHallo\r\n";
        let p7s = id.sign_detached(content, NOW).unwrap();
        match verify_detached(content, &p7s) {
            Signature::Valid { signer, certificate } => {
                assert_eq!(signer, *id.info());
                assert_eq!(certificate, id.certs[0]);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(verify_detached(b"Content-Type: text/plain\r\n\r\nHallo!\r\n", &p7s), Signature::Broken);
        let mut broken = p7s.clone();
        let n = broken.len() - 10;
        broken[n] ^= 1;
        assert_eq!(verify_detached(content, &broken), Signature::Broken);
        assert!(matches!(verify_detached(content, b"rommel"), Signature::Unchecked(_)));
    }

    #[test]
    fn signs_a_message_as_multipart_signed() {
        let raw = b"From: Jij <jij@example.org>\r\nTo: sanne@example.org\r\nSubject: Hoi\r\nMIME-Version: 1.0\r\nContent-Type: text/plain;\r\n charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nHallo Sanne\r\n";
        let signed = sign_message(&identity(), raw, NOW).unwrap();
        let text = String::from_utf8(signed.clone()).unwrap();
        let (head, body) = text.split_once("\r\n\r\n").unwrap();
        assert!(head.starts_with("From: Jij <jij@example.org>\r\nTo: sanne@example.org\r\nSubject: Hoi\r\nMIME-Version: 1.0\r\n"));
        assert!(head.contains("multipart/signed; protocol=\"application/pkcs7-signature\"; micalg=sha-256"));
        assert_eq!(head.matches("Content-Type").count(), 1, "the text's own type moved into the signed part");
        let boundary = head.split("boundary=\"").nth(1).unwrap().trim_end_matches('"');
        let parts: Vec<&str> = body.split(&format!("--{boundary}")).collect();
        assert_eq!(parts.len(), 4, "preamble, text, signature, epilogue");
        let part = parts[1].strip_prefix("\r\n").unwrap().strip_suffix("\r\n").unwrap();
        assert_eq!(part, "Content-Type: text/plain;\r\n charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nHallo Sanne");
        let (_, sig) = parts[2].split_once("\r\n\r\n").unwrap();
        let p7s = base64::engine::general_purpose::STANDARD.decode(sig.replace("\r\n", "")).unwrap();
        assert!(matches!(verify_detached(part.as_bytes(), &p7s), Signature::Valid { signer, .. } if signer.name == "Jij Tester"));
        assert!(matches!(check_message(&signed), Some(Signature::Valid { .. })));
        // Bare line feeds are made canonical before signing.
        let lf = String::from_utf8(raw.to_vec()).unwrap().replace("\r\n", "\n");
        assert!(sign_message(&identity(), lf.as_bytes(), NOW).unwrap().windows(2).all(|w| w[1] != b'\n' || w[0] == b'\r'));
    }

    #[test]
    fn checks_mail_signed_elsewhere() {
        // Signed by OpenSSL with RSA, with ECDSA, and the first one changed afterwards.
        match check_message(include_bytes!("../testdata/signed-openssl.eml")) {
            Some(Signature::Valid { signer, .. }) => assert_eq!(signer.emails, vec!["jij@example.org".to_string()]),
            other => panic!("{other:?}"),
        }
        assert!(
            matches!(check_message(include_bytes!("../testdata/signed-ec.eml")), Some(Signature::Valid { signer, .. }) if signer.name == "Ec Tester")
        );
        assert_eq!(check_message(include_bytes!("../testdata/signed-tampered.eml")), Some(Signature::Broken));
        assert_eq!(check_message(b"From: a@b.nl\r\nContent-Type: text/plain\r\n\r\nHoi\r\n"), None);
        assert!(matches!(
            check_message(b"Content-Type: multipart/signed; protocol=\"application/pkcs7-signature\"\r\n\r\nrommel\r\n"),
            Some(Signature::Unchecked(_))
        ));
        assert!(matches!(
            check_message(b"Content-Type: multipart/signed; protocol=\"application/pgp-signature\"; boundary=x\r\n\r\n--x\r\n"),
            Some(Signature::Unchecked(_))
        ));
    }
}
