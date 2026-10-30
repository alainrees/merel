//! The compose window: new mail, reply, reply all and forward, with attachments picked from
//! the computer's file dialog or dropped onto the window. "Later verzenden" puts the message
//! aside with a moment to go out, like Outlook's "Bezorging uitstellen"; until then it is a
//! draft on this computer, and the engine sends it when the app runs at that moment. Before a
//! message goes, the window looks for what Outlook warns about too: a text that speaks of an
//! attachment while none is on, and an empty subject; it then asks once, and sends on "Toch
//! verzenden".
//!
//! The window is laid out as Outlook's: Van (a list when there are several accounts), Aan with
//! CC, BCC and the address book at its end, Onderwerp with the urgency at its end, then the
//! formatting bar over the text (see [`crate::rich_edit`]). While the window is open the
//! message is kept as a draft a few seconds after each change, and the window says so at the
//! bottom: "Concept is zojuist opgeslagen".

use crate::i18n::tr;
use std::path::{Path, PathBuf};

use egui::{Margin, RichText, Stroke, Ui, vec2};

use merel_engine::SendState;
use merel_model::{Address, Draft, DraftAttachment, FolderKind, Importance, MAX_ATTACHMENTS, MessageHeader, StoredDraft};

use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};

use crate::helder::{self, Icon};
use crate::{MailApp, Tokens, calendar, dates, rich_edit};

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
    pub bcc: String,
    /// Whether the CC and BCC lines show; a line with addresses always does.
    pub show_cc: bool,
    pub show_bcc: bool,
    pub subject: String,
    pub body: String,
    /// The formatting over `body`, and the editor's undo and selection.
    pub editor: rich_edit::Editor,
    /// Opened from a draft put aside, so the window is called "Concept".
    from_draft: bool,
    /// When the window last kept the message as a draft by itself (seconds since the epoch).
    pub saved_at: Option<i64>,
    /// The fields as last seen, and when they last changed (egui time), for the automatic save.
    seen: String,
    changed_at: Option<f64>,
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
    pub error: Option<String>,
    /// The moment the message goes out, when it was put aside with "Later verzenden".
    pub send_at: Option<i64>,
    /// Outlook's "Hoge urgentie" or "Lage urgentie"; normal unless chosen.
    pub importance: Importance,
    /// Outlook's "Leesbevestiging vragen": the reader's program is asked to say when the message
    /// was read.
    pub read_receipt: bool,
    /// Whether the message is signed with the account's S/MIME certificate or OpenPGP key;
    /// `None` follows the account: signed when it has one.
    pub sign: Option<bool>,
    /// Outlook's "Versleutelen": the message goes out encrypted (S/MIME or OpenPGP) for its
    /// recipients and the sender, and only when every recipient's certificate or key is known.
    pub encrypt: bool,
    /// What Send found worth a second look, with the moment chosen if "Later verzenden" was
    /// pressed; the window shows it and waits for "Toch verzenden" or a change to the message.
    pub check: Option<Check>,
}

/// Outlook's reminders before a message goes: "Je schrijft over een bijlage…" and "Het bericht
/// heeft geen onderwerp."
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub warnings: Vec<String>,
    /// The "Later verzenden" moment the check held back, or None for a plain send.
    pub later: Option<i64>,
}

pub fn no_attachment() -> String {
    tr("compose-no-attachment")
}
pub fn no_subject() -> String {
    tr("compose-no-subject")
}
pub fn send_anyway() -> String {
    tr("compose-send-anyway")
}
pub fn look_again_label() -> String {
    tr("compose-look-again")
}
/// The tick box that signs the message with the account's certificate or OpenPGP key.
pub fn sign() -> String {
    tr("compose-sign")
}
/// The send button.
pub fn send_label() -> String {
    tr("compose-send")
}
/// The chevron beside the send button.
pub fn later() -> String {
    tr("compose-later")
}
/// The button that opens the file dialog.
pub fn attach() -> String {
    tr("compose-attach")
}
/// The menu with the read receipt.
pub fn options_label() -> String {
    tr("compose-options")
}
/// The button that keeps the message under Concepten.
pub fn put_aside_label() -> String {
    tr("compose-put-aside")
}
/// The button that throws the message away.
pub fn discard_label() -> String {
    tr("delete")
}
/// The tick box that encrypts the message for its recipients.
pub fn encrypt() -> String {
    tr("compose-encrypt")
}

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
    let mut out = vec![(tr("later-in-an-hour"), now.timestamp() + 3600)];
    let evening =
        if now.hour() < 19 { at(today, 20).map(|t| (tr("later-tonight"), t)) } else { at(tomorrow, 20).map(|t| (tr("later-tomorrow-evening"), t)) };
    out.extend(evening);
    out.extend(at(tomorrow, 8).map(|t| (tr("later-tomorrow-morning"), t)));
    let until_monday = (7 - i64::from(today.weekday().num_days_from_monday())) % 7;
    let monday = today + chrono::Duration::days(if until_monday == 0 { 7 } else { until_monday });
    out.extend(at(monday, 8).map(|t| (tr("later-monday-morning"), t)));
    out
}

/// "maandag 20 oktober om 08:00", in the computer's time zone.
pub(crate) fn when_text(ts: i64) -> String {
    let Some(t) = DateTime::from_timestamp(ts, 0).map(|t| t.with_timezone(&Local)) else { return dates::long(ts) };
    let d = t.date_naive();
    let day = format!("{} {} {}", calendar::weekday_name(d), d.day(), calendar::month_name(d.month()));
    crate::i18n::tr_args("date-at-time", &[("day", day.into()), ("time", format!("{:02}:{:02}", t.hour(), t.minute()).into())])
}

pub(crate) fn join(list: &[Address]) -> String {
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
            bcc: String::new(),
            show_cc: false,
            show_bcc: false,
            subject: String::new(),
            body: String::new(),
            editor: rich_edit::Editor::new("compose", "", None),
            from_draft: false,
            saved_at: None,
            seen: String::new(),
            changed_at: None,
            in_reply_to: None,
            attachments: Vec::new(),
            account_id: String::new(),
            stored: None,
            send_at: None,
            importance: Importance::Normal,
            read_receipt: false,
            sign: None,
            encrypt: false,
            check: None,
            initial: String::new(),
            error: None,
        };
        let Some((m, text, me)) = original else { return c };
        let header = crate::i18n::tr_args(
            "reply-header",
            &[("date", dates::long(m.date).into()), ("name", m.from.display().into()), ("email", m.from.email().into())],
        );
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
                let header = crate::i18n::tr_args(
                    "forward-header",
                    &[
                        ("name", m.from.display().into()),
                        ("email", m.from.email().into()),
                        ("date", dates::long(m.date).into()),
                        ("subject", m.subject.as_str().into()),
                        ("to", to.join("; ").into()),
                    ],
                );
                c.body = format!("\n\n{header}\n\n{text}");
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
        c.bcc = d.bcc.clone();
        c.subject = d.subject.clone();
        c.body = d.body.clone();
        c.editor = rich_edit::Editor::new("compose", &d.body, d.rich.clone());
        c.from_draft = true;
        c.in_reply_to = d.in_reply_to.clone();
        c.attachments = d.attachments.clone();
        c.account_id = d.account_id.clone();
        c.stored = Some(d.id.clone());
        c.send_at = d.send_at;
        c.importance = d.importance;
        c.read_receipt = d.read_receipt;
        c.sign = d.sign;
        c.encrypt = d.encrypt;
        c.seen = c.state();
        c
    }

    /// Remembers what the window opened with, so an untouched window is not kept as a draft.
    pub fn opened_from(mut self, account_id: &str) -> Self {
        self.account_id = account_id.to_string();
        self.initial = self.fingerprint();
        self.seen = self.state();
        self
    }

    fn fingerprint(&self) -> String {
        format!("{}\n{}\n{}\n{}\n{}", self.to, self.cc, self.bcc, self.subject, self.body)
    }

    /// Everything that is worth saving again when it changes, formatting and options included.
    fn state(&self) -> String {
        let files: Vec<(&str, usize)> = self.attachments.iter().map(|a| (a.name.as_str(), a.bytes.len())).collect();
        format!(
            "{}\n{:?}\n{:?} {:?} {} {:?} {} {:?}",
            self.fingerprint(),
            self.editor.stored(&self.body),
            files,
            self.importance,
            self.read_receipt,
            self.sign,
            self.encrypt,
            self.send_at
        )
    }

    /// The window as a draft to put aside; the id is the one it continues, or empty for a new one.
    pub fn as_stored(&self) -> StoredDraft {
        StoredDraft {
            id: self.stored.clone().unwrap_or_default(),
            account_id: self.account_id.clone(),
            to: self.to.clone(),
            cc: self.cc.clone(),
            bcc: self.bcc.clone(),
            subject: self.subject.clone(),
            body: self.body.clone(),
            in_reply_to: self.in_reply_to.clone(),
            attachments: self.attachments.clone(),
            updated: 0,
            send_at: self.send_at,
            importance: self.importance,
            read_receipt: self.read_receipt,
            sign: self.sign,
            encrypt: self.encrypt,
            rich: self.editor.stored(&self.body),
        }
    }

    /// Whether nothing has been written: no addresses, subject or files, and the text as it
    /// opened (a signature or a quoted message on its own is not worth keeping).
    pub fn is_blank(&self) -> bool {
        if !self.attachments.is_empty() {
            return false;
        }
        let empty = self.to.trim().is_empty()
            && self.cc.trim().is_empty()
            && self.bcc.trim().is_empty()
            && self.subject.trim().is_empty()
            && self.body.trim().is_empty();
        empty || self.fingerprint() == self.initial
    }

    pub fn draft(&self) -> Result<Draft, String> {
        self.as_stored().to_draft()
    }

    /// What is worth a second look before the message goes, in the order the strip shows it.
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.attachments.is_empty() && (mentions_attachment(&self.body) || mentions_attachment(&self.subject)) {
            out.push(no_attachment());
        }
        if self.subject.trim().is_empty() {
            out.push(no_subject());
        }
        out
    }

    /// Reads a file from disk and adds it, or says why it cannot go along. The size is checked
    /// before and after reading, so a file that grows under us cannot slip past the limit.
    pub fn add_file(&mut self, path: &Path) -> Result<(), String> {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("bijlage");
        let room = MAX_ATTACHMENTS.saturating_sub(self.attachments.iter().map(|a| a.bytes.len()).sum());
        let too_big = || crate::i18n::tr_args("attach-too-big", &[("name", name.into()), ("mb", (MAX_ATTACHMENTS / (1024 * 1024)).into())]);
        let meta = std::fs::metadata(path).map_err(|_| crate::i18n::tr_args("attach-unreadable", &[("name", name.into())]))?;
        if !meta.is_file() {
            return Err(crate::i18n::tr_args("attach-not-a-file", &[("name", name.into())]));
        }
        if usize::try_from(meta.len()).map_or(true, |len| len > room) {
            return Err(too_big());
        }
        let bytes = std::fs::read(path).map_err(|_| crate::i18n::tr_args("attach-unreadable", &[("name", name.into())]))?;
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
            return Err(crate::i18n::tr_args("attach-too-big", &[("name", name.into()), ("mb", (MAX_ATTACHMENTS / (1024 * 1024)).into())]));
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

    /// Sends the open message the way Outlook does: the window closes at once and the message
    /// goes out behind it. Until the server has it, it stays under Concepten, so quitting in
    /// between loses nothing; a failure brings the window back.
    fn send_in_background(&mut self, account: &str, draft: Draft) {
        self.keep_draft();
        let Some(mut c) = self.compose.take() else { return };
        c.changed_at = None;
        c.saved_at = None;
        let outbox = self.engine.send(account, draft);
        self.sending.push(Sending { outbox, window: Box::new(c) });
    }

    /// Opens a draft that was put aside, to write on or send.
    pub fn open_draft(&mut self, id: &str) {
        if let Some(d) = self.engine.session.draft(id) {
            self.compose = Some(Compose::from_stored(d));
        }
    }

    /// Keeps the open message as a draft without closing the window, quietly: the window says
    /// so at its bottom, and a failure is left for closing the window to report.
    fn keep_draft(&mut self) {
        let Some(params) = self.compose.as_ref().and_then(|c| serde_json::to_value(c.as_stored()).ok()) else { return };
        let notice = self.notice.take();
        self.run("draft.save", params);
        let failed = std::mem::replace(&mut self.notice, notice).is_some();
        let id = self.engine.session.last_draft.clone();
        if let Some(c) = self.compose.as_mut()
            && !failed
        {
            c.stored = id;
            c.saved_at = Some(Local::now().timestamp());
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
                if !self.notice.as_ref().is_some_and(|n| matches!(n.tone, crate::notice::Tone::Warning | crate::notice::Tone::Error)) {
                    self.notice = Some(crate::Notice::success(tr("compose-draft-saved")));
                }
            }
            Err(_) => self.notice = Some(crate::Notice::error(tr("compose-draft-not-saved"))),
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

/// A message on its way after its window closed: its entry in the session's outbox and the
/// window, to bring back when sending fails.
pub struct Sending {
    pub outbox: u64,
    window: Box<Compose>,
}

/// Follows the messages on their way: one that went out says so and leaves Concepten, one
/// that failed comes back in its window with the reason, as soon as no other message is open.
pub fn follow_sending(app: &mut MailApp) {
    let mut i = 0;
    while i < app.sending.len() {
        match app.engine.session.outbox.get(&app.sending[i].outbox).cloned() {
            Some(SendState::Sent) => {
                let done = app.sending.remove(i);
                app.forget_draft(done.window.stored.as_deref());
                app.notice = Some(crate::Notice::success(sent()));
            }
            Some(SendState::Failed(e)) if app.compose.is_none() => {
                let mut c = app.sending.remove(i).window;
                c.error = Some(e);
                app.compose = Some(*c);
                app.notice = Some(crate::Notice::error(not_sent()));
            }
            // Sending, or failed while another message is open: that one comes first.
            _ => i += 1,
        }
    }
}

/// What the notice says once a message went out.
pub fn sent() -> String {
    tr("compose-sent")
}

/// What the notice says when a message could not go out; the window says why.
pub fn not_sent() -> String {
    tr("compose-not-sent")
}

/// Draws the window when there is one. Returns after the window closes itself.
pub fn show(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(c) = app.compose.as_mut() else { return };
    let account =
        if c.account_id.is_empty() { app.engine.session.current_account().map(|a| a.id.clone()).unwrap_or_default() } else { c.account_id.clone() };
    let certificate = app.engine.certificate(&account);
    let pgp_key = app.engine.pgp_key(&account);
    let can_sign = certificate.is_some() || pgp_key.is_some();
    // The recipients encrypted mail cannot reach yet, for want of their certificate.
    let missing = match c.draft() {
        Ok(d) if c.encrypt => app.engine.missing_certificates(&account, &[d.to, d.cc, d.bcc].concat()),
        _ => Vec::new(),
    };
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
    // The accounts the message can go out from, as "Naam (adres)".
    let accounts: Vec<(String, String)> = app
        .engine
        .session
        .mail_accounts()
        .map(|a| {
            let (name, email) = (a.address.display(), a.address.email());
            (a.id.clone(), if name == email { email.to_string() } else { format!("{name} ({email})") })
        })
        .collect();
    let from = accounts.iter().find(|(id, _)| *id == account).map(|(_, label)| label.clone());
    let mut new_from: Option<String> = None;
    let mut picked_contact: Option<String> = None;
    let contacts: Vec<String> = app
        .engine
        .session
        .contacts
        .iter()
        .filter_map(|p| p.emails.first().map(|e| if p.name.trim().is_empty() { e.clone() } else { format!("{} <{e}>", p.name.trim()) }))
        .collect();
    let templates = app.engine.session.templates_sorted();
    let now = ctx.input(|i| i.time);
    let saved_text = c.saved_at.map(saved_text);
    let title = tr(if c.from_draft { "compose-title-draft" } else { "compose-title-new" });
    helder::item_window(ctx, t, "compose-window", &title, vec2(900.0, 700.0), &mut open, |ui| {
        // Files dropped onto the window become attachments; files hovering over it get a hint.
        // The window's own input, as it may be a window of its own.
        let dropped: Vec<PathBuf> = ui.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        c.add_files(dropped);
        let hovering = ui.input(|i| !i.raw.hovered_files.is_empty());
        ui.scope(|ui| {
            // The Helder command bar: the blue send button with its "later" half, then quiet
            // icon buttons, and on the right what puts the message away.
            helder::quiet(ui, t, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 8.0);
                    if send_button(ui, t, c.send_at.is_none()).clicked() {
                        send = true;
                    }
                    if c.send_at.is_none() {
                        later_menu(ui, t, |ui| {
                            for (label, ts) in later_options(Local::now()) {
                                if ui.button(label).clicked() {
                                    schedule = Some(ts);
                                    ui.close();
                                }
                            }
                        });
                    }
                    helder::bar_separator(ui, t);
                    if helder::tool_button(ui, t, Icon::Paperclip, &attach()).on_hover_text(tr("compose-attach-hint")).clicked() {
                        pick = true;
                    }
                    template = crate::templates::menu(ui, t, &templates);
                    if can_sign || c.sign == Some(true) {
                        let on = c.sign.unwrap_or(can_sign);
                        let hover = match (&certificate, &pgp_key) {
                            (Some(info), _) => crate::i18n::tr_args(
                                "compose-sign-smime-hint",
                                &[("name", info.name.as_str().into()), ("until", crate::dates::day(info.not_after).into())],
                            ),
                            (None, Some(key)) => {
                                crate::i18n::tr_args("compose-sign-pgp-hint", &[("name", key.name.as_str().into()), ("key", key.grouped().into())])
                            }
                            (None, None) => tr("compose-sign-missing").to_string(),
                        };
                        if helder::toggle_button(ui, t, Icon::ShieldCheck, &sign(), on).on_hover_text(hover).clicked() {
                            c.sign = Some(!on);
                        }
                    }
                    if (can_sign || c.encrypt)
                        && helder::toggle_button(ui, t, Icon::Lock, &encrypt(), c.encrypt).on_hover_text(tr("compose-encrypt-hint")).clicked()
                    {
                        c.encrypt = !c.encrypt;
                    }
                    helder::menu_button(ui, t, Some(Icon::SlidersHorizontal), RichText::new(options_label()).size(14.0).color(t.text), |ui| {
                        ui.checkbox(&mut c.read_receipt, crate::receipt::ask()).on_hover_text(tr("compose-receipt-hint"));
                    });
                    // A narrow window puts the two on a line of their own rather than on top
                    // of the buttons before them.
                    if ui.available_width() < 2.0 * helder::BUTTON_HEIGHT + 24.0 {
                        ui.end_row();
                    }
                    helder::right(ui, |ui| {
                        if icon_button(ui, t, Icon::Trash, &discard_label()).on_hover_text(tr("compose-discard-hint")).clicked() {
                            discard = true;
                        }
                        if icon_button(ui, t, Icon::Save, &put_aside_label()).on_hover_text(tr("compose-put-aside-hint")).clicked() {
                            put_aside = true;
                        }
                        if hovering {
                            ui.label(RichText::new(tr("compose-drop")).color(t.accent_text));
                        }
                    });
                });
            });
            ui.add_space(8.0);
            if let Some(check) = &c.check {
                let mut look_again = false;
                egui::Frame::new().fill(t.flag.gamma_multiply(0.15)).corner_radius(helder::CONTROL_RADIUS).inner_margin(8).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for w in &check.warnings {
                        ui.label(RichText::new(format!("⚠ {w}")).color(t.text));
                    }
                    ui.horizontal(|ui| {
                        if ui.button(send_anyway()).clicked() {
                            anyway = true;
                        }
                        if ui.button(look_again_label()).clicked() {
                            look_again = true;
                        }
                    });
                });
                ui.add_space(8.0);
                if look_again {
                    c.check = None;
                }
            }
            if !missing.is_empty() {
                egui::Frame::new().fill(t.flag.gamma_multiply(0.15)).corner_radius(helder::CONTROL_RADIUS).inner_margin(8).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(format!("🔒 {}", merel_engine::certificates::missing_text(&missing))).color(t.text));
                });
                ui.add_space(8.0);
            }
            if let Some(ts) = c.send_at {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(crate::i18n::tr_args("compose-scheduled", &[("when", when_text(ts).into())])).color(t.accent_text));
                    if ui.small_button(tr("compose-unschedule")).on_hover_text(tr("compose-unschedule-hint")).clicked() {
                        c.send_at = None;
                    }
                });
                ui.add_space(8.0);
            }
            // The message itself: one white card with the addresses, the subject, the
            // attachments, the formatting bar and the text, and what the automatic save did.
            helder::card_frame(t).inner_margin(Margin { left: 24, right: 24, top: 8, bottom: 10 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let wide = ui.available_width() - LABEL_WIDTH - 12.0;
                if let Some(from) = &from {
                    address_row(ui, t, &tr("compose-from"), |ui| {
                        if accounts.len() > 1 {
                            let r = helder::select(ui, t, from, wide, |ui| {
                                for (id, label) in &accounts {
                                    if ui.selectable_label(*id == account, label).clicked() {
                                        new_from = Some(id.clone());
                                        ui.close();
                                    }
                                }
                            });
                            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, from_label()));
                        } else {
                            let r = ui
                                .allocate_ui_with_layout(vec2(wide, helder::BUTTON_HEIGHT), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    ui.set_min_size(vec2(wide, helder::BUTTON_HEIGHT));
                                    ui.label(RichText::new(from).size(helder::FIELD_TEXT).color(t.text));
                                })
                                .response;
                            helder::underline(ui, t, &r);
                        }
                    });
                }
                let show_cc = c.show_cc || !c.cc.trim().is_empty();
                let show_bcc = c.show_bcc || !c.bcc.trim().is_empty();
                address_row(ui, t, &tr("compose-to"), |ui| {
                    let r = helder::underline_field(ui, t, &mut c.to, "", wide - TO_BUTTONS, false);
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, tr("compose-to")));
                    helder::quiet(ui, t, |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if line_button(ui, t, "CC", show_cc).on_hover_text(tr("compose-cc-hint")).clicked() {
                            c.show_cc = !show_cc;
                        }
                        if line_button(ui, t, "BCC", show_bcc).on_hover_text(tr("compose-bcc-hint")).clicked() {
                            c.show_bcc = !show_bcc;
                        }
                        address_book(ui, t, &contacts, &mut picked_contact);
                    });
                });
                crate::contacts::complete_addresses(ui, t, &app.engine.session, &mut c.to);
                if show_cc {
                    address_row(ui, t, "CC", |ui| {
                        helder::underline_field(ui, t, &mut c.cc, "", wide, false)
                            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "CC"));
                    });
                    crate::contacts::complete_addresses(ui, t, &app.engine.session, &mut c.cc);
                }
                if show_bcc {
                    address_row(ui, t, "BCC", |ui| {
                        helder::underline_field(ui, t, &mut c.bcc, "", wide, false)
                            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "BCC"));
                    });
                    crate::contacts::complete_addresses(ui, t, &app.engine.session, &mut c.bcc);
                }
                address_row(ui, t, &tr("compose-subject"), |ui| {
                    let room = wide - SUBJECT_MENU - if c.encrypt { 230.0 } else { 0.0 };
                    helder::underline_field(ui, t, &mut c.subject, "", room, false)
                        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, tr("compose-subject")));
                    if c.encrypt {
                        pill(ui, t, Icon::Lock, &tr("compose-encrypted-pill"));
                    }
                    helder::quiet(ui, t, |ui| crate::importance::subject_menu(ui, t, &mut c.importance));
                });
                if !c.attachments.is_empty() {
                    ui.add_space(12.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                        for (i, a) in c.attachments.iter().enumerate() {
                            if attachment_tile(ui, t, &a.name, a.bytes.len()) {
                                remove = Some(i);
                            }
                        }
                    });
                }
                if let Some(e) = &c.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(e).color(t.flag));
                }
                ui.add_space(10.0);
                rich_edit::bar(ui, t, &mut c.editor, &mut c.body);
                ui.add_space(4.0);
                let status = 22.0;
                egui::ScrollArea::vertical().auto_shrink(false).max_height(ui.available_height() - status).show(ui, |ui| {
                    let height = ui.available_height();
                    rich_edit::edit(ui, t, &mut c.editor, &mut c.body, &tr("compose-body"), "", height);
                });
                ui.horizontal(|ui| {
                    ui.set_height(status);
                    if let Some(text) = &saved_text {
                        helder::right(ui, |ui| ui.label(RichText::new(text).size(13.0).color(t.text_dim)));
                    }
                });
            });
        });
    });
    if discard {
        let stored = c.stored.clone();
        app.compose = None;
        app.forget_draft(stored.as_deref());
        return;
    }
    if put_aside || !open {
        app.put_aside();
        return;
    }
    if let Some(id) = new_from {
        c.account_id = id;
    }
    if let Some(person) = picked_contact {
        let to = c.to.trim_end().trim_end_matches([';', ',']).trim_end();
        c.to = if to.is_empty() { format!("{person}; ") } else { format!("{to}; {person}; ") };
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
        if let Some(paths) = rfd::FileDialog::new().set_title(tr("compose-pick-attachments")).pick_files() {
            c.error = None;
            c.add_files(paths);
        }
    }
    // Encrypted mail does not go out in the clear, and not to someone who could not open it.
    if (send || schedule.is_some() || anyway) && !missing.is_empty() {
        c.error = Some(format!("{} {}", merel_engine::certificates::missing_text(&missing), tr("compose-encrypt-off-to-send")));
        send = false;
        schedule = None;
        anyway = false;
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
                    app.notice = Some(crate::Notice::success(crate::i18n::tr_args("compose-scheduled-notice", &[("when", when_text(ts).into())])));
                }
                return;
            }
            Err(e) => c.error = Some(e),
        }
    }
    if send {
        match c.draft() {
            Ok(draft) => {
                c.error = None;
                app.send_in_background(&account, draft);
            }
            Err(e) => c.error = Some(e),
        }
        return;
    }
    // Outlook keeps what is being written as a draft as it goes: a few seconds after the last
    // change, so a crash or a closed laptop loses little.
    let state = c.state();
    if state != c.seen {
        c.seen = state;
        c.changed_at = Some(now);
    }
    if let Some(at) = c.changed_at {
        let wait = AUTOSAVE_AFTER - (now - at);
        if wait > 0.0 {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
        } else {
            c.changed_at = None;
            if !c.is_blank() {
                app.keep_draft();
                ctx.request_repaint();
            }
        }
    }
}

/// Seconds after the last change that the window keeps the message as a draft.
pub const AUTOSAVE_AFTER: f64 = 3.0;

/// What the bottom of the window says after the automatic save.
pub fn saved_just_now() -> String {
    tr("compose-saved-just-now")
}

fn saved_text(at: i64) -> String {
    let now = Local::now();
    if now.timestamp() - at < 60 {
        return saved_just_now();
    }
    match DateTime::from_timestamp(at, 0).map(|t| t.with_timezone(&Local)) {
        Some(t) => crate::i18n::tr_args("compose-saved-at", &[("time", format!("{:02}:{:02}", t.hour(), t.minute()).into())]),
        None => saved_just_now(),
    }
}

/// The width of the names before the address fields.
const LABEL_WIDTH: f32 = 96.0;
/// The room CC, BCC and the address book take at the end of the Aan line.
const TO_BUTTONS: f32 = 140.0;
/// The room the urgency menu takes at the end of the subject line.
const SUBJECT_MENU: f32 = 140.0;
/// The Van list's name for a screen reader.
pub fn from_label() -> String {
    tr("compose-from")
}
/// The address book button at the end of the Aan line.
pub fn address_book_label() -> String {
    tr("compose-address-book")
}

/// "CC" or "BCC" at the end of the Aan line: shows or hides that line.
fn line_button(ui: &mut Ui, t: &Tokens, text: &str, on: bool) -> egui::Response {
    let color = if on { t.accent_text } else { t.text };
    let r = ui.add(
        egui::Button::new(RichText::new(text).size(14.0).color(color))
            .frame_when_inactive(false)
            .selected(on)
            .corner_radius(helder::CONTROL_RADIUS)
            .min_size(vec2(36.0, 30.0)),
    );
    let label = crate::i18n::tr_args("compose-line", &[("field", text.into())]);
    r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, &label));
    r
}

/// The address book at the end of the Aan line: the contacts with an e-mail address, with a
/// field to find one; a click puts the person on the Aan line.
fn address_book(ui: &mut Ui, t: &Tokens, contacts: &[String], picked: &mut Option<String>) {
    let button = egui::Button::image(Icon::AddressBook.image(helder::ICON, t.text))
        .frame_when_inactive(false)
        .corner_radius(helder::CONTROL_RADIUS)
        .min_size(vec2(32.0, 30.0));
    let (r, _) = egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
        ui.set_min_width(280.0);
        let id = egui::Id::new("compose-address-book");
        let mut find: String = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_default();
        ui.add(egui::TextEdit::singleline(&mut find).hint_text(tr("compose-find-contacts")).desired_width(f32::INFINITY));
        ui.ctx().data_mut(|d| d.insert_temp(id, find.clone()));
        let find = find.to_lowercase();
        let found: Vec<&String> = contacts.iter().filter(|c| c.to_lowercase().contains(&find)).take(50).collect();
        if found.is_empty() {
            ui.label(RichText::new(tr("compose-no-contacts")).color(t.text_dim));
        }
        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            for person in found {
                if ui.selectable_label(false, person).clicked() {
                    *picked = Some(person.clone());
                    ui.close();
                }
            }
        });
    });
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, address_book_label()));
    let _ = r.on_hover_text(tr("compose-address-book-hint"));
}

/// One row of the address block: the name in grey, then the field.
fn address_row<R>(ui: &mut Ui, t: &Tokens, label: &str, add: impl FnOnce(&mut Ui) -> R) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(vec2(LABEL_WIDTH, helder::BUTTON_HEIGHT), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_size(vec2(LABEL_WIDTH, helder::BUTTON_HEIGHT));
            ui.label(RichText::new(label).size(14.0).color(t.text_dim));
        });
        add(ui);
    });
}

/// The blue "Verzenden" button; with `split` it is the left half, the "later" chevron the right.
fn send_button(ui: &mut Ui, t: &Tokens, split: bool) -> egui::Response {
    let r = helder::CONTROL_RADIUS;
    let corners = if split { egui::CornerRadius { nw: r, sw: r, ne: 0, se: 0 } } else { egui::CornerRadius::same(r) };
    ui.add(
        egui::Button::image_and_text(Icon::Send.image(helder::ICON, t.on_accent), RichText::new(send_label()).size(14.0).color(t.on_accent))
            .fill(t.accent)
            .stroke(Stroke::NONE)
            .corner_radius(corners)
            .min_size(vec2(0.0, helder::BUTTON_HEIGHT)),
    )
}

/// The right half of the send button: a chevron that opens the moments for "Later verzenden".
fn later_menu(ui: &mut Ui, t: &Tokens, add: impl FnOnce(&mut Ui)) {
    let r = helder::CONTROL_RADIUS;
    let button = egui::Button::image(Icon::ChevronDown.image(16.0, t.on_accent))
        .fill(t.accent)
        .stroke(Stroke::NONE)
        .corner_radius(egui::CornerRadius { nw: 0, sw: 0, ne: r, se: r })
        .min_size(vec2(30.0, helder::BUTTON_HEIGHT));
    // Glued to the send button, with a thin light line between the two.
    ui.add_space(-ui.spacing().item_spacing.x);
    let (response, _) = egui::containers::menu::MenuButton::from_button(button).ui(ui, add);
    ui.painter().vline(response.rect.left(), response.rect.shrink(8.0).y_range(), Stroke::new(1.0, t.on_accent.gamma_multiply(0.5)));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, later()));
    response.on_hover_text(tr("compose-later-hint"));
}

/// A quiet square button with only an icon; `label` is what a screen reader says.
fn icon_button(ui: &mut Ui, t: &Tokens, icon: Icon, label: &str) -> egui::Response {
    let r = ui.add(
        egui::Button::image(icon.image(helder::ICON, t.text))
            .frame_when_inactive(false)
            .corner_radius(helder::CONTROL_RADIUS)
            .min_size(vec2(helder::BUTTON_HEIGHT, helder::BUTTON_HEIGHT)),
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    r
}

/// A small rounded note with an icon, such as "Sent encrypted".
fn pill(ui: &mut Ui, t: &Tokens, icon: Icon, text: &str) {
    egui::Frame::new().fill(t.checked).corner_radius(12).inner_margin(Margin::symmetric(10, 4)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add(icon.image(14.0, t.accent_text));
        ui.label(RichText::new(text).size(12.0).color(t.accent_text));
    });
}

/// An attachment as a tile: its kind, name and size, and a cross to take it off again.
/// Whether the cross was clicked.
fn attachment_tile(ui: &mut Ui, t: &Tokens, name: &str, bytes: usize) -> bool {
    let mut gone = false;
    egui::Frame::new()
        .fill(t.window)
        .stroke(Stroke::new(1.0, t.card_border))
        .corner_radius(helder::CONTROL_RADIUS)
        .inner_margin(Margin { left: 8, right: 4, top: 6, bottom: 6 })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let kind = Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("").to_uppercase();
                let kind: String = kind.chars().take(4).collect();
                let (rect, _) = ui.allocate_exact_size(vec2(28.0, 28.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 6, t.checked);
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, kind, egui::FontId::proportional(9.0), t.accent_text);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new(name).size(13.0).color(t.text));
                    ui.label(RichText::new(crate::size_text(bytes)).size(12.0).color(t.text_dim));
                });
                let r = ui.add(egui::Button::image(Icon::X.image(14.0, t.text_dim)).frame_when_inactive(false).min_size(vec2(28.0, 28.0)));
                let label = crate::i18n::tr_args("compose-remove-named", &[("name", name.into())]);
                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
                gone = r.on_hover_text(tr("compose-remove-attachment")).clicked();
            });
        });
    gone
}

#[cfg(test)]
mod tests {
    use super::*;
    use merel_model::Flags;

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
            chat: None,
            sealed: Default::default(),
            size: 0,
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
        let dir = std::env::temp_dir().join(format!("merel-compose-{}", std::process::id()));
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
    fn the_action_row_is_one_even_line_of_buttons_and_menus() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut app = MailApp::new(merel_engine::Engine::sample());
        app.open_compose(Kind::New);
        let mut harness = Harness::builder().with_size(vec2(1100.0, 900.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app);
        harness.run();
        harness.step();
        let heights: Vec<f32> = [&send_label(), &later(), &put_aside_label(), &attach(), &crate::templates::menu_label(), &options_label()]
            .iter()
            .map(|label| harness.get_by_label(label).rect().height())
            .collect();
        assert!(
            heights.iter().all(|h| (h - helder::BUTTON_HEIGHT).abs() < 1.0),
            "every button and menu is {} high, not {heights:?}",
            helder::BUTTON_HEIGHT
        );
    }

    #[test]
    fn the_window_lists_attachments_and_removes_one_on_click() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut app = MailApp::new(merel_engine::Engine::sample());
        app.open_compose(Kind::New);
        if let Some(c) = app.compose.as_mut() {
            c.attachments.push(DraftAttachment::new("offerte.pdf", vec![0; 2048]));
            c.attachments.push(DraftAttachment::new("foto.jpg", vec![0; 10]));
        }
        let mut harness = Harness::builder().with_size(vec2(1100.0, 900.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app);
        harness.run();
        harness.get_by_label(&attach());
        harness.get_by_label("offerte.pdf");
        harness.get_by_label("2 kB");
        harness.get_by_label("foto.jpg");
        harness.get_by_label("offerte.pdf weghalen").click();
        harness.run();
        let names: Vec<String> = harness.state().compose.as_ref().map(|c| c.attachments.iter().map(|a| a.name.clone()).collect()).unwrap_or_default();
        assert_eq!(names, ["foto.jpg"]);
        assert!(harness.query_by_label("offerte.pdf").is_none());
    }

    #[test]
    fn the_urgency_menu_marks_the_message_as_urgent_and_the_draft_carries_it() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut app = MailApp::new(merel_engine::Engine::sample());
        app.open_compose(Kind::New);
        let mut h = Harness::builder().with_size(vec2(1100.0, 900.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app);
        h.run();
        h.get_by_label("Urgentie: normaal").click();
        h.run();
        h.get_all_by_label("Hoog").next().expect("the menu item").click();
        h.run();
        assert_eq!(h.state().compose.as_ref().map(|c| c.importance), Some(Importance::High));
        h.get_by_label("Urgentie: hoog");
        assert_eq!(h.state().compose.as_ref().map(|c| c.as_stored().importance), Some(Importance::High));
        assert_eq!(h.state().compose.as_ref().and_then(|c| c.as_stored().to_draft().ok()).map(|d| d.importance), None, "no recipient yet");
        h.get_by_label(&options_label()).click();
        h.run();
        h.get_by_label(&crate::receipt::ask()).click();
        h.run();
        assert_eq!(h.state().compose.as_ref().map(|c| (c.read_receipt, c.as_stored().read_receipt)), Some((true, true)));
    }

    #[test]
    fn later_verzenden_puts_the_message_aside_with_a_moment_and_can_be_undone() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut app = MailApp::new(merel_engine::Engine::sample());
        app.open_compose(Kind::New);
        let mut h = Harness::builder().with_size(vec2(1100.0, 900.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app);
        h.run();
        h.get_by_label(&later()).click();
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
        h.get_by_label(&later()).click();
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
        assert!(h.query_by_label(&later()).is_none(), "no second moment on top of the first");
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
    fn verzenden_closes_the_window_at_once_and_a_failure_brings_it_back() {
        let mut app = MailApp::new(merel_engine::Engine::sample());
        let start = |app: &mut MailApp, subject: &str| {
            app.open_compose(Kind::New);
            if let Some(c) = app.compose.as_mut() {
                c.to = "sanne@example.org".into();
                c.subject = subject.into();
                c.body = "Hoi".into();
            }
            // As send_in_background does, with an outbox entry the test decides about.
            app.keep_draft();
            let window = Box::new(app.compose.take().expect("open"));
            let outbox = 900 + app.sending.len() as u64;
            app.engine.session.outbox.insert(outbox, SendState::Sending);
            app.sending.push(Sending { outbox, window });
            outbox
        };
        let first = start(&mut app, "Planning");
        follow_sending(&mut app);
        assert!(app.compose.is_none() && app.sending.len() == 1, "on its way, the window gone");
        assert_eq!(app.engine.session.drafts.len(), 1, "kept under Concepten until the server has it");
        app.engine.session.outbox.insert(first, SendState::Sent);
        follow_sending(&mut app);
        assert!(app.sending.is_empty() && app.engine.session.drafts.is_empty());
        assert_eq!(app.notice.as_deref(), Some(sent().as_str()));

        // A failure waits for the open window, then comes back with the reason.
        let second = start(&mut app, "Offerte");
        app.engine.session.outbox.insert(second, SendState::Failed("De server weigert het bericht.".into()));
        app.open_compose(Kind::New);
        follow_sending(&mut app);
        assert_eq!(app.sending.len(), 1, "the open message comes first");
        app.compose = None;
        follow_sending(&mut app);
        let c = app.compose.as_ref().expect("back");
        assert_eq!((c.subject.as_str(), c.error.as_deref()), ("Offerte", Some("De server weigert het bericht.")));
        assert_eq!(app.notice.as_deref(), Some(not_sent().as_str()));
        assert_eq!(app.engine.session.drafts.len(), 1, "still under Concepten");
    }

    #[test]
    fn send_asks_first_about_a_missing_attachment_or_subject_and_sends_on_toch_verzenden() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut app = MailApp::new(merel_engine::Engine::sample());
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
        assert!(h.state().sending.is_empty(), "not sent yet");
        assert_eq!(c.check.as_ref().map(|k| k.warnings.clone()), Some(vec![no_attachment(), no_subject()]));
        h.get_by_label(&format!("⚠ {}", no_attachment()));
        h.get_by_label(&format!("⚠ {}", no_subject()));

        // A subject typed in takes that warning away; an attachment would take the other.
        if let Some(c) = h.state_mut().compose.as_mut() {
            c.subject = "Planning".into();
        }
        h.run();
        assert_eq!(h.state().compose.as_ref().and_then(|c| c.check.as_ref()).map(|k| k.warnings.clone()), Some(vec![no_attachment()]));
        h.get_by_label(&look_again_label()).click();
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
        let mut app = MailApp::new(merel_engine::Engine::sample());
        app.open_compose(Kind::New);
        if let Some(c) = app.compose.as_mut() {
            c.to = "sanne@example.org".into();
            c.body = "Zonder onderwerp".into();
        }
        let mut h = Harness::builder().with_size(vec2(1100.0, 900.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app);
        h.run();
        h.get_by_label(&later()).click();
        h.run();
        h.get_by_label("Over een uur").click();
        h.run();
        let c = h.state().compose.as_ref().expect("held back");
        assert!(c.check.as_ref().is_some_and(|k| k.later.is_some() && k.warnings == [no_subject()]));
        assert!(h.state().engine.session.drafts.iter().all(|d| d.send_at.is_none()), "not put aside with the moment yet");
        h.get_by_label(&send_anyway()).click();
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
        let mut app = MailApp::new(merel_engine::Engine::sample());
        app.engine.run("template.save", &serde_json::json!({ "name": "Route", "text": "Kom via de A2, afslag 9." })).unwrap();
        app.open_compose(Kind::New);
        if let Some(c) = app.compose.as_mut() {
            c.body = "Hoi Sanne,\n\n-- \nAlain".into();
        }
        let mut h = Harness::builder().with_size(vec2(1100.0, 900.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app);
        h.run();
        h.get_by_label(&crate::templates::menu_label()).click();
        h.run();
        h.get_by_label("Route").click();
        h.run();
        assert_eq!(h.state().compose.as_ref().map(|c| c.body.as_str()), Some("Hoi Sanne,\n\nKom via de A2, afslag 9.\n\n-- \nAlain"));

        // The text written so far becomes a new template through the window.
        h.get_by_label(&crate::templates::menu_label()).click();
        h.run();
        h.get_by_label(&crate::templates::save_as()).click();
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
    fn the_window_keeps_the_message_as_a_draft_a_moment_after_a_change_and_says_so() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut app = MailApp::new(merel_engine::Engine::sample());
        app.open_compose(Kind::New);
        let mut h = Harness::builder().with_size(vec2(1100.0, 900.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app);
        h.run();
        assert!(h.query_by_label(&saved_just_now()).is_none());
        if let Some(c) = h.state_mut().compose.as_mut() {
            c.subject = "Planning".into();
        }
        h.run();
        assert!(h.state().engine.session.drafts.is_empty(), "not straight away");
        // As if the change was made a while ago.
        if let Some(c) = h.state_mut().compose.as_mut() {
            c.changed_at = Some(-AUTOSAVE_AFTER);
        }
        h.run();
        let drafts = h.state().engine.session.drafts.clone();
        assert_eq!(drafts.len(), 1);
        let c = h.state().compose.as_ref().expect("the window stays open");
        assert_eq!(c.stored.as_deref(), Some(drafts[0].id.as_str()), "later saves update the same draft");
        h.get_by_label(&saved_just_now());
        h.get_by_label("Nieuw bericht");
        assert_eq!(saved_text(Local::now().timestamp() - 3600).len(), "Concept opgeslagen om 00:00".len());
    }

    #[test]
    fn cc_and_bcc_lines_show_on_request_and_the_address_book_fills_aan() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut app = MailApp::new(merel_engine::Engine::sample());
        app.engine.session.contacts.push(merel_model::Contact {
            id: "c1".into(),
            name: "Ravi Kumar".into(),
            emails: vec!["ravi@example.org".into()],
            ..merel_model::Contact::default()
        });
        app.open_compose(Kind::New);
        let mut h = Harness::builder().with_size(vec2(1100.0, 900.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app);
        h.run();
        assert!(h.query_by_label("CC").is_none() && h.query_by_label("BCC").is_none(), "hidden until asked for");
        h.get_by_label("BCC-regel").click();
        h.run();
        assert_eq!(h.query_all_by_label("BCC").count(), 2, "the line's name and its field");
        h.get_by_label(&address_book_label()).click();
        h.run();
        h.get_all_by_label("Ravi Kumar <ravi@example.org>").next().expect("the contact").click();
        h.run();
        let c = h.state_mut().compose.as_mut().expect("the window");
        assert_eq!(c.to, "Ravi Kumar <ravi@example.org>; ");
        c.bcc = "geheim@example.org".into();
        let d = c.draft().expect("a draft");
        assert_eq!(d.bcc.iter().map(|a| a.email()).collect::<Vec<_>>(), ["geheim@example.org"]);
        assert!(Compose::from_stored(&c.as_stored()).bcc == "geheim@example.org", "the blind copy stays with the draft");
    }

    #[test]
    fn formatting_from_the_bar_travels_with_the_draft() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut app = MailApp::new(merel_engine::Engine::sample());
        app.open_compose(Kind::New);
        if let Some(c) = app.compose.as_mut() {
            c.to = "sanne@example.org".into();
            c.body = "Belangrijk: morgen vrij".into();
        }
        let mut h = Harness::builder().with_size(vec2(1100.0, 900.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app);
        h.run();
        assert_eq!(h.state().compose.as_ref().and_then(|c| c.draft().ok()).map(|d| d.rich), Some(None), "plain text goes as plain text");
        h.get_by_label(&rich_edit::bullets()).click();
        h.run();
        let c = h.state().compose.as_ref().expect("the window");
        let rich = c.draft().ok().and_then(|d| d.rich).expect("formatted");
        assert_eq!(rich.paras[0].list, merel_model::rich::List::Bullet);
        let again = Compose::from_stored(&c.as_stored());
        assert_eq!(again.editor.rich.paras[0].list, merel_model::rich::List::Bullet, "and comes back with it");
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
        use merel_model::Folder;
        let mut app = MailApp::new(merel_engine::Engine::sample());
        let attachment = |name: &str, bytes: &[u8]| merel_mime::Attachment {
            name: name.into(),
            mime: "application/octet-stream".into(),
            size: bytes.len(),
            bytes: std::sync::Arc::from(bytes),
        };
        let body = merel_engine::Body { attachments: vec![attachment("offerte.pdf", b"%PDF"), attachment("foto.jpg", b"jpg")], ..Default::default() };
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
            parent: None,
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
