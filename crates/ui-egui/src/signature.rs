//! The strip over a signed message, as Outlook shows its red ribbon or seal: who signed, or
//! that the signature does not hold. An encrypted message that opened says so too.

use crate::i18n::tr;
use egui::{CornerRadius, Margin, RichText, Ui};
use merel_mime::Encryption;
use merel_smime::Signature;

use crate::Tokens;

/// The button under a signature from an issuer the computer does not know.
pub fn trust_label() -> String {
    tr("sig-trust")
}

pub fn broken() -> String {
    tr("sig-broken")
}

pub fn encrypted() -> String {
    tr("sig-encrypted")
}

/// The strip over an encrypted message that opened. One that did not open shows why in place
/// of its text, so it needs no strip.
pub fn encryption_strip(ui: &mut Ui, t: &Tokens, encryption: &Encryption) {
    if *encryption != Encryption::Opened {
        return;
    }
    egui::Frame::NONE.fill(t.checked).corner_radius(CornerRadius::same(4)).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(encrypted()).size(12.0).color(t.text));
    });
    ui.add_space(8.0);
}

pub fn pgp_broken() -> String {
    tr("sig-pgp-broken")
}

/// What the strip over an OpenPGP-signed message says, and whether it is a warning.
pub fn pgp_text(verdict: &merel_pgp::Verdict, from: &str) -> (String, bool) {
    use merel_pgp::Verdict;
    match verdict {
        Verdict::Valid { signer } if signer.covers(from) => (
            crate::i18n::tr_args(
                "sig-pgp-valid",
                &[
                    ("name", signer.name.as_str().into()),
                    ("email", from.to_lowercase().into()),
                    ("key", signer.grouped().rsplit(' ').take(2).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" ").into()),
                ],
            ),
            false,
        ),
        Verdict::Valid { signer } => (
            crate::i18n::tr_args(
                "sig-pgp-other",
                &[("signer", if signer.emails.is_empty() { signer.name.clone() } else { signer.emails.join(", ") }.into()), ("from", from.into())],
            ),
            true,
        ),
        Verdict::Broken => (pgp_broken(), true),
        Verdict::UnknownKey(_) => (tr("sig-pgp-unknown-key"), false),
        Verdict::Unchecked(reason) => (crate::i18n::tr_args("sig-pgp-unchecked", &[("reason", reason.to_string().into())]), false),
    }
}

pub fn pgp_strip(ui: &mut Ui, t: &Tokens, verdict: &merel_pgp::Verdict, from: &str) {
    let (line, warning) = pgp_text(verdict, from);
    let fill = if warning { t.flag.gamma_multiply(0.18) } else { t.checked };
    let response = egui::Frame::NONE
        .fill(fill)
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(line).size(12.0).color(t.text));
        })
        .response;
    if let merel_pgp::Verdict::Valid { signer } = verdict {
        response.on_hover_text(crate::i18n::tr_args("sig-pgp-fingerprint", &[("fingerprint", signer.grouped().into())]));
    }
    ui.add_space(8.0);
}

/// What the strip says, and whether it is a warning.
pub fn text(signature: &Signature, from: &str) -> (String, bool) {
    match signature {
        Signature::Valid { signer, trusted: true, .. } if signer.covers(from) => {
            (crate::i18n::tr_args("sig-valid", &[("name", signer.name.as_str().into()), ("email", from.to_lowercase().into())]), false)
        }
        Signature::Valid { signer, .. } if signer.covers(from) => (
            crate::i18n::tr_args(
                "sig-untrusted-issuer",
                &[
                    ("name", signer.name.as_str().into()),
                    ("email", from.to_lowercase().into()),
                    ("issuer", if signer.issuer.is_empty() { tr("unknown") } else { signer.issuer.clone() }.into()),
                ],
            ),
            true,
        ),
        Signature::Valid { signer, .. } => (
            crate::i18n::tr_args(
                "sig-other",
                &[("signer", if signer.emails.is_empty() { signer.name.clone() } else { signer.emails.join(", ") }.into()), ("from", from.into())],
            ),
            true,
        ),
        Signature::Broken => (broken(), true),
        Signature::Unchecked(reason) => (crate::i18n::tr_args("sig-unchecked", &[("reason", reason.to_string().into())]), false),
    }
}

/// Draws the strip; true when "Certificaat vertrouwen" was pressed. `can_trust` is false in
/// the junk folder, where nothing is to be trusted.
pub fn strip(ui: &mut Ui, t: &Tokens, signature: &Signature, from: &str, can_trust: bool) -> bool {
    let (line, warning) = text(signature, from);
    let offer = can_trust && matches!(signature, Signature::Valid { signer, trusted: false, .. } if signer.covers(from));
    let mut trust = false;
    let fill = if warning { t.flag.gamma_multiply(0.18) } else { t.checked };
    let response = egui::Frame::NONE
        .fill(fill)
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(line).size(12.0).color(t.text));
            if offer && ui.button(trust_label()).on_hover_text(tr("sig-trust-hint")).clicked() {
                trust = true;
            }
        })
        .response;
    if let Signature::Valid { signer, .. } = signature {
        response.on_hover_text(crate::i18n::tr_args(
            "sig-certificate",
            &[
                ("name", signer.name.as_str().into()),
                ("issuer", if signer.issuer.is_empty() { tr("unknown") } else { signer.issuer.clone() }.into()),
                ("until", crate::dates::day(signer.not_after).into()),
            ],
        ));
    }
    ui.add_space(8.0);
    trust
}

#[cfg(test)]
mod tests {
    use merel_smime::CertInfo;

    use super::*;

    fn valid() -> Signature {
        Signature::Valid {
            signer: CertInfo { name: "Sanne".into(), emails: vec!["sanne@example.org".into()], issuer: "CA".into(), not_before: 0, not_after: 0 },
            certificate: Vec::new(),
            issuers: Vec::new(),
            trusted: true,
        }
    }

    #[test]
    fn says_who_signed_and_warns_when_it_does_not_hold() {
        assert_eq!(
            text(&valid(), "Sanne@example.org"),
            ("✔ Digitaal ondertekend door Sanne <sanne@example.org>. Het bericht is onderweg niet veranderd.".to_string(), false)
        );
        let (line, warning) = text(&valid(), "bank@example.org");
        assert!(warning && line.contains("certificaat van sanne@example.org, niet van de afzender bank@example.org"));
        assert_eq!(text(&Signature::Broken, "x@y.nl"), (broken(), true));
        assert!(!text(&Signature::Unchecked("oud".into()), "x@y.nl").1);
        let Signature::Valid { signer, certificate, issuers, .. } = valid() else { unreachable!() };
        let unknown = Signature::Valid { signer, certificate, issuers, trusted: false };
        let (line, warning) = text(&unknown, "sanne@example.org");
        assert!(warning && line.contains("de uitgever (CA) is niet bekend als betrouwbaar"), "{line}");
    }

    #[test]
    fn says_whose_openpgp_key_signed() {
        use merel_pgp::{KeyInfo, Verdict};
        let signer = KeyInfo {
            fingerprint: "0123456789ABCDEF0123456789ABCDEF01234567".into(),
            name: "Jij".into(),
            emails: vec!["jij@example.org".into()],
            created: 0,
        };
        let (line, warning) = pgp_text(&Verdict::Valid { signer: signer.clone() }, "Jij@example.org");
        assert_eq!(line, "✔ Ondertekend met de OpenPGP-sleutel van Jij <jij@example.org> (0123 4567). Het bericht is onderweg niet veranderd.");
        assert!(!warning);
        assert!(pgp_text(&Verdict::Valid { signer }, "bank@example.org").1);
        assert_eq!(pgp_text(&Verdict::Broken, "x@y.nl"), (pgp_broken(), true));
        assert!(!pgp_text(&Verdict::UnknownKey("AB".into()), "x@y.nl").1);
    }
}
