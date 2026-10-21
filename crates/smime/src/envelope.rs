//! Encrypted mail (RFC 8551 section 3.3): CMS EnvelopedData with AES-256-CBC for the message
//! and RSA for the key, one copy of the key per recipient, sent as `application/pkcs7-mime`.
//!
//! The key goes out with RSAES-PKCS1-v1_5, as Outlook, Apple Mail and Thunderbird all read it;
//! RSAES-OAEP is read as well. Every decryption failure says the same thing, so the answer
//! tells an attacker nothing about why.

use aws_lc_rs::cipher::{AES_128, AES_192, AES_256, DecryptionContext, PaddedBlockDecryptingKey, PaddedBlockEncryptingKey, UnboundCipherKey};
use aws_lc_rs::rsa::{
    OAEP_SHA1_MGF1SHA1, OAEP_SHA256_MGF1SHA256, OaepPrivateDecryptingKey, Pkcs1PrivateDecryptingKey, Pkcs1PublicEncryptingKey, PrivateDecryptingKey,
    PublicEncryptingKey,
};
use base64::Engine as _;
use cms::cert::IssuerAndSerialNumber;
use cms::content_info::{CmsVersion, ContentInfo};
use cms::enveloped_data::{EncryptedContentInfo, EnvelopedData, KeyTransRecipientInfo, RecipientIdentifier, RecipientInfo, RecipientInfos};
use der::asn1::{Null, ObjectIdentifier, OctetString};
use der::{Any, Decode, Encode};
use x509_cert::Certificate;
use x509_cert::ext::pkix::{KeyUsage, SubjectKeyIdentifier};
use x509_cert::spki::AlgorithmIdentifierOwned;
use zeroize::Zeroizing;

use crate::raw::{crlf, find, header, split_entity, split_message};
use crate::{Identity, RSA_ENCRYPTION, SmimeError};

const ID_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1");
const ID_ENVELOPED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.3");
const RSAES_OAEP: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.7");
const AES128_CBC: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.1.2");
const AES192_CBC: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.1.22");
const AES256_CBC: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.1.42");
const DES_EDE3_CBC: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.3.7");
/// The DER of the SHA-256 OID, to tell OAEP parameters with SHA-256 from the SHA-1 default.
const SHA256_OID_DER: &[u8] = &[0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01];
/// Most bytes an encrypted message may hold once opened.
const MAX_CONTENT: usize = 64 * 1024 * 1024;

/// The one answer for a message that does not open, whatever the cause.
fn not_opened() -> SmimeError {
    SmimeError::Decrypt(merel_i18n::tr("smime-not-opened"))
}

/// Whether `raw` is an encrypted S/MIME message (`application/pkcs7-mime` with enveloped
/// data at the top).
pub fn is_encrypted(raw: &[u8]) -> bool {
    let raw = crlf(raw);
    let (head, _) = split_entity(&raw);
    let Some(kind) = header(head, b"content-type") else { return false };
    let kind = kind.to_ascii_lowercase();
    (kind.starts_with("application/pkcs7-mime") || kind.starts_with("application/x-pkcs7-mime"))
        && (kind.contains("enveloped-data") || (!kind.contains("smime-type") && kind.contains(".p7m")))
}

/// Whether `der` is a certificate mail can be encrypted to: an RSA key that may carry keys.
pub fn can_encrypt_to(der: &[u8]) -> bool {
    recipient_key(der).is_ok()
}

fn recipient_key(der: &[u8]) -> Result<(Certificate, PublicEncryptingKey), SmimeError> {
    let cert = Certificate::from_der(der).map_err(|_| SmimeError::RecipientKey)?;
    let tbs = cert.tbs_certificate();
    if tbs.subject_public_key_info().algorithm.oid != RSA_ENCRYPTION {
        return Err(SmimeError::RecipientKey);
    }
    if let Ok(Some((_, usage))) = tbs.get_extension::<KeyUsage>()
        && !usage.key_encipherment()
    {
        return Err(SmimeError::RecipientKey);
    }
    let spki = tbs.subject_public_key_info().to_der().map_err(|_| SmimeError::RecipientKey)?;
    let key = PublicEncryptingKey::from_der(&spki).map_err(|_| SmimeError::RecipientKey)?;
    Ok((cert, key))
}

/// Encrypts the RFC 5322 message `raw` for the holders of `recipients` (certificates, DER):
/// the content headers and the body go inside, the other headers (the subject among them)
/// stay readable on top. Include the sender's own certificate to read it back from Sent.
pub fn encrypt_message(raw: &[u8], recipients: &[Vec<u8>]) -> Result<Vec<u8>, SmimeError> {
    if recipients.is_empty() {
        return Err(SmimeError::RecipientKey);
    }
    let (outer, inner) = split_message(raw);
    let mut cek = Zeroizing::new([0u8; 32]);
    getrandom::fill(cek.as_mut_slice()).map_err(|_| SmimeError::Encrypt)?;
    let key = UnboundCipherKey::new(&AES_256, cek.as_slice()).map_err(|_| SmimeError::Encrypt)?;
    let key = PaddedBlockEncryptingKey::cbc_pkcs7(key).map_err(|_| SmimeError::Encrypt)?;
    let mut sealed = inner;
    let context = key.encrypt(&mut sealed).map_err(|_| SmimeError::Encrypt)?;
    let DecryptionContext::Iv128(iv) = context else { return Err(SmimeError::Encrypt) };
    let iv: &[u8] = iv.as_ref();

    let mut infos = Vec::new();
    for der in recipients {
        let (cert, public) = recipient_key(der)?;
        let public = Pkcs1PublicEncryptingKey::new(public).map_err(|_| SmimeError::RecipientKey)?;
        let mut wrapped = vec![0u8; public.ciphertext_size()];
        let wrapped = public.encrypt(cek.as_slice(), &mut wrapped).map_err(|_| SmimeError::Encrypt)?;
        let tbs = cert.tbs_certificate();
        infos.push(RecipientInfo::Ktri(KeyTransRecipientInfo {
            version: CmsVersion::V0,
            rid: RecipientIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
                issuer: tbs.issuer().clone(),
                serial_number: tbs.serial_number().clone(),
            }),
            key_enc_alg: AlgorithmIdentifierOwned {
                oid: RSA_ENCRYPTION,
                parameters: Some(Any::encode_from(&Null).map_err(|_| SmimeError::Encrypt)?),
            },
            enc_key: OctetString::new(wrapped.to_vec()).map_err(|_| SmimeError::Encrypt)?,
        }));
    }
    let data = EnvelopedData {
        version: CmsVersion::V0,
        originator_info: None,
        recip_infos: RecipientInfos::try_from(infos).map_err(|_| SmimeError::Encrypt)?,
        encrypted_content: EncryptedContentInfo {
            content_type: ID_DATA,
            content_enc_alg: AlgorithmIdentifierOwned {
                oid: AES256_CBC,
                parameters: Some(Any::encode_from(&OctetString::new(iv).map_err(|_| SmimeError::Encrypt)?).map_err(|_| SmimeError::Encrypt)?),
            },
            encrypted_content: Some(OctetString::new(sealed).map_err(|_| SmimeError::Encrypt)?),
        },
        unprotected_attrs: None,
    };
    let info = ContentInfo { content_type: ID_ENVELOPED_DATA, content: Any::encode_from(&data).map_err(|_| SmimeError::Encrypt)? };
    let p7m = info.to_der().map_err(|_| SmimeError::Encrypt)?;

    let mut out = outer;
    out.extend_from_slice(b"MIME-Version: 1.0\r\n");
    out.extend_from_slice(b"Content-Type: application/pkcs7-mime; smime-type=enveloped-data; name=\"smime.p7m\"\r\n");
    out.extend_from_slice(b"Content-Transfer-Encoding: base64\r\n");
    out.extend_from_slice(b"Content-Disposition: attachment; filename=\"smime.p7m\"\r\n");
    out.extend_from_slice(b"Content-Description: S/MIME Encrypted Message\r\n\r\n");
    let encoded = base64::engine::general_purpose::STANDARD.encode(&p7m);
    for line in encoded.as_bytes().chunks(76) {
        out.extend_from_slice(line);
        out.extend_from_slice(b"\r\n");
    }
    Ok(out)
}

/// Opens the encrypted message `raw` with `identity`: the result is the message as it was
/// before it was encrypted, the outer headers with the decrypted entity under them.
pub fn decrypt_message(identity: &Identity, raw: &[u8]) -> Result<Zeroizing<Vec<u8>>, SmimeError> {
    let raw = crlf(raw);
    let (head, body) = split_entity(&raw);
    let encoding = header(head, b"content-transfer-encoding").unwrap_or_default().to_ascii_lowercase();
    let p7m = if encoding == "base64" {
        let text: Vec<u8> = body.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect();
        base64::engine::general_purpose::STANDARD.decode(text).map_err(|_| not_opened())?
    } else {
        body.to_vec()
    };
    let info = ContentInfo::from_der(&p7m).map_err(|_| not_opened())?;
    if info.content_type != ID_ENVELOPED_DATA {
        return Err(not_opened());
    }
    let data: EnvelopedData = info.content.decode_as().map_err(|_| not_opened())?;
    let own = identity.certificate()?;
    let tbs = own.tbs_certificate();
    let own_ski = tbs.get_extension::<SubjectKeyIdentifier>().ok().flatten().map(|(_, s)| s);
    let mine = data.recip_infos.0.iter().find_map(|r| match r {
        RecipientInfo::Ktri(k) => match &k.rid {
            RecipientIdentifier::IssuerAndSerialNumber(isn) if isn.issuer == *tbs.issuer() && isn.serial_number == *tbs.serial_number() => Some(k),
            RecipientIdentifier::SubjectKeyIdentifier(ski) if own_ski.as_ref() == Some(ski) => Some(k),
            _ => None,
        },
        _ => None,
    });
    let Some(mine) = mine else { return Err(SmimeError::NotForMe) };
    let private = PrivateDecryptingKey::from_pkcs8(&identity.key).map_err(|_| not_opened())?;
    let mut cek = Zeroizing::new(vec![0u8; private.key_size_bytes()]);
    let cek_len = if mine.key_enc_alg.oid == RSA_ENCRYPTION {
        let key = Pkcs1PrivateDecryptingKey::new(private).map_err(|_| not_opened())?;
        key.decrypt(mine.enc_key.as_bytes(), &mut cek).map_err(|_| not_opened())?.len()
    } else if mine.key_enc_alg.oid == RSAES_OAEP {
        // RSAES-OAEP-params name SHA-1 by leaving the hash out; Outlook and OpenSSL write
        // SHA-256 explicitly when they use it.
        let params = mine.key_enc_alg.parameters.as_ref().and_then(|p| p.to_der().ok()).unwrap_or_default();
        let algorithm = if find(&params, SHA256_OID_DER).is_some() { &OAEP_SHA256_MGF1SHA256 } else { &OAEP_SHA1_MGF1SHA1 };
        let key = OaepPrivateDecryptingKey::new(private).map_err(|_| not_opened())?;
        key.decrypt(algorithm, mine.enc_key.as_bytes(), &mut cek, None).map_err(|_| not_opened())?.len()
    } else {
        return Err(SmimeError::Decrypt(merel_i18n::tr("crypto-unknown-algorithm")));
    };
    let cek = cek.get(..cek_len).ok_or_else(not_opened)?;
    let content = &data.encrypted_content;
    let cipher = match content.content_enc_alg.oid {
        o if o == AES128_CBC => &AES_128,
        o if o == AES192_CBC => &AES_192,
        o if o == AES256_CBC => &AES_256,
        o if o == DES_EDE3_CBC => return Err(SmimeError::Decrypt(merel_i18n::tr("smime-3des"))),
        _ => return Err(SmimeError::Decrypt(merel_i18n::tr("crypto-unknown-algorithm"))),
    };
    let iv = content.content_enc_alg.parameters.as_ref().and_then(|p| p.decode_as::<OctetString>().ok()).ok_or_else(not_opened)?;
    let iv: [u8; 16] = iv.as_bytes().try_into().map_err(|_| not_opened())?;
    let sealed = content.encrypted_content.as_ref().ok_or_else(not_opened)?;
    if sealed.as_bytes().len() > MAX_CONTENT {
        return Err(not_opened());
    }
    let key = UnboundCipherKey::new(cipher, cek).map_err(|_| not_opened())?;
    let key = PaddedBlockDecryptingKey::cbc_pkcs7(key).map_err(|_| not_opened())?;
    let mut plain = Zeroizing::new(sealed.as_bytes().to_vec());
    let opened = key.decrypt(&mut plain, DecryptionContext::Iv128(iv.into())).map_err(|_| not_opened())?;
    let mut out = Zeroizing::new(Vec::with_capacity(head.len() + opened.len()));
    for field in crate::fields(head) {
        let name = field.split(|b| *b == b':').next().unwrap_or_default().to_ascii_lowercase();
        if !name.starts_with(b"content-") {
            out.extend_from_slice(field);
        }
    }
    out.extend_from_slice(&crlf(opened));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_798_761_600;

    fn identity(p12: &[u8]) -> Identity {
        Identity::from_pkcs12(p12, "geheim", NOW).unwrap()
    }

    fn jij() -> Identity {
        identity(include_bytes!("../testdata/jij.p12"))
    }

    fn alain() -> Identity {
        identity(include_bytes!("../testdata/alain.p12"))
    }

    const RAW: &[u8] = b"From: Jij <jij@example.org>\r\nTo: alain@voorbeeld.nl\r\nSubject: Geheim\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nDe code is 1234.\r\n";

    #[test]
    fn encrypts_for_every_recipient_and_only_them() {
        let sealed = encrypt_message(RAW, &[alain().certs[0].clone(), jij().certs[0].clone()]).unwrap();
        let text = String::from_utf8(sealed.clone()).unwrap();
        assert!(text.contains("Subject: Geheim\r\n"), "the subject stays readable");
        assert!(text.contains("application/pkcs7-mime; smime-type=enveloped-data"));
        assert!(!text.contains("1234"), "the text does not");
        assert!(is_encrypted(&sealed) && !is_encrypted(RAW));
        for reader in [alain(), jij()] {
            let opened = decrypt_message(&reader, &sealed).unwrap();
            let opened = String::from_utf8(opened.to_vec()).unwrap();
            assert!(
                opened.starts_with("From: Jij <jij@example.org>\r\nTo: alain@voorbeeld.nl\r\nSubject: Geheim\r\nMIME-Version: 1.0\r\n"),
                "{opened}"
            );
            assert!(
                opened.ends_with("Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nDe code is 1234.\r\n"),
                "{opened}"
            );
        }
        let only_jij = encrypt_message(RAW, &[jij().certs[0].clone()]).unwrap();
        assert_eq!(decrypt_message(&alain(), &only_jij).unwrap_err(), SmimeError::NotForMe);
        let mut broken = only_jij.clone();
        let n = broken.len() - 20;
        broken[n] = if broken[n] == b'A' { b'B' } else { b'A' };
        assert!(decrypt_message(&jij(), &broken).is_err());
        assert!(encrypt_message(RAW, &[]).is_err());
        assert_eq!(encrypt_message(RAW, &[b"geen certificaat".to_vec()]).unwrap_err(), SmimeError::RecipientKey);
    }

    #[test]
    fn opens_mail_encrypted_by_openssl() {
        // `openssl smime -encrypt -aes256` (PKCS#1 v1.5) and `openssl cms -encrypt -keyopt rsa_padding_mode:oaep`.
        for file in [&include_bytes!("../testdata/encrypted-openssl.eml")[..], &include_bytes!("../testdata/encrypted-oaep.eml")[..]] {
            assert!(is_encrypted(file));
            let opened = decrypt_message(&jij(), file).unwrap();
            let opened = String::from_utf8_lossy(&opened).into_owned();
            assert!(opened.contains("Subject: Versleuteld\r\n") && opened.contains("Tot donderdag."), "{opened}");
        }
    }
}
