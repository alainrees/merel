//! Message parsing: RFC 5322 headers and MIME bodies, through `mail-parser`.
//!
//! Everything that comes out of here is bounded and cleaned: names and subjects go through
//! [`merel_model::clean_text`], addresses through [`Address::new`], and HTML goes through
//! `merel_html` into a small layout tree (no scripts, no remote loads can survive that, and
//! of the styles only colours, sizes and alignments read into numbers), with a plain-text rendering next to it for previews and replies.

use std::sync::Arc;

mod redact;
pub use redact::{Redaction, redact};

use mail_parser::{MessageParser, MimeHeaders};
use merel_html::Document;
use merel_model::{Address, ChatTag, Importance, clean_text, parse_address_list};

/// Longest message body (bytes) that is parsed; the rest is dropped with a note.
pub const MAX_BODY: usize = 8 * 1024 * 1024;
/// Width the HTML-to-text layout wraps at.
const TEXT_WIDTH: usize = 100;
/// Largest picture embedded in a message that is kept for display.
pub const MAX_INLINE_IMAGE: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedHeader {
    pub from: Option<Address>,
    pub to: Vec<Address>,
    pub subject: String,
    /// Seconds since the Unix epoch, from the Date header.
    pub date: Option<i64>,
    /// The Message-ID, with its angle brackets.
    pub message_id: Option<String>,
    /// From the `Importance` header, else `X-Priority`.
    pub importance: Importance,
    /// From `Chat-Version`, `Chat-Group-ID` and `Chat-Group-Name`: set on a chat message.
    pub chat: Option<ChatTag>,
    /// Whether the message is encrypted with OpenPGP (`multipart/encrypted`).
    pub encrypted: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attachment {
    pub name: String,
    pub mime: String,
    pub size: usize,
    /// The decoded contents, so the attachment can be saved.
    pub bytes: Arc<[u8]>,
}

/// A picture that travels inside the message and that its HTML refers to by content id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineImage {
    /// The Content-ID without its angle brackets.
    pub cid: String,
    pub mime: String,
    pub bytes: Arc<[u8]>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Body {
    /// The text of the message: the plain part, else the HTML reduced to text. Previews,
    /// replies and the reading pane without HTML use it.
    pub text: String,
    /// True when `text` was derived from an HTML part.
    pub from_html: bool,
    /// The HTML part as a layout tree, when the message has one with content.
    pub rich: Option<Document>,
    /// The embedded pictures `rich` refers to.
    pub images: Vec<InlineImage>,
    pub attachments: Vec<Attachment>,
    /// The `Authentication-Results` headers the receiving servers added (SPF, DKIM, DMARC), as
    /// text, for the phishing check.
    pub authentication: Vec<String>,
    /// The ways out a `List-Unsubscribe` header offers: `mailto:` and `https:` targets, without
    /// their angle brackets, in the header's order. Empty for mail that is not a list's.
    pub unsubscribe: Vec<String>,
    /// Where the sender asks a read receipt to go (`Disposition-Notification-To`), as Outlook's
    /// "Leesbevestiging vragen" sets it. Nothing is sent back unless the person asks for it.
    pub receipt_to: Option<Address>,
    /// The S/MIME signature on the message, when it is signed.
    pub signature: Option<merel_smime::Signature>,
    /// The OpenPGP signature on the message, when it is signed that way; the engine checks it,
    /// since only it knows the keys.
    pub pgp_signature: Option<merel_pgp::Verdict>,
    /// Set for a message that came encrypted (S/MIME): whether it opened.
    pub encrypted: Option<Encryption>,
}

/// What became of an encrypted message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Encryption {
    /// Decrypted with the account's certificate; the body is what was inside.
    Opened,
    /// Not opened, and why, in Dutch.
    Locked(String),
}

fn addresses(a: Option<&mail_parser::Address<'_>>) -> Vec<Address> {
    a.map(|a| a.iter().filter_map(|addr| Address::new(addr.name(), addr.address().unwrap_or("")).ok()).collect()).unwrap_or_default()
}

/// Parses a header block (or a whole message) for the fields the message list needs.
pub fn parse_header(bytes: &[u8]) -> ParsedHeader {
    let Some(m) = MessageParser::default().parse(bytes) else { return ParsedHeader::default() };
    ParsedHeader {
        from: addresses(m.from()).into_iter().next(),
        to: addresses(m.to()),
        subject: clean_text(m.subject().unwrap_or("")),
        date: m.date().map(|d| d.to_timestamp()),
        message_id: m.message_id().map(|id| format!("<{}>", clean_text(id.trim_matches(['<', '>'])))),
        importance: importance_of(&m),
        chat: chat_of(&m),
        encrypted: m.content_type().is_some_and(|c| {
            c.ctype().eq_ignore_ascii_case("multipart")
                && c.subtype().is_some_and(|s| s.eq_ignore_ascii_case("encrypted"))
                && c.attribute("protocol").is_some_and(|p| p.eq_ignore_ascii_case("application/pgp-encrypted"))
        }),
    }
}

/// The chat headers Delta Chat writes. The group name is free text that may be RFC 2047
/// encoded; mail-parser leaves headers it does not know raw, so it is decoded as a subject.
fn chat_of(m: &mail_parser::Message<'_>) -> Option<ChatTag> {
    let raw = |name: &str| m.headers_raw().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_string());
    let name = raw("Chat-Group-Name").and_then(|v| {
        let wrapped = format!("Subject: {v}\r\n\r\n");
        MessageParser::default().parse(wrapped.as_bytes()).and_then(|w| w.subject().map(str::to_string))
    });
    let mut tag = ChatTag::from_headers(raw("Chat-Version").as_deref(), raw("Chat-Group-ID").as_deref(), name.as_deref())?;
    tag.reactions = merel_model::Reacted::from_header(raw("Merel-Reactions").as_deref());
    tag.quote = merel_model::Quoted::from_header(raw("Merel-Quote").as_deref());
    tag.delivery = merel_model::Delivery::from_header(raw("Merel-Delivery").as_deref());
    tag.edited = raw("Merel-Edited").is_some_and(|v| v == "yes");
    tag.deleted = raw("Merel-Deleted").is_some_and(|v| v == "yes");
    tag.voice = raw("Merel-Voice").is_some_and(|v| v == "yes");
    tag.call = merel_model::CallSignal::from_headers(raw("Merel-Call").as_deref(), raw("Merel-Call-Sdp").as_deref());
    tag.handshake = merel_model::Handshake::from_headers(
        raw("Secure-Join").as_deref(),
        raw("Secure-Join-Invitenumber").as_deref(),
        raw("Secure-Join-Auth").as_deref(),
        raw("Secure-Join-Fingerprint").as_deref(),
    );
    Some(tag)
}

/// `Importance: High|Low` wins; else `X-Priority` 1–2 is high and 4–5 is low, as mail programs
/// write them.
fn importance_of(m: &mail_parser::Message<'_>) -> Importance {
    let header = |name: &str| m.headers_raw().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_ascii_lowercase());
    match header("Importance").as_deref() {
        Some(v) if v.starts_with("high") => return Importance::High,
        Some(v) if v.starts_with("low") => return Importance::Low,
        Some(v) if v.starts_with("normal") => return Importance::Normal,
        _ => {}
    }
    match header("X-Priority").and_then(|v| v.chars().next()) {
        Some('1' | '2') => Importance::High,
        Some('4' | '5') => Importance::Low,
        _ => Importance::Normal,
    }
}

/// Parses a whole message into what the reading pane shows. Oversized input is truncated.
pub fn parse_body(bytes: &[u8]) -> Body {
    // Checked on the whole message: a signature covers every byte.
    let signature = merel_smime::check_message(bytes);
    let truncated = bytes.len() > MAX_BODY;
    let bytes = bytes.get(..bytes.len().min(MAX_BODY)).unwrap_or(bytes);
    let Some(m) = MessageParser::default().parse(bytes) else {
        return Body { text: String::from_utf8_lossy(bytes).into_owned(), ..Body::default() };
    };
    // `body_text` quietly falls back to a converted HTML part, so look at the parts by kind.
    let plain = m.text_bodies().find(|p| p.is_text() && !p.is_text_html()).and_then(|p| p.text_contents());
    // `html_bodies` falls back to plain parts; only a real HTML part gets a layout tree.
    let html = m.html_bodies().find(|p| p.is_text_html()).and_then(|p| p.text_contents());
    let (mut text, from_html) = match (plain, html) {
        (Some(t), _) => (t.to_string(), false),
        (None, Some(h)) => (html_to_text(h), true),
        (None, None) => (String::new(), false),
    };
    let rich = html.map(merel_html::parse).filter(|d| !d.is_empty());
    if truncated {
        text.push_str(&format!("\n\n[{}]", merel_i18n::tr("mime-truncated")));
    }
    let wanted = rich.as_ref().map(Document::inline_image_ids).unwrap_or_default();
    let cid_of = |p: &mail_parser::MessagePart<'_>| p.content_id().map(|c| c.trim().trim_matches(['<', '>']).to_string());
    let mut images: Vec<InlineImage> = Vec::new();
    for p in &m.parts {
        let Some(cid) = cid_of(p).filter(|c| wanted.contains(c)) else { continue };
        let mime = mime_of(p);
        if mime.starts_with("image/") && p.contents().len() <= MAX_INLINE_IMAGE && !images.iter().any(|i| i.cid == cid) {
            images.push(InlineImage { cid, mime, bytes: p.contents().into() });
        }
    }
    let authentication = m
        .headers()
        .iter()
        .filter(|h| h.name.as_str().eq_ignore_ascii_case("Authentication-Results"))
        .filter_map(|h| h.value.as_text().map(clean_text).or_else(|| h.value.as_text_list().map(|l| clean_text(&l.join(" ")))))
        .collect();
    // The parser reads this header as addresses, which loses the web targets; the raw text keeps them.
    let unsubscribe = m
        .headers_raw()
        .filter(|(name, _)| name.eq_ignore_ascii_case("List-Unsubscribe"))
        .flat_map(|(_, v)| v.replace(['\r', '\n'], "").split(',').map(|t| t.trim().trim_matches(['<', '>']).trim().to_string()).collect::<Vec<_>>())
        .filter(|t| {
            let lower = t.to_ascii_lowercase();
            (lower.starts_with("mailto:") || lower.starts_with("https://")) && t.len() <= 2048 && !t.chars().any(char::is_control)
        })
        .collect();
    // The parser does not know this header, so its raw text is read as an address list.
    let receipt_to = m
        .headers_raw()
        .find(|(name, _)| name.eq_ignore_ascii_case("Disposition-Notification-To"))
        .and_then(|(_, v)| parse_address_list(&v.replace(['\r', '\n'], " ")).ok())
        .and_then(|list| list.into_iter().next());
    let pgp_signed = m.content_type().is_some_and(|c| {
        c.ctype().eq_ignore_ascii_case("multipart")
            && c.subtype().is_some_and(|s| s.eq_ignore_ascii_case("signed"))
            && c.attribute("protocol").is_some_and(|p| p.eq_ignore_ascii_case("application/pgp-signature"))
    });
    // A picture shown in the body is not listed as an attachment as well.
    let attachments = m
        .attachments()
        .filter(|p| !cid_of(p).is_some_and(|c| images.iter().any(|i| i.cid == c)))
        // The signature itself is shown as the strip over the message, not as a file.
        .filter(|p| signature.is_none() || !matches!(mime_of(p).as_str(), "application/pkcs7-signature" | "application/x-pkcs7-signature"))
        .filter(|p| !pgp_signed || mime_of(p) != "application/pgp-signature")
        .map(|p| Attachment {
            name: clean_text(p.attachment_name().unwrap_or("bijlage")),
            mime: mime_of(p),
            size: p.contents().len(),
            bytes: p.contents().into(),
        })
        .collect();
    Body { text, from_html, rich, images, attachments, authentication, unsubscribe, receipt_to, signature, pgp_signature: None, encrypted: None }
}

fn mime_of(p: &mail_parser::MessagePart<'_>) -> String {
    p.content_type().map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or("")).to_ascii_lowercase()).unwrap_or_default()
}

/// Plain text from HTML: tags, scripts, styles and remote references are all dropped.
pub fn html_to_text(html: &str) -> String {
    html2text::from_read(html.as_bytes(), TEXT_WIDTH).unwrap_or_else(|_| strip_tags(html))
}

/// Last resort when the HTML parser gives up: remove everything between `<` and `>`.
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// The first `n` characters of `text` on one line, for the message list.
pub fn preview(text: &str, n: usize) -> String {
    let one_line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    clean_text(&one_line).chars().take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authentication_results_headers_come_along_with_the_body() {
        let raw = b"From: a@example.org\r\nAuthentication-Results: mx.example;\r\n spf=fail smtp.mailfrom=example.org;\r\n dmarc=fail header.from=example.org\r\nAuthentication-Results: other.example; dkim=pass\r\nSubject: x\r\nContent-Type: text/plain\r\n\r\nHallo\r\n";
        let b = parse_body(raw);
        assert_eq!(b.authentication.len(), 2, "{:?}", b.authentication);
        assert!(b.authentication[0].contains("dmarc=fail") && b.authentication[1].contains("dkim=pass"));
        assert!(parse_body(b"From: a@example.org\r\n\r\nHallo").authentication.is_empty());
    }

    const MSG: &[u8] = b"From: Sanne de Vries <sanne@example.org>\r\nTo: jij@example.org\r\nSubject: =?UTF-8?Q?Planning_voor_donderdag?=\r\nDate: Wed, 14 Oct 2026 16:16:00 +0200\r\nMessage-ID: <m1@example.org>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nZullen we om tien uur beginnen?\r\n";

    #[test]
    fn the_list_unsubscribe_header_gives_its_mailto_and_https_targets() {
        let raw = b"From: news@example.org\r\nList-Unsubscribe: <mailto:leave@example.org?subject=unsubscribe>,\r\n <https://example.org/u?id=7>, <http://plain.example/u>\r\nSubject: x\r\nContent-Type: text/plain\r\n\r\nHallo\r\n";
        let b = parse_body(raw);
        assert_eq!(b.unsubscribe, ["mailto:leave@example.org?subject=unsubscribe", "https://example.org/u?id=7"], "plain http is left out");
        assert!(parse_body(b"From: a@example.org\r\n\r\nHallo").unsubscribe.is_empty());
    }

    #[test]
    fn a_disposition_notification_to_header_names_where_a_receipt_would_go() {
        let raw = b"From: Sanne <sanne@example.org>\r\nDisposition-Notification-To: Sanne de Vries\r\n <receipts@example.org>\r\nSubject: x\r\nContent-Type: text/plain\r\n\r\nHallo\r\n";
        let to = parse_body(raw).receipt_to.expect("the address");
        assert_eq!((to.email(), to.display()), ("receipts@example.org", "Sanne de Vries"));
        assert!(parse_body(b"From: a@example.org\r\nDisposition-Notification-To: nonsense\r\n\r\nHallo").receipt_to.is_none());
        assert!(parse_body(MSG).receipt_to.is_none());
    }

    #[test]
    fn importance_comes_from_the_importance_or_x_priority_header() {
        let header = |extra: &str| parse_header(format!("From: a@example.org\r\n{extra}\r\nSubject: x\r\n\r\nHallo").as_bytes()).importance;
        assert_eq!(header("Importance: high"), Importance::High);
        assert_eq!(header("Importance: Low"), Importance::Low);
        assert_eq!(header("X-Priority: 1 (Highest)"), Importance::High);
        assert_eq!(header("X-Priority: 5 (Lowest)"), Importance::Low);
        assert_eq!(header("Importance: normal\r\nX-Priority: 1"), Importance::Normal, "the Importance header wins");
        assert_eq!(parse_header(MSG).importance, Importance::Normal);
    }

    #[test]
    fn parses_headers_and_decodes_encoded_words() {
        let h = parse_header(MSG);
        assert_eq!(h.from.as_ref().unwrap().display(), "Sanne de Vries");
        assert_eq!(h.to[0].email(), "jij@example.org");
        assert_eq!(h.subject, "Planning voor donderdag");
        assert_eq!(h.date, Some(1_791_987_360));
        assert_eq!(h.message_id.as_deref(), Some("<m1@example.org>"));
    }

    #[test]
    fn plain_body_is_kept_and_html_is_flattened() {
        assert_eq!(parse_body(MSG).text.trim(), "Zullen we om tien uur beginnen?");
        let html = b"Content-Type: text/html\r\n\r\n<html><script>alert(1)</script><style>p{}</style><p>Hallo <b>daar</b></p><img src=\"https://t.example/px.gif\"></html>";
        let b = parse_body(html);
        assert!(b.from_html);
        assert!(b.text.contains("Hallo"));
        assert!(!b.text.contains("alert"));
        assert!(!b.text.contains("t.example"));
        let rich = b.rich.expect("layout tree");
        assert_eq!((rich.blocks.len(), rich.remote_images), (2, 1), "a paragraph and the picture");
        assert!(parse_body(MSG).rich.is_none(), "a plain message has no layout tree");
    }

    #[test]
    fn embedded_pictures_leave_the_attachment_list() {
        let msg = b"Content-Type: multipart/related; boundary=b\r\n\r\n--b\r\nContent-Type: text/html\r\n\r\n<p>Logo: <img src=\"cid:logo@x\" alt=\"Logo\"></p>\r\n--b\r\nContent-Type: image/png; name=\"logo.png\"\r\nContent-ID: <logo@x>\r\nContent-Disposition: inline; filename=\"logo.png\"\r\n\r\nPNGDATA\r\n--b\r\nContent-Type: image/png; name=\"other.png\"\r\nContent-ID: <other@x>\r\nContent-Disposition: attachment; filename=\"other.png\"\r\n\r\nPNG2\r\n--b--\r\n";
        let b = parse_body(msg);
        assert_eq!(b.images.len(), 1);
        assert_eq!((b.images[0].cid.as_str(), b.images[0].mime.as_str(), &b.images[0].bytes[..]), ("logo@x", "image/png", &b"PNGDATA"[..]));
        assert_eq!(
            b.attachments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            vec!["other.png"],
            "the unreferenced picture stays an attachment"
        );
    }

    #[test]
    fn attachments_are_listed() {
        let msg = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nhi\r\n--b\r\nContent-Type: application/pdf; name=\"offerte.pdf\"\r\nContent-Disposition: attachment; filename=\"offerte.pdf\"\r\n\r\n%PDF\r\n--b--\r\n";
        let b = parse_body(msg);
        assert_eq!(b.attachments.len(), 1);
        assert_eq!(b.attachments[0].name, "offerte.pdf");
        assert_eq!(b.attachments[0].mime, "application/pdf");
    }

    #[test]
    fn garbage_never_panics() {
        for junk in [&b""[..], b"\xff\xfe\x00", b"<<<<", b"From: \r\n\r\n", &[b'<'; 10_000][..]] {
            let _ = parse_header(junk);
            let _ = parse_body(junk);
        }
        assert_eq!(preview("a\n\n  b\tc", 3), "a b");
    }

    #[test]
    fn a_calendar_part_is_listed_as_an_attachment() {
        let msg = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nKom je ook?\r\n--b\r\nContent-Type: text/calendar; method=REQUEST; charset=utf-8\r\nContent-Disposition: attachment; filename=\"invite.ics\"\r\n\r\nBEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nSUMMARY:Kickoff\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n--b--\r\n";
        let b = parse_body(msg);
        assert_eq!(b.text.trim(), "Kom je ook?");
        assert_eq!(b.attachments.len(), 1);
        assert_eq!((b.attachments[0].name.as_str(), b.attachments[0].mime.as_str()), ("invite.ics", "text/calendar"));
        assert!(std::str::from_utf8(&b.attachments[0].bytes).unwrap().contains("SUMMARY:Kickoff"));
    }

    #[test]
    fn a_signed_message_carries_its_signature_instead_of_an_attachment() {
        let b = parse_body(include_bytes!("../../smime/testdata/signed-openssl.eml"));
        assert!(b.text.starts_with("Hallo Sanne,"), "{}", b.text);
        assert!(matches!(b.signature, Some(merel_smime::Signature::Valid { .. })));
        assert!(b.attachments.is_empty(), "smime.p7s is not listed");
        assert_eq!(parse_body(include_bytes!("../../smime/testdata/signed-tampered.eml")).signature, Some(merel_smime::Signature::Broken));
        assert_eq!(parse_body(b"Subject: x\r\n\r\nHoi").signature, None);
        // An OpenPGP signature is not listed either.
        let pgp = parse_body(include_bytes!("../../pgp/testdata/signed-gpg.eml"));
        assert!(pgp.text.starts_with("Hallo Alain"), "{}", pgp.text);
        assert!(pgp.attachments.is_empty(), "signature.asc is not listed");
    }
}
