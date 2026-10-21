//! Sending mail over SMTP with `lettre`. TLS is mandatory: implicit TLS or STARTTLS, both
//! through rustls with the platform's certificate verifier. There is no plain-text path.

use std::time::Duration;

use lettre::message::header::{ContentType, HeaderName, HeaderValue};
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::client::{Tls as SmtpTls, TlsParameters};
use lettre::{Message, SmtpTransport, Transport};
use merel_model::{Address, Draft, Importance, MAX_ATTACHMENTS, Server, Tls};
use merel_secrets::Secret;

/// How to log in.
#[derive(Clone, Debug)]
pub enum Credential {
    Password(Secret),
    /// An OAuth 2.0 access token, sent with SASL XOAUTH2.
    OAuth2(Secret),
}

#[derive(Debug, thiserror::Error)]
pub enum SmtpError {
    #[error("the message could not be built: {0}")]
    Build(String),
    #[error("could not connect to {host}: {reason}")]
    Connect { host: String, reason: String },
    #[error("the server refused the user name or password")]
    Login,
    #[error("the server refused the message: {0}")]
    Refused(String),
}

fn mailbox(a: &Address) -> Result<Mailbox, SmtpError> {
    let addr = a.email().parse::<lettre::Address>().map_err(|e| SmtpError::Build(e.to_string()))?;
    Ok(Mailbox::new(display_name(a), addr))
}

/// The name shown with an address, left out when it is an address itself: a display name like
/// `alain@voorbeeld.nl <alain@voorbeeld.nl>` is what spoofed mail looks like, Outlook marks it
/// "Niet-geverifieerd", and with the `@` it also went out base64-encoded.
fn display_name(a: &Address) -> Option<String> {
    a.name.as_deref().map(str::trim).filter(|n| !n.is_empty() && !n.contains('@')).map(str::to_string)
}

/// A Message-ID under the sender's own domain: lettre's default would put the computer's
/// host name on the wire, which says more about the sender than a mail header should.
fn message_id(from: &Address) -> String {
    use std::hash::{BuildHasher, Hasher};
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(now);
    let a = h.finish();
    let b = std::collections::hash_map::RandomState::new().build_hasher().finish();
    let domain = from.email().rsplit('@').next().unwrap_or("localhost");
    format!("<{now:x}.{a:016x}{b:016x}@{domain}>")
}

/// The RFC 5322 message for `draft`, as an SMTP server will receive it: blind copies are in
/// the envelope and not in the header.
pub fn build(from: &Address, draft: &Draft) -> Result<Message, SmtpError> {
    build_message(from, draft, false)
}

/// The message for a mail store that sends it itself (Exchange, Microsoft 365, Gmail): those
/// read the blind copies from the `Bcc` header and take it off before the message goes on.
pub fn build_for_store(from: &Address, draft: &Draft) -> Result<Message, SmtpError> {
    build_message(from, draft, true)
}

/// The body: the text alone, or with formatting the text beside an HTML version made from it,
/// for the reader's program to choose.
fn text_part(draft: &Draft) -> MultiPart {
    let rich = draft.rich.as_ref().filter(|r| !r.is_plain());
    let (plain, html) = match rich {
        Some(r) => (r.to_plain(&draft.body), r.to_html(&draft.body)),
        None => (draft.body.clone(), String::new()),
    };
    MultiPart::alternative().singlepart(SinglePart::plain(plain)).singlepart(SinglePart::html(html))
}

fn build_message(from: &Address, draft: &Draft, keep_bcc: bool) -> Result<Message, SmtpError> {
    if draft.to.is_empty() {
        return Err(SmtpError::Build("Vul ten minste één ontvanger in.".into()));
    }
    if draft.attachment_bytes() > MAX_ATTACHMENTS {
        return Err(SmtpError::Build(format!("De bijlagen zijn samen te groot; maximaal {} MB.", MAX_ATTACHMENTS / (1024 * 1024))));
    }
    let mut b = Message::builder().from(mailbox(from)?).subject(draft.subject.trim()).date_now().message_id(Some(message_id(from)));
    for a in &draft.to {
        b = b.to(mailbox(a)?);
    }
    for a in &draft.cc {
        b = b.cc(mailbox(a)?);
    }
    for a in &draft.bcc {
        b = b.bcc(mailbox(a)?);
    }
    if keep_bcc && !draft.bcc.is_empty() {
        b = b.keep_bcc();
    }
    if let Some(id) = &draft.in_reply_to {
        b = b.in_reply_to(id.clone()).references(id.clone());
    }
    // Urgency the way Outlook and Thunderbird read it: both headers, since each looks at one.
    let urgency = match draft.importance {
        Importance::High => Some(("1 (Highest)", "High")),
        Importance::Low => Some(("5 (Lowest)", "Low")),
        Importance::Normal => None,
    };
    if let Some((priority, importance)) = urgency {
        b = b
            .raw_header(HeaderValue::new(HeaderName::new_from_ascii_str("X-Priority"), priority.to_string()))
            .raw_header(HeaderValue::new(HeaderName::new_from_ascii_str("Importance"), importance.to_string()));
    }
    // Outlook's "Leesbevestiging vragen": the reader's program may answer to this address.
    if draft.read_receipt {
        b = b.raw_header(HeaderValue::new(HeaderName::new_from_ascii_str("Disposition-Notification-To"), format!("<{}>", from.email())));
    }
    // A chat message, Delta Chat's way: the version, and for a group its id and name.
    if let Some(chat) = &draft.chat {
        b = b.raw_header(HeaderValue::new(HeaderName::new_from_ascii_str("Chat-Version"), merel_model::CHAT_VERSION.to_string()));
        if let Some(id) = &chat.group_id {
            b = b.raw_header(HeaderValue::new(HeaderName::new_from_ascii_str("Chat-Group-ID"), id.clone()));
            if let Some(name) = &chat.group_name {
                b = b.raw_header(HeaderValue::new(HeaderName::new_from_ascii_str("Chat-Group-Name"), name.clone()));
            }
        }
        // A step of an invitation's handshake; the engine encrypts all but the first, which
        // then carries nothing secret.
        if let Some(h) = &chat.handshake {
            b = b.raw_header(HeaderValue::new(HeaderName::new_from_ascii_str("Secure-Join"), h.step.clone()));
            for (name, value) in
                [("Secure-Join-Invitenumber", &h.invitenumber), ("Secure-Join-Auth", &h.auth), ("Secure-Join-Fingerprint", &h.fingerprint)]
            {
                if let Some(v) = value {
                    b = b.raw_header(HeaderValue::new(HeaderName::new_from_ascii_str(name), v.clone()));
                }
            }
            // The first step goes out unencrypted, and chatmail servers only let it through
            // in this one shape: a multipart/mixed with the step as its only, plain part.
            if h.step == "vc-request" {
                return b
                    .multipart(MultiPart::mixed().singlepart(SinglePart::plain(draft.body.clone())))
                    .map_err(|e| SmtpError::Build(e.to_string()));
            }
        }
    }
    let formatted = draft.rich.as_ref().is_some_and(|r| !r.is_plain());
    if draft.attachments.is_empty() {
        if formatted {
            return b.multipart(text_part(draft)).map_err(|e| SmtpError::Build(e.to_string()));
        }
        return b.header(ContentType::TEXT_PLAIN).body(draft.body.clone()).map_err(|e| SmtpError::Build(e.to_string()));
    }
    let mut parts = if formatted {
        MultiPart::mixed().multipart(text_part(draft))
    } else {
        MultiPart::mixed().singlepart(SinglePart::builder().header(ContentType::TEXT_PLAIN).body(draft.body.clone()))
    };
    for a in &draft.attachments {
        let kind =
            ContentType::parse(&a.mime).or_else(|_| ContentType::parse("application/octet-stream")).map_err(|e| SmtpError::Build(e.to_string()))?;
        parts = parts.singlepart(Attachment::new(a.name.clone()).body(a.bytes.clone(), kind));
    }
    b.multipart(parts).map_err(|e| SmtpError::Build(e.to_string()))
}

/// Sends `message` through `server` as `username`.
pub fn send(server: &Server, username: &str, credential: &Credential, message: &Message) -> Result<(), SmtpError> {
    send_raw(server, username, credential, message, &message.formatted())
}

/// Sends `raw` to the sender and recipients of `message`: for a message that was changed after
/// it was built, such as a signed one.
pub fn send_raw(server: &Server, username: &str, credential: &Credential, message: &Message, raw: &[u8]) -> Result<(), SmtpError> {
    let params = TlsParameters::new(server.host.clone()).map_err(|e| SmtpError::Connect { host: server.host.clone(), reason: e.to_string() })?;
    let tls = match server.tls {
        Tls::Implicit => SmtpTls::Wrapper(params),
        Tls::StartTls => SmtpTls::Required(params),
    };
    let (secret, mechanisms) = match credential {
        Credential::Password(p) => (p.expose().to_string(), vec![Mechanism::Plain, Mechanism::Login]),
        Credential::OAuth2(t) => (t.expose().to_string(), vec![Mechanism::Xoauth2]),
    };
    let transport = SmtpTransport::builder_dangerous(server.host.as_str())
        .port(server.port)
        .tls(tls)
        .credentials(Credentials::new(username.to_string(), secret))
        .authentication(mechanisms)
        .timeout(Some(Duration::from_secs(30)))
        .build();
    match transport.send_raw(message.envelope(), raw) {
        Ok(_) => Ok(()),
        Err(e) if e.is_client() || e.is_permanent() && e.to_string().to_lowercase().contains("auth") => Err(SmtpError::Login),
        Err(e) if e.is_response() || e.is_permanent() || e.is_transient() => Err(SmtpError::Refused(e.to_string())),
        Err(e) => Err(SmtpError::Connect { host: server.host.clone(), reason: e.to_string() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_that_is_an_address_is_left_out() {
        let named = |name: &str| Address::new(Some(name), "alain@voorbeeld.nl").unwrap();
        assert_eq!(display_name(&named("alain@voorbeeld.nl")), None);
        assert_eq!(display_name(&named("Alain <alain@voorbeeld.nl>")), None);
        assert_eq!(display_name(&named("Alain Tester")).as_deref(), Some("Alain Tester"));
        let draft = Draft { to: vec![named("Alain Tester")], subject: "test".into(), body: "hoi".into(), ..Draft::default() };
        let raw = String::from_utf8(build(&named("alain@voorbeeld.nl"), &draft).unwrap().formatted()).unwrap();
        assert!(raw.contains("From: alain@voorbeeld.nl\r\n"), "{raw}");
    }

    #[test]
    fn a_chat_message_carries_the_chat_headers() {
        use merel_model::ChatTag;
        let from = Address::new(None, "jij@example.org").unwrap();
        let to = vec![Address::new(None, "sanne@example.org").unwrap()];
        let plain = Draft { to: to.clone(), subject: "Hoi".into(), body: "x".into(), ..Draft::default() };
        assert!(!String::from_utf8(build(&from, &plain).unwrap().formatted()).unwrap().contains("Chat-"));
        let one = Draft { chat: Some(ChatTag::default()), ..plain.clone() };
        let text = String::from_utf8(build(&from, &one).unwrap().formatted()).unwrap();
        assert!(text.contains("Chat-Version: 1.0\r\n") && !text.contains("Chat-Group"), "{text}");
        let group = ChatTag { group_id: Some("Ab-9_x".into()), group_name: Some("Familie Rees".into()), handshake: None };
        let text = String::from_utf8(build(&from, &Draft { chat: Some(group), ..plain.clone() }).unwrap().formatted()).unwrap();
        assert!(text.contains("Chat-Group-ID: Ab-9_x\r\n") && text.contains("Chat-Group-Name: Familie Rees\r\n"), "{text}");

        // The first step of an invitation: its token, and the one shape chatmail servers let pass.
        let step = merel_model::Handshake { step: "vc-request".into(), invitenumber: Some("abc123".into()), ..Default::default() };
        let request = Draft { chat: Some(ChatTag { handshake: Some(step), ..ChatTag::default() }), body: "Secure-Join: vc-request".into(), ..plain };
        let text = String::from_utf8(build(&from, &request).unwrap().formatted()).unwrap();
        assert!(text.contains("Secure-Join: vc-request\r\n") && text.contains("Secure-Join-Invitenumber: abc123\r\n"), "{text}");
        assert!(text.contains("multipart/mixed") && text.matches("Content-Type: text/plain").count() == 1, "{text}");
    }

    #[test]
    fn builds_a_plain_text_message_with_threading_headers() {
        let from = Address::new(Some("Jij"), "jij@example.org").unwrap();
        let draft = Draft {
            to: vec![Address::new(Some("Sanne"), "sanne@example.org").unwrap()],
            cc: vec![],
            subject: "Re: Planning".into(),
            body: "Prima, tien uur.\n".into(),
            in_reply_to: Some("<abc@example.org>".into()),
            attachments: Vec::new(),
            importance: Importance::High,
            read_receipt: true,
            sign: None,
            encrypt: false,
            ..Draft::default()
        };
        let m = build(&from, &draft).unwrap();
        let text = String::from_utf8(m.formatted()).unwrap();
        assert!(text.contains("From: \"Jij\" <jij@example.org>") || text.contains("From: Jij <jij@example.org>"));
        assert!(text.contains("To: \"Sanne\" <sanne@example.org>") || text.contains("To: Sanne <sanne@example.org>"));
        assert!(text.contains("In-Reply-To: <abc@example.org>"));
        assert!(text.contains("Message-ID: <") && text.contains("@example.org>"));
        assert_ne!(message_id(&from), message_id(&from));
        assert!(text.contains("Prima, tien uur."));
        assert!(text.contains("X-Priority: 1 (Highest)\r\n") && text.contains("Importance: High\r\n"), "{text}");
        assert!(text.contains("Disposition-Notification-To: <jij@example.org>\r\n"), "{text}");
    }

    #[test]
    fn attachments_make_a_mixed_message_with_the_file_type_and_name() {
        use merel_model::DraftAttachment;
        let from = Address::new(None, "jij@example.org").unwrap();
        let draft = Draft {
            to: vec![Address::new(None, "sanne@example.org").unwrap()],
            body: "Zie bijlage.".into(),
            attachments: vec![
                DraftAttachment::new("/Users/alain/Documents/offerte.pdf", b"%PDF-1.4".to_vec()),
                DraftAttachment::new("x.unknown", vec![1, 2]),
            ],
            ..Draft::default()
        };
        let text = String::from_utf8(build(&from, &draft).unwrap().formatted()).unwrap();
        assert!(text.contains("Content-Type: multipart/mixed"));
        assert!(!text.contains("Importance:") && !text.contains("X-Priority:"), "normal importance adds no header");
        assert!(!text.contains("Disposition-Notification-To"), "no receipt asked, no header");
        assert!(text.contains("Zie bijlage."));
        assert!(text.contains("Content-Type: application/pdf"), "{text}");
        assert!(text.contains("filename=\"offerte.pdf\""), "{text}");
        assert!(text.contains("Content-Type: application/octet-stream"));
        assert!(!text.contains("/Users/"), "the path stays private");
        let parsed = merel_mime::parse_body(text.as_bytes());
        assert_eq!(parsed.attachments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["offerte.pdf", "x.unknown"]);
        assert_eq!(parsed.text.trim(), "Zie bijlage.");

        let huge = Draft { attachments: vec![DraftAttachment::new("a.bin", vec![0; MAX_ATTACHMENTS + 1])], ..draft };
        assert!(build(&from, &huge).is_err());
    }

    #[test]
    fn formatted_text_goes_as_html_beside_the_text_and_attachments_after_it() {
        use merel_model::DraftAttachment;
        use merel_model::rich::{List, Rich};
        let from = Address::new(None, "jij@example.org").unwrap();
        let body = "Agenda\nbegroting\nplanning";
        let mut rich = Rich::plain(body);
        rich.apply(0..6, |s| s.bold = true);
        rich.apply_paras(1..3, |p| p.list = List::Bullet);
        let draft = Draft { to: vec![Address::new(None, "sanne@example.org").unwrap()], body: body.into(), rich: Some(rich), ..Draft::default() };
        let text = String::from_utf8(build(&from, &draft).unwrap().formatted()).unwrap();
        assert!(text.contains("Content-Type: multipart/alternative"), "{text}");
        assert!(text.contains("Content-Type: text/plain") && text.contains("Content-Type: text/html"), "{text}");
        let parsed = merel_mime::parse_body(text.as_bytes());
        assert_eq!(parsed.text.trim().replace("\r\n", "\n"), "Agenda\n• begroting\n• planning", "the text part writes the bullets out");
        assert!(parsed.rich.is_some() && !parsed.from_html, "the reader gets both parts");

        let with_file = Draft { attachments: vec![DraftAttachment::new("a.txt", b"x".to_vec())], ..draft };
        let text = String::from_utf8(build(&from, &with_file).unwrap().formatted()).unwrap();
        assert!(text.contains("multipart/mixed") && text.contains("multipart/alternative"), "{text}");
        assert_eq!(merel_mime::parse_body(text.as_bytes()).attachments.len(), 1);
    }

    #[test]
    fn blind_copies_are_in_the_envelope_and_only_in_the_header_for_a_store() {
        let from = Address::new(None, "jij@example.org").unwrap();
        let draft = Draft {
            to: vec![Address::new(None, "sanne@example.org").unwrap()],
            bcc: vec![Address::new(None, "geheim@example.org").unwrap()],
            body: "Hoi".into(),
            ..Draft::default()
        };
        let m = build(&from, &draft).unwrap();
        assert!(m.envelope().to().iter().any(|a| a.to_string() == "geheim@example.org"));
        assert!(!String::from_utf8(m.formatted()).unwrap().contains("geheim"), "SMTP readers do not see the blind copy");
        let stored = String::from_utf8(build_for_store(&from, &draft).unwrap().formatted()).unwrap();
        assert!(stored.contains("Bcc: geheim@example.org"), "{stored}");
    }

    #[test]
    fn refuses_an_empty_recipient_list() {
        let from = Address::new(None, "jij@example.org").unwrap();
        assert!(build(&from, &Draft::default()).is_err());
    }
}
