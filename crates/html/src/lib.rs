//! HTML mail reduced to a layout tree the reading pane can draw with its own text engine.
//!
//! There is no browser here. The document is parsed with html5ever (the parser behind
//! html2text), then walked once against an allow-list: paragraphs, headings, lists, quotes,
//! tables, rules, preformatted text, emphasis, links and images survive; scripts, styles,
//! forms, frames, embedded objects, hidden blocks and every attribute other than `href`, `src`
//! and `alt` are dropped. Links keep only `http`, `https` and `mailto` targets. Images stay
//! references: a picture embedded in the message (`cid:` or a `data:` URL) can be shown,
//! one on the internet is counted so the reading pane can say it blocked them. The walk is
//! bounded in depth and in nodes, so a hostile message cannot exhaust the stack or the heap.

mod dom;

use base64::Engine as _;
use html5ever::tendril::TendrilSink;
use html5ever::tree_builder::TreeBuilderOpts;
use html5ever::{ParseOpts, parse_document};
use mailcraft_model::clean_text;

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
    /// Rows of cells; a table with one column is flattened before it gets here.
    Table(Vec<Vec<Cell>>),
    /// Preformatted text, line breaks kept.
    Code(String),
    Rule,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    pub header: bool,
    pub blocks: Vec<Block>,
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub source: ImageSource,
    pub alt: String,
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

    /// The content ids of the embedded pictures the document refers to.
    /// Every link in the document as (text, href), in reading order, for the phishing check.
    pub fn links(&self) -> Vec<(String, String)> {
        fn walk(blocks: &[Block], out: &mut Vec<(String, String)>) {
            for b in blocks {
                match b {
                    Block::Paragraph(inlines) | Block::Heading { inlines, .. } => {
                        for i in inlines {
                            if let Inline::Link { href, text, .. } = i {
                                out.push((text.clone(), href.clone()));
                            }
                        }
                    }
                    Block::Quote(inner) => walk(inner, out),
                    Block::List { items, .. } => items.iter().for_each(|i| walk(i, out)),
                    Block::Table(rows) => rows.iter().flatten().for_each(|c| walk(&c.blocks, out)),
                    Block::Code(_) | Block::Rule => {}
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.blocks, &mut out);
        out
    }

    pub fn inline_image_ids(&self) -> Vec<String> {
        fn walk(blocks: &[Block], out: &mut Vec<String>) {
            for b in blocks {
                match b {
                    Block::Paragraph(inlines) | Block::Heading { inlines, .. } => {
                        for i in inlines {
                            if let Inline::Image(Image { source: ImageSource::Inline(cid), .. }) = i
                                && !out.contains(cid)
                            {
                                out.push(cid.clone());
                            }
                        }
                    }
                    Block::Quote(inner) => walk(inner, out),
                    Block::List { items, .. } => items.iter().for_each(|i| walk(i, out)),
                    Block::Table(rows) => rows.iter().flatten().for_each(|c| walk(&c.blocks, out)),
                    Block::Code(_) | Block::Rule => {}
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.blocks, &mut out);
        out
    }
}

/// Parses `html` into a [`Document`]. Never fails: broken markup yields what could be read.
pub fn parse(html: &str) -> Document {
    let opts = ParseOpts { tree_builder: TreeBuilderOpts { scripting_enabled: false, ..Default::default() }, ..Default::default() };
    let dom = parse_document(Dom::default(), opts).one(html);
    let nodes = dom.nodes();
    let mut b = Builder { nodes: &nodes, budget: MAX_NODES, remote_images: 0, truncated: false };
    let mut out = Blocks::default();
    b.children(0, &mut out, Style::default(), None, 0);
    let blocks = out.finish();
    Document { blocks, remote_images: b.remote_images, truncated: b.truncated }
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

impl Builder<'_> {
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
        let (name, _) = match &node.data {
            Data::Text(t) => {
                out.text(t, style, href);
                return;
            }
            Data::Element { name, attrs } => (&*name.local, attrs),
            Data::Document => {
                self.children(id, out, style, href, depth + 1);
                return;
            }
            Data::Other => return,
        };
        if DROPPED.contains(&name) || self.hidden(node) {
            return;
        }
        let depth = depth + 1;
        let mut s = style;
        match name {
            "br" => out.inline(Inline::Break),
            "hr" => out.block(Block::Rule),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = name.as_bytes().get(1).map(|b| b.saturating_sub(b'0')).unwrap_or(1);
                out.flush();
                let mut inner = Blocks::default();
                self.children(id, &mut inner, Style { bold: true, ..style }, href, depth);
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
            "table" => self.table(id, out, style, href, depth),
            "a" => {
                let target = self.attr(node, "href").and_then(link_target);
                self.children(id, out, style, target.as_deref().or(href), depth);
            }
            "img" => {
                if let Some(image) = self.image(node) {
                    out.inline(Inline::Image(image));
                }
            }
            _ => {
                match name {
                    "b" | "strong" => s.bold = true,
                    "i" | "em" | "cite" | "dfn" | "var" => s.italic = true,
                    "u" | "ins" => s.underline = true,
                    "s" | "strike" | "del" => s.strike = true,
                    "code" | "kbd" | "samp" | "tt" => s.code = true,
                    _ => {}
                }
                let block = BLOCKS.contains(&name);
                if block {
                    out.flush();
                }
                self.children(id, out, s, href, depth);
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

    fn table(&mut self, id: usize, out: &mut Blocks, style: Style, href: Option<&str>, depth: usize) {
        let Some(table) = self.nodes.get(id) else { return };
        let mut rows: Vec<Vec<Cell>> = Vec::new();
        let mut before: Vec<Block> = Vec::new();
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
                    before.extend(self.sub_blocks(c, Style { bold: true, ..style }, href, depth));
                }
                _ => {}
            }
        }
        for r in row_ids {
            let Some(row) = self.nodes.get(r) else { continue };
            let mut cells = Vec::new();
            for &c in &row.children {
                let header = match self.nodes.get(c).map(|n| &n.data) {
                    Some(Data::Element { name, .. }) if &*name.local == "td" => false,
                    Some(Data::Element { name, .. }) if &*name.local == "th" => true,
                    _ => continue,
                };
                let blocks = self.sub_blocks(c, if header { Style { bold: true, ..style } } else { style }, href, depth);
                cells.push(Cell { header, blocks });
            }
            if cells.iter().any(|c| !c.blocks.is_empty()) {
                rows.push(cells);
            }
        }
        out.flush();
        out.done.append(&mut before);
        if rows.iter().all(|r| r.iter().filter(|c| !c.blocks.is_empty()).count() <= 1) {
            // A layout table: one thing under the other, as the mail's author laid it out.
            for row in rows {
                for cell in row {
                    out.done.extend(cell.blocks);
                }
            }
        } else if !rows.is_empty() {
            out.done.push(Block::Table(rows));
        }
    }

    fn image(&mut self, node: &Node) -> Option<Image> {
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
        Some(Image { source, alt })
    }
}

/// The characters HTML collapses: ASCII whitespace, so a non-breaking space stays.
fn is_html_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0C')
}

/// `href` when it is a web or mail address Mailcraft is willing to open.
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
        assert!(matches!(&p[0], Inline::Image(Image { source: ImageSource::Inline(c), alt }) if c == "part1@x" && alt == "Logo"));
        assert!(matches!(&p[1], Inline::Image(Image { source: ImageSource::Remote(u), .. }) if u == "https://t.example/px.gif"));
        assert!(
            matches!(&p[2], Inline::Image(Image { source: ImageSource::Data { mime, bytes }, .. }) if mime == "image/png" && &bytes[..] == b"\x89PNG....")
        );
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
