//! Draws the layout tree of an HTML message with egui's own text engine: paragraphs,
//! headings, lists, quotes, tables, links and pictures, nothing else. There is no browser
//! here, so nothing in a message can run or load; a link opens in the system browser only
//! when clicked, and its real target shows on hover. Pictures on the internet get a
//! placeholder until the person asks for them (see [`crate::remote`]). In the Junk folder the
//! links are switched off, the way Outlook does it: drawn, with their target on hover, but a
//! click does nothing.
//!
//! A message that sets its own colours (a newsletter, a receipt) is drawn the way Outlook
//! draws it: on the white ground it was made for, with its backgrounds, its widths, its
//! columns side by side and its pictures at the size it asked for.

use crate::i18n::tr;
use egui::{Color32, CornerRadius, Margin, RichText, Sense, Stroke, TextWrapMode, Ui, vec2};

use merel_html::{Align, Block, Cell, Document, Image, ImageSource, Inline, Look, Rgb, Style, VAlign};
use merel_mime::InlineImage;
use merel_model::rich::Face;

use crate::Tokens;
use crate::found::{self, Marks};
use crate::remote::{Load, Loads};

/// Body text size, in points.
const TEXT_SIZE: f32 = 14.0;
/// Widest a picture is drawn when the message does not say how wide; taller ones scale down
/// with it.
const MAX_IMAGE: f32 = 800.0;
/// The ground a designed message is drawn on, and its text and link colours where it sets
/// none: Outlook's.
const PAPER: Color32 = Color32::WHITE;
const PAPER_INK: Color32 = Color32::from_rgb(0x1b, 0x1b, 0x1b);
const PAPER_LINK: Color32 = Color32::from_rgb(0x05, 0x63, 0xc1);
const PAPER_PANEL: Color32 = Color32::from_rgb(0xf0, 0xf0, 0xf0);
const PAPER_DIM: Color32 = Color32::from_rgb(0x5f, 0x5f, 0x5f);
/// Narrowest a column without a width of its own gets.
const MIN_COLUMN: f32 = 40.0;

/// What the person did in the message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Clicked {
    /// A `mailto:` link: the address (and nothing after a `?`).
    Mailto(String),
    /// A chat invitation (`https://i.delta.chat/#…`), which Merel takes itself.
    Invite(String),
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
    /// The words a search found, to mark; see [`crate::found`].
    found: Option<&'a Marks>,
    /// Text, link, dimmed text and shaded ground where the message sets none: the theme's,
    /// or Outlook's for a designed message.
    ink: Color32,
    link: Color32,
    dim: Color32,
    panel: Color32,
    /// How text lines up where the block it is in says so.
    align: Align,
    /// Whether the app's bold and italic fonts are loaded yet.
    faces: bool,
    data_images: usize,
    remote_images: usize,
    /// Rows of columns drawn so far, to name where each remembers its cell heights.
    rows: usize,
    clicked: Option<Clicked>,
}

/// What hovering a switched-off link says, before its target.
pub fn links_off() -> String {
    tr("junk-links-off")
}

fn rgb(c: Rgb) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
}

fn margin(padding: [u16; 4]) -> Margin {
    let side = |p: u16| i8::try_from(p).unwrap_or(i8::MAX);
    Margin { top: side(padding[0]), right: side(padding[1]), bottom: side(padding[2]), left: side(padding[3]) }
}

/// Draws `doc`. `key` names the message, so its pictures are cached under their own names;
/// `links` false draws the links but lets no click through; `found` marks what a search found.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    t: &Tokens,
    doc: &Document,
    images: &[InlineImage],
    key: &str,
    remote: Option<&Loads>,
    links: bool,
    found: Option<&Marks>,
) -> Option<Clicked> {
    let designed = doc.designed;
    let mut p = Painter {
        t,
        images,
        key,
        remote,
        links,
        found,
        ink: if designed { PAPER_INK } else { t.text },
        link: if designed { PAPER_LINK } else { t.accent_text },
        dim: if designed { PAPER_DIM } else { t.text_dim },
        panel: if designed { PAPER_PANEL } else { t.panel },
        align: Align::Left,
        faces: {
            let (family, _) = crate::fonts::family(Face::Sans, true, false);
            ui.ctx().fonts(|f| f.definitions().families.contains_key(&family))
        },
        data_images: 0,
        remote_images: 0,
        rows: 0,
        clicked: None,
    };
    if designed {
        egui::Frame::NONE.fill(PAPER).corner_radius(CornerRadius::same(4)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            p.blocks(ui, &doc.blocks);
        });
    } else {
        p.blocks(ui, &doc.blocks);
    }
    if doc.truncated {
        ui.label(RichText::new(tr("message-truncated")).color(t.text_dim).size(12.0));
    }
    p.clicked
}

impl Painter<'_> {
    fn blocks(&mut self, ui: &mut Ui, blocks: &[Block]) {
        for (i, b) in blocks.iter().enumerate() {
            // No room after the last block, so a coloured box ends where its content does.
            let gap = |ui: &mut Ui, space: f32| {
                if i + 1 < blocks.len() {
                    ui.add_space(space);
                }
            };
            match b {
                Block::Paragraph(inlines) => {
                    self.inlines(ui, inlines, TEXT_SIZE);
                    gap(ui, 8.0);
                }
                Block::Heading { level, inlines } => {
                    let size = match level {
                        1 => 22.0,
                        2 => 19.0,
                        3 => 17.0,
                        _ => 15.0,
                    };
                    if i > 0 {
                        ui.add_space(4.0);
                    }
                    self.inlines(ui, inlines, size);
                    gap(ui, 6.0);
                }
                Block::Quote(inner) => {
                    ui.horizontal_top(|ui| {
                        let (bar, _) = ui.allocate_exact_size(vec2(3.0, 0.0), Sense::hover());
                        let height = ui.vertical(|ui| self.blocks(ui, inner)).response.rect.height();
                        let rect = egui::Rect::from_min_size(bar.min, vec2(3.0, height));
                        ui.painter().rect_filled(rect, 1.0, self.t.border);
                    });
                    gap(ui, 4.0);
                }
                Block::List { ordered, items } => {
                    for (i, item) in items.iter().enumerate() {
                        ui.horizontal_top(|ui| {
                            let marker = if *ordered { format!("{}.", i + 1) } else { "•".to_string() };
                            ui.add_sized(vec2(22.0, TEXT_SIZE + 4.0), egui::Label::new(RichText::new(marker).size(TEXT_SIZE).color(self.ink)));
                            ui.vertical(|ui| self.blocks(ui, item));
                        });
                    }
                    gap(ui, 4.0);
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
                    gap(ui, 6.0);
                }
                Block::Columns(cells) => self.columns(ui, cells),
                Block::Section { look, blocks } => self.section(ui, look, blocks),
                Block::Code(code) => {
                    egui::Frame::NONE.fill(self.panel).corner_radius(CornerRadius::same(4)).inner_margin(Margin::same(8)).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(code).monospace().size(13.0).color(self.ink)).wrap());
                    });
                    gap(ui, 8.0);
                }
                Block::Rule => {
                    ui.add(egui::Separator::default().spacing(10.0));
                }
            }
        }
    }

    fn cell(&mut self, ui: &mut Ui, cell: &Cell) {
        let fill = cell.look.background.map(rgb).unwrap_or(if cell.header { self.panel } else { Color32::TRANSPARENT });
        egui::Frame::NONE.fill(fill).inner_margin(Margin::symmetric(4, 2)).show(ui, |ui| self.blocks(ui, &cell.blocks));
    }

    /// A block with a look of its own: as wide as it asks (never wider than there is room
    /// for), placed left, centred or right, on its background, with its padding.
    fn section(&mut self, ui: &mut Ui, look: &Look, blocks: &[Block]) {
        let room = ui.available_width();
        let margin = margin(look.padding);
        let width = match look.width {
            Some(w) => w.of(room) + margin.sum().x,
            None if look.fit => self.natural_width(ui, blocks).map_or(room, |w| w + margin.sum().x),
            None => room,
        }
        .clamp(1.0, room.max(1.0));
        let offset = match look.place.unwrap_or(self.align) {
            Align::Left => 0.0,
            Align::Center => (room - width) / 2.0,
            Align::Right => room - width,
        }
        .max(0.0);
        let inner = (width - margin.sum().x).max(1.0);
        let min_height = (f32::from(look.height.unwrap_or(0)) - margin.sum().y).max(0.0);
        let saved = self.align;
        self.align = look.align.unwrap_or(saved);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(offset);
            ui.allocate_ui_with_layout(vec2(width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                egui::Frame::NONE.fill(look.background.map_or(Color32::TRANSPARENT, rgb)).inner_margin(margin).show(ui, |ui| {
                    ui.set_width(inner);
                    ui.set_min_height(min_height);
                    self.blocks(ui, blocks);
                });
            });
        });
        self.align = saved;
    }

    /// One row of a layout table: its cells side by side at the widths they ask for, the
    /// rest shared by the cells that ask none, each on its background the full height of the
    /// row, its content in the middle unless the cell says otherwise.
    fn columns(&mut self, ui: &mut Ui, cells: &[Cell]) {
        let room = ui.available_width();
        let mut widths: Vec<Option<f32>> = cells.iter().map(|c| c.look.width.map(|w| w.of(room) + margin(c.look.padding).sum().x)).collect();
        let free = widths.iter().filter(|w| w.is_none()).count() as f32;
        let fixed: f32 = widths.iter().flatten().sum();
        let left_over = room - free * MIN_COLUMN;
        if fixed > left_over && fixed > 0.0 {
            let scale = left_over.max(0.0) / fixed;
            widths.iter_mut().flatten().for_each(|w| *w *= scale);
        }
        let fixed: f32 = widths.iter().flatten().sum();
        let share = if free > 0.0 { ((room - fixed) / free).max(MIN_COLUMN) } else { 0.0 };
        let widths: Vec<f32> = widths.into_iter().map(|w| w.unwrap_or(share).max(1.0)).collect();

        // How tall each cell was last time, to sit it in the row now.
        let id = egui::Id::new(("merel-html-row", self.key, self.rows));
        self.rows += 1;
        let before: Vec<f32> = ui.data(|d| d.get_temp(id)).unwrap_or_default();
        let row_height = before.iter().copied().fold(0.0, f32::max);
        let mut heights = Vec::with_capacity(cells.len());
        let mut grounds = Vec::new();
        let saved = self.align;
        let row = ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (i, (cell, &width)) in cells.iter().zip(&widths).enumerate() {
                let ground = ui.painter().add(egui::Shape::Noop);
                let was = before.get(i).copied().unwrap_or(row_height);
                let shift = match cell.look.valign.unwrap_or(VAlign::Middle) {
                    VAlign::Top => 0.0,
                    VAlign::Middle => (row_height - was) / 2.0,
                    VAlign::Bottom => row_height - was,
                }
                .max(0.0);
                let margin = margin(cell.look.padding);
                let min_height = (f32::from(cell.look.height.unwrap_or(0)) - margin.sum().y).max(0.0);
                self.align = cell.look.align.unwrap_or(saved);
                let r = ui.allocate_ui_with_layout(vec2(width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(width);
                    ui.add_space(shift);
                    egui::Frame::NONE
                        .inner_margin(margin)
                        .show(ui, |ui| {
                            ui.set_width((width - margin.sum().x).max(1.0));
                            ui.set_min_height(min_height);
                            self.blocks(ui, &cell.blocks);
                        })
                        .response
                        .rect
                        .height()
                });
                heights.push(r.inner);
                if let Some(c) = cell.look.background {
                    grounds.push((ground, r.response.rect.x_range(), rgb(c)));
                }
            }
        });
        self.align = saved;
        let y = row.response.rect.y_range();
        for (ground, x, color) in grounds {
            ui.painter().set(ground, egui::Shape::rect_filled(egui::Rect::from_x_y_ranges(x, y), 0.0, color));
        }
        if heights != before {
            ui.data_mut(|d| d.insert_temp(id, heights));
            ui.ctx().request_repaint();
        }
    }

    fn inlines(&mut self, ui: &mut Ui, inlines: &[Inline], size: f32) {
        // Centred or right-aligned text: the room before each line, when it fits on one.
        let indents: Vec<f32> = match self.align {
            Align::Left => Vec::new(),
            align => {
                let room = ui.available_width();
                self.line_widths(ui, inlines, size)
                    .into_iter()
                    .map(|w| {
                        if w > room {
                            0.0
                        } else if align == Align::Center {
                            (room - w) / 2.0
                        } else {
                            room - w
                        }
                    })
                    .collect()
            }
        };
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let mut line = 0;
            if let Some(&indent) = indents.first() {
                ui.add_space(indent);
            }
            for inline in inlines {
                match inline {
                    Inline::Text { style, .. } | Inline::Link { style, .. } if style.size == Some(0) => {}
                    Inline::Text { text, style } => match self.found.and_then(|f| f.get(text)) {
                        Some(marks) => {
                            for (piece, hit) in found::pieces(text, marks) {
                                let rich = self.rich(piece, *style, size);
                                let rich =
                                    if hit { rich.color(self.t.on_found).background_color(self.t.found) } else { rich.color(self.ink_of(*style)) };
                                ui.add(egui::Label::new(rich).wrap());
                            }
                        }
                        None => {
                            ui.add(egui::Label::new(self.rich(text, *style, size).color(self.ink_of(*style))).wrap());
                        }
                    },
                    Inline::Link { href, text, style } if !self.links => {
                        let rich = self.rich(text, *style, size).color(self.dim).underline();
                        ui.add(egui::Label::new(rich).wrap()).on_hover_text(format!("{}: {href}", links_off()));
                    }
                    Inline::Link { href, text, style } => {
                        let mut rich = self.rich(text, *style, size).color(style.color.map_or(self.link, rgb));
                        if !style.plain_link {
                            rich = rich.underline();
                        }
                        // A link with a word found in it is marked whole, so it stays one link.
                        if self.found.is_some_and(|f| f.contains_key(text)) {
                            rich = rich.color(self.t.on_found).background_color(self.t.found);
                        }
                        let r = ui
                            .add(egui::Label::new(rich).sense(Sense::click()).wrap())
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(href);
                        if r.clicked() {
                            self.open(ui, href);
                        }
                    }
                    Inline::Image(image) => self.image(ui, image),
                    Inline::Break => {
                        ui.end_row();
                        line += 1;
                        if let Some(&indent) = indents.get(line) {
                            ui.add_space(indent);
                        }
                    }
                }
            }
        });
    }

    /// How wide each line of `inlines` is, written out without wrapping.
    fn line_widths(&mut self, ui: &Ui, inlines: &[Inline], size: f32) -> Vec<f32> {
        // Naming pictures counts them; measuring must not change the names drawing gives.
        let counted = (self.data_images, self.remote_images);
        let mut widths = vec![0.0];
        for inline in inlines {
            let w = match inline {
                Inline::Text { style, .. } | Inline::Link { style, .. } if style.size == Some(0) => 0.0,
                Inline::Text { text, style } | Inline::Link { text, style, .. } => {
                    egui::WidgetText::from(self.rich(text, *style, size))
                        .into_galley(ui, Some(TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body)
                        .size()
                        .x
                }
                Inline::Image(image) => self.picture(image).map_or(0.0, |(uri, bytes)| {
                    let room = ui.available_width();
                    self.sized(egui::Image::from_bytes(uri, bytes), image, room)
                        .load_and_calc_size(ui, vec2(room, f32::INFINITY))
                        .map_or(0.0, |s| s.x)
                }),
                Inline::Break => {
                    widths.push(0.0);
                    continue;
                }
            };
            if let Some(last) = widths.last_mut() {
                *last += w;
            }
        }
        (self.data_images, self.remote_images) = counted;
        widths
    }

    /// How wide `blocks` are when nothing in them wraps, when that can be known: text,
    /// pictures and boxes of a known width. `None` for anything that takes the width there is.
    fn natural_width(&mut self, ui: &Ui, blocks: &[Block]) -> Option<f32> {
        let mut widest: f32 = 0.0;
        for b in blocks {
            let w = match b {
                Block::Paragraph(inlines) => self.line_widths(ui, inlines, TEXT_SIZE).into_iter().fold(0.0, f32::max),
                Block::Section { look, blocks } => {
                    let pad = margin(look.padding).sum().x;
                    match look.width {
                        Some(merel_html::Length::Px(px)) => f32::from(px) + pad,
                        Some(merel_html::Length::Percent(_)) => return None,
                        None if look.fit => self.natural_width(ui, blocks)? + pad,
                        None => return None,
                    }
                }
                _ => return None,
            };
            widest = widest.max(w);
        }
        Some(widest)
    }

    fn ink_of(&self, style: Style) -> Color32 {
        style.color.map_or(self.ink, rgb)
    }

    fn rich(&self, text: &str, style: Style, size: f32) -> RichText {
        let mut r = RichText::new(text).size(style.size.map_or(size, f32::from));
        if (style.bold || style.italic) && !style.code && self.faces {
            let (family, slant) = crate::fonts::family(Face::Sans, style.bold, style.italic);
            r = r.family(family);
            if slant {
                r = r.italics();
            }
        } else {
            if style.bold {
                r = r.strong();
            }
            if style.italic {
                r = r.italics();
            }
        }
        if style.underline {
            r = r.underline();
        }
        if style.strike {
            r = r.strikethrough();
        }
        if style.code {
            r = r.monospace().background_color(self.panel);
        }
        r
    }

    fn open(&mut self, ui: &Ui, href: &str) {
        let lower = href.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("mailto:") {
            let address = href.get(href.len() - rest.len()..).unwrap_or("").split('?').next().unwrap_or("").to_string();
            self.clicked = Some(Clicked::Mailto(address));
        } else if lower.starts_with(merel_engine::securejoin::INVITE_LINK) {
            self.clicked = Some(Clicked::Invite(href.to_string()));
        } else if lower.starts_with("http://") || lower.starts_with("https://") {
            ui.ctx().open_url(egui::OpenUrl::new_tab(href));
        }
    }

    /// The name and bytes of a picture that can be drawn now; `None` for one that is
    /// missing, blocked, loading or broken.
    fn picture(&mut self, image: &Image) -> Option<(String, std::sync::Arc<[u8]>)> {
        match &image.source {
            ImageSource::Inline(cid) => {
                self.images.iter().find(|i| i.cid == *cid).map(|i| (format!("bytes://merel/{}/cid/{cid}", self.key), i.bytes.clone()))
            }
            ImageSource::Data { bytes, .. } => {
                self.data_images += 1;
                Some((format!("bytes://merel/{}/data/{}", self.key, self.data_images), bytes.clone()))
            }
            ImageSource::Remote(url) => match self.remote.and_then(|r| r.get(url)) {
                Some(Load::Ready(bytes)) => {
                    self.remote_images += 1;
                    Some((format!("bytes://merel/{}/remote/{}", self.key, self.remote_images), bytes.clone()))
                }
                _ => None,
            },
        }
    }

    /// `widget` at the size the message asked for, never wider than `room`: both sides as
    /// given, or one side with the other following the picture; its own size otherwise.
    fn sized<'i>(&self, widget: egui::Image<'i>, image: &Image, room: f32) -> egui::Image<'i> {
        let width = image.width.map(|w| w.of(room));
        let height = image.height.map(f32::from);
        match (width, height) {
            (Some(w), Some(h)) => {
                let scale = (room / w).min(1.0);
                widget.fit_to_exact_size(vec2(w * scale, h * scale)).maintain_aspect_ratio(false)
            }
            (Some(w), None) => widget.fit_to_exact_size(vec2(w.min(room), f32::INFINITY)),
            (None, Some(h)) => widget.fit_to_exact_size(vec2(room, h)),
            (None, None) => widget.fit_to_original_size(1.0).max_width(room.min(MAX_IMAGE)),
        }
    }

    fn image(&mut self, ui: &mut Ui, image: &Image) {
        let Some((uri, bytes)) = self.picture(image) else {
            let what = match &image.source {
                ImageSource::Inline(_) | ImageSource::Data { .. } => "image-missing",
                ImageSource::Remote(url) => match self.remote.and_then(|r| r.get(url)) {
                    None => "image-blocked",
                    Some(Load::Pending) => "image-loading",
                    Some(_) => "image-failed",
                },
            };
            return self.placeholder(ui, &tr(what), &image.alt);
        };
        let mut widget = self.sized(egui::Image::from_bytes(uri, bytes), image, ui.available_width()).corner_radius(CornerRadius::same(2));
        if !image.alt.is_empty() {
            widget = widget.alt_text(&image.alt);
        }
        let r = ui.add(widget);
        if !image.alt.is_empty() {
            r.on_hover_text(&image.alt);
        }
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
            if let Some(c) = show(ui, &t, &doc, &[], "m1", None, true, None) {
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
            show(ui, &t, &doc, &[], "m2", Some(&loads), true, None);
        });
        h.run();
        assert!(h.query_by_label_contains("laden…: eerste").is_some());
        assert!(h.query_by_label_contains("kon niet worden geladen: tweede").is_some());
        assert!(h.query_by_label_contains("geblokkeerd").is_none());
    }

    #[test]
    fn the_words_a_search_found_are_drawn_apart_on_their_ground() {
        let doc = merel_html::parse("<p>Hierbij de <b>facturen</b> van oktober.</p>");
        let mut found = Marks::new();
        found.insert("facturen".into(), std::iter::once(0..8).collect());
        let t = Tokens::light();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(600.0, 200.0)).build_ui(move |ui| {
            show(ui, &t, &doc, &[], "m4", None, true, Some(&found));
        });
        h.run();
        assert!(h.query_by_label("facturen").is_some());
        assert!(h.query_by_label("Hierbij de ").is_some(), "the rest stays as it was");
    }

    #[test]
    fn a_newsletter_draws_its_pictures_at_their_size_beside_their_text() {
        // A 120 px picture the message asks to show at 60 px, beside its title, as Outlook
        // draws Freelance.nl's matches.
        use base64::Engine as _;
        let png = {
            let mut out = std::io::Cursor::new(Vec::new());
            image::RgbaImage::from_pixel(120, 120, image::Rgba([0xec, 0x7a, 0x31, 255])).write_to(&mut out, image::ImageFormat::Png).unwrap();
            base64::engine::general_purpose::STANDARD.encode(out.into_inner())
        };
        let doc = merel_html::parse(&format!(
            r##"<table width="100%" bgcolor="#ffffff"><tr><td width="60" style="padding-left:32px"><img src="data:image/png;base64,{png}" width="60" height="60" alt="icoon"></td>
            <td style="padding-left:24px"><a href="https://example.nl/1" style="color:#ec6a1f;text-decoration:none">GRC Specialist</a></td></tr></table>"##
        ));
        assert!(doc.designed);
        let t = Tokens::light();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(700.0, 300.0)).build_ui(move |ui| {
            egui_extras::install_image_loaders(ui.ctx());
            show(ui, &t, &doc, &[], "m5", None, true, None);
        });
        for _ in 0..4 {
            h.run();
        }
        let icon = h.get_by_label("icoon").rect();
        assert!((icon.width() - 60.0).abs() < 0.5 && (icon.height() - 60.0).abs() < 0.5, "drawn at 60 px: {icon:?}");
        let title = h.get_by_label("GRC Specialist").rect();
        assert!(title.left() >= icon.right() + 24.0, "beside the picture: {title:?} {icon:?}");
        assert!((title.center().y - icon.center().y).abs() < 2.0, "in the middle of the row: {title:?} {icon:?}");
    }

    #[test]
    fn switched_off_links_are_drawn_but_a_click_does_nothing() {
        let doc = merel_html::parse(r#"<p><a href="mailto:sanne@example.org">mail Sanne</a></p>"#);
        let clicked = Rc::new(RefCell::new(None));
        let seen = clicked.clone();
        let t = Tokens::light();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(600.0, 300.0)).build_ui(move |ui| {
            if let Some(c) = show(ui, &t, &doc, &[], "m3", None, false, None) {
                *seen.borrow_mut() = Some(c);
            }
        });
        h.run();
        h.get_by_label("mail Sanne").click();
        h.run();
        assert_eq!(*clicked.borrow(), None);
    }
}
