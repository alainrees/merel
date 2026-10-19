//! The S/MIME certificate of an account, on its card in the Backstage: what it signs as,
//! importing a .p12 or .pfx file with its password, and removing it.

use egui::{RichText, Ui};
use mailcraft_smime::CertInfo;
use zeroize::{Zeroize, Zeroizing};

use crate::{MailApp, Tokens};

pub const IMPORT: &str = "Certificaat importeren…";
pub const REPLACE: &str = "Ander certificaat…";
pub const REMOVE: &str = "Certificaat verwijderen";
pub const CONFIRM: &str = "Importeren";
pub const PASSWORD: &str = "Wachtwoord van het bestand";

/// A PKCS#12 file picked for an account, waiting for its password. Both are wiped when the
/// import is done or cancelled.
pub struct Import {
    pub account_id: String,
    pub file: String,
    pub bytes: Zeroizing<Vec<u8>>,
    pub password: String,
    pub error: Option<String>,
}

impl Drop for Import {
    fn drop(&mut self) {
        self.password.zeroize();
    }
}

pub enum Action {
    Pick(String),
    Import,
    Cancel,
    Remove(String),
}

/// The certificate part of the card of `account_id`.
pub fn section(ui: &mut Ui, t: &Tokens, import: &mut Option<Import>, account_id: &str, info: Option<&CertInfo>) -> Option<Action> {
    let mut action = None;
    ui.add_space(6.0);
    ui.label(RichText::new("Certificaat (S/MIME)").strong().color(t.text));
    match info {
        Some(info) => {
            ui.label(
                RichText::new(format!(
                    "Nieuwe mail wordt digitaal ondertekend als {} <{}>. Uitgegeven door {}, geldig tot {}.",
                    info.name,
                    info.emails.join(", "),
                    if info.issuer.is_empty() { "onbekend" } else { info.issuer.as_str() },
                    crate::dates::day(info.not_after)
                ))
                .size(12.0)
                .color(t.text_dim),
            );
        }
        None => {
            ui.label(
                RichText::new(
                    "Met een certificaat onderteken je mail digitaal: de ontvanger ziet dat het bericht van jou komt en onderweg niet is veranderd.",
                )
                .size(12.0)
                .color(t.text_dim),
            );
        }
    }
    match import.as_mut().filter(|i| i.account_id == account_id) {
        Some(pending) => {
            ui.label(RichText::new(format!("Bestand: {}", pending.file)).size(12.0).color(t.text));
            ui.horizontal(|ui| {
                ui.label(RichText::new(PASSWORD).color(t.text));
                let field = ui.add(egui::TextEdit::singleline(&mut pending.password).password(true).desired_width(200.0));
                if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    action = Some(Action::Import);
                }
            });
            if let Some(e) = &pending.error {
                ui.label(RichText::new(e).color(t.flag));
            }
            ui.horizontal(|ui| {
                if ui.button(CONFIRM).clicked() {
                    action = Some(Action::Import);
                }
                if ui.button("Annuleren").clicked() {
                    action = Some(Action::Cancel);
                }
            });
        }
        None => {
            ui.horizontal(|ui| {
                if ui
                    .button(if info.is_some() { REPLACE } else { IMPORT })
                    .on_hover_text(
                        "Kies een .p12- of .pfx-bestand met je certificaat en privésleutel, van je organisatie of een certificaatverstrekker.",
                    )
                    .clicked()
                {
                    action = Some(Action::Pick(account_id.to_string()));
                }
                if info.is_some()
                    && ui
                        .button(REMOVE)
                        .on_hover_text("Mail van dit account wordt niet meer ondertekend. Het bestand waaruit je importeerde blijft bestaan.")
                        .clicked()
                {
                    action = Some(Action::Remove(account_id.to_string()));
                }
            });
        }
    }
    action
}

impl MailApp {
    pub fn certificate_action(&mut self, action: Action) {
        match action {
            Action::Pick(account_id) => {
                let picked =
                    rfd::FileDialog::new().set_title("Certificaat kiezen").add_filter("Certificaat met sleutel", &["p12", "pfx"]).pick_file();
                if let Some(path) = picked {
                    self.pick_certificate(&account_id, &path);
                }
            }
            Action::Import => {
                let Some(pending) = self.form.certificate.as_mut() else { return };
                match self.engine.import_certificate(&pending.account_id, &pending.bytes, &pending.password) {
                    Ok(info) => {
                        self.form.certificate = None;
                        self.form.notice =
                            Some(format!("Het certificaat van {} is geïmporteerd; nieuwe mail van dit account wordt ondertekend.", info.name));
                    }
                    Err(e) => {
                        pending.password.zeroize();
                        pending.error = Some(e);
                    }
                }
            }
            Action::Cancel => self.form.certificate = None,
            Action::Remove(account_id) => {
                self.engine.remove_certificate(&account_id);
                self.form.notice = Some("Het certificaat is verwijderd; mail van dit account wordt niet meer ondertekend.".into());
            }
        }
    }

    /// Reads the file at `path` for `account_id`; the password is asked next.
    pub fn pick_certificate(&mut self, account_id: &str, path: &std::path::Path) {
        let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let read = std::fs::metadata(path)
            .ok()
            .filter(|m| m.len() <= mailcraft_engine::certificates::MAX_PKCS12 as u64)
            .and_then(|_| std::fs::read(path).ok());
        let (bytes, error) = match read {
            Some(bytes) => (bytes, None),
            None => (Vec::new(), Some("Dit bestand kan niet worden gelezen, of is te groot voor een certificaat.".to_string())),
        };
        self.form.certificate =
            Some(Import { account_id: account_id.to_string(), file, bytes: Zeroizing::new(bytes), password: String::new(), error });
    }
}

#[cfg(test)]
mod tests {
    use egui::vec2;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use mailcraft_secrets::Secret;

    use super::*;
    use crate::Tab;
    use crate::backstage::AccountForm;

    const P12: &[u8] = include_bytes!("../../smime/testdata/alain.p12");

    fn app() -> MailApp {
        let mut engine = mailcraft_engine::Engine::sample();
        let mut f = AccountForm::default();
        f.email = "alain@voorbeeld.nl".into();
        f.password = "x".into();
        f.suggest();
        engine.add_account(f.account().unwrap(), Secret::new("x")).unwrap();
        let mut app = MailApp::new(engine);
        app.tab = Tab::File;
        app
    }

    #[test]
    fn a_certificate_is_imported_with_its_password_and_removed_again() {
        let mut harness = Harness::builder().with_size(vec2(1100.0, 1000.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app());
        harness.run();
        harness.get_by_label(IMPORT);
        assert!(harness.query_by_label(REMOVE).is_none());
        let id = harness.state().engine.session.accounts[0].id.clone();
        let file = std::env::temp_dir().join(format!("mailcraft-cert-{}.p12", std::process::id()));
        std::fs::write(&file, P12).unwrap();
        harness.state_mut().pick_certificate(&id, &file);
        let _ = std::fs::remove_file(&file);
        harness.run();
        harness.get_by_label(&format!("Bestand: mailcraft-cert-{}.p12", std::process::id()));
        // A wrong password says so and is wiped; the file stays picked.
        if let Some(i) = harness.state_mut().form.certificate.as_mut() {
            i.password = "fout".into();
        }
        harness.get_by_label(CONFIRM).click();
        harness.run();
        harness.get_by_label("Het wachtwoord klopt niet, of dit is geen PKCS#12-bestand (.p12 of .pfx).");
        assert_eq!(harness.state().form.certificate.as_ref().map(|i| i.password.as_str()), Some(""));
        if let Some(i) = harness.state_mut().form.certificate.as_mut() {
            i.password = "geheim".into();
        }
        harness.get_by_label(CONFIRM).click();
        harness.run();
        assert!(harness.state().form.certificate.is_none());
        assert_eq!(harness.state().engine.certificate(&id).map(|c| c.name), Some("Alain Tester".into()));
        assert!(harness.query_all_by_label_contains("ondertekend als Alain Tester <alain@voorbeeld.nl>").count() == 1);

        // New mail from the account is signed, and the box can be unticked for one message.
        harness.state_mut().open_compose(crate::compose::Kind::New);
        harness.run();
        harness.get_by_label(crate::compose::SIGN).click();
        harness.run();
        assert_eq!(harness.state().compose.as_ref().and_then(|c| c.sign), Some(false));
        assert_eq!(harness.state().compose.as_ref().map(|c| c.as_stored().sign), Some(Some(false)), "kept with the draft");

        // Encrypting waits for the recipient's certificate, and says whose is missing.
        harness.get_by_label(crate::compose::ENCRYPT).click();
        if let Some(c) = harness.state_mut().compose.as_mut() {
            c.to = "jij@example.org".into();
            c.subject = "Contract".into();
        }
        harness.run();
        let missing = "Van jij@example.org is nog geen certificaat bekend";
        assert_eq!(harness.query_all_by_label_contains(missing).count(), 1);
        harness.get_by_label("Verzenden").click();
        harness.run();
        let c = harness.state().compose.as_ref().unwrap();
        assert!(c.draft_id.is_none() && c.error.as_deref().is_some_and(|e| e.contains(missing)), "not sent in the clear");
        assert!(c.as_stored().encrypt, "kept with the draft");
        harness.state_mut().compose = None;
        harness.run();

        harness.get_by_label(REMOVE).click();
        harness.run();
        assert!(harness.state().engine.certificate(&id).is_none());
        harness.get_by_label(IMPORT);
        harness.state_mut().open_compose(crate::compose::Kind::New);
        harness.run();
        assert!(harness.query_by_label(crate::compose::SIGN).is_none(), "without a certificate there is nothing to tick");
    }

    #[test]
    fn a_file_that_cannot_be_read_is_reported_and_cancelled() {
        let mut harness = Harness::builder().with_size(vec2(1100.0, 1000.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app());
        harness.run();
        let id = harness.state().engine.session.accounts[0].id.clone();
        harness.state_mut().pick_certificate(&id, std::path::Path::new("/bestaat/niet.p12"));
        harness.run();
        harness.get_by_label("Dit bestand kan niet worden gelezen, of is te groot voor een certificaat.");
        harness.get_by_label("Annuleren").click();
        harness.run();
        assert!(harness.state().form.certificate.is_none());
        harness.get_by_label(IMPORT);
    }
}
