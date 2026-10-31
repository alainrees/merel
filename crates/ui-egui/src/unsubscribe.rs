//! "Unsubscribe", as Outlook offers it over a newsletter: the `List-Unsubscribe` header of a
//! message names a mail address or a web page that takes you off the list. Merel shows a
//! line over such a message and, after asking, either sends the mail from the account the
//! message came in on or opens the page in the browser. Nothing happens without the click:
//! the header is the sender's word, and a visit to the page tells the sender the mail was read.
//! Once the request has gone out, the message itself goes to the Junk folder.

use crate::i18n::tr;
use egui::RichText;
use merel_engine::SendState;
use merel_model::{Address, Draft, FolderKind};

use crate::{MailApp, Tokens};

/// A way off the list, in the order Merel prefers: mail first, as it tells the sender no
/// more than the address it already has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Way {
    Mail { to: Address, subject: String },
    Web(String),
}

/// The ways the header offers, mail before web; a target that does not parse is skipped.
pub fn ways(targets: &[String]) -> Vec<Way> {
    let mut mail = Vec::new();
    let mut web = Vec::new();
    for t in targets {
        let lower = t.to_ascii_lowercase();
        if let Some(rest) = lower.starts_with("mailto:").then(|| &t[7..]) {
            let (addr, query) = rest.split_once('?').unwrap_or((rest, ""));
            let Ok(to) = Address::new(None, addr.trim()) else { continue };
            let subject = query
                .split('&')
                .find_map(|kv| kv.split_once('=').filter(|(k, _)| k.eq_ignore_ascii_case("subject")).map(|(_, v)| percent_decode(v)))
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "unsubscribe".to_string());
            mail.push(Way::Mail { to, subject });
        } else if lower.starts_with("https://") {
            web.push(Way::Web(t.clone()));
        }
    }
    mail.extend(web);
    mail
}

/// `%20` and `+` back to characters; anything odd is left as it is.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if bytes.len() >= i + 3 && s.is_char_boundary(i + 1) && s.is_char_boundary(i + 3) => {
                match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The question before anything goes out.
#[derive(Clone, Debug)]
pub struct Confirm {
    pub message_id: String,
    pub sender: String,
    pub way: Way,
}

/// The host of a web target, for the question.
fn host(url: &str) -> &str {
    url.trim_start_matches("https://").split(['/', '?', '#']).next().unwrap_or(url)
}

/// What the strip over a newsletter says.
pub(crate) fn strip() -> String {
    tr("unsubscribe-strip")
}

impl MailApp {
    /// Opens the question for the first way the message offers.
    pub fn ask_unsubscribe(&mut self, message_id: &str, sender: &str, targets: &[String]) {
        if let Some(way) = ways(targets).into_iter().next() {
            self.unsubscribe = Some(Confirm { message_id: message_id.to_string(), sender: sender.to_string(), way });
        }
    }

    /// Does what the person agreed to: the mail goes out from the account the message came in
    /// on, or the page opens in the browser. Then the message goes to Junk; a request that
    /// failed leaves it where it is.
    fn unsubscribe_now(&mut self, ctx: &egui::Context, c: &Confirm) {
        let account = self.engine.session.message(&c.message_id).and_then(|m| self.engine.session.folder(&m.folder_id)).map(|f| f.account_id.clone());
        let Some(account) = account else {
            self.notice = Some(crate::Notice::warning(tr("message-gone")));
            return;
        };
        let done = match &c.way {
            Way::Mail { to, subject } => {
                let draft = Draft { to: vec![to.clone()], subject: subject.clone(), body: "unsubscribe".to_string(), ..Draft::default() };
                let id = self.engine.send(&account, draft);
                if let Some(SendState::Failed(e)) = self.engine.session.outbox.get(&id) {
                    self.notice = Some(crate::Notice::error(e.clone()));
                    return;
                }
                crate::i18n::tr_args("unsubscribe-sent", &[("email", to.email().into())])
            }
            Way::Web(url) => {
                ctx.open_url(egui::OpenUrl::new_tab(url));
                tr("unsubscribe-page-opened")
            }
        };
        let junk = self.engine.session.folders.iter().find(|f| f.account_id == account && f.kind == FolderKind::Junk).map(|f| f.id.clone());
        let Some(junk) = junk else {
            self.notice = Some(crate::Notice::success(done));
            return;
        };
        self.run("message.move", serde_json::json!({ "id": c.message_id, "folder_id": junk }));
        if !self.notice.as_ref().is_some_and(|n| n.tone == crate::notice::Tone::Error) {
            self.notice = Some(crate::Notice::success(format!("{done} {}", tr("unsubscribe-moved"))));
        }
    }
}

/// The question window, when it is open.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(c) = app.unsubscribe.clone() else { return };
    let text = match &c.way {
        Way::Mail { to, .. } => crate::i18n::tr_args("unsubscribe-by-mail", &[("email", to.email().into()), ("sender", c.sender.as_str().into())]),
        Way::Web(url) => crate::i18n::tr_args("unsubscribe-by-web", &[("host", host(url).into()), ("sender", c.sender.as_str().into())]),
    };
    let mut go = false;
    let mut close = false;
    if crate::helder::popup(ctx, t, "unsubscribe", &tr("unsubscribe-title"), 340.0, |ui| {
        ui.label(RichText::new(text.as_str()).color(t.text));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.add(crate::helder::primary_text(t, &tr("unsubscribe-yes"))).clicked() {
                go = true;
            }
            if ui.button(tr("cancel")).clicked() {
                close = true;
            }
        });
    }) {
        close = true;
    }
    if go {
        app.unsubscribe_now(ctx, &c);
    }
    if go || close {
        app.unsubscribe = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ways_put_mail_first_and_read_the_subject() {
        let targets = [
            "https://example.org/u?id=7".to_string(),
            "mailto:leave@example.org?subject=Stop%20nieuwsbrief&x=1".to_string(),
            "mailto:not an address".to_string(),
        ];
        let ways = ways(&targets);
        assert_eq!(ways.len(), 2);
        assert_eq!(ways[0], Way::Mail { to: Address::new(None, "leave@example.org").unwrap(), subject: "Stop nieuwsbrief".into() });
        assert_eq!(ways[1], Way::Web("https://example.org/u?id=7".into()));
        assert_eq!(
            super::ways(&["mailto:a@b.nl".to_string()])[0],
            Way::Mail { to: Address::new(None, "a@b.nl").unwrap(), subject: "unsubscribe".into() }
        );
        assert_eq!(host("https://news.example.org/u?id=7"), "news.example.org");
        assert_eq!(percent_decode("a+b%2Fc%zz"), "a b/c%zz");
    }
}
