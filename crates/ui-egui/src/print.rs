//! Printing a message: Mailcraft draws mail itself and has no printer of its own, so "Afdrukken"
//! writes the open message as a page the system browser can print and opens it there. The page
//! is built from the message's layout tree, never from the mail's own HTML: every text is
//! escaped, links keep only the addresses the tree allowed, pictures that travel inside the
//! message are embedded as data, pictures on the internet are left out, and a Content Security
//! Policy on the page forbids scripts and anything from the network.

use std::fmt::Write as _;
use std::path::PathBuf;

use base64::Engine;
use chrono::{Local, TimeZone};
use mailcraft_html::{Block, Document, Image, ImageSource, Inline, Style};
use mailcraft_mime::{Body, InlineImage};
use mailcraft_model::{Address, MessageHeader};

use crate::MailApp;

/// The one file the print page is written to, in the system's temporary folder; each print
/// overwrites it, so no trail of messages is left behind.
pub fn page_path() -> PathBuf {
    std::env::temp_dir().join("mailcraft-afdruk.html")
}

/// Escapes text for HTML.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c if c.is_control() && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

fn address(a: &Address) -> String {
    if a.name.as_deref().is_some_and(|n| !n.trim().is_empty()) { format!("{} &lt;{}&gt;", esc(a.display()), esc(a.email())) } else { esc(a.email()) }
}

fn date(ts: i64) -> String {
    Local.timestamp_opt(ts, 0).single().map(|d| d.format("%d-%m-%Y %H:%M").to_string()).unwrap_or_default()
}

/// A `data:` URL for an embedded picture, when its type is an image.
fn data_url(mime: &str, bytes: &[u8]) -> Option<String> {
    let mime = mime.trim().to_ascii_lowercase();
    if !mime.starts_with("image/") || mime.chars().any(|c| !(c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '+' | '.'))) {
        return None;
    }
    Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

fn image(out: &mut String, img: &Image, images: &[InlineImage]) {
    let src = match &img.source {
        ImageSource::Inline(cid) => images.iter().find(|i| &i.cid == cid).and_then(|i| data_url(&i.mime, &i.bytes)),
        ImageSource::Data { mime, bytes } => data_url(mime, bytes),
        ImageSource::Remote(_) => None,
    };
    match src {
        Some(src) => {
            let _ = write!(out, "<img src=\"{src}\" alt=\"{}\">", esc(&img.alt));
        }
        None if img.alt.trim().is_empty() => out.push_str("<span class=\"missing\">[afbeelding niet afgedrukt]</span>"),
        None => {
            let _ = write!(out, "<span class=\"missing\">[afbeelding: {}]</span>", esc(&img.alt));
        }
    }
}

fn styled(text: &str, style: Style) -> String {
    let mut s = esc(text);
    if style.code {
        s = format!("<code>{s}</code>");
    }
    if style.bold {
        s = format!("<b>{s}</b>");
    }
    if style.italic {
        s = format!("<i>{s}</i>");
    }
    if style.underline {
        s = format!("<u>{s}</u>");
    }
    if style.strike {
        s = format!("<s>{s}</s>");
    }
    s
}

fn inlines(out: &mut String, list: &[Inline], images: &[InlineImage]) {
    for i in list {
        match i {
            Inline::Text { text, style } => out.push_str(&styled(text, *style)),
            Inline::Link { href, text, style } => {
                // The tree only holds http, https and mailto links; the check stays, cheaply.
                let safe = href.starts_with("http://") || href.starts_with("https://") || href.starts_with("mailto:");
                if safe {
                    let _ = write!(out, "<a href=\"{}\">{}</a>", esc(href), styled(text, *style));
                } else {
                    out.push_str(&styled(text, *style));
                }
            }
            Inline::Image(img) => image(out, img, images),
            Inline::Break => out.push_str("<br>"),
        }
    }
}

fn blocks(out: &mut String, list: &[Block], images: &[InlineImage]) {
    for b in list {
        match b {
            Block::Paragraph(i) => {
                out.push_str("<p>");
                inlines(out, i, images);
                out.push_str("</p>\n");
            }
            Block::Heading { level, inlines: i } => {
                let level = (*level).clamp(1, 6);
                let _ = write!(out, "<h{level}>");
                inlines(out, i, images);
                let _ = writeln!(out, "</h{level}>");
            }
            Block::Quote(inner) => {
                out.push_str("<blockquote>\n");
                blocks(out, inner, images);
                out.push_str("</blockquote>\n");
            }
            Block::List { ordered, items } => {
                let tag = if *ordered { "ol" } else { "ul" };
                let _ = writeln!(out, "<{tag}>");
                for item in items {
                    out.push_str("<li>");
                    blocks(out, item, images);
                    out.push_str("</li>\n");
                }
                let _ = writeln!(out, "</{tag}>");
            }
            Block::Table(rows) => {
                out.push_str("<table>\n");
                for row in rows {
                    out.push_str("<tr>");
                    for cell in row {
                        let tag = if cell.header { "th" } else { "td" };
                        let _ = write!(out, "<{tag}>");
                        blocks(out, &cell.blocks, images);
                        let _ = write!(out, "</{tag}>");
                    }
                    out.push_str("</tr>\n");
                }
                out.push_str("</table>\n");
            }
            Block::Code(text) => {
                let _ = writeln!(out, "<pre>{}</pre>", esc(text));
            }
            Block::Rule => out.push_str("<hr>\n"),
        }
    }
}

const STYLE: &str = "body{font-family:-apple-system,Segoe UI,Helvetica,Arial,sans-serif;font-size:11pt;color:#111;max-width:52em;margin:2em auto;padding:0 1em}\
.hint{background:#eef;border:1px solid #ccd;padding:.5em .8em;border-radius:4px;font-size:10pt}\
table.head{border-collapse:collapse;margin:1em 0}table.head th{text-align:left;padding:.15em 1em .15em 0;color:#555;font-weight:600;white-space:nowrap;vertical-align:top}\
table.head td{padding:.15em 0}h1.subject{font-size:16pt;margin:.2em 0 .6em}hr.sep{border:0;border-top:1px solid #999;margin:1em 0}\
.body table{border-collapse:collapse}.body td,.body th{border:1px solid #bbb;padding:.2em .5em;vertical-align:top}blockquote{border-left:3px solid #bbb;margin:.5em 0;padding-left:.8em;color:#444}\
pre{white-space:pre-wrap;font-family:Menlo,Consolas,monospace;font-size:10pt}img{max-width:100%}.missing{color:#888;font-style:italic}a{color:#1a4d8f}\
@media print{.hint{display:none}body{margin:0;max-width:none}}";

/// The whole page for a message: a header block with sender, recipients, date, subject and
/// attachments, a separator, then the body (the layout tree when the message had HTML, else the
/// plain text).
pub fn page(h: &MessageHeader, body: Option<&Body>) -> String {
    let mut out = String::new();
    out.push_str("<!doctype html>\n<html lang=\"nl\"><head><meta charset=\"utf-8\">\n");
    out.push_str("<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src data:; style-src 'unsafe-inline'\">\n");
    let _ = writeln!(out, "<title>{}</title>", esc(&h.subject));
    let _ = writeln!(out, "<style>{STYLE}</style></head><body>");
    out.push_str("<p class=\"hint\">Afdrukpagina van Mailcraft. Druk af met Ctrl+P (Cmd+P op een Mac); deze regel komt niet mee.</p>\n");
    let _ = writeln!(out, "<h1 class=\"subject\">{}</h1>", esc(if h.subject.trim().is_empty() { "(Zonder onderwerp)" } else { &h.subject }));
    out.push_str("<table class=\"head\">\n");
    let _ = writeln!(out, "<tr><th>Van</th><td>{}</td></tr>", address(&h.from));
    if !h.to.is_empty() {
        let to: Vec<String> = h.to.iter().map(address).collect();
        let _ = writeln!(out, "<tr><th>Aan</th><td>{}</td></tr>", to.join(", "));
    }
    let _ = writeln!(out, "<tr><th>Datum</th><td>{}</td></tr>", date(h.date));
    if let Some(b) = body
        && !b.attachments.is_empty()
    {
        let names: Vec<String> = b.attachments.iter().map(|a| esc(&a.name)).collect();
        let _ = writeln!(out, "<tr><th>Bijlagen</th><td>{}</td></tr>", names.join(", "));
    }
    out.push_str("</table>\n<hr class=\"sep\">\n<div class=\"body\">\n");
    match body {
        Some(Body { rich: Some(doc), images, .. }) if !doc.is_empty() => document(&mut out, doc, images),
        Some(b) => {
            let _ = writeln!(out, "<pre>{}</pre>", esc(&b.text));
        }
        None => out.push_str("<p class=\"missing\">[De inhoud van dit bericht is nog niet opgehaald.]</p>\n"),
    }
    out.push_str("</div>\n</body></html>\n");
    out
}

fn document(out: &mut String, doc: &Document, images: &[InlineImage]) {
    blocks(out, &doc.blocks, images);
    if doc.remote_images > 0 {
        let _ = writeln!(out, "<p class=\"missing\">[{} afbeelding(en) van internet niet afgedrukt]</p>", doc.remote_images);
    }
    if doc.truncated {
        out.push_str("<p class=\"missing\">[Het bericht was te lang; het einde is weggelaten.]</p>\n");
    }
}

/// Writes the page for the open message and hands it to the system browser.
fn write_and_open(path: &PathBuf, html: &str) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(path)?;
    std::io::Write::write_all(&mut f, html.as_bytes())?;
    drop(f);
    open::that_detached(path)
}

impl MailApp {
    /// Whether the selected message can be printed: its contents have arrived.
    pub fn can_print(&self) -> bool {
        self.engine.session.selected().is_some() && self.engine.session.selected_body().is_some()
    }

    /// "Afdrukken": the open message as a page in the browser, to print from there.
    pub fn print_selected(&mut self) {
        let Some(h) = self.engine.session.selected().cloned() else { return };
        let Some(body) = self.engine.session.selected_body().cloned() else {
            self.notice = Some("De inhoud van dit bericht is nog niet opgehaald; probeer het zo opnieuw.".into());
            return;
        };
        let html = page(&h, Some(&body));
        let path = page_path();
        self.notice = Some(match write_and_open(&path, &html) {
            Ok(()) => "Afdrukpagina geopend in de browser; druk daar af met Ctrl+P (Cmd+P op een Mac).".to_string(),
            Err(e) => {
                log::warn!("print page {}: {e}", path.display());
                format!("Afdrukken is niet gelukt: {e}")
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mailcraft_html::Cell;
    use mailcraft_model::Flags;

    use super::*;

    fn header() -> MessageHeader {
        MessageHeader {
            id: "m".into(),
            folder_id: "inbox".into(),
            remote_id: "1".into(),
            from: Address::new(Some("Sanne <b>"), "sanne@example.org").unwrap(),
            to: vec![Address::new(None, "alain@example.org").unwrap()],
            subject: "Offerte <script>alert(1)</script>".into(),
            date: 1_792_000_000,
            preview: String::new(),
            flags: Flags::default(),
            has_attachments: true,
            message_id: None,
            categories: Vec::new(),
        }
    }

    #[test]
    fn the_page_escapes_everything_and_keeps_only_safe_links_and_embedded_pictures() {
        let doc = Document {
            blocks: vec![
                Block::Heading { level: 2, inlines: vec![Inline::Text { text: "Hallo & dag".into(), style: Style::default() }] },
                Block::Paragraph(vec![
                    Inline::Link {
                        href: "https://example.org/a?b=1&c=2".into(),
                        text: "site".into(),
                        style: Style { bold: true, ..Style::default() },
                    },
                    Inline::Break,
                    Inline::Image(Image { source: ImageSource::Inline("logo".into()), alt: "Logo".into() }),
                    Inline::Image(Image { source: ImageSource::Remote("https://x/y.png".into()), alt: "Ver weg".into() }),
                ]),
                Block::Table(vec![vec![Cell {
                    header: true,
                    blocks: vec![Block::Paragraph(vec![Inline::Text { text: "Kop".into(), style: Style::default() }])],
                }]]),
                Block::Code("a < b".into()),
            ],
            remote_images: 1,
            truncated: false,
        };
        let body = Body {
            text: "tekst".into(),
            from_html: true,
            rich: Some(doc),
            images: vec![InlineImage { cid: "logo".into(), mime: "image/png".into(), bytes: Arc::from(&b"\x89PNG"[..]) }],
            attachments: vec![mailcraft_mime::Attachment {
                name: "offerte<1>.pdf".into(),
                mime: "application/pdf".into(),
                size: 3,
                bytes: Arc::from(&b"abc"[..]),
            }],
        };
        let html = page(&header(), Some(&body));
        assert!(html.contains("<title>Offerte &lt;script&gt;alert(1)&lt;/script&gt;</title>"), "{html}");
        assert!(!html.contains("<script"), "nothing from the mail is markup");
        assert!(html.contains("Sanne &lt;b&gt; &lt;sanne@example.org&gt;"));
        assert!(html.contains("<th>Bijlagen</th><td>offerte&lt;1&gt;.pdf</td>"));
        assert!(html.contains("<h2>Hallo &amp; dag</h2>"));
        assert!(html.contains("<a href=\"https://example.org/a?b=1&amp;c=2\"><b>site</b></a><br>"));
        assert!(html.contains("<img src=\"data:image/png;base64,iVBORw==\" alt=\"Logo\">"), "{html}");
        assert!(html.contains("[afbeelding: Ver weg]") && !html.contains("https://x/y.png"), "a remote picture is named, never fetched");
        assert!(html.contains("<th><p>Kop</p>\n</th>") && html.contains("<pre>a &lt; b</pre>"));
        assert!(html.contains("[1 afbeelding(en) van internet niet afgedrukt]"));
        assert!(html.contains("Content-Security-Policy"));

        let plain = Body { rich: None, text: "Gewoon <tekst>".into(), ..Body::default() };
        let html = page(&header(), Some(&plain));
        assert!(html.contains("<pre>Gewoon &lt;tekst&gt;</pre>") && !html.contains("Bijlagen"));
        assert!(page(&header(), None).contains("nog niet opgehaald"));
        assert_eq!(data_url("text/html", b"x"), None, "only pictures become data URLs");
    }
}
