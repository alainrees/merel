//! The Backstage (the File tab): accounts, and the form that adds one.

use egui::{Margin, RichText, Stroke, Ui, vec2};
use zeroize::Zeroize;

use mailcraft_engine::{AccountStatus, signin};
use mailcraft_model::{Account, Address, Protocol, Server, Tls};
use mailcraft_secrets::Secret;

use crate::{MailApp, Tab, Tokens};

/// What the person has typed so far. The password is wiped when the form is dropped.
pub struct AccountForm {
    pub kind: Protocol,
    pub name: String,
    pub email: String,
    pub username: String,
    pub password: String,
    pub imap_host: String,
    pub imap_port: String,
    pub imap_tls: Tls,
    pub smtp_host: String,
    pub smtp_port: String,
    pub smtp_tls: Tls,
    /// OAuth registration (Gmail, Microsoft 365).
    pub client_id: String,
    pub client_secret: String,
    pub error: Option<String>,
}

impl Default for AccountForm {
    fn default() -> Self {
        AccountForm {
            kind: Protocol::ImapSmtp,
            name: String::new(),
            email: String::new(),
            username: String::new(),
            password: String::new(),
            imap_host: String::new(),
            imap_port: "993".into(),
            imap_tls: Tls::Implicit,
            smtp_host: String::new(),
            smtp_port: "465".into(),
            smtp_tls: Tls::Implicit,
            client_id: String::new(),
            client_secret: String::new(),
            error: None,
        }
    }
}

impl Drop for AccountForm {
    fn drop(&mut self) {
        self.password.zeroize();
        self.client_secret.zeroize();
    }
}

fn port(s: &str, what: &str) -> Result<u16, String> {
    s.trim().parse::<u16>().ok().filter(|p| *p > 0).ok_or_else(|| format!("Vul een geldige poort in voor {what} (1 tot 65535)."))
}

fn host(s: &str, what: &str) -> Result<String, String> {
    let h = s.trim().to_ascii_lowercase();
    let ok = !h.is_empty()
        && h.len() <= 253
        && h.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
        && !h.starts_with('.')
        && !h.ends_with('.');
    if ok { Ok(h) } else { Err(format!("Vul een geldige servernaam in voor {what}, bijvoorbeeld imap.voorbeeld.nl.")) }
}

impl AccountForm {
    /// Guesses the usual server names from the address's domain.
    pub fn suggest(&mut self) {
        if let Some((_, domain)) = self.email.trim().split_once('@') {
            let domain = domain.trim();
            if self.imap_host.trim().is_empty() {
                self.imap_host = format!("imap.{domain}");
            }
            if self.smtp_host.trim().is_empty() {
                self.smtp_host = format!("smtp.{domain}");
            }
        }
        if self.username.trim().is_empty() {
            self.username = self.email.trim().to_string();
        }
    }

    /// The account the form describes, or what is wrong with it. The password stays out.
    pub fn account(&self) -> Result<Account, String> {
        let address = Address::new(Some(self.name.trim()), self.email.trim()).map_err(|_| "Vul een geldig e-mailadres in.".to_string())?;
        let username = if self.username.trim().is_empty() { address.email().to_string() } else { self.username.trim().to_string() };
        if username.chars().any(|c| c.is_control() || c == '"' || c == '\\') {
            return Err("De gebruikersnaam bevat tekens die niet zijn toegestaan.".into());
        }
        if self.password.is_empty() {
            return Err("Vul het wachtwoord in.".into());
        }
        let incoming =
            Server { host: host(&self.imap_host, "ontvangen (IMAP)")?, port: port(&self.imap_port, "ontvangen (IMAP)")?, tls: self.imap_tls };
        let outgoing =
            Server { host: host(&self.smtp_host, "verzenden (SMTP)")?, port: port(&self.smtp_port, "verzenden (SMTP)")?, tls: self.smtp_tls };
        let id = format!(
            "{}-{}",
            address.email().replace(['@', '.'], "-"),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
        );
        Ok(Account {
            id,
            name: if self.name.trim().is_empty() { address.email().to_string() } else { self.name.trim().to_string() },
            address,
            username,
            protocol: Protocol::ImapSmtp,
            auth: mailcraft_model::Auth::Password,
            incoming: Some(incoming),
            outgoing: Some(outgoing),
        })
    }
}

pub fn show(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    egui::Panel::left("backstage_nav")
        .exact_size(200.0)
        .resizable(false)
        .frame(egui::Frame::NONE.fill(t.accent).inner_margin(Margin::symmetric(0, 12)))
        .show(ui, |ui| {
            let item = |ui: &mut Ui, label: &str| {
                ui.add_sized([ui.available_width(), 34.0], egui::Button::new(RichText::new(label).size(15.0).color(t.on_accent)).frame(false))
                    .clicked()
            };
            if item(ui, "←  Terug") {
                app.tab = Tab::Home;
            }
            ui.add_space(12.0);
            ui.label(RichText::new("   Accounts").size(13.0).color(t.on_accent.gamma_multiply(0.8)));
        });
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.reading).inner_margin(Margin::same(28))).show(ui, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label(RichText::new("Accounts").size(26.0).color(t.text));
            ui.add_space(12.0);
            let statuses: Vec<(String, String, AccountStatus)> = app
                .engine
                .session
                .accounts
                .iter()
                .map(|a| {
                    (a.name.clone(), a.address.email().to_string(), app.engine.session.status.get(&a.id).cloned().unwrap_or(AccountStatus::Offline))
                })
                .collect();
            if app.engine.session.is_sample() {
                ui.label(RichText::new("Er is nog geen account. Je ziet voorbeeldmail.").color(t.text_dim));
            }
            for (name, email, status) in statuses {
                if email == "jij@example.org" {
                    continue;
                }
                egui::Frame::NONE.fill(t.panel).stroke(Stroke::new(1.0, t.border)).inner_margin(Margin::same(12)).show(ui, |ui| {
                    ui.set_width(ui.available_width().min(560.0));
                    ui.label(RichText::new(name).strong().color(t.text));
                    ui.label(RichText::new(email).color(t.text_dim));
                    let (txt, col) = match status {
                        AccountStatus::Online => ("Verbonden".to_string(), t.accent_text),
                        AccountStatus::Connecting => ("Verbinden…".to_string(), t.text_dim),
                        AccountStatus::Offline => ("Niet verbonden".to_string(), t.text_dim),
                        AccountStatus::Error(e) => (e, t.flag),
                    };
                    ui.label(RichText::new(txt).size(12.0).color(col));
                });
                ui.add_space(8.0);
            }
            ui.add_space(16.0);
            ui.label(RichText::new("Account toevoegen").size(20.0).color(t.text));
            ui.label(
                RichText::new("IMAP en SMTP. Je wachtwoord gaat in de sleutelhanger van je computer, niet in een bestand.")
                    .size(12.0)
                    .color(t.text_dim),
            );
            ui.add_space(8.0);
            form(app, ui, t);
        });
    });
}

fn field(ui: &mut Ui, t: &Tokens, label: &str, value: &mut String, password: bool) {
    ui.horizontal(|ui| {
        ui.add_sized([150.0, 24.0], egui::Label::new(RichText::new(label).color(t.text)));
        ui.add(egui::TextEdit::singleline(value).password(password).desired_width(300.0));
    });
}

fn tls_picker(ui: &mut Ui, t: &Tokens, id: &str, tls: &mut Tls) {
    ui.horizontal(|ui| {
        ui.add_sized([150.0, 24.0], egui::Label::new(RichText::new("Versleuteling").color(t.text)));
        ui.push_id(id, |ui| {
            ui.radio_value(tls, Tls::Implicit, "TLS");
            ui.radio_value(tls, Tls::StartTls, "STARTTLS");
        });
    });
}

fn form(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    ui.horizontal(|ui| {
        ui.add_sized([150.0, 24.0], egui::Label::new(RichText::new("Soort account").color(t.text)));
        ui.radio_value(&mut app.form.kind, Protocol::ImapSmtp, "IMAP/SMTP");
        ui.radio_value(&mut app.form.kind, Protocol::Gmail, "Gmail");
        ui.radio_value(&mut app.form.kind, Protocol::Microsoft365, "Microsoft 365");
    });
    ui.add_space(6.0);
    match app.form.kind {
        Protocol::ImapSmtp | Protocol::Exchange => password_form(app, ui, t),
        Protocol::Gmail | Protocol::Microsoft365 => oauth_form(app, ui, t),
    }
}

fn password_form(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let f = &mut app.form;
    field(ui, t, "Naam", &mut f.name, false);
    field(ui, t, "E-mailadres", &mut f.email, false);
    ui.horizontal(|ui| {
        ui.add_space(154.0);
        if ui.small_button("Servers invullen op basis van het adres").clicked() {
            f.suggest();
        }
    });
    field(ui, t, "Gebruikersnaam", &mut f.username, false);
    field(ui, t, "Wachtwoord", &mut f.password, true);
    ui.add_space(8.0);
    ui.label(RichText::new("Ontvangen (IMAP)").strong().color(t.text));
    field(ui, t, "Server", &mut f.imap_host, false);
    field(ui, t, "Poort", &mut f.imap_port, false);
    tls_picker(ui, t, "imap_tls", &mut f.imap_tls);
    ui.add_space(8.0);
    ui.label(RichText::new("Verzenden (SMTP)").strong().color(t.text));
    field(ui, t, "Server", &mut f.smtp_host, false);
    field(ui, t, "Poort", &mut f.smtp_port, false);
    tls_picker(ui, t, "smtp_tls", &mut f.smtp_tls);
    ui.add_space(12.0);
    if let Some(e) = &f.error {
        ui.label(RichText::new(e).color(t.flag));
    }
    let add = ui.add_sized([220.0, 30.0], egui::Button::new(RichText::new("Toevoegen en verbinden").color(t.on_accent)).fill(t.accent)).clicked();
    ui.add_space(4.0);
    ui.allocate_space(vec2(0.0, 0.0));
    if add {
        match app.form.account() {
            Ok(account) => {
                let password = Secret::new(app.form.password.as_str());
                match app.engine.add_account(account, password) {
                    Ok(()) => {
                        app.form = AccountForm::default();
                        app.tab = Tab::Home;
                    }
                    Err(e) => app.form.error = Some(e),
                }
            }
            Err(e) => app.form.error = Some(e),
        }
    }
}

fn oauth_form(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    // A finished sign-in closes the form; a failed one shows why.
    match app.engine.session.sign_in.take() {
        Some(Some(Ok(()))) => {
            app.form = AccountForm::default();
            app.tab = Tab::Home;
            return;
        }
        Some(Some(Err(e))) => app.form.error = Some(e),
        Some(None) => app.engine.session.sign_in = Some(None),
        None => {}
    }
    let busy = app.engine.session.sign_in.is_some();
    let (provider, secret_needed, help) = match app.form.kind {
        Protocol::Gmail => (
            "Google",
            true,
            "Maak in de Google Cloud Console een OAuth-client van het type Desktop app en vul de client-ID en het client-secret in. Zie docs/accounts.md.",
        ),
        _ => (
            "Microsoft",
            false,
            "Registreer de app in Microsoft Entra (public client, redirect http://localhost) en vul de Application (client) ID in. Zie docs/accounts.md.",
        ),
    };
    ui.label(RichText::new(help).size(12.0).color(t.text_dim));
    ui.add_space(6.0);
    let f = &mut app.form;
    ui.add_enabled_ui(!busy, |ui| {
        field(ui, t, "Naam", &mut f.name, false);
        field(ui, t, "E-mailadres", &mut f.email, false);
        field(ui, t, "Client-ID", &mut f.client_id, false);
        if secret_needed {
            field(ui, t, "Client-secret", &mut f.client_secret, true);
        }
    });
    ui.add_space(12.0);
    if let Some(e) = &f.error {
        ui.label(RichText::new(e).color(t.flag));
    }
    let label = if busy { "Wacht op de browser…".to_string() } else { format!("Inloggen bij {provider}") };
    let click =
        ui.add_enabled(!busy, egui::Button::new(RichText::new(label).color(t.on_accent)).fill(t.accent).min_size(vec2(220.0, 30.0))).clicked();
    if busy {
        ui.label(
            RichText::new("Log in in het browservenster dat is geopend. Daarna verschijnt het account hier vanzelf.").size(12.0).color(t.text_dim),
        );
    }
    if click {
        match signin::account(app.form.kind, &app.form.name, &app.form.email, &app.form.client_id) {
            Ok(account) => {
                let client_secret = (secret_needed && !app.form.client_secret.trim().is_empty()).then(|| Secret::new(app.form.client_secret.trim()));
                if secret_needed && client_secret.is_none() {
                    app.form.error = Some("Vul het client-secret in dat Google bij de OAuth-client toonde.".into());
                    return;
                }
                app.form.error = None;
                app.engine.sign_in(signin::SignIn { account, client_secret });
            }
            Err(e) => app.form.error = Some(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_servers_and_validates() {
        let mut f = AccountForm::default();
        f.email = "alain@voorbeeld.nl".into();
        f.password = "x".into();
        f.suggest();
        assert_eq!(f.imap_host, "imap.voorbeeld.nl");
        assert_eq!(f.username, "alain@voorbeeld.nl");
        let a = f.account().unwrap();
        assert_eq!(a.incoming.unwrap(), Server { host: "imap.voorbeeld.nl".into(), port: 993, tls: Tls::Implicit });
        f.imap_port = "0".into();
        assert!(f.account().is_err());
        f.imap_port = "993".into();
        f.imap_host = "bad host".into();
        assert!(f.account().is_err());
        f.imap_host = "imap.voorbeeld.nl".into();
        f.password.clear();
        assert!(f.account().is_err());
    }

    #[test]
    fn oauth_kinds_show_a_browser_button() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut app = MailApp::new(mailcraft_engine::Engine::sample());
        app.tab = Tab::File;
        app.form.kind = Protocol::Gmail;
        let mut harness = Harness::builder().with_size(vec2(1100.0, 760.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app);
        harness.run();
        harness.get_by_label("Inloggen bij Google");
        harness.get_by_label("Client-secret");
        harness.state_mut().form.kind = Protocol::Microsoft365;
        harness.run();
        harness.get_by_label("Inloggen bij Microsoft");
        assert!(harness.query_by_label("Client-secret").is_none());
        harness.get_by_label("Inloggen bij Microsoft").click();
        harness.run();
        assert_eq!(harness.state().form.error.as_deref(), Some("Vul een geldig e-mailadres in."));
    }
}
