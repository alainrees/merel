//! Whether a signer's certificate comes from an issuer the computer trusts: the chain from the
//! certificate through the issuers enclosed with the signature up to one of the trusted roots,
//! every link signed by the next, every issuer a certificate authority, all valid now.
//!
//! The roots are the computer's own (the engine reads them); nothing here goes online, so a
//! revoked certificate is not noticed.

use aws_lc_rs::signature::{self, UnparsedPublicKey};
use der::asn1::ObjectIdentifier;
use der::{Decode, Encode};
use x509_cert::Certificate;
use x509_cert::ext::pkix::{BasicConstraints, KeyUsage};
use x509_cert::spki::SubjectPublicKeyInfoOwned;

use crate::{
    EC_PUBLIC_KEY, ECDSA_WITH_SHA256, ECDSA_WITH_SHA384, RSA_ENCRYPTION, SECP256R1, SECP384R1, SHA256_WITH_RSA, SHA384_WITH_RSA, SHA512_WITH_RSA,
};
/// Most issuers between a certificate and its root.
const MAX_DEPTH: usize = 6;

/// Whether `certificate` (DER) chains up to one of `roots` (DER) through `issuers` (DER, the
/// other certificates that came with the signature), all valid at `now`.
pub fn issued_by_trusted(certificate: &[u8], issuers: &[Vec<u8>], roots: &[Vec<u8>], now: i64) -> bool {
    let Ok(leaf) = Certificate::from_der(certificate) else { return false };
    let issuers: Vec<Certificate> = issuers.iter().filter_map(|c| Certificate::from_der(c).ok()).collect();
    let roots: Vec<Certificate> = roots.iter().filter_map(|c| Certificate::from_der(c).ok()).collect();
    if !valid_at(&leaf, now) {
        return false;
    }
    let mut current = leaf;
    for _ in 0..MAX_DEPTH {
        let wanted = current.tbs_certificate().issuer();
        if roots.iter().any(|r| r.tbs_certificate().subject() == wanted && signed_by(&current, r.tbs_certificate().subject_public_key_info())) {
            return true;
        }
        let next = issuers.iter().find(|i| {
            i.tbs_certificate().subject() == wanted
                && i != &&current
                && is_authority(i)
                && valid_at(i, now)
                && signed_by(&current, i.tbs_certificate().subject_public_key_info())
        });
        match next {
            Some(n) => current = n.clone(),
            None => return false,
        }
    }
    false
}

fn valid_at(c: &Certificate, now: i64) -> bool {
    let v = c.tbs_certificate().validity();
    let secs = |t: x509_cert::time::Time| i64::try_from(t.to_unix_duration().as_secs()).unwrap_or(i64::MAX);
    secs(v.not_before) <= now && now <= secs(v.not_after)
}

/// A certificate authority: basic constraints say so, and key usage, when given, allows
/// signing certificates.
fn is_authority(c: &Certificate) -> bool {
    let tbs = c.tbs_certificate();
    let ca = matches!(tbs.get_extension::<BasicConstraints>(), Ok(Some((_, b))) if b.ca);
    let signs = match tbs.get_extension::<KeyUsage>() {
        Ok(Some((_, u))) => u.key_cert_sign(),
        _ => true,
    };
    ca && signs
}

/// Whether the issuer's key `spki` made the signature on `c`.
fn signed_by(c: &Certificate, spki: &SubjectPublicKeyInfoOwned) -> bool {
    let Ok(tbs) = c.tbs_certificate().to_der() else { return false };
    let Some(sig) = c.signature().as_bytes() else { return false };
    let key = spki.subject_public_key.raw_bytes();
    let alg = c.signature_algorithm().oid;
    let algorithm: &dyn signature::VerificationAlgorithm = if spki.algorithm.oid == RSA_ENCRYPTION {
        match alg {
            a if a == SHA256_WITH_RSA => &signature::RSA_PKCS1_2048_8192_SHA256,
            a if a == SHA384_WITH_RSA => &signature::RSA_PKCS1_2048_8192_SHA384,
            a if a == SHA512_WITH_RSA => &signature::RSA_PKCS1_2048_8192_SHA512,
            _ => return false,
        }
    } else if spki.algorithm.oid == EC_PUBLIC_KEY {
        let curve = spki.algorithm.parameters.as_ref().and_then(|p| p.decode_as::<ObjectIdentifier>().ok());
        match (curve, alg) {
            (Some(c), a) if c == SECP256R1 && a == ECDSA_WITH_SHA256 => &signature::ECDSA_P256_SHA256_ASN1,
            (Some(c), a) if c == SECP256R1 && a == ECDSA_WITH_SHA384 => &signature::ECDSA_P256_SHA384_ASN1,
            (Some(c), a) if c == SECP384R1 && a == ECDSA_WITH_SHA256 => &signature::ECDSA_P384_SHA256_ASN1,
            (Some(c), a) if c == SECP384R1 && a == ECDSA_WITH_SHA384 => &signature::ECDSA_P384_SHA384_ASN1,
            _ => return false,
        }
    } else {
        return false;
    };
    UnparsedPublicKey::new(algorithm, key).verify(&tbs, sig).is_ok()
}
