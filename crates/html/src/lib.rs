//! HTML mail reduced to a layout tree the reading pane can draw with its own text engine.
//!
//! There is no browser here. The document is parsed with html5ever (the parser behind
//! html2text), then walked once against an allow-list: paragraphs, headings, lists, quotes,
//! tables, rules, preformatted text, emphasis, links and images survive; scripts, styles,
//! forms, frames, embedded objects and hidden blocks are dropped. Of the attributes only
//! `href`, `src` and `alt` pass as text; the look of the mail (colours, backgrounds, widths,
//! alignment, padding, text size and weight, from `style` and the old presentation attributes)
//! is read into numbers and fixed choices (see `css`), so a newsletter can be drawn the way
//! Outlook draws it without anything of its CSS being interpreted further. Links keep only `http`, `https` and `mailto` targets. Images stay
//! references: a picture embedded in the message (`cid:` or a `data:` URL) can be shown,
//! one on the internet is counted so the reading pane can say it blocked them. The walk is
//! bounded in depth and in nodes, so a hostile message cannot exhaust the stack or the heap.

mod css;
mod dom;

use base64::Engine as _;
use html5ever::tendril::TendrilSink;
use html5ever::tree_builder::TreeBuilderOpts;
use html5ever::{ParseOpts, parse_document};
use merel_model::clean_text;

use css::Declarations;
use dom::{Data, Dom, Node};

/// Deepest nesting that is followed; anything below is dropped.
const MAX_DEPTH: usize = 64;
/// Most nodes visited for one document; the rest is dropped and `truncated` set.
const MAX_NODES: usize = 50_000;
/// Longest link target kept.
const MAX_HREF: usize = 2048;
/// Longest image embedded as a `data:` URL that is decoded (bytes after decoding).
const MAX_DATA_IMAGE: usize = 4 * 1024 * 1024;

/// The whole message body, top to bottom.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Document {
    pub blocks: Vec<Block>,
    /// Pictures that live on the internet; none is loaded.
    pub remote_images: usize,
    /// True when the document was too large and its tail was dropped.
    pub truncated: bool,
    /// True when the message sets its own colours, so it is drawn on the light ground it was
    /// made for, as Outlook does, rather than in the colours of the theme.
    pub designed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Paragraph(Vec<Inline>),
    /// `level` is 1 to 6.
    Heading {
        level: u8,
        inlines: Vec<Inline>,
    },
    Quote(Vec<Block>),
    List {
        ordered: bool,
        items: Vec<Vec<Block>>,
    },
    /// Rows of cells of a table of data (with headers or borders), drawn with lines between
    /// the rows. A table that only lays the mail out becomes [`Block::Columns`] and
    /// [`Block::Section`] instead.
    Table(Vec<Vec<Cell>>),
    /// Cells side by side, as one row of a layout table puts them.
    Columns(Vec<Cell>),
    /// Content the author gave a look of its own: a background, a width, an alignment, room
    /// around it.
    Section {
        look: Look,
        blocks: Vec<Block>,
    },
    /// Preformatted text, line breaks kept.
    Code(String),
    Rule,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    pub header: bool,
    pub blocks: Vec<Block>,
    pub look: Look,
}

/// How a block of the mail looks, as far as Merel follows it. Every field is a number or a
/// fixed choice read from the message; see `css`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Look {
    pub background: Option<Rgb>,
    /// How the text inside lines up.
    pub align: Option<Align>,
    /// Where the block itself sits when it is narrower than the room there is.
    pub place: Option<Align>,
    /// How wide the content is, padding not included.
    pub width: Option<Length>,
    /// As narrow as its content allows, as a table without a width is (a button).
    pub fit: bool,
    /// At least this tall, in pixels (a spacer row).
    pub height: Option<u16>,
    /// Room inside the edge, in pixels: top, right, bottom, left.
    pub padding: [u16; 4],
    /// Where the content of a table cell sits when the row is taller; in the middle when
    /// unset, as in a browser.
    pub valign: Option<VAlign>,
}

impl Look {
    pub fn is_plain(&self) -> bool {
        *self == Look::default()
    }
}

/// A colour the message chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VAlign {
    Top,
    Middle,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Length {
    Px(u16),
    /// Of the width there is, 1 to 100.
    Percent(u8),
}

impl Length {
    /// The width in points out of `room`.
    pub fn of(self, room: f32) -> f32 {
        match self {
            Length::Px(px) => f32::from(px),
            Length::Percent(p) => room * f32::from(p) / 100.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    Text {
        text: String,
        style: Style,
    },
    /// `href` is `http://`, `https://` or `mailto:`; the text is what the message showed for it.
    Link {
        href: String,
        text: String,
        style: Style,
    },
    Image(Image),
    Break,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub code: bool,
    /// The colour the message gave the text; the theme's when unset.
    pub color: Option<Rgb>,
    /// The size the message gave the text, in pixels; the reading pane's own when unset.
    pub size: Option<u8>,
    /// A link the message drew without its underline.
    pub plain_link: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub source: ImageSource,
    pub alt: String,
    /// The size the message asked for (`width`, `height`, or the same in CSS); the picture's
    /// own when unset.
    pub width: Option<Length>,
    pub height: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageSource {
    /// A `cid:` reference to a part of the same message, without the `cid:` and brackets.
    Inline(String),
    /// Decoded from a `data:` URL in the message itself; shared so drawing it costs no copy.
    Data { mime: String, bytes: std::sync::Arc<[u8]> },
    /// On the internet; never fetched by this crate.
    Remote(String),
}

impl Document {
    /// Whether there is anything to show.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Every link in the document as (text, href), in reading order, for the phishing check.
    pub fn links(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for_each_run(&self.blocks, &mut |inlines| {
            for i in inlines {
                if let Inline::Link { href, text, .. } = i {
                    out.push((text.clone(), href.clone()));
                }
            }
        });
        out
    }

    /// The content ids of the embedded pictures the document refers to.
    pub fn inline_image_ids(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for_each_run(&self.blocks, &mut |inlines| {
            for i in inlines {
                if let Inline::Image(Image { source: ImageSource::Inline(cid), .. }) = i
                    && !out.contains(cid)
                {
                    out.push(cid.clone());
                }
            }
        });
        out
    }
}

/// Calls `f` with every run of inlines under `blocks` (each paragraph and heading), in
/// reading order.
pub fn for_each_run<'a>(blocks: &'a [Block], f: &mut impl FnMut(&'a [Inline])) {
    for b in blocks {
        match b {
            Block::Paragraph(inlines) | Block::Heading { inlines, .. } => f(inlines),
            Block::Quote(inner) | Block::Section { blocks: inner, .. } => for_each_run(inner, f),
            Block::List { items, .. } => items.iter().for_each(|i| for_each_run(i, f)),
            Block::Table(rows) => rows.iter().flatten().for_each(|c| for_each_run(&c.blocks, f)),
            Block::Columns(cells) => cells.iter().for_each(|c| for_each_run(&c.blocks, f)),
            Block::Code(_) | Block::Rule => {}
        }
    }
}

/// Parses `html` into a [`Document`]. Never fails: broken markup yields what could be read.
pub fn parse(html: &str) -> Document {
    let opts = ParseOpts { tree_builder: TreeBuilderOpts { scripting_enabled: false, ..Default::default() }, ..Default::default() };
    let dom = parse_document(Dom::default(), opts).one(html);
    let nodes = dom.nodes();
    let mut b = Builder { nodes: &nodes, budget: MAX_NODES, remote_images: 0, truncated: false, designed: false, aligned: false };
    let mut out = Blocks::default();
    b.children(0, &mut out, Style::default(), None, 0);
    let blocks = out.finish();
    Document { blocks, remote_images: b.remote_images, truncated: b.truncated, designed: b.designed }
}

/// Blocks under construction: finished ones, plus the run of inlines not yet closed into a
/// paragraph.
#[derive(Default)]
struct Blocks {
    done: Vec<Block>,
    cur: Vec<Inline>,
    /// Whitespace seen since the last word, to become one space before the next.
    space: bool,
}

impl Blocks {
    fn text(&mut self, text: &str, style: Style, href: Option<&str>) {
        for (i, word) in text.split(is_html_space).enumerate() {
            if i > 0 || word.is_empty() {
                // A separator came before this word (or the text started or ended with one).
                self.space = true;
            }
            if word.is_empty() {
                continue;
            }
            if self.space {
                // The space belongs to the run before it, so a bold word or a link starts
                // on its first letter.
                match self.cur.last_mut() {
                    None | Some(Inline::Break) => {}
                    Some(Inline::Text { text, style: s }) if *s == Style::default() || (*s == style && href.is_none()) => text.push(' '),
                    Some(Inline::Link { text, href: h, style: s }) if *s == style && Some(h.as_str()) == href => text.push(' '),
                    Some(_) => self.cur.push(Inline::Text { text: " ".into(), style: Style::default() }),
                }
            }
            self.space = false;
            self.push_text(word.to_string(), style, href);
        }
    }

    /// Text exactly as given (inside `<pre>`), split at line breaks.
    fn raw(&mut self, text: &str, style: Style, href: Option<&str>) {
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                self.cur.push(Inline::Break);
            }
            if !line.is_empty() {
                self.push_text(line.to_string(), style, href);
            }
        }
        self.space = false;
    }

    fn push_text(&mut self, piece: String, style: Style, href: Option<&str>) {
        match (self.cur.last_mut(), href) {
            (Some(Inline::Text { text, style: s }), None) if *s == style => text.push_str(&piece),
            (Some(Inline::Link { href: h, text, style: s }), Some(href)) if *s == style && h == href => text.push_str(&piece),
            (_, None) => self.cur.push(Inline::Text { text: piece, style }),
            (_, Some(href)) => self.cur.push(Inline::Link { href: href.to_string(), text: piece, style }),
        }
    }

    fn inline(&mut self, inline: Inline) {
        if matches!(inline, Inline::Break) {
            self.space = false;
        }
        self.cur.push(inline);
    }

    /// Closes the current paragraph, if it holds anything.
    fn flush(&mut self) {
        let inlines = self.take_inlines();
        if !inlines.is_empty() {
            self.done.push(Block::Paragraph(inlines));
        }
    }

    fn take_inlines(&mut self) -> Vec<Inline> {
        self.space = false;
        let mut inlines = std::mem::take(&mut self.cur);
        while matches!(inlines.last(), Some(Inline::Break)) {
            inlines.pop();
        }
        while matches!(inlines.first(), Some(Inline::Break)) {
            inlines.remove(0);
        }
        inlines
    }

    fn block(&mut self, block: Block) {
        self.flush();
        self.done.push(block);
    }

    fn finish(mut self) -> Vec<Block> {
        self.flush();
        self.done
    }
}

struct Builder<'a> {
    nodes: &'a [Node],
    budget: usize,
    remote_images: usize,
    truncated: bool,
    designed: bool,
    /// Whether an element around the one being read centres or right-aligns its text.
    aligned: bool,
}

/// Elements whose content is never shown.
const DROPPED: &[&str] = &[
    "script",
    "style",
    "head",
    "title",
    "meta",
    "link",
    "base",
    "noscript",
    "template",
    "iframe",
    "frame",
    "frameset",
    "object",
    "embed",
    "applet",
    "svg",
    "math",
    "canvas",
    "audio",
    "video",
    "source",
    "track",
    "input",
    "button",
    "select",
    "textarea",
    "option",
    "optgroup",
    "datalist",
    "map",
    "area",
    "dialog",
    "slot",
    "xmp",
    "plaintext",
];

/// Elements that start on a new line; their content is walked like a paragraph.
const BLOCKS: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "header",
    "footer",
    "main",
    "aside",
    "nav",
    "address",
    "center",
    "figure",
    "figcaption",
    "details",
    "summary",
    "fieldset",
    "legend",
    "form",
    "dl",
    "dt",
    "dd",
    "body",
    "html",
    "menu",
    "dir",
    "caption",
    "tr",
    "td",
    "th",
    "thead",
    "tbody",
    "tfoot",
    "colgroup",
    "col",
    "li",
    "hgroup",
    "search",
];

/// Elements whose own look (background, width, alignment, padding) becomes a
/// [`Block::Section`]. Tables and their cells take theirs in `Builder::table`.
const BOXES: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "header",
    "footer",
    "main",
    "aside",
    "nav",
    "address",
    "center",
    "figure",
    "body",
    "html",
    "hgroup",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "blockquote",
];

impl Builder<'_> {
    fn declarations(&self, node: &Node) -> Declarations {
        Declarations::parse(self.attr(node, "style").unwrap_or(""))
    }

    /// `style` with what element `name` and its `style` attribute change about the text.
    fn style_of(&mut self, node: &Node, name: &str, css: &Declarations, style: Style) -> Style {
        let mut s = style;
        match name {
            "b" | "strong" | "th" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => s.bold = true,
            "i" | "em" | "cite" | "dfn" | "var" => s.italic = true,
            "u" | "ins" => s.underline = true,
            "s" | "strike" | "del" => s.strike = true,
            "code" | "kbd" | "samp" | "tt" => s.code = true,
            // A link has its own colour and underline, whatever the text around it has.
            "a" => {
                s.color = None;
                s.underline = false;
                s.plain_link = false;
            }
            "font" => {
                if let Some(c) = self.attr(node, "color").and_then(css::color) {
                    s.color = Some(c);
                }
                if let Some(px) = self.attr(node, "size").and_then(css::font_size) {
                    s.size = Some(px as u8);
                }
            }
            _ => {}
        }
        if let Some(c) = css.get("color").and_then(css::color) {
            s.color = Some(c);
        }
        if let Some(px) = css.get("font-size").and_then(css::px) {
            s.size = Some(px.clamp(0.0, 72.0).round() as u8);
        }
        match css.get("font-weight") {
            Some("bold" | "bolder" | "600" | "700" | "800" | "900") => s.bold = true,
            Some("normal" | "lighter" | "100" | "200" | "300" | "400" | "500") => s.bold = false,
            _ => {}
        }
        match css.get("font-style") {
            Some("italic" | "oblique") => s.italic = true,
            Some("normal") => s.italic = false,
            _ => {}
        }
        if let Some(d) = css.get("text-decoration").or_else(|| css.get("text-decoration-line")) {
            if d.contains("none") {
                s.underline = false;
                s.strike = false;
                s.plain_link |= name == "a";
            }
            s.underline |= d.contains("underline");
            s.strike |= d.contains("line-through");
        }
        if s.color.is_some() {
            self.designed = true;
        }
        s
    }

    /// The look of a block element: what it says in attributes, then in CSS.
    fn look(&mut self, node: &Node, name: &str, css: &Declarations) -> Look {
        let mut look = Look { background: self.attr(node, "bgcolor").and_then(css::color), ..Look::default() };
        if let Some(c) = css.get("background-color").and_then(css::color).or_else(|| css.get("background").and_then(css::background)) {
            look.background = Some(c);
        }
        let align = self.attr(node, "align").and_then(css::align);
        if name == "table" {
            look.place = align;
        } else {
            look.align = align;
        }
        if name == "center" {
            look.align = Some(Align::Center);
        }
        if let Some(a) = css.get("text-align").and_then(css::align) {
            look.align = Some(a);
        }
        let margin = css.get("margin").map(|m| m.split_whitespace().collect::<Vec<_>>()).unwrap_or_default();
        let auto = |side: &str| css.get(side) == Some("auto");
        if matches!(margin.as_slice(), [_, "auto", ..] | ["auto"]) || (auto("margin-left") && auto("margin-right")) {
            look.place = Some(Align::Center);
        }
        if name != "body" && name != "html" {
            look.width = self.attr(node, "width").and_then(css::length);
        }
        if let Some(w) = css.get("width").and_then(css::length) {
            look.width = Some(w);
        }
        if let Some(Length::Px(max)) = css.get("max-width").and_then(css::length)
            && !matches!(look.width, Some(Length::Px(w)) if w <= max)
        {
            look.width = Some(Length::Px(max));
        }
        if let Some(h) = self.attr(node, "height").and_then(css::px).or_else(|| css.get("height").and_then(css::px)) {
            look.height = Some(css::short(h)).filter(|&h| h > 0);
        }
        if let Some(p) = css.get("padding").and_then(css::sides) {
            look.padding = p;
        }
        for (i, side) in ["padding-top", "padding-right", "padding-bottom", "padding-left"].into_iter().enumerate() {
            if let Some(px) = css.get(side).and_then(css::px) {
                look.padding[i] = css::short(px);
            }
        }
        look.valign = self.attr(node, "valign").and_then(css::valign).or_else(|| css.get("vertical-align").and_then(css::valign));
        if look.background.is_some() {
            self.designed = true;
        }
        look
    }

    fn attr<'n>(&self, node: &'n Node, name: &str) -> Option<&'n str> {
        match &node.data {
            Data::Element { attrs, .. } => attrs.iter().find(|a| &*a.name.local == name).map(|a| &*a.value),
            _ => None,
        }
    }

    fn hidden(&self, node: &Node) -> bool {
        if self.attr(node, "hidden").is_some() {
            return true;
        }
        let style: String = self.attr(node, "style").unwrap_or("").chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
        style.contains("display:none") || style.contains("visibility:hidden")
    }

    fn children(&mut self, id: usize, out: &mut Blocks, style: Style, href: Option<&str>, depth: usize) {
        let Some(node) = self.nodes.get(id) else { return };
        for &c in &node.children {
            self.visit(c, out, style, href, depth);
        }
    }

    /// Blocks for the content of `id`, closed off on their own (a list item, a cell, a quote).
    fn sub_blocks(&mut self, id: usize, style: Style, href: Option<&str>, depth: usize) -> Vec<Block> {
        let mut out = Blocks::default();
        self.children(id, &mut out, style, href, depth);
        out.finish()
    }

    fn visit(&mut self, id: usize, out: &mut Blocks, style: Style, href: Option<&str>, depth: usize) {
        if self.budget == 0 || depth > MAX_DEPTH {
            self.truncated = true;
            return;
        }
        self.budget -= 1;
        let Some(node) = self.nodes.get(id) else { return };
        let name = match &node.data {
            Data::Text(t) => {
                if style.size != Some(0) {
                    out.text(t, style, href);
                }
                return;
            }
            Data::Element { name, .. } => &*name.local,
            Data::Document => {
                self.children(id, out, style, href, depth + 1);
                return;
            }
            Data::Other => return,
        };
        if DROPPED.contains(&name) || self.hidden(node) {
            return;
        }
        let css = self.declarations(node);
        let style = self.style_of(node, name, &css, style);
        if BOXES.contains(&name) {
            let look = self.look(node, name, &css);
            if !look.is_plain() {
                out.flush();
                let mut inner = Blocks::default();
                let aligned = self.aligned;
                self.aligned = look.align.map_or(aligned, |a| a != Align::Left);
                self.element(id, node, name, &css, &mut inner, style, href, depth + 1);
                self.aligned = aligned;
                let blocks = inner.finish();
                if !blocks.is_empty() || look.height.is_some() {
                    out.done.push(Block::Section { look, blocks });
                }
                return;
            }
        }
        self.element(id, node, name, &css, out, style, href, depth + 1);
    }

    /// What element `name` adds to `out`, its look already dealt with.
    #[allow(clippy::too_many_arguments)]
    fn element(&mut self, id: usize, node: &Node, name: &str, css: &Declarations, out: &mut Blocks, style: Style, href: Option<&str>, depth: usize) {
        match name {
            "br" => out.inline(Inline::Break),
            "hr" => out.block(Block::Rule),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = name.as_bytes().get(1).map(|b| b.saturating_sub(b'0')).unwrap_or(1);
                out.flush();
                let mut inner = Blocks::default();
                self.children(id, &mut inner, style, href, depth);
                // A heading holds one run; anything that became its own block follows it.
                let mut blocks = inner.done;
                let inlines = {
                    let mut b = Blocks { cur: inner.cur, ..Default::default() };
                    b.take_inlines()
                };
                if !inlines.is_empty() {
                    out.done.push(Block::Heading { level, inlines });
                }
                out.done.append(&mut blocks);
            }
            "ul" | "ol" => {
                let ordered = name == "ol";
                let items = node
                    .children
                    .iter()
                    .filter(|&&c| matches!(self.nodes.get(c).map(|n| &n.data), Some(Data::Element { name, .. }) if &*name.local == "li"))
                    .copied()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .map(|li| self.sub_blocks(li, style, href, depth))
                    .collect::<Vec<_>>();
                if !items.is_empty() {
                    out.block(Block::List { ordered, items });
                }
            }
            "blockquote" => {
                let inner = self.sub_blocks(id, style, href, depth);
                if !inner.is_empty() {
                    out.block(Block::Quote(inner));
                }
            }
            "pre" => {
                let mut inner = Blocks::default();
                self.raw_children(id, &mut inner, Style { code: true, ..style }, depth);
                let text = inner.take_inlines().into_iter().fold(String::new(), |mut acc, i| {
                    match i {
                        Inline::Text { text, .. } | Inline::Link { text, .. } => acc.push_str(&text),
                        Inline::Break => acc.push('\n'),
                        Inline::Image(_) => {}
                    }
                    acc
                });
                if !text.trim().is_empty() {
                    out.block(Block::Code(text));
                }
            }
            "table" => self.table(node, out, style, href, depth),
            "a" => {
                let target = self.attr(node, "href").and_then(link_target);
                self.children(id, out, style, target.as_deref().or(href), depth);
            }
            "img" => {
                if let Some(image) = self.image(node, css) {
                    out.inline(Inline::Image(image));
                }
            }
            _ => {
                let block = BLOCKS.contains(&name);
                if block {
                    out.flush();
                }
                self.children(id, out, style, href, depth);
                if block {
                    out.flush();
                }
            }
        }
    }

    /// The text under `id` with its whitespace kept (for `<pre>`); tags inside add nothing but
    /// line breaks.
    fn raw_children(&mut self, id: usize, out: &mut Blocks, style: Style, depth: usize) {
        if self.budget == 0 || depth > MAX_DEPTH {
            self.truncated = true;
            return;
        }
        let Some(node) = self.nodes.get(id) else { return };
        for &c in &node.children {
            self.budget = self.budget.saturating_sub(1);
            match self.nodes.get(c).map(|n| &n.data) {
                Some(Data::Text(t)) => out.raw(t, style, None),
                Some(Data::Element { name, .. }) if &*name.local == "br" => out.inline(Inline::Break),
                Some(Data::Element { name, .. }) if !DROPPED.contains(&&*name.local) => self.raw_children(c, out, style, depth + 1),
                _ => {}
            }
        }
    }

    fn table(&mut self, table: &Node, out: &mut Blocks, style: Style, href: Option<&str>, depth: usize) {
        let css = self.declarations(table);
        let mut look = self.look(table, "table", &css);
        // Without a width a table is as narrow as its content; that shows once it is placed.
        look.fit = look.width.is_none() && (look.place.is_some() || self.aligned);
        // Text inside a table starts out on the left, whatever the cell around it said: the
        // `align` of that cell places the table, not its text.
        if self.aligned {
            look.align = look.align.or(Some(Align::Left));
        }
        let aligned = self.aligned;
        let cellpadding = self.attr(table, "cellpadding").and_then(css::px).map(css::short).unwrap_or(0);
        let border = self.attr(table, "border").and_then(css::px).is_some_and(|b| b > 0.0);
        let mut rows: Vec<Vec<Cell>> = Vec::new();
        let mut blocks: Vec<Block> = Vec::new();
        let mut row_ids = Vec::new();
        for &c in &table.children {
            match self.nodes.get(c).map(|n| &n.data) {
                Some(Data::Element { name, .. }) if &*name.local == "tr" => row_ids.push(c),
                Some(Data::Element { name, .. }) if matches!(&*name.local, "thead" | "tbody" | "tfoot") => {
                    if let Some(group) = self.nodes.get(c) {
                        row_ids.extend(
                            group
                                .children
                                .iter()
                                .filter(|&&r| matches!(self.nodes.get(r).map(|n| &n.data), Some(Data::Element { name, .. }) if &*name.local == "tr")),
                        );
                    }
                }
                Some(Data::Element { name, .. }) if &*name.local == "caption" => {
                    blocks.extend(self.sub_blocks(c, Style { bold: true, ..style }, href, depth));
                }
                _ => {}
            }
        }
        for r in row_ids {
            let Some(row) = self.nodes.get(r) else { continue };
            if self.hidden(row) {
                continue;
            }
            let row_css = self.declarations(row);
            let row_look = self.look(row, "tr", &row_css);
            let row_style = self.style_of(row, "tr", &row_css, style);
            let mut cells = Vec::new();
            for &c in &row.children {
                let Some(cell) = self.nodes.get(c) else { continue };
                let header = match &cell.data {
                    Data::Element { name, .. } if &*name.local == "td" => false,
                    Data::Element { name, .. } if &*name.local == "th" => true,
                    _ => continue,
                };
                if self.hidden(cell) {
                    continue;
                }
                let cell_css = self.declarations(cell);
                let cell_style = self.style_of(cell, if header { "th" } else { "td" }, &cell_css, row_style);
                let mut look = self.look(cell, "td", &cell_css);
                if look.padding == [0; 4] {
                    look.padding = [cellpadding; 4];
                }
                look.background = look.background.or(row_look.background);
                look.align = look.align.or(row_look.align);
                look.valign = look.valign.or(row_look.valign);
                self.aligned = look.align.is_some_and(|a| a != Align::Left);
                let blocks = self.sub_blocks(c, cell_style, href, depth);
                self.aligned = aligned;
                cells.push(Cell { header, blocks, look });
            }
            if cells.iter().any(|c| !c.blocks.is_empty() || c.look.height.is_some() || c.look.background.is_some()) {
                rows.push(cells);
            }
        }
        if border || rows.iter().flatten().any(|c| c.header) {
            // A table of data, drawn as one.
            if !rows.is_empty() {
                blocks.push(Block::Table(rows));
            }
        } else {
            // A layout table: each row as the author laid it out, cells side by side only
            // where there is more than one with something in it.
            for mut row in rows {
                let filled = row.iter().filter(|c| !c.blocks.is_empty() || c.look.background.is_some()).count();
                if filled > 1 {
                    blocks.push(Block::Columns(row));
                    continue;
                }
                let tallest = row.iter().filter_map(|c| c.look.height).max();
                let at = row.iter().position(|c| !c.blocks.is_empty() || c.look.background.is_some()).unwrap_or(0);
                if at >= row.len() {
                    continue;
                }
                let mut cell = row.swap_remove(at);
                cell.look.height = cell.look.height.max(tallest);
                // A lone cell is as wide as its table, whatever width it claims.
                cell.look.width = None;
                cell.look.fit = look.fit;
                if cell.look.is_plain() {
                    blocks.append(&mut cell.blocks);
                } else {
                    blocks.push(Block::Section { look: cell.look, blocks: cell.blocks });
                }
            }
        }
        out.flush();
        if look.is_plain() {
            out.done.append(&mut blocks);
        } else if !blocks.is_empty() {
            out.done.push(Block::Section { look, blocks });
        }
    }

    fn image(&mut self, node: &Node, css: &Declarations) -> Option<Image> {
        let src = self.attr(node, "src")?.trim();
        let alt = clean_text(self.attr(node, "alt").unwrap_or("").trim()).chars().take(200).collect();
        let lower = src.to_ascii_lowercase();
        let source = if let Some(cid) = lower.strip_prefix("cid:") {
            let cid = src.get(src.len() - cid.len()..).unwrap_or("").trim_matches(['<', '>']);
            if cid.is_empty() {
                return None;
            }
            ImageSource::Inline(clean_text(cid))
        } else if lower.starts_with("data:image/") {
            let (meta, data) = src.split_once(',')?;
            let mime = meta.get(5..)?.split(';').next()?.to_ascii_lowercase();
            if !meta.to_ascii_lowercase().contains(";base64")
                || !matches!(mime.as_str(), "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/bmp")
            {
                return None;
            }
            let data: String = data.chars().filter(|c| !c.is_whitespace()).collect();
            if data.len() / 4 * 3 > MAX_DATA_IMAGE {
                return None;
            }
            let bytes = base64::engine::general_purpose::STANDARD.decode(&data).ok()?;
            ImageSource::Data { mime, bytes: bytes.into() }
        } else if lower.starts_with("http://") || lower.starts_with("https://") {
            self.remote_images += 1;
            ImageSource::Remote(src.chars().take(MAX_HREF).collect())
        } else {
            return None;
        };
        let mut width = self.attr(node, "width").and_then(css::length);
        if let Some(w) = css.get("width").and_then(css::length) {
            width = Some(w);
        }
        if let Some(Length::Px(max)) = css.get("max-width").and_then(css::length)
            && !matches!(width, Some(Length::Px(w)) if w <= max)
        {
            width = Some(Length::Px(max));
        }
        let height = css.get("height").or_else(|| self.attr(node, "height")).and_then(css::length).and_then(|h| match h {
            Length::Px(px) => Some(px),
            Length::Percent(_) => None,
        });
        Some(Image { source, alt, width, height })
    }
}

/// The characters HTML collapses: ASCII whitespace, so a non-breaking space stays.
fn is_html_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0C')
}

/// `href` when it is a web or mail address Merel is willing to open.
fn link_target(href: &str) -> Option<String> {
    let href: String = href.chars().filter(|c| !c.is_control() && !c.is_whitespace()).collect();
    if href.chars().count() > MAX_HREF {
        return None;
    }
    let lower = href.to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")).then_some(href)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_listed_in_reading_order_with_their_text() {
        let d = parse(
            r#"<p>Zie <a href="https://example.nl">de site</a>.</p><ul><li><a href="mailto:s@example.org">mail</a></li></ul><blockquote><a href="https://b.example/x">https://a.example</a></blockquote>"#,
        );
        assert_eq!(
            d.links(),
            [
                ("de site".to_string(), "https://example.nl".to_string()),
                ("mail".to_string(), "mailto:s@example.org".to_string()),
                ("https://a.example".to_string(), "https://b.example/x".to_string()),
            ]
        );
    }

    fn text(blocks: &[Block]) -> String {
        fn inl(inlines: &[Inline], out: &mut String) {
            for i in inlines {
                match i {
                    Inline::Text { text, .. } | Inline::Link { text, .. } => out.push_str(text),
                    Inline::Image(img) => out.push_str(&format!("[img {}]", img.alt)),
                    Inline::Break => out.push('|'),
                }
            }
        }
        let mut out = String::new();
        for b in blocks {
            match b {
                Block::Paragraph(i) | Block::Heading { inlines: i, .. } => inl(i, &mut out),
                Block::Quote(inner) => out.push_str(&format!("> {}", text(inner))),
                Block::List { items, .. } => items.iter().for_each(|i| out.push_str(&format!("* {}", text(i)))),
                Block::Table(rows) => rows.iter().for_each(|r| r.iter().for_each(|c| out.push_str(&format!("[{}]", text(&c.blocks))))),
                Block::Columns(cells) => cells.iter().for_each(|c| out.push_str(&format!("({})", text(&c.blocks).trim_end()))),
                Block::Section { blocks, .. } => out.push_str(text(blocks).trim_end()),
                Block::Code(c) => out.push_str(&format!("`{c}`")),
                Block::Rule => out.push_str("---"),
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn dangerous_content_is_dropped_and_text_survives() {
        let html = r#"<html><head><title>T</title><style>p{color:red}</style></head><body>
            <script>alert(1)</script><noscript>no</noscript>
            <div style="display: none">preheader</div><p hidden>h</p>
            <p>Hallo   <b>daar</b>,<br>hoe <i>gaat</i> het?</p>
            <form><input value="x"><button>Klik</button>tekst</form>
            <iframe src="https://evil.example"></iframe><svg><script>1</script></svg>
            <textarea>t</textarea><object data="x"></object></body></html>"#;
        let d = parse(html);
        assert_eq!(text(&d.blocks), "Hallo daar,|hoe gaat het?\ntekst\n");
        assert!(!d.truncated && d.remote_images == 0);
        let Block::Paragraph(p) = &d.blocks[0] else { panic!("paragraph") };
        // A space between two runs goes to the plain one, so a bold word or a link starts on
        // its first letter and nothing styled trails it.
        assert_eq!(p[0], Inline::Text { text: "Hallo ".into(), style: Style::default() });
        assert_eq!(p[1], Inline::Text { text: "daar".into(), style: Style { bold: true, ..Style::default() } });
        assert_eq!(p[5], Inline::Text { text: "gaat".into(), style: Style { italic: true, ..Style::default() } });
        assert_eq!(p[6], Inline::Text { text: " het?".into(), style: Style::default() });
    }

    #[test]
    fn links_keep_only_safe_targets() {
        let html = r#"<a href="https://example.nl/x?a=1&amp;b=2">site</a> <a href="javascript:alert(1)">js</a>
            <a href="mailto:a@b.nl">mail</a> <a href="  HTTPS://x.nl/p a">spaced</a> <a href="ftp://x">ftp</a> <a href="/relative">rel</a>"#;
        let d = parse(html);
        let Block::Paragraph(p) = &d.blocks[0] else { panic!("paragraph") };
        let links: Vec<(&str, &str)> = p
            .iter()
            .filter_map(|i| match i {
                Inline::Link { href, text, .. } => Some((href.as_str(), text.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(links, vec![("https://example.nl/x?a=1&b=2", "site"), ("mailto:a@b.nl", "mail"), ("HTTPS://x.nl/pa", "spaced")]);
        assert_eq!(text(&d.blocks).trim(), "site js mail spaced ftp rel");
    }

    #[test]
    fn structure_is_kept() {
        let html = "<h2>Kop</h2><ul><li>een</li><li>twee <ul><li>diep</li></ul></li></ul><ol><li>x</li></ol>
            <blockquote><p>citaat</p></blockquote><hr><pre>a  b\n  c</pre>
            <table><caption>C</caption><thead><tr><th>K1</th><th>K2</th></tr></thead><tbody><tr><td>1</td><td>2</td></tr></tbody></table>
            <table><tr><td><p>alleen</p></td></tr><tr><td>layout</td></tr></table>";
        let d = parse(html);
        assert!(matches!(&d.blocks[0], Block::Heading { level: 2, .. }));
        let Block::List { ordered: false, items } = &d.blocks[1] else { panic!("list") };
        assert_eq!(items.len(), 2);
        assert!(matches!(items[1].as_slice(), [Block::Paragraph(_), Block::List { .. }]));
        assert!(matches!(&d.blocks[2], Block::List { ordered: true, .. }));
        assert!(matches!(&d.blocks[3], Block::Quote(_)));
        assert_eq!(d.blocks[4], Block::Rule);
        assert_eq!(d.blocks[5], Block::Code("a  b\n  c".into()));
        assert_eq!(text(&d.blocks[6..7]), "C\n");
        let Block::Table(rows) = &d.blocks[7] else { panic!("table, got {:?}", d.blocks[7]) };
        assert_eq!(rows.len(), 2);
        assert!(rows[0][0].header && !rows[1][0].header);
        assert_eq!(text(&d.blocks[8..]), "alleen\nlayout\n", "a one-column table is flattened");
    }

    #[test]
    fn images_are_classified_and_never_fetched() {
        let png = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG....");
        let html = format!(
            r#"<img src="cid:&lt;part1@x&gt;" alt="Logo"><img src="https://t.example/px.gif" width=1 height=1>
            <img src="data:image/png;base64,{png}"><img src="data:text/html;base64,PHA+"><img src="file:///etc/passwd"><img src="cid:part1@x">"#
        );
        let d = parse(&html);
        assert_eq!(d.remote_images, 1);
        assert_eq!(d.inline_image_ids(), vec!["part1@x".to_string()]);
        let Block::Paragraph(p) = &d.blocks[0] else { panic!("paragraph") };
        assert_eq!(p.len(), 4, "{p:?}");
        assert!(matches!(&p[0], Inline::Image(Image { source: ImageSource::Inline(c), alt, .. }) if c == "part1@x" && alt == "Logo"));
        assert!(matches!(&p[1], Inline::Image(Image { source: ImageSource::Remote(u), .. }) if u == "https://t.example/px.gif"));
        assert!(
            matches!(&p[2], Inline::Image(Image { source: ImageSource::Data { mime, bytes }, .. }) if mime == "image/png" && &bytes[..] == b"\x89PNG....")
        );
    }

    /// A newsletter laid out with tables, the way mail tools write them (Freelance.nl's
    /// "Nieuwe matches"): a grey ground, a 600 px column in the middle, an orange header, rows
    /// of a 60 px icon beside a title, a button.
    const NEWSLETTER: &str = r##"<body style="margin:0;background-color:#eaeef2">
        <table width="100%" bgcolor="#eaeef2" cellpadding="0" cellspacing="0" border="0"><tr><td align="center" style="padding:24px 16px">
         <table width="600" cellpadding="0" cellspacing="0" border="0" style="max-width:600px">
          <tr><td bgcolor="#ec7a31" align="center" style="padding:40px 24px;font-size:32px;font-weight:bold;color:#ffffff">Nieuwe matches</td></tr>
          <tr><td bgcolor="#ffffff" style="padding:32px 40px;color:#1a1a1a">
            <p style="font-size:17px">Dit zijn de nieuwste opdrachten.</p>
            <table width="100%" cellpadding="0" cellspacing="0" border="0">
              <tr><td width="60" style="padding:12px 0 12px 32px"><img src="https://www.freelance.nl/img/match.png" width="60" height="60" alt=""></td>
                  <td valign="top" style="padding-left:24px"><a href="https://www.freelance.nl/opdracht/1" style="color:#ec6a1f;font-weight:bold;text-decoration:none">GRC Specialist</a></td></tr>
            </table>
            <table cellpadding="0" cellspacing="0" border="0" style="margin:28px auto 8px auto"><tr><td bgcolor="#ec7a31" style="padding:14px 28px"><a href="https://www.freelance.nl/matches" style="color:#fff">Bekijk alle matches</a></td></tr></table>
          </td></tr>
         </table>
        </td></tr></table></body>"##;

    fn sections(blocks: &[Block]) -> Vec<(&Look, &[Block])> {
        let mut out = Vec::new();
        for b in blocks {
            if let Block::Section { look, blocks } = b {
                out.push((look, blocks.as_slice()));
                out.extend(sections(blocks));
            }
            if let Block::Columns(cells) = b {
                cells.iter().for_each(|c| out.extend(sections(&c.blocks)));
            }
        }
        out
    }

    fn columns(blocks: &[Block]) -> Vec<&[Cell]> {
        let mut out = Vec::new();
        for b in blocks {
            match b {
                Block::Columns(cells) => out.push(cells.as_slice()),
                Block::Section { blocks, .. } => out.extend(columns(blocks)),
                _ => {}
            }
        }
        out
    }

    #[test]
    fn a_newsletter_keeps_its_look() {
        let d = parse(NEWSLETTER);
        assert!(d.designed, "it sets its own colours, so it is drawn on its own ground");
        let all = sections(&d.blocks);
        let grey = Some(Rgb(0xea, 0xee, 0xf2));
        assert!(all.iter().any(|(l, _)| l.background == grey), "the grey ground");
        let column = all.iter().find(|(l, _)| l.width == Some(Length::Px(600))).expect("the 600 px column");
        assert_eq!(column.0.place, None, "placed by the cell around it, which centres it");
        let header = all.iter().find(|(l, _)| l.background == Some(Rgb(0xec, 0x7a, 0x31)) && l.align == Some(Align::Center)).expect("orange header");
        assert_eq!(header.0.padding, [40, 24, 40, 24]);
        let Block::Paragraph(p) = &header.1[0] else { panic!("header text, got {:?}", header.1) };
        let Inline::Text { text, style } = &p[0] else { panic!("text") };
        assert_eq!(text, "Nieuwe matches");
        assert!(style.bold && style.size == Some(32) && style.color == Some(Rgb(255, 255, 255)));

        let rows = columns(&d.blocks);
        assert_eq!(rows.len(), 1, "the icon beside its title");
        let [icon, title] = rows[0] else { panic!("two cells") };
        assert_eq!(icon.look.width, Some(Length::Px(60)));
        assert_eq!(icon.look.padding, [12, 0, 12, 32]);
        assert_eq!(title.look.valign, Some(VAlign::Top));
        let Block::Paragraph(p) = &icon.blocks[0] else { panic!("icon") };
        let Inline::Image(img) = &p[0] else { panic!("image") };
        assert_eq!((img.width, img.height), (Some(Length::Px(60)), Some(60)), "drawn at 60 px, not its own size");
        let Block::Paragraph(p) = &title.blocks[0] else { panic!("title") };
        let Inline::Link { style, .. } = &p[0] else { panic!("link") };
        assert!(style.plain_link && style.bold && style.color == Some(Rgb(0xec, 0x6a, 0x1f)));

        let button = all.iter().find(|(l, _)| l.place == Some(Align::Center)).expect("the centred button");
        assert!(button.0.fit, "a table without a width is as narrow as its content");
        assert_eq!(button.1.len(), 1);
        let Block::Section { look, .. } = &button.1[0] else { panic!("the button cell") };
        assert!(look.fit && look.background == Some(Rgb(0xec, 0x7a, 0x31)));
    }

    #[test]
    fn a_table_starts_its_text_on_the_left_and_plain_mail_stays_plain() {
        let d = parse(r#"<div align="center"><table><tr><td>links</td></tr></table></div>"#);
        let all = sections(&d.blocks);
        assert_eq!(all[0].0.align, Some(Align::Center));
        assert_eq!(all[1].0.align, Some(Align::Left), "the centring places the table, not its text");
        let d = parse("<p>Hoi <b>Sanne</b>,</p><p>tot morgen.</p>");
        assert!(!d.designed && sections(&d.blocks).is_empty());
        // A font size of 0 hides the space between inline-block columns; there is no text.
        assert_eq!(text(&parse(r#"<div style="font-size:0">  <span>&nbsp;</span></div>"#).blocks), "");
    }

    #[test]
    fn hostile_nesting_and_size_are_bounded() {
        let deep = "<div>".repeat(5000) + "diep" + &"</div>".repeat(5000);
        let d = parse(&deep);
        assert!(d.truncated);
        let many = "<p>x</p>".repeat(60_000);
        let d = parse(&many);
        assert!(d.truncated && d.blocks.len() < 60_000);
        assert_eq!(parse(""), Document::default());
        assert_eq!(parse("<p>   </p><br><br>"), Document::default());
    }

    #[test]
    fn entities_and_whitespace_follow_html_rules() {
        assert_eq!(text(&parse("<p>de site <a href='https://x.nl'>een twee</a> drie</p>").blocks), "de site een twee drie\n");
        let d = parse("<p>Caf&eacute;\n\n  &amp; <span> thee </span>&nbsp;!</p><p>twee</p>");
        // A regular space before a non-breaking one stays, as in a browser.
        assert_eq!(text(&d.blocks), "Café & thee \u{a0}!\ntwee\n");
    }
}
