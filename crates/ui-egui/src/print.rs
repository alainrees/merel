//! Printing a message: Merel draws mail itself and has no printer of its own, so "Afdrukken"
//! writes the open message as a page the system browser can print and opens it there. The page
//! is built from the message's layout tree, never from the mail's own HTML: every text is
//! escaped, links keep only the addresses the tree allowed, pictures that travel inside the
//! message are embedded as data, pictures on the internet are left out, and a Content Security
//! Policy on the page forbids scripts and anything from the network.

use crate::i18n::tr;
use std::fmt::Write as _;
use std::path::PathBuf;

use base64::Engine;
use chrono::{Datelike, Days, Local, NaiveDate, TimeZone};
use merel_html::{Align, Block, Document, Image, ImageSource, Inline, Length, Look, Style};
use merel_mime::{Body, InlineImage};
use merel_model::{Address, Appointment, MessageHeader};

use crate::{MailApp, calendar};

/// The one file the print page is written to, in the system's temporary folder; each print
/// overwrites it, so no trail of messages is left behind.
pub fn page_path() -> PathBuf {
    std::env::temp_dir().join("merel-afdruk.html")
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
            let width = match img.width {
                Some(merel_html::Length::Px(px)) => format!(" width=\"{px}\""),
                Some(merel_html::Length::Percent(p)) => format!(" width=\"{p}%\""),
                None => String::new(),
            };
            let _ = write!(out, "<img src=\"{src}\" alt=\"{}\"{width}>", esc(&img.alt));
        }
        None if img.alt.trim().is_empty() => {
            let _ = write!(out, "<span class=\"missing\">[{}]</span>", esc(&tr("print-image-left-out")));
        }
        None => {
            let _ = write!(out, "<span class=\"missing\">[{}: {}]</span>", esc(&tr("print-image")), esc(&img.alt));
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
            Block::Columns(cells) => {
                out.push_str("<table class=\"cols\"><tr>");
                for cell in cells {
                    let _ = write!(out, "<td style=\"{}\">", look_css(&cell.look));
                    blocks(out, &cell.blocks, images);
                    out.push_str("</td>");
                }
                out.push_str("</tr></table>\n");
            }
            Block::Section { look, blocks: inner } => {
                let _ = writeln!(out, "<div style=\"{}\">", look_css(look));
                blocks(out, inner, images);
                out.push_str("</div>\n");
            }
            Block::Code(text) => {
                let _ = writeln!(out, "<pre>{}</pre>", esc(text));
            }
            Block::Rule => out.push_str("<hr>\n"),
        }
    }
}

/// The look of a block as CSS. Every part is a number or a fixed word, so nothing from the
/// message reaches the page as text.
fn look_css(look: &Look) -> String {
    let mut css = String::new();
    if let Some(c) = look.background {
        let _ = write!(css, "background:#{:02x}{:02x}{:02x};", c.0, c.1, c.2);
    }
    if let Some(a) = look.align {
        css.push_str(match a {
            Align::Left => "text-align:left;",
            Align::Center => "text-align:center;",
            Align::Right => "text-align:right;",
        });
    }
    match look.width {
        Some(Length::Px(px)) => {
            let _ = write!(css, "max-width:{px}px;");
        }
        Some(Length::Percent(p)) => {
            let _ = write!(css, "width:{p}%;");
        }
        None => {}
    }
    if look.place == Some(Align::Center) {
        css.push_str("margin:0 auto;");
    }
    let [t, r, b, l] = look.padding;
    let _ = write!(css, "padding:{t}px {r}px {b}px {l}px;");
    if let Some(h) = look.height {
        let _ = write!(css, "min-height:{h}px;");
    }
    css
}

const STYLE: &str = "body{font-family:-apple-system,Segoe UI,Helvetica,Arial,sans-serif;font-size:11pt;color:#111;max-width:52em;margin:2em auto;padding:0 1em}\
.hint{background:#eef;border:1px solid #ccd;padding:.5em .8em;border-radius:4px;font-size:10pt}\
table.head{border-collapse:collapse;margin:1em 0}table.head th{text-align:left;padding:.15em 1em .15em 0;color:#555;font-weight:600;white-space:nowrap;vertical-align:top}\
table.head td{padding:.15em 0}h1.subject{font-size:16pt;margin:.2em 0 .6em}hr.sep{border:0;border-top:1px solid #999;margin:1em 0}\
.body table{border-collapse:collapse}.body table.cols{width:100%}.body table.cols td{border:0;padding:0;vertical-align:middle}.body td,.body th{border:1px solid #bbb;padding:.2em .5em;vertical-align:top}blockquote{border-left:3px solid #bbb;margin:.5em 0;padding-left:.8em;color:#444}\
pre{white-space:pre-wrap;font-family:Menlo,Consolas,monospace;font-size:10pt}img{max-width:100%}.missing{color:#888;font-style:italic}a{color:#1a4d8f}\
h2.day{font-size:12pt;margin:1.2em 0 .3em;padding-bottom:.15em;border-bottom:1px solid #999}table.cal{border-collapse:collapse;width:100%}\
table.cal td{padding:.25em .8em .25em 0;vertical-align:top}table.cal td.time{white-space:nowrap;color:#555;width:9em}table.cal td.where{color:#555}\
@media print{.hint{display:none}body{margin:0;max-width:none}}";

/// The page for a stretch of the calendar: a heading per day that has appointments, with each
/// one's time, title and place. `first` and `last` are the days shown, both included;
/// `list` holds the occurrences touching them, as `appointments_between` gives them.
pub fn calendar_page(title: &str, first: NaiveDate, last: NaiveDate, list: &[Appointment]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "<!doctype html>\n<html lang=\"{}\"><head><meta charset=\"utf-8\">", crate::i18n::current().tag());
    out.push_str("<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'\">\n");
    let _ = writeln!(out, "<title>{} {}</title>", esc(&tr("print-calendar")), esc(title));
    let _ = writeln!(out, "<style>{STYLE}</style></head><body>");
    let _ = writeln!(out, "<p class=\"hint\">{}</p>", esc(&tr("print-hint")));
    let _ = writeln!(out, "<h1 class=\"subject\">{}: {}</h1>", esc(&tr("print-calendar")), esc(title));
    let mut day = first;
    let mut any = false;
    while day <= last {
        let from = calendar::to_timestamp(day.and_hms_opt(0, 0, 0).unwrap_or_default()).unwrap_or(i64::MIN);
        let next = day.checked_add_days(Days::new(1)).unwrap_or(day);
        let to = calendar::to_timestamp(next.and_hms_opt(0, 0, 0).unwrap_or_default()).unwrap_or(i64::MAX);
        let today: Vec<&Appointment> = list.iter().filter(|a| a.start < to && a.end > from).collect();
        if !today.is_empty() {
            any = true;
            let _ = writeln!(out, "<h2 class=\"day\">{} {} {}</h2>", calendar::weekday_name(day), day.day(), calendar::month_name(day.month()));
            out.push_str("<table class=\"cal\">\n");
            for a in today {
                let where_ =
                    if a.location.trim().is_empty() { String::new() } else { format!(" <td class=\"where\">{}</td>", esc(a.location.trim())) };
                let _ = writeln!(out, "<tr><td class=\"time\">{}</td><td>{}</td>{where_}</tr>", esc(&time_cell(a, from, to)), esc(a.title.trim()));
            }
            out.push_str("</table>\n");
        }
        if next == day {
            break;
        }
        day = next;
    }
    if !any {
        let _ = writeln!(out, "<p class=\"missing\">{}</p>", esc(&tr("print-no-appointments")));
    }
    out.push_str("</body></html>\n");
    out
}

/// The time column for one day: "Hele dag", "10:00–11:30", or where a longer appointment
/// starts or ends on another day, "10:00 – verder" and "tot 11:30".
fn time_cell(a: &Appointment, day_start: i64, day_end: i64) -> String {
    if a.all_day {
        return tr("all-day");
    }
    let clock = |ts: i64| calendar::to_local(ts).format("%H:%M").to_string();
    match (a.start >= day_start, a.end <= day_end) {
        (true, true) => format!("{}–{}", clock(a.start), clock(a.end)),
        (true, false) => crate::i18n::tr_args("print-time-on", &[("time", clock(a.start).into())]),
        (false, true) => crate::i18n::tr_args("print-time-until", &[("time", clock(a.end).into())]),
        (false, false) => tr("all-day"),
    }
}

/// The whole page for a message: a header block with sender, recipients, date, subject and
/// attachments, a separator, then the body (the layout tree when the message had HTML, else the
/// plain text).
pub fn page(h: &MessageHeader, body: Option<&Body>) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "<!doctype html>\n<html lang=\"{}\"><head><meta charset=\"utf-8\">", crate::i18n::current().tag());
    out.push_str("<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src data:; style-src 'unsafe-inline'\">\n");
    let _ = writeln!(out, "<title>{}</title>", esc(&h.subject));
    let _ = writeln!(out, "<style>{STYLE}</style></head><body>");
    let _ = writeln!(out, "<p class=\"hint\">{}</p>", esc(&tr("print-hint")));
    let no_subject = tr("print-no-subject");
    let _ = writeln!(out, "<h1 class=\"subject\">{}</h1>", esc(if h.subject.trim().is_empty() { &no_subject } else { &h.subject }));
    out.push_str("<table class=\"head\">\n");
    let _ = writeln!(out, "<tr><th>{}</th><td>{}</td></tr>", esc(&tr("compose-from")), address(&h.from));
    if !h.to.is_empty() {
        let to: Vec<String> = h.to.iter().map(address).collect();
        let _ = writeln!(out, "<tr><th>{}</th><td>{}</td></tr>", esc(&tr("compose-to")), to.join(", "));
    }
    let _ = writeln!(out, "<tr><th>{}</th><td>{}</td></tr>", esc(&tr("print-date")), date(h.date));
    if let Some(b) = body
        && !b.attachments.is_empty()
    {
        let names: Vec<String> = b.attachments.iter().map(|a| esc(&a.name)).collect();
        let _ = writeln!(out, "<tr><th>{}</th><td>{}</td></tr>", esc(&tr("print-attachments")), names.join(", "));
    }
    out.push_str("</table>\n<hr class=\"sep\">\n<div class=\"body\">\n");
    match body {
        Some(Body { rich: Some(doc), images, .. }) if !doc.is_empty() => document(&mut out, doc, images),
        Some(b) => {
            let _ = writeln!(out, "<pre>{}</pre>", esc(&b.text));
        }
        None => {
            let _ = writeln!(out, "<p class=\"missing\">[{}]</p>", esc(&tr("print-not-fetched")));
        }
    }
    out.push_str("</div>\n</body></html>\n");
    out
}

fn document(out: &mut String, doc: &Document, images: &[InlineImage]) {
    blocks(out, &doc.blocks, images);
    if doc.remote_images > 0 {
        let _ =
            writeln!(out, "<p class=\"missing\">[{}]</p>", esc(&crate::i18n::tr_args("print-remote-images", &[("count", doc.remote_images.into())])));
    }
    if doc.truncated {
        let _ = writeln!(out, "<p class=\"missing\">[{}]</p>", esc(&tr("print-truncated")));
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

    /// "Afdrukken" in the calendar: the days of the open view (day, work week, week or month)
    /// with their appointments, as a page in the browser.
    pub fn print_calendar(&mut self) {
        let (first, last) = calendar::period(self.calendar.view, self.calendar.focus);
        let from = calendar::to_timestamp(first.and_hms_opt(0, 0, 0).unwrap_or_default()).unwrap_or(i64::MIN);
        let after = last.checked_add_days(Days::new(1)).unwrap_or(last);
        let to = calendar::to_timestamp(after.and_hms_opt(0, 0, 0).unwrap_or_default()).unwrap_or(i64::MAX);
        let list = calendar::shown(self, self.engine.session.appointments_between(from, to));
        let html = calendar_page(&calendar::period_title(self.calendar.view, self.calendar.focus), first, last, &list);
        let path = page_path();
        self.notice = Some(match write_and_open(&path, &html) {
            Ok(()) => crate::Notice::info(tr("print-opened")),
            Err(e) => {
                log::warn!("print page {}: {e}", path.display());
                crate::Notice::error(crate::i18n::tr_args("print-failed", &[("error", e.to_string().into())]))
            }
        });
    }

    /// "Afdrukken": the open message as a page in the browser, to print from there.
    pub fn print_selected(&mut self) {
        let Some(h) = self.engine.session.selected().cloned() else { return };
        let Some(body) = self.engine.session.selected_body().cloned() else {
            self.notice = Some(crate::Notice::warning(tr("print-not-fetched-yet")));
            return;
        };
        let html = page(&h, Some(&body));
        let path = page_path();
        self.notice = Some(match write_and_open(&path, &html) {
            Ok(()) => crate::Notice::info(tr("print-opened")),
            Err(e) => {
                log::warn!("print page {}: {e}", path.display());
                crate::Notice::error(crate::i18n::tr_args("print-failed", &[("error", e.to_string().into())]))
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use merel_html::Cell;
    use merel_model::Flags;

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
            importance: Default::default(),
            chat: None,
            sealed: Default::default(),
            size: 0,
        }
    }

    #[test]
    fn the_calendar_page_groups_the_appointments_by_day_and_escapes_them() {
        let d = |y, m, day| chrono::NaiveDate::from_ymd_opt(y, m, day).unwrap();
        let at = |day: chrono::NaiveDate, h, min| calendar::to_timestamp(day.and_hms_opt(h, min, 0).unwrap()).unwrap();
        let appointment = |id: &str, title: &str, start: i64, end: i64, all_day: bool, location: &str| Appointment {
            id: id.into(),
            calendar_id: "local".into(),
            title: title.into(),
            start,
            end,
            all_day,
            location: location.into(),
            notes: String::new(),
            notes_rich: None,
            recurrence: None,
            reminder: None,
            private: false,
            show_as: merel_model::ShowAs::Busy,
            categories: Vec::new(),
            meeting: None,
            online: None,
            make_online: false,
        };
        let mon = d(2026, 10, 19);
        let tue = d(2026, 10, 20);
        let list = [
            appointment("a", "Overleg <ontwerp>", at(mon, 10, 0), at(mon, 11, 30), false, "Kamer 2 & 3"),
            appointment("b", "Studiedag", at(tue, 0, 0), at(d(2026, 10, 21), 0, 0), true, ""),
            appointment("c", "Beurs", at(tue, 15, 0), at(d(2026, 10, 21), 12, 0), false, "Utrecht"),
        ];
        let html = calendar_page("19 – 23 oktober 2026", mon, d(2026, 10, 23), &list);
        assert!(html.contains("<title>Agenda 19 – 23 oktober 2026</title>") && html.contains("default-src 'none'"), "{html}");
        assert!(html.contains("<h2 class=\"day\">maandag 19 oktober</h2>"), "{html}");
        assert!(
            html.contains("<td class=\"time\">10:00–11:30</td><td>Overleg &lt;ontwerp&gt;</td> <td class=\"where\">Kamer 2 &amp; 3</td>"),
            "{html}"
        );
        assert!(html.contains("<td class=\"time\">Hele dag</td><td>Studiedag</td></tr>"), "{html}");
        assert!(html.contains("<td class=\"time\">15:00 – verder</td><td>Beurs</td>"), "the start day of a two-day appointment");
        assert!(
            html.contains("<h2 class=\"day\">woensdag 21 oktober</h2>") && html.contains("<td class=\"time\">tot 12:00</td><td>Beurs</td>"),
            "{html}"
        );
        assert!(!html.contains("donderdag 22 oktober"), "a day without appointments is left out");
        assert!(!html.contains("<script"));
        let empty = calendar_page("vrijdag 23 oktober 2026", d(2026, 10, 23), d(2026, 10, 23), &[]);
        assert!(empty.contains("Geen afspraken in deze periode."));
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
                    Inline::Image(Image { source: ImageSource::Inline("logo".into()), alt: "Logo".into(), width: None, height: None }),
                    Inline::Image(Image { source: ImageSource::Remote("https://x/y.png".into()), alt: "Ver weg".into(), width: None, height: None }),
                ]),
                Block::Table(vec![vec![Cell {
                    header: true,
                    blocks: vec![Block::Paragraph(vec![Inline::Text { text: "Kop".into(), style: Style::default() }])],
                    look: Look::default(),
                }]]),
                Block::Code("a < b".into()),
            ],
            remote_images: 1,
            truncated: false,
            designed: false,
        };
        let body = Body {
            text: "tekst".into(),
            from_html: true,
            rich: Some(doc),
            images: vec![InlineImage { cid: "logo".into(), mime: "image/png".into(), bytes: Arc::from(&b"\x89PNG"[..]) }],
            attachments: vec![merel_mime::Attachment {
                name: "offerte<1>.pdf".into(),
                mime: "application/pdf".into(),
                size: 3,
                bytes: Arc::from(&b"abc"[..]),
            }],
            authentication: Vec::new(),
            unsubscribe: Vec::new(),
            receipt_to: None,
            sender: None,
            signature: None,
            pgp_signature: None,
            encrypted: None,
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
