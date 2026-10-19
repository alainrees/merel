//! The compose window: new mail, reply, reply all and forward, with attachments picked from
//! the computer's file dialog or dropped onto the window.

use std::path::{Path, PathBuf};

use egui::{Margin, RichText, Stroke, Ui, vec2};

use mailcraft_engine::SendState;
use mailcraft_model::{Address, Draft, DraftAttachment, MAX_ATTACHMENTS, MessageHeader, StoredDraft, parse_address_list};

use crate::{MailApp, Tokens, dates};

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

fn prefixed(subject: &str, prefix: &str) -> String {
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
        let to = parse_address_list(&self.to).map_err(|_| "Controleer de adressen bij Aan.".to_string())?;
        let cc = parse_address_list(&self.cc).map_err(|_| "Controleer de adressen bij CC.".to_string())?;
        if to.is_empty() {
            return Err("Vul ten minste één ontvanger in bij Aan.".into());
        }
        Ok(Draft {
            to,
            cc,
            subject: self.subject.clone(),
            body: self.body.clone(),
            in_reply_to: self.in_reply_to.clone(),
            attachments: self.attachments.clone(),
        })
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
        self.compose = Some(Compose::new(kind, original).with_signature(&signature).opened_from(&account_id));
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
    let mut discard = false;
    let mut put_aside = false;
    let mut pick = false;
    let mut remove = None;
    egui::Window::new(if c.stored.is_some() { "Concept" } else { "Nieuw bericht" })
        .open(&mut open)
        .default_size(vec2(680.0, 520.0))
        .min_size(vec2(420.0, 320.0))
        .collapsible(false)
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.reading).stroke(Stroke::new(1.0, t.border)).inner_margin(Margin::same(14)))
        .show(ctx, |ui| {
            ui.add_enabled_ui(!sending, |ui| {
                ui.horizontal(|ui| {
                    let b = egui::Button::new(RichText::new("Verzenden").color(t.on_accent)).fill(t.accent).min_size(vec2(110.0, 30.0));
                    if ui.add(b).clicked() {
                        send = true;
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
                    if sending {
                        ui.label(RichText::new("Verzenden…").color(t.text_dim));
                    } else if hovering {
                        ui.label(RichText::new("Laat los om toe te voegen").color(t.accent_text));
                    }
                });
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
    if pick {
        // The computer's own dialog; it blocks until the person has chosen or cancelled.
        if let Some(paths) = rfd::FileDialog::new().set_title("Bijlagen kiezen").pick_files() {
            c.error = None;
            c.add_files(paths);
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
    fn a_window_knows_when_nothing_was_written_and_round_trips_as_a_draft() {
        let h = header();
        let untouched = Compose::new(Kind::Reply, Some((&h, "tien uur?", "me@x.nl"))).with_signature("Alain").opened_from("acc");
        assert!(untouched.is_blank(), "a quote and a signature alone are not a draft");
        let mut written = Compose::new(Kind::Reply, Some((&h, "tien uur?", "me@x.nl"))).with_signature("Alain").opened_from("acc");
        written.body = format!("Prima.{}", written.body);
        assert!(!written.is_blank());
        written.attachments.push(DraftAttachment::new("a.txt", b"x".to_vec()));
        let stored = written.as_stored();
        assert_eq!((stored.id.as_str(), stored.account_id.as_str(), stored.to.as_str()), ("", "acc", "Sanne <sanne@example.org>"));
        assert_eq!(stored.in_reply_to.as_deref(), Some("<x@example.org>"));
        let again = Compose::from_stored(&StoredDraft { id: "d-1".into(), ..stored });
        assert_eq!((again.stored.as_deref(), again.subject.as_str(), again.attachments.len()), (Some("d-1"), "Re: Planning", 1));
        assert!(again.body.starts_with("Prima."));
        let mut emptied = again;
        emptied.to.clear();
        emptied.subject.clear();
        emptied.body.clear();
        emptied.attachments.clear();
        assert!(emptied.is_blank(), "a draft emptied on purpose is not kept");
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
