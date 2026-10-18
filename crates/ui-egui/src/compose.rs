//! The compose window: new mail, reply, reply all and forward.

use egui::{Margin, RichText, Stroke, Ui, vec2};

use mailcraft_engine::SendState;
use mailcraft_model::{Address, Draft, MessageHeader, parse_address_list};

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

    pub fn draft(&self) -> Result<Draft, String> {
        let to = parse_address_list(&self.to).map_err(|_| "Controleer de adressen bij Aan.".to_string())?;
        let cc = parse_address_list(&self.cc).map_err(|_| "Controleer de adressen bij CC.".to_string())?;
        if to.is_empty() {
            return Err("Vul ten minste één ontvanger in bij Aan.".into());
        }
        Ok(Draft { to, cc, subject: self.subject.clone(), body: self.body.clone(), in_reply_to: self.in_reply_to.clone() })
    }
}

impl MailApp {
    pub fn open_compose(&mut self, kind: Kind) {
        let session = &self.engine.session;
        let me = session.current_account().map(|a| a.address.email().to_string()).unwrap_or_default();
        let original = session.selected().map(|m| {
            let text = session.bodies.get(&m.id).map(|b| b.text.as_str()).unwrap_or(m.preview.as_str());
            (m, text, me.as_str())
        });
        self.compose = Some(Compose::new(kind, original));
    }
}

/// Draws the window when there is one. Returns after the window closes itself.
pub fn show(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(c) = app.compose.as_mut() else { return };
    // Follow the draft through the outbox.
    if let Some(id) = c.draft_id {
        match app.engine.session.outbox.get(&id) {
            Some(SendState::Sent) => {
                app.compose = None;
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
    let mut open = true;
    let mut send = false;
    let mut discard = false;
    egui::Window::new("Nieuw bericht")
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
                    if ui.add(egui::Button::new("Verwijderen").min_size(vec2(100.0, 30.0))).clicked() {
                        discard = true;
                    }
                    if sending {
                        ui.label(RichText::new("Verzenden…").color(t.text_dim));
                    }
                });
                ui.add_space(8.0);
                field(ui, t, "Aan", &mut c.to);
                field(ui, t, "CC", &mut c.cc);
                field(ui, t, "Onderwerp", &mut c.subject);
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
    if discard || !open {
        app.compose = None;
        return;
    }
    if send {
        match c.draft() {
            Ok(draft) => {
                let account = app.engine.session.current_account().map(|a| a.id.clone()).unwrap_or_default();
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
    fn forward_has_no_recipient_and_re_is_not_doubled() {
        let h = MessageHeader { subject: "Re: Planning".into(), ..header() };
        let c = Compose::new(Kind::Forward, Some((&h, "t", "me@x.nl")));
        assert_eq!(c.subject, "Fw: Re: Planning");
        assert!(c.draft().is_err());
        assert_eq!(Compose::new(Kind::Reply, Some((&h, "t", "me@x.nl"))).subject, "Re: Planning");
    }
}
