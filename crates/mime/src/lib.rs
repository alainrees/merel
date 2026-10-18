//! Message parsing: RFC 5322 headers and MIME bodies, through `mail-parser`.
//!
//! Everything that comes out of here is bounded and cleaned: names and subjects go through
//! [`mailcraft_model::clean_text`], addresses through [`Address::new`], and HTML is reduced
//! to plain text (no scripts, no styles, no remote loads can survive that).

use mail_parser::{MessageParser, MimeHeaders};
use mailcraft_model::{Address, clean_text};

/// Longest message body (bytes) that is parsed; the rest is dropped with a note.
pub const MAX_BODY: usize = 8 * 1024 * 1024;
/// Width the HTML-to-text layout wraps at.
const TEXT_WIDTH: usize = 100;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedHeader {
    pub from: Option<Address>,
    pub to: Vec<Address>,
    pub subject: String,
    /// Seconds since the Unix epoch, from the Date header.
    pub date: Option<i64>,
    /// The Message-ID, with its angle brackets.
    pub message_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attachment {
    pub name: String,
    pub mime: String,
    pub size: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Body {
    /// The text shown in the reading pane.
    pub text: String,
    /// True when `text` was derived from an HTML part.
    pub from_html: bool,
    pub attachments: Vec<Attachment>,
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
    }
}

/// Parses a whole message into what the reading pane shows. Oversized input is truncated.
pub fn parse_body(bytes: &[u8]) -> Body {
    let truncated = bytes.len() > MAX_BODY;
    let bytes = bytes.get(..bytes.len().min(MAX_BODY)).unwrap_or(bytes);
    let Some(m) = MessageParser::default().parse(bytes) else {
        return Body { text: String::from_utf8_lossy(bytes).into_owned(), ..Body::default() };
    };
    // `body_text` quietly falls back to a converted HTML part, so look at the parts by kind.
    let plain = m.text_bodies().find(|p| p.is_text() && !p.is_text_html()).and_then(|p| p.text_contents());
    let (mut text, from_html) = match plain {
        Some(t) => (t.to_string(), false),
        None => match m.html_bodies().find_map(|p| p.text_contents()) {
            Some(h) => (html_to_text(h), true),
            None => (String::new(), false),
        },
    };
    if truncated {
        text.push_str("\n\n[Bericht afgekapt: te groot om volledig te tonen.]");
    }
    let attachments = m
        .attachments()
        .map(|p| Attachment {
            name: clean_text(p.attachment_name().unwrap_or("bijlage")),
            mime: p.content_type().map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or(""))).unwrap_or_default(),
            size: p.contents().len(),
        })
        .collect();
    Body { text, from_html, attachments }
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

    const MSG: &[u8] = b"From: Sanne de Vries <sanne@example.org>\r\nTo: jij@example.org\r\nSubject: =?UTF-8?Q?Planning_voor_donderdag?=\r\nDate: Wed, 14 Oct 2026 16:16:00 +0200\r\nMessage-ID: <m1@example.org>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nZullen we om tien uur beginnen?\r\n";

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
}
