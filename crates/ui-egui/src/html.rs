//! Draws the layout tree of an HTML message with egui's own text engine: paragraphs,
//! headings, lists, quotes, tables, links and pictures, nothing else. There is no browser
//! here, so nothing in a message can run or load; a link opens in the system browser only
//! when clicked, and its real target shows on hover. Pictures on the internet get a
//! placeholder until the person asks for them (see [`crate::remote`]). In the Junk folder the
//! links are switched off, the way Outlook does it: drawn, with their target on hover, but a
//! click does nothing.

use crate::i18n::tr;
use egui::{Color32, CornerRadius, Margin, RichText, Sense, Stroke, Ui, vec2};

use merel_html::{Block, Cell, Document, Image, ImageSource, Inline, Style};
use merel_mime::InlineImage;

use crate::Tokens;
use crate::remote::{Load, Loads};

/// Body text size, in points.
const TEXT_SIZE: f32 = 14.0;
/// Widest a picture is drawn; taller ones scale down with it.
const MAX_IMAGE: f32 = 800.0;

/// What the person did in the message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Clicked {
    /// A `mailto:` link: the address (and nothing after a `?`).
    Mailto(String),
}

struct Painter<'a> {
    t: &'a Tokens,
    images: &'a [InlineImage],
    /// Keeps the image cache of one message apart from the next.
    key: &'a str,
    /// Pictures on the internet the person asked for; `None` keeps them all blocked.
    remote: Option<&'a Loads>,
    /// Whether a click on a link opens it; off for a message in the Junk folder.
    links: bool,
    data_images: usize,
    remote_images: usize,
    clicked: Option<Clicked>,
}

/// What hovering a switched-off link says, before its target.
pub fn links_off() -> String {
    tr("junk-links-off")
}

/// Draws `doc`. `key` names the message, so its pictures are cached under their own names;
/// `links` false draws the links but lets no click through.
pub fn show(ui: &mut Ui, t: &Tokens, doc: &Document, images: &[InlineImage], key: &str, remote: Option<&Loads>, links: bool) -> Option<Clicked> {
    let mut p = Painter { t, images, key, remote, links, data_images: 0, remote_images: 0, clicked: None };
    p.blocks(ui, &doc.blocks);
    if doc.truncated {
        ui.label(RichText::new(tr("message-truncated")).color(t.text_dim).size(12.0));
    }
    p.clicked
}

impl Painter<'_> {
    fn blocks(&mut self, ui: &mut Ui, blocks: &[Block]) {
        for b in blocks {
            match b {
                Block::Paragraph(inlines) => {
                    self.inlines(ui, inlines, TEXT_SIZE);
                    ui.add_space(8.0);
                }
                Block::Heading { level, inlines } => {
                    let size = match level {
                        1 => 22.0,
                        2 => 19.0,
                        3 => 17.0,
                        _ => 15.0,
                    };
                    ui.add_space(4.0);
                    self.inlines(ui, inlines, size);
                    ui.add_space(6.0);
                }
                Block::Quote(inner) => {
                    ui.horizontal_top(|ui| {
                        let (bar, _) = ui.allocate_exact_size(vec2(3.0, 0.0), Sense::hover());
                        let height = ui.vertical(|ui| self.blocks(ui, inner)).response.rect.height();
                        let rect = egui::Rect::from_min_size(bar.min, vec2(3.0, height));
                        ui.painter().rect_filled(rect, 1.0, self.t.border);
                    });
                    ui.add_space(4.0);
                }
                Block::List { ordered, items } => {
                    for (i, item) in items.iter().enumerate() {
                        ui.horizontal_top(|ui| {
                            let marker = if *ordered { format!("{}.", i + 1) } else { "•".to_string() };
                            ui.add_sized(vec2(22.0, TEXT_SIZE + 4.0), egui::Label::new(RichText::new(marker).size(TEXT_SIZE).color(self.t.text)));
                            ui.vertical(|ui| self.blocks(ui, item));
                        });
                    }
                    ui.add_space(4.0);
                }
                Block::Table(rows) => {
                    let columns = rows.iter().map(Vec::len).max().unwrap_or(1).clamp(1, 8);
                    for row in rows {
                        ui.columns(columns, |cols| {
                            for (cell, col) in row.iter().zip(cols.iter_mut()) {
                                self.cell(col, cell);
                            }
                        });
                        ui.add(egui::Separator::default().spacing(4.0));
                    }
                    ui.add_space(6.0);
                }
                Block::Code(code) => {
                    egui::Frame::NONE.fill(self.t.panel).corner_radius(CornerRadius::same(4)).inner_margin(Margin::same(8)).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(code).monospace().size(13.0).color(self.t.text)).wrap());
                    });
                    ui.add_space(8.0);
                }
                Block::Rule => {
                    ui.add(egui::Separator::default().spacing(10.0));
                }
            }
        }
    }

    fn cell(&mut self, ui: &mut Ui, cell: &Cell) {
        let fill = if cell.header { self.t.panel } else { Color32::TRANSPARENT };
        egui::Frame::NONE.fill(fill).inner_margin(Margin::symmetric(4, 2)).show(ui, |ui| self.blocks(ui, &cell.blocks));
    }

    fn inlines(&mut self, ui: &mut Ui, inlines: &[Inline], size: f32) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for inline in inlines {
                match inline {
                    Inline::Text { text, style } => {
                        ui.add(egui::Label::new(self.rich(text, *style, size).color(self.t.text)).wrap());
                    }
                    Inline::Link { href, text, style } if !self.links => {
                        let rich = self.rich(text, *style, size).color(self.t.text_dim).underline();
                        ui.add(egui::Label::new(rich).wrap()).on_hover_text(format!("{}: {href}", links_off()));
                    }
                    Inline::Link { href, text, style } => {
                        let rich = self.rich(text, *style, size).color(self.t.accent_text).underline();
                        let r = ui
                            .add(egui::Label::new(rich).sense(Sense::click()).wrap())
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(href);
                        if r.clicked() {
                            self.open(ui, href);
                        }
                    }
                    Inline::Image(image) => self.image(ui, image),
                    Inline::Break => ui.end_row(),
                }
            }
        });
    }

    fn rich(&self, text: &str, style: Style, size: f32) -> RichText {
        let mut r = RichText::new(text).size(size);
        if style.bold {
            r = r.strong();
        }
        if style.italic {
            r = r.italics();
        }
        if style.underline {
            r = r.underline();
        }
        if style.strike {
            r = r.strikethrough();
        }
        if style.code {
            r = r.monospace().background_color(self.t.panel);
        }
        r
    }

    fn open(&mut self, ui: &Ui, href: &str) {
        let lower = href.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("mailto:") {
            let address = href.get(href.len() - rest.len()..).unwrap_or("").split('?').next().unwrap_or("").to_string();
            self.clicked = Some(Clicked::Mailto(address));
        } else if lower.starts_with("http://") || lower.starts_with("https://") {
            ui.ctx().open_url(egui::OpenUrl::new_tab(href));
        }
    }

    fn image(&mut self, ui: &mut Ui, image: &Image) {
        let (uri, bytes) = match &image.source {
            ImageSource::Inline(cid) => match self.images.iter().find(|i| i.cid == *cid) {
                Some(i) => (format!("bytes://merel/{}/cid/{cid}", self.key), i.bytes.clone()),
                None => return self.placeholder(ui, &tr("image-missing"), &image.alt),
            },
            ImageSource::Data { bytes, .. } => {
                self.data_images += 1;
                (format!("bytes://merel/{}/data/{}", self.key, self.data_images), bytes.clone())
            }
            ImageSource::Remote(url) => match self.remote.and_then(|r| r.get(url)) {
                None => return self.placeholder(ui, &tr("image-blocked"), &image.alt),
                Some(Load::Pending) => return self.placeholder(ui, &tr("image-loading"), &image.alt),
                Some(Load::Failed) => return self.placeholder(ui, &tr("image-failed"), &image.alt),
                Some(Load::Ready(bytes)) => {
                    self.remote_images += 1;
                    (format!("bytes://merel/{}/remote/{}", self.key, self.remote_images), bytes.clone())
                }
            },
        };
        let width = ui.available_width().min(MAX_IMAGE);
        let widget = egui::Image::from_bytes(uri, bytes).fit_to_original_size(1.0).max_width(width).corner_radius(CornerRadius::same(2));
        let mut r = ui.add(widget);
        if !image.alt.is_empty() {
            r = r.on_hover_text(&image.alt);
        }
        let _ = r;
    }

    fn placeholder(&self, ui: &mut Ui, what: &str, alt: &str) {
        egui::Frame::NONE
            .fill(self.t.panel)
            .stroke(Stroke::new(1.0, self.t.border))
            .corner_radius(CornerRadius::same(3))
            .inner_margin(Margin::symmetric(8, 4))
            .show(ui, |ui| {
                let text = if alt.is_empty() { what.to_string() } else { format!("{what}: {alt}") };
                ui.label(RichText::new(text).size(12.0).color(self.t.text_dim));
            });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use egui_kittest::kittest::Queryable;

    use super::*;

    #[test]
    fn links_are_drawn_and_a_mail_link_asks_for_the_compose_window() {
        let doc = merel_html::parse(
            r#"<h1>Kop</h1><p>Zie <a href="https://example.nl">de site</a> of <a href="mailto:sanne@example.org?subject=x">mail Sanne</a>.</p>
            <ul><li>een</li><li>twee</li></ul><table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td>2</td></tr></table>
            <blockquote>citaat</blockquote><pre>code</pre><img src="https://t.example/p.gif" alt="pixel">"#,
        );
        let clicked = Rc::new(RefCell::new(None));
        let seen = clicked.clone();
        let t = Tokens::light();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(900.0, 700.0)).build_ui(move |ui| {
            if let Some(c) = show(ui, &t, &doc, &[], "m1", None, true) {
                *seen.borrow_mut() = Some(c);
            }
        });
        h.run();
        for label in ["Kop", "de site", "een", "twee", "A", "2", "citaat", "code"] {
            assert!(h.query_by_label(label).is_some(), "{label} is drawn");
        }
        assert!(h.query_by_label_contains("geblokkeerd: pixel").is_some(), "a remote picture is a placeholder");
        h.get_by_label("mail Sanne").click();
        h.run();
        assert_eq!(*clicked.borrow(), Some(Clicked::Mailto("sanne@example.org".into())));
    }

    #[test]
    fn remote_pictures_follow_what_the_loader_knows() {
        let doc = merel_html::parse(r#"<p><img src="https://t.example/a.png" alt="eerste"> <img src="https://t.example/b.png" alt="tweede"></p>"#);
        let mut loads = Loads::new();
        loads.insert("https://t.example/a.png".into(), Load::Pending);
        loads.insert("https://t.example/b.png".into(), Load::Failed);
        let t = Tokens::light();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(600.0, 300.0)).build_ui(move |ui| {
            show(ui, &t, &doc, &[], "m2", Some(&loads), true);
        });
        h.run();
        assert!(h.query_by_label_contains("laden…: eerste").is_some());
        assert!(h.query_by_label_contains("kon niet worden geladen: tweede").is_some());
        assert!(h.query_by_label_contains("geblokkeerd").is_none());
    }

    #[test]
    fn switched_off_links_are_drawn_but_a_click_does_nothing() {
        let doc = merel_html::parse(r#"<p><a href="mailto:sanne@example.org">mail Sanne</a></p>"#);
        let clicked = Rc::new(RefCell::new(None));
        let seen = clicked.clone();
        let t = Tokens::light();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(600.0, 300.0)).build_ui(move |ui| {
            if let Some(c) = show(ui, &t, &doc, &[], "m3", None, false) {
                *seen.borrow_mut() = Some(c);
            }
        });
        h.run();
        h.get_by_label("mail Sanne").click();
        h.run();
        assert_eq!(*clicked.borrow(), None);
    }
}
