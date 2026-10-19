//! The compose window: new mail, reply, reply all and forward, with attachments picked from
//! the computer's file dialog or dropped onto the window. "Later verzenden" puts the message
//! aside with a moment to go out, like Outlook's "Bezorging uitstellen"; until then it is a
//! draft on this computer, and the engine sends it when the app runs at that moment. Before a
//! message goes, the window looks for what Outlook warns about too: a text that speaks of an
//! attachment while none is on, and an empty subject; it then asks once, and sends on "Toch
//! verzenden".

use std::path::{Path, PathBuf};

use egui::{Margin, RichText, Stroke, Ui, vec2};

use mailcraft_engine::SendState;
use mailcraft_model::{Address, Draft, DraftAttachment, FolderKind, Importance, MAX_ATTACHMENTS, MessageHeader, StoredDraft};

use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};

use crate::{MailApp, Tokens, calendar, dates};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    New,
    Reply,
    ReplyAll,
    Forward,
}

pub struct Compose {
    pub to: String,
    pub cc: String,
    pub subject: String,
    pub body: String,
    pub in_reply_to: Option<String>,
    /// Files going along, in the order they were added.
    pub attachments: Vec<DraftAttachment>,
    /// The account the message goes out from.
    pub account_id: String,
    /// The draft put aside that this window continues, if any.
    pub stored: Option<String>,
    /// The fields as the window opened with them (addresses, subject, signature, quote);
    /// unchanged means nothing was written.
    initial: String,
    /// Set once Send is pressed; tracks the draft in the session's outbox.
    pub draft_id: Option<u64>,
    pub error: Option<String>,
    /// The moment the message goes out, when it was put aside with "Later verzenden".
    pub send_at: Option<i64>,
    /// Outlook's "Hoge urgentie" or "Lage urgentie"; normal unless chosen.
    pub importance: Importance,
    /// Outlook's "Leesbevestiging vragen": the reader's program is asked to say when the message
    /// was read.
    pub read_receipt: bool,
    /// What Send found worth a second look, with the moment chosen if "Later verzenden" was
    /// pressed; the window shows it and waits for "Toch verzenden" or a change to the message.
    pub check: Option<Check>,
}

/// Outlook's reminders before a message goes: "Je schrijft over een bijlage…" and "Het bericht
/// heeft geen onderwerp."
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub warnings: Vec<&'static str>,
    /// The "Later verzenden" moment the check held back, or None for a plain send.
    pub later: Option<i64>,
}

pub const NO_ATTACHMENT: &str = "Je schrijft over een bijlage, maar er zit er geen bij.";
pub const NO_SUBJECT: &str = "Het bericht heeft geen onderwerp.";
pub const SEND_ANYWAY: &str = "Toch verzenden";
pub const LOOK_AGAIN: &str = "Nog even nakijken";

/// Words that say an attachment is meant to be on, in Dutch and English.
const ATTACHMENT_WORDS: &[&str] = &[
    "bijlage",
    "bijlagen",
    "bijgevoegd",
    "bijgevoegde",
    "bijgesloten",
    "meegestuurd",
    "meegestuurde",
    "meegezonden",
    "aangehecht",
    "attached",
    "attachment",
    "attachments",
    "enclosed",
];

/// Whether the person's own text speaks of an attachment. Quoted lines, the signature and a
/// forwarded message underneath are left out: a reply to "zie bijlage" is not a promise of one.
pub fn mentions_attachment(text: &str) -> bool {
    text.lines()
        .take_while(|l| *l != "-- " && !l.starts_with("----------"))
        .filter(|l| !l.starts_with('>'))
        .flat_map(|l| l.split(|c: char| !c.is_alphanumeric()))
        .any(|w| ATTACHMENT_WORDS.contains(&w.to_lowercase().as_str()))
}

/// The moments "Later verzenden" offers, from `now`: in an hour, this (or tomorrow) evening at
/// eight, tomorrow at eight, and Monday at eight.
pub fn later_options(now: DateTime<Local>) -> Vec<(String, i64)> {
    let at =
        |date: chrono::NaiveDate, hour: u32| date.and_hms_opt(hour, 0, 0).and_then(|t| Local.from_local_datetime(&t).single()).map(|t| t.timestamp());
    let today = now.date_naive();
    let tomorrow = today.succ_opt().unwrap_or(today);
    let mut out = vec![("Over een uur".to_string(), now.timestamp() + 3600)];
    let evening = if now.hour() < 19 {
        at(today, 20).map(|t| ("Vanavond om 20:00".to_string(), t))
    } else {
        at(tomorrow, 20).map(|t| ("Morgenavond om 20:00".to_string(), t))
    };
    out.extend(evening);
    out.extend(at(tomorrow, 8).map(|t| ("Morgenochtend om 08:00".to_string(), t)));
    let until_monday = (7 - i64::from(today.weekday().num_days_from_monday())) % 7;
    let monday = today + chrono::Duration::days(if until_monday == 0 { 7 } else { until_monday });
    out.extend(at(monday, 8).map(|t| ("Maandagochtend om 08:00".to_string(), t)));
    out
}

/// "maandag 20 oktober om 08:00", in the computer's time zone.
pub(crate) fn when_text(ts: i64) -> String {
    let Some(t) = DateTime::from_timestamp(ts, 0).map(|t| t.with_timezone(&Local)) else { return dates::long(ts) };
    let d = t.date_naive();
    format!("{} {} {} om {:02}:{:02}", calendar::weekday_name(d), d.day(), calendar::month_name(d.month()), t.hour(), t.minute())
}

fn join(list: &[Address]) -> String {
    list.iter()
        .map(|a| match &a.name {
            Some(n) => format!("{n} <{}>", a.email()),
            None => a.email().to_string(),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

pub(crate) fn prefixed(subject: &str, prefix: &str) -> String {
    let s = subject.trim();
    if s.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase()) { s.to_string() } else { format!("{prefix} {s}") }
}

fn quoted(text: &str) -> String {
    text.lines().map(|l| format!("> {l}")).collect::<Vec<_>>().join("\n")
}

impl Compose {
    pub fn new(kind: Kind, original: Option<(&MessageHeader, &str, &str)>) -> Self {
        let mut c = Compose {
            to: String::new(),
            cc: String::new(),
            subject: String::new(),
            body: String::new(),
            in_reply_to: None,
            attachments: Vec::new(),
            account_id: String::new(),
            stored: None,
            send_at: None,
            importance: Importance::Normal,
            read_receipt: false,
            check: None,
            initial: String::new(),
            draft_id: None,
            error: None,
        };
        let Some((m, text, me)) = original else { return c };
        let header = format!("Op {} schreef {} <{}>:", dates::long(m.date), m.from.display(), m.from.email());
        match kind {
            Kind::New => {}
            Kind::Reply | Kind::ReplyAll => {
                c.to = join(std::slice::from_ref(&m.from));
                if kind == Kind::ReplyAll {
                    let others: Vec<Address> = m.to.iter().filter(|a| !a.email().eq_ignore_ascii_case(me)).cloned().collect();
                    c.cc = join(&others);
                }
                c.subject = prefixed(&m.subject, "Re:");
                c.in_reply_to = m.message_id.clone();
                c.body = format!("\n\n{header}\n{}", quoted(text));
            }
            Kind::Forward => {
                c.subject = prefixed(&m.subject, "Fw:");
                let to: Vec<&str> = m.to.iter().map(|a| a.email()).collect();
                c.body = format!(
                    "\n\n---------- Doorgestuurd bericht ----------\nVan: {} <{}>\nDatum: {}\nOnderwerp: {}\nAan: {}\n\n{text}",
                    m.from.display(),
                    m.from.email(),
                    dates::long(m.date),
                    m.subject,
                    to.join("; ")
                );
            }
        }
        c
    }

    /// Puts `signature` under the text, above any quoted or forwarded message, with the usual
    /// `-- ` marker so mail programs recognise it. Nothing changes for an empty signature.
    pub fn with_signature(mut self, signature: &str) -> Self {
        let signature = signature.trim_matches(['\n', ' ', '\t']);
        if signature.is_empty() {
            return self;
        }
        self.body = format!("\n\n-- \n{signature}{}", self.body);
        self
    }

    /// Continues a draft that was put aside.
    pub fn from_stored(d: &StoredDraft) -> Self {
        let mut c = Compose::new(Kind::New, None);
        c.to = d.to.clone();
        c.cc = d.cc.clone();
        c.subject = d.subject.clone();
        c.body = d.body.clone();
        c.in_reply_to = d.in_reply_to.clone();
        c.attachments = d.attachments.clone();
        c.account_id = d.account_id.clone();
        c.stored = Some(d.id.clone());
        c.send_at = d.send_at;
        c.importance = d.importance;
        c.read_receipt = d.read_receipt;
        c
    }

    /// Remembers what the window opened with, so an untouched window is not kept as a draft.
    pub fn opened_from(mut self, account_id: &str) -> Self {
        self.account_id = account_id.to_string();
        self.initial = self.fingerprint();
        self
    }

    fn fingerprint(&self) -> String {
        format!("{}\n{}\n{}\n{}", self.to, self.cc, self.subject, self.body)
    }

    /// The window as a draft to put aside; the id is the one it continues, or empty for a new one.
    pub fn as_stored(&self) -> StoredDraft {
        StoredDraft {
            id: self.stored.clone().unwrap_or_default(),
            account_id: self.account_id.clone(),
            to: self.to.clone(),
            cc: self.cc.clone(),
            subject: self.subject.clone(),
            body: self.body.clone(),
            in_reply_to: self.in_reply_to.clone(),
            attachments: self.attachments.clone(),
            updated: 0,
            send_at: self.send_at,
            importance: self.importance,
            read_receipt: self.read_receipt,
        }
    }

    /// Whether nothing has been written: no addresses, subject or files, and the text as it
    /// opened (a signature or a quoted message on its own is not worth keeping).
    pub fn is_blank(&self) -> bool {
        if !self.attachments.is_empty() {
            return false;
        }
        let empty = self.to.trim().is_empty() && self.cc.trim().is_empty() && self.subject.trim().is_empty() && self.body.trim().is_empty();
        empty || self.fingerprint() == self.initial
    }

    pub fn draft(&self) -> Result<Draft, String> {
        self.as_stored().to_draft()
    }

    /// What is worth a second look before the message goes, in the order the strip shows it.
    pub fn warnings(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.attachments.is_empty() && (mentions_attachment(&self.body) || mentions_attachment(&self.subject)) {
            out.push(NO_ATTACHMENT);
        }
        if self.subject.trim().is_empty() {
            out.push(NO_SUBJECT);
        }
        out
    }

    /// Reads a file from disk and adds it, or says why it cannot go along. The size is checked
    /// before and after reading, so a file that grows under us cannot slip past the limit.
    pub fn add_file(&mut self, path: &Path) -> Result<(), String> {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("bijlage");
        let room = MAX_ATTACHMENTS.saturating_sub(self.attachments.iter().map(|a| a.bytes.len()).sum());
        let too_big = || format!("{name} past er niet meer bij: bijlagen mogen samen {} MB zijn.", MAX_ATTACHMENTS / (1024 * 1024));
        let meta = std::fs::metadata(path).map_err(|_| format!("Kan {name} niet lezen."))?;
        if !meta.is_file() {
            return Err(format!("{name} is geen bestand."));
        }
        if usize::try_from(meta.len()).map_or(true, |len| len > room) {
            return Err(too_big());
        }
        let bytes = std::fs::read(path).map_err(|_| format!("Kan {name} niet lezen."))?;
        if bytes.len() > room {
            return Err(too_big());
        }
        self.attachments.push(DraftAttachment::new(name, bytes));
        Ok(())
    }

    /// Adds an attachment that is already in memory (one taken from a forwarded message), within
    /// the same size limit. The media type follows the name, not what the original claimed.
    pub fn add_attachment(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), String> {
        let room = MAX_ATTACHMENTS.saturating_sub(self.attachments.iter().map(|a| a.bytes.len()).sum());
        if bytes.len() > room {
            return Err(format!("{name} past er niet meer bij: bijlagen mogen samen {} MB zijn.", MAX_ATTACHMENTS / (1024 * 1024)));
        }
        self.attachments.push(DraftAttachment::new(name, bytes));
        Ok(())
    }

    /// Adds every file it can; the first problem shows in the window.
    pub fn add_files(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for path in paths {
            if let Err(e) = self.add_file(&path) {
                self.error = Some(e);
            }
        }
    }
}

impl MailApp {
    pub fn open_compose(&mut self, kind: Kind) {
        let session = &self.engine.session;
        let me = session.current_account().map(|a| a.address.email().to_string()).unwrap_or_default();
        let signature = session.current_account().map(|a| a.signature.clone()).unwrap_or_default();
        let original = session.selected().map(|m| {
            let text = session.bodies.get(&m.id).map(|b| b.text.as_str()).unwrap_or(m.preview.as_str());
            (m, text, me.as_str())
        });
        let account_id = session.current_account().map(|a| a.id.clone()).unwrap_or_default();
        // Forwarding takes the attachments along, as Outlook does; not from the Junk folder,
        // where they are not to be trusted or passed on.
        let forwarded: Vec<(String, Vec<u8>)> = match (kind, session.selected()) {
            (Kind::Forward, Some(m)) if !session.folder(&m.folder_id).is_some_and(|f| f.kind == FolderKind::Junk) => {
                session.bodies.get(&m.id).map(|b| b.attachments.iter().map(|a| (a.name.clone(), a.bytes.to_vec())).collect()).unwrap_or_default()
            }
            _ => Vec::new(),
        };
        let mut c = Compose::new(kind, original).with_signature(&signature).opened_from(&account_id);
        for (name, bytes) in forwarded {
            if let Err(e) = c.add_attachment(&name, bytes) {
                c.error = Some(e);
            }
        }
        self.compose = Some(c);
    }

    /// Opens a draft that was put aside, to write on or send.
    pub fn open_draft(&mut self, id: &str) {
        if let Some(d) = self.engine.session.draft(id) {
            self.compose = Some(Compose::from_stored(d));
        }
    }

    /// Closes the compose window. What was written is put aside as a draft; an untouched
    /// window, or one emptied on purpose, leaves nothing behind.
    fn put_aside(&mut self) {
        let Some(c) = self.compose.take() else { return };
        if c.is_blank() {
            self.forget_draft(c.stored.as_deref());
            return;
        }
        match serde_json::to_value(c.as_stored()) {
            Ok(params) => {
                self.run("draft.save", params);
                if self.notice.is_none() {
                    self.notice = Some("Concept bewaard. Je vindt het onder Concepten bij het account.".into());
                }
            }
            Err(_) => self.notice = Some("Het concept kon niet worden bewaard.".into()),
        }
    }

    /// Drops the stored draft a window came from, if any.
    fn forget_draft(&mut self, id: Option<&str>) {
        if let Some(id) = id
            && self.engine.session.draft(id).is_some()
        {
            self.run("draft.delete", serde_json::json!({ "id": id }));
        }
    }
}

/// Draws the window when there is one. Returns after the window closes itself.
pub fn show(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(c) = app.compose.as_mut() else { return };
    // Follow the draft through the outbox.
    if let Some(id) = c.draft_id {
        match app.engine.session.outbox.get(&id) {
            Some(SendState::Sent) => {
                let stored = c.stored.clone();
                app.compose = None;
                app.forget_draft(stored.as_deref());
                app.notice = Some("Bericht verzonden.".into());
                return;
            }
            Some(SendState::Failed(e)) => {
                c.error = Some(e.clone());
                c.draft_id = None;
            }
            _ => {}
        }
    }
    let sending = c.draft_id.is_some();
    // Files dropped onto the window become attachments; files hovering over it get a hint.
    if !sending {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        c.add_files(dropped);
    }
    let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
    let mut open = true;
    let mut send = false;
    let mut schedule: Option<i64> = None;
    let mut discard = false;
    let mut put_aside = false;
    let mut pick = false;
    let mut remove = None;
    let mut anyway = false;
    let mut template: Option<crate::templates::Pick> = None;
    // The message may have changed since the check: the strip follows it and goes when nothing
    // is left to say.
    if c.check.is_some() {
        let current = c.warnings();
        match c.check.as_mut() {
            Some(check) if !current.is_empty() => check.warnings = current,
            _ => c.check = None,
        }
    }
    egui::Window::new(if c.stored.is_some() { "Concept" } else { "Nieuw bericht" })
        .open(&mut open)
        .default_size(vec2(680.0, 520.0))
        .min_size(vec2(420.0, 320.0))
        .collapsible(false)
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.reading).stroke(Stroke::new(1.0, t.border)).inner_margin(Margin::same(14)))
        .show(ctx, |ui| {
            ui.add_enabled_ui(!sending, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let b = egui::Button::new(RichText::new("Verzenden").color(t.on_accent)).fill(t.accent).min_size(vec2(110.0, 30.0));
                    if ui.add(b).clicked() {
                        send = true;
                    }
                    if c.send_at.is_none() {
                        ui.menu_button("Later verzenden ▾", |ui| {
                            for (label, ts) in later_options(Local::now()) {
                                if ui.button(label).clicked() {
                                    schedule = Some(ts);
                                    ui.close();
                                }
                            }
                        })
                        .response
                        .on_hover_text(
                            "Zet het bericht opzij en verstuurt het op dat moment, als Mailcraft dan draait. Tot die tijd staat het onder Concepten.",
                        );
                    }
                    if ui
                        .add(egui::Button::new("Opslaan als concept").min_size(vec2(140.0, 30.0)))
                        .on_hover_text(
                            "Sluit het venster en bewaart het bericht onder Concepten bij het account. Het venster sluiten doet hetzelfde.",
                        )
                        .clicked()
                    {
                        put_aside = true;
                    }
                    if ui
                        .add(egui::Button::new("Verwijderen").min_size(vec2(100.0, 30.0)))
                        .on_hover_text("Gooit het bericht weg, ook als concept.")
                        .clicked()
                    {
                        discard = true;
                    }
                    if ui
                        .add(egui::Button::new("📎 Bijlage toevoegen").min_size(vec2(150.0, 30.0)))
                        .on_hover_text("Kies een of meer bestanden. Je kunt ze ook naar dit venster slepen.")
                        .clicked()
                    {
                        pick = true;
                    }
                    crate::importance::chooser(ui, t, &mut c.importance);
                    ui.checkbox(&mut c.read_receipt, crate::receipt::ASK).on_hover_text(
                        "Vraagt het mailprogramma van de ontvanger om je te laten weten dat het bericht is geopend; de ontvanger beslist zelf of dat gebeurt.",
                    );
                    template = crate::templates::menu(ui, &app.engine.session.templates_sorted());
                    if sending {
                        ui.label(RichText::new("Verzenden…").color(t.text_dim));
                    } else if hovering {
                        ui.label(RichText::new("Laat los om toe te voegen").color(t.accent_text));
                    }
                });
                if let Some(check) = &c.check {
                    let mut look_again = false;
                    egui::Frame::new().fill(t.flag.gamma_multiply(0.15)).corner_radius(4).inner_margin(8).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        for w in &check.warnings {
                            ui.label(RichText::new(format!("⚠ {w}")).color(t.text));
                        }
                        ui.horizontal(|ui| {
                            if ui.button(SEND_ANYWAY).clicked() {
                                anyway = true;
                            }
                            if ui.button(LOOK_AGAIN).clicked() {
                                look_again = true;
                            }
                        });
                    });
                    ui.add_space(6.0);
                    if look_again {
                        c.check = None;
                    }
                }
                if let Some(ts) = c.send_at {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(format!("⏰ Wordt verzonden op {}.", when_text(ts))).color(t.accent_text));
                        if ui
                            .small_button("Niet meer uitstellen")
                            .on_hover_text("Het wordt weer een gewoon concept; verzend het zelf wanneer je wilt.")
                            .clicked()
                        {
                            c.send_at = None;
                        }
                    });
                }
                ui.add_space(8.0);
                field(ui, t, "Aan", &mut c.to);
                crate::contacts::complete_addresses(ui, t, &app.engine.session, &mut c.to);
                field(ui, t, "CC", &mut c.cc);
                crate::contacts::complete_addresses(ui, t, &app.engine.session, &mut c.cc);
                field(ui, t, "Onderwerp", &mut c.subject);
                if !c.attachments.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.add_sized([80.0, 24.0], egui::Label::new(RichText::new("Bijlagen").color(t.text_dim)));
                        for (i, a) in c.attachments.iter().enumerate() {
                            egui::Frame::NONE
                                .fill(t.panel)
                                .stroke(Stroke::new(1.0, t.border))
                                .corner_radius(6.0)
                                .inner_margin(Margin::symmetric(8, 3))
                                .show(ui, |ui| {
                                    ui.label(RichText::new(format!("📎 {} ({})", a.name, crate::size_text(a.bytes.len()))).size(12.0).color(t.text));
                                    if ui.small_button("✕").on_hover_text("Bijlage weghalen").clicked() {
                                        remove = Some(i);
                                    }
                                });
                        }
                    });
                }
                if let Some(e) = &c.error {
                    ui.label(RichText::new(e).color(t.flag));
                }
                ui.add_space(6.0);
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    ui.add_sized(
                        ui.available_size(),
                        egui::TextEdit::multiline(&mut c.body).font(egui::TextStyle::Body).desired_width(f32::INFINITY),
                    );
                });
            });
        });
    if discard {
        let stored = c.stored.clone();
        app.compose = None;
        app.forget_draft(stored.as_deref());
        return;
    }
    if (put_aside || !open) && !sending {
        app.put_aside();
        return;
    }
    if let Some(i) = remove.filter(|i| *i < c.attachments.len()) {
        c.attachments.remove(i);
        c.error = None;
    }
    match template {
        Some(crate::templates::Pick::Insert(text)) => c.body = crate::templates::insert(&c.body, &text),
        Some(crate::templates::Pick::SaveAs) => {
            let text = crate::templates::own_text(&c.body).to_string();
            let state = app.templates.get_or_insert_with(Default::default);
            state.form = Some(crate::templates::Form { id: None, name: String::new(), text, error: None });
        }
        Some(crate::templates::Pick::Manage) => {
            app.templates.get_or_insert_with(Default::default);
        }
        None => {}
    }
    if pick {
        // The computer's own dialog; it blocks until the person has chosen or cancelled.
        if let Some(paths) = rfd::FileDialog::new().set_title("Bijlagen kiezen").pick_files() {
            c.error = None;
            c.add_files(paths);
        }
    }
    if anyway && let Some(check) = c.check.take() {
        match check.later {
            Some(ts) => schedule = Some(ts),
            None => send = true,
        }
    } else if send || schedule.is_some() {
        // Outlook's reminders: a message that speaks of an attachment without one, or without a
        // subject, waits for "Toch verzenden". The recipient check comes first, as it always did.
        let warnings = c.warnings();
        if !warnings.is_empty() && c.draft().is_ok() {
            c.check = Some(Check { warnings, later: schedule });
            send = false;
            schedule = None;
        }
    }
    if let Some(ts) = schedule {
        match c.draft() {
            Ok(_) => {
                c.send_at = Some(ts);
                c.error = None;
                app.notice = None;
                app.put_aside();
                if app.engine.session.last_draft.as_deref().and_then(|id| app.engine.session.draft(id)).is_some_and(|d| d.send_at == Some(ts)) {
                    app.notice = Some(format!(
                        "Het bericht wordt verzonden op {} als Mailcraft dan draait. Tot die tijd staat het onder Concepten; open het daar om dat te veranderen.",
                        when_text(ts)
                    ));
                }
                return;
            }
            Err(e) => c.error = Some(e),
        }
    }
    if send {
        match c.draft() {
            Ok(draft) => {
                let account = if c.account_id.is_empty() {
                    app.engine.session.current_account().map(|a| a.id.clone()).unwrap_or_default()
                } else {
                    c.account_id.clone()
                };
                c.error = None;
                c.draft_id = Some(app.engine.send(&account, draft));
            }
            Err(e) => c.error = Some(e),
        }
    }
}

fn field(ui: &mut Ui, t: &Tokens, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        ui.add_sized([80.0, 24.0], egui::Label::new(RichText::new(label).color(t.text_dim)));
        ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailcraft_model::Flags;

    fn header() -> MessageHeader {
        MessageHeader {
            id: "m".into(),
            folder_id: "f".into(),
            remote_id: "1".into(),
            from: Address::new(Some("Sanne"), "sanne@example.org").unwrap(),
            to: vec![Address::new(None, "jij@example.org").unwrap(), Address::new(None, "ravi@example.org").unwrap()],
            subject: "Planning".into(),
            date: 0,
            preview: "p".into(),
            flags: Flags::default(),
            has_attachments: false,
            message_id: Some("<x@example.org>".into()),
            categories: Vec::new(),
            importance: Default::default(),
        }
    }

    #[test]
    fn reply_all_leaves_me_out_and_quotes_the_text() {
        let h = header();
        let c = Compose::new(Kind::ReplyAll, Some((&h, "tien uur?", "jij@example.org")));
        assert_eq!(c.to, "Sanne <sanne@example.org>");
        assert_eq!(c.cc, "ravi@example.org");
        assert_eq!(c.subject, "Re: Planning");
        assert!(c.body.contains("> tien uur?"));
        assert_eq!(c.in_reply_to.as_deref(), Some("<x@example.org>"));
        let d = c.draft().unwrap();
        assert_eq!(d.to[0].email(), "sanne@example.org");
        assert_eq!(d.cc.len(), 1);
    }

    #[test]
    fn the_signature_sits_under_the_text_and_above_the_quote() {
        let h = header();
        let new = Compose::new(Kind::New, None).with_signature("Groet,\nAlain\n");
        assert_eq!(new.body, "\n\n-- \nGroet,\nAlain");
        let reply = Compose::new(Kind::Reply, Some((&h, "tien uur?", "me@x.nl"))).with_signature("Alain");
        let (sig, quote) = (reply.body.find("-- \nAlain").unwrap(), reply.body.find("> tien uur?").unwrap());
        assert!(sig < quote, "{}", reply.body);
        assert_eq!(Compose::new(Kind::New, None).with_signature("  \n").body, "");
    }

    #[test]
    fn files_become_attachments_within_the_size_limit() {
        let dir = std::env::temp_dir().join(format!("mailcraft-compose-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("offerte.pdf");
        std::fs::write(&file, b"%PDF-1.4").unwrap();
        let mut c = Compose::new(Kind::New, None);
        c.add_file(&file).unwrap();
        assert_eq!(
            (c.attachments[0].name.as_str(), c.attachments[0].mime.as_str(), c.attachments[0].bytes.as_slice()),
            ("offerte.pdf", "application/pdf", &b"%PDF-1.4"[..])
        );
        assert!(c.add_file(&dir.join("bestaat-niet.txt")).is_err());
        assert!(c.add_file(&dir).is_err(), "a folder is not a file");
        c.attachments.push(DraftAttachment::new("groot.bin", vec![0; MAX_ATTACHMENTS - 4]));
        let e = c.add_file(&file).unwrap_err();
        assert!(e.contains("25 MB"), "{e}");
        assert_eq!(c.attachments.len(), 2);
        c.to = "sanne@example.org".into();
        assert_eq!(c.draft().unwrap().attachments.len(), 2, "attachments travel with the draft");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_window_lists_attachments_and_removes_one_on_click() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut app = MailApp::new(mailcraft_engine::Engine::sample());
        app.open_compose(Kind::New);
        if let Some(c) = app.compose.as_mut() {
            c.attachments.push(DraftAttachment::new("offerte.pdf", vec![0; 2048]));
            c.attachments.push(DraftAttachment::new("foto.jpg", vec![0; 10]));
        }
        let mut harness = Harness::builder().with_size(vec2(1100.0, 900.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app);
        harness.run();
        harness.get_by_label("📎 Bijlage toevoegen");
        harness.get_by_label("📎 offerte.pdf (2 kB)");
        harness.get_by_label("📎 foto.jpg (10 B)");
        harness.get_all_by_label("✕").next().expect("a remove button").click();
        harness.run();
        let names: Vec<String> = harness.state().compose.as_ref().map(|c| c.attachments.iter().map(|a| a.name.clone()).collect()).unwrap_or_default();
        assert_eq!(names, ["foto.jpg"]);
        assert!(harness.query_by_label("📎 offerte.pdf (2 kB)").is_none());
    }

    #[test]
    fn the_urgency_menu_marks_the_message_as_urgent_and_the_draft_carries_it() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut app = MailApp::new(mailcraft_engine::Engine::sample());
        app.open_compose(Kind::New);
        let mut h = Harness::builder().with_size(vec2(1100.0, 900.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app);
        h.run();
        h.get_by_label("Urgentie ▾").click();
        h.run();
        h.get_all_by_label("Hoog").next().expect("the menu item").click();
        h.run();
        assert_eq!(h.state().compose.as_ref().map(|c| c.importance), Some(Importance::High));
        h.get_by_label("! Hoge urgentie");
        assert_eq!(h.state().compose.as_ref().map(|c| c.as_stored().importance), Some(Importance::High));
        assert_eq!(h.state().compose.as_ref().and_then(|c| c.as_stored().to_draft().ok()).map(|d| d.importance), None, "no recipient yet");
        h.get_by_label(crate::receipt::ASK).click();
        h.run();
        assert_eq!(h.state().compose.as_ref().map(|c| (c.read_receipt, c.as_stored().read_receipt)), Some((true, true)));
    }

    #[test]
    fn later_verzenden_puts_the_message_aside_with_a_moment_and_can_be_undone() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut app = MailApp::new(mailcraft_engine::Engine::sample());
        app.open_compose(Kind::New);
        let mut h = Harness::builder().with_size(vec2(1100.0, 900.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app);
        h.run();
        h.get_by_label("Later verzenden ▾").click();
        h.run();
        h.get_by_label("Over een uur").click();
        h.run();
        let c = h.state().compose.as_ref().expect("without a recipient the window stays");
        assert!(c.error.as_deref().is_some_and(|e| e.contains("ontvanger")) && c.send_at.is_none(), "{:?}", c.error);

        if let Some(c) = h.state_mut().compose.as_mut() {
            c.to = "sanne@example.org".into();
            c.subject = "Later".into();
        }
        h.run();
        h.get_by_label("Later verzenden ▾").click();
        h.run();
        h.get_by_label("Over een uur").click();
        h.run();
        assert!(h.state().compose.is_none(), "put aside");
        let now = Local::now().timestamp();
        let drafts = h.state().engine.session.drafts.clone();
        assert_eq!(drafts.len(), 1);
        let send_at = drafts[0].send_at.expect("a moment");
        assert!((send_at - now - 3600).abs() < 60, "{send_at} vs {now}");
        assert!(h.state().notice.as_deref().is_some_and(|n| n.contains("wordt verzonden op")), "{:?}", h.state().notice);

        // Opened again, the moment shows and can be dropped; closing keeps it as a plain draft.
        let id = drafts[0].id.clone();
        h.state_mut().open_draft(&id);
        h.run();
        assert!(h.query_by_label("Later verzenden ▾").is_none(), "no second moment on top of the first");
        h.get_by_label("Niet meer uitstellen").click();
        h.run();
        assert_eq!(h.state().compose.as_ref().and_then(|c| c.send_at), None);
        h.state_mut().put_aside();
        assert_eq!(h.state().engine.session.draft(&id).and_then(|d| d.send_at), None);
    }

    #[test]
    fn a_text_about_an_attachment_is_found_outside_quotes_and_signature() {
        assert!(mentions_attachment("Zie de bijlage voor de offerte."));
        assert!(mentions_attachment("Hierbij BIJGEVOEGD het verslag"));
        assert!(mentions_attachment("Please find the report attached."));
        assert!(!mentions_attachment("Tot morgen!\n\n> Zie bijlage"));
        assert!(!mentions_attachment("Tot morgen!\n\n-- \nAlain, bijlagen op aanvraag"));
        assert!(!mentions_attachment("Kijk even.\n\n---------- Doorgestuurd bericht ----------\nVan: x\n\nzie bijlage"));
        assert!(!mentions_attachment("De bijlagenkamer is open"), "whole words only");
    }

    #[test]
    fn send_asks_first_about_a_missing_attachment_or_subject_and_sends_on_toch_verzenden() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut app = MailApp::new(mailcraft_engine::Engine::sample());
        app.open_compose(Kind::New);
        if let Some(c) = app.compose.as_mut() {
            c.to = "sanne@example.org".into();
            c.body = "Hoi Sanne, in de bijlage de planning.".into();
        }
        let mut h = Harness::builder().with_size(vec2(1100.0, 900.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app);
        h.run();
        h.get_by_label("Verzenden").click();
        h.run();
        let c = h.state().compose.as_ref().expect("the window stays");
        assert_eq!(c.draft_id, None, "not sent yet");
        assert_eq!(c.check.as_ref().map(|k| k.warnings.clone()), Some(vec![NO_ATTACHMENT, NO_SUBJECT]));
        h.get_by_label(&format!("⚠ {NO_ATTACHMENT}"));
        h.get_by_label(&format!("⚠ {NO_SUBJECT}"));

        // A subject typed in takes that warning away; an attachment would take the other.
        if let Some(c) = h.state_mut().compose.as_mut() {
            c.subject = "Planning".into();
        }
        h.run();
        assert_eq!(h.state().compose.as_ref().and_then(|c| c.check.as_ref()).map(|k| k.warnings.clone()), Some(vec![NO_ATTACHMENT]));
        h.get_by_label(LOOK_AGAIN).click();
        h.run();
        assert_eq!(h.state().compose.as_ref().and_then(|c| c.check.clone()), None);
        if let Some(c) = h.state_mut().compose.as_mut() {
            c.attachments.push(DraftAttachment::new("planning.pdf", vec![0; 10]));
        }
        h.run();
        h.get_by_label("Verzenden").click();
        h.run();
        // The sample account cannot send, so the attempt shows as its error; the point is that no
        // question came in between.
        let c = h.state().compose.as_ref().expect("the window stays");
        assert!(c.check.is_none() && c.error.as_deref().is_some_and(|e| e.contains("echt account")), "sent without a question: {:?}", c.error);
    }

    #[test]
    fn toch_verzenden_sends_and_later_verzenden_is_checked_too() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut app = MailApp::new(mailcraft_engine::Engine::sample());
        app.open_compose(Kind::New);
        if let Some(c) = app.compose.as_mut() {
            c.to = "sanne@example.org".into();
            c.body = "Zonder onderwerp".into();
        }
        let mut h = Harness::builder().with_size(vec2(1100.0, 900.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app);
        h.run();
        h.get_by_label("Later verzenden ▾").click();
        h.run();
        h.get_by_label("Over een uur").click();
        h.run();
        let c = h.state().compose.as_ref().expect("held back");
        assert!(c.check.as_ref().is_some_and(|k| k.later.is_some() && k.warnings == [NO_SUBJECT]));
        assert!(h.state().engine.session.drafts.is_empty());
        h.get_by_label(SEND_ANYWAY).click();
        h.run();
        assert!(h.state().compose.is_none(), "put aside with the moment");
        assert!(h.state().engine.session.drafts.first().and_then(|d| d.send_at).is_some());

        // Without a recipient the old message about the recipient comes first and no check shows.
        h.state_mut().open_compose(Kind::New);
        if let Some(c) = h.state_mut().compose.as_mut() {
            c.body = "zie bijlage".into();
        }
        h.run();
        h.get_by_label("Verzenden").click();
        h.run();
        let c = h.state().compose.as_ref().expect("the window stays");
        assert!(c.check.is_none() && c.error.as_deref().is_some_and(|e| e.contains("ontvanger")), "{:?}", c.error);
    }

    #[test]
    fn a_template_is_dropped_in_above_the_signature_and_the_text_can_be_saved_as_one() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut app = MailApp::new(mailcraft_engine::Engine::sample());
        app.engine.run("template.save", &serde_json::json!({ "name": "Route", "text": "Kom via de A2, afslag 9." })).unwrap();
        app.open_compose(Kind::New);
        if let Some(c) = app.compose.as_mut() {
            c.body = "Hoi Sanne,\n\n-- \nAlain".into();
        }
        let mut h = Harness::builder().with_size(vec2(1100.0, 900.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app);
        h.run();
        h.get_by_label(crate::templates::MENU).click();
        h.run();
        h.get_by_label("Route").click();
        h.run();
        assert_eq!(h.state().compose.as_ref().map(|c| c.body.as_str()), Some("Hoi Sanne,\n\nKom via de A2, afslag 9.\n\n-- \nAlain"));

        // The text written so far becomes a new template through the window.
        h.get_by_label(crate::templates::MENU).click();
        h.run();
        h.get_by_label(crate::templates::SAVE_AS).click();
        h.run();
        let form = h.state().templates.as_ref().and_then(|s| s.form.clone()).expect("the window with the form");
        assert_eq!(form.text, "Hoi Sanne,\n\nKom via de A2, afslag 9.");
        h.get_by_label("Opslaan").click();
        h.run();
        assert!(h.state().templates.as_ref().and_then(|s| s.form.as_ref()).is_some_and(|f| f.error.is_some()), "no name yet");
        if let Some(f) = h.state_mut().templates.as_mut().and_then(|s| s.form.as_mut()) {
            f.name = "Begroeting".into();
        }
        h.run();
        h.get_by_label("Opslaan").click();
        h.run();
        let names: Vec<String> = h.state().engine.session.templates_sorted().iter().map(|t| t.name.clone()).collect();
        assert_eq!(names, ["Begroeting", "Route"]);
        h.get_all_by_label("Sjabloon verwijderen").next().expect("a delete button per template").click();
        h.run();
        let names: Vec<String> = h.state().engine.session.templates_sorted().iter().map(|t| t.name.clone()).collect();
        assert_eq!(names, ["Route"], "the first in the list, Begroeting, is gone");
        h.get_by_label("Sluiten").click();
        h.run();
        assert!(h.state().templates.is_none());
    }

    #[test]
    fn the_moments_offered_are_whole_hours_ahead() {
        let noon = Local.with_ymd_and_hms(2026, 10, 21, 12, 0, 0).single().unwrap(); // a Wednesday
        let options = later_options(noon);
        let labels: Vec<&str> = options.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["Over een uur", "Vanavond om 20:00", "Morgenochtend om 08:00", "Maandagochtend om 08:00"]);
        assert_eq!(options[0].1, noon.timestamp() + 3600);
        assert_eq!(when_text(options[1].1), "woensdag 21 oktober om 20:00");
        assert_eq!(when_text(options[2].1), "donderdag 22 oktober om 08:00");
        assert_eq!(when_text(options[3].1), "maandag 26 oktober om 08:00");
        let late = Local.with_ymd_and_hms(2026, 10, 26, 21, 30, 0).single().unwrap(); // a Monday evening
        let labels: Vec<String> = later_options(late).into_iter().map(|(l, _)| l).collect();
        assert_eq!(labels[1], "Morgenavond om 20:00");
        assert_eq!(when_text(later_options(late)[3].1), "maandag 2 november om 08:00", "next Monday, not today");
    }

    #[test]
    fn a_window_knows_when_nothing_was_written_and_round_trips_as_a_draft() {
        let h = header();
        let untouched = Compose::new(Kind::Reply, Some((&h, "tien uur?", "me@x.nl"))).with_signature("Alain").opened_from("acc");
        assert!(untouched.is_blank(), "a quote and a signature alone are not a draft");
        let mut written = Compose::new(Kind::Reply, Some((&h, "tien uur?", "me@x.nl"))).with_signature("Alain").opened_from("acc");
        written.body = format!("Prima.{}", written.body);
        assert!(!written.is_blank());
        written.attachments.push(DraftAttachment::new("a.txt", b"x".to_vec()));
        written.importance = Importance::Low;
        written.read_receipt = true;
        let stored = written.as_stored();
        assert_eq!((stored.importance, stored.read_receipt), (Importance::Low, true));
        assert_eq!((stored.id.as_str(), stored.account_id.as_str(), stored.to.as_str()), ("", "acc", "Sanne <sanne@example.org>"));
        assert_eq!(stored.in_reply_to.as_deref(), Some("<x@example.org>"));
        let again = Compose::from_stored(&StoredDraft { id: "d-1".into(), ..stored });
        assert_eq!((again.stored.as_deref(), again.subject.as_str(), again.attachments.len()), (Some("d-1"), "Re: Planning", 1));
        assert_eq!(again.importance, Importance::Low, "the urgency comes back with the draft");
        assert!(again.read_receipt, "so does the request for a read receipt");
        assert!(again.body.starts_with("Prima."));
        let mut emptied = again;
        emptied.to.clear();
        emptied.subject.clear();
        emptied.body.clear();
        emptied.attachments.clear();
        assert!(emptied.is_blank(), "a draft emptied on purpose is not kept");
    }

    #[test]
    fn forwarding_takes_the_attachments_along_except_from_the_junk_folder() {
        use mailcraft_model::Folder;
        let mut app = MailApp::new(mailcraft_engine::Engine::sample());
        let attachment = |name: &str, bytes: &[u8]| mailcraft_mime::Attachment {
            name: name.into(),
            mime: "application/octet-stream".into(),
            size: bytes.len(),
            bytes: std::sync::Arc::from(bytes),
        };
        let body =
            mailcraft_engine::Body { attachments: vec![attachment("offerte.pdf", b"%PDF"), attachment("foto.jpg", b"jpg")], ..Default::default() };
        app.engine.session.bodies.insert("m2".into(), body);
        app.engine.session.selected_message = Some("m2".into());
        app.open_compose(Kind::Forward);
        let c = app.compose.as_ref().expect("the window");
        let names: Vec<(&str, &str)> = c.attachments.iter().map(|a| (a.name.as_str(), a.mime.as_str())).collect();
        assert_eq!(names, [("offerte.pdf", "application/pdf"), ("foto.jpg", "image/jpeg")], "the type follows the name");
        assert!(c.subject.starts_with("Fw:") && c.error.is_none());
        app.open_compose(Kind::Reply);
        assert!(app.compose.as_ref().expect("the window").attachments.is_empty(), "a reply does not carry them");

        let junk = Folder {
            id: "junk".into(),
            account_id: "sample".into(),
            name: "Ongewenst".into(),
            path: "Junk".into(),
            kind: FolderKind::Junk,
            unread: 0,
        };
        app.engine.session.folders.push(junk);
        if let Some(m) = app.engine.session.messages.iter_mut().find(|m| m.id == "m2") {
            m.folder_id = "junk".into();
        }
        app.open_compose(Kind::Forward);
        assert!(app.compose.as_ref().expect("the window").attachments.is_empty(), "nothing leaves the Junk folder");

        let mut c = Compose::new(Kind::New, None);
        assert!(c.add_attachment("groot.bin", vec![0; MAX_ATTACHMENTS + 1]).unwrap_err().contains("past er niet meer bij"));
        assert!(c.attachments.is_empty());
    }

    #[test]
    fn forward_has_no_recipient_and_re_is_not_doubled() {
        let h = MessageHeader { subject: "Re: Planning".into(), ..header() };
        let c = Compose::new(Kind::Forward, Some((&h, "t", "me@x.nl")));
        assert_eq!(c.subject, "Fw: Re: Planning");
        assert!(c.draft().is_err());
        assert_eq!(Compose::new(Kind::Reply, Some((&h, "t", "me@x.nl"))).subject, "Re: Planning");
    }
}
