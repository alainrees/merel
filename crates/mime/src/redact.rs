//! Taking the reader out of a message before it goes to someone else: the addresses and the
//! name of the person it was sent to, in the headers and in the text, wherever they can be read.
//! What "Melden als phishing/spam" does before a message goes to the Merel team.
//!
//! Text parts sent as base64 or quoted-printable are decoded, cleaned and encoded again the
//! same way, so the message keeps its structure; attachments and pictures are left alone. A
//! DKIM signature no longer holds afterwards, but the `Authentication-Results` the receiving
//! server added stay, and they say what the check found.

use base64::Engine as _;
use mail_parser::{Encoding, MessageParser, PartType};

/// What stands in for a whole address.
pub const HIDDEN_ADDRESS: &str = "ontvanger@example.invalid";
/// What stands in for a name, or for the part of an address before the `@`.
pub const HIDDEN_NAME: &str = "Ontvanger";

/// Who is taken out: the reader's addresses and names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Redaction {
    pub addresses: Vec<String>,
    pub names: Vec<String>,
}

impl Redaction {
    /// The strings to find and what replaces each, longest first, so that an address goes
    /// whole before its parts are looked at.
    fn pairs(&self) -> Vec<(String, &'static str, bool)> {
        let mut out: Vec<(String, &'static str, bool)> = Vec::new();
        let mut add = |find: &str, with: &'static str, word: bool| {
            let find = find.trim();
            if find.chars().count() >= 3 && !out.iter().any(|(f, _, _)| f.eq_ignore_ascii_case(find)) {
                out.push((find.to_string(), with, word));
            }
        };
        for a in &self.addresses {
            add(a, HIDDEN_ADDRESS, false);
            add(&a.replace('@', "%40"), HIDDEN_ADDRESS, false);
            add(&a.replace('@', "=40"), HIDDEN_ADDRESS, false);
        }
        for n in &self.names {
            add(n, HIDDEN_NAME, true);
        }
        for a in &self.addresses {
            if let Some((local, _)) = a.split_once('@') {
                add(local, HIDDEN_NAME, true);
            }
        }
        for n in &self.names {
            for w in n.split_whitespace() {
                add(w, HIDDEN_NAME, true);
            }
        }
        out.sort_by_key(|(f, _, _)| std::cmp::Reverse(f.len()));
        out
    }

    pub fn is_empty(&self) -> bool {
        self.pairs().is_empty()
    }
}

/// The message with the reader taken out; the bytes as they were when there is nothing to take.
pub fn redact(raw: &[u8], who: &Redaction) -> Vec<u8> {
    let pairs = who.pairs();
    if pairs.is_empty() {
        return raw.to_vec();
    }
    // The encoded parts: text is decoded, cleaned and encoded again; anything else (files,
    // pictures) is copied as it is, so a name can never match inside base64 by chance.
    let mut spans: Vec<(usize, usize, Encoding, bool)> = MessageParser::default()
        .parse(raw)
        .map(|message| {
            message
                .parts
                .iter()
                .filter(|p| matches!(p.encoding, Encoding::Base64 | Encoding::QuotedPrintable))
                .map(|p| {
                    (p.raw_body_offset() as usize, p.raw_end_offset() as usize, p.encoding, matches!(p.body, PartType::Text(_) | PartType::Html(_)))
                })
                .filter(|(start, end, _, _)| start < end && *end <= raw.len())
                .collect()
        })
        .unwrap_or_default();
    spans.sort_by_key(|(start, _, _, _)| *start);
    let mut out = Vec::with_capacity(raw.len());
    let mut at = 0;
    for (start, end, encoding, text) in spans {
        if start < at {
            continue;
        }
        // Everything that is readable as it stands: the headers and plain parts.
        out.extend(replace_all(&raw[at..start], &pairs));
        at = end;
        let slice = &raw[start..end];
        let body_len = slice.len() - trailing_newlines(slice);
        let (encoded, tail) = slice.split_at(body_len);
        let cleaned = if !text {
            None
        } else if encoding == Encoding::Base64 {
            let compact: Vec<u8> = encoded.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect();
            base64::engine::general_purpose::STANDARD
                .decode(&compact)
                .ok()
                .map(|decoded| (replace_all(&decoded, &pairs), decoded))
                .filter(|(clean, decoded)| clean != decoded)
                .map(|(clean, _)| base64_lines(&clean))
        } else {
            let decoded = qp_decode(encoded);
            Some(replace_all(&decoded, &pairs)).filter(|clean| *clean != decoded).map(|clean| qp_encode(&clean))
        };
        out.extend(cleaned.as_deref().unwrap_or(encoded));
        out.extend_from_slice(tail);
    }
    out.extend(replace_all(&raw[at..], &pairs));
    out
}

fn trailing_newlines(b: &[u8]) -> usize {
    b.iter().rev().take_while(|c| matches!(c, b'\r' | b'\n')).count()
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b >= 0x80
}

/// Every occurrence of every pair, ASCII letters matched without regard to case; a name only
/// where it stands as a word of its own.
fn replace_all(text: &[u8], pairs: &[(String, &'static str, bool)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    'outer: while i < text.len() {
        for (find, with, word) in pairs {
            let f = find.as_bytes();
            if text.len() - i >= f.len()
                && text[i..i + f.len()].eq_ignore_ascii_case(f)
                && (!word || ((i == 0 || !is_word(text[i - 1])) && text.get(i + f.len()).is_none_or(|b| !is_word(*b))))
            {
                out.extend_from_slice(with.as_bytes());
                i += f.len();
                continue 'outer;
            }
        }
        out.push(text[i]);
        i += 1;
    }
    out
}

fn base64_lines(bytes: &[u8]) -> Vec<u8> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let lines: Vec<&[u8]> = encoded.as_bytes().chunks(76).collect();
    lines.join(&b"\r\n"[..])
}

fn qp_decode(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'=' {
            if b[i + 1..].starts_with(b"\r\n") {
                i += 3;
                continue;
            }
            if b[i + 1..].starts_with(b"\n") {
                i += 2;
                continue;
            }
            let hex = b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(v) = hex {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// Quoted-printable with CRLF line ends, lines of at most 76 characters.
fn qp_encode(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len() + b.len() / 8);
    let text = String::from_utf8_lossy(b).replace("\r\n", "\n");
    let raw = if std::str::from_utf8(b).is_ok() { text.into_bytes() } else { b.iter().copied().filter(|c| *c != b'\r').collect() };
    for (n, line) in raw.split(|c| *c == b'\n').enumerate() {
        if n > 0 {
            out.extend_from_slice(b"\r\n");
        }
        let mut width = 0;
        for (k, &c) in line.iter().enumerate() {
            let last = k + 1 == line.len();
            let plain = (c == b' ' || c == b'\t') && !last || (33..=126).contains(&c) && c != b'=';
            let piece = if plain { vec![c] } else { format!("={c:02X}").into_bytes() };
            if width + piece.len() > 75 {
                out.extend_from_slice(b"=\r\n");
                width = 0;
            }
            width += piece.len();
            out.extend_from_slice(&piece);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me() -> Redaction {
        Redaction { addresses: vec!["alain.rees@example.nl".into()], names: vec!["Alain Rees".into()] }
    }

    #[test]
    fn the_reader_leaves_the_headers_and_the_plain_text() {
        let raw = b"From: Bank <no-reply@bank.example>\r\nTo: \"Alain Rees\" <Alain.Rees@Example.nl>\r\nSubject: Beste alain, actie nodig\r\n\r\nBeste Alain,\r\nKlik hier: https://x.example/?u=alain.rees%40example.nl\r\nGroet, Reesink en Alainstraat\r\n";
        let out = String::from_utf8(redact(raw, &me())).unwrap();
        assert!(out.contains("To: \"Ontvanger\" <ontvanger@example.invalid>"), "{out}");
        assert!(out.contains("Subject: Beste Ontvanger, actie nodig"), "{out}");
        assert!(out.contains("Beste Ontvanger,\r\n"), "{out}");
        assert!(out.contains("?u=ontvanger@example.invalid"), "{out}");
        assert!(out.contains("From: Bank <no-reply@bank.example>"), "the sender stays: {out}");
        assert!(out.contains("Reesink en Alainstraat"), "only whole words: {out}");
        assert!(!out.to_ascii_lowercase().contains("alain.rees"), "{out}");
    }

    #[test]
    fn encoded_text_parts_are_cleaned_and_keep_their_encoding() {
        let html = base64::engine::general_purpose::STANDARD.encode("<p>Hallo Alain Rees, uw rekening alain.rees@example.nl is geblokkeerd</p>");
        let raw = format!(
            "From: x@bad.example\r\nTo: alain.rees@example.nl\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nHallo Alain, mail naar alain.rees=40example=\r\n.nl caf=C3=A9\r\n--b\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{html}\r\n--b--\r\n"
        );
        let out = redact(raw.as_bytes(), &me());
        let parsed = MessageParser::default().parse(&out).unwrap();
        let text = parsed.body_text(0).unwrap().to_string();
        let html = parsed.body_html(0).unwrap().to_string();
        assert!(text.contains("Hallo Ontvanger, mail naar ontvanger@example.invalid café"), "{text}");
        assert!(html.contains("Hallo Ontvanger, uw rekening ontvanger@example.invalid is geblokkeerd"), "{html}");
        let s = String::from_utf8_lossy(&out);
        assert!(s.contains("--b--") && s.contains("Content-Transfer-Encoding: base64"), "{s}");
        assert!(!s.to_ascii_lowercase().contains("alain"), "{s}");
    }

    #[test]
    fn nothing_to_take_leaves_the_bytes_as_they_were() {
        let raw = b"Subject: hoi\r\n\r\nGeen namen hier\r\n";
        assert_eq!(redact(raw, &Redaction::default()), raw.to_vec());
        assert_eq!(redact(raw, &me()), raw.to_vec());
        assert!(Redaction { addresses: vec![], names: vec!["Al".into()] }.is_empty(), "too short to look for");
    }

    #[test]
    fn quoted_printable_round_trips() {
        let s = "Een regel met = teken en é\r\n".to_string() + &"x".repeat(200);
        assert_eq!(qp_decode(&qp_encode(s.as_bytes())), s.into_bytes());
    }
}
