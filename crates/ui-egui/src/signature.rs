//! The strip over a signed message, as Outlook shows its red ribbon or seal: who signed, or
//! that the signature does not hold. An encrypted message that opened says so too.

use egui::{CornerRadius, Margin, RichText, Ui};
use mailcraft_mime::Encryption;
use mailcraft_smime::Signature;

use crate::Tokens;

/// The button under a signature from an issuer the computer does not know.
pub const TRUST: &str = "Certificaat vertrouwen";

pub const BROKEN: &str = "⚠ De digitale handtekening klopt niet. Het bericht is na het ondertekenen veranderd, of de handtekening is vervalst.";

pub const ENCRYPTED: &str = "🔒 Versleuteld bericht. Alleen de ontvangers kunnen het lezen; het onderwerp was onderweg niet versleuteld.";

/// The strip over an encrypted message that opened. One that did not open shows why in place
/// of its text, so it needs no strip.
pub fn encryption_strip(ui: &mut Ui, t: &Tokens, encryption: &Encryption) {
    if *encryption != Encryption::Opened {
        return;
    }
    egui::Frame::NONE.fill(t.checked).corner_radius(CornerRadius::same(4)).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(ENCRYPTED).size(12.0).color(t.text));
    });
    ui.add_space(8.0);
}

pub const PGP_BROKEN: &str = "⚠ De OpenPGP-handtekening klopt niet. Het bericht is na het ondertekenen veranderd, of de handtekening is vervalst.";

/// What the strip over an OpenPGP-signed message says, and whether it is a warning.
pub fn pgp_text(verdict: &mailcraft_pgp::Verdict, from: &str) -> (String, bool) {
    use mailcraft_pgp::Verdict;
    match verdict {
        Verdict::Valid { signer } if signer.covers(from) => (
            format!(
                "✔ Ondertekend met de OpenPGP-sleutel van {} <{}> ({}). Het bericht is onderweg niet veranderd.",
                signer.name,
                from.to_lowercase(),
                signer.grouped().rsplit(' ').take(2).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" ")
            ),
            false,
        ),
        Verdict::Valid { signer } => (
            format!(
                "⚠ Ondertekend met de OpenPGP-sleutel van {}, niet van de afzender {}. Wees voorzichtig.",
                if signer.emails.is_empty() { signer.name.clone() } else { signer.emails.join(", ") },
                from
            ),
            true,
        ),
        Verdict::Broken => (PGP_BROKEN.to_string(), true),
        Verdict::UnknownKey(_) => (
            "Dit bericht is met OpenPGP ondertekend, maar de sleutel van de afzender is nog niet bekend, dus Mailcraft kan de handtekening niet controleren.".to_string(),
            false,
        ),
        Verdict::Unchecked(reason) => (format!("Dit bericht is met OpenPGP ondertekend, maar Mailcraft kan de handtekening niet controleren: {reason}."), false),
    }
}

pub fn pgp_strip(ui: &mut Ui, t: &Tokens, verdict: &mailcraft_pgp::Verdict, from: &str) {
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
    if let mailcraft_pgp::Verdict::Valid { signer } = verdict {
        response.on_hover_text(format!(
            "Vingerafdruk {}. Mailcraft kreeg deze sleutel met eerdere mail van dit adres (Autocrypt); vergelijk de vingerafdruk met de afzender als je zeker wilt zijn.",
            signer.grouped()
        ));
    }
    ui.add_space(8.0);
}

/// What the strip says, and whether it is a warning.
pub fn text(signature: &Signature, from: &str) -> (String, bool) {
    match signature {
        Signature::Valid { signer, trusted: true, .. } if signer.covers(from) => {
            (format!("✔ Digitaal ondertekend door {} <{}>. Het bericht is onderweg niet veranderd.", signer.name, from.to_lowercase()), false)
        }
        Signature::Valid { signer, .. } if signer.covers(from) => (
            format!(
                "⚠ Ondertekend als {} <{}>, maar de uitgever ({}) is niet bekend als betrouwbaar. Iedereen kan zo'n certificaat maken; vertrouw het alleen als je weet dat het van deze afzender is.",
                signer.name,
                from.to_lowercase(),
                if signer.issuer.is_empty() { "onbekend" } else { signer.issuer.as_str() }
            ),
            true,
        ),
        Signature::Valid { signer, .. } => (
            format!(
                "⚠ Ondertekend met een certificaat van {}, niet van de afzender {}. Wees voorzichtig.",
                if signer.emails.is_empty() { signer.name.clone() } else { signer.emails.join(", ") },
                from
            ),
            true,
        ),
        Signature::Broken => (BROKEN.to_string(), true),
        Signature::Unchecked(reason) => {
            (format!("Dit bericht is digitaal ondertekend, maar Mailcraft kan de handtekening niet controleren: {reason}."), false)
        }
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
            if offer
                && ui
                    .button(TRUST)
                    .on_hover_text(
                        "Bewaart dit certificaat voor dit adres: de handtekening telt voortaan als betrouwbaar en je kunt versleuteld terugmailen. Doe dit alleen als je zeker weet dat het van de afzender is, bijvoorbeeld omdat je het hebt nagevraagd.",
                    )
                    .clicked()
            {
                trust = true;
            }
        })
        .response;
    if let Signature::Valid { signer, .. } = signature {
        response.on_hover_text(format!(
            "Certificaat van {}, uitgegeven door {}, geldig tot {}. Mailcraft controleert niet online of het is ingetrokken.",
            signer.name,
            if signer.issuer.is_empty() { "onbekend" } else { signer.issuer.as_str() },
            crate::dates::day(signer.not_after)
        ));
    }
    ui.add_space(8.0);
    trust
}

#[cfg(test)]
mod tests {
    use mailcraft_smime::CertInfo;

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
        assert_eq!(text(&Signature::Broken, "x@y.nl"), (BROKEN.to_string(), true));
        assert!(!text(&Signature::Unchecked("oud".into()), "x@y.nl").1);
        let Signature::Valid { signer, certificate, issuers, .. } = valid() else { unreachable!() };
        let unknown = Signature::Valid { signer, certificate, issuers, trusted: false };
        let (line, warning) = text(&unknown, "sanne@example.org");
        assert!(warning && line.contains("de uitgever (CA) is niet bekend als betrouwbaar"), "{line}");
    }

    #[test]
    fn says_whose_openpgp_key_signed() {
        use mailcraft_pgp::{KeyInfo, Verdict};
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
        assert_eq!(pgp_text(&Verdict::Broken, "x@y.nl"), (PGP_BROKEN.to_string(), true));
        assert!(!pgp_text(&Verdict::UnknownKey("AB".into()), "x@y.nl").1);
    }
}
