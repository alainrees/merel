//! The strip over a signed message, as Outlook shows its red ribbon or seal: who signed, or
//! that the signature does not hold.

use egui::{CornerRadius, Margin, RichText, Ui};
use mailcraft_smime::Signature;

use crate::Tokens;

pub const BROKEN: &str = "⚠ De digitale handtekening klopt niet. Het bericht is na het ondertekenen veranderd, of de handtekening is vervalst.";

/// What the strip says, and whether it is a warning.
pub fn text(signature: &Signature, from: &str) -> (String, bool) {
    match signature {
        Signature::Valid { signer, .. } if signer.covers(from) => {
            (format!("✔ Digitaal ondertekend door {} <{}>. Het bericht is onderweg niet veranderd.", signer.name, from.to_lowercase()), false)
        }
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

pub fn strip(ui: &mut Ui, t: &Tokens, signature: &Signature, from: &str) {
    let (line, warning) = text(signature, from);
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
    if let Signature::Valid { signer, .. } = signature {
        response.on_hover_text(format!(
            "Certificaat van {}, uitgegeven door {}, geldig tot {}. Mailcraft controleert nog niet of deze uitgever te vertrouwen is.",
            signer.name,
            if signer.issuer.is_empty() { "onbekend" } else { signer.issuer.as_str() },
            crate::dates::day(signer.not_after)
        ));
    }
    ui.add_space(8.0);
}

#[cfg(test)]
mod tests {
    use mailcraft_smime::CertInfo;

    use super::*;

    fn valid() -> Signature {
        Signature::Valid {
            signer: CertInfo { name: "Sanne".into(), emails: vec!["sanne@example.org".into()], issuer: "CA".into(), not_before: 0, not_after: 0 },
            certificate: Vec::new(),
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
    }
}
