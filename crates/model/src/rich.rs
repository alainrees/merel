//! Formatted text, the way Outlook's editor makes it: bold, italic, underlined, struck through,
//! a typeface, a size, a text colour and a highlight on stretches of the text, and bullets,
//! numbering, indentation and alignment on its paragraphs.
//!
//! The text itself stays a plain string, so search, quoting, templates and the plain-text part
//! of a message keep working on it; [`Rich`] lies over it and says how each character looks
//! and how each line is laid out. HTML is made from it here and nowhere else: every character
//! of the text is escaped, and the only tags and style values written are ones this module
//! chooses from its own enums and numbers. Neither the text nor a draft file edited by hand can
//! put markup, a script, a link or an attribute into a message.

use std::ops::Range;

use serde::{Deserialize, Serialize};

/// The size text has unless chosen otherwise, in points.
pub const DEFAULT_SIZE: u8 = 12;
/// The sizes the editor offers, in points, as Outlook does.
pub const SIZES: [u8; 14] = [8, 9, 10, 11, 12, 14, 16, 18, 20, 24, 28, 36, 48, 72];
/// Smallest and largest size kept, in points.
const SIZE_RANGE: (u8, u8) = (6, 96);
/// Deepest indentation kept, in steps.
pub const MAX_INDENT: u8 = 8;

/// A typeface. The app carries each one, so the editor shows what the reader will see; the
/// message names well-known look-alikes after it for readers who lack it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Face {
    #[default]
    Sans,
    Serif,
    Mono,
}

impl Face {
    pub const ALL: [Face; 3] = [Face::Sans, Face::Serif, Face::Mono];

    /// The name the editor shows.
    pub fn name(self) -> &'static str {
        match self {
            Face::Sans => "Inter",
            Face::Serif => "DejaVu Serif",
            Face::Mono => "DejaVu Sans Mono",
        }
    }

    /// The CSS font list, with look-alikes for a reader who does not have the font.
    fn css(self) -> &'static str {
        match self {
            Face::Sans => "Inter, Aptos, Calibri, Helvetica, Arial, sans-serif",
            Face::Serif => "'DejaVu Serif', Georgia, 'Times New Roman', serif",
            Face::Mono => "'DejaVu Sans Mono', Consolas, 'Courier New', monospace",
        }
    }
}

/// A colour as red, green and blue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

fn is_false(b: &bool) -> bool {
    !b
}

fn is_sans(f: &Face) -> bool {
    *f == Face::Sans
}

/// How a stretch of text looks. The default is plain text in the message's own font and size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    #[serde(skip_serializing_if = "is_false")]
    pub bold: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub italic: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub underline: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub strike: bool,
    #[serde(skip_serializing_if = "is_sans")]
    pub face: Face,
    /// In points; `None` is [`DEFAULT_SIZE`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<Rgb>,
    /// The marker colour behind the text, Outlook's "Markeren".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight: Option<Rgb>,
}

impl Style {
    /// The size in points.
    pub fn points(&self) -> u8 {
        self.size.unwrap_or(DEFAULT_SIZE)
    }

    /// The same style with the size inside the kept range, and the default size as `None`.
    fn normalized(mut self) -> Self {
        self.size = self.size.map(|s| s.clamp(SIZE_RANGE.0, SIZE_RANGE.1)).filter(|s| *s != DEFAULT_SIZE);
        self
    }
}

/// The kind of list a paragraph belongs to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum List {
    #[default]
    None,
    Bullet,
    Number,
}

/// How a paragraph's lines sit between the margins.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// How one paragraph (one line of the text) is laid out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Para {
    pub list: List,
    /// Steps in from the left margin.
    pub indent: u8,
    pub align: Align,
}

/// A stretch of `len` characters in one style.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub len: usize,
    #[serde(default)]
    pub style: Style,
}

/// The formatting over a text: runs that together cover every character, and one [`Para`]
/// per line. [`Rich::fit`] makes any value, whatever a file said, cover its text exactly.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rich {
    pub runs: Vec<Run>,
    pub paras: Vec<Para>,
}

fn push_run(out: &mut Vec<Run>, len: usize, style: Style) {
    if len == 0 {
        return;
    }
    match out.last_mut() {
        Some(last) if last.style == style => last.len += len,
        _ => out.push(Run { len, style }),
    }
}

/// The character ranges of the lines of `text`, without their line breaks.
pub fn line_ranges(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    for line in text.split('\n') {
        let end = start + line.chars().count();
        out.push(start..end);
        start = end + 1;
    }
    out
}

/// The line `pos` (a character index) falls on.
pub fn line_of(text: &str, pos: usize) -> usize {
    text.chars().take(pos).filter(|c| *c == '\n').count()
}

/// `rich` fitted over `text`, or `None` when there is nothing to keep: how an item stores the
/// formatting of its notes.
pub fn fitted(rich: Option<Rich>, text: &str) -> Option<Rich> {
    rich.map(|mut r| {
        r.fit(text);
        r
    })
    .filter(|r| !r.is_plain())
}

impl Rich {
    /// Plain formatting over `text`.
    pub fn plain(text: &str) -> Self {
        let mut r = Rich::default();
        r.fit(text);
        r
    }

    /// Whether nothing is formatted, so the text can go out as it is.
    pub fn is_plain(&self) -> bool {
        self.runs.iter().all(|r| r.style == Style::default()) && self.paras.iter().all(|p| *p == Para::default())
    }

    /// Makes the formatting cover `text` exactly: runs cut or stretched to its length, one
    /// paragraph per line, sizes and indents within bounds.
    pub fn fit(&mut self, text: &str) {
        let mut left = text.chars().count();
        let mut runs = Vec::with_capacity(self.runs.len());
        for run in self.runs.drain(..) {
            let len = run.len.min(left);
            left -= len;
            push_run(&mut runs, len, run.style.normalized());
        }
        let last = runs.last().map(|r| r.style).unwrap_or_default();
        push_run(&mut runs, left, last);
        self.runs = runs;
        let lines = text.split('\n').count();
        self.paras.resize(lines, Para::default());
        for p in &mut self.paras {
            p.indent = p.indent.min(MAX_INDENT);
        }
    }

    /// The style of the character at `pos`.
    pub fn style_of(&self, pos: usize) -> Style {
        let mut start = 0;
        for run in &self.runs {
            if pos < start + run.len {
                return run.style;
            }
            start += run.len;
        }
        self.runs.last().map(|r| r.style).unwrap_or_default()
    }

    /// The style typing at `pos` continues: that of the character before it, as in Outlook.
    pub fn style_at(&self, pos: usize) -> Style {
        self.style_of(pos.saturating_sub(1))
    }

    /// The style of each run that touches `range`, in order.
    pub fn styles_in(&self, range: Range<usize>) -> Vec<Style> {
        let mut out = Vec::new();
        let mut start = 0;
        for run in &self.runs {
            let end = start + run.len;
            if start < range.end && range.start < end {
                out.push(run.style);
            }
            start = end;
        }
        out
    }

    /// Removes `remove` characters at `at` and puts `insert` new ones in `style` there.
    fn splice(&mut self, at: usize, remove: usize, insert: usize, style: Style) {
        let mut out = Vec::with_capacity(self.runs.len() + 2);
        let mut start = 0;
        let mut inserted = false;
        for run in &self.runs {
            let end = start + run.len;
            if start < at {
                push_run(&mut out, end.min(at) - start, run.style);
            }
            if !inserted && end >= at {
                push_run(&mut out, insert, style);
                inserted = true;
            }
            let after = start.max(at + remove);
            if end > after {
                push_run(&mut out, end - after, run.style);
            }
            start = end;
        }
        if !inserted {
            push_run(&mut out, insert, style);
        }
        self.runs = out;
    }

    /// Follows an edit of the text from `old` to `new`: what was typed or pasted takes the style
    /// `typing`, the rest keeps its own. A new line continues the paragraph it was made in (a
    /// list stays a list); lines joined keep the first one's layout.
    pub fn edited(&mut self, old: &str, new: &str, typing: Style) {
        self.edited_at(old, new, typing, None);
    }

    /// [`Rich::edited`] for an edit made at `cursor`: where a change could have been made in
    /// more than one place (a line break after another one), it is taken to be there. Enter at
    /// the end of a list item then continues the list, Enter before a line keeps that line's.
    pub fn edited_at(&mut self, old: &str, new: &str, typing: Style, cursor: Option<usize>) {
        let (o, n): (Vec<char>, Vec<char>) = (old.chars().collect(), new.chars().collect());
        let same = o.iter().zip(&n).take_while(|(a, b)| a == b).count();
        let prefix = cursor.map_or(same, |c| same.min(c));
        let room = o.len().min(n.len()) - prefix;
        let suffix = o.iter().rev().zip(n.iter().rev()).take(room).take_while(|(a, b)| a == b).count();
        let (removed, inserted) = (o.len() - prefix - suffix, n.len() - prefix - suffix);
        let line = o[..prefix].iter().filter(|c| **c == '\n').count();
        let gone = o[prefix..prefix + removed].iter().filter(|c| **c == '\n').count();
        let added = n[prefix..prefix + inserted].iter().filter(|c| **c == '\n').count();
        let base = self.paras.get(line).copied().unwrap_or_default();
        let from = (line + 1).min(self.paras.len());
        let to = (line + 1 + gone).min(self.paras.len());
        self.paras.splice(from..to, std::iter::repeat_n(base, added));
        self.splice(prefix, removed, inserted, typing.normalized());
        self.fit(new);
    }

    /// Changes the style of every character in `range`.
    pub fn apply(&mut self, range: Range<usize>, f: impl Fn(&mut Style)) {
        let mut out = Vec::with_capacity(self.runs.len() + 2);
        let mut start = 0;
        for run in &self.runs {
            let end = start + run.len;
            let (a, b) = (start.max(range.start).min(end), end.min(range.end).max(start));
            push_run(&mut out, a - start, run.style);
            if b > a {
                let mut s = run.style;
                f(&mut s);
                push_run(&mut out, b - a, s.normalized());
            }
            push_run(&mut out, end - b.max(a), run.style);
            start = end;
        }
        self.runs = out;
    }

    /// Changes the layout of lines `lines` (inclusive of both ends).
    pub fn apply_paras(&mut self, lines: Range<usize>, f: impl Fn(&mut Para)) {
        let end = lines.end.min(self.paras.len());
        for p in &mut self.paras[lines.start.min(end)..end] {
            f(p);
            p.indent = p.indent.min(MAX_INDENT);
        }
    }

    /// The marker before line `i`: "•" for a bullet, "3." for the third of a numbered list.
    /// Numbering counts the lines before it in the same list at the same depth.
    pub fn marker(&self, i: usize) -> Option<String> {
        let p = self.paras.get(i)?;
        match p.list {
            List::None => None,
            List::Bullet => Some("•".into()),
            List::Number => {
                let mut n = 1;
                for q in self.paras[..i].iter().rev() {
                    if q.list == List::None || q.indent < p.indent {
                        break;
                    }
                    if q.list == List::Number && q.indent == p.indent {
                        n += 1;
                    }
                }
                Some(format!("{n}."))
            }
        }
    }

    /// The text as it goes out in the plain-text part: list markers and indentation written
    /// out, the rest as typed.
    pub fn to_plain(&self, text: &str) -> String {
        if self.paras.iter().all(|p| p.list == List::None && p.indent == 0) {
            return text.to_string();
        }
        text.split('\n')
            .enumerate()
            .map(|(i, line)| {
                let p = self.paras.get(i).copied().unwrap_or_default();
                let pad = "    ".repeat(usize::from(p.indent));
                match self.marker(i) {
                    Some(m) => format!("{pad}{m} {line}"),
                    None => format!("{pad}{line}"),
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The text as an HTML document. Only this function writes markup, and only its own.
    pub fn to_html(&self, text: &str) -> String {
        let mut fit = self.clone();
        fit.fit(text);
        let chars: Vec<char> = text.chars().collect();
        let mut body = String::new();
        // The list open around the lines being written: its kind and depth.
        let mut open: Option<(List, u8)> = None;
        for (i, range) in line_ranges(text).into_iter().enumerate() {
            let p = fit.paras.get(i).copied().unwrap_or_default();
            let want = (p.list != List::None).then_some((p.list, p.indent));
            if open != want {
                if let Some((kind, _)) = open {
                    body.push_str(if kind == List::Number { "</ol>" } else { "</ul>" });
                }
                if let Some((kind, indent)) = want {
                    let tag = if kind == List::Number { "ol" } else { "ul" };
                    body.push_str(&format!("<{tag} style=\"margin-top: 0; margin-bottom: 0; margin-left: {}px\">", 24 + 40 * u32::from(indent)));
                }
                open = want;
            }
            let mut css = String::new();
            if p.list == List::None && p.indent > 0 {
                css.push_str(&format!("margin-left: {}px; ", 40 * u32::from(p.indent)));
            }
            match p.align {
                Align::Left => {}
                Align::Center => css.push_str("text-align: center; "),
                Align::Right => css.push_str("text-align: right; "),
            }
            let tag = if open.is_some() { "li" } else { "div" };
            body.push('<');
            body.push_str(tag);
            if !css.is_empty() {
                body.push_str(&format!(" style=\"{}\"", css.trim_end()));
            }
            body.push('>');
            if range.is_empty() {
                body.push_str("<br>");
            }
            let mut start = 0;
            for run in &fit.runs {
                let end = start + run.len;
                let (a, b) = (start.max(range.start), end.min(range.end));
                if a < b {
                    body.push_str(&span(&run.style, &chars[a..b]));
                }
                start = end;
            }
            body.push_str("</");
            body.push_str(tag);
            body.push('>');
        }
        if let Some((kind, _)) = open {
            body.push_str(if kind == List::Number { "</ol>" } else { "</ul>" });
        }
        format!(
            "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"></head><body><div style=\"font-family: {}; font-size: {DEFAULT_SIZE}pt\">{body}</div></body></html>\n",
            Face::Sans.css()
        )
    }
}

/// One stretch of text in its style, escaped.
fn span(style: &Style, text: &[char]) -> String {
    let mut css = String::new();
    if style.face != Face::Sans {
        css.push_str(&format!("font-family: {}; ", style.face.css()));
    }
    if let Some(size) = style.size {
        css.push_str(&format!("font-size: {size}pt; "));
    }
    if let Some(c) = style.color {
        css.push_str(&format!("color: {}; ", c.hex()));
    }
    if let Some(c) = style.highlight {
        css.push_str(&format!("background-color: {}; ", c.hex()));
    }
    let tags: Vec<&str> = [(style.bold, "b"), (style.italic, "i"), (style.underline, "u"), (style.strike, "s")]
        .into_iter()
        .filter_map(|(on, tag)| on.then_some(tag))
        .collect();
    let mut out = String::new();
    if !css.is_empty() {
        out.push_str(&format!("<span style=\"{}\">", css.trim_end()));
    }
    for tag in &tags {
        out.push_str(&format!("<{tag}>"));
    }
    out.push_str(&escape(text));
    for tag in tags.iter().rev() {
        out.push_str(&format!("</{tag}>"));
    }
    if !css.is_empty() {
        out.push_str("</span>");
    }
    out
}

/// `text` safe inside HTML: the five special characters as entities, a run of spaces kept by
/// turning every other one (and a leading one) into a no-break space, a tab as four.
fn escape(text: &[char]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev_space = true;
    for c in text {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '\t' => out.push_str("&nbsp;&nbsp;&nbsp;&nbsp;"),
            ' ' if prev_space => out.push_str("&nbsp;"),
            c if c.is_control() => {}
            c => out.push(*c),
        }
        prev_space = *c == ' ' && !prev_space;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold() -> Style {
        Style { bold: true, ..Style::default() }
    }

    #[test]
    fn fit_makes_any_formatting_cover_its_text() {
        let mut r = Rich { runs: vec![Run { len: 3, style: bold() }, Run { len: 100, style: Style::default() }], paras: vec![] };
        r.fit("hallo\nwereld");
        assert_eq!(r.runs, [Run { len: 3, style: bold() }, Run { len: 9, style: Style::default() }]);
        assert_eq!(r.paras.len(), 2);
        let mut r = Rich { runs: vec![Run { len: 2, style: bold() }], paras: vec![Para { indent: 200, ..Para::default() }; 5] };
        r.fit("abcd");
        assert_eq!(r.runs, [Run { len: 4, style: bold() }], "stretched with the last style");
        assert_eq!(r.paras, [Para { indent: MAX_INDENT, ..Para::default() }]);
        let mut r = Rich { runs: vec![Run { len: 2, style: Style { size: Some(12), ..Style::default() } }], paras: vec![] };
        r.fit("ab");
        assert!(r.is_plain(), "the default size is no formatting");
        assert!(Rich::plain("").runs.is_empty());
    }

    #[test]
    fn typing_takes_the_style_and_the_rest_keeps_its_own() {
        let mut r = Rich::plain("Hallo wereld");
        r.apply(0..5, |s| s.bold = true);
        assert_eq!(r.runs, [Run { len: 5, style: bold() }, Run { len: 7, style: Style::default() }]);
        // Typing inside the bold word continues it.
        r.edited("Hallo wereld", "Halllo wereld", r.style_at(4));
        assert_eq!(r.runs[0], Run { len: 6, style: bold() });
        // Replacing a selection across both: "lo we" becomes "X" in plain.
        r.edited("Halllo wereld", "HallXreld", Style::default());
        assert_eq!(r.runs, [Run { len: 4, style: bold() }, Run { len: 5, style: Style::default() }]);
        // Deleting everything leaves nothing.
        r.edited("HallXreld", "", Style::default());
        assert!(r.runs.is_empty() && r.paras.len() == 1);
    }

    #[test]
    fn a_new_line_continues_its_list_and_joined_lines_keep_the_first_layout() {
        let mut r = Rich::plain("een\ntwee");
        r.apply_paras(0..1, |p| p.list = List::Bullet);
        // Enter at the end of "een": without the cursor the change could as well be before "twee".
        r.edited_at("een\ntwee", "een\n\ntwee", Style::default(), Some(3));
        assert_eq!(r.paras.iter().map(|p| p.list).collect::<Vec<_>>(), [List::Bullet, List::Bullet, List::None]);
        // Enter before "twee" keeps that line's layout for both.
        let mut other = Rich::plain("een\ntwee");
        other.apply_paras(0..1, |p| p.list = List::Bullet);
        other.edited_at("een\ntwee", "een\n\ntwee", Style::default(), Some(4));
        assert_eq!(other.paras.iter().map(|p| p.list).collect::<Vec<_>>(), [List::Bullet, List::None, List::None]);
        r.edited_at("een\n\ntwee", "een\ndrie\ntwee", Style::default(), Some(4));
        r.edited_at("een\ndrie\ntwee", "eendrie\ntwee", Style::default(), Some(4));
        assert_eq!(r.paras.iter().map(|p| p.list).collect::<Vec<_>>(), [List::Bullet, List::None]);
    }

    #[test]
    fn numbering_counts_within_its_list() {
        let mut r = Rich::plain("a\nb\nc\nd\ne");
        r.apply_paras(0..3, |p| p.list = List::Number);
        r.apply_paras(1..2, |p| p.indent = 1);
        r.apply_paras(4..5, |p| p.list = List::Number);
        let markers: Vec<Option<String>> = (0..5).map(|i| r.marker(i)).collect();
        assert_eq!(markers, [Some("1.".into()), Some("1.".into()), Some("2.".into()), None, Some("1.".into())]);
        assert_eq!(r.to_plain("a\nb\nc\nd\ne"), "1. a\n    1. b\n2. c\nd\n1. e");
    }

    #[test]
    fn html_escapes_the_text_and_writes_only_its_own_markup() {
        let text = "<script>alert('x')</script> & \"zo\"\nVet  en\ttab";
        let mut r = Rich::plain(text);
        let bold_start = text.find("Vet").map(|b| text[..b].chars().count()).unwrap_or(0);
        r.apply(bold_start..bold_start + 3, |s| {
            s.bold = true;
            s.color = Some(Rgb(0xc0, 0, 0));
        });
        let html = r.to_html(text);
        assert!(!html.contains("<script"), "{html}");
        assert!(html.contains("&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt; &amp; &quot;zo&quot;"), "{html}");
        assert!(html.contains("<span style=\"color: #c00000;\"><b>Vet</b></span>&nbsp; en&nbsp;&nbsp;&nbsp;&nbsp;tab"), "{html}");
        assert!(html.starts_with("<!DOCTYPE html>") && html.contains("<meta charset=\"utf-8\">"));
    }

    #[test]
    fn html_puts_lists_alignment_and_empty_lines_in_place() {
        let text = "Punten\neen\ntwee\n\nMidden";
        let mut r = Rich::plain(text);
        r.apply_paras(1..3, |p| p.list = List::Bullet);
        r.apply_paras(4..5, |p| p.align = Align::Center);
        r.apply(0..6, |s| {
            s.size = Some(18);
            s.face = Face::Serif;
            s.highlight = Some(Rgb(255, 255, 0));
        });
        let html = r.to_html(text);
        assert!(html.contains("<div><span style=\"font-family: 'DejaVu Serif', Georgia, 'Times New Roman', serif; font-size: 18pt; background-color: #ffff00;\">Punten</span></div>"), "{html}");
        assert!(
            html.contains("<ul style=\"margin-top: 0; margin-bottom: 0; margin-left: 24px\"><li>een</li><li>twee</li></ul><div><br></div>"),
            "{html}"
        );
        assert!(html.contains("<div style=\"text-align: center;\">Midden</div>"), "{html}");
    }

    #[test]
    fn formatting_from_a_file_cannot_break_out_of_its_bounds() {
        let json = r#"{"runs":[{"len":99999999,"style":{"bold":true,"size":250}}],"paras":[{"list":"number","indent":255}]}"#;
        let mut r: Rich = serde_json::from_str(json).expect("valid json");
        r.fit("x");
        assert_eq!(r.runs, [Run { len: 1, style: Style { bold: true, size: Some(96), ..Style::default() } }]);
        assert_eq!(r.paras[0].indent, MAX_INDENT);
        assert!(serde_json::from_str::<Rich>(r#"{"runs":[{"len":1,"style":{"face":"<script>"}}]}"#).is_err(), "only known faces");
    }
}
