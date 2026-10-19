//! The Backstage (the File tab): accounts, and the form that adds one.

use egui::{Margin, RichText, Stroke, Ui, vec2};
use zeroize::Zeroize;

use mailcraft_engine::{AccountStatus, signin};
use mailcraft_model::{Account, Address, Dav, Protocol, Server, Tls};
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
    /// CalDAV and CardDAV servers next to an IMAP account; empty means mail only.
    pub dav_calendar: String,
    pub dav_contacts: String,
    /// OAuth registration (Gmail, Microsoft 365).
    pub client_id: String,
    pub client_secret: String,
    pub error: Option<String>,
    /// How a sign-in started from the account list ended.
    pub notice: Option<String>,
    /// The account whose Remove button was clicked once; the second click removes it.
    pub confirm_remove: Option<String>,
    /// The account whose signature is being edited, with the text so far.
    pub signature_edit: Option<(String, String)>,
    /// The account the form is changing, when it is not adding one. The password may then stay
    /// empty, which keeps the one in the keychain.
    pub editing: Option<String>,
    /// Brings the form into view once, after Edit was clicked on an account further up.
    pub scroll_to_form: bool,
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
            dav_calendar: String::new(),
            dav_contacts: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            error: None,
            notice: None,
            confirm_remove: None,
            signature_edit: None,
            editing: None,
            scroll_to_form: false,
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
    /// The form filled with a password account, ready to change it; the password stays empty.
    pub fn editing(account: &Account) -> Self {
        let mut f = AccountForm::default();
        f.kind = account.protocol;
        f.editing = Some(account.id.clone());
        f.scroll_to_form = true;
        f.name = account.name.clone();
        f.email = account.address.email().to_string();
        f.username = account.username.clone();
        if let Some(incoming) = &account.incoming {
            f.imap_host = incoming.host.clone();
            f.imap_port = incoming.port.to_string();
            f.imap_tls = incoming.tls;
        }
        if let Some(outgoing) = &account.outgoing {
            f.smtp_host = outgoing.host.clone();
            f.smtp_port = outgoing.port.to_string();
            f.smtp_tls = outgoing.tls;
        }
        if let Some(dav) = &account.dav {
            f.dav_calendar = dav.calendar.clone();
            f.dav_contacts = dav.contacts.clone().unwrap_or_default();
        }
        f
    }

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
            if self.dav_calendar.trim().is_empty() {
                let (calendar, contacts) = match domain.to_ascii_lowercase().as_str() {
                    "fastmail.com" | "fastmail.fm" => ("https://caldav.fastmail.com/", "https://carddav.fastmail.com/"),
                    "icloud.com" | "me.com" | "mac.com" => ("https://caldav.icloud.com/", "https://contacts.icloud.com/"),
                    "posteo.de" | "posteo.net" => ("https://posteo.de:8443/", ""),
                    "mailbox.org" => ("https://dav.mailbox.org/", ""),
                    _ => ("", ""),
                };
                self.dav_calendar = calendar.to_string();
                self.dav_contacts = contacts.to_string();
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
        // Exchange logs in as DOMAIN\user; IMAP quotes the name, so a backslash stays out there.
        let backslash_ok = self.kind == Protocol::Exchange;
        if username.chars().any(|c| c.is_control() || c == '"' || (c == '\\' && !backslash_ok)) {
            return Err("De gebruikersnaam bevat tekens die niet zijn toegestaan.".into());
        }
        if self.password.is_empty() && self.editing.is_none() {
            return Err("Vul het wachtwoord in.".into());
        }
        let (protocol, incoming, outgoing) = if self.kind == Protocol::Exchange {
            (Protocol::Exchange, Server { host: host(&self.imap_host, "Exchange")?, port: 443, tls: Tls::Implicit }, None)
        } else {
            (
                Protocol::ImapSmtp,
                Server { host: host(&self.imap_host, "ontvangen (IMAP)")?, port: port(&self.imap_port, "ontvangen (IMAP)")?, tls: self.imap_tls },
                Some(Server {
                    host: host(&self.smtp_host, "verzenden (SMTP)")?,
                    port: port(&self.smtp_port, "verzenden (SMTP)")?,
                    tls: self.smtp_tls,
                }),
            )
        };
        let dav = if protocol == Protocol::ImapSmtp && !self.dav_calendar.trim().is_empty() {
            Some(
                Dav::new(&self.dav_calendar, &self.dav_contacts)
                    .map_err(|_| "Vul voor agenda en adresboek een https-adres in, bijvoorbeeld https://caldav.fastmail.com/.".to_string())?,
            )
        } else {
            if !self.dav_contacts.trim().is_empty() && protocol == Protocol::ImapSmtp {
                return Err("Vul ook het agenda-adres (CalDAV) in; het adresboek alleen is niet genoeg.".into());
            }
            None
        };
        let id = match &self.editing {
            Some(id) => id.clone(),
            None => format!(
                "{}-{}",
                address.email().replace(['@', '.'], "-"),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
            ),
        };
        Ok(Account {
            id,
            name: if self.name.trim().is_empty() { address.email().to_string() } else { self.name.trim().to_string() },
            address,
            username,
            protocol,
            auth: mailcraft_model::Auth::Password,
            incoming: Some(incoming),
            outgoing,
            dav,
            signature: String::new(),
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
            ui.add_space(12.0);
            // Printing lives here, as in Outlook's File menu; Ctrl+P on the Start tab does the same.
            if item(ui, "Afdrukken") {
                if app.module == crate::Module::Calendar {
                    app.print_calendar();
                } else if app.can_print() {
                    app.print_selected();
                } else {
                    app.notice = Some("Open eerst een bericht om af te drukken.".into());
                }
                app.tab = Tab::Home;
            }
            // Outlook's "Opslaan als": the open message as an .eml file in Downloads; Ctrl+S too.
            if item(ui, crate::eml::SAVE_AS) {
                app.save_as_file();
                app.tab = Tab::Home;
            }
            if item(ui, "Ongewenste e-mail") {
                app.junk = Some(crate::junk::Options::default());
                app.tab = Tab::Home;
            }
        });
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.reading).inner_margin(Margin::same(28))).show(ui, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label(RichText::new("Accounts").size(26.0).color(t.text));
            ui.add_space(12.0);
            // A sign-in started from this list reports here; the OAuth form consumes its own.
            let oauth_form = matches!(app.form.kind, Protocol::Gmail | Protocol::Microsoft365);
            if !oauth_form {
                match app.engine.session.sign_in.take() {
                    Some(Some(Ok(()))) => app.form.notice = Some("Opnieuw aangemeld; het account verbindt opnieuw.".into()),
                    Some(Some(Err(e))) => app.form.notice = Some(e),
                    Some(None) => app.engine.session.sign_in = Some(None),
                    None => {}
                }
            }
            let busy = app.engine.session.sign_in.is_some();
            let statuses: Vec<(String, String, String, bool, AccountStatus, String)> = app
                .engine
                .session
                .accounts
                .iter()
                .map(|a| {
                    (
                        a.id.clone(),
                        a.name.clone(),
                        a.address.email().to_string(),
                        matches!(a.auth, mailcraft_model::Auth::OAuth2 { .. }),
                        app.engine.session.status.get(&a.id).cloned().unwrap_or(AccountStatus::Offline),
                        a.signature.clone(),
                    )
                })
                .collect();
            if app.engine.session.is_sample() {
                ui.label(RichText::new("Er is nog geen account. Je ziet voorbeeldmail.").color(t.text_dim));
            }
            if let Some(n) = &app.form.notice {
                ui.label(RichText::new(n).color(t.text_dim));
                ui.add_space(8.0);
            }
            let mut again = None;
            let mut edit = None;
            let mut remove = None;
            let confirming = app.form.confirm_remove.clone();
            let mut save_signature = None;
            for (id, name, email, oauth, status, signature) in statuses {
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
                    ui.add_space(4.0);
                    if oauth {
                        let label = if busy { "Wacht op de browser…" } else { "Opnieuw aanmelden" };
                        if ui
                            .add_enabled(!busy, egui::Button::new(label))
                            .on_hover_text("Log opnieuw in via de browser, bijvoorbeeld om toestemming te geven voor agenda, contacten en taken.")
                            .clicked()
                        {
                            again = Some(id.clone());
                        }
                    } else if ui
                        .add_enabled(app.form.editing.as_deref() != Some(id.as_str()), egui::Button::new("Account bewerken"))
                        .on_hover_text("Verander de naam, de servers, de gebruikersnaam of het wachtwoord; het account verbindt daarna opnieuw.")
                        .clicked()
                    {
                        edit = Some(id.clone());
                    }
                    ui.add_space(6.0);
                    let editing = app.form.signature_edit.as_ref().is_some_and(|(e, _)| *e == id);
                    if editing {
                        ui.label(RichText::new("Handtekening").strong().color(t.text));
                        if let Some((_, text)) = app.form.signature_edit.as_mut() {
                            ui.add(
                                egui::TextEdit::multiline(text).desired_rows(4).desired_width(f32::INFINITY).hint_text("Met vriendelijke groet,\n…"),
                            );
                        }
                        ui.label(
                            RichText::new("Komt onder elk nieuw bericht, antwoord en doorgestuurd bericht van dit account.")
                                .size(12.0)
                                .color(t.text_dim),
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Handtekening opslaan").clicked() {
                                save_signature = app.form.signature_edit.take();
                            }
                            if ui.button("Annuleren").clicked() {
                                app.form.signature_edit = None;
                            }
                        });
                    } else {
                        if !signature.is_empty() {
                            ui.label(RichText::new(format!("Handtekening: {}", signature.lines().next().unwrap_or(""))).size(12.0).color(t.text_dim));
                        }
                        if ui.button(if signature.is_empty() { "Handtekening instellen" } else { "Handtekening bewerken" }).clicked() {
                            app.form.signature_edit = Some((id.clone(), signature.clone()));
                        }
                    }
                    ui.add_space(4.0);
                    if confirming.as_deref() == Some(id.as_str()) {
                        ui.label(
                            RichText::new(
                                "Dit account, het wachtwoord en de offline kopie verdwijnen van deze computer. Je mail blijft op de server.",
                            )
                            .size(12.0)
                            .color(t.text),
                        );
                        ui.horizontal(|ui| {
                            if ui.add(egui::Button::new(RichText::new("Ja, verwijderen").color(t.on_accent)).fill(t.flag)).clicked() {
                                remove = Some(id.clone());
                            }
                            if ui.button("Annuleren").clicked() {
                                app.form.confirm_remove = None;
                            }
                        });
                    } else if ui
                        .button("Account verwijderen")
                        .on_hover_text("Haalt het account van deze computer; vraagt eerst om bevestiging.")
                        .clicked()
                    {
                        app.form.confirm_remove = Some(id.clone());
                    }
                });
                ui.add_space(8.0);
            }
            if let Some(id) = again {
                app.form.notice = None;
                app.engine.sign_in_again(&id);
            }
            if let Some(account) = edit.and_then(|id| app.engine.session.accounts.iter().find(|a| a.id == id)) {
                app.form = AccountForm::editing(account);
            }
            if let Some((id, text)) = save_signature {
                app.form.notice = match app.engine.set_signature(&id, &text) {
                    Ok(()) => Some("De handtekening is opgeslagen.".into()),
                    Err(e) => Some(e),
                };
            }
            if let Some(id) = remove {
                if app.form.editing.as_deref() == Some(id.as_str()) {
                    app.form = AccountForm::default();
                }
                app.form.confirm_remove = None;
                app.form.notice = Some(match app.engine.remove_account(&id) {
                    Ok(()) => "Het account is verwijderd.".into(),
                    Err(e) => e,
                });
            }
            ui.add_space(16.0);
            let (heading, help) = if app.form.editing.is_some() {
                (
                    format!("Account bewerken: {}", app.form.email),
                    "Verander wat nodig is en sla op; het account verbindt daarna opnieuw met de nieuwe gegevens.",
                )
            } else {
                ("Account toevoegen".to_string(), "IMAP en SMTP. Je wachtwoord gaat in de sleutelhanger van je computer, niet in een bestand.")
            };
            let heading = ui.label(RichText::new(heading).size(20.0).color(t.text));
            if std::mem::take(&mut app.form.scroll_to_form) {
                heading.scroll_to_me(Some(egui::Align::TOP));
            }
            ui.label(RichText::new(help).size(12.0).color(t.text_dim));
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
        if app.form.editing.is_some() {
            // The kind of an account is fixed; its servers and sign-in are what change.
            ui.label(RichText::new(if app.form.kind == Protocol::Exchange { "Exchange" } else { "IMAP/SMTP" }).color(t.text));
        } else {
            ui.radio_value(&mut app.form.kind, Protocol::ImapSmtp, "IMAP/SMTP");
            ui.radio_value(&mut app.form.kind, Protocol::Gmail, "Gmail");
            ui.radio_value(&mut app.form.kind, Protocol::Microsoft365, "Microsoft 365");
            ui.radio_value(&mut app.form.kind, Protocol::Exchange, "Exchange");
        }
    });
    ui.add_space(6.0);
    match app.form.kind {
        Protocol::ImapSmtp | Protocol::Exchange => password_form(app, ui, t),
        Protocol::Gmail | Protocol::Microsoft365 => oauth_form(app, ui, t),
    }
}

fn password_form(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let f = &mut app.form;
    let exchange = f.kind == Protocol::Exchange;
    if exchange {
        ui.label(
            RichText::new("Voor Exchange Server 2013 of nieuwer, via Exchange Web Services (EWS) over HTTPS. Verzenden gaat via dezelfde server. Zie docs/accounts.md.")
                .size(12.0)
                .color(t.text_dim),
        );
        ui.add_space(6.0);
    }
    field(ui, t, "Naam", &mut f.name, false);
    field(ui, t, "E-mailadres", &mut f.email, false);
    if !exchange {
        ui.horizontal(|ui| {
            ui.add_space(154.0);
            if ui.small_button("Servers invullen op basis van het adres").clicked() {
                f.suggest();
            }
        });
    }
    field(ui, t, "Gebruikersnaam", &mut f.username, false);
    field(ui, t, "Wachtwoord", &mut f.password, true);
    if f.editing.is_some() {
        ui.horizontal(|ui| {
            ui.add_space(154.0);
            ui.label(RichText::new("Laat leeg om het huidige wachtwoord te houden.").size(12.0).color(t.text_dim));
        });
    }
    ui.add_space(8.0);
    if exchange {
        ui.label(RichText::new("Exchange-server").strong().color(t.text));
        field(ui, t, "Server", &mut f.imap_host, false);
        ui.label(RichText::new("Bijvoorbeeld mail.bedrijf.nl; Mailcraft gebruikt https://<server>/EWS/Exchange.asmx. Gebruikersnaam als DOMEIN\\naam of als e-mailadres.").size(12.0).color(t.text_dim));
    } else {
        ui.label(RichText::new("Ontvangen (IMAP)").strong().color(t.text));
        field(ui, t, "Server", &mut f.imap_host, false);
        field(ui, t, "Poort", &mut f.imap_port, false);
        tls_picker(ui, t, "imap_tls", &mut f.imap_tls);
        ui.add_space(8.0);
        ui.label(RichText::new("Verzenden (SMTP)").strong().color(t.text));
        field(ui, t, "Server", &mut f.smtp_host, false);
        field(ui, t, "Poort", &mut f.smtp_port, false);
        tls_picker(ui, t, "smtp_tls", &mut f.smtp_tls);
        ui.add_space(8.0);
        ui.label(RichText::new("Agenda en adresboek (CalDAV/CardDAV), optioneel").strong().color(t.text));
        field(ui, t, "Agenda", &mut f.dav_calendar, false);
        field(ui, t, "Adresboek", &mut f.dav_contacts, false);
        ui.label(
            RichText::new("Het https-adres van de server, bijvoorbeeld https://caldav.fastmail.com/ of https://cloud.voorbeeld.nl/remote.php/dav/. Laat het adresboek leeg als het op dezelfde server staat.")
                .size(12.0)
                .color(t.text_dim),
        );
    }
    ui.add_space(12.0);
    if let Some(e) = &f.error {
        ui.label(RichText::new(e).color(t.flag));
    }
    let editing = f.editing.is_some();
    let label = if editing { "Opslaan en opnieuw verbinden" } else { "Toevoegen en verbinden" };
    let mut save = false;
    let mut cancel = false;
    ui.horizontal(|ui| {
        save = ui.add_sized([220.0, 30.0], egui::Button::new(RichText::new(label).color(t.on_accent)).fill(t.accent)).clicked();
        if editing {
            cancel = ui.add_sized([110.0, 30.0], egui::Button::new("Annuleren")).clicked();
        }
    });
    ui.add_space(4.0);
    ui.allocate_space(vec2(0.0, 0.0));
    if cancel {
        app.form = AccountForm::default();
        return;
    }
    if save {
        match app.form.account() {
            Ok(account) if editing => {
                let password = (!app.form.password.is_empty()).then(|| Secret::new(app.form.password.as_str()));
                match app.engine.update_account(account, password) {
                    Ok(()) => {
                        app.form = AccountForm::default();
                        app.form.notice = Some("Het account is opgeslagen en verbindt opnieuw.".into());
                    }
                    Err(e) => app.form.error = Some(e),
                }
            }
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
            "Registreer de app in Microsoft Entra (public client, redirect http://localhost, Microsoft Graph-machtigingen) en vul de Application (client) ID in. Zie docs/accounts.md.",
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
        assert!(f.dav_calendar.is_empty(), "an unknown domain gets no DAV guess");
        let a = f.account().unwrap();
        assert_eq!(a.incoming.unwrap(), Server { host: "imap.voorbeeld.nl".into(), port: 993, tls: Tls::Implicit });
        assert!(a.dav.is_none());
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
    fn dav_servers_are_suggested_and_checked() {
        let mut f = AccountForm::default();
        f.email = "alain@fastmail.com".into();
        f.password = "x".into();
        f.suggest();
        assert_eq!(f.dav_calendar, "https://caldav.fastmail.com/");
        assert_eq!(f.dav_contacts, "https://carddav.fastmail.com/");
        let a = f.account().unwrap();
        assert_eq!(a.dav, Some(Dav { calendar: "https://caldav.fastmail.com/".into(), contacts: Some("https://carddav.fastmail.com/".into()) }));
        f.dav_contacts.clear();
        assert_eq!(f.account().unwrap().dav.unwrap().contacts, None);
        f.dav_calendar = "http://caldav.fastmail.com/".into();
        assert!(f.account().is_err(), "plain http is refused");
        f.dav_calendar.clear();
        f.dav_contacts = "https://carddav.fastmail.com/".into();
        assert!(f.account().is_err(), "an address book without a calendar is refused");
    }

    #[test]
    fn exchange_accounts_take_one_server_and_a_domain_user() {
        let mut f = AccountForm::default();
        f.kind = Protocol::Exchange;
        f.email = "alain@bedrijf.nl".into();
        f.username = "BEDRIJF\\alain".into();
        f.password = "x".into();
        f.imap_host = "mail.bedrijf.nl".into();
        let a = f.account().unwrap();
        assert_eq!(a.protocol, Protocol::Exchange);
        assert_eq!(a.username, "BEDRIJF\\alain");
        assert_eq!(a.incoming.unwrap(), Server { host: "mail.bedrijf.nl".into(), port: 443, tls: Tls::Implicit });
        assert!(a.outgoing.is_none());
        f.kind = Protocol::ImapSmtp;
        assert!(f.account().is_err(), "a backslash in an IMAP user name is refused");
    }

    #[test]
    fn a_signature_is_set_from_the_account_list_and_lands_in_new_mail() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut engine = mailcraft_engine::Engine::sample();
        let mut f = AccountForm::default();
        f.email = "alain@voorbeeld.nl".into();
        f.password = "x".into();
        f.suggest();
        engine.add_account(f.account().unwrap(), Secret::new("x")).unwrap();
        let mut app = MailApp::new(engine);
        app.tab = Tab::File;
        let mut harness = Harness::builder().with_size(vec2(1100.0, 900.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app);
        harness.run();
        harness.get_by_label("Handtekening instellen").click();
        harness.run();
        if let Some((_, text)) = harness.state_mut().form.signature_edit.as_mut() {
            *text = "Groet,\nAlain\u{7}\n\n".into();
        }
        harness.get_by_label("Handtekening opslaan").click();
        harness.run();
        let account = harness.state().engine.session.accounts[0].clone();
        assert_eq!(account.signature, "Groet,\nAlain", "control characters and trailing blank lines go");
        assert!(harness.query_by_label("Handtekening bewerken").is_some());
        harness.state_mut().open_compose(crate::compose::Kind::New);
        assert_eq!(harness.state().compose.as_ref().map(|c| c.body.as_str()), Some("\n\n-- \nGroet,\nAlain"));
    }

    #[test]
    fn removing_an_account_asks_first_and_then_brings_the_sample_back() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut engine = mailcraft_engine::Engine::sample();
        let mut f = AccountForm::default();
        f.email = "alain@voorbeeld.nl".into();
        f.password = "x".into();
        f.suggest();
        engine.add_account(f.account().unwrap(), Secret::new("x")).unwrap();
        assert!(!engine.session.is_sample());
        let mut app = MailApp::new(engine);
        app.tab = Tab::File;
        let mut harness = Harness::builder().with_size(vec2(1100.0, 900.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app);
        harness.run();
        harness.get_by_label("Account verwijderen").click();
        harness.run();
        assert!(!harness.state().engine.session.is_sample(), "one click only asks");
        harness.get_by_label("Annuleren").click();
        harness.run();
        assert!(harness.query_by_label("Ja, verwijderen").is_none());
        harness.get_by_label("Account verwijderen").click();
        harness.run();
        harness.get_by_label("Ja, verwijderen").click();
        harness.run();
        assert!(harness.state().engine.session.is_sample(), "the sample mail is back");
        assert_eq!(harness.state().form.notice.as_deref(), Some("Het account is verwijderd."));
    }

    #[test]
    fn an_account_is_edited_from_the_list_and_keeps_its_mail() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut engine = mailcraft_engine::Engine::sample();
        let mut f = AccountForm::default();
        f.email = "alain@fastmail.com".into();
        f.password = "x".into();
        f.suggest();
        engine.add_account(f.account().unwrap(), Secret::new("x")).unwrap();
        engine.set_signature(&engine.session.accounts[0].id.clone(), "Groet").unwrap();
        let mut app = MailApp::new(engine);
        app.tab = Tab::File;
        let mut harness = Harness::builder().with_size(vec2(1100.0, 1200.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app);
        harness.run();
        harness.get_by_label("Account bewerken").click();
        harness.run();
        {
            let f = &harness.state().form;
            assert!(f.editing.is_some() && f.password.is_empty());
            assert_eq!(
                (f.imap_host.as_str(), f.imap_port.as_str(), f.dav_calendar.as_str()),
                ("imap.fastmail.com", "993", "https://caldav.fastmail.com/")
            );
            assert_eq!(f.dav_contacts, "https://carddav.fastmail.com/");
        }
        harness.get_by_label("Account bewerken: alain@fastmail.com");
        assert!(harness.query_by_label("Gmail").is_none(), "the kind cannot change");
        harness.get_by_label("Annuleren").click();
        harness.run();
        assert!(harness.state().form.editing.is_none());
        harness.get_by_label("Account bewerken").click();
        harness.run();
        {
            let f = &mut harness.state_mut().form;
            f.name = "Alain".into();
            f.imap_port = "143".into();
            f.imap_tls = Tls::StartTls;
            f.dav_calendar.clear();
            f.dav_contacts.clear();
        }
        harness.get_by_label("Opslaan en opnieuw verbinden").click();
        harness.run();
        let s = &harness.state().engine.session;
        assert_eq!(s.accounts.len(), 1);
        let a = &s.accounts[0];
        assert_eq!((a.name.as_str(), a.signature.as_str()), ("Alain", "Groet"));
        assert_eq!(a.incoming.clone().unwrap(), Server { host: "imap.fastmail.com".into(), port: 143, tls: Tls::StartTls });
        assert!(a.dav.is_none(), "cleared DAV servers mean mail only");
        assert_eq!(harness.state().form.notice.as_deref(), Some("Het account is opgeslagen en verbindt opnieuw."));
        assert!(harness.state().form.editing.is_none());
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
